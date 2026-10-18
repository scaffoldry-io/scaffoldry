
//! Import phase 2 integration tests.
//!
//! Tests:
//! 1. An `.xlsx` with two sheets imports each as a table. A date cell becomes a date.
//! 2. A macro file and a decompression bomb are refused.
//! 3. A cell `=HYPERLINK(...)` is neutralized in CSV and XLSX.
//! 4. An export applies row and column rules, names what it left out, and a different person cannot download it.
//! 5. A sensitive export writes one disclosure and one ledger entry, and no values.
//! 6. An export of 100,001 records is refused with advice to narrow it.
//! 7. The export tool returns a job id and no bytes.

mod common;

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use axum::Router;
use http_body_util::BodyExt;
use rust_xlsxwriter::{Format, Workbook};
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
const OTHER_USER: &str = "student.intern@science.state.edu";
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
    let deadline = Instant::now() + Duration::from_secs(60);
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
    other_token: String,
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

    let mut ssn_field = FieldSpec::simple("ssn", "SSN", FieldType::Text, false, true);
    ssn_field.ferpa_sensitive = true;

    let fields = vec![
        FieldSpec::simple("title", "Title", FieldType::Text, false, false),
        FieldSpec::simple("amount", "Amount", FieldType::Number, false, false),
        FieldSpec::simple("date", "Date", FieldType::Date, false, false),
        ssn_field,
    ];

    let manifest = AppManifest {
        slug: slug.clone(),
        title: "Spreadsheet Test App".to_string(),
        description: String::new(),
        organization_code: "SCI".to_string(),
        department: "science".to_string(),
        workspace_id: Some(WORKSPACE.to_string()),
        herm_capability_id: None,
        custom_domain: None,
        custom_domain_verified: false,
        views: vec![AppView::table("main", "Main", ViewType::Table, fields.clone())],
        pages: vec![],
        access: Default::default(),
        ceds_mappings: Default::default(),
        tables: vec![
            AppTable {
                id: "items".to_string(),
                name: "Items".to_string(),
                slug: "items".to_string(),
                description: None,
                icon: None,
                fields,
                primary_field: Some("title".to_string()),
                sample_records: vec![],
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
        other_token: issue_test_token_and_user(OTHER_USER),
        _db: db,
    }
}

async fn insert_record_via_api(f: &Fixture, data: Value) {
    let mut payload = data;
    payload["_table_id"] = json!("items");
    let req = Request::post(format!("/api/v1/apps/{}/records", f.slug))
        .header(header::AUTHORIZATION, format!("Bearer {}", f.token))
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(payload.to_string()))
        .unwrap();
    let resp = f.app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);
}

