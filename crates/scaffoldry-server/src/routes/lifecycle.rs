//! Lifecycle phase 2 routes. The rules are in `service::lifecycle`.

use crate::guard::session_user;
use crate::service::{lifecycle, ServiceError};
use crate::state::SharedState;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::routing::post;
use axum::{Json, Router};
use serde_json::Value;

pub fn router() -> Router<SharedState> {
    Router::new()
        .route("/apps/{slug}/tables/{table_id}/fields/{field}/restore", post(restore_field))
        .route("/apps/{slug}/archive", post(|s, h, p, b| set_app(s, h, p, b, true)))
        .route("/apps/{slug}/unarchive", post(|s, h, p, b| set_app(s, h, p, b, false)))
        .route("/workspaces/{id}/archive", post(|s, h, p, b| set_workspace(s, h, p, b, true)))
        .route("/workspaces/{id}/unarchive", post(|s, h, p, b| set_workspace(s, h, p, b, false)))
}

fn reason_of(body: &Option<Json<Value>>) -> String {
    body.as_ref().and_then(|b| b.get("reason")).and_then(Value::as_str).unwrap_or("").to_string()
}

async fn restore_field(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Path((slug, table, field)): Path<(String, String, String)>,
    body: Option<Json<Value>>,
) -> Result<(StatusCode, Json<Value>), (StatusCode, Json<Value>)> {
    let user = session_user(&state, &headers).ok_or_else(|| ServiceError::unauthorized("Unauthorized").into_pair())?;
    let reason = reason_of(&body);
    let proposal = tokio::task::spawn_blocking(move || lifecycle::restore_field(&user, &slug, &table, &field, &reason, &state))
        .await
        .map_err(|_| ServiceError::internal("The proposal stopped").into_pair())?
        .map_err(ServiceError::into_pair)?;
    Ok((StatusCode::CREATED, Json(proposal)))
}

async fn set_app(State(state): State<SharedState>, headers: HeaderMap, Path(slug): Path<String>, body: Option<Json<Value>>, archive: bool) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let user = session_user(&state, &headers).ok_or_else(|| ServiceError::unauthorized("Unauthorized").into_pair())?;
    let reason = reason_of(&body);
    tokio::task::spawn_blocking(move || lifecycle::set_app_archived(&user, &slug, archive, &reason, &state))
        .await
        .map_err(|_| ServiceError::internal("The change stopped").into_pair())?
        .map(Json)
        .map_err(ServiceError::into_pair)
}

async fn set_workspace(State(state): State<SharedState>, headers: HeaderMap, Path(id): Path<String>, body: Option<Json<Value>>, archive: bool) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let user = session_user(&state, &headers).ok_or_else(|| ServiceError::unauthorized("Unauthorized").into_pair())?;
    let reason = reason_of(&body);
    tokio::task::spawn_blocking(move || lifecycle::set_workspace_archived(&user, &id, archive, &reason, &state))
        .await
        .map_err(|_| ServiceError::internal("The change stopped").into_pair())?
        .map(Json)
        .map_err(ServiceError::into_pair)
}
