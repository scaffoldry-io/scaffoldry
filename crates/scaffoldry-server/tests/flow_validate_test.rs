//! Workflows phase 7: `POST /flows/validate`.
//!
//! The editor sends the flow it holds. The server returns each problem with a path to the step, the grants as
//! sentences, the parties, the checks a proposal would raise, and the `describe_flow` sentences. It writes nothing.
//!
//! A path is a list of numbers. The first is the step's index in `steps`. When that step is a branch, the next number
//! is the arm (the last arm is `otherwise`), then the step's index in that arm, and so on.

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use common::TestDb;
use http_body_util::BodyExt;
use scaffoldry_core::workflow::{Branch, FlowInput, Target};
use scaffoldry_core::{ActionType, AutomationRule, ProcessStep, StepKind, TriggerEvent};
use scaffoldry_engine::HostRouter;
use scaffoldry_server::build_app_with_state;
use scaffoldry_server::service::flows;
use scaffoldry_server::service::identity::issue_test_token_and_user;
use scaffoldry_server::state::ServerState;
use serde_json::{json, Value};
use std::sync::Arc;
use tower::ServiceExt;

const OWNER: &str = "einstein@physics.state.edu";
const STRANGER: &str = "visitor@chemistry.state.edu";
const WORKSPACE: &str = "ws-physics-optics";

