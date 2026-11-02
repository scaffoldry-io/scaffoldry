//! Lifecycle phase 3 routes: retention policies, a preview of the next purge, and certificates of destruction. A
//! Platform Admin or `compliance`. The rules are in `service::retention`.

use crate::service::{retention, ServiceError};
use crate::state::{AuthUser, SharedState};
use axum::extract::{Path, State};
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::IntoResponse;
use axum::routing::{get, post, put};
use axum::{Extension, Json, Router};
use serde_json::Value;

pub fn router() -> Router<SharedState> {
    Router::new()
        .route("/admin/retention/policies", get(list).post(create))
        .route("/admin/retention/policies/{id}", put(update))
        .route("/admin/retention/preview", post(preview))
        .route("/admin/purges", get(certificates))
        .route("/admin/purges/{id}", get(certificate))
}

fn caller(user: Option<Extension<AuthUser>>, headers: &HeaderMap, state: &SharedState) -> Result<AuthUser, ServiceError> {
    if let Some(Extension(u)) = user {
        return Ok(u);
    }
    crate::guard::session_user(state, headers).ok_or_else(|| ServiceError::Unauthorized("A valid session is required".to_string()))
}

async fn list(State(state): State<SharedState>, user: Option<Extension<AuthUser>>, headers: HeaderMap) -> Result<impl IntoResponse, ServiceError> {
    let caller = caller(user, &headers, &state)?;
    Ok(Json(retention::list(&caller, &state)?))
}

async fn create(State(state): State<SharedState>, user: Option<Extension<AuthUser>>, headers: HeaderMap, body: Option<Json<Value>>) -> Result<impl IntoResponse, ServiceError> {
    let caller = caller(user, &headers, &state)?;
    let body = body.map(|Json(b)| b).unwrap_or(Value::Null);
    let (policy, _) = retention::save(&caller, None, &body, &state)?;
    Ok((StatusCode::CREATED, Json(policy)))
}

async fn update(
    State(state): State<SharedState>,
    user: Option<Extension<AuthUser>>,
    headers: HeaderMap,
    Path(id): Path<String>,
    body: Option<Json<Value>>,
) -> Result<impl IntoResponse, ServiceError> {
    let caller = caller(user, &headers, &state)?;
    let body = body.map(|Json(b)| b).unwrap_or(Value::Null);
    let id = uuid::Uuid::parse_str(&id).map_err(|_| ServiceError::NotFound("Retention policy not found".to_string()))?;
    let (policy, _) = retention::save(&caller, Some(id), &body, &state)?;
    Ok(Json(policy))
}

async fn preview(State(state): State<SharedState>, user: Option<Extension<AuthUser>>, headers: HeaderMap, body: Option<Json<Value>>) -> Result<impl IntoResponse, ServiceError> {
    let caller = caller(user, &headers, &state)?;
    let body = body.map(|Json(b)| b).unwrap_or(Value::Null);
    Ok((StatusCode::ACCEPTED, Json(retention::preview(&caller, &body, &state)?)))
}

async fn certificates(State(state): State<SharedState>, user: Option<Extension<AuthUser>>, headers: HeaderMap) -> Result<impl IntoResponse, ServiceError> {
    let caller = caller(user, &headers, &state)?;
    Ok(Json(retention::certificates(&caller, &state)?))
}

/// One certificate, as a JSON download.
async fn certificate(State(state): State<SharedState>, user: Option<Extension<AuthUser>>, headers: HeaderMap, Path(id): Path<String>) -> Result<impl IntoResponse, ServiceError> {
    let caller = caller(user, &headers, &state)?;
    let body = retention::certificate(&caller, &id, &state)?;
    let mut resp = Json(body).into_response();
    if let Ok(v) = HeaderValue::from_str(&format!("attachment; filename=\"purge-certificate-{id}.json\"")) {
        resp.headers_mut().insert(header::CONTENT_DISPOSITION, v);
    }
    Ok(resp)
}
