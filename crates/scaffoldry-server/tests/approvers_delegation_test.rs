//! Approvers phase 4: delegation. A position holder names someone to decide for them while away, for a window of at
//! most 365 days and with a reason. Resolution reads the time, so a delegation ends on `ends_at` without anyone
//! acting. Every create and revoke is a `DelegationChanged` ledger entry.
//!
//! The world is the one the phase 3 tests use: Physics in Science, a chair in each department and a dean.
//!
//! The tests share one database, so they run one at a time behind a lock and reset the people,
//! organization and position tables first. Records and process instances are placed directly in
//! state, so each test names exactly who submitted and who started what.

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::Router;
use http_body_util::BodyExt;
use scaffoldry_core::approver::{ApproverSpec, Origin};
use scaffoldry_core::{
    AutomationRule, ProcessInstance, ProcessStatus, ProcessStep, StepKind, TriggerEvent,
};
use scaffoldry_server::build_app_with_state;
use scaffoldry_server::service::identity::issue_test_token_and_user;
use scaffoldry_server::state::{CollaboratorRecord, DatasetRecord, ServerState, WorkspaceRecord};
use serde_json::{json, Value};
use std::sync::Arc;
use tower::ServiceExt;
use uuid::Uuid;

const ROOT_UNIT: &str = "00000000-0000-0000-0000-000000000001";
const APP: &str = "approver-engine-app";
const WS: &str = "ws-approver-engine";

struct World {
    app: Router,
    state: Arc<ServerState>,
    admin: String,
    physics: String,
    college: String,
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

fn person(prefix: &str) -> String {
    format!("{prefix}-{}@state.edu", &Uuid::new_v4().to_string()[..8])
}

impl World {
    async fn unit(&self, name: &str, org_type: &str, parent: &str) -> String {
        let (s, b) = send(&self.app, "POST", "/api/v1/orgs", &self.admin, Some(json!({
            "name": name, "code": &name[..4].to_uppercase(), "org_type": org_type, "parent_id": parent
        }))).await;
        assert_eq!(s, StatusCode::CREATED, "create unit: {b}");
        b["id"].as_str().unwrap().to_string()
    }

    /// A user placed in `department`, who is also a collaborator on the app's workspace.
    async fn user(&self, who: &str, department: &str) -> String {
        let (s, _) = send(&self.app, "POST", "/api/v1/admin/users", &self.admin, Some(json!({
            "userName": who, "name": "Test Person", "email": who, "affiliation": "faculty",
            "department": department, "title": "Lecturer", "reason": "Fixture"
        }))).await;
        assert_eq!(s, StatusCode::CREATED, "creating a test user failed");
        self.state.collaborators.write().unwrap().entry(WS.to_string()).or_default().push(CollaboratorRecord {
            id: format!("collab-{who}"),
            workspace_id: WS.to_string(),
            eppn: who.to_string(),
            name: who.to_string(),
            role: "editor".to_string(),
            scoped_affiliation: "faculty".to_string(),
            department: "physics".to_string(),
            added_at: String::new(),
            app_slug: String::new(),
        });
        issue_test_token_and_user(who)
    }

    async fn hold(&self, unit: &str, who: &str, replace: bool) {
        let (s, b) = send(&self.app, "POST", &format!("/api/v1/orgs/{unit}/positions/chair/holders"), &self.admin,
            Some(json!({ "eppn": who, "replace": replace, "reason": "Fixture" }))).await;
        assert_eq!(s, StatusCode::CREATED, "assign: {b}");
    }

    fn rule(&self, id: &str, spec: Option<ApproverSpec>, role: &str) -> AutomationRule {
        AutomationRule { version: 0, home_workspace: None, grants: vec![],
            id: id.to_string(),
            app_slug: APP.to_string(),
            name: id.to_string(),
            description: String::new(),
            enabled: true,
            trigger: TriggerEvent::RecordCreated,
            predicates: vec![],
            actions: vec![],
            steps: vec![ProcessStep {
                id: "step-approval".to_string(),
                when: vec![],
                kind: StepKind::UserTask {
                    role: role.to_string(),
                    prompt: "Decide".to_string(),
                    approve: vec![scaffoldry_core::ActionType::UpdateRecordStatus { new_status: "Approved".into() }],
                    reject: vec![],
                    approver: spec, due: None, remind_every: None, on_due: vec![], on_fail: vec![], decision: Default::default(), reject_rule: Default::default(), kind: Default::default(), show: vec![], fill: vec![], require: vec![], decline_reason: false,
                },
            }],
        }
    }