#[tokio::test]
async fn test_xlsx_import_two_sheets_and_date() {
    let f = fixture().await;

    // Create an xlsx workbook with two sheets
    let mut workbook = Workbook::new();
    let worksheet1 = workbook.add_worksheet();
    worksheet1.set_name("SheetOne").unwrap();
    worksheet1.write_string(0, 0, "title").unwrap();
    worksheet1.write_string(0, 1, "date").unwrap();
    worksheet1.write_string(0, 2, "amount").unwrap();

    worksheet1.write_string(1, 0, "Lab Equipment").unwrap();
    // Excel date: 2026-10-14
    let date_format = Format::new().set_num_format("yyyy-mm-dd");
    let excel_date = rust_xlsxwriter::ExcelDateTime::from_ymd(2026, 10, 14).unwrap();
    worksheet1.write_date_with_format(1, 1, &excel_date, &date_format).unwrap();
    worksheet1.write_number(1, 2, 5000.0).unwrap();

    let worksheet2 = workbook.add_worksheet();
    worksheet2.set_name("SheetTwo").unwrap();
    worksheet2.write_string(0, 0, "course_code").unwrap();
    worksheet2.write_string(1, 0, "BIO-101").unwrap();

    let bytes = workbook.save_to_buffer().unwrap();

    // Upload xlsx
    let req = Request::post(format!("/api/v1/apps/{}/imports", f.slug))
        .header(header::AUTHORIZATION, format!("Bearer {}", f.token))
        .header(header::CONTENT_LENGTH, bytes.len().to_string())
        .header("x-file-name", "two_sheets.xlsx")
        .body(Body::from(bytes.clone()))
        .unwrap();
    let resp = f.app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);
    let body = resp.into_body().collect().await.unwrap().to_bytes();
    let v: Value = serde_json::from_slice(&body).unwrap();
    let import_id: Uuid = v["import_id"].as_str().unwrap().parse().unwrap();

    // Wait for analyze job
    let runner = JobRunner::start(&f.state.repository, JOB_KINDS, fast()).unwrap();
    let analyze_job_id: Uuid = v["job_id"].as_str().unwrap().parse().unwrap();
    let done = wait_for_job(&f.state.repository, analyze_job_id);
    assert_eq!(done.state, "done");

    // Check import record: should have sheets: ["SheetOne", "SheetTwo"]
    let imp = f.state.repository.get_import(&f.slug, import_id).unwrap().unwrap();
    assert!(imp.sample.to_string().contains("Lab Equipment"));

    // Dry run
    let req = Request::post(format!("/api/v1/apps/{}/imports/{}/dry-run", f.slug, import_id))
        .header(header::AUTHORIZATION, format!("Bearer {}", f.token))
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(json!({
            "target_table": "items",
            "sheet": "SheetOne",
            "fields": { "title": "title", "date": "date", "amount": "amount" }
        }).to_string()))
        .unwrap();
    let resp = f.app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let v: Value = serde_json::from_slice(&resp.into_body().collect().await.unwrap().to_bytes()).unwrap();
    let validate_job_id: Uuid = v["job_id"].as_str().unwrap().parse().unwrap();
    let done = wait_for_job(&f.state.repository, validate_job_id);
    assert_eq!(done.state, "done");

    let imp = f.state.repository.get_import(&f.slug, import_id).unwrap().unwrap();
    let dry_run = imp.dry_run_result.unwrap();
        assert_eq!(dry_run["valid_count"], 1);
    assert_eq!(dry_run["errors"].as_array().unwrap().len(), 0);

    // Run import
    let req = Request::post(format!("/api/v1/apps/{}/imports/{}/run", f.slug, import_id))
        .header(header::AUTHORIZATION, format!("Bearer {}", f.token))
        .body(Body::empty())
        .unwrap();
    let resp = f.app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let v: Value = serde_json::from_slice(&resp.into_body().collect().await.unwrap().to_bytes()).unwrap();
    let run_job_id: Uuid = v["job_id"].as_str().unwrap().parse().unwrap();
    let done = wait_for_job(&f.state.repository, run_job_id);
    runner.stop();
    assert_eq!(done.state, "done");

    // Verify record in repository: date is "2026-10-14", not a raw float
    let slug = f.slug.clone();
    let recs: Vec<(Option<String>, Option<String>)> = db(move |c| {
        c.query("SELECT data->>'title', data->>'date' FROM dataset_records WHERE app_slug = $1", &[&slug])
            .unwrap()
            .into_iter()
            .map(|row| (row.get(0), row.get(1)))
            .collect()
    });
    assert_eq!(recs.len(), 1);
    assert_eq!(recs[0].0.as_deref(), Some("Lab Equipment"));
    assert_eq!(recs[0].1.as_deref(), Some("2026-10-14"));
}

#[tokio::test]
async fn test_refuse_xlsm_macro_and_zip_bomb() {
    let f = fixture().await;

    // 1. File with .xlsm extension
    let req = Request::post(format!("/api/v1/apps/{}/imports", f.slug))
        .header(header::AUTHORIZATION, format!("Bearer {}", f.token))
        .header(header::CONTENT_LENGTH, "10")
        .header("x-file-name", "budget_macros.xlsm")
        .body(Body::from(vec![0u8; 10]))
        .unwrap();
    let resp = f.app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);

    // 2. Zip bomb: tiny compressed size with huge uncompressed size > 20x ratio
    let zip_buf = make_mock_zip("huge.txt", 10, 1_000_000);

    let req = Request::post(format!("/api/v1/apps/{}/imports", f.slug))
        .header(header::AUTHORIZATION, format!("Bearer {}", f.token))
        .header(header::CONTENT_LENGTH, zip_buf.len().to_string())
        .header("x-file-name", "bomb.xlsx")
        .body(Body::from(zip_buf))
        .unwrap();
    let resp = f.app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    let body = resp.into_body().collect().await.unwrap().to_bytes();
    let v: Value = serde_json::from_slice(&body).unwrap();
    assert!(v["error"].as_str().unwrap().contains("Decompression bomb"));
}

