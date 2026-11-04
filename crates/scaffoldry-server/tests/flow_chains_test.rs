//! Workflows phase 3: triggers, chaining, and safety.
//!
//! Flows start on a schedule, when another flow calls them, when a flow completes, and when a process step is decided.
//! Calls are typed. A cycle of flows is refused when a version is saved, and a chain stops at depth 5. A flow that fails
//! 20 runs in a row is turned off until a new version is saved, and a flow starts at most `flows.max_runs_per_minute`.

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use common::TestDb;
use http_body_util::BodyExt;
use scaffoldry_core::approver::ApproverSpec;
use scaffoldry_core::workflow::{FlowInput, InputKind, ScheduleEvery, Target};
use scaffoldry_core::{ActionType, AutomationRule, ProcessStatus, ProcessStep, StepKind, TriggerEvent};
use scaffoldry_engine::{AppManifest, HostRouter};
use scaffoldry_server::build_app_with_state;
use scaffoldry_server::service::identity::issue_test_token_and_user;
use scaffoldry_server::service::{flow_chains, flows, processes};
use scaffoldry_server::state::{DatasetRecord, ServerState};
use serde_json::{json, Map, Value};
use std::sync::Arc;
use tower::ServiceExt;

const ADMIN: &str = "jordan.lee@state.edu";
const OWNER: &str = "einstein@physics.state.edu";
const CO_OWNER: &str = "bohr@physics.state.edu";
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
    let (s, b) = send(&app, "POST", &format!("/api/v1/workspaces/{WORKSPACE}/apps"), OWNER, Some(manifest)).await;
    assert_eq!(s, StatusCode::CREATED, "{b}");
    issue_test_token_and_user(CO_OWNER);
    let (s, b) = send(&app, "POST", &format!("/api/v1/workspaces/{WORKSPACE}/collaborators"), ADMIN, Some(json!({ "eppn": CO_OWNER, "role": "owner" }))).await;
    assert!(s.is_success(), "{b}");
    World { db, state, app, slug }
}

fn manifest(w: &World) -> AppManifest {
    w.state.engine.read().unwrap().resolve_by_slug(&w.slug).cloned().expect("app")
}

fn step(id: &str, action: ActionType) -> ProcessStep {
    ProcessStep { id: id.into(), when: vec![], kind: StepKind::Service { action } }
}

fn log(w: &World, id: &str, note: &str) -> ProcessStep {
    step(id, ActionType::CreateRecord { id: id.into(), app: w.slug.clone(), table: "logs".into(), fields: vec![("note".into(), note.into())] })
}

fn start(id: &str, flow: &str, inputs: Vec<(&str, &str)>) -> ProcessStep {
    step(id, ActionType::StartFlow { flow_id: flow.into(), inputs: inputs.into_iter().map(|(n, e)| (n.to_string(), e.to_string())).collect() })
}

fn called(inputs: Vec<(&str, InputKind)>) -> TriggerEvent {
    TriggerEvent::Called { inputs: inputs.into_iter().map(|(n, k)| FlowInput { name: n.into(), kind: k }).collect() }
}

fn flow(w: &World, id: &str, name: &str, trigger: TriggerEvent, steps: Vec<ProcessStep>) -> AutomationRule {
    let mut rule = AutomationRule {
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
    };
    flows::derive(&w.state, &manifest(w), &mut rule);
    rule
}

fn install(w: &World, rule: &AutomationRule) {
    w.state.automations.write().unwrap().entry(w.slug.clone()).or_default().push(rule.clone());
}

fn logs(w: &World) -> Vec<Value> {
    let slug = w.slug.clone();
    w.db.run(move |c| c.query("SELECT data FROM dataset_records WHERE app_slug = $1 AND table_id = 'logs' ORDER BY created_at", &[&slug]).unwrap().iter().map(|r| r.get(0)).collect())
}

async fn application(w: &World, amount: i64) -> String {
    let (s, b) = send(&w.app, "POST", &format!("/api/v1/apps/{}/records", w.slug), OWNER, Some(json!({ "data": { "_table_id": "applications", "name": "Ada Okoro", "amount": amount } }))).await;
    assert!(s.is_success(), "{b}");
    b["id"].as_str().or_else(|| b["record"]["id"].as_str()).unwrap().to_string()
}

