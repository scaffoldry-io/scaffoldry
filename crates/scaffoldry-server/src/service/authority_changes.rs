//! Decision authority phase 2: the matrix in the console. A unit proposes a change, a different person confirms it, and
//! before either, the effect on the last 90 days of proposals is counted from what was stored. Nothing here reruns a check.

use crate::service::admin::{admin_write, require_platform_admin_or_compliance};
use crate::service::approvers::resolution_at;
use crate::service::authority::{raised_in, unit_path};
use crate::service::library::{Problems, OSCAL_CONTROL};
use crate::service::ServiceError;
use crate::state::{AuthUser, SharedState};
use scaffoldry_core::approver::ApproverSpec;
use scaffoldry_core::authority::{built_in_path, effective_for_tier, required, MatrixRow, Path, SignalRow, Tier, BUILT_IN_CHECK_TIER, FLAGGED};
use scaffoldry_core::ledger::DecisionType;
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashMap};
use uuid::Uuid;

const WINDOW_DAYS: i32 = 90;
const MAX_PROPOSALS: i64 = 5000;
const CHANGE_CONTROL: &str = "CM-03";

fn internal(e: impl ToString) -> ServiceError {
    ServiceError::Internal(e.to_string())
}

/// What a caller may do: anything (a Platform Admin or compliance), or inside the subtree of each unit they administer.
struct Rights {
    platform: bool,
    admin_units: Vec<Uuid>,
}

fn rights(state: &SharedState, caller: &AuthUser) -> Result<Rights, ServiceError> {
    let platform = require_platform_admin_or_compliance(caller, state).is_ok();
    let admin_units: Vec<Uuid> = state.roles_of(&caller.eppn).iter().filter(|r| r.scoped_affiliation == "unit_admin").map(|r| r.organization_id).collect();
    if !platform && admin_units.is_empty() {
        return Err(ServiceError::forbidden("Only a Platform Admin, a compliance caller, or an Org Unit Admin may do this"));
    }
    Ok(Rights { platform, admin_units })
}

impl Rights {
    /// Whether the caller acts for `unit`: they administer it or a unit above it.
    fn acts_for(&self, state: &SharedState, unit: Uuid) -> bool {
        self.platform || unit_path(state, Some(unit)).iter().any(|u| self.admin_units.contains(u))
    }

    fn require(&self, state: &SharedState, unit: Uuid) -> Result<(), ServiceError> {
        if self.acts_for(state, unit) {
            Ok(())
        } else {
            Err(ServiceError::forbidden("You act for this unit only if you administer it or a unit above it, or you are a Platform Admin or compliance"))
        }
    }
}

fn parse_unit(state: &SharedState, text: &str) -> Result<Uuid, ServiceError> {
    let id = Uuid::parse_str(text.trim()).map_err(|_| ServiceError::NotFound("Unit not found".to_string()))?;
    if !state.organizations.read().unwrap_or_else(|p| p.into_inner()).contains_key(&id) {
        return Err(ServiceError::NotFound("Unit not found".to_string()));
    }
    Ok(id)
}

fn unit_name(state: &SharedState, id: Uuid) -> String {
    state.organizations.read().unwrap_or_else(|p| p.into_inner()).get(&id).map(|o| o.name.clone()).unwrap_or_else(|| "A unit".to_string())
}

/// The units at or below `unit`.
fn subtree(state: &SharedState, unit: Uuid) -> Vec<Uuid> {
    let orgs = state.organizations.read().unwrap_or_else(|p| p.into_inner());
    let mut out = vec![unit];
    let mut i = 0;
    while i < out.len() {
        let parent = out[i];
        let kids: Vec<Uuid> = orgs.values().filter(|o| o.parent_id == Some(parent) && !out.contains(&o.id)).map(|o| o.id).collect();
        out.extend(kids);
        i += 1;
    }
    out
}

