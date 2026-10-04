//! Workspace and Collaborator Management Endpoints

use crate::state::{CollaboratorRecord, SharedState, WorkspaceRecord};
use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    routing::get,
    Json, Router,
};
use serde_json::{json, Value};
use uuid::Uuid;

pub fn router() -> Router<SharedState> {
    Router::new()
        .route("/workspaces", get(list_workspaces).post(create_workspace))
        .route("/workspaces/{id}", get(get_workspace))
        .route(
            "/workspaces/{id}/collaborators",
            get(list_collaborators).post(add_collaborator),
        )
}

async fn list_workspaces(State(state): State<SharedState>) -> impl IntoResponse {
    let ws = state.workspaces.read().unwrap();
    let list: Vec<WorkspaceRecord> = ws.values().cloned().collect();
    Json(list)
}

async fn create_workspace(
    State(state): State<SharedState>,
    Json(payload): Json<Value>,
) -> Result<(StatusCode, Json<WorkspaceRecord>), (StatusCode, Json<Value>)> {
    let name = payload["name"]
        .as_str()
        .ok_or_else(|| (StatusCode::BAD_REQUEST, Json(json!({"error": "name is required"}))))?
        .to_string();

    let code = payload["code"]
        .as_str()
        .ok_or_else(|| (StatusCode::BAD_REQUEST, Json(json!({"error": "code is required"}))))?
        .to_string();

    let org = payload
        .get("organization")
        .and_then(|v| v.as_str())
        .unwrap_or("University")
        .to_string();

    let id = Uuid::new_v4().to_string();
    let record = WorkspaceRecord {
        id: id.clone(),
        name,
        code,
        organization: org,
    };

    state.workspaces.write().unwrap().insert(id, record.clone());
    Ok((StatusCode::CREATED, Json(record)))
}

async fn get_workspace(
    State(state): State<SharedState>,
    Path(id): Path<String>,
) -> Result<Json<WorkspaceRecord>, StatusCode> {
    let ws = state.workspaces.read().unwrap();
    let record = ws.get(&id).ok_or(StatusCode::NOT_FOUND)?;
    Ok(Json(record.clone()))
}

async fn list_collaborators(
    State(state): State<SharedState>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let collabs = state.collaborators.read().unwrap();
    let list = collabs.get(&id).cloned().unwrap_or_default();
    Json(list)
}

async fn add_collaborator(
    State(state): State<SharedState>,
    Path(id): Path<String>,
    Json(payload): Json<Value>,
) -> Result<(StatusCode, Json<CollaboratorRecord>), (StatusCode, Json<Value>)> {
    let eppn = payload["eppn"]
        .as_str()
        .ok_or_else(|| (StatusCode::BAD_REQUEST, Json(json!({"error": "eppn is required"}))))?
        .to_string();

    let role = payload
        .get("role")
        .and_then(|v| v.as_str())
        .unwrap_or("Viewer")
        .to_string();

    let scoped_affiliation = payload
        .get("scoped_affiliation")
        .and_then(|v| v.as_str())
        .unwrap_or("member")
        .to_string();

    let collab_id = Uuid::new_v4().to_string();
    let record = CollaboratorRecord {
        id: collab_id,
        workspace_id: id.clone(),
        eppn,
        role,
        scoped_affiliation,
    };

    let mut collabs = state.collaborators.write().unwrap();
    collabs.entry(id).or_default().push(record.clone());

    Ok((StatusCode::CREATED, Json(record)))
}
