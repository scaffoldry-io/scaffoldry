//! Access model phase 7: explaining access, and reviewing it by person and by object. What a grant reaches is decided by
//! `scaffoldry_core::reach`. This file asks that function the questions and says the answers in words an administrator
//! can read. It writes nothing.

use crate::service::access::{decide_for, principal_of, workspace_ctx};
use crate::service::coordinates::{cedar_place, object_for, with_world};
use crate::service::members::{self, Role};
use crate::service::ServiceError;
use crate::state::{AuthUser, SharedState, WorkspaceRecord};
use scaffoldry_core::reach::{role_at, trace, AnchorKind, Grant, HolderSet, Miss, Object};
use scaffoldry_engine::AppManifest;
use serde_json::{json, Value};
use uuid::Uuid;

fn bad(msg: impl Into<String>) -> ServiceError {
    ServiceError::BadRequest(msg.into())
}

/// A workspace, an app in it, or a table of that app: the three things a grant can reach.
pub struct Target {
    /// The container id, as a grant anchors on it: `workspace:<id>`, `app:<slug>`, `table:<slug>/<table id>`.
    pub container: String,
    pub workspace: WorkspaceRecord,
    pub app: Option<AppManifest>,
    pub table: Option<String>,
}

impl Target {
    pub fn object(&self, state: &SharedState) -> Option<Object> {
        with_world(state, |places, tree| {
            let app = self.app.as_ref().map(|m| (m.slug.as_str(), Some(m)));
            object_for(places, tree, &self.workspace, app, self.table.as_deref())
        })
        .ok()
    }
}

fn manifest_of(state: &SharedState, slug: &str) -> Option<AppManifest> {
    use scaffoldry_engine::HostRouter;
    state.engine.read().unwrap_or_else(|p| p.into_inner()).resolve_by_slug(crate::service::preview::stored_slug(slug)).cloned()
}

fn workspace_named(state: &SharedState, id: &str) -> Option<WorkspaceRecord> {
    state.workspaces.read().unwrap_or_else(|p| p.into_inner()).get(id).cloned()
}

/// Reads `workspace:<id>`, `app:<slug>` or `table:<slug>/<table id>`.
pub fn parse_target(state: &SharedState, text: &str) -> Result<Target, ServiceError> {
    let not_found = || ServiceError::NotFound(format!("'{text}' is not a workspace, app or table here"));
    let hint = || bad("object must be workspace:<id>, app:<slug> or table:<slug>/<table id>");
    let (kind, rest) = text.split_once(':').ok_or_else(hint)?;
    match kind {
        "workspace" => {
            let workspace = workspace_named(state, rest).ok_or_else(not_found)?;
            Ok(Target { container: text.to_string(), workspace, app: None, table: None })
        }
        "app" | "table" => {
            let (slug, table) = match (kind, rest.split_once('/')) {
                ("app", None) => (rest, None),
                ("table", Some((slug, table))) if !table.is_empty() => (slug, Some(table.to_string())),
                _ => return Err(hint()),
            };
            let manifest = manifest_of(state, slug).ok_or_else(not_found)?;
            if table.as_deref().is_some_and(|t| !manifest.tables.iter().any(|x| x.id == t)) {
                return Err(not_found());
            }
            let workspace = manifest.workspace_id.as_deref().and_then(|w| workspace_named(state, w)).ok_or_else(not_found)?;
            Ok(Target { container: text.to_string(), workspace, app: Some(manifest), table })
        }
        _ => Err(hint()),
    }
}

/// Every workspace, app and table, for a review that asks about all of them.
pub fn all_targets(state: &SharedState) -> Vec<Target> {
    let workspaces: Vec<WorkspaceRecord> = state.workspaces.read().unwrap_or_else(|p| p.into_inner()).values().cloned().collect();
    let manifests = state.engine.read().unwrap_or_else(|p| p.into_inner()).list_manifests();
    let mut out = Vec::new();
    for ws in workspaces {
        out.push(Target { container: format!("workspace:{}", ws.id), workspace: ws.clone(), app: None, table: None });
        for m in manifests.iter().filter(|m| m.workspace_id.as_deref() == Some(ws.id.as_str())) {
            out.push(Target { container: format!("app:{}", m.slug), workspace: ws.clone(), app: Some(m.clone()), table: None });
            for t in &m.tables {
                out.push(Target { container: crate::service::coordinates::table_container(&m.slug, &t.id), workspace: ws.clone(), app: Some(m.clone()), table: Some(t.id.clone()) });
            }
        }
    }
    out
}

/// One grant in words: who holds it and where it is anchored.
fn grant_json(g: &Grant) -> Value {
    let anchor = match g.anchor_kind {
        AnchorKind::Container => g.anchor_id.clone().unwrap_or_default(),
        AnchorKind::Unit => format!("unit:{}", g.anchor_id.clone().unwrap_or_default()),
        AnchorKind::HolderUnit => "holder_unit".to_string(),
    };
    // A membership or an appointment is a grant that is not stored. It has no id to give.
    let (id, source) = match g.id {
        i if i > 0 => (json!(i), "stored"),
        i if i <= -1_000_000 => (Value::Null, "appointment"),
        _ => (Value::Null, "membership"),
    };
    json!({
        "grant_id": id, "source": source, "holder": format!("{}:{}", g.holder_kind.as_str(), g.holder_id),
        "role": g.role.as_str(), "anchor": anchor, "depth": g.depth, "domain": g.domain,
    })
}

/// Everything one person holds that could reach an object in this workspace: their holder set, and every grant they hold,
/// including memberships of the workspace.
pub fn grants_of(state: &SharedState, eppn: &str, workspace: &str) -> Option<(HolderSet, Vec<Grant>)> {
    let (holders, mut grants) = members::standing(eppn, state)?;
    grants.extend(members::membership_grants(workspace, state).into_iter().filter(|g| holders.is_holder(g)));
    Some((holders, grants))
}

