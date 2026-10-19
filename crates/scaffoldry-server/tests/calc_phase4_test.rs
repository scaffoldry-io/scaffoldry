//! Calculation graph phase 4 tests: time, defaults, and constraints.

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::Router;
use http_body_util::BodyExt;
use scaffoldry_engine::{AppManifest, AppTable, FieldConstraints, FieldSpec, FieldType, SetOn};
use scaffoldry_server::build_app_with_state;
use scaffoldry_server::jobs::{register_volatile_schedule, scheduler_tick, JobConfig, JobRunner, JOB_KINDS};
use scaffoldry_server::service::identity::issue_test_token_and_user;
use scaffoldry_server::state::ServerState;
use serde_json::{json, Value};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;
use tower::ServiceExt;
use uuid::Uuid;

static LOCK: Mutex<()> = Mutex::const_new(());

const OWNER: &str = "prof.curie@science.state.edu";
const WORKSPACE: &str = "ws-bio-lab";

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

struct World {
    app: Router,
    state: Arc<ServerState>,
    token: String,
    slug: String,
    db: common::TestDb,
}

impl World {
    fn run<R: Send + 'static>(&self, f: impl FnOnce(&mut postgres::Client) -> R + Send + 'static) -> R {
        self.db.run(f)
    }
}

async fn world(m: AppManifest) -> World {
    let db = common::TestDb::new();
    let state = db.state();
    state.seed_demo().expect("seed");
    let slug = m.slug.clone();
    state.engine.write().unwrap().register_manifest(m.clone()).unwrap();
    let _ = state.repository.upsert_app_manifest(&m);
    World {
        app: build_app_with_state(state.clone()).expect("router"),
        token: issue_test_token_and_user(OWNER),
        slug,
        state,
        db,
    }
}

async fn create(w: &World, table: &str, mut data: Value) -> (StatusCode, Value) {
    data["_table_id"] = json!(table);
    send(&w.app, "POST", &format!("/api/v1/apps/{}/records", w.slug), &w.token, Some(json!({ "data": data }))).await
}

async fn get(w: &World, id: &str) -> Value {
    send(&w.app, "GET", &format!("/api/v1/apps/{}/records/{id}", w.slug), &w.token, None).await.1
}

async fn patch(w: &World, id: &str, version: i64, data: Value) -> (StatusCode, Value) {
    send(&w.app, "PATCH", &format!("/api/v1/apps/{}/records/{id}", w.slug), &w.token, Some(json!({ "data": data, "version": version }))).await
}

fn fast() -> JobConfig {
    JobConfig {
        workers: 1,
        lock_secs: 10,
        heartbeat_secs: 2,
        recovery_secs: 5,
        poll_ms: 10,
        scheduler_secs: 1,
        backoff_secs: [1, 2, 4],
    }
}

