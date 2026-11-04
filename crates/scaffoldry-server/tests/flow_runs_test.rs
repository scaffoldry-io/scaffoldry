//! Workflows phase 10: run history, the timeline of one run, retry, and version comparison.
//!
//! A run's timeline comes from `process_events`: each step with its outcome and its effects in words, with the records
//! it wrote as ids, and the first error. No field value appears. A failed run is retried by an owner or an
//! administrator, with a reason, under the same run id, so the dedupe keys stop any effect from being written twice. A
//! run that failed outside its grants waits for a new version.

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use common::TestDb;
use http_body_util::BodyExt;
use scaffoldry_core::{ActionType, AutomationRule, ProcessStep, StepKind, TriggerEvent};
use scaffoldry_engine::HostRouter;
use scaffoldry_server::build_app_with_state;
use scaffoldry_server::service::identity::issue_test_token_and_user;
use scaffoldry_server::service::{flows, processes};
use scaffoldry_server::state::{DatasetRecord, ServerState};
use serde_json::{json, Value};
use std::sync::Arc;
use tower::ServiceExt;

const ADMIN: &str = "jordan.lee@state.edu";
const OWNER: &str = "einstein@physics.state.edu";
const EDITOR: &str = "feynman@physics.state.edu";
const WORKSPACE: &str = "ws-physics-optics";
const SECRET: &str = "Ada Okoro-7734";

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

async fn world() -> World {
    let db = TestDb::new();
    let state = db.state();
    state.seed_demo().expect("seed");
    let app = build_app_with_state(state.clone()).expect("router");
    let slug = format!("housing-{}", &uuid::Uuid::new_v4().simple().to_string()[..8]);
    let manifest = json!({
        "slug": slug, "title": "Housing", "description": "", "organization_code": "PHYS", "department": "physics", "workspace_id": WORKSPACE, "views": [],
        "tables": [
            { "id": "applications", "name": "Applications", "slug": "applications", "primary_field": "name", "fields": [field("name", "Text"), field("status", "Text")] },
            { "id": "logs", "name": "Logs", "slug": "logs", "primary_field": "note", "fields": [field("note", "Text")] },
        ],
    });
    let (s, b) = send(&app, "POST", &format!("/api/v1/workspaces/{WORKSPACE}/apps"), OWNER, Some(manifest)).await;
    assert_eq!(s, StatusCode::CREATED, "{b}");
    issue_test_token_and_user(EDITOR);
    let (s, b) = send(&app, "POST", &format!("/api/v1/workspaces/{WORKSPACE}/collaborators"), ADMIN, Some(json!({ "eppn": EDITOR, "role": "editor" }))).await;
    assert!(s.is_success(), "{b}");
    World { db, state, app, slug }
}

fn log(w: &World, id: &str, note: &str) -> ProcessStep {
    ProcessStep { id: id.into(), when: vec![], kind: StepKind::Service { action: ActionType::CreateRecord { id: id.into(), app: w.slug.clone(), table: "logs".into(), fields: vec![("note".into(), note.into())] } } }
}

fn flow(w: &World, version: u32, steps: Vec<ProcessStep>) -> AutomationRule {
    let mut rule = AutomationRule {
        id: "open-file".into(),
        app_slug: w.slug.clone(),
        name: "Open file".into(),
        description: String::new(),
        enabled: true,
        trigger: TriggerEvent::RecordCreated,
        predicates: vec![],
        actions: vec![],
        steps,
        version,
        home_workspace: Some(WORKSPACE.into()),
        grants: vec![],
    };
    let manifest = w.state.engine.read().unwrap().resolve_by_slug(&w.slug).cloned().unwrap();
    flows::derive(&w.state, &manifest, &mut rule);
    rule
}

fn install(w: &World, rule: &AutomationRule) {
    let mut all = w.state.automations.write().unwrap();
    let list = all.entry(w.slug.clone()).or_default();
    list.retain(|r| r.id != rule.id);
    list.push(rule.clone());
}

fn trigger(w: &World) -> DatasetRecord {
    let id = uuid::Uuid::new_v4().to_string();
    let rec = DatasetRecord {
        id: id.clone(),
        app_slug: w.slug.clone(),
        data: json!({ "_table_id": "applications", "name": SECRET, "status": "New" }),
        ceds_mapping: json!({}),
        is_ferpa_sensitive: false,
        created_at: chrono::Utc::now().to_rfc3339(),
        created_by: Some(OWNER.into()),
        version: 1,
    };
    w.state.insert_record(&rec, &scaffoldry_server::repository::WriteCtx::system("test")).unwrap();
    w.state.find_record(&w.slug, &id).unwrap().unwrap()
}

