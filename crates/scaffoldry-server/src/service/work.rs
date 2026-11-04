//! Workflows phase 5: My Work.
//!
//! The items a step opens: one for each person who may act on it, with how they came to (`holder`, `delegate`,
//! `assigned`, `escalated`), and a `waiting` item for the person who started the process and the record's creator. The
//! items a proposal opens: a `review` for each owner and admin of each party, except the proposer, who waits. A list
//! shows only the items a person can still act on: the app's read, or a task grant. Acting re-checks everything live,
//! through the same service functions as every other route.
//!
//! Drift is repaired by `reresolve_work`, after a change of who holds what, and by the nightly `reconcile_work`. Both
//! resolve every waiting instance live and open or close items to match.

use crate::repository_work::{NewWork, WorkFilter, WorkItem, WorkKind, WorkText};
use crate::service::access::{authorize_app, AppAction};
use crate::service::history::Origin;
use crate::service::ServiceError;
use crate::state::{AuthUser, SharedState};
use scaffoldry_core::approver::Via;
use scaffoldry_core::workflow::{find_step, TaskKind};
use scaffoldry_core::{AutomationRule, ProcessInstance, ProcessStatus, StepKind};
use scaffoldry_engine::HostRouter;
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};

/// The most items one page returns.
pub const MAX_PAGE: i64 = 200;

fn app_names(state: &SharedState, slug: &str) -> (String, Option<String>) {
    let eng = state.engine.read().ok();
    let m = eng.as_ref().and_then(|e| e.resolve_by_slug(slug).cloned());
    (m.as_ref().map(|m| m.title.clone()).unwrap_or_else(|| slug.to_string()), m.and_then(|m| m.workspace_id))
}

/// Who may act on the step an instance waits at, and how: the committee's voters, or the assignee, the people the
/// approver resolves to with their delegates, and the people a timer escalated it to.
fn actors(state: &SharedState, inst: &ProcessInstance) -> Vec<(String, &'static str, Option<String>)> {
    let mut out: Vec<(String, &'static str, Option<String>)> = vec![];
    let mut push = |who: String, via: &'static str, acting_for: Option<String>| {
        if !out.iter().any(|(p, _, _)| p.eq_ignore_ascii_case(&who)) {
            out.push((who, via, acting_for));
        }
    };
    let resolved: Vec<(String, &'static str, Option<String>)> = crate::service::approvers::resolution_for(state, inst)
        .map(|(_, _, r)| {
            r.approvers
                .into_iter()
                .map(|a| match a.via {
                    Via::DelegateOf(holder) => (a.eppn, "delegate", Some(holder)),
                    Via::Named => (a.eppn, "assigned", None),
                    Via::Holder | Via::Role => (a.eppn, "holder", None),
                })
                .collect()
        })
        .unwrap_or_default();
    if !inst.voters.is_empty() {
        for v in &inst.voters {
            let (via, acting_for) = resolved.iter().find(|(p, _, _)| p.eq_ignore_ascii_case(v)).map(|(_, via, a)| (*via, a.clone())).unwrap_or(("holder", None));
            push(v.clone(), via, acting_for);
        }
    } else if let Some(who) = &inst.assigned_to {
        push(who.clone(), "assigned", None);
    } else {
        for (p, via, a) in resolved {
            push(p, via, a);
        }
    }
    for e in &inst.escalated_to {
        push(e.clone(), "escalated", None);
    }
    out
}

/// The items a waiting instance should have open now.
pub fn work_items_for(state: &SharedState, rule: &AutomationRule, inst: &ProcessInstance, due: Option<chrono::DateTime<chrono::Utc>>) -> Vec<NewWork> {
    if inst.status != ProcessStatus::Waiting {
        return vec![];
    }
    let Some(step) = inst.waiting_step_id.as_deref() else { return vec![] };
    let kind = match find_step(&rule.steps, step).map(|s| &s.kind) {
        Some(StepKind::UserTask { kind: TaskKind::Complete, .. }) => WorkKind::Task,
        Some(StepKind::UserTask { .. }) => WorkKind::Decision,
        _ => return vec![],
    };
    let (app_title, workspace) = app_names(state, &inst.app_slug);
    let item = |person: String, kind: WorkKind, via: &'static str, acting_for: Option<String>| NewWork {
        person,
        kind,
        app_slug: Some(inst.app_slug.clone()),
        workspace_id: workspace.clone(),
        flow_id: Some(inst.rule_id.clone()),
        instance_id: Some(inst.id.clone()),
        step_id: Some(step.to_string()),
        record_id: Some(inst.record_id.clone()),
        proposal_id: None,
        text: WorkText::for_step(kind, &rule.name, &app_title),
        via,
        acting_for,
        due_at: due,
    };
    let mut out: Vec<NewWork> = actors(state, inst).into_iter().map(|(p, via, a)| item(p, kind, via, a)).collect();
    let creator = state.find_record(&inst.app_slug, &inst.record_id).ok().flatten().and_then(|r| r.created_by);
    let mut waiting: Vec<String> = vec![];
    for who in inst.started_by.iter().chain(creator.iter()) {
        if !who.is_empty() && !waiting.iter().any(|w| w.eq_ignore_ascii_case(who)) {
            waiting.push(who.clone());
        }
    }
    out.extend(waiting.into_iter().map(|p| item(p, WorkKind::Waiting, "holder", None)));
    out
}

