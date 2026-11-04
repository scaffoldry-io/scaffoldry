//! Workflows phase 4: tasks of more than one kind, and the task grant that goes with an open step.
//!
//! A `Complete` task asks its assignee to fill some fields of one record and press Done, or to decline. While the step
//! is open, the assignee holds a task grant on that record alone: read of `show` and `fill`, write of `fill`. The grant
//! gives them nothing else of the app, and it closes in the transaction that closes the step.
//!
//! The world: a financial aid app in a Physics workspace. The aid officer has no rights to the app. The task asks them
//! for the award letter, and lets them see the program and the status.

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::Router;
use chrono::{Duration, Utc};
use common::TestDb;
use http_body_util::BodyExt;
use scaffoldry_core::approver::ApproverSpec;
use scaffoldry_core::standards::eduperson::{EduPersonAffiliation, EduPersonIdentity};
use scaffoldry_core::workflow::{DueAction, DurationSpec, TaskKind};
use scaffoldry_core::{ActionType, AutomationRule, ProcessStatus, ProcessStep, StepKind, TriggerEvent};
use scaffoldry_server::build_app_with_state;
use scaffoldry_server::service::identity::issue_test_token_and_user;
use scaffoldry_server::state::{CollaboratorRecord, DatasetRecord, ServerState, WorkspaceRecord};
use serde_json::{json, Value};
use std::sync::Arc;
use tower::ServiceExt;
use uuid::Uuid;

const ROOT_UNIT: &str = "00000000-0000-0000-0000-000000000001";
const APP: &str = "task-grants-app";
const WS: &str = "ws-task-grants";

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

fn person(prefix: &str) -> String {
    format!("{prefix}-{}@state.edu", &Uuid::new_v4().to_string()[..8])
}

struct World {
    db: TestDb,
    app: Router,
    state: Arc<ServerState>,
    student: String,
    officer: String,
    officer_t: String,
    dean: String,
    dean_t: String,
}

fn field(name: &str, label: &str, sensitive: bool) -> Value {
    json!({ "name": name, "label": label, "field_type": "Text", "required": false, "ferpa_sensitive": sensitive })
}

fn manifest() -> Value {
    json!({
        "slug": APP, "title": "Financial aid", "description": "", "organization_code": "PHYS", "department": "physics", "workspace_id": WS, "views": [],
        "tables": [{ "id": "applications", "name": "Applications", "slug": "applications", "primary_field": "title", "fields": [
            field("title", "Title", false), field("program", "Program", false), field("status", "Status", false),
            field("award_letter", "Award letter", false), field("ssn", "Social security number", true)
        ] }],
    })
}

impl World {
    /// A person the platform knows. `member` makes them an editor of the workspace.
    async fn user(app: &Router, state: &ServerState, admin: &str, who: &str, member: bool) -> String {
        let (s, _) = send(app, "POST", "/api/v1/admin/users", admin, Some(json!({
            "userName": who, "name": "Test Person", "email": who, "affiliation": "staff",
            "department": "Aid", "title": "Officer", "reason": "Fixture"
        }))).await;
        assert_eq!(s, StatusCode::CREATED, "creating a test user failed");
        if member {
            state.collaborators.write().unwrap().entry(WS.to_string()).or_default().push(CollaboratorRecord {
                id: format!("collab-{who}"),
                workspace_id: WS.to_string(),
                eppn: who.to_string(),
                name: who.to_string(),
                role: "editor".to_string(),
                scoped_affiliation: "staff".to_string(),
                department: "physics".to_string(),
                added_at: String::new(),
                app_slug: String::new(),
            });
        }
        issue_test_token_and_user(who)
    }

