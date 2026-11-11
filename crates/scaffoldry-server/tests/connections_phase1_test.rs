//! Connections phase 1: datasets and the file-drop connector.
//!
//! A connection copies a file from the drop directory into a read-only dataset on a schedule. Only the sync writes the
//! dataset's records. A changed file updates only the changed rows, and a truncated file stops the run.

mod common;

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use axum::Router;
use common::TestDb;
use http_body_util::BodyExt;
use scaffoldry_server::build_app_with_state;
use scaffoldry_server::service::identity::issue_test_token_and_user;
use scaffoldry_server::state::ServerState;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tower::ServiceExt;

const ADMIN: &str = "jordan.lee@state.edu";
const FACULTY: &str = "einstein@physics.state.edu";

struct World {
    db: TestDb,
    state: Arc<ServerState>,
    app: Router,
    drop: PathBuf,
}

fn world() -> World {
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
    Conn { id: b["connection"]["id"].as_str().unwrap().to_string(), app }
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

fn trashed(w: &World, app: &str) -> i64 {
    count(w, "SELECT COUNT(*) FROM dataset_records WHERE app_slug = $1 AND deleted_at IS NOT NULL", app)
}

fn history_rows(w: &World, app: &str) -> i64 {
    count(w, "SELECT COUNT(*) FROM record_history WHERE app_slug = $1", app)
}

fn counts(run: &Value) -> [i64; 4] {
    ["added", "changed", "removed", "unchanged"].map(|k| run[k].as_i64().unwrap_or(-1))
}

// 1. A fixture CSV of 1,000 rows syncs into a dataset table. A second run on the same file is `unchanged` and writes
// nothing.
#[tokio::test(flavor = "multi_thread")]
async fn a_thousand_rows_sync_and_a_second_run_on_the_same_file_writes_nothing() {
    let w = world();
    put(&w, "sis/programs.csv", &programs(&rows(1000)));
    let c = connect(&w, "sis/programs.csv").await;

    let first = run(&w, &c, false).await;
    assert_eq!(first["state"], "done", "{first}");
    assert_eq!(counts(&first), [1000, 0, 0, 0], "{first}");
    assert_eq!(live(&w, &c.app), 1000);
    let history = history_rows(&w, &c.app);
    assert_eq!(history, 1000, "one history row for each record");

    let second = run(&w, &c, false).await;
    assert_eq!(second["state"], "unchanged", "{second}");
    assert_eq!(counts(&second), [0, 0, 0, 0], "{second}");
    assert_eq!(history_rows(&w, &c.app), history, "nothing was written");
    assert_eq!(first["source_hash"], second["source_hash"]);

    let (s, b) = send(&w.app, "GET", &format!("/api/v1/admin/connections/{}/runs", c.id), ADMIN, None).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_eq!(b["runs"].as_array().unwrap().len(), 2, "{b}");
}

// 2. A changed file updates only the changed rows, adds new ones, and trashes removed ones, with exact counts.
#[tokio::test(flavor = "multi_thread")]
async fn a_changed_file_updates_only_what_changed_with_exact_counts() {
    let w = world();
    let mut data = rows(1000);
    put(&w, "sis/programs.csv", &programs(&data));
    let c = connect(&w, "sis/programs.csv").await;
    run(&w, &c, false).await;

    // 10 rows change, 20 are removed, 5 are new.
    for row in data.iter_mut().take(10) {
        row.2 += 10;
    }
    data.drain(100..120);
    data.extend((1001..=1005).map(|i| (format!("P{i:04}"), format!("Program {i}"), 4)));
    put(&w, "sis/programs.csv", &programs(&data));

    let second = run(&w, &c, false).await;
    assert_eq!(second["state"], "done", "{second}");
    assert_eq!(counts(&second), [5, 10, 20, 970], "{second}");
    assert_eq!(live(&w, &c.app), 985);
    assert_eq!(trashed(&w, &c.app), 20, "removed rows go to trash, not away");
    let untouched = count(&w, "SELECT COUNT(*) FROM dataset_records WHERE app_slug = $1 AND deleted_at IS NULL AND version = 1", &c.app);
    assert_eq!(untouched, 985 - 10, "a row that did not change keeps its version");
}

// 3. A file with 30 percent of the previous rows stops with `source_shrank` and changes nothing. A person may approve it.
#[tokio::test(flavor = "multi_thread")]
async fn a_truncated_file_stops_with_source_shrank_until_a_person_approves() {
    let w = world();
    put(&w, "sis/programs.csv", &programs(&rows(1000)));
    let c = connect(&w, "sis/programs.csv").await;
    run(&w, &c, false).await;

    put(&w, "sis/programs.csv", &programs(&rows(300)));
    let stopped = run(&w, &c, false).await;
    assert_eq!(stopped["state"], "failed", "{stopped}");
    assert!(stopped["error"].as_str().unwrap().starts_with("source_shrank"), "{stopped}");
    assert_eq!(live(&w, &c.app), 1000, "nothing changed");
    assert_eq!(trashed(&w, &c.app), 0);

    // The same file is not `unchanged`: it was never applied. A person's approval applies it.
    let approved = run(&w, &c, true).await;
    assert_eq!(approved["state"], "done", "{approved}");
    assert_eq!(counts(&approved), [0, 0, 700, 300], "{approved}");
    assert_eq!(live(&w, &c.app), 300);
}

// 4. A path that escapes the drop directory by `..`, by an absolute path, or by a link is refused.
#[tokio::test(flavor = "multi_thread")]
async fn a_path_that_escapes_the_drop_directory_is_refused() {
    let w = world();
    for path in ["../outside.csv", "sis/../../outside.csv", "/etc/passwd", ""] {
        let (name, app) = (unique("Escape"), unique("escape"));
        let (s, b) = send(&w.app, "POST", "/api/v1/admin/connections", ADMIN, Some(body(&name, &app, path))).await;
        assert_eq!(s, StatusCode::BAD_REQUEST, "{path:?}: {b}");
    }

    // A link inside the directory that points outside it passes the name check and fails at the run.
    let outside = std::env::temp_dir().join(format!("scaffoldry-outside-{}.csv", uuid::Uuid::new_v4()));
    std::fs::write(&outside, programs(&rows(5))).unwrap();
    std::os::unix::fs::symlink(&outside, w.drop.join("sis/linked.csv")).unwrap();
    let c = connect(&w, "sis/linked.csv").await;
    let r = run(&w, &c, false).await;
    assert_eq!(r["state"], "failed", "{r}");
    assert!(r["error"].as_str().unwrap().contains("drop directory"), "{r}");
    assert_eq!(live(&w, &c.app), 0, "nothing was read");
    let _ = Path::new(&outside);
}

// 5. A write to a dataset record through the REST route, the MCP tool, or an import is `read_only_source`.
#[tokio::test(flavor = "multi_thread")]
async fn every_write_route_refuses_a_dataset_record() {
    let w = world();
    put(&w, "sis/programs.csv", &programs(&rows(3)));
    let c = connect(&w, "sis/programs.csv").await;
    run(&w, &c, false).await;
    let id: String = {
        let app = c.app.clone();
        w.db.run(move |cl| cl.query_one("SELECT id FROM dataset_records WHERE app_slug = $1 ORDER BY id LIMIT 1", &[&app]).unwrap().get(0))
    };
    let records = format!("/api/v1/apps/{}/records", c.app);

    // REST, as a Platform Admin: create, update, delete.
    let (s, b) = send(&w.app, "POST", &records, ADMIN, Some(json!({ "data": { "_table_id": "programs", "code": "X1", "name": "X" } }))).await;
    assert_eq!((s, b["code"].as_str()), (StatusCode::CONFLICT, Some("read_only_source")), "{b}");
    let (s, b) = send(&w.app, "PATCH", &format!("{records}/{id}"), ADMIN, Some(json!({ "data": { "_table_id": "programs", "code": "P0001", "name": "Changed" } }))).await;
    assert_eq!((s, b["code"].as_str()), (StatusCode::CONFLICT, Some("read_only_source")), "{b}");
    let (s, b) = send(&w.app, "DELETE", &format!("{records}/{id}"), ADMIN, None).await;
    assert_eq!((s, b["code"].as_str()), (StatusCode::CONFLICT, Some("read_only_source")), "{b}");

    // MCP.
    let payload = json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": { "name": "create_record", "arguments": { "app_slug": c.app, "data": { "_table_id": "programs", "code": "X2", "name": "X" } } } });
    let req = Request::builder()
        .method("POST")
        .uri("/api/mcp")
        .header("authorization", format!("Bearer {}", issue_test_token_and_user(ADMIN)))
        .header("content-type", "application/json")
        .body(Body::from(payload.to_string()))
        .unwrap();
    let resp = w.app.clone().oneshot(req).await.unwrap();
    let text = String::from_utf8_lossy(&resp.into_body().collect().await.unwrap().to_bytes()).to_lowercase();
    assert!(text.contains("read-only"), "the tool says the dataset is read-only: {text}");

    // An import.
    let csv = b"code,name\nX3,X\n";
    let req = Request::post(format!("/api/v1/apps/{}/imports", c.app))
        .header(header::AUTHORIZATION, format!("Bearer {}", issue_test_token_and_user(ADMIN)))
        .header(header::CONTENT_LENGTH, csv.len().to_string())
        .header("x-file-name", "more.csv")
        .body(Body::from(csv.to_vec()))
        .unwrap();
    let resp = w.app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::CONFLICT);
    let v: Value = serde_json::from_slice(&resp.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(v["code"], "read_only_source", "{v}");

    assert_eq!(live(&w, &c.app), 3, "nothing was written");
    assert_eq!(history_rows(&w, &c.app), 3);

    // Behind the routes, the database refuses a write that is not the sync's, so a writer that skipped the check is
    // still stopped.
    let app = c.app.clone();
    let refused = w.db.run(move |cl| cl.execute("UPDATE dataset_records SET data = jsonb_set(data, '{name}', '\"Changed\"') WHERE app_slug = $1", &[&app]).is_err());
    assert!(refused, "a direct update of a dataset record is refused by the database");
}

// 5b. A scan: the only code that writes a record with the sync's actor kind is the sync, and every source file that
// calls a record writer is one this list knows.
#[test]
fn only_the_sync_writes_dataset_records() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut syncs = Vec::new();
    let mut writers = Vec::new();
    let writer_calls = ["insert_record_in_tx(", "update_record_in_tx(", "delete_record_in_tx(", "restore_record_in_tx(", ".insert_record(", ".save_record_checked(", ".delete_record_checked("];
    fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
        for e in std::fs::read_dir(dir).unwrap().flatten() {
            let p = e.path();
            if p.is_dir() {
                walk(&p, out);
            } else if p.extension().is_some_and(|x| x == "rs") {
                out.push(p);
            }
        }
    }
    let mut files = Vec::new();
    walk(&src, &mut files);
    for f in files {
        let text = std::fs::read_to_string(&f).unwrap();
        let name = f.strip_prefix(&src).unwrap().to_string_lossy().replace('\\', "/");
        // A line that builds a write with the sync's kind. A `match` arm that names the kind for display is not one.
        if text.lines().any(|l| l.contains("ActorKind::Sync") && !l.contains("=>")) && name != "repository.rs" {
            syncs.push(name.clone());
        }
        if writer_calls.iter().any(|c| text.contains(c)) {
            writers.push(name);
        }
    }
    syncs.sort();
    assert_eq!(syncs, vec!["connections.rs".to_string()], "only the connection sync uses the sync's actor kind");
    // Every one of these refuses a dataset before it writes. A new file that calls a writer fails this test, so the
    // person adding it decides how it refuses a dataset.
    let known = [
        "connections.rs",
        "calc_jobs.rs",
        "import_jobs.rs",
        "purge.rs",
        "repository.rs",
        "repository_links.rs",
        "service/flows.rs",
        "service/records.rs",
        "service/undo.rs",
        "snapshot_jobs.rs",
        "state.rs",
        "service/tools.rs",
        // Helpers the guarded services call, and the two that write into apps of their own: synthetic data and trials.
        "repository_timers.rs",
        "repository_trash.rs",
        "repository_undo.rs",
        "repository_votes.rs",
        "service/synthesize.rs",
        "service/trials.rs",
    ];
    writers.sort();
    let unknown: Vec<&String> = writers.iter().filter(|w| !known.contains(&w.as_str())).collect();
    assert!(unknown.is_empty(), "a file calls a record writer that this scan does not know: {unknown:?}");
}

