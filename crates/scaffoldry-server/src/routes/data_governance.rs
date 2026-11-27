//! Admin console phase 4: data governance. Each route is for a Platform Admin or a compliance officer, and the service checks that first.

use crate::service::{catalog, data_governance, ServiceError};
use crate::state::{AuthUser, SharedState};
use axum::{
    extract::{Path, Query, State},
    http::HeaderMap,
    response::IntoResponse,
    routing::{get, put},
    Extension, Json, Router,
};
use serde_json::Value;
use std::collections::HashMap;

pub fn router() -> Router<SharedState> {
    Router::new()
        .route("/admin/fields", get(fields))
        .route("/admin/labels", put(labels))
        .route("/admin/datasets", get(datasets))
        .route("/admin/datasets/{id}/classification", put(classify))
        .route("/admin/datasets/{id}/discoverability", put(discoverability))
        .route("/admin/datasets/{id}/label-floor", put(label_floor))
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

async fn fields(State(state): State<SharedState>, user: Option<Extension<AuthUser>>, headers: HeaderMap, Query(q): Query<HashMap<String, String>>) -> Result<impl IntoResponse, ServiceError> {
    let caller = caller(user, &headers, &state)?;
    Ok(Json(tokio::task::spawn_blocking(move || data_governance::fields(&caller, &q, &state)).await.map_err(stopped)??))
}

async fn labels(State(state): State<SharedState>, user: Option<Extension<AuthUser>>, headers: HeaderMap, body: Option<Json<Value>>) -> Result<impl IntoResponse, ServiceError> {
    let caller = caller(user, &headers, &state)?;
    let body = body.map(|Json(b)| b).unwrap_or(Value::Null);
    Ok(Json(tokio::task::spawn_blocking(move || data_governance::put_labels(&caller, &body, &state)).await.map_err(stopped)??))
}

async fn datasets(State(state): State<SharedState>, user: Option<Extension<AuthUser>>, headers: HeaderMap) -> Result<impl IntoResponse, ServiceError> {
    let caller = caller(user, &headers, &state)?;
    Ok(Json(tokio::task::spawn_blocking(move || data_governance::datasets(&caller, &state)).await.map_err(stopped)??))
}

async fn classify(State(state): State<SharedState>, user: Option<Extension<AuthUser>>, headers: HeaderMap, Path(id): Path<String>, body: Option<Json<Value>>) -> Result<impl IntoResponse, ServiceError> {
    let caller = caller(user, &headers, &state)?;
    let body = body.map(|Json(b)| b).unwrap_or(Value::Null);
    Ok(Json(tokio::task::spawn_blocking(move || data_governance::classify(&caller, &id, &body, &state)).await.map_err(stopped)??))
}

/// Data layer phase 2d: who may discover a dataset, which fields are shown as restricted, and its steward position.
async fn discoverability(State(state): State<SharedState>, user: Option<Extension<AuthUser>>, headers: HeaderMap, Path(id): Path<String>, body: Option<Json<Value>>) -> Result<impl IntoResponse, ServiceError> {
    let caller = caller(user, &headers, &state)?;
    let body = body.map(|Json(b)| b).unwrap_or(Value::Null);
    Ok(Json(tokio::task::spawn_blocking(move || catalog::set_policy(&caller, &id, &body, &state)).await.map_err(stopped)??))
}

/// Data layer phase 3: the categories every row of a dataset carries, whatever its own tags say.
async fn label_floor(State(state): State<SharedState>, user: Option<Extension<AuthUser>>, headers: HeaderMap, Path(id): Path<String>, body: Option<Json<Value>>) -> Result<impl IntoResponse, ServiceError> {
    let caller = caller(user, &headers, &state)?;
    let body = body.map(|Json(b)| b).unwrap_or(Value::Null);
    Ok(Json(tokio::task::spawn_blocking(move || crate::service::row_labels::set_floor(&caller, &id, &body, &state)).await.map_err(stopped)??))
}
