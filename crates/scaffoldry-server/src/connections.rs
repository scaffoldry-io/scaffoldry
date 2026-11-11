//! Connections phase 1: scheduled, read-only copies of external data.
//!
//! A connection names a source and the dataset it fills. The sync reads the source, compares it with the dataset, and
//! writes only what changed, through the same record writers every write uses, with the sync as the actor. Nothing else
//! writes a dataset's records: the services refuse first, and a database trigger stands behind them.
//!
//! This phase builds the file drop. `SCAFFOLDRY_DROP_DIR` names a directory the operator mounts, so a file drop reaches
//! no network. A path is resolved inside it, after refusing `..`, an absolute path, and a link that leaves it.
//!
//! A run:
//! 1. Reads the file and hashes it. The same hash as the last good run ends the run as `unchanged`, writing nothing.
//! 2. Checks the columns against the connection's `column_map`, and the whole file for bad values and repeated keys. A
//!    file with a fault changes nothing.
//! 3. Compares the row count with the last good run. Under half stops the run as `source_shrank`, until a person
//!    approves it, to protect against a file that was cut short.
//! 4. Writes in batches of 1,000, each in one transaction. A record keeps its id by its key. One that did not change is
//!    not written and gets no history. A record whose key is gone goes to trash, and comes back if its key returns.

use crate::repository::{delete_record_in_tx, insert_record_in_tx, restore_record_in_tx, update_record_in_tx, ActorKind, RepositoryError};
use crate::state::{DatasetRecord, SharedState};
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::path::{Component, Path, PathBuf};
use uuid::Uuid;

/// Records written in one transaction.
pub const BATCH: usize = 1_000;
/// A run still marked running after this long is taken to have died with its server.
const STALE_RUNNING_SECS: i64 = 3_600;
/// The largest file a run reads. A run holds the file and its rows in memory, so the limit is modest. A larger source
/// belongs to the PostgreSQL connector of phase 3, which streams.
pub const MAX_FILE_BYTES: u64 = 256 << 20;
/// Runs kept for each connection.
pub const KEEP_RUNS: i64 = 100;
/// The file may hold at least this share of the last good run's rows, or the run stops.
const SHRINK_FLOOR_PERCENT: usize = 50;

/// The types a column may be mapped to.
pub const COLUMN_TYPES: [&str; 4] = ["Text", "Number", "Date", "Boolean"];

/// A source column, the field it fills, and its type.
#[derive(Debug, Clone)]
pub struct Column {
    pub source: String,
    pub field: String,
    pub kind: String,
}

/// A connection, as the sync reads it.
#[derive(Debug, Clone)]
pub struct Conn {
    pub id: Uuid,
    pub name: String,
    pub kind: String,
    pub config: Value,
    pub dataset_app: String,
    pub dataset_table: String,
    pub key_field: String,
    pub columns: Vec<Column>,
    pub sensitivity: String,
}

pub fn columns_of(map: &Value) -> Vec<Column> {
    map.as_array()
        .map(|a| {
            a.iter()
                .filter_map(|c| Some(Column { source: c.get("source")?.as_str()?.to_string(), field: c.get("field")?.as_str()?.to_string(), kind: c.get("type")?.as_str()?.to_string() }))
                .collect()
        })
        .unwrap_or_default()
}

/// Whether a connection's sensitivity makes its fields sensitive. Public and directory data is not.
pub fn is_sensitive(sensitivity: &str) -> bool {
    !matches!(sensitivity.to_lowercase().as_str(), "public" | "directory")
}

/// Whether `rel` is a name inside a directory: not empty, not absolute, and with no `..` or `.` part. No file is touched.
pub fn check_name(rel: &str) -> Result<(), String> {
    let name = Path::new(rel);
    if rel.trim().is_empty() || name.is_absolute() || rel.starts_with(['/', '\\']) {
        return Err("The path must be a name inside the drop directory, such as sis/programs.csv".to_string());
    }
    if name.components().any(|c| !matches!(c, Component::Normal(_))) {
        return Err("The path may not hold '..' or '.' parts".to_string());
    }
    Ok(())
}

