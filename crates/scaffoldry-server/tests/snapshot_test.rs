//! Import and export, phase 5: snapshots.
//!
//! A snapshot is an app's definition and every record, as compressed JSON Lines in the file store. It holds everything the
//! app's data holds, so it is as sensitive as the app: the app's owner or an admin reads it, no agent tool returns it, and a
//! download of a sensitive one is a disclosure. Restore makes a NEW app. It never overwrites a live one. A change that
//! loses data needs a recent snapshot first. NIST SP 800-53 CP-9, CP-10 and AU-2.
//!
//! The brief's fifth test, "a held app's snapshots are not expired", waits on retention policies and legal holds
//! (lifecycle phases 3 and 4). Neither exists yet, so snapshots carry `expires_at` and nothing purges them.

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::Router;
use http_body_util::BodyExt;
use scaffoldry_server::build_app_with_state;
use scaffoldry_server::jobs::{get_job, JobConfig, JobRunner, JOB_KINDS};
use scaffoldry_server::service::identity::issue_test_token_and_user;
use scaffoldry_server::state::{CollaboratorRecord, ServerState};
use serde_json::{json, Value};
use std::sync::OnceLock;
use std::time::{Duration, Instant};
use tower::ServiceExt;

const WORKSPACE: &str = "ws-physics-optics";
const OWNER: &str = "einstein@physics.state.edu";
const EDITOR: &str = "snapshot.editor@physics.state.edu";
const ADMIN: &str = "jordan.lee@state.edu";

