//! Workflows phase 10: a flow's runs, one run's timeline, retrying a failed run, and comparing two versions.
//!
//! The timeline is read from `process_events`. Each step says what it did in words and names the records it wrote by
//! id, with a link only when the viewer may open the app. No field value is stored or shown. A failed run is retried
//! by an owner of the flow's workspace or an administrator, with a reason, on the ledger. A run that failed outside
//! its grants waits for a new version of the flow.

use crate::service::access::{authorize_app, authorize_workspace, AppAction};
use crate::service::{flow_validate, flows, ServiceError};
use crate::state::{AuthUser, RecordDecisionInput, SharedState};
use scaffoldry_core::ledger::DecisionType;
use scaffoldry_core::{AutomationRule, ProcessInstance, ProcessStatus, TriggerEvent};
use serde_json::{json, Value};
use std::collections::HashMap;

const PAGE: i64 = 50;

fn internal(e: impl std::fmt::Display) -> ServiceError {
    ServiceError::Internal(e.to_string())
}

/// The live flow, and the caller may manage it: the same right as editing it.
fn flow_for(caller: &AuthUser, flow_id: &str, state: &SharedState) -> Result<AutomationRule, ServiceError> {
    let rule = flows::find_flow(state, flow_id).ok_or_else(|| ServiceError::NotFound(format!("There is no live flow '{flow_id}'")))?;
    let manifest = flow_validate::manifest_of(&rule, state)?;
    flow_validate::authorize(caller, &rule, &manifest, state)?;
    Ok(rule)
}

/// What started a run, in words.
fn trigger_words(rule: &AutomationRule, record_id: &str) -> String {
    let rec = if record_id.is_empty() { "a record".to_string() } else { format!("record {}", &record_id[..record_id.len().min(8)]) };
    match &rule.trigger {
        TriggerEvent::RecordCreated => format!("Started when {rec} was created"),
        TriggerEvent::RecordUpdated => format!("Started when {rec} was updated"),
        TriggerEvent::FieldChanged { field_name } => format!("Started when {field_name} changed on {rec}"),
        TriggerEvent::StatusChanged { to_status } => format!("Started when the status of {rec} changed to {to_status}"),
        TriggerEvent::Schedule { .. } => "Started on its schedule".to_string(),
        TriggerEvent::Called { .. } => "Started by another flow".to_string(),
        _ => "Started by another flow's outcome".to_string(),
    }
}

fn status_of(s: ProcessStatus) -> &'static str {
    match s {
        ProcessStatus::Waiting => "Waiting",
        ProcessStatus::Completed => "Completed",
        ProcessStatus::Rejected => "Rejected",
        ProcessStatus::Failed => "Failed",
    }
}

