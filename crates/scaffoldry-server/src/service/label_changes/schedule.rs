//! Data layer phase 6b: a label change that runs on a date (`docs/plans/data-layer.md`, "Changing a label", steps 4 to 6).
//!
//! - **Schedule.** A decided change is approved with `effective_at` instead of applying now. The date must be at least the notice period
//!   away. The minimum is set by category class in the setting `sensitivity.notice_days`. A steward may lengthen the period and may not
//!   shorten it. Nothing is applied, and nobody is told.
//! - **The plan.** When the approvals are complete the server builds the communications plan from the evaluation: who loses access,
//!   the owners of the workspace, the stewards, and the compliance office. The steward reads it and approves it. Only then is each person
//!   on it told, through the inbox and, as each person chose, email. Each notice is recorded.
//! - **Exceptions.** In the notice period a person who will lose access may ask to keep it, with a justification and an expiry. The
//!   steward decides. An approved exception is a grant with that expiry. The phase 3 gate waives the categories the change added for that
//!   person once the change is applied, and only while the grant stands.
//! - **The date.** The job `apply_label_changes` applies each change whose date has come and whose plan was approved, through the phase 6
//!   path, with one `DataLabelChanged` entry.
//! - **Withdrawal.** The steward may withdraw a scheduled change. Its exceptions end, and the people who were told are told.
//!
//! Not built: reminders, the urgent path, acknowledgements and the closing report.

use super::{ask_of, bad, ledger_of, load, losers, plan_of, protective, reason_of, stewards_of, storage, Ask, Kind};
use crate::notifications::{notify, Count, DatasetRef, Flag, Notice, Recipients};
use crate::repository::AppendDecisionParams;
use crate::repository_label_changes::{DecideOutcome, DecideReq, Schedule};
use crate::repository_label_schedule::{ExceptionGrant, ExceptionOutcome, NewException, PlanOutcome, WithdrawOutcome};
use crate::service::{catalog, ServiceError};
use crate::state::{AuthUser, SharedState};
use chrono::{DateTime, Duration, SecondsFormat, Utc};
use scaffoldry_core::ledger::DecisionType;
use scaffoldry_data::changes::Change;
use scaffoldry_engine::sensitivity::Category;
use scaffoldry_engine::HostRouter;
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
use uuid::Uuid;

/// Each person on a plan with the audiences that name them.
type People = Vec<(String, Vec<String>)>;

/// The audiences of a plan, in the order the plan lists them.
const AUDIENCES: [&str; 4] = ["lose_access", "owners", "stewards", "compliance"];
/// The furthest ahead a change may be scheduled, in days.
const MAX_AHEAD_DAYS: i64 = 366;
/// The most people the steward's read lists by name.
const LISTED: usize = 50;
/// The most due changes one run of the job applies.
const BATCH: i64 = 50;
/// The actor of a change the job applies on its date.
const JOB_ACTOR: &str = "system";

// ---------------------------------------------------------------------------------------------------------------------
// The notice period
// ---------------------------------------------------------------------------------------------------------------------

/// The shortest notice, in days, when the institution sets none: by class of category, and by whether the change adds a label (people
/// lose access) or only takes one away (people gain it). Raising gets the full period. Lowering needs less warning, because nobody loses.
const DEFAULTS: [(&str, i64, i64); 2] = [("protected", 30, 14), ("other", 14, 7)];

/// Checks the setting `sensitivity.notice_days`: an object with `protected` and `other`, each holding `raise` and `lower` as whole days
/// from 1 to 365. A part left out takes its default. The shape is published in `sensitivity-settings.schema.json`.
pub fn valid_notice_days(value: &Value) -> bool {
    let Some(map) = value.as_object() else { return false };
    map.iter().all(|(class, v)| {
        DEFAULTS.iter().any(|(c, _, _)| c == class)
            && v.as_object().is_some_and(|m| m.iter().all(|(k, d)| (k == "raise" || k == "lower") && d.as_i64().is_some_and(|n| (1..=365).contains(&n))))
    })
}

/// The categories a change adds to a row or a floor.
fn added(ask: &Ask) -> Vec<String> {
    match ask.kind {
        Kind::Tags => ask.add.clone(),
        Kind::Floor => ask.floor_after.iter().filter(|c| !ask.floor_before.contains(c)).cloned().collect(),
    }
}

