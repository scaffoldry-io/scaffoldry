//! Data layer phase 10: the reads and the decisions of row shares.
//!
//! Every statement on `row_shares` and `row_share_rows` is in `scaffoldry_data::shares`. This file runs them in a transaction with the
//! ledger entry, so a decision either does all of it or none.

use crate::repository::{append_ledger_in_tx, AppendDecisionParams, PostgresRepository, RepositoryError};
use scaffoldry_data::shares::{self, Cover, NewShare, Share, Which};
use uuid::Uuid;

/// What a decision on a request did.
pub enum Decided {
    Missing,
    /// The request is not waiting any more. Holds the status it has.
    Not(String),
    Done,
}

/// What an end did.
pub enum Ended {
    Missing,
    /// The share is not approved. Holds the status it has.
    Not(String),
    Done,
}

impl PostgresRepository {
    pub fn insert_row_share(&self, n: NewShare<'_>) -> Result<(), RepositoryError> {
        let owned = (n.id, n.dataset_id.to_string(), n.app_slug.to_string(), n.table_id.to_string(), n.records.to_vec(), n.grantee.to_string(), n.access.to_string());
        let rest = (n.fields.to_vec(), n.kind.to_string(), n.categories.to_vec(), n.justification.to_string(), n.requested_by.to_string(), n.expires_at.to_string(), n.assessment.clone());
        self.with_client(move |c| {
            let new = NewShare {
                id: owned.0,
                dataset_id: &owned.1,
                app_slug: &owned.2,
                table_id: &owned.3,
                records: &owned.4,
                grantee: &owned.5,
                access: &owned.6,
                fields: &rest.0,
                kind: &rest.1,
                categories: &rest.2,
                justification: &rest.3,
                requested_by: &rest.4,
                expires_at: &rest.5,
                assessment: &rest.6,
            };
            Ok(shares::insert(c, &new)?)
        })
    }

    pub fn row_share(&self, id: Uuid) -> Result<Option<Share>, RepositoryError> {
        self.with_client(move |c| Ok(shares::get(c, id)?))
    }

    pub fn row_shares_of(&self, dataset: &str, status: Option<String>, record: Option<String>, limit: i64) -> Result<Vec<Share>, RepositoryError> {
        let dataset = dataset.to_string();
        self.with_client(move |c| {
            let which = match &status {
                Some(s) => Which::Status(s),
                None => Which::Live,
            };
            Ok(shares::list(c, &dataset, which, record.as_deref(), limit)?)
        })
    }

    /// Approves a waiting request that has not run out: the status, the lookup rows and the ledger entry commit together.
    pub fn approve_row_share(&self, id: Uuid, by: &str, note: &str, ledger: AppendDecisionParams) -> Result<Decided, RepositoryError> {
        let (by, note) = (by.to_string(), note.to_string());
        self.with_client(move |c| {
            let mut tx = c.transaction()?;
            let Some(share) = shares::get_for_update(&mut tx, id)? else { return Ok(Decided::Missing) };
            if share.status != "Requested" {
                return Ok(Decided::Not(share.status));
            }
            if !shares::approve(&mut tx, &share, &by, &note)? {
                return Ok(Decided::Not("decided".to_string()));
            }
            append_ledger_in_tx(&mut tx, ledger)?;
            tx.commit()?;
            Ok(Decided::Done)
        })
    }

    /// Closes a waiting request without granting anything. `ledger` is written for a decline, and not for a withdrawal of a request.
    pub fn close_row_share_request(&self, id: Uuid, status: &str, by: &str, note: &str, ledger: Option<AppendDecisionParams>) -> Result<Decided, RepositoryError> {
        let (status, by, note) = (status.to_string(), by.to_string(), note.to_string());
        self.with_client(move |c| {
            let mut tx = c.transaction()?;
            let Some(share) = shares::get_for_update(&mut tx, id)? else { return Ok(Decided::Missing) };
            if share.status != "Requested" || !shares::close_request(&mut tx, id, &status, &by, &note)? {
                return Ok(Decided::Not(share.status));
            }
            if let Some(l) = ledger {
                append_ledger_in_tx(&mut tx, l)?;
            }
            tx.commit()?;
            Ok(Decided::Done)
        })
    }

    /// Ends an approved share, by its date or early. The status, the removal of the lookup rows and the ledger entry commit together.
    pub fn end_row_share(&self, id: Uuid, status: &str, by: Option<&str>, note: &str, ledger: AppendDecisionParams) -> Result<Ended, RepositoryError> {
        let (status, by, note) = (status.to_string(), by.map(str::to_string), note.to_string());
        self.with_client(move |c| {
            let mut tx = c.transaction()?;
            let Some(share) = shares::get_for_update(&mut tx, id)? else { return Ok(Ended::Missing) };
            if share.status != "Approved" || !shares::end(&mut tx, id, &status, by.as_deref(), &note)? {
                return Ok(Ended::Not(share.status));
            }
            append_ledger_in_tx(&mut tx, ledger)?;
            tx.commit()?;
            Ok(Ended::Done)
        })
    }

    pub fn due_row_shares(&self, now: &str, limit: i64) -> Result<Vec<Uuid>, RepositoryError> {
        let now = now.to_string();
        self.with_client(move |c| Ok(shares::due(c, &now, limit)?))
    }

    pub fn lapse_row_shares(&self, now: &str) -> Result<u64, RepositoryError> {
        let now = now.to_string();
        self.with_client(move |c| Ok(shares::lapse(c, &now)?))
    }

    pub fn unnotified_row_shares(&self, limit: i64) -> Result<Vec<Share>, RepositoryError> {
        self.with_client(move |c| Ok(shares::unnotified(c, limit)?))
    }

    pub fn mark_row_share_notified(&self, id: Uuid) -> Result<(), RepositoryError> {
        self.with_client(move |c| Ok(shares::mark_notified(c, id)?))
    }

    /// Whether the person holds a live share on the table.
    pub fn has_live_row_share(&self, grantee: &str, app: &str, table: &str) -> Result<bool, RepositoryError> {
        let (grantee, app, table) = (grantee.to_lowercase(), app.to_string(), table.to_string());
        self.with_client(move |c| Ok(shares::has_live(c, &grantee, &app, &table)?))
    }

    /// The live shares the person holds on these rows.
    pub fn row_share_covers(&self, grantee: &str, app: &str, ids: Vec<String>) -> Result<Vec<Cover>, RepositoryError> {
        let (grantee, app) = (grantee.to_lowercase(), app.to_string());
        self.with_client(move |c| Ok(shares::covers(c, &grantee, &app, &ids)?))
    }
}
