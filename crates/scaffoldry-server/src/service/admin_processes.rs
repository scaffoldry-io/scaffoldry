//! Admin console phase 6: the Business processes panel. It reads the definitions and their versions, turns one off or on
//! through `save_rule`, simulates one, and lists the instances that wait, with who can decide each. An administrator may
//! reassign a waiting instance or cancel it. Nothing here shows a record's contents.
//!
//! Turning a rule off or on is a new version saved by `save_rule` with no proposal. `save_rule` writes the one ledger entry.
//! A reassign or a cancel writes its ledger entry first, through `admin_write`, and changes the instance only if that
//! succeeded.

use crate::service::access::{authorize_app, AppAction};
use crate::service::admin::{admin_write, decode_cursor, encode_cursor, require_platform_admin, require_reason};
use crate::service::{approvers, flows, identity, processes, ServiceError};
use crate::state::{AuthUser, SharedState};
use chrono::{DateTime, Duration, Utc};
use scaffoldry_core::approver::{effective_approver, ApproverSpec};
use scaffoldry_core::ledger::DecisionType;
use scaffoldry_core::workflow::{all_steps, describe_rule};
use scaffoldry_core::{AutomationRule, ProcessInstance, ProcessStatus, StepKind, TriggerEvent};
use serde_json::{json, Value};
use std::collections::HashMap;

const DEFAULT_STALE_DAYS: i64 = 14;
const STATUSES: [&str; 4] = ["Waiting", "Completed", "Rejected", "Failed"];

fn storage(e: crate::repository::RepositoryError) -> ServiceError {
    ServiceError::StorageUnavailable(e.to_string())
}

fn status_name(s: &ProcessStatus) -> &'static str {
    match s {
        ProcessStatus::Waiting => "Waiting",
        ProcessStatus::Completed => "Completed",
        ProcessStatus::Rejected => "Rejected",
        ProcessStatus::Failed => "Failed",
    }
}

/// The `process.stale_days` setting, 14 unless set.
fn stale_days(state: &SharedState) -> i64 {
    state.settings.read().ok().and_then(|s| s.get("process.stale_days").and_then(Value::as_i64)).unwrap_or(DEFAULT_STALE_DAYS)
}

fn started(inst: &ProcessInstance) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(&inst.started_at).ok().map(|t| t.with_timezone(&Utc))
}

fn age_days(inst: &ProcessInstance, now: DateTime<Utc>) -> i64 {
    started(inst).map_or(0, |t| (now - t).num_days().max(0))
}

/// A waiting instance older than the `process.stale_days` setting.
fn is_stale(inst: &ProcessInstance, days: i64, now: DateTime<Utc>) -> bool {
    inst.status == ProcessStatus::Waiting && started(inst).is_some_and(|t| now - t > Duration::days(days))
}

/// How many instances wait, and how many of those are stale. For the Overview.
pub fn counts(state: &SharedState) -> (i64, i64) {
    let waiting = state.repository.list_process_instances(None, Some("Waiting")).unwrap_or_default();
    let (days, now) = (stale_days(state), Utc::now());
    (waiting.len() as i64, waiting.iter().filter(|i| is_stale(i, days, now)).count() as i64)
}

/// Why nobody can decide, in words.
fn problem_words(code: &str) -> String {
    match code {
        "position_vacant" => "The position that decides this step is vacant.",
        "only_submitter" => "The only person who could decide it made the record or started the process, and may not.",
        "person_unavailable" => "The person named to decide this step is inactive or on hold.",
        "role_empty" => "Nobody in the workspace has the role that decides this step.",
        "too_few_voters" => "The committee has fewer voters than it needs.",
        _ => "Nobody can decide this step now.",
    }
    .to_string()
}

fn query_flag(q: &HashMap<String, String>, key: &str) -> Result<Option<bool>, ServiceError> {
    match q.get(key).map(String::as_str) {
        None | Some("") => Ok(None),
        Some("true") => Ok(Some(true)),
        Some("false") => Ok(Some(false)),
        Some(_) => Err(ServiceError::BadRequest(format!("'{key}' must be true or false"))),
    }
}