/// The categories a change takes away from a row or a floor.
fn removed(ask: &Ask) -> Vec<String> {
    match ask.kind {
        Kind::Tags => ask.remove.clone(),
        Kind::Floor => ask.floor_before.iter().filter(|c| !ask.floor_after.contains(c)).cloned().collect(),
    }
}

/// The shortest notice for this change, in days. The class is the most protective category it touches. A change that adds a label takes
/// the period for raising, even when it also removes one. A change that only removes labels takes the period for lowering.
pub(super) fn min_days(ask: &Ask, cats: &[Category], state: &SharedState) -> i64 {
    let (adds, removes) = (added(ask), removed(ask));
    let class = if adds.iter().chain(removes.iter()).any(|c| protective(c, cats)) { "protected" } else { "other" };
    let way = if adds.is_empty() { "lower" } else { "raise" };
    let set = state.settings.read().unwrap_or_else(|p| p.into_inner()).get("sensitivity.notice_days").cloned().unwrap_or(Value::Null);
    let configured = set.get(class).and_then(|c| c.get(way)).and_then(Value::as_i64).filter(|n| (1..=365).contains(n));
    let default = DEFAULTS.iter().find(|(c, _, _)| *c == class).map(|(_, raise, lower)| if way == "raise" { *raise } else { *lower }).unwrap_or(30);
    configured.unwrap_or(default)
}

/// Whole days from now to `at`, rounded up, and never below zero.
fn days_until(at: DateTime<Utc>, now: DateTime<Utc>) -> i64 {
    ((at - now).num_seconds() + 86_399).div_euclid(86_400).max(0)
}

fn rfc3339(at: DateTime<Utc>) -> String {
    at.to_rfc3339_opts(SecondsFormat::Secs, true)
}

fn parse_time(text: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(text).ok().map(|d| d.with_timezone(&Utc))
}

/// The date a request asks for, checked against the notice period. `None` when the body names no date and none was kept. A first approver
/// of two may have asked for one, which the change keeps (`kept`). The approver who completes the change uses it unless they name another,
/// so an approver who says nothing cannot apply at once a change that was asked for on a date.
pub(super) fn effective_of(body: &Value, kept: Option<&str>, ask: &Ask, cats: &[Category], state: &SharedState, now: DateTime<Utc>) -> Result<Option<DateTime<Utc>>, ServiceError> {
    let given = body.get("effective_at").filter(|v| !v.is_null()).cloned().or_else(|| kept.map(|k| Value::String(k.to_string())));
    let Some(v) = given else { return Ok(None) };
    let at = v.as_str().and_then(parse_time).ok_or_else(|| bad("effective_at is a date and time such as 2026-12-30T09:00:00Z"))?;
    if at <= now {
        return Err(bad("effective_at is in the past"));
    }
    if at > now + Duration::days(MAX_AHEAD_DAYS) {
        return Err(bad(format!("effective_at is more than {MAX_AHEAD_DAYS} days ahead. Choose an earlier date")));
    }
    let min = min_days(ask, cats, state);
    let earliest = now + Duration::days(min);
    if at < earliest {
        return Err(bad(format!(
            "The notice period for this change is at least {min} days, so the earliest date is {}. A steward may lengthen the period and may not shorten it",
            rfc3339(earliest)
        )));
    }
    Ok(Some(at))
}

// ---------------------------------------------------------------------------------------------------------------------
// The plan
// ---------------------------------------------------------------------------------------------------------------------

/// The workspace's owners and admins.
fn owners(ask: &Ask, state: &SharedState) -> BTreeSet<String> {
    let workspace = {
        let engine = state.engine.read().unwrap_or_else(|p| p.into_inner());
        engine.resolve_by_slug(&ask.app).and_then(|m| m.workspace_id.clone())
    };
    let Some(ws) = workspace else { return BTreeSet::new() };
    crate::service::members::people_with_roles(&ws, state).into_iter().filter(|(_, r)| *r >= crate::service::members::Role::Admin).map(|(p, _)| p).collect()
}

/// The people who hold a compliance role today.
fn compliance(state: &SharedState) -> BTreeSet<String> {
    state.repository.all_holders().unwrap_or_default().into_iter().filter(|h| h.affiliation == "compliance" && h.in_force()).map(|h| h.eppn).collect()
}

