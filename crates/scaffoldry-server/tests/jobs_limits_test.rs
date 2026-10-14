//! Jobs phase 4: limits and visibility.
//!
//! The tests share one database. Each uses users and kinds of its own. A kind no runner serves
//! stays queued, so every test cancels what it queued.

mod common;

use scaffoldry_server::jobs::{cancel_job, enqueue, get_job, JobConfig, JobCtx, JobError, JobKind, JobRunner, JobsError, JOB_KINDS};
use scaffoldry_server::repository::PostgresRepository;
use serde_json::{json, Value};
use std::time::{Duration, Instant};
use uuid::Uuid;

fn repo() -> PostgresRepository {
    PostgresRepository::connect(None).expect("database is reachable")
}

fn unique(prefix: &str) -> String {
    format!("{prefix}-{}@state.edu", &Uuid::new_v4().to_string()[..8])
}

fn sql(repo: &PostgresRepository, statement: String) {
    repo.with_client(move |c| {
        c.batch_execute(&statement)?;
        Ok(())
    })
    .expect("sql");
}

fn count(repo: &PostgresRepository, statement: String) -> i64 {
    repo.with_client(move |c| Ok(c.query_one(&statement, &[])?.get::<_, i64>(0))).expect("count")
}

// 1. The sixth job from one user is refused with quota_exceeded. A dedupe hit is not a new job. Past
//    the server limit on queued jobs, anyone is refused. Finishing a job frees a place.
#[test]
fn the_sixth_job_from_one_user_and_the_server_limit_are_refused() {
    let _db = common::TestDb::new();
    let repo = repo();
    let user = unique("quota");
    let ids: Vec<Uuid> = (0..5).map(|_| enqueue(&repo, "t_limits_kind", json!({}), &user, None).expect("within the limit")).collect();

    let sixth = enqueue(&repo, "t_limits_kind", json!({}), &user, None);
    match sixth {
        Err(JobsError::Quota(message)) => assert!(message.contains('5'), "the plain sentence names the limit: {message}"),
        other => panic!("the sixth job should be refused with Quota, got {other:?}"),
    }

    // A job that already holds the dedupe key is returned, so it is not counted twice.
    let key = format!("limits-{}", Uuid::new_v4());
    let other = unique("quota-dedupe");
    let first = enqueue(&repo, "t_limits_kind", json!({}), &other, Some(&key)).unwrap();
    for _ in 0..4 {
        enqueue(&repo, "t_limits_kind", json!({}), &other, None).unwrap();
    }
    assert_eq!(enqueue(&repo, "t_limits_kind", json!({}), &other, Some(&key)).unwrap(), first, "the live job is returned at the limit");

    // Cancelling one frees a place.
    cancel_job(&repo, ids[0]).unwrap();
    assert!(enqueue(&repo, "t_limits_kind", json!({}), &user, None).is_ok(), "a place is free again");

    // The server limit counts queued jobs from everyone.
    let queued = count(&repo, "SELECT count(*) FROM jobs WHERE state = 'queued'".to_string());
    std::env::set_var("SCAFFOLDRY_JOB_LIMIT_QUEUED", (queued + 1).to_string());
    let a = enqueue(&repo, "t_limits_kind", json!({}), &unique("server"), None);
    let b = enqueue(&repo, "t_limits_kind", json!({}), &unique("server"), None);
    std::env::remove_var("SCAFFOLDRY_JOB_LIMIT_QUEUED");
    assert!(a.is_ok(), "one place was left");
    match b {
        Err(JobsError::Quota(message)) => assert!(message.to_lowercase().contains("busy"), "{message}"),
        other => panic!("the server limit should refuse, got {other:?}"),
    }

    sql(&repo, "UPDATE jobs SET state = 'cancelled', finished_at = NOW() WHERE kind = 't_limits_kind' AND state = 'queued'".to_string());
}

fn fast() -> JobConfig {
    JobConfig { workers: 1, poll_ms: 20, backoff_secs: [0, 0, 0], ..JobConfig::default() }
}

fn purge_kind() -> Vec<JobKind> {
    JOB_KINDS.iter().filter(|k| k.name == "purge_jobs").map(|k| JobKind { name: k.name, title: k.title, description: k.description, max_attempts: k.max_attempts, max_concurrent: k.max_concurrent, run: k.run }).collect()
}