/// `rel` inside `root`, or why not. A name that is absolute, empty, or holds `..` is refused before any file is
/// touched. A link that leaves `root` is refused after the path is resolved.
pub fn resolve_in(root: &Path, rel: &str) -> Result<PathBuf, String> {
    check_name(rel)?;
    let name = Path::new(rel);
    let root = root.canonicalize().map_err(|_| "The drop directory is not there".to_string())?;
    let full = root.join(name).canonicalize().map_err(|_| format!("The file {rel} is not in the drop directory"))?;
    if !full.starts_with(&root) {
        return Err("The path leaves the drop directory".to_string());
    }
    Ok(full)
}

/// A name for the record of `key`, the same on every run, so a record keeps its id and the links to it stay.
pub fn record_id(conn: Uuid, table: &str, key: &str) -> String {
    let digest = Sha256::digest(format!("{conn}|{table}|{key}").as_bytes());
    let mut bytes = [0u8; 16];
    bytes.copy_from_slice(&digest[..16]);
    bytes[6] = (bytes[6] & 0x0f) | 0x50;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    Uuid::from_bytes(bytes).to_string()
}

fn typed(kind: &str, raw: &str) -> Result<Value, String> {
    let raw = raw.trim();
    match kind {
        "Text" => Ok(Value::String(raw.to_string())),
        "Number" if raw.is_empty() => Ok(Value::Null),
        "Number" => {
            let n: f64 = raw.replace(',', "").parse().map_err(|_| format!("'{raw}' is not a number"))?;
            if !n.is_finite() {
                return Err(format!("'{raw}' is not a number"));
            }
            Ok(if n.fract() == 0.0 && n.abs() < 9e15 { json!(n as i64) } else { json!(n) })
        }
        "Boolean" if raw.is_empty() => Ok(Value::Null),
        "Boolean" => match raw.to_lowercase().as_str() {
            "true" | "yes" | "y" | "1" => Ok(json!(true)),
            "false" | "no" | "n" | "0" => Ok(json!(false)),
            _ => Err(format!("'{raw}' is not true or false")),
        },
        "Date" if raw.is_empty() => Ok(Value::Null),
        "Date" => {
            let ok = chrono::NaiveDate::parse_from_str(raw, "%Y-%m-%d").is_ok() || chrono::DateTime::parse_from_rfc3339(raw).is_ok();
            if ok {
                Ok(Value::String(raw.to_string()))
            } else {
                Err(format!("'{raw}' is not a date (use 2026-11-02)"))
            }
        }
        other => Err(format!("The column type '{other}' is not one of Text, Number, Date or Boolean")),
    }
}

/// The rows of a file as `(key, data)`, or why the file cannot be used. Nothing is written here.
/// A row of the file: its key, and its typed fields.
type Row = (String, Map<String, Value>);

fn read_rows(conn: &Conn, bytes: &[u8]) -> Result<Vec<Row>, String> {
    let format = conn.config.get("format").and_then(Value::as_str).unwrap_or("csv");
    let mut raw: Vec<HashMap<String, String>> = Vec::new();
    match format {
        "csv" => {
            let (headers, rows) = crate::import_jobs::parse_csv_stream(bytes, ',', None)?;
            let mapped: HashSet<&str> = conn.columns.iter().map(|c| c.source.as_str()).collect();
            let present: HashSet<&str> = headers.iter().map(String::as_str).collect();
            let missing: Vec<&str> = conn.columns.iter().map(|c| c.source.as_str()).filter(|s| !present.contains(s)).collect();
            let extra: Vec<&str> = headers.iter().map(String::as_str).filter(|h| !mapped.contains(h)).collect();
            if !missing.is_empty() || !extra.is_empty() {
                let mut parts = Vec::new();
                for m in &missing {
                    parts.push(format!("The source no longer has \"{m}\"."));
                }
                for e in &extra {
                    parts.push(format!("The source added \"{e}\"."));
                }
                return Err(format!("schema_drift: {}", parts.join(" ")));
            }
            for row in rows {
                if row.len() == 1 && row[0].is_empty() {
                    continue;
                }
                raw.push(headers.iter().cloned().zip(row.into_iter().chain(std::iter::repeat(String::new()))).collect());
            }
        }
        "jsonl" => {
            let text = std::str::from_utf8(bytes).map_err(|_| "Invalid file encoding: only UTF-8 is supported.".to_string())?;
            for (i, line) in text.lines().enumerate().filter(|(_, l)| !l.trim().is_empty()) {
                let v: Value = serde_json::from_str(line).map_err(|e| format!("Line {}: not JSON ({e})", i + 1))?;
                let obj = v.as_object().ok_or_else(|| format!("Line {}: not an object", i + 1))?;
                raw.push(
                    obj.iter()
                        .map(|(k, v)| (k.clone(), if let Some(s) = v.as_str() { s.to_string() } else if v.is_null() { String::new() } else { v.to_string() }))
                        .collect(),
                );
            }
        }
        other => return Err(format!("The format '{other}' is not csv or jsonl")),
    }
    let key_source = conn.columns.iter().find(|c| c.field == conn.key_field).map(|c| c.source.clone()).unwrap_or_default();
    let mut seen = HashSet::new();
    let mut out = Vec::with_capacity(raw.len());
    for (i, row) in raw.iter().enumerate() {
        let n = i + 2;
        let mut data = Map::new();
        data.insert("_table_id".into(), Value::String(conn.dataset_table.clone()));
        for col in &conn.columns {
            let cell = row.get(&col.source).map(String::as_str).unwrap_or("");
            data.insert(col.field.clone(), typed(&col.kind, cell).map_err(|e| format!("bad_value: row {n}, column \"{}\": {e}", col.source))?);
        }
        let key = row.get(&key_source).map(|k| k.trim().to_string()).unwrap_or_default();
        if key.is_empty() {
            return Err(format!("bad_key: row {n} has no value in the key column \"{key_source}\""));
        }
        if !seen.insert(key.clone()) {
            return Err(format!("duplicate_key: the key \"{key}\" is on more than one row (row {n})"));
        }
        out.push((key, data));
    }
    Ok(out)
}

