//! Field types phase 1: select options with ids and colors.
//!
//! Acceptance tests:
//! 1. Renaming `Admitted` to `Offer made` changes the label everywhere and leaves records unchanged.
//! 2. Removing an option in use is flagged and shows the count. Removing an unused one is not.
//! 3. Conversion: 1,000 records with string values become ids, with none lost, and history holds each old string.
//!    A value matching no option becomes blank and is counted.
//! 4. A write with an id that is not an option is 400.
//! 5. A kanban over a select field groups by id and shows the current label and color.
//! 6. A multi-select with three values makes three index rows and filters by any.

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::Router;
use http_body_util::BodyExt;
use scaffoldry_engine::{
    AppManifest, AppTable, AppView, FieldSpec, FieldType, SelectOption, SelectOptionColor, ViewType,
};
use scaffoldry_server::build_app_with_state;
use scaffoldry_server::jobs::{get_job, JobConfig, JobRunner, JOB_KINDS};
use scaffoldry_server::service::identity::issue_test_token_and_user;
use scaffoldry_server::state::ServerState;
use serde_json::{json, Value};
use std::sync::Arc;
use tower::ServiceExt;
use uuid::Uuid;


const OWNER: &str = "prof.curie@science.state.edu";
const SECOND: &str = "jordan.lee@state.edu";
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

struct Cleanup(String);
impl Drop for Cleanup {
    fn drop(&mut self) {
        let slug = self.0.clone();
        let url = db_url();
        let _ = std::thread::spawn(move || {
            if let Ok(mut c) = postgres::Client::connect(&url, postgres::NoTls) {
                let _ = c.execute("DELETE FROM jobs WHERE payload->>'app_slug' = $1", &[&slug]);
                let _ = c.execute("DELETE FROM record_index WHERE app_slug = $1", &[&slug]);
                let _ = c.execute("DELETE FROM record_history WHERE app_slug = $1", &[&slug]);
                let _ = c.execute("DELETE FROM dataset_records WHERE app_slug = $1", &[&slug]);
                let _ = c.execute("DELETE FROM app_proposals WHERE app_slug = $1", &[&slug]);
                let _ = c.execute("DELETE FROM app_manifests WHERE slug = $1", &[&slug]);
            }
        })
        .join();
    }
}

struct World {
    app: Router,
    token: String,
    _second_token: String,
    state: Arc<ServerState>,
    slug: String,
    _cleanup: Cleanup,
}

async fn world() -> World {
    let state = Arc::new(ServerState::new().expect("state"));
    state.seed_demo().expect("seed");
    let slug = format!("ft-{}", &Uuid::new_v4().to_string()[..8]);
    World {
        app: build_app_with_state(state.clone()).expect("router"),
        token: issue_test_token_and_user(OWNER),
        _second_token: issue_test_token_and_user(SECOND),
        state,
        _cleanup: Cleanup(slug.clone()),
        slug,
    }
}

