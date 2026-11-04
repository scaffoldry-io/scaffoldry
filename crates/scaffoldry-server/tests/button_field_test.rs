//! Field types phase 5: a button field that starts a process.
//!
//! A `Button` field stores nothing. Pressing it checks `run_automation` on the record and the rule's conditions, then
//! starts the rule with its `Manual` trigger. The validator refuses a button whose rule does not exist or is not manual.

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use common::TestDb;
use http_body_util::BodyExt;
use scaffoldry_core::approver::ApproverSpec;
use scaffoldry_core::{ActionType, AutomationRule, ConditionOperator, FieldPredicate, ProcessStep, StepKind, TriggerEvent};
use scaffoldry_server::build_app_with_state;
use scaffoldry_server::service::identity::issue_test_token_and_user;
use scaffoldry_server::state::ServerState;
use serde_json::{json, Value};
use std::sync::Arc;
use tower::ServiceExt;

const ADMIN: &str = "jordan.lee@state.edu";
const OWNER: &str = "einstein@physics.state.edu";
const DECIDER: &str = "bohr@physics.state.edu";
/// An editor of the workspace from another department: may read and edit, but not run the app's automations.
const OUTSIDER: &str = "student.smith@science.state.edu";
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

async fn tool(app: &axum::Router, who: &str, name: &str, args: Value) -> Result<Value, String> {
    let payload = json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": { "name": name, "arguments": args } });
    let (_, v) = send(app, "POST", "/api/mcp", who, Some(payload)).await;
    if let Some(err) = v.get("error") {
        return Err(err["message"].to_string());
    }
    if v["result"]["isError"] == true {
        return Err(v["result"]["content"][0]["text"].to_string());
    }
    Ok(v["result"]["structuredContent"].clone())
}

fn field(name: &str, kind: &str) -> Value {
    json!({ "name": name, "label": name, "field_type": kind, "required": false, "ferpa_sensitive": false })
}

fn button(name: &str, label: &str, rule: &str) -> Value {
    json!({ "name": name, "label": label, "field_type": "Button", "required": false, "ferpa_sensitive": false, "button_rule": rule })
}

fn app_manifest(slug: &str, extra: Vec<Value>) -> Value {
    let mut fields = vec![field("title", "Text"), field("status", "Text")];
    fields.extend(extra);
    json!({
        "slug": slug, "title": "Grants", "description": "", "organization_code": "PHYS", "department": "physics", "workspace_id": WORKSPACE, "views": [],
        "tables": [{ "id": "grants", "name": "Grants", "slug": "grants", "primary_field": "title", "fields": fields }],
    })
}

struct World {
    db: TestDb,
    state: Arc<ServerState>,
    app: axum::Router,
    slug: String,
}

