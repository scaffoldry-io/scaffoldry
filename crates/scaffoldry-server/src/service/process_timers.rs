//! Process v2 phase 3: timers and escalation. No timer ever decides.
//!
//! A user task may be `due` some whole days after it is entered, in `platform.timezone`, and say what happens then in
//! `on_due`. Entering the step writes the first timer. The `fire_timers` job, every minute, claims the due timers and
//! applies them: a reminder, an escalation to more people, or a failure. A fired timer writes the step's next one: the
//! next `on_due` action, `due` later, and a repeat of a reminder `remind_every` later, at most five reminders in all.
//!
//! A timer's action is stored as `{ "index", "action", "sent" }`: its place in `on_due`, the `DueAction`, and how many
//! reminders it has sent before this one.

use crate::repository::ActorKind;
use crate::repository_timers::{NewTimer, Timer, TimerOutcome};
use crate::state::SharedState;
use chrono::{DateTime, Utc};
use scaffoldry_core::approver::{resolve_approvers, ApproverSpec, Origin};
use scaffoldry_core::workflow::{find_step, DueAction, DurationSpec, MAX_REMINDERS};
use scaffoldry_core::{AutomationRule, ProcessInstance, ProcessStatus, StepKind};
use serde_json::{json, Value};

/// The most timers one run of the job applies. The next run applies the rest.
pub const MAX_PER_RUN: i64 = 500;

/// The parts of a user task a timer needs.
struct Due<'a> {
    due: DurationSpec,
    remind_every: Option<DurationSpec>,
    on_due: &'a [DueAction],
    on_fail: &'a [scaffoldry_core::ActionType],
    approver: ApproverSpec,
}

fn due_of<'a>(rule: &'a AutomationRule, step_id: &str) -> Option<Due<'a>> {
    match &find_step(&rule.steps, step_id)?.kind {
        StepKind::UserTask { role, approver, due: Some(due), remind_every, on_due, on_fail, .. } if !on_due.is_empty() => Some(Due {
            due: *due,
            remind_every: *remind_every,
            on_due,
            on_fail,
            approver: scaffoldry_core::approver::effective_approver(role, approver),
        }),
        _ => None,
    }
}

fn timer(step_id: &str, index: usize, action: &DueAction, sent: u32, fire_at: DateTime<Utc>) -> NewTimer {
    NewTimer { step_id: step_id.to_string(), action: json!({ "index": index, "action": action, "sent": sent }), fire_at }
}

/// The timers entering `step_id` writes: the first `on_due` action, `due` after `entered`.
pub fn timers_for_entry(state: &SharedState, rule: &AutomationRule, step_id: &str, entered: DateTime<Utc>) -> Vec<NewTimer> {
    let Some(d) = due_of(rule, step_id) else { return vec![] };
    let at = scaffoldry_engine::flow::due_at(entered, d.due.days, &state.platform_timezone());
    vec![timer(step_id, 0, &d.on_due[0], 0, at)]
}

/// The timers a fired one writes: on its first firing, the next `on_due` action `due` later, and for a reminder with
/// `remind_every`, its repeat, until it has sent `MAX_REMINDERS`. A `Fail` writes none.
fn next_timers(state: &SharedState, d: &Due<'_>, step_id: &str, index: usize, action: &DueAction, sent: u32, fired_for: DateTime<Utc>) -> Vec<NewTimer> {
    let zone = state.platform_timezone();
    let mut out = vec![];
    if matches!(action, DueAction::Fail { .. }) {
        return out;
    }
    if sent == 0 {
        if let Some(next) = d.on_due.get(index + 1) {
            out.push(timer(step_id, index + 1, next, 0, scaffoldry_engine::flow::due_at(fired_for, d.due.days, &zone)));
        }
    }
    if let (DueAction::Remind, Some(every)) = (action, d.remind_every) {
        if sent + 1 < MAX_REMINDERS {
            out.push(timer(step_id, index, action, sent + 1, scaffoldry_engine::flow::due_at(fired_for, every.days, &zone)));
        }
    }
    out
}

/// A person's display name, or their eppn when the platform has no record of them.
pub fn display_name(state: &SharedState, eppn: &str) -> String {
    state
        .repository
        .users_by_names(&[eppn.to_string()])
        .ok()
        .and_then(|u| u.first().map(crate::service::people::display_name))
        .unwrap_or_else(|| eppn.to_string())
}

