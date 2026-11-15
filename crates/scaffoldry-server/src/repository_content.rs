//! Sensitive content phase 2: findings, authorized stores, and the label rows a decision writes.
//!
//! A decision, the label it raises, the records' flags, the cache counter and the ledger entry commit together.

use crate::repository::{append_ledger_in_tx, reflag_table, AppendDecisionParams, PostgresRepository, RepositoryError};
use crate::repository_labels::FlagTable;
use serde_json::{json, Value};
use uuid::Uuid;

/// One place a category may live.
#[derive(Debug, Clone)]
pub struct StoreSpec {
    pub category: String,
    pub scope: String,
    pub app_slug: String,
    pub table_id: String,
    pub field: String,
    pub workspace_id: String,
}

/// A field that a decision or a designation puts a category on.
#[derive(Debug, Clone)]
pub struct CategoryLabel {
    pub app_slug: String,
    pub table_id: String,
    pub field: String,
}

/// How a decision came out.
pub enum Decided {
    Done,
    Missing,
    /// The finding was decided already. The text is its status.
    Already(String),
}

/// Filters of the findings list.
#[derive(Debug, Default, Clone)]
pub struct FindingFilter {
    pub status: Option<String>,
    pub category: Option<String>,
    pub app_slug: Option<String>,
}

const FINDING_COLUMNS: &str = "id, app_slug, table_id, field, detector_id, category, source, match_count, record_count, sample_record_ids, status, decided_by, \
    to_char(decided_at AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS\"Z\"'), reason, \
    to_char(first_seen AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS\"Z\"'), to_char(last_seen AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS\"Z\"')";

fn finding_json(r: &postgres::Row) -> Value {
    json!({
        "id": r.get::<_, Uuid>(0), "app_slug": r.get::<_, String>(1), "table_id": r.get::<_, String>(2), "field": r.get::<_, String>(3),
        "detector_id": r.get::<_, String>(4), "category": r.get::<_, String>(5), "source": r.get::<_, String>(6),
        "match_count": r.get::<_, i64>(7), "record_count": r.get::<_, i64>(8), "sample_record_ids": r.get::<_, Vec<String>>(9),
        "status": r.get::<_, String>(10), "decided_by": r.get::<_, Option<String>>(11), "decided_at": r.get::<_, Option<String>>(12),
        "reason": r.get::<_, Option<String>>(13), "first_seen": r.get::<_, String>(14), "last_seen": r.get::<_, String>(15),
    })
}

/// Puts a category on a field's label. An existing label keeps its flag and gains the category.
fn label_with_category(tx: &mut postgres::Transaction<'_>, l: &CategoryLabel, category: &str, note: &str, by: &str) -> Result<(), postgres::Error> {
    tx.execute(
        "INSERT INTO data_labels (app_slug, table_id, field, ferpa_sensitive, note, set_by, categories) VALUES ($1, $2, $3, false, $4, $5, ARRAY[$6]::text[]) \
         ON CONFLICT (app_slug, table_id, field) DO UPDATE SET \
            categories = (SELECT COALESCE(array_agg(DISTINCT x), '{}') FROM unnest(data_labels.categories || ARRAY[$6]::text[]) AS x), \
            note = $4, set_by = $5, set_at = NOW()",
        &[&l.app_slug, &l.table_id, &l.field, &note, &by, &category],
    )?;
    Ok(())
}

impl PostgresRepository {
    /// A page of findings, newest seen first, and how many there are in all.
    pub fn list_findings(&self, f: FindingFilter, offset: i64, limit: i64) -> Result<(Vec<Value>, i64), RepositoryError> {
        self.with_client(move |c| {
            let rows = c.query(
                &format!(
                    "SELECT {FINDING_COLUMNS} FROM content_findings WHERE ($1::text IS NULL OR status = $1) AND ($2::text IS NULL OR category = $2) AND ($3::text IS NULL OR app_slug = $3) \
                     ORDER BY last_seen DESC, id OFFSET $4 LIMIT $5"
                ),
                &[&f.status, &f.category, &f.app_slug, &offset, &limit],
            )?;
            let total: i64 = c
                .query_one(
                    "SELECT count(*) FROM content_findings WHERE ($1::text IS NULL OR status = $1) AND ($2::text IS NULL OR category = $2) AND ($3::text IS NULL OR app_slug = $3)",
                    &[&f.status, &f.category, &f.app_slug],
                )?
                .get(0);
            Ok((rows.iter().map(finding_json).collect(), total))
        })
    }

    /// One finding, as the list shows it.
    pub fn get_finding(&self, id: Uuid) -> Result<Option<Value>, RepositoryError> {
        self.with_client(move |c| Ok(c.query_opt(&format!("SELECT {FINDING_COLUMNS} FROM content_findings WHERE id = $1"), &[&id])?.map(|r| finding_json(&r))))
    }

    /// Decides an open finding. A confirm puts the category on the field's label and sets the stored flags of its table again, in the same
    /// transaction. A finding decided already is left alone.
    #[allow(clippy::too_many_arguments)]
    pub fn decide_finding(&self, id: Uuid, confirm: bool, by: &str, reason: &str, label: Option<(CategoryLabel, String)>, flags: Vec<FlagTable>, ledger: AppendDecisionParams) -> Result<Decided, RepositoryError> {
        let (by, reason) = (by.to_string(), reason.to_string());
        self.with_client(move |c| {
            let mut tx = c.transaction()?;
            let Some(status) = tx.query_opt("SELECT status FROM content_findings WHERE id = $1 FOR UPDATE", &[&id])?.map(|r| r.get::<_, String>(0)) else { return Ok(Decided::Missing) };
            if status != "open" {
                return Ok(Decided::Already(status));
            }
            let new_status = if confirm { "confirmed" } else { "dismissed" };
            tx.execute("UPDATE content_findings SET status = $2, decided_by = $3, decided_at = NOW(), reason = $4 WHERE id = $1", &[&id, &new_status, &by, &reason])?;
            if let Some((l, category)) = &label {
                label_with_category(&mut tx, l, category, &reason, &by)?;
                for t in &flags {
                    reflag_table(&mut tx, &t.app_slug, &t.table_id, &t.names)?;
                }
                crate::cache::bump(&mut tx, "labels")?;
            }
            append_ledger_in_tx(&mut tx, ledger)?;
            tx.commit()?;
            Ok(Decided::Done)
        })
    }