// 1. A TODAY() field is recomputed by the nightly job and not by anything else. Setting the time zone shifts the run.
#[tokio::test]
async fn a_today_field_is_recomputed_by_the_nightly_job_and_not_by_anything_else_setting_the_time_zone_shifts_the_run() {
    let _guard = LOCK.lock().await;
    let slug = format!("cg4-today-{}", &Uuid::new_v4().to_string()[..8]);
    let mut today_field = FieldSpec::simple("today_val", "today_val", FieldType::Formula, false, false);
    today_field.formula_expression = Some("TODAY()".to_string());

    let m = AppManifest {
        slug: slug.clone(),
        title: "TodayTest".into(),
        description: "".into(),
        organization_code: "BIO".into(),
        department: "biology".into(),
        workspace_id: Some(WORKSPACE.into()),
        herm_capability_id: None,
        custom_domain: None,
        custom_domain_verified: false,
        tables: vec![AppTable {
            id: "t1".into(),
            name: "Things".into(),
            slug: "things".into(),
            description: None,
            icon: None,
            fields: vec![
                FieldSpec::simple("code", "code", FieldType::Text, false, false),
                today_field,
            ],
            primary_field: None,
            sample_records: vec![],
        }],
        relationships: vec![],
        views: vec![],
        ceds_mappings: Default::default(),
        pages: vec![],
        access: Default::default(),
    };

    let w = world(m).await;
    let (s, res) = create(&w, "t1", json!({ "code": "first" })).await;
    assert_eq!(s, StatusCode::CREATED, "{res}");
    let id = res["id"].as_str().unwrap().to_string();
    let version = res["version"].as_i64().unwrap();

    // 1a. Simulate past value
    let past = "2020-01-01";
    let slug_clone = slug.clone();
    let id_clone = id.clone();
    w.run(move |c| {
        c.execute(
            "UPDATE dataset_records SET data = jsonb_set(data, '{today_val}', '\"2020-01-01\"'::jsonb) WHERE app_slug = $1 AND id = $2",
            &[&slug_clone, &id_clone],
        ).unwrap();
    });

    let rec = get(&w, &id).await;
    assert_eq!(rec["data"]["today_val"], past, "past date set directly");

    // 1b. Normal update does NOT recompute TODAY()
    let (s_patch, p_res) = patch(&w, &id, version, json!({ "code": "updated" })).await;
    assert_eq!(s_patch, StatusCode::OK, "{p_res}");

    let rec_after_patch = get(&w, &id).await;
    assert_eq!(rec_after_patch["data"]["today_val"], past, "today_val must not change on normal write");

    // 1c. Nightly job recalc_volatile DOES recompute it
    let repo = &w.state.repository;
    register_volatile_schedule(repo, "UTC").unwrap();
    w.run(|c| {
        c.execute("DELETE FROM jobs WHERE kind = 'recalc_volatile'", &[]).unwrap();
        c.execute("UPDATE job_schedules SET next_run_at = NOW() - INTERVAL '1 second' WHERE kind = 'recalc_volatile'", &[]).unwrap();
    });

    let other_kinds: Vec<String> = w.run(|c| {
        c.query("SELECT kind FROM job_schedules WHERE kind != 'recalc_volatile'", &[])
            .unwrap()
            .into_iter()
            .map(|r| r.get(0))
            .collect()
    });
    let repo_c = repo.clone();
    let spawned = std::thread::spawn(move || scheduler_tick(&repo_c, &other_kinds)).join().unwrap().unwrap();
    assert_eq!(spawned, 1, "recalc_volatile job spawned");

    let repo_c = repo.clone();
    let runner = std::thread::spawn(move || JobRunner::start(&repo_c, JOB_KINDS, fast())).join().unwrap().unwrap();
    tokio::time::sleep(Duration::from_millis(500)).await;
    runner.stop();

    let rec_after_job = get(&w, &id).await;
    assert_ne!(rec_after_job["data"]["today_val"], past, "today_val recomputed by nightly job");
    assert!(rec_after_job["data"]["today_val"].is_string());

    // 1d. Setting the timezone shifts the run
    register_volatile_schedule(repo, "UTC").unwrap();
    let utc_next: chrono::DateTime<chrono::Utc> = w.run(|c| {
        c.query_one("SELECT next_run_at FROM job_schedules WHERE kind = 'recalc_volatile'", &[]).unwrap().get(0)
    });

    register_volatile_schedule(repo, "America/New_York").unwrap();
    let ny_next: chrono::DateTime<chrono::Utc> = w.run(|c| {
        c.query_one("SELECT next_run_at FROM job_schedules WHERE kind = 'recalc_volatile'", &[]).unwrap().get(0)
    });

    assert_ne!(utc_next, ny_next, "Setting timezone shifts the next run");
}

