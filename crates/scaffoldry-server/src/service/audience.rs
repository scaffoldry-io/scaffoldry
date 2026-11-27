//! Data layer phase 3b: audience fields and conditions (`docs/plans/data-layer.md`, "Audience fields" and "Conditions").
//!
//! - **The policy** is in the manifest, per table (`AppTable::audience`, published in `app-manifest.schema.json`). The dataset keeps a copy.
//!   `scaffoldry_data::audience` keeps `record_audience` current in the transaction of every record write.
//! - **The read** is in `scope::plan_reading` (a list and a count), `scope::share_cover` (a single row and a write) and
//!   `ScopePlan::clause_at`. A name grants only up to the policy, and a category's requirement is still asked.
//! - **Acceptance.** A policy may ask a unit named in a field to accept first. The unit's steward does it here. The value grants nothing
//!   before. The acceptance is recorded as `AccessGrantChanged`, as a share's approval is.
//! - **Conditions.** A person's record holds a small map from a condition to its last day. An admin sets it here, with an owner and a source,
//!   and each change is a ledger entry. A category may name conditions (`sensitivity-settings.schema.json`). The check is in
//!   `row_labels::gate_for`: a date comparison on a read, with no job.
//!
//! Not built here: a reminder before a condition lapses, the owner's steward maintaining a condition (an admin does it in version 1), and
//! a sync that sets conditions (the `source` field records one).

use crate::repository::AppendDecisionParams;
use crate::repository_audience::{source_of, Accepted};
use crate::repository_catalog::CatalogRow;
use crate::service::label_changes::{is_admin_or_compliance, reason_of};
use crate::service::{scope, ServiceError};
use crate::state::{AuthUser, SharedState};
use scaffoldry_core::approver::{ApproverSpec, Origin};
use scaffoldry_core::ledger::DecisionType;
use scaffoldry_engine::AudienceGrant;
use serde_json::{json, Value};
use std::collections::BTreeSet;
use uuid::Uuid;

/// The most waiting values a list shows.
const LISTED: i64 = 200;

fn bad(m: impl Into<String>) -> ServiceError {
    ServiceError::BadRequest(m.into())
}

fn storage(e: crate::repository::RepositoryError) -> ServiceError {
    ServiceError::StorageUnavailable(e.to_string())
}

fn dataset_of(id: &str, state: &SharedState) -> Result<CatalogRow, ServiceError> {
    let row = state.repository.catalog_row(id).map_err(storage)?.ok_or_else(|| ServiceError::NotFound("Dataset not found".to_string()))?;
    if row.published_id.is_some() {
        return Err(bad("A published dataset holds no rows, so it has no audience"));
    }
    Ok(row)
}

/// The people who hold the dataset's steward position at the receiving unit or above it.
fn stewards_at(row: &CatalogRow, unit: Uuid, state: &SharedState) -> Vec<String> {
    let Some(key) = &row.steward_position else { return Vec::new() };
    let spec = ApproverSpec::Position { key: key.clone(), from: Origin::Workspace, walk_up: true };
    crate::service::approvers::resolved_approvers(state, &spec, unit, &[], None)
}

fn may_accept(caller: &AuthUser, row: &CatalogRow, unit: Uuid, state: &SharedState) -> bool {
    is_admin_or_compliance(caller, state) || stewards_at(row, unit, state).iter().any(|s| s.eq_ignore_ascii_case(&caller.eppn))
}

fn refused() -> ServiceError {
    ServiceError::forbidden("Only the steward of the receiving unit, a Platform Admin or a compliance officer accepts a unit's place in an audience")
}

