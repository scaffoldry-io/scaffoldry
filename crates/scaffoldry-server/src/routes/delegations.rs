//! Approvers phase 4 routes: delegation. The rules are in `service::delegations`.

use crate::service::{delegations, ServiceError};
use crate::state::{AuthUser, SharedState};
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::routing::{delete, get};
use axum::{Extension, Json, Router};
use serde_json::Value;
use std::collections::HashMap;

pub fn router() -> Router<SharedState> {
    Router::new()
        .route("/me/delegations", get(mine).post(create))
        .route("/me/delegations/people", get(people))
        .route("/me/delegations/{id}", delete(revoke_mine))
        .route("/admin/delegations", get(list_all))
        .route("/admin/delegations/{id}", delete(revoke_admin))
}

fn caller(user: Option<Extension<AuthUser>>, headers: &HeaderMap, state: &SharedState) -> Result<AuthUser, ServiceError> {
    if let Some(Extension(u)) = user {
        return Ok(u);
    }
    crate::guard::session_user(state, headers).ok_or_else(|| ServiceError::Unauthorized("A valid session is required".to_string()))
}

async fn blocking<T: Send + 'static>(f: impl FnOnce() -> Result<T, ServiceError> + Send + 'static) -> Result<T, ServiceError> {
    tokio::task::spawn_blocking(f).await.map_err(|_| ServiceError::internal("The request stopped"))?
}

fn body_of(body: Option<Json<Value>>) -> Value {
    body.map(|Json(b)| b).unwrap_or(Value::Null)
}

async fn mine(State(state): State<SharedState>, user: Option<Extension<AuthUser>>, headers: HeaderMap) -> Result<Json<Value>, ServiceError> {
    let caller = caller(user, &headers, &state)?;
    Ok(Json(blocking(move || delegations::mine(&caller, &state)).await?))
}

async fn people(State(state): State<SharedState>, user: Option<Extension<AuthUser>>, headers: HeaderMap, Query(q): Query<HashMap<String, String>>) -> Result<Json<Value>, ServiceError> {
    caller(user, &headers, &state)?;
    let q = q.get("q").cloned().unwrap_or_default();
    Ok(Json(blocking(move || delegations::people(&q, &state)).await?))
}

async fn create(State(state): State<SharedState>, user: Option<Extension<AuthUser>>, headers: HeaderMap, body: Option<Json<Value>>) -> Result<(StatusCode, Json<Value>), ServiceError> {
    let caller = caller(user, &headers, &state)?;
    let body = body_of(body);
    Ok((StatusCode::CREATED, Json(blocking(move || delegations::create(&caller, &body, &state)).await?)))
}

async fn revoke_mine(State(state): State<SharedState>, user: Option<Extension<AuthUser>>, headers: HeaderMap, Path(id): Path<String>, body: Option<Json<Value>>) -> Result<Json<Value>, ServiceError> {
    let caller = caller(user, &headers, &state)?;
    let body = body_of(body);
    Ok(Json(blocking(move || delegations::revoke_mine(&caller, &id, &body, &state)).await?))
}

async fn list_all(State(state): State<SharedState>, user: Option<Extension<AuthUser>>, headers: HeaderMap, Query(q): Query<HashMap<String, String>>) -> Result<Json<Value>, ServiceError> {
    let caller = caller(user, &headers, &state)?;
    Ok(Json(blocking(move || delegations::list_all(&caller, &q, &state)).await?))
}

async fn revoke_admin(State(state): State<SharedState>, user: Option<Extension<AuthUser>>, headers: HeaderMap, Path(id): Path<String>, body: Option<Json<Value>>) -> Result<Json<Value>, ServiceError> {
    let caller = caller(user, &headers, &state)?;
    let body = body_of(body);
    Ok(Json(blocking(move || delegations::revoke_admin(&caller, &id, &body, &state)).await?))
}