/// A change the matrix could hold, checked: a signal that raises a flagged check, or a row that tightens a tier.
fn validate(state: &SharedState, body: &Value) -> Result<Value, ServiceError> {
    let mut p = Problems::default();
    let kind = body.get("kind").and_then(Value::as_str);
    let normal = match kind {
        Some("signal") => {
            let check = p.text(body, "check_name", 64, true).unwrap_or_default();
            if !check.is_empty() && !FLAGGED.iter().any(|(n, _)| *n == check) {
                p.add("check_name", "check_name is not a check that can be raised");
            }
            let tier = body.get("tier").and_then(Value::as_str).and_then(Tier::parse);
            if tier.is_none() {
                p.add("tier", "tier is Low, Moderate or High");
            }
            p.done()?;
            let tier = tier.unwrap_or(Tier::Low);
            if tier < BUILT_IN_CHECK_TIER {
                return Err(ServiceError::AuthorityLoosens(format!("A flagged check is at least {}. {} is lower", BUILT_IN_CHECK_TIER.as_str(), tier.as_str())));
            }
            json!({ "kind": "signal", "check_name": check, "tier": tier.as_str() })
        }
        Some("row") => {
            let row = body.get("row").cloned().unwrap_or(Value::Null);
            let tier = row.get("tier").and_then(Value::as_str).and_then(Tier::parse);
            let path = row.get("path").and_then(Value::as_str).and_then(Path::parse);
            if tier.is_none() {
                p.add("row.tier", "tier is Low, Moderate or High");
            }
            if path.is_none() {
                p.add("row.path", "path is direct, second_person or approver");
            }
            let mut approver = Value::Null;
            match (path, row.get("approver").filter(|a| !a.is_null())) {
                (Some(Path::Approver), None) => p.add("row.approver", "an approver row names an approver"),
                (Some(Path::Approver), Some(raw)) => match serde_json::from_value::<ApproverSpec>(raw.clone()) {
                    Ok(spec) => match crate::service::approvers::check_spec(state, &spec) {
                        Ok(()) => approver = raw.clone(),
                        Err(why) => p.add("row.approver", why),
                    },
                    Err(_) => p.add("row.approver", "approver is a Person, a Position or a Role"),
                },
                (Some(_), Some(_)) => p.add("row.approver", "only an approver row names an approver"),
                _ => {}
            }
            let optional = |p: &mut Problems, key: &str, max: usize| p.text(&row, key, max, false).filter(|s| !s.is_empty());
            let control = optional(&mut p, "control_id", 32);
            let policy = optional(&mut p, "library_policy_key", 64);
            let rule = optional(&mut p, "library_rule_key", 64);
            p.done()?;
            let (tier, path) = (tier.unwrap_or(Tier::Low), path.unwrap_or(Path::Direct));
            if path < built_in_path(tier) {
                return Err(ServiceError::AuthorityLoosens(format!("A {} change needs at least {}. {} is lower", tier.as_str(), built_in_path(tier).as_str(), path.as_str())));
            }
            let mut out = json!({ "tier": tier.as_str(), "path": path.as_str() });
            for (k, v) in [("approver", approver), ("control_id", json!(control)), ("library_policy_key", json!(policy)), ("library_rule_key", json!(rule))] {
                if !v.is_null() {
                    out[k] = v;
                }
            }
            json!({ "kind": "row", "row": out })
        }
        _ => {
            p.add("kind", "kind is signal or row");
            p.done()?;
            Value::Null
        }
    };
    Ok(normal)
}

/// The tables with `change` applied at `unit`, for a what-if.
fn with_change(unit: Uuid, change: &Value, tables: &(Vec<SignalRow>, Vec<MatrixRow>)) -> (Vec<SignalRow>, Vec<MatrixRow>) {
    let (mut signals, mut rows) = tables.clone();
    if change["kind"] == "signal" {
        let check = change["check_name"].as_str().unwrap_or_default().to_string();
        signals.retain(|s| !(s.organization_id == unit && s.check_name == check));
        signals.push(SignalRow { organization_id: unit, check_name: check, tier: Tier::parse(change["tier"].as_str().unwrap_or_default()).unwrap_or(Tier::Low) });
    } else {
        let r = &change["row"];
        let tier = Tier::parse(r["tier"].as_str().unwrap_or_default()).unwrap_or(Tier::Low);
        rows.retain(|x| !(x.organization_id == unit && x.tier == tier));
        rows.push(MatrixRow {
            organization_id: unit,
            tier,
            path: Path::parse(r["path"].as_str().unwrap_or_default()).unwrap_or(Path::Direct),
            approver: serde_json::from_value(r["approver"].clone()).ok(),
            control_id: None,
            library_policy_key: None,
            library_rule_key: None,
        });
    }
    (signals, rows)
}

