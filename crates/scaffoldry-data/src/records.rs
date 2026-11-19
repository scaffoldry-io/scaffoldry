//! Reads and writes of `dataset_records`.

use postgres::GenericClient;
use serde_json::Value;

/// The next page of a table after a cursor. The row comparison walks the paging index.
pub const PAGE_SQL: &str = "SELECT id, app_slug, data, ceds_mapping, is_ferpa_sensitive, created_at::text AS created_at_text, created_by, version \
     FROM dataset_records \
     WHERE app_slug = $1 AND table_id = $2 AND deleted_at IS NULL AND (created_at, id) > ($3::text::timestamptz, $4::text) \
     ORDER BY created_at, id LIMIT $5";

/// The first page of a table.
pub const FIRST_PAGE_SQL: &str = "SELECT id, app_slug, data, ceds_mapping, is_ferpa_sensitive, created_at::text AS created_at_text, created_by, version \
     FROM dataset_records \
     WHERE app_slug = $1 AND table_id = $2 AND deleted_at IS NULL \
     ORDER BY created_at, id LIMIT $3";

/// A record to store. `table_id` is the table the data names.
pub struct NewRecord<'a> {
    pub id: &'a str,
    pub app_slug: &'a str,
    pub data: &'a Value,
    pub ceds_mapping: &'a Value,
    pub is_ferpa_sensitive: bool,
    pub created_by: Option<&'a str>,
    pub table_id: &'a str,
}

/// A record soft-deleted by `soft_delete`, as it was.
pub struct Deleted {
    pub data: Value,
    pub version: i32,
    pub table_id: String,
}

/// Replaces a record's JSON without raising its version.
pub fn set_data<C: GenericClient>(tx: &mut C, app_slug: &str, id: &str, data: &Value) -> Result<(), postgres::Error> {
    tx.execute("UPDATE dataset_records SET data = $3 WHERE app_slug = $1 AND id = $2", &[&app_slug, &id, data])?;
    Ok(())
}

/// Inserts a new record at version 1.
pub fn insert<C: GenericClient>(tx: &mut C, r: &NewRecord<'_>) -> Result<(), postgres::Error> {
    tx.execute(
        "INSERT INTO dataset_records (id, app_slug, data, ceds_mapping, is_ferpa_sensitive, created_by, table_id) \
         VALUES ($1, $2, $3, $4, $5, $6, $7)",
        &[&r.id, &r.app_slug, &r.data, &r.ceds_mapping, &r.is_ferpa_sensitive, &r.created_by, &r.table_id],
    )?;
    Ok(())
}

/// Reads a live record's JSON before an upsert, locking the row. The id alone names it.
pub fn data_for_update<C: GenericClient>(tx: &mut C, id: &str) -> Result<Option<Value>, postgres::Error> {
    Ok(tx.query_opt("SELECT data FROM dataset_records WHERE id = $1 FOR UPDATE", &[&id])?.map(|r| r.get(0)))
}

/// Inserts a record, or replaces the one with its id and raises its version. Returns the version.
pub fn upsert<C: GenericClient>(tx: &mut C, r: &NewRecord<'_>) -> Result<i32, postgres::Error> {
    Ok(tx
        .query_one(
            "INSERT INTO dataset_records (id, app_slug, data, ceds_mapping, is_ferpa_sensitive, created_by, table_id) \
             VALUES ($1, $2, $3, $4, $5, $6, $7) \
             ON CONFLICT (id) DO UPDATE SET \
                data = EXCLUDED.data, \
                ceds_mapping = EXCLUDED.ceds_mapping, \
                is_ferpa_sensitive = EXCLUDED.is_ferpa_sensitive, \
                table_id = EXCLUDED.table_id, \
                version = dataset_records.version + 1 \
             RETURNING version",
            &[&r.id, &r.app_slug, &r.data, &r.ceds_mapping, &r.is_ferpa_sensitive, &r.created_by, &r.table_id],
        )?
        .get(0))
}

/// A live record's JSON and version, locked for update.
pub fn live_for_update<C: GenericClient>(tx: &mut C, app_slug: &str, id: &str) -> Result<Option<(Value, i32)>, postgres::Error> {
    Ok(tx
        .query_opt(
            "SELECT data, version FROM dataset_records WHERE app_slug = $1 AND id = $2 AND deleted_at IS NULL FOR UPDATE",
            &[&app_slug, &id],
        )?
        .map(|r| (r.get(0), r.get(1))))
}

/// Saves new JSON and the table it names, and raises the version. Returns the new version.
pub fn save_data<C: GenericClient>(tx: &mut C, app_slug: &str, id: &str, data: &Value, table_id: &str) -> Result<i32, postgres::Error> {
    Ok(tx
        .query_one(
            "UPDATE dataset_records SET data = $1, table_id = $2, version = version + 1 \
             WHERE app_slug = $3 AND id = $4 RETURNING version",
            &[&data, &table_id, &app_slug, &id],
        )?
        .get(0))
}

/// The version of a live record.
pub fn live_version<C: GenericClient>(tx: &mut C, app_slug: &str, id: &str) -> Result<Option<i32>, postgres::Error> {
    Ok(tx
        .query_opt("SELECT version FROM dataset_records WHERE app_slug = $1 AND id = $2 AND deleted_at IS NULL", &[&app_slug, &id])?
        .map(|r| r.get(0)))
}

/// Moves a live record to trash. With `expected_version`, a record at another version is left alone.
/// `None` when nothing moved.
pub fn soft_delete<C: GenericClient>(
    tx: &mut C,
    app_slug: &str,
    id: &str,
    expected_version: Option<i32>,
    actor: &str,
    batch: Option<uuid::Uuid>,
) -> Result<Option<Deleted>, postgres::Error> {
    let row = match expected_version {
        Some(expected) => tx.query_opt(
            "UPDATE dataset_records SET deleted_at = NOW(), deleted_by = $4, delete_batch = $5 \
             WHERE app_slug = $1 AND id = $2 AND version = $3 AND deleted_at IS NULL RETURNING data, version, table_id",
            &[&app_slug, &id, &expected, &actor, &batch],
        )?,
        None => tx.query_opt(
            "UPDATE dataset_records SET deleted_at = NOW(), deleted_by = $3, delete_batch = $4 \
             WHERE app_slug = $1 AND id = $2 AND deleted_at IS NULL RETURNING data, version, table_id",
            &[&app_slug, &id, &actor, &batch],
        )?,
    };
    Ok(row.map(|r| Deleted { data: r.get(0), version: r.get(1), table_id: r.get(2) }))
}

/// Brings a record out of trash. `None` when it was not in trash.
pub fn restore<C: GenericClient>(tx: &mut C, app_slug: &str, id: &str) -> Result<Option<Deleted>, postgres::Error> {
    let row = tx.query_opt(
        "UPDATE dataset_records SET deleted_at = NULL, deleted_by = NULL, delete_batch = NULL \
         WHERE app_slug = $1 AND id = $2 AND deleted_at IS NOT NULL RETURNING data, version, table_id",
        &[&app_slug, &id],
    )?;
    Ok(row.map(|r| Deleted { data: r.get(0), version: r.get(1), table_id: r.get(2) }))
}
