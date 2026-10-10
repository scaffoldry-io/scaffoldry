//! A durable job queue on PostgreSQL.
//!
//! Work that takes longer than a request becomes a row in `jobs`. Worker threads in this process
//! claim rows with `FOR UPDATE SKIP LOCKED`. Each worker owns one database connection.
//!
//! Handlers are at-least-once. A worker that stops mid-job loses its lock after a while, and
//! another worker runs the job again from its last checkpoint. Write every handler so it can be
//! resumed, and call `JobCtx::cancelled` between batches. That call also renews the lock, so a
//! single batch must finish well inside `JobConfig::lock_secs`.

use crate::repository::{PostgresRepository, RepositoryError};
use postgres::Client;
use serde::Serialize;
use serde_json::Value;
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};
use uuid::Uuid;

/// Serializes claims, so a per-kind cap is exact when several workers claim at once.
const CLAIM_LOCK: i64 = 742_200;
pub const SCHEDULER_LOCK: i64 = 742_201;
const LOG_LINES: usize = 200;
/// A job never carries these. It stores ids and counts.
const FORBIDDEN_KEYS: [&str; 3] = ["token", "secret", "password"];

#[derive(Debug, thiserror::Error)]
pub enum JobsError {
    #[error("Database error: {0}")]
    Db(#[from] postgres::Error),
    #[error("{0}")]
    Repository(#[from] RepositoryError),
    #[error("Refused: {0}")]
    Refused(String),
    #[error("Conflict: {0}")]
    Conflict(String),
    #[error("Not found: {0}")]
    NotFound(String),
}

/// What a handler returns when it does not succeed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JobError {
    /// Try again later, up to the kind's `max_attempts`.
    Retry(String),
    /// Fail now. Retrying would not help.
    Permanent(String),
    /// The handler saw `cancelled()` and stopped.
    Cancelled,
}

impl From<postgres::Error> for JobError {
    fn from(e: postgres::Error) -> Self {
        JobError::Retry(e.to_string())
    }
}

pub struct JobKind {
    pub name: &'static str,
    pub max_attempts: i32,
    pub max_concurrent: usize,
    pub run: fn(&JobCtx, Value) -> Result<Value, JobError>,
}

/// Every kind of job this server can run. Later briefs add one entry each.
pub static JOB_KINDS: &[JobKind] = &[];

#[derive(Debug, Clone)]
pub struct JobConfig {
    pub workers: usize,
    pub lock_secs: i64,
    pub heartbeat_secs: u64,
    pub recovery_secs: u64,
    pub poll_ms: u64,
    /// Delay before the first, second, and third retry.
    pub backoff_secs: [i64; 3],
    pub scheduler_secs: u64,
}

impl Default for JobConfig {
    fn default() -> Self {
        let workers = std::env::var("SCAFFOLDRY_JOB_WORKERS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(2);
        Self {
            workers,
            lock_secs: 60,
            heartbeat_secs: 20,
            recovery_secs: 30,
            poll_ms: 500,
            backoff_secs: [30, 300, 1800],
            scheduler_secs: 15,
        }
    }
}

/// What a person may read about a job. The payload and the lock are not part of it.
#[derive(Debug, Clone, Serialize)]
pub struct Job {
    pub id: Uuid,
    pub kind: String,
    pub state: String,
    pub progress_done: i64,
    pub progress_total: Option<i64>,
    pub result: Option<Value>,
    pub error: Option<String>,
    pub log: Value,
    pub attempts: i32,
    pub cancel_requested: bool,
    pub created_by: String,
    pub created_at: String,
    pub started_at: Option<String>,
    pub finished_at: Option<String>,
}

const JOB_COLUMNS: &str = "id, kind, state, progress_done, progress_total, result, error, log, attempts, \
     cancel_requested, created_by, created_at::text, started_at::text, finished_at::text";

fn job_from_row(row: &postgres::Row) -> Job {
    Job {
        id: row.get(0),
        kind: row.get(1),
        state: row.get(2),
        progress_done: row.get(3),
        progress_total: row.get(4),
        result: row.get(5),
        error: row.get(6),
        log: row.get(7),
        attempts: row.get(8),
        cancel_requested: row.get(9),
        created_by: row.get(10),
        created_at: row.get(11),
        started_at: row.get(12),
        finished_at: row.get(13),
    }
}

/// Refuses a value that holds a key named like a secret, at any depth.
pub fn check_clean(value: &Value) -> Result<(), String> {
    match value {
        Value::Object(map) => {
            for (key, inner) in map {
                if FORBIDDEN_KEYS.contains(&key.to_lowercase().as_str()) {
                    return Err(format!("a job may not carry a key named '{key}'. Store ids and counts"));
                }
                check_clean(inner)?;
            }
            Ok(())
        }
        Value::Array(items) => items.iter().try_for_each(check_clean),
        _ => Ok(()),
    }
}

/// The one way to create a job. With a `dedupe_key`, a queued or running job that holds the
/// key is returned instead of a new one.
pub fn enqueue(
    repo: &PostgresRepository,
    kind: &str,
    payload: Value,
    created_by: &str,
    dedupe_key: Option<&str>,
) -> Result<Uuid, JobsError> {
    check_clean(&payload).map_err(JobsError::Refused)?;
    let kind = kind.to_string();
    let created_by = created_by.to_string();
    let key = dedupe_key.map(str::to_string);
    Ok(repo.with_client(move |c| {
        for _ in 0..3 {
            let id = Uuid::new_v4();
            let inserted = c.query_opt(
                "INSERT INTO jobs (id, kind, payload, created_by, dedupe_key) VALUES ($1, $2, $3, $4, $5) \
                 ON CONFLICT (dedupe_key) WHERE dedupe_key IS NOT NULL AND state IN ('queued', 'running') \
                 DO NOTHING RETURNING id",
                &[&id, &kind, &payload, &created_by, &key],
            )?;
            if let Some(row) = inserted {
                return Ok(row.get::<_, Uuid>(0));
            }
            if let Some(k) = &key {
                let live = c.query_opt(
                    "SELECT id FROM jobs WHERE dedupe_key = $1 AND state IN ('queued', 'running')",
                    &[k],
                )?;
                if let Some(row) = live {
                    return Ok(row.get::<_, Uuid>(0));
                }
            }
            // The holder finished between the insert and the read. Try again.
        }
        Err(RepositoryError::NotFound("could not enqueue the job".to_string()))
    })?)
}

pub fn get_job(repo: &PostgresRepository, id: Uuid) -> Result<Option<Job>, JobsError> {
    Ok(repo.with_client(move |c| {
        let row = c.query_opt(&format!("SELECT {JOB_COLUMNS} FROM jobs WHERE id = $1"), &[&id])?;
        Ok(row.map(|r| job_from_row(&r)))
    })?)
}

/// The caller's own jobs, newest first.
pub fn list_jobs(repo: &PostgresRepository, created_by: &str, limit: i64, offset: i64) -> Result<Vec<Job>, JobsError> {
    let created_by = created_by.to_string();
    Ok(repo.with_client(move |c| {
        let rows = c.query(
            &format!(
                "SELECT {JOB_COLUMNS} FROM jobs WHERE created_by = $1 ORDER BY created_at DESC, id LIMIT $2 OFFSET $3"
            ),
            &[&created_by, &limit, &offset],
        )?;
        Ok(rows.iter().map(job_from_row).collect())
    })?)
}

/// A queued job is cancelled at once. A running job is asked to stop at its next batch.
/// A finished job is left alone.
pub fn cancel_job(repo: &PostgresRepository, id: Uuid) -> Result<Option<Job>, JobsError> {
    Ok(repo.with_client(move |c| {
        let row = c.query_opt(
            &format!(
                "UPDATE jobs SET \
                   state = CASE WHEN state = 'queued' THEN 'cancelled' ELSE state END, \
                   cancel_requested = CASE WHEN state IN ('queued', 'running') THEN TRUE ELSE cancel_requested END, \
                   finished_at = CASE WHEN state = 'queued' THEN NOW() ELSE finished_at END \
                 WHERE id = $1 RETURNING {JOB_COLUMNS}"
            ),
            &[&id],
        )?;
        Ok(row.map(|r| job_from_row(&r)))
    })?)
}


/// Registers a recurring schedule. Inserts at boot if missing, preserving next_run_at if already present.
pub fn register_schedule(
    repo: &PostgresRepository,
    kind: &str,
    every_seconds: i32,
) -> Result<(), JobsError> {
    let kind = kind.to_string();
    repo.with_client(move |c| {
        c.execute(
            "INSERT INTO job_schedules (kind, every_seconds, next_run_at, enabled) \
             VALUES ($1, $2, NOW(), TRUE) \
             ON CONFLICT (kind) DO UPDATE SET every_seconds = EXCLUDED.every_seconds \
             WHERE job_schedules.every_seconds != EXCLUDED.every_seconds",
            &[&kind, &every_seconds],
        )?;
        Ok(())
    })?;
    Ok(())
}

pub fn scheduler_tick_client(
    client: &mut Client,
    disabled_kinds: &[String],
) -> Result<usize, JobsError> {
    let mut tx = client.transaction()?;
    let locked: bool = tx
        .query_one("SELECT pg_try_advisory_xact_lock($1)", &[&SCHEDULER_LOCK])?
        .get(0);
    if !locked {
        tx.rollback()?;
        return Ok(0);
    }

    let rows = tx.query(
        "SELECT kind, every_seconds, next_run_at FROM job_schedules \
         WHERE enabled = TRUE AND next_run_at <= NOW() \
         ORDER BY next_run_at FOR UPDATE",
        &[],
    )?;

    let now = chrono::Utc::now();
    let mut spawned = 0;

    for row in rows {
        let kind: String = row.get(0);
        let every_seconds: i32 = row.get(1);
        let old_next_run: chrono::DateTime<chrono::Utc> = row.get(2);

        let elapsed_secs = (now - old_next_run).num_seconds();
        let every_secs = every_seconds as i64;
        let steps = if elapsed_secs < 0 {
            1
        } else {
            (elapsed_secs / every_secs) + 1
        };
        let new_next_run = old_next_run + chrono::Duration::seconds(steps * every_secs);

        tx.execute(
            "UPDATE job_schedules SET next_run_at = $1 WHERE kind = $2",
            &[&new_next_run, &kind],
        )?;

        if disabled_kinds.contains(&kind) {
            continue;
        }

        let job_id = Uuid::new_v4();
        let inserted = tx.query_opt(
            "INSERT INTO jobs (id, kind, payload, created_by, dedupe_key) \
             VALUES ($1, $2, '{}'::jsonb, 'scheduler@scaffoldry.internal', $2) \
             ON CONFLICT (dedupe_key) WHERE dedupe_key IS NOT NULL AND state IN ('queued', 'running') \
             DO NOTHING RETURNING id",
            &[&job_id, &kind],
        )?;
        if inserted.is_some() {
            spawned += 1;
        }
    }

    tx.commit()?;
    Ok(spawned)
}

pub fn scheduler_tick(
    repo: &PostgresRepository,
    disabled_kinds: &[String],
) -> Result<usize, JobsError> {
    let mut client = repo.dedicated_client()?;
    scheduler_tick_client(&mut client, disabled_kinds)
}

pub fn retry_job(repo: &PostgresRepository, id: Uuid) -> Result<Job, JobsError> {
    repo.with_client(move |c| {
        let existing = c.query_opt(
            &format!("SELECT {JOB_COLUMNS} FROM jobs WHERE id = $1 FOR UPDATE"),
            &[&id],
        )?;
        let Some(row) = existing else {
            return Err(RepositoryError::NotFound(format!("Job {id} not found")));
        };
        let job = job_from_row(&row);
        if job.state != "failed" {
            return Err(RepositoryError::Conflict(format!(
                "Job {id} is in state '{}' and cannot be retried",
                job.state
            )));
        }
        let updated = c.query_one(
            &format!(
                "UPDATE jobs SET state = 'queued', attempts = 0, error = NULL, \
                        finished_at = NULL, run_after = NOW() \
                 WHERE id = $1 RETURNING {JOB_COLUMNS}"
            ),
            &[&id],
        )?;
        Ok(job_from_row(&updated))
    })
    .map_err(|e| match e {
        RepositoryError::Conflict(msg) => JobsError::Conflict(msg),
        RepositoryError::NotFound(msg) => JobsError::NotFound(msg),
        other => JobsError::Repository(other),
    })
}

#[derive(Debug, Clone, Serialize)]
pub struct AdminJobRow {
    pub id: Uuid,
    pub kind: String,
    pub owner: String,
    pub state: String,
    pub progress: Option<f64>,
    pub attempts: i32,
    pub created: String,
    pub age: i64,
    pub last_log_line: Option<String>,
}

pub fn list_admin_jobs(
    repo: &PostgresRepository,
    state_filter: Option<&str>,
    kind_filter: Option<&str>,
    owner_filter: Option<&str>,
    limit: usize,
    offset: usize,
) -> Result<Vec<AdminJobRow>, JobsError> {
    let state_filter = state_filter.map(str::to_string);
    let kind_filter = kind_filter.map(str::to_string);
    let owner_filter = owner_filter.map(str::to_string);
    let limit = limit as i64;
    let offset = offset as i64;

    Ok(repo.with_client(move |c| {
        let rows = c.query(
            "SELECT id, kind, created_by, state, progress_done, progress_total, attempts, \
                    created_at::text, EXTRACT(EPOCH FROM (NOW() - created_at))::bigint, log \
             FROM jobs \
             WHERE ($1::text IS NULL OR state = $1) \
               AND ($2::text IS NULL OR kind = $2) \
               AND ($3::text IS NULL OR created_by = $3) \
             ORDER BY created_at DESC, id DESC \
             LIMIT $4 OFFSET $5",
            &[&state_filter, &kind_filter, &owner_filter, &limit, &offset],
        )?;

        let mut out = Vec::with_capacity(rows.len());
        for r in rows {
            let done: i64 = r.get(4);
            let total: Option<i64> = r.get(5);
            let progress = total.filter(|&t| t > 0).map(|t| (done as f64 / t as f64) * 100.0);
            let log_val: Value = r.get(9);
            let last_log_line = log_val
                .as_array()
                .and_then(|arr| arr.last())
                .and_then(|v| v.as_str())
                .map(String::from);

            out.push(AdminJobRow {
                id: r.get(0),
                kind: r.get(1),
                owner: r.get(2),
                state: r.get(3),
                progress,
                attempts: r.get(6),
                created: r.get(7),
                age: r.get(8),
                last_log_line,
            });
        }
        Ok(out)
    })?)
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct JobsOverview {
    pub queue_depth: i64,
    pub oldest_queued_age_secs: Option<i64>,
    pub failed_last_24h: i64,
}

pub fn get_jobs_overview(repo: &PostgresRepository) -> Result<JobsOverview, JobsError> {
    Ok(repo.with_client(|c| {
        let q_row = c.query_one(
            "SELECT COUNT(*)::bigint, \
                    EXTRACT(EPOCH FROM (NOW() - MIN(created_at)))::bigint \
             FROM jobs WHERE state = 'queued'",
            &[],
        )?;
        let queue_depth: i64 = q_row.get(0);
        let oldest_queued_age_secs: Option<i64> = q_row.get(1);

        let f_row = c.query_one(
            "SELECT COUNT(*)::bigint FROM jobs \
             WHERE state = 'failed' AND finished_at >= NOW() - INTERVAL '24 hours'",
            &[],
        )?;
        let failed_last_24h: i64 = f_row.get(0);

        Ok(JobsOverview {
            queue_depth,
            oldest_queued_age_secs,
            failed_last_24h,
        })
    })?)
}

#[derive(Debug, Clone)]
pub struct ClaimedJob {
    pub id: Uuid,
    pub kind: String,
    pub payload: Value,
    pub checkpoint: Option<Value>,
    pub attempts: i32,
}

/// Claims the oldest runnable job of a known kind, skipping kinds already at their cap.
pub fn claim_next(
    client: &mut Client,
    worker: &str,
    kinds: &'static [JobKind],
    cfg: &JobConfig,
) -> Result<Option<ClaimedJob>, JobsError> {
    if kinds.is_empty() {
        return Ok(None);
    }
    let mut tx = client.transaction()?;
    tx.query("SELECT pg_advisory_xact_lock($1)", &[&CLAIM_LOCK])?;

    let mut running: HashMap<String, i64> = HashMap::new();
    for row in tx.query("SELECT kind, COUNT(*) FROM jobs WHERE state = 'running' GROUP BY kind", &[])? {
        running.insert(row.get(0), row.get(1));
    }
    let known: Vec<String> = kinds.iter().map(|k| k.name.to_string()).collect();
    let capped: Vec<String> = kinds
        .iter()
        .filter(|k| running.get(k.name).copied().unwrap_or(0) >= k.max_concurrent as i64)
        .map(|k| k.name.to_string())
        .collect();

    let lock_secs = cfg.lock_secs as f64;
    let row = tx.query_opt(
        "UPDATE jobs SET state = 'running', locked_by = $1, \
                locked_until = NOW() + make_interval(secs => $2), \
                attempts = attempts + 1, started_at = COALESCE(started_at, NOW()) \
         WHERE id = ( \
             SELECT id FROM jobs WHERE state = 'queued' AND run_after <= NOW() \
               AND kind = ANY($3) AND NOT (kind = ANY($4)) \
             ORDER BY run_after, created_at FOR UPDATE SKIP LOCKED LIMIT 1) \
         RETURNING id, kind, payload, checkpoint, attempts",
        &[&worker, &lock_secs, &known, &capped],
    )?;
    let claimed = row.map(|r| ClaimedJob {
        id: r.get(0),
        kind: r.get(1),
        payload: r.get(2),
        checkpoint: r.get(3),
        attempts: r.get(4),
    });
    if let Some(job) = &claimed {
        if let Some(kind) = kinds.iter().find(|k| k.name == job.kind) {
            tx.execute("UPDATE jobs SET max_attempts = $2 WHERE id = $1", &[&job.id, &kind.max_attempts])?;
        }
    }
    tx.commit()?;
    Ok(claimed)
}

/// Returns running jobs whose lock has expired to the queue, or fails them when they are out of
/// attempts. One worker runs this every `recovery_secs`.
pub fn recover(client: &mut Client, cfg: &JobConfig) -> Result<u64, JobsError> {
    let backoff: Vec<i64> = cfg.backoff_secs.to_vec();
    Ok(client.execute(
        "UPDATE jobs SET \
           state = CASE WHEN cancel_requested THEN 'cancelled' \
                        WHEN attempts >= max_attempts THEN 'failed' ELSE 'queued' END, \
           error = COALESCE(error, 'The worker stopped before the job finished'), \
           finished_at = CASE WHEN cancel_requested OR attempts >= max_attempts THEN NOW() END, \
           run_after = NOW() + make_interval(secs => ($1::bigint[])[LEAST(GREATEST(attempts, 1), 3)]), \
           locked_by = NULL, locked_until = NULL \
         WHERE state = 'running' AND locked_until < NOW()",
        &[&backoff],
    )?)
}

/// What a handler sees while it runs. It owns the worker's connection for the length of the job.
pub struct JobCtx {
    id: Uuid,
    worker: String,
    repo: PostgresRepository,
    client: RefCell<Client>,
    checkpoint: RefCell<Option<Value>>,
    lock_secs: f64,
    heartbeat: Duration,
    last_beat: Cell<Instant>,
}

impl JobCtx {
    fn into_client(self) -> Client {
        self.client.into_inner()
    }

    /// The repository, for the data work the job does.
    pub fn repository(&self) -> &PostgresRepository {
        &self.repo
    }

    /// Renews the lock when it is due. Every context call does this.
    fn beat(&self) -> Result<(), JobError> {
        if self.last_beat.get().elapsed() < self.heartbeat {
            return Ok(());
        }
        self.client.borrow_mut().execute(
            "UPDATE jobs SET locked_until = NOW() + make_interval(secs => $3) WHERE id = $1 AND locked_by = $2",
            &[&self.id, &self.worker, &self.lock_secs],
        )?;
        self.last_beat.set(Instant::now());
        Ok(())
    }

    pub fn progress(&self, done: i64, total: i64) -> Result<(), JobError> {
        self.beat()?;
        self.client.borrow_mut().execute(
            "UPDATE jobs SET progress_done = $2, progress_total = $3 WHERE id = $1",
            &[&self.id, &done, &total],
        )?;
        Ok(())
    }

    /// Saves where the job is, so a later attempt resumes from here.
    pub fn checkpoint(&self, value: Value) -> Result<(), JobError> {
        check_clean(&value).map_err(JobError::Permanent)?;
        self.beat()?;
        self.client
            .borrow_mut()
            .execute("UPDATE jobs SET checkpoint = $2 WHERE id = $1", &[&self.id, &value])?;
        *self.checkpoint.borrow_mut() = Some(value);
        Ok(())
    }

    pub fn load_checkpoint(&self) -> Option<Value> {
        self.checkpoint.borrow().clone()
    }

    /// True once a person has asked to cancel. Call it between batches.
    pub fn cancelled(&self) -> Result<bool, JobError> {
        self.beat()?;
        let row = self
            .client
            .borrow_mut()
            .query_one("SELECT cancel_requested FROM jobs WHERE id = $1", &[&self.id])?;
        Ok(row.get(0))
    }

    /// Appends a line to the job's log. The log keeps the last 200 lines.
    pub fn log(&self, line: &str) -> Result<(), JobError> {
        self.beat()?;
        let mut client = self.client.borrow_mut();
        let row = client.query_one("SELECT log FROM jobs WHERE id = $1", &[&self.id])?;
        let mut lines: Vec<Value> = row.get::<_, Value>(0).as_array().cloned().unwrap_or_default();
        lines.push(Value::String(line.to_string()));
        if lines.len() > LOG_LINES {
            lines.drain(..lines.len() - LOG_LINES);
        }
        client.execute("UPDATE jobs SET log = $2 WHERE id = $1", &[&self.id, &Value::Array(lines)])?;
        Ok(())
    }

    /// Checks a result before it is stored. A result with a secret-like key fails the job.
    pub fn result(&self, value: Value) -> Result<Value, JobError> {
        check_clean(&value).map_err(JobError::Permanent)?;
        Ok(value)
    }
}

fn finish(client: &mut Client, job: &ClaimedJob, worker: &str, outcome: Result<Value, JobError>, cfg: &JobConfig) {
    let outcome = match outcome {
        Ok(value) => match check_clean(&value) {
            Ok(()) => Ok(value),
            Err(why) => Err(JobError::Permanent(why)),
        },
        Err(e) => Err(e),
    };
    let done = match outcome {
        Ok(value) => client.execute(
            "UPDATE jobs SET state = 'done', result = $3, finished_at = NOW(), locked_by = NULL, locked_until = NULL \
             WHERE id = $1 AND locked_by = $2",
            &[&job.id, &worker, &value],
        ),
        Err(JobError::Cancelled) => client.execute(
            "UPDATE jobs SET state = 'cancelled', finished_at = NOW(), locked_by = NULL, locked_until = NULL \
             WHERE id = $1 AND locked_by = $2",
            &[&job.id, &worker],
        ),
        Err(JobError::Permanent(why)) => client.execute(
            "UPDATE jobs SET state = 'failed', error = $3, finished_at = NOW(), locked_by = NULL, locked_until = NULL \
             WHERE id = $1 AND locked_by = $2",
            &[&job.id, &worker, &why],
        ),
        Err(JobError::Retry(why)) => {
            let backoff: Vec<i64> = cfg.backoff_secs.to_vec();
            client.execute(
                "UPDATE jobs SET error = $3, locked_by = NULL, locked_until = NULL, \
                   state = CASE WHEN cancel_requested THEN 'cancelled' \
                                WHEN attempts >= max_attempts THEN 'failed' ELSE 'queued' END, \
                   finished_at = CASE WHEN cancel_requested OR attempts >= max_attempts THEN NOW() END, \
                   run_after = NOW() + make_interval(secs => ($4::bigint[])[LEAST(GREATEST(attempts, 1), 3)]) \
                 WHERE id = $1 AND locked_by = $2",
                &[&job.id, &worker, &why, &backoff],
            )
        }
    };
    if let Err(e) = done {
        eprintln!("Warning: could not record the outcome of job {}: {e}", job.id);
    }
}

fn run_job(
    client: Client,
    repo: &PostgresRepository,
    kinds: &'static [JobKind],
    cfg: &JobConfig,
    worker: &str,
    job: ClaimedJob,
) -> Client {
    let Some(kind) = kinds.iter().find(|k| k.name == job.kind) else {
        return client;
    };
    let ctx = JobCtx {
        id: job.id,
        worker: worker.to_string(),
        repo: repo.clone(),
        client: RefCell::new(client),
        checkpoint: RefCell::new(job.checkpoint.clone()),
        lock_secs: cfg.lock_secs as f64,
        heartbeat: Duration::from_secs(cfg.heartbeat_secs),
        last_beat: Cell::new(Instant::now()),
    };
    let outcome = match catch_unwind(AssertUnwindSafe(|| (kind.run)(&ctx, job.payload.clone()))) {
        Ok(result) => result,
        Err(_) => Err(JobError::Retry("The handler panicked".to_string())),
    };
    let mut client = ctx.into_client();
    finish(&mut client, &job, worker, outcome, cfg);
    client
}

fn worker_loop(index: usize, repo: PostgresRepository, kinds: &'static [JobKind], cfg: JobConfig, stop: Arc<AtomicBool>) {
    let worker = format!("w{index}-{}", &Uuid::new_v4().simple().to_string()[..8]);
    let mut client = match repo.dedicated_client() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("Warning: job worker {index} could not connect: {e}");
            return;
        }
    };
    let poll = Duration::from_millis(cfg.poll_ms);
    let mut next_recovery = Instant::now();
    while !stop.load(Ordering::SeqCst) {
        // One worker runs the recovery pass.
        if index == 0 && Instant::now() >= next_recovery {
            if let Err(e) = recover(&mut client, &cfg) {
                eprintln!("Warning: job recovery failed: {e}");
            }
            next_recovery = Instant::now() + Duration::from_secs(cfg.recovery_secs);
        }
        match claim_next(&mut client, &worker, kinds, &cfg) {
            Ok(Some(job)) => client = run_job(client, &repo, kinds, &cfg, &worker, job),
            Ok(None) => std::thread::sleep(poll),
            Err(e) => {
                eprintln!("Warning: job claim failed: {e}");
                std::thread::sleep(poll);
            }
        }
    }
}

/// The worker threads. Dropping it, or calling `stop`, lets each worker finish its current
/// batch and then exit.
pub struct JobRunner {
    stop: Arc<AtomicBool>,
    threads: Vec<JoinHandle<()>>,
}

impl JobRunner {
    pub fn start(repo: &PostgresRepository, kinds: &'static [JobKind], cfg: JobConfig) -> Result<Self, JobsError> {
        let stop = Arc::new(AtomicBool::new(false));
        let mut threads = Vec::new();
        for index in 0..cfg.workers.max(1) {
            let (repo, cfg, stop) = (repo.clone(), cfg.clone(), stop.clone());
            let handle = std::thread::Builder::new()
                .name(format!("scaffoldry-job-worker-{index}"))
                .spawn(move || worker_loop(index, repo, kinds, cfg, stop))
                .map_err(|e| JobsError::Refused(format!("could not start a job worker: {e}")))?;
            threads.push(handle);
        }
        let (s_repo, s_cfg, s_stop) = (repo.clone(), cfg.clone(), stop.clone());
        let sched_handle = std::thread::Builder::new()
            .name("scaffoldry-job-scheduler".to_string())
            .spawn(move || {
                let mut client = match s_repo.dedicated_client() {
                    Ok(c) => c,
                    Err(e) => {
                        eprintln!("Warning: job scheduler could not connect: {e}");
                        return;
                    }
                };
                let poll = Duration::from_secs(s_cfg.scheduler_secs);
                while !s_stop.load(Ordering::SeqCst) {
                    let disabled: Vec<String> = s_repo
                        .get_platform_settings()
                        .ok()
                        .and_then(|s| s.get("jobs.schedules_disabled").cloned())
                        .and_then(|v| v.as_array().cloned())
                        .map(|arr| {
                            arr.iter()
                                .filter_map(|x| x.as_str().map(String::from))
                                .collect()
                        })
                        .unwrap_or_default();

                    if let Err(e) = scheduler_tick_client(&mut client, &disabled) {
                        eprintln!("Warning: job scheduler tick failed: {e}");
                    }
                    let start = Instant::now();
                    while !s_stop.load(Ordering::SeqCst) && Instant::now().duration_since(start) < poll {
                        std::thread::sleep(Duration::from_millis(50));
                    }
                }
            })
            .map_err(|e| JobsError::Refused(format!("could not start job scheduler: {e}")))?;
        threads.push(sched_handle);
        Ok(Self { stop, threads })
    }

    pub fn stop(mut self) {
        self.shutdown();
    }

    fn shutdown(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        for handle in self.threads.drain(..) {
            let _ = handle.join();
        }
    }
}

impl Drop for JobRunner {
    fn drop(&mut self) {
        self.shutdown();
    }
}