    /// The award-letter task, assigned to the officer.
    fn rule(&self, id: &str, show: &[&str], on_due: Vec<DueAction>) -> AutomationRule {
        AutomationRule {
            version: 0,
            home_workspace: None,
            grants: vec![],
            id: id.to_string(),
            app_slug: APP.to_string(),
            name: format!("Award letter {id}"),
            description: String::new(),
            enabled: true,
            trigger: TriggerEvent::RecordCreated,
            predicates: vec![],
            actions: vec![],
            steps: vec![ProcessStep {
                id: "provide-letter".to_string(),
                when: vec![],
                kind: StepKind::UserTask {
                    role: String::new(),
                    prompt: "Provide the award letter".to_string(),
                    approve: vec![ActionType::UpdateRecordStatus { new_status: "Awarded".into() }],
                    reject: vec![ActionType::UpdateRecordStatus { new_status: "Declined".into() }],
                    approver: Some(ApproverSpec::Person { eppn: self.officer.clone() }),
                    due: (!on_due.is_empty()).then_some(DurationSpec { days: 2 }),
                    remind_every: None,
                    on_due,
                    on_fail: vec![],
                    decision: Default::default(),
                    reject_rule: Default::default(),
                    kind: TaskKind::Complete,
                    show: show.iter().map(|s| s.to_string()).collect(),
                    fill: vec!["award_letter".to_string()],
                    require: vec!["award_letter".to_string()],
                    decline_reason: true,
                },
            }],
        }
    }

    /// A record the student submitted, with the rule run on it as the student. Returns `(instance, record)`.
    fn start(&self, rule: AutomationRule) -> (String, String) {
        let rule_id = rule.id.clone();
        self.state.automations.write().unwrap().insert(APP.to_string(), vec![rule]);
        let record_id = self.record("Fall aid");
        let data = json!({ "id": record_id, "_table_id": "applications", "title": "Fall aid", "program": "Physics BSc", "status": "Submitted", "ssn": "123-45-6789" });
        let identity = EduPersonIdentity { eppn: self.student.clone(), realm: "state.edu".into(), affiliations: vec![EduPersonAffiliation::Student] };
        let mut applied = vec![];
        scaffoldry_server::service::records::run_automations(
            &self.state,
            APP,
            TriggerEvent::RecordCreated,
            &data,
            &scaffoldry_server::service::records::Actor { identity: &identity, department: "Physics" },
            0,
            &mut applied,
        );
        let id = format!("{rule_id}:{record_id}");
        assert!(self.state.repository.get_process_instance(&id).unwrap().is_some(), "the rule started an instance");
        (id, record_id)
    }

    fn record(&self, title: &str) -> String {
        let id = format!("rec-{}", &Uuid::new_v4().simple().to_string()[..8]);
        self.state
            .put_record(
                &DatasetRecord {
                    id: id.clone(),
                    app_slug: APP.to_string(),
                    data: json!({ "_table_id": "applications", "title": title, "program": "Physics BSc", "status": "Submitted", "ssn": "123-45-6789" }),
                    ceds_mapping: Default::default(),
                    is_ferpa_sensitive: false,
                    created_at: String::new(),
                    created_by: Some(self.student.clone()),
                    version: 1,
                },
                &scaffoldry_server::repository::WriteCtx::system("test fixture"),
            )
            .unwrap();
        id
    }

    async fn get(&self, token: &str, record: &str) -> (StatusCode, Value) {
        send(&self.app, "GET", &format!("/api/v1/apps/{APP}/records/{record}"), token, None).await
    }

    async fn put(&self, token: &str, record: &str, data: Value) -> (StatusCode, Value) {
        let version = self.state.find_record(APP, record).unwrap().unwrap().version;
        send(&self.app, "PATCH", &format!("/api/v1/apps/{APP}/records/{record}"), token, Some(json!({ "data": data, "version": version }))).await
    }

    async fn decide(&self, token: &str, instance: &str, decision: &str, comment: Option<&str>) -> (StatusCode, Value) {
        let mut body = json!({ "decision": decision });
        if let Some(c) = comment {
            body["comment"] = json!(c);
        }
        send(&self.app, "POST", &format!("/api/v1/apps/{APP}/processes/{instance}/decide"), token, Some(body)).await
    }

    /// `(person, state)` of each of the instance's task grants.
    fn grants(&self, instance: &str) -> Vec<(String, String)> {
        let instance = instance.to_string();
        self.db.run(move |c| {
            c.query("SELECT person, state FROM task_grants WHERE instance_id = $1 ORDER BY person", &[&instance]).unwrap().iter().map(|r| (r.get(0), r.get(1))).collect()
        })
    }
}

