//! `POST /apps/{slug}/fields/convert-preview`. The rules are in `service::calc`.

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
    Router::new().route("/apps/{slug}/fields/convert-preview", post(convert_preview))
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
