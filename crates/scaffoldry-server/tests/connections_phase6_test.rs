//! Connections phase 6: the connections panel.
//!
//! The console shows each connection's health in words, its run history, and what the next run is. A failing
//! connection tells the owner and the Platform Admins once. The Overview counts them.

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::Router;
use common::TestDb;
use http_body_util::BodyExt;
use scaffoldry_server::build_app_with_state;
use scaffoldry_server::service::identity::issue_test_token_and_user;
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

fn unique(prefix: &str) -> String {
    format!("{prefix}-{}", &uuid::Uuid::new_v4().simple().to_string()[..8])
}

struct Conn {
    id: String,
    app: String,
}

fn rows(codes: &[&str]) -> String {
    let mut out = String::from("Code,Name,Credits\n");
    for code in codes {
        out.push_str(&format!("{code},Program {code},3\n"));
    }
    out
}

fn spec(name: &str, app: &str, path: &str, extra: Value) -> Value {
    let mut v = json!({
        "name": name, "kind": "file_drop", "reason": "The registrar's nightly programs file",
        "config": { "path": path, "format": "csv" },
        "dataset_app": app, "dataset_table": "programs", "key_field": "code",
        "column_map": [
            { "source": "Code", "field": "code", "type": "Text" },
            { "source": "Name", "field": "name", "type": "Text" },
            { "source": "Credits", "field": "credits", "type": "Number" },
        ],
        "sensitivity": "Directory",
    });
    v.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
    v
}

async fn connect(w: &World, path: &str, extra: Value) -> Conn {
    let (name, app) = (unique("Programs"), unique("programs").to_lowercase());
    let (s, b) = send(&w.app, "POST", "/api/v1/admin/connections", ADMIN, Some(spec(&name, &app, path, extra))).await;
    assert_eq!(s, StatusCode::CREATED, "{b}");
    Conn { id: b["connection"]["id"].as_str().unwrap().to_string(), app }
}

async fn run(w: &World, c: &Conn) -> Value {
    let (s, b) = send(&w.app, "POST", &format!("/api/v1/admin/connections/{}/run", c.id), ADMIN, Some(json!({}))).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    b["run"].clone()
}

async fn connection(w: &World, c: &Conn) -> Value {
    let (_, b) = send(&w.app, "GET", "/api/v1/admin/connections", ADMIN, None).await;
    b["connections"].as_array().unwrap().iter().find(|x| x["id"] == c.id.as_str()).cloned().unwrap_or_else(|| panic!("the connection: {b}"))
}

fn told(w: &World, c: &Conn) -> Vec<String> {
    let app = c.app.clone();
    w.db.run(move |db| db.query("SELECT recipient FROM notifications WHERE kind = 'sync_failed' AND app_slug = $1 ORDER BY recipient", &[&app]).unwrap().iter().map(|r| r.get(0)).collect())
}

async fn overview(w: &World) -> Value {
    let (s, b) = send(&w.app, "GET", "/api/v1/admin/overview", ADMIN, None).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    b["connections"].clone()
}

// 1. A failing connection shows Failing with its error, and tells the owner and the Platform Admins once, not on every
// retry. When it recovers and fails again, that is a new failure and tells them again.
#[tokio::test(flavor = "multi_thread")]
async fn a_failing_connection_says_so_and_notifies_once() {
    let w = world();
    let c = connect(&w, "sis/programs.csv", json!({})).await;
    for _ in 0..3 {
        let r = run(&w, &c).await;
        assert_eq!(r["state"], "failed", "{r}");
    }
    let conn = connection(&w, &c).await;
    assert_eq!(conn["state"], "Failing", "{conn}");
    assert!(!conn["last_run"]["error"].as_str().unwrap_or("").is_empty(), "{conn}");
    assert_eq!(conn["last_result"], conn["last_run"]["error"], "a failed run says its error: {conn}");
    assert_eq!(told(&w, &c), vec![ADMIN.to_string()], "one notice for the admin, though it failed three times");

    put(&w, "sis/programs.csv", &rows(&["P1", "P2"]));
    assert_eq!(run(&w, &c).await["state"], "done");
    assert_eq!(connection(&w, &c).await["state"], "Healthy");
    std::fs::remove_file(w.drop.join("sis/programs.csv")).unwrap();
    assert_eq!(run(&w, &c).await["state"], "failed");
    assert_eq!(told(&w, &c).len(), 2, "a new failure after a good run tells again");
}