/// A grants app with three buttons: `review` starts a decision, `mark` sets the status, `submitted_only` needs the
/// status to be Submitted.
async fn world() -> World {
    let db = TestDb::new();
    let state = db.state();
    state.seed_demo().expect("seed");
    let app = build_app_with_state(state.clone()).expect("router");
    let slug = format!("grants-{}", &uuid::Uuid::new_v4().simple().to_string()[..8]);
    let (s, b) = send(&app, "POST", &format!("/api/v1/workspaces/{WORKSPACE}/apps"), OWNER, Some(app_manifest(&slug, vec![]))).await;
    assert_eq!(s, StatusCode::CREATED, "{b}");
    for (who, role) in [(DECIDER, "owner"), (OUTSIDER, "editor")] {
        issue_test_token_and_user(who);
        let (s, b) = send(&app, "POST", &format!("/api/v1/workspaces/{WORKSPACE}/collaborators"), ADMIN, Some(json!({ "eppn": who, "role": role }))).await;
        assert!(s.is_success(), "{b}");
    }
    let w = World { db, state, app, slug };
    let rules = [
        manual(&w, "start-review", vec![], StepKind::UserTask { role: String::new(), prompt: "Review the grant".into(), approve: vec![], reject: vec![], approver: Some(ApproverSpec::Person { eppn: DECIDER.into() }), due: None, remind_every: None, on_due: vec![], on_fail: vec![], decision: Default::default(), reject_rule: Default::default(), kind: Default::default(), show: vec![], fill: vec![], require: vec![], decline_reason: false }),
        manual(&w, "mark-reviewed", vec![], StepKind::Service { action: ActionType::UpdateRecordStatus { new_status: "Under review".into() } }),
        manual(&w, "submit-only", vec![FieldPredicate { field_name: "status".into(), operator: ConditionOperator::Equals, expected_value: "Submitted".into() }], StepKind::Service { action: ActionType::UpdateRecordStatus { new_status: "Sent".into() } }),
    ];
    let buttons = vec![button("review", "Start review", &rule_id(&w, "start-review")), button("mark", "Mark reviewed", &rule_id(&w, "mark-reviewed")), button("send", "Send", &rule_id(&w, "submit-only"))];
    let rules_json: Vec<Value> = rules.iter().map(|r| serde_json::to_value(r).unwrap()).collect();
    let made = tool(&w.app, OWNER, "propose_app_change", json!({ "workspace_id": WORKSPACE, "manifest": app_manifest(&w.slug, buttons), "rules": rules_json, "summary": "Buttons" })).await.expect("proposed");
    let id = made["proposal_id"].as_str().unwrap().to_string();
    let digest = tool(&w.app, DECIDER, "get_proposal", json!({ "proposal_id": id })).await.unwrap()["digest"].clone();
    tool(&w.app, DECIDER, "decide_proposal", json!({ "proposal_id": id, "decision": "approve", "rationale": "Reviewed", "digest": digest })).await.expect("approved");
    w
}

fn manual(w: &World, id: &str, predicates: Vec<FieldPredicate>, kind: StepKind) -> AutomationRule {
    AutomationRule {
        id: format!("{id}-{}", &w.slug[7..]),
        app_slug: w.slug.clone(),
        name: id.into(),
        description: String::new(),
        enabled: true,
        trigger: TriggerEvent::Manual,
        predicates,
        actions: vec![],
        steps: vec![ProcessStep { id: "only".into(), when: vec![], kind }],
        version: 0,
        home_workspace: None,
        grants: vec![],
    }
}

fn rule_id(w: &World, name: &str) -> String {
    format!("{name}-{}", &w.slug[7..])
}

async fn record(w: &World, status: &str) -> String {
    let (s, b) = send(&w.app, "POST", &format!("/api/v1/apps/{}/records", w.slug), OWNER, Some(json!({ "data": { "_table_id": "grants", "title": "Telescope", "status": status } }))).await;
    assert!(s.is_success(), "{b}");
    b["id"].as_str().or_else(|| b["record"]["id"].as_str()).unwrap().to_string()
}

async fn press(w: &World, who: &str, id: &str, field: &str) -> (StatusCode, Value) {
    send(&w.app, "POST", &format!("/api/v1/apps/{}/records/{id}/buttons/{field}", w.slug), who, None).await
}

// 1. Pressing a button starts one instance for the record and returns its id. Pressing again while it waits starts no
// second one.
#[tokio::test(flavor = "multi_thread")]
async fn pressing_starts_one_instance() {
    let w = world().await;
    let id = record(&w, "Draft").await;
    let (s, b) = press(&w, OWNER, &id, "review").await;
    assert_eq!(s, StatusCode::OK, "{b}");
    let instance = b["instance_id"].as_str().expect("an instance id").to_string();
    assert_eq!(instance, format!("{}:{id}", rule_id(&w, "start-review")));
    let (s, again) = press(&w, OWNER, &id, "review").await;
    assert_eq!(s, StatusCode::OK, "{again}");
    assert_eq!(again["instance_id"], json!(instance), "the waiting instance, not a new one");
    let rule = rule_id(&w, "start-review");
    let n: i64 = w.db.run(move |c| c.query_one("SELECT COUNT(*) FROM process_instances WHERE rule_id = $1", &[&rule]).unwrap().get(0));
    assert_eq!(n, 1);
}

