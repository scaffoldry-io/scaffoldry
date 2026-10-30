//! Trial copies of proposed apps (proposal-preview phase 3). A trial is a row here, and a workspace, an app and synthetic
//! records elsewhere. Destroying one removes all of it and keeps this row, marked, as the record that it existed.

use crate::repository::{PostgresRepository, RepositoryError};
use uuid::Uuid;

#[derive(Debug, Clone)]
pub struct TrialRow {
    pub id: Uuid,
    pub proposal_id: Uuid,
    pub owner: String,
    pub app_slug: String,
    pub workspace_id: String,
    pub records: i32,
    pub created_at: String,
    pub last_opened_at: String,
}

const COLUMNS: &str = "id, proposal_id, owner, app_slug, workspace_id, records, \
     to_char(created_at AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS\"Z\"'), \
     to_char(last_opened_at AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS\"Z\"')";

fn row_of(r: &postgres::Row) -> TrialRow {
    TrialRow { id: r.get(0), proposal_id: r.get(1), owner: r.get(2), app_slug: r.get(3), workspace_id: r.get(4), records: r.get(5), created_at: r.get(6), last_opened_at: r.get(7) }
}

/// Tables that can hold rows of an app. A trial's owner can fill any of them by using it, so each is cleared. `record_history`
/// is not on the list: it is append-only, and what it holds for a trial is synthetic.
const APP_TABLES: &[&str] = &[
    "record_index",
    "record_links",
    "record_comments",
    "trash_links",
    "index_status",
    "calc_status",
    "app_views",
    "page_settings",
    "file_refs",
    "app_imports",
    "workflow_automations",
    "process_instances",
    "dataset_records",
];

impl PostgresRepository {
    /// Records a trial, if its owner has fewer than `limit` open. The count and the insert happen under one lock, so two
    /// requests at once cannot both slip under the limit. False when the owner is at the limit.
    pub fn reserve_trial(&self, t: &TrialRow, limit: i64) -> Result<bool, RepositoryError> {
        let t = t.clone();
        self.with_client(move |c| {
            let mut tx = c.transaction()?;
            tx.execute("SELECT pg_advisory_xact_lock(hashtext($1))", &[&format!("trials:{}", t.owner.to_lowercase())])?;
            let open: i64 = tx.query_one("SELECT COUNT(*) FROM proposal_trials WHERE lower(owner) = lower($1) AND destroyed_at IS NULL", &[&t.owner])?.get(0);
            if open >= limit {
                return Ok(false);
            }
            tx.execute(
                "INSERT INTO proposal_trials (id, proposal_id, owner, app_slug, workspace_id) VALUES ($1, $2, $3, $4, $5)",
                &[&t.id, &t.proposal_id, &t.owner, &t.app_slug, &t.workspace_id],
            )?;
            tx.commit()?;
            Ok(true)
        })
    }

    pub fn set_trial_records(&self, id: Uuid, records: i32) -> Result<(), RepositoryError> {
        self.with_client(move |c| {
            c.execute("UPDATE proposal_trials SET records = $2 WHERE id = $1", &[&id, &records])?;
            Ok(())
        })
    }

    pub fn open_trials_of(&self, owner: &str) -> Result<Vec<TrialRow>, RepositoryError> {
        let owner = owner.to_string();
        self.with_client(move |c| {
            Ok(c.query(&format!("SELECT {COLUMNS} FROM proposal_trials WHERE lower(owner) = lower($1) AND destroyed_at IS NULL ORDER BY created_at"), &[&owner])?.iter().map(row_of).collect())
        })
    }

    pub fn trials_of_proposal(&self, proposal_id: Uuid) -> Result<Vec<TrialRow>, RepositoryError> {
        self.with_client(move |c| Ok(c.query(&format!("SELECT {COLUMNS} FROM proposal_trials WHERE proposal_id = $1 AND destroyed_at IS NULL"), &[&proposal_id])?.iter().map(row_of).collect()))
    }

    /// Open trials not opened for `days` days.
    pub fn stale_trials(&self, days: i64) -> Result<Vec<TrialRow>, RepositoryError> {
        self.with_client(move |c| {
            Ok(c.query(&format!("SELECT {COLUMNS} FROM proposal_trials WHERE destroyed_at IS NULL AND last_opened_at < NOW() - make_interval(days => ($1::bigint)::int)"), &[&days])?.iter().map(row_of).collect())
        })
    }

    /// Removes a trial's records and everything else it holds, its app, its workspace and its owner's place in it, then marks
    /// the row. One transaction. The caches other servers keep are told to reload.
    pub fn destroy_trial(&self, t: &TrialRow) -> Result<(), RepositoryError> {
        let t = t.clone();
        self.with_client(move |c| {
            let mut tx = c.transaction()?;
            let present: Vec<String> = tx
                .query("SELECT DISTINCT table_name::text FROM information_schema.columns WHERE column_name = 'app_slug' AND table_schema = current_schema() AND table_name = ANY($1)", &[&APP_TABLES.iter().map(|s| s.to_string()).collect::<Vec<_>>()])?
                .iter()
                .map(|r| r.get(0))
                .collect();
            for table in APP_TABLES.iter().filter(|t| present.iter().any(|p| p == *t)) {
                tx.execute(&format!("DELETE FROM {table} WHERE app_slug = $1"), &[&t.app_slug])?;
            }
            tx.execute("DELETE FROM app_manifests WHERE slug = $1", &[&t.app_slug])?;
            tx.execute("DELETE FROM workspace_collaborators WHERE workspace_id = $1", &[&t.workspace_id])?;
            tx.execute("DELETE FROM workspaces WHERE id = $1", &[&t.workspace_id])?;
            tx.execute("UPDATE proposal_trials SET destroyed_at = NOW() WHERE id = $1 AND destroyed_at IS NULL", &[&t.id])?;
            crate::cache::bump(&mut tx, "manifests")?;
            crate::cache::bump(&mut tx, "workspaces")?;
            tx.commit()?;
            Ok(())
        })
    }

    /// Starts a trial's days over. False when the person has no such open trial.
    pub fn touch_trial(&self, id: Uuid, owner: &str) -> Result<bool, RepositoryError> {
        let owner = owner.to_string();
        self.with_client(move |c| Ok(c.execute("UPDATE proposal_trials SET last_opened_at = NOW() WHERE id = $1 AND lower(owner) = lower($2) AND destroyed_at IS NULL", &[&id, &owner])? > 0))
    }

    /// A whole-number platform setting as stored, for work that runs without the server's cache.
    pub fn setting_i64(&self, key: &str) -> Result<Option<i64>, RepositoryError> {
        let key = key.to_string();
        self.with_client(move |c| Ok(c.query_opt("SELECT value::text FROM platform_settings WHERE key = $1", &[&key])?.and_then(|r| r.get::<_, String>(0).trim().parse::<i64>().ok())))
    }
}
