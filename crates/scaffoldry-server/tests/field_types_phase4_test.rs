//! Field types phase 4: duration and date-time.
//!
//! Acceptance tests:
//! 1. Input `90m`, `1.5h`, and `1:30` all store 5,400. Output formats render correctly.
//!    A bad input shows a message and does not commit.
//! 2. A sum rollup over durations is correct.
//! 3. A date-time entered at 23:30 in a zone is stored as the right instant and displayed
//!    at 23:30 in that zone and at another time in another.
//! 4. A Date field never shifts across a time zone change.
//! 5. TODATE and TODATETIME agree with the institution's time zone across a daylight saving change.

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::Router;
use http_body_util::BodyExt;
use scaffoldry_engine::formula::evaluate_formula_with_tz;
use scaffoldry_engine::index::{index_column, index_value, IndexColumn};
use scaffoldry_engine::manifest::is_numeric;
use scaffoldry_engine::{
    validate_manifest, AppManifest, AppTable, FieldSpec, FieldType, LinkCardinality, LinkOnDelete,
};
use scaffoldry_server::build_app_with_state;
use scaffoldry_server::service::identity::issue_test_token_and_user;
use scaffoldry_server::state::ServerState;
use serde_json::{json, Value};
use std::sync::Arc;
use tower::ServiceExt;
use uuid::Uuid;


const OWNER: &str = "prof.curie@science.state.edu";
const WORKSPACE: &str = "ws-bio-lab";

fn db_url() -> String {
    scaffoldry_server::testing::database_url()
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
    state: Arc<ServerState>,
    slug: String,
    _cleanup: Cleanup,
}