/// `GET /admin/authority`: the built-in answer, the rows on the path to the root, and the answer for each tier.
pub fn show(state: &SharedState, caller: &AuthUser, unit: Option<&str>) -> Result<Value, ServiceError> {
    let rights = rights(state, caller)?;
    let unit = parse_unit(state, unit.unwrap_or_default())?;
    rights.require(state, unit)?;
    let tables = state.repository.authority_tables().map_err(internal)?;
    let path = unit_path(state, Some(unit));
    let (signals, rows): (Vec<&SignalRow>, Vec<&MatrixRow>) = (tables.0.iter().filter(|s| path.contains(&s.organization_id)).collect(), tables.1.iter().filter(|r| path.contains(&r.organization_id)).collect());
    let on_path_rows: Vec<MatrixRow> = rows.iter().map(|r| (*r).clone()).collect();

    let check_tier = |name: &str| signals.iter().filter(|s| s.check_name == name).map(|s| s.tier).fold(BUILT_IN_CHECK_TIER, Tier::max);
    let tiers: Vec<Value> = [Tier::Low, Tier::Moderate, Tier::High]
        .into_iter()
        .map(|tier| {
            let eff = effective_for_tier(tier, &path, &on_path_rows);
            let raised_by: Vec<Value> = FLAGGED.iter().filter(|(n, _)| check_tier(n) == tier).map(|(n, s)| json!({ "check": n, "sentence": s })).collect();
            let resolved: Vec<Value> = match (&eff.approver, eff.path) {
                (Some(spec), Path::Approver) => resolution_at(state, spec, unit, &[], None).approvers.iter().map(|a| json!({ "eppn": a.eppn, "display_name": a.display_name, "via": a.via })).collect(),
                _ => Vec::new(),
            };
            let source = match eff.source {
                None => json!({ "kind": "built_in" }),
                Some(id) => json!({ "kind": if state.organizations.read().unwrap_or_else(|p| p.into_inner()).get(&id).is_some_and(|o| o.parent_id.is_none()) { "institution" } else { "unit" }, "organization_id": id, "name": unit_name(state, id) }),
            };
            json!({ "tier": tier.as_str(), "raised_by": raised_by, "path": eff.path.as_str(), "approver": eff.approver, "resolved": resolved, "source": source })
        })
        .collect();

    let row_views: Vec<Value> = rows
        .iter()
        .map(|r| {
            // A row has an effect when the units above it, and the built-in answer, do not already ask for as much.
            let others: Vec<MatrixRow> = on_path_rows.iter().filter(|o| !(o.organization_id == r.organization_id && o.tier == r.tier)).cloned().collect();
            let rest = effective_for_tier(r.tier, &unit_path(state, Some(r.organization_id)), &others);
            json!({
                "organization_id": r.organization_id, "unit_name": unit_name(state, r.organization_id), "tier": r.tier.as_str(), "path": r.path.as_str(), "approver": r.approver,
                "control_id": r.control_id, "library_policy_key": r.library_policy_key, "library_rule_key": r.library_rule_key,
                "has_effect": r.path > rest.path,
            })
        })
        .collect();
    let signal_views: Vec<Value> = signals.iter().map(|s| json!({ "organization_id": s.organization_id, "unit_name": unit_name(state, s.organization_id), "check_name": s.check_name, "tier": s.tier.as_str() })).collect();
    Ok(json!({
        "organization_id": unit,
        "unit_name": unit_name(state, unit),
        "built_in": { "check_tier": BUILT_IN_CHECK_TIER.as_str(), "paths": { "Low": built_in_path(Tier::Low).as_str(), "Moderate": built_in_path(Tier::Moderate).as_str(), "High": built_in_path(Tier::High).as_str() } },
        "signals": signal_views,
        "rows": row_views,
        "tiers": tiers,
    }))
}

