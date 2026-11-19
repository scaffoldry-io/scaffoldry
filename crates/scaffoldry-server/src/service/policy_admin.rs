//! Admin console phase 5: the institutional policy as stored, versioned, testable data.
//!
//! A draft is checked when it is stored. It runs against every test case before it can be switched on, and the person who wrote
//! it cannot switch it on. Switching on writes the ledger entry, retires the old version and activates the new one in one
//! transaction, and every server then reads the new policy through the `policy` counter.

use crate::repository::{AppendDecisionParams, RepositoryError};
use crate::repository_policy::{PolicyVersionRow, StoredCase};
use crate::service::admin::{admin_write, decode_cursor, encode_cursor, require_platform_admin};
use crate::service::ServiceError;
use crate::state::{AuthUser, SharedState};
use scaffoldry_core::ledger::DecisionType;
use scaffoldry_policy::cases::{run_case, CaseOutcome, PolicyCase};
use scaffoldry_policy::{check_source, ScaffoldryPolicyEngine};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashMap};
use uuid::Uuid;

/// The longest policy source a draft may hold, in bytes.
pub const MAX_SOURCE: usize = 200_000;
const MAX_LIMIT: usize = 200;

fn storage(e: RepositoryError) -> ServiceError {
    match e {
        RepositoryError::NotFound(m) => ServiceError::NotFound(m),
        RepositoryError::Conflict(m) => ServiceError::Conflict(m),
        other => ServiceError::StorageUnavailable(other.to_string()),
    }
}

fn bad(m: impl Into<String>) -> ServiceError {
    ServiceError::BadRequest(m.into())
}

fn sha256_hex(text: &str) -> String {
    Sha256::digest(text.as_bytes()).iter().map(|b| format!("{b:02x}")).collect()
}

fn id_of(raw: &str) -> Result<Uuid, ServiceError> {
    Uuid::parse_str(raw).map_err(|_| ServiceError::NotFound("No such item".to_string()))
}

fn version_json(v: &PolicyVersionRow, with_source: bool) -> Value {
    let mut out = json!({
        "id": v.id, "version": v.version, "status": v.status, "note": v.note, "created_by": v.created_by,
        "created_at": v.created_at, "activated_by": v.activated_by, "activated_at": v.activated_at,
    });
    if with_source {
        out["source"] = json!(v.source);
    }
    out
}

fn case_json(c: &StoredCase) -> Value {
    json!({ "id": c.id, "name": c.name, "principal": c.principal, "action": c.action, "resource": c.resource, "expected": c.expected, "created_by": c.created_by, "created_at": c.created_at })
}

/// `limit` and `cursor` from a query. The limit is at most 200.
fn paging(q: &HashMap<String, String>) -> Result<(usize, usize), ServiceError> {
    let limit = match q.get("limit") {
        Some(l) => l.parse::<usize>().map_err(|_| bad("limit must be a number"))?.clamp(1, MAX_LIMIT),
        None => 50,
    };
    let offset = match q.get("cursor") {
        Some(c) if !c.is_empty() => decode_cursor(c)?,
        _ => 0,
    };
    Ok((limit, offset))
}

fn reason_of(body: &Value) -> Result<String, ServiceError> {
    let reason = body.get("reason").and_then(Value::as_str).map(str::trim).unwrap_or("");
    if reason.is_empty() || reason.chars().count() > 500 {
        return Err(bad("A reason between 1 and 500 characters is required"));
    }
    Ok(reason.to_string())
}

/// Parses and type-checks a source. Every problem comes back with its line and column.
fn checked(source: &str) -> Result<(), ServiceError> {
    if source.trim().is_empty() {
        return Err(bad("The policy source is empty"));
    }
    if source.len() > MAX_SOURCE {
        return Err(ServiceError::TooLarge(format!("A policy source is at most {MAX_SOURCE} bytes")));
    }
    let problems = check_source(source);
    if problems.is_empty() {
        return Ok(());
    }
    let fields: BTreeMap<String, String> = problems
        .iter()
        .enumerate()
        .map(|(i, p)| (format!("{:03}", i + 1), format!("line {}, column {}: {}", p.line, p.column, p.message)))
        .collect();
    Err(ServiceError::Invalid { message: format!("The policy has {} problem{}", fields.len(), if fields.len() == 1 { "" } else { "s" }), fields })
}

fn version_row(state: &SharedState, raw_id: &str) -> Result<PolicyVersionRow, ServiceError> {
    state.repository.get_policy_version(id_of(raw_id)?).map_err(storage)?.ok_or_else(|| ServiceError::NotFound("No such policy version".to_string()))
}

fn engine_of(row: &PolicyVersionRow) -> Result<ScaffoldryPolicyEngine, ServiceError> {
    ScaffoldryPolicyEngine::new(&row.source).map_err(|e| bad(format!("Version {} does not run: {e}", row.version)))
}

fn outcomes(engine: &ScaffoldryPolicyEngine, cases: &[StoredCase]) -> Vec<CaseOutcome> {
    cases.iter().map(|c| run_case(engine, &c.as_case())).collect()
}

