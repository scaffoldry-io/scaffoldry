//! Snapshot jobs (import-export phase 5): take one, and restore one into a new app.
//!
//! A snapshot file is gzip of JSON Lines. The first line is `{"snapshot":1,"app_slug":..,"definition":<bundle>}`. Every
//! other line is one record: id, data, ceds_mapping, is_ferpa_sensitive, created_by, created_at. A snapshot is built in
//! memory, so the number of records is bounded by the setting `snapshots.max_records` (200,000 unless set).

use crate::files;
use crate::jobs::{JobCtx, JobError};
use crate::repository::{insert_record_in_tx, ActorKind, WriteCtx};
use crate::repository_proposals::Approval;
use crate::state::DatasetRecord;
use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use flate2::Compression;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use uuid::Uuid;

pub const DEFAULT_MAX_RECORDS: i64 = 200_000;
const PAGE: i64 = 1_000;

fn id_of(payload: &Value, key: &str) -> Result<Uuid, JobError> {
    payload.get(key).and_then(Value::as_str).and_then(|s| Uuid::parse_str(s).ok()).ok_or_else(|| JobError::Permanent(format!("Missing or invalid {key}")))
}

fn retry(e: impl std::fmt::Display) -> JobError {
    JobError::Retry(e.to_string())
}

/// Writes the snapshot file and returns its hash, record count and size.
fn build(ctx: &JobCtx, id: Uuid) -> Result<(String, i64, i64), String> {
    let repo = ctx.repository();
    let snapshot = repo.get_snapshot(id).map_err(|e| e.to_string())?.ok_or("The snapshot is gone")?;
    let max = repo.setting_i64("snapshots.max_records").map_err(|e| e.to_string())?.filter(|n| *n > 0).unwrap_or(DEFAULT_MAX_RECORDS);
    let total = repo.live_record_count(&snapshot.app_slug).map_err(|e| e.to_string())?;
    if total > max {
        return Err(format!("The app holds {total} records and a snapshot holds at most {max}"));
    }
    let dir = files::dir().ok_or("File storage is not configured, so a snapshot cannot be kept")?;

    let mut out = GzEncoder::new(Vec::new(), Compression::default());
    let header = json!({ "snapshot": 1, "app_slug": snapshot.app_slug, "definition": snapshot.definition });
    writeln!(out, "{header}").map_err(|e| e.to_string())?;
    let (mut after, mut written) = (String::new(), 0i64);
    loop {
        if ctx.cancelled().map_err(|_| "Cancelled".to_string())? {
            return Err("The snapshot was cancelled".to_string());
        }
        let page = repo.records_after(&snapshot.app_slug, &after, PAGE).map_err(|e| e.to_string())?;
        let Some(last) = page.last() else { break };
        after = last.id.clone();
        for r in &page {
            let line = json!({ "id": r.id, "data": r.data, "ceds_mapping": r.ceds_mapping, "is_ferpa_sensitive": r.is_ferpa_sensitive, "created_by": r.created_by, "created_at": r.created_at });
            writeln!(out, "{line}").map_err(|e| e.to_string())?;
            written += 1;
        }
        let _ = ctx.progress(written, total.max(written));
    }
    let bytes = out.finish().map_err(|e| e.to_string())?;
    let sha = files::store_bytes(&dir, &bytes).map_err(|e| format!("The snapshot could not be stored: {e}"))?;
    Ok((sha, written, bytes.len() as i64))
}

/// Takes a snapshot. With `then_approve`, the snapshot was asked for by an approval that waits for it: the approval is
/// finished here when the snapshot is ready, and the proposal goes back to pending when it is not.
pub fn snapshot_take(ctx: &JobCtx, payload: Value) -> Result<Value, JobError> {
    let id = id_of(&payload, "snapshot_id")?;
    let then = payload.get("then_approve").cloned().filter(|v| v.is_object());
    let repo = ctx.repository();
    let proposal = then.as_ref().and_then(|t| t.get("proposal_id").and_then(Value::as_str)).and_then(|s| Uuid::parse_str(s).ok());
    let release = |why: &str| {
        let _ = repo.fail_snapshot(id, why);
        if let Some(p) = proposal {
            let _ = repo.move_proposal(p, "Approving", "Pending");
        }
    };

    let (sha, records, size) = match build(ctx, id) {
        Ok(done) => done,
        Err(why) => {
            release(&why);
            return Err(JobError::Permanent(why));
        }
    };
    if let Err(e) = repo.finish_snapshot(id, &sha, records, size) {
        release(&e.to_string());
        return Err(JobError::Permanent(e.to_string()));
    }

    let mut approved = false;
    if let (Some(t), Some(pid)) = (&then, proposal) {
        let text = |k: &str| t.get(k).and_then(Value::as_str).unwrap_or_default().to_string();
        let other = t.get("other_person").and_then(Value::as_bool).unwrap_or(false);
        match repo.approve_proposal(pid, &text("decider"), &text("rationale"), &text("digest"), other) {
            Ok(Approval::Approved(_)) => approved = true,
            Ok(_) => {
                // Superseded, or someone else decided it. A proposal still waiting goes back to pending.
                let _ = repo.move_proposal(pid, "Approving", "Pending");
            }
            Err(e) => {
                let _ = repo.move_proposal(pid, "Approving", "Pending");
                return Err(JobError::Permanent(format!("The snapshot is ready, but the approval was not kept: {e}")));
            }
        }
    }
    ctx.result(json!({ "snapshot_id": id.to_string(), "record_count": records, "size": size, "approved": approved }))
}