    /// A record `created_by` the submitter, and a waiting instance of `rule_id` on it.
    fn waiting(&self, rule: AutomationRule, record_id: &str, submitter: &str, started_by: Option<&str>, assigned_to: Option<&str>) -> String {
        let rule_id = rule.id.clone();
        self.state.automations.write().unwrap().entry(APP.to_string()).or_default().push(rule);
        self.state.put_record(&DatasetRecord {
            id: record_id.to_string(),
            app_slug: APP.to_string(),
            data: json!({ "status": "UnderReview" }),
            ceds_mapping: Default::default(),
            is_ferpa_sensitive: false,
            created_at: String::new(),
            created_by: Some(submitter.to_string()),
            version: 1,
        }, &scaffoldry_server::repository::WriteCtx::system("test fixture")).unwrap();
        let id = format!("{rule_id}:{record_id}");
        self.state.repository.upsert_process_instance(&ProcessInstance { rule_version: 0,
            id: id.clone(),
            rule_id,
            app_slug: APP.to_string(),
            record_id: record_id.to_string(),
            status: ProcessStatus::Waiting,
            waiting_step_id: Some("step-approval".to_string()),
            role: Some("chair".to_string()),
            prompt: Some("Decide".to_string()),
            log: vec![],
            started_by: started_by.map(str::to_string),
            started_at: String::new(),
            assigned_to: assigned_to.map(str::to_string),
            no_approver: None, escalated_to: vec![], escalated_unit: None, voters: vec![],
        }).unwrap();
        id
    }

    async fn decide(&self, token: &str, instance: &str) -> StatusCode {
        send(&self.app, "POST", &format!("/api/v1/apps/{APP}/processes/{instance}/decide"), token, Some(json!({ "decision": "approve" }))).await.0
    }

    async fn list(&self, token: &str, query: &str) -> Vec<Value> {
        let (s, b) = send(&self.app, "GET", &format!("/api/v1/apps/{APP}/processes{query}"), token, None).await;
        assert_eq!(s, StatusCode::OK, "{b}");
        b.as_array().cloned().unwrap_or_default()
    }

    fn instance_row<'a>(rows: &'a [Value], id: &str) -> &'a Value {
        rows.iter().find(|r| r["id"] == id).unwrap_or_else(|| panic!("instance {id} is listed"))
    }
}

fn chair(walk_up: bool) -> ApproverSpec {
    ApproverSpec::Position { key: "chair".into(), from: Origin::Submitter, walk_up }
}

struct People {
    sam: String,        // Physics faculty, the usual submitter
    rivera: String,     // Physics chair
    okafor: String,     // Chemistry chair
    dean: String,       // College dean
    sam_t: String,
    rivera_t: String,
    okafor_t: String,
    dean_t: String,
}