/// Who `spec` resolves to for the instance's record, by eppn.
fn resolve_for(state: &SharedState, inst: &ProcessInstance, spec: &ApproverSpec) -> Vec<String> {
    let named: Vec<String> = match spec {
        ApproverSpec::Person { eppn } => vec![eppn.clone()],
        _ => vec![],
    };
    let ctx = crate::service::approvers::context(state, &inst.app_slug, &inst.record_id, inst.started_by.as_deref(), &named);
    resolve_approvers(spec, &ctx).approvers.into_iter().map(|a| a.eppn).collect()
}

/// Where an `EscalateUp` goes: the step's position, found from the unit above where it was last found, walking up as
/// `walk_up` does. Returns the people and the unit they were found at. `None` when the step's approver is not a
/// position, or there is no unit above.
fn escalate_up_target(state: &SharedState, inst: &ProcessInstance, spec: &ApproverSpec) -> Option<(Vec<String>, String)> {
    let ApproverSpec::Position { key, .. } = spec else { return None };
    let found = match &inst.escalated_unit {
        Some(u) => u.clone(),
        None => crate::service::approvers::resolution_for(state, inst)?.2.unit_id?,
    };
    let found: uuid::Uuid = found.parse().ok()?;
    let parent = state.organizations.read().ok()?.get(&found)?.parent_id?;
    let up = ApproverSpec::Position { key: key.clone(), from: Origin::Workspace, walk_up: true };
    let submitter = state.find_record(&inst.app_slug, &inst.record_id).ok().flatten().and_then(|r| r.created_by);
    let r = crate::service::approvers::resolution_at(state, &up, parent, &[], submitter.as_deref());
    let unit = r.unit_id.clone().unwrap_or_else(|| parent.to_string());
    Some((r.approvers.into_iter().map(|a| a.eppn).collect(), unit))
}

/// A notice about the instance's step.
fn timer_notice_parts(state: &SharedState, inst: &ProcessInstance) -> (crate::notifications::AppRef, crate::notifications::TableRef, crate::notifications::StepRef, Option<String>) {
    let record = state.find_record(&inst.app_slug, &inst.record_id).ok().flatten();
    let table = record.as_ref().and_then(|r| r.data.get("_table_id").and_then(Value::as_str).map(str::to_string)).unwrap_or_default();
    (
        crate::notifications::AppRef(inst.app_slug.clone()),
        crate::notifications::TableRef(table),
        crate::notifications::StepRef(inst.id.clone()),
        record.and_then(|r| r.created_by),
    )
}