// 6. The history of a changed record shows the sync as the actor, with the kind `sync`.
#[tokio::test(flavor = "multi_thread")]
async fn the_history_of_a_changed_record_names_the_sync() {
    let w = world();
    let mut data = rows(5);
    put(&w, "sis/programs.csv", &programs(&data));
    let c = connect(&w, "sis/programs.csv").await;
    run(&w, &c, false).await;
    data[0].1 = "Renamed".to_string();
    put(&w, "sis/programs.csv", &programs(&data));
    run(&w, &c, false).await;

    let app = c.app.clone();
    let id: String = w.db.run(move |cl| cl.query_one("SELECT id FROM dataset_records WHERE app_slug = $1 AND data->>'code' = 'P0001'", &[&app]).unwrap().get(0));
    let (s, b) = send(&w.app, "GET", &format!("/api/v1/apps/{}/records/{id}/history", c.app), ADMIN, None).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    let entries = b["entries"].as_array().unwrap_or_else(|| panic!("entries: {b}"));
    assert_eq!(entries.len(), 2, "{b}");
    for e in entries {
        assert_eq!(e["actor_kind"], "sync", "{e}");
        assert!(e["actor"].as_str().unwrap().starts_with("sync:"), "{e}");
    }
    assert!(entries.iter().any(|e| e["changes"]["name"]["new"] == "Renamed"), "{b}");
}

