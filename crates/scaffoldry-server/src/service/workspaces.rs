//! Sovereign Workspace Service Layer with Cedar Policy ABAC & Decision Ledger Audit

use crate::service::ServiceError;
use crate::state::{
    AuthUser, CollaboratorRecord, RecordDecisionInput, SharedState, WorkspaceRecord,
};
use chrono::Utc;
use scaffoldry_core::DecisionType;
use scaffoldry_policy::{PolicyDecision, WorkspaceActionInput};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceResponse {
    #[serde(flatten)]
    pub workspace: WorkspaceRecord,
    pub collaborators: Vec<CollaboratorRecord>,
    pub user_role: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateWorkspacePayload {
    pub name: String,
    pub code: String,
    pub organization: Option<String>,
    pub department: Option<String>,
    pub description: Option<String>,
    pub icon: Option<String>,
    pub visibility: Option<String>,
    pub allowed_affiliations: Option<Vec<String>>,
    pub data_classification: Option<String>,
    pub organization_id: Option<uuid::Uuid>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct UpdateWorkspacePayload {
    pub name: Option<String>,
    pub description: Option<String>,
    pub department: Option<String>,
    pub visibility: Option<String>,
    pub allowed_affiliations: Option<Vec<String>>,
    pub data_classification: Option<String>,
    pub icon: Option<String>,
    pub cedar_policy_guard: Option<String>,
    pub organization_id: Option<uuid::Uuid>,
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CollaboratorPayload {
    pub eppn: String,
    pub role: Option<String>,
    pub name: Option<String>,
    pub scoped_affiliation: Option<String>,
    pub department: Option<String>,
}

pub fn list_workspaces(
    caller: &AuthUser,
    state: &SharedState,
) -> Result<Vec<WorkspaceResponse>, ServiceError> {
    let ws_guard = state
        .workspaces
        .read()
        .map_err(|e| ServiceError::Internal(e.to_string()))?;
    let collabs_guard = state
        .collaborators
        .read()
        .map_err(|e| ServiceError::Internal(e.to_string()))?;

    let org_caller = crate::service::organizations::OrgCaller {
        eppn: caller.eppn.clone(),
        affiliation: caller.affiliation.clone(),
    };
    let orgs_map = state.organizations.read().unwrap();
    let all_orgs: Vec<crate::state::OrganizationNode> = orgs_map.values().cloned().collect();
    let roles: Vec<crate::state::RoleRow> = state.roles.read().unwrap().clone();

    let mut response_list = Vec::new();

    for ws in ws_guard.values() {
        let collabs = collabs_guard.get(&ws.id).cloned().unwrap_or_default();
        let (is_member, member_role): (bool, Option<String>) =
            match collabs.iter().find(|m| m.eppn == caller.eppn) {
                Some(m) => (true, Some(m.role.clone())),
                None => (false, None),
            };

        let in_org_scope = match ws.organization_id {
            Some(org_id) => crate::service::organizations::unit_in_scope(&org_caller, org_id, &all_orgs, &roles),
            None => caller.affiliation == "central_admin",
        };

        if !in_org_scope && !is_member {
            continue;
        }

        if in_org_scope && !is_member {
            response_list.push(WorkspaceResponse {
                workspace: ws.clone(),
                collaborators: collabs,
                user_role: Some("admin".to_string()),
            });
            continue;
        }

        let norm_role = member_role.as_ref().map(|r| r.to_lowercase());

        let decision = state
            .policy_engine
            .authorize_workspace_action(&WorkspaceActionInput {
                principal_eppn: &caller.eppn,
                principal_affiliation: &caller.affiliation,
                principal_department: &caller.department,
                action_name: "access_workspace",
                workspace_id: &ws.id,
                workspace_department: &ws.department,
                workspace_visibility: &ws.visibility,
                is_member,
                member_role: norm_role.as_deref(),
            });

        if let Ok(res) = decision {
            if res.decision == PolicyDecision::Allow {
                response_list.push(WorkspaceResponse {
                    workspace: ws.clone(),
                    collaborators: collabs,
                    user_role: member_role,
                });
            }
        }
    }

    Ok(response_list)
}

pub fn get_workspace(
    caller: &AuthUser,
    id: &str,
    state: &SharedState,
) -> Result<WorkspaceResponse, ServiceError> {
    let ws = {
        let ws_guard = state
            .workspaces
            .read()
            .map_err(|e| ServiceError::Internal(e.to_string()))?;
        ws_guard
            .get(id)
            .cloned()
            .ok_or_else(|| ServiceError::NotFound(format!("Workspace '{id}' not found")))?
    };

    let collabs = {
        let collabs_guard = state
            .collaborators
            .read()
            .map_err(|e| ServiceError::Internal(e.to_string()))?;
        collabs_guard.get(id).cloned().unwrap_or_default()
    };

    let (is_member, member_role): (bool, Option<String>) =
        match collabs.iter().find(|m| m.eppn == caller.eppn) {
            Some(m) => (true, Some(m.role.clone())),
            None => (false, None),
        };

    let norm_role = member_role.as_ref().map(|r| r.to_lowercase());

    let decision = state
        .policy_engine
        .authorize_workspace_action(&WorkspaceActionInput {
            principal_eppn: &caller.eppn,
            principal_affiliation: &caller.affiliation,
            principal_department: &caller.department,
            action_name: "access_workspace",
            workspace_id: &ws.id,
            workspace_department: &ws.department,
            workspace_visibility: &ws.visibility,
            is_member,
            member_role: norm_role.as_deref(),
        })
        .map_err(|e| ServiceError::Internal(e.to_string()))?;

    if decision.decision != PolicyDecision::Allow {
        return Err(ServiceError::Forbidden {
            message: "403 Forbidden: Cedar Policy restricts access to this workspace".to_string(),
            reasons: decision.reasons,
            diagnostics: decision.diagnostics,
            policy: decision.deciding_policy,
        });
    }

    Ok(WorkspaceResponse {
        workspace: ws,
        collaborators: collabs,
        user_role: member_role,
    })
}

pub fn create_workspace(
    caller: &AuthUser,
    payload: CreateWorkspacePayload,
    state: &SharedState,
) -> Result<WorkspaceRecord, ServiceError> {
    let target_org_id = payload.organization_id.unwrap_or_else(|| {
        uuid::Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap()
    });

    {
        let orgs_map = state.organizations.read().unwrap();
        if !orgs_map.contains_key(&target_org_id) {
            return Err(ServiceError::BadRequest(format!("Unknown organization_id: {target_org_id}")));
        }
        let all_orgs: Vec<_> = orgs_map.values().cloned().collect();
        let roles = state.roles.read().unwrap().clone();
        let org_caller = crate::service::organizations::OrgCaller {
            eppn: caller.eppn.clone(),
            affiliation: caller.affiliation.clone(),
        };
        if !crate::service::organizations::unit_in_scope(&org_caller, target_org_id, &all_orgs, &roles) {
            return Err(ServiceError::forbidden("organization_id is not in scope"));
        }
    }

    let id = Uuid::new_v4().to_string();
    let now = Utc::now().to_rfc3339();

    let lead = if caller.name.is_empty() {
        "Principal Investigator".to_string()
    } else {
        caller.name.clone()
    };

    let record = WorkspaceRecord {
        id: id.clone(),
        name: payload.name.clone(),
        code: payload.code.clone(),
        organization: payload.organization.unwrap_or_else(|| "University".to_string()),
        department: payload.department.unwrap_or_else(|| "general".to_string()),
        description: payload.description.unwrap_or_default(),
        icon: payload.icon.unwrap_or_else(|| "📁".to_string()),
        lead,
        visibility: payload.visibility.unwrap_or_else(|| "restricted".to_string()),
        allowed_affiliations: payload.allowed_affiliations.unwrap_or_else(|| {
            vec![
                "faculty".to_string(),
                "staff".to_string(),
                "student".to_string(),
            ]
        }),
        data_classification: payload
            .data_classification
            .unwrap_or_else(|| "Internal".to_string()),
        cedar_policy_guard: None,
        created_at: now.clone(),
        organization_id: payload.organization_id.or_else(|| {
            uuid::Uuid::parse_str("00000000-0000-0000-0000-000000000001").ok()
        }),
    };

    state.append_ledger_entry(RecordDecisionInput {
        principal: caller.eppn.clone(),
        organization_code: payload.code,
        app_slug: None,
        decision_type: DecisionType::WorkspaceCreated,
        oscal_control_id: "AC-02".to_string(),
        rationale: format!(
            "Principal {} created workspace {}",
            caller.eppn, record.name
        ),
        payload: &json!({
            "workspace_id": id,
            "name": record.name,
            "department": record.department,
            "visibility": record.visibility
        }),
    }).map_err(|e| ServiceError::Internal(e.to_string()))?;

    {
        let mut ws_guard = state
            .workspaces
            .write()
            .map_err(|e| ServiceError::Internal(e.to_string()))?;
        ws_guard.insert(id.clone(), record.clone());
    }

    let initial_owner = CollaboratorRecord {
        id: Uuid::new_v4().to_string(),
        workspace_id: id.clone(),
        eppn: caller.eppn.clone(),
        name: caller.name.clone(),
        role: "owner".to_string(),
        scoped_affiliation: caller.affiliation.clone(),
        department: caller.department.clone(),
        added_at: now,
    };

    if let Some(ref repo) = state.repository {
        repo.upsert_workspace(&record).map_err(|e| ServiceError::Internal(e.to_string()))?;
        repo.upsert_collaborator(&initial_owner).map_err(|e| ServiceError::Internal(e.to_string()))?;
    }

    {
        let mut collabs_guard = state
            .collaborators
            .write()
            .map_err(|e| ServiceError::Internal(e.to_string()))?;
        collabs_guard.entry(id.clone()).or_default().push(initial_owner);
    }

    Ok(record)
}

pub fn update_workspace(
    caller: &AuthUser,
    id: &str,
    payload: UpdateWorkspacePayload,
    state: &SharedState,
) -> Result<WorkspaceRecord, ServiceError> {
    let mut ws = {
        let ws_guard = state
            .workspaces
            .read()
            .map_err(|e| ServiceError::Internal(e.to_string()))?;
        ws_guard
            .get(id)
            .cloned()
            .ok_or_else(|| ServiceError::NotFound(format!("Workspace '{id}' not found")))?
    };

    let collabs = {
        let collabs_guard = state
            .collaborators
            .read()
            .map_err(|e| ServiceError::Internal(e.to_string()))?;
        collabs_guard.get(id).cloned().unwrap_or_default()
    };

    let (is_member, member_role) = match collabs.iter().find(|m| m.eppn == caller.eppn) {
        Some(m) => (true, Some(m.role.as_str())),
        None => (false, None),
    };

    let norm_role = member_role.map(str::to_lowercase);

    let is_admin = {
        let orgs: Vec<crate::state::OrganizationNode> = state.organizations.read().map(|g| g.values().cloned().collect()).unwrap_or_default();
        let roles: Vec<crate::state::RoleRow> = state.roles.read().map(|g| g.clone()).unwrap_or_default();
        let org_caller = crate::service::organizations::OrgCaller {
            eppn: caller.eppn.clone(),
            affiliation: caller.affiliation.clone(),
        };
        crate::service::organizations::is_platform_admin(&org_caller, &orgs, &roles)
    };
    let effective_aff = if is_admin { "central_admin" } else { &caller.affiliation };

    let decision = state
        .policy_engine
        .authorize_workspace_action(&WorkspaceActionInput {
            principal_eppn: &caller.eppn,
            principal_affiliation: effective_aff,
            principal_department: &caller.department,
            action_name: "manage_workspace",
            workspace_id: &ws.id,
            workspace_department: &ws.department,
            workspace_visibility: &ws.visibility,
            is_member,
            member_role: norm_role.as_deref(),
        })
        .map_err(|e| ServiceError::Internal(e.to_string()))?;

    if decision.decision != PolicyDecision::Allow {
        return Err(ServiceError::Forbidden {
            message: "403 Forbidden: Cedar Policy restricts managing this workspace to Owners and Admins".to_string(),
            reasons: decision.reasons,
            diagnostics: decision.diagnostics,
            policy: decision.deciding_policy,
        });
    }

    if let Some(ref class) = payload.data_classification {
        match class.as_str() {
            "Public" | "Internal" | "Restricted" | "FERPA Sensitive" => {}
            _ => {
                return Err(ServiceError::BadRequest(
                    "Invalid data_classification: must be one of Public, Internal, Restricted, FERPA Sensitive".to_string(),
                ));
            }
        }
    }
    if let Some(ref vis) = payload.visibility {
        match vis.as_str() {
            "restricted" | "departmental" | "institutional" => {}
            _ => {
                return Err(ServiceError::BadRequest(
                    "Invalid visibility: must be one of restricted, departmental, institutional".to_string(),
                ));
            }
        }
    }
    if let Some(org_id) = payload.organization_id {
        let exists = {
            let orgs = state.organizations.read().map_err(|e| ServiceError::Internal(e.to_string()))?;
            orgs.contains_key(&org_id)
        };
        if !exists {
            return Err(ServiceError::BadRequest("Organization unit does not exist".to_string()));
        }
    }

    let old_org_id = ws.organization_id;
    let old_visibility = ws.visibility.clone();
    let old_classification = ws.data_classification.clone();

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
    if let Some(org_id) = payload.organization_id {
        ws.organization_id = Some(org_id);
    }

    let rationale = payload.reason.filter(|r| !r.trim().is_empty()).unwrap_or_else(|| {
        format!(
            "Principal {} updated configuration for workspace {}",
            caller.eppn, ws.name
        )
    });

    let mut changes = serde_json::Map::new();
    if old_org_id != ws.organization_id {
        changes.insert(
            "organization_id".to_string(),
            json!({ "old": old_org_id, "new": ws.organization_id }),
        );
    }
    if old_visibility != ws.visibility {
        changes.insert(
            "visibility".to_string(),
            json!({ "old": old_visibility, "new": ws.visibility }),
        );
    }
    if old_classification != ws.data_classification {
        changes.insert(
            "data_classification".to_string(),
            json!({ "old": old_classification, "new": ws.data_classification }),
        );
    }

    state.append_ledger_entry(RecordDecisionInput {
        principal: caller.eppn.clone(),
        organization_code: ws.code.clone(),
        app_slug: None,
        decision_type: DecisionType::WorkspaceUpdated,
        oscal_control_id: "AC-03".to_string(),
        rationale,
        payload: &json!({
            "workspace_id": id,
            "visibility": ws.visibility,
            "old_visibility": old_visibility,
            "organization_id": ws.organization_id,
            "old_organization_id": old_org_id,
            "data_classification": ws.data_classification,
            "old_data_classification": old_classification,
            "changes": changes,
        }),
    }).map_err(|e| ServiceError::Internal(e.to_string()))?;

    if let Some(ref repo) = state.repository {
        repo.upsert_workspace(&ws).map_err(|e| ServiceError::Internal(e.to_string()))?;
    }

    {
        let mut ws_guard = state
            .workspaces
            .write()
            .map_err(|e| ServiceError::Internal(e.to_string()))?;
        ws_guard.insert(id.to_string(), ws.clone());
    }

    Ok(ws)
}

pub fn list_collaborators(
    caller: &AuthUser,
    workspace_id: &str,
    state: &SharedState,
) -> Result<Vec<CollaboratorRecord>, ServiceError> {
    // Ensures caller has access to workspace
    let _ = get_workspace(caller, workspace_id, state)?;
    let collabs_guard = state
        .collaborators
        .read()
        .map_err(|e| ServiceError::Internal(e.to_string()))?;
    Ok(collabs_guard.get(workspace_id).cloned().unwrap_or_default())
}

pub fn add_collaborator(
    caller: &AuthUser,
    workspace_id: &str,
    payload: CollaboratorPayload,
    state: &SharedState,
) -> Result<CollaboratorRecord, ServiceError> {
    let ws = {
        let ws_guard = state
            .workspaces
            .read()
            .map_err(|e| ServiceError::Internal(e.to_string()))?;
        ws_guard
            .get(workspace_id)
            .cloned()
            .ok_or_else(|| ServiceError::NotFound(format!("Workspace '{workspace_id}' not found")))?
    };

    let collabs = {
        let collabs_guard = state
            .collaborators
            .read()
            .map_err(|e| ServiceError::Internal(e.to_string()))?;
        collabs_guard.get(workspace_id).cloned().unwrap_or_default()
    };

    let (is_member, member_role) = match collabs.iter().find(|m| m.eppn == caller.eppn) {
        Some(m) => (true, Some(m.role.as_str())),
        None => (false, None),
    };
    let norm_role = member_role.map(str::to_lowercase);

    let decision = state
        .policy_engine
        .authorize_workspace_action(&WorkspaceActionInput {
            principal_eppn: &caller.eppn,
            principal_affiliation: &caller.affiliation,
            principal_department: &caller.department,
            action_name: "manage_workspace",
            workspace_id: &ws.id,
            workspace_department: &ws.department,
            workspace_visibility: &ws.visibility,
            is_member,
            member_role: norm_role.as_deref(),
        })
        .map_err(|e| ServiceError::Internal(e.to_string()))?;

    if decision.decision != PolicyDecision::Allow {
        return Err(ServiceError::Forbidden {
            message: "403 Forbidden: Cedar Policy restricts managing members to Owners and Admins".to_string(),
            reasons: decision.reasons,
            diagnostics: decision.diagnostics,
            policy: decision.deciding_policy,
        });
    }

    if collabs.iter().any(|c| c.eppn == payload.eppn) {
        return Err(ServiceError::BadRequest(format!(
            "User {} is already a member of workspace {}",
            payload.eppn, workspace_id
        )));
    }

    let role_str = payload.role.unwrap_or_else(|| "Viewer".to_string());
    let name_str = payload.name.unwrap_or_else(|| payload.eppn.clone());
    let affiliation_str = payload
        .scoped_affiliation
        .unwrap_or_else(|| "faculty".to_string());
    let dept_str = payload
        .department
        .unwrap_or_else(|| ws.department.clone());

    let new_member = CollaboratorRecord {
        id: Uuid::new_v4().to_string(),
        workspace_id: workspace_id.to_string(),
        eppn: payload.eppn.clone(),
        name: name_str,
        role: role_str.clone(),
        scoped_affiliation: affiliation_str,
        department: dept_str,
        added_at: Utc::now().to_rfc3339(),
    };

    state.append_ledger_entry(RecordDecisionInput {
        principal: caller.eppn.clone(),
        organization_code: ws.code,
        app_slug: None,
        decision_type: DecisionType::WorkspaceMemberAdded,
        oscal_control_id: "AC-02".to_string(),
        rationale: format!(
            "Principal {} added {} as {} to workspace {}",
            caller.eppn, payload.eppn, role_str, ws.name
        ),
        payload: &json!({
            "workspace_id": workspace_id,
            "target_eppn": payload.eppn,
            "role": role_str
        }),
    }).map_err(|e| ServiceError::Internal(e.to_string()))?;

    if let Some(ref repo) = state.repository {
        repo.upsert_collaborator(&new_member).map_err(|e| ServiceError::Internal(e.to_string()))?;
    }

    {
        let mut collabs_guard = state
            .collaborators
            .write()
            .map_err(|e| ServiceError::Internal(e.to_string()))?;
        collabs_guard
            .entry(workspace_id.to_string())
            .or_default()
            .push(new_member.clone());
    }

    Ok(new_member)
}

pub fn update_collaborator_role(
    caller: &AuthUser,
    workspace_id: &str,
    target_eppn: &str,
    new_role: &str,
    state: &SharedState,
) -> Result<CollaboratorRecord, ServiceError> {
    let ws = {
        let ws_guard = state
            .workspaces
            .read()
            .map_err(|e| ServiceError::Internal(e.to_string()))?;
        ws_guard
            .get(workspace_id)
            .cloned()
            .ok_or_else(|| ServiceError::NotFound(format!("Workspace '{workspace_id}' not found")))?
    };

    let mut collabs = {
        let collabs_guard = state
            .collaborators
            .read()
            .map_err(|e| ServiceError::Internal(e.to_string()))?;
        collabs_guard.get(workspace_id).cloned().unwrap_or_default()
    };

    let (is_member, member_role) = match collabs.iter().find(|m| m.eppn == caller.eppn) {
        Some(m) => (true, Some(m.role.as_str())),
        None => (false, None),
    };
    let norm_role = member_role.map(str::to_lowercase);

    let decision = state
        .policy_engine
        .authorize_workspace_action(&WorkspaceActionInput {
            principal_eppn: &caller.eppn,
            principal_affiliation: &caller.affiliation,
            principal_department: &caller.department,
            action_name: "manage_workspace",
            workspace_id: &ws.id,
            workspace_department: &ws.department,
            workspace_visibility: &ws.visibility,
            is_member,
            member_role: norm_role.as_deref(),
        })
        .map_err(|e| ServiceError::Internal(e.to_string()))?;

    if decision.decision != PolicyDecision::Allow {
        return Err(ServiceError::Forbidden {
            message: "403 Forbidden: Cedar Policy restricts managing member roles to Owners and Admins".to_string(),
            reasons: decision.reasons,
            diagnostics: decision.diagnostics,
            policy: decision.deciding_policy,
        });
    }

    let member = collabs
        .iter_mut()
        .find(|m| m.eppn == target_eppn)
        .ok_or_else(|| ServiceError::NotFound(format!("Member '{target_eppn}' not found")))?;

    member.role = new_role.to_string();
    let updated = member.clone();

    state.append_ledger_entry(RecordDecisionInput {
        principal: caller.eppn.clone(),
        organization_code: ws.code,
        app_slug: None,
        decision_type: DecisionType::WorkspaceMemberRoleUpdated,
        oscal_control_id: "AC-03".to_string(),
        rationale: format!(
            "Principal {} modified role of {} to {} in workspace {}",
            caller.eppn, target_eppn, new_role, ws.name
        ),
        payload: &json!({
            "workspace_id": workspace_id,
            "target_eppn": target_eppn,
            "new_role": new_role
        }),
    }).map_err(|e| ServiceError::Internal(e.to_string()))?;

    if let Some(ref repo) = state.repository {
        repo.upsert_collaborator(&updated).map_err(|e| ServiceError::Internal(e.to_string()))?;
    }

    {
        let mut collabs_guard = state
            .collaborators
            .write()
            .map_err(|e| ServiceError::Internal(e.to_string()))?;
        collabs_guard.insert(workspace_id.to_string(), collabs);
    }

    Ok(updated)
}

pub fn remove_collaborator(
    caller: &AuthUser,
    workspace_id: &str,
    target_eppn: &str,
    state: &SharedState,
) -> Result<(), ServiceError> {
    let ws = {
        let ws_guard = state
            .workspaces
            .read()
            .map_err(|e| ServiceError::Internal(e.to_string()))?;
        ws_guard
            .get(workspace_id)
            .cloned()
            .ok_or_else(|| ServiceError::NotFound(format!("Workspace '{workspace_id}' not found")))?
    };

    let collabs = {
        let collabs_guard = state
            .collaborators
            .read()
            .map_err(|e| ServiceError::Internal(e.to_string()))?;
        collabs_guard.get(workspace_id).cloned().unwrap_or_default()
    };

    let (is_member, member_role) = match collabs.iter().find(|m| m.eppn == caller.eppn) {
        Some(m) => (true, Some(m.role.as_str())),
        None => (false, None),
    };
    let norm_role = member_role.map(str::to_lowercase);

    let decision = state
        .policy_engine
        .authorize_workspace_action(&WorkspaceActionInput {
            principal_eppn: &caller.eppn,
            principal_affiliation: &caller.affiliation,
            principal_department: &caller.department,
            action_name: "manage_workspace",
            workspace_id: &ws.id,
            workspace_department: &ws.department,
            workspace_visibility: &ws.visibility,
            is_member,
            member_role: norm_role.as_deref(),
        })
        .map_err(|e| ServiceError::Internal(e.to_string()))?;

    if decision.decision != PolicyDecision::Allow {
        return Err(ServiceError::Forbidden {
            message: "403 Forbidden: Cedar Policy restricts managing member roles to Owners and Admins".to_string(),
            reasons: decision.reasons,
            diagnostics: decision.diagnostics,
            policy: decision.deciding_policy,
        });
    }

    let mut remaining = collabs;
    let initial_len = remaining.len();
    remaining.retain(|m| m.eppn != target_eppn);

    if remaining.len() == initial_len {
        return Err(ServiceError::NotFound(format!("Member '{target_eppn}' not found")));
    }

    state.append_ledger_entry(RecordDecisionInput {
        principal: caller.eppn.clone(),
        organization_code: ws.code,
        app_slug: None,
        decision_type: DecisionType::WorkspaceMemberRemoved,
        oscal_control_id: "AC-02".to_string(),
        rationale: format!(
            "Principal {} removed member {} from workspace {}",
            caller.eppn, target_eppn, ws.name
        ),
        payload: &json!({
            "workspace_id": workspace_id,
            "removed_eppn": target_eppn
        }),
    }).map_err(|e| ServiceError::Internal(e.to_string()))?;

    if let Some(ref repo) = state.repository {
        repo.remove_collaborator(workspace_id, target_eppn).map_err(|e| ServiceError::Internal(e.to_string()))?;
    }

    {
        let mut collabs_guard = state
            .collaborators
            .write()
            .map_err(|e| ServiceError::Internal(e.to_string()))?;
        collabs_guard.insert(workspace_id.to_string(), remaining);
    }

    Ok(())
}

pub fn manage_workspace_member(
    caller: &AuthUser,
    workspace_id: &str,
    action: &str,
    eppn: &str,
    role: Option<&str>,
    name: Option<&str>,
    state: &SharedState,
) -> Result<Value, ServiceError> {
    match action.to_lowercase().as_str() {
        "add" => {
            let res = add_collaborator(
                caller,
                workspace_id,
                CollaboratorPayload {
                    eppn: eppn.to_string(),
                    role: role.map(str::to_string),
                    name: name.map(str::to_string),
                    scoped_affiliation: None,
                    department: None,
                },
                state,
            )?;
            Ok(serde_json::to_value(res).unwrap_or(json!({ "status": "added" })))
        }
        "update" => {
            let r = role.ok_or_else(|| ServiceError::BadRequest("Role is required for update".to_string()))?;
            let res = update_collaborator_role(caller, workspace_id, eppn, r, state)?;
            Ok(serde_json::to_value(res).unwrap_or(json!({ "status": "updated" })))
        }
        "remove" | "delete" => {
            remove_collaborator(caller, workspace_id, eppn, state)?;
            Ok(json!({ "status": "removed", "eppn": eppn, "workspace_id": workspace_id }))
        }
        other => Err(ServiceError::BadRequest(format!(
            "Unsupported member action: {other}. Expected 'add', 'update', or 'remove'."
        ))),
    }
}