/// The people of each audience, built from the evaluation. Nobody types them. Each person is once, with every audience that names them.
fn audiences(ask: &Ask, row: &crate::repository_catalog::CatalogRow, cats: &[Category], state: &SharedState) -> Result<BTreeMap<&'static str, BTreeSet<String>>, ServiceError> {
    let mut out: BTreeMap<&'static str, BTreeSet<String>> = BTreeMap::new();
    out.insert("lose_access", losers(ask, cats, state)?);
    out.insert("owners", owners(ask, state));
    out.insert("stewards", stewards_of(row, state).into_iter().map(|s| s.to_lowercase()).collect());
    out.insert("compliance", compliance(state));
    Ok(out)
}

fn audience_name(id: &str) -> &'static str {
    match id {
        "lose_access" => "People who will lose access",
        "owners" => "Owners of the app",
        "stewards" => "Stewards of the dataset",
        _ => "Privacy and compliance",
    }
}

/// What an audience is told, in plain words. It names the dataset and the date, and no record value.
fn told(id: &str, dataset: &str, date: &str) -> String {
    let text = match id {
        "lose_access" => format!("Access to some rows of {dataset} changes on {date}, and you will lose it. You may ask for an exception before then, with a reason and an end date. A steward decides. Reading the dataset shows the notice."),
        "owners" => format!("Who may read some rows of {dataset} changes on {date}. The apps, automations and processes you own that read it may be affected. Check them before then."),
        "stewards" => format!("A change to who may read some rows of {dataset} takes effect on {date}. You decide the exceptions people ask for until then."),
        _ => format!("A change to who may read some rows of {dataset} takes effect on {date}. The ledger records it."),
    };
    text.chars().take(500).collect()
}

/// The plan document (`label-change-plan.schema.json`) and the people it tells, each with the audiences that name them.
fn plan_doc(ask: &Ask, row: &crate::repository_catalog::CatalogRow, at: DateTime<Utc>, now: DateTime<Utc>, min: i64, cats: &[Category], state: &SharedState) -> Result<(Value, People), ServiceError> {
    let people = audiences(ask, row, cats, state)?;
    let dataset = catalog::name_of(state, &row.id).unwrap_or_else(|| "the dataset".to_string());
    let date = at.format("%Y-%m-%d").to_string();
    let days = days_until(at, now).max(1);
    let list: Vec<Value> = AUDIENCES.iter().map(|id| json!({ "id": id, "name": audience_name(id), "count": people[id].len(), "told": told(id, &dataset, &date) })).collect();
    let mut by_person: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for id in AUDIENCES {
        for p in &people[id] {
            by_person.entry(p.clone()).or_default().push(id.to_string());
        }
    }
    let kind = if ask.kind == Kind::Floor { "floor" } else { "tags" };
    let plan = json!({
        "effective_at": rfc3339(at), "notice_days": days, "notice_days_min": min,
        "change": { "kind": kind, "adds": added(ask), "removes": removed(ask) },
        "audiences": list,
        "timeline": [{ "action": "first_notice", "days_before": days }, { "action": "effective", "days_before": 0 }],
        "channels": ["inbox", "email", "api"],
    });
    Ok((plan, by_person.into_iter().collect()))
}

/// Builds what a final approval for a date stores: the plan, its people and the ledger entry. Returns it with the part of the answer the
/// steward reads next.
pub(super) fn hold(caller: &AuthUser, change: &Change, ask: &Ask, row: &crate::repository_catalog::CatalogRow, at: DateTime<Utc>, reason: &str, state: &SharedState) -> Result<(Schedule, Value), ServiceError> {
    let cats = state.sensitivity_categories();
    let (now, min) = (Utc::now(), min_days(ask, &cats, state));
    let (plan, audience) = plan_doc(ask, row, at, now, min, &cats, state)?;
    let counts: Value = plan["audiences"].as_array().map(|a| a.iter().map(|x| (x["id"].as_str().unwrap_or("").to_string(), x["count"].clone())).collect::<serde_json::Map<_, _>>()).map(Value::Object).unwrap_or(Value::Null);
    let ledger = AppendDecisionParams {
        principal: caller.eppn.clone(),
        organization_code: "DIV-ADMIN-CONSOLE".to_string(),
        app_slug: Some(ask.app.clone()),
        decision_type: DecisionType::DataLabelChanged,
        oscal_control_id: "AC-03".to_string(),
        rationale: reason.to_string(),
        payload: json!({ "change": change.id.to_string(), "dataset": ask.dataset_id, "setting": "label_change_schedule", "event": "scheduled", "effective_at": rfc3339(at), "notice_days": plan["notice_days"], "audiences": counts }),
    };
    let described = json!({
        "effective_at": rfc3339(at), "notice_days": plan["notice_days"], "notice_days_min": min, "plan": plan,
        "next": format!("Nothing has changed and nobody is told. Read the plan, then approve it: POST /data/label-changes/{}/plan/approve", change.id),
    });
    Ok((Schedule { at: rfc3339(at), plan, app_slug: ask.app.clone(), table_id: ask.table.clone(), audience, ledger }, described))
}

