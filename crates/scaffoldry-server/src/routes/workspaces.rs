//! Workspace and Collaborator Management Endpoints with Cedar Policy ABAC & Cryptographic Audit
//! Enforces workspace sharing boundaries, least privilege roles, and OSCAL AC-02/AC-03 controls.

use crate::state::{AuthUser, CollaboratorRecord, RecordDecisionInput, SharedState, WorkspaceRecord};
use axum::{
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    routing::get,
    Json, Router,
};
use chrono::Utc;
use scaffoldry_core::DecisionType;
use scaffoldry_policy::{PolicyDecision, WorkspaceActionInput};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use uuid::Uuid;

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

#[derive(Debug, Serialize)]
pub struct WorkspaceResponse {
    #[serde(flatten)]
    pub workspace: WorkspaceRecord,
    pub collaborators: Vec<CollaboratorRecord>,
    pub user_role: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateWorkspacePayload {
    pub name: Option<String>,
    pub description: Option<String>,
    pub department: Option<String>,
    pub visibility: Option<String>,
    pub allowed_affiliations: Option<Vec<String>>,
    pub data_classification: Option<String>,
    pub icon: Option<String>,
    pub cedar_policy_guard: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct CollaboratorPayload {
    pub eppn: String,
    pub role: Option<String>,
    pub name: Option<String>,
    pub scoped_affiliation: Option<String>,
    pub department: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateRolePayload {
    pub role: String,
}

/// Identity comes from the session only. Never from headers or bodies.
fn require_caller(
    headers: &HeaderMap,
    state: &SharedState,
) -> Result<AuthUser, (StatusCode, Json<Value>)> {
    crate::guard::session_user(state, headers).ok_or_else(crate::guard::unauthorized)
}

async fn list_workspaces(
    State(state): State<SharedState>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, (StatusCode, Json<Value>)> {
    let caller = Some(require_caller(&headers, &state)?);
    let ws_guard = state.workspaces.read().unwrap();
    let collabs_guard = state.collaborators.read().unwrap();

    let mut response_list = Vec::new();

    for ws in ws_guard.values() {
        let collabs = collabs_guard.get(&ws.id).cloned().unwrap_or_default();
        let (is_member, member_role): (bool, Option<String>) = if let Some(ref c) = caller {
            match collabs.iter().find(|m| m.eppn == c.eppn) {
                Some(m) => (true, Some(m.role.clone())),
                None => (false, None),
            }
        } else {
            (false, None)
        };

        if let Some(ref c) = caller {
            let decision = state.policy_engine.authorize_workspace_action(&WorkspaceActionInput {
                principal_eppn: &c.eppn,
                principal_affiliation: &c.affiliation,
                principal_department: &c.department,
                action_name: "access_workspace",
                workspace_id: &ws.id,
                workspace_department: &ws.department,
                workspace_visibility: &ws.visibility,
                is_member,
                member_role: member_role.as_deref(),
            });

            match decision {
                Ok(res) if res.decision == PolicyDecision::Allow => {
                    response_list.push(WorkspaceResponse {
                        workspace: ws.clone(),
                        collaborators: collabs,
                        user_role: member_role,
                    });
                }
                _ => {}
            }
        } else {
            // Unreachable behind the session guard. Fail closed if it is ever reached.
            continue;
        }
    }

    Ok(Json(json!(response_list)))
}

async fn create_workspace(
    State(state): State<SharedState>,
    headers: HeaderMap,
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

    let department = payload
        .get("department")
        .and_then(|v| v.as_str())
        .unwrap_or("general")
        .to_string();

    let description = payload
        .get("description")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    let icon = payload
        .get("icon")
        .and_then(|v| v.as_str())
        .unwrap_or("📁")
        .to_string();

    let visibility = payload
        .get("visibility")
        .and_then(|v| v.as_str())
        .unwrap_or("restricted")
        .to_string();

    let data_classification = payload
        .get("data_classification")
        .and_then(|v| v.as_str())
        .unwrap_or("Internal")
        .to_string();

    let allowed_affiliations: Vec<String> = payload
        .get("allowed_affiliations")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|s| s.as_str().map(|str_val| str_val.to_string()))
                .collect()
        })
        .unwrap_or_else(|| vec!["faculty".to_string(), "staff".to_string(), "student".to_string()]);

    let id = Uuid::new_v4().to_string();
    let now = Utc::now().to_rfc3339();

    let caller = Some(require_caller(&headers, &state)?);
    let lead = caller.as_ref().map(|c| c.name.clone()).unwrap_or_else(|| "Principal Investigator".to_string());

    let record = WorkspaceRecord {
        id: id.clone(),
        name: name.clone(),
        code: code.clone(),
        organization: org.clone(),
        department: department.clone(),
        description,
        icon,
        lead,
        visibility: visibility.clone(),
        allowed_affiliations,
        data_classification,
        cedar_policy_guard: None,
        created_at: now.clone(),
    };

    state.workspaces.write().unwrap().insert(id.clone(), record.clone());

    if let Some(ref c) = caller {
        let initial_owner = CollaboratorRecord {
            id: Uuid::new_v4().to_string(),
            workspace_id: id.clone(),
            eppn: c.eppn.clone(),
            name: c.name.clone(),
            role: "owner".to_string(),
            scoped_affiliation: c.affiliation.clone(),
            department: c.department.clone(),
            added_at: now.clone(),
        };
        state
            .collaborators
            .write()
            .unwrap()
            .entry(id.clone())
            .or_default()
            .push(initial_owner);

        let _ = state.append_ledger_entry(RecordDecisionInput {
            principal: c.eppn.clone(),
            organization_code: code,
            app_slug: None,
            decision_type: DecisionType::WorkspaceCreated,
            oscal_control_id: "AC-02".to_string(),
            rationale: format!("Principal {} created workspace {}", c.eppn, name),
            payload: &payload,
        });
    }

    Ok((StatusCode::CREATED, Json(record)))
}

async fn get_workspace(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<WorkspaceResponse>, (StatusCode, Json<Value>)> {
    let ws = {
        let ws_guard = state.workspaces.read().unwrap();
        ws_guard.get(&id).cloned().ok_or_else(|| {
            (
                StatusCode::NOT_FOUND,
                Json(json!({ "error": "Workspace not found" })),
            )
        })?
    };

    let collabs = {
        let collabs_guard = state.collaborators.read().unwrap();
        collabs_guard.get(&id).cloned().unwrap_or_default()
    };

    let caller = Some(require_caller(&headers, &state)?);
    let (is_member, member_role): (bool, Option<String>) = if let Some(ref c) = caller {
        match collabs.iter().find(|m| m.eppn == c.eppn) {
            Some(m) => (true, Some(m.role.clone())),
            None => (false, None),
        }
    } else {
        (false, None)
    };

    if let Some(ref c) = caller {
        let decision = state.policy_engine.authorize_workspace_action(&WorkspaceActionInput {
            principal_eppn: &c.eppn,
            principal_affiliation: &c.affiliation,
            principal_department: &c.department,
            action_name: "access_workspace",
            workspace_id: &ws.id,
            workspace_department: &ws.department,
            workspace_visibility: &ws.visibility,
            is_member,
            member_role: member_role.as_deref(),
        });

        match decision {
            Ok(res) if res.decision == PolicyDecision::Allow => {}
            Ok(res) => {
                return Err((
                    StatusCode::FORBIDDEN,
                    Json(json!({
                        "error": "403 Forbidden: Cedar Policy restricts access to this workspace",
                        "diagnostics": res.diagnostics,
                        "reasons": res.reasons,
                        "workspace_id": ws.id,
                        "visibility": ws.visibility,
                    })),
                ));
            }
            Err(e) => {
                return Err((
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(json!({ "error": format!("Policy evaluation error: {e}") })),
                ));
            }
        }
    }

    Ok(Json(WorkspaceResponse {
        workspace: ws,
        collaborators: collabs,
        user_role: member_role,
    }))
}

async fn update_workspace(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(payload): Json<UpdateWorkspacePayload>,
) -> Result<Json<WorkspaceRecord>, (StatusCode, Json<Value>)> {
    let mut ws = {
        let ws_guard = state.workspaces.read().unwrap();
        ws_guard.get(&id).cloned().ok_or_else(|| {
            (
                StatusCode::NOT_FOUND,
                Json(json!({ "error": "Workspace not found" })),
            )
        })?
    };

    let collabs = {
        let collabs_guard = state.collaborators.read().unwrap();
        collabs_guard.get(&id).cloned().unwrap_or_default()
    };

    let caller = Some(require_caller(&headers, &state)?);
    let (is_member, member_role) = if let Some(ref c) = caller {
        match collabs.iter().find(|m| m.eppn == c.eppn) {
            Some(m) => (true, Some(m.role.as_str())),
            None => (false, None),
        }
    } else {
        (false, None)
    };

    if let Some(ref c) = caller {
        let decision = state.policy_engine.authorize_workspace_action(&WorkspaceActionInput {
            principal_eppn: &c.eppn,
            principal_affiliation: &c.affiliation,
            principal_department: &c.department,
            action_name: "manage_workspace",
            workspace_id: &ws.id,
            workspace_department: &ws.department,
            workspace_visibility: &ws.visibility,
            is_member,
            member_role,
        });

        match decision {
            Ok(res) if res.decision == PolicyDecision::Allow => {}
            _ => {
                return Err((
                    StatusCode::FORBIDDEN,
                    Json(json!({ "error": "403 Forbidden: Cedar Policy restricts managing this workspace to Owners and Admins" })),
                ));
            }
        }
    }

    if let Some(name) = payload.name {
        ws.name = name;
    }
    if let Some(description) = payload.description {
        ws.description = description;
    }
    if let Some(department) = payload.department {
        ws.department = department;
    }
    if let Some(visibility) = payload.visibility {
        ws.visibility = visibility;
    }
    if let Some(affiliations) = payload.allowed_affiliations {
        ws.allowed_affiliations = affiliations;
    }
    if let Some(classification) = payload.data_classification {
        ws.data_classification = classification;
    }
    if let Some(icon) = payload.icon {
        ws.icon = icon;
    }
    if let Some(guard) = payload.cedar_policy_guard {
        ws.cedar_policy_guard = Some(guard);
    }

    state.workspaces.write().unwrap().insert(id.clone(), ws.clone());

    if let Some(ref c) = caller {
        let _ = state.append_ledger_entry(RecordDecisionInput {
            principal: c.eppn.clone(),
            organization_code: ws.code.clone(),
            app_slug: None,
            decision_type: DecisionType::WorkspaceUpdated,
            oscal_control_id: "AC-03".to_string(),
            rationale: format!("Principal {} updated configuration for workspace {}", c.eppn, ws.name),
            payload: &json!({ "workspace_id": id, "visibility": ws.visibility }),
        });
    }

    Ok(Json(ws))
}

async fn list_collaborators(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<Vec<CollaboratorRecord>>, (StatusCode, Json<Value>)> {
    let _ = require_caller(&headers, &state)?;
    let collabs = state.collaborators.read().unwrap();
    let list = collabs.get(&id).cloned().unwrap_or_default();
    Ok(Json(list))
}

async fn add_collaborator(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(payload): Json<CollaboratorPayload>,
) -> Result<(StatusCode, Json<CollaboratorRecord>), (StatusCode, Json<Value>)> {
    let ws = {
        let ws_guard = state.workspaces.read().unwrap();
        ws_guard.get(&id).cloned().ok_or_else(|| {
            (
                StatusCode::NOT_FOUND,
                Json(json!({ "error": "Workspace not found" })),
            )
        })?
    };

    let caller = Some(require_caller(&headers, &state)?);
    let collabs = {
        let collabs_guard = state.collaborators.read().unwrap();
        collabs_guard.get(&id).cloned().unwrap_or_default()
    };

    let (is_member, member_role) = if let Some(ref c) = caller {
        match collabs.iter().find(|m| m.eppn == c.eppn) {
            Some(m) => (true, Some(m.role.as_str())),
            None => (false, None),
        }
    } else {
        (false, None)
    };

    if let Some(ref c) = caller {
        let decision = state.policy_engine.authorize_workspace_action(&WorkspaceActionInput {
            principal_eppn: &c.eppn,
            principal_affiliation: &c.affiliation,
            principal_department: &c.department,
            action_name: "manage_workspace",
            workspace_id: &ws.id,
            workspace_department: &ws.department,
            workspace_visibility: &ws.visibility,
            is_member,
            member_role,
        });

        match decision {
            Ok(res) if res.decision == PolicyDecision::Allow => {}
            _ => {
                return Err((
                    StatusCode::FORBIDDEN,
                    Json(json!({ "error": "403 Forbidden: Cedar Policy restricts managing members to Owners and Admins" })),
                ));
            }
        }
    }

    let role = payload.role.unwrap_or_else(|| "Viewer".to_string());
    let scoped_affiliation = payload.scoped_affiliation.unwrap_or_else(|| "member".to_string());
    let name = payload.name.unwrap_or_else(|| payload.eppn.clone());
    let department = payload.department.unwrap_or_else(|| "general".to_string());

    let record = CollaboratorRecord {
        id: Uuid::new_v4().to_string(),
        workspace_id: id.clone(),
        eppn: payload.eppn.clone(),
        name,
        role: role.clone(),
        scoped_affiliation,
        department,
        added_at: Utc::now().to_rfc3339(),
    };

    let mut collabs_mut = state.collaborators.write().unwrap();
    let members = collabs_mut.entry(id.clone()).or_default();
    if let Some(pos) = members.iter().position(|m| m.eppn == payload.eppn) {
        members[pos] = record.clone();
    } else {
        members.push(record.clone());
    }

    if let Some(ref c) = caller {
        let _ = state.append_ledger_entry(RecordDecisionInput {
            principal: c.eppn.clone(),
            organization_code: ws.code.clone(),
            app_slug: None,
            decision_type: DecisionType::WorkspaceMemberAdded,
            oscal_control_id: "AC-02".to_string(),
            rationale: format!(
                "Principal {} added {} as {} to workspace {}",
                c.eppn, payload.eppn, role, ws.name
            ),
            payload: &json!({ "workspace_id": id, "eppn": payload.eppn, "role": role }),
        });
    }

    Ok((StatusCode::CREATED, Json(record)))
}

async fn update_collaborator(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Path((id, eppn)): Path<(String, String)>,
    Json(payload): Json<UpdateRolePayload>,
) -> Result<Json<CollaboratorRecord>, (StatusCode, Json<Value>)> {
    let ws = {
        let ws_guard = state.workspaces.read().unwrap();
        ws_guard.get(&id).cloned().ok_or_else(|| {
            (
                StatusCode::NOT_FOUND,
                Json(json!({ "error": "Workspace not found" })),
            )
        })?
    };

    let caller = Some(require_caller(&headers, &state)?);
    let collabs = {
        let collabs_guard = state.collaborators.read().unwrap();
        collabs_guard.get(&id).cloned().unwrap_or_default()
    };

    let (is_member, member_role) = if let Some(ref c) = caller {
        match collabs.iter().find(|m| m.eppn == c.eppn) {
            Some(m) => (true, Some(m.role.as_str())),
            None => (false, None),
        }
    } else {
        (false, None)
    };

    if let Some(ref c) = caller {
        let decision = state.policy_engine.authorize_workspace_action(&WorkspaceActionInput {
            principal_eppn: &c.eppn,
            principal_affiliation: &c.affiliation,
            principal_department: &c.department,
            action_name: "manage_workspace",
            workspace_id: &ws.id,
            workspace_department: &ws.department,
            workspace_visibility: &ws.visibility,
            is_member,
            member_role,
        });

        match decision {
            Ok(res) if res.decision == PolicyDecision::Allow => {}
            _ => {
                return Err((
                    StatusCode::FORBIDDEN,
                    Json(json!({ "error": "403 Forbidden: Cedar Policy restricts role updates to Owners and Admins" })),
                ));
            }
        }
    }

    let mut collabs_mut = state.collaborators.write().unwrap();
    let members = collabs_mut.get_mut(&id).ok_or_else(|| {
        (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": "No collaborators found for workspace" })),
        )
    })?;

    let member = members.iter_mut().find(|m| m.eppn == eppn).ok_or_else(|| {
        (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": "Collaborator not found in workspace" })),
        )
    })?;

    member.role = payload.role.clone();
    let updated = member.clone();

    if let Some(ref c) = caller {
        let _ = state.append_ledger_entry(RecordDecisionInput {
            principal: c.eppn.clone(),
            organization_code: ws.code.clone(),
            app_slug: None,
            decision_type: DecisionType::WorkspaceMemberRoleUpdated,
            oscal_control_id: "AC-03".to_string(),
            rationale: format!(
                "Principal {} updated {} role to {} in workspace {}",
                c.eppn, eppn, payload.role, ws.name
            ),
            payload: &json!({ "workspace_id": id, "eppn": eppn, "role": payload.role }),
        });
    }

    Ok(Json(updated))
}

async fn remove_collaborator(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Path((id, eppn)): Path<(String, String)>,
) -> Result<StatusCode, (StatusCode, Json<Value>)> {
    let ws = {
        let ws_guard = state.workspaces.read().unwrap();
        ws_guard.get(&id).cloned().ok_or_else(|| {
            (
                StatusCode::NOT_FOUND,
                Json(json!({ "error": "Workspace not found" })),
            )
        })?
    };

    let caller = Some(require_caller(&headers, &state)?);
    let collabs = {
        let collabs_guard = state.collaborators.read().unwrap();
        collabs_guard.get(&id).cloned().unwrap_or_default()
    };

    let (is_member, member_role) = if let Some(ref c) = caller {
        match collabs.iter().find(|m| m.eppn == c.eppn) {
            Some(m) => (true, Some(m.role.as_str())),
            None => (false, None),
        }
    } else {
        (false, None)
    };

    if let Some(ref c) = caller {
        let decision = state.policy_engine.authorize_workspace_action(&WorkspaceActionInput {
            principal_eppn: &c.eppn,
            principal_affiliation: &c.affiliation,
            principal_department: &c.department,
            action_name: "manage_workspace",
            workspace_id: &ws.id,
            workspace_department: &ws.department,
            workspace_visibility: &ws.visibility,
            is_member,
            member_role,
        });

        match decision {
            Ok(res) if res.decision == PolicyDecision::Allow => {}
            _ => {
                return Err((
                    StatusCode::FORBIDDEN,
                    Json(json!({ "error": "403 Forbidden: Cedar Policy restricts member removal to Owners and Admins" })),
                ));
            }
        }
    }

    let mut collabs_mut = state.collaborators.write().unwrap();
    if let Some(members) = collabs_mut.get_mut(&id) {
        members.retain(|m| m.eppn != eppn);
    }

    if let Some(ref c) = caller {
        let _ = state.append_ledger_entry(RecordDecisionInput {
            principal: c.eppn.clone(),
            organization_code: ws.code.clone(),
            app_slug: None,
            decision_type: DecisionType::WorkspaceMemberRemoved,
            oscal_control_id: "AC-02".to_string(),
            rationale: format!(
                "Principal {} removed member {} from workspace {}",
                c.eppn, eppn, ws.name
            ),
            payload: &json!({ "workspace_id": id, "eppn": eppn }),
        });
    }

    Ok(StatusCode::NO_CONTENT)
}
