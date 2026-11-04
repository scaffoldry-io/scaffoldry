//! Process v2 phase 3: the timers of waiting user tasks, in `process_timers`.
//!
//! Entering a step writes its timers in the transaction that starts the instance (`insert_timers_in_tx`). A decision
//! cancels them in its own (`cancel_timers_in_tx`), so a decided step never escalates. The job claims due timers with
//! `FOR UPDATE SKIP LOCKED` and marks them fired in one statement, so two job threads never apply the same timer.

use crate::repository::{process_event_in_tx, status_name, update_record_in_tx, PostgresRepository, RepositoryError, WriteCtx};
use chrono::{DateTime, Utc};
use scaffoldry_core::{ProcessInstance, ProcessStatus};
use serde_json::Value;

/// A timer to write: when it fires and what it does.
#[derive(Debug, Clone)]
pub struct NewTimer {
    pub step_id: String,
    pub action: Value,
    pub fire_at: DateTime<Utc>,
}

/// A timer the job has claimed.
#[derive(Debug, Clone)]
pub struct Timer {
    pub id: uuid::Uuid,
    pub instance_id: String,
    pub step_id: String,
    pub action: Value,
    pub fire_at: DateTime<Utc>,
}

/// What applying a fired timer writes. It is written only while the instance still waits at the timer's step.
pub struct TimerOutcome {
    /// The instance as it is after the timer.
    pub next: ProcessInstance,
    /// The record the timer changes, for `Fail` with `on_fail` actions.
    pub record: Option<(String, String, Value, WriteCtx)>,
    /// The next timers of the step.
    pub timers: Vec<NewTimer>,
    /// The event's detail: what fired.
    pub detail: Value,
    /// Workflows phase 4: an escalation moves the step's task grants to these.
    pub grants: Option<Vec<crate::repository_task_grants::NewGrant>>,
}

/// Writes `timers` for `instance_id`.
pub(crate) fn insert_timers_in_tx(tx: &mut postgres::Transaction<'_>, instance_id: &str, timers: &[NewTimer]) -> Result<(), RepositoryError> {
    for t in timers {
        tx.execute(
            "INSERT INTO process_timers (id, instance_id, step_id, action, fire_at) VALUES ($1, $2, $3, $4, $5)",
            &[&uuid::Uuid::new_v4(), &instance_id, &t.step_id, &t.action, &t.fire_at],
        )?;
    }
    Ok(())
}

/// Cancels the instance's pending timers. A decision, a cancel and a reassign call this in their own transaction.
pub fn cancel_timers_in_tx(tx: &mut postgres::Transaction<'_>, instance_id: &str) -> Result<u64, RepositoryError> {
    Ok(tx.execute("UPDATE process_timers SET state = 'canceled' WHERE instance_id = $1 AND state = 'pending'", &[&instance_id])?)
}

impl PostgresRepository {
    /// Claims up to `limit` timers due at `now` and marks them fired, in one transaction. A timer another thread holds
    /// is skipped, and one it has marked is no longer pending, so each timer is claimed once.
    pub fn claim_due_timers(&self, now: DateTime<Utc>, limit: i64) -> Result<Vec<Timer>, RepositoryError> {
        self.with_client(move |c| {
            let rows = c.query(
                "UPDATE process_timers SET state = 'fired', fired_at = $1 WHERE id IN ( \
                   SELECT id FROM process_timers WHERE state = 'pending' AND fire_at <= $1 ORDER BY fire_at LIMIT $2 FOR UPDATE SKIP LOCKED) \
                 RETURNING id, instance_id, step_id, action, fire_at",
                &[&now, &limit],
            )?;
            Ok(rows.iter().map(|r| Timer { id: r.get(0), instance_id: r.get(1), step_id: r.get(2), action: r.get(3), fire_at: r.get(4) }).collect())
        })
    }

    /// Writes what a fired timer did, if the instance still waits at `step`: the instance, the record, the step's next
    /// timers and a `timer_fired` event. An instance that ended writes its `completed` event and cancels what is left.
    /// Returns false, writing nothing, when the instance has moved on since the timer was claimed.
    pub fn apply_timer(&self, step: &str, outcome: TimerOutcome) -> Result<bool, RepositoryError> {
        let step = step.to_string();
        self.with_client(move |c| {
            let mut tx = c.transaction()?;
            let Some(row) = tx.query_opt("SELECT instance_json FROM process_instances WHERE id = $1 FOR UPDATE", &[&outcome.next.id])? else {
                return Ok(false);
            };
            let stored: ProcessInstance = serde_json::from_value(row.get(0))?;
            if stored.status != ProcessStatus::Waiting || stored.waiting_step_id.as_deref() != Some(step.as_str()) {
                return Ok(false);
            }
            if let Some((slug, id, data, ctx)) = &outcome.record {
                update_record_in_tx(&mut tx, slug, id, data, None, ctx)?;
            }
            let next = &outcome.next;
            tx.execute(
                "UPDATE process_instances SET status = $2, instance_json = $3, updated_at = NOW() WHERE id = $1",
                &[&next.id, &status_name(&next.status), &serde_json::to_value(next)?],
            )?;
            process_event_in_tx(&mut tx, next, Some(&step), "timer_fired", None, &outcome.detail)?;
            if next.status == ProcessStatus::Waiting {
                insert_timers_in_tx(&mut tx, &next.id, &outcome.timers)?;
                if let Some(grants) = &outcome.grants {
                    crate::repository_task_grants::replace_task_grants_in_tx(&mut tx, &next.id, &step, grants)?;
                }
            } else {
                cancel_timers_in_tx(&mut tx, &next.id)?;
                crate::repository_task_grants::close_task_grants_in_tx(&mut tx, &next.id)?;
                process_event_in_tx(&mut tx, next, Some(&step), "step_left", None, &serde_json::json!({}))?;
                process_event_in_tx(&mut tx, next, None, "completed", None, &serde_json::json!({ "status": status_name(&next.status) }))?;
            }
            tx.commit()?;
            Ok(true)
        })
    }

    /// The instance's next pending timer: when it fires and what it does.
    pub fn next_timer(&self, instance_id: &str) -> Result<Option<(DateTime<Utc>, Value)>, RepositoryError> {
        let id = instance_id.to_string();
        self.with_client(move |c| {
            Ok(c.query_opt("SELECT fire_at, action FROM process_timers WHERE instance_id = $1 AND state = 'pending' ORDER BY fire_at LIMIT 1", &[&id])?
                .map(|r| (r.get(0), r.get(1))))
        })
    }
}