/// What the steward reads of a scheduled change: the date, the plan, who was told, and the exceptions asked.
pub(super) fn block(change: &Change, state: &SharedState) -> Result<Option<Value>, ServiceError> {
    let Some(plan) = &change.plan else { return Ok(None) };
    let people = state.repository.label_audience(change.id).map_err(storage)?;
    let counts = state.repository.label_audience_counts(change.id).map_err(storage)?;
    let exceptions = state.repository.label_exceptions(change.id).map_err(storage)?;
    Ok(Some(json!({
        "effective_at": change.effective_at, "plan": plan, "plan_approved_by": change.plan_approved_by, "plan_approved_at": change.plan_approved_at,
        "notices": { "audience": counts.audience, "sent": counts.sent, "delivered": counts.delivered },
        "people": people.iter().take(LISTED).map(|(t, sent, delivered)| json!({ "person": t.person, "audiences": t.audiences, "notice_recorded": sent, "delivered": delivered })).collect::<Vec<_>>(),
        "exceptions": exceptions.iter().map(|e| e.json()).collect::<Vec<_>>(),
    })))
}

// ---------------------------------------------------------------------------------------------------------------------
// Approving the plan
// ---------------------------------------------------------------------------------------------------------------------

fn scheduled_of(change: &Change) -> Result<DateTime<Utc>, ServiceError> {
    change.effective_at.as_deref().and_then(parse_time).ok_or_else(|| ServiceError::Conflict("This change has no date".to_string()))
}

fn not_scheduled(status: &str) -> ServiceError {
    ServiceError::Conflict(format!("This change is {}, so there is no plan to decide", status.to_lowercase()))
}

/// Tells each person on the plan who has no recorded notice, and records it. A person the inbox does not take is recorded as not
/// delivered. A failure leaves the person unrecorded, so approving the plan again tells them.
fn send_scheduled(change: &Change, row: &crate::repository_catalog::CatalogRow, at: DateTime<Utc>, state: &SharedState) -> Result<(), ServiceError> {
    let days = days_until(at, Utc::now()).max(1);
    for t in state.repository.label_unnotified(change.id).map_err(storage)? {
        let loses = t.audiences.iter().any(|a| a == "lose_access");
        let notice = Notice::LabelChangeScheduled { dataset: DatasetRef(row.id.clone()), days: Count(u32::try_from(days).unwrap_or(u32::MAX)), loses: Flag(loses) };
        match notify(state, &Recipients::Person(t.person.clone()), None, &notice) {
            Ok(n) => state.repository.label_mark_notified(change.id, &t.person, n > 0).map_err(storage)?,
            Err(e) => eprintln!("Warning: a notice of label change {} could not be recorded for a person on its plan: {e}", change.id),
        }
    }
    Ok(())
}

