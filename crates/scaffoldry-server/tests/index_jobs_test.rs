//! Jobs phase 3: index building and flag recomputation run as jobs, with a visible `building` state.
//!
//! Each test fills a private app with generated rows and deletes them when it ends.

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::Router;
use http_body_util::BodyExt;
use scaffoldry_engine::{AppManifest, AppTable, AppView, DataLabel, FieldSpec, FieldType, SortDirection, SortRule, ViewType};
use scaffoldry_server::build_app_with_state;
use scaffoldry_server::jobs::{claim_next, get_job, JobConfig, JobRunner, JOB_KINDS};
use scaffoldry_server::repository::PostgresRepository;
use scaffoldry_server::service::identity::issue_test_token_and_user;
use scaffoldry_server::state::ServerState;
use serde_json::{json, Value};
use std::time::{Duration, Instant};
use std::sync::Arc;
use tower::ServiceExt;
use uuid::Uuid;


const OWNER: &str = "prof.curie@science.state.edu";
const WORKSPACE: &str = "ws-bio-lab";
const ROWS: i64 = 200_000;

fn db_url() -> String {
    scaffoldry_server::testing::database_url()
}

fn db<R: Send + 'static>(f: impl FnOnce(&mut postgres::Client) -> R + Send + 'static) -> R {
    let url = db_url();
    std::thread::spawn(move || {
        let mut c = postgres::Client::connect(&url, postgres::NoTls).expect("database");
        f(&mut c)
    })
    .join()
    .expect("database thread")
}

struct Cleanup(String);
impl Drop for Cleanup {
    fn drop(&mut self) {
        let slug = self.0.clone();
        let url = db_url();
        let _ = std::thread::spawn(move || {
            if let Ok(mut c) = postgres::Client::connect(&url, postgres::NoTls) {
                let _ = c.execute("DELETE FROM dataset_records WHERE app_slug = $1", &[&slug]);
            }
        })
        .join();
    }
}

fn indexed(name: &str, t: FieldType) -> FieldSpec {
    let mut f = FieldSpec::simple(name, name, t, false, false);
    f.indexed = true;
    f
}

fn plain(name: &str, t: FieldType) -> FieldSpec {
    FieldSpec::simple(name, name, t, false, false)
}

/// `amount` and `stamp` and `tags` are indexed. `plain_text` is not.
fn fields() -> Vec<FieldSpec> {
    vec![
        indexed("amount", FieldType::Number),
        indexed("stamp", FieldType::Date),
        indexed("tags", FieldType::MultiSelect),
        plain("plain_text", FieldType::Text),
        plain("score", FieldType::Number),
    ]
}

fn manifest(slug: &str, views: Vec<AppView>) -> AppManifest {
    AppManifest {
        slug: slug.to_string(),
        title: "Index".to_string(),
        description: String::new(),
        organization_code: "BIO".to_string(),
        department: "biology".to_string(),
        workspace_id: Some(WORKSPACE.to_string()),
        herm_capability_id: None,
        coordinates: None, sealed: false,
        custom_domain: None,
        custom_domain_verified: false,
        tables: vec![AppTable {
            id: "t1".to_string(),
            name: "Things".to_string(),
            slug: "things".to_string(),
            description: None,
            icon: None,
            fields: fields(),
            primary_field: None,
            sample_records: vec![],
            coordinates: None, sealed: false,
            row_place: Default::default(),
        }],
        relationships: vec![],
        views,
        ceds_mappings: Default::default(),
        pages: vec![],
        access: Default::default(),
    }
}

struct World {
    state: Arc<ServerState>,
    app: Router,
    token: String,
    slug: String,
    _cleanup: Cleanup,
}

async fn world() -> World {
    let state = Arc::new(ServerState::new().expect("state"));
    state.seed_demo().expect("seed");
    let slug = format!("idx-{}", &Uuid::new_v4().to_string()[..8]);
    state.engine.write().unwrap().register_manifest(manifest(&slug, vec![])).unwrap();
    World {
        app: build_app_with_state(state.clone()).expect("router"),
        token: issue_test_token_and_user(OWNER),
        state,
        _cleanup: Cleanup(slug.clone()),
        slug,
    }
}

