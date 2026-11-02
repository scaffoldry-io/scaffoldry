//! Integrations phase 1 routes: the secrets store. A Platform Admin only. The rules are in `service::secrets`.

use crate::service::{secrets, ServiceError};
use crate::state::{AuthUser, SharedState};
use axum::extract::{Path, State};
use axum::http::HeaderMap;
use axum::response::IntoResponse;
use axum::routing::{get, put};
use axum::{Extension, Json, Router};
use serde_json::Value;

pub fn router() -> Router<SharedState> {
    Router::new().route("/admin/secrets", get(list)).route("/admin/secrets/{name}", put(write).delete(remove))
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

async fn list(State(state): State<SharedState>, user: Option<Extension<AuthUser>>, headers: HeaderMap) -> Result<impl IntoResponse, ServiceError> {
    let caller = caller(user, &headers, &state)?;
    Ok(Json(blocking(move || secrets::list(&caller, &state)).await?))
}

async fn write(
    State(state): State<SharedState>,
    user: Option<Extension<AuthUser>>,
    headers: HeaderMap,
    Path(name): Path<String>,
    body: Option<Json<Value>>,
) -> Result<impl IntoResponse, ServiceError> {
    let caller = caller(user, &headers, &state)?;
    let body = body.map(|Json(b)| b).unwrap_or(Value::Null);
    Ok(Json(blocking(move || secrets::put(&caller, &name, &body, &state)).await?))
}

async fn remove(
    State(state): State<SharedState>,
    user: Option<Extension<AuthUser>>,
    headers: HeaderMap,
    Path(name): Path<String>,
    body: Option<Json<Value>>,
) -> Result<impl IntoResponse, ServiceError> {
    let caller = caller(user, &headers, &state)?;
    let body = body.map(|Json(b)| b).unwrap_or(Value::Null);
    Ok(Json(blocking(move || secrets::delete(&caller, &name, &body, &state)).await?))
}
