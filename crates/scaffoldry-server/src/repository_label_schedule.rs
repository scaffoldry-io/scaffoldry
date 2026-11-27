//! Data layer phase 6b: the people a scheduled label change tells, the exceptions to it, and the steps that follow its plan.
//!
//! The change itself is in `scaffoldry_data::changes`, which holds every statement on `label_changes`. The tables here name no row and no
//! value: the audience is people and reasons, and an exception is a person, a justification, an expiry and a grant.

use crate::repository::{append_ledger_in_tx, AppendDecisionParams, PostgresRepository, RepositoryError};
use scaffoldry_data::changes;
use serde_json::{json, Value};
use uuid::Uuid;

/// One person on a plan, and what is recorded of telling them.
#[derive(Debug, Clone)]
pub struct Told {
    pub person: String,
    pub audiences: Vec<String>,
}

/// How many people a plan names, how many have a recorded notice, and how many of those the inbox took.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Counts {
    pub audience: i64,
    pub sent: i64,
    pub delivered: i64,
}

/// A request to keep access after the date.
#[derive(Debug, Clone)]
pub struct Exception {
    pub id: Uuid,
    pub change_id: Uuid,
    pub app_slug: String,
    pub table_id: String,
    pub person: String,
    pub justification: String,
    pub expires_at: String,
    pub adds: Vec<String>,
    pub status: String,
    pub requested_at: String,
    pub decided_by: Option<String>,
    pub decision_note: Option<String>,
    pub grant_id: Option<i64>,
}

impl Exception {
    pub fn json(&self) -> Value {
        json!({
            "id": self.id, "change_id": self.change_id, "person": self.person, "justification": self.justification, "expires_at": self.expires_at,
            "status": self.status, "requested_at": self.requested_at, "decided_by": self.decided_by, "decision_note": self.decision_note, "grant_id": self.grant_id,
        })
    }
}

const EXCEPTION_COLUMNS: &str = "id, change_id, app_slug, table_id, person, justification, \
    to_char(expires_at AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS\"Z\"'), adds, status, \
    to_char(requested_at AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS\"Z\"'), decided_by, decision_note, grant_id";

fn exception_of(r: &postgres::Row) -> Exception {
    Exception {
        id: r.get(0),
        change_id: r.get(1),
        app_slug: r.get(2),
        table_id: r.get(3),
        person: r.get(4),
        justification: r.get(5),
        expires_at: r.get(6),
        adds: r.get(7),
        status: r.get(8),
        requested_at: r.get(9),
        decided_by: r.get(10),
        decision_note: r.get(11),
        grant_id: r.get(12),
    }
}

/// What a request for an exception holds.
pub struct NewException {
    pub change_id: Uuid,
    pub app_slug: String,
    pub table_id: String,
    pub person: String,
    pub justification: String,
    /// RFC 3339.
    pub expires_at: String,
    pub adds: Vec<String>,
}

/// The grant an approved exception makes: a viewer grant on the app, with the exception's expiry.
pub struct ExceptionGrant {
    pub holder: String,
    pub anchor: String,
    pub until: String,
    pub reason: String,
    pub created_by: String,
}

pub enum ExceptionOutcome {
    Missing,
    /// The exception, or its change, is not in the state a decision needs. Holds the status it has.
    Not(String),
    Done { grant_id: Option<i64> },
}

pub enum PlanOutcome {
    Missing,
    NotScheduled(String),
    /// The plan was approved by this call (`true`) or earlier (`false`).
    Approved { first: bool },
}

pub enum WithdrawOutcome {
    Missing,
    NotScheduled(String),
    Done { grants_revoked: usize },
}

impl PostgresRepository {
    /// The people a plan names, and what is recorded of each.
    pub fn label_audience(&self, change: Uuid) -> Result<Vec<(Told, bool, bool)>, RepositoryError> {
        self.with_client(move |c| {
            Ok(c.query("SELECT person, audiences, notified_at IS NOT NULL, delivered FROM label_change_audience WHERE change_id = $1 ORDER BY person", &[&change])?
                .iter()
                .map(|r| (Told { person: r.get(0), audiences: r.get(1) }, r.get(2), r.get(3)))
                .collect())
        })
    }

