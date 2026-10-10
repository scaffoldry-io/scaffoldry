use scaffoldry_engine::HostRouter;
use crate::service::organizations::{unit_in_scope, OrgCaller};
use crate::service::ServiceError;
use crate::state::{AuthUser, SharedState};
use scaffoldry_policy::{PolicyDecision, WorkspaceActionInput};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppAction {
    Read,
    WriteRecords,
    Manage,
}

pub fn authorize_app(
    caller: &AuthUser,
    slug: &str,
    action: AppAction,
    state: &SharedState,
) -> Result<(), ServiceError> {
    let app = {
        let engine = state
            .engine
            .read()
            .map_err(|e| ServiceError::Internal(e.to_string()))?;
        engine
            .resolve_by_slug(slug)
            .cloned()
            .ok_or_else(|| ServiceError::NotFound(format!("App '{slug}' not found")))?
    };

    let ws_id = match app.workspace_id {
        Some(ref id) => id.clone(),
        None => {
            // An app with no workspace_id is visible to a Platform Admin only.
            if caller.affiliation == "central_admin" {
                return Ok(());
            } else {
                return Err(ServiceError::forbidden(
                    "App has no workspace; Platform Admin access required",
                ));
            }
        }
    };

    let ws = {
        let ws_guard = state
            .workspaces
            .read()
            .map_err(|e| ServiceError::Internal(e.to_string()))?;
        ws_guard
            .get(&ws_id)
            .cloned()
            .ok_or_else(|| ServiceError::NotFound(format!("Workspace '{ws_id}' not found")))?
    };

    let collabs = {
        let collabs_guard = state
            .collaborators
            .read()
            .map_err(|e| ServiceError::Internal(e.to_string()))?;
        collabs_guard.get(&ws_id).cloned().unwrap_or_default()
    };

    let (is_member, member_role): (bool, Option<String>) =
        match collabs.iter().find(|m| m.eppn == caller.eppn) {
            Some(m) => (true, Some(m.role.clone())),
            None => (false, None),
        };

    let norm_role = member_role.as_ref().map(|r| r.to_lowercase());

    let action_name = match action {
        AppAction::Read => "read_app",
        AppAction::WriteRecords => "write_record",
        AppAction::Manage => "manage_app",
    };

    let decision = state
        .policy_engine
        .authorize_workspace_action(&WorkspaceActionInput {
            principal_eppn: &caller.eppn,
            principal_affiliation: &caller.affiliation,
            principal_department: &caller.department,
            action_name,
            workspace_id: &ws.id,
            workspace_department: &ws.department,
            workspace_visibility: &ws.visibility,
            is_member,
            member_role: norm_role.as_deref(),
        })
        .map_err(|e| ServiceError::Internal(e.to_string()))?;

    if decision.decision == PolicyDecision::Allow {
        return Ok(());
    }

    // If denied and action is Read or Manage, check unit_in_scope for Org Unit Admin
    if matches!(action, AppAction::Read | AppAction::Manage) {
        if let Some(org_id) = ws.organization_id {
            let org_caller = OrgCaller {
                eppn: caller.eppn.clone(),
                affiliation: caller.affiliation.clone(),
            };
            let (orgs, roles) = {
                let o_guard = state
                    .organizations
                    .read()
                    .map_err(|e| ServiceError::Internal(e.to_string()))?;
                let r_guard = state
                    .roles
                    .read()
                    .map_err(|e| ServiceError::Internal(e.to_string()))?;
                let org_list: Vec<_> = o_guard.values().cloned().collect();
                (org_list, r_guard.clone())
            };
            if unit_in_scope(&org_caller, org_id, &orgs, &roles) {
                return Ok(());
            }
        }
    }

    Err(ServiceError::Forbidden {
        message: "403 Forbidden: Cedar Policy restricts access to this app".to_string(),
        reasons: decision.reasons,
        diagnostics: decision.diagnostics,
        policy: decision.deciding_policy,
    })
}
