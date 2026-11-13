//! Connections phase 2: when the source changes shape.
//!
//! A changed source stops the run and makes a proposal for the dataset. Approving it changes the dataset's definition
//! and the connection's column map together, and the next run goes ahead. Nothing is added or dropped on its own.

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::Router;
use common::TestDb;
use http_body_util::BodyExt;
use scaffoldry_server::build_app_with_state;
use scaffoldry_server::service::identity::issue_test_token_and_user;
use scaffoldry_server::jobs::{get_job, JobConfig, JobRunner, JOB_KINDS};
use scaffoldry_server::state::ServerState;
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::Arc;
use tower::ServiceExt;

const ADMIN: &str = "jordan.lee@state.edu";

struct World {
    db: TestDb,
    state: Arc<ServerState>,
    app: Router,
    drop: PathBuf,
}

/// One files directory for the whole binary: the setting is a process variable. A snapshot is written there.
fn files_dir() {
    static DIR: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();
    DIR.get_or_init(|| {
        let dir = std::env::temp_dir().join(format!("scaffoldry-snapshots-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        std::env::set_var("SCAFFOLDRY_FILES_DIR", &dir);
        dir
    });
}

fn world() -> World {
    files_dir();
    let db = TestDb::new();
    let mut state = ServerState::connect(db.url()).expect("server");
    let drop = std::env::temp_dir().join(format!("scaffoldry-drop-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(drop.join("sis")).unwrap();
    state.drop_dir = Some(drop.clone());
    let state = Arc::new(state);
    state.seed_demo().expect("seed");
    let app = build_app_with_state(state.clone()).expect("router");
    World { db, state, app, drop }
}

async fn send(app: &Router, method: &str, uri: &str, who: &str, body: Option<Value>) -> (StatusCode, Value) {
    let b = Request::builder().method(method).uri(uri).header("authorization", format!("Bearer {}", issue_test_token_and_user(who)));
    let req = match body {
        Some(v) => b.header("content-type", "application/json").body(Body::from(v.to_string())).unwrap(),
        None => b.body(Body::empty()).unwrap(),
    };
    let resp = app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
}

fn put(w: &World, rel: &str, text: &str) {
    let path = w.drop.join(rel);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

/// A programs file: `rows` as `(code, name, credits)`.
fn programs(rows: &[(String, String, i64)]) -> String {
    let mut out = String::from("Code,Name,Credits\n");
    for (code, name, credits) in rows {
        out.push_str(&format!("{code},{name},{credits}\n"));
    }
    out
}

fn rows(n: usize) -> Vec<(String, String, i64)> {
    (1..=n).map(|i| (format!("P{i:04}"), format!("Program {i}"), 3 + (i % 3) as i64)).collect()
}

fn unique(prefix: &str) -> String {
    format!("{prefix}-{}", &uuid::Uuid::new_v4().simple().to_string()[..8])
}

struct Conn {
    id: String,
    app: String,
    name: String,
}

fn body(name: &str, app: &str, path: &str) -> Value {
    json!({
        "name": name, "kind": "file_drop", "reason": "The registrar's nightly programs file",
        "config": { "path": path, "format": "csv" },
        "dataset_app": app, "dataset_table": "programs", "key_field": "code",
        "column_map": [
            { "source": "Code", "field": "code", "type": "Text" },
            { "source": "Name", "field": "name", "type": "Text" },
            { "source": "Credits", "field": "credits", "type": "Number" },
        ],
        "sensitivity": "Directory",
    })
}

async fn connect(w: &World, path: &str) -> Conn {
    let (name, app) = (unique("Programs"), unique("programs").to_lowercase());
    let (s, b) = send(&w.app, "POST", "/api/v1/admin/connections", ADMIN, Some(body(&name, &app, path))).await;
    assert_eq!(s, StatusCode::CREATED, "{b}");
    Conn { id: b["connection"]["id"].as_str().unwrap().to_string(), app, name }
}

async fn run(w: &World, c: &Conn, approve: bool) -> Value {
    let (s, b) = send(&w.app, "POST", &format!("/api/v1/admin/connections/{}/run", c.id), ADMIN, Some(json!({ "approve_shrink": approve, "reason": "The registrar confirmed the cut" }))).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    b["run"].clone()
}

fn count(w: &World, sql: &str, app: &str) -> i64 {
    let (sql, app) = (sql.to_string(), app.to_string());
    w.db.run(move |c| c.query_one(sql.as_str(), &[&app]).unwrap().get(0))
}

fn live(w: &World, app: &str) -> i64 {
    count(w, "SELECT COUNT(*) FROM dataset_records WHERE app_slug = $1 AND deleted_at IS NULL", app)
}


fn proposals_of(w: &World, app: &str) -> Vec<(String, String, String)> {
    let app = app.to_string();
    w.db.run(move |c| c.query("SELECT id::text, status, summary FROM app_proposals WHERE app_slug = $1 ORDER BY created_at", &[&app]).unwrap().iter().map(|r| (r.get(0), r.get(1), r.get(2))).collect())
}

async fn connection(w: &World, c: &Conn) -> Value {
    let (_, b) = send(&w.app, "GET", "/api/v1/admin/connections", ADMIN, None).await;
    b["connections"].as_array().unwrap().iter().find(|x| x["id"] == c.id.as_str()).cloned().unwrap_or_else(|| panic!("the connection: {b}"))
}

async fn decide(w: &World, id: &str, decision: &str) {
    let (s, p) = send(&w.app, "GET", &format!("/api/v1/proposals/{id}"), ADMIN, None).await;
    assert_eq!(s, StatusCode::OK, "{p}");
    let (s, b) = send(&w.app, "POST", &format!("/api/v1/proposals/{id}/decision"), ADMIN, Some(json!({ "decision": decision, "rationale": "Reviewed the change", "digest": p["digest"] }))).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    // An approval that removes a field waits for a snapshot, which a job takes. Wait for it.
    if decision == "approve" {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(120);
        loop {
            let status = proposal_status(w, id);
            if status.eq_ignore_ascii_case("approved") {
                break;
            }
            assert!(std::time::Instant::now() < deadline, "the proposal stays {status}");
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
    }
}

fn proposal_status(w: &World, id: &str) -> String {
    let id = uuid::Uuid::parse_str(id).unwrap();
    w.db.run(move |c| c.query_one("SELECT status FROM app_proposals WHERE id = $1", &[&id]).unwrap().get(0))
}

fn data_of(w: &World, app: &str, code: &str) -> Value {
    let (app, code) = (app.to_string(), code.to_string());
    w.db.run(move |c| c.query_one("SELECT data FROM dataset_records WHERE app_slug = $1 AND data->>'code' = $2", &[&app, &code]).unwrap().get(0))
}

fn with_campus(rows: &[(String, String, i64)]) -> String {
    let mut out = String::from("Code,Name,Credits,Campus\n");
    for (code, name, credits) in rows {
        out.push_str(&format!("{code},{name},{credits},North\n"));
    }
    out
}

// 1. A file with a new column stops the run and creates one proposal naming the column. Approving it lets the next
// run add the column's values.
#[tokio::test(flavor = "multi_thread")]
async fn a_new_column_stops_the_run_and_one_proposal_names_it() {
    let w = world();
    let data = rows(20);
    put(&w, "sis/programs.csv", &programs(&data));
    let c = connect(&w, "sis/programs.csv").await;
    run(&w, &c, false).await;

    put(&w, "sis/programs.csv", &with_campus(&data));
    let stopped = run(&w, &c, false).await;
    assert_eq!(stopped["state"], "failed", "{stopped}");
    assert!(stopped["error"].as_str().unwrap().starts_with("schema_drift"), "{stopped}");
    assert!(stopped["error"].as_str().unwrap().contains("The source added \"Campus\"."), "{stopped}");
    let list = proposals_of(&w, &c.app);
    assert_eq!(list.len(), 1, "one proposal: {list:?}");
    assert!(list[0].2.contains("Campus"), "the summary names the column: {list:?}");
    assert!(list[0].2.contains(&c.name), "and the connection: {list:?}");
    let conn = connection(&w, &c).await;
    assert_eq!(conn["state"], "Needs review", "{conn}");
    assert_eq!(conn["pending_proposal"], list[0].0.as_str());

    // Another run before anyone decides makes no second proposal and changes nothing.
    run(&w, &c, false).await;
    assert_eq!(proposals_of(&w, &c.app).len(), 1);
    assert_eq!(live(&w, &c.app), 20);
    assert!(data_of(&w, &c.app, "P0001").get("campus").is_none());

    decide(&w, &list[0].0, "approve").await;
    let after = connection(&w, &c).await;
    assert_eq!(after["state"], "Healthy", "{after}");
    assert!(after["column_map"].to_string().contains("Campus"), "the column map and the manifest changed together: {after}");
    let ran = run(&w, &c, false).await;
    assert_eq!(ran["state"], "done", "{ran}");
    assert_eq!((ran["changed"].as_i64(), ran["added"].as_i64()), (Some(20), Some(0)), "{ran}");
    assert_eq!(data_of(&w, &c.app, "P0001")["campus"], "North");
}

// 2. A file missing a mapped column stops the run, and the proposal retires the field. No data is dropped.
#[tokio::test(flavor = "multi_thread")]
async fn a_missing_column_retires_its_field_and_keeps_the_values() {
    let w = world();
    let runner = JobRunner::start(&w.state.repository, JOB_KINDS, JobConfig { workers: 2, poll_ms: 20, backoff_secs: [0, 0, 0], ..JobConfig::default() }).unwrap();
    let data = rows(10);
    put(&w, "sis/programs.csv", &programs(&data));
    let c = connect(&w, "sis/programs.csv").await;
    run(&w, &c, false).await;

    let mut text = String::from("Code,Name\n");
    for (code, name, _) in &data {
        text.push_str(&format!("{code},{name}\n"));
    }
    put(&w, "sis/programs.csv", &text);
    let stopped = run(&w, &c, false).await;
    assert!(stopped["error"].as_str().unwrap().contains("The source no longer has \"Credits\"."), "{stopped}");
    let list = proposals_of(&w, &c.app);
    assert_eq!(list.len(), 1, "{list:?}");
    decide(&w, &list[0].0, "approve").await;

    let manifest: Value = {
        let app = c.app.clone();
        w.db.run(move |cl| cl.query_one("SELECT manifest FROM app_manifests WHERE slug = $1", &[&app]).unwrap().get(0))
    };
    let table = &manifest["tables"][0];
    assert!(table["fields"].to_string().find("credits").is_none(), "the field left the table: {table}");
    assert!(table["retired_fields"].to_string().contains("credits"), "and is retired: {table}");

    let ran = run(&w, &c, false).await;
    assert_eq!(ran["state"], "done", "{ran}");
    assert_eq!(ran["changed"], 0, "nothing was rewritten: {ran}");
    assert_eq!(data_of(&w, &c.app, "P0001")["credits"], 4, "the retired field's values are kept");
    runner.stop();
}

// 3. Rejecting a proposal leaves the connection `Needs review` and no data changed. A scheduled run makes no new one.
#[tokio::test(flavor = "multi_thread")]
async fn a_rejected_proposal_leaves_the_connection_stopped() {
    let w = world();
    let data = rows(10);
    put(&w, "sis/programs.csv", &programs(&data));
    let c = connect(&w, "sis/programs.csv").await;
    run(&w, &c, false).await;
    put(&w, "sis/programs.csv", &with_campus(&data));
    run(&w, &c, false).await;
    let id = proposals_of(&w, &c.app)[0].0.clone();
    decide(&w, &id, "reject").await;

    let conn = connection(&w, &c).await;
    assert_eq!(conn["state"], "Needs review", "{conn}");
    assert!(conn["review_note"].as_str().unwrap().contains("rejected"), "the console can say so: {conn}");
    let ran = run(&w, &c, false).await;
    assert_eq!(ran["state"], "failed", "{ran}");
    assert!(ran["error"].as_str().unwrap().contains("rejected"), "{ran}");
    assert_eq!(proposals_of(&w, &c.app).len(), 1, "no new proposal for the same change");
    assert!(data_of(&w, &c.app, "P0001").get("campus").is_none());
    assert_eq!(live(&w, &c.app), 10);

    // Put the file back as it was, and the connection goes on.
    put(&w, "sis/programs.csv", &programs(&data));
    let back = run(&w, &c, false).await;
    assert_eq!(back["state"], "unchanged", "{back}");
    assert_eq!(connection(&w, &c).await["state"], "Healthy");
}

// 4. A type change shows a conversion preview with counts, and approving it converts nothing by itself.
#[tokio::test(flavor = "multi_thread")]
async fn a_type_change_shows_a_conversion_preview_with_counts() {
    let w = world();
    let data = rows(30);
    put(&w, "sis/programs.csv", &programs(&data));
    let c = connect(&w, "sis/programs.csv").await;
    run(&w, &c, false).await;

    // The credits of three programs are now words, so the column is text.
    let mut text = String::from("Code,Name,Credits\n");
    for (i, (code, name, credits)) in data.iter().enumerate() {
        let value = if i < 3 { "three".to_string() } else { credits.to_string() };
        text.push_str(&format!("{code},{name},{value}\n"));
    }
    put(&w, "sis/programs.csv", &text);

    let repo = w.state.repository.clone();
    let runner = JobRunner::start(&repo, JOB_KINDS, JobConfig { workers: 2, poll_ms: 20, backoff_secs: [0, 0, 0], ..JobConfig::default() }).unwrap();
    // The first run asks for a preview of the conversion, and says so.
    let first = run(&w, &c, false).await;
    assert_eq!(first["state"], "failed", "{first}");
    assert!(first["error"].as_str().unwrap().contains("preview"), "{first}");
    assert!(proposals_of(&w, &c.app).is_empty(), "no proposal before the preview is done");
    let app = c.app.clone();
    let job: uuid::Uuid = w.db.run(move |cl| cl.query_one("SELECT id FROM jobs WHERE kind = 'convert_preview' AND payload->>'app_slug' = $1", &[&app]).unwrap().get(0));
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(120);
    loop {
        let j = get_job(&repo, job).unwrap().unwrap();
        if j.state == "done" {
            break;
        }
        assert!(j.state != "failed" && std::time::Instant::now() < deadline, "the preview job: {:?}", j);
        std::thread::sleep(std::time::Duration::from_millis(50));
    }

    let second = run(&w, &c, false).await;
    assert_eq!(second["state"], "failed", "{second}");
    let list = proposals_of(&w, &c.app);
    assert_eq!(list.len(), 1, "{list:?}");
    let (s, p) = send(&w.app, "GET", &format!("/api/v1/proposals/{}", list[0].0), ADMIN, None).await;
    assert_eq!(s, StatusCode::OK, "{p}");
    let checks = p["checks"].as_array().or_else(|| p["proposal"]["checks"].as_array()).cloned().unwrap_or_default();
    let loss = checks.iter().find(|x| x["name"] == "data_loss").unwrap_or_else(|| panic!("a data_loss check: {p}"));
    let preview = &loss["previews"][0];
    assert_eq!(preview["scanned"], 30, "{loss}");
    // The preview reads what the dataset holds now: 30 numbers that would be written as text, and none lost.
    assert_eq!(preview["converts"], 30, "{loss}");
    assert_eq!(preview["blanks"], 0, "{loss}");

    // Approving it queues no conversion of the dataset's records: only the sync writes them.
    decide(&w, &list[0].0, "approve").await;
    let app = c.app.clone();
    let converts: i64 = w.db.run(move |cl| cl.query_one("SELECT COUNT(*) FROM jobs WHERE kind = 'convert_field' AND payload->>'app_slug' = $1", &[&app]).unwrap().get(0));
    assert_eq!(converts, 0);
    let ran = run(&w, &c, false).await;
    assert_eq!(ran["state"], "done", "{ran}");
    assert_eq!(data_of(&w, &c.app, "P0001")["credits"], "three", "the next run writes the column as text");
    assert_eq!(data_of(&w, &c.app, "P0010")["credits"], "4", "numbers too");
    runner.stop();
}
