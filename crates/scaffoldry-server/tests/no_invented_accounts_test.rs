//! The server does not make up a person. A job with no caller fails and says so, and a simulation names who it simulates.

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use scaffoldry_server::build_app_with_state;
use scaffoldry_server::jobs::{enqueue, get_job, JobConfig, JobRunner, JOB_KINDS};
use scaffoldry_server::service::identity::issue_test_token_and_user;
use serde_json::{json, Value};
use std::time::{Duration, Instant};
use tower::ServiceExt;
use uuid::Uuid;

const ADMIN: &str = "jordan.lee@state.edu";

fn wait_for(repo: &scaffoldry_server::repository::PostgresRepository, id: Uuid) -> scaffoldry_server::jobs::Job {
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        let job = get_job(repo, id).unwrap().unwrap();
        if job.state == "done" || job.state == "failed" {
            return job;
        }
        assert!(Instant::now() < deadline, "job stuck in {}", job.state);
        std::thread::sleep(Duration::from_millis(30));
    }
}

fn run_without_caller(kind: &'static str, payload: Value) -> scaffoldry_server::jobs::Job {
    let db = common::TestDb::bare();
    let state = db.state();
    let repo = &*state.repository;
    let id = enqueue(repo, kind, payload, ADMIN, None).expect("enqueue");
    let runner = JobRunner::start(repo, JOB_KINDS, JobConfig { workers: 1, poll_ms: 20, backoff_secs: [0, 0, 0], ..JobConfig::default() }).unwrap();
    let job = wait_for(repo, id);
    runner.stop();
    job
}

// 1. A job whose payload names no caller fails, and its error says what is missing. No one is made up.
#[test]
fn an_export_with_no_caller_fails_and_names_what_is_missing() {
    let job = run_without_caller("export_run", json!({ "export_id": Uuid::new_v4().to_string(), "app_slug": "any-app", "format": "csv" }));
    assert_eq!(job.state, "failed", "{job:?}");
    assert!(job.error.as_deref().is_some_and(|e| e.contains("caller_eppn")), "{:?}", job.error);
}

#[test]
fn an_import_with_no_caller_fails_and_names_what_is_missing() {
    let job = run_without_caller("import_run", json!({ "import_id": Uuid::new_v4().to_string(), "app_slug": "any-app" }));
    assert_eq!(job.state, "failed", "{job:?}");
    assert!(job.error.as_deref().is_some_and(|e| e.contains("caller_eppn")), "{:?}", job.error);
}

// 2. A simulation says who it simulates. The server supplies no person, no app and no department of its own.
#[tokio::test]
async fn a_simulation_must_name_the_person_the_app_and_the_action() {
    let db = common::TestDb::bare();
    let app = build_app_with_state(db.state()).expect("router");
    let admin = issue_test_token_and_user(ADMIN);
    let full = json!({ "eppn": "prof.test@physics.edu", "affiliation": "faculty", "action": "write", "app_slug": "physics-grants", "department": "physics" });
    for missing in ["eppn", "affiliation", "action", "app_slug", "department"] {
        let mut body = full.clone();
        body.as_object_mut().unwrap().remove(missing);
        let req = Request::builder()
            .method("POST")
            .uri("/api/v1/policies/simulate")
            .header("authorization", format!("Bearer {admin}"))
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap();
        let resp = app.clone().oneshot(req).await.unwrap();
        let status = resp.status();
        let text = String::from_utf8_lossy(&resp.into_body().collect().await.unwrap().to_bytes()).to_string();
        assert_eq!(status, StatusCode::BAD_REQUEST, "without {missing}: {text}");
        assert!(text.contains(missing), "the answer names {missing}: {text}");
    }
    let req = Request::builder()
        .method("POST")
        .uri("/api/v1/policies/simulate")
        .header("authorization", format!("Bearer {admin}"))
        .header("content-type", "application/json")
        .body(Body::from(full.to_string()))
        .unwrap();
    assert_eq!(app.clone().oneshot(req).await.unwrap().status(), StatusCode::OK);
}

// 3. No source file names a made-up person.
#[test]
fn no_source_file_names_a_made_up_person() {
    fn walk(dir: &std::path::Path, hits: &mut Vec<String>) {
        for entry in std::fs::read_dir(dir).unwrap().flatten() {
            let path = entry.path();
            if path.is_dir() {
                walk(&path, hits);
            } else if path.extension().is_some_and(|e| e == "rs") {
                let text = std::fs::read_to_string(&path).unwrap_or_default();
                for needle in ["user@university.edu", "sim.user@university.edu"] {
                    if text.contains(needle) {
                        hits.push(format!("{}: {needle}", path.display()));
                    }
                }
            }
        }
    }
    let crates = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    let mut hits = Vec::new();
    for entry in std::fs::read_dir(&crates).unwrap().flatten() {
        let src = entry.path().join("src");
        if src.is_dir() {
            walk(&src, &mut hits);
        }
    }
    assert!(hits.is_empty(), "a made-up person is written into: {hits:?}");
}
