//! Data layer phase 9: system users for automations. The service checks who may.

use crate::service::{system_users as svc, ServiceError};
use crate::state::{AuthUser, SharedState};
use axum::{
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    routing::{get, post},
    Extension, Json, Router,
};
use serde_json::Value;

pub fn router() -> Router<SharedState> {
    Router::new()
        .route("/data/system-users", post(register))
        .route("/data/system-users/{flow_id}", get(read).patch(change))
        .route("/data/system-users/{flow_id}/grants", post(approve))
        .route("/data/system-users/{flow_id}/grants/{grant_id}/revoke", post(revoke))
}

fn caller(user: Option<Extension<AuthUser>>, headers: &HeaderMap, state: &SharedState) -> Result<AuthUser, ServiceError> {
    if let Some(Extension(u)) = user {
        return Ok(u);
    }
    crate::guard::session_user(state, headers).ok_or_else(|| ServiceError::Unauthorized("A valid session is required".to_string()))
}

fn stopped(_: tokio::task::JoinError) -> ServiceError {
    ServiceError::Internal("The request stopped".to_string())
}

fn body_of(body: Option<Json<Value>>) -> Value {
    body.map(|Json(b)| b).unwrap_or(Value::Null)
}

async fn register(State(state): State<SharedState>, user: Option<Extension<AuthUser>>, headers: HeaderMap, body: Option<Json<Value>>) -> Result<impl IntoResponse, ServiceError> {
    let caller = caller(user, &headers, &state)?;
    let body = body_of(body);
    let made = tokio::task::spawn_blocking(move || svc::register(&caller, &body, &state)).await.map_err(stopped)??;
    Ok((StatusCode::CREATED, Json(made)))
}

async fn read(State(state): State<SharedState>, user: Option<Extension<AuthUser>>, headers: HeaderMap, Path(flow): Path<String>) -> Result<impl IntoResponse, ServiceError> {
    let caller = caller(user, &headers, &state)?;
    Ok(Json(tokio::task::spawn_blocking(move || svc::get(&caller, &flow, &state)).await.map_err(stopped)??))
}

async fn change(State(state): State<SharedState>, user: Option<Extension<AuthUser>>, headers: HeaderMap, Path(flow): Path<String>, body: Option<Json<Value>>) -> Result<impl IntoResponse, ServiceError> {
    let caller = caller(user, &headers, &state)?;
    let body = body_of(body);
    Ok(Json(tokio::task::spawn_blocking(move || svc::change(&caller, &flow, &body, &state)).await.map_err(stopped)??))
}

async fn approve(State(state): State<SharedState>, user: Option<Extension<AuthUser>>, headers: HeaderMap, Path(flow): Path<String>, body: Option<Json<Value>>) -> Result<impl IntoResponse, ServiceError> {
    let caller = caller(user, &headers, &state)?;
    let body = body_of(body);
    let made = tokio::task::spawn_blocking(move || svc::approve(&caller, &flow, &body, &state)).await.map_err(stopped)??;
    Ok((StatusCode::CREATED, Json(made)))
}

async fn revoke(State(state): State<SharedState>, user: Option<Extension<AuthUser>>, headers: HeaderMap, Path((flow, grant)): Path<(String, String)>, body: Option<Json<Value>>) -> Result<impl IntoResponse, ServiceError> {
    let caller = caller(user, &headers, &state)?;
    let body = body_of(body);
    Ok(Json(tokio::task::spawn_blocking(move || svc::revoke(&caller, &flow, &grant, &body, &state)).await.map_err(stopped)??))
}
