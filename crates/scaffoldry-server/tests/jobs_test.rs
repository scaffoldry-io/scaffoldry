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


// Phase 2 Tests
use scaffoldry_core::ledger::DecisionType;
use scaffoldry_server::service::admin::ADMIN_ROUTES;
use scaffoldry_server::jobs::{register_schedule, scheduler_tick, retry_job};

#[test]
fn test_phase2_decision_type_job_cancelled() {
    assert!(DecisionType::ALL.contains(&DecisionType::JobCancelled));
    assert_eq!(DecisionType::JobCancelled.as_str(), "JobCancelled");
    assert_eq!(DecisionType::parse("JobCancelled"), Some(DecisionType::JobCancelled));
}

#[test]
fn test_phase2_admin_routes_listed_and_guarded() {
    let routes = [
        ("GET", "/admin/jobs"),
        ("GET", "/admin/jobs/{id}"),
        ("POST", "/admin/jobs/{id}/cancel"),
        ("POST", "/admin/jobs/{id}/retry"),
    ];

    for (m, p) in routes {
        assert!(
            ADMIN_ROUTES.iter().any(|(rm, rp)| *rm == m && *rp == p),
            "ADMIN_ROUTES must list {m} {p}"
        );
    }
}

#[test]
fn test_phase2_schedule_two_simulated_threads_create_exactly_one_job() {
    let repo = repo();
    let kind = "t_p2_sched_1".to_string();
    register_schedule(&repo, &kind, 60).expect("register schedule");

    let k_clone = kind.clone();
    repo.with_client(move |c| {
        c.execute(
            "UPDATE job_schedules SET next_run_at = NOW() - INTERVAL '1 second' WHERE kind = $1",
            &[&k_clone],
        )?;
        c.execute("DELETE FROM jobs WHERE kind = $1", &[&k_clone])?;
        Ok(())
    }).unwrap();

    let repo1 = repo.clone();
    let repo2 = repo.clone();
    let h1 = std::thread::spawn(move || scheduler_tick(&repo1, &[]));
    let h2 = std::thread::spawn(move || scheduler_tick(&repo2, &[]));

    let res1 = h1.join().unwrap().unwrap();
    let res2 = h2.join().unwrap().unwrap();

    assert_eq!(res1 + res2, 1, "only one job should be created across two threads");

    let k_check = kind.clone();
    let count: i64 = repo.with_client(move |c| {
        Ok(c.query_one("SELECT COUNT(*) FROM jobs WHERE kind = $1", &[&k_check])?.get(0))
    }).unwrap();
    assert_eq!(count, 1, "exactly 1 job exists in database for this schedule");
}

#[test]
fn test_phase2_tick_delayed_by_90_seconds_cadence_maintained() {
    let repo = repo();
    let kind = "t_p2_sched_2".to_string();
    register_schedule(&repo, &kind, 60).expect("register schedule");

    let k_clone = kind.clone();
    repo.with_client(move |c| {
        c.execute(
            "UPDATE job_schedules SET next_run_at = NOW() - INTERVAL '90 seconds' WHERE kind = $1",
            &[&k_clone],
        )?;
        c.execute("DELETE FROM jobs WHERE kind = $1", &[&k_clone])?;
        Ok(())
    }).unwrap();

    let spawned = scheduler_tick(&repo, &[]).expect("scheduler tick");
    assert_eq!(spawned, 1, "a tick delayed by 90 seconds creates one job, not two");

    let k_check = kind.clone();
    let secs_until_next: i64 = repo.with_client(move |c| {
        Ok(c.query_one(
            "SELECT EXTRACT(EPOCH FROM (next_run_at - NOW()))::bigint FROM job_schedules WHERE kind = $1",
            &[&k_check],
        )?.get(0))
    }).unwrap();

    assert!(
        (25..=35).contains(&secs_until_next),
        "next run must be ~30s in the future on original cadence, got {secs_until_next}s"
    );

    let second_spawned = scheduler_tick(&repo, &[]).expect("second tick");
    assert_eq!(second_spawned, 0, "immediate subsequent tick should create 0 jobs");
}

#[test]
fn test_phase2_schedules_disabled_stops_it() {
    let repo = repo();
    let kind = "t_p2_sched_3".to_string();
    register_schedule(&repo, &kind, 60).expect("register schedule");

    let k_clone = kind.clone();
    repo.with_client(move |c| {
        c.execute(
            "UPDATE job_schedules SET next_run_at = NOW() - INTERVAL '10 seconds' WHERE kind = $1",
            &[&k_clone],
        )?;
        c.execute("DELETE FROM jobs WHERE kind = $1", &[&k_clone])?;
        Ok(())
    }).unwrap();

    let disabled = vec![kind.clone()];
    let spawned = scheduler_tick(&repo, &disabled).expect("scheduler tick with disabled");
    assert_eq!(spawned, 0, "disabled schedule must not spawn jobs");

    let k_check = kind.clone();
    let count: i64 = repo.with_client(move |c| {
        Ok(c.query_one("SELECT COUNT(*) FROM jobs WHERE kind = $1", &[&k_check])?.get(0))
    }).unwrap();
    assert_eq!(count, 0, "no jobs created for disabled kind");
}

