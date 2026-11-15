//! Sensitive content phase 3: an import is read by the detectors. The dry run says, for each column, which detectors matched and in how many
//! rows. A `block` detector makes the matching rows rejected rows. A `flag` detector leaves a finding, and none keeps a value.

mod common;

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use axum::Router;
use http_body_util::BodyExt;
use scaffoldry_engine::{AppManifest, AppTable, AppView, FieldSpec, FieldType, ViewType};
use scaffoldry_server::build_app_with_state;
use scaffoldry_server::jobs::{get_job, JobConfig, JobRunner, JOB_KINDS};
use scaffoldry_server::service::identity::issue_test_token_and_user;
use scaffoldry_server::state::{ServerState, SharedState};
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tower::ServiceExt;
use uuid::Uuid;

const USER: &str = "prof.curie@science.state.edu";
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

fn fast() -> JobConfig {
    JobConfig { workers: 2, poll_ms: 20, backoff_secs: [0, 0, 0], ..JobConfig::default() }
}

fn wait_for_job(repo: &scaffoldry_server::repository::PostgresRepository, id: Uuid) -> scaffoldry_server::jobs::Job {
    let deadline = Instant::now() + Duration::from_secs(120);
    loop {
        let job = get_job(repo, id).expect("query").expect("job exists");
        if job.state == "done" || job.state == "failed" {
            if job.state == "failed" {
                eprintln!("JOB FAILED: kind={} error={:?} log={:?}", job.kind, job.error, job.log);
            }
            return job;
        }
        assert!(Instant::now() < deadline, "job stuck in {}", job.state);
        std::thread::sleep(Duration::from_millis(50));
    }
}

struct Fixture {
    app: Router,
    state: SharedState,
    slug: String,
    _dir: PathBuf,
    token: String,
    _db: common::TestDb,
}

async fn fixture() -> Fixture {
    let db = common::TestDb::new();
    let dir = std::env::temp_dir().join(format!("scaffoldry-export-files-{}", Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    std::env::set_var("SCAFFOLDRY_FILES_DIR", &dir);

    let state = Arc::new(ServerState::new().expect("state"));
    state.seed_demo().expect("seed");
    let slug = format!("exp-{}", &Uuid::new_v4().to_string()[..8]);

    let fields = vec![
        FieldSpec::simple("title", "Title", FieldType::Text, false, false),
        FieldSpec::simple("notes", "Notes", FieldType::LongText, false, false),
    ];

    let manifest = AppManifest {
        slug: slug.clone(),
        title: "Intake".to_string(),
        description: String::new(),
        organization_code: "SCI".to_string(),
        department: "science".to_string(),
        workspace_id: Some(WORKSPACE.to_string()),
        herm_capability_id: None,
        coordinates: None, sealed: false, integrations: Default::default(), kind: Default::default(),
        custom_domain: None,
        custom_domain_verified: false,
        views: vec![AppView::table("main", "Main", ViewType::Table, fields.clone())],
        pages: vec![],
        access: Default::default(),
        ceds_mappings: Default::default(),
        tables: vec![
            AppTable {
                retired_fields: Vec::new(),
                id: "items".to_string(),
                name: "Items".to_string(),
                slug: "items".to_string(),
                description: None,
                icon: None,
                fields,
                primary_field: Some("title".to_string()),
                sample_records: vec![],
                coordinates: None, sealed: false,
                row_place: Default::default(),
            },
        ],
        relationships: vec![],
    };
    state.repository.upsert_app_manifest(&manifest).unwrap();
    state.engine.write().unwrap().register_manifest(manifest).unwrap();

    Fixture {
        app: build_app_with_state(state.clone()).expect("router"),
        state,
        slug,
        _dir: dir,
        token: issue_test_token_and_user(USER),
        _db: db,
    }
}


const SSN: &str = "123-45-6789";
const SSN_B: &str = "234-56-7890";

fn set_detector_action(f: &Fixture, id: &str, action: &str) {
    let settings = f.state.repository.get_platform_settings().unwrap();
    let (_, defaults) = scaffoldry_engine::sensitivity::default_settings();
    let mut list: Value = settings.get("sensitivity.detectors").cloned().unwrap_or_else(|| serde_json::to_value(defaults).unwrap());
    for d in list.as_array_mut().unwrap() {
        if d["id"] == id {
            d["action"] = json!(action);
        }
    }
    let (id, action) = (id.to_string(), action.to_string());
    let _ = (id, action);
    db(move |c| {
        c.execute(
            "INSERT INTO platform_settings (key, value, updated_by) VALUES ('sensitivity.detectors', $1, 'test') ON CONFLICT (key) DO UPDATE SET value = $1",
            &[&list],
        )
        .unwrap();
    });
}

/// Uploads a CSV, waits for the analysis, and asks for a dry run. Returns the import id and the dry run's result.
async fn dry_run(f: &Fixture, csv: &str) -> (Uuid, Value) {
    let req = Request::post(format!("/api/v1/apps/{}/imports", f.slug))
        .header(header::AUTHORIZATION, format!("Bearer {}", f.token))
        .header(header::CONTENT_LENGTH, csv.len().to_string())
        .header("x-file-name", "people.csv")
        .body(Body::from(csv.to_string()))
        .unwrap();
    let resp = f.app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);
    let v: Value = serde_json::from_slice(&resp.into_body().collect().await.unwrap().to_bytes()).unwrap();
    let import_id: Uuid = v["import_id"].as_str().unwrap().parse().unwrap();
    let analyze: Uuid = v["job_id"].as_str().unwrap().parse().unwrap();
    let runner = JobRunner::start(&f.state.repository, JOB_KINDS, fast()).unwrap();
    assert_eq!(wait_for_job(&f.state.repository, analyze).state, "done");
    let req = Request::post(format!("/api/v1/apps/{}/imports/{}/dry-run", f.slug, import_id))
        .header(header::AUTHORIZATION, format!("Bearer {}", f.token))
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(json!({ "target_table": "items", "fields": { "title": "title", "notes": "notes" } }).to_string()))
        .unwrap();
    let resp = f.app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let v: Value = serde_json::from_slice(&resp.into_body().collect().await.unwrap().to_bytes()).unwrap();
    let job: Uuid = v["job_id"].as_str().unwrap().parse().unwrap();
    assert_eq!(wait_for_job(&f.state.repository, job).state, "done");
    runner.stop();
    let imp = f.state.repository.get_import(&f.slug, import_id).unwrap().unwrap();
    (import_id, imp.dry_run_result.unwrap())
}