// ---- Rows in the database ---------------------------------------------------------------------------------------

fn ts(col: &str) -> String {
    format!("to_char({col} AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS\"Z\"')")
}

fn run_json(r: &postgres::Row) -> Value {
    json!({
        "id": r.get::<_, Uuid>(0), "state": r.get::<_, String>(1), "started_at": r.get::<_, String>(2), "finished_at": r.get::<_, Option<String>>(3),
        "added": r.get::<_, i64>(4), "changed": r.get::<_, i64>(5), "removed": r.get::<_, i64>(6), "unchanged": r.get::<_, i64>(7),
        "error": r.get::<_, Option<String>>(8), "source_hash": r.get::<_, Option<String>>(9).map(|h| h.trim().to_string()),
    })
}

fn run_select() -> String {
    format!("SELECT id, state, {}, {}, added, changed, removed, unchanged, error, source_hash FROM sync_runs", ts("started_at"), ts("finished_at"))
}

/// A connection's newest runs.
pub fn runs(repo: &crate::repository::PostgresRepository, id: Uuid) -> Result<Vec<Value>, RepositoryError> {
    repo.with_client(move |c| Ok(c.query(&format!("{} WHERE connection_id = $1 ORDER BY started_at DESC, id LIMIT {KEEP_RUNS}", run_select()), &[&id])?.iter().map(run_json).collect()))
}

/// The connection with this id, as the sync reads it.
pub fn load(repo: &crate::repository::PostgresRepository, id: Uuid) -> Result<Option<Conn>, RepositoryError> {
    repo.with_client(move |c| {
        Ok(c.query_opt("SELECT id, name, kind, config, dataset_app, dataset_table, key_field, column_map, sensitivity FROM connections WHERE id = $1", &[&id])?.map(|r| Conn {
            id: r.get(0),
            name: r.get(1),
            kind: r.get(2),
            config: r.get(3),
            dataset_app: r.get(4),
            dataset_table: r.get(5),
            key_field: r.get(6),
            columns: columns_of(&r.get::<_, Value>(7)),
            sensitivity: r.get(8),
        }))
    })
}

/// Why a run did not happen.
#[derive(Debug)]
pub enum RunError {
    NotFound,
    /// Another run of the connection is going.
    Busy,
    /// A connection that is not due, when the caller asked for one that is.
    NotDue,
    Storage(String),
}

impl From<RepositoryError> for RunError {
    fn from(e: RepositoryError) -> Self {
        RunError::Storage(e.to_string())
    }
}

const DUE: &str = "c.enabled AND ( \
    (c.every_minutes IS NOT NULL AND NOT EXISTS (SELECT 1 FROM sync_runs r WHERE r.connection_id = c.id AND r.started_at > NOW() - make_interval(mins => c.every_minutes))) \
    OR (c.at_time IS NOT NULL AND (NOW() AT TIME ZONE $1::text)::time >= c.at_time::time \
        AND NOT EXISTS (SELECT 1 FROM sync_runs r WHERE r.connection_id = c.id AND r.started_at >= ((date_trunc('day', NOW() AT TIME ZONE $1::text) + c.at_time::time) AT TIME ZONE $1::text))))";

