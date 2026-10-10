//! Workspace and Collaborator Management Endpoints (REST Adapter)
//! Enforces workspace sharing boundaries, least privilege roles, and OSCAL AC-02/AC-03 controls
//! via the sovereign service layer.

use crate::service::workspaces as workspace_service;
pub use crate::service::workspaces::{
    CollaboratorPayload, CreateWorkspacePayload, UpdateWorkspacePayload, WorkspaceResponse,
};
use crate::service::ServiceError;
use crate::state::{AuthUser, CollaboratorRecord, SharedState, WorkspaceRecord};
use axum::{
    extract::{Extension, Path, State},
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    routing::get,
    Json, Router,
};
use serde::Deserialize;
use serde_json::Value;

pub fn router() -> Router<SharedState> {
    Router::new()
        .route("/workspaces", get(list_workspaces).post(create_workspace))
        .route("/workspaces/{id}", get(get_workspace).put(update_workspace))
        .route(
            "/workspaces/{id}/collaborators",
            get(list_collaborators).post(add_collaborator),
        )
        .route(
            "/workspaces/{id}/collaborators/{eppn}",
            axum::routing::put(update_collaborator).delete(remove_collaborator),
        )
}

#[derive(Debug, Deserialize)]
pub struct UpdateRolePayload {
    pub role: String,
}

fn resolve_caller(
    user: Option<Extension<AuthUser>>,
    headers: &HeaderMap,
    state: &SharedState,
) -> Result<AuthUser, ServiceError> {
    if let Some(Extension(u)) = user {
        return Ok(u);
    }
    crate::guard::session_user(state, headers)
        .ok_or_else(|| ServiceError::Unauthorized("A valid session is required".to_string()))
}

async fn list_workspaces(
    State(state): State<SharedState>,
    user: Option<Extension<AuthUser>>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, ServiceError> {
    let caller = resolve_caller(user, &headers, &state)?;
    let list = workspace_service::list_workspaces(&caller, &state)?;
    Ok(Json(list))
}

async fn create_workspace(
    State(state): State<SharedState>,
    user: Option<Extension<AuthUser>>,
    headers: HeaderMap,
    Json(payload): Json<Value>,
) -> Result<(StatusCode, Json<WorkspaceRecord>), ServiceError> {
    let caller = resolve_caller(user, &headers, &state)?;

    let name = payload["name"]
        .as_str()
        .ok_or_else(|| ServiceError::BadRequest("name is required".to_string()))?
        .to_string();

    let code = payload["code"]
        .as_str()
        .ok_or_else(|| ServiceError::BadRequest("code is required".to_string()))?
        .to_string();

    let input = CreateWorkspacePayload {
        name,
        code,
        organization: payload.get("organization").and_then(|v| v.as_str()).map(str::to_string),
        department: payload.get("department").and_then(|v| v.as_str()).map(str::to_string),
        description: payload.get("description").and_then(|v| v.as_str()).map(str::to_string),
        icon: payload.get("icon").and_then(|v| v.as_str()).map(str::to_string),
        visibility: payload.get("visibility").and_then(|v| v.as_str()).map(str::to_string),
        allowed_affiliations: payload
            .get("allowed_affiliations")
            .and_then(|v| v.as_array())
            .map(|arr| arr.iter().filter_map(|s| s.as_str().map(str::to_string)).collect()),
        data_classification: payload.get("data_classification").and_then(|v| v.as_str()).map(str::to_string),
        organization_id: payload.get("organization_id").and_then(|v| v.as_str()).and_then(|s| uuid::Uuid::parse_str(s).ok()),
    };

    let ws = workspace_service::create_workspace(&caller, input, &state)?;
    Ok((StatusCode::CREATED, Json(ws)))
}

async fn get_workspace(
    State(state): State<SharedState>,
    user: Option<Extension<AuthUser>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<WorkspaceResponse>, ServiceError> {
    let caller = resolve_caller(user, &headers, &state)?;
    let ws = workspace_service::get_workspace(&caller, &id, &state)?;
    Ok(Json(ws))
}

async fn update_workspace(
    State(state): State<SharedState>,
    user: Option<Extension<AuthUser>>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(payload): Json<UpdateWorkspacePayload>,
) -> Result<Json<WorkspaceRecord>, ServiceError> {
    let caller = resolve_caller(user, &headers, &state)?;
    let ws = workspace_service::update_workspace(&caller, &id, payload, &state)?;
    Ok(Json(ws))
}

async fn list_collaborators(
    State(state): State<SharedState>,
    user: Option<Extension<AuthUser>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<Vec<CollaboratorRecord>>, ServiceError> {
    let caller = resolve_caller(user, &headers, &state)?;
    let collabs = workspace_service::list_collaborators(&caller, &id, &state)?;
    Ok(Json(collabs))
}

async fn add_collaborator(
    State(state): State<SharedState>,
    user: Option<Extension<AuthUser>>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(payload): Json<CollaboratorPayload>,
) -> Result<(StatusCode, Json<CollaboratorRecord>), ServiceError> {
    let caller = resolve_caller(user, &headers, &state)?;
    let member = workspace_service::add_collaborator(&caller, &id, payload, &state)?;
    Ok((StatusCode::CREATED, Json(member)))
}

async fn update_collaborator(
    State(state): State<SharedState>,
    user: Option<Extension<AuthUser>>,
    headers: HeaderMap,
    Path((id, eppn)): Path<(String, String)>,
    Json(payload): Json<UpdateRolePayload>,
) -> Result<Json<CollaboratorRecord>, ServiceError> {
    let caller = resolve_caller(user, &headers, &state)?;
    let member = workspace_service::update_collaborator_role(&caller, &id, &eppn, &payload.role, &state)?;
    Ok(Json(member))
}

async fn remove_collaborator(
    State(state): State<SharedState>,
    user: Option<Extension<AuthUser>>,
    headers: HeaderMap,
    Path((id, eppn)): Path<(String, String)>,
) -> Result<StatusCode, ServiceError> {
    let caller = resolve_caller(user, &headers, &state)?;
    workspace_service::remove_collaborator(&caller, &id, &eppn, &state)?;
    Ok(StatusCode::NO_CONTENT)
}