/// `GET /data/datasets/{id}/audience/pending`: the unit values that wait for their unit to accept, which the caller may accept. A Platform Admin
/// or a compliance officer sees all of them. A steward sees the ones for the units they are steward of.
pub fn pending(caller: &AuthUser, id: &str, state: &SharedState) -> Result<Value, ServiceError> {
    let row = dataset_of(id, state)?;
    let waiting = state.repository.audience_waiting(&row.app_slug, &row.table_id, LISTED).map_err(storage)?;
    let admin = is_admin_or_compliance(caller, state);
    let mut by_unit: std::collections::HashMap<Uuid, bool> = std::collections::HashMap::new();
    let mut out = Vec::new();
    for w in waiting {
        let allowed = *by_unit.entry(w.unit).or_insert_with(|| admin || stewards_at(&row, w.unit, state).iter().any(|s| s.eq_ignore_ascii_case(&caller.eppn)));
        if allowed {
            out.push(json!({ "record_id": w.record_id, "field": w.field, "unit": w.unit.to_string() }));
        }
    }
    Ok(json!({ "dataset_id": row.id, "pending": out }))
}

/// `POST /data/datasets/{id}/records/{record_id}/audience/accept`. The body names the `unit` and a `reason`. The steward of the receiving unit
/// accepts the unit's place in every audience field of the row that asks for it. From then the unit's people read the row, cut to the fields of
/// the policy. The acceptance and one `AccessGrantChanged` entry commit together. Accepting what waits no longer is a conflict.
pub fn accept(caller: &AuthUser, id: &str, record_id: &str, body: &Value, state: &SharedState) -> Result<Value, ServiceError> {
    let row = dataset_of(id, state)?;
    let unit = body.get("unit").and_then(Value::as_str).and_then(|u| Uuid::parse_str(u.trim()).ok()).ok_or_else(|| bad("unit is the id of the unit that accepts"))?;
    let reason = reason_of(body)?;
    if !may_accept(caller, &row, unit, state) {
        return Err(refused());
    }
    let rules = scope::audience_rules(&row.app_slug, &row.table_id, state);
    if !rules.iter().any(|r| r.unit_acceptance) {
        return Err(bad("This dataset does not ask a unit to accept before it gains access"));
    }
    // What the acceptance opens: the fields of the policy for each field of the row that named the unit.
    let (dataset, record, by, why) = (row.id.clone(), record_id.to_string(), caller.eppn.to_lowercase(), reason.clone());
    let ledger_rules = rules.clone();
    let app = row.app_slug.clone();
    let done = state
        .repository
        .accept_audience(&row.app_slug, &row.table_id, record_id, unit, &caller.eppn, move |fields: &[String]| {
            let hit: Vec<&scaffoldry_engine::AudienceRule> = ledger_rules.iter().filter(|r| fields.contains(&r.field)).collect();
            let write = hit.iter().any(|r| r.grants == AudienceGrant::Write);
            let reach: BTreeSet<&String> = hit.iter().flat_map(|r| r.fields.iter()).collect();
            AppendDecisionParams {
                principal: by.clone(),
                organization_code: "DIV-ADMIN-CONSOLE".to_string(),
                app_slug: Some(app),
                decision_type: DecisionType::AccessGrantChanged,
                oscal_control_id: "AC-02".to_string(),
                rationale: why.chars().take(500).collect(),
                payload: json!({
                    "op": "create", "source": "audience", "dataset": dataset, "record": record, "holder_kind": "unit", "holder": unit.to_string(),
                    "access": if write { "write" } else { "read" }, "audience_fields": fields, "fields": reach, "accepted_by": by,
                }),
            }
        })
        .map_err(storage)?;
    match done {
        Accepted::NotNamed => Err(ServiceError::NotFound("This row does not name that unit".to_string())),
        Accepted::NothingWaits => Err(ServiceError::Conflict("That unit's place in this row's audience is accepted already, or the policy never asked it to accept".to_string())),
        Accepted::Done(n) => Ok(json!({ "dataset_id": row.id, "record_id": record_id, "unit": unit.to_string(), "accepted_fields": n })),
    }
}

// ---------------------------------------------------------------------------------------------------------------------
// Conditions
// ---------------------------------------------------------------------------------------------------------------------

/// The closed list of conditions in settings.
fn listed(state: &SharedState) -> Vec<scaffoldry_engine::sensitivity::Condition> {
    let value = state.settings.read().unwrap_or_else(|p| p.into_inner()).get("sensitivity.conditions").cloned().unwrap_or_else(|| json!([]));
    scaffoldry_engine::sensitivity::validate_conditions(&value).unwrap_or_default()
}

