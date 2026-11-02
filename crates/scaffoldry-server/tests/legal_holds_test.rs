//! Lifecycle, phase 4: legal holds override every destruction path.
//!
//! A held record may go to trash and come back. It is never destroyed. One function, `is_held`, answers for every path:
//! retention, the history limit, the retired-value strip and the orphan blob purge. Only a Platform Admin or `compliance`
//! places, lists or sees a hold, and a hold is released by a second person. NIST SP 800-53 AU-11(1) and SI-12; FRCP 37(e).

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::Router;
use http_body_util::BodyExt;
use scaffoldry_server::build_app_with_state;
use scaffoldry_server::jobs::{enqueue, get_job, JobConfig, JobRunner, JOB_KINDS};
use scaffoldry_server::service::identity::issue_test_token_and_user;
use scaffoldry_server::state::ServerState;
use serde_json::{json, Value};
use std::time::{Duration, Instant};
use tower::ServiceExt;

const WORKSPACE: &str = "ws-physics-optics";
const OWNER: &str = "einstein@physics.state.edu";
const ADMIN: &str = "jordan.lee@state.edu";
const COMPLIANCE: &str = "hold.officer@state.edu";

struct World {
    db: common::TestDb,
    state: std::sync::Arc<ServerState>,
    app: Router,
    owner: String,
    admin: String,
    compliance: String,
    slug: String,
}

async fn rest(app: &Router, method: &str, uri: &str, token: &str, body: Option<Value>) -> (StatusCode, Value) {
    let b = Request::builder().method(method).uri(uri).header("authorization", format!("Bearer {token}"));
    let req = match body {
        Some(v) => b.header("content-type", "application/json").body(Body::from(v.to_string())).unwrap(),
        None => b.body(Body::empty()).unwrap(),
    };
    let resp = app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
}

async fn world() -> World {
    let db = common::TestDb::new();
    let state = db.state();
    state.seed_demo().expect("seed");
    let compliance = issue_test_token_and_user(COMPLIANCE);
    db.run(|c| {
        c.execute(
            "INSERT INTO roles (id, person_id, organization_id, role_title, scoped_affiliation, is_primary, source) \
             SELECT gen_random_uuid(), p.id, o.id, 'Compliance Officer', 'compliance', true, 'api' \
             FROM persons p, organizations o WHERE LOWER(p.eppn) = LOWER($1) AND o.parent_id IS NULL",
            &[&COMPLIANCE],
        )
        .unwrap();
    });
    let tag = uuid::Uuid::new_v4().to_string()[..8].to_string();
    let app = build_app_with_state(state.clone()).expect("router");
    let w = World { db, state, app, owner: issue_test_token_and_user(OWNER), admin: issue_test_token_and_user(ADMIN), compliance, slug: format!("hold-{tag}") };
    let manifest = json!({
        "slug": w.slug, "title": "Held", "description": "Records under a hold", "organization_code": "PHYS", "department": "physics",
        "workspace_id": WORKSPACE, "views": [],
        "tables": [{ "id": "cases", "name": "Cases", "slug": "cases", "primary_field": "title", "fields": [
            { "name": "title", "label": "Title", "field_type": "Text", "required": false, "ferpa_sensitive": false },
            { "name": "matter", "label": "Matter", "field_type": "Text", "required": false, "ferpa_sensitive": false }
        ]}],
    });
    let (s, b) = rest(&w.app, "POST", &format!("/api/v1/workspaces/{WORKSPACE}/apps"), &w.owner, Some(manifest)).await;
    assert_eq!(s, StatusCode::CREATED, "{b}");
    // Trash is kept a year, by policy.
    let (_, list) = rest(&w.app, "GET", "/api/v1/admin/retention/policies", &w.admin, None).await;
    let mut p = list["policies"].as_array().unwrap().iter().find(|p| p["classification"] == "Internal").cloned().unwrap();
    p["keep_trash_days"] = json!(365);
    p["reason"] = json!("Set");
    let (s, b) = rest(&w.app, "PUT", &format!("/api/v1/admin/retention/policies/{}", p["id"].as_str().unwrap()), &w.admin, Some(p)).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    w
}

async fn create(w: &World, title: &str, matter: &str) -> String {
    let (s, r) = rest(&w.app, "POST", &format!("/api/v1/apps/{}/records", w.slug), &w.owner, Some(json!({ "data": { "_table_id": "cases", "title": title, "matter": matter } }))).await;
    assert_eq!(s, StatusCode::CREATED, "{r}");
    r["id"].as_str().unwrap().to_string()
}

/// Puts a record in trash, then moves its time in trash back past the year.
async fn trash_long_ago(w: &World, id: &str) {
    let (s, b) = rest(&w.app, "DELETE", &format!("/api/v1/apps/{}/records/{id}", w.slug), &w.owner, None).await;
    assert!(s.is_success(), "a held record may still go to trash: {s} {b}");
    let id = id.to_string();
    w.db.run(move |c| {
        c.execute("UPDATE dataset_records SET deleted_at = NOW() - interval '400 days' WHERE id = $1", &[&id]).unwrap();
    });
}

