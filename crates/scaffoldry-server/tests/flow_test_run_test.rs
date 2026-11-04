//! Workflows phase 9: `POST /flows/{id}/test-run`.
//!
//! A test run evaluates a flow's conditions and expressions against a real record or sample values. A find reads real
//! data. Nothing is written or sent, and no work item, notification or history entry is made. Each decision is assumed
//! decided, by default with its first outcome. The answer is a trace: what each step did or would do, in words.

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use common::TestDb;
use http_body_util::BodyExt;
use scaffoldry_core::approver::ApproverSpec;
use scaffoldry_core::workflow::{Branch, ConditionOperator, FieldPredicate};
use scaffoldry_core::{ActionType, AutomationRule, ProcessStep, StepKind, TriggerEvent};
use scaffoldry_server::build_app_with_state;
use scaffoldry_server::service::identity::issue_test_token_and_user;
use scaffoldry_server::state::ServerState;
use serde_json::{json, Value};
use std::sync::Arc;
use tower::ServiceExt;

const ADMIN: &str = "jordan.lee@state.edu";
const OWNER: &str = "einstein@physics.state.edu";
const STRANGER: &str = "visitor@chemistry.state.edu";
const CO_OWNER: &str = "bohr@physics.state.edu";
const WORKSPACE: &str = "ws-physics-optics";
const FOREIGN_OWNER: &str = "prof.curie@science.state.edu";
const FOREIGN_WS: &str = "ws-bio-lab";

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
    foreign: String,
}

async fn world() -> World {
    let db = TestDb::new();
    let state = db.state();
    state.seed_demo().expect("seed");
    let app = build_app_with_state(state.clone()).expect("router");
    let tag = &uuid::Uuid::new_v4().simple().to_string()[..8];
    let (slug, foreign) = (format!("housing-{tag}"), format!("aid-{tag}"));
    let housing = json!({
        "slug": slug, "title": "Housing", "description": "", "organization_code": "PHYS", "department": "physics", "workspace_id": WORKSPACE, "views": [],
        "tables": [
            { "id": "applications", "name": "Applications", "slug": "applications", "primary_field": "name", "fields": [field("name", "Text"), field("email", "Text"), field("amount", "Number"), field("status", "Text")] },
            { "id": "logs", "name": "Logs", "slug": "logs", "primary_field": "note", "fields": [field("note", "Text"), field("by", "Text")] },
        ],
    });
    let (s, b) = send(&app, "POST", &format!("/api/v1/workspaces/{WORKSPACE}/apps"), OWNER, Some(housing)).await;
    assert_eq!(s, StatusCode::CREATED, "{b}");
    issue_test_token_and_user(CO_OWNER);
    let (s, b) = send(&app, "POST", &format!("/api/v1/workspaces/{WORKSPACE}/collaborators"), ADMIN, Some(json!({ "eppn": CO_OWNER, "role": "owner" }))).await;
    assert!(s.is_success(), "{b}");
    let mut email = field("email", "Text");
    email["indexed"] = json!(true);
    let aid = json!({
        "slug": foreign, "title": "Financial Aid", "description": "", "organization_code": "BIO", "department": "biology", "workspace_id": FOREIGN_WS, "views": [],
        "tables": [{ "id": "students", "name": "Students", "slug": "students", "primary_field": "name", "fields": [field("name", "Text"), email, field("gpa", "Number")] }],
    });
    let (s, b) = send(&app, "POST", &format!("/api/v1/workspaces/{FOREIGN_WS}/apps"), FOREIGN_OWNER, Some(aid)).await;
    assert_eq!(s, StatusCode::CREATED, "{b}");
    World { db, state, app, slug, foreign }
}

async fn record(w: &World, slug: &str, who: &str, data: Value) -> String {
    let (s, b) = send(&w.app, "POST", &format!("/api/v1/apps/{slug}/records"), who, Some(json!({ "data": data }))).await;
    assert!(s.is_success(), "{b}");
    b["id"].as_str().or_else(|| b["record"]["id"].as_str()).unwrap().to_string()
}

fn step(id: &str, action: ActionType) -> ProcessStep {
    ProcessStep { id: id.into(), when: vec![], kind: StepKind::Service { action } }
}