// 2. purge_jobs removes a 31-day-old done job and keeps a 31-day-old failed one. A failed job goes
//    after 90 days. A recent one stays.
#[test]
fn purge_jobs_keeps_failed_jobs_longer_than_finished_ones() {
    let _db = common::TestDb::new();
    let repo = repo();
    let tag = Uuid::new_v4().to_string()[..8].to_string();
    let creator = unique("purge");
    let insert = |state: &str, age_days: i32| -> Uuid {
        let id = Uuid::new_v4();
        sql(&repo, format!("INSERT INTO jobs (id, kind, payload, created_by, state, finished_at) VALUES ('{id}', 't_purge_{tag}', '{{}}', '{creator}', '{state}', NOW() - interval '{age_days} days')"));
        id
    };
    let done_old = insert("done", 31);
    let cancelled_old = insert("cancelled", 31);
    let failed_old = insert("failed", 31);
    let failed_very_old = insert("failed", 91);
    let done_recent = insert("done", 29);

    let kinds: &'static [JobKind] = Box::leak(purge_kind().into_boxed_slice());
    assert_eq!(kinds.len(), 1, "purge_jobs is a registered kind");
    let runner = JobRunner::start(&repo, kinds, fast()).expect("runner");
    let id = enqueue(&repo, "purge_jobs", json!({}), "scheduler@scaffoldry.internal", None).expect("enqueue");
    let deadline = Instant::now() + Duration::from_secs(30);
    let job = loop {
        let job = get_job(&repo, id).unwrap().unwrap();
        if job.state == "done" || job.state == "failed" {
            break job;
        }
        assert!(Instant::now() < deadline, "purge_jobs stuck in {}", job.state);
        std::thread::sleep(Duration::from_millis(20));
    };
    drop(runner);
    assert_eq!(job.state, "done", "{:?}", job.error);

    let exists = |id: Uuid| count(&repo, format!("SELECT count(*) FROM jobs WHERE id = '{id}'")) == 1;
    assert!(!exists(done_old), "a 31-day-old done job is deleted");
    assert!(!exists(cancelled_old), "a 31-day-old cancelled job is deleted");
    assert!(exists(failed_old), "a 31-day-old failed job is kept");
    assert!(!exists(failed_very_old), "a 91-day-old failed job is deleted");
    assert!(exists(done_recent), "a 29-day-old done job is kept");
    assert!(job.result.as_ref().is_some_and(|r| r["deleted"].as_i64().unwrap_or(0) >= 3), "the result counts what it deleted: {:?}", job.result);

    sql(&repo, format!("DELETE FROM jobs WHERE kind = 't_purge_{tag}'"));
}

// 3. Every kind has a plain name and a one-sentence description. A job read back carries them.
#[test]
fn every_job_kind_has_a_plain_name_and_a_one_sentence_description() {
    let _db = common::TestDb::new();
    assert!(JOB_KINDS.iter().any(|k| k.name == "purge_jobs"));
    for kind in JOB_KINDS {
        assert!(!kind.title.trim().is_empty(), "{} has no title", kind.name);
        assert_ne!(kind.title, kind.name, "{}: the title is a plain name, not the kind", kind.name);
        let d = kind.description.trim();
        assert!(!d.is_empty(), "{} has no description", kind.name);
        assert!(d.ends_with('.'), "{}: a sentence ends with a full stop: {d}", kind.name);
        assert!(!d[..d.len() - 1].contains(". "), "{}: one sentence only: {d}", kind.name);
    }
}

fn nothing(_ctx: &JobCtx, _p: Value) -> Result<Value, JobError> {
    Ok(json!({}))
}

#[test]
fn a_job_read_back_carries_its_plain_name_and_description() {
    let _db = common::TestDb::new();
    let repo = repo();
    let user = unique("title");
    let id = enqueue(&repo, "recalc_field", json!({}), &user, None).unwrap();
    let job = get_job(&repo, id).unwrap().unwrap();
    let kind = JOB_KINDS.iter().find(|k| k.name == "recalc_field").unwrap();
    assert_eq!(job.title, kind.title);
    assert_eq!(job.description, kind.description);

    // A kind that is not registered still reads back, under its own name.
    let id = enqueue(&repo, "t_limits_unknown", json!({}), &user, None).unwrap();
    let job = get_job(&repo, id).unwrap().unwrap();
    assert_eq!(job.title, "t_limits_unknown");
    assert_eq!(job.description, "");
    let _ = nothing;
    sql(&repo, format!("UPDATE jobs SET state = 'cancelled', finished_at = NOW() WHERE created_by = '{user}' AND state = 'queued'"));
}
