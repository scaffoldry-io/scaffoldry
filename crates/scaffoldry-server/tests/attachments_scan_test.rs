//! Attachments phase 2: scanning through a daemon on a socket.
//!
//! The scanner is a fake on a temporary socket. It speaks the daemon's streaming protocol and answers as the test says.
//! The real daemon is never run. The tests share one database and one environment variable, so they run one at a time.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::Router;
use http_body_util::BodyExt;
use scaffoldry_engine::{AppManifest, AppTable, AppView, FieldSpec, FieldType, ViewType};
use scaffoldry_server::build_app_with_state;
use scaffoldry_server::service::identity::issue_test_token_and_user;
use scaffoldry_server::state::{ServerState, SharedState};
use serde_json::{json, Value};
use std::io::{Read, Write};
use std::os::unix::net::UnixListener;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::sync::Mutex;
use tower::ServiceExt;
use uuid::Uuid;

static LOCK: Mutex<()> = Mutex::const_new(());

const OWNER: &str = "prof.curie@science.state.edu";
const ADMIN: &str = "jordan.lee@state.edu";
const WORKSPACE: &str = "ws-bio-lab";
const EICAR: &[u8] = b"X5O!P%@AP[4\\PZX54(P^)7CC)7}$EICAR-STANDARD-ANTIVIRUS-TEST-FILE!$H+H*";

fn db_url() -> String {
    std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://scaffoldry:scaffoldry_dev_password@127.0.0.1:5433/scaffoldry".to_string())
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

/// What the fake scanner does with a stream.
#[derive(Clone, Copy, PartialEq)]
enum Mode {
    /// Always "OK".
    Clean,
    /// Always "FOUND".
    Infected,
    /// "FOUND" when the stream holds the EICAR string, "OK" otherwise.
    Eicar,
    /// Reads the stream and never answers.
    Silent,
}

struct Fake {
    socket: PathBuf,
    stop: Arc<AtomicBool>,
}

impl Drop for Fake {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        let _ = std::os::unix::net::UnixStream::connect(&self.socket);
        let _ = std::fs::remove_file(&self.socket);
    }
}

fn fake_scanner(mode: Mode) -> Fake {
    let socket = std::env::temp_dir().join(format!("scaffoldry-clamd-{}.sock", Uuid::new_v4()));
    let listener = UnixListener::bind(&socket).unwrap();
    let stop = Arc::new(AtomicBool::new(false));
    let flag = stop.clone();
    std::thread::spawn(move || {
        for conn in listener.incoming() {
            if flag.load(Ordering::SeqCst) {
                break;
            }
            let Ok(mut conn) = conn else { continue };
            std::thread::spawn(move || {
                // The command ends with a NUL.
                let mut command = Vec::new();
                let mut byte = [0u8; 1];
                while conn.read_exact(&mut byte).is_ok() {
                    if byte[0] == 0 {
                        break;
                    }
                    command.push(byte[0]);
                }
                if command == b"zPING" || command == b"PING" {
                    let _ = conn.write_all(b"PONG\0");
                    return;
                }
                assert!(command == b"zINSTREAM" || command == b"INSTREAM", "unexpected command {:?}", String::from_utf8_lossy(&command));
                let mut body = Vec::new();
                loop {
                    let mut len = [0u8; 4];
                    if conn.read_exact(&mut len).is_err() {
                        return;
                    }
                    let n = u32::from_be_bytes(len) as usize;
                    if n == 0 {
                        break;
                    }
                    let mut chunk = vec![0u8; n];
                    if conn.read_exact(&mut chunk).is_err() {
                        return;
                    }
                    body.extend(chunk);
                }
                let answer: &[u8] = match mode {
                    Mode::Clean => b"stream: OK\0",
                    Mode::Infected => b"stream: Fake.Virus FOUND\0",
                    Mode::Eicar if body.windows(EICAR.len()).any(|w| w == EICAR) => b"stream: Eicar-Test-Signature FOUND\0",
                    Mode::Eicar => b"stream: OK\0",
                    Mode::Silent => {
                        std::thread::sleep(std::time::Duration::from_secs(5));
                        return;
                    }
                };
                let _ = conn.write_all(answer);
            });
        }
    });
    Fake { socket, stop }
}

/// Removes the scanner settings and the timeout, so a failed test leaves nothing for the next.
struct Reset;

impl Drop for Reset {
    fn drop(&mut self) {
        std::env::remove_var("SCAFFOLDRY_SCAN_TIMEOUT_MS");
        db(|c| {
            c.execute("DELETE FROM platform_settings WHERE key IN ('files.scanner', 'files.require_scan')", &[]).unwrap();
        });
    }
}

