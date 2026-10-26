//! Access model phase 14: AI tools draft, people decide.
//!
//! A draft is a pending `access_changes` row that applies nothing. It says who it was drafted for, which tool made it and
//! the task, and it ends seven days after it is stored. A change that widens access is confirmed by someone other than the
//! person it was drafted for. Suggestions are queries over what is stored, never a model's guess.

use crate::repository_grants::GrantFilter;
use crate::service::access_grants::{build_grant_change, build_revoke_change, Built};
use crate::service::history::Origin;
use crate::service::members::{self, Role};
use crate::service::ServiceError;
use crate::state::{AuthUser, SharedState};
use scaffoldry_core::reach::Grant;
use serde_json::{json, Value};
use uuid::Uuid;

fn internal(e: impl ToString) -> ServiceError {
    ServiceError::Internal(e.to_string())
}

/// The kinds of change an AI tool may draft here.
const KINDS: &[&str] = &["grant", "revoke", "request_rule", "hierarchy"];

/// What one draft is made of: the change, its checks, and who it affects.
struct Drafted {
    built: Built,
    kind: &'static str,
}

fn merge(change: &Value, reason: &str) -> Value {
    let mut body = change.clone();
    if let Value::Object(m) = &mut body {
        m.insert("reason".to_string(), json!(reason));
    }
    body
}

fn shape(state: &SharedState, caller: &AuthUser, kind: &str, change: &Value, reason: &str) -> Result<Drafted, ServiceError> {
    match kind {
        "grant" => Ok(Drafted { built: build_grant_change(state, caller, &merge(change, reason))?, kind: "grant" }),
        "revoke" => {
            let id = change.get("grant_id").and_then(Value::as_i64).ok_or_else(|| ServiceError::bad_request("change.grant_id is required to draft a revocation"))?;
            Ok(Drafted { built: build_revoke_change(state, caller, id, reason)?, kind: "revoke" })
        }
        "request_rule" => {
            let (change, reason, sentence) = crate::service::access_requests::build_rule_change(state, caller, &merge(change, reason))?;
            Ok(Drafted { built: Built { change, widens: true, reason, sentence, grant: None }, kind: "request_rule" })
        }
        "hierarchy" => {
            let (change, reason, sentence) = crate::service::hierarchies::build(state, caller, &merge(change, reason))?;
            Ok(Drafted { built: Built { change, widens: false, reason, sentence, grant: None }, kind: "hierarchy" })
        }
        "coordinates" | "seal" | "mask" => Err(ServiceError::bad_request(format!(
            "A {kind} change is part of an app or a workspace's coordinates. Propose it with propose_app_change, which waits for a person the same way. draft_access_change drafts: {}",
            KINDS.join(", ")
        ))),
        other => Err(ServiceError::bad_request(format!("kind '{other}' is not one of {}", KINDS.join(", ")))),
    }
}

/// The checks a draft passed, as the console shows them.
fn checks(d: &Drafted) -> Value {
    json!([
        { "name": "rights", "outcome": "pass", "messages": ["You may propose this change."] },
        { "name": "widens", "outcome": if d.built.widens { "warn" } else { "pass" }, "messages": [if d.built.widens { "It gives access, so a different person must confirm it." } else { "It only narrows access." }] },
    ])
}

