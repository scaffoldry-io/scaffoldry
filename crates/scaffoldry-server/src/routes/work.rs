//! Workflows phase 5 routes: My Work. Each reads or acts on the caller's own items only. The rules are in
//! `service::work`.

use crate::repository_work::WorkFilter;
use crate::service::{work, ServiceError};
use crate::state::{AuthUser, SharedState};
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::routing::{get, post};
use axum::{Extension, Json, Router};
use serde_json::Value;
use std::collections::HashMap;

pub fn router() -> Router<SharedState> {
    Router::new()
        .route("/me/work", get(list))
        .route("/me/work/summary", get(summary))
        .route("/me/work/{id}", get(item))
        .route("/me/work/{id}/act", post(act))
}

fn caller(user: Option<Extension<AuthUser>>, headers: &HeaderMap, state: &SharedState) -> Result<AuthUser, ServiceError> {
    if let Some(Extension(u)) = user {
        return Ok(u);
    }
    crate::guard::session_user(state, headers).ok_or_else(|| ServiceError::Unauthorized("A valid session is required".to_string()))
}

async fn blocking<T: Send + 'static>(f: impl FnOnce() -> Result<T, ServiceError> + Send + 'static) -> Result<T, ServiceError> {
    tokio::task::spawn_blocking(f).await.map_err(|_| ServiceError::internal("The request stopped"))?
}

/// The list's query: `kind`, `app`, `workspace`, `overdue`, `acting_for`, `cursor`, `limit`.
pub fn filter_of(q: &HashMap<String, String>) -> WorkFilter {
    WorkFilter {
        kind: q.get("kind").cloned(),
        app: q.get("app").cloned(),
        workspace: q.get("workspace").cloned(),
        overdue: q.get("overdue").is_some_and(|v| v == "true" || v == "1"),
        acting_for: q.get("acting_for").cloned(),
    }
}

async fn list(State(state): State<SharedState>, user: Option<Extension<AuthUser>>, headers: HeaderMap, Query(q): Query<HashMap<String, String>>) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let me = caller(user, &headers, &state).map_err(ServiceError::into_pair)?;
    if q.get("state").is_some_and(|s| s != "open") {
        return Err(ServiceError::bad_request("My Work lists open items only").into_pair());
    }
    let limit = q.get("limit").and_then(|l| l.parse().ok()).unwrap_or(50);
    blocking(move || work::list(&me, &filter_of(&q), q.get("cursor").map(String::as_str), limit, &state)).await.map(Json).map_err(ServiceError::into_pair)
}

async fn summary(State(state): State<SharedState>, user: Option<Extension<AuthUser>>, headers: HeaderMap) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let me = caller(user, &headers, &state).map_err(ServiceError::into_pair)?;
    blocking(move || work::summary(&me, &state)).await.map(Json).map_err(ServiceError::into_pair)
}

async fn item(State(state): State<SharedState>, user: Option<Extension<AuthUser>>, headers: HeaderMap, Path(id): Path<String>) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let me = caller(user, &headers, &state).map_err(ServiceError::into_pair)?;
    blocking(move || work::get(&me, &id, &state)).await.map(Json).map_err(ServiceError::into_pair)
}

async fn act(State(state): State<SharedState>, user: Option<Extension<AuthUser>>, headers: HeaderMap, Path(id): Path<String>, Json(body): Json<Value>) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let me = caller(user, &headers, &state).map_err(ServiceError::into_pair)?;
    let origin = crate::guard::origin(&state, &headers).map_err(ServiceError::into_pair)?;
    blocking(move || work::act_on_work(&me, &origin, &id, &body, &state)).await.map(Json).map_err(ServiceError::into_pair)
}
