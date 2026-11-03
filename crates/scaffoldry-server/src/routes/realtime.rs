//! Real-time phase 1: `GET /apps/{slug}/events`, the changes to one app as server-sent events.
//!
//! Events: `ping` when the stream opens and every 15 seconds, `change` for each committed write, `resync` when changes
//! may have been missed, and `closed` when the reader may no longer read the app. A change names the table, the record,
//! its version and the kind of write. It carries no value, so a reader fetches the record through the usual checks.
//!
//! Phase 3: the stream also carries `presence`, the list of who has the app open when it connects (with this stream's
//! id) and then `joined` and `left`. The desk pings `POST /apps/{slug}/presence` with that id every 15 seconds. Who
//! may read the app may see who else has it open, and nobody else. `GET /me/events` carries the signed-in person's own
//! `notification`, `job` and `proposal` events: ids, counts and states, never words.

use crate::guard::session_user;
use crate::realtime::{self, Event};
use crate::service::access::{authorize_app, AppAction};
use crate::service::ServiceError;
use crate::state::{AuthUser, SharedState};
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::sse::{Event as SseEvent, Sse};
use axum::routing::get;
use axum::{Json, Router};
use serde_json::{json, Value};
use std::convert::Infallible;
use std::pin::Pin;
use std::task::{Context, Poll};
use std::time::Duration;
use tokio::sync::{broadcast, mpsc};

pub fn router() -> Router<SharedState> {
    Router::new()
        .route("/apps/{slug}/events", get(events))
        .route("/apps/{slug}/presence", axum::routing::post(ping))
        .route("/me/events", get(me_events))
}

/// Tells every server, through the database, that a stream is here or gone.
async fn tell_presence(state: &SharedState, notice: Value) {
    let state = state.clone();
    let _ = tokio::task::spawn_blocking(move || send_presence(&state, &notice)).await;
}

fn send_presence(state: &SharedState, notice: &Value) {
    let payload = notice.to_string();
    let _ = state.repository.with_client(move |c| {
        c.execute("SELECT pg_notify($1, $2)", &[&realtime::PRESENCE_CHANNEL, &payload])?;
        Ok(())
    });
}

/// Says the stream is gone when it ends, however it ends.
struct Leave {
    state: SharedState,
    notice: Value,
}

impl Drop for Leave {
    fn drop(&mut self) {
        let (state, notice) = (self.state.clone(), self.notice.clone());
        match tokio::runtime::Handle::try_current() {
            Ok(h) => {
                h.spawn_blocking(move || send_presence(&state, &notice));
            }
            Err(_) => send_presence(&state, &notice),
        }
    }
}

fn presence_of(app: &str, stream: &str, user: &AuthUser, here: bool) -> Value {
    let name = if user.name.trim().is_empty() { user.eppn.clone() } else { user.name.clone() };
    json!({ "app": app, "stream": stream, "address": user.eppn.to_lowercase(), "name": name, "here": here })
}

/// `POST /apps/{slug}/presence` with `{ stream }`: this person still has the app open.
async fn ping(State(state): State<SharedState>, headers: HeaderMap, Path(slug): Path<String>, Json(body): Json<Value>) -> Result<StatusCode, (StatusCode, Json<Value>)> {
    let user = session_user(&state, &headers).ok_or_else(|| ServiceError::unauthorized("Unauthorized").into_pair())?;
    may_read(&state, &user, &slug, false).await.map_err(|e| match e {
        ServiceError::Internal(_) | ServiceError::StorageUnavailable(_) => e.into_pair(),
        _ => ServiceError::forbidden("You may not read this app").into_pair(),
    })?;
    let stream = body.get("stream").and_then(Value::as_str).filter(|s| !s.is_empty() && s.len() <= 64).ok_or_else(|| ServiceError::bad_request("A stream id is needed").into_pair())?;
    tell_presence(&state, presence_of(&slug, stream, &user, true)).await;
    Ok(StatusCode::NO_CONTENT)
}

/// `GET /me/events`: the signed-in person's own events.
async fn me_events(State(state): State<SharedState>, headers: HeaderMap) -> Result<Sse<Body>, (StatusCode, Json<Value>)> {
    let user = session_user(&state, &headers).ok_or_else(|| ServiceError::unauthorized("Unauthorized").into_pair())?;
    let hub = state.realtime_streams.clone();
    let permit = hub
        .admit(&user.eppn, limit(&state, "realtime.max_streams_per_user", realtime::DEFAULT_MAX_PER_USER), limit(&state, "realtime.max_streams", realtime::DEFAULT_MAX_STREAMS))
        .ok_or_else(|| ServiceError::RateLimited("Too many open event streams. Close one and try again".into()).into_pair())?;
    let url = state.repository.database_url();
    let listening = {
        let hub = hub.clone();
        tokio::task::spawn_blocking(move || hub.ensure_listening(&url, Duration::from_secs(5))).await.unwrap_or(false)
    };
    if !listening {
        return Err(ServiceError::StorageUnavailable("Live updates are not available right now".into()).into_pair());
    }
    let mut mine = hub.subscribe_me(&user.eppn);
    let (tx, rx) = mpsc::channel::<SseEvent>(16);
    tokio::spawn(async move {
        let _permit = permit;
        if tx.send(named("ping", &json!({}))).await.is_err() {
            return;
        }
        let mut ping = tokio::time::interval(Duration::from_secs(15));
        ping.tick().await;
        loop {
            let next = tokio::select! {
                got = mine.recv() => match got {
                    Ok(mut e) => {
                        let kind = e["type"].as_str().unwrap_or("").to_string();
                        let Some(o) = e.as_object_mut() else { continue };
                        o.remove("to");
                        o.remove("type");
                        match kind.as_str() {
                            "notification" | "job" | "proposal" => named(&kind, &e),
                            _ => continue,
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(_)) => named("resync", &json!({})),
                    Err(broadcast::error::RecvError::Closed) => return,
                },
                _ = ping.tick() => named("ping", &json!({})),
                _ = tx.closed() => return,
            };
            if tx.send(next).await.is_err() {
                return;
            }
        }
    });
    Ok(Sse::new(Body(rx)))
}

/// The receiving end of a stream, as the body `Sse` reads.
struct Body(mpsc::Receiver<SseEvent>);

impl futures_core::Stream for Body {
    type Item = Result<SseEvent, Infallible>;
    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.0.poll_recv(cx).map(|e| e.map(Ok))
    }
}

