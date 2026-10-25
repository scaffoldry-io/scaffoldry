//! Policy library phase 1: the admin routes. The service functions in `service::library` do the work.

use axum::{
    body::Bytes,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    routing::{get, post},
    Json, Router,
};
use serde_json::Value;

use crate::guard::session_user;
use crate::service::library;
use crate::service::ServiceError;
use crate::state::{AuthUser, SharedState};

pub fn router() -> Router<SharedState> {
    Router::new()
        .route(
            "/admin/library/policies",
            get(list_policies).post(create_policy),
        )
        .route(
            "/admin/library/policies/{id}",
            get(get_policy).patch(patch_policy),
        )
        .route("/admin/library/policies/{id}/sources", post(add_source))
        .route("/admin/library/sources/{id}/snapshots", post(add_snapshot))
        .route("/admin/library/snapshots/{id}", get(get_snapshot))
        .route("/admin/library/policies/{id}/rules", post(draft_rule))
}

fn caller(state: &SharedState, headers: &HeaderMap) -> Result<AuthUser, ServiceError> {
    session_user(state, headers).ok_or_else(|| ServiceError::unauthorized("Unauthorized"))
}

/// A body that is not a JSON object reads as null, so the service refuses an unauthorized caller first and then names the missing fields.
fn parse(bytes: &Bytes) -> Value {
    serde_json::from_slice(bytes)
        .ok()
        .filter(Value::is_object)
        .unwrap_or(Value::Null)
}

async fn blocking<T: Send + 'static>(
    f: impl FnOnce() -> Result<T, ServiceError> + Send + 'static,
) -> Result<T, ServiceError> {
    tokio::task::spawn_blocking(f)
        .await
        .map_err(|_| ServiceError::Internal("The request stopped before it finished".to_string()))?
}

async fn list_policies(
    State(state): State<SharedState>,
    headers: HeaderMap,
) -> Result<Json<Value>, ServiceError> {
    let c = caller(&state, &headers)?;
    blocking(move || library::list_policies(&state, &c))
        .await
        .map(Json)
}

async fn create_policy(
    State(state): State<SharedState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<(StatusCode, Json<Value>), ServiceError> {
    let c = caller(&state, &headers)?;
    let body = parse(&body);
    blocking(move || library::create_policy(&state, &c, &body))
        .await
        .map(|v| (StatusCode::CREATED, Json(v)))
}

async fn get_policy(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<Value>, ServiceError> {
    let c = caller(&state, &headers)?;
    blocking(move || library::get_policy(&state, &c, &id))
        .await
        .map(Json)
}

async fn patch_policy(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    body: Bytes,
) -> Result<Json<Value>, ServiceError> {
    let c = caller(&state, &headers)?;
    let body = parse(&body);
    blocking(move || library::patch_policy(&state, &c, &id, &body))
        .await
        .map(Json)
}

async fn add_source(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    body: Bytes,
) -> Result<(StatusCode, Json<Value>), ServiceError> {
    let c = caller(&state, &headers)?;
    let body = parse(&body);
    blocking(move || library::add_source(&state, &c, &id, &body))
        .await
        .map(|v| (StatusCode::CREATED, Json(v)))
}

async fn add_snapshot(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    body: Bytes,
) -> Result<(StatusCode, Json<Value>), ServiceError> {
    let c = caller(&state, &headers)?;
    let body = parse(&body);
    let (stored, snapshot) =
        blocking(move || library::add_snapshot(&state, &c, &id, &body)).await?;
    Ok((
        if stored {
            StatusCode::CREATED
        } else {
            StatusCode::OK
        },
        Json(serde_json::json!({ "stored": stored, "snapshot": snapshot })),
    ))
}

async fn get_snapshot(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<Value>, ServiceError> {
    let c = caller(&state, &headers)?;
    blocking(move || library::get_snapshot(&state, &c, &id))
        .await
        .map(Json)
}

async fn draft_rule(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    body: Bytes,
) -> Result<(StatusCode, Json<Value>), ServiceError> {
    let c = caller(&state, &headers)?;
    let body = parse(&body);
    blocking(move || library::draft_rule(&state, &c, &id, &body))
        .await
        .map(|v| (StatusCode::CREATED, Json(v)))
}