    pub fn label_audience_counts(&self, change: Uuid) -> Result<Counts, RepositoryError> {
        self.with_client(move |c| {
            let r = c.query_one(
                "SELECT count(*), count(notified_at), count(*) FILTER (WHERE delivered) FROM label_change_audience WHERE change_id = $1",
                &[&change],
            )?;
            Ok(Counts { audience: r.get(0), sent: r.get(1), delivered: r.get(2) })
        })
    }

    /// The people on the plan who have no recorded notice yet.
    pub fn label_unnotified(&self, change: Uuid) -> Result<Vec<Told>, RepositoryError> {
        self.with_client(move |c| {
            Ok(c.query("SELECT person, audiences FROM label_change_audience WHERE change_id = $1 AND notified_at IS NULL ORDER BY person", &[&change])?
                .iter()
                .map(|r| Told { person: r.get(0), audiences: r.get(1) })
                .collect())
        })
    }

    /// Records the notice for one person.
    pub fn label_mark_notified(&self, change: Uuid, person: &str, delivered: bool) -> Result<(), RepositoryError> {
        let person = person.to_string();
        self.with_client(move |c| {
            c.execute("UPDATE label_change_audience SET notified_at = NOW(), delivered = $3 WHERE change_id = $1 AND person = lower($2) AND notified_at IS NULL", &[&change, &person, &delivered])?;
            Ok(())
        })
    }

    /// The people who were told of a change and not yet told it was withdrawn.
    pub fn label_unwithdrawn(&self, change: Uuid) -> Result<Vec<String>, RepositoryError> {
        self.with_client(move |c| {
            Ok(c.query("SELECT person FROM label_change_audience WHERE change_id = $1 AND notified_at IS NOT NULL AND withdrawn_notified_at IS NULL ORDER BY person", &[&change])?
                .iter()
                .map(|r| r.get(0))
                .collect())
        })
    }

    pub fn label_mark_withdrawn_notified(&self, change: Uuid, person: &str) -> Result<(), RepositoryError> {
        let person = person.to_string();
        self.with_client(move |c| {
            c.execute("UPDATE label_change_audience SET withdrawn_notified_at = NOW() WHERE change_id = $1 AND person = lower($2) AND withdrawn_notified_at IS NULL", &[&change, &person])?;
            Ok(())
        })
    }

    /// The audiences a person is in for a change, when the change has a recorded notice for them or a plan that is waiting.
    pub fn label_audience_of(&self, change: Uuid, person: &str) -> Result<Option<(Vec<String>, bool)>, RepositoryError> {
        let person = person.to_string();
        self.with_client(move |c| {
            Ok(c.query_opt("SELECT audiences, notified_at IS NOT NULL FROM label_change_audience WHERE change_id = $1 AND person = lower($2)", &[&change, &person])?.map(|r| (r.get(0), r.get(1))))
        })
    }

    /// The changes with a recorded notice for this person on this table, newest first. A read of the table's records shows them.
    pub fn label_upcoming(&self, person: &str, app: &str, table: &str) -> Result<Vec<(Uuid, Vec<String>)>, RepositoryError> {
        let (person, app, table) = (person.to_string(), app.to_string(), table.to_string());
        self.with_client(move |c| {
            Ok(c.query(
                "SELECT change_id, audiences FROM label_change_audience WHERE person = lower($1) AND app_slug = $2 AND table_id = $3 AND notified_at IS NOT NULL AND withdrawn_notified_at IS NULL ORDER BY notified_at DESC LIMIT 5",
                &[&person, &app, &table],
            )?
            .iter()
            .map(|r| (r.get(0), r.get(1)))
            .collect())
        })
    }

