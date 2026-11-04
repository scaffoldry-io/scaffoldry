//! Workflows phase 5: My Work, in `work_items`.
//!
//! Items open and close in the transaction of the change that causes them: entering a step, deciding it, a timer, a
//! restart, a proposal. A title and a summary hold names only. They are made by `WorkText`, whose fields nobody outside
//! this module can set, so no producer can put a record value in them.

use crate::repository::{PostgresRepository, RepositoryError};
use chrono::{DateTime, Utc};
use serde::Serialize;

/// What an item asks of its person.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkKind {
    Decision,
    Task,
    Review,
    Waiting,
}

impl WorkKind {
    pub fn as_str(self) -> &'static str {
        match self {
            WorkKind::Decision => "decision",
            WorkKind::Task => "task",
            WorkKind::Review => "review",
            WorkKind::Waiting => "waiting",
        }
    }
}

/// An item's title and summary, made from names only.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkText {
    title: String,
    summary: String,
}

fn cut(s: String, n: usize) -> String {
    s.chars().take(n).collect()
}

impl WorkText {
    /// The words for an item about a step of a process. `process` and `app` are the rule's and the app's names.
    pub fn for_step(kind: WorkKind, process: &str, app: &str) -> WorkText {
        let (title, summary) = match kind {
            WorkKind::Decision => (format!("Decide: {process}"), format!("A step of {process} in {app} waits for your decision.")),
            WorkKind::Task => (format!("Task: {process}"), format!("A step of {process} in {app} asks you to complete it.")),
            WorkKind::Review => (format!("Review: {process}"), format!("A step of {process} in {app} asks for your review.")),
            WorkKind::Waiting => (format!("Waiting: {process}"), format!("Your request in {app} waits at a step of {process}.")),
        };
        WorkText { title: cut(title, 200), summary: cut(summary, 300) }
    }

    /// The words for an item about a proposed change to an app. `app` is the app's name.
    pub fn for_proposal(kind: WorkKind, app: &str) -> WorkText {
        let (title, summary) = match kind {
            WorkKind::Waiting => (format!("Waiting: your change to {app}"), format!("Your proposed change to {app} waits for approval.")),
            _ => (format!("Review: a change to {app}"), format!("A proposed change to {app} needs your approval.")),
        };
        WorkText { title: cut(title, 200), summary: cut(summary, 300) }
    }

    pub fn title(&self) -> &str {
        &self.title
    }

    pub fn summary(&self) -> &str {
        &self.summary
    }
}

/// An item to open.
#[derive(Debug, Clone)]
pub struct NewWork {
    pub person: String,
    pub kind: WorkKind,
    pub app_slug: Option<String>,
    pub workspace_id: Option<String>,
    pub flow_id: Option<String>,
    pub instance_id: Option<String>,
    pub step_id: Option<String>,
    pub record_id: Option<String>,
    pub proposal_id: Option<uuid::Uuid>,
    pub text: WorkText,
    /// `holder`, `delegate`, `assigned` or `escalated`.
    pub via: &'static str,
    pub acting_for: Option<String>,
    pub due_at: Option<DateTime<Utc>>,
}

/// An item as stored.
#[derive(Debug, Clone, Serialize)]
pub struct WorkItem {
    pub id: uuid::Uuid,
    pub person: String,
    pub kind: String,
    pub state: String,
    pub app_slug: Option<String>,
    pub workspace_id: Option<String>,
    pub flow_id: Option<String>,
    pub instance_id: Option<String>,
    pub step_id: Option<String>,
    pub record_id: Option<String>,
    pub proposal_id: Option<uuid::Uuid>,
    pub title: String,
    pub summary: String,
    pub via: String,
    pub acting_for: Option<String>,
    pub due_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub closed_at: Option<DateTime<Utc>>,
    pub closed_reason: Option<String>,
}

const ITEM_COLUMNS: &str = "id, person, kind, state, app_slug, workspace_id, flow_id, instance_id, step_id, record_id, proposal_id, title, summary, via, acting_for, due_at, created_at, closed_at, closed_reason";