fn trigger_record(w: &World) -> DatasetRecord {
    let id = uuid::Uuid::new_v4().to_string();
    let rec = DatasetRecord {
        id: id.clone(),
        app_slug: w.slug.clone(),
        data: json!({ "_table_id": "applications", "name": "Ada", "amount": 10 }),
        ceds_mapping: json!({}),
        is_ferpa_sensitive: false,
        created_at: chrono::Utc::now().to_rfc3339(),
        created_by: Some(OWNER.into()),
        version: 1,
    };
    let ctx = scaffoldry_server::repository::WriteCtx::system("test");
    w.state.insert_record(&rec, &ctx).unwrap();
    w.state.find_record(&w.slug, &id).unwrap().unwrap()
}

fn instances_of(w: &World, rule: &str) -> Vec<Value> {
    let rule = rule.to_string();
    w.db.run(move |c| c.query("SELECT instance_json FROM process_instances WHERE rule_id = $1", &[&rule]).unwrap().iter().map(|r| r.get(0)).collect())
}

// 1. A daily 07:00 schedule in the institution's zone starts one run per day, once, even with two scheduler threads.
#[tokio::test(flavor = "multi_thread")]
async fn a_daily_schedule_starts_one_run_per_day() {
    let w = world().await;
    w.state.settings.write().unwrap().insert("platform.timezone".into(), json!("America/New_York"));
    let f = flow(&w, "morning", "Morning log", TriggerEvent::Schedule { every: ScheduleEvery::Daily, at: "07:00".into() }, vec![log(&w, "write", "\"tick\"")]);
    assert!(processes::validate_rule(&w.state, &manifest(&w), &f).is_empty(), "{:?}", processes::validate_rule(&w.state, &manifest(&w), &f));
    install(&w, &f);

    // 07:30 in New York on a November day is 12:30 UTC.
    let now = chrono::DateTime::parse_from_rfc3339("2026-11-05T12:30:00Z").unwrap().with_timezone(&chrono::Utc);
    let threads: Vec<_> = (0..2)
        .map(|_| {
            let state = w.state.clone();
            std::thread::spawn(move || flow_chains::fire_schedules(&state, now).expect("fire"))
        })
        .collect();
    let started: usize = threads.into_iter().map(|t| t.join().unwrap()).sum();
    assert_eq!(started, 1, "two scheduler threads start one run");
    assert_eq!(logs(&w).len(), 1);
    assert_eq!(flow_chains::fire_schedules(&w.state, now + chrono::Duration::hours(1)).unwrap(), 0, "not again the same day");
    // 06:30 local the next day: not due yet. 07:05: due.
    assert_eq!(flow_chains::fire_schedules(&w.state, now + chrono::Duration::hours(23)).unwrap(), 0);
    assert_eq!(flow_chains::fire_schedules(&w.state, now + chrono::Duration::hours(23) + chrono::Duration::minutes(35)).unwrap(), 1, "the next day's run");
    assert_eq!(logs(&w).len(), 2);
}

// 2. A flow that starts a `Called` flow passes typed inputs. A wrong type fails validation.
#[tokio::test(flavor = "multi_thread")]
async fn a_called_flow_gets_typed_inputs() {
    let w = world().await;
    let callee = flow(&w, "record-award", "Record award", called(vec![("amount", InputKind::Number), ("applicant", InputKind::RecordId)]), vec![log(&w, "write", "{input.amount}")]);
    assert!(processes::validate_rule(&w.state, &manifest(&w), &callee).is_empty(), "{:?}", processes::validate_rule(&w.state, &manifest(&w), &callee));
    install(&w, &callee);
    let caller = flow(&w, "on-apply", "On application", TriggerEvent::RecordCreated, vec![start("call", "record-award", vec![("amount", "{trigger.amount}"), ("applicant", "{trigger.id}")])]);
    assert!(processes::validate_rule(&w.state, &manifest(&w), &caller).is_empty(), "{:?}", processes::validate_rule(&w.state, &manifest(&w), &caller));
    install(&w, &caller);

    application(&w, 500).await;
    let written = logs(&w);
    assert_eq!(written.len(), 1, "{written:?} caller {:?} callee {:?}", instances_of(&w, "on-apply"), instances_of(&w, "record-award"));
    assert_eq!(written[0]["note"].as_f64(), Some(500.0), "the input reached the called flow: {written:?}");

    let wrong = flow(&w, "wrong", "Wrong call", TriggerEvent::RecordCreated, vec![start("call", "record-award", vec![("amount", "\"a lot\""), ("applicant", "{trigger.id}")])]);
    let problems = processes::validate_rule(&w.state, &manifest(&w), &wrong);
    assert!(problems.iter().any(|p| p.contains("amount") && p.contains("Number")), "{problems:?}");
    let missing = flow(&w, "missing", "Missing input", TriggerEvent::RecordCreated, vec![start("call", "record-award", vec![("amount", "1")])]);
    assert!(processes::validate_rule(&w.state, &manifest(&w), &missing).iter().any(|p| p.contains("applicant")), "a missing input is named");
}