/// `POST /admin/authority/changes`: stores a pending change. Nothing is applied.
pub fn propose(state: &SharedState, caller: &AuthUser, body: &Value) -> Result<Value, ServiceError> {
    let rights = rights(state, caller)?;
    let mut p = Problems::default();
    let unit_text = p.text(body, "organization_id", 64, true);
    let reason = p.text(body, "reason", 500, true);
    p.done()?;
    let unit = parse_unit(state, unit_text.as_deref().unwrap_or_default())?;
    rights.require(state, unit)?;
    let change = validate(state, body.get("change").unwrap_or(&Value::Null))?;
    let stored = state.repository.authority_insert_change(unit, &change, reason.as_deref().unwrap_or_default(), &caller.eppn).map_err(internal)?;
    Ok(json!({ "change": stored }))
}

/// `GET /admin/authority/changes`: the changes the caller acts for, newest first.
pub fn list(state: &SharedState, caller: &AuthUser, unit: Option<&str>) -> Result<Value, ServiceError> {
    let rights = rights(state, caller)?;
    let units = match unit.filter(|u| !u.is_empty()) {
        Some(text) => {
            let unit = parse_unit(state, text)?;
            rights.require(state, unit)?;
            Some(vec![unit])
        }
        None if rights.platform => None,
        None => Some(rights.admin_units.iter().flat_map(|u| subtree(state, *u)).collect()),
    };
    let mut changes = state.repository.authority_changes(units).map_err(internal)?;
    for c in &mut changes {
        let id = c["organization_id"].as_str().and_then(|u| Uuid::parse_str(u).ok());
        c["unit_name"] = json!(id.map(|id| unit_name(state, id)));
    }
    Ok(json!({ "changes": changes }))
}

/// `POST /admin/authority/changes/{id}/decide`: a different person confirms or rejects.
pub fn decide(state: &SharedState, caller: &AuthUser, id: &str, body: &Value) -> Result<Value, ServiceError> {
    let rights = rights(state, caller)?;
    let id = Uuid::parse_str(id).map_err(|_| ServiceError::NotFound("Change not found".to_string()))?;
    let change = state.repository.authority_change(id).map_err(internal)?.ok_or_else(|| ServiceError::NotFound("Change not found".to_string()))?;
    let unit = change["organization_id"].as_str().and_then(|u| Uuid::parse_str(u).ok()).ok_or_else(|| internal("change without unit"))?;
    rights.require(state, unit)?;
    if change["proposed_by"].as_str().is_some_and(|p| p.eq_ignore_ascii_case(&caller.eppn)) {
        return Err(ServiceError::forbidden("A different person must decide a change you proposed"));
    }
    let mut p = Problems::default();
    let confirm = match body.get("decision").and_then(Value::as_str) {
        Some("confirm") => true,
        Some("reject") => false,
        _ => {
            p.add("decision", "decision is confirm or reject");
            false
        }
    };
    let note = p.text(body, "note", 500, true).unwrap_or_default();
    p.done()?;
    if change["status"] != "Pending" {
        return Err(ServiceError::Conflict("This change is already decided".to_string()));
    }
    let conflict = || ServiceError::Conflict("This change is already decided".to_string());
    let read = |state: &SharedState| -> Result<Value, ServiceError> { Ok(json!({ "change": state.repository.authority_change(id).map_err(internal)? })) };
    if !confirm {
        if !state.repository.authority_decide(id, false, &caller.eppn).map_err(internal)? {
            return Err(conflict());
        }
        return read(state);
    }
    let payload = json!({ "action": "authority_changed", "change_id": id, "organization_id": unit, "change": change["change"], "proposed_by": change["proposed_by"], "decided_by": caller.eppn });
    admin_write(state, caller, DecisionType::AuthorityChanged, CHANGE_CONTROL, &note, &payload, || {
        if !state.repository.authority_decide(id, true, &caller.eppn).map_err(internal)? {
            return Err(conflict());
        }
        read(state)
    })
}

#[derive(Default, Clone, Copy)]
struct Counts {
    direct: i64,
    second_person: i64,
    approver: i64,
}

