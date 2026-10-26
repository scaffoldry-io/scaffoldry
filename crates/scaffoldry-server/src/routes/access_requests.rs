//! Access model phase 13: asking for access. The same service functions the MCP tool calls.

use crate::guard::{read_origin, session_user};
use crate::service::access_requests as requests;
use crate::service::ServiceError;
use crate::state::{AuthUser, SharedState};
use axum::{
    body::Bytes,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    routing::{get, post},
    Json, Router,
};
use serde_json::Value;
use uuid::Uuid;

/// The routes of a person.
pub fn router() -> Router<SharedState> {
    Router::new()
        .route("/access/requests", post(file))
        .route("/me/access/requests", get(mine))
        .route("/me/access/requests/to-decide", get(to_decide))
        .route("/access/requests/{id}/confirm", post(confirm))
        .route("/access/requests/{id}/decide", post(decide))
        .route("/access/requests/{id}/withdraw", post(withdraw))
}

/// The routes of an administrator, mounted at the API root as well.
pub fn admin_router() -> Router<SharedState> {
    Router::new().route("/admin/access/request-rules", get(list_rules).post(propose_rule))
}

fn caller(state: &SharedState, headers: &HeaderMap) -> Result<AuthUser, ServiceError> {
    session_user(state, headers).ok_or_else(|| ServiceError::unauthorized("Unauthorized"))
}

fn parse(body: &Bytes) -> Result<Value, ServiceError> {
    if body.is_empty() {
        return Ok(Value::Null);
    }
    serde_json::from_slice(body).map_err(|e| ServiceError::BadRequest(format!("The body is not valid JSON: {e}")))
}

fn id_of(id: &str) -> Result<Uuid, ServiceError> {
    Uuid::parse_str(id).map_err(|_| ServiceError::not_found("Request not found"))
}

async fn blocking<T: Send + 'static>(f: impl FnOnce() -> Result<T, ServiceError> + Send + 'static) -> Result<T, ServiceError> {
    tokio::task::spawn_blocking(f).await.map_err(|_| ServiceError::Internal("The request stopped before it finished".to_string()))?
}

async fn file(State(state): State<SharedState>, headers: HeaderMap, body: Bytes) -> Result<(StatusCode, Json<Value>), ServiceError> {
    let user = caller(&state, &headers)?;
    let origin = read_origin(&state, &headers);
    let body = parse(&body)?;
    blocking(move || requests::file(&state, &user, &origin, &body)).await.map(|v| (StatusCode::CREATED, Json(v)))
}

async fn mine(State(state): State<SharedState>, headers: HeaderMap) -> Result<Json<Value>, ServiceError> {
    let user = caller(&state, &headers)?;
    blocking(move || requests::mine(&state, &user)).await.map(Json)
}

async fn to_decide(State(state): State<SharedState>, headers: HeaderMap) -> Result<Json<Value>, ServiceError> {
    let user = caller(&state, &headers)?;
    blocking(move || requests::to_decide(&state, &user)).await.map(Json)
}

async fn confirm(State(state): State<SharedState>, headers: HeaderMap, Path(id): Path<String>) -> Result<Json<Value>, ServiceError> {
    let user = caller(&state, &headers)?;
    let origin = read_origin(&state, &headers);
    blocking(move || requests::confirm(&state, &user, &origin, id_of(&id)?)).await.map(Json)
}

async fn decide(State(state): State<SharedState>, headers: HeaderMap, Path(id): Path<String>, body: Bytes) -> Result<Json<Value>, ServiceError> {
    let user = caller(&state, &headers)?;
    let body = parse(&body)?;
    blocking(move || requests::decide(&state, &user, id_of(&id)?, &body)).await.map(Json)
}

async fn withdraw(State(state): State<SharedState>, headers: HeaderMap, Path(id): Path<String>) -> Result<Json<Value>, ServiceError> {
    let user = caller(&state, &headers)?;
    blocking(move || requests::withdraw(&state, &user, id_of(&id)?)).await.map(Json)
}

async fn list_rules(State(state): State<SharedState>, headers: HeaderMap) -> Result<Json<Value>, ServiceError> {
    let user = caller(&state, &headers)?;
    blocking(move || requests::list_rules(&state, &user)).await.map(Json)
}

async fn propose_rule(State(state): State<SharedState>, headers: HeaderMap, body: Bytes) -> Result<(StatusCode, Json<Value>), ServiceError> {
    let user = caller(&state, &headers)?;
    let body = parse(&body)?;
    blocking(move || requests::propose_rule(&state, &user, &body)).await.map(|v| (StatusCode::ACCEPTED, Json(v)))
}
