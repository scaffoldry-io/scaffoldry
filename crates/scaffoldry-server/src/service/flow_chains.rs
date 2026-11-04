//! Workflows phase 3: flows that start other flows, on a schedule, when a flow completes, and when a process step is
//! decided. Calls are typed against the called flow's inputs. A cycle of flows is refused when a version is saved.

use crate::state::{DatasetRecord, SharedState};
use chrono::{DateTime, Duration, Utc};
use scaffoldry_core::workflow::{all_steps, InputKind, StepKind};
use scaffoldry_core::{ActionType, AutomationRule, ProcessInstance, ProcessStatus, TriggerEvent};
use scaffoldry_engine::flow::expr_refs;
use scaffoldry_engine::{AppManifest, FieldType};
use serde_json::{Map, Value};

/// The type an expression has, when it can be told without running it: a literal, a record id, a field of the trigger
/// record, or an input. `None` when it cannot be told; such an expression is not refused.
pub fn type_of(expr: &str, rule: &AutomationRule, manifest: Option<&AppManifest>) -> Option<InputKind> {
    let e = expr.trim();
    if e.len() >= 2 && e.starts_with('"') && e.ends_with('"') && !e[1..e.len() - 1].contains('"') {
        return Some(InputKind::Text);
    }
    if e.parse::<f64>().is_ok() {
        return Some(InputKind::Number);
    }
    if e.eq_ignore_ascii_case("true") || e.eq_ignore_ascii_case("false") {
        return Some(InputKind::Boolean);
    }
    let refs = expr_refs(e);
    let whole = refs.len() == 1 && e.starts_with('{') && e.ends_with('}') && e.matches('{').count() == 1;
    if !whole {
        return None;
    }
    let (scope, field) = &refs[0];
    if field == "id" {
        return Some(InputKind::RecordId);
    }
    match scope.as_str() {
        "input" => match &rule.trigger {
            TriggerEvent::Called { inputs } => inputs.iter().find(|i| &i.name == field).map(|i| i.kind),
            _ => None,
        },
        "trigger" => {
            let f = manifest?.tables.iter().flat_map(|t| t.fields.iter()).find(|f| &f.name == field)?;
            Some(match f.field_type {
                FieldType::Number | FieldType::Currency | FieldType::Percent | FieldType::Rating | FieldType::Duration { .. } | FieldType::Count | FieldType::Autonumber => InputKind::Number,
                FieldType::Date | FieldType::DateTime | FieldType::CreatedTime | FieldType::LastModifiedTime => InputKind::Date,
                FieldType::Boolean | FieldType::Checkbox => InputKind::Boolean,
                FieldType::Link => InputKind::RecordId,
                _ => InputKind::Text,
            })
        }
        _ => None,
    }
}

/// The flows that start from a rule: the ones it starts, and the ones waiting for it to complete or be decided.
fn next_of<'a>(rule: &AutomationRule, all: &'a [AutomationRule]) -> Vec<&'a AutomationRule> {
    let mut out: Vec<&AutomationRule> = Vec::new();
    for s in all_steps(&rule.steps) {
        if let StepKind::Service { action: ActionType::StartFlow { flow_id, .. } } = &s.kind {
            out.extend(all.iter().filter(|r| &r.id == flow_id));
        }
    }
    out.extend(all.iter().filter(|r| match &r.trigger {
        TriggerEvent::FlowCompleted { flow_id, .. } => flow_id == &rule.id,
        TriggerEvent::ProcessDecided { rule: decided, .. } => decided == &rule.id,
        _ => false,
    }));
    out
}

/// A path from `rule` back to itself through the active flows, as names, when there is one.
fn cycle_from(rule: &AutomationRule, all: &[AutomationRule]) -> Option<Vec<String>> {
    fn walk<'a>(at: &'a AutomationRule, target: &str, all: &'a [AutomationRule], path: &mut Vec<&'a AutomationRule>) -> bool {
        for next in next_of(at, all) {
            if next.id == target {
                path.push(next);
                return true;
            }
            if path.iter().any(|p| p.id == next.id) {
                continue;
            }
            path.push(next);
            if walk(next, target, all, path) {
                return true;
            }
            path.pop();
        }
        false
    }
    let start = all.iter().find(|r| r.id == rule.id)?;
    let mut path = vec![start];
    walk(start, &rule.id, all, &mut path).then(|| path.iter().map(|r| r.name.clone()).collect())
}

/// What stops a rule's triggers and calls from being saved: a bad schedule, a call to a flow that is not a called flow,
/// a missing, extra or wrongly typed input, and a cycle through the active flows.
pub fn validate(state: &SharedState, manifest: &AppManifest, rule: &AutomationRule) -> Vec<String> {
    let mut problems = Vec::new();
    if let TriggerEvent::Schedule { every, at } = &rule.trigger {
        problems.extend(scaffoldry_engine::flow::schedule_problems(every, at));
    }
    // The flows as they would be with this version live.
    let mut all: Vec<AutomationRule> = state.automations.read().map(|a| a.values().flatten().filter(|r| r.enabled && r.id != rule.id).cloned().collect()).unwrap_or_default();
    all.push(rule.clone());
    for s in all_steps(&rule.steps) {
        let StepKind::Service { action: ActionType::StartFlow { flow_id, inputs } } = &s.kind else { continue };
        let Some(target) = all.iter().find(|r| &r.id == flow_id) else {
            problems.push(format!("step '{}' starts the flow '{flow_id}', which is not live", s.id));
            continue;
        };
        let TriggerEvent::Called { inputs: declared } = &target.trigger else {
            problems.push(format!("step '{}' starts '{}', which is not a flow other flows start. Give it a Called trigger", s.id, target.name));
            continue;
        };
        for d in declared {
            match inputs.iter().find(|(n, _)| n == &d.name) {
                None => problems.push(format!("step '{}': '{}' needs its input '{}' ({:?})", s.id, target.name, d.name, d.kind)),
                Some((_, e)) => {
                    if let Some(kind) = type_of(e, rule, Some(manifest)) {
                        if kind != d.kind {
                            problems.push(format!("step '{}': the input '{}' of '{}' is a {:?}, and {e} is a {kind:?}", s.id, d.name, target.name, d.kind));
                        }
                    }
                }
            }
        }
        for (n, _) in inputs.iter().filter(|(n, _)| !declared.iter().any(|d| &d.name == n)) {
            problems.push(format!("step '{}': '{}' has no input '{n}'", s.id, target.name));
        }
    }
    if let Some(path) = cycle_from(rule, &all) {
        let (first, rest) = path.split_first().expect("a path has a start");
        let mut sentence = format!("{first} starts {}", rest[0]);
        for name in &rest[1..] {
            sentence.push_str(&format!(", which starts {name}"));
        }
        problems.push(format!("the flows would start each other without end: {sentence}."));
    }
    problems
}

