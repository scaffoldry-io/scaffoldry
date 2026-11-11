//! Integrations phase 4 routes: a webhook's call log, and turning a webhook on again after it was turned off.

use crate::service::access::{authorize_app, authorize_workspace, AppAction};
use crate::service::ServiceError;
use crate::state::{AuthUser, SharedState};
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::routing::{get, post};
use axum::{Extension, Json, Router};
use scaffoldry_engine::HostRouter;
use serde_json::{json, Value};

pub fn router() -> Router<SharedState> {
    Router::new()
        .route("/apps/{slug}/webhooks/{id}/deliveries", get(deliveries))
        .route("/apps/{slug}/webhooks/{id}/enable", post(enable))
}

fn caller(user: Option<Extension<AuthUser>>, headers: &HeaderMap, state: &SharedState) -> Result<AuthUser, (StatusCode, Json<Value>)> {
    match user {
        Some(Extension(u)) => Ok(u),
        None => crate::guard::session_user(state, headers).ok_or_else(|| ServiceError::unauthorized("A valid session is required").into_pair()),
    }
}

/// The app's workspace, when the app has a webhook with this id.
fn webhook_of(state: &SharedState, slug: &str, id: &str) -> Result<Option<String>, ServiceError> {
    let eng = state.engine.read().map_err(|e| ServiceError::Internal(e.to_string()))?;
    let m = eng.resolve_by_slug(slug).ok_or_else(|| ServiceError::NotFound(format!("App '{slug}' not found")))?;
    if !m.integrations.webhooks.iter().any(|w| w.id == id) {
        return Err(ServiceError::NotFound(format!("The app has no webhook '{id}'")));
    }
    Ok(m.workspace_id.clone())
}

async fn deliveries(State(state): State<SharedState>, user: Option<Extension<AuthUser>>, headers: HeaderMap, Path((slug, id)): Path<(String, String)>) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let caller = caller(user, &headers, &state)?;
    tokio::task::spawn_blocking(move || -> Result<Value, ServiceError> {
        authorize_app(&caller, &slug, AppAction::Manage, &state)?;
        webhook_of(&state, &slug, &id)?;
        let log = crate::webhooks::call_log(&state.repository, &slug, &id).map_err(|e| ServiceError::Internal(e.to_string()))?;
        Ok(json!({ "deliveries": log }))
    })
    .await
    .map_err(|_| ServiceError::internal("The request stopped").into_pair())?
    .map(Json)
    .map_err(ServiceError::into_pair)
}

/// An owner of the app's workspace turns a webhook on again, after fixing what made it fail.
async fn enable(State(state): State<SharedState>, user: Option<Extension<AuthUser>>, headers: HeaderMap, Path((slug, id)): Path<(String, String)>) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let caller = caller(user, &headers, &state)?;
    tokio::task::spawn_blocking(move || -> Result<Value, ServiceError> {
        let ws = webhook_of(&state, &slug, &id)?.ok_or_else(|| ServiceError::forbidden("The app has no workspace, so nobody owns its webhooks"))?;
        authorize_workspace(&caller, &ws, "manage_workspace", &state).map_err(|_| ServiceError::forbidden("Only an owner of the app's workspace may turn a webhook on"))?;
        crate::webhooks::enable(&state.repository, &slug, &id).map_err(|e| ServiceError::Internal(e.to_string()))?;
        Ok(json!({ "app": slug, "webhook": id, "enabled": true }))
    })
    .await
    .map_err(|_| ServiceError::internal("The request stopped").into_pair())?
    .map(Json)
    .map_err(ServiceError::into_pair)
}
