//! Process v2, phase 1: measuring the process.
//!
//! Every transition of a process instance writes one `process_events` row in the same transaction as the change: the
//! instance starts, a step is entered and waits, it is decided and left, and the instance ends. An event holds ids, step
//! ids and counts, never a record value. `GET /apps/{slug}/process-analytics` reads only this table and the instances'
//! status, and is for the app's owners and admins and Platform Admins.

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use common::TestDb;
use http_body_util::BodyExt;
use scaffoldry_core::{ActionType, AutomationRule, ProcessStep, StepKind, TriggerEvent};
use scaffoldry_server::build_app_with_state;
use scaffoldry_server::service::identity::issue_test_token_and_user;
use scaffoldry_server::state::ServerState;
use serde_json::{json, Value};
use std::sync::Arc;
use tower::ServiceExt;

const ADMIN: &str = "jordan.lee@state.edu";
const OWNER: &str = "einstein@physics.state.edu";
const WORKSPACE: &str = "ws-physics-optics";
const VALUE: &str = "Confidential-Process-VALUE-5521";

async fn send(app: &axum::Router, method: &str, uri: &str, who: &str, body: Option<Value>) -> (StatusCode, Value) {
    let b = Request::builder().method(method).uri(uri).header("authorization", format!("Bearer {}", issue_test_token_and_user(who)));
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
    db: TestDb,
    state: Arc<ServerState>,
    app: axum::Router,
    slug: String,
}

async fn world() -> World {
    let db = TestDb::new();
    let state = db.state();
    state.seed_demo().expect("seed");
    let app = build_app_with_state(state.clone()).expect("router");
    let slug = format!("proc-{}", &uuid::Uuid::new_v4().simple().to_string()[..8]);
    let manifest = json!({
        "slug": slug, "title": "Admissions", "description": "", "organization_code": "PHYS", "department": "physics", "workspace_id": WORKSPACE, "views": [],
        "tables": [{ "id": "applicants", "name": "Applicants", "slug": "applicants", "primary_field": "title", "fields": [
            { "name": "title", "label": "Title", "field_type": "Text", "required": false, "ferpa_sensitive": false },
            { "name": "status", "label": "Status", "field_type": "Text", "required": false, "ferpa_sensitive": false }
        ] }],
    });
    let (s, b) = send(&app, "POST", &format!("/api/v1/workspaces/{WORKSPACE}/apps"), OWNER, Some(manifest)).await;
    assert_eq!(s, StatusCode::CREATED, "{b}");
    World { db, state, app, slug }
}

/// A three-step rule: a service step, a decision by the owner, and a service step after it.
fn three_steps(slug: &str) -> AutomationRule {
    AutomationRule {
        home_workspace: None,
        grants: vec![],
        version: 0,
        id: "rule-admit".to_string(),
        app_slug: slug.to_string(),
        name: "Admit".to_string(),
        description: String::new(),
        enabled: true,
        trigger: TriggerEvent::RecordCreated,
        predicates: vec![],
        actions: vec![],
        steps: vec![
            ProcessStep { id: "intake".into(), when: vec![], kind: StepKind::Service { action: ActionType::UpdateRecordStatus { new_status: "Received".into() } } },
            ProcessStep {
                id: "dean".into(),
                when: vec![],
                kind: StepKind::UserTask {
                    role: String::new(),
                    prompt: "Dean review".into(),
                    approve: vec![ActionType::UpdateRecordStatus { new_status: "Admitted".into() }],
                    reject: vec![],
                    approver: Some(scaffoldry_core::approver::ApproverSpec::Person { eppn: OWNER.to_string() }), due: None, remind_every: None, on_due: vec![], on_fail: vec![],
                },
            },
            ProcessStep { id: "notify".into(), when: vec![], kind: StepKind::Service { action: ActionType::UpdateRecordStatus { new_status: "Done".into() } } },
        ],
    }
}

