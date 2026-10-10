//! Jobs phase 1: a durable queue on PostgreSQL.
//!
//! The tests share one database. Each test uses kind names of its own, and a runner claims only
//! the kinds in its registry, so tests do not take each other's jobs.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use scaffoldry_server::jobs::{
    claim_next, enqueue, get_job, cancel_job, JobConfig, JobCtx, JobError, JobKind, JobRunner,
};
use scaffoldry_server::repository::PostgresRepository;
use scaffoldry_server::{build_app_with_state, state::ServerState};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tower::ServiceExt;

fn repo() -> PostgresRepository {
    PostgresRepository::connect(None).expect("database is reachable")
}

fn fast(workers: usize) -> JobConfig {
    JobConfig {
        workers,
        poll_ms: 20,
        backoff_secs: [0, 0, 0],
        ..JobConfig::default()
    }
}

fn wait_for(repo: &PostgresRepository, id: uuid::Uuid, states: &[&str]) -> scaffoldry_server::jobs::Job {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let job = get_job(repo, id).expect("query").expect("job exists");
        if states.contains(&job.state.as_str()) {
            return job;
        }
        assert!(Instant::now() < deadline, "job {id} stuck in state {}", job.state);
        std::thread::sleep(Duration::from_millis(20));
    }
}

// 1. Eight threads claiming from 100 queued jobs run each job exactly once.
static RAN: Mutex<Option<HashMap<i64, usize>>> = Mutex::new(None);

fn count_handler(_ctx: &JobCtx, payload: Value) -> Result<Value, JobError> {
    let n = payload["n"].as_i64().unwrap();
    let mut guard = RAN.lock().unwrap();
    *guard.get_or_insert_with(HashMap::new).entry(n).or_insert(0) += 1;
    Ok(json!({ "n": n }))
}
static K1: &[JobKind] = &[JobKind { name: "t1_count", max_attempts: 3, max_concurrent: 100, run: count_handler }];

#[test]
fn eight_workers_run_each_of_one_hundred_jobs_exactly_once() {
    let repo = repo();
    let ids: Vec<_> = (0..100)
        .map(|n| enqueue(&repo, "t1_count", json!({ "n": n }), "t1@state.edu", None).unwrap())
        .collect();
    let runner = JobRunner::start(&repo, K1, fast(8)).unwrap();
    for id in &ids {
        wait_for(&repo, *id, &["done"]);
    }
    runner.stop();
    let guard = RAN.lock().unwrap();
    let ran = guard.as_ref().unwrap();
    assert_eq!(ran.len(), 100);
    assert!(ran.values().all(|c| *c == 1), "every job ran once");
}

// 2. A worker killed after claiming: the job returns to queued after the lock expires and runs again.
fn ok_handler(_ctx: &JobCtx, _p: Value) -> Result<Value, JobError> {
    Ok(json!({}))
}
static K2: &[JobKind] = &[JobKind { name: "t2_recover", max_attempts: 3, max_concurrent: 5, run: ok_handler }];

#[test]
fn a_job_whose_worker_died_runs_again_after_the_lock_expires() {
    let repo = repo();
    let cfg = JobConfig { lock_secs: 1, recovery_secs: 1, ..fast(1) };
    let id = enqueue(&repo, "t2_recover", json!({}), "t2@state.edu", None).unwrap();

    // A worker claims the job and its connection is dropped without finishing.
    {
        let mut ghost = repo.dedicated_client().unwrap();
        let claimed = claim_next(&mut ghost, "ghost", K2, &cfg).unwrap().expect("claimed");
        assert_eq!(claimed.id, id);
        assert_eq!(claimed.attempts, 1);
    }
    std::thread::sleep(Duration::from_millis(1500));

    let runner = JobRunner::start(&repo, K2, cfg).unwrap();
    let job = wait_for(&repo, id, &["done"]);
    runner.stop();
    assert_eq!(job.attempts, 2);
}

// 3. Errors retry then fail. A permanent error fails at once.
static BOOMS: AtomicUsize = AtomicUsize::new(0);
fn boom_handler(_ctx: &JobCtx, _p: Value) -> Result<Value, JobError> {
    let n = BOOMS.fetch_add(1, Ordering::SeqCst) + 1;
    Err(JobError::Retry(format!("boom {n}")))
}
fn permanent_handler(_ctx: &JobCtx, _p: Value) -> Result<Value, JobError> {
    Err(JobError::Permanent("nope".to_string()))
}
static K3: &[JobKind] = &[
    JobKind { name: "t3_boom", max_attempts: 3, max_concurrent: 5, run: boom_handler },
    JobKind { name: "t3_perm", max_attempts: 3, max_concurrent: 5, run: permanent_handler },
];

