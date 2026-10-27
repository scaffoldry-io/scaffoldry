//! Decision authority phase 2: the admin routes. `service::authority_changes` does the work.

use axum::{
    body::Bytes,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    routing::{get, post},
    Json, Router,
};
use serde::Deserialize;
use serde_json::Value;

use crate::guard::session_user;
use crate::service::authority_changes as changes;
use crate::service::authority_load;
use crate::service::ServiceError;
use crate::state::{AuthUser, SharedState};

pub fn router() -> Router<SharedState> {
    Router::new()
        .route("/admin/authority", get(show))
        .route("/admin/authority/impact", post(impact))
        .route("/admin/authority/load", get(load))
        .route("/admin/authority/changes", get(list).post(propose))
        .route("/admin/authority/changes/{id}/decide", post(decide))
}

#[derive(Deserialize)]
struct UnitQuery {
    organization_id: Option<String>,
}

fn caller(state: &SharedState, headers: &HeaderMap) -> Result<AuthUser, ServiceError> {
    session_user(state, headers).ok_or_else(|| ServiceError::unauthorized("Unauthorized"))
}

/// A body that is not a JSON object reads as null, so an unauthorized caller is refused before the fields are named.
fn parse(bytes: &Bytes) -> Value {
    serde_json::from_slice(bytes).ok().filter(Value::is_object).unwrap_or(Value::Null)
}

async fn blocking<T: Send + 'static>(f: impl FnOnce() -> Result<T, ServiceError> + Send + 'static) -> Result<T, ServiceError> {
    tokio::task::spawn_blocking(f).await.map_err(|_| ServiceError::Internal("The request stopped before it finished".to_string()))?
}

async fn show(State(state): State<SharedState>, headers: HeaderMap, Query(q): Query<UnitQuery>) -> Result<Json<Value>, ServiceError> {
    let c = caller(&state, &headers)?;
    blocking(move || changes::show(&state, &c, q.organization_id.as_deref())).await.map(Json)
}

async fn load(State(state): State<SharedState>, headers: HeaderMap) -> Result<Json<Value>, ServiceError> {
    let c = caller(&state, &headers)?;
    blocking(move || authority_load::load(&state, &c)).await.map(Json)
}

async fn list(State(state): State<SharedState>, headers: HeaderMap, Query(q): Query<UnitQuery>) -> Result<Json<Value>, ServiceError> {
    let c = caller(&state, &headers)?;
    blocking(move || changes::list(&state, &c, q.organization_id.as_deref())).await.map(Json)
}

async fn propose(State(state): State<SharedState>, headers: HeaderMap, body: Bytes) -> Result<(StatusCode, Json<Value>), ServiceError> {
    let c = caller(&state, &headers)?;
    let body = parse(&body);
    blocking(move || changes::propose(&state, &c, &body)).await.map(|v| (StatusCode::CREATED, Json(v)))
}

async fn impact(State(state): State<SharedState>, headers: HeaderMap, body: Bytes) -> Result<Json<Value>, ServiceError> {
    let c = caller(&state, &headers)?;
    let body = parse(&body);
    blocking(move || changes::impact(&state, &c, &body)).await.map(Json)
}

async fn decide(State(state): State<SharedState>, headers: HeaderMap, Path(id): Path<String>, body: Bytes) -> Result<Json<Value>, ServiceError> {
    let c = caller(&state, &headers)?;
    let body = parse(&body);
    blocking(move || changes::decide(&state, &c, &id, &body)).await.map(Json)
}
