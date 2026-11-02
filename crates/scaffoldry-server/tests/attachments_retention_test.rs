//! Attachments, phase 4: orphan blobs are purged, and bundles carry their files.
//!
//! A reference a record lets go of, or a record retention destroys, leaves the blob alone until `purge_orphan_blobs` runs.
//! The job deletes a blob only when no live reference remains and none was made in the last 24 hours, and its ledger entry
//! holds a count, never a file name. A bundle carries each sample file by hash, with its bytes beside the document, and an
//! import checks every hash before it writes anything. NIST SP 800-53 SI-12, MP-6 and SI-7.
//!
//! The brief's second test, "a held reference's blob is untouched by the job", waits on legal holds (`lifecycle.md` phase 4).

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
use sha2::{Digest, Sha256};
use std::sync::OnceLock;
use std::time::{Duration, Instant};
use tower::ServiceExt;

const WORKSPACE: &str = "ws-physics-optics";
const OWNER: &str = "einstein@physics.state.edu";

fn files_dir() -> std::path::PathBuf {
    static DIR: OnceLock<std::path::PathBuf> = OnceLock::new();
    DIR.get_or_init(|| {
        let dir = std::env::temp_dir().join(format!("scaffoldry-orphans-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        std::env::set_var("SCAFFOLDRY_FILES_DIR", &dir);
        dir
    })
    .clone()
}

struct World {
    db: common::TestDb,
    state: std::sync::Arc<ServerState>,
    app: Router,
    owner: String,
    slug: String,
}

async fn send(app: &Router, req: Request<Body>) -> (StatusCode, Value) {
    let resp = app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
}

async fn rest(w: &World, method: &str, uri: &str, body: Option<Value>) -> (StatusCode, Value) {
    let b = Request::builder().method(method).uri(uri).header("authorization", format!("Bearer {}", w.owner));
    let req = match body {
        Some(v) => b.header("content-type", "application/json").body(Body::from(v.to_string())).unwrap(),
        None => b.body(Body::empty()).unwrap(),
    };
    send(&w.app, req).await
}

async fn upload(w: &World, name: &str, bytes: &[u8]) -> Value {
    let req = Request::builder()
        .method("POST")
        .uri(format!("/api/v1/apps/{}/files", w.slug))
        .header("authorization", format!("Bearer {}", w.owner))
        .header("content-type", "text/plain")
        .header("content-length", bytes.len().to_string())
        .header("x-file-name", name)
        .body(Body::from(bytes.to_vec()))
        .unwrap();
    let (s, b) = send(&w.app, req).await;
    assert!(s.is_success(), "{s} {b}");
    b
}

async fn world() -> World {
    files_dir();
    let db = common::TestDb::new();
    let state = db.state();
    state.seed_demo().expect("seed");
    let tag = uuid::Uuid::new_v4().to_string()[..8].to_string();
    let app = build_app_with_state(state.clone()).expect("router");
    let w = World { db, state, app, owner: issue_test_token_and_user(OWNER), slug: format!("orph-{tag}") };
    let manifest = json!({
        "slug": w.slug, "title": "Orphans", "description": "Files that lose their records", "organization_code": "PHYS", "department": "physics",
        "workspace_id": WORKSPACE, "views": [],
        "tables": [{ "id": "cases", "name": "Cases", "slug": "cases", "primary_field": "title", "fields": [
            { "name": "title", "label": "Title", "field_type": "Text", "required": false, "ferpa_sensitive": false },
            { "name": "doc", "label": "Documents", "field_type": "Attachment", "required": false, "ferpa_sensitive": false }
        ]}],
    });
    let (s, b) = rest(&w, "POST", &format!("/api/v1/workspaces/{WORKSPACE}/apps"), Some(manifest)).await;
    assert_eq!(s, StatusCode::CREATED, "{b}");
    w
}

/// A record whose `doc` field holds the upload.
async fn record_with(w: &World, title: &str, file: &Value) -> Value {
    let (s, r) = rest(w, "POST", &format!("/api/v1/apps/{}/records", w.slug), Some(json!({ "data": { "_table_id": "cases", "title": title, "doc": [file] } }))).await;
    assert_eq!(s, StatusCode::CREATED, "{r}");
    r
}

async fn clear(w: &World, record: &Value) {
    let id = record["id"].as_str().unwrap();
    let (_, now) = rest(w, "GET", &format!("/api/v1/apps/{}/records/{id}", w.slug), None).await;
    let (s, b) = rest(w, "PATCH", &format!("/api/v1/apps/{}/records/{id}", w.slug), Some(json!({ "version": now["version"], "data": { "_table_id": "cases", "title": now["data"]["title"], "doc": [] } }))).await;
    assert_eq!(s, StatusCode::OK, "{b}");
}

fn sha(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}

fn blob_exists(sha: &str) -> bool {
    files_dir().join(&sha[0..2]).join(&sha[2..4]).join(sha).exists()
}

fn file_row(w: &World, sha: &str) -> bool {
    let sha = sha.to_string();
    w.db.run(move |c| c.query_one("SELECT count(*) FROM files WHERE sha256 = $1", &[&sha]).unwrap().get::<_, i64>(0) == 1)
}

/// Moves every reference's upload time back, so the 24-hour grace has passed.
fn age_refs(w: &World) {
    let slug = w.slug.clone();
    w.db.run(move |c| {
        c.execute("UPDATE file_refs SET uploaded_at = NOW() - interval '25 hours' WHERE app_slug = $1", &[&slug]).unwrap();
    });
}

fn run_job(w: &World) -> scaffoldry_server::jobs::Job {
    let runner = JobRunner::start(&w.state.repository, JOB_KINDS, JobConfig { workers: 1, poll_ms: 20, backoff_secs: [0, 0, 0], ..JobConfig::default() }).unwrap();
    let id = enqueue(&w.state.repository, "purge_orphan_blobs", json!({}), "test@scaffoldry.internal", None).expect("queued");
    let deadline = Instant::now() + Duration::from_secs(120);
    let job = loop {
        let job = get_job(&w.state.repository, id).unwrap().expect("the job");
        if job.state == "done" || job.state == "failed" {
            break job;
        }
        assert!(Instant::now() < deadline, "the job is stuck in {}", job.state);
        std::thread::sleep(Duration::from_millis(50));
    };
    runner.stop();
    job
}

// 1. Removing the only reference leaves the blob until the job runs, then deletes it. A second reference keeps it.
#[tokio::test]
async fn an_orphan_blob_waits_for_the_job_and_a_second_reference_keeps_it() {
    let w = world().await;
    let lone = b"only one record holds this";
    let shared = b"two records hold this";
    let a = record_with(&w, "A", &upload(&w, "lone.txt", lone).await).await;
    let b = record_with(&w, "B", &upload(&w, "shared.txt", shared).await).await;
    record_with(&w, "C", &upload(&w, "shared-again.txt", shared).await).await;
    clear(&w, &a).await;
    clear(&w, &b).await;
    assert!(blob_exists(&sha(lone)), "removing the reference does not touch the blob");

    // Within the 24-hour grace, the job leaves even an orphan.
    assert_eq!(run_job(&w).state, "done");
    assert!(blob_exists(&sha(lone)) && file_row(&w, &sha(lone)), "a reference made in the last 24 hours keeps it");

    age_refs(&w);
    assert_eq!(run_job(&w).state, "done");
    assert!(!blob_exists(&sha(lone)) && !file_row(&w, &sha(lone)), "the orphan is gone");
    assert!(blob_exists(&sha(shared)) && file_row(&w, &sha(shared)), "C still holds the shared bytes");
}

// 4. The purge ledger entry holds a count and no file names.
#[tokio::test]
async fn the_purge_ledger_entry_holds_a_count_and_no_file_name() {
    let w = world().await;
    let r = record_with(&w, "A", &upload(&w, "transcript-of-jane-doe.txt", b"private bytes").await).await;
    clear(&w, &r).await;
    age_refs(&w);
    assert_eq!(run_job(&w).state, "done");
    let entries: Vec<(Value, String)> =
        w.db.run(|c| c.query("SELECT payload, rationale FROM governance_ledger WHERE decision_type = 'BlobsPurged'", &[]).unwrap().iter().map(|r| (r.get(0), r.get(1))).collect());
    assert_eq!(entries.len(), 1, "one entry for the run");
    let (payload, rationale) = &entries[0];
    assert!(payload["count"].as_i64().unwrap_or(0) >= 1, "the entry counts the blobs");
    let all = format!("{payload} {rationale}");
    assert!(!all.contains("transcript") && !all.contains(&sha(b"private bytes")), "no file name or hash is kept");
}

// 3. Importing a bundle with a corrupted file fails with its name and writes nothing.
#[tokio::test]
async fn a_bundle_carries_its_files_and_an_import_refuses_a_damaged_one_by_name() {
    let w = world().await;
    record_with(&w, "A", &upload(&w, "award-letter.txt", b"the signed award letter").await).await;
    let (s, bundle) = rest(&w, "POST", &format!("/api/v1/apps/{}/bundle", w.slug), Some(json!({ "sample_data": true }))).await;
    assert_eq!(s, StatusCode::OK, "{bundle}");
    let key = sha(b"the signed award letter");
    let held = &bundle["files"][&key];
    assert_eq!(held["name"], "award-letter.txt", "the bundle carries the file by its hash: {}", bundle["files"]);
    assert_eq!(bundle["sample_data"]["cases"][0]["doc"][0]["sha256"], key.as_str(), "the sample record names the hash");

    // An intact bundle imports, and says what came with it.
    let new_slug = format!("{}-copy", w.slug);
    let (s, ok) = rest(&w, "POST", &format!("/api/v1/workspaces/{WORKSPACE}/bundle-import"), Some(json!({ "bundle": bundle, "slug": new_slug }))).await;
    assert!(s.is_success(), "{s} {ok}");

    // One byte changed in the file, and nothing is written.
    let mut damaged = bundle.clone();
    damaged["files"][&key]["bytes"] = json!(base64_of(b"the signed award lettex"));
    let other = format!("{}-bad", w.slug);
    let proposals_before: i64 = w.db.run(|c| c.query_one("SELECT count(*) FROM app_proposals", &[]).unwrap().get(0));
    let (s, b) = rest(&w, "POST", &format!("/api/v1/workspaces/{WORKSPACE}/bundle-import"), Some(json!({ "bundle": damaged, "slug": other }))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{b}");
    assert!(b["error"].as_str().unwrap_or("").contains("award-letter.txt"), "the refusal names the file: {b}");
    let proposals_after: i64 = w.db.run(|c| c.query_one("SELECT count(*) FROM app_proposals", &[]).unwrap().get(0));
    assert_eq!(proposals_after, proposals_before, "no proposal is written");
    assert!(w.state.repository.get_app_manifest(&other).unwrap().is_none(), "no app is written");
}

fn base64_of(bytes: &[u8]) -> String {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

// A snapshot's file is a blob with no record reference. Neither purge may take it while the snapshot names it.
#[tokio::test]
async fn a_snapshot_file_survives_both_file_purges() {
    let w = world().await;
    let bytes = b"a gzip snapshot stands in here";
    let key = sha(bytes);
    let path = files_dir().join(&key[0..2]).join(&key[2..4]).join(&key);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, bytes).unwrap();
    let (slug, k) = (w.slug.clone(), key.clone());
    w.db.run(move |c| {
        c.execute("INSERT INTO files (sha256, size, media_type) VALUES ($1, 30, 'application/gzip') ON CONFLICT DO NOTHING", &[&k]).unwrap();
        c.execute(
            "INSERT INTO app_snapshots (id, app_slug, name, kind, state, file_sha256, definition, created_by) VALUES ($1, $2, 'Nightly', 'manual', 'ready', $3, '{}', 'x')",
            &[&uuid::Uuid::new_v4(), &slug, &k],
        )
        .unwrap();
    });
    assert_eq!(run_job(&w).state, "done");
    scaffoldry_server::files::purge_unattached(&w.state.repository, 0).expect("the hourly cleanup runs");
    assert!(blob_exists(&key) && file_row(&w, &key), "the snapshot's file is kept");
}