/// Who may ask about a person's access to an object: the person, a Platform Admin or compliance, or an Org Unit Admin whose
/// subtree holds a unit the object sits at.
fn may_explain(state: &SharedState, caller: &AuthUser, person: &str, object: &Object) -> bool {
    if caller.eppn.eq_ignore_ascii_case(person) || crate::service::admin::require_platform_admin_or_compliance(caller, state).is_ok() {
        return true;
    }
    let admin_units: Vec<Uuid> = state.roles_of(&caller.eppn).iter().filter(|r| r.scoped_affiliation == "unit_admin").map(|r| r.organization_id).collect();
    let tree = state.tree.read().unwrap_or_else(|p| p.into_inner());
    admin_units.iter().any(|a| object.units.values().any(|u| tree.contains(*a, *u)))
}

/// The person as the policy engine knows them, with a plain member standing in for an address the directory lacks.
fn user_named(state: &SharedState, eppn: &str, department: &str) -> AuthUser {
    crate::service::identity::resolve_user(eppn, state).unwrap_or_else(|| AuthUser {
        eppn: eppn.to_string(),
        name: eppn.to_string(),
        role_title: "Member".to_string(),
        affiliation: "member".to_string(),
        department: department.to_string(),
        view_as_by: None,
        via_client: None,
    })
}

/// What the workspace's guards and the app's rules say about this person on this object.
fn narrowing(state: &SharedState, target: &Target, person: &AuthUser, role: Option<Role>) -> (Value, Vec<Value>, Vec<String>) {
    let ws = &target.workspace;
    let mut ctx = workspace_ctx(state, ws, role.map(|r| r.as_str()), target.app.as_ref().map(|_| Default::default()));
    if let Some(m) = &target.app {
        (ctx.domain_path, ctx.sensitivity_rank) = cedar_place(state, ws, Some(m));
    }
    let action = if target.app.is_some() { "read_app" } else { "access_workspace" };
    let decision = decide_for(state, &principal_of(state, person), action, &scaffoldry_policy::entities::Resource::Workspace(ctx));
    let policy = json!({
        "action": action, "allowed": decision.allowed, "deciding": decision.policy, "reasons": decision.reasons, "error": decision.error,
    });
    let guards: Vec<Value> = state
        .guards
        .read()
        .unwrap_or_else(|p| p.into_inner())
        .get(&ws.id)
        .and_then(|versions| versions.iter().max_by_key(|g| g.version))
        .and_then(|g| g.rules.as_array().cloned())
        .unwrap_or_default();
    let rules: Vec<String> = match (&target.app, &target.table) {
        (Some(m), Some(table)) => {
            let mine = role.map(|r| r.as_str());
            m.access
                .rows
                .iter()
                .filter(|r| &r.table_id == table && r.ops.contains(&scaffoldry_core::access::RowOp::Read))
                .filter(|r| !mine.is_some_and(|me| r.except_roles.iter().any(|x| x.eq_ignore_ascii_case(me))))
                .map(|r| r.id.clone())
                .collect()
        }
        _ => Vec::new(),
    };
    (policy, guards, rules)
}

/// Why a person has the role they have on an object, or none: each grant that reached it with its path, each grant that did
/// not with the first axis that failed, any sealed container that stopped one, and what the policy engine, the guards and
/// the rules add.
pub fn explain(state: &SharedState, caller: &AuthUser, person: &str, object_text: &str) -> Result<Value, ServiceError> {
    let person = person.trim().to_lowercase();
    if person.is_empty() {
        return Err(bad("person is required"));
    }
    let target = parse_target(state, object_text)?;
    let object = target.object(state).ok_or_else(|| ServiceError::Internal("The unit tree could not be read".to_string()))?;
    if !may_explain(state, caller, &person, &object) {
        return Err(ServiceError::forbidden("You may explain your own access. Another person's needs a Platform Admin, compliance, or an Org Unit Admin of the unit the object sits in"));
    }
    let (holders, grants) = grants_of(state, &person, &target.workspace.id).ok_or_else(|| ServiceError::Internal("The person's grants could not be read".to_string()))?;
    let now = chrono::Utc::now().timestamp();
    let (traces, role) = with_world(state, |places, tree| {
        let traces: Vec<(Grant, scaffoldry_core::reach::Trace)> = grants.iter().map(|g| (g.clone(), trace(g, &holders, &object, tree, places, now))).collect();
        (traces, role_at(&holders, &grants, &object, tree, places, now).map(|(r, _)| r))
    })?;
    let mut reached = Vec::new();
    let mut missed = Vec::new();
    let mut sealed_by: Vec<String> = Vec::new();
    for (g, t) in &traces {
        let mut j = grant_json(g);
        match &t.missed {
            None => {
                j["path"] = json!(t.path);
                reached.push(j);
            }
            Some(m) => {
                j["axis"] = json!(m.axis());
                if let Miss::Sealed(container) = m {
                    j["sealed_by"] = json!(container);
                    if !sealed_by.contains(container) {
                        sealed_by.push(container.clone());
                    }
                }
                missed.push(j);
            }
        }
    }
    let user = user_named(state, &person, &target.workspace.department);
    let (policy, guards, rules) = narrowing(state, &target, &user, role);
    Ok(json!({
        "person": person, "object": object_text, "role": role.map(|r| r.as_str()),
        "platform_admin": principal_of(state, &user).is_platform_admin,
        "reached": reached, "missed": missed, "sealed_by": sealed_by,
        "policy": policy, "guards": guards, "rules": rules,
    }))
}