async fn send(app: &axum::Router, uri: &str, who: &str, body: Value) -> (StatusCode, Value) {
    let req = Request::builder()
        .method("POST")
        .uri(uri)
        .header("authorization", format!("Bearer {}", issue_test_token_and_user(who)))
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap();
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

async fn world() -> World {
    let db = TestDb::new();
    let state = db.state();
    state.seed_demo().expect("seed");
    let app = build_app_with_state(state.clone()).expect("router");
    let slug = format!("housing-{}", &uuid::Uuid::new_v4().simple().to_string()[..8]);
    let manifest = json!({
        "slug": slug, "title": "Housing", "description": "", "organization_code": "PHYS", "department": "physics", "workspace_id": WORKSPACE, "views": [],
        "tables": [
            { "id": "applications", "name": "Applications", "slug": "applications", "primary_field": "name", "fields": [field("name", "Text"), field("amount", "Number"), field("status", "Text")] },
            { "id": "logs", "name": "Logs", "slug": "logs", "primary_field": "note", "fields": [field("note", "Text"), field("by", "Text")] },
        ],
    });
    let req = Request::builder()
        .method("POST")
        .uri(format!("/api/v1/workspaces/{WORKSPACE}/apps"))
        .header("authorization", format!("Bearer {}", issue_test_token_and_user(OWNER)))
        .header("content-type", "application/json")
        .body(Body::from(manifest.to_string()))
        .unwrap();
    assert_eq!(app.clone().oneshot(req).await.unwrap().status(), StatusCode::CREATED);
    World { db, state, app, slug }
}

fn step(id: &str, action: ActionType) -> ProcessStep {
    ProcessStep { id: id.into(), when: vec![], kind: StepKind::Service { action } }
}

fn log(w: &World, id: &str, fields: Vec<(&str, &str)>) -> ProcessStep {
    step(id, ActionType::CreateRecord { id: id.into(), app: w.slug.clone(), table: "logs".into(), fields: fields.into_iter().map(|(f, e)| (f.to_string(), e.to_string())).collect() })
}

fn flow(w: &World, id: &str, name: &str, trigger: TriggerEvent, steps: Vec<ProcessStep>) -> AutomationRule {
    AutomationRule {
        id: id.into(),
        app_slug: w.slug.clone(),
        name: name.into(),
        description: String::new(),
        enabled: true,
        trigger,
        predicates: vec![],
        actions: vec![],
        steps,
        version: 1,
        home_workspace: Some(WORKSPACE.into()),
        grants: vec![],
    }
}

fn called() -> TriggerEvent {
    TriggerEvent::Called { inputs: Vec::<FlowInput>::new() }
}

fn start(id: &str, to: &str) -> ProcessStep {
    step(id, ActionType::StartFlow { flow_id: to.into(), inputs: vec![] })
}

fn stored_rows(w: &World) -> (i64, i64) {
    w.db.run(|c| {
        let rules: i64 = c.query_one("SELECT COUNT(*) FROM workflow_rule_versions", &[]).unwrap().get(0);
        let proposals: i64 = c.query_one("SELECT COUNT(*) FROM app_proposals", &[]).unwrap().get(0);
        (rules, proposals)
    })
}

fn problem_at(body: &Value, path: Value) -> Option<&Value> {
    body["problems"].as_array()?.iter().find(|p| p["path"] == path)
}

// A missing field inside a branch is reported at the branch arm's step. A valid flow comes back with its grants as
// sentences, its parties, its checks and its description, and nothing is stored.
#[tokio::test(flavor = "multi_thread")]
async fn a_missing_field_has_the_path_of_its_step_and_nothing_is_stored() {
    let w = world().await;
    let before = stored_rows(&w);
    let good = flow(&w, "aid", "Open aid file", TriggerEvent::RecordCreated, vec![log(&w, "first", vec![("note", "\"opened\"")])]);
    let (s, body) = send(&w.app, "/api/v1/flows/validate", OWNER, serde_json::to_value(&good).unwrap()).await;
    assert_eq!(s, StatusCode::OK, "{body}");
    assert_eq!(body["problems"], json!([]), "{body}");
    assert!(body["grants"].as_str().unwrap().contains("It may create records in Housing / Logs."), "{body}");
    assert_eq!(body["parties"], json!([WORKSPACE]), "{body}");
    assert!(body["checks"].as_array().unwrap().iter().any(|c| c["name"] == "rule_schema"), "{body}");
    assert!(!body["description"].as_array().unwrap().is_empty(), "{body}");

    let arm = Branch { when: vec![], steps: vec![log(&w, "inner", vec![("nope", "\"x\"")])] };
    let branch = ProcessStep { id: "split".into(), when: vec![], kind: StepKind::Branch { branches: vec![arm], otherwise: vec![] } };
    let bad = flow(&w, "aid", "Open aid file", TriggerEvent::RecordCreated, vec![log(&w, "first", vec![("note", "\"opened\"")]), branch]);
    let (s, body) = send(&w.app, "/api/v1/flows/validate", OWNER, serde_json::to_value(&bad).unwrap()).await;
    assert_eq!(s, StatusCode::OK, "{body}");
    let p = problem_at(&body, json!([1, 0, 0])).unwrap_or_else(|| panic!("a problem at [1, 0, 0]: {body}"));
    assert_eq!(p["step"], "inner");
    assert!(p["message"].as_str().unwrap().contains("nope"), "{p}");
    assert_eq!(stored_rows(&w), before, "validating stores no rule version and no proposal");
}

// A step that names a step the flow does not have is reported at that step.
#[tokio::test(flavor = "multi_thread")]
async fn an_unknown_step_reference_has_the_path_of_the_step_that_names_it() {
    let w = world().await;
    let update = step("touch", ActionType::UpdateRecord { target: Target::Created { step: "ghost".into() }, fields: vec![("note".into(), "\"x\"".into())] });
    let bad = flow(&w, "aid", "Open aid file", TriggerEvent::RecordCreated, vec![log(&w, "first", vec![("note", "\"a\"")]), update]);
    let (s, body) = send(&w.app, "/api/v1/flows/validate", OWNER, serde_json::to_value(&bad).unwrap()).await;
    assert_eq!(s, StatusCode::OK, "{body}");
    let p = problem_at(&body, json!([1])).unwrap_or_else(|| panic!("a problem at [1]: {body}"));
    assert!(p["message"].as_str().unwrap().contains("ghost"), "{p}");
}

// A cycle across flows is reported at the step that starts the next flow.
#[tokio::test(flavor = "multi_thread")]
async fn a_cycle_across_flows_has_the_path_of_the_starting_step() {
    let w = world().await;
    for (id, name, next) in [("open", "Open aid file", "notify"), ("notify", "Notify housing", "close")] {
        let mut rule = flow(&w, id, name, called(), vec![start("next", next)]);
        let manifest = w.state.engine.read().unwrap().resolve_by_slug(&w.slug).cloned().unwrap();
        flows::derive(&w.state, &manifest, &mut rule);
        w.state.automations.write().unwrap().entry(w.slug.clone()).or_default().push(rule);
    }
    let closing = flow(&w, "close", "Close file", called(), vec![log(&w, "first", vec![("note", "\"a\"")]), start("again", "open")]);
    let (s, body) = send(&w.app, "/api/v1/flows/validate", OWNER, serde_json::to_value(&closing).unwrap()).await;
    assert_eq!(s, StatusCode::OK, "{body}");
    let p = problem_at(&body, json!([1])).unwrap_or_else(|| panic!("a problem at [1]: {body}"));
    assert!(p["message"].as_str().unwrap().contains("Close file starts Open aid file"), "{p}");
}

// Someone who cannot manage the app or own its workspace may not validate its flows.
#[tokio::test(flavor = "multi_thread")]
async fn only_a_manager_owner_or_admin_may_validate() {
    let w = world().await;
    let good = flow(&w, "aid", "Open aid file", TriggerEvent::RecordCreated, vec![log(&w, "first", vec![("note", "\"a\"")])]);
    let (s, body) = send(&w.app, "/api/v1/flows/validate", STRANGER, serde_json::to_value(&good).unwrap()).await;
    assert_eq!(s, StatusCode::FORBIDDEN, "{body}");
}
