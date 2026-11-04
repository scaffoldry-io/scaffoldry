//! Process v2, phase 1: how a process performs, from `process_events` and the instances' status.
//!
//! Per rule and step: how many instances entered it, the median and 90th percentile time spent in it, the share of
//! finished instances that ended Completed, Rejected, Failed or Cancelled, and how many wait there now, by age in
//! buckets of 1, 3, 7, 14 and 30 days. Nothing here reads a record. A person's own queue stays on the desk.

use crate::service::access::{authorize_app, AppAction};
use crate::service::ServiceError;
use crate::state::{AuthUser, SharedState};
use chrono::{DateTime, NaiveDate, Utc};
use scaffoldry_core::StepKind;
use serde_json::{json, Map, Value};
use std::collections::HashMap;

/// The waiting buckets, as `(key, lower bound in days)`.
pub const BUCKETS: [(&str, f64); 6] = [("under_1_day", 0.0), ("1_to_3_days", 1.0), ("3_to_7_days", 3.0), ("7_to_14_days", 7.0), ("14_to_30_days", 14.0), ("over_30_days", 30.0)];
pub const OUTCOMES: [&str; 4] = ["Completed", "Rejected", "Failed", "Cancelled"];

fn storage(e: crate::repository::RepositoryError) -> ServiceError {
    ServiceError::StorageUnavailable(e.to_string())
}

fn day(q: &HashMap<String, String>, key: &str) -> Result<Option<DateTime<Utc>>, ServiceError> {
    let Some(raw) = q.get(key).filter(|s| !s.is_empty()) else { return Ok(None) };
    if let Ok(t) = DateTime::parse_from_rfc3339(raw) {
        return Ok(Some(t.with_timezone(&Utc)));
    }
    NaiveDate::parse_from_str(raw, "%Y-%m-%d")
        .map(|d| Some(d.and_hms_opt(0, 0, 0).unwrap().and_utc()))
        .map_err(|_| ServiceError::bad_request(format!("{key} must be a date, such as 2026-10-01")))
}

/// A duration in words, for the note on the slowest step.
fn in_words(seconds: f64) -> String {
    let (n, unit) = if seconds >= 86_400.0 {
        ((seconds / 86_400.0).round(), "day")
    } else if seconds >= 3_600.0 {
        ((seconds / 3_600.0).round(), "hour")
    } else {
        ((seconds / 60.0).round().max(1.0), "minute")
    };
    format!("{n} {unit}{}", if n == 1.0 { "" } else { "s" })
}

