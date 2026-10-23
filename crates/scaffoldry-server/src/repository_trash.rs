//! The trash: records that were deleted and not yet destroyed.

use crate::repository::{record_from_row, restore_record_in_tx, PostgresRepository, RepositoryError, TrashRow, WriteCtx};
use std::collections::HashMap;

const COLUMNS: &str = "id, app_slug, data, ceds_mapping, is_ferpa_sensitive, created_at::text AS created_at_text, created_by, version, \
    deleted_by, to_char(deleted_at AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS.MS\"Z\"'), delete_batch, \
    GREATEST(0, $1::bigint - floor(extract(epoch from (NOW() - deleted_at)) / 86400))::bigint";

fn trash_row(r: &postgres::Row) -> TrashRow {
    TrashRow {
        record: record_from_row(r),
        deleted_by: r.get::<_, Option<String>>(8).unwrap_or_default(),
        deleted_at: r.get(9),
        delete_batch: r.get(10),
        days_left: r.get(11),
    }
}

impl PostgresRepository {
    /// A page of the trash, newest first. `only_by` limits it to what one person deleted. `before` is the
    /// `(deleted_at, id)` of the last entry of the previous page.
    pub fn trash_page(
        &self,
        app_slug: &str,
        only_by: Option<&str>,
        keep_days: i64,
        before: Option<(String, String)>,
        limit: i64,
    ) -> Result<Vec<TrashRow>, RepositoryError> {
        let (slug, only) = (app_slug.to_string(), only_by.map(str::to_string));
        let (at, id) = before.unzip();
        self.with_client(move |client| {
            Ok(client
                .query(
                    &format!(
                        "SELECT {COLUMNS} FROM dataset_records WHERE app_slug = $2 AND deleted_at IS NOT NULL \
                           AND ($3::text IS NULL OR deleted_by = $3) \
                           AND ($4::text IS NULL OR (deleted_at, id) < ($4::text::timestamptz, $5::text)) \
                         ORDER BY deleted_at DESC, id DESC LIMIT $6"
                    ),
                    &[&keep_days, &slug, &only, &at, &id, &limit],
                )?
                .iter()
                .map(trash_row)
                .collect())
        })
    }

    /// The trashed records named by `selector`: one record id, or the id of a batch deleted together.
    pub fn trashed_by_selector(&self, app_slug: &str, selector: &str, keep_days: i64) -> Result<Vec<TrashRow>, RepositoryError> {
        let (slug, selector) = (app_slug.to_string(), selector.to_string());
        self.with_client(move |client| {
            Ok(client
                .query(
                    &format!(
                        "SELECT {COLUMNS} FROM dataset_records WHERE app_slug = $2 AND deleted_at IS NOT NULL \
                           AND (id = $3 OR delete_batch::text = $3) ORDER BY id"
                    ),
                    &[&keep_days, &slug, &selector],
                )?
                .iter()
                .map(trash_row)
                .collect())
        })
    }

    /// Brings the trashed records named by `selector` back, with their links, in one transaction. The
    /// context of each table says how its records are indexed. Returns the ids restored.
    pub fn restore_trash(&self, app_slug: &str, selector: &str, tables: HashMap<String, WriteCtx>) -> Result<Vec<String>, RepositoryError> {
        let (slug, selector) = (app_slug.to_string(), selector.to_string());
        self.with_client(move |client| {
            let mut tx = client.transaction()?;
            let rows = tx.query(
                "SELECT id, table_id FROM dataset_records WHERE app_slug = $1 AND deleted_at IS NOT NULL \
                   AND (id = $2 OR delete_batch::text = $2) ORDER BY id FOR UPDATE",
                &[&slug, &selector],
            )?;
            let mut restored: Vec<(String, String)> = Vec::new();
            // First every record is live again, then the links between them come back.
            for row in &rows {
                let (id, table): (String, String) = (row.get(0), row.get(1));
                let Some(ctx) = tables.get(&table) else { continue };
                if restore_record_in_tx(&mut tx, &slug, &id, ctx)? {
                    restored.push((id, table));
                }
            }
            for (id, table) in &restored {
                if let Some(ctx) = tables.get(table) {
                    crate::repository_links::restore_links_in_tx(&mut tx, &slug, id, ctx)?;
                }
            }
            tx.commit()?;
            Ok(restored.into_iter().map(|(id, _)| id).collect())
        })
    }

    /// Records by id, live or in trash. For a check of what the reader could see.
    pub fn records_with_trashed(&self, app_slug: &str, ids: &[String]) -> Result<Vec<crate::state::DatasetRecord>, RepositoryError> {
        let app_slug = scaffoldry_engine::stored_slug(app_slug);
        let (slug, ids) = (app_slug.to_string(), ids.to_vec());
        self.with_client(move |client| {
            let rows = client.query(
                "SELECT id, app_slug, data, ceds_mapping, is_ferpa_sensitive, created_at::text AS created_at_text, created_by, version \
                 FROM dataset_records WHERE app_slug = $1 AND id = ANY($2)",
                &[&slug, &ids],
            )?;
            Ok(rows.iter().map(crate::repository::record_from_row).collect())
        })
    }
}