/// `POST /data/label-changes/{id}/plan/approve`. The steward has read the plan. Each person on it is told, and each notice is recorded.
/// Nothing is sent before this. Approving again sends only what is missing.
pub fn approve_plan(caller: &AuthUser, id: &str, body: &Value, state: &SharedState) -> Result<Value, ServiceError> {
    let (change, row) = load(caller, id, state)?;
    let reason = reason_of(body)?;
    if change.status != "Scheduled" {
        return Err(not_scheduled(&change.status));
    }
    let at = scheduled_of(&change)?;
    let ask = ask_of(&change, &row, state)?;
    let cats = state.sensitivity_categories();
    let (now, min) = (Utc::now(), min_days(&ask, &cats, state));
    if at < now + Duration::days(min) && change.plan_approved_at.is_none() {
        return Err(ServiceError::Conflict(format!(
            "Telling people now would give them less than the minimum notice of {min} days. Withdraw this change and schedule a later date"
        )));
    }
    let ledger = AppendDecisionParams {
        principal: caller.eppn.clone(),
        organization_code: "DIV-ADMIN-CONSOLE".to_string(),
        app_slug: Some(ask.app.clone()),
        decision_type: DecisionType::DataLabelChanged,
        oscal_control_id: "AC-03".to_string(),
        rationale: reason,
        payload: json!({ "change": change.id.to_string(), "dataset": ask.dataset_id, "setting": "label_change_schedule", "event": "plan_approved", "effective_at": rfc3339(at) }),
    };
    match state.repository.approve_label_plan(change.id, &caller.eppn, ledger).map_err(storage)? {
        PlanOutcome::Missing => return Err(ServiceError::NotFound("Label change not found".to_string())),
        PlanOutcome::NotScheduled(s) => return Err(not_scheduled(&s)),
        PlanOutcome::Approved { .. } => {}
    }
    send_scheduled(&change, &row, at, state)?;
    let counts = state.repository.label_audience_counts(change.id).map_err(storage)?;
    Ok(json!({
        "id": change.id, "status": "Scheduled", "effective_at": rfc3339(at),
        "notices": { "audience": counts.audience, "sent": counts.sent, "delivered": counts.delivered },
        "next": "The people on the plan are told. They may ask for an exception until the date. Decide each: POST /data/label-changes/{id}/exceptions/{exception}/decide",
    }))
}

// ---------------------------------------------------------------------------------------------------------------------
// Withdrawing
// ---------------------------------------------------------------------------------------------------------------------

/// `POST /data/label-changes/{id}/withdraw`. The steward withdraws a scheduled change before its date. The change is closed, so the job
/// finds nothing to apply on the date. Its exceptions end and their grants are revoked. The people who were told are told it is withdrawn.
pub fn withdraw(caller: &AuthUser, id: &str, body: &Value, state: &SharedState) -> Result<Value, ServiceError> {
    let (change, row) = load(caller, id, state)?;
    let reason = reason_of(body)?;
    if change.status != "Scheduled" {
        return Err(ServiceError::Conflict(format!("This change is {}, so it cannot be withdrawn", change.status.to_lowercase())));
    }
    let ledger = AppendDecisionParams {
        principal: caller.eppn.clone(),
        organization_code: "DIV-ADMIN-CONSOLE".to_string(),
        app_slug: Some(row.app_slug.clone()),
        decision_type: DecisionType::DataLabelChanged,
        oscal_control_id: "AC-03".to_string(),
        rationale: reason.clone(),
        payload: json!({ "change": change.id.to_string(), "dataset": row.id, "setting": "label_change_schedule", "event": "withdrawn" }),
    };
    let revoked = match state.repository.withdraw_label_change(change.id, &caller.eppn, &reason, ledger).map_err(storage)? {
        WithdrawOutcome::Missing => return Err(ServiceError::NotFound("Label change not found".to_string())),
        WithdrawOutcome::NotScheduled(s) => return Err(ServiceError::Conflict(format!("This change is {}, so it cannot be withdrawn", s.to_lowercase()))),
        WithdrawOutcome::Done { grants_revoked } => grants_revoked,
    };
    let mut told = 0;
    for person in state.repository.label_unwithdrawn(change.id).map_err(storage)? {
        let notice = Notice::LabelChangeWithdrawn { dataset: DatasetRef(row.id.clone()) };
        match notify(state, &Recipients::Person(person.clone()), None, &notice) {
            Ok(_) => {
                state.repository.label_mark_withdrawn_notified(change.id, &person).map_err(storage)?;
                told += 1;
            }
            Err(e) => eprintln!("Warning: the withdrawal of label change {} could not be told to a person on its plan: {e}", change.id),
        }
    }
    Ok(json!({ "id": change.id, "status": "Withdrawn", "told": told, "grants_revoked": revoked }))
}

// ---------------------------------------------------------------------------------------------------------------------
// Exceptions
// ---------------------------------------------------------------------------------------------------------------------