fn events(w: &World, instance: &str) -> Vec<(String, Option<String>, Value)> {
    let instance = instance.to_string();
    w.db.run(move |c| {
        c.query("SELECT kind, step_id, detail FROM process_events WHERE instance_id = $1 ORDER BY id", &[&instance]).unwrap().iter().map(|r| (r.get(0), r.get(1), r.get(2))).collect()
    })
}

// 1. A process run end to end writes its events in order. 2. No event holds a record value.
#[tokio::test]
async fn a_run_writes_its_events_in_order_and_none_holds_a_record_value() {
    let w = world().await;
    w.state.automations.write().unwrap().entry(w.slug.clone()).or_default().push(three_steps(&w.slug));
    let (s, b) = send(&w.app, "POST", &format!("/api/v1/apps/{}/records", w.slug), ADMIN, Some(json!({ "data": { "_table_id": "applicants", "title": VALUE, "status": "Submitted" } }))).await;
    assert!(s.is_success(), "{b}");
    let instance: String = {
        let slug = w.slug.clone();
        w.db.run(move |c| c.query_one("SELECT id FROM process_instances WHERE app_slug = $1", &[&slug]).unwrap().get(0))
    };
    let (s, b) = send(&w.app, "POST", &format!("/api/v1/apps/{}/processes/{instance}/decide", w.slug), OWNER, Some(json!({ "decision": "approve" }))).await;
    assert_eq!(s, StatusCode::OK, "{b}");

    let got = events(&w, &instance);
    let kinds: Vec<&str> = got.iter().map(|e| e.0.as_str()).collect();
    assert_eq!(kinds, vec!["started", "step_entered", "waiting", "decided", "step_left", "completed"], "{got:?}");
    assert!(got.iter().skip(1).all(|e| e.1.as_deref() == Some("dean") || e.0 == "completed"), "the steps are named by id: {got:?}");
    assert_eq!(got[3].2["decision"], "approve", "{got:?}");
    assert_eq!(got[2].2["approvers"], 1, "the waiting event counts the approvers: {got:?}");
    assert_eq!(got[5].2["status"], "Completed");

    let all: Vec<String> = w.db.run(|c| c.query("SELECT row_to_json(e)::text FROM process_events e", &[]).unwrap().iter().map(|r| r.get(0)).collect());
    assert!(!all.is_empty());
    for e in &all {
        assert!(!e.contains(VALUE), "a record value in an event: {e}");
        assert!(!e.contains("Submitted") && !e.contains("Admitted"), "a field value in an event: {e}");
    }
}

/// Fixture events: an instance that entered `step` at `entered_hours_ago` and, when given, left it after `took_hours`.
fn put_instance(w: &World, id: &str, status: &str, entered_hours_ago: f64, took_hours: Option<f64>) {
    let (id, slug, status) = (id.to_string(), w.slug.clone(), status.to_string());
    w.db.run(move |c| {
        c.execute(
            "INSERT INTO process_instances (id, app_slug, rule_id, status, instance_json, updated_at) VALUES ($1, $2, 'rule-admit', $3, '{}', NOW())",
            &[&id, &slug, &status],
        )
        .unwrap();
        c.execute(
            "INSERT INTO process_events (instance_id, app_slug, rule_id, rule_version, step_id, kind, at) VALUES ($1, $2, 'rule-admit', 0, 'dean', 'step_entered', NOW() - make_interval(secs => $3))",
            &[&id, &slug, &(entered_hours_ago * 3600.0)],
        )
        .unwrap();
        if let Some(t) = took_hours {
            c.execute(
                "INSERT INTO process_events (instance_id, app_slug, rule_id, rule_version, step_id, kind, at) VALUES ($1, $2, 'rule-admit', 0, 'dean', 'step_left', NOW() - make_interval(secs => $3))",
                &[&id, &slug, &((entered_hours_ago - t) * 3600.0)],
            )
            .unwrap();
        }
    });
}

fn step<'a>(b: &'a Value, step: &str) -> &'a Value {
    b["rules"][0]["steps"].as_array().unwrap().iter().find(|s| s["step_id"] == step).unwrap_or_else(|| panic!("no step {step} in {b}"))
}