// 2. A person without `run_automation` on the record sees the button disabled with a reason, and the route is 403.
#[tokio::test(flavor = "multi_thread")]
async fn a_person_who_may_not_run_it_sees_it_disabled() {
    let w = world().await;
    let id = record(&w, "Draft").await;
    let (s, b) = send(&w.app, "GET", &format!("/api/v1/apps/{}/records/{id}/buttons", w.slug), OUTSIDER, None).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    let review = b["buttons"].as_array().unwrap().iter().find(|x| x["field"] == "review").cloned().expect("the button");
    assert_eq!(review["label"], "Start review");
    assert_eq!(review["enabled"], false, "{review}");
    assert!(review["reason"].as_str().is_some_and(|r| !r.is_empty()), "with a reason: {review}");
    let (s, _) = press(&w, OUTSIDER, &id, "review").await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    let (_, mine) = send(&w.app, "GET", &format!("/api/v1/apps/{}/records/{id}/buttons", w.slug), OWNER, None).await;
    assert!(mine["buttons"].as_array().unwrap().iter().all(|x| x["field"] == "send" || x["enabled"] == true), "the owner may press: {mine}");
}

// 3. A record that fails the rule's conditions is 409, with the conditions in words.
#[tokio::test(flavor = "multi_thread")]
async fn a_record_that_fails_the_conditions_is_409() {
    let w = world().await;
    let id = record(&w, "Draft").await;
    let (s, b) = press(&w, OWNER, &id, "send").await;
    assert_eq!(s, StatusCode::CONFLICT, "{b}");
    assert!(b.to_string().contains("status is Submitted"), "{b}");
    let ok = record(&w, "Submitted").await;
    let (s, b) = press(&w, OWNER, &ok, "send").await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_eq!(w.state.find_record(&w.slug, &ok).unwrap().unwrap().data["status"], "Sent");
}

// 4. The validator refuses a button whose rule does not exist, or is not manual.
#[tokio::test(flavor = "multi_thread")]
async fn a_button_needs_a_manual_rule_of_its_app() {
    let w = world().await;
    let missing = tool(&w.app, OWNER, "propose_app_change", json!({ "workspace_id": WORKSPACE, "manifest": app_manifest(&w.slug, vec![button("ghost", "Ghost", "no-such-rule")]), "summary": "A button" })).await;
    let err = missing.expect_err("refused");
    assert!(err.contains("button") && err.contains("no-such-rule"), "{err}");

    let mut created = manual(&w, "on-create", vec![], StepKind::Service { action: ActionType::UpdateRecordStatus { new_status: "New".into() } });
    created.trigger = TriggerEvent::RecordCreated;
    let rules = vec![serde_json::to_value(&created).unwrap()];
    let not_manual = tool(&w.app, OWNER, "propose_app_change", json!({ "workspace_id": WORKSPACE, "manifest": app_manifest(&w.slug, vec![button("auto", "Auto", &created.id)]), "rules": rules, "summary": "A button" })).await;
    let err = not_manual.expect_err("refused");
    assert!(err.contains("Manual"), "{err}");
}

// 5. Every press is in history as an `automation` batch of the changes it made.
#[tokio::test(flavor = "multi_thread")]
async fn a_press_is_an_automation_batch_in_history() {
    let w = world().await;
    let id = record(&w, "Draft").await;
    let (s, b) = press(&w, OWNER, &id, "mark").await;
    assert_eq!(s, StatusCode::OK, "{b}");
    let (s, h) = send(&w.app, "GET", &format!("/api/v1/apps/{}/records/{id}/history", w.slug), OWNER, None).await;
    assert_eq!(s, StatusCode::OK, "{h}");
    let latest = &h["entries"][0];
    assert_eq!(latest["actor_kind"], "automation", "{latest}");
    assert_eq!(latest["actor"], OWNER, "the person who pressed: {latest}");
    assert!(latest["batch_id"].as_str().is_some_and(|b| !b.is_empty()), "{latest}");
    assert!(latest["changes"]["status"].is_object(), "the change it made: {latest}");
    assert_eq!(w.state.find_record(&w.slug, &id).unwrap().unwrap().data["status"], "Under review");
}
