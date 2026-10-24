//! Access model phase 4: creating, listing, revoking and confirming grants. What a grant reaches is decided by
//! `scaffoldry_core::reach`. This file decides who may write one and how.

use crate::repository_grants::{ChangeDecision, GrantFilter, GrantRow, RevokeOutcome};
use crate::service::admin::admin_write;
use crate::service::members::{self, Role};
use crate::service::ServiceError;
use crate::state::{AuthUser, SharedState};
use scaffoldry_core::coordinates::{sensitivity_rank, SENSITIVITY_LEVELS};
use scaffoldry_core::ledger::DecisionType;
use scaffoldry_core::reach::{AnchorKind, Grant, HolderKind};
use serde_json::{json, Value};
use uuid::Uuid;

const SOURCES: [&str; 6] = ["membership", "appointment", "rule", "request", "task", "flow"];

/// A grant as a caller asks for it, checked.
struct NewGrant {
    holder_kind: HolderKind,
    holder_id: String,
    role: Role,
    hierarchy: Option<String>,
    anchor_kind: AnchorKind,
    anchor_id: Option<String>,
    depth: Option<i64>,
    domain: Option<String>,
    clearance: Option<String>,
    valid_until: Option<String>,
    source: String,
    reason: String,
}

impl NewGrant {
    fn json(&self) -> Value {
        json!({
            "holder_kind": self.holder_kind.as_str(), "holder_id": self.holder_id, "role": self.role.as_str(),
            "hierarchy": self.hierarchy, "anchor_kind": self.anchor_kind.as_str(), "anchor_id": self.anchor_id, "depth": self.depth,
            "domain": self.domain, "clearance": self.clearance, "valid_until": self.valid_until, "source": self.source, "reason": self.reason,
        })
    }

    /// Reads a stored grant back. The stored form was checked when it was proposed.
    fn of_json(v: &Value) -> Option<NewGrant> {
        let text = |k: &str| v[k].as_str().map(str::to_string);
        Some(NewGrant {
            holder_kind: HolderKind::parse(v["holder_kind"].as_str()?)?,
            holder_id: text("holder_id")?,
            role: Role::parse(v["role"].as_str()?)?,
            hierarchy: text("hierarchy"),
            anchor_kind: AnchorKind::parse(v["anchor_kind"].as_str()?)?,
            anchor_id: text("anchor_id"),
            depth: v["depth"].as_i64(),
            domain: text("domain"),
            clearance: text("clearance"),
            valid_until: text("valid_until"),
            source: text("source")?,
            reason: text("reason")?,
        })
    }
}

fn bad(msg: impl Into<String>) -> ServiceError {
    ServiceError::BadRequest(msg.into())
}

fn internal(e: impl ToString) -> ServiceError {
    ServiceError::Internal(e.to_string())
}

/// What a caller may do with grants: anything (a Platform Admin or compliance), or inside the subtree of each unit they
/// administer, or on a workspace they own or administer.
struct Rights {
    platform: bool,
    admin_units: Vec<Uuid>,
}

fn rights(state: &SharedState, caller: &AuthUser) -> Rights {
    let platform = crate::service::admin::require_platform_admin_or_compliance(caller, state).is_ok();
    let admin_units = state.roles_of(&caller.eppn).iter().filter(|r| r.scoped_affiliation == "unit_admin").map(|r| r.organization_id).collect();
    Rights { platform, admin_units }
}

/// The workspace an anchor names: `workspace:<id>`, or `app:<slug>` or `table:<slug>/<table id>` through the app's workspace.
fn workspace_of_anchor(state: &SharedState, anchor: &str) -> Option<String> {
    if let Some(id) = anchor.strip_prefix("workspace:") {
        return state.workspaces.read().unwrap_or_else(|p| p.into_inner()).contains_key(id).then(|| id.to_string());
    }
    use scaffoldry_engine::HostRouter;
    if let Some(rest) = anchor.strip_prefix("table:") {
        let (slug, table) = rest.split_once('/')?;
        let engine = state.engine.read().unwrap_or_else(|p| p.into_inner());
        return engine.resolve_by_slug(slug).filter(|m| m.tables.iter().any(|t| t.id == table)).and_then(|m| m.workspace_id.clone());
    }
    let slug = anchor.strip_prefix("app:")?;
    state.engine.read().unwrap_or_else(|p| p.into_inner()).resolve_by_slug(slug).and_then(|m| m.workspace_id.clone())
}

