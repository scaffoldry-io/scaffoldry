//! Process v2 phase 4: committee votes.
//!
//! A user task may need every voter (`AllOf`) or some number of them (`AtLeast(n)`) to approve, and closes rejected on the
//! first rejection (`FirstReject`) or when more than half reject (`MajorityReject`). The voters are fixed when the step is
//! entered: the people the approver resolves to then, with their delegates, less the submitter and the starter.
//!
//! The world: Physics has a four-seat review committee. Sam submits, and sits on it. Ash, Bea and Cy sit on it too.

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::Router;
use http_body_util::BodyExt;
use scaffoldry_core::approver::{ApproverSpec, Origin};
use scaffoldry_core::standards::eduperson::{EduPersonAffiliation, EduPersonIdentity};
use scaffoldry_core::workflow::{DecisionRule, RejectRule};
use scaffoldry_core::{ActionType, AutomationRule, ProcessStatus, ProcessStep, StepKind, TriggerEvent};
use scaffoldry_server::build_app_with_state;
use scaffoldry_server::service::identity::issue_test_token_and_user;
use scaffoldry_server::state::{CollaboratorRecord, DatasetRecord, ServerState, WorkspaceRecord};
use serde_json::{json, Value};
use std::sync::Arc;
use tower::ServiceExt;
use uuid::Uuid;

const ROOT_UNIT: &str = "00000000-0000-0000-0000-000000000001";
const APP: &str = "process-votes-app";
const WS: &str = "ws-process-votes";

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

struct Member {
    eppn: String,
    token: String,
}

struct World {
    db: common::TestDb,
    app: Router,
    state: Arc<ServerState>,
    admin: String,
    physics: String,
    sam: Member,
    ash: Member,
    bea: Member,
    cy: Member,
}