async fn world() -> (World, People) {
    let url = scaffoldry_server::testing::database_url();
    tokio::task::spawn_blocking(move || {
        if let Ok(mut c) = postgres::Client::connect(&url, postgres::NoTls) {
            let _ = c.batch_execute(
                "TRUNCATE TABLE scim_users, scim_groups, roles, persons, organizations, workspaces, api_tokens, position_types, process_instances, workflow_automations, delegations CASCADE;",
            );
        }
    })
    .await
    .unwrap();
    let admin = issue_test_token_and_user("jordan.lee@state.edu");
    let state = Arc::new(ServerState::new().expect("state"));
    let app = build_app_with_state(state.clone()).expect("router");
    let mut w = World { app, state, admin, physics: String::new(), college: String::new() };

    w.college = w.unit("Science", "College", ROOT_UNIT).await;
    w.physics = w.unit("Physics", "Department", &w.college.clone()).await;
    let chem = w.unit("Chemistry", "Department", &w.college.clone()).await;

    let ws: WorkspaceRecord = serde_json::from_value(json!({
        "id": WS, "name": "Approver engine", "code": "AENG", "organization": "State",
        "visibility": "restricted", "organization_id": w.physics,
    })).unwrap();
    w.state.persist_workspace(ws).unwrap();
    w.state.engine.write().unwrap().register_manifest(scaffoldry_engine::AppManifest {
        slug: APP.to_string(),
        title: "Approver engine".to_string(),
        description: "Phase 3".to_string(),
        organization_code: "PHYS".to_string(),
        department: "physics".to_string(),
        workspace_id: Some(WS.to_string()),
        herm_capability_id: None,
        coordinates: None, sealed: false, integrations: Default::default(), kind: Default::default(),
        custom_domain: None,
        custom_domain_verified: false,
        tables: vec![],
        relationships: vec![],
        views: vec![],
        pages: vec![],
        access: Default::default(),
            ceds_mappings: Default::default(),
    }).unwrap();

    let (sam, rivera, okafor, dean) = (person("sam"), person("rivera"), person("okafor"), person("dean"));
    let sam_t = w.user(&sam, "Physics").await;
    let rivera_t = w.user(&rivera, "Physics").await;
    let okafor_t = w.user(&okafor, "Chemistry").await;
    let dean_t = w.user(&dean, "Science").await;

    let (s, b) = send(&w.app, "POST", "/api/v1/admin/positions", &w.admin, Some(json!({
        "key": "chair", "name": "Chair", "description": "", "org_types": ["Department", "College"],
        "max_holders": 1, "reason": "Fixture"
    }))).await;
    assert_eq!(s, StatusCode::CREATED, "{b}");
    w.hold(&w.physics.clone(), &rivera, false).await;
    w.hold(&chem, &okafor, false).await;
    w.hold(&w.college.clone(), &dean, false).await;

    (w, People { sam, rivera, okafor, dean, sam_t, rivera_t, okafor_t, dean_t })
}


async fn delegate(w: &World, token: &str, body: Value) -> (StatusCode, Value) {
    send(&w.app, "POST", "/api/v1/me/delegations", token, Some(body)).await
}

fn window(days: i64) -> (String, String) {
    let now = chrono::Utc::now();
    ((now - chrono::Duration::hours(1)).to_rfc3339(), (now + chrono::Duration::days(days)).to_rfc3339())
}

fn ledger_count(w: &World) -> i64 {
    let url = scaffoldry_server::testing::database_url();
    let _ = &w.state;
    std::thread::spawn(move || {
        let mut c = postgres::Client::connect(&url, postgres::NoTls).unwrap();
        c.query_one("SELECT count(*) FROM governance_ledger WHERE decision_type = 'DelegationChanged'", &[]).unwrap().get::<_, i64>(0)
    })
    .join()
    .unwrap()
}

fn run_sql(sql: &'static str, id: String) {
    let url = scaffoldry_server::testing::database_url();
    std::thread::spawn(move || {
        let mut c = postgres::Client::connect(&url, postgres::NoTls).unwrap();
        let id: Uuid = id.parse().unwrap();
        c.execute(sql, &[&id]).unwrap();
    })
    .join()
    .unwrap();
}

fn approvers_of<'a>(rows: &'a [Value], id: &str) -> Vec<(&'a str, Value)> {
    World::instance_row(rows, id)["approvers"].as_array().unwrap().iter().map(|a| (a["eppn"].as_str().unwrap(), a["via"].clone())).collect()
}

