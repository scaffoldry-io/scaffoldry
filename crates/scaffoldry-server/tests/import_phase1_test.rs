//! Import phase 1 integration tests.
//!
//! Tests:
//! 1. A file of 10,000 rows with 30 bad ones: the dry run reports 30 errors with row numbers and sentences, and writes nothing.
//! 2. Run writes 9,970 records, each in history as an `import` batch, and the rejected file holds the 30 rows with reasons.
//! 3. An `upsert_key` updates matching records and creates the others.
//! 4. A link value that matches two targets is an error and is never guessed.
//! 5. An unknown select option is rejected by default, blank by mapping, and added to the proposal by `propose_option`.
//! 6. Kill the job mid-run: it resumes from its checkpoint and no row is written twice.
//! 7. A file over the row limit is refused before the job starts.
//! 9. An import by a person with row rules cannot create a row outside their `create` scope: those rows are rejected with a reason.

mod common;

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use axum::Router;
use http_body_util::BodyExt;
use scaffoldry_engine::{AppManifest, AppTable, AppView, FieldSpec, FieldType, SelectOption, SelectOptionColor, ViewType};
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
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        let job = get_job(repo, id).expect("query").expect("job exists");
        if job.state == "done" || job.state == "failed" {
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
}

async fn fixture() -> Fixture {
    let dir = std::env::temp_dir().join(format!("scaffoldry-import-files-{}", Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    std::env::set_var("SCAFFOLDRY_FILES_DIR", &dir);

    let state = Arc::new(ServerState::new().expect("state"));
    state.seed_demo().expect("seed");
    let slug = format!("imp-{}", &Uuid::new_v4().to_string()[..8]);

    let select_opts = vec![
        SelectOption { id: "opt-1".to_string(), label: "Active".to_string(), color: SelectOptionColor::Ok, order: 0 },
        SelectOption { id: "opt-2".to_string(), label: "Pending".to_string(), color: SelectOptionColor::Warn, order: 1 },
        SelectOption { id: "opt-3".to_string(), label: "Closed".to_string(), color: SelectOptionColor::Neutral, order: 2 },
    ];

    let mut status_field = FieldSpec::simple("status", "Status", FieldType::Select, false, false);
    status_field.select_options = select_opts;

    let mut project_field = FieldSpec::simple("project", "Project", FieldType::Link, false, false);
    project_field.target_table_id = Some("projects".to_string());
    project_field.target_display_field = Some("name".to_string());

    let fields = vec![
        FieldSpec::simple("title", "Title", FieldType::Text, false, false),
        FieldSpec::simple("amount", "Amount", FieldType::Number, false, false),
        status_field,
        project_field,
    ];

    let proj_fields = vec![
        FieldSpec::simple("name", "Name", FieldType::Text, false, true),
    ];

    let manifest = AppManifest {
        slug: slug.clone(),
        title: "Import Test App".to_string(),
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
            AppTable {
                id: "projects".to_string(),
                name: "Projects".to_string(),
                slug: "projects".to_string(),
                description: None,
                icon: None,
                fields: proj_fields,
                primary_field: Some("name".to_string()),
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
    }
}

async fn upload_csv(f: &Fixture, name: &str, content: &[u8]) -> Uuid {
    let req = Request::post(format!("/api/v1/apps/{}/imports", f.slug))
        .header(header::AUTHORIZATION, format!("Bearer {}", f.token))
        .header(header::CONTENT_LENGTH, content.len().to_string())
        .header("x-file-name", name)
        .body(Body::from(content.to_vec()))
        .unwrap();

    let res = f.app.clone().oneshot(req).await.unwrap();
    let status = res.status();
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    if status != StatusCode::CREATED {
        panic!("Upload failed: status={}, body={}", status, String::from_utf8_lossy(&bytes));
    }
    let body: Value = serde_json::from_slice(&bytes).unwrap();
    Uuid::parse_str(body["import_id"].as_str().unwrap()).unwrap()
}

#[tokio::test]
async fn test_dry_run_reports_30_errors_and_writes_nothing() {
    let f = fixture().await;
    let repo = &f.state.repository;

    // Generate CSV with 10,000 rows, 30 of which have invalid number in 'amount'
    let mut csv = String::from("title,amount,status\n");
    for i in 1..=10_000 {
        if i <= 30 {
            csv.push_str(&format!("Item {},NOT_A_NUMBER,Active\n", i));
        } else {
            csv.push_str(&format!("Item {},{},Active\n", i, i * 10));
        }
    }

    let import_id = upload_csv(&f, "test_10k.csv", csv.as_bytes()).await;

    // Run dry run
    let req = Request::post(format!("/api/v1/apps/{}/imports/{}/dry-run", f.slug, import_id))
        .header(header::AUTHORIZATION, format!("Bearer {}", f.token))
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(json!({
            "target_table": "items",
            "column_mappings": {
                "title": "title",
                "amount": "amount",
                "status": "status"
            }
        }).to_string()))
        .unwrap();

    let res = f.app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    let body: Value = serde_json::from_slice(&bytes).unwrap();
    let job_id = Uuid::parse_str(body["job_id"].as_str().unwrap()).unwrap();

    let runner = JobRunner::start(repo, JOB_KINDS, fast()).unwrap();
    let job = wait_for_job(repo, job_id);
    runner.stop();

    assert_eq!(job.state, "done", "{:?}", job.error);
    let result = job.result.unwrap();
    assert_eq!(result["invalid_count"], 30);
    assert_eq!(result["valid_count"], 9970);
    assert_eq!(result["errors"].as_array().unwrap().len(), 30);

    // Verify written nothing to dataset_records
    let count: i64 = {
        let slug = f.slug.clone();
        db(move |c| c.query_one("SELECT count(*) FROM dataset_records WHERE app_slug = $1", &[&slug]).unwrap().get(0))
    };
    assert_eq!(count, 0, "Dry run must write nothing");
}

#[tokio::test]
async fn test_run_writes_9970_records_import_history_and_rejected_file() {
    let f = fixture().await;
    let repo = &f.state.repository;

    let mut csv = String::from("title,amount,status\n");
    for i in 1..=10_000 {
        if i <= 30 {
            csv.push_str(&format!("Item {},BAD_NUM,Active\n", i));
        } else {
            csv.push_str(&format!("Item {},{},Active\n", i, i * 5));
        }
    }

    let import_id = upload_csv(&f, "test_run.csv", csv.as_bytes()).await;

    // Run import with accept_errors = true
    let req = Request::post(format!("/api/v1/apps/{}/imports/{}/run?accept_errors=true", f.slug, import_id))
        .header(header::AUTHORIZATION, format!("Bearer {}", f.token))
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(json!({
            "target_table": "items",
            "column_mappings": {
                "title": "title",
                "amount": "amount",
                "status": "status"
            }
        }).to_string()))
        .unwrap();

    let res = f.app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    let body: Value = serde_json::from_slice(&bytes).unwrap();
    let job_id = Uuid::parse_str(body["job_id"].as_str().unwrap()).unwrap();

    let runner = JobRunner::start(repo, JOB_KINDS, fast()).unwrap();
    let job = wait_for_job(repo, job_id);
    runner.stop();

    assert_eq!(job.state, "done", "{:?}", job.error);
    let result = job.result.unwrap();
    assert_eq!(result["created_count"], 9970);
    assert_eq!(result["rejected_count"], 30);
    assert!(result["rejected_file_ref_id"].is_string());

    // Verify 9,970 records written
    let count: i64 = {
        let slug = f.slug.clone();
        db(move |c| c.query_one("SELECT count(*) FROM dataset_records WHERE app_slug = $1", &[&slug]).unwrap().get(0))
    };
    assert_eq!(count, 9970);

    // Verify record_history actor_kind is 'import'
    let history_count: i64 = {
        let slug = f.slug.clone();
        db(move |c| c.query_one("SELECT count(*) FROM record_history WHERE app_slug = $1 AND actor_kind = 'import'", &[&slug]).unwrap().get(0))
    };
    assert_eq!(history_count, 9970);

    // Verify rejected file download
    let req_rej = Request::get(format!("/api/v1/apps/{}/imports/{}/rejected", f.slug, import_id))
        .header(header::AUTHORIZATION, format!("Bearer {}", f.token))
        .body(Body::empty())
        .unwrap();
    let res_rej = f.app.clone().oneshot(req_rej).await.unwrap();
    assert_eq!(res_rej.status(), StatusCode::OK);
    let rej_bytes = res_rej.into_body().collect().await.unwrap().to_bytes();
    let rej_csv = String::from_utf8(rej_bytes.to_vec()).unwrap();
    assert!(rej_csv.contains("_import_error"));
    assert!(rej_csv.contains("BAD_NUM"));
}

#[tokio::test]
async fn test_upsert_key_updates_matching_and_creates_others() {
    let f = fixture().await;
    let repo = &f.state.repository;

    // Seed 2 existing records with titles 'Item 1' and 'Item 2'
    let initial_csv = String::from("title,amount,status\nItem 1,100,Active\nItem 2,200,Active\n");
    let init_import = upload_csv(&f, "init.csv", initial_csv.as_bytes()).await;
    let req_init = Request::post(format!("/api/v1/apps/{}/imports/{}/run", f.slug, init_import))
        .header(header::AUTHORIZATION, format!("Bearer {}", f.token))
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(json!({
            "target_table": "items",
            "column_mappings": { "title": "title", "amount": "amount", "status": "status" }
        }).to_string()))
        .unwrap();
    let res_init = f.app.clone().oneshot(req_init).await.unwrap();
    let init_job_id = Uuid::parse_str(serde_json::from_slice::<Value>(&res_init.into_body().collect().await.unwrap().to_bytes()).unwrap()["job_id"].as_str().unwrap()).unwrap();
    let runner = JobRunner::start(repo, JOB_KINDS, fast()).unwrap();
    let job = wait_for_job(repo, init_job_id);
    runner.stop();
    assert_eq!(job.state, "done");

    // Now import CSV with Item 1 (updated amount 999) and Item 3 (new) using upsert_key = title
    let upsert_csv = "title,amount,status\nItem 1,999,Active\nItem 3,300,Active\n";
    let up_import = upload_csv(&f, "upsert.csv", upsert_csv.as_bytes()).await;

    let req_up = Request::post(format!("/api/v1/apps/{}/imports/{}/run", f.slug, up_import))
        .header(header::AUTHORIZATION, format!("Bearer {}", f.token))
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(json!({
            "target_table": "items",
            "upsert_key": "title",
            "column_mappings": { "title": "title", "amount": "amount", "status": "status" }
        }).to_string()))
        .unwrap();
    let res_up = f.app.clone().oneshot(req_up).await.unwrap();
    let up_job_id = Uuid::parse_str(serde_json::from_slice::<Value>(&res_up.into_body().collect().await.unwrap().to_bytes()).unwrap()["job_id"].as_str().unwrap()).unwrap();
    let runner2 = JobRunner::start(repo, JOB_KINDS, fast()).unwrap();
    let job2 = wait_for_job(repo, up_job_id);
    runner2.stop();

    assert_eq!(job2.state, "done");
    let res = job2.result.unwrap();
    assert_eq!(res["created_count"], 1); // Item 3
    assert_eq!(res["updated_count"], 1); // Item 1

    // Verify Item 1 amount was updated
    let amt: f64 = {
        let slug = f.slug.clone();
        db(move |c| c.query_one("SELECT (data->'amount')::float8 FROM dataset_records WHERE app_slug = $1 AND data->>'title' = 'Item 1'", &[&slug]).unwrap().get(0))
    };
    assert_eq!(amt, 999.0);
}

