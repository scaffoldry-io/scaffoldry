//! Real-time updates, phase 1: one listener per server, and the streams it feeds.
//!
//! Every record write sends `pg_notify('scaffoldry_changes', ...)` inside its transaction (`repository::write_history`),
//! so PostgreSQL delivers it to every server only if the write commits. Each server runs one listener thread on its own
//! connection, named `scaffoldry-realtime`, that republishes each notice on a broadcast channel of 256 per app. A stream
//! that falls further behind is told to `resync`. A lost listener connection is reopened with backoff, and every stream
//! is told to `resync`, because notices sent meanwhile are gone. Nothing here is a copy of stored data: it is the open
//! connections of this process, so no other server can be wrong about it.
//!
//! Phase 3 adds two more channels on the same listener. `scaffoldry_presence` carries every server's joins, pings and
//! leaves, so each server's presence map shows who has an app open anywhere. Someone not heard from within the timeout
//! (45 seconds) is dropped. `scaffoldry_me` carries a person's own events, sent by table triggers: a notice's id and
//! count, a job's state and percent, a proposal's status.

use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex, Weak};
use std::time::{Duration, Instant};
use postgres::fallible_iterator::FallibleIterator;
use tokio::sync::broadcast;

/// What a stream is sent.
#[derive(Debug, Clone)]
pub enum Event {
    /// `{ table, record, version, kind }`.
    Change(Value),
    /// Events may have been missed. Reload.
    Resync,
    /// Someone joined or left: `{ joined | left: { address, name }, count }`.
    Presence(Value),
}

/// How far a reader may fall behind before it is told to resync.
pub const BACKLOG: usize = 256;
pub const CHANNEL: &str = "scaffoldry_changes";
pub const PRESENCE_CHANNEL: &str = "scaffoldry_presence";
pub const ME_CHANNEL: &str = "scaffoldry_me";
/// How often the desk pings presence while a stream is open.
pub const PRESENCE_PING: Duration = Duration::from_secs(15);

/// How long someone may be silent before they are dropped from presence: 45 seconds. A test sets it shorter.
pub fn presence_timeout() -> Duration {
    let secs = std::env::var("SCAFFOLDRY_PRESENCE_TIMEOUT_SECS").ok().and_then(|v| v.parse::<u64>().ok()).filter(|n| *n > 0).unwrap_or(45);
    Duration::from_secs(secs)
}

/// One open stream, as presence knows it.
struct Seen {
    address: String,
    name: String,
    at: Instant,
}
pub const DEFAULT_MAX_PER_USER: usize = 20;
pub const DEFAULT_MAX_STREAMS: usize = 5_000;

/// The open streams of this process.
#[derive(Default)]
pub struct Hub {
    channels: Mutex<HashMap<String, broadcast::Sender<Event>>>,
    /// Who has each app open, on any server: app, then stream id.
    presence: Mutex<HashMap<String, HashMap<String, Seen>>>,
    /// Each signed-in person's own events, by lowercase address.
    me: Mutex<HashMap<String, broadcast::Sender<Value>>>,
    per_user: Mutex<HashMap<String, usize>>,
    total: AtomicUsize,
    started: AtomicBool,
    /// Set once the listener has run `LISTEN`, so a stream opened after it hears every later write.
    listening: (Mutex<bool>, Condvar),
}

/// A place among the open streams. Dropping it frees the place.
pub struct Permit {
    hub: Arc<Hub>,
    user: String,
}

impl Drop for Permit {
    fn drop(&mut self) {
        self.hub.total.fetch_sub(1, Ordering::SeqCst);
        if let Ok(mut m) = self.hub.per_user.lock() {
            if let Some(n) = m.get_mut(&self.user) {
                *n = n.saturating_sub(1);
                if *n == 0 {
                    m.remove(&self.user);
                }
            }
        }
    }
}

impl Hub {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// A place for one more stream, or `None` past either limit.
    pub fn admit(self: &Arc<Self>, user: &str, max_per_user: usize, max_streams: usize) -> Option<Permit> {
        let user = user.to_lowercase();
        let mut m = self.per_user.lock().ok()?;
        let mine = m.get(&user).copied().unwrap_or(0);
        if mine >= max_per_user || self.total.load(Ordering::SeqCst) >= max_streams {
            return None;
        }
        m.insert(user.clone(), mine + 1);
        self.total.fetch_add(1, Ordering::SeqCst);
        Some(Permit { hub: self.clone(), user })
    }