/// The offset a cursor names, and the page size: 1 to 200, 50 unless asked.
fn page_args(q: &HashMap<String, String>) -> Result<(usize, usize), ServiceError> {
    let offset = match q.get("cursor").filter(|c| !c.is_empty()) {
        Some(c) => decode_cursor(c)?,
        None => 0,
    };
    let limit = match q.get("limit").filter(|l| !l.is_empty()) {
        Some(l) => l.parse::<usize>().map_err(|_| ServiceError::BadRequest("'limit' must be a number".to_string()))?.clamp(1, 200),
        None => 50,
    };
    Ok((offset, limit))
}

fn page_of(rows: Vec<Value>, next: Option<usize>) -> Value {
    json!({ "rows": rows, "next_cursor": next.map(encode_cursor) })
}

fn workspace_of(state: &SharedState, app_slug: &str) -> (Option<String>, String) {
    let id = state.engine.read().ok().and_then(|e| scaffoldry_engine::HostRouter::resolve_by_slug(&*e, app_slug).and_then(|m| m.workspace_id.clone()));
    let name = id.as_ref().and_then(|i| state.workspaces.read().ok().and_then(|w| w.get(i).map(|w| w.name.clone()))).unwrap_or_default();
    (id, name)
}

/// Every process definition: a rule that is not a flow, which has its own panel.
fn all_rules(state: &SharedState) -> Vec<AutomationRule> {
    let mut out: Vec<AutomationRule> = state.automations.read().map(|m| m.values().flatten().filter(|r| !flows::is_flow(r)).cloned().collect()).unwrap_or_default();
    out.sort_by(|a, b| (&a.app_slug, &a.id).cmp(&(&b.app_slug, &b.id)));
    out
}

fn find_rule(state: &SharedState, rule_id: &str) -> Result<AutomationRule, ServiceError> {
    all_rules(state).into_iter().find(|r| r.id == rule_id).ok_or_else(|| ServiceError::NotFound(format!("There is no process '{rule_id}'")))
}

fn user_task_roles(rule: &AutomationRule) -> Vec<String> {
    let mut roles: Vec<String> = all_steps(&rule.steps)
        .into_iter()
        .filter_map(|s| match &s.kind {
            StepKind::UserTask { role, approver, .. } => Some(match effective_approver(role, approver) {
                ApproverSpec::Role { name } => name,
                ApproverSpec::Position { key, .. } => key,
                ApproverSpec::Person { eppn } => eppn,
            }),
            _ => None,
        })
        .collect();
    roles.dedup();
    roles
}

/// `GET /admin/processes`. Query: `app`, `enabled`, `search`, `cursor`, `limit`.
pub fn list(caller: &AuthUser, q: &HashMap<String, String>, state: &SharedState) -> Result<Value, ServiceError> {
    require_platform_admin(caller, state)?;
    let (offset, limit) = page_args(q)?;
    let enabled = query_flag(q, "enabled")?;
    let search = q.get("search").map(|s| s.trim().to_lowercase()).filter(|s| !s.is_empty());
    let app = q.get("app").filter(|a| !a.is_empty());
    let rules: Vec<AutomationRule> = all_rules(state)
        .into_iter()
        .filter(|r| app.is_none_or(|a| &r.app_slug == a))
        .filter(|r| enabled.is_none_or(|e| r.enabled == e))
        .filter(|r| search.as_ref().is_none_or(|s| r.name.to_lowercase().contains(s) || r.id.to_lowercase().contains(s) || r.app_slug.to_lowercase().contains(s)))
        .collect();
    let next = (rules.len() > offset + limit).then_some(offset + limit);
    let page: Vec<AutomationRule> = rules.into_iter().skip(offset).take(limit).collect();
    let ids: Vec<String> = page.iter().map(|r| r.id.clone()).collect();
    let mut waiting: HashMap<String, i64> = HashMap::new();
    for inst in state.repository.list_process_instances(None, Some("Waiting")).map_err(storage)? {
        *waiting.entry(inst.rule_id).or_default() += 1;
    }
    let changed: HashMap<String, (String, DateTime<Utc>)> = state.repository.rule_last_changes(&ids).map_err(storage)?.into_iter().map(|(id, by, at)| (id, (by, at))).collect();
    let rows = page
        .iter()
        .map(|r| {
            let (ws_id, ws_name) = workspace_of(state, &r.app_slug);
            let last = changed.get(&r.id);
            json!({
                "rule_id": r.id,
                "name": r.name,
                "app_slug": r.app_slug,
                "workspace_id": ws_id,
                "workspace_name": ws_name,
                "version": r.version,
                "enabled": r.enabled,
                "trigger": describe_rule(r).first().cloned().unwrap_or_default(),
                "step_count": all_steps(&r.steps).len(),
                "user_task_roles": user_task_roles(r),
                "waiting_count": waiting.get(&r.id).copied().unwrap_or(0),
                "last_changed_by": last.map(|l| l.0.clone()),
                "last_changed_at": last.map(|l| l.1.to_rfc3339()),
            })
        })
        .collect();
    Ok(page_of(rows, next))
}

