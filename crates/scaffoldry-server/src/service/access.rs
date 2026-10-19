use scaffoldry_engine::HostRouter;
use crate::service::ServiceError;
use crate::state::{AuthUser, SharedState, WorkspaceRecord};
use scaffoldry_policy::entities::{PrincipalCtx, Resource, WorkspaceCtx};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

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
    #[serde(default)]
    pub reasons: Vec<String>,
    #[serde(default)]
    pub diagnostics: Vec<String>,
    /// Set when no decision could be made, for example a guard whose stored source no longer compiles. `allowed` is
    /// false. A caller turns this into an `internal` error. It is never a grant and never a plain denial.
    #[serde(default)]
    pub error: Option<String>,
}

impl Decision {
    fn failed(message: impl Into<String>) -> Self {
        Self { allowed: false, policy: None, reasons: Vec::new(), diagnostics: Vec::new(), error: Some(message.into()) }
    }
}

/// The institutional policies plus the guards of one workspace, cached by (workspace_id, version). A guard that cannot
/// be read, parsed, or compiled is an error. It is never skipped, because a skipped guard widens access.
fn policy_set_for(state: &SharedState, ws_id: Option<&str>) -> Result<scaffoldry_policy::PolicySet, String> {
    let Some(wid) = ws_id.filter(|w| !w.is_empty()) else { return Ok(state.policy_engine.policies().clone()) };
    let current_guard = state.guards.read().unwrap_or_else(|p| p.into_inner()).get(wid).and_then(|h| h.last().cloned());
    let Some(guard) = current_guard else { return Ok(state.policy_engine.policies().clone()) };

    let cache_key = (wid.to_string(), guard.version);
    if let Some(cached) = state.guard_policy_cache.read().unwrap_or_else(|p| p.into_inner()).get(&cache_key).cloned() {
        return Ok(cached);
    }

    let mut ps = state.policy_engine.policies().clone();
    match serde_json::from_value::<Vec<scaffoldry_core::GuardRule>>(guard.rules.clone()) {
        Ok(rules) => {
            let org_names = state
                .organizations
                .read()
                .unwrap_or_else(|p| p.into_inner())
                .iter()
                .map(|(id, node)| (id.to_string(), node.name.clone()))
                .collect();
            for (i, rule) in rules.iter().enumerate() {
                let text = rule.compile_policy_text(wid, i + 1, &org_names).map_err(|e| format!("guard {} of workspace '{wid}' does not compile: {e}", i + 1))?;
                let policy_id = format!("guard-{wid}-{}", i + 1);
                let policy = scaffoldry_policy::parse_guard_policy(&policy_id, &text).map_err(|e| format!("guard {} of workspace '{wid}' is not a valid policy: {e}", i + 1))?;
                ps.add(policy).map_err(|e| format!("guard {} of workspace '{wid}' cannot be added: {e}", i + 1))?;
            }
        }
        Err(_) if !guard.compiled.trim().is_empty() => {
            let raw_set = scaffoldry_policy::validate_raw_source(&guard.compiled).map_err(|e| format!("the guard source of workspace '{wid}' is not valid: {e}"))?;
            for p in raw_set.policies() {
                ps.add(p.clone()).map_err(|e| format!("the guard source of workspace '{wid}' cannot be added: {e}"))?;
            }
        }
        // A guard version with no rules at all holds nothing to enforce.
        Err(_) if guard.rules.is_null() => {}
        Err(e) => return Err(format!("the guard rules of workspace '{wid}' cannot be read: {e}")),
    }
    state.guard_policy_cache.write().unwrap_or_else(|p| p.into_inner()).insert(cache_key, ps.clone());
    Ok(ps)
}

/// A person's units: every unit where they hold a role, and every ancestor of each. This is the one function. Cedar
/// (`principal_of`) and the row and column rules (`scope::person_of`) both call it, so a person in Physics is in the
/// College of Science for a guard, for a rule in Rust, and for a rule in SQL.
pub fn unit_ids_of(state: &SharedState, roles: &[crate::state::RoleRow]) -> Vec<String> {
    let role_unit_ids: Vec<String> = roles.iter().map(|r| r.organization_id.to_string()).collect();
    scaffoldry_policy::entities::unit_ids_for(&role_unit_ids, &unit_parents(state))
}

