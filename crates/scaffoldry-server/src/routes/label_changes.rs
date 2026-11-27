//! Data layer phase 6: label changes. A steward reads the evaluation and decides. The service checks who may.
//! Phase 6b adds the plan, the withdrawal and the exceptions of a change that runs on a date.

use crate::service::{label_changes as svc, ServiceError};
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
        .route("/data/datasets/{id}/label-changes", post(propose))
        .route("/data/label-changes", get(list))
        .route("/data/label-changes/{id}", get(read))
        .route("/data/label-changes/{id}/decide", post(decide))
        .route("/data/label-changes/{id}/apply", post(apply))
        .route("/data/label-changes/{id}/plan/approve", post(approve_plan))
        .route("/data/label-changes/{id}/withdraw", post(withdraw))
        .route("/data/label-changes/{id}/exceptions", post(request_exception).get(list_exceptions))
        .route("/data/label-changes/{id}/exceptions/{exception}/decide", post(decide_exception))
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

async fn propose(State(state): State<SharedState>, user: Option<Extension<AuthUser>>, headers: HeaderMap, Path(id): Path<String>, body: Option<Json<Value>>) -> Result<impl IntoResponse, ServiceError> {
    let caller = caller(user, &headers, &state)?;
    let body = body_of(body);
    let made = tokio::task::spawn_blocking(move || svc::propose(&caller, &id, &body, &state)).await.map_err(stopped)??;
    Ok((StatusCode::CREATED, Json(made)))
}

async fn list(State(state): State<SharedState>, user: Option<Extension<AuthUser>>, headers: HeaderMap, Query(q): Query<HashMap<String, String>>) -> Result<impl IntoResponse, ServiceError> {
    let caller = caller(user, &headers, &state)?;
    Ok(Json(tokio::task::spawn_blocking(move || svc::list(&caller, &q, &state)).await.map_err(stopped)??))
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

async fn apply(State(state): State<SharedState>, user: Option<Extension<AuthUser>>, headers: HeaderMap, Path(id): Path<String>) -> Result<impl IntoResponse, ServiceError> {
    let caller = caller(user, &headers, &state)?;
    Ok(Json(tokio::task::spawn_blocking(move || svc::apply(&caller, &id, &state)).await.map_err(stopped)??))
}

async fn approve_plan(State(state): State<SharedState>, user: Option<Extension<AuthUser>>, headers: HeaderMap, Path(id): Path<String>, body: Option<Json<Value>>) -> Result<impl IntoResponse, ServiceError> {
    let caller = caller(user, &headers, &state)?;
    let body = body_of(body);
    Ok(Json(tokio::task::spawn_blocking(move || svc::schedule::approve_plan(&caller, &id, &body, &state)).await.map_err(stopped)??))
}

async fn withdraw(State(state): State<SharedState>, user: Option<Extension<AuthUser>>, headers: HeaderMap, Path(id): Path<String>, body: Option<Json<Value>>) -> Result<impl IntoResponse, ServiceError> {
    let caller = caller(user, &headers, &state)?;
    let body = body_of(body);
    Ok(Json(tokio::task::spawn_blocking(move || svc::schedule::withdraw(&caller, &id, &body, &state)).await.map_err(stopped)??))
}

async fn request_exception(State(state): State<SharedState>, user: Option<Extension<AuthUser>>, headers: HeaderMap, Path(id): Path<String>, body: Option<Json<Value>>) -> Result<impl IntoResponse, ServiceError> {
    let caller = caller(user, &headers, &state)?;
    let body = body_of(body);
    let made = tokio::task::spawn_blocking(move || svc::schedule::request_exception(&caller, &id, &body, &state)).await.map_err(stopped)??;
    Ok((StatusCode::CREATED, Json(made)))
}

async fn list_exceptions(State(state): State<SharedState>, user: Option<Extension<AuthUser>>, headers: HeaderMap, Path(id): Path<String>) -> Result<impl IntoResponse, ServiceError> {
    let caller = caller(user, &headers, &state)?;
    Ok(Json(tokio::task::spawn_blocking(move || svc::schedule::list_exceptions(&caller, &id, &state)).await.map_err(stopped)??))
}

async fn decide_exception(State(state): State<SharedState>, user: Option<Extension<AuthUser>>, headers: HeaderMap, Path((id, exception)): Path<(String, String)>, body: Option<Json<Value>>) -> Result<impl IntoResponse, ServiceError> {
    let caller = caller(user, &headers, &state)?;
    let body = body_of(body);
    Ok(Json(tokio::task::spawn_blocking(move || svc::schedule::decide_exception(&caller, &id, &exception, &body, &state)).await.map_err(stopped)??))
}