/// `GET /admin/processes/{rule_id}`: the live rule, its sentences, its versions, and its instances by status.
pub fn detail(caller: &AuthUser, rule_id: &str, state: &SharedState) -> Result<Value, ServiceError> {
    require_platform_admin(caller, state)?;
    let rule = find_rule(state, rule_id)?;
    let versions: Vec<Value> = state
        .repository
        .rule_versions(rule_id)
        .map_err(storage)?
        .iter()
        .map(|v| json!({
            "version": v.version,
            "changed_by": v.author,
            "changed_at": v.at.to_rfc3339(),
            "summary": v.summary,
            "proposal_id": v.proposal_id,
            "sha256": crate::repository_proposals::manifest_sha256(&v.rule),
        }))
        .collect();
    let mut counts: HashMap<&str, i64> = STATUSES.iter().map(|s| (*s, 0)).collect();
    for inst in state.repository.list_process_instances(Some(&rule.app_slug), None).map_err(storage)?.iter().filter(|i| i.rule_id == rule_id) {
        *counts.entry(status_name(&inst.status)).or_default() += 1;
    }
    let (ws_id, ws_name) = workspace_of(state, &rule.app_slug);
    Ok(json!({
        "rule": rule,
        "sentences": describe_rule(&rule),
        "workspace_id": ws_id,
        "workspace_name": ws_name,
        "versions": versions,
        "instance_counts": counts,
    }))
}

/// `GET /admin/processes/{rule_id}/versions/{v}`: one version as it was, read-only.
pub fn version(caller: &AuthUser, rule_id: &str, v: &str, state: &SharedState) -> Result<Value, ServiceError> {
    require_platform_admin(caller, state)?;
    let v: i32 = v.parse().map_err(|_| ServiceError::BadRequest("The version is a whole number".to_string()))?;
    find_rule(state, rule_id)?;
    let rows = state.repository.rule_versions(rule_id).map_err(storage)?;
    let row = rows.iter().find(|r| r.version == v).ok_or_else(|| ServiceError::NotFound(format!("There is no version {v} of this process")))?;
    let rule: AutomationRule = serde_json::from_value(row.rule.clone()).map_err(|e| ServiceError::Internal(e.to_string()))?;
    Ok(json!({
        "version": row.version,
        "changed_by": row.author,
        "changed_at": row.at.to_rfc3339(),
        "summary": row.summary,
        "sha256": crate::repository_proposals::manifest_sha256(&row.rule),
        "rule": row.rule,
        "sentences": describe_rule(&rule),
    }))
}

/// `PATCH /admin/processes/{rule_id}`: turns a rule off or on. Off is always allowed; on is checked in full. A new version through `save_rule` with no proposal, because this
/// is the administrator's own authority. `save_rule` writes the ledger entry, and the reason is its rationale. A rule that
/// is off starts no new instance. Instances that wait can still be decided.
pub fn set_enabled(caller: &AuthUser, rule_id: &str, body: &Value, state: &SharedState) -> Result<Value, ServiceError> {
    require_platform_admin(caller, state)?;
    let reason = require_reason(body.get("reason").and_then(Value::as_str).unwrap_or(""))?.to_string();
    let enabled = body.get("enabled").and_then(Value::as_bool).ok_or_else(|| ServiceError::BadRequest("Say whether the process is 'enabled': true or false".to_string()))?;
    let mut rule = find_rule(state, rule_id)?;
    if rule.enabled == enabled {
        return Err(ServiceError::BadRequest(format!("The process is already {}", if enabled { "on" } else { "off" })));
    }
    let summary = format!("Turned {} by {}. Reason: {reason}", if enabled { "on" } else { "off" }, caller.eppn);
    let app = rule.app_slug.clone();
    // Turning off is never refused for what is wrong with the rule. Turning on checks it in full.
    let saved = if enabled {
        rule.enabled = true;
        processes::save_rule(&caller.eppn, &app, rule, None, &summary, state)?
    } else {
        processes::save_rule_disabled(&caller.eppn, &app, rule, &summary, state)?
    };
    Ok(json!({ "rule_id": saved.id, "enabled": saved.enabled, "version": saved.version }))
}