/// What Cedar knows about a person: their units with all ancestors, and the units they administer.
pub fn principal_of(state: &SharedState, caller: &AuthUser) -> PrincipalCtx {
    let roles = state.roles_of(&caller.eppn);
    let admin_unit_ids: Vec<String> = roles.iter().filter(|r| r.scoped_affiliation == "unit_admin").map(|r| r.organization_id.to_string()).collect();
    // A Platform Admin is a central administrator, or holds the `platform_admin` role on the root unit.
    let is_platform_admin = {
        let orgs: Vec<crate::state::OrganizationNode> = state.organizations.read().unwrap_or_else(|p| p.into_inner()).values().cloned().collect();
        let who = crate::service::organizations::OrgCaller { eppn: caller.eppn.clone(), affiliation: caller.affiliation.clone() };
        crate::service::organizations::is_platform_admin(&who, &orgs, &roles)
    };
    PrincipalCtx {
        eppn: caller.eppn.clone(),
        name: caller.name.clone(),
        scoped_affiliation: if is_platform_admin { "central_admin".to_string() } else { caller.affiliation.clone() },
        department: caller.department.clone(),
        unit_ids: unit_ids_of(state, &roles),
        admin_unit_ids,
        is_platform_admin,
    }
}

/// Each unit's parent, by id.
fn unit_parents(state: &SharedState) -> std::collections::HashMap<String, Option<String>> {
    state.organizations.read().unwrap_or_else(|p| p.into_inner()).iter().map(|(id, node)| (id.to_string(), node.parent_id.map(|p| p.to_string()))).collect()
}

/// Unified access evaluation for all resources and actions.
/// Builds policy set from institutional policy plus workspace guards, cached by (workspace_id, version).
/// When no decision can be made the result carries `error` and is not allowed.
pub fn decide(
    state: &SharedState,
    caller: &AuthUser,
    action: &str,
    resource: &Resource,
) -> Decision {
    decide_for(state, &principal_of(state, caller), action, resource)
}

/// The same evaluation for a principal that is already built. The policy simulator uses it for a person who may not
/// exist. It is the only place a decision is made, so a guard failure or an engine error reads the same everywhere.
pub fn decide_for(
    state: &SharedState,
    principal: &PrincipalCtx,
    action: &str,
    resource: &Resource,
) -> Decision {
    let ws_id = match resource {
        Resource::Workspace(w) => Some(w.workspace_id.as_str()),
        Resource::App(a) => Some(a.workspace_id.as_str()),
        Resource::Record(r) => Some(r.workspace_id.as_str()),
        Resource::System(_) => None,
    };
    let policy_set = match policy_set_for(state, ws_id) {
        Ok(ps) => ps,
        Err(e) => return Decision::failed(e),
    };
    match scaffoldry_policy::entities::authorize_with_policy_set(&state.policy_engine, &policy_set, principal, action, resource) {
        Ok(res) => Decision {
            allowed: res.decision == scaffoldry_policy::PolicyDecision::Allow,
            policy: res.deciding_policy,
            reasons: res.reasons,
            diagnostics: res.diagnostics,
            error: None,
        },
        Err(e) => Decision::failed(format!("the policy engine could not decide {action}: {e}")),
    }
}

/// What the policy simulator asks: could a person with this affiliation and department take `action` on a record of
/// this app and department? The person need not exist, so no role or unit is added. Guards are not applied.
#[allow(clippy::too_many_arguments)]
pub fn simulate_record_action(
    state: &SharedState,
    eppn: &str,
    affiliation: &str,
    principal_department: &str,
    action: &str,
    app_slug: &str,
    resource_department: &str,
    is_ferpa_sensitive: bool,
) -> Result<Decision, ServiceError> {
    let principal = PrincipalCtx {
        eppn: eppn.to_string(),
        name: eppn.to_string(),
        scoped_affiliation: affiliation.to_string(),
        department: principal_department.to_string(),
        unit_ids: Vec::new(),
        admin_unit_ids: Vec::new(),
        is_platform_admin: false,
    };
    let resource = Resource::Record(scaffoldry_policy::entities::RecordCtx {
        app_slug: app_slug.to_string(),
        department: resource_department.to_string(),
        workspace_id: String::new(),
        is_ferpa_sensitive,
        categories: Default::default(),
    });
    let decision = decide_for(state, &principal, action, &resource);
    match decision.error {
        Some(e) => Err(ServiceError::Internal(e)),
        None => Ok(decision),
    }
}

/// The governance ledger as Cedar sees it: a system resource of the central administration.
pub fn ledger_resource() -> scaffoldry_policy::entities::SystemCtx {
    scaffoldry_policy::entities::SystemCtx { id: "governance-ledger".to_string(), department: "central_admin".to_string(), is_ferpa_sensitive: false }
}

/// Decides one action and turns the result into `Ok` or an error. A denial is `Forbidden` with `denied` as its message
/// and the reasons the engine gave. A decision that could not be made is `Internal`.
pub fn require(
    state: &SharedState,
    caller: &AuthUser,
    action: &str,
    resource: &Resource,
    denied: &str,
) -> Result<(), ServiceError> {
    let decision = decide(state, caller, action, resource);
    if let Some(e) = decision.error {
        return Err(ServiceError::Internal(e));
    }
    if decision.allowed {
        return Ok(());
    }
    Err(ServiceError::Forbidden { message: denied.to_string(), reasons: decision.reasons, diagnostics: decision.diagnostics, policy: decision.policy })
}

