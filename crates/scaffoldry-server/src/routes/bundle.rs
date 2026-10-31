//! App bundles. The rules are in `service::bundle`.

use crate::guard::session_user;
use crate::service::bundle;
use crate::service::ServiceError;
use crate::state::SharedState;
use axum::{
    body::Bytes,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    routing::post,
    Json, Router,
};
use serde_json::{json, Value};

pub fn router() -> Router<SharedState> {
    Router::new()
        .route("/apps/{slug}/bundle", post(export))
        .route("/workspaces/{id}/bundle-import", post(import))
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

/// The body is optional. `{ "sample_data": true }` adds up to 5,000 sample records.
fn json_body(body: &Bytes) -> Result<Value, (StatusCode, Json<Value>)> {
    if body.is_empty() {
        return Ok(json!({}));
    }
    serde_json::from_slice(body).map_err(|e| ServiceError::bad_request(format!("Invalid request body: {e}")).into_pair())
}

async fn export(State(state): State<SharedState>, headers: HeaderMap, Path(slug): Path<String>, body: Bytes) -> Reply {
    let user = user_of(&state, &headers)?;
    let sample = json_body(&body)?.get("sample_data").and_then(Value::as_bool).unwrap_or(false);
    blocking(move || bundle::export(&user, &slug, sample, &state)).await.map(|v| (StatusCode::OK, Json(v)))
}

async fn import(State(state): State<SharedState>, headers: HeaderMap, Path(id): Path<String>, body: Bytes) -> Reply {
    let user = user_of(&state, &headers)?;
    let body = json_body(&body)?;
    blocking(move || bundle::import(&user, &id, &body, &state)).await.map(|v| (StatusCode::CREATED, Json(v)))
}
