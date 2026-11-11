//! Integrations phase 5 routes: the console's outbound panels. A Platform Admin only. The rules are in `service::outbound`.

use crate::service::{outbound, ServiceError};
use crate::state::{AuthUser, SharedState};
use axum::extract::{Path, State};
use axum::http::HeaderMap;
use axum::response::IntoResponse;
use axum::routing::{get, post, put};
use axum::{Extension, Json, Router};
use serde_json::Value;

pub fn router() -> Router<SharedState> {
    Router::new()
        .route("/admin/egress", get(egress))
        .route("/admin/egress/allow", put(egress_save))
        .route("/admin/egress/test", post(egress_test))
        .route("/admin/webhooks", get(webhooks))
        .route("/admin/webhooks/{app}/{id}/deliveries", get(deliveries))
        .route("/admin/webhooks/{app}/{id}/disable", post(disable))
        .route("/admin/webhooks/{app}/{id}/enable", post(enable))
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

fn body_of(body: Option<Json<Value>>) -> Value {
    body.map(|Json(b)| b).unwrap_or(Value::Null)
}

async fn egress(State(state): State<SharedState>, user: Option<Extension<AuthUser>>, headers: HeaderMap) -> Result<impl IntoResponse, ServiceError> {
    let caller = caller(user, &headers, &state)?;
    Ok(Json(blocking(move || outbound::egress(&caller, &state)).await?))
}

async fn egress_save(State(state): State<SharedState>, user: Option<Extension<AuthUser>>, headers: HeaderMap, body: Option<Json<Value>>) -> Result<impl IntoResponse, ServiceError> {
    let caller = caller(user, &headers, &state)?;
    let body = body_of(body);
    Ok(Json(blocking(move || outbound::egress_save(&caller, &body, &state)).await?))
}

async fn egress_test(State(state): State<SharedState>, user: Option<Extension<AuthUser>>, headers: HeaderMap, body: Option<Json<Value>>) -> Result<impl IntoResponse, ServiceError> {
    let caller = caller(user, &headers, &state)?;
    let body = body_of(body);
    Ok(Json(blocking(move || outbound::egress_test(&caller, &body, &state)).await?))
}

async fn webhooks(State(state): State<SharedState>, user: Option<Extension<AuthUser>>, headers: HeaderMap) -> Result<impl IntoResponse, ServiceError> {
    let caller = caller(user, &headers, &state)?;
    Ok(Json(blocking(move || outbound::webhooks(&caller, &state)).await?))
}

async fn deliveries(State(state): State<SharedState>, user: Option<Extension<AuthUser>>, headers: HeaderMap, Path((app, id)): Path<(String, String)>) -> Result<impl IntoResponse, ServiceError> {
    let caller = caller(user, &headers, &state)?;
    Ok(Json(blocking(move || outbound::deliveries(&caller, &app, &id, &state)).await?))
}

async fn disable(State(state): State<SharedState>, user: Option<Extension<AuthUser>>, headers: HeaderMap, Path((app, id)): Path<(String, String)>, body: Option<Json<Value>>) -> Result<impl IntoResponse, ServiceError> {
    let caller = caller(user, &headers, &state)?;
    let body = body_of(body);
    Ok(Json(blocking(move || outbound::set_enabled(&caller, &app, &id, false, &body, &state)).await?))
}

async fn enable(State(state): State<SharedState>, user: Option<Extension<AuthUser>>, headers: HeaderMap, Path((app, id)): Path<(String, String)>, body: Option<Json<Value>>) -> Result<impl IntoResponse, ServiceError> {
    let caller = caller(user, &headers, &state)?;
    let body = body_of(body);
    Ok(Json(blocking(move || outbound::set_enabled(&caller, &app, &id, true, &body, &state)).await?))
}