/// The Cedar entity for a workspace, for someone who holds `role` in it (`None` for a non-member). It carries the
/// workspace's unit and every ancestor of it. `sensitivity` is set when the question is about one app in the
/// workspace: whether its fields hold protected data, and the categories of those fields.
pub fn workspace_ctx(state: &SharedState, ws: &WorkspaceRecord, role: Option<&str>, sensitivity: Option<(bool, BTreeSet<String>)>) -> WorkspaceCtx {
    let unit_id = ws.organization_id.map(|u| u.to_string()).unwrap_or_default();
    let unit_path_ids = match ws.organization_id {
        Some(_) => scaffoldry_policy::entities::unit_ids_for(std::slice::from_ref(&unit_id), &unit_parents(state)),
        None => Vec::new(),
    };
    let (is_ferpa_sensitive, categories) = sensitivity.unwrap_or_default();
    WorkspaceCtx {
        workspace_id: ws.id.clone(),
        department: ws.department.clone(),
        visibility: ws.visibility.clone(),
        data_classification: ws.data_classification.clone(),
        member_role: role.unwrap_or_default().to_string(),
        unit_id,
        unit_path_ids,
        is_member: role.is_some(),
        is_ferpa_sensitive,
        categories,
    }
}

/// The Cedar resource for a workspace as `caller` meets it. `app_slug` narrows their membership to that app.
pub fn workspace_resource(state: &SharedState, caller: &AuthUser, ws: &WorkspaceRecord, app_slug: Option<&str>, sensitivity: Option<(bool, BTreeSet<String>)>) -> Resource {
    let role = match app_slug {
        Some(slug) => crate::service::members::member_role_in_app(caller, &ws.id, slug, state),
        None => crate::service::members::member_role(caller, &ws.id, state),
    };
    Resource::Workspace(workspace_ctx(state, ws, role.as_ref().map(|r| r.as_str()), sensitivity))
}

/// Whether any field of the app is protected under the effective rule, and the categories its fields carry.
fn app_sensitivity(app: &scaffoldry_engine::AppManifest, state: &SharedState) -> (bool, BTreeSet<String>) {
    let categories = state.sensitivity_categories();
    let labels = state.labels.read().unwrap_or_else(|p| p.into_inner());
    let mut protected = false;
    let mut carried = BTreeSet::new();
    for table in &app.tables {
        for f in &table.fields {
            let label = labels.get(&(app.slug.clone(), table.id.clone(), f.name.clone()));
            let s = scaffoldry_engine::effective_ferpa_sensitive(f, label, &categories);
            protected |= s.protected;
            carried.extend(s.categories);
        }
    }
    (protected, carried)
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
    authorize_workspace_record(caller, &ws, cedar_action, state)
}

/// The same check for a workspace the caller already holds.
pub fn authorize_workspace_record(caller: &AuthUser, ws: &WorkspaceRecord, cedar_action: &str, state: &SharedState) -> Result<(), ServiceError> {
    authorize_workspace_denying(caller, ws, cedar_action, &format!("403 Forbidden: Cedar Policy denies {cedar_action} on this workspace"), state)
}

/// The same check with the words a refusal uses.
pub fn authorize_workspace_denying(caller: &AuthUser, ws: &WorkspaceRecord, cedar_action: &str, denied: &str, state: &SharedState) -> Result<(), ServiceError> {
    let resource = workspace_resource(state, caller, ws, None, None);
    require(state, caller, cedar_action, &resource, denied)
}

/// Whether the caller may take `cedar_action` on a workspace, for a list that filters. A decision that cannot be made
/// is an error, so a list never drops a workspace without saying why.
pub fn workspace_allowed(caller: &AuthUser, ws: &WorkspaceRecord, cedar_action: &str, state: &SharedState) -> Result<bool, ServiceError> {
    let resource = workspace_resource(state, caller, ws, None, None);
    let decision = decide(state, caller, cedar_action, &resource);
    match decision.error {
        Some(e) => Err(ServiceError::Internal(e)),
        None => Ok(decision.allowed),
    }
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

    let action_name = match action {
        AppAction::Read => "read_app",
        AppAction::WriteRecords => "write_record",
        AppAction::Comment => "comment",
        AppAction::Manage => "manage_app",
    };

    let resource = workspace_resource(state, caller, &ws, Some(slug), Some(app_sensitivity(&app, state)));
    // A forbid from a guard is final. A unit administrator is let in by a Cedar permit, never by a fallback after a deny.
    require(state, caller, action_name, &resource, "403 Forbidden: Cedar Policy restricts access to this app")
}