#[tokio::test]
async fn test_link_value_matching_two_targets_is_error() {
    let f = fixture().await;
    let repo = &f.state.repository;

    // Create two projects with the exact same name "Apollo"
    let p1 = "name\nApollo\n";
    let p2 = "name\nApollo\n";
    let imp1 = upload_csv(&f, "p1.csv", p1.as_bytes()).await;
    let req1 = Request::post(format!("/api/v1/apps/{}/imports/{}/run", f.slug, imp1))
        .header(header::AUTHORIZATION, format!("Bearer {}", f.token))
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(json!({ "target_table": "projects", "column_mappings": { "name": "name" } }).to_string()))
        .unwrap();
    let res1 = f.app.clone().oneshot(req1).await.unwrap();
    let job1 = Uuid::parse_str(serde_json::from_slice::<Value>(&res1.into_body().collect().await.unwrap().to_bytes()).unwrap()["job_id"].as_str().unwrap()).unwrap();

    let imp2 = upload_csv(&f, "p2.csv", p2.as_bytes()).await;
    let req2 = Request::post(format!("/api/v1/apps/{}/imports/{}/run", f.slug, imp2))
        .header(header::AUTHORIZATION, format!("Bearer {}", f.token))
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(json!({ "target_table": "projects", "column_mappings": { "name": "name" } }).to_string()))
        .unwrap();
    let res2 = f.app.clone().oneshot(req2).await.unwrap();
    let job2 = Uuid::parse_str(serde_json::from_slice::<Value>(&res2.into_body().collect().await.unwrap().to_bytes()).unwrap()["job_id"].as_str().unwrap()).unwrap();

    let runner = JobRunner::start(repo, JOB_KINDS, fast()).unwrap();
    wait_for_job(repo, job1);
    wait_for_job(repo, job2);

    // Now dry run an import linking to "Apollo"
    let link_csv = "title,amount,project\nTest Item,100,Apollo\n";
    let imp3 = upload_csv(&f, "link.csv", link_csv.as_bytes()).await;
    let req3 = Request::post(format!("/api/v1/apps/{}/imports/{}/dry-run", f.slug, imp3))
        .header(header::AUTHORIZATION, format!("Bearer {}", f.token))
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(json!({
            "target_table": "items",
            "column_mappings": { "title": "title", "amount": "amount", "project": "project" }
        }).to_string()))
        .unwrap();
    let res3 = f.app.clone().oneshot(req3).await.unwrap();
    let job3 = Uuid::parse_str(serde_json::from_slice::<Value>(&res3.into_body().collect().await.unwrap().to_bytes()).unwrap()["job_id"].as_str().unwrap()).unwrap();

    let job_res = wait_for_job(repo, job3);
    runner.stop();

    assert_eq!(job_res.state, "done");
    let result = job_res.result.unwrap();
    assert_eq!(result["invalid_count"], 1);
    assert_eq!(result["valid_count"], 0);
    let err_msg = result["errors"][0]["message"].as_str().unwrap();
    assert!(err_msg.contains("ambiguous"), "Error must state that link target is ambiguous: {}", err_msg);
}