/// One files directory for the whole binary: the setting is a process variable.
fn files_dir() {
    static DIR: OnceLock<std::path::PathBuf> = OnceLock::new();
    DIR.get_or_init(|| {
        let dir = std::env::temp_dir().join(format!("scaffoldry-snapshots-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        std::env::set_var("SCAFFOLDRY_FILES_DIR", &dir);
        dir
    });
}

struct World {
    db: common::TestDb,
    state: std::sync::Arc<ServerState>,
    app: Router,
    owner: String,
    editor: String,
    admin: String,
    slug: String,
    tag: String,
}

async fn rest_raw(app: &Router, method: &str, uri: &str, token: &str, body: Option<Value>) -> (StatusCode, Vec<u8>) {
    let b = Request::builder().method(method).uri(uri).header("authorization", format!("Bearer {token}"));
    let req = match body {
        Some(v) => b.header("content-type", "application/json").body(Body::from(v.to_string())).unwrap(),
        None => b.body(Body::empty()).unwrap(),
    };
    let resp = app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    (status, resp.into_body().collect().await.unwrap().to_bytes().to_vec())
}

async fn rest(app: &Router, method: &str, uri: &str, token: &str, body: Option<Value>) -> (StatusCode, Value) {
    let (s, bytes) = rest_raw(app, method, uri, token, body).await;
    (s, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
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

fn field(name: &str, sensitive: bool) -> Value {
    json!({ "name": name, "label": name, "field_type": "Text", "required": false, "ferpa_sensitive": sensitive })
}

fn manifest(slug: &str, fields: &[(&str, bool)]) -> Value {
    json!({
        "slug": slug, "title": "Snapshot source", "description": "An app to protect", "organization_code": "PHYS", "department": "physics",
        "workspace_id": WORKSPACE, "views": [],
        "tables": [{ "id": "cases", "name": "Cases", "slug": "cases", "fields": fields.iter().map(|(n, s)| field(n, *s)).collect::<Vec<_>>() }],
    })
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
    World {
        db,
        state,
        app,
        owner: issue_test_token_and_user(OWNER),
        editor: issue_test_token_and_user(EDITOR),
        admin: issue_test_token_and_user(ADMIN),
        slug: format!("snap-{tag}"),
        tag,
    }
}

/// Makes the source app live and puts `n` records in it.
async fn live_app(w: &World, fields: &[(&str, bool)], n: i64) {
    let (s, b) = rest(&w.app, "POST", &format!("/api/v1/workspaces/{WORKSPACE}/apps"), &w.owner, Some(manifest(&w.slug, fields))).await;
    // A sensitive field is a flagged change, so it may wait for a second person.
    if s == StatusCode::ACCEPTED {
        let id = b["proposal_id"].as_str().expect("a proposal").to_string();
        let (_, full) = rest(&w.app, "GET", &format!("/api/v1/proposals/{id}"), &w.owner, None).await;
        tool_as(w, &w.admin, "decide_proposal", json!({ "proposal_id": id, "decision": "approve", "rationale": "Set up", "digest": full["digest"] })).await.expect("live");
    } else {
        assert_eq!(s, StatusCode::CREATED, "{b}");
    }
    let slug = w.slug.clone();
    w.db.run(move |c| {
        c.execute(
            "INSERT INTO dataset_records (id, app_slug, data, ceds_mapping, is_ferpa_sensitive, table_id, created_at) \
             SELECT 'rec-' || $1 || '-' || lpad(g::text, 8, '0'), $1, \
                    jsonb_build_object('_table_id', 'cases', 'title', 'Case ' || g, 'note', 'n' || g, 'ssn', '000-00-' || lpad((g % 10000)::text, 4, '0')), \
                    '{}'::jsonb, false, 'cases', TIMESTAMPTZ '2026-01-01 00:00:00+00' + g * interval '1 millisecond' \
             FROM generate_series(1, $2::bigint) AS g",
            &[&slug, &n],
        )
        .unwrap();
    });
}

fn fast() -> JobConfig {
    JobConfig { workers: 2, poll_ms: 20, backoff_secs: [0, 0, 0], ..JobConfig::default() }
}

fn wait_job(w: &World, id: &str) -> scaffoldry_server::jobs::Job {
    let id: uuid::Uuid = id.parse().expect("a job id");
    let deadline = Instant::now() + Duration::from_secs(240);
    loop {
        let job = get_job(&w.state.repository, id).unwrap().expect("the job");
        if job.state == "done" || job.state == "failed" {
            return job;
        }
        assert!(Instant::now() < deadline, "the job is stuck in {}", job.state);
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// A fingerprint of an app's records: how many, and a hash of every record's content in a fixed order.
fn fingerprint(w: &World, slug: &str) -> (i64, String) {
    let slug = slug.to_string();
    w.db.run(move |c| {
        let r = c
            .query_one(
                "SELECT count(*), COALESCE(md5(string_agg(data::text, ',' ORDER BY data->>'title')), '') FROM dataset_records WHERE app_slug = $1 AND deleted_at IS NULL",
                &[&slug],
            )
            .unwrap();
        (r.get(0), r.get(1))
    })
}

async fn take(w: &World, who: &str, name: &str) -> (StatusCode, Value) {
    rest(&w.app, "POST", &format!("/api/v1/apps/{}/snapshots", w.slug), who, Some(json!({ "name": name }))).await
}

// 1. A manual snapshot of a 20,000-record app restores into a new app with the same records. The original is untouched.
#[tokio::test]
async fn a_snapshot_of_twenty_thousand_records_restores_into_a_new_app() {
    let w = world().await;
    live_app(&w, &[("title", false), ("note", false), ("ssn", false)], 20_000).await;
    let before = fingerprint(&w, &w.slug);
    assert_eq!(before.0, 20_000);
    let version_before: Option<i32> = w.state.repository.app_manifest_version(&w.slug).unwrap();

    let runner = JobRunner::start(&w.state.repository, JOB_KINDS, fast()).unwrap();
    let (s, made) = take(&w, &w.owner, "Before the term").await;
    assert_eq!(s, StatusCode::ACCEPTED, "{made}");
    assert_eq!(made["kind"], "manual", "{made}");
    let done = wait_job(&w, made["job_id"].as_str().expect("a job id"));
    assert_eq!(done.state, "done", "{done:?}");

    let (_, list) = rest(&w.app, "GET", &format!("/api/v1/apps/{}/snapshots", w.slug), &w.owner, None).await;
    let row = list["snapshots"].as_array().and_then(|l| l.first()).cloned().expect("the snapshot is listed");
    assert_eq!(row["state"], "ready", "{row}");
    assert_eq!(row["record_count"], 20_000, "{row}");
    assert!(row["size"].as_i64().unwrap_or(0) > 0, "{row}");
    let id = row["id"].as_str().unwrap().to_string();

    let new_slug = format!("restored-{}", w.tag);
    let (s, r) = rest(&w.app, "POST", &format!("/api/v1/snapshots/{id}/restore"), &w.owner, Some(json!({ "slug": new_slug }))).await;
    assert_eq!(s, StatusCode::ACCEPTED, "{r}");
    assert_eq!(r["approved"], true, "an unchanged definition restores in one step: {r}");
    let restore = wait_job(&w, r["job_id"].as_str().expect("the restore job"));
    assert_eq!(restore.state, "done", "{restore:?}");

    let restored = fingerprint(&w, &new_slug);
    assert_eq!(restored.0, 20_000, "the new app has every record");
    assert_eq!(restored.1, before.1, "and the same content");
    assert_eq!(fingerprint(&w, &w.slug), before, "the original is untouched");
    assert_eq!(w.state.repository.app_manifest_version(&w.slug).unwrap(), version_before, "and so is its definition");
    runner.stop();
}

// 2. A change that loses data takes a snapshot first, and only then applies. A snapshot that fails leaves it unapproved.
async fn lose_a_field(w: &World, field: &str, remaining: &[(&str, bool)]) -> (String, String) {
    let p = tool_as(w, &w.owner, "propose_app_change", json!({ "workspace_id": WORKSPACE, "manifest": manifest(&w.slug, remaining), "summary": format!("Drop {field}") }))
        .await
        .expect("proposed");
    let loss = p["checks"].as_array().and_then(|l| l.iter().find(|c| c["name"] == "data_loss")).cloned().unwrap_or(Value::Null);
    assert_eq!(loss["outcome"], "fail", "dropping a field with records is data loss: {p}");
    (p["proposal_id"].as_str().expect("a proposal").to_string(), p["digest"].as_str().unwrap_or("").to_string())
}

async fn approve(w: &World, id: &str, digest: &str) -> Result<Value, String> {
    tool_as(w, &w.admin, "decide_proposal", json!({ "proposal_id": id, "decision": "approve", "rationale": "Reviewed", "digest": digest })).await
}

fn status_of(w: &World, id: &str) -> String {
    let id: uuid::Uuid = id.parse().unwrap();
    w.state.repository.get_proposal(id).unwrap().expect("the proposal").status
}

fn wait_status(w: &World, id: &str, want: &str) {
    let deadline = Instant::now() + Duration::from_secs(60);
    while status_of(w, id) != want {
        assert!(Instant::now() < deadline, "the proposal stayed {}", status_of(w, id));
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn has_field(w: &World, field: &str) -> bool {
    let m = w.state.repository.get_app_manifest(&w.slug).unwrap().expect("the app");
    m.tables.iter().any(|t| t.fields.iter().any(|f| f.name == field))
}

fn snapshots_of(w: &World, kind: &str) -> Vec<(String, i64)> {
    let (slug, kind) = (w.slug.clone(), kind.to_string());
    w.db.run(move |c| c.query("SELECT state, COALESCE(record_count, 0) FROM app_snapshots WHERE app_slug = $1 AND kind = $2 ORDER BY created_at", &[&slug, &kind]).unwrap().iter().map(|r| (r.get(0), r.get(1))).collect())
}

#[tokio::test]
async fn a_change_that_loses_data_takes_a_snapshot_and_only_then_applies() {
    let w = world().await;
    live_app(&w, &[("title", false), ("note", false), ("ssn", false)], 50).await;
    let runner = JobRunner::start(&w.state.repository, JOB_KINDS, fast()).unwrap();

    let (id, digest) = lose_a_field(&w, "note", &[("title", false), ("ssn", false)]).await;
    let held = approve(&w, &id, &digest).await.expect("the approval is taken");
    assert_eq!(held["status"], "Approving", "the approval waits for the snapshot: {held}");
    assert!(has_field(&w, "note") || status_of(&w, &id) != "Approved", "nothing is applied before the snapshot");

    wait_status(&w, &id, "Approved");
    assert!(!has_field(&w, "note"), "the change is applied once the snapshot is done");
    assert_eq!(snapshots_of(&w, "pre_change"), vec![("ready".to_string(), 50)], "and the snapshot holds the records as they were");

    // A snapshot from the last seven days counts, so the next such change applies at once.
    let (id2, digest2) = lose_a_field(&w, "ssn", &[("title", false)]).await;
    let now = approve(&w, &id2, &digest2).await.expect("the approval is taken");
    assert_eq!(now["status"], "Approved", "a recent snapshot is enough: {now}");
    assert_eq!(snapshots_of(&w, "pre_change").len(), 1, "no second snapshot was taken");
    runner.stop();
}

#[tokio::test]
async fn a_snapshot_older_than_seven_days_does_not_count() {
    let w = world().await;
    live_app(&w, &[("title", false), ("note", false), ("ssn", false)], 20).await;
    let runner = JobRunner::start(&w.state.repository, JOB_KINDS, fast()).unwrap();
    let (s, made) = take(&w, &w.owner, "Old").await;
    assert_eq!(s, StatusCode::ACCEPTED, "{made}");
    assert_eq!(wait_job(&w, made["job_id"].as_str().unwrap()).state, "done");
    let slug = w.slug.clone();
    w.db.run(move |c| c.execute("UPDATE app_snapshots SET created_at = NOW() - interval '8 days' WHERE app_slug = $1", &[&slug]).unwrap());

    let (id, digest) = lose_a_field(&w, "note", &[("title", false), ("ssn", false)]).await;
    let held = approve(&w, &id, &digest).await.expect("the approval is taken");
    assert_eq!(held["status"], "Approving", "an eight-day-old snapshot is not recent enough: {held}");
    wait_status(&w, &id, "Approved");
    assert_eq!(snapshots_of(&w, "pre_change").len(), 1, "a new one was taken");
    runner.stop();
}

#[tokio::test]
async fn a_snapshot_that_fails_leaves_the_proposal_unapproved() {
    let w = world().await;
    live_app(&w, &[("title", false), ("note", false), ("ssn", false)], 50).await;
    // A snapshot is built in memory, so its size is bounded. Below the app's size, it fails.
    w.db.run(|c| c.execute("INSERT INTO platform_settings (key, value, updated_by) VALUES ('snapshots.max_records', '5'::jsonb, 'test') ON CONFLICT (key) DO UPDATE SET value = '5'::jsonb", &[]).unwrap());
    let runner = JobRunner::start(&w.state.repository, JOB_KINDS, fast()).unwrap();

    let (id, digest) = lose_a_field(&w, "note", &[("title", false), ("ssn", false)]).await;
    let held = approve(&w, &id, &digest).await.expect("the approval is taken");
    assert_eq!(held["status"], "Approving", "{held}");
    wait_status(&w, &id, "Pending");
    assert!(has_field(&w, "note"), "nothing was applied");
    let kinds = snapshots_of(&w, "pre_change");
    assert_eq!(kinds.len(), 1, "{kinds:?}");
    assert_eq!(kinds[0].0, "failed", "the snapshot says it failed");
    runner.stop();
}

// 3. A snapshot is not downloadable by an editor, and no MCP tool returns its content.
#[tokio::test]
async fn a_snapshot_is_not_downloadable_by_an_editor_and_no_tool_returns_it() {
    let w = world().await;
    live_app(&w, &[("title", false), ("note", false)], 30).await;
    let runner = JobRunner::start(&w.state.repository, JOB_KINDS, fast()).unwrap();
    let (s, made) = take(&w, &w.owner, "Mine").await;
    assert_eq!(s, StatusCode::ACCEPTED, "{made}");
    assert_eq!(wait_job(&w, made["job_id"].as_str().unwrap()).state, "done");
    let id = made["snapshot_id"].as_str().expect("a snapshot id").to_string();

    let (s, _) = take(&w, &w.editor, "Theirs").await;
    assert_eq!(s, StatusCode::FORBIDDEN, "an editor cannot take one");
    let (s, _) = rest(&w.app, "GET", &format!("/api/v1/apps/{}/snapshots", w.slug), &w.editor, None).await;
    assert_eq!(s, StatusCode::FORBIDDEN, "nor list them");
    let (s, _) = rest_raw(&w.app, "GET", &format!("/api/v1/snapshots/{id}/download"), &w.editor, None).await;
    assert_eq!(s, StatusCode::FORBIDDEN, "nor download one");
    let (s, bytes) = rest_raw(&w.app, "GET", &format!("/api/v1/snapshots/{id}/download"), &w.owner, None).await;
    assert_eq!(s, StatusCode::OK, "the owner can");
    assert_eq!(&bytes[..2], &[0x1f, 0x8b], "and it is compressed");

    let (_, listing) = rest(&w.app, "POST", "/api/mcp", &w.owner, Some(json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/list" }))).await;
    let names: Vec<String> = listing["result"]["tools"].as_array().unwrap().iter().filter_map(|t| t["name"].as_str().map(str::to_string)).collect();
    // `read_library_snapshot` reads the policy library's own snapshot, which is not an app's data.
    assert!(!names.iter().any(|n| n.contains("snapshot") && n != "read_library_snapshot"), "no tool returns an app snapshot: {names:?}");
    runner.stop();
}

// 4. A sensitive snapshot's download writes a disclosure.
#[tokio::test]
async fn a_sensitive_snapshots_download_writes_a_disclosure() {
    let w = world().await;
    live_app(&w, &[("title", false), ("ssn", true)], 10).await;
    let runner = JobRunner::start(&w.state.repository, JOB_KINDS, fast()).unwrap();
    let (s, made) = take(&w, &w.owner, "Sensitive").await;
    assert_eq!(s, StatusCode::ACCEPTED, "{made}");
    assert_eq!(wait_job(&w, made["job_id"].as_str().unwrap()).state, "done");
    let id = made["snapshot_id"].as_str().unwrap().to_string();

    let count = |w: &World| -> i64 {
        let slug = w.slug.clone();
        w.db.run(move |c| c.query_one("SELECT count(*) FROM disclosure_log WHERE app_slug = $1 AND kind = 'snapshot'", &[&slug]).unwrap().get(0))
    };
    assert_eq!(count(&w), 0, "taking a snapshot discloses nothing to anyone");
    let (s, _) = rest_raw(&w.app, "GET", &format!("/api/v1/snapshots/{id}/download"), &w.owner, None).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(count(&w), 1, "the download is a disclosure");
    let fields: Vec<String> = w.db.run({
        let slug = w.slug.clone();
        move |c| c.query_one("SELECT fields FROM disclosure_log WHERE app_slug = $1 AND kind = 'snapshot'", &[&slug]).unwrap().get(0)
    });
    assert_eq!(fields, vec!["ssn".to_string()], "and names the sensitive field, not its values");
    runner.stop();
}