// 2. A volatile field on a 600,000-row table is refused.
#[tokio::test]
async fn a_volatile_field_on_a_600_000_row_table_is_refused() {
    let _guard = LOCK.lock().await;
    let slug = format!("cg4-big-{}", &Uuid::new_v4().to_string()[..8]);

    let mut today_field = FieldSpec::simple("today_val", "today_val", FieldType::Formula, false, false);
    today_field.formula_expression = Some("TODAY()".to_string());

    let m = AppManifest {
        slug: slug.clone(),
        title: "BigTableTest".into(),
        description: "".into(),
        organization_code: "BIO".into(),
        department: "biology".into(),
        workspace_id: Some(WORKSPACE.into()),
        herm_capability_id: None,
        custom_domain: None,
        custom_domain_verified: false,
        tables: vec![AppTable {
            id: "t1".into(),
            name: "Huge".into(),
            slug: "huge".into(),
            description: None,
            icon: None,
            fields: vec![
                FieldSpec::simple("code", "code", FieldType::Text, false, false),
            ],
            primary_field: None,
            sample_records: vec![],
        }],
        relationships: vec![],
        views: vec![],
        ceds_mappings: Default::default(),
        pages: vec![],
        access: Default::default(),
    };

    let w = world(m.clone()).await;

    // Simulate 600,000 rows in dataset_records for table t1
    let slug_clone = slug.clone();
    w.run(move |c| {
        c.execute(
            "INSERT INTO dataset_records (id, app_slug, table_id, data, ceds_mapping, is_ferpa_sensitive, created_by, version) \
             SELECT 'dummy-' || g, $1, 't1', '{\"_table_id\": \"t1\"}'::jsonb, '{}'::jsonb, false, 'admin', 1 \
             FROM generate_series(1, 500001) AS g",
            &[&slug_clone],
        ).unwrap();
    });

    // Now try proposing a manifest with a volatile formula on this table
    let mut updated_manifest = m.clone();
    updated_manifest.tables[0].fields.push(today_field);
    let mut body = serde_json::to_value(&updated_manifest).unwrap();
    body["version"] = json!(1);

    let (st, res) = send(
        &w.app,
        "PUT",
        &format!("/api/v1/apps/{}", w.slug),
        &w.token,
        Some(body),
    ).await;

    assert_eq!(st, StatusCode::BAD_REQUEST, "{res}");
    let err = res["error"].as_str().unwrap();
    assert!(err.contains("volatile formulas") && err.contains("500,000 rows"), "{err}");
}

// 3. A default fills a missing value on create and does not override a supplied one.
#[tokio::test]
async fn a_default_fills_a_missing_value_on_create_and_does_not_override_a_supplied_one() {
    let _guard = LOCK.lock().await;
    let slug = format!("cg4-def-{}", &Uuid::new_v4().to_string()[..8]);

    let mut status_field = FieldSpec::simple("status", "status", FieldType::Text, false, false);
    status_field.default = Some(json!("draft"));

    let m = AppManifest {
        slug: slug.clone(),
        title: "DefTest".into(),
        description: "".into(),
        organization_code: "BIO".into(),
        department: "biology".into(),
        workspace_id: Some(WORKSPACE.into()),
        herm_capability_id: None,
        custom_domain: None,
        custom_domain_verified: false,
        tables: vec![AppTable {
            id: "t1".into(),
            name: "Items".into(),
            slug: "items".into(),
            description: None,
            icon: None,
            fields: vec![
                FieldSpec::simple("name", "name", FieldType::Text, false, false),
                status_field,
            ],
            primary_field: None,
            sample_records: vec![],
        }],
        relationships: vec![],
        views: vec![],
        ceds_mappings: Default::default(),
        pages: vec![],
        access: Default::default(),
    };

    let w = world(m).await;

    // Create 1: missing status -> default draft
    let (s1, r1) = create(&w, "t1", json!({ "name": "Item 1" })).await;
    assert_eq!(s1, StatusCode::CREATED, "{r1}");
    assert_eq!(r1["data"]["status"], "draft");

    // Create 2: supplied status -> not overridden
    let (s2, r2) = create(&w, "t1", json!({ "name": "Item 2", "status": "published" })).await;
    assert_eq!(s2, StatusCode::CREATED, "{r2}");
    assert_eq!(r2["data"]["status"], "published");
}