// 3. A cycle across three flows fails validation with the path in words.
#[tokio::test(flavor = "multi_thread")]
async fn a_cycle_of_flows_fails_with_the_path() {
    let w = world().await;
    install(&w, &flow(&w, "open", "Open aid file", called(vec![]), vec![start("next", "notify", vec![])]));
    install(&w, &flow(&w, "notify", "Notify housing", called(vec![]), vec![start("next", "close", vec![])]));
    let closing = flow(&w, "close", "Close file", called(vec![]), vec![start("next", "open", vec![])]);
    let problems = processes::validate_rule(&w.state, &manifest(&w), &closing);
    assert!(
        problems.iter().any(|p| p.contains("Close file starts Open aid file, which starts Notify housing, which starts Close file.")),
        "{problems:?}"
    );
}

// 4. A chain of six fails the sixth, with the chain recorded.
#[tokio::test(flavor = "multi_thread")]
async fn a_chain_of_six_fails_the_sixth() {
    let w = world().await;
    for n in (2..=6).rev() {
        let mut steps = vec![log(&w, "write", &format!("\"step {n}\""))];
        if n < 6 {
            steps.push(start("next", &format!("c{}", n + 1), vec![]));
        }
        install(&w, &flow(&w, &format!("c{n}"), &format!("Chain {n}"), called(vec![]), steps));
    }
    install(&w, &flow(&w, "c1", "Chain 1", TriggerEvent::RecordCreated, vec![log(&w, "write", "\"step 1\""), start("next", "c2", vec![])]));
    application(&w, 1).await;
    assert_eq!(logs(&w).len(), 5, "the first five ran: {:?}", logs(&w));
    let sixth = instances_of(&w, "c6");
    assert_eq!(sixth.len(), 1, "{sixth:?}");
    assert_eq!(sixth[0]["status"], "Failed");
    let log_text = sixth[0]["log"].to_string();
    assert!(log_text.contains("Chain 1 > Chain 2 > Chain 3 > Chain 4 > Chain 5 > Chain 6"), "{log_text}");
}

// 5. Twenty consecutive failures turn the flow off and tell its owners. Saving a new version turns it on again.
#[tokio::test(flavor = "multi_thread")]
async fn twenty_failures_trip_the_breaker() {
    let w = world().await;
    let mut broken = flow(&w, "fragile", "Fragile flow", TriggerEvent::RecordCreated, vec![log(&w, "write", "\"x\"")]);
    broken.grants.clear();
    let trigger = trigger_record(&w);
    for n in 0..20 {
        let inst = flows::run_flow(&w.state, &broken, &trigger, OWNER, &format!("run-{n}"));
        assert_eq!(inst.status, ProcessStatus::Failed);
    }
    let (failures, tripped) = flows::breaker(&w.state, "fragile");
    assert_eq!(failures, 20);
    assert!(tripped, "turned off after twenty");
    let told: i64 = w.db.run(|c| c.query_one("SELECT COUNT(*) FROM notifications WHERE lower(recipient) = lower($1) AND kind = 'process_notice'", &[&OWNER]).unwrap().get(0));
    assert!(told >= 1, "the owners are told");
    let next = flows::run_flow(&w.state, &broken, &trigger, OWNER, "run-20");
    assert!(next.log.join(" ").contains("turned off"), "{:?}", next.log);

    let fixed = flow(&w, "fragile", "Fragile flow", TriggerEvent::RecordCreated, vec![log(&w, "write", "\"x\"")]);
    processes::save_rule(OWNER, &w.slug, fixed.clone(), None, "Fixed the grants", &w.state).expect("saved");
    assert_eq!(flows::breaker(&w.state, "fragile"), (0, false), "a new version clears the breaker");
    assert_eq!(flows::run_flow(&w.state, &fixed, &trigger, OWNER, "run-21").status, ProcessStatus::Completed);
}

