//! Approved app templates. The rules are in `service::templates`.

use crate::guard::session_user;
use crate::service::templates;
use crate::service::ServiceError;
use crate::state::SharedState;
use axum::{
    body::Bytes,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    routing::{get, post},
    Json, Router,
};
use serde_json::{json, Value};

pub fn router() -> Router<SharedState> {
    Router::new()
        .route("/templates", get(list).post(publish))
        .route("/templates/{id}/decision", post(decide))
        .route("/templates/{id}/retire", post(retire))
        .route("/workspaces/{ws}/templates/{id}/instantiate", post(instantiate))
}

type Reply = Result<(StatusCode, Json<Value>), (StatusCode, Json<Value>)>;

async fn blocking<T: Send + 'static>(f: impl FnOnce() -> Result<T, ServiceError> + Send + 'static) -> Result<T, (StatusCode, Json<Value>)> {
    tokio::task::spawn_blocking(f)
        .await
        .map_err(|_| ServiceError::internal("The request stopped").into_pair())?
        .map_err(ServiceError::into_pair)
}

fn user_of(state: &SharedState, headers: &HeaderMap) -> Result<crate::state::AuthUser, (StatusCode, Json<Value>)> {
    session_user(state, headers).ok_or_else(|| ServiceError::unauthorized("Unauthorized").into_pair())
}

fn json_body(body: &Bytes) -> Result<Value, (StatusCode, Json<Value>)> {
    if body.is_empty() {
        return Ok(json!({}));
    }
    serde_json::from_slice(body).map_err(|e| ServiceError::bad_request(format!("Invalid request body: {e}")).into_pair())
}

async fn list(State(state): State<SharedState>, headers: HeaderMap) -> Reply {
    let user = user_of(&state, &headers)?;
    blocking(move || templates::list(&user, &state)).await.map(|v| (StatusCode::OK, Json(v)))
}

async fn publish(State(state): State<SharedState>, headers: HeaderMap, body: Bytes) -> Reply {
    let user = user_of(&state, &headers)?;
    let body = json_body(&body)?;
    blocking(move || templates::publish(&user, &body, &state)).await.map(|v| (StatusCode::CREATED, Json(v)))
}

async fn decide(State(state): State<SharedState>, headers: HeaderMap, Path(id): Path<String>, body: Bytes) -> Reply {
    let user = user_of(&state, &headers)?;
    let body = json_body(&body)?;
    blocking(move || templates::decide(&user, &id, &body, &state)).await.map(|v| (StatusCode::OK, Json(v)))
}

async fn retire(State(state): State<SharedState>, headers: HeaderMap, Path(id): Path<String>, body: Bytes) -> Reply {
    let user = user_of(&state, &headers)?;
    let body = json_body(&body)?;
    blocking(move || templates::retire(&user, &id, &body, &state)).await.map(|v| (StatusCode::OK, Json(v)))
}

async fn instantiate(State(state): State<SharedState>, headers: HeaderMap, Path((ws, id)): Path<(String, String)>, body: Bytes) -> Reply {
    let user = user_of(&state, &headers)?;
    let body = json_body(&body)?;
    blocking(move || templates::instantiate(&user, &ws, &id, &body, &state)).await.map(|v| (StatusCode::CREATED, Json(v)))
}