// 4. status_changed_on changes only when status changes.
#[tokio::test]
async fn status_changed_on_changes_only_when_status_changes() {
    let _guard = LOCK.lock().await;
    let slug = format!("cg4-chg-{}", &Uuid::new_v4().to_string()[..8]);

    let mut chg_field = FieldSpec::simple("status_changed_on", "status_changed_on", FieldType::Text, false, false);
    chg_field.set_on = Some(SetOn::Change { fields: vec!["status".to_string()] });
    chg_field.formula_expression = Some("NOW()".to_string());

    let m = AppManifest {
        slug: slug.clone(),
        title: "ChangeTest".into(),
        description: "".into(),
        organization_code: "BIO".into(),
        department: "biology".into(),
        workspace_id: Some(WORKSPACE.into()),
        herm_capability_id: None,
        custom_domain: None,
        custom_domain_verified: false,
        tables: vec![AppTable {
            id: "t1".into(),
            name: "Tickets".into(),
            slug: "tickets".into(),
            description: None,
            icon: None,
            fields: vec![
                FieldSpec::simple("title", "title", FieldType::Text, false, false),
                FieldSpec::simple("status", "status", FieldType::Text, false, false),
                chg_field,
            ],
            primary_field: None,
            sample_records: vec![],
        }],
        relationships: vec![],
        views: vec![],
        ceds_mappings: Default::default(),
        pages: vec![],
        access: Default::default(),
    };

    let w = world(m).await;

    let (s, res) = create(&w, "t1", json!({ "title": "Ticket 1", "status": "open", "status_changed_on": "2026-01-01T00:00:00Z" })).await;
    assert_eq!(s, StatusCode::CREATED, "{res}");
    let id = res["id"].as_str().unwrap().to_string();
    let v1 = res["version"].as_i64().unwrap();
    let initial_ts = res["data"]["status_changed_on"].as_str().unwrap().to_string();

    tokio::time::sleep(Duration::from_millis(50)).await;

    // Update title only -> status_changed_on UNCHANGED
    let (s_patch1, p1) = patch(&w, &id, v1, json!({ "title": "Ticket Renamed" })).await;
    assert_eq!(s_patch1, StatusCode::OK, "{p1}");
    assert_eq!(p1["data"]["status_changed_on"], initial_ts, "timestamp must not change when status did not change");
    let v2 = p1["version"].as_i64().unwrap();

    tokio::time::sleep(Duration::from_millis(50)).await;

    // Update status -> status_changed_on CHANGES
    let (s_patch2, p2) = patch(&w, &id, v2, json!({ "status": "closed" })).await;
    assert_eq!(s_patch2, StatusCode::OK, "{p2}");
    assert_ne!(p2["data"]["status_changed_on"], initial_ts, "timestamp must change when status changes");
}

// 5. Two concurrent writes of the same unique value: exactly one succeeds.
#[tokio::test]
async fn two_concurrent_writes_of_the_same_unique_value_exactly_one_succeeds() {
    let _guard = LOCK.lock().await;
    let slug = format!("cg4-uniq-{}", &Uuid::new_v4().to_string()[..8]);

    let mut serial_field = FieldSpec::simple("serial", "serial", FieldType::Text, false, false);
    serial_field.constraints = Some(FieldConstraints {
        unique: true,
        required: false,
        min: None,
        max: None,
    });

    let m = AppManifest {
        slug: slug.clone(),
        title: "UniqueTest".into(),
        description: "".into(),
        organization_code: "BIO".into(),
        department: "biology".into(),
        workspace_id: Some(WORKSPACE.into()),
        herm_capability_id: None,
        custom_domain: None,
        custom_domain_verified: false,
        tables: vec![AppTable {
            id: "t1".into(),
            name: "Serials".into(),
            slug: "serials".into(),
            description: None,
            icon: None,
            fields: vec![
                serial_field,
            ],
            primary_field: None,
            sample_records: vec![],
        }],
        relationships: vec![],
        views: vec![],
        ceds_mappings: Default::default(),
        pages: vec![],
        access: Default::default(),
    };

    let w = Arc::new(world(m).await);

    let w1 = w.clone();
    let w2 = w.clone();

    let h1 = tokio::spawn(async move {
        create(&w1, "t1", json!({ "serial": "SN-MATCH" })).await
    });
    let h2 = tokio::spawn(async move {
        create(&w2, "t1", json!({ "serial": "SN-MATCH" })).await
    });

    let (res1, res2) = (h1.await.unwrap(), h2.await.unwrap());

    let (s1, b1) = res1;
    let (s2, b2) = res2;

    let one_created = (s1 == StatusCode::CREATED && s2 == StatusCode::BAD_REQUEST)
        || (s2 == StatusCode::CREATED && s1 == StatusCode::BAD_REQUEST);

    assert!(one_created, "Exactly one succeeds (s1={s1}, s2={s2}, b1={b1}, b2={b2})");

    let failed_body = if s1 == StatusCode::BAD_REQUEST { b1 } else { b2 };
    assert_eq!(failed_body["code"], "constraint_violated");
    assert_eq!(failed_body["field"], "serial");
    assert_eq!(failed_body["constraint"], "unique");
}