/// `POST /admin/processes/{rule_id}/simulate`: the app's simulate function, run as `principal`. Body: `record` or `record_id`,
/// `principal`, and optionally `event` (the rule's own trigger when left out).
pub fn simulate(caller: &AuthUser, rule_id: &str, body: &Value, state: &SharedState) -> Result<Value, ServiceError> {
    require_platform_admin(caller, state)?;
    let rule = find_rule(state, rule_id)?;
    let who = body.get("principal").and_then(Value::as_str).map(str::trim).filter(|p| !p.is_empty()).ok_or_else(|| ServiceError::BadRequest("Name the person to simulate as, in 'principal'".to_string()))?;
    let principal = identity::resolve_user(who, state).ok_or_else(|| ServiceError::BadRequest(format!("The platform does not know a person called '{who}'")))?;
    let record = match (body.get("record"), body.get("record_id").and_then(Value::as_str)) {
        (Some(r), _) if r.is_object() => r.clone(),
        (_, Some(id)) => state.find_record(&rule.app_slug, id).map_err(ServiceError::Internal)?.ok_or_else(|| ServiceError::NotFound("That record does not exist".to_string()))?.data,
        _ => return Err(ServiceError::BadRequest("Send a sample 'record', or a 'record_id'".to_string())),
    };
    let event: TriggerEvent = match body.get("event") {
        Some(e) if !e.is_null() => serde_json::from_value(e.clone()).map_err(|e| ServiceError::BadRequest(format!("Invalid trigger event: {e}")))?,
        _ => rule.trigger.clone(),
    };
    let results = processes::simulate_rules(state, &rule.app_slug, &principal, &event, &record)?;
    let result = results.into_iter().find(|r| r.rule_id == rule_id).ok_or_else(|| ServiceError::NotFound("The process is not live in its app".to_string()))?;
    Ok(json!({ "result": result }))
}

fn instance_row(state: &SharedState, inst: &ProcessInstance, rule_names: &HashMap<String, String>, days: i64, now: DateTime<Utc>) -> Value {
    let view = approvers::instance_view(state, inst);
    let deciders: Vec<String> = view["approvers"].as_array().map(|a| a.iter().filter_map(|p| p["display_name"].as_str().map(str::to_string)).collect()).unwrap_or_default();
    json!({
        "id": inst.id,
        "app_slug": inst.app_slug,
        "rule_id": inst.rule_id,
        "rule_name": rule_names.get(&inst.rule_id).cloned().unwrap_or_else(|| inst.rule_id.clone()),
        "rule_version": inst.rule_version,
        "record_id": inst.record_id,
        "status": status_name(&inst.status),
        "waiting_step_id": inst.waiting_step_id,
        "deciders": deciders,
        "no_approver": inst.no_approver,
        "no_approver_reason": inst.no_approver.as_deref().map(problem_words),
        "assigned_to": inst.assigned_to,
        "started_at": inst.started_at,
        "age_days": age_days(inst, now),
        "stale": is_stale(inst, days, now),
    })
}

fn rule_names(state: &SharedState) -> HashMap<String, String> {
    state.automations.read().map(|m| m.values().flatten().map(|r| (r.id.clone(), r.name.clone())).collect()).unwrap_or_default()
}

