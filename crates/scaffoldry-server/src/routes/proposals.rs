//! Proposals for the workspace screen. The same service functions the MCP tools call.

use crate::guard::session_user;
use crate::service::access::authorize_workspace;
use crate::service::ServiceError;
use crate::state::SharedState;
use axum::{
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    routing::{get, post},
    Json, Router,
};
use serde::Deserialize;
use serde_json::Value;

pub fn router() -> Router<SharedState> {
    Router::new()
        .route("/workspaces/{id}/proposals", get(list_proposals))
        .route("/proposals/{id}", get(get_proposal))
        .route("/proposals/{id}/decision", post(decide_proposal))
        .route("/proposals/{id}/preview", post(preview_proposal))
}

type Reply = Result<Json<Value>, (StatusCode, Json<Value>)>;

async fn blocking<T: Send + 'static>(f: impl FnOnce() -> Result<T, ServiceError> + Send + 'static) -> Result<T, (StatusCode, Json<Value>)> {
    tokio::task::spawn_blocking(f)
        .await
        .map_err(|_| ServiceError::internal("The read stopped").into_pair())?
        .map_err(ServiceError::into_pair)
}

async fn list_proposals(State(state): State<SharedState>, headers: HeaderMap, Path(ws): Path<String>) -> Reply {
    let user = session_user(&state, &headers).ok_or_else(|| ServiceError::unauthorized("Unauthorized").into_pair())?;
    blocking(move || {
        authorize_workspace(&user, &ws, "manage_workspace", &state)?;
        crate::service::proposals::list(&ws, &state)
    })
    .await
    .map(Json)
}

async fn get_proposal(State(state): State<SharedState>, headers: HeaderMap, Path(id): Path<String>) -> Reply {
    let user = session_user(&state, &headers).ok_or_else(|| ServiceError::unauthorized("Unauthorized").into_pair())?;
    blocking(move || {
        let ws = crate::service::proposals::workspace_of(&id, &state)?;
        authorize_workspace(&user, &ws, "manage_workspace", &state)?;
        crate::service::proposals::get(&id, &state)
    })
    .await
    .map(Json)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Decision {
    decision: String,
    rationale: String,
    /// To approve: the digest of the proposal that was reviewed.
    #[serde(default)]
    digest: Option<String>,
}

async fn decide_proposal(State(state): State<SharedState>, headers: HeaderMap, Path(id): Path<String>, Json(body): Json<Decision>) -> Reply {
    let user = session_user(&state, &headers).ok_or_else(|| ServiceError::unauthorized("Unauthorized").into_pair())?;
    blocking(move || crate::service::proposals::decide(&user, &id, &body.decision, &body.rationale, body.digest.as_deref(), &state)).await.map(Json)
}

/// Opens a read-only preview of the proposed app. The reply holds a token for it.
async fn preview_proposal(State(state): State<SharedState>, headers: HeaderMap, Path(id): Path<String>) -> Reply {
    let user = session_user(&state, &headers).ok_or_else(|| ServiceError::unauthorized("Unauthorized").into_pair())?;
    blocking(move || crate::service::preview::start(&user, &id, &state)).await.map(Json)
}
