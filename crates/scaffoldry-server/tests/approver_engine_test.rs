//! Approvers phase 3: the engine asks the resolver who may decide a waiting step.
//!
//! The tests share one database, so they run one at a time behind a lock and reset the people,
//! organization and position tables first. Records and process instances are placed directly in
//! state, so each test names exactly who submitted and who started what.

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
use tokio::sync::{Mutex, MutexGuard};
use tower::ServiceExt;
use uuid::Uuid;

static LOCK: Mutex<()> = Mutex::const_new(());
const ROOT_UNIT: &str = "00000000-0000-0000-0000-000000000001";
const APP: &str = "approver-engine-app";
const WS: &str = "ws-approver-engine";

struct World {
    app: Router,
    state: Arc<ServerState>,
    admin: String,
    physics: String,
    college: String,
    _guard: MutexGuard<'static, ()>,
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
        });
        issue_test_token_and_user(who)
    }

    async fn hold(&self, unit: &str, who: &str, replace: bool) {
        let (s, b) = send(&self.app, "POST", &format!("/api/v1/orgs/{unit}/positions/chair/holders"), &self.admin,
            Some(json!({ "eppn": who, "replace": replace, "reason": "Fixture" }))).await;
        assert_eq!(s, StatusCode::CREATED, "assign: {b}");
    }

    fn rule(&self, id: &str, spec: Option<ApproverSpec>, role: &str) -> AutomationRule {
        AutomationRule {
            id: id.to_string(),
            app_slug: APP.to_string(),
            name: id.to_string(),
            description: String::new(),
            enabled: true,
            trigger: TriggerEvent::RecordCreated,
            cedar_policy_guard: None,
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
                    approver: spec,
                },
            }],
        }
    }

    /// A record `created_by` the submitter, and a waiting instance of `rule_id` on it.
    fn waiting(&self, rule: AutomationRule, record_id: &str, submitter: &str, started_by: Option<&str>, assigned_to: Option<&str>) -> String {
        let rule_id = rule.id.clone();
        self.state.automations.write().unwrap().entry(APP.to_string()).or_default().push(rule);
        self.state.records.write().unwrap().entry(APP.to_string()).or_default().push(DatasetRecord {
            id: record_id.to_string(),
            app_slug: APP.to_string(),
            data: json!({ "status": "UnderReview" }),
            ceds_mapping: Default::default(),
            is_ferpa_sensitive: false,
            created_at: String::new(),
            created_by: Some(submitter.to_string()),
        });
        let id = format!("{rule_id}:{record_id}");
        self.state.process_instances.write().unwrap().insert(id.clone(), ProcessInstance {
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
            no_approver: None,
        });
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
    let guard = LOCK.lock().await;
    tokio::task::spawn_blocking(|| {
        let url = std::env::var("DATABASE_URL")
            .unwrap_or_else(|_| "postgres://scaffoldry:scaffoldry_dev_password@127.0.0.1:5433/scaffoldry".to_string());
        if let Ok(mut c) = postgres::Client::connect(&url, postgres::NoTls) {
            let _ = c.batch_execute(
                "TRUNCATE TABLE scim_users, scim_groups, roles, persons, organizations, workspaces, api_tokens, position_types, process_instances, workflow_automations CASCADE;",
            );
        }
    })
    .await
    .unwrap();
    let admin = issue_test_token_and_user("jordan.lee@state.edu");
    let state = Arc::new(ServerState::new().expect("state"));
    let app = build_app_with_state(state.clone()).expect("router");
    let mut w = World { app, state, admin, physics: String::new(), college: String::new(), _guard: guard };

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
        custom_domain: None,
        custom_domain_verified: false,
        tables: vec![],
        relationships: vec![],
        views: vec![],
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

// 1. The Physics chair can decide. The Chemistry chair and the submitter cannot.
#[tokio::test]
async fn only_the_submitters_department_chair_may_decide() {
    let (w, p) = world().await;
    let id = w.waiting(w.rule("r1", Some(chair(true)), "chair"), "rec-1", &p.sam, None, None);
    assert_eq!(w.decide(&p.okafor_t, &id).await, StatusCode::FORBIDDEN, "the Chemistry chair");
    assert_eq!(w.decide(&p.sam_t, &id).await, StatusCode::FORBIDDEN, "the submitter");
    assert_eq!(w.decide(&p.rivera_t, &id).await, StatusCode::OK, "the Physics chair");
}

// 2. A vacant chair: the dean is asked, or the instance is flagged with the reason.
#[tokio::test]
async fn a_vacant_position_asks_the_next_unit_up_or_reports_the_problem() {
    let (w, p) = world().await;
    let up = w.waiting(w.rule("up", Some(chair(true)), "chair"), "rec-up", &p.sam, None, None);
    let stay = w.waiting(w.rule("stay", Some(chair(false)), "chair"), "rec-stay", &p.sam, None, None);

    let (s, _) = send(&w.app, "DELETE", &format!("/api/v1/orgs/{}/positions/chair/holders/{}", w.physics, p.rivera), &w.admin,
        Some(json!({ "reason": "Chair left" }))).await;
    assert_eq!(s, StatusCode::OK);

    let rows = w.list(&p.sam_t, "").await;
    let up_row = World::instance_row(&rows, &up);
    assert_eq!(up_row["status"], "Waiting");
    assert_eq!(up_row["no_approver"], Value::Null);
    assert_eq!(up_row["approvers"][0]["eppn"], p.dean.as_str());
    assert_eq!(up_row["approvers"][0]["via"], "Holder");

    let stay_row = World::instance_row(&rows, &stay);
    assert_eq!(stay_row["status"], "Waiting", "a vacant position does not fail or skip the step");
    assert_eq!(stay_row["no_approver"], "position_vacant");
    assert!(stay_row["approvers"].as_array().unwrap().is_empty());
}

// 3. The submitter is the chair. They cannot decide their own record. The dean can.
#[tokio::test]
async fn a_chair_cannot_decide_their_own_record_but_the_dean_can() {
    let (w, p) = world().await;
    let id = w.waiting(w.rule("r3", Some(chair(true)), "chair"), "rec-3", &p.rivera, None, None);
    assert_eq!(w.decide(&p.rivera_t, &id).await, StatusCode::FORBIDDEN);
    assert_eq!(w.decide(&p.dean_t, &id).await, StatusCode::OK);
}

// 4. The chair changes while the instance waits.
#[tokio::test]
async fn when_the_chair_changes_the_new_chair_decides_and_the_old_one_cannot() {
    let (w, p) = world().await;
    let id = w.waiting(w.rule("r4", Some(chair(true)), "chair"), "rec-4", &p.sam, None, None);
    let newcomer = person("newchair");
    let newcomer_t = w.user(&newcomer, "Physics").await;
    w.hold(&w.physics.clone(), &newcomer, true).await;

    assert_eq!(w.decide(&p.rivera_t, &id).await, StatusCode::FORBIDDEN, "the old chair");
    assert_eq!(w.decide(&newcomer_t, &id).await, StatusCode::OK, "the new chair");
}

// 5. A named person who goes on hold cannot decide, and the instance says why.
#[tokio::test]
async fn a_named_person_on_hold_cannot_decide_and_the_instance_says_so() {
    let (w, p) = world().await;
    let id = w.waiting(w.rule("r5", Some(ApproverSpec::Person { eppn: p.okafor.clone() }), "chair"), "rec-5", &p.sam, None, None);
    assert_eq!(w.list(&p.sam_t, "").await.len(), 1);

    let (s, user) = send(&w.app, "GET", &format!("/api/v1/admin/users?search={}", p.okafor), &w.admin, None).await;
    assert_eq!(s, StatusCode::OK);
    let uid = user["users"][0]["id"].as_str().unwrap().to_string();
    let (s, _) = send(&w.app, "POST", &format!("/api/v1/admin/users/{uid}/hold"), &w.admin, Some(json!({ "hold": true, "reason": "Under review" }))).await;
    assert_eq!(s, StatusCode::OK);

    assert_eq!(w.decide(&p.okafor_t, &id).await, StatusCode::UNAUTHORIZED, "a held user's token stops working");
    let rows = w.list(&p.sam_t, "").await;
    assert_eq!(World::instance_row(&rows, &id)["no_approver"], "person_unavailable");
}

// 6. An old Role-only rule behaves as before, with the submitter now excluded.
#[tokio::test]
async fn an_old_role_rule_still_matches_by_role_or_affiliation() {
    let (w, p) = world().await;
    let id = w.waiting(w.rule("r6", None, "faculty"), "rec-6", &p.sam, None, None);
    assert_eq!(w.decide(&p.sam_t, &id).await, StatusCode::FORBIDDEN, "never the submitter");
    assert_eq!(w.decide(&p.okafor_t, &id).await, StatusCode::OK, "any other faculty collaborator");
}

// 7. mine=true is exactly the waiting instances the caller can decide.
#[tokio::test]
async fn mine_returns_only_what_the_caller_can_decide_now() {
    let (w, p) = world().await;
    let physics = w.waiting(w.rule("m1", Some(chair(true)), "chair"), "rec-m1", &p.sam, None, None);
    let chem_rule = w.waiting(w.rule("m2", Some(ApproverSpec::Person { eppn: p.okafor.clone() }), "chair"), "rec-m2", &p.sam, None, None);
    let own = w.waiting(w.rule("m3", Some(chair(true)), "chair"), "rec-m3", &p.rivera, None, None);

    let ids = |rows: Vec<Value>| -> Vec<String> {
        let mut v: Vec<String> = rows.iter().map(|r| r["id"].as_str().unwrap().to_string()).collect();
        v.sort();
        v
    };
    let mut want_rivera = vec![physics.clone()];
    want_rivera.sort();
    assert_eq!(ids(w.list(&p.rivera_t, "?mine=true").await), want_rivera);
    assert_eq!(ids(w.list(&p.okafor_t, "?mine=true").await), vec![chem_rule.clone()]);
    assert_eq!(ids(w.list(&p.dean_t, "?mine=true").await), vec![own.clone()]);
    assert!(w.list(&p.sam_t, "?mine=true").await.is_empty(), "a submitter decides nothing");
    // Without the filter everyone who can read still sees all three, with their approvers.
    assert_eq!(w.list(&p.sam_t, "").await.len(), 3);
}

// An administrator's reassignment narrows the step to one person, still never the submitter.
#[tokio::test]
async fn an_assigned_instance_can_only_be_decided_by_the_assignee() {
    let (w, p) = world().await;
    let id = w.waiting(w.rule("a1", Some(chair(true)), "chair"), "rec-a1", &p.sam, None, Some(&p.dean));
    assert_eq!(w.decide(&p.rivera_t, &id).await, StatusCode::FORBIDDEN, "the chair is not the assignee");
    assert_eq!(w.decide(&p.dean_t, &id).await, StatusCode::OK);

    let blocked = w.waiting(w.rule("a2", Some(chair(true)), "chair"), "rec-a2", &p.sam, None, Some(&p.sam));
    assert_eq!(w.decide(&p.sam_t, &blocked).await, StatusCode::FORBIDDEN, "an assignee who submitted the record");
}

// The person whose change started the process never decides it either.
#[tokio::test]
async fn the_starter_is_excluded() {
    let (w, p) = world().await;
    let id = w.waiting(w.rule("s1", Some(chair(true)), "chair"), "rec-s1", &p.sam, Some(&p.rivera), None);
    assert_eq!(w.decide(&p.rivera_t, &id).await, StatusCode::FORBIDDEN, "the starter holds the position");
    assert_eq!(w.decide(&p.dean_t, &id).await, StatusCode::OK, "the walk goes up past the starter");
}

// Saving a rule checks its approvers. A position that resolves to nobody still saves.
#[tokio::test]
async fn saving_a_rule_validates_its_approver_but_allows_a_vacant_position() {
    let (w, _p) = world().await;
    let save = |rule: Value| {
        let app = w.app.clone();
        let admin = w.admin.clone();
        async move { send(&app, "POST", &format!("/api/v1/apps/{APP}/automations"), &admin, Some(rule)).await }
    };
    let rule_with = |spec: Value, role: &str| -> Value {
        let mut r = serde_json::to_value(w.rule("save-rule", None, role)).unwrap();
        r["steps"][0]["kind"]["UserTask"]["approver"] = spec;
        r
    };

    let (s, b) = save(rule_with(json!({ "Position": { "key": "no-such", "from": "Submitter", "walk_up": true } }), "chair")).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    assert!(b["error"].as_str().unwrap().contains("step-approval"), "the step id is named: {b}");
    let (s, _) = save(rule_with(json!({ "Person": { "eppn": "ghost@state.edu" } }), "chair")).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    let (s, _) = save(rule_with(json!({ "Role": { "name": "  " } }), "chair")).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    let (s, b) = save(rule_with(Value::Null, "")).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "an empty role name with no approver: {b}");

    let (s, b) = save(rule_with(json!({ "Position": { "key": "chair", "from": "Workspace", "walk_up": false } }), "chair")).await;
    assert_eq!(s, StatusCode::CREATED, "{b}");

    // Retire the position: a rule that names it is refused, and the vacancy check is still not one.
    send(&w.app, "PATCH", "/api/v1/admin/positions/chair", &w.admin, Some(json!({ "retired": true, "reason": "Retire" }))).await;
    let (s, _) = save(rule_with(json!({ "Position": { "key": "chair", "from": "Workspace", "walk_up": false } }), "chair")).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
}