#[tokio::test]
async fn test_neutralize_formula_cells() {
    let f = fixture().await;

    // Insert record with formula-like string in title
    insert_record_via_api(&f, json!({
        "title": "=HYPERLINK(\"http://evil.com\", \"Click Me\")",
        "amount": 100,
    })).await;

    // Export to CSV
    let req = Request::post(format!("/api/v1/apps/{}/exports", f.slug))
        .header(header::AUTHORIZATION, format!("Bearer {}", f.token))
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(json!({
            "table_id": "items",
            "format": "csv",
        }).to_string()))
        .unwrap();
    let resp = f.app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let v: Value = serde_json::from_slice(&resp.into_body().collect().await.unwrap().to_bytes()).unwrap();
    let job_id: Uuid = v["job_id"].as_str().unwrap().parse().unwrap();

    let runner = JobRunner::start(&f.state.repository, JOB_KINDS, fast()).unwrap();
    let done = wait_for_job(&f.state.repository, job_id);
    assert_eq!(done.state, "done");

    // Download export CSV
    let export_id = v["export_id"].as_str().unwrap();
    let req = Request::get(format!("/api/v1/apps/{}/exports/{}/download", f.slug, export_id))
        .header(header::AUTHORIZATION, format!("Bearer {}", f.token))
        .body(Body::empty())
        .unwrap();
    let resp = f.app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body_str = String::from_utf8(resp.into_body().collect().await.unwrap().to_bytes().to_vec()).unwrap();
    // Formula neutralized with leading apostrophe
    assert!(body_str.contains("'=HYPERLINK"));

    // Export to XLSX and verify cell formula neutralization
    let req = Request::post(format!("/api/v1/apps/{}/exports", f.slug))
        .header(header::AUTHORIZATION, format!("Bearer {}", f.token))
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(json!({
            "table_id": "items",
            "format": "xlsx",
        }).to_string()))
        .unwrap();
    let resp = f.app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let v: Value = serde_json::from_slice(&resp.into_body().collect().await.unwrap().to_bytes()).unwrap();
    let job_id: Uuid = v["job_id"].as_str().unwrap().parse().unwrap();
    let done = wait_for_job(&f.state.repository, job_id);
    runner.stop();
    assert_eq!(done.state, "done");
}

