//! Approved app templates (import-export phase 4). A template is a bundle a second person reviewed.

use crate::repository::{PostgresRepository, RepositoryError};
use serde_json::Value;
use uuid::Uuid;

const COLUMNS: &str = "id, name, description, category, bundle, bundle_sha256, version, status, unit_id, published_by, \
                       to_char(published_at AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS\"Z\"'), decided_by, \
                       to_char(decided_at AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS\"Z\"'), rationale";

#[derive(Debug, Clone)]
pub struct TemplateRow {
    pub id: Uuid,
    pub name: String,
    pub description: String,
    pub category: String,
    pub bundle: Value,
    pub bundle_sha256: String,
    pub version: i32,
    /// `pending`, `approved`, `rejected` or `retired`.
    pub status: String,
    pub unit_id: Option<Uuid>,
    pub published_by: String,
    pub published_at: String,
    pub decided_by: Option<String>,
    pub decided_at: Option<String>,
    pub rationale: String,
}

fn row_of(r: &postgres::Row) -> TemplateRow {
    TemplateRow {
        id: r.get(0),
        name: r.get(1),
        description: r.get(2),
        category: r.get(3),
        bundle: r.get(4),
        bundle_sha256: r.get::<_, String>(5).trim().to_string(),
        version: r.get(6),
        status: r.get(7),
        unit_id: r.get(8),
        published_by: r.get(9),
        published_at: r.get(10),
        decided_by: r.get(11),
        decided_at: r.get(12),
        rationale: r.get(13),
    }
}

impl PostgresRepository {
    pub fn insert_template(&self, t: &TemplateRow) -> Result<(), RepositoryError> {
        let t = t.clone();
        self.with_client(move |c| {
            c.execute(
                "INSERT INTO app_templates (id, name, description, category, bundle, bundle_sha256, version, status, unit_id, published_by) \
                 VALUES ($1, $2, $3, $4, $5, $6, $7, 'pending', $8, $9)",
                &[&t.id, &t.name, &t.description, &t.category, &t.bundle, &t.bundle_sha256, &t.version, &t.unit_id, &t.published_by],
            )?;
            Ok(())
        })
    }

    pub fn get_template(&self, id: Uuid) -> Result<Option<TemplateRow>, RepositoryError> {
        self.with_client(move |c| Ok(c.query_opt(&format!("SELECT {COLUMNS} FROM app_templates WHERE id = $1"), &[&id])?.as_ref().map(row_of)))
    }

    /// Approved templates, by category then name.
    pub fn approved_templates(&self) -> Result<Vec<TemplateRow>, RepositoryError> {
        self.with_client(move |c| {
            let rows = c.query(&format!("SELECT {COLUMNS} FROM app_templates WHERE status = 'approved' ORDER BY category, name, id"), &[])?;
            Ok(rows.iter().map(row_of).collect())
        })
    }

    /// Moves a pending template to `approved` or `rejected`. Returns the status found when it was not pending.
    pub fn decide_template(&self, id: Uuid, status: &str, by: &str, rationale: &str) -> Result<Result<(), String>, RepositoryError> {
        let (status, by, rationale) = (status.to_string(), by.to_string(), rationale.to_string());
        self.with_client(move |c| {
            let done = c.execute(
                "UPDATE app_templates SET status = $2, decided_by = $3, decided_at = NOW(), rationale = $4 WHERE id = $1 AND status = 'pending'",
                &[&id, &status, &by, &rationale],
            )?;
            if done == 1 {
                return Ok(Ok(()));
            }
            Ok(Err(c.query_opt("SELECT status FROM app_templates WHERE id = $1", &[&id])?.map(|r| r.get(0)).unwrap_or_else(|| "missing".to_string())))
        })
    }

    /// Retires an approved template. Returns the status found when it was not approved.
    pub fn retire_template(&self, id: Uuid, by: &str, reason: &str) -> Result<Result<(), String>, RepositoryError> {
        let (by, reason) = (by.to_string(), reason.to_string());
        self.with_client(move |c| {
            let done = c.execute(
                "UPDATE app_templates SET status = 'retired', decided_by = $2, decided_at = NOW(), rationale = $3 WHERE id = $1 AND status = 'approved'",
                &[&id, &by, &reason],
            )?;
            if done == 1 {
                return Ok(Ok(()));
            }
            Ok(Err(c.query_opt("SELECT status FROM app_templates WHERE id = $1", &[&id])?.map(|r| r.get(0)).unwrap_or_else(|| "missing".to_string())))
        })
    }
}