pub fn active(caller: &AuthUser, state: &SharedState) -> Result<Value, ServiceError> {
    require_platform_admin(caller, state)?;
    let row = state.repository.active_policy().map_err(storage)?.ok_or_else(|| ServiceError::Internal("No policy version is active".to_string()))?;
    Ok(json!({ "version": version_json(&row, false), "source": row.source, "policies": state.policy().policy_summaries() }))
}

pub fn versions(caller: &AuthUser, q: &HashMap<String, String>, state: &SharedState) -> Result<Value, ServiceError> {
    require_platform_admin(caller, state)?;
    let (limit, offset) = paging(q)?;
    let mut rows = state.repository.list_policy_versions(limit as i64 + 1, offset as i64).map_err(storage)?;
    let more = rows.len() > limit;
    rows.truncate(limit);
    Ok(json!({ "rows": rows.iter().map(|r| version_json(r, false)).collect::<Vec<_>>(), "next_cursor": more.then(|| encode_cursor(offset + limit)) }))
}

pub fn version(caller: &AuthUser, id: &str, state: &SharedState) -> Result<Value, ServiceError> {
    require_platform_admin(caller, state)?;
    Ok(version_json(&version_row(state, id)?, true))
}

fn note_of(body: &Value, fallback: String) -> Result<String, ServiceError> {
    match body.get("note").and_then(Value::as_str).map(str::trim) {
        Some(n) if n.chars().count() > 500 => Err(bad("A note is at most 500 characters")),
        Some(n) if !n.is_empty() => Ok(n.to_string()),
        _ => Ok(fallback),
    }
}

/// Checks a source without storing it. Always answers with the problems, an empty list when the source is good.
pub fn validate(caller: &AuthUser, body: &Value, state: &SharedState) -> Result<Value, ServiceError> {
    require_platform_admin(caller, state)?;
    let source = body.get("source").and_then(Value::as_str).ok_or_else(|| bad("source is required"))?;
    if source.len() > MAX_SOURCE {
        return Err(ServiceError::TooLarge(format!("A policy source is at most {MAX_SOURCE} bytes")));
    }
    let problems = if source.trim().is_empty() { Vec::new() } else { check_source(source) };
    Ok(json!({ "ok": !source.trim().is_empty() && problems.is_empty(), "problems": problems }))
}

pub fn create_draft(caller: &AuthUser, body: &Value, state: &SharedState) -> Result<Value, ServiceError> {
    require_platform_admin(caller, state)?;
    let source = body.get("source").and_then(Value::as_str).ok_or_else(|| bad("source is required"))?;
    checked(source)?;
    let note = note_of(body, String::new())?;
    let row = state.repository.insert_policy_draft(source, &note, &caller.eppn).map_err(storage)?;
    Ok(version_json(&row, true))
}

pub fn clone_version(caller: &AuthUser, id: &str, body: &Value, state: &SharedState) -> Result<Value, ServiceError> {
    require_platform_admin(caller, state)?;
    let from = version_row(state, id)?;
    let note = note_of(body, format!("Cloned from version {}", from.version))?;
    let row = state.repository.insert_policy_draft(&from.source, &note, &caller.eppn).map_err(storage)?;
    Ok(version_json(&row, true))
}

pub fn run_cases(caller: &AuthUser, id: &str, state: &SharedState) -> Result<Value, ServiceError> {
    require_platform_admin(caller, state)?;
    let row = version_row(state, id)?;
    let results = outcomes(&engine_of(&row)?, &state.repository.list_policy_cases().map_err(storage)?);
    let failed = results.iter().filter(|r| !r.pass).count();
    Ok(json!({ "version": version_json(&row, false), "results": results, "passed": results.len() - failed, "failed": failed }))
}

pub fn activate(caller: &AuthUser, id: &str, body: &Value, state: &SharedState) -> Result<Value, ServiceError> {
    require_platform_admin(caller, state)?;
    let reason = reason_of(body)?;
    let row = version_row(state, id)?;
    if row.status != "Draft" {
        return Err(ServiceError::Conflict(format!("Version {} is {}, not a draft. Clone it to make a draft.", row.version, row.status)));
    }
    // A different person, always. The setup identity is a different person from any author. No setting relaxes this.
    if row.created_by.eq_ignore_ascii_case(&caller.eppn) {
        return Err(ServiceError::forbidden("You wrote this draft, so a different Platform Admin must activate it"));
    }
    let failing: Vec<String> = outcomes(&engine_of(&row)?, &state.repository.list_policy_cases().map_err(storage)?).into_iter().filter(|r| !r.pass).map(|r| r.name).collect();
    if !failing.is_empty() {
        return Err(ServiceError::Conflict(format!("These test cases fail: {}", failing.join("; "))));
    }
    let (principal, rationale) = (caller.eppn.clone(), reason);
    let done = state
        .repository
        .activate_policy(row.id, &caller.eppn, move |target, previous| AppendDecisionParams {
            principal,
            organization_code: "DIV-ADMIN-CONSOLE".to_string(),
            app_slug: None,
            decision_type: DecisionType::PolicyRevision,
            oscal_control_id: "AC-03".to_string(),
            rationale,
            payload: json!({ "version": target.version, "source_sha256": sha256_hex(&target.source), "previous_version": previous }),
        })
        .map_err(storage)?;
    crate::service::reload_after_write(state)?;
    let row = version_row(state, id)?;
    Ok(json!({ "version": version_json(&row, false), "previous_version": done.previous, "ledger_sequence": done.entry.sequence }))
}