// A new instance records who started it and flags a step nobody can decide.
#[tokio::test]
async fn a_new_instance_records_its_starter_and_flags_a_vacant_step_with_a_log_line() {
    let (w, p) = world().await;
    // A department with no chair.
    let empty = w.unit("Geology", "Department", &w.college.clone()).await;
    let geo = person("geo");
    let _ = w.user(&geo, "Geology").await;
    let _ = empty;
    let rule = w.rule("n1", Some(chair(false)), "chair");
    w.state.automations.write().unwrap().entry(APP.to_string()).or_default().push(rule);
    w.state.records.write().unwrap().entry(APP.to_string()).or_default().push(DatasetRecord {
        id: "rec-n1".to_string(), app_slug: APP.to_string(), data: json!({ "id": "rec-n1", "status": "New" }),
        ceds_mapping: Default::default(), is_ferpa_sensitive: false, created_at: String::new(), created_by: Some(geo.clone()),
    });

    let identity = scaffoldry_core::standards::eduperson::EduPersonIdentity {
        eppn: p.sam.clone(),
        realm: "state.edu".to_string(),
        affiliations: vec![scaffoldry_core::standards::eduperson::EduPersonAffiliation::Faculty],
    };
    let mut applied = vec![];
    scaffoldry_server::service::records::run_automations(
        &w.state, APP, TriggerEvent::RecordCreated, &json!({ "id": "rec-n1", "status": "New" }),
        &scaffoldry_server::service::records::Actor { identity: &identity, department: "physics" }, 0, &mut applied,
    );
    let inst = w.state.process_instances.read().unwrap().get("n1:rec-n1").cloned().expect("the instance was created");
    assert_eq!(inst.started_by.as_deref(), Some(p.sam.as_str()));
    assert!(!inst.started_at.is_empty());
    assert_eq!(inst.no_approver.as_deref(), Some("position_vacant"));
    assert!(inst.log.iter().any(|l| l.contains("position_vacant")), "the first time it is set, a log line says so: {:?}", inst.log);
}