fn item_of(r: &postgres::Row) -> WorkItem {
    WorkItem {
        id: r.get(0),
        person: r.get(1),
        kind: r.get(2),
        state: r.get(3),
        app_slug: r.get(4),
        workspace_id: r.get(5),
        flow_id: r.get(6),
        instance_id: r.get(7),
        step_id: r.get(8),
        record_id: r.get(9),
        proposal_id: r.get(10),
        title: r.get(11),
        summary: r.get(12),
        via: r.get(13),
        acting_for: r.get(14),
        due_at: r.get(15),
        created_at: r.get(16),
        closed_at: r.get(17),
        closed_reason: r.get(18),
    }
}

/// Opens `items`. A person is stored in lower case, so one person has one list.
pub(crate) fn open_work_in_tx(tx: &mut postgres::Transaction<'_>, items: &[NewWork]) -> Result<(), RepositoryError> {
    for w in items {
        tx.execute(
            "INSERT INTO work_items (id, person, kind, app_slug, workspace_id, flow_id, instance_id, step_id, record_id, proposal_id, title, summary, via, acting_for, due_at) \
             VALUES ($1, lower($2), $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, lower($14), $15)",
            &[
                &uuid::Uuid::new_v4(),
                &w.person,
                &w.kind.as_str(),
                &w.app_slug,
                &w.workspace_id,
                &w.flow_id,
                &w.instance_id,
                &w.step_id,
                &w.record_id,
                &w.proposal_id,
                &w.text.title,
                &w.text.summary,
                &w.via,
                &w.acting_for,
                &w.due_at,
            ],
        )?;
    }
    Ok(())
}

/// Closes every open item of an instance with `reason`.
pub fn close_work_in_tx(tx: &mut postgres::Transaction<'_>, instance_id: &str, reason: &str) -> Result<u64, RepositoryError> {
    Ok(tx.execute("UPDATE work_items SET state = 'closed', closed_at = NOW(), closed_reason = $2 WHERE instance_id = $1 AND state = 'open'", &[&instance_id, &reason])?)
}

/// Closes the items of the people who act on an instance, and keeps the `waiting` ones: an escalation or a reassignment
/// moves who acts and not who waits.
pub fn close_actor_work_in_tx(tx: &mut postgres::Transaction<'_>, instance_id: &str, reason: &str) -> Result<u64, RepositoryError> {
    Ok(tx.execute(
        "UPDATE work_items SET state = 'closed', closed_at = NOW(), closed_reason = $2 WHERE instance_id = $1 AND state = 'open' AND kind <> 'waiting'",
        &[&instance_id, &reason],
    )?)
}

/// A page of one person's open items, due first then oldest. `$2..$4` are the cursor (due, created, id), all null for
/// the first page. Items with no due date come after the dated ones.
pub const PAGE_SQL: &str = "SELECT id, person, kind, state, app_slug, workspace_id, flow_id, instance_id, step_id, record_id, proposal_id, title, summary, via, acting_for, due_at, created_at, closed_at, closed_reason \
     FROM work_items WHERE person = lower($1) AND state = 'open' \
       AND ($4::uuid IS NULL \
            OR ($2::timestamptz IS NOT NULL AND ((due_at, created_at, id) > ($2, $3, $4) OR due_at IS NULL)) \
            OR ($2::timestamptz IS NULL AND due_at IS NULL AND (created_at, id) > ($3, $4))) \
     ORDER BY due_at ASC NULLS LAST, created_at, id LIMIT $5";

/// What a list may be narrowed to.
#[derive(Debug, Clone, Default)]
pub struct WorkFilter {
    pub kind: Option<String>,
    pub app: Option<String>,
    pub workspace: Option<String>,
    pub overdue: bool,
    pub acting_for: Option<String>,
}

impl WorkFilter {
    fn keeps(&self, w: &WorkItem, now: DateTime<Utc>) -> bool {
        self.kind.as_deref().is_none_or(|k| w.kind == k)
            && self.app.as_deref().is_none_or(|a| w.app_slug.as_deref() == Some(a))
            && self.workspace.as_deref().is_none_or(|s| w.workspace_id.as_deref() == Some(s))
            && (!self.overdue || w.due_at.is_some_and(|d| d < now))
            && self.acting_for.as_deref().is_none_or(|p| w.acting_for.as_deref().is_some_and(|a| a.eq_ignore_ascii_case(p)))
    }
}