/// `POST /data/label-changes/{id}/exceptions`. A person who will lose access asks to keep it. The body names `justification` (1 to 500
/// characters) and `expires_at`, after the change takes effect and within `access.request_max_days` of it. A person the change does not
/// affect gets the same 404 as a change that does not exist.
pub fn request_exception(caller: &AuthUser, id: &str, body: &Value, state: &SharedState) -> Result<Value, ServiceError> {
    let missing = || ServiceError::NotFound("Label change not found".to_string());
    let id = Uuid::parse_str(id).map_err(|_| missing())?;
    let change = state.repository.label_change(id).map_err(storage)?.ok_or_else(missing)?;
    let (audiences, told) = state.repository.label_audience_of(id, &caller.eppn).map_err(storage)?.ok_or_else(missing)?;
    if !audiences.iter().any(|a| a == "lose_access") {
        return Err(missing());
    }
    if change.status != "Scheduled" {
        return Err(ServiceError::Conflict(format!("This change is {}, so there is nothing to ask an exception to", change.status.to_lowercase())));
    }
    if !told || change.plan_approved_at.is_none() {
        return Err(ServiceError::Conflict("The notice period has not begun: the steward has not approved the plan".to_string()));
    }
    let at = scheduled_of(&change)?;
    if at <= Utc::now() {
        return Err(ServiceError::Conflict("The date has passed, so an exception can no longer be asked".to_string()));
    }
    let justification = body.get("justification").and_then(Value::as_str).map(str::trim).unwrap_or("");
    if justification.is_empty() || justification.chars().count() > 500 {
        return Err(bad("A justification between 1 and 500 characters is required"));
    }
    let expires = body.get("expires_at").and_then(Value::as_str).and_then(parse_time).ok_or_else(|| bad("expires_at is a date and time such as 2027-01-31T09:00:00Z"))?;
    let longest = Duration::days(crate::service::access_requests::max_days(state));
    if expires <= at {
        return Err(bad("An exception that ends before the change takes effect is no exception. Choose a later expires_at"));
    }
    if expires > at + longest {
        return Err(bad(format!("An exception lasts at most {} days after the change takes effect", longest.num_days())));
    }
    let row = state.repository.catalog_row(&change.dataset_id).map_err(storage)?.ok_or_else(missing)?;
    let ask = ask_of(&change, &row, state)?;
    let made = state
        .repository
        .request_label_exception(NewException {
            change_id: change.id,
            app_slug: ask.app.clone(),
            table_id: ask.table.clone(),
            person: caller.eppn.clone(),
            justification: justification.to_string(),
            expires_at: rfc3339(expires),
            adds: added(&ask),
        })
        .map_err(storage)?
        .ok_or_else(|| ServiceError::Conflict("You have an exception for this change already".to_string()))?;
    let stored = state.repository.label_exception(made).map_err(storage)?.ok_or_else(|| ServiceError::Internal("The exception was not stored".to_string()))?;
    Ok(stored.json())
}

/// `GET /data/label-changes/{id}/exceptions`: the exceptions asked for a change, for its steward.
pub fn list_exceptions(caller: &AuthUser, id: &str, state: &SharedState) -> Result<Value, ServiceError> {
    let (change, _) = load(caller, id, state)?;
    let list = state.repository.label_exceptions(change.id).map_err(storage)?;
    Ok(json!({ "exceptions": list.iter().map(|e| e.json()).collect::<Vec<_>>() }))
}

/// Where an exception's grant is anchored. It names the change, which is no workspace, app or table, so the grant reaches no object and
/// adds no role or row anywhere. It carries the holder, the reason and the expiry, and the gate reads it to waive the category.
fn exception_anchor(change: Uuid) -> String {
    format!("label_change:{change}")
}