#[test]
fn errors_retry_three_times_and_a_permanent_error_fails_at_once() {
    let repo = repo();
    let boom = enqueue(&repo, "t3_boom", json!({}), "t3@state.edu", None).unwrap();
    let perm = enqueue(&repo, "t3_perm", json!({}), "t3@state.edu", None).unwrap();
    let runner = JobRunner::start(&repo, K3, fast(2)).unwrap();
    let boom_job = wait_for(&repo, boom, &["failed"]);
    let perm_job = wait_for(&repo, perm, &["failed"]);
    runner.stop();

    assert_eq!(boom_job.attempts, 3);
    assert_eq!(boom_job.error.as_deref(), Some("boom 3"), "the last error is kept");
    assert_eq!(perm_job.attempts, 1);
    assert_eq!(perm_job.error.as_deref(), Some("nope"));
}

// 4. A resumed job sees the checkpoint the first run wrote.
fn checkpoint_handler(ctx: &JobCtx, _p: Value) -> Result<Value, JobError> {
    match ctx.load_checkpoint() {
        None => {
            ctx.checkpoint(json!({ "batch": 2 }))?;
            Err(JobError::Retry("interrupted".to_string()))
        }
        Some(cp) => ctx.result(json!({ "resumed_from": cp["batch"] })),
    }
}
static K4: &[JobKind] = &[JobKind { name: "t4_checkpoint", max_attempts: 3, max_concurrent: 5, run: checkpoint_handler }];

#[test]
fn a_resumed_job_sees_its_checkpoint() {
    let repo = repo();
    let id = enqueue(&repo, "t4_checkpoint", json!({}), "t4@state.edu", None).unwrap();
    let runner = JobRunner::start(&repo, K4, fast(1)).unwrap();
    let job = wait_for(&repo, id, &["done"]);
    runner.stop();
    assert_eq!(job.result.unwrap()["resumed_from"], 2);
    assert_eq!(job.attempts, 2);
}

// 5. A kind capped at one never runs two at once.
static CURRENT: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);
fn slow_handler(_ctx: &JobCtx, _p: Value) -> Result<Value, JobError> {
    let now = CURRENT.fetch_add(1, Ordering::SeqCst) + 1;
    PEAK.fetch_max(now, Ordering::SeqCst);
    std::thread::sleep(Duration::from_millis(30));
    CURRENT.fetch_sub(1, Ordering::SeqCst);
    Ok(json!({}))
}
static K5: &[JobKind] = &[JobKind { name: "t5_cap", max_attempts: 3, max_concurrent: 1, run: slow_handler }];

#[test]
fn a_kind_with_a_cap_of_one_never_runs_two_at_once() {
    let repo = repo();
    let ids: Vec<_> = (0..10)
        .map(|_| enqueue(&repo, "t5_cap", json!({}), "t5@state.edu", None).unwrap())
        .collect();
    let runner = JobRunner::start(&repo, K5, fast(4)).unwrap();
    for id in &ids {
        wait_for(&repo, *id, &["done"]);
    }
    runner.stop();
    assert_eq!(PEAK.load(Ordering::SeqCst), 1);
}

// 6. A dedupe key returns the live job. After it finishes the key is free.
static K6: &[JobKind] = &[JobKind { name: "t6_dedupe", max_attempts: 3, max_concurrent: 5, run: ok_handler }];

#[test]
fn a_dedupe_key_returns_the_live_job_and_is_free_after_it_finishes() {
    let repo = repo();
    let key = format!("t6-{}", uuid::Uuid::new_v4());
    let first = enqueue(&repo, "t6_dedupe", json!({}), "t6@state.edu", Some(&key)).unwrap();
    let second = enqueue(&repo, "t6_dedupe", json!({}), "t6@state.edu", Some(&key)).unwrap();
    assert_eq!(first, second, "a queued job with the key is returned");

    let runner = JobRunner::start(&repo, K6, fast(1)).unwrap();
    wait_for(&repo, first, &["done"]);
    runner.stop();
    let third = enqueue(&repo, "t6_dedupe", json!({}), "t6@state.edu", Some(&key)).unwrap();
    assert_ne!(first, third, "after it finished the key can be used again");
}