async fn world() -> World {
    let db = TestDb::new();
    let admin = issue_test_token_and_user("jordan.lee@state.edu");
    let state = db.state();
    let app = build_app_with_state(state.clone()).expect("router");
    let (s, b) = send(&app, "POST", "/api/v1/orgs", &admin, Some(json!({ "name": "Physics", "code": "PHYS", "org_type": "Department", "parent_id": ROOT_UNIT }))).await;
    assert_eq!(s, StatusCode::CREATED, "{b}");
    let ws: WorkspaceRecord = serde_json::from_value(json!({
        "id": WS, "name": "Financial aid", "code": "FAID", "organization": "State", "visibility": "restricted", "organization_id": b["id"],
    }))
    .unwrap();
    state.persist_workspace(ws).unwrap();
    state.engine.write().unwrap().register_manifest(serde_json::from_value(manifest()).unwrap()).unwrap();
    let (student, officer, dean) = (person("student"), person("officer"), person("dean"));
    World::user(&app, &state, &admin, &student, true).await;
    let officer_t = World::user(&app, &state, &admin, &officer, false).await;
    let dean_t = World::user(&app, &state, &admin, &dean, false).await;
    World { db, app, state, student, officer, officer_t, dean, dean_t }
}

// 1. A `Complete` task assigned to a person with no rights to the app lets them read the `show` fields and write the
// `fill` fields of that one record, and nothing else. A list, a search, another record, and an unlisted field are refused.
#[tokio::test(flavor = "multi_thread")]
async fn a_task_grant_opens_one_record_and_only_its_listed_fields() {
    let w = world().await;
    let (_, rec) = w.start(w.rule("aid", &["program", "status"], vec![]));
    let other = w.record("Spring aid");

    let (s, b) = w.get(&w.officer_t, &rec).await;
    assert_eq!(s, StatusCode::OK, "the officer reads the record through the task: {b}");
    let data = &b["data"];
    assert_eq!(data["program"], "Physics BSc");
    assert_eq!(data["status"], "Submitted");
    assert!(data.get("award_letter").is_some() || data.as_object().is_some(), "{b}");
    for unlisted in ["title", "ssn"] {
        assert!(data.get(unlisted).is_none(), "'{unlisted}' is not on the task: {b}");
    }

    let (s, b) = w.put(&w.officer_t, &rec, json!({ "award_letter": "Letter 2026-114" })).await;
    assert_eq!(s, StatusCode::OK, "the officer fills the listed field: {b}");
    assert_eq!(w.state.find_record(APP, &rec).unwrap().unwrap().data["award_letter"], "Letter 2026-114");
    assert_eq!(w.state.find_record(APP, &rec).unwrap().unwrap().data["title"], "Fall aid", "the fields the task did not name are kept");

    let (s, _) = w.put(&w.officer_t, &rec, json!({ "status": "Awarded" })).await;
    assert!(s == StatusCode::FORBIDDEN || s == StatusCode::BAD_REQUEST, "a field the task does not fill is refused: {s}");
    assert_eq!(w.state.find_record(APP, &rec).unwrap().unwrap().data["status"], "Submitted");

    assert!(!w.get(&w.officer_t, &other).await.0.is_success(), "another record");
    assert!(!send(&w.app, "GET", &format!("/api/v1/apps/{APP}/records"), &w.officer_t, None).await.0.is_success(), "the list");
    assert!(!send(&w.app, "GET", &format!("/api/v1/apps/{APP}/records?search=Fall"), &w.officer_t, None).await.0.is_success(), "a search");
    assert!(!send(&w.app, "GET", &format!("/api/v1/apps/{APP}"), &w.officer_t, None).await.0.is_success(), "the app");
    assert!(!w.put(&w.officer_t, &other, json!({ "award_letter": "x" })).await.0.is_success(), "a write to another record");

    // The proposal's change list says what the task asks and what it shows.
    let words = scaffoldry_core::workflow::describe_rule(&w.rule("words", &["program", "status"], vec![])).join(" ");
    assert!(words.contains(&format!("Asks {} to provide award_letter on the record. They can see program and status of that record only, for as long as the task is open.", w.officer)), "{words}");
}