/// `GET /admin/process-instances`. Query: `status`, `app`, `rule`, `search`, `stale`, `assigned`, `unassigned`, `cursor`, `limit`.
/// Oldest first, so what is stuck comes first.
pub fn list_instances(caller: &AuthUser, q: &HashMap<String, String>, state: &SharedState) -> Result<Value, ServiceError> {
    require_platform_admin(caller, state)?;
    let (offset, limit) = page_args(q)?;
    let status = match q.get("status").map(String::as_str).filter(|s| !s.is_empty()) {
        Some(s) => Some(STATUSES.iter().find(|n| n.eq_ignore_ascii_case(s)).copied().ok_or_else(|| ServiceError::BadRequest("'status' is Waiting, Completed, Rejected or Failed".to_string()))?),
        None => None,
    };
    let (stale, assigned, unassigned) = (query_flag(q, "stale")?, query_flag(q, "assigned")?, query_flag(q, "unassigned")?);
    let app = q.get("app").filter(|a| !a.is_empty());
    let rule = q.get("rule").filter(|r| !r.is_empty());
    let (days, now) = (stale_days(state), Utc::now());
    let mut all = state.repository.list_process_instances(app.map(String::as_str), status).map_err(storage)?;
    all.retain(|i| rule.is_none_or(|r| &i.rule_id == r));
    if let Some(search) = q.get("search").map(|s| s.trim().to_lowercase()).filter(|s| !s.is_empty()) {
        let names = rule_names(state);
        all.retain(|i| i.id.to_lowercase().contains(&search) || i.app_slug.to_lowercase().contains(&search) || names.get(&i.rule_id).is_some_and(|n| n.to_lowercase().contains(&search)));
    }
    // Who may decide changes as people are appointed, so the flag is worked out again here. It is not written back.
    if unassigned.is_some() {
        for inst in all.iter_mut() {
            approvers::refresh_no_approver(state, inst);
        }
    }
    all.retain(|i| {
        stale.is_none_or(|s| is_stale(i, days, now) == s)
            && assigned.is_none_or(|a| i.assigned_to.is_some() == a)
            && unassigned.is_none_or(|u| (i.status == ProcessStatus::Waiting && i.no_approver.is_some()) == u)
    });
    all.sort_by(|a, b| (&a.started_at, &a.id).cmp(&(&b.started_at, &b.id)));
    let next = (all.len() > offset + limit).then_some(offset + limit);
    let names = rule_names(state);
    let rows = all
        .into_iter()
        .skip(offset)
        .take(limit)
        .map(|mut inst| {
            approvers::refresh_no_approver(state, &mut inst);
            instance_row(state, &inst, &names, days, now)
        })
        .collect();
    Ok(page_of(rows, next))
}

fn instance(state: &SharedState, id: &str) -> Result<ProcessInstance, ServiceError> {
    state.repository.get_process_instance(id).map_err(storage)?.ok_or_else(|| ServiceError::NotFound("There is no such process instance".to_string()))
}

/// `GET /admin/process-instances/{id}`: the instance and its log. Not the record's contents.
pub fn instance_detail(caller: &AuthUser, id: &str, state: &SharedState) -> Result<Value, ServiceError> {
    require_platform_admin(caller, state)?;
    let mut inst = instance(state, id)?;
    approvers::refresh_no_approver(state, &mut inst);
    let mut row = instance_row(state, &inst, &rule_names(state), stale_days(state), Utc::now());
    row["log"] = json!(inst.log);
    row["prompt"] = json!(inst.prompt);
    row["started_by"] = json!(inst.started_by);
    Ok(row)
}

/// `GET /admin/process-instances/{id}/approvers`: who can decide now, with how each came to be one, and the unit a position was
/// found at.
pub fn instance_approvers(caller: &AuthUser, id: &str, state: &SharedState) -> Result<Value, ServiceError> {
    require_platform_admin(caller, state)?;
    let inst = instance(state, id)?;
    if inst.status != ProcessStatus::Waiting {
        return Ok(json!({ "instance_id": inst.id, "approvers": [], "unit_id": null, "unit_name": null, "problem": null, "problem_reason": null }));
    }
    let view = approvers::instance_view(state, &inst);
    let (unit_id, unit_name, problem) = match approvers::resolution_for(state, &inst) {
        Some((_, ctx, r)) => {
            let unit_name = r.unit_id.as_deref().and_then(|u| ctx.units.iter().find(|x| x.id == u).map(|x| x.name.clone()));
            let problem = if inst.assigned_to.is_some() { None } else { r.problem.map(approvers::problem_code) };
            (r.unit_id, unit_name, problem)
        }
        None => (None, None, None),
    };
    Ok(json!({
        "instance_id": inst.id,
        "assigned_to": inst.assigned_to,
        "approvers": view["approvers"],
        "unit_id": unit_id,
        "unit_name": unit_name,
        "problem": problem,
        "problem_reason": problem.as_deref().map(problem_words),
    }))
}

