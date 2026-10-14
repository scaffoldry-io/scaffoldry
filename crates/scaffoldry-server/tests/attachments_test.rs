//! Attachments phase 1: upload, store by hash, and download through the record checks.
//!
//! The files directory is a fresh temporary directory for each test. The directory is named by an environment
//! variable, so the tests run one at a time.

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::Router;
use http_body_util::BodyExt;
use scaffoldry_engine::{AppManifest, AppTable, AppView, FieldSpec, FieldType, ViewType};
use scaffoldry_server::build_app_with_state;
use scaffoldry_server::service::identity::issue_test_token_and_user;
use scaffoldry_server::state::{ServerState, SharedState};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::Mutex;
use tower::ServiceExt;
use uuid::Uuid;

static LOCK: Mutex<()> = Mutex::const_new(());

const OWNER: &str = "prof.curie@science.state.edu";
const EDITOR: &str = "student.smith@science.state.edu";
const VIEWER: &str = "marcus.vance@state.edu";
const OUTSIDER: &str = "sarah.connor@state.edu";
const WORKSPACE: &str = "ws-bio-lab";

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

struct Fixture {
    app: Router,
    state: SharedState,
    slug: String,
    dir: PathBuf,
    owner: String,
    editor: String,
    viewer: String,
    outsider: String,
}