/// `GET /apps/{slug}/process-analytics?rule=&from=&to=`, for the app's owners and admins and Platform Admins.
pub fn analytics(caller: &AuthUser, app_slug: &str, q: &HashMap<String, String>, state: &SharedState) -> Result<Value, ServiceError> {
    authorize_app(caller, app_slug, AppAction::Manage, state)?;
    let (from, to) = (day(q, "from")?, day(q, "to")?);
    let rule = q.get("rule").filter(|s| !s.is_empty()).cloned();
    let (slug, r1, r2, r3) = (app_slug.to_string(), rule.clone(), rule.clone(), rule.clone());
    let (steps, outcomes, waiting) = state
        .repository
        .with_client(move |c| {
            // Time in a step: from the first time an instance entered it to the last time it left it.
            let steps = c.query(
                "WITH s AS ( \
                    SELECT instance_id, rule_id, step_id, min(at) FILTER (WHERE kind = 'step_entered') AS entered, max(at) FILTER (WHERE kind = 'step_left') AS left_at \
                    FROM process_events WHERE app_slug = $1 AND step_id IS NOT NULL AND ($2::text IS NULL OR rule_id = $2) \
                      AND ($3::timestamptz IS NULL OR at >= $3) AND ($4::timestamptz IS NULL OR at < $4) \
                    GROUP BY instance_id, rule_id, step_id) \
                 SELECT rule_id, step_id, count(entered), \
                        percentile_cont(0.5) WITHIN GROUP (ORDER BY EXTRACT(EPOCH FROM left_at - entered)) FILTER (WHERE left_at IS NOT NULL AND entered IS NOT NULL), \
                        percentile_cont(0.9) WITHIN GROUP (ORDER BY EXTRACT(EPOCH FROM left_at - entered)) FILTER (WHERE left_at IS NOT NULL AND entered IS NOT NULL) \
                 FROM s GROUP BY rule_id, step_id ORDER BY rule_id, step_id",
                &[&slug, &r1, &from, &to],
            )?;
            let outcomes = c.query(
                "SELECT rule_id, status, count(*) FROM process_instances WHERE app_slug = $1 AND ($2::text IS NULL OR rule_id = $2) AND status <> 'Waiting' GROUP BY rule_id, status",
                &[&slug, &r2],
            )?;
            // A waiting instance's age is from the last time it entered the step it waits at.
            let waiting = c.query(
                "SELECT i.rule_id, e.step_id, EXTRACT(EPOCH FROM NOW() - max(e.at))::float8 / 86400.0 \
                 FROM process_instances i JOIN process_events e ON e.instance_id = i.id AND e.kind = 'step_entered' \
                 WHERE i.app_slug = $1 AND i.status = 'Waiting' AND ($2::text IS NULL OR i.rule_id = $2) \
                 GROUP BY i.id, i.rule_id, e.step_id",
                &[&slug, &r3],
            )?;
            Ok((steps, outcomes, waiting))
        })
        .map_err(storage)?;

    // Step names from the rule definitions: a decision's prompt, else the step's id.
    let rules = state.automations.read().map(|a| a.get(app_slug).cloned().unwrap_or_default()).unwrap_or_default();
    let name_of = |rule_id: &str, step_id: &str| -> String {
        rules
            .iter()
            .find(|r| r.id == rule_id)
            .and_then(|r| r.steps.iter().find(|s| s.id == step_id))
            .map(|s| match &s.kind {
                StepKind::UserTask { prompt, .. } if !prompt.trim().is_empty() => prompt.clone(),
                _ => s.id.clone(),
            })
            .unwrap_or_else(|| step_id.to_string())
    };

    let mut by_rule: Vec<(String, Vec<Value>)> = Vec::new();
    let mut slowest: Option<(f64, String)> = None;
    for r in &steps {
        let (rule_id, step_id): (String, String) = (r.get(0), r.get(1));
        let (median, p90): (Option<f64>, Option<f64>) = (r.get(3), r.get(4));
        let ages: Vec<f64> = waiting.iter().filter(|w| w.get::<_, String>(0) == rule_id && w.get::<_, Option<String>>(1).as_deref() == Some(step_id.as_str())).map(|w| w.get::<_, f64>(2)).collect();
        let mut buckets = Map::new();
        for (i, (key, low)) in BUCKETS.iter().enumerate() {
            let high = BUCKETS.get(i + 1).map(|b| b.1).unwrap_or(f64::INFINITY);
            buckets.insert(key.to_string(), json!(ages.iter().filter(|a| **a >= *low && **a < high).count()));
        }
        let name = name_of(&rule_id, &step_id);
        if let Some(m) = median {
            if slowest.as_ref().is_none_or(|(s, _)| m > *s) {
                slowest = Some((m, name.clone()));
            }
        }
        let row = json!({ "step_id": step_id, "name": name, "entered": r.get::<_, i64>(2), "median_seconds": median, "p90_seconds": p90, "waiting": ages.len(), "waiting_by_age": buckets });
        match by_rule.iter_mut().find(|(id, _)| *id == rule_id) {
            Some((_, list)) => list.push(row),
            None => by_rule.push((rule_id, vec![row])),
        }
    }
    let rules_out: Vec<Value> = by_rule
        .into_iter()
        .map(|(rule_id, steps)| {
            let finished: i64 = outcomes.iter().filter(|o| o.get::<_, String>(0) == rule_id).map(|o| o.get::<_, i64>(2)).sum();
            let mut shares = Map::new();
            for o in OUTCOMES {
                let n: i64 = outcomes.iter().filter(|x| x.get::<_, String>(0) == rule_id && x.get::<_, String>(1) == o).map(|x| x.get::<_, i64>(2)).sum();
                shares.insert(o.to_string(), json!(if finished > 0 { n as f64 / finished as f64 } else { 0.0 }));
            }
            let name = rules.iter().find(|r| r.id == rule_id).map(|r| r.name.clone()).unwrap_or_else(|| rule_id.clone());
            json!({ "rule_id": rule_id, "name": name, "finished": finished, "outcomes": shares, "steps": steps })
        })
        .collect();
    let note = slowest.map(|(m, name)| format!("{name} takes a median of {}.", in_words(m)));
    Ok(json!({ "rules": rules_out, "slowest": note }))
}
