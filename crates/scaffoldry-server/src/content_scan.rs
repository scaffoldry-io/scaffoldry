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
use scaffoldry_engine::detect::{scan, Action, DetectorSpec};
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
pub struct Store {
    pub category: String,
    pub scope: String,
    pub app_slug: String,
    pub table_id: String,
    pub field: String,
    pub workspace_id: String,
}

/// What a field of a write or a file holds that a detector matched, outside every authorized store. Never the value.
#[derive(Debug, Clone)]
pub struct Hit {
    pub field: String,
    pub detector_id: String,
    pub category: String,
    pub action: Action,
    pub matches: i64,
    /// Only the first 64 KB of the text was read.
    pub truncated: bool,
}

/// The note a finding carries when only the start of a field was read.
pub const TRUNCATED_NOTE: &str = "Only the first 64 KB of this field was read.";

/// The result of reading one piece of text: what was found outside the stores, and how many matches were expected inside them.
pub struct Checked {
    pub hits: Vec<Hit>,
    pub expected: Vec<(String, i64)>,
}

/// Bytes of a field a check reads.
pub const READ_LIMIT: usize = 65_536;

/// Everything a scan needs to know, read once before the first batch.
#[derive(Debug, Clone)]
pub struct Plan {
    scope: Scope,
    workspace: Option<String>,
    fields: Vec<String>,
    detectors: Vec<Armed>,
    stores: Vec<Store>,
}

/// What a batch counted for one field and detector: matches, records, up to five record ids, and whether only the start of a field was read.
type Counted = (i64, i64, Vec<String>, bool);

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

impl Plan {
    /// A plan from the parts. `detectors` and `categories` are the two settings, or `None` for the defaults of a new installation.
    pub fn build(scope: &Scope, workspace: Option<String>, table_fields: &[FieldSpec], detectors: Option<Value>, categories: Option<Value>, stores: Vec<Store>) -> Plan {
        let (default_categories, default_detectors) = default_settings();
        let detectors: Vec<DetectorSpec> = detectors.and_then(|v| validate_detectors(&v).ok()).unwrap_or(default_detectors);
        let categories: Vec<Category> = categories.and_then(|v| serde_json::from_value(v).ok()).unwrap_or(default_categories);
        // A detector no category lists protects nothing and is not run.
        let armed = detectors
            .into_iter()
            .filter(|d| d.enabled)
            .filter_map(|d| categories.iter().find(|c| c.detectors.contains(&d.id)).map(|c| Armed { category: c.id.clone(), spec: d }))
            .collect();
        Plan { scope: scope.clone(), workspace, fields: table_fields.iter().filter(|f| is_scannable(f.field_type)).map(|f| f.name.clone()).collect(), detectors: armed, stores }
    }
}

/// Reads what a check of this table needs, from what the server holds. Nothing is written.
pub fn plan(state: &SharedState, scope: &Scope) -> Result<Plan, String> {
    use scaffoldry_engine::HostRouter;
    let (workspace, fields): (Option<String>, Vec<FieldSpec>) = {
        let engine = state.engine.read().map_err(|e| e.to_string())?;
        let app = engine.resolve_by_slug(&scope.app_slug).ok_or_else(|| format!("the app '{}' is gone", scope.app_slug))?;
        let table = app.tables.iter().find(|t| t.id == scope.table_id).ok_or_else(|| format!("the table '{}' is gone", scope.table_id))?;
        (app.workspace_id.clone(), table.fields.clone())
    };
    let stores = state.authorized_stores.read().map_err(|e| e.to_string())?.clone();
    Ok(Plan::build(scope, workspace, &fields, setting(state, "sensitivity.detectors"), setting(state, "sensitivity.categories"), stores))
}

/// The same plan from the database alone, for a job that is not given the server's state, such as an import.
pub fn plan_from_repo(repo: &PostgresRepository, scope: &Scope) -> Result<Plan, String> {
    let manifest = repo.get_app_manifest(&scope.app_slug).map_err(|e| e.to_string())?.ok_or_else(|| format!("the app '{}' is gone", scope.app_slug))?;
    let table = manifest.tables.iter().find(|t| t.id == scope.table_id || t.slug == scope.table_id).ok_or_else(|| format!("the table '{}' is gone", scope.table_id))?;
    let settings = repo.get_platform_settings().map_err(|e| e.to_string())?;
    let stores = repo.store_rows().map_err(|e| e.to_string())?;
    let scope = Scope { app_slug: scope.app_slug.clone(), table_id: table.id.clone() };
    Ok(Plan::build(&scope, manifest.workspace_id.clone(), &table.fields, settings.get("sensitivity.detectors").cloned(), settings.get("sensitivity.categories").cloned(), stores))
}