/// Drafts a change as `caller`, or with `store` false, says what drafting it would do. A draft applies nothing.
pub fn draft(state: &SharedState, caller: &AuthUser, origin: &Origin, body: &Value, store: bool) -> Result<Value, ServiceError> {
    // Before anything in the body is read: a person with no right to change access at all is refused, whatever they send.
    let rights = crate::service::access_grants::rights(state, caller);
    if !rights.platform && rights.admin_units.is_empty() {
        let ids = members::workspace_ids(state);
        if !members::member_roles(&caller.eppn, &ids, state).into_iter().flatten().any(|r| r >= Role::Admin) {
            return Err(ServiceError::GrantOutsideScope("You may not give access here".to_string()));
        }
    }
    let kind = body.get("kind").and_then(Value::as_str).ok_or_else(|| ServiceError::bad_request("Argument 'kind' is required"))?;
    let change = body.get("change").cloned().unwrap_or_else(|| json!({}));
    let reason = body.get("reason").and_then(Value::as_str).map(str::trim).filter(|r| !r.is_empty() && r.len() <= 500).ok_or_else(|| ServiceError::bad_request("Argument 'reason' is required and may be at most 500 characters"))?;
    let d = shape(state, caller, kind, &change, reason)?;
    let impact = match (d.kind, &d.built.grant) {
        ("grant", Some(g)) => crate::service::access_review::impact_of_new_grant(state, g)?,
        ("revoke", Some(g)) => crate::service::access_review::impact_of_revoke(state, g)?,
        _ => json!({ "kind": d.kind, "gain": [], "lose": [], "per_role": {}, "note": "This change has no per-person effect to count before it is confirmed." }),
    };
    let (id, expires) = if store {
        let (client, task) = match origin.session {
            Some(session) => state.repository.session_client_and_task(session).map_err(internal)?.map_or((None, None), |(c, t)| (Some(c), t)),
            None => (None, None),
        };
        // The sentence is stored with the change, so the console reads out what was drafted.
        let mut stored = d.built.change.clone();
        stored["sentence"] = json!(d.built.sentence);
        let (id, expires) = state.repository.propose_access_draft(&stored, d.built.widens, &d.built.reason, &caller.eppn, client.as_deref(), task.as_deref()).map_err(internal)?;
        (Some(id), Some(expires))
    } else {
        (None, None)
    };
    Ok(json!({
        "status": if store { "Pending" } else { "Preview" },
        "change_id": id, "kind": d.kind, "widens": d.built.widens, "reason": d.built.reason,
        "sentences": [d.built.sentence], "checks": checks(&d), "impact": impact, "expires_at": expires,
        "message": if store { "Nothing has changed. A person confirms this in the console." } else { "Nothing was stored." },
    }))
}

// --- Suggestions ------------------------------------------------------------------------------------------------

/// How long a grant may go unused before it is suggested for removal.
const UNUSED_DAYS: i64 = 180;

struct Scope {
    all: bool,
    units: Vec<Uuid>,
}

impl Scope {
    fn of(state: &SharedState, caller: &AuthUser) -> Result<Scope, ServiceError> {
        let r = crate::service::access_grants::rights(state, caller);
        if !r.platform && r.admin_units.is_empty() {
            return Err(ServiceError::forbidden("Platform Admin, compliance, or Org Unit Admin required"));
        }
        Ok(Scope { all: r.platform, units: r.admin_units })
    }

    fn holds(&self, state: &SharedState, unit: Option<Uuid>) -> bool {
        self.all || unit.is_some_and(|u| self.units.iter().any(|a| state.tree.read().unwrap_or_else(|p| p.into_inner()).contains(*a, u)))
    }
}

fn workspace_unit(state: &SharedState, id: &str) -> Option<Uuid> {
    state.workspaces.read().unwrap_or_else(|p| p.into_inner()).get(id).and_then(|w| w.organization_id)
}

