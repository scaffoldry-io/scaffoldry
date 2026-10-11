//! The job that recomputes the records that roll up a changed record, when there are too many to do
//! inline. It works in batches of 200, each in its own transaction, and saves where it stopped.

use crate::jobs::{JobCtx, JobError};
use crate::repository::WriteCtx;
use crate::repository_links::{recompute_owners, ComputedSpec, TableCalc, INLINE_OWNERS};
use serde_json::{json, Value};
use std::collections::HashMap;

/// Owners recomputed in one transaction.
const BATCH: i64 = (INLINE_OWNERS * 5) as i64;

fn retry(e: impl std::fmt::Display) -> JobError {
    JobError::Retry(e.to_string())
}

fn text(payload: &Value, key: &str) -> Result<String, JobError> {
    payload.get(key).and_then(Value::as_str).map(str::to_string).ok_or_else(|| JobError::Permanent(format!("the job has no '{key}'")))
}

/// Recomputes every record that links to `record_id`, and the few that used to.
pub fn recalc_owners(ctx: &JobCtx, payload: Value) -> Result<Value, JobError> {
    let (app_slug, record_id, actor) = (text(&payload, "app_slug")?, text(&payload, "record_id")?, text(&payload, "actor")?);
    let computed: Vec<ComputedSpec> = serde_json::from_value(payload["plan"]["computed"].clone()).map_err(|e| JobError::Permanent(e.to_string()))?;
    let tables: HashMap<String, TableCalc> = serde_json::from_value(payload["plan"]["tables"].clone()).map_err(|e| JobError::Permanent(e.to_string()))?;
    let dropped: Vec<(String, String)> = serde_json::from_value(payload["dropped"].clone()).unwrap_or_default();
    let repo = ctx.repository();

    let mut write = WriteCtx::system(&actor);
    write.computed = computed;
    write.calc = std::sync::Arc::new(tables);

    let total: i64 = {
        let id = record_id.clone();
        repo.with_client(move |c| Ok(c.query_one("SELECT count(DISTINCT from_record) FROM record_links WHERE to_record = $1", &[&id])?.get(0))).map_err(retry)?
    };
    let (mut after, mut done) = match ctx.load_checkpoint() {
        Some(cp) => (cp.get("after").and_then(Value::as_str).map(str::to_string), cp.get("done").and_then(Value::as_i64).unwrap_or(0)),
        None => (None, 0),
    };
    let mut first = after.is_none();
    loop {
        if ctx.cancelled()? {
            return Err(JobError::Cancelled);
        }
        let (slug, id, from, w, extra) = (app_slug.clone(), record_id.clone(), after.clone(), write.clone(), if first { dropped.clone() } else { Vec::new() });
        let batch: Vec<(String, String)> = repo
            .with_client(move |c| {
                let mut tx = c.transaction()?;
                // Whole owners at a time, so no owner is split between two batches.
                let rows = tx.query(
                    "WITH page AS (SELECT DISTINCT from_record FROM record_links WHERE to_record = $1 AND ($2::text IS NULL OR from_record > $2) ORDER BY from_record LIMIT $3) \
                     SELECT l.from_record, l.field FROM record_links l JOIN page p ON p.from_record = l.from_record WHERE l.to_record = $1 ORDER BY l.from_record, l.field",
                    &[&id, &from, &BATCH],
                )?;
                let owners: Vec<(String, String)> = rows.iter().map(|r| (r.get(0), r.get(1))).collect();
                recompute_owners(&mut tx, &slug, owners.iter().cloned().chain(extra).collect(), &w)?;
                tx.commit()?;
                Ok(owners)
            })
            .map_err(retry)?;
        first = false;
        let mut seen: Vec<&String> = batch.iter().map(|(o, _)| o).collect();
        seen.dedup();
        done += seen.len() as i64;
        if let Some((last, _)) = batch.last() {
            after = Some(last.clone());
        }
        ctx.checkpoint(json!({ "after": after, "done": done }))?;
        ctx.progress(done, total.max(done))?;
        if (seen.len() as i64) < BATCH {
            break;
        }
    }
    ctx.result(json!({ "owners": done }))
}
