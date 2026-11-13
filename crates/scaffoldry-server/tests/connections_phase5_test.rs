//! Connections phase 5: linking apps to datasets.
//!
//! A link field may name a dataset app. The link stores the record's id, which the sync keeps by its key, so a record
//! whose other fields change keeps its links. A lookup shows the dataset's current values. A record the source removes
//! unlinks, and the link returns with the record. The picker and the lookup follow the dataset's access rules.

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::Router;
use common::TestDb;
use http_body_util::BodyExt;
use scaffoldry_server::build_app_with_state;
use scaffoldry_server::jobs::{get_job, JobConfig, JobRunner, JOB_KINDS};
use scaffoldry_server::service::identity::issue_test_token_and_user;
use scaffoldry_server::state::ServerState;
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::Arc;
use tower::ServiceExt;

const ADMIN: &str = "jordan.lee@state.edu";
const OWNER: &str = "einstein@physics.state.edu";
const CO_OWNER: &str = "bohr@physics.state.edu";
const STRANGER: &str = "visitor@chemistry.state.edu";
const WORKSPACE: &str = "ws-physics-optics";

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

fn unique(prefix: &str) -> String {
    format!("{prefix}-{}", &uuid::Uuid::new_v4().simple().to_string()[..8])
}

/// A programs file: `(code, name)`.
fn programs(rows: &[(&str, &str)]) -> String {
    let mut out = String::from("Code,Name\n");
    for (code, name) in rows {
        out.push_str(&format!("{code},{name}\n"));
    }
    out
}

fn put(w: &World, text: &str) {
    std::fs::write(w.drop.join("sis/programs.csv"), text).unwrap();
}

struct Dataset {
    id: String,
    app: String,
}

/// A dataset of programs, synced from a file.
async fn dataset(w: &World, rows: &[(&str, &str)]) -> Dataset {
    put(w, &programs(rows));
    let app = unique("programs").to_lowercase();
    let body = json!({
        "name": unique("Programs"), "kind": "file_drop", "reason": "The registrar's programs", "config": { "path": "sis/programs.csv", "format": "csv" },
        "dataset_app": app, "dataset_table": "programs", "key_field": "code",
        "column_map": [{ "source": "Code", "field": "code", "type": "Text" }, { "source": "Name", "field": "name", "type": "Text" }], "sensitivity": "Directory",
    });
    let (s, b) = send(&w.app, "POST", "/api/v1/admin/connections", ADMIN, Some(body)).await;
    assert_eq!(s, StatusCode::CREATED, "{b}");
    let id = b["connection"]["id"].as_str().unwrap().to_string();
    let d = Dataset { id, app };
    sync(w, &d).await;
    d
}

async fn sync(w: &World, d: &Dataset) -> Value {
    let (s, b) = send(&w.app, "POST", &format!("/api/v1/admin/connections/{}/run", d.id), ADMIN, Some(json!({ "approve_shrink": true, "reason": "Test" }))).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    b["run"].clone()
}

fn record_id(w: &World, app: &str, code: &str) -> String {
    let (app, code) = (app.to_string(), code.to_string());
    w.db.run(move |c| c.query_one("SELECT id FROM dataset_records WHERE app_slug = $1 AND data->>'code' = $2", &[&app, &code]).unwrap().get(0))
}

/// The people who may read the datasets: Platform Admins, and anyone named here.
async fn let_read(w: &World, who: &str) {
    let (s, b) = send(&w.app, "POST", "/api/v1/workspaces/system-datasets/collaborators", ADMIN, Some(json!({ "eppn": who, "role": "viewer" }))).await;
    assert!(s.is_success(), "{b}");
}

fn field(name: &str, kind: &str) -> Value {
    json!({ "name": name, "label": name, "field_type": kind, "required": false, "ferpa_sensitive": false })
}

/// An app of applications that link to a program in the dataset, with a lookup of the program's name.
fn manifest(slug: &str, d: &Dataset) -> Value {
    let mut program = field("program", "Link");
    program["link_target_app"] = json!(d.app);
    program["link_target_table"] = json!("programs");
    program["link_cardinality"] = json!("One");
    let mut shown = field("program_name", "Lookup");
    shown["computed_via"] = json!("program");
    shown["target_display_field"] = json!("name");
    json!({
        "slug": slug, "title": "Applications", "description": "", "organization_code": "PHYS", "department": "physics", "workspace_id": WORKSPACE, "views": [],
        "tables": [{ "id": "applications", "name": "Applications", "slug": "applications", "primary_field": "name", "fields": [field("name", "Text"), program, shown, field("program_code", "Text")] }],
    })
}

