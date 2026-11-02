//! Real-time updates, phase 1: one listener per server, and the streams it feeds.
//!
//! Every record write sends `pg_notify('scaffoldry_changes', ...)` inside its transaction (`repository::write_history`),
//! so PostgreSQL delivers it to every server only if the write commits. Each server runs one listener thread on its own
//! connection, named `scaffoldry-realtime`, that republishes each notice on a broadcast channel of 256 per app. A stream
//! that falls further behind is told to `resync`. A lost listener connection is reopened with backoff, and every stream
//! is told to `resync`, because notices sent meanwhile are gone. Nothing here is a copy of stored data: it is the open
//! connections of this process, so no other server can be wrong about it.

use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex, Weak};
use std::time::Duration;
use postgres::fallible_iterator::FallibleIterator;
use tokio::sync::broadcast;

/// What a stream is sent.
#[derive(Debug, Clone)]
pub enum Event {
    /// `{ table, record, version, kind }`.
    Change(Value),
    /// Events may have been missed. Reload.
    Resync,
}

/// How far a reader may fall behind before it is told to resync.
pub const BACKLOG: usize = 256;
pub const CHANNEL: &str = "scaffoldry_changes";
pub const DEFAULT_MAX_PER_USER: usize = 20;
pub const DEFAULT_MAX_STREAMS: usize = 5_000;

/// The open streams of this process.
#[derive(Default)]
pub struct Hub {
    channels: Mutex<HashMap<String, broadcast::Sender<Event>>>,
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
        let mut client = match conn.and_then(|mut c| c.batch_execute(&format!("LISTEN {CHANNEL}")).map(|_| c)) {
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
        loop {
            let Some(h) = hub.upgrade() else { return };
            let mut notes = client.notifications();
            let mut it = notes.timeout_iter(Duration::from_secs(1));
            let mut failed = false;
            loop {
                match it.next() {
                    Ok(Some(n)) => {
                        if let Some((app, change)) = serde_json::from_str::<Value>(n.payload()).ok().as_ref().and_then(change_of) {
                            h.publish(&app, Event::Change(change));
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
