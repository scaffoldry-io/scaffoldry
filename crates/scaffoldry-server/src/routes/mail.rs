//! Notifications phase 2 routes: the mail console. A Platform Admin only. The rules are in `service::mail`.

use crate::service::{mail, ServiceError};
use crate::state::{AuthUser, SharedState};
use axum::extract::State;
use axum::http::HeaderMap;
use axum::routing::{get, post};
use axum::{Extension, Json, Router};
use serde_json::Value;

pub fn router() -> Router<SharedState> {
    Router::new().route("/admin/mail", get(status)).route("/admin/mail/test", post(test))
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

async fn status(State(state): State<SharedState>, user: Option<Extension<AuthUser>>, headers: HeaderMap) -> Result<Json<Value>, ServiceError> {
    let caller = caller(user, &headers, &state)?;
    Ok(Json(blocking(move || mail::status(&caller, &state)).await?))
}

async fn test(State(state): State<SharedState>, user: Option<Extension<AuthUser>>, headers: HeaderMap, body: Option<Json<Value>>) -> Result<Json<Value>, ServiceError> {
    let caller = caller(user, &headers, &state)?;
    let body = body.map(|Json(b)| b).unwrap_or(Value::Null);
    Ok(Json(blocking(move || mail::send_test(&caller, &body, &state)).await?))
}