fn present(w: &World, id: &str) -> bool {
    let id = id.to_string();
    w.db.run(move |c| c.query_one("SELECT count(*) FROM dataset_records WHERE id = $1", &[&id]).unwrap().get::<_, i64>(0) == 1)
}

fn runner(w: &World) -> JobRunner {
    JobRunner::start(&w.state.repository, JOB_KINDS, JobConfig { workers: 2, poll_ms: 20, backoff_secs: [0, 0, 0], ..JobConfig::default() }).unwrap()
}

fn wait(w: &World, id: uuid::Uuid) -> scaffoldry_server::jobs::Job {
    let deadline = Instant::now() + Duration::from_secs(120);
    loop {
        let job = get_job(&w.state.repository, id).unwrap().expect("the job");
        if job.state == "done" || job.state == "failed" {
            return job;
        }
        assert!(Instant::now() < deadline, "the job is stuck in {}", job.state);
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn purge_now(w: &World) {
    let r = runner(w);
    let id = enqueue(&w.state.repository, "purge_expired", json!({}), "test@scaffoldry.internal", None).unwrap();
    assert_eq!(wait(w, id).state, "done");
    r.stop();
}

/// Places a hold as the compliance officer, and waits for its records to be pinned.
async fn place(w: &World, scope: Value) -> Value {
    let mut body = scope;
    body["name"] = json!("Smith v. State");
    body["case_ref"] = json!("2026-CV-0142");
    body["reason"] = json!("Litigation hold from counsel");
    let r = runner(w);
    let (s, hold) = rest(&w.app, "POST", "/api/v1/admin/holds", &w.compliance, Some(body)).await;
    assert_eq!(s, StatusCode::CREATED, "{hold}");
    if let Some(job) = hold["pin_job_id"].as_str() {
        assert_eq!(wait(w, job.parse().unwrap()).state, "done");
    }
    r.stop();
    hold
}

fn held_skipped(w: &World) -> Vec<i32> {
    let slug = w.slug.clone();
    w.db.run(move |c| c.query("SELECT held_skipped FROM purge_certificates WHERE app_slug = $1 ORDER BY run_at", &[&slug]).unwrap().iter().map(|r| r.get(0)).collect())
}

// 1. A held record in trash past retention is not destroyed, and the certificate reports `held_skipped`. After release, the
//    next run destroys it.
#[tokio::test]
async fn a_held_record_outlives_retention_until_the_hold_is_released() {
    let w = world().await;
    let held = create(&w, "Held", "smith").await;
    let free = create(&w, "Free", "other").await;
    let narrow = place(&w, json!({ "scope_kind": "query", "app_slug": w.slug, "table_id": "cases", "filter": { "matter": "smith" } })).await;
    trash_long_ago(&w, &held).await;
    trash_long_ago(&w, &free).await;

    purge_now(&w);
    assert!(present(&w, &held), "a held record is never destroyed");
    assert!(!present(&w, &free), "the record outside the hold is");
    assert_eq!(held_skipped(&w), vec![1], "the certificate says one was held");

    let (s, b) = rest(&w.app, "POST", &format!("/api/v1/admin/holds/{}/release", narrow["id"].as_str().unwrap()), &w.admin, Some(json!({ "reason": "The case settled" }))).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    purge_now(&w);
    assert!(!present(&w, &held), "after release, the next run destroys it");
    let pinned: i64 = w.db.run(|c| c.query_one("SELECT count(*) FROM hold_records", &[]).unwrap().get(0));
    assert_eq!(pinned, 0, "a released hold's pinned rows are removed");
}

// 2. A workspace hold protects a record created after the hold. A query hold without `include_future` does not protect a
//    later record.
#[tokio::test]
async fn a_workspace_hold_covers_later_records_and_a_pinned_query_does_not() {
    let w = world().await;
    let before = create(&w, "Before", "smith").await;
    let query = place(&w, json!({ "scope_kind": "query", "app_slug": w.slug, "table_id": "cases", "filter": { "matter": "smith" }, "include_future": false })).await;
    let later_match = create(&w, "Later", "smith").await;
    for id in [&before, &later_match] {
        trash_long_ago(&w, id).await;
    }
    purge_now(&w);
    assert!(present(&w, &before), "the pinned record is held");
    assert!(!present(&w, &later_match), "a pinned query does not hold a record written after it");

    rest(&w.app, "POST", &format!("/api/v1/admin/holds/{}/release", query["id"].as_str().unwrap()), &w.admin, Some(json!({ "reason": "Replaced" }))).await;
    place(&w, json!({ "scope_kind": "workspace", "workspace_id": WORKSPACE })).await;
    let after = create(&w, "After", "anything").await;
    trash_long_ago(&w, &after).await;
    purge_now(&w);
    assert!(present(&w, &after), "a workspace hold covers a record made after it");
}

// 3. Each destruction path calls `is_held`. A scan test finds no destruction that does not.
#[test]
fn every_destruction_path_asks_is_held() {
    let src = std::fs::read_to_string(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/purge.rs")).unwrap();
    // Each function that chooses what to destroy, and the SQL it runs.
    for path in ["fn plan(", "fn destroy_old_history(", "fn strip_retired(", "pub fn purge_orphan_blobs("] {
        let at = src.find(path).unwrap_or_else(|| panic!("{path} is gone; this test names the destruction paths"));
        let body = &src[at..];
        let end = body[1..].find("\nfn ").or_else(|| body[1..].find("\npub fn ")).map(|e| e + 1).unwrap_or(body.len());
        assert!(body[..end].contains("is_held"), "{path} destroys without asking is_held");
    }
    // `destroy` deletes only what a path chose, and checks again in its own transaction.
    let at = src.find("pub fn destroy(").expect("destroy");
    assert!(src[at..at + 2_000].contains("is_held"), "destroy checks is_held in its transaction");
}

// 4. The person who placed a hold cannot release it. Another person can.
#[tokio::test]
async fn the_person_who_placed_a_hold_cannot_release_it() {
    let w = world().await;
    let hold = place(&w, json!({ "scope_kind": "app", "app_slug": w.slug })).await;
    let id = hold["id"].as_str().unwrap();
    let (s, b) = rest(&w.app, "POST", &format!("/api/v1/admin/holds/{id}/release"), &w.compliance, Some(json!({ "reason": "Mine" }))).await;
    assert_eq!(s, StatusCode::FORBIDDEN, "{b}");
    assert!(b["error"].as_str().unwrap_or("").contains("second person"), "{b}");
    let (s, _) = rest(&w.app, "POST", &format!("/api/v1/admin/holds/{id}/release"), &w.admin, Some(json!({}))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "a release needs a reason");
    let (s, b) = rest(&w.app, "POST", &format!("/api/v1/admin/holds/{id}/release"), &w.admin, Some(json!({ "reason": "Counsel lifted it" }))).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    let entries: i64 = w.db.run(|c| c.query_one("SELECT count(*) FROM governance_ledger WHERE decision_type = 'LegalHoldChanged'", &[]).unwrap().get(0));
    assert_eq!(entries, 2, "placing and releasing are each in the ledger");
}

// 5. An editor sees no sign of a hold. A `compliance` caller sees it.
#[tokio::test]
async fn only_compliance_and_platform_admins_see_a_hold() {
    let w = world().await;
    let id = create(&w, "Held", "smith").await;
    place(&w, json!({ "scope_kind": "app", "app_slug": w.slug })).await;
    let (_, record) = rest(&w.app, "GET", &format!("/api/v1/apps/{}/records/{id}", w.slug), &w.owner, None).await;
    assert!(!record.to_string().to_lowercase().contains("hold"), "the record says nothing: {record}");
    let (s, _) = rest(&w.app, "GET", &format!("/api/v1/apps/{}/records/{id}/hold", w.slug), &w.owner, None).await;
    assert_eq!(s, StatusCode::NOT_FOUND, "the owner cannot learn there is a hold");
    let (s, _) = rest(&w.app, "GET", "/api/v1/admin/holds", &w.owner, None).await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    let (s, seen) = rest(&w.app, "GET", &format!("/api/v1/apps/{}/records/{id}/hold", w.slug), &w.compliance, None).await;
    assert_eq!(s, StatusCode::OK, "{seen}");
    assert_eq!(seen["held"], true, "{seen}");
    assert_eq!(seen["marker"], "Under legal hold");
    let (s, list) = rest(&w.app, "GET", "/api/v1/admin/holds", &w.compliance, None).await;
    assert_eq!(s, StatusCode::OK, "{list}");
    assert!(list["holds"].as_array().is_some_and(|l| l.iter().any(|h| h["case_ref"] == "2026-CV-0142" && h["scope"].as_str().unwrap_or("").contains(&w.slug))), "{list}");
}

// 6. The preview count equals the pinned count.
#[tokio::test]
async fn the_preview_counts_what_the_hold_then_pins() {
    let w = world().await;
    for i in 0..7 {
        create(&w, &format!("S{i}"), "smith").await;
    }
    create(&w, "Other", "jones").await;
    let scope = json!({ "scope_kind": "query", "app_slug": w.slug, "table_id": "cases", "filter": { "matter": "smith" } });
    let r = runner(&w);
    let (s, queued) = rest(&w.app, "POST", "/api/v1/admin/holds/preview", &w.compliance, Some(scope.clone())).await;
    assert_eq!(s, StatusCode::ACCEPTED, "{queued}");
    let preview = wait(&w, queued["job_id"].as_str().unwrap().parse().unwrap());
    r.stop();
    let result = preview.result.unwrap_or(Value::Null);
    assert_eq!(result["records"], 7, "{result}");
    assert_eq!(result["sentence"], "This hold covers 7 records across 1 app.", "{result}");
    let hold = place(&w, scope).await;
    let id: uuid::Uuid = hold["id"].as_str().unwrap().parse().unwrap();
    let pinned: i64 = w.db.run(move |c| c.query_one("SELECT count(*) FROM hold_records WHERE hold_id = $1", &[&id]).unwrap().get(0));
    assert_eq!(pinned, 7, "the hold pins what the preview counted");
}
