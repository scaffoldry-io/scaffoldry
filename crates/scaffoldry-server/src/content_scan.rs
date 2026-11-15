//! Sensitive content phase 2: scanning stored records for what the organization's detectors look for.
//!
//! The job `scan_records` reads one table in batches, by the order the table is already indexed in, runs the enabled detectors over each
//! text and number field, and records findings. A finding holds a field, a detector, counts and up to five record ids, and never a value.
//! A match inside an authorized store for the detector's category raises no finding. It is counted as expected.
//!
//! A batch is one transaction, and the job's checkpoint is written in that transaction, so a worker that dies resumes after the last
//! batch that counted and never counts one twice. No batch holds a lock for longer than it takes to read 1,000 records and write a few rows.

use crate::jobs::{JobCtx, JobError};
use crate::repository::{PostgresRepository, RepositoryError};
use crate::state::SharedState;
use scaffoldry_engine::detect::{scan, DetectorSpec};
use scaffoldry_engine::sensitivity::{default_settings, validate_detectors, Category};
use scaffoldry_engine::{FieldSpec, FieldType};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use uuid::Uuid;

/// Records in a batch unless the job says otherwise.
pub const BATCH: i64 = 1_000;
/// Record ids a finding keeps.
pub const SAMPLES: usize = 5;

/// What to scan: one table of one app.
#[derive(Debug, Clone)]
pub struct Scope {
    pub app_slug: String,
    pub table_id: String,
}

/// A detector and the category it belongs to.
#[derive(Debug, Clone)]
struct Armed {
    spec: DetectorSpec,
    category: String,
}

/// A place a category is allowed to live.
#[derive(Debug, Clone)]
struct Store {
    category: String,
    scope: String,
    app_slug: String,
    table_id: String,
    field: String,
    workspace_id: String,
}

/// Everything a scan needs to know, read once before the first batch.
#[derive(Debug, Clone)]
pub struct Plan {
    scope: Scope,
    workspace: Option<String>,
    fields: Vec<String>,
    detectors: Vec<Armed>,
    stores: Vec<Store>,
}

/// What a batch counted for one field and detector: matches, records, and up to five record ids.
type Counted = (i64, i64, Vec<String>);

/// One batch's result: how many records were read and the cursor after the last.
#[derive(Debug, Clone)]
pub struct Batch {
    pub read: usize,
    pub last: Option<String>,
}

fn is_scannable(t: FieldType) -> bool {
    matches!(t, FieldType::Text | FieldType::LongText | FieldType::Email | FieldType::Phone | FieldType::Url | FieldType::Number | FieldType::Currency | FieldType::Percent | FieldType::Rating)
}

fn setting(state: &SharedState, key: &str) -> Option<Value> {
    state.settings.read().ok().and_then(|s| s.get(key).cloned())
}

/// The detectors that are on, each with its category. A detector no category lists protects nothing and is not run.
fn armed(state: &SharedState) -> Vec<Armed> {
    let (default_categories, default_detectors) = default_settings();
    let detectors: Vec<DetectorSpec> = setting(state, "sensitivity.detectors").and_then(|v| validate_detectors(&v).ok()).unwrap_or(default_detectors);
    let categories: Vec<Category> = setting(state, "sensitivity.categories").and_then(|v| serde_json::from_value(v).ok()).unwrap_or(default_categories);
    detectors
        .into_iter()
        .filter(|d| d.enabled)
        .filter_map(|d| categories.iter().find(|c| c.detectors.contains(&d.id)).map(|c| Armed { category: c.id.clone(), spec: d }))
        .collect()
}

/// Reads what a scan of this table needs. Nothing is written.
pub fn plan(state: &SharedState, scope: &Scope) -> Result<Plan, String> {
    use scaffoldry_engine::HostRouter;
    let (workspace, fields): (Option<String>, Vec<FieldSpec>) = {
        let engine = state.engine.read().map_err(|e| e.to_string())?;
        let app = engine.resolve_by_slug(&scope.app_slug).ok_or_else(|| format!("the app '{}' is gone", scope.app_slug))?;
        let table = app.tables.iter().find(|t| t.id == scope.table_id).ok_or_else(|| format!("the table '{}' is gone", scope.table_id))?;
        (app.workspace_id.clone(), table.fields.clone())
    };
    let stores = state
        .repository
        .with_client(move |c| {
            Ok(c.query("SELECT category, scope, app_slug, table_id, field, workspace_id FROM authorized_stores", &[])?
                .iter()
                .map(|r| Store { category: r.get(0), scope: r.get(1), app_slug: r.get(2), table_id: r.get(3), field: r.get(4), workspace_id: r.get(5) })
                .collect::<Vec<_>>())
        })
        .map_err(|e| e.to_string())?;
    Ok(Plan {
        scope: scope.clone(),
        workspace,
        fields: fields.into_iter().filter(|f| is_scannable(f.field_type)).map(|f| f.name).collect(),
        detectors: armed(state),
        stores,
    })
}