/// The items a proposal opens: a review for each owner and admin of each party, except the proposer, and a wait for the
/// proposer.
pub fn proposal_items(state: &SharedState, workspaces: &[String], app_slug: &str, proposer: &str, proposal: uuid::Uuid) -> Vec<NewWork> {
    let (app_title, _) = app_names(state, app_slug);
    let item = |person: String, kind: WorkKind, ws: &str| NewWork {
        person,
        kind,
        app_slug: Some(app_slug.to_string()),
        workspace_id: Some(ws.to_string()),
        flow_id: None,
        instance_id: None,
        step_id: None,
        record_id: None,
        proposal_id: Some(proposal),
        text: WorkText::for_proposal(kind, &app_title),
        via: "holder",
        acting_for: None,
        due_at: None,
    };
    let mut out = vec![];
    let mut seen: HashSet<String> = HashSet::new();
    for ws in workspaces {
        for c in crate::service::members::people_records(ws, state) {
            let role = c.role.to_ascii_lowercase();
            if (role == "owner" || role == "admin") && !c.eppn.eq_ignore_ascii_case(proposer) && seen.insert(c.eppn.to_lowercase()) {
                out.push(item(c.eppn, WorkKind::Review, ws));
            }
        }
    }
    if let Some(home) = workspaces.first() {
        out.push(item(proposer.to_string(), WorkKind::Waiting, home));
    }
    out
}

fn key(person: &str, kind: &str, via: &str, instance: &str) -> String {
    format!("{}|{kind}|{via}|{instance}", person.to_lowercase())
}

/// Resolves every waiting instance live and opens or closes items to match. Returns how many repairs it made.
fn repair(state: &SharedState, reason: &str) -> Result<usize, String> {
    let open = state.repository.open_instance_work().map_err(|e| e.to_string())?;
    let mut by_instance: HashMap<String, Vec<WorkItem>> = HashMap::new();
    for w in open {
        by_instance.entry(w.instance_id.clone().unwrap_or_default()).or_default().push(w);
    }
    let waiting = state.repository.list_process_instances(None, Some("Waiting")).map_err(|e| e.to_string())?;
    let mut to_open = vec![];
    let mut to_close = vec![];
    let mut seen_instances: HashSet<String> = HashSet::new();
    for inst in &waiting {
        seen_instances.insert(inst.id.clone());
        let Some(rule) = crate::service::processes::rule_for(state, inst) else { continue };
        let have = by_instance.remove(&inst.id).unwrap_or_default();
        let due = have.iter().find_map(|w| w.due_at).or_else(|| state.repository.next_timer(&inst.id).ok().flatten().map(|(at, _)| at));
        let want = work_items_for(state, &rule, inst, due);
        let want_keys: HashSet<String> = want.iter().map(|w| key(&w.person, w.kind.as_str(), w.via, &inst.id)).collect();
        let have_keys: HashSet<String> = have.iter().map(|w| key(&w.person, &w.kind, &w.via, &inst.id)).collect();
        to_open.extend(want.into_iter().filter(|w| !have_keys.contains(&key(&w.person, w.kind.as_str(), w.via, &inst.id))));
        to_close.extend(have.iter().filter(|w| !want_keys.contains(&key(&w.person, &w.kind, &w.via, &inst.id))).map(|w| w.id));
    }
    // Items of instances that are no longer waiting.
    for (_, items) in by_instance {
        to_close.extend(items.into_iter().map(|w| w.id));
    }
    if to_open.is_empty() && to_close.is_empty() {
        return Ok(0);
    }
    state.repository.repair_work(to_open, to_close, reason).map_err(|e| e.to_string())
}

/// After a change to who holds a position, a delegation, a hold, a group or a person.
pub fn reresolve(state: &SharedState) -> Result<usize, String> {
    repair(state, "reresolved")
}

/// The nightly repair. Its count is the job's result, which the Overview shows.
pub fn reconcile(state: &SharedState) -> Result<usize, String> {
    repair(state, "reconciled")
}