// 6. A min violation is constraint_violated naming the field.
#[tokio::test]
async fn a_min_violation_is_constraint_violated_naming_the_field() {
    let _guard = LOCK.lock().await;
    let slug = format!("cg4-min-{}", &Uuid::new_v4().to_string()[..8]);

    let mut score_field = FieldSpec::simple("score", "score", FieldType::Number, false, false);
    score_field.constraints = Some(FieldConstraints {
        unique: false,
        required: false,
        min: Some(10.0),
        max: Some(100.0),
    });

    let m = AppManifest {
        slug: slug.clone(),
        title: "MinTest".into(),
        description: "".into(),
        organization_code: "BIO".into(),
        department: "biology".into(),
        workspace_id: Some(WORKSPACE.into()),
        herm_capability_id: None,
        custom_domain: None,
        custom_domain_verified: false,
        tables: vec![AppTable {
            id: "t1".into(),
            name: "Scores".into(),
            slug: "scores".into(),
            description: None,
            icon: None,
            fields: vec![score_field],
            primary_field: None,
            sample_records: vec![],
        }],
        relationships: vec![],
        views: vec![],
        ceds_mappings: Default::default(),
        pages: vec![],
        access: Default::default(),
    };

    let w = world(m).await;

    let (s, body) = create(&w, "t1", json!({ "score": 5.0 })).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(body["code"], "constraint_violated");
    assert_eq!(body["field"], "score");
    assert_eq!(body["constraint"], "min");
}

// 7. Adding unique to a field with 3 duplicate values raises data_loss and shows examples.
#[tokio::test]
async fn adding_unique_to_a_field_with_3_duplicate_values_raises_data_loss_and_shows_examples() {
    let _guard = LOCK.lock().await;
    let slug = format!("cg4-dupe-{}", &Uuid::new_v4().to_string()[..8]);

    let code_field = FieldSpec::simple("code", "code", FieldType::Text, false, false);

    let m = AppManifest {
        slug: slug.clone(),
        title: "DupeTest".into(),
        description: "".into(),
        organization_code: "BIO".into(),
        department: "biology".into(),
        workspace_id: Some(WORKSPACE.into()),
        herm_capability_id: None,
        custom_domain: None,
        custom_domain_verified: false,
        tables: vec![AppTable {
            id: "t1".into(),
            name: "Codes".into(),
            slug: "codes".into(),
            description: None,
            icon: None,
            fields: vec![code_field],
            primary_field: None,
            sample_records: vec![],
        }],
        relationships: vec![],
        views: vec![],
        ceds_mappings: Default::default(),
        pages: vec![],
        access: Default::default(),
    };

    let w = world(m.clone()).await;

    // Create 3 records with the same code
    for i in 1..=3 {
        let (s, res) = create(&w, "t1", json!({ "code": "DUPE-VALUE" })).await;
        assert_eq!(s, StatusCode::CREATED, "record {i} created: {res}");
    }

    // Now propose a new manifest making code unique
    let mut updated = m.clone();
    updated.tables[0].fields[0].constraints = Some(FieldConstraints {
        unique: true,
        required: false,
        min: None,
        max: None,
    });
    let mut body = serde_json::to_value(&updated).unwrap();
    body["version"] = json!(1);

    let (st, res) = send(
        &w.app,
        "PUT",
        &format!("/api/v1/apps/{}", w.slug),
        &w.token,
        Some(body),
    ).await;

    assert_eq!(st, StatusCode::ACCEPTED, "proposal pending: {res}");
    let checks = res["checks"].as_array().unwrap();
    let data_loss = checks.iter().find(|c| c["name"] == "data_loss").expect("data_loss check present");
    assert_eq!(data_loss["outcome"], "fail", "data_loss must fail: {data_loss}");

    let previews = data_loss["previews"].as_array().expect("previews present on data_loss");
    let dupe_preview = previews.iter().find(|p| p["field"] == "code").expect("code preview found");
    assert_eq!(dupe_preview["duplicates"], 3, "count of duplicates must be 3");
    assert!(!dupe_preview["examples"].as_array().unwrap().is_empty(), "examples present");
}