struct Fixture {
    app: Router,
    state: SharedState,
    slug: String,
    dir: PathBuf,
    owner: String,
    admin: String,
}

async fn fixture() -> Fixture {
    db(|c| {
        c.execute("DELETE FROM platform_settings WHERE key IN ('files.scanner', 'files.require_scan')", &[]).unwrap();
    });
    let dir = std::env::temp_dir().join(format!("scaffoldry-files-{}", Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    std::env::set_var("SCAFFOLDRY_FILES_DIR", &dir);
    let state = Arc::new(ServerState::new().expect("state"));
    state.seed_demo().expect("seed");
    let slug = format!("scan-{}", &Uuid::new_v4().to_string()[..8]);
    let fields = || {
        vec![
            FieldSpec::simple("title", "Title", FieldType::Text, false, false),
            FieldSpec::simple("doc", "Documents", FieldType::Attachment { allow_multiple: true }, false, false),
        ]
    };
    state
        .engine
        .write()
        .unwrap()
        .register_manifest(AppManifest {
            slug: slug.clone(),
            title: "Scanning".to_string(),
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
        admin: issue_test_token_and_user(ADMIN),
    }
}

struct Reply {
    status: StatusCode,
    bytes: Vec<u8>,
}

impl Reply {
    fn json(&self) -> Value {
        serde_json::from_slice(&self.bytes).unwrap_or(Value::Null)
    }
    fn text(&self) -> String {
        String::from_utf8_lossy(&self.bytes).to_string()
    }
}

async fn call(app: &Router, req: Request<Body>) -> Reply {
    let resp = app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    Reply { status, bytes: resp.into_body().collect().await.unwrap().to_bytes().to_vec() }
}

async fn upload(f: &Fixture, name: &str, bytes: Vec<u8>) -> Reply {
    let req = Request::builder()
        .method("POST")
        .uri(format!("/api/v1/apps/{}/files", f.slug))
        .header("authorization", format!("Bearer {}", f.owner))
        .header("content-length", bytes.len().to_string())
        .header("x-file-name", name)
        .body(Body::from(bytes))
        .unwrap();
    call(&f.app, req).await
}

async fn json_call(f: &Fixture, method: &str, token: &str, uri: &str, body: Option<Value>) -> Reply {
    let b = Request::builder().method(method).uri(uri).header("authorization", format!("Bearer {token}"));
    let req = match body {
        Some(v) => b.header("content-type", "application/json").body(Body::from(v.to_string())).unwrap(),
        None => b.body(Body::empty()).unwrap(),
    };
    call(&f.app, req).await
}

async fn set_setting(f: &Fixture, key: &str, value: Value) {
    let r = json_call(f, "PUT", &f.admin, &format!("/api/v1/settings/{key}"), Some(value)).await;
    assert_eq!(r.status, StatusCode::OK, "{key}: {}", r.text());
}

async fn use_scanner(f: &Fixture, fake: &Fake) {
    set_setting(f, "files.scanner", json!({ "kind": "clamd", "socket": fake.socket.to_str().unwrap() })).await;
}

async fn create(f: &Fixture, files: &[&Reply]) -> Reply {
    let doc: Vec<Value> = files.iter().map(|r| json!({ "ref_id": r.json()["ref_id"], "name": r.json()["name"] })).collect();
    json_call(f, "POST", &f.owner, &format!("/api/v1/apps/{}/records", f.slug), Some(json!({ "data": { "_table_id": "t1", "doc": doc } }))).await
}

fn png(tag: u8) -> Vec<u8> {
    let mut b = vec![0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];
    b.extend(std::iter::repeat_n(tag, 200));
    b
}

fn blob_exists(dir: &Path) -> usize {
    fn walk(p: &Path) -> usize {
        std::fs::read_dir(p).map(|d| d.flatten().map(|e| if e.path().is_dir() { walk(&e.path()) } else { 1 }).sum()).unwrap_or(0)
    }
    walk(dir)
}

fn ref_id_of(r: &Reply) -> Uuid {
    r.json()["ref_id"].as_str().unwrap().parse().unwrap()
}

fn scan(f: &Fixture, id: Uuid) -> String {
    let repo = f.state.repository.clone().expect("repository");
    scaffoldry_server::scan::scan_file(&repo, id).expect("scan")
}

fn state_of(id: Uuid) -> String {
    db(move |c| c.query_one("SELECT scan_state FROM file_refs WHERE id = $1", &[&id]).unwrap().get(0))
}

// 1. A clean answer sets clean. An infected answer removes the blob, marks the reference, and writes a ledger entry.
#[tokio::test]
async fn a_clean_answer_is_clean_and_an_infected_answer_removes_the_file() {
    let _g = LOCK.lock().await;
    let _reset = Reset;
    let f = fixture().await;

    let fake = fake_scanner(Mode::Clean);
    use_scanner(&f, &fake).await;
    let up = upload(&f, "a.png", png(1)).await;
    assert_eq!(up.status, StatusCode::CREATED, "{}", up.text());
    assert_eq!(up.json()["scan_state"], "pending");
    let id = ref_id_of(&up);
    assert_eq!(state_of(id), "pending");
    let queued: i64 = db(|c| c.query_one("SELECT COUNT(*) FROM jobs WHERE kind = 'scan_file' AND state IN ('queued', 'running')", &[]).unwrap().get(0));
    assert!(queued >= 1, "the upload queues a scan");
    // Many uploads at once do not meet the per-person limit on jobs.
    for tag in 20..28u8 {
        let more = upload(&f, "m.png", png(tag)).await;
        assert_eq!(more.json()["scan_state"], "pending", "{}", more.text());
    }
    assert_eq!(scan(&f, id), "clean");
    assert_eq!(state_of(id), "clean");
    assert_eq!(create(&f, &[&up]).await.status, StatusCode::CREATED);
    drop(fake);

    let fake = fake_scanner(Mode::Infected);
    use_scanner(&f, &fake).await;
    let bad = upload(&f, "b.png", png(2)).await;
    let bad_id = ref_id_of(&bad);
    let blobs = blob_exists(&f.dir);
    let before: i64 = db(|c| c.query_one("SELECT COUNT(*) FROM governance_ledger WHERE decision_type = 'FileInfected'", &[]).unwrap().get(0));
    assert_eq!(scan(&f, bad_id), "infected");
    assert_eq!(state_of(bad_id), "infected");
    assert_eq!(blob_exists(&f.dir), blobs - 1, "the infected blob is gone and the others stay");
    let deleted: bool = db(move |c| c.query_one("SELECT deleted_at IS NOT NULL FROM file_refs WHERE id = $1", &[&bad_id]).unwrap().get(0));
    assert!(deleted, "the reference is marked deleted");
    let (after, rationale): (i64, String) = db(|c| {
        let n: i64 = c.query_one("SELECT COUNT(*) FROM governance_ledger WHERE decision_type = 'FileInfected'", &[]).unwrap().get(0);
        let r: String = c.query_one("SELECT rationale FROM governance_ledger WHERE decision_type = 'FileInfected' ORDER BY sequence DESC LIMIT 1", &[]).unwrap().get(0);
        (n, r)
    });
    assert_eq!(after, before + 1);
    assert!(rationale.contains(OWNER), "{rationale}");
    // The record never holds an infected file.
    assert_eq!(create(&f, &[&bad]).await.status, StatusCode::BAD_REQUEST);
}

// 2. A scanner that does not answer in time is an error result.
#[tokio::test]
async fn a_scanner_that_does_not_answer_is_an_error() {
    let _g = LOCK.lock().await;
    let _reset = Reset;
    let f = fixture().await;
    std::env::set_var("SCAFFOLDRY_SCAN_TIMEOUT_MS", "300");
    let fake = fake_scanner(Mode::Silent);
    use_scanner(&f, &fake).await;
    let up = upload(&f, "slow.png", png(3)).await;
    let id = ref_id_of(&up);
    let started = std::time::Instant::now();
    assert_eq!(scan(&f, id), "error");
    assert!(started.elapsed() < std::time::Duration::from_secs(4), "{:?}", started.elapsed());
    assert_eq!(state_of(id), "error");
}

// 3. A pending file cannot be attached or downloaded.
#[tokio::test]
async fn a_pending_file_cannot_be_attached_or_downloaded() {
    let _g = LOCK.lock().await;
    let _reset = Reset;
    let f = fixture().await;
    let fake = fake_scanner(Mode::Clean);
    use_scanner(&f, &fake).await;
    let up = upload(&f, "p.png", png(4)).await;
    let id = ref_id_of(&up);
    let refused = create(&f, &[&up]).await;
    assert_eq!(refused.status, StatusCode::BAD_REQUEST, "{}", refused.text());
    assert_eq!(scan(&f, id), "clean");
    let made = create(&f, &[&up]).await;
    assert_eq!(made.status, StatusCode::CREATED, "{}", made.text());
    let record = made.json()["id"].as_str().unwrap().to_string();
    let uri = format!("/api/v1/apps/{}/records/{}/files/{}", f.slug, record, id);
    assert_eq!(json_call(&f, "GET", &f.owner, &uri, None).await.status, StatusCode::OK);
    db(move |c| c.execute("UPDATE file_refs SET scan_state = 'pending' WHERE id = $1", &[&id]).unwrap());
    assert_eq!(json_call(&f, "GET", &f.owner, &uri, None).await.status, StatusCode::NOT_FOUND);
}

// 4. With no scanner a file is skipped. With require_scan an upload is scanner_unavailable.
#[tokio::test]
async fn with_no_scanner_a_file_is_skipped_and_require_scan_refuses_uploads() {
    let _g = LOCK.lock().await;
    let _reset = Reset;
    let f = fixture().await;
    let up = upload(&f, "s.png", png(5)).await;
    assert_eq!(up.status, StatusCode::CREATED);
    assert_eq!(up.json()["scan_state"], "skipped");
    assert_eq!(create(&f, &[&up]).await.status, StatusCode::CREATED, "a skipped file attaches");

    set_setting(&f, "files.require_scan", json!(true)).await;
    let refused = upload(&f, "t.png", png(6)).await;
    assert_eq!(refused.status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(refused.json()["code"], "scanner_unavailable", "{}", refused.text());
    assert_eq!(blob_exists(&f.dir.clone()), 1, "the refused upload leaves no file");

    // A scanner that is set but does not answer counts the same.
    set_setting(&f, "files.scanner", json!({ "kind": "clamd", "socket": "/nonexistent/clamd.ctl" })).await;
    assert_eq!(upload(&f, "u.png", png(7)).await.json()["code"], "scanner_unavailable");
    // With the scanner back, the upload goes.
    let fake = fake_scanner(Mode::Clean);
    use_scanner(&f, &fake).await;
    assert_eq!(upload(&f, "v.png", png(8)).await.status, StatusCode::CREATED);
    // Neither setting is accepted in a wrong shape.
    for (key, value) in [("files.scanner", json!({ "kind": "other", "socket": "/x" })), ("files.scanner", json!("clamd")), ("files.require_scan", json!("yes"))] {
        let r = json_call(&f, "PUT", &f.admin, &format!("/api/v1/settings/{key}"), Some(value)).await;
        assert_eq!(r.status, StatusCode::BAD_REQUEST, "{key}");
    }
}

// 5. The EICAR string is infected by the fake's matching rule.
#[tokio::test]
async fn the_eicar_string_is_treated_as_infected() {
    let _g = LOCK.lock().await;
    let _reset = Reset;
    let f = fixture().await;
    let fake = fake_scanner(Mode::Eicar);
    use_scanner(&f, &fake).await;
    let bad = upload(&f, "eicar.txt", EICAR.to_vec()).await;
    assert_eq!(bad.status, StatusCode::CREATED, "{}", bad.text());
    let ok = upload(&f, "fine.txt", b"just some words".to_vec()).await;
    assert_eq!(scan(&f, ref_id_of(&bad)), "infected");
    assert_eq!(scan(&f, ref_id_of(&ok)), "clean");
}

// 6. The Overview reports the scanner.
#[tokio::test]
async fn the_overview_reports_the_scanner() {
    let _g = LOCK.lock().await;
    let _reset = Reset;
    let f = fixture().await;
    let none = json_call(&f, "GET", &f.admin, "/api/v1/admin/overview", None).await.json();
    assert_eq!(none["files"]["scanner"]["configured"], false, "{none}");

    let fake = fake_scanner(Mode::Clean);
    use_scanner(&f, &fake).await;
    let up = upload(&f, "o.png", png(9)).await;
    assert_eq!(scan(&f, ref_id_of(&up)), "clean");
    let seen = json_call(&f, "GET", &f.admin, "/api/v1/admin/overview", None).await.json();
    let scanner = &seen["files"]["scanner"];
    assert_eq!(scanner["configured"], true, "{seen}");
    assert_eq!(scanner["reachable"], true, "{seen}");
    assert_eq!(scanner["last_result"], "clean", "{seen}");
    assert!(seen["files"]["error"].is_number() && seen["files"]["skipped"].is_number(), "{seen}");

    let before = seen["files"]["error"].as_i64().unwrap();
    let id = ref_id_of(&upload(&f, "e.png", png(10)).await);
    db(move |c| c.execute("UPDATE file_refs SET scan_state = 'error' WHERE id = $1", &[&id]).unwrap());
    drop(fake);
    let down = json_call(&f, "GET", &f.admin, "/api/v1/admin/overview", None).await.json();
    assert_eq!(down["files"]["scanner"]["reachable"], false, "{down}");
    assert_eq!(down["files"]["error"].as_i64().unwrap(), before + 1);
}