// 1b. A change that needs review tells them too, and only once.
#[tokio::test(flavor = "multi_thread")]
async fn a_change_needing_review_notifies_once() {
    let w = world();
    put(&w, "sis/programs.csv", &rows(&["P1", "P2"]));
    let c = connect(&w, "sis/programs.csv", json!({})).await;
    assert_eq!(run(&w, &c).await["state"], "done");
    put(&w, "sis/programs.csv", "Code,Name,Credits,Campus\nP1,Program P1,3,North\nP2,Program P2,3,North\n");
    run(&w, &c).await;
    run(&w, &c).await;
    assert_eq!(connection(&w, &c).await["state"], "Needs review");
    assert_eq!(told(&w, &c).len(), 1, "{:?}", told(&w, &c));
}

// 2. Disable stops the schedule and asks for a reason. Enable asks for one too.
#[tokio::test(flavor = "multi_thread")]
async fn disable_stops_the_schedule_and_asks_for_a_reason() {
    let w = world();
    put(&w, "sis/programs.csv", &rows(&["P1"]));
    let c = connect(&w, "sis/programs.csv", json!({ "every_minutes": 60 })).await;
    let uri = format!("/api/v1/admin/connections/{}", c.id);

    let fresh = connection(&w, &c).await;
    assert!(fresh["next_run"].is_string(), "a scheduled connection that never ran is due now: {fresh}");
    let (s, b) = send(&w.app, "PATCH", &uri, ADMIN, Some(json!({ "enabled": false }))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{b}");
    assert_eq!(connection(&w, &c).await["state"], "Healthy", "nothing changed without a reason");

    let (s, b) = send(&w.app, "PATCH", &uri, ADMIN, Some(json!({ "enabled": false, "reason": "The registrar is moving the export" }))).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    let off = connection(&w, &c).await;
    assert_eq!(off["state"], "Disabled", "{off}");
    assert!(off["next_run"].is_null(), "a disabled connection has no next run: {off}");
    let before = scaffoldry_server::connections::runs(&w.state.repository, c.id.parse().unwrap()).unwrap().len();
    scaffoldry_server::connections::sync_due(&w.state).unwrap();
    let after = scaffoldry_server::connections::runs(&w.state.repository, c.id.parse().unwrap()).unwrap().len();
    assert_eq!(before, after, "the schedule does not run a disabled connection");

    let (s, b) = send(&w.app, "PATCH", &uri, ADMIN, Some(json!({ "enabled": true, "reason": "The export is back" }))).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_eq!(connection(&w, &c).await["state"], "Healthy");
}

// 2b. The next run follows the schedule: an hour after the last run for an interval.
#[tokio::test(flavor = "multi_thread")]
async fn the_next_run_follows_the_schedule() {
    let w = world();
    put(&w, "sis/programs.csv", &rows(&["P1"]));
    let c = connect(&w, "sis/programs.csv", json!({ "every_minutes": 60 })).await;
    let r = run(&w, &c).await;
    let started = chrono::DateTime::parse_from_rfc3339(r["started_at"].as_str().unwrap()).unwrap();
    let next = chrono::DateTime::parse_from_rfc3339(connection(&w, &c).await["next_run"].as_str().unwrap()).unwrap();
    assert_eq!((next - started).num_minutes(), 60);
    let daily = connect(&w, "sis/programs.csv", json!({ "at_time": "02:30" })).await;
    let n = connection(&w, &daily).await;
    assert!(n["next_run"].is_string(), "{n}");
    let none = connect(&w, "sis/programs.csv", json!({})).await;
    assert!(connection(&w, &none).await["next_run"].is_null(), "no schedule, no next run");
}

// 2c. The last result is in words.
#[tokio::test(flavor = "multi_thread")]
async fn the_last_result_is_in_words() {
    let w = world();
    let c = connect(&w, "sis/programs.csv", json!({})).await;
    assert_eq!(connection(&w, &c).await["last_result"], "Not run yet");
    put(&w, "sis/programs.csv", &rows(&["P1", "P2", "P3"]));
    run(&w, &c).await;
    assert_eq!(connection(&w, &c).await["last_result"], "Added 3");
    put(&w, "sis/programs.csv", "Code,Name,Credits\nP1,Renamed,3\nP2,Program P2,3\nP4,Program P4,3\n");
    run(&w, &c).await;
    assert_eq!(connection(&w, &c).await["last_result"], "Added 1, changed 1, removed 1");
    run(&w, &c).await;
    assert_eq!(connection(&w, &c).await["last_result"], "No change since the last run");
}

// 3. The history shows the last 100 runs.
#[tokio::test(flavor = "multi_thread")]
async fn the_history_shows_the_last_hundred_runs() {
    let w = world();
    put(&w, "sis/programs.csv", &rows(&["P1"]));
    let c = connect(&w, "sis/programs.csv", json!({})).await;
    let id: uuid::Uuid = c.id.parse().unwrap();
    w.db.run(move |db| {
        db.execute(
            "INSERT INTO sync_runs (id, connection_id, state, started_at, finished_at) \
             SELECT gen_random_uuid(), $1, 'unchanged', NOW() - make_interval(mins => n), NOW() - make_interval(mins => n) FROM generate_series(10, 130) n",
            &[&id],
        )
        .unwrap();
    });
    run(&w, &c).await;
    let (s, b) = send(&w.app, "GET", &format!("/api/v1/admin/connections/{}/runs", c.id), ADMIN, None).await;
    assert_eq!(s, StatusCode::OK);
    let runs = b["runs"].as_array().unwrap();
    assert_eq!(runs.len(), 100, "the newest hundred");
    assert_eq!(runs[0]["state"], "done", "the newest first");
}

// 4. The Overview counts match the fixture, and show the age of the oldest successful sync.
#[tokio::test(flavor = "multi_thread")]
async fn the_overview_counts_match_the_fixture() {
    let w = world();
    let before = overview(&w).await;
    put(&w, "sis/good.csv", &rows(&["P1", "P2"]));
    put(&w, "sis/drift.csv", &rows(&["P1"]));
    let good = connect(&w, "sis/good.csv", json!({})).await;
    let failing = connect(&w, "sis/missing.csv", json!({})).await;
    let review = connect(&w, "sis/drift.csv", json!({})).await;
    let off = connect(&w, "sis/good.csv", json!({})).await;
    run(&w, &good).await;
    run(&w, &failing).await;
    run(&w, &review).await;
    put(&w, "sis/drift.csv", "Code,Name,Credits,Campus\nP1,Program P1,3,North\n");
    run(&w, &review).await;
    let (s, b) = send(&w.app, "PATCH", &format!("/api/v1/admin/connections/{}", off.id), ADMIN, Some(json!({ "enabled": false, "reason": "Paused" }))).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    let id: uuid::Uuid = good.id.parse().unwrap();
    w.db.run(move |db| {
        db.execute("UPDATE sync_runs SET started_at = NOW() - INTERVAL '2 days', finished_at = NOW() - INTERVAL '2 days' WHERE connection_id = $1", &[&id]).unwrap();
    });

    let after = overview(&w).await;
    let delta = |k: &str| after[k].as_i64().unwrap() - before[k].as_i64().unwrap();
    assert_eq!((delta("healthy"), delta("failing"), delta("needs_review"), delta("disabled")), (1, 1, 1, 1), "{before} {after}");
    let age = after["oldest_success_age_secs"].as_i64().unwrap();
    assert!((172_800..172_800 + 600).contains(&age), "two days old: {age}");
}

// 5. Edit mapping opens the column map with a sample of what the last run wrote.
#[tokio::test(flavor = "multi_thread")]
async fn the_mapping_shows_a_sample_of_the_last_file() {
    let w = world();
    put(&w, "sis/programs.csv", &rows(&["P1", "P2", "P3", "P4", "P5"]));
    let c = connect(&w, "sis/programs.csv", json!({})).await;
    let (_, empty) = send(&w.app, "GET", &format!("/api/v1/admin/connections/{}/mapping", c.id), ADMIN, None).await;
    assert_eq!(empty["columns"].as_array().unwrap().len(), 3);
    assert_eq!(empty["columns"][0]["sample"].as_array().unwrap().len(), 0, "nothing synced yet: {empty}");
    run(&w, &c).await;
    let (s, b) = send(&w.app, "GET", &format!("/api/v1/admin/connections/{}/mapping", c.id), ADMIN, None).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    let code = &b["columns"][0];
    assert_eq!((code["source"].as_str(), code["field"].as_str(), code["type"].as_str(), code["key"].as_bool()), (Some("Code"), Some("code"), Some("Text"), Some(true)));
    assert_eq!(code["sample"], json!(["P1", "P2", "P3"]), "{b}");
    let (s, _) = send(&w.app, "GET", &format!("/api/v1/admin/connections/{}/mapping", c.id), "someone.else@state.edu", None).await;
    assert_eq!(s, StatusCode::FORBIDDEN);
}