fn cursor_of(w: &WorkItem) -> String {
    format!("{}|{}|{}", w.due_at.map(|d| d.to_rfc3339()).unwrap_or_default(), w.created_at.to_rfc3339(), w.id)
}

type Cursor = (Option<DateTime<Utc>>, Option<DateTime<Utc>>, Option<uuid::Uuid>);

/// Open items by kind, how many are overdue, and when the oldest was opened.
pub type WorkSummary = (Vec<(String, i64)>, i64, Option<DateTime<Utc>>);

fn parse_cursor(c: Option<&str>) -> Cursor {
    let Some(c) = c else { return (None, None, None) };
    let mut parts = c.splitn(3, '|');
    let due = parts.next().and_then(|d| DateTime::parse_from_rfc3339(d).ok()).map(|d| d.with_timezone(&Utc));
    let created = parts.next().and_then(|d| DateTime::parse_from_rfc3339(d).ok()).map(|d| d.with_timezone(&Utc));
    let id = parts.next().and_then(|i| i.parse().ok());
    (due, created, id)
}

impl PostgresRepository {
    /// One page of a person's open items and the cursor of the next page. The filter applies within the page.
    pub fn work_page(&self, person: &str, filter: &WorkFilter, cursor: Option<&str>, limit: i64) -> Result<(Vec<WorkItem>, Option<String>), RepositoryError> {
        let (person, filter, limit) = (person.to_string(), filter.clone(), limit.clamp(1, 500));
        let (due, created, id) = parse_cursor(cursor);
        self.with_client(move |c| {
            let rows = c.query(PAGE_SQL, &[&person, &due, &created, &id, &(limit + 1)])?;
            let mut items: Vec<WorkItem> = rows.iter().map(item_of).collect();
            let next = if items.len() as i64 > limit {
                items.truncate(limit as usize);
                items.last().map(cursor_of)
            } else {
                None
            };
            let now = Utc::now();
            Ok((items.into_iter().filter(|w| filter.keeps(w, now)).collect(), next))
        })
    }

    /// One item.
    pub fn work_item(&self, id: uuid::Uuid) -> Result<Option<WorkItem>, RepositoryError> {
        self.with_client(move |c| Ok(c.query_opt(&format!("SELECT {ITEM_COLUMNS} FROM work_items WHERE id = $1"), &[&id])?.as_ref().map(item_of)))
    }

    /// Counts of a person's open items by kind, how many are overdue, and when the oldest was opened.
    pub fn work_summary(&self, person: &str) -> Result<WorkSummary, RepositoryError> {
        let person = person.to_string();
        self.with_client(move |c| {
            let kinds = c
                .query("SELECT kind, count(*) FROM work_items WHERE person = lower($1) AND state = 'open' GROUP BY kind ORDER BY kind", &[&person])?
                .iter()
                .map(|r| (r.get(0), r.get(1)))
                .collect();
            let row = c.query_one("SELECT count(*) FILTER (WHERE due_at < NOW()), min(created_at) FROM work_items WHERE person = lower($1) AND state = 'open'", &[&person])?;
            Ok((kinds, row.get(0), row.get(1)))
        })
    }

    /// Every open item about a process instance, for the repair jobs.
    pub fn open_instance_work(&self) -> Result<Vec<WorkItem>, RepositoryError> {
        self.with_client(|c| Ok(c.query(&format!("SELECT {ITEM_COLUMNS} FROM work_items WHERE state = 'open' AND instance_id IS NOT NULL"), &[])?.iter().map(item_of).collect()))
    }

    /// The repairs a job found: items to open and items to close, in one transaction.
    pub fn repair_work(&self, open: Vec<NewWork>, close: Vec<uuid::Uuid>, reason: &str) -> Result<usize, RepositoryError> {
        let reason = reason.to_string();
        self.with_client(move |c| {
            let mut tx = c.transaction()?;
            open_work_in_tx(&mut tx, &open)?;
            let mut closed = 0;
            for id in &close {
                closed += tx.execute("UPDATE work_items SET state = 'closed', closed_at = NOW(), closed_reason = $2 WHERE id = $1 AND state = 'open'", &[id, &reason])? as usize;
            }
            tx.commit()?;
            Ok(open.len() + closed)
        })
    }
}
