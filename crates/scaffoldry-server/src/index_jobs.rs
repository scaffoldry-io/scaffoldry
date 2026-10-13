//! Job handlers that touch many records: building an index, and catching stored flags up with a
//! label. Each commits one batch at a time and saves a checkpoint, so a stopped job resumes.

use crate::jobs::{JobCtx, JobError};
use crate::repository_index::copy_batch;
use scaffoldry_engine::{AppManifest, FieldSpec};
use serde_json::{json, Value};

/// Records per batch.
const BATCH: i64 = 2_000;

struct Target {
    app_slug: String,
    table_id: String,
    fields: Vec<String>,
    batch: i64,
}

fn target(payload: &Value) -> Result<Target, JobError> {
    let text = |key: &str| payload.get(key).and_then(Value::as_str).map(str::to_string).ok_or_else(|| JobError::Permanent(format!("the job has no '{key}'")));
    let fields = payload
        .get("fields")
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_string)).collect::<Vec<_>>())
        .filter(|f| !f.is_empty())
        .ok_or_else(|| JobError::Permanent("the job names no fields".to_string()))?;
    let batch = payload.get("batch_size").and_then(Value::as_i64).filter(|n| *n > 0).unwrap_or(BATCH);
    Ok(Target { app_slug: text("app_slug")?, table_id: text("table_id")?, fields, batch })
}

fn retry(e: impl std::fmt::Display) -> JobError {
    JobError::Retry(e.to_string())
}

fn resume_point(ctx: &JobCtx) -> (Option<(String, String)>, i64) {
    match ctx.load_checkpoint() {
        Some(cp) => {
            let at = cp.get("created_at").and_then(Value::as_str);
            let id = cp.get("id").and_then(Value::as_str);
            let done = cp.get("done").and_then(Value::as_i64).unwrap_or(0);
            (at.zip(id).map(|(a, i)| (a.to_string(), i.to_string())), done)
        }
        None => (None, 0),
    }
}

fn save(ctx: &JobCtx, last: &Option<(String, String)>, done: i64, total: i64) -> Result<(), JobError> {
    if let Some((at, id)) = last {
        ctx.checkpoint(json!({ "created_at": at, "id": id, "done": done }))?;
    }
    ctx.progress(done, total)
}

/// Writes the index rows of the named fields for every record of a table, then marks the fields
/// `ready`.
pub fn build_index(ctx: &JobCtx, payload: Value) -> Result<Value, JobError> {
    let t = target(&payload)?;
    let repo = ctx.repository();
    let (slug, table, names) = (t.app_slug.clone(), t.table_id.clone(), t.fields.clone());
    // The approved manifest says what each field is.
    let specs: Vec<FieldSpec> = repo
        .with_client(move |c| {
            let row = c.query_opt("SELECT manifest FROM app_manifests WHERE slug = $1", &[&slug])?;
            let manifest: Option<AppManifest> = row.and_then(|r| serde_json::from_value(r.get::<_, Value>(0)).ok());
            Ok(manifest
                .and_then(|m| m.tables.into_iter().find(|x| x.id == table))
                .map(|x| x.fields.into_iter().filter(|f| names.contains(&f.name)).collect())
                .unwrap_or_default())
        })
        .map_err(retry)?;
    if specs.is_empty() {
        return ctx.result(json!({ "records": 0, "skipped": "the app, table, or fields are gone" }));
    }

    let (slug, table) = (t.app_slug.clone(), t.table_id.clone());
    let total: i64 = repo
        .with_client(move |c| Ok(c.query_one("SELECT count(*) FROM dataset_records WHERE app_slug = $1 AND table_id = $2 AND deleted_at IS NULL", &[&slug, &table])?.get(0)))
        .map_err(retry)?;
    let (mut after, mut done) = resume_point(ctx);
    loop {
        if ctx.cancelled()? {
            return Err(JobError::Cancelled);
        }
        let (slug, table, specs, from, size) = (t.app_slug.clone(), t.table_id.clone(), specs.clone(), after.clone(), t.batch);
        let batch = repo
            .with_client(move |c| {
                let mut tx = c.transaction()?;
                let b = copy_batch(&mut tx, &slug, &table, &specs, from.as_ref(), size)?;
                tx.commit()?;
                Ok(b)
            })
            .map_err(retry)?;
        done += batch.read as i64;
        if batch.last.is_some() {
            after = batch.last;
        }
        save(ctx, &after, done, total.max(done))?;
        if (batch.read as i64) < t.batch {
            break;
        }
    }
    let (slug, table, names) = (t.app_slug.clone(), t.table_id.clone(), t.fields.clone());
    repo.with_client(move |c| {
        c.execute(
            "UPDATE index_status SET state = 'ready', updated_at = NOW() WHERE app_slug = $1 AND table_id = $2 AND field = ANY($3)",
            &[&slug, &table, &names],
        )?;
        Ok(())
    })
    .map_err(retry)?;
    ctx.result(json!({ "records": done, "fields": t.fields.len() }))
}

/// Raises the stored flag of every record that holds one of the named fields. A label only raises,
/// so nothing is lowered. Reads already apply the label, so this only catches the stored flags up.
pub fn recompute_flags(ctx: &JobCtx, payload: Value) -> Result<Value, JobError> {
    let t = target(&payload)?;
    let repo = ctx.repository();
    let (mut after, mut done) = resume_point(ctx);
    let mut raised = 0i64;
    loop {
        if ctx.cancelled()? {
            return Err(JobError::Cancelled);
        }
        let (read, changed, last) = repo
            .raise_flags_batch(&t.app_slug, &t.table_id, &t.fields, after.as_ref(), t.batch)
            .map_err(retry)?;
        done += read as i64;
        raised += changed as i64;
        if last.is_some() {
            after = last;
        }
        save(ctx, &after, done, done)?;
        if (read as i64) < t.batch {
            break;
        }
    }
    ctx.result(json!({ "records": done, "raised": raised }))
}

/// Samples a view's walk so a jump to any row starts at the nearest anchor. The payload is an `AnchorSpec`.
pub fn build_anchors(ctx: &JobCtx, payload: Value) -> Result<Value, JobError> {
    let spec: crate::repository_index::AnchorSpec = serde_json::from_value(payload).map_err(|e| JobError::Permanent(format!("the job is not an anchor spec: {e}")))?;
    let repo = ctx.repository();
    let total = repo.build_anchors(spec.view_id, &spec.hash(), &spec.query()).map_err(retry)?;
    ctx.progress(total, total)?;
    ctx.result(json!({ "rows": total, "anchors": total / crate::repository_index::ANCHOR_STEP as i64 + 1 }))
}
