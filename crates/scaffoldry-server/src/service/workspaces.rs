//! Sovereign Workspace Service Layer with Cedar Policy ABAC & Decision Ledger Audit

use crate::service::members::{self, Kind, Role};
use crate::service::ServiceError;
use crate::state::{
    AuthUser, CollaboratorRecord, RecordDecisionInput, SharedState, WorkspaceRecord,
};
use chrono::Utc;
use scaffoldry_core::DecisionType;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceResponse {
    #[serde(flatten)]
    pub workspace: WorkspaceRecord,
    pub collaborators: Vec<CollaboratorRecord>,
    pub user_role: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub guards: Vec<String>,
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
    pub data_classification: Option<String>,
    pub organization_id: Option<uuid::Uuid>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct UpdateWorkspacePayload {
    pub name: Option<String>,
    pub description: Option<String>,
    pub department: Option<String>,
    pub visibility: Option<String>,
    pub data_classification: Option<String>,
    pub icon: Option<String>,
    pub organization_id: Option<uuid::Uuid>,
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CollaboratorPayload {
    /// The address of a person. For a group, unit, or position, send `principal_type` and `principal_id`.
    #[serde(default)]
    pub eppn: String,
    #[serde(default)]
    pub principal_type: Option<String>,
    #[serde(default)]
    pub principal_id: Option<String>,
    pub role: Option<String>,
    pub name: Option<String>,
    pub scoped_affiliation: Option<String>,
    pub department: Option<String>,
    /// Limits the membership to one app of the workspace. Empty or absent means the whole workspace.
    #[serde(default)]
    pub app_slug: Option<String>,
    /// Why the grant is made. Optional. When given, the ledger records it as the rationale.
    #[serde(default)]
    pub reason: Option<String>,
}

pub fn list_workspaces(
    caller: &AuthUser,
    state: &SharedState,
) -> Result<Vec<WorkspaceResponse>, ServiceError> {
    // Copy what the loop needs and let the locks go. The loop asks for a role, and a role reads these same locks. A
    // thread that holds a read lock and asks again while a writer waits is stuck, and so is the writer.
    let snapshot: Vec<(WorkspaceRecord, Vec<CollaboratorRecord>)> = {
        let ws_guard = state
            .workspaces
            .read()
            .map_err(|e| ServiceError::Internal(e.to_string()))?;
        let collabs_guard = state
            .collaborators
            .read()
            .map_err(|e| ServiceError::Internal(e.to_string()))?;
        ws_guard
            .values()
            .map(|ws| (ws.clone(), collabs_guard.get(&ws.id).cloned().unwrap_or_default()))
            .collect()
    };

    let org_caller = crate::service::organizations::OrgCaller {
        eppn: caller.eppn.clone(),
        affiliation: caller.affiliation.clone(),
    };
    let roles = state.roles_of(&caller.eppn);
    let ids: Vec<String> = snapshot.iter().map(|(ws, _)| ws.id.clone()).collect();
    let member_roles = crate::service::members::member_roles(&caller.eppn, &ids, state);

    let mut response_list = Vec::new();

    for ((ws, collabs), role) in snapshot.iter().zip(member_roles) {
        let (is_member, member_role) = (role.is_some(), role.map(|r| r.as_str().to_string()));

        let in_org_scope = match ws.organization_id {
            Some(org_id) => {
                let tree = state.tree.read().unwrap_or_else(|p| p.into_inner());
                crate::service::organizations::unit_in_reach(&org_caller, org_id, &tree, &roles)
            }
            None => caller.affiliation == "central_admin",
        };

        if !in_org_scope && !is_member {
            continue;
        }

        if in_org_scope && !is_member {
            response_list.push(WorkspaceResponse {
                workspace: ws.clone(),
                collaborators: collabs.clone(),
                user_role: Some("admin".to_string()),
                guards: Vec::new(),
            });
            continue;
        }

        if crate::service::access::workspace_allowed(caller, ws, "access_workspace", state)? {
            response_list.push(WorkspaceResponse {
                workspace: ws.clone(),
                collaborators: collabs.clone(),
                user_role: member_role,
                guards: Vec::new(),
            });
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

    let role = crate::service::members::member_role(caller, id, state);
    let member_role = role.map(|r| r.as_str().to_string());

    crate::service::access::authorize_workspace_denying(
        caller,
        &ws,
        "access_workspace",
        "403 Forbidden: Cedar Policy restricts access to this workspace",
        state,
    )?;

    let guard_sentences: Vec<String> = {
        let g_guard = state.guards.read().ok();
        if let Some(ref g_map) = g_guard {
            if let Some(history) = g_map.get(id) {
                if let Some(current) = history.last() {
                    let org_names = state
                        .organizations
                        .read()
                        .map(|orgs| {
                            orgs.iter()
                                .map(|(id, node)| (id.to_string(), node.name.clone()))
                                .collect()
                        })
                        .unwrap_or_default();
                    if let Ok(rules) = serde_json::from_value::<Vec<scaffoldry_core::GuardRule>>(
                        current.rules.clone(),
                    ) {
                        rules.iter().map(|r| r.sentence(&org_names)).collect()
                    } else {
                        Vec::new()
                    }
                } else {
                    Vec::new()
                }
            } else {
                Vec::new()
            }
        } else {
            Vec::new()
        }
    };
    Ok(WorkspaceResponse {
        workspace: ws,
        collaborators: collabs,
        user_role: member_role,
        guards: guard_sentences,
    })
}

pub fn create_workspace(
    caller: &AuthUser,
    payload: CreateWorkspacePayload,
    state: &SharedState,
) -> Result<WorkspaceRecord, ServiceError> {
    let target_org_id = payload
        .organization_id
        .unwrap_or_else(|| uuid::Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap());

    {
        let orgs_map = state.organizations.read().unwrap();
        if !orgs_map.contains_key(&target_org_id) {
            return Err(ServiceError::BadRequest(format!(
                "Unknown organization_id: {target_org_id}"
            )));
        }
        let tree = state.tree.read().unwrap();
        let roles = state.roles_of(&caller.eppn);
        let org_caller = crate::service::organizations::OrgCaller {
            eppn: caller.eppn.clone(),
            affiliation: caller.affiliation.clone(),
        };
        if !crate::service::organizations::unit_in_reach(&org_caller, target_org_id, &tree, &roles)
        {
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
        organization: payload
            .organization
            .unwrap_or_else(|| "University".to_string()),
        department: payload.department.unwrap_or_else(|| "general".to_string()),
        description: payload.description.unwrap_or_default(),
        icon: payload.icon.unwrap_or_else(|| "📁".to_string()),
        lead,
        visibility: payload
            .visibility
            .unwrap_or_else(|| "restricted".to_string()),
        data_classification: payload
            .data_classification
            .unwrap_or_else(|| "Internal".to_string()),
        coordinates: Default::default(),
        created_at: now.clone(),
        organization_id: payload
            .organization_id
            .or_else(|| uuid::Uuid::parse_str("00000000-0000-0000-0000-000000000001").ok()),
        synthetic: false,
    };

    state
        .append_ledger_entry(RecordDecisionInput {
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
        })
        .map_err(|e| ServiceError::Internal(e.to_string()))?;

    let initial_owner = CollaboratorRecord {
        id: Uuid::new_v4().to_string(),
        workspace_id: id.clone(),
        eppn: caller.eppn.clone(),
        name: caller.name.clone(),
        role: "owner".to_string(),
        scoped_affiliation: caller.affiliation.clone(),
        department: caller.department.clone(),
        added_at: now,
        app_slug: String::new(),
    };

    {
        let repo = &state.repository;
        repo.upsert_workspace(&record)
            .map_err(|e| ServiceError::Internal(e.to_string()))?;
        repo.upsert_collaborator(&initial_owner)
            .map_err(|e| ServiceError::Internal(e.to_string()))?;
    }

    super::reload_after_write(state)?;

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

    crate::service::access::authorize_workspace_denying(
        caller,
        &ws,
        "manage_workspace",
        "403 Forbidden: Cedar Policy restricts managing this workspace to Owners and Admins",
        state,
    )?;

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
                    "Invalid visibility: must be one of restricted, departmental, institutional"
                        .to_string(),
                ));
            }
        }
    }
    if let Some(org_id) = payload.organization_id {
        let exists = {
            let orgs = state
                .organizations
                .read()
                .map_err(|e| ServiceError::Internal(e.to_string()))?;
            orgs.contains_key(&org_id)
        };
        if !exists {
            return Err(ServiceError::BadRequest(
                "Organization unit does not exist".to_string(),
            ));
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
    if let Some(classification) = payload.data_classification {
        ws.data_classification = classification;
    }
    if let Some(icon) = payload.icon {
        ws.icon = icon;
    }

    if let Some(org_id) = payload.organization_id {
        ws.organization_id = Some(org_id);
    }

    let rationale = payload
        .reason
        .filter(|r| !r.trim().is_empty())
        .unwrap_or_else(|| {
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

    state
        .append_ledger_entry(RecordDecisionInput {
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
        })
        .map_err(|e| ServiceError::Internal(e.to_string()))?;

    {
        let repo = &state.repository;
        repo.upsert_workspace(&ws)
            .map_err(|e| ServiceError::Internal(e.to_string()))?;
    }

    super::reload_after_write(state)?;

    Ok(ws)
}

/// True when `target` is a person who is the only person holding the owner role.
fn is_last_user_owner(collabs: &[CollaboratorRecord], target: &str) -> bool {
    let owners: Vec<&CollaboratorRecord> = collabs
        .iter()
        .filter(|c| {
            members::split_principal(&c.eppn).0 == Kind::User
                && Role::parse(&c.role) == Some(Role::Owner)
        })
        .collect();
    owners.len() == 1 && owners[0].eppn == target
}

/// A unit is a unit id. A position is `{key}@{unit id}`. Whether the group or unit exists is not
/// checked, so a membership can be set up before the directory sync that fills it.
fn check_principal(kind: Kind, id: &str) -> Result<(), ServiceError> {
    let bad = |what: &str| ServiceError::BadRequest(format!("principal_id must be {what}"));
    match kind {
        Kind::User | Kind::Group => Ok(()),
        Kind::Unit => Uuid::parse_str(id)
            .map(|_| ())
            .map_err(|_| bad("an org unit id")),
        Kind::Position => match id.split_once('@') {
            Some((key, unit)) if !key.is_empty() && Uuid::parse_str(unit).is_ok() => Ok(()),
            _ => Err(bad("{position key}@{unit id}")),
        },
    }
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

/// The app a membership names, when it names one. The app must belong to the workspace.
fn app_of_membership(
    app: Option<&str>,
    workspace_id: &str,
    state: &SharedState,
) -> Result<String, ServiceError> {
    use scaffoldry_engine::HostRouter;
    let Some(app) = app.map(str::trim).filter(|a| !a.is_empty()) else {
        return Ok(String::new());
    };
    let engine = state
        .engine
        .read()
        .map_err(|e| ServiceError::Internal(e.to_string()))?;
    let belongs = engine
        .resolve_by_slug(app)
        .is_some_and(|m| m.workspace_id.as_deref() == Some(workspace_id));
    if !belongs {
        return Err(ServiceError::BadRequest(format!(
            "App '{app}' is not in workspace {workspace_id}"
        )));
    }
    Ok(app.to_string())
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
        ws_guard.get(workspace_id).cloned().ok_or_else(|| {
            ServiceError::NotFound(format!("Workspace '{workspace_id}' not found"))
        })?
    };

    let collabs = {
        let collabs_guard = state
            .collaborators
            .read()
            .map_err(|e| ServiceError::Internal(e.to_string()))?;
        collabs_guard.get(workspace_id).cloned().unwrap_or_default()
    };

    crate::service::access::authorize_workspace_denying(
        caller,
        &ws,
        "manage_workspace",
        "403 Forbidden: Cedar Policy restricts managing members to Owners and Admins",
        state,
    )?;

    let (kind, principal_id) = match (
        payload.principal_type.as_deref(),
        payload.principal_id.as_deref().filter(|s| !s.is_empty()),
    ) {
        (None, None) if !payload.eppn.is_empty() => {
            let (k, id) = members::split_principal(&payload.eppn);
            (k, id.to_string())
        }
        (kind, Some(id)) => (
            kind.map_or(Some(Kind::User), Kind::parse).ok_or_else(|| {
                ServiceError::BadRequest(
                    "principal_type must be user, group, unit, or position".to_string(),
                )
            })?,
            id.to_string(),
        ),
        _ => {
            return Err(ServiceError::BadRequest(
                "principal_id is required".to_string(),
            ))
        }
    };
    check_principal(kind, &principal_id)?;
    let key = members::stored_key(kind, &principal_id);
    let app = app_of_membership(payload.app_slug.as_deref(), workspace_id, state)?;

    if collabs
        .iter()
        .any(|c| c.eppn.eq_ignore_ascii_case(&key) && c.app_slug == app)
    {
        let scope = if app.is_empty() {
            format!("workspace {workspace_id}")
        } else {
            format!("app {app}")
        };
        return Err(ServiceError::BadRequest(format!(
            "{key} is already a member of {scope}"
        )));
    }

    let role_str = payload.role.unwrap_or_else(|| "Viewer".to_string());
    let role = Role::parse(&role_str).ok_or_else(|| {
        ServiceError::BadRequest(
            "role must be viewer, commenter, editor, admin, or owner".to_string(),
        )
    })?;
    if kind != Kind::User && role == Role::Owner {
        return Err(ServiceError::BadRequest(
            "Only a person can be an owner".to_string(),
        ));
    }
    if !app.is_empty() && role == Role::Owner {
        return Err(ServiceError::BadRequest(
            "A membership limited to one app cannot be an owner".to_string(),
        ));
    }
    let name_str = payload.name.unwrap_or_else(|| key.clone());
    let affiliation_str = payload
        .scoped_affiliation
        .unwrap_or_else(|| "faculty".to_string());
    let dept_str = payload.department.unwrap_or_else(|| ws.department.clone());

    let new_member = CollaboratorRecord {
        id: Uuid::new_v4().to_string(),
        workspace_id: workspace_id.to_string(),
        eppn: key.clone(),
        name: name_str,
        role: role_str.clone(),
        scoped_affiliation: affiliation_str,
        department: dept_str,
        added_at: Utc::now().to_rfc3339(),
        app_slug: app.clone(),
    };

    state
        .append_ledger_entry(RecordDecisionInput {
            principal: caller.eppn.clone(),
            organization_code: ws.code,
            app_slug: Some(app.clone()).filter(|a| !a.is_empty()),
            decision_type: if kind == Kind::User {
                DecisionType::WorkspaceMemberAdded
            } else {
                DecisionType::AccessRoleGranted
            },
            oscal_control_id: "AC-02".to_string(),
            rationale: match payload
                .reason
                .as_deref()
                .map(str::trim)
                .filter(|r| !r.is_empty())
            {
                Some(reason) => reason.to_string(),
                None => format!(
                    "Principal {} added {} as {} to {}",
                    caller.eppn,
                    key,
                    role_str,
                    if app.is_empty() {
                        format!("workspace {}", ws.name)
                    } else {
                        format!("app {app} in workspace {}", ws.name)
                    }
                ),
            },
            payload: &json!({
                "workspace_id": workspace_id,
                "target_eppn": key,
                "principal_type": kind.as_str(),
                "principal_id": principal_id,
                "role": role_str,
                "app_slug": app
            }),
        })
        .map_err(|e| ServiceError::Internal(e.to_string()))?;

    {
        let repo = &state.repository;
        repo.upsert_collaborator(&new_member)
            .map_err(|e| ServiceError::Internal(e.to_string()))?;
    }

    super::reload_after_write(state)?;

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
        ws_guard.get(workspace_id).cloned().ok_or_else(|| {
            ServiceError::NotFound(format!("Workspace '{workspace_id}' not found"))
        })?
    };

    let mut collabs = {
        let collabs_guard = state
            .collaborators
            .read()
            .map_err(|e| ServiceError::Internal(e.to_string()))?;
        collabs_guard.get(workspace_id).cloned().unwrap_or_default()
    };

    crate::service::access::authorize_workspace_denying(
        caller,
        &ws,
        "manage_workspace",
        "403 Forbidden: Cedar Policy restricts managing member roles to Owners and Admins",
        state,
    )?;

    let wanted = Role::parse(new_role).ok_or_else(|| {
        ServiceError::BadRequest(
            "role must be viewer, commenter, editor, admin, or owner".to_string(),
        )
    })?;
    if members::split_principal(target_eppn).0 != Kind::User && wanted == Role::Owner {
        return Err(ServiceError::BadRequest(
            "Only a person can be an owner".to_string(),
        ));
    }
    if wanted != Role::Owner && is_last_user_owner(&collabs, target_eppn) {
        return Err(ServiceError::Conflict(
            "A workspace keeps at least one person as owner".to_string(),
        ));
    }
    let member = collabs
        .iter_mut()
        .find(|m| m.eppn == target_eppn && m.app_slug.is_empty())
        .ok_or_else(|| ServiceError::NotFound(format!("Member '{target_eppn}' not found")))?;

    member.role = new_role.to_string();
    let updated = member.clone();

    state
        .append_ledger_entry(RecordDecisionInput {
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
        })
        .map_err(|e| ServiceError::Internal(e.to_string()))?;

    {
        let repo = &state.repository;
        repo.upsert_collaborator(&updated)
            .map_err(|e| ServiceError::Internal(e.to_string()))?;
    }

    super::reload_after_write(state)?;

    Ok(updated)
}

/// Removes one membership. `app` is empty for the workspace-wide one, or names the app a limited one reaches.
pub fn remove_collaborator(
    caller: &AuthUser,
    workspace_id: &str,
    target_eppn: &str,
    app: &str,
    state: &SharedState,
) -> Result<(), ServiceError> {
    let ws = {
        let ws_guard = state
            .workspaces
            .read()
            .map_err(|e| ServiceError::Internal(e.to_string()))?;
        ws_guard.get(workspace_id).cloned().ok_or_else(|| {
            ServiceError::NotFound(format!("Workspace '{workspace_id}' not found"))
        })?
    };

    let collabs = {
        let collabs_guard = state
            .collaborators
            .read()
            .map_err(|e| ServiceError::Internal(e.to_string()))?;
        collabs_guard.get(workspace_id).cloned().unwrap_or_default()
    };

    crate::service::access::authorize_workspace_denying(
        caller,
        &ws,
        "manage_workspace",
        "403 Forbidden: Cedar Policy restricts managing member roles to Owners and Admins",
        state,
    )?;

    if app.is_empty() && is_last_user_owner(&collabs, target_eppn) {
        return Err(ServiceError::Conflict(
            "A workspace keeps at least one person as owner".to_string(),
        ));
    }
    let mut remaining = collabs;
    let initial_len = remaining.len();
    remaining.retain(|m| !(m.eppn == target_eppn && m.app_slug == app));

    if remaining.len() == initial_len {
        return Err(ServiceError::NotFound(format!(
            "Member '{target_eppn}' not found"
        )));
    }

    state
        .append_ledger_entry(RecordDecisionInput {
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
                "removed_eppn": target_eppn,
                "app_slug": app
            }),
        })
        .map_err(|e| ServiceError::Internal(e.to_string()))?;

    {
        let repo = &state.repository;
        repo.remove_collaborator(workspace_id, target_eppn, app)
            .map_err(|e| ServiceError::Internal(e.to_string()))?;
    }

    super::reload_after_write(state)?;

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
                    principal_type: None,
                    principal_id: None,
                    role: role.map(str::to_string),
                    name: name.map(str::to_string),
                    scoped_affiliation: None,
                    department: None,
                    app_slug: None,
                    reason: None,
                },
                state,
            )?;
            Ok(serde_json::to_value(res).unwrap_or(json!({ "status": "added" })))
        }
        "update" => {
            let r = role.ok_or_else(|| {
                ServiceError::BadRequest("Role is required for update".to_string())
            })?;
            let res = update_collaborator_role(caller, workspace_id, eppn, r, state)?;
            Ok(serde_json::to_value(res).unwrap_or(json!({ "status": "updated" })))
        }
        "remove" | "delete" => {
            remove_collaborator(caller, workspace_id, eppn, "", state)?;
            Ok(json!({ "status": "removed", "eppn": eppn, "workspace_id": workspace_id }))
        }
        other => Err(ServiceError::BadRequest(format!(
            "Unsupported member action: {other}. Expected 'add', 'update', or 'remove'."
        ))),
    }
}
