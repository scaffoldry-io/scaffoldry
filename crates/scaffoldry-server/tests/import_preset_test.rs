//! Import and export, phase 6: moving off a spreadsheet tool.
//!
//! A mapping preset for files exported from spreadsheet-database tools. It matches columns to fields by name (spaces and case
//! do not matter), splits a multi-select column, recognizes a linked-record column by its header, reads checkbox columns as
//! booleans, reads currency strings such as `$1,200.50`, and reports an attachment column's file addresses without fetching
//! them. The dry run lists every decision, and a person can turn any one off. NIST SP 800-53 SI-10.
//!
//! Tests.
//! 1. A fixture CSV with a multi-select, a linked column, a checkbox and a currency column maps as described, each decision listed.
//! 2. An attachment column produces a report of addresses and fetches none.
//! 3. Turning off one decision changes the dry run.
//! 4. A file without the preset is read as before.
//! 5. A checkbox cell the preset cannot read is a row error that names the cell.

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

/// A file the way spreadsheet-database tools write it: titles with spaces, a comma-separated multi-select, "checked" for a
/// checkbox, currency with a symbol and thousands marks, and attachments as `name (address)` pairs. The address is one no
/// machine answers at, so a fetch would hang and show in the test's time.
const FILE: &str = "First Name,Tags,Project,Done,Budget,Files\n\
Ada,\"red, blue\",Apollo,checked,\"$1,200.50\",\"plan.pdf (http://10.255.255.1/a/plan.pdf), logo.png (http://10.255.255.1/b/logo.png)\"\n\
Grace,green,Apollo,,$30.00,\n\
Linus,\"red, green, blue\",Apollo,checked,($5.25),\n";

fn db<R: Send + 'static>(f: impl FnOnce(&mut postgres::Client) -> R + Send + 'static) -> R {
    let url = scaffoldry_server::testing::database_url();
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

fn field(name: &str, label: &str, ty: FieldType) -> FieldSpec {
    FieldSpec::simple(name, label, ty, false, false)
}

async fn fixture() -> Fixture {
    let testdb = common::TestDb::new();
    let dir = std::env::temp_dir().join(format!("scaffoldry-preset-files-{}", Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    std::env::set_var("SCAFFOLDRY_FILES_DIR", &dir);

    let state = Arc::new(ServerState::new().expect("state"));
    state.seed_demo().expect("seed");
    let slug = format!("preset-{}", &Uuid::new_v4().to_string()[..8]);

    let mut tags = field("tags", "Tags", FieldType::MultiSelect);
    tags.select_options = ["red", "green", "blue"]
        .iter()
        .enumerate()
        .map(|(i, l)| SelectOption { id: format!("opt-{i}"), label: l.to_string(), color: SelectOptionColor::Neutral, order: i as i32 })
        .collect();
    let mut project = field("project", "Project", FieldType::Link);
    project.target_table_id = Some("projects".to_string());
    project.target_display_field = Some("name".to_string());
    let fields = vec![
        field("first_name", "First Name", FieldType::Text),
        tags,
        project,
        field("done", "Done", FieldType::Checkbox),
        field("budget", "Budget", FieldType::Currency),
        field("files", "Files", FieldType::Attachment { allow_multiple: true }),
    ];
    let table = |id: &str, name: &str, fields: Vec<FieldSpec>, primary: &str| AppTable {
        id: id.to_string(),
        name: name.to_string(),
        slug: id.to_string(),
        description: None,
        icon: None,
        fields,
        primary_field: Some(primary.to_string()),
        sample_records: vec![],
        coordinates: None,
        sealed: false,
        row_place: Default::default(),
    };
    let manifest = AppManifest {
        slug: slug.clone(),
        title: "Preset Test App".to_string(),
        description: String::new(),
        organization_code: "SCI".to_string(),
        department: "science".to_string(),
        workspace_id: Some(WORKSPACE.to_string()),
        herm_capability_id: None,
        coordinates: None,
        sealed: false,
        custom_domain: None,
        custom_domain_verified: false,
        views: vec![AppView::table("main", "Main", ViewType::Table, fields.clone())],
        pages: vec![],
        access: Default::default(),
        ceds_mappings: Default::default(),
        tables: vec![
            table("tasks", "Tasks", fields, "first_name"),
            table("projects", "Projects", vec![FieldSpec::simple("name", "Name", FieldType::Text, false, true)], "name"),
        ],
        relationships: vec![],
    };
    state.repository.upsert_app_manifest(&manifest).unwrap();
    state.engine.write().unwrap().register_manifest(manifest).unwrap();

    // The project the file's linked column names.
    let s = slug.clone();
    db(move |c| {
        c.execute(
            "INSERT INTO dataset_records (id, app_slug, data, ceds_mapping, is_ferpa_sensitive, table_id) \
             VALUES ('rec-apollo-' || $1, $1, '{\"_table_id\":\"projects\",\"name\":\"Apollo\"}'::jsonb, '{}'::jsonb, false, 'projects')",
            &[&s],
        )
        .unwrap();
    });

    Fixture { app: build_app_with_state(state.clone()).expect("router"), state, slug, _dir: dir, token: issue_test_token_and_user(USER), _db: testdb }
}

async fn upload(f: &Fixture, content: &str) -> Uuid {
    let req = Request::post(format!("/api/v1/apps/{}/imports", f.slug))
        .header(header::AUTHORIZATION, format!("Bearer {}", f.token))
        .header(header::CONTENT_LENGTH, content.len().to_string())
        .header("x-file-name", "from-another-tool.csv")
        .body(Body::from(content.as_bytes().to_vec()))
        .unwrap();
    let res = f.app.clone().oneshot(req).await.unwrap();
    let status = res.status();
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    assert_eq!(status, StatusCode::CREATED, "{}", String::from_utf8_lossy(&bytes));
    Uuid::parse_str(serde_json::from_slice::<Value>(&bytes).unwrap()["import_id"].as_str().unwrap()).unwrap()
}

/// Posts to a step of an import and waits for its job. Returns the job's result.
async fn step(f: &Fixture, id: Uuid, what: &str, mapping: Value) -> Value {
    let req = Request::post(format!("/api/v1/apps/{}/imports/{id}/{what}", f.slug))
        .header(header::AUTHORIZATION, format!("Bearer {}", f.token))
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(mapping.to_string()))
        .unwrap();
    let res = f.app.clone().oneshot(req).await.unwrap();
    let status = res.status();
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&bytes));
    let job_id = Uuid::parse_str(serde_json::from_slice::<Value>(&bytes).unwrap()["job_id"].as_str().unwrap()).unwrap();
    let runner = JobRunner::start(&f.state.repository, JOB_KINDS, fast()).unwrap();
    let job = wait_for_job(&f.state.repository, job_id);
    runner.stop();
    assert_eq!(job.state, "done", "{:?}", job.error);
    job.result.unwrap_or(Value::Null)
}