pub fn cases(caller: &AuthUser, q: &HashMap<String, String>, state: &SharedState) -> Result<Value, ServiceError> {
    require_platform_admin(caller, state)?;
    let (limit, offset) = paging(q)?;
    let all = state.repository.list_policy_cases().map_err(storage)?;
    let page: Vec<Value> = all.iter().skip(offset).take(limit).map(case_json).collect();
    let more = all.len() > offset + limit;
    Ok(json!({ "rows": page, "next_cursor": more.then(|| encode_cursor(offset + limit)) }))
}

fn entity_of(body: &Value, field: &str) -> Result<Value, ServiceError> {
    let v = body.get(field).filter(|v| v.is_object()).ok_or_else(|| bad(format!("{field} is required and is an entity: {{ \"type\", \"id\", \"attrs\" }}")))?;
    for key in ["type", "id"] {
        if v.get(key).and_then(Value::as_str).map(str::is_empty).unwrap_or(true) {
            return Err(bad(format!("{field} needs a {key}")));
        }
    }
    Ok(v.clone())
}

pub fn add_case(caller: &AuthUser, body: &Value, state: &SharedState) -> Result<Value, ServiceError> {
    require_platform_admin(caller, state)?;
    let text = |k: &str, max: usize| -> Result<String, ServiceError> {
        let v = body.get(k).and_then(Value::as_str).map(str::trim).unwrap_or("");
        if v.is_empty() || v.chars().count() > max {
            return Err(bad(format!("{k} is required and is at most {max} characters")));
        }
        Ok(v.to_string())
    };
    let case = PolicyCase { name: text("name", 255)?, principal: entity_of(body, "principal")?, action: text("action", 64)?, resource: entity_of(body, "resource")?, expected: text("expected", 5)? };
    if case.expected != "Allow" && case.expected != "Deny" {
        return Err(bad("expected is Allow or Deny"));
    }
    // A case that cannot be asked is no use. The running engine says why.
    let probe = run_case(&state.policy(), &case);
    if probe.actual == "Error" {
        return Err(bad(format!("This case cannot be asked: {}", probe.error)));
    }
    let reason = reason_of(body)?;
    let id = Uuid::new_v4();
    admin_write(state, caller, DecisionType::PolicyRevision, "AC-03", &reason, &json!({ "change": "case_added", "case_id": id, "name": case.name }), || {
        state.repository.insert_policy_case(id, &case, &caller.eppn).map_err(storage)
    })?;
    let stored = state.repository.get_policy_case(id).map_err(storage)?.ok_or_else(|| ServiceError::Internal("The case was not stored".to_string()))?;
    Ok(case_json(&stored))
}

pub fn case(caller: &AuthUser, id: &str, state: &SharedState) -> Result<Value, ServiceError> {
    require_platform_admin(caller, state)?;
    let c = state.repository.get_policy_case(id_of(id)?).map_err(storage)?.ok_or_else(|| ServiceError::NotFound("No such test case".to_string()))?;
    Ok(case_json(&c))
}

pub fn remove_case(caller: &AuthUser, id: &str, body: &Value, state: &SharedState) -> Result<Value, ServiceError> {
    require_platform_admin(caller, state)?;
    let id = id_of(id)?;
    let c = state.repository.get_policy_case(id).map_err(storage)?.ok_or_else(|| ServiceError::NotFound("No such test case".to_string()))?;
    let reason = reason_of(body)?;
    admin_write(state, caller, DecisionType::PolicyRevision, "AC-03", &reason, &json!({ "change": "case_removed", "case_id": id, "name": c.name }), || {
        state.repository.delete_policy_case(id).map_err(storage)
    })?;
    Ok(json!({ "removed": id, "name": c.name }))
}

pub fn simulate(caller: &AuthUser, body: &Value, state: &SharedState) -> Result<Value, ServiceError> {
    require_platform_admin(caller, state)?;
    let principal = entity_of(body, "principal")?;
    let resource = entity_of(body, "resource")?;
    let action = body.get("action").and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty()).ok_or_else(|| bad("action is required"))?;
    let (engine, version) = match body.get("version_id").and_then(Value::as_str).filter(|s| !s.is_empty()) {
        Some(raw) => {
            let row = version_row(state, raw)?;
            (std::sync::Arc::new(engine_of(&row)?), row.version)
        }
        None => {
            let row = state.repository.active_policy().map_err(storage)?;
            (state.policy(), row.map(|r| r.version).unwrap_or(0))
        }
    };
    let result = engine.is_allowed(&principal, action, &resource).map_err(|e| bad(e.to_string()))?;
    Ok(json!({ "version": version, "decision": result.decision, "reasons": result.reasons, "diagnostics": result.diagnostics, "deciding_policy": result.deciding_policy }))
}
