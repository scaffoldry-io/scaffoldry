//! Sensitive content phase 3: a plain-text file a record holds is read for sensitive content once. Other files are said to be unread.

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::Router;
use http_body_util::BodyExt;
use scaffoldry_engine::{AppManifest, AppTable, AppView, FieldSpec, FieldType, ViewType};
use scaffoldry_server::build_app_with_state;
use scaffoldry_server::jobs::{JobConfig, JobRunner, JobState, JOB_KINDS};
use scaffoldry_server::repository::PostgresRepository;
use scaffoldry_server::service::identity::issue_test_token_and_user;
use scaffoldry_server::state::{ServerState, SharedState};
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::Arc;
use tower::ServiceExt;
use uuid::Uuid;

const OWNER: &str = "prof.curie@science.state.edu";
const WORKSPACE: &str = "ws-bio-lab";
const SSN: &str = "123-45-6789";

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
    _dir: PathBuf,
    owner: String,
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
            coordinates: None, sealed: false, integrations: Default::default(), kind: Default::default(),
            custom_domain: None,
            custom_domain_verified: false,
            views: vec![AppView::table("main", "Main", ViewType::Table, fields())],
            pages: vec![],
            access: Default::default(),
            ceds_mappings: Default::default(),
            tables: vec![AppTable {
                retired_fields: Vec::new(), key: None, unit_field: None,
                id: "t1".to_string(),
                name: "Things".to_string(),
                slug: "things".to_string(),
                description: None,
                icon: None,
                fields: fields(),
                primary_field: Some("title".to_string()),
                sample_records: vec![],
                coordinates: None, sealed: false,
                row_place: Default::default(),
            }],
            relationships: vec![],
        })
        .unwrap();
    Fixture {
        app: build_app_with_state(state.clone()).expect("router"),
        state,
        slug,
        _dir: dir,
        owner: issue_test_token_and_user(OWNER),
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
}

async fn call(app: &Router, req: Request<Body>) -> Reply {
    let resp = app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes().to_vec();
    Reply { status, bytes }
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


fn png() -> Vec<u8> {
    let mut b = vec![0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];
    b.extend(b"\x00\x00\x00\x0dIHDR".iter());
    b.extend(std::iter::repeat_n(7u8, 200));
    b
}

/// Runs the queued jobs until none is left.
fn drain(f: &Fixture) {
    let repo = PostgresRepository::connect(None).expect("database");
    let cfg = JobConfig { workers: 2, poll_ms: 20, backoff_secs: [0, 0, 0], state: Some(JobState(f.state.clone())), ..JobConfig::default() };
    let runner = JobRunner::start(&repo, JOB_KINDS, cfg).unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
    loop {
        let open: i64 = db(|c| c.query_one("SELECT count(*) FROM jobs WHERE kind = 'scan_file_content' AND state NOT IN ('done','failed')", &[]).unwrap().get(0));
        if open == 0 || std::time::Instant::now() > deadline {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    runner.stop();
}

fn state_of(ref_id: &str) -> String {
    let id: Uuid = ref_id.parse().unwrap();
    db(move |c| c.query_one("SELECT content_scan_state FROM file_refs WHERE id = $1", &[&id]).unwrap().get(0))
}

// A csv with a social security number is read once and leaves a finding with a count and no value. A png is `unsupported`. A file no record holds is not read.
#[tokio::test]
async fn a_plain_text_file_is_read_once_and_a_picture_is_said_to_be_unread() {
    let _db = common::TestDb::new();
    let f = fixture().await;
    let csv = upload(&f, &f.owner, "people.csv", "text/csv", format!("name,id\nAda,{SSN}\n").into_bytes()).await;
    let pic = upload(&f, &f.owner, "pic.png", "image/png", png()).await;
    let loose = upload(&f, &f.owner, "loose.csv", "text/csv", format!("id\n{SSN}\n").into_bytes()).await;
    let (c, p, l) = (csv.json()["ref_id"].as_str().unwrap().to_string(), pic.json()["ref_id"].as_str().unwrap().to_string(), loose.json()["ref_id"].as_str().unwrap().to_string());
    let made = create(&f, &f.owner, json!({ "title": "t", "doc": [{ "ref_id": c, "name": "people.csv" }, { "ref_id": p, "name": "pic.png" }] })).await;
    assert_eq!(made.status, StatusCode::CREATED, "{}", made.json());
    drain(&f);
    assert_eq!(state_of(&c), "found");
    assert_eq!(state_of(&p), "unsupported", "a picture is never reported as clean");
    assert_eq!(state_of(&l), "skipped", "a file no record holds is not read");
    let slug = f.slug.clone();
    let rows: Vec<(String, String, i64, String)> = db(move |c| {
        c.query("SELECT field, detector_id, match_count, source FROM content_findings WHERE app_slug = $1", &[&slug]).unwrap().iter().map(|r| (r.get(0), r.get(1), r.get(2), r.get(3))).collect()
    });
    assert_eq!(rows.len(), 1, "{rows:?}");
    assert_eq!((rows[0].0.as_str(), rows[0].1.as_str(), rows[0].2, rows[0].3.as_str()), ("doc", "us_ssn", 1, "file"));
    let stored: String = db(|c| c.query_one("SELECT coalesce(string_agg(row_to_json(f)::text, ' '), '') FROM content_findings f", &[]).unwrap().get(0));
    assert!(!stored.contains(SSN), "no value is kept");
    // The record lists the state, so the screen can say what was not read.
    let id = made.json()["id"].as_str().unwrap().to_string();
    let listed = get(&f, &f.owner, &format!("/api/v1/apps/{}/records/{id}/files", f.slug)).await.json();
    let states: Vec<(String, String)> = listed["files"].as_array().unwrap().iter().map(|x| (x["name"].as_str().unwrap().to_string(), x["content_scan_state"].as_str().unwrap().to_string())).collect();
    assert!(states.contains(&("people.csv".to_string(), "found".to_string())) && states.contains(&("pic.png".to_string(), "unsupported".to_string())), "{states:?}");
    let slug = f.slug.clone();
    db(move |c| {
        let _ = c.execute("DELETE FROM content_findings WHERE app_slug = $1", &[&slug]);
    });
}