    pub fn subscribe(&self, app: &str) -> broadcast::Receiver<Event> {
        let mut m = self.channels.lock().unwrap_or_else(|p| p.into_inner());
        m.entry(app.to_string()).or_insert_with(|| broadcast::channel(BACKLOG).0).subscribe()
    }

    fn publish(&self, app: &str, event: Event) {
        let m = self.channels.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(tx) = m.get(app) {
            let _ = tx.send(event);
        }
    }

    /// A person's own events.
    pub fn subscribe_me(&self, who: &str) -> broadcast::Receiver<Value> {
        let mut m = self.me.lock().unwrap_or_else(|p| p.into_inner());
        m.entry(who.to_lowercase()).or_insert_with(|| broadcast::channel(BACKLOG).0).subscribe()
    }

    fn publish_me(&self, who: &str, event: Value) {
        let mut m = self.me.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(tx) = m.get(who) {
            if tx.send(event).is_err() {
                // Nobody listens any more.
                m.remove(who);
            }
        }
    }

    /// Who has `app` open: each person once, by address.
    pub fn present(&self, app: &str) -> Vec<Value> {
        let m = self.presence.lock().unwrap_or_else(|p| p.into_inner());
        let mut people: Vec<(String, String)> = m.get(app).map(|s| s.values().map(|v| (v.address.clone(), v.name.clone())).collect()).unwrap_or_default();
        people.sort();
        people.dedup_by(|a, b| a.0 == b.0);
        people.into_iter().map(|(address, name)| json!({ "address": address, "name": name })).collect()
    }

    fn count(streams: &HashMap<String, Seen>) -> usize {
        let mut a: Vec<&str> = streams.values().map(|s| s.address.as_str()).collect();
        a.sort_unstable();
        a.dedup();
        a.len()
    }

    /// A stream joined or pinged. The first stream of a person is `joined`.
    fn seen(&self, app: &str, stream: &str, address: &str, name: &str) {
        let event = {
            let mut m = self.presence.lock().unwrap_or_else(|p| p.into_inner());
            let streams = m.entry(app.to_string()).or_default();
            let new_person = !streams.values().any(|s| s.address == address);
            match streams.get_mut(stream) {
                // A ping refreshes only a stream of the same person.
                Some(s) if s.address == address => s.at = Instant::now(),
                Some(_) => return,
                None => {
                    streams.insert(stream.to_string(), Seen { address: address.to_string(), name: name.to_string(), at: Instant::now() });
                }
            }
            new_person.then(|| json!({ "joined": { "address": address, "name": name }, "count": Self::count(streams) }))
        };
        if let Some(e) = event {
            self.publish(app, Event::Presence(e));
        }
    }

    /// Removes streams. A person whose last stream went is `left`.
    fn forget(&self, app: &str, gone: impl Fn(&str, &Seen) -> bool) {
        let events = {
            let mut m = self.presence.lock().unwrap_or_else(|p| p.into_inner());
            let Some(streams) = m.get_mut(app) else { return };
            let removed: Vec<(String, String)> = streams.iter().filter(|(id, s)| gone(id, s)).map(|(id, s)| (id.clone(), s.address.clone())).collect();
            let mut events = Vec::new();
            for (id, address) in removed {
                let name = streams.remove(&id).map(|s| s.name).unwrap_or_default();
                if !streams.values().any(|s| s.address == address) {
                    events.push(json!({ "left": { "address": address, "name": name }, "count": Self::count(streams) }));
                }
            }
            if streams.is_empty() {
                m.remove(app);
            }
            events
        };
        for e in events {
            self.publish(app, Event::Presence(e));
        }
    }

    /// Drops everyone silent for longer than `timeout`.
    fn sweep(&self, timeout: Duration) {
        let apps: Vec<String> = self.presence.lock().unwrap_or_else(|p| p.into_inner()).keys().cloned().collect();
        for app in apps {
            self.forget(&app, |_, s| s.at.elapsed() > timeout);
        }
    }