/// Whether the caller may create (or revoke, or confirm) this grant.
fn scope_allows(state: &SharedState, rights: &Rights, caller: &AuthUser, g: &NewGrant) -> Result<(), ServiceError> {
    if rights.platform {
        return Ok(());
    }
    let outside = |why: &str| Err(ServiceError::GrantOutsideScope(format!("You may not give this access: {why}")));
    match g.anchor_kind {
        AnchorKind::Unit => {
            if g.role > Role::Admin {
                return outside("an Org Unit Admin may grant at most admin");
            }
            let anchor = g.anchor_id.as_deref().and_then(|a| Uuid::parse_str(a).ok());
            let tree = state.tree.read().unwrap_or_else(|p| p.into_inner());
            match anchor {
                Some(a) if rights.admin_units.iter().any(|u| tree.contains(*u, a)) => Ok(()),
                _ => outside("the anchor is outside the units you administer"),
            }
        }
        AnchorKind::Container => {
            let ws = g.anchor_id.as_deref().and_then(|a| workspace_of_anchor(state, a));
            match ws.and_then(|ws| members::member_role(caller, &ws, state)) {
                Some(mine) if mine >= Role::Admin && g.role <= mine => Ok(()),
                _ => outside("you do not own or administer that workspace, or the role is above yours"),
            }
        }
        AnchorKind::HolderUnit => outside("a grant anchored at the holder's unit is for a Platform Admin"),
    }
}

fn parse_new_grant(state: &SharedState, body: &Value) -> Result<NewGrant, ServiceError> {
    let text = |k: &str| body.get(k).and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty()).map(str::to_string);
    let need = |k: &str| text(k).ok_or_else(|| bad(format!("{k} is required")));
    let holder_kind = HolderKind::parse(&need("holder_kind")?).ok_or_else(|| bad(format!("holder_kind must be one of {}", HolderKind::ALL.join(", "))))?;
    let anchor_kind = AnchorKind::parse(&need("anchor_kind")?).ok_or_else(|| bad(format!("anchor_kind must be one of {}", AnchorKind::ALL.join(", "))))?;
    let role = Role::parse(&need("role")?).ok_or_else(|| bad("role must be one of viewer, commenter, editor, admin, owner"))?;
    let reason = need("reason")?;
    if reason.len() > 500 {
        return Err(bad("reason may be at most 500 characters"));
    }
    let holder_id = need("holder_id")?;
    let source = text("source").unwrap_or_else(|| "request".to_string());
    if !SOURCES.contains(&source.as_str()) {
        return Err(bad(format!("source must be one of {}", SOURCES.join(", "))));
    }
    let depth = match body.get("depth") {
        None | Some(Value::Null) => None,
        Some(d) => Some(d.as_i64().filter(|d| (0..=1000).contains(d)).ok_or_else(|| bad("depth must be a whole number from 0"))?),
    };
    let g = NewGrant { holder_kind, holder_id, role, hierarchy: text("hierarchy"), anchor_kind, anchor_id: text("anchor_id"), depth, domain: text("domain"), clearance: text("clearance"), valid_until: None, source, reason };
    let mut g = g;
    if let Some(until) = text("valid_until") {
        let parsed = chrono::DateTime::parse_from_rfc3339(&until).map_err(|_| bad("valid_until must be an RFC 3339 time"))?;
        if parsed <= chrono::Utc::now() {
            return Err(bad("valid_until must be in the future"));
        }
        g.valid_until = Some(parsed.to_utc().to_rfc3339());
    }
    if let Some(c) = &g.clearance {
        if sensitivity_rank(c).is_none() {
            return Err(bad(format!("clearance must be one of {}", SENSITIVITY_LEVELS.join(", "))));
        }
    }
    if let Some(d) = &g.domain {
        if !crate::service::coordinates::domains(state)?.iter().any(|(k, _)| k == d) {
            return Err(bad(format!("domain '{d}' does not exist")));
        }
    }
    check_anchor(state, &g)?;
    check_holder(state, &g)?;
    check_synthetic_grant(state, &g)?;
    Ok(g)
}