#[tokio::test]
async fn test_unknown_select_option_strategies() {
    let f = fixture().await;
    let repo = &f.state.repository;

    let csv_content = "title,amount,status\nItem 1,10,UnknownOption\n";
    let imp = upload_csv(&f, "select.csv", csv_content.as_bytes()).await;

    // 1. Default (reject): reports error
    let req1 = Request::post(format!("/api/v1/apps/{}/imports/{}/dry-run", f.slug, imp))
        .header(header::AUTHORIZATION, format!("Bearer {}", f.token))
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(json!({
            "target_table": "items",
            "column_mappings": { "title": "title", "amount": "amount", "status": "status" }
        }).to_string()))
        .unwrap();
    let res1 = f.app.clone().oneshot(req1).await.unwrap();
    let job1 = Uuid::parse_str(serde_json::from_slice::<Value>(&res1.into_body().collect().await.unwrap().to_bytes()).unwrap()["job_id"].as_str().unwrap()).unwrap();

    let runner = JobRunner::start(repo, JOB_KINDS, fast()).unwrap();
    let job_res1 = wait_for_job(repo, job1);
    assert_eq!(job_res1.result.unwrap()["invalid_count"], 1);

    // 2. blank strategy: valid count 1, no errors
    let req2 = Request::post(format!("/api/v1/apps/{}/imports/{}/dry-run", f.slug, imp))
        .header(header::AUTHORIZATION, format!("Bearer {}", f.token))
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(json!({
            "target_table": "items",
            "column_mappings": { "title": "title", "amount": "amount", "status": "status" },
            "select_strategies": { "status": "blank" }
        }).to_string()))
        .unwrap();
    let res2 = f.app.clone().oneshot(req2).await.unwrap();
    let job2 = Uuid::parse_str(serde_json::from_slice::<Value>(&res2.into_body().collect().await.unwrap().to_bytes()).unwrap()["job_id"].as_str().unwrap()).unwrap();
    let job_res2 = wait_for_job(repo, job2);
    assert_eq!(job_res2.result.unwrap()["valid_count"], 1);

    // 3. propose_option: valid count 1, proposed_options contains UnknownOption
    let req3 = Request::post(format!("/api/v1/apps/{}/imports/{}/dry-run", f.slug, imp))
        .header(header::AUTHORIZATION, format!("Bearer {}", f.token))
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(json!({
            "target_table": "items",
            "column_mappings": { "title": "title", "amount": "amount", "status": "status" },
            "select_strategies": { "status": "propose_option" }
        }).to_string()))
        .unwrap();
    let res3 = f.app.clone().oneshot(req3).await.unwrap();
    let job3 = Uuid::parse_str(serde_json::from_slice::<Value>(&res3.into_body().collect().await.unwrap().to_bytes()).unwrap()["job_id"].as_str().unwrap()).unwrap();
    let job_res3 = wait_for_job(repo, job3);
    runner.stop();

    let res3_json = job_res3.result.unwrap();
    assert_eq!(res3_json["valid_count"], 1);
    assert!(res3_json["proposed_options"]["status"].as_array().unwrap().iter().any(|v| v == "UnknownOption"));
}