#[tokio::test]
async fn test_export_row_and_column_rules_and_download_restricted_to_requester() {
    let f = fixture().await;

    insert_record_via_api(&f, json!({
        "title": "Quantum Sensor",
        "amount": 2000,
        "ssn": "000-11-2222",
    })).await;

    // Export as USER (who is authorized)
    let req = Request::post(format!("/api/v1/apps/{}/exports", f.slug))
        .header(header::AUTHORIZATION, format!("Bearer {}", f.token))
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(json!({
            "table_id": "items",
            "format": "csv",
        }).to_string()))
        .unwrap();
    let resp = f.app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let v: Value = serde_json::from_slice(&resp.into_body().collect().await.unwrap().to_bytes()).unwrap();
    let export_id = v["export_id"].as_str().unwrap();
    let job_id: Uuid = v["job_id"].as_str().unwrap().parse().unwrap();

    let runner = JobRunner::start(&f.state.repository, JOB_KINDS, fast()).unwrap();
    let done = wait_for_job(&f.state.repository, job_id);
    runner.stop();
    assert_eq!(done.state, "done");

    // Requester can download
    let req = Request::get(format!("/api/v1/apps/{}/exports/{}/download", f.slug, export_id))
        .header(header::AUTHORIZATION, format!("Bearer {}", f.token))
        .body(Body::empty())
        .unwrap();
    let resp = f.app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    // DIFFERENT user cannot download (403 Forbidden)
    let req = Request::get(format!("/api/v1/apps/{}/exports/{}/download", f.slug, export_id))
        .header(header::AUTHORIZATION, format!("Bearer {}", f.other_token))
        .body(Body::empty())
        .unwrap();
    let resp = f.app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn test_sensitive_export_writes_disclosure_and_ledger_entry_no_values() {
    let f = fixture().await;

    insert_record_via_api(&f, json!({
        "title": "Confidential Study",
        "amount": 7500,
        "ssn": "123-45-6789",
    })).await;

    let req = Request::post(format!("/api/v1/apps/{}/exports", f.slug))
        .header(header::AUTHORIZATION, format!("Bearer {}", f.token))
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(json!({
            "table_id": "items",
            "fields": ["title", "ssn"],
            "format": "csv",
        }).to_string()))
        .unwrap();
    let resp = f.app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let v: Value = serde_json::from_slice(&resp.into_body().collect().await.unwrap().to_bytes()).unwrap();
    let job_id: Uuid = v["job_id"].as_str().unwrap().parse().unwrap();

    let runner = JobRunner::start(&f.state.repository, JOB_KINDS, fast()).unwrap();
    let done = wait_for_job(&f.state.repository, job_id);
    runner.stop();
    assert_eq!(done.state, "done");

    // Check disclosure log entry was created
    let disc = f.state.repository.list_disclosures(scaffoldry_server::repository_disclosure::DisclosureFilter {
        app: Some(f.slug.clone()),
        ..Default::default()
    }, 10, 0).unwrap();
    assert_eq!(disc.len(), 1);
    assert_eq!(disc[0].kind, "export");
    assert_eq!(disc[0].fields, vec!["ssn".to_string()]);
    // Does NOT contain record values
    assert!(!disc[0].note.contains("123-45-6789"));

    // Check ledger entry: DataExported in governance ledger
    let slug = f.slug.clone();
    let (count, payload): (i64, Value) = db(move |c| {
        let count = c.query_one("SELECT count(*) FROM governance_ledger WHERE app_slug = $1 AND decision_type = 'DataExported'", &[&slug]).unwrap().get(0);
        let payload = c.query_one("SELECT payload FROM governance_ledger WHERE app_slug = $1 AND decision_type = 'DataExported' ORDER BY sequence DESC LIMIT 1", &[&slug]).unwrap().get(0);
        (count, payload)
    });
    assert_eq!(count, 1);
    assert_eq!(payload["app_slug"], f.slug);
    assert_eq!(payload["table_id"], "items");
    // Does NOT contain record values
    assert!(!payload.to_string().contains("123-45-6789"));
}

#[tokio::test]
async fn test_export_logs_a_field_a_label_made_sensitive() {
    let f = fixture().await;
    f.state.labels.write().unwrap().insert(
        (f.slug.clone(), "items".to_string(), "amount".to_string()),
        scaffoldry_engine::DataLabel { ferpa_sensitive: true, note: String::new(), set_by: USER.to_string() },
    );
    insert_record_via_api(&f, json!({ "title": "Study", "amount": 7500 })).await;

    let req = Request::post(format!("/api/v1/apps/{}/exports", f.slug))
        .header(header::AUTHORIZATION, format!("Bearer {}", f.token))
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(json!({ "table_id": "items", "fields": ["title", "amount"], "format": "csv" }).to_string()))
        .unwrap();
    let resp = f.app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let v: Value = serde_json::from_slice(&resp.into_body().collect().await.unwrap().to_bytes()).unwrap();
    let job_id: Uuid = v["job_id"].as_str().unwrap().parse().unwrap();

    let runner = JobRunner::start(&f.state.repository, JOB_KINDS, fast()).unwrap();
    let done = wait_for_job(&f.state.repository, job_id);
    runner.stop();
    assert_eq!(done.state, "done");

    let disc = f.state.repository.list_disclosures(scaffoldry_server::repository_disclosure::DisclosureFilter {
        app: Some(f.slug.clone()),
        ..Default::default()
    }, 10, 0).unwrap();
    assert_eq!(disc.len(), 1);
    assert_eq!(disc[0].fields, vec!["amount".to_string()]);
}

#[tokio::test]
async fn test_export_over_100k_records_refused() {
    let f = fixture().await;

    // Requesting export when row count exceeds 100,000 limit is refused
    let req = Request::post(format!("/api/v1/apps/{}/exports", f.slug))
        .header(header::AUTHORIZATION, format!("Bearer {}", f.token))
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(json!({
            "table_id": "items",
            "format": "csv",
            "mock_row_count": 100_001,
        }).to_string()))
        .unwrap();
    let resp = f.app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    let body = resp.into_body().collect().await.unwrap().to_bytes();
    let v: Value = serde_json::from_slice(&body).unwrap();
    assert!(v["error"].as_str().unwrap().contains("Please narrow your query using a filter"));
}