// 7. The catalog no longer returns rows. A signed-in person gets metadata, and nobody gets typed sample rows.
#[tokio::test(flavor = "multi_thread")]
async fn the_catalog_returns_metadata_and_never_rows() {
    let w = world();
    let (s, _) = {
        let req = Request::get("/api/v1/datasets").body(Body::empty()).unwrap();
        let resp = w.app.clone().oneshot(req).await.unwrap();
        (resp.status(), ())
    };
    assert_eq!(s, StatusCode::UNAUTHORIZED, "the catalog needs a signed-in person");
    for who in [FACULTY, ADMIN] {
        let (s, b) = send(&w.app, "GET", "/api/v1/datasets", who, None).await;
        assert_eq!(s, StatusCode::OK, "{b}");
        assert!(!b.to_string().contains("sample_data"), "{who} sees no sample rows: {b}");
        let first = b["datasets"][0]["id"].as_str().expect("a dataset").to_string();
        let (s, b) = send(&w.app, "GET", &format!("/api/v1/datasets/{first}"), who, None).await;
        assert_eq!(s, StatusCode::OK, "{b}");
        assert!(!b.to_string().contains("sample_data"), "{b}");
    }
}

// 8. A schedule fires once under two scheduler threads, and a due connection runs once under two callers.
#[tokio::test(flavor = "multi_thread")]
async fn a_schedule_fires_once_under_two_schedulers_and_a_due_connection_runs_once() {
    let w = world();
    put(&w, "sis/programs.csv", &programs(&rows(50)));
    let c = connect(&w, "sis/programs.csv").await;
    let (s, b) = send(&w.app, "PATCH", &format!("/api/v1/admin/connections/{}", c.id), ADMIN, Some(json!({ "every_minutes": 60, "reason": "Hourly" }))).await;
    assert_eq!(s, StatusCode::OK, "{b}");

    let repo = w.state.repository.clone();
    scaffoldry_server::jobs::register_schedule(&repo, "sync_due_connections", 60).unwrap();
    w.db.run(|cl| cl.execute("UPDATE job_schedules SET next_run_at = NOW() - INTERVAL '1 second' WHERE kind = 'sync_due_connections'", &[]).unwrap());
    let (r1, r2) = (repo.clone(), repo.clone());
    let t1 = std::thread::spawn(move || scaffoldry_server::jobs::scheduler_tick(&r1, &[]));
    let t2 = std::thread::spawn(move || scaffoldry_server::jobs::scheduler_tick(&r2, &[]));
    let (a, b) = (t1.join().unwrap().unwrap(), t2.join().unwrap().unwrap());
    let queued: i64 = w.db.run(|cl| cl.query_one("SELECT COUNT(*) FROM jobs WHERE kind = 'sync_due_connections'", &[]).unwrap().get(0));
    assert_eq!(queued, 1, "one job, not two ({a} and {b} spawned)");

    // The job's work, run twice at once, runs the due connection once.
    let (s1, s2) = (w.state.clone(), w.state.clone());
    let h1 = std::thread::spawn(move || scaffoldry_server::connections::sync_due(&s1));
    let h2 = std::thread::spawn(move || scaffoldry_server::connections::sync_due(&s2));
    h1.join().unwrap().unwrap();
    h2.join().unwrap().unwrap();
    let id = uuid::Uuid::parse_str(&c.id).unwrap();
    let runs: i64 = w.db.run(move |cl| cl.query_one("SELECT COUNT(*) FROM sync_runs WHERE connection_id = $1", &[&id]).unwrap().get(0));
    assert_eq!(runs, 1, "one run");
    assert_eq!(live(&w, &c.app), 50);
}