/// Applies one claimed timer. Returns whether it changed the instance: false when the instance has moved on.
fn apply_timer(state: &SharedState, t: &Timer) -> Result<bool, String> {
    let Some(inst) = state.repository.get_process_instance(&t.instance_id).map_err(|e| e.to_string())? else { return Ok(false) };
    if inst.status != ProcessStatus::Waiting || inst.waiting_step_id.as_deref() != Some(t.step_id.as_str()) {
        return Ok(false);
    }
    let Some(rule) = crate::service::processes::rule_for(state, &inst) else { return Ok(false) };
    let Some(d) = due_of(&rule, &t.step_id) else { return Ok(false) };
    let action: DueAction = serde_json::from_value(t.action["action"].clone()).map_err(|e| format!("timer {}: {e}", t.id))?;
    let index = t.action["index"].as_u64().unwrap_or(0) as usize;
    let sent = t.action["sent"].as_u64().unwrap_or(0) as u32;

    let mut next = inst.clone();
    let mut record = None;
    // Who is told after the write, and with which notice.
    let mut escalated: Option<(Vec<String>, Vec<String>)> = None;
    match &action {
        DueAction::Remind => next.log.push(format!("Reminded the people who may decide: step '{}' is overdue", t.step_id)),
        DueAction::Escalate { approver } => {
            let (before, _) = crate::service::approvers::current_approvers(state, &inst);
            let added = add_escalated(&mut next, resolve_for(state, &inst, approver));
            next.log.push(format!("Escalated step '{}' to {}", t.step_id, words_list(&added)));
            escalated = Some((before, added));
        }
        DueAction::EscalateUp => match escalate_up_target(state, &inst, &d.approver) {
            Some((people, unit)) => {
                let (before, _) = crate::service::approvers::current_approvers(state, &inst);
                let added = add_escalated(&mut next, people);
                next.escalated_unit = Some(unit);
                next.log.push(format!("Escalated step '{}' one unit up to {}", t.step_id, words_list(&added)));
                escalated = Some((before, added));
            }
            None => next.log.push(format!("Could not escalate step '{}': there is no unit above", t.step_id)),
        },
        DueAction::Fail { reason } => {
            next.status = ProcessStatus::Failed;
            next.waiting_step_id = None;
            next.log.push(format!("Failed at step '{}': {reason}", t.step_id));
            let effects = crate::service::processes::step_effects(d.on_fail, &mut next.log);
            if !effects.is_empty() {
                if let Some(rec) = state.find_record(&inst.app_slug, &inst.record_id)? {
                    let mut data = rec.data.clone();
                    scaffoldry_core::workflow::apply_field_effects(&mut data, &effects);
                    let ctx = crate::service::history::ctx_for("process-timer", ActorKind::Automation, &inst.app_slug, &data, state);
                    record = Some((inst.app_slug.clone(), inst.record_id.clone(), data, ctx));
                }
            }
        }
    }
    let timers = next_timers(state, &d, &t.step_id, index, &action, sent, t.fire_at);
    let detail = json!({ "action": action, "sent": sent });
    if !state.repository.apply_timer(&t.step_id, TimerOutcome { next: next.clone(), record, timers, detail }).map_err(|e| e.to_string())? {
        return Ok(false);
    }

    let (app, table, step, owner) = timer_notice_parts(state, &next);
    match (&action, escalated) {
        (DueAction::Remind, _) => {
            let overdue = (Utc::now() - t.fire_at).num_days().max(0) as u32 + d.due.days;
            let notice = crate::notifications::Notice::ProcessOverdue { app, table, step, days: crate::notifications::Count(overdue) };
            let _ = crate::notifications::notify(state, &crate::notifications::Recipients::Approvers(next.id.clone()), None, &notice);
        }
        (_, Some((before, added))) if !added.is_empty() => {
            let notice = crate::notifications::Notice::ProcessEscalated { app, table, step };
            let mut told: Vec<String> = added;
            for p in before.into_iter().chain(owner) {
                if !told.iter().any(|t| t.eq_ignore_ascii_case(&p)) {
                    told.push(p);
                }
            }
            let _ = crate::notifications::notify(state, &crate::notifications::Recipients::People(told), None, &notice);
        }
        _ => {}
    }
    Ok(true)
}

/// Adds the people not already escalated to. Returns the ones added.
fn add_escalated(inst: &mut ProcessInstance, people: Vec<String>) -> Vec<String> {
    let mut added = vec![];
    for p in people {
        if !inst.escalated_to.iter().any(|e| e.eq_ignore_ascii_case(&p)) {
            inst.escalated_to.push(p.clone());
            added.push(p);
        }
    }
    added
}

fn words_list(people: &[String]) -> String {
    if people.is_empty() {
        "nobody new".to_string()
    } else {
        people.join(", ")
    }
}

/// Applies every timer due at `now`, once. Returns how many changed an instance.
pub fn fire_due(state: &SharedState, now: DateTime<Utc>) -> Result<usize, String> {
    let claimed = state.repository.claim_due_timers(now, MAX_PER_RUN).map_err(|e| e.to_string())?;
    let mut applied = 0;
    for t in &claimed {
        match apply_timer(state, t) {
            Ok(true) => applied += 1,
            Ok(false) => {}
            Err(e) => eprintln!("Warning: process timer {} could not be applied: {e}", t.id),
        }
    }
    Ok(applied)
}

/// The `fire_timers` job, run every minute.
pub fn fire_timers_job(ctx: &crate::jobs::JobCtx, _payload: Value) -> Result<Value, crate::jobs::JobError> {
    let state = ctx.state().ok_or_else(|| crate::jobs::JobError::Permanent("the timers need the server's state, and this runner has none".to_string()))?;
    let applied = fire_due(state, Utc::now()).map_err(crate::jobs::JobError::Retry)?;
    Ok(json!({ "applied": applied }))
}