/// `GET /flows/{id}/runs?status=&from=&to=&cursor=`: one row per run, newest first, with the failure rate over the last
/// seven days and the circuit breaker's state.
pub fn runs(caller: &AuthUser, flow_id: &str, q: &HashMap<String, String>, state: &SharedState) -> Result<Value, ServiceError> {
    let rule = flow_for(caller, flow_id, state)?;
    let pick = |k: &str| q.get(k).filter(|v| !v.is_empty()).cloned();
    let cursor = pick("cursor").and_then(|c| c.split_once('|').map(|(a, i)| (a.to_string(), i.to_string())));
    let rows = state.repository.flow_runs(&rule.id, pick("status"), pick("from"), pick("to"), cursor, PAGE).map_err(internal)?;
    let mut out = Vec::new();
    for row in &rows {
        let Ok(inst) = serde_json::from_value::<ProcessInstance>(row.instance.clone()) else { continue };
        let started = chrono::DateTime::parse_from_rfc3339(&inst.started_at).ok().map(|t| t.with_timezone(&chrono::Utc));
        let duration_ms = match (started, row.ended_at) {
            (Some(s), Some(e)) => Some((e - s).num_milliseconds().max(0)),
            _ => None,
        };
        out.push(json!({
            "instance_id": inst.id,
            "status": status_of(inst.status),
            "started_at": inst.started_at,
            "duration_ms": duration_ms,
            "trigger": trigger_words(&rule, &inst.record_id),
            "started_by": inst.started_by,
            "writes": row.end.as_ref().and_then(|e| e.get("writes")).cloned().unwrap_or(Value::Null),
            "version": inst.rule_version,
        }));
    }
    let next = (rows.len() as i64 == PAGE)
        .then(|| out.last().map(|r| format!("{}|{}", r["started_at"].as_str().unwrap_or_default(), r["instance_id"].as_str().unwrap_or_default())))
        .flatten();
    let (total, failed) = state.repository.flow_run_counts(&rule.id, chrono::Utc::now() - chrono::Duration::days(7)).map_err(internal)?;
    let rate = if total == 0 {
        "No run ended in the last 7 days.".to_string()
    } else {
        format!("{failed} of {total} {} failed in the last 7 days ({}%).", if total == 1 { "run" } else { "runs" }, (failed * 100 + total / 2) / total)
    };
    let (in_a_row, off) = flows::breaker(state, &rule.id);
    let breaker = if off {
        format!("The flow is turned off after {} failed runs in a row. A new version turns it on again.", flows::BREAKER_FAILURES)
    } else {
        format!("The flow is on. {in_a_row} of the last runs failed in a row; it turns off at {}.", flows::BREAKER_FAILURES)
    };
    Ok(json!({
        "runs": out,
        "next_cursor": next,
        "last_7_days": { "runs": total, "failed": failed, "sentence": rate },
        "breaker": { "failures_in_a_row": in_a_row, "off": off, "sentence": breaker },
    }))
}

fn instance(state: &SharedState, id: &str) -> Result<ProcessInstance, ServiceError> {
    state.repository.get_process_instance(id).map_err(internal)?.ok_or_else(|| ServiceError::NotFound(format!("There is no run '{id}'")))
}

