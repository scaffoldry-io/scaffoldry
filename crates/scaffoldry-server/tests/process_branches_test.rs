//! Process v2 phase 2: branches, manual triggers and date triggers.
//!
//! A branch runs the steps of the first arm whose conditions hold, or its `otherwise` steps, and the choice is a process
//! event. A `Manual` trigger needs a button that names the rule. A `DateField` trigger is started by the hourly
//! `scan_date_triggers` job, once per record and date.

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use common::TestDb;
use http_body_util::BodyExt;
use scaffoldry_core::workflow::{Branch, Target};
use scaffoldry_core::{ActionType, AutomationRule, ConditionOperator, FieldPredicate, ProcessStep, StepKind, TriggerEvent};
use scaffoldry_engine::{AppManifest, HostRouter};
use scaffoldry_server::build_app_with_state;
use scaffoldry_server::service::identity::issue_test_token_and_user;
use scaffoldry_server::service::processes::validate_rule;
use scaffoldry_server::state::ServerState;
use serde_json::{json, Value};
use std::sync::Arc;
use tower::ServiceExt;

const OWNER: &str = "einstein@physics.state.edu";
const WORKSPACE: &str = "ws-physics-optics";

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

fn field(name: &str, kind: &str) -> Value {
    json!({ "name": name, "label": name, "field_type": kind, "required": false, "ferpa_sensitive": false })
}

struct World {
    db: TestDb,
    state: Arc<ServerState>,
    app: axum::Router,
    slug: String,
}

/// One app with grants, which have an amount, a status and an indexed deadline, and a button that names `button_rule`.
async fn world(button_rule: &str) -> World {
    let db = TestDb::new();
    let state = db.state();
    state.seed_demo().expect("seed");
    let app = build_app_with_state(state.clone()).expect("router");
    let slug = format!("grants-{}", &uuid::Uuid::new_v4().simple().to_string()[..8]);
    let mut deadline = field("deadline", "Date");
    deadline["indexed"] = json!(true);
    let mut button = field("start_review", "Button");
    button["button_rule"] = json!(button_rule);
    let mut title = field("title", "Text");
    title["required"] = json!(true);
    let manifest = json!({
        "slug": slug, "title": "Grants", "description": "", "organization_code": "PHYS", "department": "physics", "workspace_id": WORKSPACE, "views": [],
        "tables": [
            { "id": "grants", "name": "Grants", "slug": "grants", "primary_field": "title", "fields": [title, field("amount", "Number"), field("status", "Text"), field("note", "Text"), deadline, button] },
            { "id": "reviews", "name": "Reviews", "slug": "reviews", "primary_field": "summary", "fields": [{ "name": "summary", "label": "summary", "field_type": "Text", "required": true, "ferpa_sensitive": false }, field("grade", "Text")] },
        ],
    });
    let (s, b) = send(&app, "POST", &format!("/api/v1/workspaces/{WORKSPACE}/apps"), OWNER, Some(manifest)).await;
    assert_eq!(s, StatusCode::CREATED, "{b}");
    World { db, state, app, slug }
}

fn manifest(w: &World) -> AppManifest {
    w.state.engine.read().unwrap().resolve_by_slug(&w.slug).cloned().expect("app")
}

fn above(field: &str, value: &str) -> FieldPredicate {
    FieldPredicate { field_name: field.into(), operator: ConditionOperator::GreaterThan, expected_value: value.into() }
}

fn set_status(id: &str, status: &str) -> ProcessStep {
    ProcessStep { id: id.into(), when: vec![], kind: StepKind::Service { action: ActionType::UpdateRecordStatus { new_status: status.into() } } }
}

fn branch(id: &str, arms: Vec<Branch>, otherwise: Vec<ProcessStep>) -> ProcessStep {
    ProcessStep { id: id.into(), when: vec![], kind: StepKind::Branch { branches: arms, otherwise } }
}

fn rule(w: &World, id: &str, trigger: TriggerEvent, steps: Vec<ProcessStep>) -> AutomationRule {
    AutomationRule {
        id: id.into(),
        app_slug: w.slug.clone(),
        name: format!("Rule {id}"),
        description: String::new(),
        enabled: true,
        trigger,
        predicates: vec![],
        actions: vec![],
        steps,
        version: 0,
        home_workspace: None,
        grants: vec![],
    }
}

/// The dean branch for a grant over 10,000, and the chair otherwise.
fn routing(w: &World, id: &str) -> AutomationRule {
    rule(
        w,
        id,
        TriggerEvent::RecordCreated,
        vec![branch("route", vec![Branch { when: vec![above("amount", "10000")], steps: vec![set_status("to_dean", "Dean review")] }], vec![set_status("to_chair", "Chair review")])],
    )
}