fn shown(person: &str, c: crate::repository_audience::Conditions) -> Value {
    json!({ "person": person.to_lowercase(), "conditions": c.until, "sources": c.sources })
}

/// `GET /admin/people/{eppn}/conditions`: the conditions on a person's record, with the owner and source of each.
pub fn conditions(caller: &AuthUser, eppn: &str, state: &SharedState) -> Result<Value, ServiceError> {
    crate::service::admin::require_platform_admin_or_compliance(caller, state)?;
    let found = state.repository.conditions_of(eppn).map_err(storage)?.ok_or_else(|| ServiceError::NotFound("That person is not in the directory".to_string()))?;
    Ok(shown(eppn, found))
}

/// A date as a condition holds it: `YYYY-MM-DD`, the last day the condition holds.
fn day_of(v: &Value) -> Result<String, ServiceError> {
    let text = v.as_str().map(str::trim).unwrap_or("");
    let ok = text.len() == 10 && chrono::NaiveDate::parse_from_str(text, "%Y-%m-%d").is_ok();
    if !ok {
        return Err(bad("until is a date written YYYY-MM-DD, the last day the condition holds, or null to clear it"));
    }
    Ok(text.to_string())
}

/// `PUT /admin/people/{eppn}/conditions/{id}`. A Platform Admin or a compliance officer sets the last day a condition holds for a person, with the
/// condition's `owner` (the position whose steward maintains it, from the list in settings) and a `source` (`manual` or `sync`), or clears it
/// with `until: null`. A date already past is recorded as it is, because a lapse is a fact. The condition must be on the list in settings. The change
/// and one `UserAccessChanged` entry commit together.
pub fn set_condition(caller: &AuthUser, eppn: &str, id: &str, body: &Value, state: &SharedState) -> Result<Value, ServiceError> {
    crate::service::admin::require_platform_admin_or_compliance(caller, state)?;
    let condition = listed(state).into_iter().find(|c| c.id == id).ok_or_else(|| bad(format!("'{}' is not a condition that settings list (sensitivity.conditions)", id.chars().take(64).collect::<String>())))?;
    let reason = reason_of(body)?;
    let until = match body.get("until") {
        Some(Value::Null) => None,
        Some(v) => Some(day_of(v)?),
        None => return Err(bad("until is required: a date written YYYY-MM-DD, or null to clear it")),
    };
    let owner = body.get("owner").and_then(Value::as_str).map(str::trim).unwrap_or(&condition.owner).to_string();
    if owner != condition.owner {
        return Err(bad(format!("The owner of '{}' is '{}'", condition.id, condition.owner)));
    }
    let source = body.get("source").and_then(Value::as_str).map(str::trim).unwrap_or("manual").to_string();
    if source != "manual" && source != "sync" {
        return Err(bad("source is manual or sync"));
    }
    let now = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
    let (by, who, cid, op_until, sourced) = (caller.eppn.to_lowercase(), eppn.to_lowercase(), condition.id.clone(), until.clone(), (owner.clone(), source.clone()));
    let found = state
        .repository
        .set_condition(eppn, id, until.as_deref(), source_of(&owner, &source, &caller.eppn.to_lowercase(), &now), move |was: Option<&str>| AppendDecisionParams {
            principal: by.clone(),
            organization_code: "DIV-ADMIN-CONSOLE".to_string(),
            app_slug: None,
            decision_type: DecisionType::UserAccessChanged,
            oscal_control_id: "AC-02".to_string(),
            rationale: reason.chars().take(500).collect(),
            payload: json!({ "op": if op_until.is_some() { "set" } else { "clear" }, "source": "condition", "person": who, "condition": cid, "from": was, "until": op_until, "owner": sourced.0, "condition_source": sourced.1, "set_by": by }),
        })
        .map_err(storage)?
        .ok_or_else(|| ServiceError::NotFound("That person is not in the directory".to_string()))?;
    Ok(shown(eppn, found))
}