pub fn reresolve_job(ctx: &crate::jobs::JobCtx, _payload: Value) -> Result<Value, crate::jobs::JobError> {
    let state = ctx.state().ok_or_else(|| crate::jobs::JobError::Permanent("re-resolving work needs the server's state".to_string()))?;
    Ok(json!({ "repairs": reresolve(state).map_err(crate::jobs::JobError::Retry)? }))
}

pub fn reconcile_job(ctx: &crate::jobs::JobCtx, _payload: Value) -> Result<Value, crate::jobs::JobError> {
    let state = ctx.state().ok_or_else(|| crate::jobs::JobError::Permanent("reconciling work needs the server's state".to_string()))?;
    Ok(json!({ "repairs": reconcile(state).map_err(crate::jobs::JobError::Retry)? }))
}

/// Asks for a re-resolve. Who holds what changed, so open items may name the wrong people until it runs. The decide
/// route never trusts an item, so nothing waits on it.
pub fn enqueue_reresolve(state: &SharedState, by: &str) {
    // One waiting re-resolve covers every change before it runs.
    let _ = crate::jobs::enqueue(&state.repository, "reresolve_work", json!({}), by, Some("reresolve_work"));
}

/// Whether a person may still see an item: the app's read, a task grant on the step, or for a proposal, a place in its
/// workspace.
fn visible(caller: &AuthUser, w: &WorkItem, state: &SharedState) -> bool {
    if let Some(inst) = &w.instance_id {
        if state.repository.holds_task_grant(&caller.eppn, inst).unwrap_or(false) {
            return true;
        }
    }
    if w.proposal_id.is_some() {
        if let Some(ws) = &w.workspace_id {
            return crate::service::members::people_records(ws, state).iter().any(|c| c.eppn.eq_ignore_ascii_case(&caller.eppn))
                || crate::service::library::is_admin(state, caller);
        }
    }
    match &w.app_slug {
        Some(slug) => authorize_app(caller, slug, AppAction::Read, state).is_ok(),
        None => false,
    }
}

fn view(w: &WorkItem) -> Value {
    serde_json::to_value(w).unwrap_or(Value::Null)
}

/// `GET /me/work`: the caller's open items, by due date then age.
pub fn list(caller: &AuthUser, filter: &WorkFilter, cursor: Option<&str>, limit: i64, state: &SharedState) -> Result<Value, ServiceError> {
    let (items, next) = state.repository.work_page(&caller.eppn, filter, cursor, limit.clamp(1, MAX_PAGE)).map_err(|e| ServiceError::StorageUnavailable(e.to_string()))?;
    let shown: Vec<Value> = items.iter().filter(|w| visible(caller, w, state)).map(view).collect();
    Ok(json!({ "items": shown, "next_cursor": next }))
}

/// `GET /me/work/summary`.
pub fn summary(caller: &AuthUser, state: &SharedState) -> Result<Value, ServiceError> {
    let (kinds, overdue, oldest) = state.repository.work_summary(&caller.eppn).map_err(|e| ServiceError::StorageUnavailable(e.to_string()))?;
    let by_kind: serde_json::Map<String, Value> = kinds.into_iter().map(|(k, n)| (k, json!(n))).collect();
    let oldest_days = oldest.map(|t| (chrono::Utc::now() - t).num_days());
    Ok(json!({ "by_kind": by_kind, "overdue": overdue, "oldest_days": oldest_days }))
}

fn own_item(caller: &AuthUser, id: &str, state: &SharedState) -> Result<WorkItem, ServiceError> {
    let id: uuid::Uuid = id.parse().map_err(|_| ServiceError::NotFound("Work item not found".to_string()))?;
    state
        .repository
        .work_item(id)
        .map_err(|e| ServiceError::StorageUnavailable(e.to_string()))?
        .filter(|w| w.person.eq_ignore_ascii_case(&caller.eppn) && visible(caller, w, state))
        .ok_or_else(|| ServiceError::NotFound("Work item not found".to_string()))
}

