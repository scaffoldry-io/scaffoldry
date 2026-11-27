//! Data layer phase 5: the AI review.
//!
//! The review job `review_records` reads one dataset in batches, as `scan_records` does. For a row whose content matches a category
//! and that does not already carry it, it writes a finding in the content findings table, with the source `review`. A finding is one
//! for each field and category. It holds the field, the category, a count and up to five record ids, and never a value.
//!
//! The review only proposes. It changes no tag, floor, label, grant or access. A person reads the finding and decides, and the
//! decision is phase 6.
//!
//! A row already carries a category when the dataset floor holds it, when the row is tagged with it, or when the field's own
//! sensitivity holds it. Such a row gets no finding at all. A note that says "this is consistent" would be one more thing to read for
//! no decision, so the review says nothing.
//!
//! The server calls no model. The detector is a trait, and the first one reuses the content scan's detectors, mapped to categories by
//! the `detectors` list of each category in the sensitivity settings. Another detector can be plugged in by implementing [`Detector`].
//!
//! A batch is one transaction, and the job's checkpoint is written in it, so a worker that dies resumes after the last batch that
//! counted and never counts one twice.

use crate::content_scan::{text_of, Batch, Scope, SAMPLES, TRUNCATED_NOTE};
use crate::jobs::{JobCtx, JobError};
use crate::repository::{PostgresRepository, RepositoryError};
use crate::service::row_labels;
use crate::state::{AuthUser, SharedState};
use scaffoldry_engine::{effective_sensitivity, HostRouter};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use uuid::Uuid;

/// The job kind.
pub const KIND: &str = "review_records";
/// The source a review finding carries.
pub const SOURCE: &str = "review";
/// Records in a batch unless the job says otherwise.
pub const BATCH: i64 = crate::content_scan::BATCH;

/// What a detector says of one field's text: a category it signals, and how many matches signal it. Never the matched text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Signal {
    pub category: String,
    pub matches: i64,
    /// Only the first part of the text was read.
    pub truncated: bool,
}

/// Reads the text of one field of one row and says which categories it signals.
pub trait Detector: Send + Sync {
    fn read(&self, field: &str, text: &str) -> Vec<Signal>;
}

/// The first detector: the content scan's patterns. A match inside an authorized store for its category is expected and signals nothing.
pub struct Patterns(crate::content_scan::Plan);

impl Detector for Patterns {
    fn read(&self, field: &str, text: &str) -> Vec<Signal> {
        let mut by: BTreeMap<String, Signal> = BTreeMap::new();
        for h in self.0.check_text(field, text).hits {
            let s = by.entry(h.category.clone()).or_insert(Signal { category: h.category, matches: 0, truncated: false });
            s.matches += h.matches;
            s.truncated |= h.truncated;
        }
        by.into_values().collect()
    }
}

/// What a review of one table needs, read once before the first batch.
#[derive(Clone)]
pub struct Review {
    scope: Scope,
    detector: Arc<dyn Detector>,
    fields: Vec<String>,
    /// The categories each field holds in its own right, from the field and its label.
    carried: BTreeMap<String, BTreeSet<String>>,
}

/// What a batch counted for one field and category: matches, rows, up to five record ids, and whether only the start of a field was read.
type Counted = (i64, i64, Vec<String>, bool);

/// The review of a table, with the content scan's detectors.
pub fn plan(state: &SharedState, scope: &Scope) -> Result<Review, String> {
    let plan = crate::content_scan::plan(state, scope)?;
    let categories = state.sensitivity_categories();
    let fields: Vec<scaffoldry_engine::FieldSpec> = {
        let engine = state.engine.read().map_err(|e| e.to_string())?;
        let app = engine.resolve_by_slug(&scope.app_slug).ok_or_else(|| format!("the app '{}' is gone", scope.app_slug))?;
        app.tables.iter().find(|t| t.id == scope.table_id).map(|t| t.fields.clone()).ok_or_else(|| format!("the table '{}' is gone", scope.table_id))?
    };
    let labels = state.labels.read().map_err(|e| e.to_string())?;
    let carried = fields
        .iter()
        .map(|f| (f.name.clone(), effective_sensitivity(f, labels.get(&(scope.app_slug.clone(), scope.table_id.clone(), f.name.clone())), &categories).categories))
        .collect();
    Ok(Review { scope: scope.clone(), fields: plan.fields().to_vec(), carried, detector: Arc::new(Patterns(plan)) })
}

impl Review {
    /// The same review with another detector.
    pub fn with_detector(mut self, detector: Arc<dyn Detector>) -> Review {
        self.detector = detector;
        self
    }