/// `POST /admin/process-instances/{id}/reassign`. `eppn` names the person who decides from now on, or is null to return the step
/// to its role.
pub fn reassign(caller: &AuthUser, id: &str, body: &Value, state: &SharedState) -> Result<Value, ServiceError> {
    require_platform_admin(caller, state)?;
    let reason = require_reason(body.get("reason").and_then(Value::as_str).unwrap_or(""))?.to_string();
    let mut inst = instance(state, id)?;
    if inst.status != ProcessStatus::Waiting {
        return Err(ServiceError::Conflict("Only a waiting process instance can be reassigned".to_string()));
    }
    let target: Option<String> = match body.get("eppn") {
        None | Some(Value::Null) => None,
        Some(Value::String(s)) if !s.trim().is_empty() => Some(s.trim().to_string()),
        Some(_) => return Err(ServiceError::BadRequest("'eppn' is a person's user name, or null".to_string())),
    };
    if let Some(who) = &target {
        check_target(who, &inst, state)?;
    }
    let from = inst.assigned_to.clone();
    let payload = json!({ "instance_id": inst.id, "app_slug": inst.app_slug, "rule_id": inst.rule_id, "from": from, "to": target });
    admin_write(state, caller, DecisionType::ProcessInstanceReassigned, "AC-03", &reason, &payload, || {
        inst.assigned_to = target.clone();
        match &target {
            Some(who) => {
                inst.no_approver = None;
                inst.log.push(format!("reassigned to {who} by {}: {reason}", caller.eppn));
            }
            None => {
                inst.log.push(format!("returned to role-based approval by {}: {reason}", caller.eppn));
                approvers::refresh_no_approver(state, &mut inst);
            }
        }
        if state.repository.update_waiting_instance(&inst).map_err(storage)? {
            Ok(())
        } else {
            Err(ServiceError::Conflict("The instance was decided while you were reassigning it".to_string()))
        }
    })?;
    Ok(json!({ "id": inst.id, "assigned_to": inst.assigned_to }))
}

/// The person a step is reassigned to must be known, active, off hold, able to read the app, and not the person who made the
/// record or started the process.
fn check_target(who: &str, inst: &ProcessInstance, state: &SharedState) -> Result<(), ServiceError> {
    let bad = |m: String| ServiceError::BadRequest(m);
    if let Some(u) = state.repository.users_by_names(&[who.to_string()]).map_err(storage)?.first() {
        if !u.active {
            return Err(bad(format!("{who} is not active")));
        }
        if u.admin_hold {
            return Err(bad(format!("{who} is on hold")));
        }
    }
    let user = identity::resolve_user(who, state).ok_or_else(|| bad(format!("The platform does not know a person called '{who}'")))?;
    let creator = state.find_record(&inst.app_slug, &inst.record_id).ok().flatten().and_then(|r| r.created_by);
    for (role, other) in [("made the record", creator.as_deref()), ("started the process", inst.started_by.as_deref())] {
        if other.is_some_and(|o| o.eq_ignore_ascii_case(who)) {
            return Err(ServiceError::SeparationOfDuties(format!("{who} {role}, so they cannot decide it")));
        }
    }
    authorize_app(&user, &inst.app_slug, AppAction::Read, state).map_err(|_| bad(format!("{who} cannot read the app '{}'", inst.app_slug)))?;
    Ok(())
}

/// `POST /admin/process-instances/{id}/cancel`: a waiting instance becomes `Failed`. No effect runs and the record is unchanged.
pub fn cancel(caller: &AuthUser, id: &str, body: &Value, state: &SharedState) -> Result<Value, ServiceError> {
    require_platform_admin(caller, state)?;
    let reason = require_reason(body.get("reason").and_then(Value::as_str).unwrap_or(""))?.to_string();
    let mut inst = instance(state, id)?;
    let step = match (&inst.status, inst.waiting_step_id.clone()) {
        (ProcessStatus::Waiting, Some(step)) => step,
        _ => return Err(ServiceError::Conflict("Only a waiting process instance can be cancelled".to_string())),
    };
    inst.status = ProcessStatus::Failed;
    inst.log.push(format!("cancelled by {}: {reason}", caller.eppn));
    let payload = json!({ "instance_id": inst.id, "app_slug": inst.app_slug, "rule_id": inst.rule_id, "step": step });
    admin_write(state, caller, DecisionType::ProcessInstanceCancelled, "AC-03", &reason, &payload, || {
        if state.repository.cancel_waiting_instance(&inst, &step, &caller.eppn).map_err(storage)? {
            Ok(())
        } else {
            Err(ServiceError::Conflict("The instance was decided while you were cancelling it".to_string()))
        }
    })?;
    Ok(json!({ "id": inst.id, "status": "Failed" }))
}
