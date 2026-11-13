//! Guests phase 1: asking for a link, and trading it for a session. Neither needs a sign-in.

use crate::service::{guests, ServiceError};
use crate::state::SharedState;
use axum::{
    extract::{Path, State},
    http::HeaderMap,
    response::IntoResponse,
    routing::post,
    Json, Router,
};
use serde_json::Value;

pub fn router() -> Router<SharedState> {
    Router::new().route("/guest/verify", post(verify)).route("/guest/{app}/{form}/start", post(start))
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