fn limit(state: &SharedState, key: &str, default: usize) -> usize {
    state.settings.read().ok().and_then(|s| s.get(key).and_then(Value::as_u64)).filter(|n| *n > 0).map(|n| n as usize).unwrap_or(default)
}

/// How often a stream checks that its reader may still read the app. A test sets it short.
fn reauth_every() -> Duration {
    let secs = std::env::var("SCAFFOLDRY_REALTIME_REAUTH_SECS").ok().and_then(|v| v.parse::<u64>().ok()).filter(|n| *n > 0).unwrap_or(60);
    Duration::from_secs(secs)
}

fn named(name: &str, data: &Value) -> SseEvent {
    SseEvent::default().event(name).data(data.to_string())
}

async fn may_read(state: &SharedState, user: &AuthUser, slug: &str, refresh: bool) -> Result<(), ServiceError> {
    let (state, user, slug) = (state.clone(), user.clone(), slug.to_string());
    tokio::task::spawn_blocking(move || {
        if refresh {
            state.refresh().map_err(|e| ServiceError::Internal(e.to_string()))?;
        }
        authorize_app(&user, &slug, AppAction::Read, &state)
    })
    .await
    .map_err(|_| ServiceError::internal("The check stopped"))?
}

async fn events(State(state): State<SharedState>, headers: HeaderMap, Path(slug): Path<String>) -> Result<Sse<Body>, (StatusCode, Json<Value>)> {
    let user = session_user(&state, &headers).ok_or_else(|| ServiceError::unauthorized("Unauthorized").into_pair())?;
    may_read(&state, &user, &slug, false).await.map_err(|e| match e {
        ServiceError::Internal(_) | ServiceError::StorageUnavailable(_) => e.into_pair(),
        _ => ServiceError::forbidden("You may not read this app").into_pair(),
    })?;
    let hub = state.realtime_streams.clone();
    let permit = hub
        .admit(&user.eppn, limit(&state, "realtime.max_streams_per_user", realtime::DEFAULT_MAX_PER_USER), limit(&state, "realtime.max_streams", realtime::DEFAULT_MAX_STREAMS))
        .ok_or_else(|| ServiceError::RateLimited("Too many open event streams. Close one and try again".into()).into_pair())?;
    let url = state.repository.database_url();
    let listening = {
        let hub = hub.clone();
        tokio::task::spawn_blocking(move || hub.ensure_listening(&url, Duration::from_secs(5))).await.unwrap_or(false)
    };
    if !listening {
        return Err(ServiceError::StorageUnavailable("Live updates are not available right now".into()).into_pair());
    }
    let mut changes = hub.subscribe(&slug);
    let stream = uuid::Uuid::new_v4().to_string();
    let (tx, rx) = mpsc::channel::<SseEvent>(16);
    tokio::spawn(async move {
        let _permit = permit;
        if tx.send(named("ping", &json!({}))).await.is_err() {
            return;
        }
        // Who is here already, then this stream joins. It leaves when it ends, however it ends.
        let present = hub.present(&slug);
        let count = present.len();
        if tx.send(named("presence", &json!({ "present": present, "count": count, "stream": stream }))).await.is_err() {
            return;
        }
        tell_presence(&state, presence_of(&slug, &stream, &user, true)).await;
        let _leave = Leave { state: state.clone(), notice: presence_of(&slug, &stream, &user, false) };
        let mut ping = tokio::time::interval(Duration::from_secs(15));
        ping.tick().await;
        let mut reauth = tokio::time::interval(reauth_every());
        reauth.tick().await;
        loop {
            let next = tokio::select! {
                got = changes.recv() => match got {
                    Ok(Event::Change(c)) => named("change", &c),
                    Ok(Event::Presence(p)) => named("presence", &p),
                    Ok(Event::Resync) | Err(broadcast::error::RecvError::Lagged(_)) => named("resync", &json!({})),
                    Err(broadcast::error::RecvError::Closed) => return,
                },
                _ = ping.tick() => named("ping", &json!({})),
                _ = reauth.tick() => match may_read(&state, &user, &slug, true).await {
                    Ok(()) => continue,
                    Err(ServiceError::Internal(_)) | Err(ServiceError::StorageUnavailable(_)) => continue,
                    Err(_) => {
                        let _ = tx.send(named("closed", &json!({ "reason": "You may no longer read this app" }))).await;
                        return;
                    }
                },
                _ = tx.closed() => return,
            };
            // A reader that does not keep up is told to resync instead of holding the writer back.
            match tx.try_send(next) {
                Ok(()) => {}
                Err(mpsc::error::TrySendError::Full(_)) => {
                    while changes.try_recv().is_ok() {}
                    if tx.send(named("resync", &json!({}))).await.is_err() {
                        return;
                    }
                }
                Err(mpsc::error::TrySendError::Closed(_)) => return,
            }
        }
    });
    Ok(Sse::new(Body(rx)))
}