/// Creates the app through the route. A pending proposal is approved by a second owner.
async fn make_app(w: &World, body: Value) -> (StatusCode, Value) {
    let (s, b) = send(&w.app, "POST", &format!("/api/v1/workspaces/{WORKSPACE}/apps"), OWNER, Some(body)).await;
    if s != StatusCode::ACCEPTED {
        return (s, b);
    }
    let id = b["proposal_id"].as_str().unwrap().to_string();
    let (s2, b2) = send(&w.app, "POST", &format!("/api/v1/workspaces/{WORKSPACE}/collaborators"), ADMIN, Some(json!({ "eppn": CO_OWNER, "role": "owner" }))).await;
    assert!(s2.is_success(), "{b2}");
    let (_, p) = send(&w.app, "GET", &format!("/api/v1/proposals/{id}"), CO_OWNER, None).await;
    let (s4, b4) = send(&w.app, "POST", &format!("/api/v1/proposals/{id}/decision"), CO_OWNER, Some(json!({ "decision": "approve", "rationale": "Reviewed", "digest": p["digest"] }))).await;
    assert!(s4.is_success(), "{b4}");
    (StatusCode::CREATED, b4)
}

async fn applications(w: &World, d: &Dataset) -> String {
    let slug = unique("apps").to_lowercase();
    let (s, b) = make_app(w, manifest(&slug, d)).await;
    assert_eq!(s, StatusCode::CREATED, "{b}");
    slug
}

async fn create(w: &World, slug: &str, who: &str, data: Value) -> (StatusCode, Value) {
    let mut d = data;
    d["_table_id"] = json!("applications");
    send(&w.app, "POST", &format!("/api/v1/apps/{slug}/records"), who, Some(json!({ "data": d }))).await
}

/// A record as a page shows it, which is where a lookup is filled in. Read through the tool, as a page is.
async fn read(w: &World, slug: &str, id: &str, who: &str) -> Value {
    let payload = json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": { "name": "list_records", "arguments": { "app_slug": slug, "table_id": "applications", "limit": 200 } } });
    let (_, v) = send(&w.app, "POST", "/api/mcp", who, Some(payload)).await;
    let rows = v["result"]["structuredContent"]["records"].as_array().cloned().unwrap_or_default();
    rows.into_iter().find(|r| r["id"] == id).unwrap_or_else(|| panic!("the record {id} is on the page: {v}"))
}

// 1. An application links to a program in a dataset. The program's name changes in the source. The link holds, and the
// lookup shows the new name.
#[tokio::test(flavor = "multi_thread")]
async fn a_link_holds_when_the_programs_name_changes_in_the_source() {
    let w = world();
    let d = dataset(&w, &[("P1", "Physics"), ("P2", "Chemistry")]).await;
    let_read(&w, OWNER).await;
    let slug = applications(&w, &d).await;
    let program = record_id(&w, &d.app, "P1");
    let (s, b) = create(&w, &slug, OWNER, json!({ "name": "Ada", "program": program })).await;
    assert_eq!(s, StatusCode::CREATED, "{b}");
    let id = b["id"].as_str().unwrap().to_string();
    let first = read(&w, &slug, &id, OWNER).await;
    assert_eq!(first["data"]["program"], program.as_str(), "{first}");
    assert_eq!(first["data"]["program_name"], "Physics", "the lookup reads the dataset: {first}");

    put(&w, &programs(&[("P1", "Physics and Astronomy"), ("P2", "Chemistry")]));
    let run = sync(&w, &d).await;
    assert_eq!(run["changed"], 1, "{run}");
    assert_eq!(record_id(&w, &d.app, "P1"), program, "the record keeps its id");
    let after = read(&w, &slug, &id, OWNER).await;
    assert_eq!(after["data"]["program"], program.as_str(), "the link holds: {after}");
    assert_eq!(after["data"]["program_name"], "Physics and Astronomy", "{after}");
}

