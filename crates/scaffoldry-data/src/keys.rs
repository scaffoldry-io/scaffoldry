//! A dataset's declared key, and `record_keys`, the table that makes it unique.
//!
//! The key is one or more fields of a table. A row's key text is those values, each trimmed, joined with a separator,
//! and lower-cased when the key says to fold case. A row whose key fields are not all filled has no key text, and holds
//! no key. `record_keys (app_slug, table_id, key_text, record_id)` has a unique constraint on the first three, so two
//! live rows of a table cannot share a key text. A trashed row holds no key.
//!
//! The declared key is kept as JSON on the dataset's row (`datasets.key_spec`), written in the same transaction as the
//! manifest that declares it. Every write reads it in its own transaction, so a write through any door, whether a
//! request, an import or a sync, is checked the same way.
//!
//! The refusal carries no row. A taken key is a unique violation of `uq_record_keys` and nothing more: the caller turns
//! it into [`TAKEN`], and never reads the table to say who holds the key.

use postgres::error::SqlState;
use postgres::GenericClient;
use serde_json::Value;
use std::collections::{BTreeSet, HashMap};

/// What a writer is told when a key is taken. It says nothing about the row that holds it.
pub const TAKEN: &str = "this key is already used";

/// The unique constraint that refuses a second row with a key.
pub const UNIQUE: &str = "uq_record_keys";

const SEPARATOR: char = '\u{1f}';

/// A declared key: the fields, and whether case is folded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeySpec {
    pub fields: Vec<String>,
    pub case_fold: bool,
}

impl KeySpec {
    pub fn to_json(&self) -> Value {
        serde_json::json!({ "fields": self.fields, "case_fold": self.case_fold })
    }

    pub fn from_json(v: &Value) -> Option<Self> {
        let fields: Vec<String> = v.get("fields")?.as_array()?.iter().filter_map(|f| f.as_str().map(str::to_string)).collect();
        (!fields.is_empty()).then(|| Self { fields, case_fold: v.get("case_fold").and_then(Value::as_bool).unwrap_or(false) })
    }
}

fn part(v: &Value, case_fold: bool) -> Option<String> {
    let text = match v {
        Value::Null => return None,
        Value::String(s) => s.trim().to_string(),
        other => other.to_string(),
    };
    if text.is_empty() {
        return None;
    }
    Some(if case_fold { text.to_lowercase() } else { text })
}

/// The key text of a row's data, or `None` when a key field is missing or blank. The same rule everywhere a key is
/// made: a write, a backfill, an import and a sync.
pub fn key_text(data: &Value, spec: &KeySpec) -> Option<String> {
    let parts: Option<Vec<String>> = spec.fields.iter().map(|f| data.get(f).and_then(|v| part(v, spec.case_fold))).collect();
    parts.map(|p| p.join(&SEPARATOR.to_string()))
}

/// True when the error is a second row taking a key.
pub fn is_taken(e: &postgres::Error) -> bool {
    e.as_db_error().is_some_and(|d| d.code() == &SqlState::UNIQUE_VIOLATION && d.constraint() == Some(UNIQUE))
}

/// The key a table declares, as the dataset row holds it.
pub fn spec_for<C: GenericClient>(c: &mut C, app_slug: &str, table_id: &str) -> Result<Option<KeySpec>, postgres::Error> {
    let row = c.query_opt("SELECT key_spec FROM datasets WHERE app_slug = $1 AND table_id = $2", &[&app_slug, &table_id])?;
    Ok(row.and_then(|r| r.get::<_, Option<Value>>(0)).and_then(|v| KeySpec::from_json(&v)))
}

/// Gives a record its key, or takes it away when the data has none. A key held by another record raises the unique
/// violation, which [`is_taken`] names. Nothing is written for a table that declares no key.
pub fn write<C: GenericClient>(tx: &mut C, app_slug: &str, table_id: &str, record_id: &str, data: &Value) -> Result<(), postgres::Error> {
    let spec = spec_for(tx, app_slug, table_id)?;
    write_with(tx, spec.as_ref(), app_slug, table_id, record_id, data)
}

/// [`write`] with the key already read, so a caller that reads both specs pays one round trip (`crate::specs`).
pub fn write_with<C: GenericClient>(tx: &mut C, spec: Option<&KeySpec>, app_slug: &str, table_id: &str, record_id: &str, data: &Value) -> Result<(), postgres::Error> {
    let Some(spec) = spec else { return Ok(()) };
    match key_text(data, spec) {
        Some(text) => {
            tx.execute(
                "INSERT INTO record_keys (app_slug, table_id, key_text, record_id) VALUES ($1, $2, $3, $4) \
                 ON CONFLICT (record_id) DO UPDATE SET app_slug = EXCLUDED.app_slug, table_id = EXCLUDED.table_id, key_text = EXCLUDED.key_text",
                &[&app_slug, &table_id, &text, &record_id],
            )?;
        }
        None => clear(tx, record_id)?,
    }
    Ok(())
}