// 1. A delegate inside the window is among the step's approvers, as `DelegateOf(chair)`, and can decide.
// 2. The day after `ends_at`, they cannot.
#[tokio::test]
async fn a_delegate_decides_inside_the_window_and_not_after_it() {
    let _db = common::TestDb::new();
    let (w, p) = world().await;
    let ada = person("ada");
    let ada_t = w.user(&ada, "Physics").await;
    let (starts, ends) = window(14);
    let (s, d) = delegate(&w, &p.rivera_t, json!({ "delegate": ada, "position_key": "chair", "starts_at": starts, "ends_at": ends, "reason": "Away at a conference" })).await;
    assert_eq!(s, StatusCode::CREATED, "{d}");
    let id = d["id"].as_str().expect("an id").to_string();

    let inst = w.waiting(w.rule("d1", Some(chair(true)), "chair"), "rec-d1", &p.sam, None, None);
    let rows = w.list(&p.sam_t, "").await;
    let approvers = approvers_of(&rows, &inst);
    assert!(approvers.iter().any(|(e, v)| *e == p.rivera && v == "Holder"), "{approvers:?}");
    assert!(approvers.iter().any(|(e, v)| *e == ada && *v == json!({ "DelegateOf": p.rivera })), "the delegate, for the chair: {approvers:?}");

    // After the window, without anyone acting.
    run_sql("UPDATE delegations SET starts_at = NOW() - INTERVAL '20 days', ends_at = NOW() - INTERVAL '1 day' WHERE id = $1", id);
    assert_eq!(w.decide(&ada_t, &inst).await, StatusCode::FORBIDDEN, "the day after ends_at");
    let rows = w.list(&p.sam_t, "").await;
    assert!(!approvers_of(&rows, &inst).iter().any(|(e, _)| *e == ada));

    // A fresh delegation inside its window: the delegate decides.
    let (starts, ends) = window(3);
    let (s, _) = delegate(&w, &p.rivera_t, json!({ "delegate": ada, "starts_at": starts, "ends_at": ends, "reason": "Away again" })).await;
    assert_eq!(s, StatusCode::CREATED);
    assert_eq!(w.decide(&ada_t, &inst).await, StatusCode::OK, "a delegate inside the window decides");
}