// 2. The source removes the program. The link is removed, then the program returns and the link returns.
#[tokio::test(flavor = "multi_thread")]
async fn a_removed_program_unlinks_and_the_link_returns_with_it() {
    let w = world();
    let d = dataset(&w, &[("P1", "Physics"), ("P2", "Chemistry"), ("P3", "Biology")]).await;
    let_read(&w, OWNER).await;
    let slug = applications(&w, &d).await;
    let program = record_id(&w, &d.app, "P1");
    let (_, b) = create(&w, &slug, OWNER, json!({ "name": "Ada", "program": program })).await;
    let id = b["id"].as_str().unwrap().to_string();

    put(&w, &programs(&[("P2", "Chemistry"), ("P3", "Biology")]));
    let run = sync(&w, &d).await;
    assert_eq!(run["removed"], 1, "{run}");
    let gone = read(&w, &slug, &id, OWNER).await;
    let held = gone["data"].get("program").cloned().unwrap_or(Value::Null);
    assert!(held.is_null() || held == json!([]) || held == json!(""), "the link is removed: {gone}");
    assert!(gone["data"].get("program_name").is_none_or(|v| v.is_null()), "{gone}");
    let links: i64 = {
        let p = program.clone();
        w.db.run(move |c| c.query_one("SELECT COUNT(*) FROM record_links WHERE to_record = $1", &[&p]).unwrap().get(0))
    };
    assert_eq!(links, 0);

    put(&w, &programs(&[("P1", "Physics"), ("P2", "Chemistry"), ("P3", "Biology")]));
    let back = sync(&w, &d).await;
    assert_eq!(back["added"], 1, "{back}");
    let again = read(&w, &slug, &id, OWNER).await;
    assert_eq!(again["data"]["program"], program.as_str(), "the link returns with the program: {again}");
    assert_eq!(again["data"]["program_name"], "Physics", "{again}");
}

// 3. A restricted dataset's picker returns only what the person may see, and a link to a record of a dataset a person
// cannot see is refused.
#[tokio::test(flavor = "multi_thread")]
async fn a_restricted_datasets_picker_and_links_follow_who_may_read_it() {
    let w = world();
    let d = dataset(&w, &[("P1", "Physics"), ("P2", "Chemistry")]).await;
    let picker = format!("/api/v1/apps/{}/tables/programs/link-options", d.app);

    // Nobody outside the system workspace reads it.
    let (s, b) = send(&w.app, "GET", &picker, STRANGER, None).await;
    assert!(s == StatusCode::FORBIDDEN || s == StatusCode::NOT_FOUND, "{s} {b}");
    let (s, b) = send(&w.app, "GET", &picker, OWNER, None).await;
    assert!(s == StatusCode::FORBIDDEN || s == StatusCode::NOT_FOUND, "{s} {b}");

    // Without access, a link to one of its records is refused.
    let slug = {
        let s = unique("apps").to_lowercase();
        // The proposal for the app is made by someone who may read the dataset.
        let_read(&w, OWNER).await;
        let (st, b) = make_app(&w, manifest(&s, &d)).await;
        assert_eq!(st, StatusCode::CREATED, "{b}");
        s
    };
    let program = record_id(&w, &d.app, "P1");
    let (s, b) = create(&w, &slug, "bohr@physics.state.edu", json!({ "name": "Niels", "program": program })).await;
    assert!(!s.is_success(), "{s} {b}");

    // Once she may read it, the picker finds the programs, with a label and a second field.
    let (s, b) = send(&w.app, "GET", &format!("{picker}?q=chem"), OWNER, None).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    let options = b["options"].as_array().unwrap_or_else(|| panic!("options: {b}"));
    assert_eq!(options.len(), 1, "{b}");
    assert_eq!(options[0]["label"], "P2", "{b}");
    assert_eq!(options[0]["secondary"], "Chemistry", "{b}");
}

// A link to an app that is not a dataset, or to a table the dataset does not have, is refused when the app is proposed.
#[tokio::test(flavor = "multi_thread")]
async fn a_link_must_name_a_dataset_and_one_of_its_tables() {
    let w = world();
    let d = dataset(&w, &[("P1", "Physics")]).await;
    let_read(&w, OWNER).await;
    let mut m = manifest(&unique("apps").to_lowercase(), &d);
    m["tables"][0]["fields"][1]["link_target_table"] = json!("nope");
    let (s, b) = send(&w.app, "POST", &format!("/api/v1/workspaces/{WORKSPACE}/apps"), OWNER, Some(m)).await;
    let text = b.to_string();
    assert!(s == StatusCode::BAD_REQUEST || text.contains("\"outcome\":\"fail\""), "{s} {b}");
    assert!(text.contains("nope"), "the problem names the table: {b}");

    let mut m = manifest(&unique("apps").to_lowercase(), &d);
    m["tables"][0]["fields"][1]["link_target_app"] = json!("no-such-app");
    let (s, b) = send(&w.app, "POST", &format!("/api/v1/workspaces/{WORKSPACE}/apps"), OWNER, Some(m)).await;
    assert!(s == StatusCode::BAD_REQUEST || b.to_string().contains("\"outcome\":\"fail\""), "{s} {b}");
    assert!(b.to_string().contains("no-such-app"), "{b}");

    // A mirror field would write the dataset, so a link to one is one-way.
    let mut m = manifest(&unique("apps").to_lowercase(), &d);
    m["tables"][0]["fields"][1]["link_related_field"] = json!("applications");
    let (s, b) = send(&w.app, "POST", &format!("/api/v1/workspaces/{WORKSPACE}/apps"), OWNER, Some(m)).await;
    assert!(s == StatusCode::BAD_REQUEST || b.to_string().contains("\"outcome\":\"fail\""), "{s} {b}");
}