fn preset(off: &[&str]) -> Value {
    json!({ "target_table": "tasks", "preset": "other_tool", "preset_off": off })
}

fn decision<'a>(result: &'a Value, id: &str) -> &'a Value {
    result["decisions"].as_array().and_then(|l| l.iter().find(|d| d["id"] == id)).unwrap_or_else(|| panic!("no decision {id}: {result}"))
}

// 1. The preset maps the fixture as described, and every decision is listed with a sentence.
#[tokio::test]
async fn a_file_from_another_tool_maps_as_described_and_each_decision_is_listed() {
    let f = fixture().await;
    let id = upload(&f, FILE).await;
    let result = step(&f, id, "dry-run", preset(&[])).await;

    assert_eq!((result["valid_count"].clone(), result["invalid_count"].clone()), (json!(3), json!(0)), "{result}");
    for d in ["name:First Name", "multi:Tags", "link:Project", "checkbox:Done", "currency:Budget", "attachment:Files"] {
        let found = decision(&result, d);
        assert_eq!(found["enabled"], true, "{found}");
        assert!(found["sentence"].as_str().is_some_and(|s| s.len() > 20 && s.ends_with('.')), "a decision says what it did in a sentence: {found}");
    }
    let rows = result["preview"].as_array().expect("a preview of the first rows");
    assert_eq!(rows[0]["first_name"], "Ada", "{result}");
    assert_eq!(rows[0]["tags"], json!(["red", "blue"]), "{result}");
    assert_eq!(rows[0]["project"], format!("rec-apollo-{}", f.slug), "the linked name is resolved by the primary field of Projects: {result}");
    assert_eq!(rows[0]["done"], true, "{result}");
    assert_eq!(rows[0]["budget"], json!(1200.5), "{result}");
    assert_eq!(rows[1]["done"], false, "a blank checkbox is no: {result}");
    assert_eq!(rows[2]["budget"], json!(-5.25), "parentheses are a negative amount: {result}");

    // Running it writes what the preview showed.
    let run = step(&f, id, "run?accept_errors=false", preset(&[])).await;
    assert_eq!(run["created_count"].as_i64().or(run["created"].as_i64()), Some(3), "{run}");
    let slug = f.slug.clone();
    let (n, tags, done, budget): (i64, Value, Value, Value) = db(move |c| {
        let r = c
            .query_one(
                "SELECT (SELECT count(*) FROM dataset_records WHERE app_slug = $1 AND table_id = 'tasks'), \
                        (SELECT data->'tags' FROM dataset_records WHERE app_slug = $1 AND data->>'first_name' = 'Ada'), \
                        (SELECT data->'done' FROM dataset_records WHERE app_slug = $1 AND data->>'first_name' = 'Ada'), \
                        (SELECT data->'budget' FROM dataset_records WHERE app_slug = $1 AND data->>'first_name' = 'Ada')",
                &[&slug],
            )
            .unwrap();
        (r.get(0), r.get(1), r.get(2), r.get(3))
    });
    assert_eq!(n, 3);
    assert_eq!(tags, json!(["red", "blue"]));
    assert_eq!(done, json!(true));
    assert_eq!(budget, json!(1200.5));
}