/// Memberships that stored grants and appointments already give the same role or more, so the membership adds nothing.
fn covered_memberships(state: &SharedState, scope: &Scope, out: &mut Vec<Value>) {
    use crate::service::access_review::Target;
    let now = chrono::Utc::now().timestamp();
    for m in members::direct_memberships(state) {
        if !scope.holds(state, workspace_unit(state, &m.workspace_id)) {
            continue;
        }
        let Some(role) = Role::parse(&m.role) else { continue };
        let Some(ws) = state.workspaces.read().unwrap_or_else(|p| p.into_inner()).get(&m.workspace_id).cloned() else { continue };
        let (container, app) = if m.app_slug.is_empty() {
            (format!("workspace:{}", ws.id), None)
        } else {
            let manifest = {
                use scaffoldry_engine::HostRouter;
                state.engine.read().unwrap_or_else(|p| p.into_inner()).resolve_by_slug(&m.app_slug).cloned()
            };
            (format!("app:{}", m.app_slug), manifest)
        };
        let target = Target { container: container.clone(), workspace: ws, app, table: None };
        let Some(object) = target.object(state) else { continue };
        let Some((holders, grants)) = members::standing(&m.eppn, state) else { continue };
        let others = crate::service::coordinates::with_world(state, |places, tree| scaffoldry_core::reach::role_at(&holders, &grants, &object, tree, places, now).map(|(r, _)| r)).ok().flatten();
        if others.is_some_and(|r| r >= role) {
            out.push(json!({
                "kind": "membership_covered", "person": m.eppn, "workspace_id": m.workspace_id, "container": container, "role": role.as_str(),
                "sentence": format!("{} is a {} of {} and already has at least that through a grant or an appointment. The membership can go.", m.eppn, role.as_str(), container),
            }));
        }
    }
}

/// People who hold one position at one unit and are members, with one role, of every workspace in that unit's subtree. One
/// `holder_unit` grant gives them what the memberships do, and no more.
fn holder_unit_groups(state: &SharedState, scope: &Scope, out: &mut Vec<Value>) {
    let memberships: Vec<_> = members::direct_memberships(state).into_iter().filter(|m| m.app_slug.is_empty()).collect();
    let workspaces: Vec<(String, Option<Uuid>)> = state.workspaces.read().unwrap_or_else(|p| p.into_inner()).values().map(|w| (w.id.clone(), w.organization_id)).collect();
    let mut seen: std::collections::BTreeSet<(String, Uuid, String)> = Default::default();
    for m in &memberships {
        let Some(role) = Role::parse(&m.role) else { continue };
        for held in state.roles_of(&m.eppn).into_iter().filter_map(|r| r.position_key.clone().map(|k| (k, r.organization_id))) {
            let (key, unit) = held;
            if !seen.insert((key.clone(), unit, role.as_str().to_string())) || !scope.holds(state, Some(unit)) {
                continue;
            }
            let holders: Vec<String> = state.repository.appointed_to_position(&key, Some(unit)).unwrap_or_default();
            if holders.is_empty() {
                continue;
            }
            let inside: Vec<&String> = {
                let tree = state.tree.read().unwrap_or_else(|p| p.into_inner());
                workspaces.iter().filter(|(_, u)| u.is_some_and(|u| tree.contains(unit, u))).map(|(id, _)| id).collect()
            };
            if inside.is_empty() {
                continue;
            }
            // Every holder is a member of every workspace inside, with this role. Then the grant adds nothing.
            let mut covered = Vec::new();
            let mut complete = true;
            for h in &holders {
                for ws in &inside {
                    match memberships.iter().find(|x| x.eppn.eq_ignore_ascii_case(h) && &&x.workspace_id == ws) {
                        Some(x) if Role::parse(&x.role) == Some(role) => covered.push(json!({ "person": x.eppn, "workspace_id": x.workspace_id })),
                        _ => complete = false,
                    }
                }
            }
            if !complete || covered.len() < 2 {
                continue;
            }
            let hierarchy = state.tree.read().unwrap_or_else(|p| p.into_inner()).hierarchy_of(unit).unwrap_or("academic").to_string();
            let unit_name = state.organizations.read().unwrap_or_else(|p| p.into_inner()).get(&unit).map(|o| o.name.clone()).unwrap_or_default();
            out.push(json!({
                "kind": "holder_unit_grant", "position": key, "unit": unit.to_string(), "role": role.as_str(), "memberships": covered,
                "grant": { "holder_kind": "position", "holder_id": format!("{key}@{unit}"), "role": role.as_str(), "hierarchy": hierarchy, "anchor_kind": "holder_unit", "source": "task", "reason": "One grant for the position replaces its holders' memberships" },
                "sentence": format!("{} memberships held by the {key} of {unit_name} can be replaced by one {} grant for the position.", covered.len(), role.as_str()),
            }));
        }
    }
}

