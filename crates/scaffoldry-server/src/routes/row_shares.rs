//! Data layer phase 10: row shares. A person shares rows with another person for a time. A steward decides. The service checks who may.

use crate::service::{row_shares as svc, ServiceError};
use crate::state::{AuthUser, SharedState};
use axum::{
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    routing::{get, post},
    Extension, Json, Router,
};
use serde_json::Value;
use std::collections::HashMap;

pub fn router() -> Router<SharedState> {
    Router::new()
        .route("/data/datasets/{id}/row-shares", post(request).get(list))
        .route("/data/row-shares/{id}", get(read))
        .route("/data/row-shares/{id}/decide", post(decide))
        .route("/data/row-shares/{id}/withdraw", post(withdraw))
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

/// 201 for a share that was stored, and 200 for one inside what the grantee already reads, which is not.
async fn request(State(state): State<SharedState>, user: Option<Extension<AuthUser>>, headers: HeaderMap, Path(id): Path<String>, body: Option<Json<Value>>) -> Result<impl IntoResponse, ServiceError> {
    let caller = caller(user, &headers, &state)?;
    let body = body_of(body);
    let (stored, made) = tokio::task::spawn_blocking(move || svc::request(&caller, &id, &body, &state)).await.map_err(stopped)??;
    Ok((if stored { StatusCode::CREATED } else { StatusCode::OK }, Json(made)))
}

async fn list(State(state): State<SharedState>, user: Option<Extension<AuthUser>>, headers: HeaderMap, Path(id): Path<String>, Query(q): Query<HashMap<String, String>>) -> Result<impl IntoResponse, ServiceError> {
    let caller = caller(user, &headers, &state)?;
    Ok(Json(tokio::task::spawn_blocking(move || svc::list(&caller, &id, &q, &state)).await.map_err(stopped)??))
}

async fn read(State(state): State<SharedState>, user: Option<Extension<AuthUser>>, headers: HeaderMap, Path(id): Path<String>) -> Result<impl IntoResponse, ServiceError> {
    let caller = caller(user, &headers, &state)?;
    Ok(Json(tokio::task::spawn_blocking(move || svc::get(&caller, &id, &state)).await.map_err(stopped)??))
}

async fn decide(State(state): State<SharedState>, user: Option<Extension<AuthUser>>, headers: HeaderMap, Path(id): Path<String>, body: Option<Json<Value>>) -> Result<impl IntoResponse, ServiceError> {
    let caller = caller(user, &headers, &state)?;
    let body = body_of(body);
    Ok(Json(tokio::task::spawn_blocking(move || svc::decide(&caller, &id, &body, &state)).await.map_err(stopped)??))
}

async fn withdraw(State(state): State<SharedState>, user: Option<Extension<AuthUser>>, headers: HeaderMap, Path(id): Path<String>, body: Option<Json<Value>>) -> Result<impl IntoResponse, ServiceError> {
    let caller = caller(user, &headers, &state)?;
    let body = body_of(body);
    Ok(Json(tokio::task::spawn_blocking(move || svc::withdraw(&caller, &id, &body, &state)).await.map_err(stopped)??))
}