async fn send(app: &Router, method: &str, uri: &str, token: &str, body: Option<Value>) -> (StatusCode, Value) {
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

async fn tool(w: &World, name: &str, args: Value) -> (bool, Value) {
    let payload = json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": { "name": name, "arguments": args } });
    let (_, v) = send(&w.app, "POST", "/api/mcp", &w.token, Some(payload)).await;
    let result = &v["result"];
    if result["isError"] == true {
        return (true, result["content"][0]["text"].clone());
    }
    (false, result["structuredContent"].clone())
}

async fn create(w: &World, data: Value) -> Value {
    let mut d = data;
    d["_table_id"] = json!("t1");
    let (s, body) = send(&w.app, "POST", &format!("/api/v1/apps/{}/records", w.slug), &w.token, Some(json!({ "data": d }))).await;
    assert_eq!(s, StatusCode::CREATED, "{body}");
    body
}

const VIEWER: &str = "marcus.vance@state.edu";
const SMALL: i64 = 5_000;

fn repo() -> PostgresRepository {
    PostgresRepository::connect(None).expect("database is reachable")
}

fn fast() -> JobConfig {
    JobConfig { workers: 2, poll_ms: 20, backoff_secs: [0, 0, 0], ..JobConfig::default() }
}

/// Rows `1..=n`, with `score` and `plain_text`, and no index rows at all.
fn fill(slug: &str, n: i64) {
    let slug = slug.to_string();
    db(move |c| {
        c.execute(
            "INSERT INTO dataset_records (id, app_slug, data, ceds_mapping, is_ferpa_sensitive, table_id, created_at) \
             SELECT 'rec-' || $1 || '-' || lpad(g::text, 8, '0'), $1, \
                    jsonb_build_object('_table_id', 't1', 'plain_text', 'x', 'score', g), \
                    '{}'::jsonb, false, 't1', TIMESTAMPTZ '2026-01-01 00:00:00+00' + (g / 10) * interval '1 millisecond' \
             FROM generate_series(1, $2::bigint) AS g",
            &[&slug, &n],
        )
        .expect("fill");
        c.batch_execute("ANALYZE dataset_records").expect("analyze");
    });
}

fn index_rows(slug: &str, field: &str) -> i64 {
    let (slug, field) = (slug.to_string(), field.to_string());
    db(move |c| c.query_one("SELECT count(*) FROM record_index WHERE app_slug = $1 AND field = $2", &[&slug, &field]).unwrap().get(0))
}

fn status(slug: &str, field: &str) -> Option<String> {
    let (slug, field) = (slug.to_string(), field.to_string());
    db(move |c| c.query_opt("SELECT state FROM index_status WHERE app_slug = $1 AND table_id = 't1' AND field = $2", &[&slug, &field]).unwrap().map(|r| r.get(0)))
}

fn job_of(slug: &str, field: &str) -> uuid::Uuid {
    let (slug, field) = (slug.to_string(), field.to_string());
    db(move |c| {
        let row = c.query_one("SELECT job_id FROM index_status WHERE app_slug = $1 AND table_id = 't1' AND field = $2", &[&slug, &field]).unwrap();
        row.get::<_, Option<uuid::Uuid>>(0).expect("the status row names its job")
    })
}

/// Makes the app live, fills it, and approves a view that sorts on `score`, which joins the index.
async fn approve_sort_on_score(w: &World, rows: i64) -> Duration {
    let live = manifest(&w.slug, vec![]);
    let (s, body) = send(&w.app, "POST", &format!("/api/v1/workspaces/{WORKSPACE}/apps"), &w.token, Some(serde_json::to_value(&live).unwrap())).await;
    assert!(s == StatusCode::CREATED || s == StatusCode::CONFLICT, "{s} {body}");
    fill(&w.slug, rows);
    let mut view = AppView::table("by-score", "By score", ViewType::Table, vec![]);
    view.table_id = Some("t1".to_string());
    view.sort_rules = vec![SortRule { id: "s".into(), field_name: "score".into(), direction: SortDirection::Asc }];
    let proposed = manifest(&w.slug, vec![view]);
    let (err, p) = tool(w, "propose_app_change", json!({ "workspace_id": WORKSPACE, "manifest": serde_json::to_value(&proposed).unwrap(), "summary": "Sort by score" })).await;
    assert!(!err, "{}", p);
    let started = Instant::now();
    let (err, done) = tool(w, "decide_proposal", json!({ "proposal_id": p["proposal_id"], "decision": "approve", "rationale": "ok", "digest": p["digest"] })).await;
    assert!(!err, "{}", done);
    started.elapsed()
}

fn wait_for_job(repo: &PostgresRepository, id: uuid::Uuid) -> scaffoldry_server::jobs::Job {
    let deadline = Instant::now() + Duration::from_secs(120);
    loop {
        let job = get_job(repo, id).expect("query").expect("job exists");
        if job.state == "done" || job.state == "failed" {
            return job;
        }
        assert!(Instant::now() < deadline, "job stuck in {}", job.state);
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn sort_args(slug: &str) -> Value {
    json!({ "app_slug": slug, "table_id": "t1", "sort": { "field_name": "score", "direction": "Desc" }, "limit": 3 })
}

/// A tool call that keeps the whole result, so a test can read the code.
async fn tool_full(w: &World, name: &str, args: Value) -> Value {
    let payload = json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": { "name": name, "arguments": args } });
    let (_, v) = send(&w.app, "POST", "/api/mcp", &w.token, Some(payload)).await;
    v["result"].clone()
}

// 1. The approval returns at once and leaves `building`. The job then builds every row.
#[tokio::test]
async fn approving_an_indexed_field_on_a_big_table_returns_at_once_and_a_job_builds_the_index() {
    let _db = common::TestDb::new();
    let w = world().await;
    let took = approve_sort_on_score(&w, ROWS).await;
    println!("jobs: the approval returned in {took:?} on {ROWS} rows");
    assert!(took < Duration::from_secs(1), "the approval took {took:?}");
    assert_eq!(status(&w.slug, "score").as_deref(), Some("building"));
    assert_eq!(index_rows(&w.slug, "score"), 0, "the approval copies nothing");

    let repo = repo();
    let id = job_of(&w.slug, "score");
    let runner = JobRunner::start(&repo, JOB_KINDS, fast()).unwrap();
    let job = wait_for_job(&repo, id);
    runner.stop();
    assert_eq!(job.state, "done", "{:?}", job.error);
    assert_eq!(status(&w.slug, "score").as_deref(), Some("ready"));
    assert_eq!(index_rows(&w.slug, "score"), ROWS, "every record has an index row");
}

// 2. A sort on a building field is refused with a percent. After the job it works.
#[tokio::test]
async fn a_sort_on_a_building_field_is_refused_with_a_percent_and_works_after_the_job() {
    let _db = common::TestDb::new();
    let w = world().await;
    approve_sort_on_score(&w, SMALL).await;
    let id = job_of(&w.slug, "score");
    db(move |c| c.execute("UPDATE jobs SET progress_done = 2500, progress_total = 5000 WHERE id = $1", &[&id]).unwrap());

    let result = tool_full(&w, "list_records", sort_args(&w.slug)).await;
    assert_eq!(result["isError"], true, "{result}");
    assert_eq!(result["structuredContent"]["code"], "index_building", "{result}");
    assert_eq!(result["content"][0]["text"], "This field is still being prepared for sorting. It is 50% done.");

    let repo = repo();
    let runner = JobRunner::start(&repo, JOB_KINDS, fast()).unwrap();
    wait_for_job(&repo, id);
    runner.stop();
    let (err, page) = tool(&w, "list_records", sort_args(&w.slug)).await;
    assert!(!err, "{page}");
    assert_eq!(page["records"][0]["data"]["score"], SMALL);
}

// 3. A record written during the build has its index row at once, and the job leaves it alone.
#[tokio::test]
async fn a_record_written_during_the_build_keeps_its_row_and_the_job_does_not_touch_it() {
    let _db = common::TestDb::new();
    let w = world().await;
    approve_sort_on_score(&w, SMALL).await;
    let written = create(&w, json!({ "score": 7_000_000, "plain_text": "new" })).await;
    let rid = written["id"].as_str().unwrap().to_string();
    let read = {
        let rid = rid.clone();
        move || { let rid = rid.clone(); db(move |c| c.query("SELECT num, xmin::text FROM record_index WHERE record_id = $1 AND field = 'score'", &[&rid]).unwrap().iter().map(|r| (r.get::<_, Option<f64>>(0), r.get::<_, String>(1))).collect::<Vec<_>>()) }
    };
    let before = read();
    assert_eq!(before.len(), 1, "a write during the build writes the field's row");
    assert_eq!(before[0].0, Some(7_000_000.0));

    let repo = repo();
    let id = job_of(&w.slug, "score");
    let runner = JobRunner::start(&repo, JOB_KINDS, fast()).unwrap();
    wait_for_job(&repo, id);
    runner.stop();
    assert_eq!(read(), before, "the job did not rewrite the row");
    assert_eq!(index_rows(&w.slug, "score"), SMALL + 1, "and every other record has exactly one row");
}

// 4. A worker that died halfway: the job resumes from its checkpoint and the index is complete.
#[tokio::test]
async fn a_killed_build_resumes_from_its_checkpoint_with_no_duplicate_rows() {
    let _db = common::TestDb::new();
    let w = world().await;
    approve_sort_on_score(&w, SMALL).await;
    let repo = repo();
    let id = job_of(&w.slug, "score");
    let cfg = JobConfig { lock_secs: 1, recovery_secs: 1, ..fast() };

    // A worker claims the job, writes the first half, saves its checkpoint, and dies.
    {
        let (repo, cfg) = (repo.clone(), cfg.clone());
        let claimed = std::thread::spawn(move || {
            let mut ghost = repo.dedicated_client().unwrap();
            claim_next(&mut ghost, "ghost", JOB_KINDS, &cfg).unwrap().is_some()
        })
        .join()
        .unwrap();
        assert!(claimed, "the ghost claimed a build");
    }
    let slug = w.slug.clone();
    let half = SMALL / 2;
    db(move |c| {
        c.execute(
            "INSERT INTO record_index (app_slug, table_id, field, record_id, num) \
             SELECT app_slug, 't1', 'score', id, (data->>'score')::float8 FROM dataset_records \
             WHERE app_slug = $1 AND (data->>'score')::int <= $2",
            &[&slug, &(half as i32)],
        )
        .unwrap();
        let last = c
            .query_one("SELECT created_at::text, id FROM dataset_records WHERE app_slug = $1 AND (data->>'score')::int = $2", &[&slug, &(half as i32)])
            .unwrap();
        let cp = json!({ "created_at": last.get::<_, String>(0), "id": last.get::<_, String>(1) });
        c.execute("UPDATE jobs SET checkpoint = $2, locked_until = NOW() - interval '1 second' WHERE id = $1", &[&id, &cp]).unwrap();
    });
    let first = {
        let slug = w.slug.clone();
        db(move |c| c.query_one("SELECT xmin::text FROM record_index WHERE app_slug = $1 AND field = 'score' ORDER BY record_id LIMIT 1", &[&slug]).unwrap().get::<_, String>(0))
    };

    let runner = JobRunner::start(&repo, JOB_KINDS, cfg).unwrap();
    let job = wait_for_job(&repo, id);
    runner.stop();
    assert_eq!(job.state, "done", "{:?}", job.error);
    assert_eq!(index_rows(&w.slug, "score"), SMALL, "complete, one row per record");
    let again = {
        let slug = w.slug.clone();
        db(move |c| c.query_one("SELECT xmin::text FROM record_index WHERE app_slug = $1 AND field = 'score' ORDER BY record_id LIMIT 1", &[&slug]).unwrap().get::<_, String>(0))
    };
    assert_eq!(first, again, "the first half was not written twice");
    assert_eq!(status(&w.slug, "score").as_deref(), Some("ready"));
}

// 5. A raised label protects a read at once. The job then catches the stored flags up.
#[tokio::test]
async fn a_raised_label_protects_a_read_before_the_flag_job_has_run() {
    let _db = common::TestDb::new();
    let w = world().await;
    fill(&w.slug, 200);
    let viewer = issue_test_token_and_user(VIEWER);
    let read = |token: String| {
        let payload = json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": { "name": "list_records", "arguments": { "app_slug": w.slug, "table_id": "t1", "limit": 5 } } });
        let app = w.app.clone();
        async move { send(&app, "POST", "/api/mcp", &token, Some(payload)).await.1["result"]["structuredContent"].clone() }
    };
    let before = read(viewer.clone()).await;
    assert!(before["records"][0]["data"].get("plain_text").is_some(), "no label yet: {before}");

    w.state.labels.write().unwrap().insert(
        (w.slug.clone(), "t1".to_string(), "plain_text".to_string()),
        DataLabel { ferpa_sensitive: true, note: "now sensitive".into(), set_by: "compliance@state.edu".into() },
    );
    let after = read(viewer).await;
    assert!(after["records"][0]["data"].get("plain_text").is_none(), "the label protects the read at once: {after}");
    let stored: i64 = {
        let slug = w.slug.clone();
        db(move |c| c.query_one("SELECT count(*) FROM dataset_records WHERE app_slug = $1 AND is_ferpa_sensitive", &[&slug]).unwrap().get(0))
    };
    assert_eq!(stored, 0, "the stored flags have not caught up yet");

    let repo = repo();
    let id = scaffoldry_server::jobs::enqueue(&repo, "recompute_flags", json!({ "app_slug": w.slug, "table_id": "t1", "fields": ["plain_text"] }), "compliance@state.edu", None).unwrap();
    let runner = JobRunner::start(&repo, JOB_KINDS, fast()).unwrap();
    let job = wait_for_job(&repo, id);
    runner.stop();
    assert_eq!(job.state, "done", "{:?}", job.error);
    let stored: i64 = {
        let slug = w.slug.clone();
        db(move |c| c.query_one("SELECT count(*) FROM dataset_records WHERE app_slug = $1 AND is_ferpa_sensitive", &[&slug]).unwrap().get(0))
    };
    assert_eq!(stored, 200, "the job raised every record that holds the field");
}