fn person_of(g: &Grant) -> String {
    g.holder_id.clone()
}

fn grants_past_their_end(state: &SharedState, scope: &Scope, out: &mut Vec<Value>) {
    let rows = state.repository.list_grant_rows(&GrantFilter { holder: None, anchor: None, hierarchy: None, source: None, state: "expired".to_string(), after: 0, limit: 500 }).unwrap_or_default();
    for r in rows {
        let unit = r.grant.anchor_id.as_deref().and_then(|a| a.strip_prefix("workspace:").and_then(|w| workspace_unit(state, w)).or_else(|| Uuid::parse_str(a).ok()));
        if !scope.all && !scope.holds(state, unit) {
            continue;
        }
        out.push(json!({
            "kind": "grant_expired", "grant_id": r.grant.id, "person": person_of(&r.grant), "role": r.grant.role.as_str(), "valid_until": r.valid_until,
            "sentence": format!("The grant of {} to {} ended on {} and was never revoked. Revoke it so the record is clean.", r.grant.role.as_str(), person_of(&r.grant), r.valid_until.as_deref().unwrap_or("")),
        }));
    }
}

fn grants_nothing_used(state: &SharedState, scope: &Scope, out: &mut Vec<Value>) {
    // The claim needs receipts. With none stored, nothing can be said about use.
    for id in state.repository.unused_grant_ids(UNUSED_DAYS).unwrap_or_default() {
        let Ok(Some(r)) = state.repository.grant_row(id) else { continue };
        let unit = r.grant.anchor_id.as_deref().and_then(|a| a.strip_prefix("workspace:").and_then(|w| workspace_unit(state, w)).or_else(|| Uuid::parse_str(a).ok()));
        if !scope.all && !scope.holds(state, unit) {
            continue;
        }
        out.push(json!({
            "kind": "grant_unused", "grant_id": id, "person": person_of(&r.grant), "role": r.grant.role.as_str(),
            "sentence": format!("The {} grant to {} has reached nothing in {UNUSED_DAYS} days. Consider revoking it.", r.grant.role.as_str(), person_of(&r.grant)),
        }));
    }
}

fn unmasked_fields(state: &SharedState, scope: &Scope, out: &mut Vec<Value>) {
    let categories = state.sensitivity_categories();
    let manifests = state.engine.read().unwrap_or_else(|p| p.into_inner()).list_manifests();
    for m in manifests {
        if m.slug.contains('~') {
            continue;
        }
        let unit = m.workspace_id.as_deref().and_then(|w| workspace_unit(state, w));
        if !scope.holds(state, unit) {
            continue;
        }
        for t in &m.tables {
            for f in &t.fields {
                let s = scaffoldry_engine::effective_ferpa_sensitive(f, None, &categories);
                if s.protected && f.mask.is_none() {
                    out.push(json!({
                        "kind": "field_unmasked", "app_slug": m.slug, "table_id": t.id, "field": f.name,
                        "sentence": format!("'{}' in {} is protected and names no mask, so a reader who is not cleared sees nothing of it. Choose a mask if part of it is safe to show.", f.name, m.title),
                    }));
                }
            }
        }
    }
}

/// What could be simpler, from queries over what is stored.
pub fn suggestions(state: &SharedState, caller: &AuthUser) -> Result<Value, ServiceError> {
    let scope = Scope::of(state, caller)?;
    let mut out = Vec::new();
    covered_memberships(state, &scope, &mut out);
    holder_unit_groups(state, &scope, &mut out);
    grants_past_their_end(state, &scope, &mut out);
    grants_nothing_used(state, &scope, &mut out);
    unmasked_fields(state, &scope, &mut out);
    Ok(json!({ "suggestions": out }))
}