#[tokio::test]
async fn test_phase2_admin_routes_faculty_is_403() {
    let repo = repo();
    let mut server_state = ServerState::new().expect("server state");
    server_state.repository = Some(Arc::new(repo));
    let state = Arc::new(server_state);

    let faculty_token = scaffoldry_server::service::identity::issue_test_token_and_user("faculty.curie@state.edu");
    let app = build_app_with_state(state).expect("app router");

    let admin_paths = [
        ("GET", "/api/v1/admin/jobs"),
        ("GET", "/api/v1/admin/jobs/00000000-0000-0000-0000-000000000001"),
        ("POST", "/api/v1/admin/jobs/00000000-0000-0000-0000-000000000001/cancel"),
        ("POST", "/api/v1/admin/jobs/00000000-0000-0000-0000-000000000001/retry"),
    ];

    for (method, uri) in admin_paths {
        let mut req_builder = Request::builder()
            .method(method)
            .uri(uri)
            .header("authorization", format!("Bearer {faculty_token}"));
        if method == "POST" {
            req_builder = req_builder.header("content-type", "application/json");
        }
        let req = if method == "POST" {
            req_builder.body(Body::from(r#"{"reason": "Faculty test attempt"}"#)).unwrap()
        } else {
            req_builder.body(Body::empty()).unwrap()
        };

        let resp = app.clone().oneshot(req).await.unwrap();
        assert_eq!(
            resp.status(),
            StatusCode::FORBIDDEN,
            "Faculty caller to {method} {uri} must receive 403 Forbidden"
        );
    }
}

static K_RETRY: &[JobKind] = &[JobKind {
    name: "t_p2_retry",
    max_attempts: 1,
    max_concurrent: 10,
    run: |_ctx, payload| {
        if payload["fail"].as_bool().unwrap_or(false) {
            Err(JobError::Permanent("boom".to_string()))
        } else {
            Ok(json!({"ok": true}))
        }
    },
}];

#[test]
fn test_phase2_retry_failed_job_and_conflict_on_done() {
    let repo = repo();
    let runner = JobRunner::start(&repo, K_RETRY, fast(2)).unwrap();

    let fail_id = enqueue(&repo, "t_p2_retry", json!({"fail": true}), "user@scaffoldry.internal", None).unwrap();
    let fail_job = wait_for(&repo, fail_id, &["failed"]);
    assert_eq!(fail_job.state, "failed");

    let retried = retry_job(&repo, fail_id).expect("retry failed job");
    assert_eq!(retried.state, "queued");
    assert_eq!(retried.attempts, 0);

    let done_id = enqueue(&repo, "t_p2_retry", json!({"fail": false}), "user@scaffoldry.internal", None).unwrap();
    let done_job = wait_for(&repo, done_id, &["done"]);
    assert_eq!(done_job.state, "done");

    let conflict = retry_job(&repo, done_id);
    assert!(
        matches!(conflict, Err(scaffoldry_server::jobs::JobsError::Conflict(_))),
        "retry on a done job must return Conflict, got {conflict:?}"
    );

    runner.stop();
}

#[tokio::test]
async fn test_phase2_mcp_get_job_privacy() {
    let repo = repo();
    let mut server_state = ServerState::new().expect("server state");
    server_state.repository = Some(Arc::new(repo.clone()));
    let state = Arc::new(server_state);
    let app = build_app_with_state(state).expect("app router");

    let alice_token = scaffoldry_server::service::identity::issue_test_token_and_user("alice@scaffoldry.internal");
    let bob_token = scaffoldry_server::service::identity::issue_test_token_and_user("bob@scaffoldry.internal");

    let alice_job_id = enqueue(&repo, "t_p2_mcp", json!({}), "alice@scaffoldry.internal", None).unwrap();

    let mcp_call = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "tools/call",
        "params": {
            "name": "get_job",
            "arguments": {
                "job_id": alice_job_id.to_string()
            }
        }
    });

    let req_bob = Request::builder()
        .method("POST")
        .uri("/api/v1/mcp")
        .header("authorization", format!("Bearer {bob_token}"))
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&mcp_call).unwrap()))
        .unwrap();

    let resp_bob = app.clone().oneshot(req_bob).await.unwrap();
    assert_eq!(resp_bob.status(), StatusCode::OK);
    let bytes = resp_bob.into_body().collect().await.unwrap().to_bytes();
    let body_bob: Value = serde_json::from_slice(&bytes).unwrap();
    let result_bob = &body_bob["result"];
    assert_eq!(result_bob["isError"], true, "Bob should receive error for Alice's job");
    assert!(
        result_bob["content"][0]["text"].as_str().unwrap().contains("not found"),
        "Error message must state 'not found'"
    );

    let req_alice = Request::builder()
        .method("POST")
        .uri("/api/v1/mcp")
        .header("authorization", format!("Bearer {alice_token}"))
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&mcp_call).unwrap()))
        .unwrap();

    let resp_alice = app.clone().oneshot(req_alice).await.unwrap();
    assert_eq!(resp_alice.status(), StatusCode::OK);
    let bytes_alice = resp_alice.into_body().collect().await.unwrap().to_bytes();
    let body_alice: Value = serde_json::from_slice(&bytes_alice).unwrap();
    let result_alice = &body_alice["result"];
    assert_eq!(result_alice["isError"], Value::Null, "Alice should successfully see her job");
    assert!(
        result_alice["content"][0]["text"].as_str().unwrap().contains(&alice_job_id.to_string()),
        "Job details returned to creator"
    );
}