fn check_synthetic_grant(state: &SharedState, g: &NewGrant) -> Result<(), ServiceError> {
    let is_synth_person = g.holder_kind == HolderKind::Person && g.holder_id.to_lowercase().ends_with("@synthetic.invalid");
    if is_synth_person {
        match g.anchor_kind {
            AnchorKind::Unit => {
                let anchor = g.anchor_id.as_deref().and_then(|a| Uuid::parse_str(a).ok());
                if let Some(anchor_id) = anchor {
                    let orgs = state.organizations.read().unwrap_or_else(|p| p.into_inner());
                    if let Some(org) = orgs.get(&anchor_id) {
                        if !org.synthetic {
                            return Err(bad("A grant held by a synthetic person may only anchor inside the synthetic subtree"));
                        }
                    }
                }
            }
            AnchorKind::Container => {
                if let Some(anchor) = g.anchor_id.as_deref() {
                    if let Some(ws_id) = workspace_of_anchor(state, anchor) {
                        let ws_map = state.workspaces.read().unwrap_or_else(|p| p.into_inner());
                        if let Some(ws) = ws_map.get(&ws_id) {
                            if !ws.synthetic {
                                return Err(bad("A grant held by a synthetic person may only anchor inside the synthetic subtree"));
                            }
                        }
                    }
                }
            }
            AnchorKind::HolderUnit => {
                return Err(bad("A grant held by a synthetic person may only anchor inside the synthetic subtree"));
            }
        }
    }
    Ok(())
}

fn check_anchor(state: &SharedState, g: &NewGrant) -> Result<(), ServiceError> {
    match g.anchor_kind {
        AnchorKind::Container => {
            if g.hierarchy.is_some() || g.depth.is_some() {
                return Err(bad("a container anchor has no hierarchy or depth"));
            }
            let anchor = g.anchor_id.as_deref().ok_or_else(|| bad("anchor_id is required for a container anchor"))?;
            workspace_of_anchor(state, anchor).ok_or_else(|| bad(format!("container '{anchor}' does not exist. Use workspace:<id>, app:<slug> or table:<slug>/<table id>")))?;
        }
        AnchorKind::Unit | AnchorKind::HolderUnit => {
            let hierarchy = g.hierarchy.as_deref().ok_or_else(|| bad("hierarchy is required for a unit anchor"))?;
            let known = state.organizations.read().unwrap_or_else(|p| p.into_inner()).values().any(|o| o.hierarchy_key == hierarchy);
            if !known {
                return Err(bad(format!("hierarchy '{hierarchy}' does not exist")));
            }
            if g.anchor_kind == AnchorKind::HolderUnit {
                if g.anchor_id.is_some() || g.holder_kind != HolderKind::Position {
                    return Err(bad("a holder_unit anchor belongs to a position holder and names no anchor_id"));
                }
            } else {
                let anchor = g.anchor_id.as_deref().and_then(|a| Uuid::parse_str(a).ok()).ok_or_else(|| bad("anchor_id must be a unit id"))?;
                if state.tree.read().unwrap_or_else(|p| p.into_inner()).hierarchy_of(anchor) != Some(hierarchy) {
                    return Err(bad(format!("unit {anchor} is not a unit of the '{hierarchy}' hierarchy")));
                }
            }
        }
    }
    Ok(())
}