// 3. Revoking removes the delegate at once. The delegator revokes their own. Someone else cannot, but a Platform Admin
// can, with a reason.
#[tokio::test]
async fn revoking_removes_the_delegate_at_once() {
    let _db = common::TestDb::new();
    let (w, p) = world().await;
    let ada = person("ada");
    let ada_t = w.user(&ada, "Physics").await;
    let (starts, ends) = window(14);
    let (_, d) = delegate(&w, &p.rivera_t, json!({ "delegate": ada, "starts_at": starts, "ends_at": ends, "reason": "Leave" })).await;
    let id = d["id"].as_str().unwrap().to_string();
    let inst = w.waiting(w.rule("d3", Some(chair(true)), "chair"), "rec-d3", &p.sam, None, None);

    assert!(!p.okafor.is_empty(), "the Chemistry chair is in the world");
    let (s, _) = send(&w.app, "DELETE", &format!("/api/v1/me/delegations/{id}"), &p.okafor_t, None).await;
    assert_eq!(s, StatusCode::NOT_FOUND, "someone else's delegation is not theirs to revoke");
    let (s, b) = send(&w.app, "DELETE", &format!("/api/v1/me/delegations/{id}"), &p.rivera_t, None).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_eq!(w.decide(&ada_t, &inst).await, StatusCode::FORBIDDEN, "revoked at once");

    // The admin route needs a reason.
    let (starts, ends) = window(5);
    let (_, d) = delegate(&w, &p.rivera_t, json!({ "delegate": ada, "starts_at": starts, "ends_at": ends, "reason": "Leave" })).await;
    let id = d["id"].as_str().unwrap().to_string();
    let (s, _) = send(&w.app, "DELETE", &format!("/api/v1/admin/delegations/{id}"), &w.admin, Some(json!({}))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    let (s, _) = send(&w.app, "DELETE", &format!("/api/v1/admin/delegations/{id}"), &p.okafor_t, Some(json!({ "reason": "No" }))).await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    let (s, b) = send(&w.app, "DELETE", &format!("/api/v1/admin/delegations/{id}"), &w.admin, Some(json!({ "reason": "The chair asked" }))).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_eq!(w.decide(&ada_t, &inst).await, StatusCode::FORBIDDEN);

    // Both lists show it, revoked.
    let (s, mine) = send(&w.app, "GET", "/api/v1/me/delegations", &p.rivera_t, None).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(mine["made"].as_array().unwrap().len(), 2, "{mine}");
    assert!(mine["made"].as_array().unwrap().iter().all(|d| d["state"] == "revoked"), "{mine}");
    let (_, theirs) = send(&w.app, "GET", "/api/v1/me/delegations", &ada_t, None).await;
    assert_eq!(theirs["received"].as_array().unwrap().len(), 2, "{theirs}");
    let (s, all) = send(&w.app, "GET", &format!("/api/v1/admin/delegations?delegator={}&state=revoked", p.rivera), &w.admin, None).await;
    assert_eq!(s, StatusCode::OK, "{all}");
    assert_eq!(all["delegations"].as_array().unwrap().len(), 2, "{all}");
    let (s, _) = send(&w.app, "GET", "/api/v1/admin/delegations", &p.rivera_t, None).await;
    assert_eq!(s, StatusCode::FORBIDDEN);
}

// 4. A person who holds no position cannot delegate a position, or delegate at all.
// 5. 400 days is refused, so is delegating to oneself, to someone unknown, or without a reason.
#[tokio::test]
async fn a_delegation_needs_a_position_a_short_window_another_person_and_a_reason() {
    let _db = common::TestDb::new();
    let (w, p) = world().await;
    let ada = person("ada");
    w.user(&ada, "Physics").await;
    let (starts, ends) = window(10);
    let ok = |extra: Value| {
        let mut b = json!({ "delegate": ada, "starts_at": starts, "ends_at": ends, "reason": "Away" });
        b.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
        b
    };
    let (s, b) = delegate(&w, &p.sam_t, ok(json!({ "position_key": "chair" }))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "sam holds no chair: {b}");
    let (s, _) = delegate(&w, &p.sam_t, ok(json!({}))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "sam holds no position at all");
    let (_, long) = window(400);
    let (s, b) = delegate(&w, &p.rivera_t, ok(json!({ "ends_at": long }))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "400 days: {b}");
    let (s, _) = delegate(&w, &p.rivera_t, ok(json!({ "delegate": p.rivera }))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "to oneself");
    let (s, _) = delegate(&w, &p.rivera_t, ok(json!({ "delegate": "nobody.at.all@state.edu" }))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "someone who does not resolve");
    let (s, _) = delegate(&w, &p.rivera_t, ok(json!({ "reason": "  " }))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "no reason");
    let (s, _) = delegate(&w, &p.rivera_t, ok(json!({ "starts_at": ends, "ends_at": starts }))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "ends before it starts");
    let (s, b) = delegate(&w, &p.rivera_t, ok(json!({ "position_key": "chair" }))).await;
    assert_eq!(s, StatusCode::CREATED, "{b}");
}

// 6. A delegate who submitted the record is not an approver for it.
#[tokio::test]
async fn a_delegate_who_submitted_the_record_is_not_its_approver() {
    let _db = common::TestDb::new();
    let (w, p) = world().await;
    let (starts, ends) = window(10);
    let (s, _) = delegate(&w, &p.rivera_t, json!({ "delegate": p.sam, "starts_at": starts, "ends_at": ends, "reason": "Away" })).await;
    assert_eq!(s, StatusCode::CREATED);
    let inst = w.waiting(w.rule("d6", Some(chair(true)), "chair"), "rec-d6", &p.sam, None, None);
    let rows = w.list(&p.rivera_t, "").await;
    assert!(!approvers_of(&rows, &inst).iter().any(|(e, _)| *e == p.sam), "never the submitter");
    assert_eq!(w.decide(&p.sam_t, &inst).await, StatusCode::FORBIDDEN);
}

// 7. Every create and revoke writes a ledger entry.
#[tokio::test]
async fn every_create_and_revoke_writes_a_ledger_entry() {
    let _db = common::TestDb::new();
    let (w, p) = world().await;
    let ada = person("ada");
    w.user(&ada, "Physics").await;
    let before = ledger_count(&w);
    let (starts, ends) = window(10);
    let (_, d) = delegate(&w, &p.rivera_t, json!({ "delegate": ada, "starts_at": starts, "ends_at": ends, "reason": "Away" })).await;
    assert_eq!(ledger_count(&w), before + 1, "created");
    let id = d["id"].as_str().unwrap();
    send(&w.app, "DELETE", &format!("/api/v1/me/delegations/{id}"), &p.rivera_t, None).await;
    assert_eq!(ledger_count(&w), before + 2, "revoked");
    // A refused one writes nothing.
    delegate(&w, &p.rivera_t, json!({ "delegate": p.rivera, "starts_at": starts, "ends_at": ends, "reason": "Away" })).await;
    assert_eq!(ledger_count(&w), before + 2);
}

// The form finds a person by the start of their name or address: two letters at least, ten at most, active people only.
#[tokio::test]
async fn the_form_finds_active_people_by_name() {
    let _db = common::TestDb::new();
    let (w, p) = world().await;
    let (s, _) = send(&w.app, "GET", "/api/v1/me/delegations/people?q=d", &p.rivera_t, None).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "one letter is too few");
    let (s, b) = send(&w.app, "GET", "/api/v1/me/delegations/people?q=dean", &p.rivera_t, None).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    let people = b["people"].as_array().unwrap();
    assert!(people.iter().any(|x| x["eppn"] == p.dean.as_str() && x["name"].is_string()), "{b}");
    assert!(people.len() <= 10);
    // The list of one's own delegations names the other person in words.
    let (starts, ends) = window(5);
    delegate(&w, &p.rivera_t, json!({ "delegate": p.dean, "starts_at": starts, "ends_at": ends, "reason": "Away" })).await;
    let (_, mine) = send(&w.app, "GET", "/api/v1/me/delegations", &p.dean_t, None).await;
    assert!(mine["received"][0]["delegator_name"].is_string(), "{mine}");
    assert_eq!(mine["positions"], json!(["chair"]), "{mine}");
}

