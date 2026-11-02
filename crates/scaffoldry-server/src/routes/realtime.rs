//! Real-time phase 1: `GET /apps/{slug}/events`, the changes to one app as server-sent events.
//!
//! Events: `ping` when the stream opens and every 15 seconds, `change` for each committed write, `resync` when changes
//! may have been missed, and `closed` when the reader may no longer read the app. A change names the table, the record,
//! its version and the kind of write. It carries no value, so a reader fetches the record through the usual checks.

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
    Router::new().route("/apps/{slug}/events", get(events))
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
    let (tx, rx) = mpsc::channel::<SseEvent>(16);
    tokio::spawn(async move {
        let _permit = permit;
        if tx.send(named("ping", &json!({}))).await.is_err() {
            return;
        }
        let mut ping = tokio::time::interval(Duration::from_secs(15));
        ping.tick().await;
        let mut reauth = tokio::time::interval(reauth_every());
        reauth.tick().await;
        loop {
            let next = tokio::select! {
                got = changes.recv() => match got {
                    Ok(Event::Change(c)) => named("change", &c),
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
