//! Guests phase 1: asking for a link, and trading it for a session. Neither needs a sign-in.

use crate::service::{guest_forms, guests, ServiceError};
use crate::state::SharedState;
use axum::{
    body::Body,
    extract::{DefaultBodyLimit, Path, State},
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use serde_json::Value;

pub fn router() -> Router<SharedState> {
    Router::new()
        .route("/guest/verify", post(verify))
        .route("/guest/stop", post(stop))
        .route("/guest/{app}/{form}", get(overview))
        .route("/guest/{app}/{form}/start", post(start))
        .route("/guest/{app}/{form}/records", post(create_record))
        .route("/guest/{app}/{form}/records/{id}", get(get_record).put(update_record))
        .route("/guest/{app}/{form}/files", post(upload).layer(DefaultBodyLimit::disable()))
}

type Refusal = (StatusCode, Json<serde_json::Value>);

fn join_error(_: tokio::task::JoinError) -> ServiceError {
    ServiceError::Internal("The request stopped".to_string())
}

async fn create_record(State(state): State<SharedState>, headers: HeaderMap, Path((app, form)): Path<(String, String)>, body: Option<Json<Value>>) -> Result<impl IntoResponse, ServiceError> {
    let body = body.map(|Json(b)| b).unwrap_or(Value::Null);
    let made = tokio::task::spawn_blocking(move || {
        let ctx = guest_forms::context(&state, &headers, &app, &form)?;
        let origin = crate::guard::origin(&state, &headers)?;
        guest_forms::create(&ctx, &origin, &body, &state)
    })
    .await
    .map_err(join_error)??;
    Ok((StatusCode::CREATED, Json(made)))
}

async fn get_record(State(state): State<SharedState>, headers: HeaderMap, Path((app, form, id)): Path<(String, String, String)>) -> Result<impl IntoResponse, ServiceError> {
    let made = tokio::task::spawn_blocking(move || guest_forms::get(&guest_forms::context(&state, &headers, &app, &form)?, &id, &state)).await.map_err(join_error)??;
    Ok(Json(made))
}

async fn update_record(State(state): State<SharedState>, headers: HeaderMap, Path((app, form, id)): Path<(String, String, String)>, body: Option<Json<Value>>) -> Result<impl IntoResponse, ServiceError> {
    let body = body.map(|Json(b)| b).unwrap_or(Value::Null);
    let made = tokio::task::spawn_blocking(move || {
        let ctx = guest_forms::context(&state, &headers, &app, &form)?;
        let origin = crate::guard::origin(&state, &headers)?;
        guest_forms::update(&ctx, &origin, &id, &body, &state)
    })
    .await
    .map_err(join_error)??;
    Ok(Json(made))
}

/// A guest's file. The session is checked first, then the upload goes the way a member's does, with a scan always needed and the guest's size limit.
async fn upload(State(state): State<SharedState>, headers: HeaderMap, Path((app, form)): Path<(String, String)>, body: Body) -> Result<(StatusCode, Json<Value>), Refusal> {
    let ctx = {
        let (state, headers) = (state.clone(), headers.clone());
        tokio::task::spawn_blocking(move || guest_forms::context(&state, &headers, &app, &form)).await.map_err(|e| join_error(e).into_pair())?.map_err(ServiceError::into_pair)?
    };
    crate::routes::files::upload_for(&state, &headers, &ctx.app, &ctx.guest, body, true).await
}

async fn start(State(state): State<SharedState>, headers: HeaderMap, Path((app, form)): Path<(String, String)>, body: Option<Json<Value>>) -> Result<impl IntoResponse, ServiceError> {
    let body = body.map(|Json(b)| b).unwrap_or(Value::Null);
    let made = tokio::task::spawn_blocking(move || guests::start(&state, &headers, &app, &form, &body)).await.map_err(|_| ServiceError::Internal("The request stopped".to_string()))??;
    Ok(Json(made))
}

async fn verify(State(state): State<SharedState>, body: Option<Json<Value>>) -> Result<impl IntoResponse, ServiceError> {
    let body = body.map(|Json(b)| b).unwrap_or(Value::Null);
    let made = tokio::task::spawn_blocking(move || guests::verify(&state, &body)).await.map_err(|_| ServiceError::Internal("The request stopped".to_string()))??;
    Ok(Json(made))
}

async fn overview(State(state): State<SharedState>, headers: HeaderMap, Path((app, form)): Path<(String, String)>) -> Result<impl IntoResponse, ServiceError> {
    let made = tokio::task::spawn_blocking(move || guest_forms::overview(&guest_forms::context(&state, &headers, &app, &form)?, &state)).await.map_err(join_error)??;
    Ok(Json(made))
}

async fn stop(State(state): State<SharedState>, body: Option<Json<Value>>) -> Result<impl IntoResponse, ServiceError> {
    let body = body.map(|Json(b)| b).unwrap_or(Value::Null);
    let made = tokio::task::spawn_blocking(move || guest_forms::stop(&state, &body)).await.map_err(join_error)??;
    Ok(Json(made))
}