/// Frees a record's key. A trashed record holds none.
pub fn clear<C: GenericClient>(tx: &mut C, record_id: &str) -> Result<(), postgres::Error> {
    tx.execute("DELETE FROM record_keys WHERE record_id = $1", &[&record_id])?;
    Ok(())
}

/// The record that holds a key text, if one does.
pub fn find<C: GenericClient>(c: &mut C, app_slug: &str, table_id: &str, key_text: &str) -> Result<Option<String>, postgres::Error> {
    Ok(c.query_opt("SELECT record_id FROM record_keys WHERE app_slug = $1 AND table_id = $2 AND key_text = $3", &[&app_slug, &table_id, &key_text])?.map(|r| r.get(0)))
}

/// Rows read at a time when a key is declared over rows that exist.
const PAGE: i64 = 2000;

/// The most record ids a list of duplicates carries. A longer list is cut, and the count says by how much.
pub const MAX_LISTED: usize = 200;

/// Records that share one key text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Duplicates {
    /// Each group is the ids of the live records that share a key, sorted. At most [`MAX_LISTED`] ids in all.
    pub groups: Vec<Vec<String>>,
    /// Ids left out of `groups` because the list was full.
    pub more: usize,
}

/// Sets the key of a table, or takes it away with `None`, and keys every live row to match. The earlier keys of the
/// table are dropped first. When live rows share a key text, nothing is kept in the caller's transaction that matters:
/// the duplicates come back and the caller rolls the transaction back.
pub fn declare<C: GenericClient>(tx: &mut C, app_slug: &str, table_id: &str, spec: Option<&KeySpec>) -> Result<Result<u64, Duplicates>, postgres::Error> {
    let stored = spec.map(KeySpec::to_json);
    tx.execute("UPDATE datasets SET key_spec = $3 WHERE app_slug = $1 AND table_id = $2", &[&app_slug, &table_id, &stored])?;
    tx.execute("DELETE FROM record_keys WHERE app_slug = $1 AND table_id = $2", &[&app_slug, &table_id])?;
    let Some(spec) = spec else { return Ok(Ok(0)) };

    let mut kept = 0u64;
    // A key text that more than one record wanted, and the records that lost it.
    let mut lost: HashMap<String, BTreeSet<String>> = HashMap::new();
    let mut after: Option<(String, String)> = None;
    loop {
        let rows = match &after {
            None => tx.query(crate::records::FIRST_PAGE_SQL, &[&app_slug, &table_id, &PAGE])?,
            Some((at, id)) => tx.query(crate::records::PAGE_SQL, &[&app_slug, &table_id, at, id, &PAGE])?,
        };
        let mut keys: Vec<String> = Vec::new();
        let mut ids: Vec<String> = Vec::new();
        for r in &rows {
            let (id, data): (String, Value) = (r.get("id"), r.get("data"));
            if let Some(text) = key_text(&data, spec) {
                keys.push(text);
                ids.push(id);
            }
        }
        if !ids.is_empty() {
            // A key text already taken by an earlier row (in this page or before it) is not inserted.
            let taken: Vec<String> = tx
                .query(
                    "WITH wanted AS (SELECT * FROM unnest($3::text[], $4::text[]) AS w(k, r)), \
                          done AS (INSERT INTO record_keys (app_slug, table_id, key_text, record_id) \
                                   SELECT $1, $2, k, r FROM wanted ON CONFLICT (app_slug, table_id, key_text) DO NOTHING RETURNING record_id) \
                     SELECT r FROM wanted WHERE r NOT IN (SELECT record_id FROM done)",
                    &[&app_slug, &table_id, &keys, &ids],
                )?
                .iter()
                .map(|r| r.get(0))
                .collect();
            for (k, id) in keys.iter().zip(&ids) {
                if taken.contains(id) {
                    lost.entry(k.clone()).or_default().insert(id.clone());
                }
            }
            kept += (ids.len() - taken.len()) as u64;
        }
        match rows.last() {
            Some(last) if rows.len() as i64 == PAGE => after = Some((last.get("created_at_text"), last.get("id"))),
            _ => break,
        }
    }
    if lost.is_empty() {
        return Ok(Ok(kept));
    }

    // Each group is the record that holds the key now, and the ones that lost it.
    let texts: Vec<String> = lost.keys().cloned().collect();
    let mut groups: Vec<Vec<String>> = Vec::new();
    for r in tx.query("SELECT key_text, record_id FROM record_keys WHERE app_slug = $1 AND table_id = $2 AND key_text = ANY($3)", &[&app_slug, &table_id, &texts])? {
        let (text, holder): (String, String) = (r.get(0), r.get(1));
        let mut group: Vec<String> = lost.remove(&text).unwrap_or_default().into_iter().collect();
        group.push(holder);
        group.sort();
        groups.push(group);
    }
    groups.sort();
    let mut left = MAX_LISTED;
    let mut more = 0;
    for g in &mut groups {
        let keep = g.len().min(left);
        more += g.len() - keep;
        g.truncate(keep);
        left -= keep;
    }
    groups.retain(|g| !g.is_empty());
    Ok(Err(Duplicates { groups, more }))
}