// 6. `ProcessDecided` and `FlowCompleted` start the right flows with the right record.
#[tokio::test(flavor = "multi_thread")]
async fn decisions_and_completions_start_the_flows_that_wait_for_them() {
    let w = world().await;
    let review = AutomationRule {
        id: "review".into(),
        app_slug: w.slug.clone(),
        name: "Review".into(),
        description: String::new(),
        enabled: true,
        trigger: TriggerEvent::RecordCreated,
        predicates: vec![],
        actions: vec![],
        steps: vec![ProcessStep {
            id: "decide".into(),
            when: vec![],
            kind: StepKind::UserTask { role: String::new(), prompt: "Decide the application".into(), approve: vec![], reject: vec![], approver: Some(ApproverSpec::Person { eppn: CO_OWNER.into() }), due: None, remind_every: None, on_due: vec![], on_fail: vec![], decision: Default::default(), reject_rule: Default::default(), kind: Default::default(), show: vec![], fill: vec![], require: vec![], decline_reason: false },
        }],
        version: 1,
        home_workspace: None,
        grants: vec![],
    };
    install(&w, &review);
    let fund = flow(
        &w,
        "fund",
        "Fund it",
        TriggerEvent::ProcessDecided { app: w.slug.clone(), rule: "review".into(), decision: "approve".into() },
        vec![step("mark", ActionType::UpdateRecord { target: Target::Trigger, fields: vec![("status".into(), "\"Funded\"".into())] })],
    );
    install(&w, &fund);
    let refuse = flow(&w, "refuse", "Refuse it", TriggerEvent::ProcessDecided { app: w.slug.clone(), rule: "review".into(), decision: "reject".into() }, vec![log(&w, "write", "\"refused\"")]);
    install(&w, &refuse);
    let after = flow(&w, "after", "After funding", TriggerEvent::FlowCompleted { flow_id: "fund".into(), status: "Completed".into() }, vec![log(&w, "write", "{trigger.name}")]);
    install(&w, &after);

    let id = application(&w, 100).await;
    let (s, b) = send(&w.app, "POST", &format!("/api/v1/apps/{}/processes/review:{id}/decide", w.slug), CO_OWNER, Some(json!({ "decision": "approve" }))).await;
    assert!(s.is_success(), "{b}");
    assert_eq!(w.state.find_record(&w.slug, &id).unwrap().unwrap().data["status"], "Funded", "the approve flow ran on the decided record");
    let written = logs(&w);
    assert_eq!(written.len(), 1, "only the completion flow wrote a log, not the reject flow: {written:?}");
    assert_eq!(written[0]["note"], "Ada Okoro", "with the same record");
}

// 7. A burst past the rate cap starts no more runs and records it.
#[tokio::test(flavor = "multi_thread")]
async fn a_burst_past_the_rate_cap_starts_nothing_more() {
    let w = world().await;
    w.state.settings.write().unwrap().insert("flows.max_runs_per_minute".into(), json!(3));
    let f = flow(&w, "busy", "Busy flow", TriggerEvent::RecordCreated, vec![log(&w, "write", "\"busy\"")]);
    let trigger = trigger_record(&w);
    let runs: Vec<_> = (0..5).map(|n| flows::run_flow(&w.state, &f, &trigger, OWNER, &format!("burst-{n}"))).collect();
    assert_eq!(runs.iter().filter(|r| r.status == ProcessStatus::Completed).count(), 3);
    let limited: Vec<_> = runs.iter().filter(|r| r.log.join(" ").contains("rate_limited")).collect();
    assert_eq!(limited.len(), 2, "{runs:?}");
    assert_eq!(logs(&w).len(), 3, "the limited runs wrote nothing");
    assert_eq!(flows::breaker(&w.state, "busy").0, 0, "a limited run is not a failure of the flow");
    let _ = Map::<String, Value>::new();
}