impl Plan {
    /// Whether an authorized store for `category` covers this field.
    fn covers(&self, category: &str, field: &str) -> bool {
        self.stores.iter().any(|s| {
            s.category == category
                && match s.scope.as_str() {
                    "workspace" => self.workspace.as_deref() == Some(s.workspace_id.as_str()),
                    "app" => s.app_slug == self.scope.app_slug,
                    "table" => s.app_slug == self.scope.app_slug && s.table_id == self.scope.table_id,
                    "field" => s.app_slug == self.scope.app_slug && s.table_id == self.scope.table_id && s.field == field,
                    _ => false,
                }
        })
    }
}

/// The text a field is read as. A number is read as its digits.
fn text_of(v: &Value) -> Option<String> {
    match v {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string().chars().filter(char::is_ascii_digit).collect()),
        _ => None,
    }
}

/// Starts a scan of a table: what the last scan counted for it is set to nothing, so this scan's counts are its own. A finding's status,
/// its decision and its first-seen time stay.
pub fn begin(repo: &PostgresRepository, plan: &Plan) -> Result<(), RepositoryError> {
    let (app, table) = (plan.scope.app_slug.clone(), plan.scope.table_id.clone());
    repo.with_client(move |c| {
        let mut tx = c.transaction()?;
        tx.execute("UPDATE content_findings SET match_count = 0, record_count = 0, sample_record_ids = '{}' WHERE app_slug = $1 AND table_id = $2 AND source = 'scan'", &[&app, &table])?;
        tx.execute("DELETE FROM content_expected WHERE app_slug = $1 AND table_id = $2", &[&app, &table])?;
        tx.commit()?;
        Ok(())
    })
}

