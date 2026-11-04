//! Workflows phase 4: task grants, in `task_grants`.
//!
//! A grant opens with its step and closes with it, in the same transaction. Reading one is the only way a person without
//! rights to an app reaches a record of it, and only that record and the grant's fields.

use crate::repository::{PostgresRepository, RepositoryError};

/// A grant to open: who, on which record, and which fields.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewGrant {
    pub person: String,
    pub app_slug: String,
    pub table_id: String,
    pub record_id: String,
    pub read_fields: Vec<String>,
    pub write_fields: Vec<String>,
}

/// An open grant.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskGrant {
    pub instance_id: String,
    pub step_id: String,
    pub person: String,
    pub record_id: String,
    pub read_fields: Vec<String>,
    pub write_fields: Vec<String>,
}

/// Opens `grants` on the instance's step.
pub(crate) fn open_task_grants_in_tx(tx: &mut postgres::Transaction<'_>, instance_id: &str, step_id: &str, grants: &[NewGrant]) -> Result<(), RepositoryError> {
    for g in grants {
        tx.execute(
            "INSERT INTO task_grants (instance_id, step_id, person, app_slug, table_id, record_id, read_fields, write_fields) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8) \
             ON CONFLICT (instance_id, step_id, person) DO UPDATE SET state = 'open', closed_at = NULL, \
                 read_fields = EXCLUDED.read_fields, write_fields = EXCLUDED.write_fields, record_id = EXCLUDED.record_id",
            &[&instance_id, &step_id, &g.person, &g.app_slug, &g.table_id, &g.record_id, &g.read_fields, &g.write_fields],
        )?;
    }
    Ok(())
}

/// Closes every open grant of the instance. Each path that moves an instance's status out of waiting calls this in its
/// own transaction, and a scan test holds them to it.
pub fn close_task_grants_in_tx(tx: &mut postgres::Transaction<'_>, instance_id: &str) -> Result<u64, RepositoryError> {
    Ok(tx.execute("UPDATE task_grants SET state = 'closed', closed_at = NOW() WHERE instance_id = $1 AND state = 'open'", &[&instance_id])?)
}

/// Moves the step's grants to `grants`: an escalation or a reassignment closes the old ones and opens the new.
pub fn replace_task_grants_in_tx(tx: &mut postgres::Transaction<'_>, instance_id: &str, step_id: &str, grants: &[NewGrant]) -> Result<(), RepositoryError> {
    close_task_grants_in_tx(tx, instance_id)?;
    open_task_grants_in_tx(tx, instance_id, step_id, grants)
}

impl PostgresRepository {
    /// The open grant `person` holds on this record of `app_slug`, if any.
    pub fn open_task_grant(&self, person: &str, app_slug: &str, record_id: &str) -> Result<Option<TaskGrant>, RepositoryError> {
        let (p, a, r) = (person.to_string(), app_slug.to_string(), record_id.to_string());
        self.with_client(move |c| {
            Ok(c.query_opt(
                "SELECT instance_id, step_id, person, record_id, read_fields, write_fields FROM task_grants \
                 WHERE lower(person) = lower($1) AND app_slug = $2 AND record_id = $3 AND state = 'open' LIMIT 1",
                &[&p, &a, &r],
            )?
            .map(|row| TaskGrant { instance_id: row.get(0), step_id: row.get(1), person: row.get(2), record_id: row.get(3), read_fields: row.get(4), write_fields: row.get(5) }))
        })
    }

    /// Whether `person` holds an open grant on the instance.
    pub fn holds_task_grant(&self, person: &str, instance_id: &str) -> Result<bool, RepositoryError> {
        let (p, i) = (person.to_string(), instance_id.to_string());
        self.with_client(move |c| Ok(c.query_opt("SELECT 1 FROM task_grants WHERE lower(person) = lower($1) AND instance_id = $2 AND state = 'open'", &[&p, &i])?.is_some()))
    }
}