/// Starts one rule: a flow through the flow runner, or a rule with a record through the rule engine.
fn start_rule(state: &SharedState, rule: &AutomationRule, trigger: Option<&DatasetRecord>, by: &str, chain: &[String]) {
    if crate::service::flow_parties::is_disabled(state, &rule.id) || !rule.enabled {
        return;
    }
    if crate::service::flows::is_flow(rule) {
        crate::service::flows::start_flow(state, rule, trigger, Map::new(), by, &uuid::Uuid::new_v4().to_string(), chain);
    } else if let Some(rec) = trigger {
        let others: Vec<String> = state.automations.read().map(|a| a.get(&rule.app_slug).map(|l| l.iter().filter(|r| r.id != rule.id).map(|r| r.id.clone()).collect()).unwrap_or_default()).unwrap_or_default();
        let _ = crate::service::date_triggers::start_dated(state, rule, &others, &rec.id);
    }
}

fn status_name(s: ProcessStatus) -> &'static str {
    match s {
        ProcessStatus::Waiting => "Waiting",
        ProcessStatus::Completed => "Completed",
        ProcessStatus::Rejected => "Rejected",
        ProcessStatus::Failed => "Failed",
    }
}

fn waiting_for(state: &SharedState, matches: impl Fn(&TriggerEvent) -> bool) -> Vec<AutomationRule> {
    state.automations.read().map(|a| a.values().flatten().filter(|r| matches(&r.trigger)).cloned().collect()).unwrap_or_default()
}

/// A flow run ended: the flows waiting for it to end with this status start, one level deeper, with the same record.
pub fn on_completed(state: &SharedState, rule: &AutomationRule, inst: &ProcessInstance, trigger: Option<&DatasetRecord>, chain: &[String]) {
    let status = status_name(inst.status);
    let by = inst.started_by.clone().unwrap_or_default();
    for next in waiting_for(state, |t| matches!(t, TriggerEvent::FlowCompleted { flow_id, status: s } if flow_id == &rule.id && s.eq_ignore_ascii_case(status))) {
        start_rule(state, &next, trigger, &by, chain);
    }
}

/// A process step was decided: the flows waiting for that decision start, with the decided record.
pub fn on_decided(state: &SharedState, app: &str, rule_id: &str, decision: &str, record_id: &str, by: &str) {
    let waiting = waiting_for(state, |t| matches!(t, TriggerEvent::ProcessDecided { app: a, rule, decision: d } if a == app && rule == rule_id && d.eq_ignore_ascii_case(decision)));
    if waiting.is_empty() {
        return;
    }
    let Ok(Some(rec)) = state.find_record(app, record_id) else { return };
    for next in waiting {
        start_rule(state, &next, Some(&rec), by, &[]);
    }
}

/// Starts each scheduled flow whose latest time at or before `now`, in `platform.timezone`, fell in the last day and has
/// not started yet. The key of flow and time makes a doubled tick, or a second scheduler, start one run.
pub fn fire_schedules(state: &SharedState, now: DateTime<Utc>) -> Result<usize, String> {
    let zone = state.settings.read().ok().and_then(|s| s.get("platform.timezone").and_then(Value::as_str).map(str::to_string)).unwrap_or_else(|| "UTC".to_string());
    let mut started = 0;
    for rule in waiting_for(state, |t| matches!(t, TriggerEvent::Schedule { .. })) {
        let TriggerEvent::Schedule { every, at } = &rule.trigger else { continue };
        let Some(when) = scaffoldry_engine::flow::last_occurrence(every, at, &zone, now) else { continue };
        if when <= now - Duration::days(1) || !rule.enabled || crate::service::flow_parties::is_disabled(state, &rule.id) {
            continue;
        }
        let key = format!("schedule:{}:{}", rule.id, when.to_rfc3339());
        if !state.repository.claim_once(&key, &rule.id).map_err(|e| e.to_string())? {
            continue;
        }
        crate::service::flows::start_flow(state, &rule, None, Map::new(), "system:schedule", &uuid::Uuid::new_v4().to_string(), &[]);
        started += 1;
    }
    Ok(started)
}

/// The `fire_schedules` job, run every five minutes.
pub fn fire_schedules_job(ctx: &crate::jobs::JobCtx, _payload: Value) -> Result<Value, crate::jobs::JobError> {
    let state = ctx.state().ok_or_else(|| crate::jobs::JobError::Permanent("the schedule needs the server's state, and this runner has none".to_string()))?;
    let started = fire_schedules(state, Utc::now()).map_err(crate::jobs::JobError::Retry)?;
    Ok(serde_json::json!({ "started": started }))
}
