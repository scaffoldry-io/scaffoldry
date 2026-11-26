//! Data layer phase 2d: the catalog's stored policy, and its search.
//!
//! The catalog is built on the `datasets` table of phase 1. A row there is one catalog entry. A published dataset has a row
//! of its own, marked by `published_id`, so both kinds of dataset have a dataset id. The row holds policy only: who may
//! discover the dataset, which fields are shown as "a restricted field", and the steward position. The name, the fields and
//! the owner stay in the manifest or in `published_datasets`, so there is one source for each.

use crate::repository::{append_ledger_in_tx, AppendDecisionParams, PostgresRepository, RepositoryError};
use serde_json::Value;

/// The `app_slug` of the rows that stand for published datasets. No app can be named this.
pub const PUBLISHED_SLUG: &str = "~published";

/// One dataset's stored policy.
#[derive(Debug, Clone)]
pub struct CatalogRow {
    pub id: String,
    pub app_slug: String,
    pub table_id: String,
    pub published_id: Option<String>,
    pub discoverability: Value,
    pub restricted_fields: Vec<String>,
    pub steward_position: Option<String>,
}

/// A change to the policy. A part left `None` stays as it is. A steward of `Some(None)` clears it.
#[derive(Debug, Clone, Default)]
pub struct PolicyChange {
    pub discoverability: Option<Value>,
    pub restricted_fields: Option<Vec<String>>,
    pub steward_position: Option<Option<String>>,
}

const COLUMNS: &str = "id, app_slug, table_id, published_id, discoverability, restricted_fields, steward_position";

fn row_of(r: &postgres::Row) -> CatalogRow {
    CatalogRow {
        id: r.get(0),
        app_slug: r.get(1),
        table_id: r.get(2),
        published_id: r.get(3),
        discoverability: r.get(4),
        restricted_fields: r.get(5),
        steward_position: r.get(6),
    }
}

/// A published dataset gets a dataset row in the transaction that stores it. A dataset that has one keeps it.
pub fn ensure_published_in_tx(tx: &mut postgres::Transaction<'_>, published_id: &str) -> Result<(), RepositoryError> {
    tx.execute(
        "INSERT INTO datasets (id, app_slug, table_id, published_id) VALUES ('ds_' || replace(gen_random_uuid()::text, '-', ''), $1, $2, $2) \
         ON CONFLICT (app_slug, table_id) DO NOTHING",
        &[&PUBLISHED_SLUG, &published_id],
    )?;
    Ok(())
}

impl PostgresRepository {
    /// Every dataset with its policy, in a fixed order.
    pub fn catalog_rows(&self) -> Result<Vec<CatalogRow>, RepositoryError> {
        self.with_client(|c| Ok(c.query(&format!("SELECT {COLUMNS} FROM datasets ORDER BY app_slug, table_id"), &[])?.iter().map(row_of).collect()))
    }

    /// One dataset, named by its dataset id or by the id of the published dataset it stands for.
    pub fn catalog_row(&self, id: &str) -> Result<Option<CatalogRow>, RepositoryError> {
        let id = id.to_string();
        self.with_client(move |c| Ok(c.query_opt(&format!("SELECT {COLUMNS} FROM datasets WHERE id = $1 OR published_id = $1"), &[&id])?.as_ref().map(row_of)))
    }

    /// The ids whose text matches every pattern, case-insensitively. A pattern is a `LIKE` pattern made by `like_pattern`.
    /// The text is catalog metadata, never a row.
    pub fn catalog_match(&self, ids: Vec<String>, haystacks: Vec<String>, patterns: Vec<String>) -> Result<Vec<String>, RepositoryError> {
        self.with_client(move |c| {
            Ok(c.query("SELECT t.id FROM unnest($1::text[], $2::text[]) AS t(id, hay) WHERE t.hay ILIKE ALL ($3::text[])", &[&ids, &haystacks, &patterns])?
                .iter()
                .map(|r| r.get(0))
                .collect())
        })
    }

    /// The live size and the freshness of a table.
    pub fn catalog_footprint(&self, app_slug: &str, table_ids: Vec<String>) -> Result<scaffoldry_data::catalog::Footprint, RepositoryError> {
        let app = app_slug.to_string();
        self.with_client(move |c| Ok(scaffoldry_data::catalog::footprint(c, &app, &table_ids)?))
    }

    /// Changes a dataset's policy and appends the ledger entry in the same transaction. `None` when there is no such dataset.
    /// Otherwise the row as it is now. The ledger closure is given the row as it was.
    pub fn set_catalog_policy(
        &self,
        id: &str,
        change: PolicyChange,
        ledger: impl FnOnce(&CatalogRow, &CatalogRow) -> AppendDecisionParams + Send + 'static,
    ) -> Result<Option<CatalogRow>, RepositoryError> {
        let id = id.to_string();
        self.with_client(move |c| {
            let mut tx = c.transaction()?;
            let Some(was) = tx.query_opt(&format!("SELECT {COLUMNS} FROM datasets WHERE id = $1 OR published_id = $1 FOR UPDATE"), &[&id])?.as_ref().map(row_of) else {
                return Ok(None);
            };
            let mut now = was.clone();
            if let Some(d) = change.discoverability {
                now.discoverability = d;
            }
            if let Some(f) = change.restricted_fields {
                now.restricted_fields = f;
            }
            if let Some(s) = change.steward_position {
                now.steward_position = s;
            }
            tx.execute(
                "UPDATE datasets SET discoverability = $2, restricted_fields = $3, steward_position = $4 WHERE id = $1",
                &[&now.id, &now.discoverability, &now.restricted_fields, &now.steward_position],
            )?;
            append_ledger_in_tx(&mut tx, ledger(&was, &now))?;
            tx.commit()?;
            Ok(Some(now))
        })
    }
}
