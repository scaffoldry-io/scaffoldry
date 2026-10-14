use scaffoldry_engine::HostRouter;
use crate::service::organizations::{unit_in_scope, OrgCaller};
use crate::service::ServiceError;
use crate::state::{AuthUser, SharedState};
use scaffoldry_policy::entities::{PrincipalCtx, Resource, WorkspaceCtx};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppAction {
    Read,
    WriteRecords,
    /// Add a comment. A commenter may, and may not change the record.
    Comment,
    Manage,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Decision {
    pub allowed: bool,
    pub policy: Option<scaffoldry_policy::PolicyRef>,
}

/// Unified access evaluation for all resources and actions.
/// Builds policy set from institutional policy plus workspace guards, cached by (workspace_id, version).
pub fn decide(
    state: &SharedState,
    caller: &AuthUser,
    action: &str,
    resource: &Resource,
) -> Decision {
    let ws_id = match resource {
        Resource::Workspace(w) => Some(w.workspace_id.as_str()),
        Resource::App(a) => Some(a.workspace_id.as_str()),
        Resource::Record(r) => Some(r.workspace_id.as_str()),
        Resource::System(_) => None,
    };

    let policy_set = if let Some(wid) = ws_id {
        if wid.is_empty() {
            state.policy_engine.policies().clone()
        } else {
            let current_guard = {
                state
                    .guards
                    .read()
                    .ok()
                    .and_then(|g| g.get(wid).and_then(|h| h.last().cloned()))
            };
            if let Some(guard) = current_guard {
                let cache_key = (wid.to_string(), guard.version);
                let cached = state
                    .guard_policy_cache
                    .read()
                    .ok()
                    .and_then(|c| c.get(&cache_key).cloned());
                if let Some(ps) = cached {
                    ps
                } else {
                    let mut ps = state.policy_engine.policies().clone();
                    if let Ok(rules) =
                        serde_json::from_value::<Vec<scaffoldry_core::GuardRule>>(guard.rules.clone())
                    {
                        let org_names = state
                            .organizations
                            .read()
                            .map(|orgs| {
                                orgs.iter()
                                    .map(|(id, node)| (id.to_string(), node.name.clone()))
                                    .collect()
                            })
                            .unwrap_or_default();
                        for (i, rule) in rules.iter().enumerate() {
                            if let Ok(text) = rule.compile_policy_text(wid, i + 1, &org_names) {
                                let policy_id = format!("guard-{wid}-{}", i + 1);
                                if let Ok(p) =
                                    scaffoldry_policy::parse_guard_policy(&policy_id, &text)
                                {
                                    let _ = ps.add(p);
                                }
                            }
                        }
                    } else if !guard.compiled.trim().is_empty() {
                        if let Ok(raw_set) = scaffoldry_policy::validate_raw_source(&guard.compiled)
                        {
                            for p in raw_set.policies() {
                                let _ = ps.add(p.clone());
                            }
                        }
                    }
                    if let Ok(mut c_write) = state.guard_policy_cache.write() {
                        c_write.insert(cache_key, ps.clone());
                    }
                    ps
                }
            } else {
                state.policy_engine.policies().clone()
            }
        }
    } else {
        state.policy_engine.policies().clone()
    };

    let role_unit_ids: Vec<String> = state.roles_of(&caller.eppn).iter().map(|r| r.organization_id.to_string()).collect();

    let org_parents: std::collections::HashMap<String, Option<String>> = {
        let o_read = state.organizations.read().ok();
        let mut map = std::collections::HashMap::new();
        if let Some(orgs) = o_read {
            for (id, node) in orgs.iter() {
                map.insert(id.to_string(), node.parent_id.map(|p| p.to_string()));
            }
        }
        map
    };

    let all_unit_ids = scaffoldry_policy::entities::unit_ids_for(&role_unit_ids, &org_parents);

    let is_platform_admin = caller.affiliation == "central_admin"
        ;

    let principal = PrincipalCtx {
        eppn: caller.eppn.clone(),
        name: caller.name.clone(),
        scoped_affiliation: caller.affiliation.clone(),
        department: caller.department.clone(),
        unit_ids: all_unit_ids,
        is_platform_admin,
    };

    match scaffoldry_policy::entities::authorize_with_policy_set(
        &state.policy_engine,
        &policy_set,
        &principal,
        action,
        resource,
    ) {
        Ok(res) => Decision {
            allowed: res.decision == scaffoldry_policy::PolicyDecision::Allow,
            policy: res.deciding_policy,
        },
        Err(_) => Decision {
            allowed: false,
            policy: None,
        },
    }
}

/// Checks one Cedar workspace action for the caller: `access_workspace`, `manage_workspace`.
pub fn authorize_workspace(
    caller: &AuthUser,
    ws_id: &str,
    cedar_action: &str,
    state: &SharedState,
) -> Result<(), ServiceError> {
    let ws = {
        let ws_guard = state
            .workspaces
            .read()
            .map_err(|e| ServiceError::Internal(e.to_string()))?;
        ws_guard
            .get(ws_id)
            .cloned()
            .ok_or_else(|| ServiceError::NotFound(format!("Workspace '{ws_id}' not found")))?
    };

    let role = crate::service::members::member_role(caller, ws_id, state);
    let (is_member, member_role) = (role.is_some(), role.map(|r| r.as_str().to_string()).unwrap_or_default());

    let resource = Resource::Workspace(WorkspaceCtx {
        workspace_id: ws.id.clone(),
        department: ws.department.clone(),
        visibility: ws.visibility.clone(),
        data_classification: ws.data_classification.clone(),
        member_role,
        unit_id: ws.organization_id.map(|u| u.to_string()).unwrap_or_default(),
        is_member,
    });

    let decision = decide(state, caller, cedar_action, &resource);
    if decision.allowed {
        return Ok(());
    }

    Err(ServiceError::Forbidden {
        message: format!("403 Forbidden: Cedar Policy denies {cedar_action} on this workspace"),
        reasons: vec![],
        diagnostics: vec![],
        policy: decision.policy,
    })
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

    let role = crate::service::members::member_role(caller, &ws_id, state);
    let (is_member, member_role) = (role.is_some(), role.map(|r| r.as_str().to_string()).unwrap_or_default());

    let action_name = match action {
        AppAction::Read => "read_app",
        AppAction::WriteRecords => "write_record",
        AppAction::Comment => "comment",
        AppAction::Manage => "manage_app",
    };

    let resource = Resource::Workspace(WorkspaceCtx {
        workspace_id: ws.id.clone(),
        department: ws.department.clone(),
        visibility: ws.visibility.clone(),
        data_classification: ws.data_classification.clone(),
        member_role,
        unit_id: ws.organization_id.map(|u| u.to_string()).unwrap_or_default(),
        is_member,
    });

    let decision = decide(state, caller, action_name, &resource);
    if decision.allowed {
        return Ok(());
    }

    if matches!(action, AppAction::Read | AppAction::Manage) {
        if let Some(org_id) = ws.organization_id {
            let org_caller = OrgCaller {
                eppn: caller.eppn.clone(),
                affiliation: caller.affiliation.clone(),
            };
            let orgs: Vec<_> = state
                .organizations
                .read()
                .map_err(|e| ServiceError::Internal(e.to_string()))?
                .values()
                .cloned()
                .collect();
            let roles = state.roles_of(&caller.eppn);
            if unit_in_scope(&org_caller, org_id, &orgs, &roles) {
                return Ok(());
            }
        }
    }

    Err(ServiceError::Forbidden {
        message: "403 Forbidden: Cedar Policy restricts access to this app".to_string(),
        reasons: vec![],
        diagnostics: vec![],
        policy: decision.policy,
    })
}
