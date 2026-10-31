//! Snapshots (import-export phase 5): the rows that say a snapshot exists and what it holds. The bytes are in the file store.

use crate::repository::{PostgresRepository, RepositoryError};
use serde_json::Value;
use uuid::Uuid;

const COLUMNS: &str = "id, app_slug, name, kind, state, file_sha256, record_count, size, error, definition, created_by, \
                       to_char(created_at AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS\"Z\"')";

#[derive(Debug, Clone)]
pub struct SnapshotRow {
    pub id: Uuid,
    pub app_slug: String,
    pub name: String,
    /// `manual`, `scheduled` or `pre_change`.
    pub kind: String,
    /// `pending`, `ready` or `failed`.
    pub state: String,
    pub file_sha256: Option<String>,
    pub record_count: Option<i64>,
    pub size: Option<i64>,
    pub error: Option<String>,
    pub definition: Value,
    pub created_by: String,
    pub created_at: String,
}

fn row_of(r: &postgres::Row) -> SnapshotRow {
    SnapshotRow {
        id: r.get(0),
        app_slug: r.get(1),
        name: r.get(2),
        kind: r.get(3),
        state: r.get(4),
        file_sha256: r.get::<_, Option<String>>(5).map(|s| s.trim().to_string()),
        record_count: r.get(6),
        size: r.get(7),
        error: r.get(8),
        definition: r.get(9),
        created_by: r.get(10),
        created_at: r.get(11),
    }
}

/// One record as a snapshot holds it.
#[derive(Debug, Clone)]
pub struct StoredRecord {
    pub id: String,
    pub data: Value,
    pub ceds_mapping: Value,
    pub is_ferpa_sensitive: bool,
    pub created_by: Option<String>,
    pub created_at: String,
}

impl PostgresRepository {
    pub fn insert_snapshot(&self, s: &SnapshotRow) -> Result<(), RepositoryError> {
        let s = s.clone();
        self.with_client(move |c| {
            c.execute(
                "INSERT INTO app_snapshots (id, app_slug, name, kind, definition, created_by) VALUES ($1, $2, $3, $4, $5, $6)",
                &[&s.id, &s.app_slug, &s.name, &s.kind, &s.definition, &s.created_by],
            )?;
            Ok(())
        })
    }

    pub fn get_snapshot(&self, id: Uuid) -> Result<Option<SnapshotRow>, RepositoryError> {
        self.with_client(move |c| Ok(c.query_opt(&format!("SELECT {COLUMNS} FROM app_snapshots WHERE id = $1"), &[&id])?.as_ref().map(row_of)))
    }

    /// An app's snapshots, newest first.
    pub fn list_snapshots(&self, app_slug: &str) -> Result<Vec<SnapshotRow>, RepositoryError> {
        let app = app_slug.to_string();
        self.with_client(move |c| {
            let rows = c.query(&format!("SELECT {COLUMNS} FROM app_snapshots WHERE app_slug = $1 ORDER BY created_at DESC, id"), &[&app])?;
            Ok(rows.iter().map(row_of).collect())
        })
    }

    /// A ready snapshot of the app made in the last `days` days.
    pub fn recent_snapshot(&self, app_slug: &str, days: i32) -> Result<Option<Uuid>, RepositoryError> {
        let app = app_slug.to_string();
        self.with_client(move |c| {
            Ok(c.query_opt(
                "SELECT id FROM app_snapshots WHERE app_slug = $1 AND state = 'ready' AND created_at > NOW() - make_interval(days => $2) ORDER BY created_at DESC LIMIT 1",
                &[&app, &days],
            )?
            .map(|r| r.get(0)))
        })
    }

    /// Marks a snapshot ready and keeps its bytes. The file reference is attached, so the unused-upload purge leaves it.
    pub fn finish_snapshot(&self, id: Uuid, sha256: &str, records: i64, size: i64) -> Result<(), RepositoryError> {
        let sha = sha256.to_string();
        self.with_client(move |c| {
            let mut tx = c.transaction()?;
            let (app, name): (String, String) = {
                let r = tx.query_one("SELECT app_slug, name FROM app_snapshots WHERE id = $1", &[&id])?;
                (r.get(0), r.get(1))
            };
            tx.execute("INSERT INTO files (sha256, size, media_type) VALUES ($1, $2, 'application/gzip') ON CONFLICT (sha256) DO NOTHING", &[&sha, &size])?;
            tx.execute(
                "INSERT INTO file_refs (id, sha256, app_slug, name, uploaded_by, scan_state, attached_at) VALUES ($1, $2, $3, $4, 'snapshot', 'skipped', NOW())",
                &[&id, &sha, &app, &format!("{name}.jsonl.gz")],
            )?;
            tx.execute("UPDATE app_snapshots SET state = 'ready', file_sha256 = $2, record_count = $3, size = $4, error = NULL WHERE id = $1", &[&id, &sha, &records, &size])?;
            tx.commit()?;
            Ok(())
        })
    }

    pub fn fail_snapshot(&self, id: Uuid, error: &str) -> Result<(), RepositoryError> {
        let error = error.to_string();
        self.with_client(move |c| {
            c.execute("UPDATE app_snapshots SET state = 'failed', error = $2 WHERE id = $1", &[&id, &error])?;
            Ok(())
        })
    }

    /// Moves a proposal from one status to another. False when it was not in `from`.
    pub fn move_proposal(&self, id: Uuid, from: &str, to: &str) -> Result<bool, RepositoryError> {
        let (from, to) = (from.to_string(), to.to_string());
        self.with_client(move |c| Ok(c.execute("UPDATE app_proposals SET status = $3 WHERE id = $1 AND status = $2", &[&id, &from, &to])? == 1))
    }

    /// The live records of an app after `after`, in id order.
    pub fn records_after(&self, app_slug: &str, after: &str, limit: i64) -> Result<Vec<StoredRecord>, RepositoryError> {
        let (app, after) = (app_slug.to_string(), after.to_string());
        self.with_client(move |c| {
            let rows = c.query(
                "SELECT id, data, ceds_mapping, is_ferpa_sensitive, created_by, to_char(created_at AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS.US\"Z\"') \
                 FROM dataset_records WHERE app_slug = $1 AND deleted_at IS NULL AND id > $2 ORDER BY id LIMIT $3",
                &[&app, &after, &limit],
            )?;
            Ok(rows
                .iter()
                .map(|r| StoredRecord { id: r.get(0), data: r.get(1), ceds_mapping: r.get(2), is_ferpa_sensitive: r.get(3), created_by: r.get(4), created_at: r.get(5) })
                .collect())
        })
    }

    pub fn live_record_count(&self, app_slug: &str) -> Result<i64, RepositoryError> {
        let app = app_slug.to_string();
        self.with_client(move |c| Ok(c.query_one("SELECT count(*) FROM dataset_records WHERE app_slug = $1 AND deleted_at IS NULL", &[&app])?.get(0)))
    }
}