// A faculty caller is 403 on every route, and each change of a connection is a ledger entry with its reason.
#[tokio::test(flavor = "multi_thread")]
async fn routes_are_for_platform_admins_and_every_change_is_on_the_ledger() {
    let w = world();
    put(&w, "sis/programs.csv", &programs(&rows(3)));
    let c = connect(&w, "sis/programs.csv").await;
    for (method, uri, b) in [
        ("GET", "/api/v1/admin/connections".to_string(), None),
        ("POST", "/api/v1/admin/connections".to_string(), Some(body("X", "x-app", "sis/programs.csv"))),
        ("PATCH", format!("/api/v1/admin/connections/{}", c.id), Some(json!({ "enabled": false, "reason": "Stop" }))),
        ("GET", format!("/api/v1/admin/connections/{}/runs", c.id), None),
        ("POST", format!("/api/v1/admin/connections/{}/run", c.id), Some(json!({}))),
    ] {
        let (s, v) = send(&w.app, method, &uri, FACULTY, b).await;
        assert_eq!(s, StatusCode::FORBIDDEN, "{method} {uri}: {v}");
    }
    let ledger = |w: &World| -> i64 { w.db.run(|cl| cl.query_one("SELECT COUNT(*) FROM governance_ledger WHERE decision_type = 'ConnectionChanged'", &[]).unwrap().get(0)) };
    assert_eq!(ledger(&w), 1, "the creation");
    let (s, _) = send(&w.app, "PATCH", &format!("/api/v1/admin/connections/{}", c.id), ADMIN, Some(json!({ "enabled": false }))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "a change needs a reason");
    let (s, b) = send(&w.app, "PATCH", &format!("/api/v1/admin/connections/{}", c.id), ADMIN, Some(json!({ "enabled": false, "reason": "The registrar is migrating" }))).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_eq!(b["connection"]["enabled"], false);
    assert_eq!(ledger(&w), 2);

    // The dataset is an app of kind Dataset in the system workspace.
    let (s, b) = send(&w.app, "GET", &format!("/api/v1/apps/{}", c.app), ADMIN, None).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_eq!(b["kind"], "Dataset", "{b}");
    assert_eq!(b["workspace_id"], "system-datasets", "{b}");
}