// 2. An attachment column is a report of addresses. Nothing is fetched and no file is attached.
#[tokio::test]
async fn an_attachment_column_is_a_report_of_addresses_and_fetches_none() {
    let f = fixture().await;
    let id = upload(&f, FILE).await;
    let refs = |f: &Fixture| -> i64 {
        let slug = f.slug.clone();
        db(move |c| c.query_one("SELECT count(*) FROM file_refs WHERE app_slug = $1", &[&slug]).unwrap().get(0))
    };
    let before = refs(&f);

    let started = Instant::now();
    let result = step(&f, id, "dry-run", preset(&[])).await;
    let report = result["attachment_report"].as_array().expect("a report of file addresses");
    assert_eq!(report.len(), 2, "{result}");
    assert_eq!(report[0]["row"], 1);
    assert_eq!(report[0]["column"], "Files");
    assert_eq!(report[0]["name"], "plan.pdf");
    assert_eq!(report[0]["url"], "http://10.255.255.1/a/plan.pdf");
    assert_eq!(report[1]["name"], "logo.png");
    assert!(decision(&result, "attachment:Files")["sentence"].as_str().unwrap().contains("not fetched"), "{result}");

    step(&f, id, "run?accept_errors=false", preset(&[])).await;
    assert!(started.elapsed() < Duration::from_secs(30), "an address that no machine answers would have held the job");
    assert_eq!(refs(&f), before, "no file was fetched or attached");
    let slug = f.slug.clone();
    let attached: i64 = db(move |c| c.query_one("SELECT count(*) FROM dataset_records WHERE app_slug = $1 AND table_id = 'tasks' AND data ? 'files' AND jsonb_array_length(data->'files') > 0", &[&slug]).unwrap().get(0));
    assert_eq!(attached, 0, "the files field holds nothing");
}

// 3. Turning off one decision changes the dry run, and only that decision's column.
#[tokio::test]
async fn turning_off_one_decision_changes_the_dry_run() {
    let f = fixture().await;
    let id = upload(&f, FILE).await;
    let on = step(&f, id, "dry-run", preset(&[])).await;
    let off = step(&f, id, "dry-run", preset(&["currency:Budget"])).await;

    assert_eq!(decision(&off, "currency:Budget")["enabled"], false, "{off}");
    assert_eq!(decision(&off, "checkbox:Done")["enabled"], true, "the others stand: {off}");
    assert_eq!(on["preview"][0]["budget"], json!(1200.5));
    assert_eq!(off["preview"][0]["budget"], "$1,200.50", "the cell is read as the file wrote it: {off}");
    assert_eq!(off["preview"][0]["done"], on["preview"][0]["done"], "another column is unchanged");

    // Without the name match, the column is not mapped and the field is left out.
    let unnamed = step(&f, id, "dry-run", preset(&["name:First Name"])).await;
    assert!(unnamed["preview"][0].get("first_name").is_none(), "{unnamed}");
}

// 4. A file imported without the preset is read as it always was.
#[tokio::test]
async fn without_the_preset_nothing_changes() {
    let f = fixture().await;
    let id = upload(&f, "First Name,Budget\nAda,12\n").await;
    let result = step(&f, id, "dry-run", json!({ "target_table": "tasks", "column_mappings": { "First Name": "first_name", "Budget": "budget" } })).await;
    assert!(result.get("decisions").is_none() && result.get("preview").is_none() && result.get("attachment_report").is_none(), "{result}");
    assert_eq!(result["valid_count"], 1);
}

// 5. A checkbox cell the preset cannot read is a row error that names the cell.
#[tokio::test]
async fn a_checkbox_cell_that_cannot_be_read_is_a_row_error_naming_the_cell() {
    let f = fixture().await;
    let id = upload(&f, "First Name,Done\nAda,checked\nGrace,maybe\n").await;
    let result = step(&f, id, "dry-run", preset(&[])).await;
    assert_eq!((result["valid_count"].clone(), result["invalid_count"].clone()), (json!(1), json!(1)), "{result}");
    let error = &result["errors"][0];
    assert_eq!(error["row"], 2, "{result}");
    assert_eq!(error["column"], "Done", "{result}");
    assert!(error["message"].as_str().unwrap().contains("maybe"), "{result}");
}