impl Counts {
    fn add(&mut self, p: Path) {
        match p {
            Path::Direct => self.direct += 1,
            Path::SecondPerson => self.second_person += 1,
            Path::Approver => self.approver += 1,
        }
    }

    fn json(self) -> Value {
        json!({ "direct": self.direct, "second_person": self.second_person, "approver": self.approver })
    }
}

/// `POST /admin/authority/impact`: runs `required` over the last 90 days of proposals in the unit and below, as the
/// matrix is and as it would be with `change`. It reads stored checks, stores nothing, and reruns nothing.
pub fn impact(state: &SharedState, caller: &AuthUser, body: &Value) -> Result<Value, ServiceError> {
    let rights = rights(state, caller)?;
    let mut p = Problems::default();
    let unit_text = p.text(body, "organization_id", 64, true);
    p.done()?;
    let unit = parse_unit(state, unit_text.as_deref().unwrap_or_default())?;
    rights.require(state, unit)?;
    let change = validate(state, body.get("change").unwrap_or(&Value::Null))?;

    let tables = state.repository.authority_tables().map_err(internal)?;
    let what_if = with_change(unit, &change, &tables);
    let below = subtree(state, unit);
    let workspaces: HashMap<String, Option<Uuid>> = state.workspaces.read().unwrap_or_else(|p| p.into_inner()).iter().filter(|(_, w)| w.organization_id.is_some_and(|o| below.contains(&o))).map(|(id, w)| (id.clone(), w.organization_id)).collect();
    let ids: Vec<String> = workspaces.keys().cloned().collect();
    let proposals = if ids.is_empty() { Vec::new() } else { state.repository.proposals_in(&ids, WINDOW_DAYS, MAX_PROPOSALS).map_err(internal)? };

    let (mut before, mut after) = (Counts::default(), Counts::default());
    let mut load: BTreeMap<String, (String, i64, i64)> = BTreeMap::new();
    let mut unassigned = Vec::new();
    let mut resolved_cache: HashMap<(String, Uuid, String), Vec<(String, String)>> = HashMap::new();
    let mut resolve = |spec: &ApproverSpec, at: Uuid, proposer: &str| -> Vec<(String, String)> {
        let key = (serde_json::to_string(spec).unwrap_or_default(), at, proposer.to_lowercase());
        resolved_cache.entry(key).or_insert_with(|| resolution_at(state, spec, at, &[], Some(proposer)).approvers.into_iter().map(|a| (a.eppn, a.display_name)).collect()).clone()
    };
    for pr in &proposals {
        let ws_unit = workspaces.get(&pr.workspace_id).copied().flatten();
        let path = unit_path(state, ws_unit);
        let raised = raised_in(&pr.checks);
        let now = required(&raised, &path, &tables.0, &tables.1);
        let then = required(&raised, &path, &what_if.0, &what_if.1);
        before.add(now.path);
        after.add(then.path);
        for (need, is_after) in [(&now, false), (&then, true)] {
            let (Some(spec), Some(at)) = (&need.approver, ws_unit) else { continue };
            if need.path != Path::Approver {
                continue;
            }
            let people = resolve(spec, at, &pr.proposed_by);
            if people.is_empty() && is_after {
                unassigned.push(json!({ "proposal_id": pr.id, "workspace_id": pr.workspace_id, "summary": pr.summary }));
            }
            for (eppn, name) in people {
                let entry = load.entry(eppn).or_insert((name, 0, 0));
                if is_after {
                    entry.2 += 1;
                } else {
                    entry.1 += 1;
                }
            }
        }
    }
    let load: Vec<Value> = load.into_iter().map(|(eppn, (name, b, a))| json!({ "eppn": eppn, "display_name": name, "before": b, "after": a })).collect();
    Ok(json!({ "organization_id": unit, "window_days": WINDOW_DAYS, "proposals": proposals.len(), "before": before.json(), "after": after.json(), "load": load, "unassigned": unassigned, "truncated": proposals.len() as i64 >= MAX_PROPOSALS, "oscal_control": OSCAL_CONTROL }))
}