fn timezone(state: &SharedState) -> String {
    state.settings.read().ok().and_then(|s| s.get("platform.timezone").and_then(Value::as_str).map(str::to_string)).unwrap_or_else(|| "UTC".to_string())
}

/// Opens a run. `None` when another run of the connection is going, or, with `only_if_due`, when it is not due.
fn start_run(state: &SharedState, id: Uuid, only_if_due: bool) -> Result<Option<Uuid>, RepositoryError> {
    let (run, tz) = (Uuid::new_v4(), timezone(state));
    state.repository.with_client(move |c| {
        c.execute(
            "UPDATE sync_runs SET state = 'failed', finished_at = NOW(), error = 'The run stopped before it finished' \
             WHERE connection_id = $1 AND state = 'running' AND started_at < NOW() - make_interval(secs => $2)",
            &[&id, &(STALE_RUNNING_SECS as f64)],
        )?;
        let started = if only_if_due {
            c.execute(&format!("INSERT INTO sync_runs (id, connection_id, state) SELECT $2, c.id, 'running' FROM connections c WHERE c.id = $3 AND {DUE} ON CONFLICT DO NOTHING"), &[&tz, &run, &id])?
        } else {
            c.execute("INSERT INTO sync_runs (id, connection_id, state) VALUES ($1, $2, 'running') ON CONFLICT DO NOTHING", &[&run, &id])?
        };
        Ok((started == 1).then_some(run))
    })
}

struct Counts {
    added: i64,
    changed: i64,
    removed: i64,
    unchanged: i64,
}

fn finish_run(state: &SharedState, run: Uuid, outcome: &str, n: &Counts, error: Option<&str>, hash: Option<&str>) -> Result<(), RepositoryError> {
    let (outcome, error, hash) = (outcome.to_string(), error.map(str::to_string), hash.map(str::to_string));
    let (a, ch, r, u) = (n.added, n.changed, n.removed, n.unchanged);
    state.repository.with_client(move |c| {
        c.execute(
            "UPDATE sync_runs SET state = $2, finished_at = NOW(), added = $3, changed = $4, removed = $5, unchanged = $6, error = $7, source_hash = $8 WHERE id = $1",
            &[&run, &outcome, &a, &ch, &r, &u, &error, &hash],
        )?;
        // Only the newest runs are kept.
        c.execute(
            "DELETE FROM sync_runs WHERE id IN (SELECT id FROM sync_runs s WHERE s.connection_id = (SELECT connection_id FROM sync_runs WHERE id = $1) \
             ORDER BY started_at DESC OFFSET $2)",
            &[&run, &KEEP_RUNS],
        )?;
        Ok(())
    })?;
    Ok(())
}

/// The hash of the last run that applied or confirmed its file, and the rows the last run that wrote held.
fn last_good(state: &SharedState, id: Uuid) -> Result<(Option<String>, Option<usize>), RepositoryError> {
    state.repository.with_client(move |c| {
        let hash = c.query_opt("SELECT source_hash FROM sync_runs WHERE connection_id = $1 AND state IN ('done', 'unchanged') ORDER BY started_at DESC LIMIT 1", &[&id])?.and_then(|r| r.get::<_, Option<String>>(0)).map(|h| h.trim().to_string());
        let rows = c.query_opt("SELECT added + changed + unchanged FROM sync_runs WHERE connection_id = $1 AND state = 'done' ORDER BY started_at DESC LIMIT 1", &[&id])?.map(|r| r.get::<_, i64>(0) as usize);
        Ok((hash, rows))
    })
}