fn check_holder(state: &SharedState, g: &NewGrant) -> Result<(), ServiceError> {
    let unit_exists = |id: &str| Uuid::parse_str(id).ok().is_some_and(|u| state.organizations.read().unwrap_or_else(|p| p.into_inner()).contains_key(&u));
    match g.holder_kind {
        HolderKind::Person => {
            if !g.holder_id.contains('@') {
                return Err(bad("a person holder is an address"));
            }
        }
        HolderKind::Position => {
            let (key, unit) = g.holder_id.split_once('@').ok_or_else(|| bad("a position holder is key@<unit id> or key@*"))?;
            let key_ok = !key.is_empty() && key.len() <= 63 && key.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-');
            if !key_ok || (unit != "*" && !unit_exists(unit)) {
                return Err(bad("a position holder is key@<unit id> or key@*, with a position key and a unit that exists"));
            }
        }
        HolderKind::Unit => {
            if !unit_exists(&g.holder_id) {
                return Err(bad(format!("holder unit '{}' does not exist", g.holder_id)));
            }
        }
        HolderKind::Group => {
            if state.repository.group_by_id(&g.holder_id).map_err(internal)?.is_none() {
                return Err(bad(format!("holder group '{}' does not exist", g.holder_id)));
            }
        }
    }
    // A grant to the root of a hierarchy is a grant to everyone in it. Institutional visibility already says that.
    if g.holder_kind == HolderKind::Unit {
        let root = Uuid::parse_str(&g.holder_id).ok().and_then(|u| state.organizations.read().unwrap_or_else(|p| p.into_inner()).get(&u).map(|o| o.parent_id.is_none())).unwrap_or(false);
        if root && !(g.anchor_kind == AnchorKind::Container && g.role == Role::Viewer) {
            return Err(ServiceError::GrantTooBroad("A grant to the top unit of a hierarchy reaches everyone in it. Anchor it on one workspace as a viewer, or use the workspace's institutional visibility".to_string()));
        }
    }
    Ok(())
}

/// A pending change as a grant the reach rule can read, for the impact of deciding it. `None` when no grant change has this
/// id. The caller needs the right to decide the change, which is the right to see what it would do.
pub fn pending_grant(state: &SharedState, caller: &AuthUser, id: Uuid) -> Result<Option<Grant>, ServiceError> {
    let Some((change, status, _, _)) = state.repository.access_change(id).map_err(internal)? else { return Ok(None) };
    let g = NewGrant::of_json(&change["grant"]).ok_or_else(|| ServiceError::Internal("The stored change is not a grant".to_string()))?;
    scope_allows(state, &rights(state, caller), caller, &g).map_err(|_| ServiceError::forbidden("You do not hold the right to decide this change"))?;
    if status != "Pending" {
        return Err(ServiceError::conflict("That change has already been decided"));
    }
    Ok(Some(Grant {
        id: i64::MAX,
        holder_kind: g.holder_kind,
        holder_id: g.holder_id,
        role: g.role,
        hierarchy: g.hierarchy,
        anchor_kind: g.anchor_kind,
        anchor_id: g.anchor_id,
        depth: g.depth.and_then(|d| i16::try_from(d).ok()),
        domain: g.domain,
        valid_from: 0,
        valid_until: g.valid_until.as_deref().and_then(|u| chrono::DateTime::parse_from_rfc3339(u).ok()).map(|d| d.timestamp()),
        revoked: false,
        clearance: g.clearance.as_deref().and_then(sensitivity_rank),
    }))
}

