//! Connections phase 3 and 4: what every source that is not a file shares.
//!
//! A source hands its rows over in batches, twice. The first pass checks every row and counts them, and writes nothing,
//! so a bad value, a repeated key, a column that changed, or a result that shrank stops the run before anything changes.
//! The second pass writes only what changed, 1,000 rows at a time. A source decides how it gives its rows twice: the
//! database reads its query again inside one snapshot, and the HTTP source keeps what it fetched.

use crate::connections::{key_source_of, last_good, sync_ctx, trash_removed, type_one, write_batch, Conn, Counts, Fault, RunOpts, Source, BATCH, SHRINK_FLOOR_PERCENT};
use crate::state::SharedState;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};

/// A source of rows, one JSON object for each.
pub(crate) trait Feed {
    /// The columns the source has, when it knows them before it gives a row. A query does. A web service does not.
    fn columns(&mut self) -> Result<Option<Vec<String>>, String>;
    /// Hands every row to `f`, a batch at a time. Called twice, and a source gives the same rows both times.
    fn each(&mut self, pass: u8, f: &mut dyn FnMut(Vec<Value>) -> Result<(), String>) -> Result<(), String>;
}

/// A row as text by column, the way a csv row is.
pub(crate) fn as_text(row: &Value) -> HashMap<String, String> {
    row.as_object()
        .map(|o| {
            o.iter()
                .map(|(k, v)| {
                    let text = match v {
                        Value::String(s) => s.clone(),
                        Value::Null => String::new(),
                        other => other.to_string(),
                    };
                    (k.clone(), text)
                })
                .collect()
        })
        .unwrap_or_default()
}

/// One run over a feed.
pub(crate) fn drive(state: &SharedState, conn: &Conn, opts: &RunOpts, feed: &mut dyn Feed) -> Result<(Option<Counts>, String), Fault> {
    // A column the source gained or lost is a change of shape, handled as a file's is.
    let known = feed.columns()?;
    if let Some(headers) = &known {
        if let Some(drift) = crate::connection_drift::find(conn, &Source { headers: Some(headers.clone()), rows: Vec::new() }) {
            return Err(Fault::Drift(drift));
        }
    }

    // First pass: every row is checked and counted, and nothing is written.
    let key_source = key_source_of(conn);
    let (mut keys, mut hasher, mut total) = (HashSet::new(), Sha256::new(), 0usize);
    let mut union: Vec<String> = Vec::new();
    let mut union_seen: HashSet<String> = HashSet::new();
    feed.each(1, &mut |batch| {
        for row in batch {
            hasher.update(row.to_string().as_bytes());
            hasher.update(b"\n");
            total += 1;
            if known.is_none() {
                if let Some(o) = row.as_object() {
                    for k in o.keys().filter(|k| union_seen.insert((*k).clone())) {
                        union.push(k.clone());
                    }
                }
            }
            type_one(conn, &key_source, &as_text(&row), total + 1, &mut keys)?;
        }
        Ok(())
    })?;
    drop(keys);
    if known.is_none() {
        if let Some(drift) = crate::connection_drift::find(conn, &Source { headers: Some(union), rows: Vec::new() }) {
            return Err(Fault::Drift(drift));
        }
    }
    if conn.pending_proposal.is_some() {
        crate::connection_drift::withdraw(state, conn);
    }
    let hash: String = hasher.finalize().iter().map(|b| format!("{b:02x}")).collect();
    let (last_hash, last_rows) = last_good(state, conn.id).map_err(|e| e.to_string())?;
    if last_hash.as_deref() == Some(hash.as_str()) {
        return Ok((None, hash));
    }
    if let Some(prev) = last_rows.filter(|p| *p > 0) {
        if !opts.approve_shrink && total * 100 < prev * SHRINK_FLOOR_PERCENT {
            return Err(format!("source_shrank: the source gave {total} rows and the last run held {prev}. A person must approve a result this much smaller").into());
        }
    }

    // Second pass: the same rows, written in batches.
    let ctx = sync_ctx(state, conn);
    let ferpa = crate::connections::is_sensitive(&conn.sensitivity);
    let (mut seen, mut keys) = (HashSet::with_capacity(total), HashSet::new());
    let mut n = Counts { added: 0, changed: 0, removed: 0, unchanged: 0 };
    let mut pending: Vec<(String, Value)> = Vec::with_capacity(BATCH);
    let flush = |pending: &mut Vec<(String, Value)>, n: &mut Counts| -> Result<(), String> {
        if pending.is_empty() {
            return Ok(());
        }
        let (a, c, u) = write_batch(state, conn, &ctx, std::mem::take(pending), ferpa).map_err(|e| format!("A batch could not be written: {e}"))?;
        n.added += a;
        n.changed += c;
        n.unchanged += u;
        Ok(())
    };
    let mut seen_rows = 0usize;
    feed.each(2, &mut |batch| {
        for row in batch {
            seen_rows += 1;
            let (key, data) = type_one(conn, &key_source, &as_text(&row), seen_rows + 1, &mut keys)?;
            let id = crate::connections::record_id(conn.id, &conn.dataset_table, &key);
            seen.insert(id.clone());
            pending.push((id, Value::Object(data)));
            if pending.len() >= BATCH {
                flush(&mut pending, &mut n)?;
            }
        }
        Ok(())
    })?;
    flush(&mut pending, &mut n)?;
    n.removed = trash_removed(state, conn, &ctx, seen).map_err(|e| format!("Removed rows could not be trashed: {e}"))?;
    Ok((Some(n), hash))
}