impl Plan {
    /// Reads one piece of text as the field `field`. A match inside an authorized store for the detector's category is expected and is not a hit.
    pub fn check_text(&self, field: &str, text: &str) -> Checked {
        let truncated = text.len() > READ_LIMIT;
        let text: std::borrow::Cow<str> = if truncated { text.chars().take(READ_LIMIT).collect::<String>().into() } else { text.into() };
        let mut out = Checked { hits: Vec::new(), expected: Vec::new() };
        for d in &self.detectors {
            let n = scan(&text, &d.spec).len() as i64;
            if n == 0 {
                continue;
            }
            if self.covers(&d.category, field) {
                out.expected.push((d.category.clone(), n));
            } else {
                out.hits.push(Hit { field: field.to_string(), detector_id: d.spec.id.clone(), category: d.category.clone(), action: d.spec.action, matches: n, truncated });
            }
        }
        out
    }

    /// Reads text as one field, whole. The caller has limited its size already.
    pub fn check_whole(&self, field: &str, text: &str) -> Checked {
        let mut out = Checked { hits: Vec::new(), expected: Vec::new() };
        for d in &self.detectors {
            let n = scan(text, &d.spec).len() as i64;
            if n == 0 {
                continue;
            }
            if self.covers(&d.category, field) {
                out.expected.push((d.category.clone(), n));
            } else {
                out.hits.push(Hit { field: field.to_string(), detector_id: d.spec.id.clone(), category: d.category.clone(), action: d.spec.action, matches: n, truncated: false });
            }
        }
        out
    }

    /// Looks at the fields a write changed. A field that was not changed is not in `changes`, so no detector reads it.
    pub fn check(&self, changes: &serde_json::Map<String, Value>) -> Vec<Hit> {
        let mut hits = Vec::new();
        for field in &self.fields {
            if let Some(text) = changes.get(field).and_then(text_of) {
                hits.extend(self.check_text(field, &text).hits);
            }
        }
        hits
    }