    /// One presence notice from any server: `{ app, stream, address, name, here }`.
    fn presence_notice(&self, n: &Value) {
        let (Some(app), Some(stream), Some(address)) = (n["app"].as_str(), n["stream"].as_str(), n["address"].as_str()) else { return };
        if n["here"].as_bool() == Some(true) {
            self.seen(app, stream, address, n["name"].as_str().unwrap_or(address));
        } else {
            self.forget(app, |id, s| id == stream && s.address == address);
        }
    }

    fn resync_all(&self) {
        let m = self.channels.lock().unwrap_or_else(|p| p.into_inner());
        for tx in m.values() {
            let _ = tx.send(Event::Resync);
        }
    }

    fn set_listening(&self, on: bool) {
        let (lock, cv) = &self.listening;
        if let Ok(mut l) = lock.lock() {
            *l = on;
            cv.notify_all();
        }
    }

    /// Starts the listener the first time a stream opens, and waits until it is listening, up to `wait`.
    pub fn ensure_listening(self: &Arc<Self>, database_url: &str, wait: Duration) -> bool {
        if !self.started.swap(true, Ordering::SeqCst) {
            let hub = Arc::downgrade(self);
            let url = database_url.to_string();
            let _ = std::thread::Builder::new().name("scaffoldry-realtime".to_string()).spawn(move || listen(hub, url));
        }
        let (lock, cv) = &self.listening;
        let Ok(guard) = lock.lock() else { return false };
        let (guard, _) = cv.wait_timeout_while(guard, wait, |on| !*on).unwrap_or_else(|p| p.into_inner());
        *guard
    }
}

/// The change a stream is sent for a notice: four keys, no other.
pub fn change_of(notice: &Value) -> Option<(String, Value)> {
    let app = notice.get("app")?.as_str()?.to_string();
    Some((app, json!({ "table": notice.get("table")?, "record": notice.get("record")?, "version": notice.get("version")?, "kind": notice.get("kind")? })))
}

/// The listener: one connection, `LISTEN`, republish. It ends when the hub is gone.
fn listen(hub: Weak<Hub>, url: String) {
    let mut backoff = Duration::from_millis(100);
    let mut lost_before = false;
    loop {
        let Some(h) = hub.upgrade() else { return };
        let conn = url.parse::<postgres::Config>().and_then(|mut cfg| {
            cfg.application_name("scaffoldry-realtime");
            cfg.connect(postgres::NoTls)
        });
        let mut client = match conn.and_then(|mut c| c.batch_execute(&format!("LISTEN {CHANNEL}; LISTEN {PRESENCE_CHANNEL}; LISTEN {ME_CHANNEL}")).map(|_| c)) {
            Ok(c) => c,
            Err(_) => {
                drop(h);
                std::thread::sleep(backoff);
                backoff = (backoff * 2).min(Duration::from_secs(5));
                continue;
            }
        };
        backoff = Duration::from_millis(100);
        if lost_before {
            // Notices sent while the connection was down are gone. Every stream reloads.
            h.resync_all();
        }
        h.set_listening(true);
        drop(h);
        let mut swept = Instant::now();
        loop {
            let Some(h) = hub.upgrade() else { return };
            let mut notes = client.notifications();
            let mut it = notes.timeout_iter(Duration::from_secs(1));
            let mut failed = false;
            loop {
                match it.next() {
                    Ok(Some(n)) => {
                        // A busy channel never goes quiet for a second, so the sweep runs here too.
                        if swept.elapsed() >= Duration::from_secs(1) {
                            h.sweep(presence_timeout());
                            swept = Instant::now();
                        }
                        let Ok(payload) = serde_json::from_str::<Value>(n.payload()) else { continue };
                        match n.channel() {
                            PRESENCE_CHANNEL => h.presence_notice(&payload),
                            ME_CHANNEL => {
                                if let Some(who) = payload["to"].as_str().map(str::to_string) {
                                    h.publish_me(&who, payload);
                                }
                            }
                            _ => {
                                if let Some((app, change)) = change_of(&payload) {
                                    h.publish(&app, Event::Change(change));
                                }
                            }
                        }
                    }
                    Ok(None) => break,
                    Err(_) => {
                        failed = true;
                        break;
                    }
                }
            }
            drop(it);
            drop(notes);
            h.sweep(presence_timeout());
            swept = Instant::now();
            drop(h);
            if failed || client.is_closed() {
                break;
            }
        }
        if let Some(h) = hub.upgrade() {
            h.set_listening(false);
        }
        lost_before = true;
    }
}