    /// Approves the plan once. The plan, the approval and the ledger entry commit together. Nothing is sent here.
    pub fn approve_label_plan(&self, id: Uuid, by: &str, ledger: AppendDecisionParams) -> Result<PlanOutcome, RepositoryError> {
        let by = by.to_string();
        self.with_client(move |c| {
            let mut tx = c.transaction()?;
            let Some(change) = changes::get_for_update(&mut tx, id)? else { return Ok(PlanOutcome::Missing) };
            if change.status != "Scheduled" {
                return Ok(PlanOutcome::NotScheduled(change.status));
            }
            let first = changes::approve_plan(&mut tx, id, &by)?;
            if first {
                append_ledger_in_tx(&mut tx, ledger)?;
            }
            tx.commit()?;
            Ok(PlanOutcome::Approved { first })
        })
    }

    /// Withdraws a scheduled change: the status, the exceptions that hung on it, their grants and the ledger entry commit together. The
    /// effective-date job reads only `Scheduled`, so nothing is left to cancel.
    pub fn withdraw_label_change(&self, id: Uuid, by: &str, note: &str, ledger: AppendDecisionParams) -> Result<WithdrawOutcome, RepositoryError> {
        let (by, note) = (by.to_string(), note.to_string());
        self.with_client(move |c| {
            let mut tx = c.transaction()?;
            let Some(change) = changes::get_for_update(&mut tx, id)? else { return Ok(WithdrawOutcome::Missing) };
            if change.status != "Scheduled" {
                return Ok(WithdrawOutcome::NotScheduled(change.status));
            }
            changes::set_status(&mut tx, id, "Withdrawn", &by, &note, None)?;
            let grants: Vec<i64> = tx
                .query("UPDATE label_change_exceptions SET status = 'Withdrawn', decided_at = NOW(), decision_note = 'The change was withdrawn' WHERE change_id = $1 AND status IN ('Requested', 'Approved') RETURNING grant_id", &[&id])?
                .iter()
                .filter_map(|r| r.get::<_, Option<i64>>(0))
                .collect();
            let revoked = tx.execute("UPDATE access_grants SET revoked_at = NOW(), revoked_by = $2 WHERE id = ANY($1) AND revoked_at IS NULL", &[&grants, &by])?;
            append_ledger_in_tx(&mut tx, ledger)?;
            tx.commit()?;
            Ok(WithdrawOutcome::Done { grants_revoked: revoked as usize })
        })
    }

    /// Stores a request for an exception. `None` when this person has a live one for the change already.
    pub fn request_label_exception(&self, n: NewException) -> Result<Option<Uuid>, RepositoryError> {
        self.with_client(move |c| {
            let id = Uuid::new_v4();
            let made = c.execute(
                "INSERT INTO label_change_exceptions (id, change_id, app_slug, table_id, person, justification, expires_at, adds) \
                 VALUES ($1, $2, $3, $4, lower($5), $6, $7::text::timestamptz, $8) \
                 ON CONFLICT (change_id, person) WHERE status IN ('Requested', 'Approved') DO NOTHING",
                &[&id, &n.change_id, &n.app_slug, &n.table_id, &n.person, &n.justification, &n.expires_at, &n.adds],
            )?;
            Ok((made == 1).then_some(id))
        })
    }

    pub fn label_exception(&self, id: Uuid) -> Result<Option<Exception>, RepositoryError> {
        self.with_client(move |c| Ok(c.query_opt(&format!("SELECT {EXCEPTION_COLUMNS} FROM label_change_exceptions WHERE id = $1"), &[&id])?.as_ref().map(exception_of)))
    }

    /// The exceptions asked for a change, oldest first.
    pub fn label_exceptions(&self, change: Uuid) -> Result<Vec<Exception>, RepositoryError> {
        self.with_client(move |c| Ok(c.query(&format!("SELECT {EXCEPTION_COLUMNS} FROM label_change_exceptions WHERE change_id = $1 ORDER BY requested_at, id"), &[&change])?.iter().map(exception_of).collect()))
    }

    /// A person's live exception for a change, if they have one.
    pub fn label_exception_of(&self, change: Uuid, person: &str) -> Result<Option<Exception>, RepositoryError> {
        let person = person.to_string();
        self.with_client(move |c| {
            Ok(c.query_opt(&format!("SELECT {EXCEPTION_COLUMNS} FROM label_change_exceptions WHERE change_id = $1 AND person = lower($2) AND status IN ('Requested', 'Approved')"), &[&change, &person])?.as_ref().map(exception_of))
        })
    }