/// `GET /me/work/{id}`: the item and what is needed to act on it.
pub fn get(caller: &AuthUser, id: &str, state: &SharedState) -> Result<Value, ServiceError> {
    let w = own_item(caller, id, state)?;
    let mut out = json!({ "item": view(&w) });
    let Some(inst_id) = &w.instance_id else { return Ok(out) };
    let Some(inst) = state.repository.get_process_instance(inst_id).map_err(|e| ServiceError::StorageUnavailable(e.to_string()))? else { return Ok(out) };
    let rule = crate::service::processes::rule_for(state, &inst);
    let step = rule.as_ref().zip(w.step_id.as_deref()).and_then(|(r, s)| find_step(&r.steps, s).cloned());
    let (show, fill) = match step.as_ref().map(|s| &s.kind) {
        Some(StepKind::UserTask { show, fill, .. }) => (show.clone(), fill.clone()),
        _ => (vec![], vec![]),
    };
    // The record as this person may see it: through their own rights, or through the task's grant.
    let values = crate::service::records::fetch_record(caller, &inst.app_slug, &inst.record_id, state).ok().map(|r| {
        let mut m = serde_json::Map::new();
        for f in show.iter().chain(fill.iter()) {
            if let Some(v) = r.data.get(f) {
                m.insert(f.clone(), v.clone());
            }
        }
        Value::Object(m)
    });
    let history: Vec<String> = state
        .repository
        .with_client({
            let id = inst.id.clone();
            move |c| Ok(c.query("SELECT kind, step_id, at FROM process_events WHERE instance_id = $1 ORDER BY at, id", &[&id])?.iter().map(|r| (r.get::<_, String>(0), r.get::<_, Option<String>>(1), r.get::<_, chrono::DateTime<chrono::Utc>>(2))).collect::<Vec<_>>())
        })
        .unwrap_or_default()
        .into_iter()
        .map(|(kind, step, at)| event_words(&kind, step.as_deref(), at))
        .collect();
    if let Some(map) = out.as_object_mut() {
        map.insert("instance".to_string(), crate::service::approvers::instance_view(state, &inst));
        map.insert("prompt".to_string(), json!(inst.prompt));
        map.insert("show".to_string(), values.unwrap_or(Value::Null));
        map.insert("fill".to_string(), json!(fill));
        map.insert("history".to_string(), json!(history));
        if let Some(t) = crate::service::process_votes::tally_view(state, &inst) {
            map.insert("tally".to_string(), t);
        }
    }
    Ok(out)
}

/// A process event in words, for the item's history.
fn event_words(kind: &str, step: Option<&str>, at: chrono::DateTime<chrono::Utc>) -> String {
    let when = at.format("%-d %b %H:%M");
    let step = step.map(|s| format!(" '{s}'")).unwrap_or_default();
    match kind {
        "started" => format!("{when}: the process started."),
        "step_entered" => format!("{when}: it reached the step{step}."),
        "waiting" => format!("{when}: the step{step} began to wait."),
        "voted" => format!("{when}: a vote was cast on{step}."),
        "decided" => format!("{when}: the step{step} was decided."),
        "timer_fired" => format!("{when}: a timer fired on{step}."),
        "step_left" => format!("{when}: it left the step{step}."),
        "completed" => format!("{when}: the process ended."),
        "branch" => format!("{when}: a branch was chosen."),
        other => format!("{when}: {other}."),
    }
}

/// `POST /me/work/{id}/act`: `approve` or `reject` with a comment, `done` with field values, `decline` with a reason,
/// or a vote. It runs the same service functions as the other routes, which check everything live.
pub fn act_on_work(caller: &AuthUser, origin: &Origin, id: &str, body: &Value, state: &SharedState) -> Result<Value, ServiceError> {
    let w = own_item(caller, id, state)?;
    let decision = body.get("decision").and_then(Value::as_str).ok_or_else(|| ServiceError::bad_request("Say what you decide: approve, reject, done or decline."))?.to_lowercase();
    let comment = body.get("comment").or_else(|| body.get("reason")).and_then(Value::as_str);
    if let Some(pid) = w.proposal_id {
        let rationale = comment.unwrap_or("Decided from My Work");
        return crate::service::proposals::decide(caller, &pid.to_string(), &decision, rationale, body.get("digest").and_then(Value::as_str), state);
    }
    let (Some(slug), Some(inst)) = (w.app_slug.as_deref(), w.instance_id.as_deref()) else {
        return Err(ServiceError::bad_request("This item is waiting on someone else. There is nothing for you to do."));
    };
    if w.kind == "waiting" {
        return Err(ServiceError::bad_request("This item is waiting on someone else. There is nothing for you to do."));
    }
    // A task's fields are written first, through the person's own rights or the task's grant.
    if decision == "done" {
        if let (Some(fields), Some(record)) = (body.get("fields").filter(|f| f.as_object().is_some_and(|m| !m.is_empty())), w.record_id.as_deref()) {
            let version = state.find_record(slug, record).map_err(ServiceError::Internal)?.map(|r| r.version).ok_or_else(|| ServiceError::NotFound("Record not found".to_string()))?;
            crate::service::records::update_record(caller, origin, slug, record, &json!({ "data": fields, "version": version }), state)?;
        }
    }
    let done = crate::service::processes::decide_process(caller, origin, slug, inst, &decision, comment, state)?;
    Ok(json!({ "instance": done }))
}