async fn create(w: &World, data: Value) -> String {
    let (s, b) = send(&w.app, "POST", &format!("/api/v1/apps/{}/records", w.slug), OWNER, Some(json!({ "data": data }))).await;
    assert!(s.is_success(), "{b}");
    b["id"].as_str().or_else(|| b["record"]["id"].as_str()).unwrap().to_string()
}

fn branch_events(w: &World, rule_id: &str) -> Vec<(String, String, Value)> {
    let rule_id = rule_id.to_string();
    w.db.run(move |c| {
        c.query("SELECT instance_id, COALESCE(step_id, ''), detail FROM process_events WHERE rule_id = $1 AND kind = 'branch' ORDER BY id", &[&rule_id])
            .unwrap()
            .iter()
            .map(|r| (r.get(0), r.get(1), r.get(2)))
            .collect()
    })
}

// 1. A grant over the threshold takes the dean branch; one under it takes the default. The events show which.
#[tokio::test(flavor = "multi_thread")]
async fn a_branch_takes_the_first_arm_that_holds_and_records_the_choice() {
    let w = world("none").await;
    let r = routing(&w, "route-grants");
    assert!(validate_rule(&w.state, &manifest(&w), &r).is_empty(), "{:?}", validate_rule(&w.state, &manifest(&w), &r));
    w.state.automations.write().unwrap().entry(w.slug.clone()).or_default().push(r);

    let big = create(&w, json!({ "_table_id": "grants", "title": "Telescope", "amount": 25000 })).await;
    let small = create(&w, json!({ "_table_id": "grants", "title": "Lens", "amount": 500 })).await;
    assert_eq!(w.state.find_record(&w.slug, &big).unwrap().unwrap().data["status"], "Dean review");
    assert_eq!(w.state.find_record(&w.slug, &small).unwrap().unwrap().data["status"], "Chair review");

    let events = branch_events(&w, "route-grants");
    assert_eq!(events.len(), 2, "{events:?}");
    let of = |rec: &str| events.iter().find(|(inst, _, _)| inst.ends_with(rec)).cloned().unwrap_or_else(|| panic!("no event for {rec}: {events:?}"));
    assert_eq!(of(&big).1, "route");
    assert_eq!(of(&big).2["taken"], json!("1"), "the first arm: {:?}", of(&big));
    assert_eq!(of(&small).2["taken"], json!("otherwise"), "{:?}", of(&small));
    for (_, _, detail) in &events {
        assert!(!detail.to_string().contains("25000") && !detail.to_string().contains("Telescope"), "no record value in an event: {detail}");
    }
}

// 2. A branch nested three levels deep fails validation.
#[tokio::test(flavor = "multi_thread")]
async fn a_branch_three_levels_deep_fails_validation() {
    let w = world("none").await;
    let arm = |steps| Branch { when: vec![above("amount", "1")], steps };
    let three = branch("one", vec![arm(vec![branch("two", vec![arm(vec![branch("three", vec![arm(vec![set_status("deep", "Deep")])], vec![])])], vec![])])], vec![]);
    let problems = validate_rule(&w.state, &manifest(&w), &rule(&w, "deep", TriggerEvent::RecordCreated, vec![three]));
    assert!(problems.iter().any(|p| p.contains("three") && p.contains("two levels")), "{problems:?}");
    let two = branch("one", vec![arm(vec![branch("two", vec![arm(vec![set_status("ok", "Ok")])], vec![])])], vec![]);
    assert!(validate_rule(&w.state, &manifest(&w), &rule(&w, "two", TriggerEvent::RecordCreated, vec![two])).is_empty(), "two levels are allowed");
}

// 4. A rule that would create a record in the table that starts it fails validation. A created record must have every
// required field.
#[tokio::test(flavor = "multi_thread")]
async fn a_rule_that_would_start_itself_fails_validation() {
    let w = world("none").await;
    let make = |table: &str, fields: Vec<(&str, &str)>| ProcessStep {
        id: "make".into(),
        when: vec![],
        kind: StepKind::Service {
            action: ActionType::CreateRecord { id: "make".into(), app: w.slug.clone(), table: table.into(), fields: fields.into_iter().map(|(f, e)| (f.to_string(), e.to_string())).collect() },
        },
    };
    let itself = rule(&w, "loop", TriggerEvent::RecordCreated, vec![make("grants", vec![("title", "\"Copy\"")])]);
    let problems = validate_rule(&w.state, &manifest(&w), &itself);
    assert!(problems.iter().any(|p| p.contains("grants") && p.contains("start itself")), "{problems:?}");

    let missing = rule(&w, "missing", TriggerEvent::RecordCreated, vec![make("reviews", vec![("grade", "\"A\"")])]);
    let problems = validate_rule(&w.state, &manifest(&w), &missing);
    assert!(problems.iter().any(|p| p.contains("summary") && p.contains("required")), "{problems:?}");

    let fine = rule(&w, "fine", TriggerEvent::RecordCreated, vec![make("reviews", vec![("summary", "\"New grant\"")])]);
    let fine_problems = validate_rule(&w.state, &manifest(&w), &fine);
    assert!(fine_problems.is_empty(), "{fine_problems:?}");
    let _ = Target::Trigger;
}