// 7. Cancelling.
fn cancellable_handler(ctx: &JobCtx, _p: Value) -> Result<Value, JobError> {
    for _ in 0..400 {
        if ctx.cancelled()? {
            return Err(JobError::Cancelled);
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    Ok(json!({}))
}
static K7: &[JobKind] = &[JobKind { name: "t7_cancel", max_attempts: 3, max_concurrent: 5, run: cancellable_handler }];

#[test]
fn cancelling_a_queued_job_is_immediate_and_a_running_job_stops_at_its_next_batch() {
    let repo = repo();
    let queued = enqueue(&repo, "t7_cancel", json!({}), "t7@state.edu", None).unwrap();
    let after = cancel_job(&repo, queued).unwrap().expect("job exists");
    assert_eq!(after.state, "cancelled", "a queued job is cancelled at once");

    let running = enqueue(&repo, "t7_cancel", json!({}), "t7@state.edu", None).unwrap();
    let runner = JobRunner::start(&repo, K7, fast(1)).unwrap();
    wait_for(&repo, running, &["running"]);
    let flagged = cancel_job(&repo, running).unwrap().expect("job exists");
    assert!(flagged.cancel_requested);
    let job = wait_for(&repo, running, &["cancelled"]);
    runner.stop();
    assert_eq!(job.state, "cancelled");
}

// 8. Only the creator and a Platform Admin can see a job.
async fn body_json(resp: axum::response::Response) -> Value {
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    serde_json::from_slice(&bytes).unwrap_or(Value::Null)
}

fn get(path: &str, token: &str) -> Request<Body> {
    Request::builder()
        .uri(path)
        .header("authorization", format!("Bearer {token}"))
        .body(Body::empty())
        .unwrap()
}

#[tokio::test]
async fn only_the_creator_and_a_platform_admin_can_read_or_cancel_a_job() {
    let state = Arc::new(ServerState::new().expect("state"));
    let repo = state.repository.clone().expect("postgres");
    let app = build_app_with_state(state).expect("router");

    let owner = scaffoldry_server::service::identity::issue_test_token_and_user("prof.curie@science.state.edu");
    let stranger = scaffoldry_server::service::identity::issue_test_token_and_user("sarah.connor@state.edu");
    let admin = scaffoldry_server::service::identity::issue_test_token_and_user("jordan.lee@state.edu");

    let id = enqueue(&repo, "t8_view", json!({}), "prof.curie@science.state.edu", None).unwrap();
    let path = format!("/api/v1/jobs/{id}");

    let resp = app.clone().oneshot(get(&path, &owner)).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    assert_eq!(body["state"], "queued");
    assert!(body.get("payload").is_none(), "the payload is not exposed");

    let resp = app.clone().oneshot(get(&path, &stranger)).await.unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND, "a stranger cannot learn the job exists");

    let resp = app.clone().oneshot(get(&path, &admin)).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    // The list is the caller's own jobs.
    let resp = app.clone().oneshot(get("/api/v1/jobs", &owner)).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let list = body_json(resp).await;
    assert!(list["jobs"].as_array().unwrap().iter().any(|j| j["id"] == id.to_string()));
    let resp = app.clone().oneshot(get("/api/v1/jobs", &stranger)).await.unwrap();
    let list = body_json(resp).await;
    assert!(!list["jobs"].as_array().unwrap().iter().any(|j| j["id"] == id.to_string()));

    // Cancel: a stranger gets 404 and the job is untouched. The creator cancels it.
    let cancel = |token: &str| {
        Request::builder()
            .method("POST")
            .uri(format!("{path}/cancel"))
            .header("authorization", format!("Bearer {token}"))
            .body(Body::empty())
            .unwrap()
    };
    let resp = app.clone().oneshot(cancel(&stranger)).await.unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    assert_eq!(get_job(&repo, id).unwrap().unwrap().state, "queued");
    let resp = app.clone().oneshot(cancel(&owner)).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(body_json(resp).await["state"], "cancelled");

    // No session, no job.
    let anon = Request::builder().uri(&path).body(Body::empty()).unwrap();
    assert_eq!(app.oneshot(anon).await.unwrap().status(), StatusCode::UNAUTHORIZED);
}

// 9. A payload or result that names a token or a secret is refused.
fn leaky_result_handler(ctx: &JobCtx, _p: Value) -> Result<Value, JobError> {
    ctx.result(json!({ "count": 3, "token": "abc" }))
}
static K9: &[JobKind] = &[JobKind { name: "t9_leaky", max_attempts: 3, max_concurrent: 5, run: leaky_result_handler }];

#[test]
fn a_payload_or_result_with_a_token_or_secret_key_is_refused() {
    let repo = repo();
    assert!(enqueue(&repo, "t9_leaky", json!({ "token": "abc" }), "t9@state.edu", None).is_err());
    assert!(enqueue(&repo, "t9_leaky", json!({ "a": { "Secret": 1 } }), "t9@state.edu", None).is_err());
    assert!(enqueue(&repo, "t9_leaky", json!({ "items": [{ "password": "x" }] }), "t9@state.edu", None).is_err());
    assert!(enqueue(&repo, "t9_leaky", json!({ "count": 3 }), "t9@state.edu", None).is_ok());

    let id = enqueue(&repo, "t9_leaky", json!({}), "t9@state.edu", None).unwrap();
    let runner = JobRunner::start(&repo, K9, fast(1)).unwrap();
    let job = wait_for(&repo, id, &["failed"]);
    runner.stop();
    assert_eq!(job.attempts, 1, "a refused result is permanent");
    assert!(job.error.unwrap().contains("token"));
}