/// `GET /flows/runs/{instance_id}`: each step of the latest attempt with its time, outcome and effects, the first error,
/// and the retries. Records are named by id, with a link only when the viewer may open the app.
pub fn timeline(caller: &AuthUser, instance_id: &str, state: &SharedState) -> Result<Value, ServiceError> {
    let inst = instance(state, instance_id)?;
    let rule = flow_for(caller, &inst.rule_id, state)?;
    let paths = flow_validate::step_paths(&rule.steps);
    let events = state.repository.instance_events(&inst.id).map_err(internal)?;
    let mut readable: HashMap<String, bool> = HashMap::new();
    let mut may_open = |app: &str| *readable.entry(app.to_string()).or_insert_with(|| authorize_app(caller, app, AppAction::Read, state).is_ok());

    // Only the latest attempt's steps: a retry starts the list again.
    let last_retry = events.iter().rposition(|(k, ..)| k == "flow_retry");
    let mut previous = chrono::DateTime::parse_from_rfc3339(&inst.started_at).ok().map(|t| t.with_timezone(&chrono::Utc));
    let mut steps = Vec::new();
    let mut first_error = Value::Null;
    let mut retries = Vec::new();
    for (i, (kind, step, detail, at)) in events.iter().enumerate() {
        match kind.as_str() {
            "flow_retry" => {
                retries.push(json!({ "by": detail.get("by"), "reason": detail.get("reason"), "at": at.to_rfc3339() }));
                previous = Some(*at);
            }
            "flow_step" if last_retry.is_none_or(|r| i > r) => {
                let step = step.clone().unwrap_or_default();
                let effects: Vec<Value> = detail
                    .get("effects")
                    .and_then(Value::as_array)
                    .map(|list| {
                        list.iter()
                            .map(|e| {
                                let records: Vec<Value> = e
                                    .get("records")
                                    .and_then(Value::as_array)
                                    .map(|rs| {
                                        rs.iter()
                                            .map(|r| {
                                                let app = r.get("app").and_then(Value::as_str).unwrap_or_default();
                                                let id = r.get("id").and_then(Value::as_str).unwrap_or_default();
                                                let link = may_open(app).then(|| format!("/apps/{app}/records/{id}"));
                                                json!({ "app": app, "table": r.get("table"), "id": id, "link": link })
                                            })
                                            .collect()
                                    })
                                    .unwrap_or_default();
                                json!({ "sentence": e.get("sentence"), "records": records })
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                let outcome = detail.get("outcome").and_then(Value::as_str).unwrap_or("done");
                if outcome == "failed" && first_error.is_null() {
                    first_error = json!({ "step": step, "message": detail.get("error") });
                }
                steps.push(json!({
                    "step": step,
                    "path": paths.get(&step),
                    "outcome": outcome,
                    "started_at": previous.map(|t| t.to_rfc3339()),
                    "ended_at": at.to_rfc3339(),
                    "effects": effects,
                }));
                previous = Some(*at);
            }
            _ => {}
        }
    }
    Ok(json!({
        "instance_id": inst.id,
        "flow": rule.id,
        "version": inst.rule_version,
        "status": status_of(inst.status),
        "started_at": inst.started_at,
        "started_by": inst.started_by,
        "trigger": trigger_words(&rule, &inst.record_id),
        "steps": steps,
        "first_error": first_error,
        "retries": retries,
        "retry": retry_blocker(&inst, &rule).map_or(json!({ "allowed": inst.status == ProcessStatus::Failed }), |why| json!({ "allowed": false, "why": why })),
    }))
}

/// Why a run may not be retried yet, if it may not.
fn retry_blocker(inst: &ProcessInstance, rule: &AutomationRule) -> Option<String> {
    if inst.status != ProcessStatus::Failed {
        return Some("Only a failed run can be retried.".to_string());
    }
    let grant = inst.log.iter().any(|l| l.contains("flow_grant_exceeded"));
    (grant && rule.version <= inst.rule_version)
        .then(|| "This run failed outside the flow's approved grants. Retrying it needs a new version of the flow, approved and live.".to_string())
}

/// `POST /flows/runs/{instance_id}/retry`, body `{ "reason": "…" }`. An owner of the flow's workspace or an
/// administrator only.
pub fn retry(caller: &AuthUser, instance_id: &str, body: &Value, state: &SharedState) -> Result<Value, ServiceError> {
    let reason = body.get("reason").and_then(Value::as_str).map(str::trim).unwrap_or_default().to_string();
    let inst = instance(state, instance_id)?;
    let rule = flows::find_flow(state, &inst.rule_id).ok_or_else(|| ServiceError::NotFound(format!("The flow '{}' is not live", inst.rule_id)))?;
    let manifest = flow_validate::manifest_of(&rule, state)?;
    let ws = manifest.workspace_id.clone().or(rule.home_workspace.clone()).ok_or_else(|| ServiceError::forbidden("The flow has no workspace, so nobody owns it"))?;
    authorize_workspace(caller, &ws, "manage_workspace", state).map_err(|_| ServiceError::forbidden("Only an owner of the flow's workspace or an administrator may retry a run"))?;
    if reason.is_empty() {
        return Err(ServiceError::BadRequest("Say why the run is retried".to_string()));
    }
    if let Some(why) = retry_blocker(&inst, &rule) {
        return Err(ServiceError::Conflict(why));
    }
    let payload = json!({ "instance_id": inst.id, "flow": rule.id, "failed_version": inst.rule_version, "version": rule.version });
    state
        .append_ledger_entry(RecordDecisionInput {
            principal: caller.eppn.clone(),
            organization_code: manifest.organization_code.clone(),
            app_slug: (!rule.app_slug.is_empty()).then(|| rule.app_slug.clone()),
            decision_type: DecisionType::FlowRunRetried,
            oscal_control_id: "SI-11".to_string(),
            rationale: reason.clone(),
            payload: &payload,
        })
        .map_err(|e| ServiceError::Internal(format!("{e:?}")))?;
    let after = flows::retry_run(state, &inst, &rule, &caller.eppn, &reason);
    Ok(json!({ "instance_id": after.id, "status": status_of(after.status), "version": after.rule_version }))
}

/// `GET /flows/{id}/versions`: each stored version with its author, approvers, proposal and summary.
pub fn versions(caller: &AuthUser, flow_id: &str, state: &SharedState) -> Result<Value, ServiceError> {
    let rule = flow_for(caller, flow_id, state)?;
    let rows = state.repository.rule_versions(&rule.id).map_err(internal)?;
    let list: Vec<Value> = rows
        .iter()
        .map(|v| json!({ "version": v.version, "author": v.author, "approvers": v.approvers, "proposal_id": v.proposal_id, "summary": v.summary, "at": v.at.to_rfc3339(), "live": v.version as u32 == rule.version }))
        .collect();
    Ok(json!({ "versions": list }))
}

/// `GET /flows/{id}/versions/compare?from=&to=`: each step marked added, changed, removed or the same, in words, with
/// its path in the version it is in, both versions' steps for the canvas, and the changed sentences.
pub fn compare(caller: &AuthUser, flow_id: &str, q: &HashMap<String, String>, state: &SharedState) -> Result<Value, ServiceError> {
    let rule = flow_for(caller, flow_id, state)?;
    let num = |k: &str| q.get(k).and_then(|v| v.parse::<i32>().ok()).ok_or_else(|| ServiceError::BadRequest(format!("Name the version '{k}' to compare")));
    let (from, to) = (num("from")?, num("to")?);
    let rows = state.repository.rule_versions(&rule.id).map_err(internal)?;
    let get = |n: i32| -> Result<AutomationRule, ServiceError> {
        let row = rows.iter().find(|v| v.version == n).ok_or_else(|| ServiceError::NotFound(format!("There is no version {n} of this flow")))?;
        serde_json::from_value(row.rule.clone()).map_err(internal)
    };
    let (a, b) = (get(from)?, get(to)?);
    let (pa, pb) = (flow_validate::step_paths(&a.steps), flow_validate::step_paths(&b.steps));
    let all = |r: &AutomationRule| scaffoldry_core::workflow::all_steps(&r.steps).into_iter().map(|s| (s.id.clone(), serde_json::to_value(&s.kind).unwrap_or_default())).collect::<Vec<_>>();
    let (sa, sb) = (all(&a), all(&b));
    let words = |p: &Option<&Vec<usize>>| p.map(|p| p.iter().map(|n| (n + 1).to_string()).collect::<Vec<_>>().join(".")).unwrap_or_default();
    let mut steps = Vec::new();
    for (id, kind) in &sb {
        let before = sa.iter().find(|(i, _)| i == id);
        let path = pb.get(id);
        let (change, verb) = match before {
            None => ("added", "is added"),
            Some((_, k)) if k != kind || pa.get(id) != path => ("changed", "is changed"),
            Some(_) => ("same", "is the same"),
        };
        steps.push(json!({ "step": id, "change": change, "path": path, "sentence": format!("Step {} {verb}.", words(&path)) }));
    }
    for (id, _) in sa.iter().filter(|(i, _)| !sb.iter().any(|(j, _)| j == i)) {
        let path = pa.get(id);
        steps.push(json!({ "step": id, "change": "removed", "path": path, "sentence": format!("Step {} of version {from} is removed.", words(&path)) }));
    }
    let (da, db) = (scaffoldry_core::workflow::describe_rule(&a), scaffoldry_core::workflow::describe_rule(&b));
    // Every step that differs, then the rule's own sentences that differ.
    let mut sentences: Vec<String> = steps.iter().filter(|s| s["change"] != "same").filter_map(|s| s["sentence"].as_str().map(str::to_string)).collect();
    sentences.extend(da.iter().filter(|l| !db.contains(l)).map(|l| format!("Before: {l}")));
    sentences.extend(db.iter().filter(|l| !da.contains(l)).map(|l| format!("Now: {l}")));
    if a.trigger != b.trigger {
        sentences.insert(0, "The trigger is changed.".to_string());
    }
    Ok(json!({ "from": from, "to": to, "steps": steps, "sentences": sentences, "from_steps": a.steps, "to_steps": b.steps }))
}