fn log(w: &World, id: &str, note: &str) -> ProcessStep {
    step(id, ActionType::CreateRecord { id: id.into(), app: w.slug.clone(), table: "logs".into(), fields: vec![("note".into(), note.into())] })
}

fn status(id: &str, to: &str) -> ProcessStep {
    step(id, ActionType::UpdateRecordStatus { new_status: to.into() })
}

fn pred(field: &str, op: ConditionOperator, v: &str) -> FieldPredicate {
    FieldPredicate { field_name: field.into(), operator: op, expected_value: v.into() }
}

fn flow(w: &World, steps: Vec<ProcessStep>) -> AutomationRule {
    AutomationRule {
        id: "aid-check".into(),
        app_slug: w.slug.clone(),
        name: "Aid check".into(),
        description: String::new(),
        enabled: true,
        trigger: TriggerEvent::RecordCreated,
        predicates: vec![],
        actions: vec![],
        steps,
        version: 0,
        home_workspace: Some(WORKSPACE.into()),
        grants: vec![],
    }
}

async fn test_run(w: &World, who: &str, f: &AutomationRule, extra: Value) -> (StatusCode, Value) {
    let mut body = json!({ "flow": f });
    if let (Some(b), Some(e)) = (body.as_object_mut(), extra.as_object()) {
        b.extend(e.clone());
    }
    send(&w.app, "POST", &format!("/api/v1/flows/{}/test-run", f.id), who, Some(body)).await
}

fn at(trace: &Value, path: Value) -> &Value {
    trace["trace"].as_array().unwrap().iter().find(|t| t["path"] == path).unwrap_or_else(|| panic!("no trace entry at {path}: {trace}"))
}

/// Rows in the tables a run would touch, so a test can say nothing changed.
fn counts(w: &World) -> Value {
    w.db.run(|c| {
        let mut out = serde_json::Map::new();
        for t in ["dataset_records", "record_history", "process_instances", "work_items", "notifications", "email_outbox", "flow_effects", "process_timers"] {
            let n: i64 = c.query_one(&format!("SELECT COUNT(*) FROM {t}"), &[]).unwrap().get(0);
            out.insert(t.to_string(), json!(n));
        }
        Value::Object(out)
    })
}

fn branchy(w: &World) -> AutomationRule {
    let big = Branch { when: vec![pred("amount", ConditionOperator::GreaterThan, "100")], steps: vec![status("big", "Big")] };
    let small = Branch { when: vec![pred("amount", ConditionOperator::LessThan, "10")], steps: vec![status("small", "Small")] };
    let split = ProcessStep { id: "split".into(), when: vec![], kind: StepKind::Branch { branches: vec![big, small], otherwise: vec![status("mid", "Mid")] } };
    flow(w, vec![log(w, "note", "{trigger.name}"), split])
}

// 1. A branch's taken arm and its skipped arms are reported, and nothing is written.
#[tokio::test(flavor = "multi_thread")]
async fn a_test_run_reports_the_branch_taken_and_writes_nothing() {
    let w = world().await;
    let id = record(&w, &w.slug, OWNER, json!({ "_table_id": "applications", "name": "Ada Okoro", "amount": 50, "status": "New" })).await;
    let before = counts(&w);
    let (s, body) = test_run(&w, OWNER, &branchy(&w), json!({ "record_id": id })).await;
    assert_eq!(s, StatusCode::OK, "{body}");
    let note = at(&body, json!([0]));
    assert_eq!(note["outcome"], "would write", "{note}");
    assert!(note["sentence"].as_str().unwrap().contains("would create 1 record in Housing / Logs and set note"), "{note}");
    assert_eq!(at(&body, json!([1]))["outcome"], "ran");
    assert_eq!(at(&body, json!([1, 0, 0]))["outcome"], "skipped (condition was false)");
    assert_eq!(at(&body, json!([1, 1, 0]))["outcome"], "skipped (condition was false)");
    let mid = at(&body, json!([1, 2, 0]));
    assert_eq!(mid["outcome"], "would write");
    assert!(mid["sentence"].as_str().unwrap().contains("would set the status to Mid"), "{mid}");
    assert!(at(&body, json!([1]))["sentence"].as_str().unwrap().contains("otherwise"), "{body}");
    assert_eq!(body["first_failure"], Value::Null);
    assert_eq!(counts(&w), before, "a test run writes nothing");
    let rec = w.state.find_record(&w.slug, &id).unwrap().unwrap();
    assert_eq!(rec.data["status"], "New");

    // Sample values instead of a record give the other arm.
    let (s, body) = test_run(&w, OWNER, &branchy(&w), json!({ "values": { "_table_id": "applications", "name": "Bo", "amount": 500 } })).await;
    assert_eq!(s, StatusCode::OK, "{body}");
    assert_eq!(at(&body, json!([1, 0, 0]))["outcome"], "would write");
    assert_eq!(at(&body, json!([1, 2, 0]))["outcome"], "skipped (condition was false)");
}