fn set_max_writes(w: &World, n: i64) {
    w.state.settings.write().unwrap().insert("flows.max_writes_per_run".into(), json!(n));
}

fn count(w: &World, sql: &str, arg: &str) -> i64 {
    let (sql, arg) = (sql.to_string(), arg.to_string());
    w.db.run(move |c| c.query_one(&sql, &[&arg]).unwrap().get(0))
}

fn three_notes(w: &World) -> Vec<ProcessStep> {
    vec![log(w, "a", "{trigger.name}"), log(w, "b", "\"second\""), log(w, "c", "\"third\"")]
}

// 1. A failed run's timeline shows the failed step and its error. A retry resumes and writes each effect once.
#[tokio::test(flavor = "multi_thread")]
async fn a_failed_run_shows_its_step_and_a_retry_writes_each_effect_once() {
    let w = world().await;
    let rule = flow(&w, 1, three_notes(&w));
    install(&w, &rule);
    set_max_writes(&w, 2);
    let inst = flows::run_flow(&w.state, &rule, &trigger(&w), OWNER, "run-1");
    assert_eq!(format!("{:?}", inst.status), "Failed");

    let (s, tl) = send(&w.app, "GET", &format!("/api/v1/flows/runs/{}", inst.id), OWNER, None).await;
    assert_eq!(s, StatusCode::OK, "{tl}");
    let steps = tl["steps"].as_array().unwrap();
    assert_eq!(steps.iter().map(|s| s["step"].as_str().unwrap()).collect::<Vec<_>>(), ["a", "b", "c"], "{tl}");
    assert_eq!(steps[0]["outcome"], "done");
    assert!(steps[0]["effects"][0]["sentence"].as_str().unwrap().contains("created 1 record in Housing / Logs"), "{tl}");
    assert!(steps[0]["effects"][0]["records"][0]["id"].is_string(), "{tl}");
    assert_eq!(steps[2]["outcome"], "failed");
    assert!(tl["first_error"]["message"].as_str().unwrap().contains("the limit is 2 writes per run"), "{tl}");
    assert_eq!(tl["first_error"]["step"], "c");

    let (s, runs) = send(&w.app, "GET", "/api/v1/flows/open-file/runs?status=Failed", OWNER, None).await;
    assert_eq!(s, StatusCode::OK, "{runs}");
    let row = &runs["runs"][0];
    assert_eq!(row["instance_id"], inst.id);
    assert_eq!(row["status"], "Failed");
    assert_eq!(row["started_by"], OWNER);
    assert_eq!(row["writes"], 2);
    assert!(row["trigger"].as_str().unwrap().contains("created"), "{row}");

    set_max_writes(&w, 10);
    let (s, body) = send(&w.app, "POST", &format!("/api/v1/flows/runs/{}/retry", inst.id), OWNER, Some(json!({ "reason": "The write limit was raised" }))).await;
    assert_eq!(s, StatusCode::OK, "{body}");
    assert_eq!(body["status"], "Completed", "{body}");
    assert_eq!(count(&w, "SELECT COUNT(*) FROM flow_effects WHERE instance_id = $1", &inst.id), 3, "each effect once");
    assert_eq!(count(&w, "SELECT COUNT(*) FROM dataset_records WHERE app_slug = $1 AND table_id = 'logs'", &w.slug), 3, "three notes, not five");
    assert_eq!(count(&w, "SELECT COUNT(*) FROM governance_ledger WHERE decision_type = $1", "FlowRunRetried"), 1, "the retry is on the ledger");

    let (_, tl) = send(&w.app, "GET", &format!("/api/v1/flows/runs/{}", inst.id), OWNER, None).await;
    assert_eq!(tl["status"], "Completed", "{tl}");
    assert!(tl["retries"].as_array().is_some_and(|r| r.len() == 1), "{tl}");
}

// 2. A run that failed outside its grants waits for a new version.
#[tokio::test(flavor = "multi_thread")]
async fn a_grant_failure_waits_for_a_new_version() {
    let w = world().await;
    let mut rule = flow(&w, 1, three_notes(&w));
    rule.grants.clear();
    install(&w, &rule);
    let inst = flows::run_flow(&w.state, &rule, &trigger(&w), OWNER, "run-g");
    assert!(inst.log.iter().any(|l| l.contains("flow_grant_exceeded")), "{:?}", inst.log);

    let (s, body) = send(&w.app, "POST", &format!("/api/v1/flows/runs/{}/retry", inst.id), OWNER, Some(json!({ "reason": "try again" }))).await;
    assert_eq!(s, StatusCode::CONFLICT, "{body}");
    assert!(body.to_string().contains("new version"), "{body}");

    install(&w, &flow(&w, 2, three_notes(&w)));
    let (s, body) = send(&w.app, "POST", &format!("/api/v1/flows/runs/{}/retry", inst.id), OWNER, Some(json!({ "reason": "version 2 holds the grants" }))).await;
    assert_eq!(s, StatusCode::OK, "{body}");
    assert_eq!(body["status"], "Completed", "{body}");
}

