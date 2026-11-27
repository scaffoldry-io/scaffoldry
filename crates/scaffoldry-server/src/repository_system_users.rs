//! Data layer phase 9: the statements on `system_users` and `system_grants`.
//!
//! Neither table names a row of a dataset, so they are not behind the data door. A decision runs in one transaction with its ledger entry.
//! Every date is compared by the database's clock.

use crate::repository::{append_ledger_in_tx, AppendDecisionParams, PostgresRepository, RepositoryError};
use uuid::Uuid;

const STAMP: &str = "'YYYY-MM-DD\"T\"HH24:MI:SS\"Z\"'";

fn at(column: &str) -> String {
    format!("to_char({column} AT TIME ZONE 'UTC', {STAMP})")
}

/// An automation's identity, its owner and its notes.
#[derive(Debug, Clone)]
pub struct SystemUser {
    pub flow_id: String,
    pub app_slug: String,
    pub name: String,
    pub notes: String,
    /// The owner as a membership key: an address, `group:{id}` or `position:{key}@{unit}`.
    pub owner: String,
    pub created_by: String,
    pub created_at: String,
}

/// One grant. `lapsed` is true when it is active and its review date has come, by the database's clock.
#[derive(Debug, Clone)]
pub struct SystemGrant {
    pub id: Uuid,
    pub flow_id: String,
    pub dataset_id: String,
    pub app_slug: String,
    pub table_id: String,
    pub action: String,
    pub fields: Vec<String>,
    pub reason: String,
    pub approved_by: String,
    pub approved_at: String,
    pub review_by: String,
    pub status: String,
    pub lapsed: bool,
    pub revoked_by: Option<String>,
    pub revoke_reason: Option<String>,
}

/// What a grant to store holds.
pub struct NewGrant {
    pub id: Uuid,
    pub flow_id: String,
    pub dataset_id: String,
    pub app_slug: String,
    pub table_id: String,
    pub action: String,
    pub fields: Vec<String>,
    pub reason: String,
    pub approved_by: String,
    pub review_by: String,
}

/// What a revoke did.
pub enum Revoked {
    Missing,
    /// The grant is not active. Holds the status it has.
    Not(String),
    Done,
}

fn grant_of(r: &postgres::Row) -> SystemGrant {
    SystemGrant {
        id: r.get(0),
        flow_id: r.get(1),
        dataset_id: r.get(2),
        app_slug: r.get(3),
        table_id: r.get(4),
        action: r.get(5),
        fields: r.get(6),
        reason: r.get(7),
        approved_by: r.get(8),
        approved_at: r.get(9),
        review_by: r.get(10),
        status: r.get(11),
        lapsed: r.get(12),
        revoked_by: r.get(13),
        revoke_reason: r.get(14),
    }
}

fn grant_columns() -> String {
    format!(
        "id, flow_id, dataset_id, app_slug, table_id, action, fields, reason, approved_by, {}, {}, status, (status = 'Active' AND review_by <= NOW()), revoked_by, revoke_reason",
        at("approved_at"),
        at("review_by")
    )
}

impl PostgresRepository {
    pub fn system_user(&self, flow_id: &str) -> Result<Option<SystemUser>, RepositoryError> {
        let flow_id = flow_id.to_string();
        self.with_client(move |c| {
            let sql = format!("SELECT flow_id, app_slug, name, notes, owner, created_by, {} FROM system_users WHERE flow_id = $1", at("created_at"));
            Ok(c.query_opt(sql.as_str(), &[&flow_id])?.map(|r| SystemUser { flow_id: r.get(0), app_slug: r.get(1), name: r.get(2), notes: r.get(3), owner: r.get(4), created_by: r.get(5), created_at: r.get(6) }))
        })
    }

    /// Stores a new system user. False when the flow already has one.
    pub fn insert_system_user(&self, flow_id: &str, app_slug: &str, name: &str, notes: &str, owner: &str, by: &str) -> Result<bool, RepositoryError> {
        let (flow_id, app_slug, name, notes, owner, by) = (flow_id.to_string(), app_slug.to_string(), name.to_string(), notes.to_string(), owner.to_string(), by.to_string());
        self.with_client(move |c| {
            Ok(c.execute("INSERT INTO system_users (flow_id, app_slug, name, notes, owner, created_by, updated_by) VALUES ($1, $2, $3, $4, $5, $6, $6) ON CONFLICT (flow_id) DO NOTHING", &[&flow_id, &app_slug, &name, &notes, &owner, &by])? == 1)
        })
    }