fn finder(w: &World) -> AutomationRule {
    let find = step("student", ActionType::FindRecord { id: "student".into(), app: w.foreign.clone(), table: "students".into(), field: "email".into(), equals: "{trigger.email}".into(), limit: 5 });
    flow(w, vec![find])
}

// 2. A find reads real data. A tester who cannot read the foreign app sees a count and ids, not values.
#[tokio::test(flavor = "multi_thread")]
async fn a_find_reads_real_data_and_hides_a_foreign_apps_values() {
    let w = world().await;
    let student = record(&w, &w.foreign, FOREIGN_OWNER, json!({ "_table_id": "students", "name": "Ada Okoro", "email": "ada@state.edu", "gpa": 3.9 })).await;
    let values = json!({ "values": { "_table_id": "applications", "name": "Ada", "email": "ada@state.edu" } });

    let (s, body) = test_run(&w, OWNER, &finder(&w), values.clone()).await;
    assert_eq!(s, StatusCode::OK, "{body}");
    let found = at(&body, json!([0]));
    assert_eq!(found["outcome"], "ran", "{found}");
    assert_eq!(found["count"], 1, "{found}");
    assert_eq!(found["records"][0]["id"], student);
    assert!(found["records"][0].get("values").is_none_or(Value::is_null), "values hidden from a tester who cannot read the app: {found}");
    assert!(!body.to_string().contains("3.9"), "no value of the foreign record leaks: {body}");

    let (s, body) = test_run(&w, ADMIN, &finder(&w), values.clone()).await;
    assert_eq!(s, StatusCode::OK, "{body}");
    assert_eq!(at(&body, json!([0]))["records"][0]["values"]["name"], "Ada Okoro", "{body}");

    // A value copied out of the foreign record into a record the tester could read is hidden too.
    let mut copies = finder(&w);
    copies.steps.push(log(&w, "copy", "{student.gpa}"));
    let (s, body) = test_run(&w, OWNER, &copies, values).await;
    assert_eq!(s, StatusCode::OK, "{body}");
    assert_eq!(at(&body, json!([1]))["outcome"], "would write", "{body}");
    assert!(!body.to_string().contains("3.9"), "a copied foreign value does not leak: {body}");
}

fn asks(w: &World) -> AutomationRule {
    let task = ProcessStep {
        id: "review".into(),
        when: vec![],
        kind: StepKind::UserTask {
            role: String::new(),
            prompt: "Review the request".into(),
            approve: vec![ActionType::UpdateRecordStatus { new_status: "Approved".into() }],
            reject: vec![ActionType::UpdateRecordStatus { new_status: "Rejected".into() }],
            on_fail: vec![],
            approver: Some(ApproverSpec::Role { name: "owner".into() }),
            decision: Default::default(),
            reject_rule: Default::default(),
            due: None,
            remind_every: None,
            on_due: vec![],
            kind: Default::default(),
            show: vec![],
            fill: vec![],
            require: vec![],
            decline_reason: Default::default(),
        },
    };
    flow(w, vec![task, log(w, "after", "\"decided\"")])
}