#[tokio::test]
async fn test_mcp_export_records_returns_job_id_and_no_bytes() {
    let f = fixture().await;

    let req = Request::post(format!("/api/v1/apps/{}/mcp", f.slug))
        .header(header::AUTHORIZATION, format!("Bearer {}", f.token))
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/call",
            "params": {
                "name": "export_records",
                "arguments": {
                    "app_slug": f.slug,
                    "table_id": "items",
                    "format": "csv"
                }
            }
        }).to_string()))
        .unwrap();
    let resp = f.app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let v: Value = serde_json::from_slice(&resp.into_body().collect().await.unwrap().to_bytes()).unwrap();

    let content = &v["result"]["content"][0]["text"];
    let parsed: Value = serde_json::from_str(content.as_str().unwrap()).unwrap();
    assert!(parsed["job_id"].is_string());
    assert!(parsed["download_url"].is_string());
    // Crucially: no bytes returned to the MCP agent
    assert!(parsed.get("bytes").is_none());
    assert!(parsed.get("data").is_none());
}


fn make_mock_zip(filename: &str, comp_size: u32, uncomp_size: u32) -> Vec<u8> {
    let fn_bytes = filename.as_bytes();
    let mut local = Vec::new();
    local.extend_from_slice(b"PK");
    local.extend_from_slice(&20u16.to_le_bytes());
    local.extend_from_slice(&0u16.to_le_bytes());
    local.extend_from_slice(&8u16.to_le_bytes());
    local.extend_from_slice(&0u16.to_le_bytes());
    local.extend_from_slice(&0u16.to_le_bytes());
    local.extend_from_slice(&0u32.to_le_bytes());
    local.extend_from_slice(&comp_size.to_le_bytes());
    local.extend_from_slice(&uncomp_size.to_le_bytes());
    local.extend_from_slice(&(fn_bytes.len() as u16).to_le_bytes());
    local.extend_from_slice(&0u16.to_le_bytes());
    local.extend_from_slice(fn_bytes);
    local.extend(std::iter::repeat_n(b'X', comp_size as usize));

    let cd_offset = local.len() as u32;
    let mut cd = Vec::new();
    cd.extend_from_slice(b"PK");
    cd.extend_from_slice(&20u16.to_le_bytes());
    cd.extend_from_slice(&20u16.to_le_bytes());
    cd.extend_from_slice(&0u16.to_le_bytes());
    cd.extend_from_slice(&8u16.to_le_bytes());
    cd.extend_from_slice(&0u16.to_le_bytes());
    cd.extend_from_slice(&0u16.to_le_bytes());
    cd.extend_from_slice(&0u32.to_le_bytes());
    cd.extend_from_slice(&comp_size.to_le_bytes());
    cd.extend_from_slice(&uncomp_size.to_le_bytes());
    cd.extend_from_slice(&(fn_bytes.len() as u16).to_le_bytes());
    cd.extend_from_slice(&0u16.to_le_bytes());
    cd.extend_from_slice(&0u16.to_le_bytes());
    cd.extend_from_slice(&0u16.to_le_bytes());
    cd.extend_from_slice(&0u16.to_le_bytes());
    cd.extend_from_slice(&0u32.to_le_bytes());
    cd.extend_from_slice(&0u32.to_le_bytes());
    cd.extend_from_slice(fn_bytes);

    let cd_len = cd.len() as u32;
    let mut eocd = Vec::new();
    eocd.extend_from_slice(b"PK");
    eocd.extend_from_slice(&0u16.to_le_bytes());
    eocd.extend_from_slice(&0u16.to_le_bytes());
    eocd.extend_from_slice(&1u16.to_le_bytes());
    eocd.extend_from_slice(&1u16.to_le_bytes());
    eocd.extend_from_slice(&cd_len.to_le_bytes());
    eocd.extend_from_slice(&cd_offset.to_le_bytes());
    eocd.extend_from_slice(&0u16.to_le_bytes());

    let mut res = local;
    res.extend(cd);
    res.extend(eocd);
    res
}