/// `POST /data/label-changes/{id}/exceptions/{exception}/decide`. The steward approves or declines. An approval records a grant, a
/// viewer grant anchored on the change, which reaches no object, with the exception's expiry. The gate waives what the
/// change added for that person once the change is applied.
pub fn decide_exception(caller: &AuthUser, id: &str, exception: &str, body: &Value, state: &SharedState) -> Result<Value, ServiceError> {
    let (change, row) = load(caller, id, state)?;
    let missing = || ServiceError::NotFound("Exception not found".to_string());
    let xid = Uuid::parse_str(exception).map_err(|_| missing())?;
    let ex = state.repository.label_exception(xid).map_err(storage)?.filter(|e| e.change_id == change.id).ok_or_else(missing)?;
    let reason = reason_of(body)?;
    let approve = match body.get("decision").and_then(Value::as_str) {
        Some("approve") => true,
        Some("decline") => false,
        _ => return Err(bad("decision is approve or decline")),
    };
    if ex.status != "Requested" {
        return Err(ServiceError::Conflict(format!("This exception is {}, so it cannot be decided now", ex.status.to_lowercase())));
    }
    let ask = ask_of(&change, &row, state)?;
    let grant = if approve {
        if parse_time(&ex.expires_at).is_none_or(|e| e <= Utc::now()) {
            return Err(ServiceError::Conflict("This exception has already run out, so it cannot be granted".to_string()));
        }
        // An exception waives the categories the change adds for the whole dataset, not for named rows. For a change to named rows, that
        // would also open the rows that already carry the category. The steward is told and the exception is not granted.
        if ask.kind == Kind::Tags && state.repository.label_others_carry(&ask.app, &ask.table, ex.adds.clone(), ask.records.clone()).map_err(storage)? {
            return Err(ServiceError::Conflict("Other rows of this dataset already carry the category this change adds. An exception would open them to this person. Decline it, or share those rows another way".to_string()));
        }
        // An exception keeps what the person reads today, so it is refused when they read nothing of the app now.
        let reads = crate::service::identity::resolve_user(&ex.person, state)
            .is_some_and(|u| crate::service::access::authorize_app(&u, &ask.app, crate::service::access::AppAction::Read, state).is_ok());
        if !reads {
            return Err(ServiceError::Conflict("This person reads nothing of the app today, so there is no access to keep. An exception does not give access".to_string()));
        }
        let why: String = ex.justification.chars().take(400).collect();
        Some(ExceptionGrant {
            holder: ex.person.clone(),
            anchor: exception_anchor(change.id),
            until: ex.expires_at.clone(),
            reason: format!("Exception to label change {}: {why}", change.id),
            created_by: caller.eppn.clone(),
        })
    } else {
        None
    };
    let (decision_type, control, payload) = if approve {
        (
            DecisionType::AccessGrantChanged,
            "AC-02",
            json!({ "op": "create", "source": "label_change_exception", "change": change.id.to_string(), "exception": xid.to_string(), "holder": ex.person, "role": "viewer", "anchor": exception_anchor(change.id), "valid_until": ex.expires_at }),
        )
    } else {
        (DecisionType::AccessRequestDecided, "AC-03", json!({ "outcome": "Declined", "source": "label_change_exception", "change": change.id.to_string(), "exception": xid.to_string(), "requester": ex.person }))
    };
    let ledger = AppendDecisionParams { principal: caller.eppn.clone(), organization_code: "DIV-ADMIN-CONSOLE".to_string(), app_slug: Some(ask.app.clone()), decision_type, oscal_control_id: control.to_string(), rationale: reason.clone(), payload };
    match state.repository.decide_label_exception(xid, &caller.eppn, &reason, grant, ledger).map_err(storage)? {
        ExceptionOutcome::Missing => Err(missing()),
        ExceptionOutcome::Not(s) => Err(ServiceError::Conflict(format!("This is {}, so the exception cannot be decided now", s.to_lowercase()))),
        ExceptionOutcome::Done { .. } => Ok(state.repository.label_exception(xid).map_err(storage)?.ok_or_else(missing)?.json()),
    }
}

// ---------------------------------------------------------------------------------------------------------------------
// The read
// ---------------------------------------------------------------------------------------------------------------------