// 2. `Done` with a `require` field blank is refused with the field named. With it filled, the step completes and the
// grant closes in the same transaction.
#[tokio::test(flavor = "multi_thread")]
async fn done_needs_the_required_fields_and_closes_the_grant() {
    let w = world().await;
    let (id, rec) = w.start(w.rule("done", &["program"], vec![]));
    assert_eq!(w.grants(&id), vec![(w.officer.clone(), "open".to_string())]);

    let (s, b) = w.decide(&w.officer_t, &id, "done", None).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{b}");
    assert!(b.to_string().contains("award_letter"), "the blank field is named: {b}");

    assert_eq!(w.put(&w.officer_t, &rec, json!({ "award_letter": "Letter 7" })).await.0, StatusCode::OK);
    let (s, b) = w.decide(&w.officer_t, &id, "done", None).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    let inst = w.state.repository.get_process_instance(&id).unwrap().unwrap();
    assert_eq!(inst.status, ProcessStatus::Completed);
    assert_eq!(w.state.find_record(APP, &rec).unwrap().unwrap().data["status"], "Awarded", "done runs the step's approve actions");
    assert_eq!(w.grants(&id), vec![(w.officer.clone(), "closed".to_string())]);
    assert!(!w.get(&w.officer_t, &rec).await.0.is_success(), "a closed grant reads nothing");
    assert!(!w.put(&w.officer_t, &rec, json!({ "award_letter": "Changed" })).await.0.is_success(), "a closed grant writes nothing");
}

// 3. `Decline` takes the decline outcome and the flow continues on that branch.
#[tokio::test(flavor = "multi_thread")]
async fn decline_takes_the_decline_outcome() {
    let w = world().await;
    let (id, rec) = w.start(w.rule("decline", &["program"], vec![]));
    assert_eq!(w.decide(&w.officer_t, &id, "decline", None).await.0, StatusCode::BAD_REQUEST, "this task needs a reason to decline");
    assert_eq!(w.decide(&w.officer_t, &id, "approve", None).await.0, StatusCode::BAD_REQUEST, "a Complete task is done or declined, not approved");

    let (s, b) = w.decide(&w.officer_t, &id, "decline", Some("The student withdrew")).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    let inst = w.state.repository.get_process_instance(&id).unwrap().unwrap();
    assert_eq!(inst.status, ProcessStatus::Rejected);
    assert_eq!(w.state.find_record(APP, &rec).unwrap().unwrap().data["status"], "Declined", "decline runs the step's reject actions");
    assert!(inst.log.iter().any(|l| l.contains("The student withdrew")), "the reason is in the log: {:?}", inst.log);

    let needle = id.clone();
    let decision: String = w.db.run(move |c| {
        c.query_one("SELECT payload->>'decision' FROM governance_ledger WHERE decision_type = 'WorkflowRuleApproved' AND payload->>'instance_id' = $1", &[&needle]).unwrap().get(0)
    });
    assert_eq!(decision, "decline", "the outcome a waiting flow matches on is decline");
    assert_eq!(w.grants(&id)[0].1, "closed");
}

// 4. A reassign or an escalation closes the old grant and opens the new one, and the old person can no longer read or
// write.
#[tokio::test(flavor = "multi_thread")]
async fn an_escalation_moves_the_grant() {
    let w = world().await;
    let escalate = vec![DueAction::Escalate { approver: ApproverSpec::Person { eppn: w.dean.clone() } }];
    let (id, rec) = w.start(w.rule("moved", &["program"], escalate));
    assert_eq!(w.get(&w.officer_t, &rec).await.0, StatusCode::OK);
    assert!(!w.get(&w.dean_t, &rec).await.0.is_success(), "the dean has no grant yet");

    assert_eq!(scaffoldry_server::service::process_timers::fire_due(&w.state, Utc::now() + Duration::days(3)).unwrap(), 1);
    let mut want = vec![(w.dean.clone(), "open".to_string()), (w.officer.clone(), "closed".to_string())];
    want.sort();
    assert_eq!(w.grants(&id), want);
    assert!(!w.get(&w.officer_t, &rec).await.0.is_success(), "the officer can no longer read");
    assert!(!w.put(&w.officer_t, &rec, json!({ "award_letter": "Late" })).await.0.is_success(), "or write");
    let (s, b) = w.get(&w.dean_t, &rec).await;
    assert_eq!(s, StatusCode::OK, "the dean reads through the new grant: {b}");
    assert_eq!(b["data"]["program"], "Physics BSc");
}