// Approvers phase 5: the builder asks who a step would go to today. The answer names people, the unit and the path,
// or the problem. It needs the app's Manage scope and creates nothing.
#[tokio::test]
async fn the_preview_says_who_a_step_would_go_to_today_and_creates_nothing() {
    let _db = common::TestDb::new();
    let (w, p) = world().await;
    let path = format!("/api/v1/apps/{APP}/automations/preview-approvers");
    let spec = json!({ "Position": { "key": "chair", "from": "Submitter", "walk_up": true } });
    let before = w.list(&w.admin, "").await.len();
    let (s, b) = send(&w.app, "POST", &path, &w.admin, Some(json!({ "approver": spec, "created_by": p.sam }))).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_eq!(b["approvers"][0]["eppn"], p.rivera.as_str(), "{b}");
    assert_eq!(b["approvers"][0]["via"], "Holder");
    assert!(b["approvers"][0]["display_name"].is_string());
    assert_eq!(b["unit_id"], w.physics.as_str());
    assert_eq!(b["unit_name"], "Physics");
    assert!(b["problem"].is_null());
    assert_eq!(w.list(&w.admin, "").await.len(), before, "nothing was created");

    // Without walking up, a vacant chair is the problem, in its code.
    let (s, _) = send(&w.app, "DELETE", &format!("/api/v1/orgs/{}/positions/chair/holders/{}", w.physics, p.rivera), &w.admin, Some(json!({ "reason": "Left" }))).await;
    assert_eq!(s, StatusCode::OK);
    let (_, b) = send(&w.app, "POST", &path, &w.admin, Some(json!({ "approver": { "Position": { "key": "chair", "from": "Submitter", "walk_up": false } }, "created_by": p.sam }))).await;
    assert_eq!(b["problem"], "position_vacant", "{b}");
    assert_eq!(b["start_unit_name"], "Physics", "the problem names where the search started: {b}");
    assert_eq!(b["approvers"], json!([]));
    // A bad spec is a 400. Someone who may not manage the app is refused.
    let (s, _) = send(&w.app, "POST", &path, &w.admin, Some(json!({ "approver": { "Position": { "key": "nope" } } }))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    let (s, _) = send(&w.app, "POST", &path, &p.sam_t, Some(json!({ "approver": spec }))).await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    // The builder's list of position types, for anyone who may manage the app.
    let (s, b) = send(&w.app, "GET", &format!("/api/v1/apps/{APP}/automations/approver-options"), &w.admin, None).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_eq!(b["positions"], json!([{ "key": "chair", "name": "Chair" }]));
    let (s, _) = send(&w.app, "GET", &format!("/api/v1/apps/{APP}/automations/approver-options"), &p.sam_t, None).await;
    assert_eq!(s, StatusCode::FORBIDDEN);
}
