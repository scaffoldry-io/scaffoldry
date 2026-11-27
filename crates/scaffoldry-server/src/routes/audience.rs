//! Data layer phase 3b: audience fields and conditions. The steward of a receiving unit accepts its place in an audience. A Platform Admin
//! or a compliance officer sets a condition on a person's record. The service checks who may.

use crate::service::{audience as svc, ServiceError};
use crate::state::{AuthUser, SharedState};
use axum::{
    extract::{Path, State},
    http::HeaderMap,
    response::IntoResponse,
    routing::{get, post, put},
    Extension, Json, Router,
};
use serde_json::Value;

pub fn router() -> Router<SharedState> {
    Router::new()
        .route("/data/datasets/{id}/audience/pending", get(pending))
        .route("/data/datasets/{id}/records/{record_id}/audience/accept", post(accept))
        .route("/admin/people/{eppn}/conditions", get(conditions))
        .route("/admin/people/{eppn}/conditions/{id}", put(set_condition))
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

async fn pending(State(state): State<SharedState>, user: Option<Extension<AuthUser>>, headers: HeaderMap, Path(id): Path<String>) -> Result<impl IntoResponse, ServiceError> {
    let caller = caller(user, &headers, &state)?;
    Ok(Json(tokio::task::spawn_blocking(move || svc::pending(&caller, &id, &state)).await.map_err(stopped)??))
}

async fn accept(State(state): State<SharedState>, user: Option<Extension<AuthUser>>, headers: HeaderMap, Path((id, record_id)): Path<(String, String)>, body: Option<Json<Value>>) -> Result<impl IntoResponse, ServiceError> {
    let caller = caller(user, &headers, &state)?;
    let body = body_of(body);
    Ok(Json(tokio::task::spawn_blocking(move || svc::accept(&caller, &id, &record_id, &body, &state)).await.map_err(stopped)??))
}

async fn conditions(State(state): State<SharedState>, user: Option<Extension<AuthUser>>, headers: HeaderMap, Path(eppn): Path<String>) -> Result<impl IntoResponse, ServiceError> {
    let caller = caller(user, &headers, &state)?;
    Ok(Json(tokio::task::spawn_blocking(move || svc::conditions(&caller, &eppn, &state)).await.map_err(stopped)??))
}

async fn set_condition(State(state): State<SharedState>, user: Option<Extension<AuthUser>>, headers: HeaderMap, Path((eppn, id)): Path<(String, String)>, body: Option<Json<Value>>) -> Result<impl IntoResponse, ServiceError> {
    let caller = caller(user, &headers, &state)?;
    let body = body_of(body);
    Ok(Json(tokio::task::spawn_blocking(move || svc::set_condition(&caller, &eppn, &id, &body, &state)).await.map_err(stopped)??))
}