// 5. A task showing a sensitive field raises `task_sensitivity`. The proposer cannot approve alone.
#[tokio::test(flavor = "multi_thread")]
async fn a_task_that_shows_a_sensitive_field_needs_a_second_person() {
    const OWNER: &str = "einstein@physics.state.edu";
    const WORKSPACE: &str = "ws-physics-optics";
    let db = TestDb::new();
    let state = db.state();
    state.seed_demo().expect("seed");
    let app = build_app_with_state(state.clone()).expect("router");
    let owner = issue_test_token_and_user(OWNER);
    let slug = format!("aid-{}", &Uuid::new_v4().simple().to_string()[..8]);
    let mut m = manifest();
    m["slug"] = json!(slug);
    m["workspace_id"] = json!(WORKSPACE);
    // A second person may decide in the workspace. The app holds a sensitive field, so creating it is itself a proposal,
    // which that person approves. The rule then comes on its own, so the only reason it needs a second person is the task.
    const SECOND: &str = "bohr@physics.state.edu";
    let admin = issue_test_token_and_user("jordan.lee@state.edu");
    let second = issue_test_token_and_user(SECOND);
    let (s, b) = send(&app, "POST", &format!("/api/v1/workspaces/{WORKSPACE}/collaborators"), &admin, Some(json!({ "eppn": SECOND, "role": "owner" }))).await;
    assert!(s.is_success(), "{b}");
    let tool = |token: String, name: &'static str, args: Value| {
        let app = app.clone();
        async move {
            let call = json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": { "name": name, "arguments": args } });
            send(&app, "POST", "/api/mcp", &token, Some(call)).await.1
        }
    };
    let (s, b) = send(&app, "POST", &format!("/api/v1/workspaces/{WORKSPACE}/apps"), &owner, Some(m.clone())).await;
    assert!(s.is_success(), "{b}");
    if let Some(pid) = b["proposal_id"].as_str().or_else(|| b["proposal"]["id"].as_str()) {
        let digest = tool(second.clone(), "get_proposal", json!({ "proposal_id": pid })).await["result"]["structuredContent"]["digest"].clone();
        let v = tool(second.clone(), "decide_proposal", json!({ "proposal_id": pid, "decision": "approve", "rationale": "Reviewed", "digest": digest })).await;
        assert!(v["result"]["isError"] != true && v.get("error").is_none(), "the second person approves the app: {v}");
    }

    let rule = json!({
        "id": format!("aid-task-{slug}"), "app_slug": slug, "name": "Ask for the letter", "description": "", "enabled": true,
        "trigger": "RecordCreated", "predicates": [], "actions": [], "version": 0,
        "steps": [{ "id": "letter", "when": [], "kind": { "UserTask": {
            "role": "", "prompt": "Provide the letter", "approve": [], "reject": [],
            "approver": { "Person": { "eppn": "bohr@physics.state.edu" } },
            "kind": "Complete", "show": ["program", "ssn"], "fill": ["award_letter"], "require": ["award_letter"]
        } } }]
    });
    let v = tool(owner.clone(), "propose_app_change", json!({ "workspace_id": WORKSPACE, "app_slug": slug, "rules": [rule], "summary": "Ask for the award letter" })).await;
    assert!(v["result"]["isError"] != true, "{v}");
    let made = &v["result"]["structuredContent"];
    let pid = made["proposal_id"].as_str().unwrap_or_else(|| panic!("{v}")).to_string();
    let text = made.to_string();
    assert!(text.contains("task_sensitivity"), "the check is named: {made}");
    let checks = made["checks"].as_array().cloned().unwrap_or_default();
    let task = checks.iter().find(|c| c["name"] == "task_sensitivity").unwrap_or_else(|| panic!("{made}"));
    assert_eq!(task["outcome"], "fail", "{task}");

    for c in &checks {
        if c["name"] != "task_sensitivity" {
            assert_ne!(c["outcome"], "fail", "only the task needs a second person here: {c}");
        }
    }
    let digest = tool(owner.clone(), "get_proposal", json!({ "proposal_id": pid })).await["result"]["structuredContent"]["digest"].clone();
    let v = tool(owner.clone(), "decide_proposal", json!({ "proposal_id": pid, "decision": "approve", "rationale": "Mine", "digest": digest.clone() })).await;
    assert!(v.get("error").is_some() || v["result"]["isError"] == true, "the proposer cannot approve alone: {v}");
    let v = tool(second.clone(), "decide_proposal", json!({ "proposal_id": pid, "decision": "approve", "rationale": "Reviewed", "digest": digest })).await;
    assert!(v.get("error").is_none() && v["result"]["isError"] != true, "a second person can: {v}");
}

