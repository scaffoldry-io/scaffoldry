//! Connections phase 1 routes: data connections and their runs. A Platform Admin only. The rules are in
//! `service::connections`.

use crate::service::{connections, ServiceError};
use crate::state::{AuthUser, SharedState};
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Extension, Json, Router};
use serde_json::Value;

pub fn router() -> Router<SharedState> {
    Router::new()
        .route("/admin/connections", get(list).post(create))
        .route("/admin/connections/{id}", axum::routing::patch(update))
        .route("/admin/connections/{id}/runs", get(runs))
        .route("/admin/connections/{id}/run", post(run_now))
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

async fn list(State(state): State<SharedState>, user: Option<Extension<AuthUser>>, headers: HeaderMap) -> Result<impl IntoResponse, ServiceError> {
    let caller = caller(user, &headers, &state)?;
    Ok(Json(blocking(move || connections::list(&caller, &state)).await?))
}

async fn create(State(state): State<SharedState>, user: Option<Extension<AuthUser>>, headers: HeaderMap, body: Option<Json<Value>>) -> Result<impl IntoResponse, ServiceError> {
    let caller = caller(user, &headers, &state)?;
    let body = body_of(body);
    Ok((StatusCode::CREATED, Json(blocking(move || connections::create(&caller, &body, &state)).await?)))
}

async fn update(State(state): State<SharedState>, user: Option<Extension<AuthUser>>, headers: HeaderMap, Path(id): Path<String>, body: Option<Json<Value>>) -> Result<impl IntoResponse, ServiceError> {
    let caller = caller(user, &headers, &state)?;
    let body = body_of(body);
    Ok(Json(blocking(move || connections::update(&caller, &id, &body, &state)).await?))
}

async fn runs(State(state): State<SharedState>, user: Option<Extension<AuthUser>>, headers: HeaderMap, Path(id): Path<String>) -> Result<impl IntoResponse, ServiceError> {
    let caller = caller(user, &headers, &state)?;
    Ok(Json(blocking(move || connections::runs(&caller, &id, &state)).await?))
}

async fn run_now(State(state): State<SharedState>, user: Option<Extension<AuthUser>>, headers: HeaderMap, Path(id): Path<String>, body: Option<Json<Value>>) -> Result<impl IntoResponse, ServiceError> {
    let caller = caller(user, &headers, &state)?;
    let body = body_of(body);
    Ok(Json(blocking(move || connections::run_now(&caller, &id, &body, &state)).await?))
}
