//! Import phase 1: The database backing for app imports.

use crate::repository::{PostgresRepository, RepositoryError};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppImport {
    pub id: Uuid,
    pub app_slug: String,
    pub file_ref_id: Uuid,
    pub target_table_id: Option<String>,
    pub status: String,
    pub delimiter: String,
    pub encoding: String,
    pub columns: Value,
    pub sample: Value,
    pub inferred_types: Value,
    pub mapping: Option<Value>,
    pub dry_run_result: Option<Value>,
    pub dry_run_at: Option<DateTime<Utc>>,
    pub run_result: Option<Value>,
    pub checkpoint: Option<Value>,
    pub rejected_file_ref_id: Option<Uuid>,
    pub created_by: String,
    pub created_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
}

fn import_from_row(row: &postgres::Row) -> AppImport {
    AppImport {
        id: row.get(0),
        app_slug: row.get(1),
        file_ref_id: row.get(2),
        target_table_id: row.get(3),
        status: row.get(4),
        delimiter: row.get(5),
        encoding: row.get(6),
        columns: row.get(7),
        sample: row.get(8),
        inferred_types: row.get(9),
        mapping: row.get(10),
        dry_run_result: row.get(11),
        dry_run_at: row.get(12),
        run_result: row.get(13),
        checkpoint: row.get(14),
        rejected_file_ref_id: row.get(15),
        created_by: row.get(16),
        created_at: row.get(17),
        expires_at: row.get(18),
    }
}

pub struct NewImport<'a> {
    pub id: Uuid,
    pub app_slug: &'a str,
    pub file_ref_id: Uuid,
    pub target_table_id: Option<&'a str>,
    pub created_by: &'a str,
}

impl PostgresRepository {
    pub fn insert_import(&self, new: NewImport<'_>) -> Result<AppImport, RepositoryError> {
        let (id, file_ref_id) = (new.id, new.file_ref_id);
        let (app, target, by) = (
            new.app_slug.to_string(),
            new.target_table_id.map(str::to_string),
            new.created_by.to_string(),
        );
        self.with_client(move |c| {
            let row = c.query_one(
                "INSERT INTO app_imports (id, app_slug, file_ref_id, target_table_id, created_by)                  VALUES ($1, $2, $3, $4, $5)                  RETURNING id, app_slug, file_ref_id, target_table_id, status, delimiter, encoding,                            columns, sample, inferred_types, mapping, dry_run_result, dry_run_at,                            run_result, checkpoint, rejected_file_ref_id, created_by, created_at, expires_at",
                &[&id, &app, &file_ref_id, &target, &by],
            )?;
            Ok(import_from_row(&row))
        })
    }

    pub fn get_import(&self, app_slug: &str, id: Uuid) -> Result<Option<AppImport>, RepositoryError> {
        let (app, id) = (app_slug.to_string(), id);
        self.with_client(move |c| {
            let row = c.query_opt(
                "SELECT id, app_slug, file_ref_id, target_table_id, status, delimiter, encoding,                         columns, sample, inferred_types, mapping, dry_run_result, dry_run_at,                         run_result, checkpoint, rejected_file_ref_id, created_by, created_at, expires_at                  FROM app_imports WHERE app_slug = $1 AND id = $2",
                &[&app, &id],
            )?;
            Ok(row.as_ref().map(import_from_row))
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub fn update_import_analysis(
        &self,
        id: Uuid,
        status: &str,
        delimiter: &str,
        encoding: &str,
        columns: &Value,
        sample: &Value,
        inferred_types: &Value,
    ) -> Result<(), RepositoryError> {
        let (id, status, delimiter, encoding) = (id, status.to_string(), delimiter.to_string(), encoding.to_string());
        let (columns, sample, inferred_types) = (columns.clone(), sample.clone(), inferred_types.clone());
        self.with_client(move |c| {
            c.execute(
                "UPDATE app_imports SET status = $2, delimiter = $3, encoding = $4, columns = $5,                         sample = $6, inferred_types = $7 WHERE id = $1",
                &[&id, &status, &delimiter, &encoding, &columns, &sample, &inferred_types],
            )?;
            Ok(())
        })
    }

    pub fn update_import_mapping(
        &self,
        id: Uuid,
        target_table_id: &str,
        mapping: &Value,
    ) -> Result<(), RepositoryError> {
        let (id, target, mapping) = (id, target_table_id.to_string(), mapping.clone());
        self.with_client(move |c| {
            c.execute(
                "UPDATE app_imports SET target_table_id = $2, mapping = $3 WHERE id = $1",
                &[&id, &target, &mapping],
            )?;
            Ok(())
        })
    }

    pub fn update_import_dry_run(
        &self,
        id: Uuid,
        status: &str,
        dry_run_result: &Value,
    ) -> Result<(), RepositoryError> {
        let (id, status, dry_run_result) = (id, status.to_string(), dry_run_result.clone());
        self.with_client(move |c| {
            c.execute(
                "UPDATE app_imports SET status = $2, dry_run_result = $3, dry_run_at = NOW() WHERE id = $1",
                &[&id, &status, &dry_run_result],
            )?;
            Ok(())
        })
    }

    pub fn update_import_checkpoint(
        &self,
        id: Uuid,
        checkpoint: &Value,
    ) -> Result<(), RepositoryError> {
        let (id, checkpoint) = (id, checkpoint.clone());
        self.with_client(move |c| {
            c.execute(
                "UPDATE app_imports SET checkpoint = $2 WHERE id = $1",
                &[&id, &checkpoint],
            )?;
            Ok(())
        })
    }

    pub fn update_import_completed(
        &self,
        id: Uuid,
        status: &str,
        run_result: &Value,
        rejected_file_ref_id: Option<Uuid>,
    ) -> Result<(), RepositoryError> {
        let (id, status, run_result) = (id, status.to_string(), run_result.clone());
        self.with_client(move |c| {
            c.execute(
                "UPDATE app_imports SET status = $2, run_result = $3, rejected_file_ref_id = $4 WHERE id = $1",
                &[&id, &status, &run_result, &rejected_file_ref_id],
            )?;
            Ok(())
        })
    }
}