async fn send(app: &Router, method: &str, uri: &str, token: &str, body: Option<Value>) -> (StatusCode, Value) {
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

async fn tool(w: &World, name: &str, args: Value) -> (bool, Value) {
    let payload = json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": { "name": name, "arguments": args } });
    let (_, v) = send(&w.app, "POST", "/api/mcp", &w.token, Some(payload)).await;
    let result = &v["result"];
    if result["isError"] == true {
        return (true, result["content"][0]["text"].clone());
    }
    (false, result["structuredContent"].clone())
}

fn option(id: &str, label: &str, color: SelectOptionColor, order: i32) -> SelectOption {
    SelectOption {
        id: id.to_string(),
        label: label.to_string(),
        color,
        order,
    }
}

fn manifest_with_select(slug: &str, options: Vec<SelectOption>) -> AppManifest {
    let mut status_field = FieldSpec::simple("status", "Status", FieldType::Select, false, false);
    status_field.select_options = options;

    AppManifest {
        slug: slug.to_string(),
        title: "Field Types Test".to_string(),
        description: String::new(),
        organization_code: "BIO".to_string(),
        department: "biology".to_string(),
        workspace_id: Some(WORKSPACE.to_string()),
        herm_capability_id: None,
        custom_domain: None,
        custom_domain_verified: false,
        tables: vec![AppTable {
            id: "t1".to_string(),
            name: "Applicants".to_string(),
            slug: "applicants".to_string(),
            description: None,
            icon: None,
            fields: vec![
                FieldSpec::simple("name", "Name", FieldType::Text, false, false),
                status_field,
            ],
            primary_field: Some("name".to_string()),
            sample_records: vec![],
        }],
        relationships: vec![],
        views: vec![],
        ceds_mappings: Default::default(),
        pages: vec![],
        access: Default::default(),
    }
}

// 1. Renaming `Admitted` to `Offer made` changes the label everywhere and leaves records unchanged.
#[tokio::test]
async fn renaming_option_changes_label_everywhere_and_leaves_records_unchanged() {
    let _db = common::TestDb::new();
    let w = world().await;

    let initial_manifest = manifest_with_select(
        &w.slug,
        vec![
            option("opt_admitted", "Admitted", SelectOptionColor::Ok, 0),
            option("opt_rejected", "Rejected", SelectOptionColor::Bad, 1),
        ],
    );
    w.state.persist_app_manifest(initial_manifest.clone()).expect("persist initial");

    // Write record with option ID
    let (status, rec) = send(
        &w.app,
        "POST",
        &format!("/api/v1/apps/{}/records", w.slug),
        &w.token,
        Some(json!({
            "data": {
                "_table_id": "t1",
                "name": "Alice",
                "status": "opt_admitted"
            }
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{rec}");
    let record_id = rec["id"].as_str().unwrap().to_string();

    // Now update manifest: rename `Admitted` to `Offer made`, keeping same id `opt_admitted`
    let mut updated_manifest = initial_manifest.clone();
    updated_manifest.tables[0].fields[1].select_options[0].label = "Offer made".to_string();

    // Proposing rename must NOT flag data_loss
    let (err, proposal) = tool(
        &w,
        "propose_app_change",
        json!({
            "workspace_id": WORKSPACE,
            "manifest": serde_json::to_value(&updated_manifest).unwrap(),
            "summary": "Rename Admitted to Offer made"
        }),
    )
    .await;
    assert!(!err, "propose_app_change failed: {proposal}");
    let checks = proposal["checks"].as_array().expect("checks");
    let data_loss = checks.iter().find(|c| c["name"] == "data_loss").expect("data_loss check");
    assert_eq!(data_loss["outcome"], "pass", "Renaming label must not flag data_loss");

    // Update the live manifest
    w.state.persist_app_manifest(updated_manifest).expect("persist updated");

    // Check the record: raw value is still `opt_admitted`
    let (status, loaded_rec) = send(
        &w.app,
        "GET",
        &format!("/api/v1/apps/{}/records/{}", w.slug, record_id),
        &w.token,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{loaded_rec}");
    assert_eq!(loaded_rec["data"]["status"], "opt_admitted", "Record status must remain unchanged");

    // Check manifest reflects the new label
    let manifest = w.state.get_app_manifest(&w.slug).expect("manifest");
    assert_eq!(manifest.tables[0].fields[1].select_options[0].label, "Offer made");
}

// 2. Removing an option in use is flagged and shows the count. Removing an unused one is not.
#[tokio::test]
async fn removing_option_in_use_is_flagged_with_count_unused_is_not() {
    let _db = common::TestDb::new();
    let w = world().await;

    let base_manifest = manifest_with_select(
        &w.slug,
        vec![
            option("opt_used", "Used", SelectOptionColor::Neutral, 0),
            option("opt_unused", "Unused", SelectOptionColor::Warn, 1),
            option("opt_keep", "Keep", SelectOptionColor::Ok, 2),
        ],
    );
    w.state.persist_app_manifest(base_manifest.clone()).expect("persist");

    // Create 3 records using opt_used
    for i in 1..=3 {
        let (status, rec) = send(
            &w.app,
            "POST",
            &format!("/api/v1/apps/{}/records", w.slug),
            &w.token,
            Some(json!({
                "data": {
                    "_table_id": "t1",
                    "name": format!("Applicant {i}"),
                    "status": "opt_used"
                }
            })),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "{rec}");
    }

    // Proposal A: Remove `opt_unused`
    let mut proposal_a_manifest = base_manifest.clone();
    proposal_a_manifest.tables[0].fields[1].select_options = vec![
        option("opt_used", "Used", SelectOptionColor::Neutral, 0),
        option("opt_keep", "Keep", SelectOptionColor::Ok, 2),
    ];
    let (err_a, prop_a) = tool(
        &w,
        "propose_app_change",
        json!({
            "workspace_id": WORKSPACE,
            "manifest": serde_json::to_value(&proposal_a_manifest).unwrap(),
            "summary": "Remove unused option"
        }),
    )
    .await;
    assert!(!err_a, "prop_a: {prop_a}");
    let checks_a = prop_a["checks"].as_array().unwrap();
    let data_loss_a = checks_a.iter().find(|c| c["name"] == "data_loss").unwrap();
    assert_eq!(data_loss_a["outcome"], "pass", "Removing unused option must pass");

    // Proposal B: Remove `opt_used`
    let mut proposal_b_manifest = base_manifest.clone();
    proposal_b_manifest.tables[0].fields[1].select_options = vec![
        option("opt_unused", "Unused", SelectOptionColor::Warn, 1),
        option("opt_keep", "Keep", SelectOptionColor::Ok, 2),
    ];
    let (_, prop_b) = tool(
        &w,
        "propose_app_change",
        json!({
            "workspace_id": WORKSPACE,
            "manifest": serde_json::to_value(&proposal_b_manifest).unwrap(),
            "summary": "Remove used option"
        }),
    )
    .await;
    let checks_b = prop_b["checks"].as_array().unwrap();
    let data_loss_b = checks_b.iter().find(|c| c["name"] == "data_loss").unwrap();
    assert_eq!(data_loss_b["outcome"], "fail", "Removing used option must fail data_loss check");
    let problems_str = serde_json::to_string(&data_loss_b["messages"]).unwrap();
    assert!(problems_str.contains("3"), "data_loss message must mention the count (3): {problems_str}");
}

// 3. Conversion: 1,000 records with string values become ids, with none lost, and history holds each old string.
//    A value matching no option becomes blank and is counted.
#[tokio::test]
async fn conversion_1000_records_with_strings_become_ids_and_history_recorded() {
    let _db = common::TestDb::new();
    let w = world().await;

    let m = manifest_with_select(
        &w.slug,
        vec![
            option("opt_draft", "Draft", SelectOptionColor::Neutral, 0),
            option("opt_approved", "Approved", SelectOptionColor::Ok, 1),
        ],
    );
    w.state.persist_app_manifest(m.clone()).expect("persist");

    // Insert 1000 records directly with raw string values in data
    // 500 "Draft", 400 "Approved", 100 "Unknown"
    let slug = w.slug.clone();
    db(move |c| {
        c.execute(
            "INSERT INTO dataset_records (id, app_slug, data, ceds_mapping, is_ferpa_sensitive, table_id, created_at) \
             SELECT 'rec-' || $1 || '-' || lpad(g::text, 6, '0'), $1, \
                    CASE \
                      WHEN g <= 500 THEN jsonb_build_object('_table_id', 't1', 'name', 'Person ' || g, 'status', 'Draft') \
                      WHEN g <= 900 THEN jsonb_build_object('_table_id', 't1', 'name', 'Person ' || g, 'status', 'Approved') \
                      ELSE jsonb_build_object('_table_id', 't1', 'name', 'Person ' || g, 'status', 'UnknownLegacyValue') \
                    END, \
                    '{}'::jsonb, false, 't1', NOW() \
             FROM generate_series(1, 1000) AS g",
            &[&slug],
        )
        .expect("insert records");
    });

    // Enqueue and run convert_select_options job
    let repo = &w.state.repository;
    let job_id = scaffoldry_server::jobs::enqueue(
        repo,
        "convert_select_options",
        json!({ "app_slug": w.slug }),
        "system",
        None,
    )
    .expect("enqueue convert_select_options");

    // Run the job to completion
    let cfg = JobConfig { workers: 1, poll_ms: 20, ..JobConfig::default() };
    let _runner = JobRunner::start(repo, JOB_KINDS, cfg).expect("start runner");

    // Poll until finished
    let mut attempts = 0;
    let job_detail = loop {
        let job = get_job(repo, job_id).expect("get_job").expect("exists");
        if job.state == "done" || job.state == "failed" {
            break job;
        }
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
        attempts += 1;
        if attempts > 100 {
            panic!("Job timed out: state={}", job.state);
        }
    };
    assert_eq!(job_detail.state, "done", "Job must finish successfully: {:?}", job_detail.error);
    let res = job_detail.result.expect("result");
    assert_eq!(res["converted"], 900, "900 records converted to ids");
    assert_eq!(res["blanked"], 100, "100 unknown records became blank");

    // Verify database values
    let slug = w.slug.clone();
    let (draft_count, approved_count, blank_count): (i64, i64, i64) = db(move |c| {
        let d: i64 = c.query_one("SELECT count(*) FROM dataset_records WHERE app_slug = $1 AND data->>'status' = 'opt_draft'", &[&slug]).unwrap().get(0);
        let a: i64 = c.query_one("SELECT count(*) FROM dataset_records WHERE app_slug = $1 AND data->>'status' = 'opt_approved'", &[&slug]).unwrap().get(0);
        let b: i64 = c.query_one("SELECT count(*) FROM dataset_records WHERE app_slug = $1 AND (data->'status' IS NULL OR data->>'status' = '')", &[&slug]).unwrap().get(0);
        (d, a, b)
    });
    assert_eq!(draft_count, 500);
    assert_eq!(approved_count, 400);
    assert_eq!(blank_count, 100);

    // Verify record history holds the old strings under actor_kind "system"
    let slug = w.slug.clone();
    let history_count: i64 = db(move |c| {
        c.query_one(
            "SELECT count(*) FROM record_history WHERE app_slug = $1 AND actor_kind = 'system'",
            &[&slug],
        )
        .unwrap()
        .get(0)
    });
    assert_eq!(history_count, 1000, "History recorded for all 1000 converted/blanked rows");
}

// 4. A write with an id that is not an option is 400.
#[tokio::test]
async fn write_with_unknown_id_is_400_naming_the_field() {
    let _db = common::TestDb::new();
    let w = world().await;

    let m = manifest_with_select(
        &w.slug,
        vec![
            option("opt_valid", "Valid", SelectOptionColor::Ok, 0),
        ],
    );
    w.state.persist_app_manifest(m).expect("persist");

    // Try creating a record with unknown option id "not_an_option"
    let (status, res) = send(
        &w.app,
        "POST",
        &format!("/api/v1/apps/{}/records", w.slug),
        &w.token,
        Some(json!({
            "data": {
                "_table_id": "t1",
                "name": "Bob",
                "status": "not_an_option"
            }
        })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{res}");
    let msg = res["message"].as_str().or_else(|| res["error"].as_str()).unwrap_or("");
    assert!(msg.to_lowercase().contains("status"), "Error message must name the field: {msg}");
}

// 5. A kanban over a select field groups by id and shows the current label and color.
#[tokio::test]
async fn kanban_over_select_field_groups_by_id_and_shows_label_and_color() {
    let _db = common::TestDb::new();
    let w = world().await;

    let mut m = manifest_with_select(
        &w.slug,
        vec![
            option("opt_todo", "To Do", SelectOptionColor::Warn, 0),
            option("opt_done", "Done", SelectOptionColor::Ok, 1),
        ],
    );
    let mut kanban_view = AppView::table("v_kanban", "Kanban View", ViewType::Kanban, vec![]);
    kanban_view.table_id = Some("t1".to_string());
    kanban_view.kanban_column_field = Some("status".to_string());
    m.views.push(kanban_view);
    w.state.persist_app_manifest(m).expect("persist");

    // Create records
    let (s1, _) = send(
        &w.app,
        "POST",
        &format!("/api/v1/apps/{}/records", w.slug),
        &w.token,
        Some(json!({ "data": { "_table_id": "t1", "name": "Task 1", "status": "opt_todo" } })),
    )
    .await;
    assert_eq!(s1, StatusCode::CREATED);

    let (s2, _) = send(
        &w.app,
        "POST",
        &format!("/api/v1/apps/{}/records", w.slug),
        &w.token,
        Some(json!({ "data": { "_table_id": "t1", "name": "Task 2", "status": "opt_done" } })),
    )
    .await;
    assert_eq!(s2, StatusCode::CREATED);

    // Call view grouping endpoint or tool
    let (err, groups) = tool(&w, "get_view_data", json!({
        "app_slug": w.slug,
        "view_id": "v_kanban"
    })).await;
    assert!(!err, "get_view_data result: {groups}");

    // Groups should be grouped by id, including label and color
    let stages = groups["stages"].as_array().expect("stages array");
    assert_eq!(stages.len(), 2);
    let todo_stage = stages.iter().find(|s| s["id"] == "opt_todo").expect("todo stage");
    assert_eq!(todo_stage["label"], "To Do");
    assert_eq!(todo_stage["color"], "warn");
    assert_eq!(todo_stage["records"].as_array().unwrap().len(), 1);

    let done_stage = stages.iter().find(|s| s["id"] == "opt_done").expect("done stage");
    assert_eq!(done_stage["label"], "Done");
    assert_eq!(done_stage["color"], "ok");
    assert_eq!(done_stage["records"].as_array().unwrap().len(), 1);
}

// 6. A multi-select with three values makes three index rows and filters by any.
#[tokio::test]
async fn multiselect_with_three_values_makes_three_index_rows_and_filters_by_any() {
    let _db = common::TestDb::new();
    let w = world().await;

    let mut tags_field = FieldSpec::simple("tags", "Tags", FieldType::MultiSelect, false, false);
    tags_field.indexed = true;
    tags_field.select_options = vec![
        option("opt_red", "Red", SelectOptionColor::Bad, 0),
        option("opt_green", "Green", SelectOptionColor::Ok, 1),
        option("opt_blue", "Blue", SelectOptionColor::Info, 2),
    ];

    let m = AppManifest {
        slug: w.slug.clone(),
        title: "MultiSelect Index Test".to_string(),
        description: String::new(),
        organization_code: "BIO".to_string(),
        department: "biology".to_string(),
        workspace_id: Some(WORKSPACE.to_string()),
        herm_capability_id: None,
        custom_domain: None,
        custom_domain_verified: false,
        tables: vec![AppTable {
            id: "t1".to_string(),
            name: "Items".to_string(),
            slug: "items".to_string(),
            description: None,
            icon: None,
            fields: vec![
                FieldSpec::simple("name", "Name", FieldType::Text, false, false),
                tags_field,
            ],
            primary_field: Some("name".to_string()),
            sample_records: vec![],
        }],
        relationships: vec![],
        views: vec![],
        ceds_mappings: Default::default(),
        pages: vec![],
        access: Default::default(),
    };
    w.state.persist_app_manifest(m).expect("persist");

    // Create a record with 3 tags
    let (status, rec) = send(
        &w.app,
        "POST",
        &format!("/api/v1/apps/{}/records", w.slug),
        &w.token,
        Some(json!({
            "data": {
                "_table_id": "t1",
                "name": "Color Item",
                "tags": ["opt_red", "opt_green", "opt_blue"]
            }
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{rec}");
    let record_id = rec["id"].as_str().unwrap().to_string();

    // Check index rows in database: exactly 3 rows for this record and field
    let slug = w.slug.clone();
    let r_id = record_id.clone();
    let rows: Vec<String> = db(move |c| {
        let rs = c.query(
            "SELECT txt FROM record_index WHERE app_slug = $1 AND record_id = $2 AND field = 'tags' ORDER BY txt",
            &[&slug, &r_id],
        ).unwrap();
        rs.into_iter().map(|r| r.get::<_, Option<String>>(0).unwrap_or_default()).collect()
    });
    assert_eq!(rows.len(), 3, "Must produce three index rows");
    assert!(rows.contains(&"opt_red".to_string()));
    assert!(rows.contains(&"opt_green".to_string()));
    assert!(rows.contains(&"opt_blue".to_string()));

    // Filter by any of the values using list_records tool
    for test_tag in ["opt_red", "opt_green", "opt_blue"] {
        let filter = json!({
            "conjunction": "And",
            "clauses": [{
                "id": "c1",
                "field_name": "tags",
                "operator": "Equals",
                "value": test_tag
            }]
        });
        let (err, res) = tool(&w, "list_records", json!({
            "app_slug": w.slug,
            "table_id": "t1",
            "filter": filter
        })).await;
        assert!(!err, "list_records should succeed: {res}");
        let returned_records = res["records"].as_array().unwrap();
        assert_eq!(returned_records.len(), 1, "Should find the record filtering by {test_tag}");
        assert_eq!(returned_records[0]["id"], record_id);
    }

    // Filter by an option not on the record returns 0 records
    let filter = json!({
        "conjunction": "And",
        "clauses": [{
            "id": "c1",
            "field_name": "tags",
            "operator": "Equals",
            "value": "opt_yellow"
        }]
    });
    let (err, res) = tool(&w, "list_records", json!({
        "app_slug": w.slug,
        "table_id": "t1",
        "filter": filter
    })).await;
    assert!(!err);
    assert_eq!(res["records"].as_array().unwrap().len(), 0);
}
