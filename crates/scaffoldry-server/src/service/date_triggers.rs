//! Process v2 phase 2: date triggers. The hourly `scan_date_triggers` job starts each rule with a `DateField` trigger for
//! every record whose date, moved by the rule's offset, fell in the window since the last scan. It reads the index on
//! the date field and never the table. Each start records the key of rule, record and date first, so a record is
//! started once for each date, even when the job runs twice over a window or is resumed.

use crate::state::SharedState;
use chrono::{DateTime, Duration, NaiveTime, Utc};
use scaffoldry_core::standards::eduperson::{EduPersonAffiliation, EduPersonIdentity};
use scaffoldry_core::{AutomationRule, TriggerEvent};
use scaffoldry_engine::HostRouter;
use serde_json::Value;

/// The most records one rule starts in one scan. The next scan starts the rest: its window begins where this one ended.
pub const MAX_PER_SCAN: i64 = 1_000;

/// When a rule's trigger fires for a date: the date at midnight UTC, moved by `offset_days`, at `at` (`HH:MM`) when set.
fn shift(offset_days: i32, at: &Option<String>) -> Duration {
    let at = at.as_deref().and_then(|t| NaiveTime::parse_from_str(t, "%H:%M").ok()).map(|t| t.signed_duration_since(NaiveTime::MIN)).unwrap_or_else(Duration::zero);
    Duration::days(offset_days as i64) + at
}

/// The table that holds a rule's date field.
fn table_of(state: &SharedState, app_slug: &str, field: &str) -> Option<String> {
    let eng = state.engine.read().ok()?;
    eng.resolve_by_slug(app_slug)?.tables.iter().find(|t| t.fields.iter().any(|f| f.name == field)).map(|t| t.id.clone())
}

/// Runs one rule for one stored record, as the record's creator, and no other rule.
pub(crate) fn start_dated(state: &SharedState, rule: &AutomationRule, others: &[String], record_id: &str) -> Result<bool, String> {
    let Some(rec) = state.find_record(&rule.app_slug, record_id)? else { return Ok(false) };
    let by = rec.created_by.clone().unwrap_or_default();
    let person = crate::service::identity::resolve_user(&by, state);
    let affiliation = match person.as_ref().map(|p| p.affiliation.to_ascii_lowercase()).as_deref() {
        Some("faculty") => EduPersonAffiliation::Faculty,
        Some("student") => EduPersonAffiliation::Student,
        Some("staff") => EduPersonAffiliation::Staff,
        Some("employee") => EduPersonAffiliation::Employee,
        _ => EduPersonAffiliation::Member,
    };
    let identity = EduPersonIdentity { eppn: by.clone(), realm: by.split('@').nth(1).unwrap_or("university.edu").to_string(), affiliations: vec![affiliation] };
    let department = person.map(|p| p.department).unwrap_or_default();
    let mut data = rec.data.clone();
    if let Some(map) = data.as_object_mut() {
        map.insert("id".to_string(), Value::String(rec.id.clone()));
    }
    // The other rules of the app count as applied, so this start runs this rule alone.
    let mut applied = others.to_vec();
    crate::service::records::run_automations(state, &rule.app_slug, rule.trigger.clone(), &data, &crate::service::records::Actor { identity: &identity, department: &department }, 0, &mut applied);
    Ok(applied.contains(&rule.id))
}

/// Starts every date-triggered rule for the records whose date, moved by the offset, falls in `(since, until]`. Returns
/// how many records it started.
pub fn scan(state: &SharedState, since: DateTime<Utc>, until: DateTime<Utc>) -> Result<usize, String> {
    let all: Vec<(String, Vec<AutomationRule>)> = state.automations.read().map_err(|e| e.to_string())?.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
    let mut started = 0;
    for (slug, rules) in all {
        let ids: Vec<String> = rules.iter().map(|r| r.id.clone()).collect();
        for rule in rules.iter().filter(|r| r.enabled && !crate::service::flow_parties::is_disabled(state, &r.id)) {
            let TriggerEvent::DateField { field, offset_days, at } = &rule.trigger else { continue };
            let Some(table) = table_of(state, &slug, field) else { continue };
            let by = shift(*offset_days, at);
            let due = state.repository.records_due(&slug, &table, field, since - by, until - by, MAX_PER_SCAN).map_err(|e| e.to_string())?;
            let others: Vec<String> = ids.iter().filter(|id| **id != rule.id).cloned().collect();
            for (record_id, date) in due {
                let key = format!("date:{}:{record_id}:{date}", rule.id);
                if !state.repository.claim_once(&key, &format!("{}:{record_id}", rule.id)).map_err(|e| e.to_string())? {
                    continue;
                }
                if start_dated(state, rule, &others, &record_id)? {
                    started += 1;
                }
            }
        }
    }
    Ok(started)
}

/// The `scan_date_triggers` job, run hourly. Its window starts where the last finished scan ended, at most seven days
/// back, and ends now. The keys make an overlap harmless.
pub fn scan_job(ctx: &crate::jobs::JobCtx, _payload: Value) -> Result<Value, crate::jobs::JobError> {
    let state = ctx.state().ok_or_else(|| crate::jobs::JobError::Permanent("the date scan needs the server's state, and this runner has none".to_string()))?;
    let until = Utc::now();
    let last: Option<DateTime<Utc>> = state
        .repository
        .with_client(|c| {
            Ok(c.query_one("SELECT MAX((result->>'until')::timestamptz) FROM jobs WHERE kind = 'scan_date_triggers' AND state = 'done'", &[])?
                .get::<_, Option<DateTime<Utc>>>(0))
        })
        .map_err(|e: crate::repository::RepositoryError| crate::jobs::JobError::Retry(e.to_string()))?;
    let since = last.unwrap_or(until - Duration::hours(1)).max(until - Duration::days(7));
    let started = scan(state, since, until).map_err(crate::jobs::JobError::Retry)?;
    Ok(serde_json::json!({ "since": since.to_rfc3339(), "until": until.to_rfc3339(), "started": started }))
}
