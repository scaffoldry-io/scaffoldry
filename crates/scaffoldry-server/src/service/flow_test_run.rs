//! Workflows phase 9: `POST /flows/{id}/test-run`. It runs a flow in dry mode and returns a trace. It writes nothing,
//! sends nothing, and makes no work item, notification, history entry or instance.
//!
//! The body names the trigger: `record_id`, a record the tester may read, or `values`, sample values. It may hold
//! `flow`, the editor's draft, which is tested in place of the live version. `outcomes` takes each decision's outcome
//! by step id; the default is `approve`. `as_person` resolves approvers as another person submitted the record; only
//! the flow's author may use it.
//!
//! Values in the trace are shown only for apps the tester may read. A foreign app's records appear as a count and ids.

use crate::service::access::{authorize_app, AppAction};
use crate::service::{flow_validate, flows, ServiceError};
use crate::state::{AuthUser, DatasetRecord, SharedState};
use scaffoldry_core::{AutomationRule, StepKind};
use serde_json::{json, Map, Value};
use std::collections::HashMap;

pub fn test_run(caller: &AuthUser, flow_id: &str, body: Value, state: &SharedState) -> Result<Value, ServiceError> {
    let draft = body.get("flow").filter(|v| !v.is_null()).cloned();
    let mut rule: AutomationRule = match &draft {
        Some(v) => serde_json::from_value(v.clone()).map_err(|e| ServiceError::BadRequest(format!("rule_schema: the flow is not a flow: {e}")))?,
        None => flows::find_flow(state, flow_id).ok_or_else(|| ServiceError::NotFound(format!("There is no live flow '{flow_id}'")))?,
    };
    rule.id = flow_id.to_string();
    let manifest = flow_validate::manifest_of(&rule, state)?;
    flow_validate::authorize(caller, &rule, &manifest, state)?;
    // A draft's grants are derived as its next version's would be. A live flow runs inside the grants it was approved with.
    if draft.is_some() {
        flows::derive(state, &manifest, &mut rule);
    }

    let as_person = body.get("as_person").and_then(Value::as_str).filter(|s| !s.is_empty()).map(str::to_string);
    if let Some(person) = &as_person {
        // The author of a draft is the person editing it. A live flow's author saved its newest version.
        let author = if draft.is_some() { Some(caller.eppn.clone()) } else { state.repository.rule_author(&rule.id).map_err(|e| ServiceError::Internal(e.to_string()))? };
        if !author.is_some_and(|a| a.eq_ignore_ascii_case(&caller.eppn)) {
            return Err(ServiceError::forbidden(format!("Only the flow's author may test it as {person}")));
        }
    }

    let trigger: Option<DatasetRecord> = if let Some(id) = body.get("record_id").and_then(Value::as_str).filter(|s| !s.is_empty()) {
        // The tester must be able to read the record they test with.
        Some(crate::service::records::fetch_record(caller, &rule.app_slug, id, state)?)
    } else if let Some(values) = body.get("values").and_then(Value::as_object) {
        Some(DatasetRecord {
            id: String::new(),
            app_slug: rule.app_slug.clone(),
            data: Value::Object(values.clone()),
            ceds_mapping: Value::Object(Map::new()),
            is_ferpa_sensitive: false,
            created_at: chrono::Utc::now().to_rfc3339(),
            created_by: Some(caller.eppn.clone()),
            version: 0,
        })
    } else {
        None
    };

    // Approvers come from the real resolver, for the record as its submitter (or `as_person`) made it.
    let submitter = as_person.clone().or_else(|| trigger.as_ref().and_then(|t| t.created_by.clone())).or_else(|| Some(caller.eppn.clone()));
    let ctx = crate::service::approvers::context_for(state, &rule.app_slug, submitter, Some(&caller.eppn), &[]);
    let resolve = |s: &scaffoldry_core::ProcessStep| -> Vec<String> {
        let StepKind::UserTask { role, approver, .. } = &s.kind else { return Vec::new() };
        let spec = scaffoldry_core::approver::effective_approver(role, approver);
        scaffoldry_core::approver::resolve_approvers(&spec, &ctx).approvers.into_iter().map(|a| a.eppn).collect()
    };
    let outcomes = body.get("outcomes").and_then(Value::as_object).cloned().unwrap_or_default();
    let run = flows::test_run(state, &rule, trigger.as_ref(), &caller.eppn, &outcomes, &resolve);

    // Values only for apps the tester may read. A record the run would create may hold values copied from anything the
    // run read, so its values show only when the tester may read every app the run found records in.
    let mut readable: HashMap<String, bool> = HashMap::new();
    let mut may_read = |app: &str| *readable.entry(app.to_string()).or_insert_with(|| authorize_app(caller, app, AppAction::Read, state).is_ok());
    let read_from: Vec<String> = run.trace.iter().filter(|t| t.outcome == "ran").filter_map(|t| t.records.as_ref().map(|(a, _)| a.clone())).collect();
    let reads_hidden = read_from.iter().any(|a| !may_read(a));
    let trace: Vec<Value> = run
        .trace
        .into_iter()
        .map(|t| {
            let mut entry = json!({ "step": t.step, "path": t.path, "outcome": t.outcome, "sentence": t.sentence });
            if let Some(who) = t.approvers {
                entry["approvers"] = json!(who);
            }
            if let Some((app, recs)) = t.records {
                let show = may_read(&app) && (t.outcome == "ran" || !reads_hidden);
                entry["count"] = json!(recs.len());
                entry["records"] = Value::Array(
                    recs.iter()
                        .map(|r| {
                            let id = if r.id.is_empty() { Value::Null } else { json!(r.id) };
                            if show {
                                let mut values = r.data.clone();
                                if let Some(m) = values.as_object_mut() {
                                    m.remove("_table_id");
                                }
                                json!({ "id": id, "values": values })
                            } else {
                                json!({ "id": id })
                            }
                        })
                        .collect(),
                );
                if !show {
                    let why = if may_read(&app) { "It may hold values from an app you may not read, so only the count and ids are shown." } else { "You may not read this app, so only the count and ids are shown." };
                    entry["hidden"] = json!(why);
                }
            }
            entry
        })
        .collect();
    let first_failure = run.first_failure.map(|(step, path, reason)| json!({ "step": step, "path": path, "reason": reason }));
    Ok(json!({ "trace": trace, "first_failure": first_failure, "writes": run.writes }))
}