// 6. A write through a task grant is in history with the flow and step named.
#[tokio::test(flavor = "multi_thread")]
async fn a_write_through_a_task_names_the_flow_and_step_in_history() {
    let w = world().await;
    let (_, rec) = w.start(w.rule("history", &["program"], vec![]));
    assert_eq!(w.put(&w.officer_t, &rec, json!({ "award_letter": "Letter 9" })).await.0, StatusCode::OK);
    let needle = rec.clone();
    let rows: Vec<(String, String, Option<String>, Option<String>)> = w.db.run(move |c| {
        c.query("SELECT actor, actor_kind, token_label, on_behalf_of FROM record_history WHERE record_id = $1 AND action = 'update' ORDER BY id", &[&needle])
            .unwrap()
            .iter()
            .map(|r| (r.get(0), r.get(1), r.get(2), r.get(3)))
            .collect()
    });
    let row = rows.iter().find(|r| r.0 == w.officer).unwrap_or_else(|| panic!("the officer's write is in history: {rows:?}"));
    assert_eq!(row.1, "user");
    assert_eq!(row.3, None, "on_behalf_of is empty");
    let label = row.2.clone().unwrap_or_default();
    assert!(label.contains("Award letter history") && label.contains("provide-letter"), "the flow and the step are named: {label}");
}

// 7. Every path that closes a step closes its grants. A scan test lists the closing paths.
#[test]
fn every_path_that_closes_a_step_closes_its_grants() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut closing = vec![];
    let mut missing = vec![];
    for entry in std::fs::read_dir(&root).unwrap() {
        let path = entry.unwrap().path();
        let file = path.file_name().unwrap().to_string_lossy().to_string();
        if !file.starts_with("repository") || path.extension().and_then(|e| e.to_str()) != Some("rs") {
            continue;
        }
        let text = std::fs::read_to_string(&path).unwrap();
        // Split into functions at each `fn ` that starts a line, after any visibility.
        let mut functions: Vec<(String, String)> = vec![];
        for line in text.lines() {
            let t = line.trim_start();
            let starts = ["fn ", "pub fn ", "pub(crate) fn "].iter().any(|p| t.starts_with(p));
            if starts {
                functions.push((t.split("fn ").nth(1).unwrap().split(['(', '<']).next().unwrap().to_string(), String::new()));
            }
            if let Some(last) = functions.last_mut() {
                last.1.push_str(line);
                last.1.push('\n');
            }
        }
        for (name, body) in functions {
            // A path that moves an instance's status: an UPDATE of the status, or an upsert that takes the new one.
            if body.contains("UPDATE process_instances SET status") || body.contains("status = EXCLUDED.status") {
                closing.push(format!("{file}::{name}"));
                if !body.contains("close_task_grants_in_tx") {
                    missing.push(format!("{file}::{name}"));
                }
            }
        }
    }
    closing.sort();
    for known in ["repository.rs::decide_process_instance", "repository_timers.rs::apply_timer", "repository_votes.rs::cast_vote"] {
        assert!(closing.contains(&known.to_string()), "{known} is found as a closing path: {closing:?}");
    }
    assert!(missing.is_empty(), "these move a step's status and leave its task grants open: {missing:?} (all closing paths: {closing:?})");
}
