//! Data layer phase 2b: a dataset declares a key.
//!
//! The key is one or more fields and a normalization: trimmed, and case-folded if declared. Two rows that normalize to
//! the same key cannot both be stored, whether by a write, an import or a sync. The refusal says only "this key is
//! already used" and names no row. The steward sets the key by a proposal. Existing duplicates block it and are listed.
//! A sync that sees a key change keeps the record id and writes a history entry. An import upsert on a declared key
//! never reports "matched multiple".

mod common;

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use axum::Router;
use http_body_util::BodyExt;
use scaffoldry_server::build_app_with_state;
use scaffoldry_server::jobs::{get_job, JobConfig, JobRunner, JOB_KINDS};
use scaffoldry_server::service::identity::issue_test_token_and_user;
use scaffoldry_server::state::{CollaboratorRecord, ServerState};
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};
use tower::ServiceExt;

const WORKSPACE: &str = "ws-physics-optics";
const OWNER: &str = "einstein@physics.state.edu";
const EDITOR: &str = "keys.editor@physics.state.edu";
const ADMIN: &str = "jordan.lee@state.edu";
const TAKEN: &str = "this key is already used";

fn files_dir() {
    static DIR: OnceLock<PathBuf> = OnceLock::new();
    DIR.get_or_init(|| {
        let dir = std::env::temp_dir().join(format!("scaffoldry-keys-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        std::env::set_var("SCAFFOLDRY_FILES_DIR", &dir);
        dir
    });
}

struct World {
    db: common::TestDb,
    state: Arc<ServerState>,
    app: Router,
    owner: String,
    editor: String,
    admin: String,
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

async fn tool_as(w: &World, token: &str, name: &str, args: Value) -> Result<Value, String> {
    let payload = json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": { "name": name, "arguments": args } });
    let (_, v) = rest(&w.app, "POST", "/api/mcp", token, Some(payload)).await;
    if let Some(err) = v.get("error") {
        return Err(err["message"].as_str().unwrap_or("").to_string());
    }
    if v["result"]["isError"] == true {
        return Err(v["result"]["content"][0]["text"].as_str().unwrap_or("").to_string());
    }
    Ok(v["result"]["structuredContent"].clone())
}

fn text(name: &str) -> Value {
    json!({ "name": name, "label": name, "field_type": "Text", "required": false, "ferpa_sensitive": false })
}

fn manifest(slug: &str, key: Option<Value>) -> Value {
    let mut table = json!({ "id": "cases", "name": "Cases", "slug": "cases", "primary_field": "title", "fields": [text("code"), text("term"), text("title")] });
    if let Some(key) = key {
        table["key"] = key;
    }
    json!({
        "slug": slug, "title": "Keyed cases", "description": "An app whose table has a key", "organization_code": "PHYS", "department": "physics",
        "workspace_id": WORKSPACE, "views": [], "tables": [table],
    })
}

fn code_key() -> Value {
    json!({ "fields": ["code"], "case_fold": true })
}

async fn world() -> World {
    files_dir();
    let db = common::TestDb::new();
    let state = db.state();
    state.seed_demo().expect("seed");
    let record = CollaboratorRecord {
        id: uuid::Uuid::new_v4().to_string(),
        workspace_id: WORKSPACE.to_string(),
        eppn: EDITOR.to_string(),
        name: EDITOR.to_string(),
        role: "editor".to_string(),
        scoped_affiliation: "faculty".to_string(),
        department: "physics".to_string(),
        added_at: String::new(),
        app_slug: String::new(),
    };
    state.repository.upsert_collaborator(&record).expect("a member");
    state.collaborators.write().unwrap().entry(WORKSPACE.to_string()).or_default().push(record);
    let tag = uuid::Uuid::new_v4().to_string()[..8].to_string();
    let app = build_app_with_state(state.clone()).expect("router");
    World { db, state, app, owner: issue_test_token_and_user(OWNER), editor: issue_test_token_and_user(EDITOR), admin: issue_test_token_and_user(ADMIN), slug: format!("keys-{tag}") }
}

async fn live_app(w: &World, key: Option<Value>) {
    let (s, b) = rest(&w.app, "POST", &format!("/api/v1/workspaces/{WORKSPACE}/apps"), &w.owner, Some(manifest(&w.slug, key))).await;
    assert_eq!(s, StatusCode::CREATED, "{b}");
}

async fn create_as(w: &World, token: &str, code: &str, term: &str) -> (StatusCode, Value) {
    rest(&w.app, "POST", &format!("/api/v1/apps/{}/records", w.slug), token, Some(json!({ "data": { "_table_id": "cases", "code": code, "term": term, "title": code } }))).await
}

async fn create(w: &World, code: &str) -> String {
    let (s, b) = create_as(w, &w.owner, code, "fall").await;
    assert_eq!(s, StatusCode::CREATED, "{b}");
    b["id"].as_str().expect("a record id").to_string()
}

async fn patch_code(w: &World, id: &str, code: &str) -> (StatusCode, Value) {
    rest(&w.app, "PATCH", &format!("/api/v1/apps/{}/records/{id}", w.slug), &w.owner, Some(json!({ "data": { "_table_id": "cases", "code": code, "term": "fall", "title": code }, "version": 1 }))).await
}

fn key_rows(w: &World) -> Vec<(String, String)> {
    let slug = w.slug.clone();
    w.db.run(move |c| c.query("SELECT key_text, record_id FROM record_keys WHERE app_slug = $1 ORDER BY key_text", &[&slug]).unwrap().iter().map(|r| (r.get(0), r.get(1))).collect())
}

fn stored_count(w: &World) -> i64 {
    let slug = w.slug.clone();
    w.db.run(move |c| c.query_one("SELECT count(*) FROM dataset_records WHERE app_slug = $1 AND deleted_at IS NULL", &[&slug]).unwrap().get(0))
}

fn refused(s: StatusCode, b: &Value, other_ids: &[&str]) {
    assert!(s.is_client_error(), "{s} {b}");
    assert_eq!(b["code"], "constraint_violated", "{b}");
    assert_eq!(b["error"], TAKEN, "{b}");
    for id in other_ids {
        assert!(!b.to_string().contains(id), "the refusal names a row: {b}");
    }
}

// 1. Two rows that normalize to the same text cannot both be stored, by a write.
#[tokio::test(flavor = "multi_thread")]
async fn two_rows_with_the_same_normalized_key_cannot_both_be_stored() {
    let w = world().await;
    live_app(&w, Some(code_key())).await;
    let first = create(&w, "EMP-1").await;
    assert_eq!(key_rows(&w), vec![("emp-1".to_string(), first.clone())]);

    // The same text with other case and spaces is the same key. The refusal names no row.
    let (s, b) = create_as(&w, &w.owner, "  emp-1 ", "fall").await;
    refused(s, &b, &[&first]);
    assert_eq!(stored_count(&w), 1, "the second row was stored");

    // Another writer gets the same words.
    let (s2, b2) = create_as(&w, &w.editor, "Emp-1", "fall").await;
    refused(s2, &b2, &[&first]);
    assert_eq!(b2["error"], b["error"]);

    // An update to a taken key is refused, and the row keeps its own.
    let second = create(&w, "EMP-2").await;
    let (s, b) = patch_code(&w, &second, "emp-1").await;
    refused(s, &b, &[&first, &second]);
    let (s, b) = patch_code(&w, &second, "EMP-3").await;
    assert_eq!(s, StatusCode::OK, "{b}");
    // The key it left is free again.
    create(&w, "emp-2").await;
    assert_eq!(key_rows(&w).len(), 3);
}

// 2. Without case folding, case is part of the key and the spaces around it are not. Several fields make one key.
#[tokio::test(flavor = "multi_thread")]
async fn a_key_of_two_fields_without_case_folding_trims_and_keeps_case() {
    let w = world().await;
    live_app(&w, Some(json!({ "fields": ["code", "term"] }))).await;
    let (s, b) = create_as(&w, &w.owner, "A", "fall").await;
    assert_eq!(s, StatusCode::CREATED, "{b}");
    let (s, b) = create_as(&w, &w.owner, "a", "fall").await;
    assert_eq!(s, StatusCode::CREATED, "case is part of this key: {b}");
    let (s, b) = create_as(&w, &w.owner, "A", "spring").await;
    assert_eq!(s, StatusCode::CREATED, "the other field makes it a different key: {b}");
    let (s, b) = create_as(&w, &w.owner, " A ", " fall").await;
    refused(s, &b, &[]);
    assert_eq!(key_rows(&w).len(), 3);
}

// 3. A deleted row frees its key. A restore takes it again, or is refused in the same words.
#[tokio::test(flavor = "multi_thread")]
async fn a_deleted_row_frees_its_key_and_a_restore_takes_it_again() {
    let w = world().await;
    live_app(&w, Some(code_key())).await;
    let first = create(&w, "EMP-1").await;
    let (s, _) = rest(&w.app, "DELETE", &format!("/api/v1/apps/{}/records/{first}", w.slug), &w.owner, None).await;
    assert!(s.is_success(), "{s}");
    assert!(key_rows(&w).is_empty(), "a deleted row keeps its key");
    let again = create(&w, "emp-1").await;

    let (s, b) = rest(&w.app, "POST", &format!("/api/v1/apps/{}/trash/{first}/restore", w.slug), &w.owner, None).await;
    refused(s, &b, &[&again, &first]);
    assert!(key_rows(&w).iter().all(|(_, id)| *id == again), "the restore left a key behind");

    // Free the key, and the restore works.
    let (s, _) = rest(&w.app, "DELETE", &format!("/api/v1/apps/{}/records/{again}", w.slug), &w.owner, None).await;
    assert!(s.is_success());
    let (s, b) = rest(&w.app, "POST", &format!("/api/v1/apps/{}/trash/{first}/restore", w.slug), &w.owner, None).await;
    assert!(s.is_success(), "{s} {b}");
    assert_eq!(key_rows(&w), vec![("emp-1".to_string(), first)]);
}

// 4. The steward sets the key by a proposal. Existing duplicates block it and are listed. Nothing is half done.
#[tokio::test(flavor = "multi_thread")]
async fn a_proposal_that_declares_a_key_over_duplicates_is_refused_with_the_groups() {
    let w = world().await;
    live_app(&w, None).await;
    let a = create(&w, "EMP-1").await;
    let b = create(&w, " emp-1").await;
    let c = create(&w, "EMP-7").await;
    assert!(key_rows(&w).is_empty(), "a table with no key has no key rows");

    let p = tool_as(&w, &w.owner, "propose_app_change", json!({ "workspace_id": WORKSPACE, "manifest": manifest(&w.slug, Some(code_key())), "summary": "Key the cases by code" }))
        .await
        .expect("proposed");
    let id = p["proposal_id"].as_str().unwrap().to_string();
    let err = tool_as(&w, &w.admin, "decide_proposal", json!({ "proposal_id": id, "decision": "approve", "rationale": "Reviewed", "digest": p["digest"] }))
        .await
        .expect_err("two rows share a key");
    assert!(err.contains(&a) && err.contains(&b), "the groups are not listed: {err}");
    assert!(!err.contains(&c), "a row that is not a duplicate is listed: {err}");
    assert!(key_rows(&w).is_empty());
    let status = w.state.repository.get_proposal(id.parse().unwrap()).unwrap().expect("the proposal").status;
    assert_eq!(status, "Pending", "the refused approval left the proposal decided");
    // The live manifest has no key: the write still goes.
    create(&w, "emp-1 ").await;

    // Clear the duplicates. The same proposal is approved, and the rows already stored are keyed.
    for dup in [&a, &b] {
        let (s, _) = rest(&w.app, "DELETE", &format!("/api/v1/apps/{}/records/{dup}", w.slug), &w.owner, None).await;
        assert!(s.is_success());
    }
    let keep = key_rows(&w);
    assert!(keep.is_empty());
    tool_as(&w, &w.admin, "decide_proposal", json!({ "proposal_id": id, "decision": "approve", "rationale": "Reviewed", "digest": p["digest"] })).await.expect("approved");
    let rows = key_rows(&w);
    assert_eq!(rows.iter().map(|(k, _)| k.as_str()).collect::<Vec<_>>(), ["emp-1", "emp-7"], "the stored rows were not keyed");
    let (s, body) = create_as(&w, &w.owner, "EMP-7", "fall").await;
    refused(s, &body, &[&c]);

    // Taking the key off by a proposal frees every key.
    let p = tool_as(&w, &w.owner, "propose_app_change", json!({ "workspace_id": WORKSPACE, "manifest": manifest(&w.slug, None), "summary": "Drop the key" })).await.expect("proposed");
    tool_as(&w, &w.admin, "decide_proposal", json!({ "proposal_id": p["proposal_id"], "decision": "approve", "rationale": "Reviewed", "digest": p["digest"] })).await.expect("approved");
    assert!(key_rows(&w).is_empty());
    create(&w, "EMP-7").await;
}

// 5. A key must name fields of the table.
#[tokio::test(flavor = "multi_thread")]
async fn a_key_that_names_a_missing_field_is_not_a_manifest() {
    let w = world().await;
    live_app(&w, None).await;
    for key in [json!({ "fields": ["nope"] }), json!({ "fields": [] })] {
        let err = tool_as(&w, &w.owner, "propose_app_change", json!({ "workspace_id": WORKSPACE, "manifest": manifest(&w.slug, Some(key)), "summary": "Bad key" })).await.expect_err("refused");
        assert!(err.contains("key"), "{err}");
    }
}

// 6. An import upsert on a declared key matches by the key. It never reports "matched multiple".
fn fast() -> JobConfig {
    JobConfig { workers: 2, poll_ms: 20, backoff_secs: [0, 0, 0], ..JobConfig::default() }
}

fn wait_for_job(w: &World, id: uuid::Uuid) -> scaffoldry_server::jobs::Job {
    let deadline = Instant::now() + Duration::from_secs(120);
    loop {
        let job = get_job(&w.state.repository, id).expect("query").expect("job exists");
        if job.state == "done" || job.state == "failed" {
            return job;
        }
        assert!(Instant::now() < deadline, "job stuck in {}", job.state);
        std::thread::sleep(Duration::from_millis(50));
    }
}

async fn import(w: &World, csv: &str, upsert_key: Option<&str>, accept_errors: bool) -> scaffoldry_server::jobs::Job {
    let req = Request::post(format!("/api/v1/apps/{}/imports", w.slug))
        .header(header::AUTHORIZATION, format!("Bearer {}", w.owner))
        .header(header::CONTENT_LENGTH, csv.len().to_string())
        .header("x-file-name", "cases.csv")
        .body(Body::from(csv.to_string()))
        .unwrap();
    let res = w.app.clone().oneshot(req).await.unwrap();
    let status = res.status();
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    assert_eq!(status, StatusCode::CREATED, "{}", String::from_utf8_lossy(&bytes));
    let import_id = serde_json::from_slice::<Value>(&bytes).unwrap()["import_id"].as_str().unwrap().to_string();
    let mut mapping = json!({ "target_table": "cases", "column_mappings": { "code": "code", "title": "title" } });
    if let Some(k) = upsert_key {
        mapping["upsert_key"] = json!(k);
    }
    let uri = format!("/api/v1/apps/{}/imports/{import_id}/run?accept_errors={accept_errors}", w.slug);
    let (s, body) = rest(&w.app, "POST", &uri, &w.owner, Some(mapping)).await;
    assert!(s.is_success(), "{s} {body}");
    let job_id: uuid::Uuid = body["job_id"].as_str().unwrap().parse().unwrap();
    let runner = JobRunner::start(&w.state.repository, JOB_KINDS, fast()).unwrap();
    let job = wait_for_job(w, job_id);
    runner.stop();
    job
}

#[tokio::test(flavor = "multi_thread")]
async fn an_import_matches_a_declared_key_and_refuses_a_second_row_with_it() {
    let w = world().await;
    live_app(&w, Some(code_key())).await;
    let first = create(&w, "EMP-1").await;

    // The upsert column the file names is the wrong one. The declared key decides.
    let job = import(&w, "code,title\n emp-1 ,Renamed\nEMP-9,New\n", Some("title"), false).await;
    assert_eq!(job.state, "done", "{:?}", job.result);
    let result = job.result.unwrap();
    assert_eq!((result["created_count"].as_i64(), result["updated_count"].as_i64()), (Some(1), Some(1)), "{result}");
    assert!(!result.to_string().contains("matched multiple"), "{result}");
    let slug = w.slug.clone();
    let id = first.clone();
    let title: String = w.db.run(move |c| c.query_one("SELECT data->>'title' FROM dataset_records WHERE app_slug = $1 AND id = $2", &[&slug, &id]).unwrap().get(0));
    assert_eq!(title, "Renamed", "the row with that key was not updated");

    // With no upsert column, a row whose key is taken is rejected in the same words, and the others are written.
    let job = import(&w, "code,title\nEMP-1,Again\nEMP-5,Fresh\nemp-5,Twice\n", None, true).await;
    assert_eq!(job.state, "done", "{:?}", job.result);
    let result = job.result.unwrap();
    assert_eq!((result["created_count"].as_i64(), result["rejected_count"].as_i64()), (Some(1), Some(2)), "{result}");
    assert_eq!(stored_count(&w), 3);

    // Without accept_errors the import stops, and the words are the same.
    let job = import(&w, "code,title\nEMP-1,Again\n", None, false).await;
    assert_eq!(job.state, "failed");
    assert!(job.error.unwrap_or_default().contains(TAKEN));
}

// 7. A sync matches on the declared key, keeps the record id when a key changes, and refuses a key that is used.
struct Sync {
    w: World,
    drop: PathBuf,
    conn: String,
    app: String,
}

fn programs(rows: &[(&str, &str)]) -> String {
    let mut out = String::from("Code,Name,Credits\n");
    for (code, name) in rows {
        out.push_str(&format!("{code},{name},3\n"));
    }
    out
}

async fn sync_world() -> Sync {
    let db = common::TestDb::new();
    let mut state = ServerState::connect(db.url()).expect("server");
    let drop = std::env::temp_dir().join(format!("scaffoldry-keys-drop-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(drop.join("sis")).unwrap();
    state.drop_dir = Some(drop.clone());
    let state = Arc::new(state);
    state.seed_demo().expect("seed");
    let app = build_app_with_state(state.clone()).expect("router");
    let w = World { db, state, app, owner: issue_test_token_and_user(OWNER), editor: String::new(), admin: issue_test_token_and_user(ADMIN), slug: String::new() };
    let tag = uuid::Uuid::new_v4().simple().to_string()[..8].to_string();
    let dataset_app = format!("programs-{tag}");
    std::fs::write(drop.join("sis/programs.csv"), programs(&[("P1", "Physics"), ("P2", "Optics"), ("P3", "Biology")])).unwrap();
    let body = json!({
        "name": format!("Programs {tag}"), "kind": "file_drop", "reason": "The registrar's programs file",
        "config": { "path": "sis/programs.csv", "format": "csv" },
        "dataset_app": dataset_app, "dataset_table": "programs", "key_field": "code",
        "column_map": [
            { "source": "Code", "field": "code", "type": "Text" },
            { "source": "Name", "field": "name", "type": "Text" },
            { "source": "Credits", "field": "credits", "type": "Number" },
        ],
        "sensitivity": "Directory",
    });
    let (s, b) = rest(&w.app, "POST", "/api/v1/admin/connections", &w.admin, Some(body)).await;
    assert_eq!(s, StatusCode::CREATED, "{b}");
    let conn = b["connection"]["id"].as_str().unwrap().to_string();
    // The dataset's table is keyed by name.
    let (_, mut manifest) = w.state.repository.get_app_manifest_row(&dataset_app).unwrap().expect("the dataset app");
    manifest["tables"][0]["key"] = json!({ "fields": ["name"], "case_fold": true });
    let m = w.state.repository.parse_manifest(manifest).expect("a manifest");
    w.state.persist_app_manifest(m).expect("the key is declared");
    Sync { w, drop, conn, app: dataset_app }
}

impl Sync {
    fn put(&self, rows: &[(&str, &str)]) {
        std::fs::write(self.drop.join("sis/programs.csv"), programs(rows)).unwrap();
    }

    async fn run(&self) -> Value {
        let (s, b) = rest(&self.w.app, "POST", &format!("/api/v1/admin/connections/{}/run", self.conn), &self.w.admin, Some(json!({ "approve_shrink": false, "reason": "A test run" }))).await;
        assert_eq!(s, StatusCode::OK, "{b}");
        b["run"].clone()
    }

    fn ids(&self) -> std::collections::BTreeMap<String, String> {
        let app = self.app.clone();
        self.w.db.run(move |c| c.query("SELECT data->>'name', id FROM dataset_records WHERE app_slug = $1 AND deleted_at IS NULL", &[&app]).unwrap().iter().map(|r| (r.get(0), r.get(1))).collect())
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_sync_keeps_the_record_id_when_a_key_changes_and_refuses_a_used_key() {
    let s = sync_world().await;
    let first = s.run().await;
    assert_eq!(first["state"], "done", "{first}");
    let before = s.ids();
    assert_eq!(before.len(), 3);
    let app = s.app.clone();
    let keyed: i64 = s.w.db.run(move |c| c.query_one("SELECT count(*) FROM record_keys WHERE app_slug = $1", &[&app]).unwrap().get(0));
    assert_eq!(keyed, 3, "the sync did not key its rows");

    // The source renames a program: its key changes. The record keeps its id and its history says so.
    s.put(&[("P1", "Applied Physics"), ("P2", "Optics"), ("P3", "Biology")]);
    let run = s.run().await;
    assert_eq!(run["state"], "done", "{run}");
    assert_eq!((run["added"].as_i64(), run["changed"].as_i64(), run["removed"].as_i64()), (Some(0), Some(1), Some(0)), "{run}");
    let after = s.ids();
    assert_eq!(after.get("Applied Physics"), before.get("Physics"), "the record id changed with its key");
    let (app, id) = (s.app.clone(), before["Physics"].clone());
    let (action, kind, changes): (String, String, Value) = s.w.db.run(move |c| {
        let r = c.query_one("SELECT action, actor_kind, changes FROM record_history WHERE app_slug = $1 AND record_id = $2 ORDER BY id DESC LIMIT 1", &[&app, &id]).unwrap();
        (r.get(0), r.get(1), r.get(2))
    });
    assert_eq!((action.as_str(), kind.as_str()), ("update", "sync"));
    assert_eq!(changes["name"]["old"], "Physics", "{changes}");
    assert_eq!(changes["name"]["new"], "Applied Physics", "{changes}");

    // The source renumbers a program and keeps its name. The key finds the record: it is not removed and added.
    s.put(&[("P1", "Applied Physics"), ("P2", " OPTICS "), ("P9", "Biology")]);
    let run = s.run().await;
    assert_eq!(run["state"], "done", "{run}");
    assert_eq!((run["added"].as_i64(), run["removed"].as_i64()), (Some(0), Some(0)), "{run}");
    let renumbered = s.ids();
    assert_eq!(renumbered.get("Biology"), before.get("Biology"), "the record id changed with its code");

    // Two rows of the file with one key cannot both be stored.
    s.put(&[("P1", "Applied Physics"), ("P2", "Optics"), ("P4", "optics")]);
    let run = s.run().await;
    assert_eq!(run["state"], "failed", "{run}");
    let error = run["error"].as_str().unwrap_or_default();
    assert!(error.contains(TAKEN), "{error}");
    for id in before.values() {
        assert!(!error.contains(id.as_str()), "the fault names a row: {error}");
    }
}