#[tokio::test]
async fn test_resumable_checkpoint_on_killed_job() {
    let f = fixture().await;
    let repo = &f.state.repository;

    // Generate 2,500 rows
    let mut csv = String::from("title,amount,status\n");
    for i in 1..=2500 {
        csv.push_str(&format!("Resume Item {},{},Active\n", i, i));
    }

    let import_id = upload_csv(&f, "resume.csv", csv.as_bytes()).await;

    // Simulate first batch of 1,000 completed and checkpointed
    repo.update_import_checkpoint(
        import_id,
        &json!({
            "processed_rows": 1000,
            "created_count": 1000,
            "updated_count": 0,
            "rejected_count": 0
        }),
    ).unwrap();

    // Now enqueue run job with pre-existing checkpoint
    let job_id = scaffoldry_server::jobs::enqueue(
        repo,
        "import_run",
        json!({
            "import_id": import_id,
            "app_slug": f.slug,
            "caller_eppn": USER,
            "mapping": {
                "target_table": "items",
                "column_mappings": { "title": "title", "amount": "amount", "status": "status" }
            }
        }),
        USER,
        None,
    ).unwrap();

    // Set job checkpoint directly to resume from row 1000
    db({
        let jid = job_id;
        move |c| {
            c.execute("UPDATE jobs SET checkpoint = $2 WHERE id = $1", &[&jid, &json!({
                "processed_rows": 1000,
                "created_count": 1000,
                "updated_count": 0,
                "rejected_count": 0
            })]).unwrap();
        }
    });

    let runner = JobRunner::start(repo, JOB_KINDS, fast()).unwrap();
    let job = wait_for_job(repo, job_id);
    runner.stop();

    assert_eq!(job.state, "done", "{:?}", job.error);
    let result = job.result.unwrap();
    assert_eq!(result["created_count"], 2500);

    // Verify exactly 1,500 new rows written (total 1,500 since rows 1..1000 were skipped from checkpoint)
    let count: i64 = {
        let slug = f.slug.clone();
        db(move |c| c.query_one("SELECT count(*) FROM dataset_records WHERE app_slug = $1", &[&slug]).unwrap().get(0))
    };
    assert_eq!(count, 1500, "Resumed job only wrote rows 1001..2500");
}