// 4. The conversion preview reports matched, unmatched and ambiguous, for a value join to a dataset.
#[tokio::test(flavor = "multi_thread")]
async fn the_conversion_preview_reports_matched_unmatched_and_ambiguous() {
    let w = world();
    // Two programs share a name, so a join on the name is ambiguous for them.
    let d = dataset(&w, &[("P1", "Physics"), ("P2", "Chemistry"), ("P3", "Chemistry")]).await;
    let_read(&w, OWNER).await;
    let slug = unique("apps").to_lowercase();
    let mut m = manifest(&slug, &d);
    // The older shape: a text field that names a program by its name, joined by value.
    m["tables"][0]["fields"] = json!([field("name", "Text"), field("program_code", "Text")]);
    m["relationships"] = json!([{
        "id": "program-join", "name": "Program", "source_table_id": "applications", "target_table_id": "programs", "source_field": "program_code", "target_field": "name",
        "relationship_type": "OneToMany", "display_field": "name", "target_app": d.app,
    }]);
    let (s, b) = make_app(&w, m).await;
    assert_eq!(s, StatusCode::CREATED, "{b}");
    for (name, value) in [("a", "Physics"), ("b", "Physics"), ("c", "Chemistry"), ("d", "Astronomy"), ("e", "")] {
        let (s, b) = create(&w, &slug, OWNER, json!({ "name": name, "program_code": value })).await;
        assert_eq!(s, StatusCode::CREATED, "{b}");
    }

    let runner = JobRunner::start(&w.state.repository, JOB_KINDS, JobConfig { workers: 2, poll_ms: 20, backoff_secs: [0, 0, 0], ..JobConfig::default() }).unwrap();
    let (s, b) = send(&w.app, "POST", &format!("/api/v1/apps/{slug}/relations/program-join/convert-preview"), OWNER, None).await;
    assert_eq!(s, StatusCode::ACCEPTED, "{b}");
    let job: uuid::Uuid = b["job_id"].as_str().unwrap().parse().unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(120);
    let result = loop {
        let j = get_job(&w.state.repository, job).unwrap().unwrap();
        if j.state == "done" {
            break j.result.unwrap();
        }
        assert!(j.state != "failed" && std::time::Instant::now() < deadline, "the preview job: {j:?}");
        std::thread::sleep(std::time::Duration::from_millis(50));
    };
    assert_eq!((result["matched"].as_i64(), result["unmatched"].as_i64(), result["ambiguous"].as_i64(), result["blank"].as_i64()), (Some(2), Some(1), Some(1), Some(1)), "{result}");

    // The conversion is proposed with a link to the dataset, flagged for the values that match none or several.
    let (s, p) = send(&w.app, "POST", &format!("/api/v1/apps/{slug}/relations/program-join/convert"), OWNER, None).await;
    assert!(s.is_success(), "{p}");
    let proposal: uuid::Uuid = p["proposal_id"].as_str().unwrap().parse().unwrap();
    let stored: Value = w.db.run(move |c| c.query_one("SELECT manifest FROM app_proposals WHERE id = $1", &[&proposal]).unwrap().get(0));
    let converted = stored["tables"][0]["fields"].as_array().and_then(|f| f.iter().find(|f| f["name"] == "program_code")).cloned().expect("the field");
    assert_eq!(converted["field_type"], "Link", "{converted}");
    assert_eq!(converted["link_target_app"], d.app.as_str(), "{converted}");
    assert_eq!(converted["link_target_table"], "programs", "{converted}");
    assert!(stored["relationships"].as_array().is_none_or(Vec::is_empty), "the join is removed: {stored}");
    let loss = p["checks"].as_array().and_then(|c| c.iter().find(|c| c["name"] == "data_loss")).cloned().expect("a data_loss check");
    assert_eq!(loss["outcome"], "fail", "values that match none or several are flagged: {loss}");
    runner.stop();
}