    /// What one row proposes: for each field and category its content signals that the row does not carry already, the field, the
    /// category, the matches, and whether only the start of the field was read.
    fn read_row(&self, data: &Value, floor: &BTreeSet<String>) -> Vec<(String, Signal)> {
        let tags: BTreeSet<String> = scaffoldry_data::gate::tags_of(data).into_iter().collect();
        let mut out = Vec::new();
        for field in &self.fields {
            let Some(text) = data.get(field).and_then(text_of) else { continue };
            for s in self.detector.read(field, &text) {
                let held = floor.contains(&s.category) || tags.contains(&s.category) || self.carried.get(field).is_some_and(|c| c.contains(&s.category));
                if !held {
                    out.push((field.clone(), s));
                }
            }
        }
        out
    }
}

/// The detector id of a review finding. The findings table is unique for a field and a detector id, so this makes one finding for each
/// field and category, apart from any finding of a scan.
fn detector_id(category: &str) -> String {
    format!("{SOURCE}:{category}")
}

/// Starts a review of a table: what the last review counted for it is set to nothing, so this review's counts are its own. A finding's
/// status, its decision and its first-seen time stay.
pub fn begin(repo: &PostgresRepository, review: &Review) -> Result<(), RepositoryError> {
    let (app, table) = (review.scope.app_slug.clone(), review.scope.table_id.clone());
    repo.with_client(move |c| {
        c.execute("UPDATE content_findings SET match_count = 0, record_count = 0, sample_record_ids = '{}', note = NULL WHERE app_slug = $1 AND table_id = $2 AND source = $3", &[&app, &table, &SOURCE])?;
        Ok(())
    })
}

/// Ends a review of a table: an open finding that counted nothing this time is withdrawn. A decided finding stays, as the record of the decision.
pub fn finish(repo: &PostgresRepository, review: &Review) -> Result<u64, RepositoryError> {
    let (app, table) = (review.scope.app_slug.clone(), review.scope.table_id.clone());
    repo.with_client(move |c| Ok(c.execute("DELETE FROM content_findings WHERE app_slug = $1 AND table_id = $2 AND source = $3 AND status = 'open' AND record_count = 0", &[&app, &table, &SOURCE])?))
}

/// Reads one batch after the cursor and counts it. With a job id, the job's checkpoint is saved in the same transaction.
pub fn review_batch(repo: &PostgresRepository, review: &Review, after: Option<&str>, limit: i64, job: Option<Uuid>) -> Result<Batch, RepositoryError> {
    let (review, after) = (review.clone(), after.map(str::to_string));
    repo.with_client(move |c| {
        let mut tx = c.transaction()?;
        let (app, table) = (review.scope.app_slug.clone(), review.scope.table_id.clone());
        let floor: BTreeSet<String> = scaffoldry_data::labels::floor_of(&mut tx, &app, &table)?.into_iter().collect();
        let cursor = after.as_deref().and_then(|a| a.split_once('|'));
        let rows = scaffoldry_data::records::page_after(&mut tx, &app, &table, cursor, limit)?;
        let mut found: BTreeMap<(String, String), Counted> = BTreeMap::new();
        for row in &rows {
            for (field, s) in review.read_row(&row.data, &floor) {
                let e = found.entry((field, s.category)).or_default();
                e.0 += s.matches;
                e.1 += 1;
                if e.2.len() < SAMPLES {
                    e.2.push(row.id.clone());
                }
                e.3 |= s.truncated;
            }
        }
        for ((field, category), (matches, records, samples, truncated)) in &found {
            let note = truncated.then_some(TRUNCATED_NOTE);
            tx.execute(
                "INSERT INTO content_findings (id, app_slug, table_id, field, detector_id, category, source, match_count, record_count, sample_record_ids, note) \
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11) \
                 ON CONFLICT (app_slug, table_id, field, detector_id) DO UPDATE SET \
                    match_count = content_findings.match_count + EXCLUDED.match_count, \
                    record_count = content_findings.record_count + EXCLUDED.record_count, \
                    sample_record_ids = (SELECT COALESCE(array_agg(x), '{}') FROM (SELECT DISTINCT x FROM unnest(content_findings.sample_record_ids || EXCLUDED.sample_record_ids) AS x LIMIT 5) s), \
                    note = COALESCE(EXCLUDED.note, content_findings.note), last_seen = NOW()",
                &[&Uuid::new_v4(), &app, &table, field, &detector_id(category), category, &SOURCE, matches, records, samples, &note],
            )?;
        }
        let last = rows.last().map(|r| format!("{}|{}", r.created_at, r.id));
        if let (Some(job), Some(l)) = (job, &last) {
            tx.execute("UPDATE jobs SET checkpoint = $2 WHERE id = $1", &[&job, &json!({ "after": l })])?;
        }
        tx.commit()?;
        Ok(Batch { read: rows.len(), last })
    })
}

fn retry(e: impl std::fmt::Display) -> JobError {
    JobError::Retry(e.to_string())
}

