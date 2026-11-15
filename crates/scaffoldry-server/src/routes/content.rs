//! Sensitive content phase 2: the review of what the scans found. Each route is for a Platform Admin or a compliance officer, and the
//! service checks that first.

use crate::service::{content_governance as gov, ServiceError};
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
        .route("/admin/content-scans", post(scan))
        .route("/admin/content-findings", get(findings))
        .route("/admin/content-findings/{id}/decide", post(decide))
        .route("/admin/authorized-stores", get(stores).put(put_store).delete(delete_store))
        .route("/admin/detector-stats", get(stats))
        .route("/admin/sensitivity-settings", get(sensitivity_settings))
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

async fn scan(State(state): State<SharedState>, user: Option<Extension<AuthUser>>, headers: HeaderMap, body: Option<Json<Value>>) -> Result<impl IntoResponse, ServiceError> {
    let caller = caller(user, &headers, &state)?;
    let body = body_of(body);
    let made = tokio::task::spawn_blocking(move || gov::scan(&caller, &body, &state)).await.map_err(stopped)??;
    Ok((StatusCode::ACCEPTED, Json(made)))
}

async fn findings(State(state): State<SharedState>, user: Option<Extension<AuthUser>>, headers: HeaderMap, Query(q): Query<HashMap<String, String>>) -> Result<impl IntoResponse, ServiceError> {
    let caller = caller(user, &headers, &state)?;
    Ok(Json(tokio::task::spawn_blocking(move || gov::findings(&caller, &q, &state)).await.map_err(stopped)??))
}

async fn decide(State(state): State<SharedState>, user: Option<Extension<AuthUser>>, headers: HeaderMap, Path(id): Path<String>, body: Option<Json<Value>>) -> Result<impl IntoResponse, ServiceError> {
    let caller = caller(user, &headers, &state)?;
    let body = body_of(body);
    Ok(Json(tokio::task::spawn_blocking(move || gov::decide(&caller, &id, &body, &state)).await.map_err(stopped)??))
}

async fn stores(State(state): State<SharedState>, user: Option<Extension<AuthUser>>, headers: HeaderMap) -> Result<impl IntoResponse, ServiceError> {
    let caller = caller(user, &headers, &state)?;
    Ok(Json(tokio::task::spawn_blocking(move || gov::stores(&caller, &state)).await.map_err(stopped)??))
}

async fn put_store(State(state): State<SharedState>, user: Option<Extension<AuthUser>>, headers: HeaderMap, body: Option<Json<Value>>) -> Result<impl IntoResponse, ServiceError> {
    let caller = caller(user, &headers, &state)?;
    let body = body_of(body);
    Ok(Json(tokio::task::spawn_blocking(move || gov::put_store(&caller, &body, &state)).await.map_err(stopped)??))
}

async fn delete_store(State(state): State<SharedState>, user: Option<Extension<AuthUser>>, headers: HeaderMap, body: Option<Json<Value>>) -> Result<impl IntoResponse, ServiceError> {
    let caller = caller(user, &headers, &state)?;
    let body = body_of(body);
    Ok(Json(tokio::task::spawn_blocking(move || gov::delete_store(&caller, &body, &state)).await.map_err(stopped)??))
}

async fn stats(State(state): State<SharedState>, user: Option<Extension<AuthUser>>, headers: HeaderMap) -> Result<impl IntoResponse, ServiceError> {
    let caller = caller(user, &headers, &state)?;
    Ok(Json(tokio::task::spawn_blocking(move || gov::stats(&caller, &state)).await.map_err(stopped)??))
}

async fn sensitivity_settings(State(state): State<SharedState>, user: Option<Extension<AuthUser>>, headers: HeaderMap) -> Result<impl IntoResponse, ServiceError> {
    let caller = caller(user, &headers, &state)?;
    Ok(Json(tokio::task::spawn_blocking(move || gov::sensitivity_settings(&caller, &state)).await.map_err(stopped)??))
}
