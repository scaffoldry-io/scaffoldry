//! Notifications phase 1 routes: the signed-in person's inbox. A person reads and marks only their own notices.

use crate::guard::session_user;
use crate::service::ServiceError;
use crate::state::SharedState;
use axum::body::Bytes;
use axum::extract::{Query, State};
use axum::http::HeaderMap;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde_json::{json, Value};
use std::collections::HashMap;

pub fn router() -> Router<SharedState> {
    Router::new()
        .route("/me/notifications", get(list))
        .route("/me/notifications/count", get(count))
        .route("/me/notifications/read", post(read))
}

fn me(state: &SharedState, headers: &HeaderMap) -> Result<String, ServiceError> {
    session_user(state, headers).map(|u| u.eppn).ok_or_else(|| ServiceError::unauthorized("Unauthorized"))
}

async fn blocking<T: Send + 'static>(f: impl FnOnce() -> Result<T, ServiceError> + Send + 'static) -> Result<T, ServiceError> {
    tokio::task::spawn_blocking(f).await.map_err(|_| ServiceError::internal("The request stopped"))?
}

fn storage(e: crate::repository::RepositoryError) -> ServiceError {
    ServiceError::StorageUnavailable(e.to_string())
}

/// The caller's notices, newest first, 50 at a time. `unread=true` lists only unread ones. `cursor` is the last id seen.
async fn list(State(state): State<SharedState>, headers: HeaderMap, Query(q): Query<HashMap<String, String>>) -> Result<Json<Value>, ServiceError> {
    let who = me(&state, &headers)?;
    let unread = q.get("unread").is_some_and(|v| v == "true");
    let limit: i64 = q.get("limit").and_then(|v| v.parse().ok()).unwrap_or(50).clamp(1, 200);
    let cursor: Option<uuid::Uuid> = q.get("cursor").and_then(|v| uuid::Uuid::parse_str(v).ok());
    blocking(move || {
        state
            .repository
            .with_client(move |c| {
                let rows = c.query(
                    "SELECT id, kind, app_slug, title, body, link, to_char(created_at AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS\"Z\"'), read_at IS NOT NULL \
                     FROM notifications n WHERE lower(recipient) = lower($1) AND (NOT $2 OR read_at IS NULL) \
                       AND ($3::uuid IS NULL OR (created_at, id) < (SELECT created_at, id FROM notifications WHERE id = $3)) \
                     ORDER BY created_at DESC, id DESC LIMIT $4",
                    &[&who, &unread, &cursor, &(limit + 1)],
                )?;
                let more = rows.len() as i64 > limit;
                let list: Vec<Value> = rows
                    .iter()
                    .take(limit as usize)
                    .map(|r| {
                        json!({ "id": r.get::<_, uuid::Uuid>(0), "kind": r.get::<_, String>(1), "app_slug": r.get::<_, Option<String>>(2), "title": r.get::<_, String>(3),
                                "body": r.get::<_, String>(4), "link": r.get::<_, Option<Value>>(5), "created_at": r.get::<_, String>(6), "read": r.get::<_, bool>(7) })
                    })
                    .collect();
                let next = if more { list.last().map(|n| n["id"].clone()) } else { None };
                Ok(json!({ "notifications": list, "next_cursor": next }))
            })
            .map_err(storage)
    })
    .await
    .map(Json)
}

async fn count(State(state): State<SharedState>, headers: HeaderMap) -> Result<Json<Value>, ServiceError> {
    let who = me(&state, &headers)?;
    blocking(move || {
        state
            .repository
            .with_client(move |c| Ok(c.query_one("SELECT count(*) FROM notifications WHERE lower(recipient) = lower($1) AND read_at IS NULL", &[&who])?.get::<_, i64>(0)))
            .map(|n| json!({ "unread": n }))
            .map_err(storage)
    })
    .await
    .map(Json)
}

/// Marks the caller's notices read: the listed `ids`, or `all: true`. Another person's id is ignored.
async fn read(State(state): State<SharedState>, headers: HeaderMap, body: Bytes) -> Result<Json<Value>, ServiceError> {
    let who = me(&state, &headers)?;
    let body: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
    let all = body.get("all").and_then(Value::as_bool).unwrap_or(false);
    let ids: Vec<uuid::Uuid> = body.get("ids").and_then(Value::as_array).map(|l| l.iter().filter_map(|v| v.as_str().and_then(|s| uuid::Uuid::parse_str(s).ok())).collect()).unwrap_or_default();
    if !all && ids.is_empty() {
        return Err(ServiceError::bad_request("Name the notices to mark read, or send all: true"));
    }
    blocking(move || {
        state
            .repository
            .with_client(move |c| {
                Ok(c.execute(
                    "UPDATE notifications SET read_at = NOW() WHERE lower(recipient) = lower($1) AND read_at IS NULL AND ($2 OR id = ANY($3))",
                    &[&who, &all, &ids],
                )?)
            })
            .map(|n| json!({ "marked": n }))
            .map_err(storage)
    })
    .await
    .map(Json)
}
