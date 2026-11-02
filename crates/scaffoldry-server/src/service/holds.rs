//! Lifecycle phase 4: placing, listing and releasing legal holds, for a Platform Admin or `compliance`. Anyone else is told
//! nothing, not even that a hold exists. The rule that keeps a held record is the database function `is_held`.

use crate::holds::{sentence_of, Scope};
use crate::service::admin::require_platform_admin_or_compliance;
use crate::service::ServiceError;
use crate::state::{AuthUser, SharedState};
use scaffoldry_core::ledger::DecisionType;
use serde_json::{json, Value};
use uuid::Uuid;

/// What the record drawer shows a Platform Admin or `compliance` caller.
pub const MARKER: &str = "Under legal hold";

fn internal(e: impl std::fmt::Display) -> ServiceError {
    ServiceError::Internal(e.to_string())
}

fn text(body: &Value, key: &str, max: usize) -> Result<String, ServiceError> {
    let v = body.get(key).and_then(Value::as_str).unwrap_or("").trim().to_string();
    if v.chars().count() > max {
        return Err(ServiceError::BadRequest(format!("'{key}' may be at most {max} characters")));
    }
    Ok(v)
}

fn scope_of(body: &Value) -> Result<Scope, ServiceError> {
    let scope: Scope = serde_json::from_value(body.clone()).map_err(|e| ServiceError::BadRequest(format!("The hold's scope is not a scope: {e}")))?;
    let problems = scope.problems();
    if !problems.is_empty() {
        return Err(ServiceError::BadRequest(problems.join("; ")));
    }
    Ok(scope)
}

fn ledger(caller: &AuthUser, reason: &str, payload: &Value, state: &SharedState) -> Result<(), ServiceError> {
    state
        .append_ledger_entry(crate::state::RecordDecisionInput {
            principal: caller.eppn.clone(),
            organization_code: String::new(),
            app_slug: payload.get("app_slug").and_then(Value::as_str).map(str::to_string),
            decision_type: DecisionType::LegalHoldChanged,
            oscal_control_id: "AU-11".to_string(),
            rationale: reason.to_string(),
            payload,
        })
        .map(|_| ())
        .map_err(|e| ServiceError::Internal(format!("The ledger could not record this, so nothing changed: {e}")))
}

pub fn list(caller: &AuthUser, state: &SharedState) -> Result<Value, ServiceError> {
    require_platform_admin_or_compliance(caller, state)?;
    let holds: Vec<Value> = state
        .repository
        .list_holds()
        .map_err(internal)?
        .into_iter()
        .map(|(h, n)| {
            let mut v = json!(h);
            v["scope"] = json!(h.scope.sentence());
            v["count"] = json!(n);
            v["active"] = json!(h.released_at.is_none());
            v
        })
        .collect();
    Ok(json!({ "holds": holds }))
}

pub fn preview(caller: &AuthUser, body: &Value, state: &SharedState) -> Result<Value, ServiceError> {
    require_platform_admin_or_compliance(caller, state)?;
    let scope = scope_of(body)?;
    let job = crate::jobs::enqueue(&state.repository, "hold_preview", json!({ "scope": scope }), &caller.eppn, None).map_err(internal)?;
    Ok(json!({ "job_id": job }))
}

/// Places a hold: the ledger entry first, then the row, then the job that pins its records when the scope needs a list.
pub fn place(caller: &AuthUser, body: &Value, state: &SharedState) -> Result<Value, ServiceError> {
    require_platform_admin_or_compliance(caller, state)?;
    let scope = scope_of(body)?;
    let name = text(body, "name", 120)?;
    let case_ref = text(body, "case_ref", 120)?;
    let reason = text(body, "reason", 2_000)?;
    if name.is_empty() || reason.is_empty() {
        return Err(ServiceError::BadRequest("A hold needs a 'name' and a 'reason'".to_string()));
    }
    if let Some(slug) = scope.app_slug.as_deref() {
        state.repository.get_app_manifest(slug).map_err(internal)?.ok_or_else(|| ServiceError::BadRequest(format!("'app_slug' names no app: '{slug}'")))?;
    }
    if let Some(ws) = scope.workspace_id.as_deref() {
        if !state.workspaces.read().map_err(internal)?.contains_key(ws) {
            return Err(ServiceError::BadRequest(format!("'workspace_id' names no workspace: '{ws}'")));
        }
    }
    ledger(caller, &reason, &json!({ "action": "placed", "name": name, "case_ref": case_ref, "scope": scope, "app_slug": scope.app_slug }), state)?;
    let id = state.repository.insert_hold(&name, &case_ref, &reason, &scope, &caller.eppn).map_err(internal)?;
    let job = if scope.pins() {
        Some(crate::jobs::enqueue(&state.repository, "pin_hold", json!({ "hold_id": id }), &caller.eppn, None).map_err(internal)?)
    } else {
        None
    };
    let hold = state.repository.get_hold(id).map_err(internal)?;
    let mut out = json!(hold);
    out["scope"] = json!(scope.sentence());
    out["pin_job_id"] = json!(job);
    Ok(out)
}

/// Releases a hold. A second person: not the one who placed it. The ledger entry first.
pub fn release(caller: &AuthUser, id: &str, body: &Value, state: &SharedState) -> Result<Value, ServiceError> {
    require_platform_admin_or_compliance(caller, state)?;
    let id = Uuid::parse_str(id).map_err(|_| ServiceError::NotFound("Hold not found".to_string()))?;
    let hold = state.repository.get_hold(id).map_err(internal)?.ok_or_else(|| ServiceError::NotFound("Hold not found".to_string()))?;
    if hold.released_at.is_some() {
        return Err(ServiceError::conflict("This hold is released already"));
    }
    if hold.created_by.eq_ignore_ascii_case(&caller.eppn) {
        return Err(ServiceError::forbidden("A hold is released by a second person: someone other than the person who placed it"));
    }
    let reason = text(body, "reason", 2_000)?;
    if reason.is_empty() {
        return Err(ServiceError::BadRequest("A release needs a 'reason'".to_string()));
    }
    ledger(caller, &reason, &json!({ "action": "released", "hold_id": id, "name": hold.name, "case_ref": hold.case_ref, "app_slug": hold.scope.app_slug }), state)?;
    if !state.repository.release_hold(id, &caller.eppn).map_err(internal)? {
        return Err(ServiceError::conflict("Another request released this hold first"));
    }
    Ok(json!({ "id": id, "released": true }))
}

/// The drawer's marker. Anyone but a Platform Admin or `compliance` gets 404, so an editor cannot learn a hold exists.
pub fn record(caller: &AuthUser, app_slug: &str, record_id: &str, state: &SharedState) -> Result<Value, ServiceError> {
    if require_platform_admin_or_compliance(caller, state).is_err() {
        return Err(ServiceError::NotFound("Not found".to_string()));
    }
    let names = state.repository.holds_of_record(app_slug, record_id).map_err(internal)?;
    let held = state.repository.is_held(app_slug, record_id).map_err(internal)?;
    Ok(json!({ "held": held, "marker": if held { Value::String(MARKER.to_string()) } else { Value::Null }, "holds": names }))
}

pub fn sentence(records: i64, apps: i64) -> String {
    sentence_of(records, apps)
}
