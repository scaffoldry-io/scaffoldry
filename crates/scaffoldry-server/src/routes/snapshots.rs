//! Snapshots. The rules are in `service::snapshots`.

use crate::guard::session_user;
use crate::service::snapshots;
use crate::service::ServiceError;
use crate::state::SharedState;
use axum::{
    body::{Body, Bytes},
    extract::{Path, State},
    http::{header, HeaderMap, StatusCode},
    response::Response,
    routing::{get, post},
    Json, Router,
};
use serde_json::{json, Value};

pub fn router() -> Router<SharedState> {
    Router::new()
        .route("/apps/{slug}/snapshots", get(list).post(take))
        .route("/snapshots/{id}/download", get(download))
        .route("/snapshots/{id}/restore", post(restore))
}

type Reply = Result<(StatusCode, Json<Value>), (StatusCode, Json<Value>)>;

async fn blocking<T: Send + 'static>(f: impl FnOnce() -> Result<T, ServiceError> + Send + 'static) -> Result<T, (StatusCode, Json<Value>)> {
    tokio::task::spawn_blocking(f)
        .await
        .map_err(|_| ServiceError::internal("The request stopped").into_pair())?
        .map_err(ServiceError::into_pair)
}

fn user_of(state: &SharedState, headers: &HeaderMap) -> Result<crate::state::AuthUser, (StatusCode, Json<Value>)> {
    session_user(state, headers).ok_or_else(|| ServiceError::unauthorized("Unauthorized").into_pair())
}

fn json_body(body: &Bytes) -> Result<Value, (StatusCode, Json<Value>)> {
    if body.is_empty() {
        return Ok(json!({}));
    }
    serde_json::from_slice(body).map_err(|e| ServiceError::bad_request(format!("Invalid request body: {e}")).into_pair())
}

async fn take(State(state): State<SharedState>, headers: HeaderMap, Path(slug): Path<String>, body: Bytes) -> Reply {
    let user = user_of(&state, &headers)?;
    let body = json_body(&body)?;
    blocking(move || snapshots::take(&user, &slug, &body, &state)).await.map(|v| (StatusCode::ACCEPTED, Json(v)))
}

async fn list(State(state): State<SharedState>, headers: HeaderMap, Path(slug): Path<String>) -> Reply {
    let user = user_of(&state, &headers)?;
    blocking(move || snapshots::list(&user, &slug, &state)).await.map(|v| (StatusCode::OK, Json(v)))
}

async fn restore(State(state): State<SharedState>, headers: HeaderMap, Path(id): Path<String>, body: Bytes) -> Reply {
    let user = user_of(&state, &headers)?;
    let body = json_body(&body)?;
    blocking(move || snapshots::restore(&user, &id, &body, &state)).await.map(|v| (StatusCode::ACCEPTED, Json(v)))
}

async fn download(State(state): State<SharedState>, headers: HeaderMap, Path(id): Path<String>) -> Result<Response, (StatusCode, Json<Value>)> {
    let user = user_of(&state, &headers)?;
    let (name, bytes) = blocking(move || snapshots::download(&user, &id, &state)).await?;
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "application/gzip")
        .header(header::CONTENT_DISPOSITION, format!("attachment; filename=\"{name}\""))
        .header("x-content-type-options", "nosniff")
        .body(Body::from(bytes))
        .map_err(|e| ServiceError::internal(e.to_string()).into_pair())
}