fn open(sha: &str) -> Result<impl BufRead, JobError> {
    let dir = files::dir().ok_or_else(|| JobError::Permanent("File storage is not configured".to_string()))?;
    let file = std::fs::File::open(files::blob_path(&dir, sha)).map_err(|e| JobError::Permanent(format!("The snapshot file is missing: {e}")))?;
    Ok(BufReader::new(GzDecoder::new(BufReader::new(file))))
}

/// The names of each table's Link fields, from the snapshot's definition.
fn link_fields(definition: &Value) -> HashMap<String, Vec<String>> {
    definition["manifest"]["tables"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|t| {
            let names: Vec<String> = t["fields"].as_array()?.iter().filter(|f| f["field_type"] == "Link").filter_map(|f| f["name"].as_str().map(str::to_string)).collect();
            Some((t["id"].as_str()?.to_string(), names)).filter(|(_, n)| !n.is_empty())
        })
        .collect()
}

fn remap(value: &mut Value, ids: &HashMap<String, String>) {
    match value {
        Value::String(s) => {
            if let Some(new) = ids.get(s.as_str()) {
                *s = new.clone();
            }
        }
        Value::Array(a) => a.iter_mut().for_each(|v| remap(v, ids)),
        _ => {}
    }
}

/// Loads a snapshot's records into an app that is live and empty. Record ids are new, so the original is untouched, and
/// the ids in Link fields follow them.
pub fn snapshot_restore(ctx: &JobCtx, payload: Value) -> Result<Value, JobError> {
    let id = id_of(&payload, "snapshot_id")?;
    let slug = payload.get("slug").and_then(Value::as_str).ok_or_else(|| JobError::Permanent("Missing slug".to_string()))?.to_string();
    let by = payload.get("caller_eppn").and_then(Value::as_str).ok_or_else(|| JobError::Permanent("Missing caller_eppn".to_string()))?.to_string();
    let repo = ctx.repository();
    let snapshot = repo.get_snapshot(id).map_err(retry)?.ok_or_else(|| JobError::Permanent("The snapshot is gone".to_string()))?;
    let sha = snapshot.file_sha256.clone().filter(|_| snapshot.state == "ready").ok_or_else(|| JobError::Permanent("The snapshot is not ready".to_string()))?;
    if repo.get_app_manifest(&slug).map_err(retry)?.is_none() {
        return Err(JobError::Permanent(format!("The app '{slug}' is not live, so there is nowhere to restore to")));
    }
    if repo.live_record_count(&slug).map_err(retry)? > 0 {
        return Err(JobError::Permanent(format!("The app '{slug}' already has records. A restore only fills an empty app")));
    }

    // Pass one: a new id for every record.
    let mut ids: HashMap<String, String> = HashMap::new();
    for line in open(&sha)?.lines().skip(1) {
        let line = line.map_err(|e| JobError::Permanent(e.to_string()))?;
        let v: Value = serde_json::from_str(&line).map_err(|e| JobError::Permanent(format!("A record in the snapshot is not JSON: {e}")))?;
        if let Some(old) = v["id"].as_str() {
            ids.insert(old.to_string(), format!("rec_{}", &Uuid::new_v4().simple().to_string()[..16]));
        }
    }
    let total = ids.len() as i64;
    let links = link_fields(&snapshot.definition);

    // Pass two: write them in batches. A batch is one history batch and one transaction.
    let mut lines = open(&sha)?.lines().skip(1).peekable();
    let mut done = 0i64;
    while lines.peek().is_some() {
        if ctx.cancelled()? {
            return Err(JobError::Cancelled);
        }
        let mut batch: Vec<DatasetRecord> = Vec::new();
        for line in lines.by_ref().take(PAGE as usize) {
            let line = line.map_err(|e| JobError::Permanent(e.to_string()))?;
            let v: Value = serde_json::from_str(&line).map_err(|e| JobError::Permanent(format!("A record in the snapshot is not JSON: {e}")))?;
            let old = v["id"].as_str().unwrap_or_default();
            let mut data = v["data"].clone();
            if let Some(names) = data.get("_table_id").and_then(Value::as_str).and_then(|t| links.get(t)) {
                for name in names {
                    if let Some(cell) = data.get_mut(name) {
                        remap(cell, &ids);
                    }
                }
            }
            batch.push(DatasetRecord {
                id: ids.get(old).cloned().unwrap_or_else(|| format!("rec_{}", &Uuid::new_v4().simple().to_string()[..16])),
                app_slug: slug.clone(),
                data,
                ceds_mapping: v["ceds_mapping"].clone(),
                is_ferpa_sensitive: v["is_ferpa_sensitive"].as_bool().unwrap_or(false),
                created_at: v["created_at"].as_str().unwrap_or_default().to_string(),
                created_by: v["created_by"].as_str().map(str::to_string),
                version: 1,
            });
        }
        let mut write_ctx = WriteCtx::new(&by, ActorKind::Import);
        write_ctx.batch_id = Uuid::new_v4();
        let n = batch.len() as i64;
        repo.with_client(move |c| {
            let mut tx = c.transaction()?;
            for rec in &batch {
                insert_record_in_tx(&mut tx, rec, &write_ctx)?;
            }
            tx.commit()?;
            Ok(())
        })
        .map_err(retry)?;
        done += n;
        ctx.progress(done, total)?;
    }
    ctx.result(json!({ "snapshot_id": id.to_string(), "app_slug": slug, "records": done }))
}