async fn run_import(f: &Fixture, import_id: Uuid) -> Value {
    let runner = JobRunner::start(&f.state.repository, JOB_KINDS, fast()).unwrap();
    let req = Request::post(format!("/api/v1/apps/{}/imports/{}/run?accept_errors=true", f.slug, import_id))
        .header(header::AUTHORIZATION, format!("Bearer {}", f.token))
        .body(Body::empty())
        .unwrap();
    let resp = f.app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let v: Value = serde_json::from_slice(&resp.into_body().collect().await.unwrap().to_bytes()).unwrap_or(Value::Null);
    assert_eq!(status, StatusCode::OK, "{v}");
    let job: Uuid = v["job_id"].as_str().unwrap().parse().unwrap();
    let done = wait_for_job(&f.state.repository, job);
    runner.stop();
    assert_eq!(done.state, "done", "{:?}", done.error);
    done.result.unwrap_or(Value::Null)
}

fn csv() -> String {
    format!("title,notes\nA,call about {SSN}\nB,nothing here\nC,\"{SSN_B} and {SSN}\"\nD,fine\n")
}

fn items(f: &Fixture) -> i64 {
    let slug = f.slug.clone();
    db(move |c| c.query_one("SELECT count(*) FROM dataset_records WHERE app_slug = $1 AND table_id = 'items'", &[&slug]).unwrap().get(0))
}

fn findings(f: &Fixture) -> Vec<(String, String, i64, i64)> {
    let slug = f.slug.clone();
    db(move |c| c.query("SELECT source, field, match_count, record_count FROM content_findings WHERE app_slug = $1", &[&slug]).unwrap().iter().map(|r| (r.get(0), r.get(1), r.get(2), r.get(3))).collect())
}

// 5. An import dry run lists the matching columns and counts. A `block` detector produces rejected rows.
#[tokio::test]
async fn an_import_dry_run_reports_the_matching_columns_and_a_block_detector_rejects_rows() {
    let f = fixture().await;
    let (id, dry) = dry_run(&f, &csv()).await;
    let report = dry["sensitive_content"].as_array().unwrap_or_else(|| panic!("no report in {dry}"));
    assert_eq!(report.len(), 1, "one column matched: {dry}");
    assert_eq!((report[0]["column"].as_str(), report[0]["detector"].as_str(), report[0]["rows"].as_i64(), report[0]["action"].as_str()), (Some("notes"), Some("us_ssn"), Some(2), Some("flag")), "{dry}");
    assert_eq!(dry["valid_count"], 4, "a flag detector rejects nothing: {dry}");
    assert!(!dry.to_string().contains(SSN) && !dry.to_string().contains(SSN_B), "the report holds no value: {dry}");

    // Under `flag`, the run imports every row and leaves one finding with the source `import`.
    let _ = run_import(&f, id).await;
    assert_eq!(items(&f), 4);
    let found = findings(&f);
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!((found[0].0.as_str(), found[0].1.as_str(), found[0].2, found[0].3), ("import", "notes", 3, 2), "three matches in two rows");

    // Under `block`, the matching rows are rejected rows, in the dry run and in the run.
    set_detector_action(&f, "us_ssn", "block");
    let (id, dry) = dry_run(&f, &csv()).await;
    assert_eq!((dry["valid_count"].as_i64(), dry["invalid_count"].as_i64()), (Some(2), Some(2)), "{dry}");
    let message = dry["errors"][0]["message"].as_str().unwrap();
    assert!(message.contains("notes") && message.contains("us_ssn") && !message.contains("123"), "{message}");
    let done = run_import(&f, id).await;
    assert_eq!(done["rejected_count"], 2, "{done}");
    assert_eq!(items(&f), 4 + 2, "only the clean rows were added");
}