    /// For each detector that is on: its id, its category and its action as a word.
    pub fn armed_actions(&self) -> Vec<(String, String, &'static str)> {
        self.detectors.iter().map(|d| (d.spec.id.clone(), d.category.clone(), match d.spec.action { Action::Flag => "flag", Action::Warn => "warn", Action::Block => "block" })).collect()
    }

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
        tx.execute("UPDATE content_findings SET match_count = 0, record_count = 0, sample_record_ids = '{}', note = NULL WHERE app_slug = $1 AND table_id = $2 AND source = 'scan'", &[&app, &table])?;
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
        // (field, detector id, category) -> counts. Expected: (field, category) -> matches.
        let mut found: BTreeMap<(String, String, String), Counted> = BTreeMap::new();
        let mut expected: BTreeMap<(String, String), i64> = BTreeMap::new();
        for row in &rows {
            let (id, data): (String, Value) = (row.get(0), row.get(1));
            for field in &plan.fields {
                let Some(text) = data.get(field).and_then(text_of) else { continue };
                let checked = plan.check_text(field, &text);
                for (category, n) in checked.expected {
                    *expected.entry((field.clone(), category)).or_default() += n;
                }
                for h in checked.hits {
                    let e = found.entry((h.field, h.detector_id, h.category)).or_default();
                    e.0 += h.matches;
                    e.1 += 1;
                    if e.2.len() < SAMPLES {
                        e.2.push(id.clone());
                    }
                    e.3 |= h.truncated;
                }
            }
        }
        for ((field, detector, category), (matches, records, samples, truncated)) in &found {
            let note = truncated.then_some(TRUNCATED_NOTE);
            tx.execute(
                "INSERT INTO content_findings (id, app_slug, table_id, field, detector_id, category, source, match_count, record_count, sample_record_ids, note) \
                 VALUES ($1, $2, $3, $4, $5, $6, 'scan', $7, $8, $9, $10) \
                 ON CONFLICT (app_slug, table_id, field, detector_id) DO UPDATE SET \
                    category = EXCLUDED.category, match_count = content_findings.match_count + EXCLUDED.match_count, \
                    record_count = content_findings.record_count + EXCLUDED.record_count, \
                    sample_record_ids = (SELECT COALESCE(array_agg(x), '{}') FROM (SELECT DISTINCT x FROM unnest(content_findings.sample_record_ids || EXCLUDED.sample_record_ids) AS x LIMIT 5) s), \
                    note = COALESCE(EXCLUDED.note, content_findings.note), last_seen = NOW()",
                &[&Uuid::new_v4(), &app, &table, field, detector, category, matches, records, samples, &note],
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

/// Leaves findings for what a write, an import or a file held. A match is counted and a record id is kept, and no value is. A repeat adds to the
/// counts of the finding that is there.
pub fn record_hits(repo: &PostgresRepository, scope: &Scope, hits: &[Hit], record_id: Option<&str>, source: &str) -> Result<(), RepositoryError> {
    let samples: Vec<String> = record_id.iter().map(|s| s.to_string()).collect();
    record_aggregate(repo, scope, &merge_hits(hits), &samples, source)
}

/// One finding's worth of hits: a field and a detector, with the matches and the number of rows or records they were in.
#[derive(Debug, Clone)]
pub struct Merged {
    pub hit: Hit,
    pub records: i64,
    /// What the finding says of how much was read, when only part was.
    pub note: Option<String>,
}

/// Rows of hits, one `Hit` for each row, become one entry for each field and detector: the matches added up and the rows counted.
pub fn merge_hits(hits: &[Hit]) -> Vec<Merged> {
    let mut by: BTreeMap<(String, String, String), Merged> = BTreeMap::new();
    for h in hits {
        let e = by.entry((h.field.clone(), h.detector_id.clone(), h.category.clone())).or_insert_with(|| Merged { hit: Hit { matches: 0, truncated: false, ..h.clone() }, records: 0, note: None });
        e.hit.matches += h.matches;
        e.hit.truncated |= h.truncated;
        if h.truncated {
            e.note = Some(TRUNCATED_NOTE.to_string());
        }
        e.records += 1;
    }
    by.into_values().collect()
}

/// Writes merged findings. `samples` are the record ids to keep, up to five.
pub fn record_aggregate(repo: &PostgresRepository, scope: &Scope, merged: &[Merged], samples: &[String], source: &str) -> Result<(), RepositoryError> {
    if merged.is_empty() {
        return Ok(());
    }
    let (app, table, source, samples, merged) = (scope.app_slug.clone(), scope.table_id.clone(), source.to_string(), samples.to_vec(), merged.to_vec());
    repo.with_client(move |c| {
        let mut tx = c.transaction()?;
        for m in &merged {
            let h = &m.hit;
            let note = m.note.as_deref();
            tx.execute(
                "INSERT INTO content_findings (id, app_slug, table_id, field, detector_id, category, source, match_count, record_count, sample_record_ids, note) \
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11) \
                 ON CONFLICT (app_slug, table_id, field, detector_id) DO UPDATE SET \
                    match_count = content_findings.match_count + EXCLUDED.match_count, record_count = content_findings.record_count + EXCLUDED.record_count, \
                    sample_record_ids = (SELECT COALESCE(array_agg(x), '{}') FROM (SELECT DISTINCT x FROM unnest(content_findings.sample_record_ids || EXCLUDED.sample_record_ids) AS x LIMIT 5) s), \
                    note = COALESCE(EXCLUDED.note, content_findings.note), last_seen = NOW()",
                &[&Uuid::new_v4(), &app, &table, &h.field, &h.detector_id, &h.category, &source, &h.matches, &m.records, &samples, &note],
            )?;
        }
        tx.commit()?;
        Ok(())
    })
}

/// The authorized stores of a category, in words, for the message of a refused write.
pub fn stores_in_words(state: &SharedState, category: &str) -> Vec<String> {
    let stores = state.authorized_stores.read().map(|s| s.clone()).unwrap_or_default();
    stores
        .iter()
        .filter(|s| s.category == category)
        .map(|s| match s.scope.as_str() {
            "workspace" => format!("workspace {}", s.workspace_id),
            "app" => format!("app {}", s.app_slug),
            "table" => format!("table {} of {}", s.table_id, s.app_slug),
            _ => format!("field {} of {} in {}", s.field, s.table_id, s.app_slug),
        })
        .collect()
}

/// Bytes of a plain-text file the content scan reads.
pub const FILE_READ_LIMIT: usize = 4 * 1024 * 1024;
const FILE_NOTE: &str = "Only the first 4 MB of this file was read.";

/// Whether the scan reads this kind of file: plain text by its name.
fn readable(name: &str) -> bool {
    let name = name.to_lowercase();
    ["txt", "csv", "json", "md"].iter().any(|ext| name.rsplit_once('.').is_some_and(|(_, e)| e == *ext))
}

/// The job `scan_file_content`. After the antivirus result, it reads the text of a plain-text attachment and leaves findings with the source
/// `file`. Other types are `unsupported` and are never reported as clean. A file is read once, and only when a record holds it.
pub fn file_job(ctx: &JobCtx, payload: Value) -> Result<Value, JobError> {
    let ref_id = payload.get("ref_id").and_then(Value::as_str).and_then(|s| s.parse::<Uuid>().ok()).ok_or_else(|| JobError::Permanent("the job has no ref_id".to_string()))?;
    let repo = ctx.repository();
    let Some(file) = repo.file_ref_any(ref_id).map_err(retry)? else { return ctx.result(json!({ "skipped": "the file is gone" })) };
    // Not yet read, held by a record, and past the antivirus scan.
    if file.deleted || !file.attached || file.content_scan_state != "skipped" || !matches!(file.scan_state.as_str(), "clean" | "skipped") {
        return ctx.result(json!({ "skipped": "not ready to read" }));
    }
    if !readable(&file.name) {
        repo.set_content_scan_state(ref_id, "unsupported").map_err(retry)?;
        return ctx.result(json!({ "state": "unsupported" }));
    }
    let (Some(table), Some(field)) = (file.table_id.clone(), file.field.clone()) else { return ctx.result(json!({ "skipped": "not held by a field" })) };
    let Some(dir) = crate::files::dir() else { return Err(JobError::Retry("file storage is not set up".to_string())) };
    let mut bytes = Vec::new();
    {
        use std::io::Read;
        let f = std::fs::File::open(crate::files::blob_path(&dir, &file.sha256)).map_err(retry)?;
        f.take(FILE_READ_LIMIT as u64 + 1).read_to_end(&mut bytes).map_err(retry)?;
    }
    let truncated = bytes.len() > FILE_READ_LIMIT;
    bytes.truncate(FILE_READ_LIMIT);
    let text = String::from_utf8_lossy(&bytes);
    let scope = Scope { app_slug: file.app_slug.clone(), table_id: table };
    let plan = match ctx.state() {
        Some(state) => plan(state, &scope),
        None => plan_from_repo(repo, &scope),
    };
    let Ok(plan) = plan else {
        repo.set_content_scan_state(ref_id, "skipped").map_err(retry)?;
        return ctx.result(json!({ "skipped": "the table is gone" }));
    };
    // The whole text is read in one piece, up to the file limit. The field is the attachment field, for the authorized stores.
    let checked = plan.check_whole(&field, &text);
    let mut merged = merge_hits(&checked.hits);
    for m in &mut merged {
        m.note = truncated.then(|| FILE_NOTE.to_string());
    }
    let record = file.record_id.clone();
    record_aggregate(repo, &scope, &merged, &record.iter().cloned().collect::<Vec<_>>(), "file").map_err(retry)?;
    let state = if checked.hits.is_empty() { "clean" } else { "found" };
    repo.set_content_scan_state(ref_id, state).map_err(retry)?;
    ctx.result(json!({ "state": state, "truncated": truncated }))
}

/// Queues the content scan of a file. One is queued for each file, and none is queued again while it waits.
pub fn enqueue_file(repo: &PostgresRepository, ref_id: Uuid) {
    let _ = crate::jobs::enqueue(repo, "scan_file_content", json!({ "ref_id": ref_id }), "scanner@scaffoldry.internal", Some(&format!("scan_file_content:{ref_id}")));
}