    /// Designates a store and labels the fields it covers with the category, at once.
    pub fn put_store(&self, store: StoreSpec, labels: Vec<CategoryLabel>, flags: Vec<FlagTable>, reason: &str, by: &str, ledger: AppendDecisionParams) -> Result<(), RepositoryError> {
        let (reason, by) = (reason.to_string(), by.to_string());
        self.with_client(move |c| {
            let mut tx = c.transaction()?;
            tx.execute(
                "INSERT INTO authorized_stores (category, scope, app_slug, table_id, field, workspace_id, reason, set_by) VALUES ($1, $2, $3, $4, $5, $6, $7, $8) \
                 ON CONFLICT (category, scope, workspace_id, app_slug, table_id, field) DO UPDATE SET reason = $7, set_by = $8, set_at = NOW()",
                &[&store.category, &store.scope, &store.app_slug, &store.table_id, &store.field, &store.workspace_id, &reason, &by],
            )?;
            for l in &labels {
                label_with_category(&mut tx, l, &store.category, &reason, &by)?;
            }
            for t in &flags {
                reflag_table(&mut tx, &t.app_slug, &t.table_id, &t.names)?;
            }
            crate::cache::bump(&mut tx, "labels")?;
            append_ledger_in_tx(&mut tx, ledger)?;
            tx.commit()?;
            Ok(())
        })
    }

    /// Removes a designation. The labels it made stay. Returns whether there was one.
    pub fn delete_store(&self, store: StoreSpec, ledger: AppendDecisionParams) -> Result<bool, RepositoryError> {
        self.with_client(move |c| {
            let mut tx = c.transaction()?;
            let n = tx.execute(
                "DELETE FROM authorized_stores WHERE category = $1 AND scope = $2 AND app_slug = $3 AND table_id = $4 AND field = $5 AND workspace_id = $6",
                &[&store.category, &store.scope, &store.app_slug, &store.table_id, &store.field, &store.workspace_id],
            )?;
            if n > 0 {
                append_ledger_in_tx(&mut tx, ledger)?;
            }
            tx.commit()?;
            Ok(n > 0)
        })
    }

    /// Every designation, by category.
    pub fn list_stores(&self) -> Result<Vec<Value>, RepositoryError> {
        self.with_client(|c| {
            Ok(c.query(
                "SELECT category, scope, app_slug, table_id, field, workspace_id, reason, set_by, to_char(set_at AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS\"Z\"') \
                 FROM authorized_stores ORDER BY category, scope, workspace_id, app_slug, table_id, field",
                &[],
            )?
            .iter()
            .map(|r| {
                json!({
                    "category": r.get::<_, String>(0), "scope": r.get::<_, String>(1), "app_slug": r.get::<_, String>(2), "table_id": r.get::<_, String>(3),
                    "field": r.get::<_, String>(4), "workspace_id": r.get::<_, String>(5), "reason": r.get::<_, String>(6), "set_by": r.get::<_, String>(7), "set_at": r.get::<_, String>(8),
                })
            })
            .collect())
        })
    }

    /// For each detector: how many findings, how many confirmed and dismissed, and the dismissed share of those decided.
    pub fn detector_stats(&self) -> Result<Vec<Value>, RepositoryError> {
        self.with_client(|c| {
            Ok(c.query(
                "SELECT detector_id, count(*), count(*) FILTER (WHERE status = 'confirmed'), count(*) FILTER (WHERE status = 'dismissed') \
                 FROM content_findings GROUP BY detector_id ORDER BY detector_id",
                &[],
            )?
            .iter()
            .map(|r| {
                let (confirmed, dismissed) = (r.get::<_, i64>(2), r.get::<_, i64>(3));
                let decided = confirmed + dismissed;
                json!({
                    "detector_id": r.get::<_, String>(0), "findings": r.get::<_, i64>(1), "confirmed": confirmed, "dismissed": dismissed,
                    "dismissed_share": if decided == 0 { 0.0 } else { dismissed as f64 / decided as f64 },
                })
            })
            .collect())
        })
    }

    /// Open findings, and the matches the scans expected inside authorized stores. For the Overview.
    pub fn content_counts(&self) -> Result<(i64, i64), RepositoryError> {
        self.with_client(|c| {
            let open: i64 = c.query_one("SELECT count(*) FROM content_findings WHERE status = 'open'", &[])?.get(0);
            let expected: i64 = c.query_one("SELECT COALESCE(sum(matches), 0)::bigint FROM content_expected", &[])?.get(0);
            Ok((open, expected))
        })
    }

    /// A changed detector's dismissed findings are open again, because the reason they were dismissed may no longer hold.
    pub fn reopen_dismissed(&self, detectors: &[String]) -> Result<u64, RepositoryError> {
        let ids = detectors.to_vec();
        self.with_client(move |c| Ok(c.execute("UPDATE content_findings SET status = 'open', decided_by = NULL, decided_at = NULL, reason = NULL WHERE status = 'dismissed' AND detector_id = ANY($1)", &[&ids])?))
    }
}
