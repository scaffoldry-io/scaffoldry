//! Admin console phase 4: the label table and the classification of published datasets.
//!
//! A label change, the flags it moves on records, the cache counter and the ledger entry commit together, so a server that reads
//! the counter after the commit sees all of it, and a failed write leaves none of it.

use crate::repository::{append_ledger_in_tx, AppendDecisionParams, PostgresRepository, RepositoryError};
use scaffoldry_core::{LedgerEntry, PublishedDataset};
use scaffoldry_engine::{DataLabel, LabelSet};

/// One change to one field's label. `None` removes the label.
#[derive(Debug, Clone)]
pub struct LabelChange {
    pub app_slug: String,
    pub table_id: String,
    pub field: String,
    pub sensitive: Option<bool>,
}

/// The effective sensitive fields of one table after a change, so its records' stored flags can be set again.
#[derive(Debug, Clone)]
pub struct FlagTable {
    pub app_slug: String,
    pub table_id: String,
    pub names: Vec<String>,
}

impl PostgresRepository {
    /// Every label. The set holds exceptions only.
    pub fn list_labels(&self) -> Result<LabelSet, RepositoryError> {
        self.with_client(|c| {
            Ok(c.query("SELECT app_slug, table_id, field, ferpa_sensitive, note, set_by FROM data_labels", &[])?
                .iter()
                .map(|r| ((r.get::<_, String>(0), r.get::<_, String>(1), r.get::<_, String>(2)), DataLabel { ferpa_sensitive: r.get(3), note: r.get(4), set_by: r.get(5) }))
                .collect())
        })
    }

    /// Writes the label changes, sets again the stored flag of every record of the tables they touch, raises the `labels`
    /// counter and appends the ledger entry, in one transaction.
    pub fn apply_labels(&self, changes: Vec<LabelChange>, note: &str, by: &str, flags: Vec<FlagTable>, ledger: AppendDecisionParams) -> Result<LedgerEntry, RepositoryError> {
        let (note, by) = (note.to_string(), by.to_string());
        self.with_client(move |c| {
            let mut tx = c.transaction()?;
            for ch in &changes {
                match ch.sensitive {
                    Some(v) => tx.execute(
                        "INSERT INTO data_labels (app_slug, table_id, field, ferpa_sensitive, note, set_by) VALUES ($1, $2, $3, $4, $5, $6) \
                         ON CONFLICT (app_slug, table_id, field) DO UPDATE SET (ferpa_sensitive, note, set_by, set_at) = ($4, $5, $6, NOW())",
                        &[&ch.app_slug, &ch.table_id, &ch.field, &v, &note, &by],
                    )?,
                    None => tx.execute("DELETE FROM data_labels WHERE app_slug = $1 AND table_id = $2 AND field = $3", &[&ch.app_slug, &ch.table_id, &ch.field])?,
                };
            }
            // A record is flagged when it holds a value in any field that is now sensitive. The same test the backfill of a category uses.
            for t in &flags {
                tx.execute(
                    "UPDATE dataset_records SET is_ferpa_sensitive = EXISTS ( \
                         SELECT 1 FROM unnest($3::text[]) AS f(name) \
                         WHERE data->f.name IS NOT NULL AND data->f.name NOT IN ('null'::jsonb, '\"\"'::jsonb, '[]'::jsonb)) \
                     WHERE app_slug = $1 AND table_id = $2",
                    &[&t.app_slug, &t.table_id, &t.names],
                )?;
            }
            crate::cache::bump(&mut tx, "labels")?;
            let entry = append_ledger_in_tx(&mut tx, ledger)?;
            tx.commit()?;
            Ok(entry)
        })
    }

    /// Sets a published dataset's sensitivity level, in the column and in the stored payload together, and appends the ledger
    /// entry. `None` when there is no such dataset. Returns the level it had.
    pub fn classify_dataset(&self, id: &str, level: &str, ledger: impl FnOnce(&str) -> AppendDecisionParams + Send + 'static) -> Result<Option<String>, RepositoryError> {
        let (id, level) = (id.to_string(), level.to_string());
        self.with_client(move |c| {
            let mut tx = c.transaction()?;
            let Some(was) = tx.query_opt("SELECT sensitivity_level FROM published_datasets WHERE id = $1 FOR UPDATE", &[&id])?.map(|r| r.get::<_, String>(0)) else { return Ok(None) };
            tx.execute(
                "UPDATE published_datasets SET sensitivity_level = $2, payload = jsonb_set(payload, '{sensitivity_level}', to_jsonb($3::text)), updated_at = NOW() WHERE id = $1",
                &[&id, &level, &level],
            )?;
            crate::cache::bump(&mut tx, "datasets")?;
            append_ledger_in_tx(&mut tx, ledger(&was))?;
            tx.commit()?;
            Ok(Some(was))
        })
    }

    /// The published datasets, oldest first.
    pub fn published_datasets(&self) -> Result<Vec<PublishedDataset>, RepositoryError> {
        self.list_published_datasets()
    }
}
