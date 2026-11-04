//! Workflows phase 4: task grants.
//!
//! While a user task is open, each person who may act on it holds a grant on that one record: read of `show` and
//! `fill`, write of `fill`. Only the single-record read and the single-record update consult a grant, and only when the
//! caller's own rights do not reach the record. A grant never opens the app, a list, a search, a link picker, or any
//! other record. A write through one is the person's own (`actor_kind` user, no `on_behalf_of`), and its history label
//! names the flow and the step.

use crate::repository_task_grants::{NewGrant, TaskGrant};
use crate::state::{AuthUser, DatasetRecord, SharedState};
use scaffoldry_core::workflow::{find_step, TaskKind};
use scaffoldry_core::{AutomationRule, ProcessInstance, StepKind};
use serde_json::Value;

/// The grants entering `step_id` of `rule` opens for `people` on the instance's record. None when the step lists no
/// fields to show or fill.
pub fn task_grants_for(state: &SharedState, rule: &AutomationRule, step_id: &str, inst: &ProcessInstance, people: &[String]) -> Vec<NewGrant> {
    let Some(StepKind::UserTask { kind, show, fill, .. }) = find_step(&rule.steps, step_id).map(|s| &s.kind) else { return vec![] };
    if show.is_empty() && fill.is_empty() {
        return vec![];
    }
    let table = state
        .find_record(&inst.app_slug, &inst.record_id)
        .ok()
        .flatten()
        .and_then(|r| r.data.get("_table_id").and_then(Value::as_str).map(str::to_string))
        .unwrap_or_default();
    let write: Vec<String> = if matches!(kind, TaskKind::Complete) { fill.clone() } else { vec![] };
    let mut read: Vec<String> = show.clone();
    for f in fill {
        if !read.contains(f) {
            read.push(f.clone());
        }
    }
    people
        .iter()
        .map(|p| NewGrant { person: p.clone(), app_slug: inst.app_slug.clone(), table_id: table.clone(), record_id: inst.record_id.clone(), read_fields: read.clone(), write_fields: write.clone() })
        .collect()
}

/// The grants a step that waits now opens: one for each person who may act on it.
pub fn grants_on_entry(state: &SharedState, rule: &AutomationRule, inst: &ProcessInstance) -> Vec<NewGrant> {
    let Some(step) = inst.waiting_step_id.as_deref() else { return vec![] };
    let people = if inst.voters.is_empty() { crate::service::approvers::current_approvers(state, inst).0 } else { inst.voters.clone() };
    task_grants_for(state, rule, step, inst, &people)
}

/// The open grant the caller holds on this record, if any.
pub fn open_grant(caller: &AuthUser, app_slug: &str, record_id: &str, state: &SharedState) -> Option<TaskGrant> {
    state.repository.open_task_grant(&caller.eppn, app_slug, record_id).ok().flatten()
}

/// The record as a grant shows it: the fields it lists, and nothing else of the data.
pub fn through_grant(mut record: DatasetRecord, grant: &TaskGrant) -> DatasetRecord {
    if let Some(map) = record.data.as_object_mut() {
        map.retain(|k, _| k == "_table_id" || grant.read_fields.iter().any(|f| f == k) || grant.write_fields.iter().any(|f| f == k));
    }
    record
}

/// The history label of a write through a grant: the flow and the step.
pub fn label(state: &SharedState, grant: &TaskGrant) -> String {
    let rule = state.repository.get_process_instance(&grant.instance_id).ok().flatten().and_then(|i| crate::service::processes::rule_for(state, &i));
    let name = rule.map(|r| r.name).unwrap_or_else(|| grant.instance_id.clone());
    format!("Task: {name}, step {}", grant.step_id).chars().take(255).collect()
}

/// The fields of `data` a grant may not write. `_table_id` is the record's own, so it never counts.
pub fn unwritable<'a>(data: &'a Value, grant: &TaskGrant) -> Vec<&'a str> {
    data.as_object()
        .map(|m| m.keys().map(String::as_str).filter(|k| *k != "_table_id" && !grant.write_fields.iter().any(|f| f == k)).collect())
        .unwrap_or_default()
}

/// Whether a value counts as filled for `require`: not null, not blank text, not an empty list.
pub fn filled(v: Option<&Value>) -> bool {
    match v {
        None | Some(Value::Null) => false,
        Some(Value::String(s)) => !s.trim().is_empty(),
        Some(Value::Array(a)) => !a.is_empty(),
        Some(_) => true,
    }
}

/// What is wrong with a rule's task steps, for the rule validator: `show` and `fill` name fields the app has, `fill`
/// names fields a person can write, and the kind fits the decision.
pub fn problems(manifest: &scaffoldry_engine::AppManifest, rule: &AutomationRule) -> Vec<String> {
    let fields: Vec<&scaffoldry_engine::FieldSpec> = manifest.tables.iter().flat_map(|t| t.fields.iter()).collect();
    let mut out = vec![];
    for step in scaffoldry_core::workflow::all_steps(&rule.steps) {
        let StepKind::UserTask { kind, show, fill, require, decision, .. } = &step.kind else { continue };
        let id = &step.id;
        for name in show.iter().chain(fill.iter()) {
            if !fields.iter().any(|f| &f.name == name) {
                out.push(format!("Step '{id}' names the field '{name}', which the app does not have."));
            }
        }
        for name in fill {
            if fields.iter().any(|f| &f.name == name && (f.field_type.is_computed() || f.field_type == scaffoldry_engine::FieldType::Button)) {
                out.push(format!("Step '{id}' asks for the field '{name}', which nobody can write: it is computed."));
            }
        }
        for name in require {
            if !fill.contains(name) {
                out.push(format!("Step '{id}' requires '{name}', which it does not ask the assignee to fill."));
            }
        }
        match kind {
            TaskKind::Complete if fill.is_empty() => out.push(format!("Step '{id}' is a Complete task with nothing to fill. List the fields in fill.")),
            TaskKind::Complete if !decision.is_single() => out.push(format!("Step '{id}' is a Complete task, which one person does. Use Review for a committee.")),
            TaskKind::Review if decision.is_single() => out.push(format!("Step '{id}' is a Review, which a committee votes on. Set decision to AllOf or AtLeast.")),
            TaskKind::Approve | TaskKind::Review if !fill.is_empty() => out.push(format!("Step '{id}' lists fields to fill, which only a Complete task does.")),
            _ => {}
        }
    }
    out
}
