//! Data layer phase 1: the stored id of each app table.
//!
//! A manifest write gives each of its tables a dataset id in the same transaction. Migration 0114 did the same for the
//! apps that already existed. The row holds identity only. A dataset that no table backs any longer is not listed.

use crate::repository::{PostgresRepository, RepositoryError};
use scaffoldry_engine::AppManifest;

/// A table that has no dataset gets one. A table that has one keeps it, so two instances saving at once agree.
pub fn ensure_datasets_in_tx(tx: &mut postgres::Transaction<'_>, m: &AppManifest) -> Result<(), RepositoryError> {
    let tables: Vec<String> = m.tables.iter().map(|t| t.id.clone()).collect();
    tx.execute(
        "INSERT INTO datasets (id, app_slug, table_id) \
         SELECT 'ds_' || replace(gen_random_uuid()::text, '-', ''), $1, t FROM unnest($2::text[]) t \
         ON CONFLICT (app_slug, table_id) DO NOTHING",
        &[&m.slug, &tables],
    )?;
    Ok(())
}

impl PostgresRepository {
    /// Every stored dataset as (id, app slug, table id).
    pub fn list_datasets(&self) -> Result<Vec<(String, String, String)>, RepositoryError> {
        self.with_client(|c| {
            Ok(c.query("SELECT id, app_slug, table_id FROM datasets ORDER BY app_slug, table_id", &[])?
                .iter()
                .map(|r| (r.get(0), r.get(1), r.get(2)))
                .collect())
        })
    }

    /// The app slug and table id a dataset id names.
    pub fn dataset_by_id(&self, id: &str) -> Result<Option<(String, String)>, RepositoryError> {
        let id = id.to_string();
        self.with_client(move |c| Ok(c.query_opt("SELECT app_slug, table_id FROM datasets WHERE id = $1", &[&id])?.map(|r| (r.get(0), r.get(1)))))
    }
}