impl World {
    async fn user(app: &Router, state: &ServerState, admin: &str, who: &str) -> String {
        let (s, _) = send(app, "POST", "/api/v1/admin/users", admin, Some(json!({
            "userName": who, "name": "Test Person", "email": who, "affiliation": "faculty",
            "department": "Physics", "title": "Lecturer", "reason": "Fixture"
        }))).await;
        assert_eq!(s, StatusCode::CREATED, "creating a test user failed");
        state.collaborators.write().unwrap().entry(WS.to_string()).or_default().push(CollaboratorRecord {
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

    async fn seat(&self, who: &str) {
        let (s, b) = send(&self.app, "POST", &format!("/api/v1/orgs/{}/positions/review/holders", self.physics), &self.admin,
            Some(json!({ "eppn": who, "replace": false, "reason": "Fixture" }))).await;
        assert_eq!(s, StatusCode::CREATED, "seat: {b}");
    }

    fn rule(&self, id: &str, decision: DecisionRule, reject_rule: RejectRule) -> AutomationRule {
        AutomationRule {
            version: 0,
            home_workspace: None,
            grants: vec![],
            id: id.to_string(),
            app_slug: APP.to_string(),
            name: id.to_string(),
            description: String::new(),
            enabled: true,
            trigger: TriggerEvent::RecordCreated,
            predicates: vec![],
            actions: vec![],
            steps: vec![ProcessStep {
                id: "committee".to_string(),
                when: vec![],
                kind: StepKind::UserTask {
                    role: "review".to_string(),
                    prompt: "Vote on the request".to_string(),
                    approve: vec![ActionType::UpdateRecordStatus { new_status: "Approved".into() }],
                    reject: vec![ActionType::UpdateRecordStatus { new_status: "Rejected".into() }],
                    approver: Some(ApproverSpec::Position { key: "review".into(), from: Origin::Submitter, walk_up: false }),
                    due: None,
                    remind_every: None,
                    on_due: vec![],
                    on_fail: vec![],
                    decision,
                    reject_rule,
                },
            }],
        }
    }

    /// Stores `rule` as the app's only rule and a record Sam submitted, then runs the rule on it as `starter`.
    fn start(&self, rule: AutomationRule, starter: &str) -> String {
        let rule_id = rule.id.clone();
        self.state.automations.write().unwrap().insert(APP.to_string(), vec![rule]);
        let record_id = format!("rec-{}", &Uuid::new_v4().simple().to_string()[..8]);
        let data = json!({ "_table_id": "requests", "title": "Equipment", "status": "Submitted" });
        self.state
            .put_record(
                &DatasetRecord {
                    id: record_id.clone(),
                    app_slug: APP.to_string(),
                    data: data.clone(),
                    ceds_mapping: Default::default(),
                    is_ferpa_sensitive: false,
                    created_at: String::new(),
                    created_by: Some(self.sam.eppn.clone()),
                    version: 1,
                },
                &scaffoldry_server::repository::WriteCtx::system("test fixture"),
            )
            .unwrap();
        let identity = EduPersonIdentity { eppn: starter.to_string(), realm: "state.edu".into(), affiliations: vec![EduPersonAffiliation::Faculty] };
        let mut with_id = data;
        with_id["id"] = json!(record_id);
        let mut applied = vec![];
        scaffoldry_server::service::records::run_automations(
            &self.state,
            APP,
            TriggerEvent::RecordCreated,
            &with_id,
            &scaffoldry_server::service::records::Actor { identity: &identity, department: "Physics" },
            0,
            &mut applied,
        );
        let id = format!("{rule_id}:{record_id}");
        assert!(self.state.repository.get_process_instance(&id).unwrap().is_some(), "the rule started an instance");
        id
    }

    async fn vote(&self, who: &Member, instance: &str, vote: &str, comment: Option<&str>) -> (StatusCode, Value) {
        let mut body = json!({ "decision": vote });
        if let Some(c) = comment {
            body["comment"] = json!(c);
        }
        send(&self.app, "POST", &format!("/api/v1/apps/{APP}/processes/{instance}/decide"), &who.token, Some(body)).await
    }

    fn status(&self, instance: &str) -> ProcessStatus {
        self.state.repository.get_process_instance(instance).unwrap().unwrap().status
    }

    fn record_status(&self, instance: &str) -> String {
        let inst = self.state.repository.get_process_instance(instance).unwrap().unwrap();
        self.state.find_record(APP, &inst.record_id).unwrap().unwrap().data["status"].as_str().unwrap_or_default().to_string()
    }
}

async fn world() -> World {
    let db = common::TestDb::new();
    let admin = issue_test_token_and_user("jordan.lee@state.edu");
    let state = db.state();
    let app = build_app_with_state(state.clone()).expect("router");

    let (s, b) = send(&app, "POST", "/api/v1/orgs", &admin, Some(json!({ "name": "Physics", "code": "PHYS", "org_type": "Department", "parent_id": ROOT_UNIT }))).await;
    assert_eq!(s, StatusCode::CREATED, "{b}");
    let physics = b["id"].as_str().unwrap().to_string();
    let ws: WorkspaceRecord = serde_json::from_value(json!({
        "id": WS, "name": "Process votes", "code": "PVOT", "organization": "State", "visibility": "restricted", "organization_id": physics,
    }))
    .unwrap();
    state.persist_workspace(ws).unwrap();
    let manifest: scaffoldry_engine::AppManifest = serde_json::from_value(json!({
        "slug": APP, "title": "Requests", "description": "", "organization_code": "PHYS", "department": "physics", "workspace_id": WS, "views": [],
        "tables": [{ "id": "requests", "name": "Requests", "slug": "requests", "primary_field": "title", "fields": [
            { "name": "title", "label": "Title", "field_type": "Text", "required": false, "ferpa_sensitive": false },
            { "name": "status", "label": "Status", "field_type": "Text", "required": false, "ferpa_sensitive": false }
        ] }],
    }))
    .unwrap();
    state.engine.write().unwrap().register_manifest(manifest).unwrap();
    let (s, b) = send(&app, "POST", "/api/v1/admin/positions", &admin, Some(json!({
        "key": "review", "name": "Review committee", "description": "", "org_types": ["Department"], "max_holders": 5, "reason": "Fixture"
    })))
    .await;
    assert_eq!(s, StatusCode::CREATED, "{b}");

    let mut members = vec![];
    for name in ["sam", "ash", "bea", "cy"] {
        let eppn = person(name);
        let token = World::user(&app, &state, &admin, &eppn).await;
        members.push(Member { eppn, token });
    }
    let mut it = members.into_iter();
    let (sam, ash, bea, cy) = (it.next().unwrap(), it.next().unwrap(), it.next().unwrap(), it.next().unwrap());
    let w = World { db, app, state, admin, physics, sam, ash, bea, cy };
    for who in [&w.sam.eppn, &w.ash.eppn, &w.bea.eppn, &w.cy.eppn] {
        w.seat(who).await;
    }
    w
}

// 1. `AllOf` with three voters stays waiting after two approvals and completes on the third. One rejection under
// `FirstReject` closes it rejected.
#[tokio::test(flavor = "multi_thread")]
async fn all_of_needs_every_voter_and_one_rejection_closes_it() {
    let w = world().await;
    let id = w.start(w.rule("all-of", DecisionRule::AllOf, RejectRule::FirstReject), &w.sam.eppn);
    let inst = w.state.repository.get_process_instance(&id).unwrap().unwrap();
    let mut voters = inst.voters.clone();
    voters.sort();
    let mut want = vec![w.ash.eppn.clone(), w.bea.eppn.clone(), w.cy.eppn.clone()];
    want.sort();
    assert_eq!(voters, want, "three voters: the committee less the submitter");

    for m in [&w.ash, &w.bea] {
        let (s, b) = w.vote(m, &id, "approve", None).await;
        assert_eq!(s, StatusCode::OK, "{b}");
        assert_eq!(w.status(&id), ProcessStatus::Waiting, "still waiting after {}", m.eppn);
    }
    let (s, b) = w.vote(&w.cy, &id, "approve", None).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_eq!(w.status(&id), ProcessStatus::Completed);
    assert_eq!(w.record_status(&id), "Approved");

    let id = w.start(w.rule("all-of-reject", DecisionRule::AllOf, RejectRule::FirstReject), &w.sam.eppn);
    w.vote(&w.ash, &id, "approve", None).await;
    let (s, b) = w.vote(&w.bea, &id, "reject", Some("The budget is not justified")).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_eq!(w.status(&id), ProcessStatus::Rejected, "one rejection closes it");
    assert_eq!(w.record_status(&id), "Rejected");
    assert_eq!(w.vote(&w.cy, &id, "approve", None).await.0, StatusCode::CONFLICT, "a closed step takes no more votes");
}

// 2. `AtLeast(2)` of 3 closes approved on the second approval and closes rejected once two reject.
#[tokio::test(flavor = "multi_thread")]
async fn at_least_two_of_three() {
    let w = world().await;
    let id = w.start(w.rule("two-of-three", DecisionRule::AtLeast(2), RejectRule::FirstReject), &w.sam.eppn);
    w.vote(&w.ash, &id, "approve", None).await;
    assert_eq!(w.status(&id), ProcessStatus::Waiting);
    w.vote(&w.bea, &id, "approve", None).await;
    assert_eq!(w.status(&id), ProcessStatus::Completed, "the second approval closes it");

    let id = w.start(w.rule("two-of-three-no", DecisionRule::AtLeast(2), RejectRule::FirstReject), &w.sam.eppn);
    w.vote(&w.ash, &id, "reject", Some("No")).await;
    assert_eq!(w.status(&id), ProcessStatus::Waiting, "two approvals are still possible");
    w.vote(&w.bea, &id, "reject", Some("No")).await;
    assert_eq!(w.status(&id), ProcessStatus::Rejected, "two rejections leave one voter, fewer than two");

    let id = w.start(w.rule("majority", DecisionRule::AllOf, RejectRule::MajorityReject), &w.sam.eppn);
    w.vote(&w.ash, &id, "reject", Some("No")).await;
    assert_eq!(w.status(&id), ProcessStatus::Waiting, "one of three is not a majority");
    w.vote(&w.bea, &id, "reject", Some("No")).await;
    assert_eq!(w.status(&id), ProcessStatus::Rejected, "two of three is");
}

// 3. A voter cannot vote twice, can change a vote, and the submitter and starter are not voters.
#[tokio::test(flavor = "multi_thread")]
async fn one_vote_each_which_may_change_and_no_vote_for_the_submitter_or_starter() {
    let w = world().await;
    // Sam submitted the record; Cy's change started the process.
    let id = w.start(w.rule("once", DecisionRule::AllOf, RejectRule::MajorityReject), &w.cy.eppn);
    let voters = w.state.repository.get_process_instance(&id).unwrap().unwrap().voters;
    assert_eq!(voters.len(), 2, "{voters:?}");
    assert_eq!(w.vote(&w.sam, &id, "approve", None).await.0, StatusCode::FORBIDDEN, "the submitter does not vote");
    assert_eq!(w.vote(&w.cy, &id, "approve", None).await.0, StatusCode::FORBIDDEN, "the starter does not vote");

    assert_eq!(w.vote(&w.ash, &id, "reject", Some("Not yet")).await.0, StatusCode::OK);
    assert_eq!(w.vote(&w.ash, &id, "reject", Some("Again")).await.0, StatusCode::CONFLICT, "the same vote twice");
    assert_eq!(w.vote(&w.ash, &id, "approve", None).await.0, StatusCode::OK, "a vote may change while the step is open");
    assert_eq!(w.status(&id), ProcessStatus::Waiting);
    w.vote(&w.bea, &id, "approve", None).await;
    assert_eq!(w.status(&id), ProcessStatus::Completed, "both voters approve, counting Ash's changed vote");
}

// 4. A person added to the position after the step started is not a voter.
#[tokio::test(flavor = "multi_thread")]
async fn someone_seated_after_the_step_started_does_not_vote() {
    let w = world().await;
    let id = w.start(w.rule("late", DecisionRule::AtLeast(2), RejectRule::FirstReject), &w.sam.eppn);
    let late = person("dee");
    let token = World::user(&w.app, &w.state, &w.admin, &late).await;
    w.seat(&late).await;
    let dee = Member { eppn: late.clone(), token };
    assert_eq!(w.vote(&dee, &id, "approve", None).await.0, StatusCode::FORBIDDEN);
    assert!(!w.state.repository.get_process_instance(&id).unwrap().unwrap().voters.contains(&late));
}

// 5. A reject without a comment is refused when required.
#[tokio::test(flavor = "multi_thread")]
async fn a_rejection_needs_a_comment_when_the_setting_says_so() {
    let w = world().await;
    let id = w.start(w.rule("comment", DecisionRule::AllOf, RejectRule::FirstReject), &w.sam.eppn);
    let (s, b) = w.vote(&w.ash, &id, "reject", None).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "required by default: {b}");
    assert_eq!(w.vote(&w.ash, &id, "reject", Some("  ")).await.0, StatusCode::BAD_REQUEST, "a blank comment is none");
    assert_eq!(w.status(&id), ProcessStatus::Waiting, "the refused vote counted for nothing");

    let (s, b) = send(&w.app, "PUT", "/api/v1/settings/process.require_reject_comment", &w.admin, Some(json!(false))).await;
    assert!(s.is_success(), "{b}");
    assert_eq!(w.vote(&w.ash, &id, "reject", None).await.0, StatusCode::OK);
    assert_eq!(w.status(&id), ProcessStatus::Rejected);
}

// 6. One ledger entry holds the tally and every voter.
#[tokio::test(flavor = "multi_thread")]
async fn one_ledger_entry_holds_the_tally_and_every_voter() {
    let w = world().await;
    let id = w.start(w.rule("ledger", DecisionRule::AtLeast(2), RejectRule::FirstReject), &w.sam.eppn);
    w.vote(&w.ash, &id, "approve", None).await;
    w.vote(&w.bea, &id, "reject", Some("Too costly")).await;
    w.vote(&w.cy, &id, "approve", None).await;
    assert_eq!(w.status(&id), ProcessStatus::Completed);

    let needle = id.clone();
    let entries: Vec<Value> = w.db.run(move |c| {
        c.query("SELECT payload FROM governance_ledger WHERE decision_type = 'WorkflowRuleApproved' AND payload->>'instance_id' = $1", &[&needle])
            .unwrap()
            .iter()
            .map(|r| r.get(0))
            .collect()
    });
    assert_eq!(entries.len(), 1, "one entry for the whole vote: {entries:?}");
    let e = &entries[0];
    assert_eq!(e["decision"], "approve");
    assert_eq!(e["tally"], json!({ "approve": 2, "reject": 1, "voters": 3 }));
    let votes = e["votes"].as_array().expect("the votes");
    assert_eq!(votes.len(), 3);
    for (m, vote) in [(&w.ash, "approve"), (&w.bea, "reject"), (&w.cy, "approve")] {
        let v = votes.iter().find(|v| v["voter"] == m.eppn.as_str()).unwrap_or_else(|| panic!("{} is in the entry", m.eppn));
        assert_eq!(v["vote"], vote);
        assert!(v["at"].as_str().is_some_and(|t| !t.is_empty()), "each vote has its time: {v}");
    }
}

// 7. The submitter cannot see individual votes.
#[tokio::test(flavor = "multi_thread")]
async fn the_submitter_cannot_see_individual_votes() {
    let w = world().await;
    let id = w.start(w.rule("secret", DecisionRule::AllOf, RejectRule::FirstReject), &w.sam.eppn);
    w.vote(&w.ash, &id, "approve", None).await;
    let uri = format!("/api/v1/apps/{APP}/processes/{id}/votes");

    let (s, b) = send(&w.app, "GET", &uri, &w.sam.token, None).await;
    assert_eq!(s, StatusCode::FORBIDDEN, "the submitter: {b}");
    for token in [&w.bea.token, &w.admin] {
        let (s, b) = send(&w.app, "GET", &uri, token, None).await;
        assert_eq!(s, StatusCode::OK, "{b}");
        let votes = b["votes"].as_array().unwrap();
        assert_eq!(votes.len(), 1);
        assert_eq!(votes[0]["voter"], w.ash.eppn.as_str());
        assert_eq!(votes[0]["vote"], "approve");
        assert_eq!(b["tally"], json!({ "approve": 1, "reject": 0, "voters": 3 }));
    }

    // The process list the submitter sees says how far the vote has got, not who voted how.
    let (_, rows) = send(&w.app, "GET", &format!("/api/v1/apps/{APP}/processes"), &w.sam.token, None).await;
    let row = rows.as_array().unwrap().iter().find(|r| r["id"] == id.as_str()).expect("listed");
    assert!(!row.to_string().contains("\"vote\""), "no individual vote in the list: {row}");
}

// Added after the code, for two parts of the brief tests 1 to 7 do not reach: a step with fewer voters than it needs
// says so with `no_approver`, and a rule that needs no approvals is refused.
#[tokio::test(flavor = "multi_thread")]
async fn too_few_voters_shows_no_approver_and_at_least_zero_is_refused() {
    let w = world().await;
    let id = w.start(w.rule("too-few", DecisionRule::AtLeast(4), RejectRule::FirstReject), &w.sam.eppn);
    let inst = w.state.repository.get_process_instance(&id).unwrap().unwrap();
    assert_eq!(inst.voters.len(), 3);
    assert_eq!(inst.no_approver.as_deref(), Some("too_few_voters"), "four approvals cannot come from three voters");

    let manifest = scaffoldry_engine::HostRouter::resolve_by_slug(&*w.state.engine.read().unwrap(), APP).unwrap().clone();
    let found = scaffoldry_server::service::processes::validate_rule(&w.state, &manifest, &w.rule("none", DecisionRule::AtLeast(0), RejectRule::FirstReject));
    assert!(found.iter().any(|p| p.contains("at least 0 approvals")), "{found:?}");
    assert!(scaffoldry_server::service::processes::validate_rule(&w.state, &manifest, &w.rule("two", DecisionRule::AtLeast(2), RejectRule::MajorityReject)).is_empty());
}