// 3. The timeline and the run list hold no record values.
#[tokio::test(flavor = "multi_thread")]
async fn the_timeline_holds_no_values() {
    let w = world().await;
    let rule = flow(&w, 1, three_notes(&w));
    install(&w, &rule);
    let inst = flows::run_flow(&w.state, &rule, &trigger(&w), OWNER, "run-v");
    assert_eq!(format!("{:?}", inst.status), "Completed");
    let (s, tl) = send(&w.app, "GET", &format!("/api/v1/flows/runs/{}", inst.id), OWNER, None).await;
    assert_eq!(s, StatusCode::OK, "{tl}");
    let (_, runs) = send(&w.app, "GET", "/api/v1/flows/open-file/runs", OWNER, None).await;
    for body in [&tl, &runs] {
        let text = body.to_string();
        assert!(!text.contains(SECRET) && !text.contains("second") && !text.contains("third"), "no value appears: {text}");
    }
    let events: Vec<Value> = {
        let id = inst.id.clone();
        w.db.run(move |c| c.query("SELECT detail FROM process_events WHERE instance_id = $1", &[&id]).unwrap().iter().map(|r| r.get(0)).collect())
    };
    assert!(!events.is_empty());
    assert!(!Value::Array(events).to_string().contains(SECRET), "the stored events hold no value either");
}

// 4. Comparing two versions marks the changed step as changed, in text.
#[tokio::test(flavor = "multi_thread")]
async fn comparing_two_versions_marks_the_changed_step() {
    let w = world().await;
    let v1 = flow(&w, 0, vec![log(&w, "a", "\"opened\""), log(&w, "b", "\"kept\"")]);
    processes::save_rule(OWNER, &w.slug, v1, None, "First version", &w.state).expect("v1");
    let v2 = flow(&w, 0, vec![log(&w, "a", "\"opened again\""), log(&w, "b", "\"kept\""), log(&w, "d", "\"new\"")]);
    processes::save_rule(OWNER, &w.slug, v2, None, "Second version", &w.state).expect("v2");

    let (s, list) = send(&w.app, "GET", "/api/v1/flows/open-file/versions", OWNER, None).await;
    assert_eq!(s, StatusCode::OK, "{list}");
    let versions = list["versions"].as_array().unwrap();
    assert_eq!(versions.len(), 2, "{list}");
    assert_eq!(versions[0]["author"], OWNER);

    let (s, cmp) = send(&w.app, "GET", "/api/v1/flows/open-file/versions/compare?from=1&to=2", OWNER, None).await;
    assert_eq!(s, StatusCode::OK, "{cmp}");
    let by = |id: &str| cmp["steps"].as_array().unwrap().iter().find(|s| s["step"] == id).cloned().unwrap_or_else(|| panic!("{id}: {cmp}"));
    assert_eq!(by("a")["change"], "changed");
    assert!(by("a")["sentence"].as_str().unwrap().contains("Step 1 is changed"), "{cmp}");
    assert_eq!(by("b")["change"], "same");
    assert_eq!(by("d")["change"], "added");
    assert!(!cmp["sentences"].as_array().unwrap().is_empty(), "{cmp}");
}

// 5. A retry by an editor who is not an owner or admin is refused.
#[tokio::test(flavor = "multi_thread")]
async fn an_editor_may_not_retry() {
    let w = world().await;
    let rule = flow(&w, 1, three_notes(&w));
    install(&w, &rule);
    set_max_writes(&w, 1);
    let inst = flows::run_flow(&w.state, &rule, &trigger(&w), OWNER, "run-e");
    let (s, body) = send(&w.app, "POST", &format!("/api/v1/flows/runs/{}/retry", inst.id), EDITOR, Some(json!({ "reason": "please" }))).await;
    assert_eq!(s, StatusCode::FORBIDDEN, "{body}");
    let (s, body) = send(&w.app, "POST", &format!("/api/v1/flows/runs/{}/retry", inst.id), OWNER, Some(json!({ "reason": "" }))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "a reason is required: {body}");
}