// 3. 50 instances with known timings give the expected median and 90th percentile, and the outcome shares.
#[tokio::test]
async fn fifty_instances_give_the_expected_median_and_ninetieth_percentile() {
    let w = world().await;
    w.state.automations.write().unwrap().entry(w.slug.clone()).or_default().push(three_steps(&w.slug));
    for i in 1..=50 {
        let status = if i <= 40 { "Completed" } else { "Rejected" };
        put_instance(&w, &format!("rule-admit:t{i}"), status, 100.0, Some(i as f64));
    }
    let (s, b) = send(&w.app, "GET", &format!("/api/v1/apps/{}/process-analytics?rule=rule-admit", w.slug), OWNER, None).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    let dean = step(&b, "dean");
    assert_eq!(dean["entered"], 50);
    assert_eq!(dean["name"], "Dean review", "the step's name from its prompt: {dean}");
    let hours = |v: &Value| v.as_f64().unwrap() / 3600.0;
    assert!((hours(&dean["median_seconds"]) - 25.5).abs() < 0.01, "median of 1..50 hours is 25.5: {dean}");
    assert!((hours(&dean["p90_seconds"]) - 45.1).abs() < 0.01, "90th percentile of 1..50 hours is 45.1: {dean}");
    let outcomes = &b["rules"][0]["outcomes"];
    assert!((outcomes["Completed"].as_f64().unwrap() - 0.8).abs() < 1e-9, "{outcomes}");
    assert!((outcomes["Rejected"].as_f64().unwrap() - 0.2).abs() < 1e-9, "{outcomes}");
    assert_eq!(outcomes["Failed"], 0.0);
    assert!(b["slowest"].as_str().is_some_and(|s| s.starts_with("Dean review takes a median of")), "{b}");
}

// 4. The waiting histogram places instances by age in buckets of 1, 3, 7, 14 and 30 days.
#[tokio::test]
async fn the_waiting_histogram_places_instances_in_the_right_buckets() {
    let w = world().await;
    w.state.automations.write().unwrap().entry(w.slug.clone()).or_default().push(three_steps(&w.slug));
    for (i, days) in [0.5, 2.0, 2.5, 5.0, 10.0, 20.0, 40.0, 41.0].iter().enumerate() {
        put_instance(&w, &format!("rule-admit:w{i}"), "Waiting", days * 24.0, None);
    }
    let (s, b) = send(&w.app, "GET", &format!("/api/v1/apps/{}/process-analytics", w.slug), ADMIN, None).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    let dean = step(&b, "dean");
    assert_eq!(dean["waiting"], 8);
    assert_eq!(dean["waiting_by_age"], json!({ "under_1_day": 1, "1_to_3_days": 2, "3_to_7_days": 1, "7_to_14_days": 1, "14_to_30_days": 1, "over_30_days": 2 }), "{dean}");
}

// 5. Someone who is not an owner or admin of the app is refused.
#[tokio::test]
async fn a_person_who_is_not_an_owner_or_admin_is_refused() {
    let w = world().await;
    let viewer = format!("viewer-{}@physics.state.edu", &w.slug[5..]);
    issue_test_token_and_user(&viewer);
    let (s, b) = send(&w.app, "POST", &format!("/api/v1/workspaces/{WORKSPACE}/collaborators"), ADMIN, Some(json!({ "eppn": viewer, "role": "viewer" }))).await;
    assert!(s.is_success(), "{b}");
    let (s, _) = send(&w.app, "GET", &format!("/api/v1/apps/{}/process-analytics", w.slug), &viewer, None).await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    let (s, _) = send(&w.app, "GET", &format!("/api/v1/apps/{}/process-analytics", w.slug), OWNER, None).await;
    assert_eq!(s, StatusCode::OK);
    let (s, _) = send(&w.app, "GET", &format!("/api/v1/apps/{}/process-analytics?from=yesterday", w.slug), OWNER, None).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "a date that is not a date");
}