    /// Decides an exception. An approval makes the grant in the same transaction, and the ledger entry is written with it. The change must
    /// still be scheduled, and the exception still waiting.
    pub fn decide_label_exception(&self, id: Uuid, by: &str, note: &str, grant: Option<ExceptionGrant>, ledger: AppendDecisionParams) -> Result<ExceptionOutcome, RepositoryError> {
        let (by, note) = (by.to_string(), note.to_string());
        self.with_client(move |c| {
            let mut tx = c.transaction()?;
            let Some(row) = tx.query_opt("SELECT status, change_id FROM label_change_exceptions WHERE id = $1 FOR UPDATE", &[&id])? else { return Ok(ExceptionOutcome::Missing) };
            let (status, change_id): (String, Uuid) = (row.get(0), row.get(1));
            if status != "Requested" {
                return Ok(ExceptionOutcome::Not(status));
            }
            let Some(change) = changes::get_for_update(&mut tx, change_id)? else { return Ok(ExceptionOutcome::Missing) };
            if change.status != "Scheduled" {
                return Ok(ExceptionOutcome::Not(change.status));
            }
            let mut grant_id = None;
            if let Some(g) = &grant {
                // The same grant a request for access makes: a person's viewer grant on the app, source `request`, with its expiry.
                let new_id: i64 = tx
                    .query_one(
                        "INSERT INTO access_grants (holder_kind, holder_id, role, anchor_kind, anchor_id, valid_until, source, reason, created_by) \
                         VALUES ('person', $1, 'viewer', 'container', $2, $3::text::timestamptz, 'request', $4, $5) RETURNING id",
                        &[&g.holder, &g.anchor, &g.until, &g.reason, &g.created_by],
                    )?
                    .get(0);
                grant_id = Some(new_id);
            }
            let to = if grant.is_some() { "Approved" } else { "Declined" };
            tx.execute("UPDATE label_change_exceptions SET status = $2, decided_by = $3, decided_at = NOW(), decision_note = $4, grant_id = $5 WHERE id = $1", &[&id, &to, &by, &note, &grant_id])?;
            append_ledger_in_tx(&mut tx, ledger)?;
            tx.commit()?;
            Ok(ExceptionOutcome::Done { grant_id })
        })
    }

    /// The categories a person's exceptions waive on a table today: those of an approved exception whose change has been applied and whose
    /// grant is neither revoked nor over. The phase 3 gate reads this.
    pub fn label_waivers(&self, person: &str, app: &str, table: &str) -> Result<Vec<String>, RepositoryError> {
        let (person, app, table) = (person.to_string(), app.to_string(), table.to_string());
        self.with_client(move |c| {
            let mut out: Vec<String> = c
                .query(
                    "SELECT unnest(e.adds) FROM label_change_exceptions e JOIN access_grants g ON g.id = e.grant_id \
                     WHERE e.person = lower($1) AND e.app_slug = $2 AND e.table_id = $3 AND e.in_force AND e.status = 'Approved' \
                       AND g.revoked_at IS NULL AND g.valid_from <= NOW() AND (g.valid_until IS NULL OR g.valid_until > NOW())",
                    &[&person, &app, &table],
                )?
                .iter()
                .map(|r| r.get(0))
                .collect();
            out.sort();
            out.dedup();
            Ok(out)
        })
    }

    /// The scheduled changes whose date has come at `now` (RFC 3339) and whose plan was approved.
    pub fn due_label_changes(&self, now: &str, limit: i64) -> Result<Vec<Uuid>, RepositoryError> {
        let now = now.to_string();
        self.with_client(move |c| Ok(changes::due(c, &now, limit)?))
    }

    /// Whether another row of the table already carries one of these tags.
    pub fn label_others_carry(&self, app: &str, table: &str, tags: Vec<String>, except: Vec<String>) -> Result<bool, RepositoryError> {
        let (app, table) = (app.to_string(), table.to_string());
        self.with_client(move |c| Ok(changes::others_carry(c, &app, &table, &tags, &except)?))
    }
}
