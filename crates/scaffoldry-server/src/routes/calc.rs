//! `POST /apps/{slug}/fields/convert-preview`, and the two routes that turn a value join into a link. The rules are in
//! `service::calc`.

use crate::guard::session_user;
use crate::service::{calc, ServiceError};
use crate::state::SharedState;
use axum::{
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    routing::post,
    Json, Router,
};
use serde_json::Value;

pub fn router() -> Router<SharedState> {
    Router::new()
        .route("/apps/{slug}/fields/convert-preview", post(convert_preview))
        .route("/apps/{slug}/relations/{id}/convert-preview", post(relation_preview))
        .route("/apps/{slug}/relations/{id}/convert", post(relation_convert))
}

async fn relation_preview(State(state): State<SharedState>, headers: HeaderMap, Path((slug, id)): Path<(String, String)>) -> Result<(StatusCode, Json<Value>), (StatusCode, Json<Value>)> {
    let user = session_user(&state, &headers).ok_or_else(|| ServiceError::unauthorized("Unauthorized").into_pair())?;
    let job = tokio::task::spawn_blocking(move || calc::relation_preview(&user, &slug, &id, &state))
        .await
        .map_err(|_| ServiceError::internal("The preview stopped").into_pair())?
        .map_err(ServiceError::into_pair)?;
    Ok((StatusCode::ACCEPTED, Json(job)))
}

async fn relation_convert(State(state): State<SharedState>, headers: HeaderMap, Path((slug, id)): Path<(String, String)>) -> Result<(StatusCode, Json<Value>), (StatusCode, Json<Value>)> {
    let user = session_user(&state, &headers).ok_or_else(|| ServiceError::unauthorized("Unauthorized").into_pair())?;
    let proposal = tokio::task::spawn_blocking(move || calc::relation_convert(&user, &slug, &id, &state))
        .await
        .map_err(|_| ServiceError::internal("The proposal stopped").into_pair())?
        .map_err(ServiceError::into_pair)?;
    Ok((StatusCode::CREATED, Json(proposal)))
}

async fn convert_preview(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Path(slug): Path<String>,
    Json(body): Json<Value>,
) -> Result<(StatusCode, Json<Value>), (StatusCode, Json<Value>)> {
    let user = session_user(&state, &headers).ok_or_else(|| ServiceError::unauthorized("Unauthorized").into_pair())?;
    let job = tokio::task::spawn_blocking(move || calc::convert_preview(&user, &slug, &body, &state))
        .await
        .map_err(|_| ServiceError::internal("The preview stopped").into_pair())?
        .map_err(ServiceError::into_pair)?;
    Ok((StatusCode::ACCEPTED, Json(job)))
}