/// Reads one batch after the cursor and counts it. With a job id, the job's checkpoint is saved in the same transaction.
pub fn scan_batch(repo: &PostgresRepository, plan: &Plan, after: Option<&str>, limit: i64, job: Option<Uuid>) -> Result<Batch, RepositoryError> {
    let (app, table, after, plan) = (plan.scope.app_slug.clone(), plan.scope.table_id.clone(), after.map(str::to_string), plan.clone());
    repo.with_client(move |c| {
        let mut tx = c.transaction()?;
        let (from_at, from_id) = match after.as_deref().and_then(|a| a.split_once('|')) {
            Some((at, id)) => (Some(at.to_string()), Some(id.to_string())),
            None => (None, None),
        };
        let rows = tx.query(
            "SELECT id, data, created_at::text FROM dataset_records WHERE app_slug = $1 AND table_id = $2 AND deleted_at IS NULL \
             AND ($3::text IS NULL OR (created_at, id) > ($3::text::timestamptz, $4::text)) ORDER BY created_at, id LIMIT $5",
            &[&app, &table, &from_at, &from_id, &limit],
        )?;
        // (field, detector id, category) -> (matches, records, sample ids). Expected: (field, category) -> matches.
        let mut found: BTreeMap<(String, String, String), Counted> = BTreeMap::new();
        let mut expected: BTreeMap<(String, String), i64> = BTreeMap::new();
        for row in &rows {
            let (id, data): (String, Value) = (row.get(0), row.get(1));
            for field in &plan.fields {
                let Some(text) = data.get(field).and_then(text_of) else { continue };
                // The first 64 KB of a field is read, as a write is checked.
                let text = if text.len() > 65_536 { text.chars().take(65_536).collect::<String>() } else { text };
                for d in &plan.detectors {
                    let n = scan(&text, &d.spec).len() as i64;
                    if n == 0 {
                        continue;
                    }
                    if plan.covers(&d.category, field) {
                        *expected.entry((field.clone(), d.category.clone())).or_default() += n;
                        continue;
                    }
                    let e = found.entry((field.clone(), d.spec.id.clone(), d.category.clone())).or_default();
                    e.0 += n;
                    e.1 += 1;
                    if e.2.len() < SAMPLES {
                        e.2.push(id.clone());
                    }
                }
            }
        }
        for ((field, detector, category), (matches, records, samples)) in &found {
            tx.execute(
                "INSERT INTO content_findings (id, app_slug, table_id, field, detector_id, category, source, match_count, record_count, sample_record_ids) \
                 VALUES ($1, $2, $3, $4, $5, $6, 'scan', $7, $8, $9) \
                 ON CONFLICT (app_slug, table_id, field, detector_id) DO UPDATE SET \
                    category = EXCLUDED.category, match_count = content_findings.match_count + EXCLUDED.match_count, \
                    record_count = content_findings.record_count + EXCLUDED.record_count, \
                    sample_record_ids = (SELECT COALESCE(array_agg(x), '{}') FROM (SELECT DISTINCT x FROM unnest(content_findings.sample_record_ids || EXCLUDED.sample_record_ids) AS x LIMIT 5) s), \
                    last_seen = NOW()",
                &[&Uuid::new_v4(), &app, &table, field, detector, category, matches, records, samples],
            )?;
        }
        for ((field, category), matches) in &expected {
            tx.execute(
                "INSERT INTO content_expected (app_slug, table_id, field, category, matches) VALUES ($1, $2, $3, $4, $5) \
                 ON CONFLICT (app_slug, table_id, field, category) DO UPDATE SET matches = content_expected.matches + EXCLUDED.matches, updated_at = NOW()",
                &[&app, &table, field, category, matches],
            )?;
        }
        let last = rows.last().map(|r| format!("{}|{}", r.get::<_, String>(2), r.get::<_, String>(0)));
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

/// The job `scan_records`. The payload names `app_slug` and `table_id`, and may name `batch_size`.
pub fn scan_job(ctx: &JobCtx, payload: Value) -> Result<Value, JobError> {
    let text = |k: &str| payload.get(k).and_then(Value::as_str).map(str::to_string).ok_or_else(|| JobError::Permanent(format!("the job has no '{k}'")));
    let scope = Scope { app_slug: text("app_slug")?, table_id: text("table_id")? };
    let batch = payload.get("batch_size").and_then(Value::as_i64).filter(|n| *n > 0).unwrap_or(BATCH);
    let state = ctx.state().ok_or_else(|| JobError::Permanent("the runner was not given the server's state".to_string()))?;
    let plan = match plan(state, &scope) {
        Ok(p) => p,
        // The app or the table is gone. There is nothing to scan.
        Err(e) => return ctx.result(json!({ "records": 0, "skipped": e })),
    };
    let repo = ctx.repository();
    // A first attempt starts the counts over. A later attempt resumes from the checkpoint and adds to them.
    let resume = ctx.load_checkpoint();
    let mut after = resume.as_ref().and_then(|c| c.get("after")).and_then(Value::as_str).map(str::to_string);
    let mut done = resume.as_ref().and_then(|c| c.get("done")).and_then(Value::as_i64).unwrap_or(0);
    if resume.is_none() {
        begin(repo, &plan).map_err(retry)?;
    }
    let (mut batches, mut longest) = (0i64, std::time::Duration::ZERO);
    loop {
        if ctx.cancelled()? {
            return Err(JobError::Cancelled);
        }
        let started = std::time::Instant::now();
        let b = scan_batch(repo, &plan, after.as_deref(), batch, Some(ctx.job_id())).map_err(retry)?;
        // A batch is one transaction. The longest one is the longest the scan held any lock.
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
    ctx.result(json!({ "records": done, "batches": batches, "longest_batch_ms": longest.as_millis() as i64 }))
}

/// Queues a scan of one table. A scan already waiting for the table is not queued again.
pub fn enqueue(repo: &PostgresRepository, app_slug: &str, table_id: &str, batch: Option<i64>, by: &str) -> Result<Uuid, crate::jobs::JobsError> {
    let mut payload = json!({ "app_slug": app_slug, "table_id": table_id });
    if let Some(b) = batch {
        payload["batch_size"] = json!(b);
    }
    crate::jobs::enqueue(repo, "scan_records", payload, by, Some(&format!("scan_records:{app_slug}:{table_id}")))
}

/// Queues a scan of every table of every app. A change to a detector or a category does this.
pub fn enqueue_all(state: &SharedState, by: &str) -> usize {
    let tables: Vec<(String, String)> = state
        .engine
        .read()
        .map(|e| e.list_manifests().into_iter().flat_map(|m| m.tables.iter().map(|t| (m.slug.clone(), t.id.clone())).collect::<Vec<_>>()).collect())
        .unwrap_or_default();
    tables.iter().filter(|(a, t)| enqueue(&state.repository, a, t, None, by).is_ok()).count()
}
