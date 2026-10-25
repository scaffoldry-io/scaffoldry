//! Undo of a batch, in one transaction. Either every record of the batch is put back, or nothing is
//! written and the records that have moved on are listed.

use crate::repository::{
    delete_record_in_tx, restore_record_in_tx, update_record_in_tx, HistoryEntry, PostgresRepository, RepositoryError, WriteCtx,
};
use serde_json::Value;
use std::collections::HashMap;

/// A record the batch cannot be undone for, and the fields that no longer hold what the batch wrote.
#[derive(Debug, Clone, PartialEq)]
pub struct UndoConflict {
    pub record_id: String,
    pub fields: Vec<String>,
}

#[derive(Debug)]
pub enum UndoOutcome {
    Undone { records: usize },
    Conflicts(Vec<UndoConflict>),
}

/// What undoing a batch writes for one table: the context of the write.
pub struct UndoTable {
    pub ctx: WriteCtx,
}

/// Two values are the same when they are equal, and numbers are the same when they are the same number.
fn same(a: Option<&Value>, b: Option<&Value>) -> bool {
    match (a, b) {
        (Some(Value::Number(x)), Some(Value::Number(y))) => x.as_f64() == y.as_f64(),
        (Some(x), Some(y)) => x == y,
        (None, None) | (Some(Value::Null), None) | (None, Some(Value::Null)) => true,
        _ => false,
    }
}

impl PostgresRepository {
    /// The entries of one batch, oldest first.
    pub fn batch_entries(&self, app_slug: &str, batch_id: &str) -> Result<Vec<HistoryEntry>, RepositoryError> {
        let (slug, batch) = (app_slug.to_string(), batch_id.to_string());
        self.with_client(move |client| entries_of(client, &slug, &batch))
    }

    /// Every entry of one record, oldest first.
    pub fn record_entries(&self, app_slug: &str, record_id: &str) -> Result<Vec<HistoryEntry>, RepositoryError> {
        let (slug, id) = (app_slug.to_string(), record_id.to_string());
        self.with_client(move |client| {
            Ok(client
                .query(&format!("{SELECT} WHERE app_slug = $1 AND record_id = $2 ORDER BY id ASC"), &[&slug, &id])?
                .iter()
                .map(entry_of)
                .collect())
        })
    }

    /// Writes the old values of a batch back, as a new batch. `tables` holds the write context of each
    /// table the batch touched.
    pub fn undo_batch(&self, app_slug: &str, batch_id: &str, tables: HashMap<String, UndoTable>) -> Result<UndoOutcome, RepositoryError> {
        let (slug, batch) = (app_slug.to_string(), batch_id.to_string());
        self.with_client(move |client| {
            let mut tx = client.transaction()?;
            let entries: Vec<HistoryEntry> = tx
                .query(&format!("{SELECT} WHERE app_slug = $1 AND batch_id = $2::text::uuid ORDER BY id ASC"), &[&slug, &batch])?
                .iter()
                .map(entry_of)
                .collect();
            let mut conflicts: Vec<UndoConflict> = Vec::new();
            let mut written = 0usize;
            let mut restored: Vec<(String, String)> = Vec::new();
            // The last change is undone first.
            for entry in entries.iter().rev() {
                let Some(table) = tables.get(&entry.table_id) else { continue };
                let current: Option<Value> = tx
                    .query_opt("SELECT data FROM dataset_records WHERE app_slug = $1 AND id = $2 AND deleted_at IS NULL FOR UPDATE", &[&slug, &entry.record_id])?
                    .map(|r| r.get(0));
                let changes = entry.changes.as_object().cloned().unwrap_or_default();
                let moved_on = |data: &Value| -> Vec<String> {
                    changes
                        .iter()
                        .filter(|(field, change)| !same(data.get(field.as_str()), change.get("new")))
                        .map(|(field, _)| field.clone())
                        .collect()
                };
                match (entry.action.as_str(), current) {
                    ("comment", _) => {}
                    ("delete", Some(_)) => conflicts.push(UndoConflict { record_id: entry.record_id.clone(), fields: Vec::new() }),
                    ("delete", None) => {
                        // The record is in trash: bring it back, with its links once the batch is live again.
                        let trashed = tx
                            .query_opt(
                                "SELECT 1 FROM dataset_records WHERE app_slug = $1 AND id = $2 AND deleted_at IS NOT NULL FOR UPDATE",
                                &[&slug, &entry.record_id],
                            )?
                            .is_some();
                        if trashed && restore_record_in_tx(&mut tx, &slug, &entry.record_id, &table.ctx)? {
                            restored.push((entry.record_id.clone(), entry.table_id.clone()));
                            written += 1;
                        } else {
                            conflicts.push(UndoConflict { record_id: entry.record_id.clone(), fields: Vec::new() });
                        }
                    }
                    (_, None) => conflicts.push(UndoConflict { record_id: entry.record_id.clone(), fields: Vec::new() }),
                    ("create", Some(data)) => {
                        let fields = moved_on(&data);
                        if fields.is_empty() {
                            delete_record_in_tx(&mut tx, &slug, &entry.record_id, None, &table.ctx)?;
                            written += 1;
                        } else {
                            conflicts.push(UndoConflict { record_id: entry.record_id.clone(), fields });
                        }
                    }
                    (_, Some(data)) => {
                        let fields = moved_on(&data);
                        if !fields.is_empty() {
                            conflicts.push(UndoConflict { record_id: entry.record_id.clone(), fields });
                            continue;
                        }
                        let mut next = data.clone();
                        if let Some(map) = next.as_object_mut() {
                            for (field, change) in &changes {
                                match change.get("old") {
                                    Some(old) => map.insert(field.clone(), old.clone()),
                                    None => map.remove(field),
                                };
                            }
                        }
                        update_record_in_tx(&mut tx, &slug, &entry.record_id, &next, None, &table.ctx)?;
                        written += 1;
                    }
                }
            }
            if !conflicts.is_empty() {
                // Dropping the transaction rolls it back, so nothing is written.
                return Ok(UndoOutcome::Conflicts(conflicts));
            }
            for (id, table) in &restored {
                if let Some(t) = tables.get(table) {
                    crate::repository_links::restore_links_in_tx(&mut tx, &slug, id, &t.ctx)?;
                }
            }
            tx.commit()?;
            Ok(UndoOutcome::Undone { records: written })
        })
    }
}

const SELECT: &str = "SELECT id, app_slug, table_id, record_id, version, action, changes, actor, actor_kind, token_label, \
    batch_id::text, reverts_batch::text, proposal_id::text, \
    to_char(at AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS.MS\"Z\"'), acted_by FROM record_history";

fn entries_of(client: &mut postgres::Client, slug: &str, batch: &str) -> Result<Vec<HistoryEntry>, RepositoryError> {
    Ok(client
        .query(&format!("{SELECT} WHERE app_slug = $1 AND batch_id = $2::text::uuid ORDER BY id ASC"), &[&slug, &batch])?
        .iter()
        .map(entry_of)
        .collect())
}

fn entry_of(r: &postgres::Row) -> HistoryEntry {
    HistoryEntry {
        id: r.get(0),
        app_slug: r.get(1),
        table_id: r.get(2),
        record_id: r.get(3),
        version: r.get(4),
        action: r.get(5),
        changes: r.get(6),
        actor: r.get(7),
        actor_kind: r.get(8),
        token_label: r.get(9),
        batch_id: r.get(10),
        reverts_batch: r.get(11),
        proposal_id: r.get(12),
        at: r.get(13),
        acted_by: r.get(14),
    }
}