/// One batch of the file, written in one transaction.
fn write_batch(state: &SharedState, conn: &Conn, ctx: &crate::repository::WriteCtx, batch: Vec<(String, Value)>, ferpa: bool) -> Result<(i64, i64, i64), RepositoryError> {
    let (app, ctx, by) = (conn.dataset_app.clone(), ctx.clone(), ctx.actor.clone());
    state.repository.with_client(move |c| {
        let mut tx = c.transaction()?;
        tx.execute("SELECT set_config('scaffoldry.sync_writer', 'on', true)", &[])?;
        let ids: Vec<String> = batch.iter().map(|(id, _)| id.clone()).collect();
        let mut existing: HashMap<String, (Value, bool)> = HashMap::new();
        for r in tx.query("SELECT id, data, deleted_at IS NOT NULL FROM dataset_records WHERE app_slug = $1 AND id = ANY($2) FOR UPDATE", &[&app, &ids])? {
            existing.insert(r.get(0), (r.get(1), r.get(2)));
        }
        let (mut added, mut changed, mut unchanged) = (0, 0, 0);
        for (id, data) in batch {
            match existing.get(&id) {
                None => {
                    let rec = DatasetRecord { id, app_slug: app.clone(), data, ceds_mapping: json!({}), is_ferpa_sensitive: ferpa, created_at: chrono::Utc::now().to_rfc3339(), created_by: Some(by.clone()), version: 1 };
                    insert_record_in_tx(&mut tx, &rec, &ctx)?;
                    added += 1;
                }
                Some((old, trashed)) => {
                    if *trashed {
                        restore_record_in_tx(&mut tx, &app, &id, &ctx)?;
                        added += 1;
                    }
                    if *old != data {
                        update_record_in_tx(&mut tx, &app, &id, &data, None, &ctx)?;
                        if !*trashed {
                            changed += 1;
                        }
                    } else if !*trashed {
                        unchanged += 1;
                    }
                }
            }
        }
        tx.commit()?;
        Ok((added, changed, unchanged))
    })
}

/// Sends the records whose key is no longer in the file to trash, in batches. Returns how many.
fn trash_removed(state: &SharedState, conn: &Conn, ctx: &crate::repository::WriteCtx, seen: HashSet<String>) -> Result<i64, RepositoryError> {
    let (app, table) = (conn.dataset_app.clone(), conn.dataset_table.clone());
    let live: Vec<String> = state.repository.with_client(move |c| Ok(c.query("SELECT id FROM dataset_records WHERE app_slug = $1 AND table_id = $2 AND deleted_at IS NULL", &[&app, &table])?.iter().map(|r| r.get(0)).collect()))?;
    let gone: Vec<String> = live.into_iter().filter(|id| !seen.contains(id)).collect();
    let mut removed = 0;
    for chunk in gone.chunks(BATCH) {
        let (app, ctx, chunk) = (conn.dataset_app.clone(), ctx.clone(), chunk.to_vec());
        removed += state.repository.with_client(move |c| {
            let mut tx = c.transaction()?;
            tx.execute("SELECT set_config('scaffoldry.sync_writer', 'on', true)", &[])?;
            let mut n = 0;
            for id in &chunk {
                if matches!(delete_record_in_tx(&mut tx, &app, id, None, &ctx)?, crate::repository::RecordWrite::Saved(_)) {
                    n += 1;
                }
            }
            tx.commit()?;
            Ok(n)
        })?;
    }
    Ok(removed)
}