/// The `upcoming_changes` block of a read of a table's records, for a reader the change affects: one with a recorded notice for it. It
/// holds the date, what changes in category names, and whether the reader will lose access. It names no other person and no row, and holds
/// no value. `None` when nothing is upcoming, so a reader who is not affected sees no block.
pub fn upcoming(caller: &AuthUser, app_slug: &str, table: &str, state: &SharedState) -> Option<Value> {
    let table_id = crate::service::row_labels::table_id_of(app_slug, table, state);
    let mine = state.repository.label_upcoming(&caller.eppn, crate::service::preview::stored_slug(app_slug), &table_id).ok()?;
    if mine.is_empty() {
        return None;
    }
    let cats = state.sensitivity_categories();
    let name = |id: &str| cats.iter().find(|c| c.id == id).map(|c| c.name.clone()).unwrap_or_else(|| "a label".to_string());
    let now = Utc::now();
    let mut out = Vec::new();
    for (id, audiences) in mine {
        let Some(change) = state.repository.label_change(id).ok().flatten().filter(|c| c.status == "Scheduled" && c.plan_approved_at.is_some()) else { continue };
        let (Some(at), Some(plan)) = (change.effective_at.as_deref().and_then(parse_time), &change.plan) else { continue };
        let ids = |k: &str| -> Vec<String> { plan["change"][k].as_array().map(|a| a.iter().filter_map(|s| s.as_str().map(str::to_string)).collect()).unwrap_or_default() };
        let (adds, removes) = (ids("adds"), ids("removes"));
        let list = |v: &[String]| v.iter().map(|c| name(c)).collect::<Vec<_>>().join(", ");
        let rows = if plan["change"]["kind"] == "floor" { "every row" } else { "some rows" };
        let mut what = Vec::new();
        if !adds.is_empty() {
            what.push(format!("{rows} of this dataset will carry the label {}", list(&adds)));
        }
        if !removes.is_empty() {
            what.push(format!("{rows} of this dataset will no longer carry the label {}", list(&removes)));
        }
        let loses = audiences.iter().any(|a| a == "lose_access");
        let mut sentence = format!("From {}, {}.", at.format("%Y-%m-%d"), what.join(", and "));
        if loses {
            sentence.push_str(" You will lose access to them unless you ask for an exception.");
        }
        let exception = state.repository.label_exception_of(id, &caller.eppn).ok().flatten().map(|e| json!({ "id": e.id, "status": e.status, "expires_at": e.expires_at }));
        let mut entry = json!({
            "id": id, "effective_at": rfc3339(at), "days_left": days_until(at, now), "adds": adds, "removes": removes,
            "what": sentence, "you_lose_access": loses, "exception": exception,
        });
        if loses && exception.is_none() {
            entry["ask_for_an_exception"] = json!(format!("POST /api/v1/data/label-changes/{id}/exceptions"));
        }
        out.push(entry);
    }
    (!out.is_empty()).then(|| Value::Array(out))
}

// ---------------------------------------------------------------------------------------------------------------------
// The date
// ---------------------------------------------------------------------------------------------------------------------

/// Applies one scheduled change through the phase 6 path: the tags or the floor, the history rows, the flags, one `DataLabelChanged`
/// entry and the status commit together. Exceptions come into force in the same transaction. False when it was not scheduled any more.
fn apply_one(state: &SharedState, id: Uuid) -> Result<bool, String> {
    let storage = |e: crate::repository::RepositoryError| e.to_string();
    let Some(change) = state.repository.label_change(id).map_err(storage)? else { return Ok(false) };
    let Some(row) = state.repository.catalog_row(&change.dataset_id).map_err(storage)? else { return Ok(false) };
    let ask = ask_of(&change, &row, state).map_err(|e| format!("{e:?}"))?;
    let note: String = format!("Applied on its date. {}", change.decision_note.clone().unwrap_or_default()).chars().take(500).collect();
    let req = DecideReq {
        id: change.id,
        by: JOB_ACTOR.to_string(),
        note: note.clone(),
        open_work: change.open_work.clone(),
        needs: 0,
        hold: false,
        release: true,
        scheduled: true,
        schedule: None,
        date: None,
        plan: plan_of(&ask, state).map_err(|e| format!("{e:?}"))?,
        ledger: ledger_of(change.id, &ask, JOB_ACTOR, &note),
    };
    Ok(matches!(state.repository.decide_label_change(req).map_err(storage)?, DecideOutcome::Applied { .. }))
}

/// Applies each scheduled change whose plan was approved and whose date is `now` or earlier. Returns how many it applied. The time is an
/// argument, so a test never waits for a date.
pub fn apply_due(state: &SharedState, now: DateTime<Utc>) -> Result<usize, String> {
    let due = state.repository.due_label_changes(&rfc3339(now), BATCH).map_err(|e| e.to_string())?;
    let mut applied = 0;
    for id in due {
        match apply_one(state, id) {
            Ok(true) => applied += 1,
            Ok(false) => {}
            Err(e) => eprintln!("Warning: label change {id} could not be applied on its date: {e}"),
        }
    }
    Ok(applied)
}

/// The `apply_label_changes` job, run every minute.
pub fn apply_job(ctx: &crate::jobs::JobCtx, _payload: Value) -> Result<Value, crate::jobs::JobError> {
    let state = ctx.state().ok_or_else(|| crate::jobs::JobError::Permanent("applying a change on its date needs the server's state, and this runner has none".to_string()))?;
    let applied = apply_due(state, Utc::now()).map_err(crate::jobs::JobError::Retry)?;
    Ok(json!({ "applied": applied }))
}