/// A unit's name.
fn unit_name(state: &SharedState, unit: &str) -> Option<String> {
    let id: uuid::Uuid = unit.parse().ok()?;
    state.organizations.read().ok()?.get(&id).map(|o| o.name.clone())
}

/// The instance's next timer for the console: when it fires, what it does, and a sentence saying so.
pub fn next_timer_view(state: &SharedState, inst: &ProcessInstance) -> Option<Value> {
    let (fire_at, action) = state.repository.next_timer(&inst.id).ok()??;
    let due: DueAction = serde_json::from_value(action["action"].clone()).ok()?;
    let day = scaffoldry_engine::flow::local_format(fire_at, &state.platform_timezone(), "%-d %b");
    let what = match &due {
        DueAction::Remind => "the people who may decide will be reminded".to_string(),
        DueAction::Fail { reason } => format!("this will fail: {}", reason.trim_end_matches('.')),
        DueAction::Escalate { approver } => format!("this will be sent to {}", spec_words(state, inst, approver)),
        DueAction::EscalateUp => {
            let rule = crate::service::processes::rule_for(state, inst)?;
            let d = due_of(&rule, inst.waiting_step_id.as_deref()?)?;
            match (&d.approver, escalate_up_target(state, inst, &d.approver)) {
                (ApproverSpec::Position { key, .. }, Some((_, unit))) => {
                    format!("this will be sent to the {} of {}", position_name(state, key), unit_name(state, &unit).unwrap_or(unit))
                }
                _ => "this cannot be sent higher: there is no unit above".to_string(),
            }
        }
    };
    Some(json!({ "fire_at": fire_at.to_rfc3339(), "action": due, "sentence": format!("On {day}, {what}.") }))
}

fn position_name(state: &SharedState, key: &str) -> String {
    state.position_types.read().ok().and_then(|p| p.get(key).map(|t| t.name.clone())).unwrap_or_else(|| key.to_string())
}

/// Who an approver spec names, in words.
fn spec_words(state: &SharedState, inst: &ProcessInstance, spec: &ApproverSpec) -> String {
    match spec {
        ApproverSpec::Person { eppn } => display_name(state, eppn),
        ApproverSpec::Role { name } => format!("the people with the {name} role"),
        ApproverSpec::Position { key, .. } => {
            let people: Vec<String> = resolve_for(state, inst, spec).iter().map(|e| display_name(state, e)).collect();
            if people.is_empty() {
                format!("the {}, who is not appointed", position_name(state, key))
            } else {
                format!("the {} ({})", position_name(state, key), people.join(", "))
            }
        }
    }
}

/// What is wrong with a user task's timers, for the rule validator.
pub fn problems(steps: &[scaffoldry_core::ProcessStep]) -> Vec<String> {
    let mut out = vec![];
    for step in scaffoldry_core::workflow::all_steps(steps) {
        let StepKind::UserTask { due, remind_every, on_due, approver, role, .. } = &step.kind else { continue };
        let id = &step.id;
        let days = |d: &Option<DurationSpec>| d.map(|d| d.days);
        if let Some(n) = days(due) {
            if n == 0 || n > scaffoldry_core::workflow::MAX_DUE_DAYS {
                out.push(format!("Step '{id}' is due in {n} days. Use 1 to {} days.", scaffoldry_core::workflow::MAX_DUE_DAYS));
            }
        }
        if let Some(n) = days(remind_every) {
            if n == 0 || n > scaffoldry_core::workflow::MAX_DUE_DAYS {
                out.push(format!("Step '{id}' reminds every {n} days. Use 1 to {} days.", scaffoldry_core::workflow::MAX_DUE_DAYS));
            }
            if !on_due.iter().any(|a| matches!(a, DueAction::Remind)) {
                out.push(format!("Step '{id}' sets remind_every but has no Remind in on_due."));
            }
        }
        if !on_due.is_empty() && due.is_none() {
            out.push(format!("Step '{id}' says what happens when it is due, but not when it is due. Set due."));
        }
        if on_due.iter().any(|a| matches!(a, DueAction::EscalateUp)) {
            let spec = scaffoldry_core::approver::effective_approver(role, approver);
            if !matches!(spec, ApproverSpec::Position { .. }) {
                out.push(format!("Step '{id}' escalates up, which needs a position to decide it. Name a position, or use Escalate."));
            }
        }
    }
    out
}