/// The work of one run, after it is open. `Ok(None)` is `unchanged`. A fault comes back as text, and the run keeps it.
fn apply(state: &SharedState, conn: &Conn, approve_shrink: bool) -> Result<(Option<Counts>, String), String> {
    if conn.kind != "file_drop" {
        return Err(format!("The connection kind '{}' is not built yet", conn.kind));
    }
    let root = state.drop_dir.as_deref().ok_or("No drop directory is configured. Set SCAFFOLDRY_DROP_DIR to the mounted directory")?;
    let rel = conn.config.get("path").and_then(Value::as_str).unwrap_or("");
    let path = resolve_in(root, rel)?;
    let meta = std::fs::metadata(&path).map_err(|e| format!("The file could not be read: {e}"))?;
    if !meta.is_file() {
        return Err("The path is not a file".to_string());
    }
    if meta.len() > MAX_FILE_BYTES {
        return Err(format!("file_too_large: the file is {} bytes, over the limit of {MAX_FILE_BYTES}", meta.len()));
    }
    let bytes = std::fs::read(&path).map_err(|e| format!("The file could not be read: {e}"))?;
    let hash: String = Sha256::digest(&bytes).iter().map(|b| format!("{b:02x}")).collect();
    let (last_hash, last_rows) = last_good(state, conn.id).map_err(|e| e.to_string())?;
    if last_hash.as_deref() == Some(hash.as_str()) {
        return Ok((None, hash));
    }
    let rows = read_rows(conn, &bytes)?;
    if let Some(prev) = last_rows.filter(|p| *p > 0) {
        if !approve_shrink && rows.len() * 100 < prev * SHRINK_FLOOR_PERCENT {
            return Err(format!("source_shrank: the file holds {} rows and the last run held {prev}. A person must approve a file this much smaller", rows.len()));
        }
    }
    let table_row = serde_json::json!({ "_table_id": conn.dataset_table });
    let actor = format!("sync:{}", conn.name);
    let mut ctx = crate::service::history::ctx_for(&actor, ActorKind::Sync, &conn.dataset_app, &table_row, state);
    ctx.token_label = Some(conn.name.clone());
    let ferpa = is_sensitive(&conn.sensitivity);

    let mut seen = HashSet::with_capacity(rows.len());
    let mut n = Counts { added: 0, changed: 0, removed: 0, unchanged: 0 };
    let mut batch: Vec<(String, Value)> = Vec::with_capacity(BATCH);
    let flush = |batch: &mut Vec<(String, Value)>, n: &mut Counts| -> Result<(), String> {
        if batch.is_empty() {
            return Ok(());
        }
        let (a, c, u) = write_batch(state, conn, &ctx, std::mem::take(batch), ferpa).map_err(|e| format!("A batch could not be written: {e}"))?;
        n.added += a;
        n.changed += c;
        n.unchanged += u;
        Ok(())
    };
    for (key, data) in rows {
        let id = record_id(conn.id, &conn.dataset_table, &key);
        seen.insert(id.clone());
        batch.push((id, Value::Object(data)));
        if batch.len() >= BATCH {
            flush(&mut batch, &mut n)?;
        }
    }
    flush(&mut batch, &mut n)?;
    n.removed = trash_removed(state, conn, &ctx, seen).map_err(|e| format!("Removed rows could not be trashed: {e}"))?;
    Ok((Some(n), hash))
}

/// Runs a connection now. A fault ends the run as `failed` with its text, and is returned as the run, not as an error.
pub fn run(state: &SharedState, id: Uuid, approve_shrink: bool, only_if_due: bool) -> Result<Value, RunError> {
    let conn = load(&state.repository, id)?.ok_or(RunError::NotFound)?;
    let run_id = match start_run(state, id, only_if_due)? {
        Some(r) => r,
        None if only_if_due => return Err(RunError::NotDue),
        None => return Err(RunError::Busy),
    };
    let zero = Counts { added: 0, changed: 0, removed: 0, unchanged: 0 };
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| apply(state, &conn, approve_shrink))).unwrap_or_else(|_| Err("The sync stopped unexpectedly".to_string()));
    match outcome {
        Ok((None, hash)) => finish_run(state, run_id, "unchanged", &zero, None, Some(&hash))?,
        Ok((Some(n), hash)) => finish_run(state, run_id, "done", &n, None, Some(&hash))?,
        Err(error) => finish_run(state, run_id, "failed", &zero, Some(&error), None)?,
    }
    Ok(state.repository.with_client(move |c| Ok(run_json(&c.query_one(&format!("{} WHERE id = $1", run_select()), &[&run_id])?)))?)
}

/// Runs each enabled connection that is due, once. Two callers at once run a connection once.
pub fn sync_due(state: &SharedState) -> Result<usize, String> {
    let tz = timezone(state);
    let ids: Vec<Uuid> = state
        .repository
        .with_client(move |c| Ok(c.query(&format!("SELECT c.id FROM connections c WHERE {DUE} ORDER BY c.name"), &[&tz])?.iter().map(|r| r.get(0)).collect()))
        .map_err(|e| e.to_string())?;
    let mut ran = 0;
    for id in ids {
        match run(state, id, false, true) {
            Ok(_) => ran += 1,
            Err(RunError::Busy | RunError::NotDue | RunError::NotFound) => {}
            Err(RunError::Storage(e)) => return Err(e),
        }
    }
    Ok(ran)
}

/// The `sync_due_connections` job.
pub fn sync_due_job(ctx: &crate::jobs::JobCtx, _payload: Value) -> Result<Value, crate::jobs::JobError> {
    let state = ctx.state().ok_or_else(|| crate::jobs::JobError::Permanent("syncing connections needs the server's state, and this runner has none".to_string()))?;
    let ran = sync_due(state).map_err(crate::jobs::JobError::Retry)?;
    Ok(json!({ "ran": ran }))
}