// 3. A decision says who it would go to, from the real resolver, and the chosen outcome runs. The resolver leaves out
// the record's submitter and the person testing, as a real run would, so the second owner is the one it goes to.
#[tokio::test(flavor = "multi_thread")]
async fn a_decision_shows_who_it_would_go_to() {
    let w = world().await;
    let id = record(&w, &w.slug, OWNER, json!({ "_table_id": "applications", "name": "Ada", "amount": 5 })).await;
    let (s, body) = test_run(&w, OWNER, &asks(&w), json!({ "record_id": id })).await;
    assert_eq!(s, StatusCode::OK, "{body}");
    let task = at(&body, json!([0]));
    assert_eq!(task["approvers"], json!([CO_OWNER]), "{task}");
    assert!(task["sentence"].as_str().unwrap().contains("approve"), "the first outcome by default: {task}");
    assert!(task["sentence"].as_str().unwrap().contains("would set the status to Approved"), "{task}");
    assert_eq!(at(&body, json!([1]))["outcome"], "would write");

    let (_, body) = test_run(&w, OWNER, &asks(&w), json!({ "record_id": id, "outcomes": { "review": "reject" } })).await;
    assert!(at(&body, json!([0]))["sentence"].as_str().unwrap().contains("would set the status to Rejected"), "{body}");

    // Only the flow's author may resolve as someone else.
    let live = { let mut f = asks(&w); f.id = "live-asks".into(); f };
    w.state.automations.write().unwrap().entry(w.slug.clone()).or_default().push(live.clone());
    let (s, body) = send(&w.app, "POST", "/api/v1/flows/live-asks/test-run", ADMIN, Some(json!({ "record_id": id, "as_person": OWNER }))).await;
    assert_eq!(s, StatusCode::FORBIDDEN, "{body}");
}

// 4. A failing step stops the trace and says why.
#[tokio::test(flavor = "multi_thread")]
async fn a_failure_stops_the_trace_with_the_reason() {
    let w = world().await;
    w.state.settings.write().unwrap().insert("flows.max_writes_per_run".into(), json!(1));
    let f = flow(&w, vec![log(&w, "one", "\"a\""), log(&w, "two", "\"b\""), log(&w, "three", "\"c\"")]);
    let (s, body) = test_run(&w, OWNER, &f, json!({ "values": { "_table_id": "applications", "name": "Ada" } })).await;
    assert_eq!(s, StatusCode::OK, "{body}");
    assert_eq!(at(&body, json!([0]))["outcome"], "would write");
    let failed = at(&body, json!([1]));
    assert_eq!(failed["outcome"], "failed", "{failed}");
    assert!(failed["sentence"].as_str().unwrap().contains("the limit is 1 writes per run"), "{failed}");
    assert_eq!(body["first_failure"]["path"], json!([1]));
    assert!(body["trace"].as_array().unwrap().iter().all(|t| t["path"] != json!([2])), "nothing after the failure: {body}");
}

// 5. A test run makes no work item, notification, history entry, instance or mail, even for a decision and a notice.
#[tokio::test(flavor = "multi_thread")]
async fn a_test_run_creates_nothing() {
    let w = world().await;
    let id = record(&w, &w.slug, OWNER, json!({ "_table_id": "applications", "name": "Ada", "amount": 5 })).await;
    let mut f = asks(&w);
    f.steps.push(step("tell", ActionType::NotifyCollaborator { role: "owner".into(), message_template: "Done".into() }));
    let before = counts(&w);
    let (s, body) = test_run(&w, OWNER, &f, json!({ "record_id": id })).await;
    assert_eq!(s, StatusCode::OK, "{body}");
    assert!(at(&body, json!([2]))["sentence"].as_str().unwrap().contains("would notify"), "{body}");
    assert_eq!(counts(&w), before);
}

// Someone who may not manage the app may not test its flows, and a record they cannot read is refused.
#[tokio::test(flavor = "multi_thread")]
async fn only_a_manager_may_test_run() {
    let w = world().await;
    let (s, body) = test_run(&w, STRANGER, &branchy(&w), json!({ "values": { "_table_id": "applications", "amount": 1 } })).await;
    assert_eq!(s, StatusCode::FORBIDDEN, "{body}");
    let (s, body) = send(&w.app, "POST", "/api/v1/flows/nope/test-run", OWNER, Some(json!({ "values": {} }))).await;
    assert_eq!(s, StatusCode::NOT_FOUND, "{body}");
}
