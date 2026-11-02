//! Lifecycle phase 4 routes: legal holds. The rules are in `service::holds`.

use crate::service::{holds, ServiceError};
use crate::state::{AuthUser, SharedState};
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Extension, Json, Router};
use serde_json::Value;

pub fn router() -> Router<SharedState> {
    Router::new()
        .route("/admin/holds", get(list).post(place))
        .route("/admin/holds/preview", post(preview))
        .route("/admin/holds/{id}/release", post(release))
        .route("/apps/{slug}/records/{id}/hold", get(record))
}

fn caller(user: Option<Extension<AuthUser>>, headers: &HeaderMap, state: &SharedState) -> Result<AuthUser, ServiceError> {
    if let Some(Extension(u)) = user {
        return Ok(u);
    }
    crate::guard::session_user(state, headers).ok_or_else(|| ServiceError::Unauthorized("A valid session is required".to_string()))
}

fn body_of(body: Option<Json<Value>>) -> Value {
    body.map(|Json(b)| b).unwrap_or(Value::Null)
}

async fn list(State(state): State<SharedState>, user: Option<Extension<AuthUser>>, headers: HeaderMap) -> Result<impl IntoResponse, ServiceError> {
    let caller = caller(user, &headers, &state)?;
    Ok(Json(holds::list(&caller, &state)?))
}

async fn place(State(state): State<SharedState>, user: Option<Extension<AuthUser>>, headers: HeaderMap, body: Option<Json<Value>>) -> Result<impl IntoResponse, ServiceError> {
    let caller = caller(user, &headers, &state)?;
    Ok((StatusCode::CREATED, Json(holds::place(&caller, &body_of(body), &state)?)))
}

async fn preview(State(state): State<SharedState>, user: Option<Extension<AuthUser>>, headers: HeaderMap, body: Option<Json<Value>>) -> Result<impl IntoResponse, ServiceError> {
    let caller = caller(user, &headers, &state)?;
    Ok((StatusCode::ACCEPTED, Json(holds::preview(&caller, &body_of(body), &state)?)))
}

async fn release(
    State(state): State<SharedState>,
    user: Option<Extension<AuthUser>>,
    headers: HeaderMap,
    Path(id): Path<String>,
    body: Option<Json<Value>>,
) -> Result<impl IntoResponse, ServiceError> {
    let caller = caller(user, &headers, &state)?;
    Ok(Json(holds::release(&caller, &id, &body_of(body), &state)?))
}

async fn record(State(state): State<SharedState>, user: Option<Extension<AuthUser>>, headers: HeaderMap, Path((slug, id)): Path<(String, String)>) -> Result<impl IntoResponse, ServiceError> {
    let caller = caller(user, &headers, &state)?;
    Ok(Json(holds::record(&caller, &slug, &id, &state)?))
}