/// An app with one table. `doc` is an attachment field anyone with access may read. `scan` is FERPA sensitive.
async fn fixture() -> Fixture {
    let dir = std::env::temp_dir().join(format!("scaffoldry-files-{}", Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    std::env::set_var("SCAFFOLDRY_FILES_DIR", &dir);
    let state = Arc::new(ServerState::new().expect("state"));
    state.seed_demo().expect("seed");
    let slug = format!("att-{}", &Uuid::new_v4().to_string()[..8]);
    let fields = || {
        vec![
            FieldSpec::simple("title", "Title", FieldType::Text, false, false),
            FieldSpec::simple("doc", "Documents", FieldType::Attachment { allow_multiple: true }, false, false),
            FieldSpec::simple("scan", "Scan", FieldType::Attachment { allow_multiple: true }, false, true),
        ]
    };
    state
        .engine
        .write()
        .unwrap()
        .register_manifest(AppManifest {
            slug: slug.clone(),
            title: "Attachments".to_string(),
            description: String::new(),
            organization_code: "BIO".to_string(),
            department: "biology".to_string(),
            workspace_id: Some(WORKSPACE.to_string()),
            herm_capability_id: None,
            custom_domain: None,
            custom_domain_verified: false,
            views: vec![AppView::table("main", "Main", ViewType::Table, fields())],
            pages: vec![],
            access: Default::default(),
            ceds_mappings: Default::default(),
            tables: vec![AppTable {
                id: "t1".to_string(),
                name: "Things".to_string(),
                slug: "things".to_string(),
                description: None,
                icon: None,
                fields: fields(),
                primary_field: Some("title".to_string()),
                sample_records: vec![],
            }],
            relationships: vec![],
        })
        .unwrap();
    Fixture {
        app: build_app_with_state(state.clone()).expect("router"),
        state,
        slug,
        dir,
        owner: issue_test_token_and_user(OWNER),
        editor: issue_test_token_and_user(EDITOR),
        viewer: issue_test_token_and_user(VIEWER),
        outsider: issue_test_token_and_user(OUTSIDER),
    }
}

struct Reply {
    status: StatusCode,
    headers: axum::http::HeaderMap,
    bytes: Vec<u8>,
}

impl Reply {
    fn json(&self) -> Value {
        serde_json::from_slice(&self.bytes).unwrap_or(Value::Null)
    }
    fn header(&self, name: &str) -> String {
        self.headers.get(name).map(|v| v.to_str().unwrap().to_string()).unwrap_or_default()
    }
}

async fn call(app: &Router, req: Request<Body>) -> Reply {
    let resp = app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let headers = resp.headers().clone();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes().to_vec();
    Reply { status, headers, bytes }
}

async fn upload(f: &Fixture, token: &str, name: &str, claimed_type: &str, bytes: Vec<u8>) -> Reply {
    let req = Request::builder()
        .method("POST")
        .uri(format!("/api/v1/apps/{}/files", f.slug))
        .header("authorization", format!("Bearer {token}"))
        .header("content-type", claimed_type)
        .header("content-length", bytes.len().to_string())
        .header("x-file-name", name)
        .body(Body::from(bytes))
        .unwrap();
    call(&f.app, req).await
}

async fn get(f: &Fixture, token: &str, uri: &str) -> Reply {
    let req = Request::builder().uri(uri).header("authorization", format!("Bearer {token}")).body(Body::empty()).unwrap();
    call(&f.app, req).await
}

async fn send_json(f: &Fixture, method: &str, token: &str, uri: &str, body: Value) -> Reply {
    let req = Request::builder()
        .method(method)
        .uri(uri)
        .header("authorization", format!("Bearer {token}"))
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap();
    call(&f.app, req).await
}

async fn create(f: &Fixture, token: &str, mut data: Value) -> Reply {
    data["_table_id"] = json!("t1");
    send_json(f, "POST", token, &format!("/api/v1/apps/{}/records", f.slug), json!({ "data": data })).await
}

fn download_uri(f: &Fixture, record: &str, ref_id: &str) -> String {
    format!("/api/v1/apps/{}/records/{}/files/{}", f.slug, record, ref_id)
}

fn png() -> Vec<u8> {
    let mut b = vec![0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];
    b.extend(b"\x00\x00\x00\x0dIHDR".iter());
    b.extend(std::iter::repeat_n(7u8, 200));
    b
}

fn hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}

fn blob_path(f: &Fixture, sha: &str) -> PathBuf {
    f.dir.join(&sha[0..2]).join(&sha[2..4]).join(sha)
}

fn files_on_disk(dir: &Path) -> usize {
    fn walk(p: &std::path::Path) -> usize {
        std::fs::read_dir(p).map(|d| d.flatten().map(|e| if e.path().is_dir() { walk(&e.path()) } else { 1 }).sum()).unwrap_or(0)
    }
    walk(dir)
}

fn doc_of(r: &Reply) -> Value {
    let j = r.json();
    json!([{ "ref_id": j["ref_id"], "name": j["name"] }])
}

// 1. A PNG goes in by hash, comes back inline with the safe headers.
#[tokio::test]
async fn a_png_is_stored_by_hash_and_served_inline_with_safe_headers() {
    let _db = common::TestDb::new();
    let _g = LOCK.lock().await;
    let f = fixture().await;
    let bytes = png();
    let up = upload(&f, &f.owner, "pic.png", "image/png", bytes.clone()).await;
    assert_eq!(up.status, StatusCode::CREATED, "{}", String::from_utf8_lossy(&up.bytes));
    let j = up.json();
    assert_eq!(j["media_type"], "image/png");
    assert_eq!(j["size"], bytes.len());
    assert_eq!(j["name"], "pic.png");
    assert!(blob_path(&f, &hex(&bytes)).exists(), "the file sits at aa/bb/sha256");

    let made = create(&f, &f.owner, json!({ "title": "one", "doc": doc_of(&up) })).await;
    assert_eq!(made.status, StatusCode::CREATED, "{}", String::from_utf8_lossy(&made.bytes));
    let id = made.json()["id"].as_str().unwrap().to_string();
    let down = get(&f, &f.owner, &download_uri(&f, &id, j["ref_id"].as_str().unwrap())).await;
    assert_eq!(down.status, StatusCode::OK);
    assert_eq!(down.bytes, bytes);
    assert_eq!(down.header("content-type"), "image/png");
    assert!(down.header("content-disposition").starts_with("inline"), "{}", down.header("content-disposition"));
    assert_eq!(down.header("x-content-type-options"), "nosniff");
    assert_eq!(down.header("content-security-policy"), "default-src 'none'; sandbox");
    assert_eq!(down.header("cache-control"), "private, no-store");
}

// 2. The client's type is ignored. The bytes decide.
#[tokio::test]
async fn the_bytes_decide_the_type_and_the_clients_claim_is_ignored() {
    let _db = common::TestDb::new();
    let _g = LOCK.lock().await;
    let f = fixture().await;
    let mut exe = b"MZ\x90\x00\x03\x00\x00\x00".to_vec();
    exe.extend(std::iter::repeat_n(1u8, 300));
    let r = upload(&f, &f.owner, "x.png", "image/png", exe).await;
    assert_eq!(r.status, StatusCode::BAD_REQUEST);
    assert_eq!(r.json()["code"], "file_type_blocked", "{}", String::from_utf8_lossy(&r.bytes));
    let mut elf = b"\x7fELF\x02\x01\x01\x00".to_vec();
    elf.extend(std::iter::repeat_n(1u8, 300));
    assert_eq!(upload(&f, &f.owner, "tool", "text/plain", elf).await.json()["code"], "file_type_blocked");

    let text = upload(&f, &f.owner, "x.png", "image/png", b"name,score\nAda,3\n".to_vec()).await;
    assert_eq!(text.status, StatusCode::CREATED, "{}", String::from_utf8_lossy(&text.bytes));
    assert_eq!(text.json()["media_type"], "text/csv");
    let made = create(&f, &f.owner, json!({ "doc": doc_of(&text) })).await;
    let id = made.json()["id"].as_str().unwrap().to_string();
    let down = get(&f, &f.owner, &download_uri(&f, &id, text.json()["ref_id"].as_str().unwrap())).await;
    assert_eq!(down.status, StatusCode::OK);
    assert!(down.header("content-disposition").starts_with("attachment"), "{}", down.header("content-disposition"));
    assert!(!down.header("content-type").starts_with("image/"));
}

// 3. A file over the limit is refused as it streams, and leaves nothing behind.
#[tokio::test]
async fn a_file_over_the_limit_is_refused_and_leaves_no_file() {
    let _db = common::TestDb::new();
    let _g = LOCK.lock().await;
    let f = fixture().await;
    f.state.settings.write().unwrap().insert("files.max_mb".to_string(), json!(1));
    let big = vec![b'a'; 2 * 1024 * 1024];
    let r = upload(&f, &f.owner, "big.txt", "text/plain", big).await;
    assert_eq!(r.status, StatusCode::PAYLOAD_TOO_LARGE);
    assert_eq!(r.json()["code"], "file_too_large", "{}", String::from_utf8_lossy(&r.bytes));
    assert_eq!(files_on_disk(&f.dir), 0, "no temporary file stays");

    // A body longer than its Content-Length says is cut off at the limit as well.
    let req = Request::builder()
        .method("POST")
        .uri(format!("/api/v1/apps/{}/files", f.slug))
        .header("authorization", format!("Bearer {}", f.owner))
        .header("content-length", "100")
        .header("x-file-name", "lie.txt")
        .body(Body::from(vec![b'a'; 2 * 1024 * 1024]))
        .unwrap();
    let lie = call(&f.app, req).await;
    assert!(lie.status == StatusCode::PAYLOAD_TOO_LARGE || lie.status == StatusCode::BAD_REQUEST, "{}", lie.status);
    assert_eq!(files_on_disk(&f.dir), 0);

    let missing = Request::builder()
        .method("POST")
        .uri(format!("/api/v1/apps/{}/files", f.slug))
        .header("authorization", format!("Bearer {}", f.owner))
        .header("x-file-name", "n.txt")
        .body(Body::from("hello"))
        .unwrap();
    assert_eq!(call(&f.app, missing).await.status, StatusCode::BAD_REQUEST, "Content-Length is required");
}

// 4. The same bytes twice make one blob and two references.
#[tokio::test]
async fn the_same_bytes_twice_are_one_blob_and_two_references() {
    let _db = common::TestDb::new();
    let _g = LOCK.lock().await;
    let f = fixture().await;
    let bytes = png();
    let a = upload(&f, &f.owner, "a.png", "image/png", bytes.clone()).await;
    let b = upload(&f, &f.owner, "b.png", "image/png", bytes.clone()).await;
    assert_eq!((a.status, b.status), (StatusCode::CREATED, StatusCode::CREATED));
    assert_ne!(a.json()["ref_id"], b.json()["ref_id"]);
    assert_eq!(files_on_disk(&f.dir), 1);
    let sha = hex(&bytes);
    let (blobs, refs) = db(move |c| {
        let blobs: i64 = c.query_one("SELECT COUNT(*) FROM files WHERE sha256 = $1", &[&sha]).unwrap().get(0);
        let refs: i64 = c.query_one("SELECT COUNT(*) FROM file_refs WHERE sha256 = $1", &[&sha]).unwrap().get(0);
        (blobs, refs)
    });
    assert_eq!(blobs, 1);
    assert!(refs >= 2, "{refs}");
}

// 5. Anyone without record or field access gets a 404, and a ref attached to another record is no key.
#[tokio::test]
async fn a_download_without_access_is_a_404() {
    let _db = common::TestDb::new();
    let _g = LOCK.lock().await;
    let f = fixture().await;
    let up = upload(&f, &f.owner, "a.png", "image/png", png()).await;
    let ref_id = up.json()["ref_id"].as_str().unwrap().to_string();
    let scan = upload(&f, &f.owner, "s.png", "image/png", {
        let mut b = png();
        b.push(9);
        b
    })
    .await;
    let scan_ref = scan.json()["ref_id"].as_str().unwrap().to_string();
    let made = create(&f, &f.owner, json!({ "doc": doc_of(&up), "scan": doc_of(&scan) })).await;
    assert_eq!(made.status, StatusCode::CREATED, "{}", String::from_utf8_lossy(&made.bytes));
    let id = made.json()["id"].as_str().unwrap().to_string();
    let other = create(&f, &f.owner, json!({ "title": "other" })).await.json()["id"].as_str().unwrap().to_string();

    assert_eq!(get(&f, &f.owner, &download_uri(&f, &id, &ref_id)).await.status, StatusCode::OK);
    assert_eq!(get(&f, &f.outsider, &download_uri(&f, &id, &ref_id)).await.status, StatusCode::NOT_FOUND, "no access to the app");
    assert_eq!(get(&f, &f.viewer, &download_uri(&f, &id, &scan_ref)).await.status, StatusCode::NOT_FOUND, "a viewer cannot see the sensitive field");
    assert_eq!(get(&f, &f.owner, &download_uri(&f, &other, &ref_id)).await.status, StatusCode::NOT_FOUND, "the ref belongs to another record");
    assert_eq!(get(&f, &f.owner, &download_uri(&f, &id, &Uuid::new_v4().to_string())).await.status, StatusCode::NOT_FOUND);
    assert_eq!(get(&f, &f.owner, &download_uri(&f, &id, "not-a-uuid")).await.status, StatusCode::NOT_FOUND);
}

// 6. A download from a sensitive field writes one file_download row.
#[tokio::test]
async fn a_sensitive_download_is_logged_once() {
    let _db = common::TestDb::new();
    let _g = LOCK.lock().await;
    let f = fixture().await;
    let scan = upload(&f, &f.owner, "s.png", "image/png", png()).await;
    let made = create(&f, &f.owner, json!({ "scan": doc_of(&scan) })).await;
    let id = made.json()["id"].as_str().unwrap().to_string();
    let plain = upload(&f, &f.owner, "p.png", "image/png", {
        let mut b = png();
        b.push(3);
        b
    })
    .await;
    let plain_rec = create(&f, &f.owner, json!({ "doc": doc_of(&plain) })).await.json()["id"].as_str().unwrap().to_string();

    let count = |slug: String| {
        db(move |c| {
            let n: i64 = c.query_one("SELECT COUNT(*) FROM disclosure_log WHERE app_slug = $1 AND kind = 'file_download'", &[&slug]).unwrap().get(0);
            n
        })
    };
    assert_eq!(count(f.slug.clone()), 0);
    assert_eq!(get(&f, &f.owner, &download_uri(&f, &plain_rec, plain.json()["ref_id"].as_str().unwrap())).await.status, StatusCode::OK);
    assert_eq!(count(f.slug.clone()), 0, "a field that is not sensitive writes nothing");
    assert_eq!(get(&f, &f.owner, &download_uri(&f, &id, scan.json()["ref_id"].as_str().unwrap())).await.status, StatusCode::OK);
    assert_eq!(count(f.slug.clone()), 1);
    let (rid, who) = {
        let slug = f.slug.clone();
        db(move |c| {
            let r = c
                .query_one(
                    "SELECT r.record_id, l.actor FROM disclosure_log l JOIN disclosure_records r ON r.disclosure_id = l.id WHERE l.app_slug = $1 AND l.kind = 'file_download'",
                    &[&slug],
                )
                .unwrap();
            (r.get::<_, String>(0), r.get::<_, String>(1))
        })
    };
    assert_eq!(rid, id);
    assert_eq!(who, OWNER);
}

// 7. Blocked names are refused, and a name is cleaned of its path.
#[tokio::test]
async fn blocked_names_are_refused_and_names_are_cleaned() {
    let _db = common::TestDb::new();
    let _g = LOCK.lock().await;
    let f = fixture().await;
    for name in ["a.exe", "b.svg", "c.html", "d.js", "e.HTM"] {
        let r = upload(&f, &f.owner, name, "text/plain", b"harmless words".to_vec()).await;
        assert_eq!(r.status, StatusCode::BAD_REQUEST, "{name}");
        assert_eq!(r.json()["code"], "file_type_blocked", "{name}");
    }
    let r = upload(&f, &f.owner, "../../etc/pass wd.txt", "text/plain", b"plain words".to_vec()).await;
    assert_eq!(r.status, StatusCode::CREATED, "{}", String::from_utf8_lossy(&r.bytes));
    let name = r.json()["name"].as_str().unwrap().to_string();
    assert!(!name.contains('/') && !name.contains(".."), "{name}");
    assert!(name.ends_with(".txt"), "{name}");
    // A browser sends the name percent-encoded.
    let accented = upload(&f, &f.owner, "r%C3%A9sum%C3%A9%20final.txt", "text/plain", b"plain words".to_vec()).await;
    assert_eq!(accented.json()["name"], "résumé final.txt", "{}", String::from_utf8_lossy(&accented.bytes));
    // A script or page by its bytes is refused whatever its name.
    let page = upload(&f, &f.owner, "notes.txt", "text/plain", b"<!DOCTYPE html><html><script>1</script></html>".to_vec()).await;
    assert_eq!(page.json()["code"], "file_type_blocked");
    let svg = upload(&f, &f.owner, "notes.txt", "text/plain", b"<svg xmlns=\"http://www.w3.org/2000/svg\"></svg>".to_vec()).await;
    assert_eq!(svg.json()["code"], "file_type_blocked");
}

// 8. A record may only hold files the writer uploaded or that it already holds.
#[tokio::test]
async fn a_record_may_only_attach_a_file_its_writer_uploaded() {
    let _db = common::TestDb::new();
    let _g = LOCK.lock().await;
    let f = fixture().await;
    let up = upload(&f, &f.owner, "a.png", "image/png", png()).await;
    let theirs = doc_of(&up);
    let r = create(&f, &f.editor, json!({ "doc": theirs })).await;
    assert_eq!(r.status, StatusCode::BAD_REQUEST, "someone else's upload is not mine to attach");
    let r = create(&f, &f.owner, json!({ "doc": [{ "ref_id": Uuid::new_v4().to_string(), "name": "ghost.png" }] })).await;
    assert_eq!(r.status, StatusCode::BAD_REQUEST, "an unknown ref");
    let r = create(&f, &f.owner, json!({ "doc": "not a list" })).await;
    assert_eq!(r.status, StatusCode::BAD_REQUEST);

    let made = create(&f, &f.owner, json!({ "doc": theirs })).await;
    assert_eq!(made.status, StatusCode::CREATED);
    // Another record cannot take the same ref, even from the person who uploaded it.
    let again = create(&f, &f.owner, json!({ "doc": theirs })).await;
    assert_eq!(again.status, StatusCode::BAD_REQUEST, "the ref is already attached elsewhere");
    // The editor can keep a file the record already holds.
    let id = made.json()["id"].as_str().unwrap().to_string();
    let v = made.json()["version"].clone();
    let kept = send_json(&f, "PATCH", &f.editor, &format!("/api/v1/apps/{}/records/{}", f.slug, id), json!({ "version": v, "data": { "title": "edited", "doc": theirs } })).await;
    assert_eq!(kept.status, StatusCode::OK, "{}", String::from_utf8_lossy(&kept.bytes));
}

// 9. An upload that is never attached goes after 24 hours. An attached one stays.
#[tokio::test]
async fn an_unattached_upload_is_removed_after_a_day() {
    let _db = common::TestDb::new();
    let _g = LOCK.lock().await;
    let f = fixture().await;
    let keep = upload(&f, &f.owner, "k.png", "image/png", png()).await;
    let lose_bytes = {
        let mut b = png();
        b.extend(b"unique");
        b
    };
    let lose = upload(&f, &f.owner, "l.png", "image/png", lose_bytes.clone()).await;
    let fresh = upload(&f, &f.owner, "f.png", "image/png", {
        let mut b = png();
        b.extend(b"fresh");
        b
    })
    .await;
    assert_eq!(create(&f, &f.owner, json!({ "doc": doc_of(&keep) })).await.status, StatusCode::CREATED);
    let old = [keep.json()["ref_id"].clone(), lose.json()["ref_id"].clone()];
    db(move |c| {
        for r in old {
            let id: Uuid = r.as_str().unwrap().parse().unwrap();
            c.execute("UPDATE file_refs SET uploaded_at = NOW() - INTERVAL '25 hours' WHERE id = $1", &[&id]).unwrap();
        }
    });
    let repo = f.state.repository.clone();
    let removed = scaffoldry_server::files::purge_unattached(&repo, 24).expect("purge");
    assert!(removed >= 1, "{removed}");
    let ids: Vec<Uuid> = [&keep, &lose, &fresh].iter().map(|r| r.json()["ref_id"].as_str().unwrap().parse().unwrap()).collect();
    let alive: Vec<bool> = db(move |c| ids.iter().map(|id| c.query_opt("SELECT 1 FROM file_refs WHERE id = $1", &[id]).unwrap().is_some()).collect());
    assert_eq!(alive, vec![true, false, true]);
    assert!(!blob_path(&f, &hex(&lose_bytes)).exists(), "the unreferenced blob goes with it");
    assert!(blob_path(&f, &hex(&png())).exists(), "a blob that is still referenced stays");
}

// 10. Boot refuses a files directory that is missing or cannot be written.
#[tokio::test]
async fn boot_refuses_a_missing_or_unwritable_files_directory() {
    let _db = common::TestDb::new();
    let _g = LOCK.lock().await;
    let missing = std::env::temp_dir().join(format!("scaffoldry-nothing-{}", Uuid::new_v4()));
    std::env::set_var("SCAFFOLDRY_FILES_DIR", &missing);
    assert!(scaffoldry_server::files::open_dir().is_err());
    let ok = std::env::temp_dir().join(format!("scaffoldry-files-{}", Uuid::new_v4()));
    std::fs::create_dir_all(&ok).unwrap();
    std::env::set_var("SCAFFOLDRY_FILES_DIR", &ok);
    assert!(scaffoldry_server::files::open_dir().is_ok());
    // A path that is a file is no directory.
    let plain = ok.join("plain");
    std::fs::write(&plain, b"x").unwrap();
    std::env::set_var("SCAFFOLDRY_FILES_DIR", &plain);
    assert!(scaffoldry_server::files::open_dir().is_err());
    std::env::remove_var("SCAFFOLDRY_FILES_DIR");
    assert!(scaffoldry_server::files::open_dir().is_ok(), "unset means uploads are off, which is not a boot error");
}

// 11. A record lists its files with their size, type and scan state, and only for the fields the person may read.
#[tokio::test]
async fn a_record_lists_its_files_for_the_fields_a_person_may_read() {
    let _db = common::TestDb::new();
    let _g = LOCK.lock().await;
    let f = fixture().await;
    let doc = upload(&f, &f.owner, "a.png", "image/png", png()).await;
    let scan = upload(&f, &f.owner, "s.png", "image/png", {
        let mut b = png();
        b.push(11);
        b
    })
    .await;
    let made = create(&f, &f.owner, json!({ "doc": doc_of(&doc), "scan": doc_of(&scan) })).await;
    let id = made.json()["id"].as_str().unwrap().to_string();
    // The record itself carries what a grid chip shows.
    let held = &made.json()["data"]["doc"][0];
    assert_eq!(held["name"], "a.png", "{}", String::from_utf8_lossy(&made.bytes));
    assert_eq!(held["size"], png().len());
    assert_eq!(held["media_type"], "image/png");

    let uri = format!("/api/v1/apps/{}/records/{}/files", f.slug, id);
    let all = get(&f, &f.owner, &uri).await;
    assert_eq!(all.status, StatusCode::OK, "{}", String::from_utf8_lossy(&all.bytes));
    let body = all.json();
    assert_eq!(body["fields"], json!(["doc", "scan"]));
    let files = body["files"].as_array().unwrap();
    assert_eq!(files.len(), 2);
    let first = files.iter().find(|x| x["field"] == "doc").unwrap();
    assert_eq!(first["ref_id"], doc.json()["ref_id"]);
    assert_eq!(first["scan_state"], "skipped");
    assert_eq!(first["media_type"], "image/png");

    let seen = get(&f, &f.viewer, &uri).await;
    assert_eq!(seen.status, StatusCode::OK);
    assert_eq!(seen.json()["fields"], json!(["doc"]), "the sensitive field is not even named");
    assert!(seen.json()["files"].as_array().unwrap().iter().all(|x| x["field"] == "doc"));
    assert_eq!(get(&f, &f.outsider, &uri).await.status, StatusCode::FORBIDDEN);
}