#[tokio::test]
async fn test_file_over_row_limit_refused() {
    let f = fixture().await;

    // Set import.max_rows to 50
    f.state.settings.write().unwrap().insert("import.max_rows".to_string(), json!(50));

    // Create file with 60 rows
    let mut csv = String::from("title,amount,status\n");
    for i in 1..=60 {
        csv.push_str(&format!("Item {},100,Active\n", i));
    }

    let import_id = upload_csv(&f, "too_many.csv", csv.as_bytes()).await;

    // Dry run request must be refused with 400 Bad Request before job starts
    let req = Request::post(format!("/api/v1/apps/{}/imports/{}/dry-run", f.slug, import_id))
        .header(header::AUTHORIZATION, format!("Bearer {}", f.token))
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(json!({
            "target_table": "items",
            "column_mappings": { "title": "title", "amount": "amount", "status": "status" }
        }).to_string()))
        .unwrap();

    let res = f.app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    let body: Value = serde_json::from_slice(&bytes).unwrap();
    assert!(body["error"].as_str().unwrap().contains("File exceeds the maximum row limit"));
}

#[tokio::test]
async fn test_row_scoping_rejects_out_of_scope_rows() {
    let f = fixture().await;
    let repo = &f.state.repository;

    use scaffoldry_core::access::{Op, Operand, Pred, RowOp, RowRule};

    // Add a rule: Create only allowed when status == "Active"
    let rule = RowRule {
        id: Uuid::new_v4().to_string(),
        table_id: "items".to_string(),
        ops: vec![RowOp::Create],
        visible_when: Pred::Cmp {
            field: "status".to_string(),
            op: Op::Eq,
            value: Operand::Literal(Value::String("Active".to_string())),
        },
        except_roles: Vec::new(),
        message: Some("Only active items may be created".to_string()),
    };
    scaffoldry_server::service::scope::set_rules(&f.slug, vec![rule], &f.state);

    let csv = "title,amount,status\nAllowed Item,100,Active\nDisallowed Item,200,Pending\n";
    let import_id = upload_csv(&f, "scoped.csv", csv.as_bytes()).await;

    // Dry run: reports 1 valid and 1 invalid
    let req = Request::post(format!("/api/v1/apps/{}/imports/{}/dry-run", f.slug, import_id))
        .header(header::AUTHORIZATION, format!("Bearer {}", f.token))
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(json!({
            "target_table": "items",
            "column_mappings": { "title": "title", "amount": "amount", "status": "status" }
        }).to_string()))
        .unwrap();

    let res = f.app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    let body: Value = serde_json::from_slice(&bytes).unwrap();
    let job_id = Uuid::parse_str(body["job_id"].as_str().unwrap()).unwrap();

    let runner = JobRunner::start(repo, JOB_KINDS, fast()).unwrap();
    let job = wait_for_job(repo, job_id);
    runner.stop();

    assert_eq!(job.state, "done");
    let result = job.result.unwrap();
    assert_eq!(result["valid_count"], 1);
    assert_eq!(result["invalid_count"], 1);
    let err_msg = result["errors"][0]["message"].as_str().unwrap();
    assert!(err_msg.contains("outside caller's create scope"), "Must mention create scope: {}", err_msg);
}