/// Stores a new grant as a change that waits for a different person.
pub fn propose_grant(state: &SharedState, caller: &AuthUser, body: &Value) -> Result<Value, ServiceError> {
    let r = rights(state, caller);
    // Only a workspace owner or admin can grant without administering a unit, and only on a container.
    if !r.platform && r.admin_units.is_empty() && body.get("anchor_kind").and_then(Value::as_str) != Some("container") {
        return Err(ServiceError::GrantOutsideScope("You may not give access here".to_string()));
    }
    let g = parse_new_grant(state, body)?;
    scope_allows(state, &r, caller, &g)?;
    let change = json!({ "kind": "grant", "grant": g.json() });
    let id = state.repository.propose_access_change(&change, true, &g.reason, &caller.eppn).map_err(internal)?;
    Ok(json!({ "change_id": id, "status": "Pending", "widens": true, "message": "A different person with the same right must confirm this grant before it takes effect." }))
}

/// A thing that is not there. Someone who could not have acted on it anyway is told they may not, so the answer does not
/// say which ids exist.
pub fn missing(state: &SharedState, caller: &AuthUser, what: &str) -> ServiceError {
    let r = rights(state, caller);
    if r.platform || !r.admin_units.is_empty() {
        ServiceError::NotFound(format!("{what} not found"))
    } else {
        ServiceError::forbidden("Platform Admin, compliance, or Org Unit Admin required")
    }
}

fn reason_of(body: &Value) -> Result<String, ServiceError> {
    body.get("reason").and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty() && s.len() <= 500).map(str::to_string).ok_or_else(|| bad("reason is required and may be at most 500 characters"))
}

/// Confirms or rejects a change. The proposer cannot confirm. The decider needs the right to make the grant.
pub fn decide_change(state: &SharedState, caller: &AuthUser, id: Uuid, body: &Value) -> Result<Value, ServiceError> {
    let (change, status, proposed_by, _) = state.repository.access_change(id).map_err(internal)?.ok_or_else(|| missing(state, caller, "Change"))?;
    if status != "Pending" {
        return Err(ServiceError::conflict("That change has already been decided"));
    }
    let g = NewGrant::of_json(&change["grant"]).ok_or_else(|| ServiceError::Internal("The stored change is not a grant".to_string()))?;
    let r = rights(state, caller);
    scope_allows(state, &r, caller, &g).map_err(|_| ServiceError::forbidden("You do not hold the right to decide this change"))?;
    let confirm = match body.get("decision").and_then(Value::as_str) {
        Some("confirm") => true,
        Some("reject") => false,
        _ => return Err(bad("decision must be 'confirm' or 'reject'")),
    };
    let reason = reason_of(body)?;
    if confirm && proposed_by.eq_ignore_ascii_case(&caller.eppn) {
        return Err(ServiceError::forbidden("The person who proposed a change cannot confirm it"));
    }
    let decide = || state.repository.decide_access_change(id, &caller.eppn, confirm).map_err(internal);
    let outcome = if confirm {
        let payload = json!({ "action": "grant_confirmed", "change_id": id, "grant": g.json(), "proposed_by": proposed_by });
        admin_write(state, caller, DecisionType::AccessGrantChanged, "AC-02", &reason, &payload, decide)?
    } else {
        decide()?
    };
    match outcome {
        ChangeDecision::Done { grant_id } => Ok(json!({ "change_id": id, "status": if confirm { "Confirmed" } else { "Rejected" }, "grant_id": grant_id })),
        ChangeDecision::Missing => Err(ServiceError::NotFound("Change not found".to_string())),
        ChangeDecision::AlreadyDecided => Err(ServiceError::conflict("That change has already been decided")),
        ChangeDecision::OwnChange => Err(ServiceError::forbidden("The person who proposed a change cannot confirm it")),
    }
}