/// The job `review_records`. The payload names `app_slug` and `table_id`, and may name `batch_size`.
pub fn review_job(ctx: &JobCtx, payload: Value) -> Result<Value, JobError> {
    let text = |k: &str| payload.get(k).and_then(Value::as_str).map(str::to_string).ok_or_else(|| JobError::Permanent(format!("the job has no '{k}'")));
    let scope = Scope { app_slug: text("app_slug")?, table_id: text("table_id")? };
    let batch = payload.get("batch_size").and_then(Value::as_i64).filter(|n| *n > 0).unwrap_or(BATCH);
    let state = ctx.state().ok_or_else(|| JobError::Permanent("the runner was not given the server's state".to_string()))?;
    let review = match plan(state, &scope) {
        Ok(p) => p,
        // The app or the table is gone. There is nothing to review.
        Err(e) => return ctx.result(json!({ "records": 0, "skipped": e })),
    };
    let repo = ctx.repository();
    // A first attempt starts the counts over. A later attempt resumes from the checkpoint and adds to them.
    let resume = ctx.load_checkpoint();
    let mut after = resume.as_ref().and_then(|c| c.get("after")).and_then(Value::as_str).map(str::to_string);
    let mut done = resume.as_ref().and_then(|c| c.get("done")).and_then(Value::as_i64).unwrap_or(0);
    if resume.is_none() {
        begin(repo, &review).map_err(retry)?;
    }
    let (mut batches, mut longest) = (0i64, std::time::Duration::ZERO);
    loop {
        if ctx.cancelled()? {
            return Err(JobError::Cancelled);
        }
        let started = std::time::Instant::now();
        let b = review_batch(repo, &review, after.as_deref(), batch, Some(ctx.job_id())).map_err(retry)?;
        longest = longest.max(started.elapsed());
        batches += 1;
        done += b.read as i64;
        if b.last.is_some() {
            after = b.last.clone();
        }
        ctx.checkpoint(json!({ "after": after, "done": done }))?;
        ctx.progress(done, done)?;
        if (b.read as i64) < batch {
            break;
        }
    }
    finish(repo, &review).map_err(retry)?;
    ctx.result(json!({ "records": done, "batches": batches, "longest_batch_ms": longest.as_millis() as i64 }))
}

/// Queues a review of one table. A review already waiting for the table is not queued again.
pub fn enqueue(repo: &PostgresRepository, app_slug: &str, table_id: &str, batch: Option<i64>, by: &str) -> Result<Uuid, crate::jobs::JobsError> {
    let mut payload = json!({ "app_slug": app_slug, "table_id": table_id });
    if let Some(b) = batch {
        payload["batch_size"] = json!(b);
    }
    crate::jobs::enqueue(repo, KIND, payload, by, Some(&format!("{KIND}:{app_slug}:{table_id}")))
}

/// Queues a review of every table of every app.
pub fn enqueue_all(state: &SharedState, by: &str) -> usize {
    crate::content_scan::all_tables(state).iter().filter(|(a, t)| enqueue(&state.repository, a, t, None, by).is_ok()).count()
}

/// The findings a reader may see, from a page of findings read for a steward. Applies to every source: a finding holds record ids, and an
/// id says a row exists.
///
/// A reader with no gate on a finding's table sees it whole. A gated reader sees it only when they may read at least one of its
/// sample rows, with the ids of the rows they may not read removed. Its counts are withheld, because a count says how many rows the
/// reader was not shown. Returns the findings and whether any was changed or left out.
pub fn visible_to(caller: &AuthUser, rows: Vec<Value>, state: &SharedState) -> (Vec<Value>, bool) {
    let mut gates: BTreeMap<(String, String), Vec<scaffoldry_data::gate::Gate>> = BTreeMap::new();
    let (mut out, mut changed) = (Vec::new(), false);
    for mut f in rows {
        let text = |k: &str| f[k].as_str().unwrap_or("").to_string();
        let (app, table) = (text("app_slug"), text("table_id"));
        let g = gates.entry((app.clone(), table.clone())).or_insert_with(|| row_labels::gates_of(caller, &app, &table, state));
        if g.is_empty() {
            out.push(f);
            continue;
        }
        changed = true;
        let ids: Vec<String> = f["sample_record_ids"].as_array().map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_string)).collect()).unwrap_or_default();
        let rows = {
            let (app, table, ids) = (app.clone(), table.clone(), ids.clone());
            // A failed read shows nothing: a reader is never shown an id that was not checked.
            state.repository.with_client(move |c| Ok(scaffoldry_data::records::data_of_ids(c, &app, &table, &ids)?)).unwrap_or_default()
        };
        let readable: BTreeSet<String> = rows.into_iter().filter(|(_, data)| row_labels::admits_with(g, data, state)).map(|(id, _)| id).collect();
        let shown: Vec<&String> = ids.iter().filter(|i| readable.contains(*i)).collect();
        if shown.is_empty() {
            continue;
        }
        f["sample_record_ids"] = json!(shown);
        f["match_count"] = Value::Null;
        f["record_count"] = Value::Null;
        f["counts_withheld"] = json!(true);
        out.push(f);
    }
    (out, changed)
}