    /// Changes the name, the notes or the owner. A part left `None` stays as it is.
    pub fn update_system_user(&self, flow_id: &str, name: Option<&str>, notes: Option<&str>, owner: Option<&str>, by: &str) -> Result<(), RepositoryError> {
        let (flow_id, name, notes, owner, by) = (flow_id.to_string(), name.map(str::to_string), notes.map(str::to_string), owner.map(str::to_string), by.to_string());
        self.with_client(move |c| {
            c.execute(
                "UPDATE system_users SET name = COALESCE($2, name), notes = COALESCE($3, notes), owner = COALESCE($4, owner), updated_by = $5, updated_at = NOW() WHERE flow_id = $1",
                &[&flow_id, &name, &notes, &owner, &by],
            )?;
            Ok(())
        })
    }

    /// Every grant of a flow, newest first.
    pub fn system_grants(&self, flow_id: &str) -> Result<Vec<SystemGrant>, RepositoryError> {
        let flow_id = flow_id.to_string();
        self.with_client(move |c| {
            let sql = format!("SELECT {} FROM system_grants WHERE flow_id = $1 ORDER BY system_grants.approved_at DESC, id", grant_columns());
            Ok(c.query(sql.as_str(), &[&flow_id])?.iter().map(grant_of).collect())
        })
    }

    pub fn system_grant(&self, id: Uuid) -> Result<Option<SystemGrant>, RepositoryError> {
        self.with_client(move |c| {
            let sql = format!("SELECT {} FROM system_grants WHERE id = $1", grant_columns());
            Ok(c.query_opt(sql.as_str(), &[&id])?.as_ref().map(grant_of))
        })
    }

    /// Stores an approved grant with its ledger entry, in one transaction.
    pub fn approve_system_grant(&self, g: NewGrant, ledger: AppendDecisionParams) -> Result<(), RepositoryError> {
        self.with_client(move |c| {
            let mut tx = c.transaction()?;
            tx.execute(
                "INSERT INTO system_grants (id, flow_id, dataset_id, app_slug, table_id, action, fields, reason, approved_by, review_by) \
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10::text::timestamptz)",
                &[&g.id, &g.flow_id, &g.dataset_id, &g.app_slug, &g.table_id, &g.action, &g.fields, &g.reason, &g.approved_by, &g.review_by],
            )?;
            append_ledger_in_tx(&mut tx, ledger)?;
            tx.commit()?;
            Ok(())
        })
    }

    /// Ends an active grant early, with its ledger entry, in one transaction.
    pub fn revoke_system_grant(&self, id: Uuid, by: &str, reason: &str, ledger: AppendDecisionParams) -> Result<Revoked, RepositoryError> {
        let (by, reason) = (by.to_string(), reason.to_string());
        self.with_client(move |c| {
            let mut tx = c.transaction()?;
            let Some(row) = tx.query_opt("SELECT status FROM system_grants WHERE id = $1 FOR UPDATE", &[&id])? else { return Ok(Revoked::Missing) };
            let status: String = row.get(0);
            if status != "Active" {
                return Ok(Revoked::Not(status));
            }
            tx.execute("UPDATE system_grants SET status = 'Revoked', revoked_by = $2, revoked_at = NOW(), revoke_reason = $3 WHERE id = $1", &[&id, &by, &reason])?;
            append_ledger_in_tx(&mut tx, ledger)?;
            tx.commit()?;
            Ok(Revoked::Done)
        })
    }

    /// Takes the lapse alerts of a flow that nobody has sent yet: the datasets of its active grants whose date has come. One statement
    /// finds and marks them, so two runs that meet the same lapse cannot both take it.
    pub fn claim_lapse_alerts(&self, flow_id: &str) -> Result<Vec<String>, RepositoryError> {
        let flow_id = flow_id.to_string();
        self.with_client(move |c| {
            Ok(c.query(
                "UPDATE system_grants SET lapse_notified_at = NOW() WHERE flow_id = $1 AND status = 'Active' AND review_by <= NOW() AND lapse_notified_at IS NULL RETURNING dataset_id",
                &[&flow_id],
            )?
            .iter()
            .map(|r| r.get(0))
            .collect())
        })
    }

    /// Gives back a claim whose alert could not be sent, so the next run tries again.
    pub fn release_lapse_alerts(&self, flow_id: &str) -> Result<(), RepositoryError> {
        let flow_id = flow_id.to_string();
        self.with_client(move |c| {
            c.execute("UPDATE system_grants SET lapse_notified_at = NULL WHERE flow_id = $1 AND status = 'Active' AND review_by <= NOW() AND lapse_notified_at >= NOW() - interval '1 minute'", &[&flow_id])?;
            Ok(())
        })
    }
}