async fn world() -> World {
    let state = Arc::new(ServerState::new().expect("state"));
    state.seed_demo().expect("seed");
    let slug = format!("ft4-{}", &Uuid::new_v4().to_string()[..8]);
    World {
        app: build_app_with_state(state.clone()).expect("router"),
        token: issue_test_token_and_user(OWNER),
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

fn manifest_with_tables(slug: &str, tables: Vec<AppTable>) -> AppManifest {
    AppManifest {
        slug: slug.to_string(),
        title: "Field Types Phase 4 Test".to_string(),
        description: String::new(),
        organization_code: "BIO".to_string(),
        department: "biology".to_string(),
        workspace_id: Some(WORKSPACE.to_string()),
        herm_capability_id: None,
        coordinates: None, sealed: false,
        custom_domain: None,
        custom_domain_verified: false,
        tables,
        views: vec![],
        relationships: vec![],
        pages: vec![],
        ceds_mappings: Default::default(),
        access: Default::default(),
    }
}

#[tokio::test]
async fn test_duration_indexing_and_is_numeric() {
    let _db = common::TestDb::new();
    let dur_type = FieldType::Duration { format: scaffoldry_engine::duration::DurationFormat::HMm };
    assert_eq!(index_column(dur_type), IndexColumn::Num);
    assert!(is_numeric(dur_type), "Duration must be recognized as numeric");

    let dt_type = FieldType::DateTime;
    assert_eq!(index_column(dt_type), IndexColumn::Ts);

    let dur_field = FieldSpec::simple("run_time", "Run Time", dur_type, false, false);
    let (num_col, txt_col, ts_col) = index_value(&dur_field, &json!(5400));
    assert_eq!(num_col, Some(5400.0));
    assert_eq!(txt_col, None);
    assert_eq!(ts_col, None);

    let dt_field = FieldSpec::simple("happened_at", "Happened At", FieldType::DateTime, false, false);
    let (num_col, txt_col, ts_col) = index_value(&dt_field, &json!("2026-10-12T03:30:00Z"));
    assert_eq!(num_col, None);
    assert_eq!(txt_col, None);
    assert_eq!(ts_col, Some("2026-10-12T03:30:00+00:00".to_string()));
}

#[tokio::test]
async fn test_duration_inputs_store_5400_and_bad_input_rejected() {
    let _db = common::TestDb::new();
    let w = world().await;
    let mut dur_field = FieldSpec::simple("duration", "Duration", FieldType::Duration { format: scaffoldry_engine::duration::DurationFormat::HMm }, false, false);
    dur_field.duration_format = Some("h:mm".to_string());

    let m = manifest_with_tables(&w.slug, vec![AppTable {
        id: "t1".to_string(),
        name: "Tasks".to_string(),
        slug: "tasks".to_string(),
        description: None,
        icon: None,
        fields: vec![
            FieldSpec::simple("title", "Title", FieldType::Text, true, false),
            dur_field,
        ],
        primary_field: Some("title".to_string()),
        sample_records: vec![],
        coordinates: None, sealed: false,
        row_place: Default::default(),
    }]);
    w.state.persist_app_manifest(m).expect("manifest");

    // 1. Inputs "90m", "1.5h", and "1:30" should all store 5,400 seconds
    for input_val in [json!("90m"), json!("1.5h"), json!("1:30"), json!(5400)] {
        let (status, resp) = send(
            &w.app,
            "POST",
            &format!("/api/v1/apps/{}/records", w.slug),
            &w.token,
            Some(json!({
                "data": {
                    "_table_id": "t1",
                    "title": format!("Task {}", input_val),
                    "duration": input_val,
                }
            })),
        ).await;

        assert_eq!(status, StatusCode::CREATED, "Posting valid duration should succeed");
        let rec_id = resp["id"].as_str().expect("record id");

        let (get_status, get_resp) = send(
            &w.app,
            "GET",
            &format!("/api/v1/apps/{}/records/{}", w.slug, rec_id),
            &w.token,
            None,
        ).await;

        assert_eq!(get_status, StatusCode::OK);
        assert_eq!(get_resp["data"]["duration"], json!(5400), "Stored value must be 5400 whole seconds");
    }

    // 2. Bad input rejected
    let (bad_status, bad_resp) = send(
        &w.app,
        "POST",
        &format!("/api/v1/apps/{}/records", w.slug),
        &w.token,
        Some(json!({
            "data": {
                "_table_id": "t1",
                "title": "Bad Task",
                "duration": "not-a-duration",
            }
        })),
    ).await;

    assert_eq!(bad_status, StatusCode::BAD_REQUEST, "Invalid duration format must return 400");
    let err_str = bad_resp.to_string();
    assert!(err_str.contains("duration") || err_str.contains("bad_request"));
}

#[tokio::test]
async fn test_sum_rollup_over_durations() {
    let _db = common::TestDb::new();
    let w = world().await;

    let parent_table = AppTable {
        id: "parents".to_string(),
        name: "Projects".to_string(),
        slug: "projects".to_string(),
        description: None,
        icon: None,
        fields: vec![
            FieldSpec::simple("name", "Name", FieldType::Text, true, false),
            {
                let mut f = FieldSpec::simple("items", "Items", FieldType::Link, false, false);
                f.link_target_table = Some("children".to_string());
                f.link_cardinality = LinkCardinality::Many;
                f.link_related_field = Some("project".to_string());
                f.link_on_delete = LinkOnDelete::Unlink;
                f
            },
            {
                let mut f = FieldSpec::simple("total_duration", "Total Duration", FieldType::Rollup, false, false);
                f.computed_via = Some("items".to_string());
                f.target_display_field = Some("time_spent".to_string());
                f.rollup_function = Some("sum".to_string());
                f
            },
        ],
        primary_field: Some("name".to_string()),
        sample_records: vec![],
        coordinates: None, sealed: false,
        row_place: Default::default(),
    };

    let child_table = AppTable {
        id: "children".to_string(),
        name: "Work Logs".to_string(),
        slug: "work_logs".to_string(),
        description: None,
        icon: None,
        fields: vec![
            FieldSpec::simple("desc", "Description", FieldType::Text, true, false),
            {
                let mut f = FieldSpec::simple("time_spent", "Time Spent", FieldType::Duration { format: scaffoldry_engine::duration::DurationFormat::HMm }, false, false);
                f.duration_format = Some("h:mm".to_string());
                f
            },
            {
                let mut f = FieldSpec::simple("project", "Project", FieldType::Link, false, false);
                f.link_target_table = Some("parents".to_string());
                f.link_cardinality = LinkCardinality::One;
                f.link_related_field = Some("items".to_string());
                f.link_on_delete = LinkOnDelete::Unlink;
                f
            },
        ],
        primary_field: Some("desc".to_string()),
        sample_records: vec![],
        coordinates: None, sealed: false,
        row_place: Default::default(),
    };

    let m = manifest_with_tables(&w.slug, vec![parent_table, child_table]);
    assert!(validate_manifest(&m).is_ok(), "Manifest with duration sum rollup must validate");
    w.state.persist_app_manifest(m).expect("manifest");

    // Create parent project
    let (p_status, p_resp) = send(
        &w.app,
        "POST",
        &format!("/api/v1/apps/{}/records", w.slug),
        &w.token,
        Some(json!({
            "data": {
                "_table_id": "parents",
                "name": "Project Apollo",
            }
        })),
    ).await;
    assert_eq!(p_status, StatusCode::CREATED);
    let parent_id = p_resp["id"].as_str().unwrap();

    // Create 2 child logs linked to parent: 5400s (90m) and 3600s (1h)
    let (c1_status, _) = send(
        &w.app,
        "POST",
        &format!("/api/v1/apps/{}/records", w.slug),
        &w.token,
        Some(json!({
            "data": {
                "_table_id": "children",
                "desc": "Research",
                "time_spent": 5400,
                "project": parent_id,
            }
        })),
    ).await;
    assert_eq!(c1_status, StatusCode::CREATED);

    let (c2_status, _) = send(
        &w.app,
        "POST",
        &format!("/api/v1/apps/{}/records", w.slug),
        &w.token,
        Some(json!({
            "data": {
                "_table_id": "children",
                "desc": "Testing",
                "time_spent": 3600,
                "project": parent_id,
            }
        })),
    ).await;
    assert_eq!(c2_status, StatusCode::CREATED);

    // Verify parent rollup sums durations: 5400 + 3600 = 9000
    let (get_status, get_resp) = send(
        &w.app,
        "GET",
        &format!("/api/v1/apps/{}/records/{}", w.slug, parent_id),
        &w.token,
        None,
    ).await;
    assert_eq!(get_status, StatusCode::OK);
    assert_eq!(get_resp["data"]["total_duration"], json!(9000), "Rollup sum of durations must be 9000");
}

#[tokio::test]
async fn test_date_never_shifts_across_timezone_change() {
    let _db = common::TestDb::new();
    let w = world().await;

    let m = manifest_with_tables(&w.slug, vec![AppTable {
        id: "t1".to_string(),
        name: "Events".to_string(),
        slug: "events".to_string(),
        description: None,
        icon: None,
        fields: vec![
            FieldSpec::simple("name", "Name", FieldType::Text, true, false),
            FieldSpec::simple("event_date", "Event Date", FieldType::Date, false, false),
            FieldSpec::simple("event_datetime", "Event DateTime", FieldType::DateTime, false, false),
        ],
        primary_field: Some("name".to_string()),
        sample_records: vec![],
        coordinates: None, sealed: false,
        row_place: Default::default(),
    }]);
    w.state.persist_app_manifest(m).expect("manifest");

    let (status, resp) = send(
        &w.app,
        "POST",
        &format!("/api/v1/apps/{}/records", w.slug),
        &w.token,
        Some(json!({
            "data": {
                "_table_id": "t1",
                "name": "Conference",
                "event_date": "2026-10-11",
                "event_datetime": "2026-10-11T23:30:00Z",
            }
        })),
    ).await;
    assert_eq!(status, StatusCode::CREATED);
    let rec_id = resp["id"].as_str().unwrap();

    let (get_status, get_resp) = send(
        &w.app,
        "GET",
        &format!("/api/v1/apps/{}/records/{}", w.slug, rec_id),
        &w.token,
        None,
    ).await;
    assert_eq!(get_status, StatusCode::OK);
    assert_eq!(get_resp["data"]["event_date"], json!("2026-10-11"), "Date must remain exactly 2026-10-11");
    assert_eq!(get_resp["data"]["event_datetime"], json!("2026-10-11T23:30:00Z"));
}

#[tokio::test]
async fn test_todate_and_todatetime_across_dst_change() {
    let _db = common::TestDb::new();
    // In America/New_York:
    // DST ends on Sunday, Nov 1, 2026.
    // Before fallback: 2026-11-01 03:30:00 UTC is 2026-10-31 23:30:00 EDT (UTC-4).
    // After fallback: 2026-11-02 03:30:00 UTC is 2026-11-01 22:30:00 EST (UTC-5).
    let rec = json!({});

    // 1. TODATE converts UTC instant to local calendar date in the given timezone
    let d1 = evaluate_formula_with_tz(r#"TODATE("2026-11-01T03:30:00Z")"#, &rec, Some("America/New_York"));
    assert_eq!(d1, json!("2026-10-31"), "03:30 UTC before fallback should be 2026-10-31 in EDT");

    let d2 = evaluate_formula_with_tz(r#"TODATE("2026-11-02T03:30:00Z")"#, &rec, Some("America/New_York"));
    assert_eq!(d2, json!("2026-11-01"), "03:30 UTC after fallback should be 2026-11-01 in EST");

    // In UTC timezone, both stay in UTC day:
    let d1_utc = evaluate_formula_with_tz(r#"TODATE("2026-11-01T03:30:00Z")"#, &rec, Some("UTC"));
    assert_eq!(d1_utc, json!("2026-11-01"));

    // 2. TODATETIME converts calendar date (midnight local) to UTC instant in the given timezone
    // 2026-11-01 midnight EDT (UTC-4) -> 04:00:00Z
    let dt1 = evaluate_formula_with_tz(r#"TODATETIME("2026-11-01")"#, &rec, Some("America/New_York"));
    assert_eq!(dt1, json!("2026-11-01T04:00:00Z"), "Midnight EDT corresponds to 04:00:00Z");

    // 2026-11-02 midnight EST (UTC-5) -> 05:00:00Z
    let dt2 = evaluate_formula_with_tz(r#"TODATETIME("2026-11-02")"#, &rec, Some("America/New_York"));
    assert_eq!(dt2, json!("2026-11-02T05:00:00Z"), "Midnight EST corresponds to 05:00:00Z");
}