// 5. A date trigger starts a record once, even if the scan runs twice over the window, and not again on the next scan.
#[tokio::test(flavor = "multi_thread")]
async fn a_date_trigger_starts_each_record_once_per_date() {
    let w = world("none").await;
    let trigger = TriggerEvent::DateField { field: "deadline".into(), offset_days: -30, at: None };
    let r = rule(&w, "remind", trigger, vec![set_status("remind", "Reminder due")]);
    assert!(validate_rule(&w.state, &manifest(&w), &r).is_empty(), "{:?}", validate_rule(&w.state, &manifest(&w), &r));
    w.state.automations.write().unwrap().entry(w.slug.clone()).or_default().push(r);

    let today = chrono::Utc::now().date_naive();
    let due = (today + chrono::Duration::days(30)).format("%Y-%m-%d").to_string();
    let later = (today + chrono::Duration::days(90)).format("%Y-%m-%d").to_string();
    let soon = create(&w, json!({ "_table_id": "grants", "title": "Due in thirty days", "deadline": due })).await;
    let far = create(&w, json!({ "_table_id": "grants", "title": "Due in ninety days", "deadline": later })).await;

    let now = chrono::Utc::now();
    let since = now - chrono::Duration::days(1);
    let started = scaffoldry_server::service::date_triggers::scan(&w.state, since, now).expect("scan");
    assert_eq!(started, 1, "one record is due");
    assert_eq!(w.state.find_record(&w.slug, &soon).unwrap().unwrap().data["status"], "Reminder due");
    assert!(w.state.find_record(&w.slug, &far).unwrap().unwrap().data.get("status").is_none_or(Value::is_null));

    // The job runs twice over the same window, then the next scan.
    w.state.overwrite_record_data(&w.slug, &soon, &json!({ "_table_id": "grants", "title": "Due in thirty days", "deadline": due, "status": "Seen" }), &scaffoldry_server::repository::WriteCtx::system("test")).unwrap();
    assert_eq!(scaffoldry_server::service::date_triggers::scan(&w.state, since, now).expect("scan"), 0, "the same window again starts nothing");
    assert_eq!(scaffoldry_server::service::date_triggers::scan(&w.state, now, now + chrono::Duration::hours(1)).expect("scan"), 0, "the next scan starts nothing");
    assert_eq!(w.state.find_record(&w.slug, &soon).unwrap().unwrap().data["status"], "Seen", "not run again");
}

// 6. A `Manual` trigger needs a button that names the rule.
#[tokio::test(flavor = "multi_thread")]
async fn a_manual_trigger_needs_a_button() {
    let w = world("start-review").await;
    let lonely = rule(&w, "no-button", TriggerEvent::Manual, vec![set_status("go", "Under review")]);
    let problems = validate_rule(&w.state, &manifest(&w), &lonely);
    assert!(problems.iter().any(|p| p.contains("button")), "{problems:?}");
    let pressed = rule(&w, "start-review", TriggerEvent::Manual, vec![set_status("go", "Under review")]);
    assert!(validate_rule(&w.state, &manifest(&w), &pressed).is_empty(), "{:?}", validate_rule(&w.state, &manifest(&w), &pressed));
}

// 7. A branch's condition on a missing field fails validation, naming the field.
#[tokio::test(flavor = "multi_thread")]
async fn a_branch_condition_on_a_missing_field_is_named() {
    let w = world("none").await;
    let bad = rule(
        &w,
        "bad",
        TriggerEvent::RecordCreated,
        vec![branch("route", vec![Branch { when: vec![above("budget_code", "3")], steps: vec![set_status("x", "X")] }], vec![])],
    );
    let problems = validate_rule(&w.state, &manifest(&w), &bad);
    assert!(problems.iter().any(|p| p.contains("budget_code")), "{problems:?}");
}

// The date field of a date trigger must be indexed, so the scan never reads the table.
#[tokio::test(flavor = "multi_thread")]
async fn a_date_trigger_needs_an_indexed_date_field() {
    let w = world("none").await;
    let r = rule(&w, "on-note", TriggerEvent::DateField { field: "note".into(), offset_days: 0, at: None }, vec![set_status("x", "X")]);
    let problems = validate_rule(&w.state, &manifest(&w), &r);
    assert!(problems.iter().any(|p| p.contains("note") && (p.contains("date") || p.contains("indexed"))), "{problems:?}");
}