/// Revokes a grant. It narrows access, so it applies at once.
pub fn revoke_grant(state: &SharedState, caller: &AuthUser, id: i64, body: &Value) -> Result<Value, ServiceError> {
    let row = state.repository.grant_row(id).map_err(internal)?.ok_or_else(|| missing(state, caller, "Grant"))?;
    let as_new = NewGrant::of_json(&row.json()).ok_or_else(|| ServiceError::Internal("The stored grant is not a grant".to_string()))?;
    scope_allows(state, &rights(state, caller), caller, &as_new)?;
    let reason = reason_of(body)?;
    if row.grant.revoked {
        return Err(ServiceError::conflict("That grant is already revoked"));
    }
    let payload = json!({ "action": "grant_revoked", "grant": row.json() });
    let outcome = admin_write(state, caller, DecisionType::AccessGrantChanged, "AC-02", &reason, &payload, || state.repository.revoke_grant(id, &caller.eppn).map_err(internal))?;
    match outcome {
        RevokeOutcome::Done => Ok(json!({ "id": id, "status": "Revoked" })),
        RevokeOutcome::Missing => Err(ServiceError::NotFound("Grant not found".to_string())),
        RevokeOutcome::AlreadyRevoked => Err(ServiceError::conflict("That grant is already revoked")),
    }
}

/// The query of the grant list.
#[derive(Debug, Default)]
pub struct ListQuery {
    pub holder: Option<String>,
    pub anchor: Option<String>,
    pub hierarchy: Option<String>,
    pub source: Option<String>,
    pub state: Option<String>,
    pub cursor: Option<String>,
    pub limit: Option<i64>,
}

/// Grants and pending changes the caller may see: all for a Platform Admin or compliance, and those anchored inside the
/// units they administer for an Org Unit Admin.
pub fn list(state: &SharedState, caller: &AuthUser, q: &ListQuery) -> Result<Value, ServiceError> {
    let r = rights(state, caller);
    if !r.platform && r.admin_units.is_empty() {
        return Err(ServiceError::forbidden("Platform Admin, compliance, or Org Unit Admin required"));
    }
    let wanted = q.state.clone().unwrap_or_else(|| "active".to_string());
    if !["active", "revoked", "expired", "pending", "all"].contains(&wanted.as_str()) {
        return Err(bad("state must be one of active, revoked, expired, pending, all"));
    }
    let after = match &q.cursor {
        Some(c) => c.parse::<i64>().map_err(|_| bad("Invalid cursor"))?,
        None => 0,
    };
    let limit = q.limit.unwrap_or(50).clamp(1, 200);
    let in_scope = |anchor_kind: AnchorKind, anchor: Option<&str>| {
        r.platform
            || (anchor_kind == AnchorKind::Unit && anchor.and_then(|a| Uuid::parse_str(a).ok()).is_some_and(|a| {
                let tree = state.tree.read().unwrap_or_else(|p| p.into_inner());
                r.admin_units.iter().any(|u| tree.contains(*u, a))
            }))
    };
    let mut grants: Vec<Value> = Vec::new();
    let mut next = Value::Null;
    if wanted != "pending" {
        let filter = GrantFilter { holder: q.holder.clone(), anchor: q.anchor.clone(), hierarchy: q.hierarchy.clone(), source: q.source.clone(), state: wanted.clone(), after, limit: limit + 1 };
        let rows: Vec<GrantRow> = state.repository.list_grant_rows(&filter).map_err(internal)?;
        let visible: Vec<GrantRow> = rows.into_iter().filter(|g| in_scope(g.grant.anchor_kind, g.grant.anchor_id.as_deref())).collect();
        if visible.len() as i64 > limit {
            next = json!(visible[limit as usize - 1].grant.id.to_string());
        }
        grants = visible.iter().take(limit as usize).map(GrantRow::json).collect();
    }
    let changes: Vec<Value> = if wanted == "pending" || wanted == "all" {
        state
            .repository
            .list_access_changes("Pending")
            .map_err(internal)?
            .into_iter()
            .filter(|c| {
                let g = &c["grant"];
                AnchorKind::parse(g["anchor_kind"].as_str().unwrap_or_default()).is_some_and(|k| in_scope(k, g["anchor_id"].as_str()))
            })
            .collect()
    } else {
        Vec::new()
    };
    Ok(json!({ "grants": grants, "changes": changes, "next_cursor": next }))
}
