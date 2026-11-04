//! Process v2 phase 3: timers and escalation. No timer ever decides.
//!
//! A user task may say when it is due and what happens then: remind, escalate to someone, escalate to the same position
//! one unit up, or fail. Entering the step writes its timers. `fire_due` applies the timers that are due, once each. A
//! decision cancels the step's timers in the same transaction.
//!
//! The world: Physics sits under the College of Science. Rivera chairs Physics, and the dean holds the chair of the
//! College. Sam, in Physics, submits.

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::Router;
use chrono::{DateTime, Duration, Timelike, Utc};
use http_body_util::BodyExt;
use scaffoldry_core::approver::{ApproverSpec, Origin};
use scaffoldry_core::standards::eduperson::{EduPersonAffiliation, EduPersonIdentity};
use scaffoldry_core::workflow::{DueAction, DurationSpec};
use scaffoldry_core::{ActionType, AutomationRule, ProcessStatus, ProcessStep, StepKind, TriggerEvent};
use scaffoldry_server::build_app_with_state;
use scaffoldry_server::service::identity::issue_test_token_and_user;
use scaffoldry_server::service::process_timers::fire_due;
use scaffoldry_server::state::{CollaboratorRecord, DatasetRecord, ServerState, WorkspaceRecord};
use serde_json::{json, Value};
use std::sync::Arc;
use tower::ServiceExt;
use uuid::Uuid;

const ROOT_UNIT: &str = "00000000-0000-0000-0000-000000000001";
const APP: &str = "process-timers-app";
const WS: &str = "ws-process-timers";

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
    db: common::TestDb,
    app: Router,
    state: Arc<ServerState>,
    admin: String,
    sam: String,
    rivera: String,
    rivera_t: String,
    dean: String,
    dean_t: String,
}

impl World {
    async fn unit(&self, name: &str, org_type: &str, parent: &str) -> String {
        let (s, b) = send(&self.app, "POST", "/api/v1/orgs", &self.admin, Some(json!({
            "name": name, "code": &name[..4].to_uppercase(), "org_type": org_type, "parent_id": parent
        }))).await;
        assert_eq!(s, StatusCode::CREATED, "create unit: {b}");
        b["id"].as_str().unwrap().to_string()
    }

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

    async fn hold(&self, unit: &str, who: &str) {
        let (s, b) = send(&self.app, "POST", &format!("/api/v1/orgs/{unit}/positions/chair/holders"), &self.admin,
            Some(json!({ "eppn": who, "replace": false, "reason": "Fixture" }))).await;
        assert_eq!(s, StatusCode::CREATED, "assign: {b}");
    }

    /// A rule whose one user task goes to the submitter's chair, is due in `days`, and does `on_due` then.
    fn rule(&self, id: &str, days: u32, remind_every: Option<u32>, on_due: Vec<DueAction>, on_fail: Vec<ActionType>) -> AutomationRule {
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
                id: "chair-review".to_string(),
                when: vec![],
                kind: StepKind::UserTask {
                    role: "chair".to_string(),
                    prompt: "Review the request".to_string(),
                    approve: vec![ActionType::UpdateRecordStatus { new_status: "Approved".into() }],
                    reject: vec![],
                    approver: Some(ApproverSpec::Position { key: "chair".into(), from: Origin::Submitter, walk_up: true }),
                    due: Some(DurationSpec { days }),
                    remind_every: remind_every.map(|days| DurationSpec { days }),
                    on_due,
                    on_fail,
                    decision: Default::default(),
                    reject_rule: Default::default(),
                },
            }],
        }
    }

    /// Stores `rule` as the app's only rule, then a record Sam submitted, and runs the app's rules on it as Sam. The
    /// rule's step is entered, so its timers are written. Returns the instance id.
    fn start(&self, rule: AutomationRule) -> String {
        let rule_id = rule.id.clone();
        self.state.automations.write().unwrap().insert(APP.to_string(), vec![rule]);
        let record_id = format!("rec-{}", &Uuid::new_v4().simple().to_string()[..8]);
        self.state
            .put_record(
                &DatasetRecord {
                    id: record_id.clone(),
                    app_slug: APP.to_string(),
                    data: json!({ "_table_id": "requests", "title": "Sabbatical", "status": "Submitted" }),
                    ceds_mapping: Default::default(),
                    is_ferpa_sensitive: false,
                    created_at: String::new(),
                    created_by: Some(self.sam.clone()),
                    version: 1,
                },
                &scaffoldry_server::repository::WriteCtx::system("test fixture"),
            )
            .unwrap();
        let identity = EduPersonIdentity { eppn: self.sam.clone(), realm: "state.edu".into(), affiliations: vec![EduPersonAffiliation::Faculty] };
        let data = json!({ "id": record_id, "_table_id": "requests", "title": "Sabbatical", "status": "Submitted" });
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
        let inst = self.state.repository.get_process_instance(&id).unwrap().expect("the rule started an instance");
        assert_eq!(inst.status, ProcessStatus::Waiting);
        id
    }

    async fn decide(&self, token: &str, instance: &str) -> (StatusCode, Value) {
        send(&self.app, "POST", &format!("/api/v1/apps/{APP}/processes/{instance}/decide"), token, Some(json!({ "decision": "approve" }))).await
    }

    /// `(state, fire_at, action)` of each of the instance's timers, oldest first.
    fn timers(&self, instance: &str) -> Vec<(String, DateTime<Utc>, Value)> {
        let instance = instance.to_string();
        self.db.run(move |c| {
            c.query("SELECT state, fire_at, action FROM process_timers WHERE instance_id = $1 ORDER BY fire_at, id", &[&instance])
                .unwrap()
                .iter()
                .map(|r| (r.get(0), r.get(1), r.get(2)))
                .collect()
        })
    }

    /// How many notices of `kind` went to `who`.
    fn notices(&self, who: &str, kind: &str) -> i64 {
        let (who, kind) = (who.to_string(), kind.to_string());
        self.db.run(move |c| c.query_one("SELECT count(*) FROM notifications WHERE lower(recipient) = lower($1) AND kind = $2", &[&who, &kind]).unwrap().get(0))
    }

    fn record_status(&self, instance: &str) -> String {
        let inst = self.state.repository.get_process_instance(instance).unwrap().unwrap();
        let rec = self.state.find_record(APP, &inst.record_id).unwrap().unwrap();
        rec.data["status"].as_str().unwrap_or_default().to_string()
    }
}

async fn world() -> World {
    let db = common::TestDb::new();
    let admin = issue_test_token_and_user("jordan.lee@state.edu");
    let state = db.state();
    let app = build_app_with_state(state.clone()).expect("router");
    let mut w = World { db, app, state, admin, sam: String::new(), rivera: String::new(), rivera_t: String::new(), dean: String::new(), dean_t: String::new() };

    let college = w.unit("Science", "College", ROOT_UNIT).await;
    let physics = w.unit("Physics", "Department", &college).await;
    let ws: WorkspaceRecord = serde_json::from_value(json!({
        "id": WS, "name": "Process timers", "code": "PTIM", "organization": "State",
        "visibility": "restricted", "organization_id": physics,
    }))
    .unwrap();
    w.state.persist_workspace(ws).unwrap();
    let manifest: scaffoldry_engine::AppManifest = serde_json::from_value(json!({
        "slug": APP, "title": "Requests", "description": "", "organization_code": "PHYS", "department": "physics", "workspace_id": WS, "views": [],
        "tables": [{ "id": "requests", "name": "Requests", "slug": "requests", "primary_field": "title", "fields": [
            { "name": "title", "label": "Title", "field_type": "Text", "required": false, "ferpa_sensitive": false },
            { "name": "status", "label": "Status", "field_type": "Text", "required": false, "ferpa_sensitive": false }
        ] }],
    }))
    .unwrap();
    w.state.engine.write().unwrap().register_manifest(manifest).unwrap();

    w.sam = person("sam");
    w.rivera = person("rivera");
    w.dean = person("dean");
    w.user(&w.sam.clone(), "Physics").await;
    w.rivera_t = w.user(&w.rivera.clone(), "Physics").await;
    w.dean_t = w.user(&w.dean.clone(), "Science").await;
    let (s, b) = send(&w.app, "POST", "/api/v1/admin/positions", &w.admin, Some(json!({
        "key": "chair", "name": "Chair", "description": "", "org_types": ["Department", "College"], "max_holders": 1, "reason": "Fixture"
    })))
    .await;
    assert_eq!(s, StatusCode::CREATED, "{b}");
    w.hold(&physics, &w.rivera.clone()).await;
    w.hold(&college, &w.dean.clone()).await;
    w
}

// 1. A step due in 3 days with `EscalateUp` adds the dean at day 3, notifies old and new approvers and the owner, and the
// dean can decide.
#[tokio::test(flavor = "multi_thread")]
async fn escalate_up_adds_the_dean_at_day_three_and_the_dean_can_decide() {
    let w = world().await;
    let id = w.start(w.rule("escalate-up", 3, None, vec![DueAction::EscalateUp], vec![]));
    let timers = w.timers(&id);
    assert_eq!(timers.len(), 1, "entering the step wrote one timer: {timers:?}");
    assert_eq!(timers[0].0, "pending");

    assert_eq!(fire_due(&w.state, Utc::now() + Duration::days(2)).unwrap(), 0, "nothing is due at day 2");
    assert_eq!(w.decide(&w.dean_t, &id).await.0, StatusCode::FORBIDDEN, "the dean may not decide before the step escalates");

    assert_eq!(fire_due(&w.state, Utc::now() + Duration::days(3) + Duration::hours(1)).unwrap(), 1, "the timer fires at day 3");
    assert_eq!(w.timers(&id)[0].0, "fired");
    assert_eq!(w.notices(&w.dean, "process_escalated"), 1, "the dean is told");
    assert_eq!(w.notices(&w.rivera, "process_escalated"), 1, "the old approver is told");
    assert_eq!(w.notices(&w.sam, "process_escalated"), 1, "the record's owner is told");

    let inst = w.state.repository.get_process_instance(&id).unwrap().unwrap();
    assert_eq!(inst.status, ProcessStatus::Waiting, "escalating decides nothing");
    let (s, b) = w.decide(&w.dean_t, &id).await;
    assert_eq!(s, StatusCode::OK, "the dean can decide now: {b}");
    assert_eq!(w.record_status(&id), "Approved");
}

// 2. A decision before the timer fires cancels it. Nothing is escalated afterwards.
#[tokio::test(flavor = "multi_thread")]
async fn a_decision_before_the_timer_cancels_it() {
    let w = world().await;
    let id = w.start(w.rule("decided-early", 3, None, vec![DueAction::EscalateUp], vec![]));
    let (s, b) = w.decide(&w.rivera_t, &id).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_eq!(w.timers(&id)[0].0, "canceled", "the decision canceled the timer in its transaction");

    assert_eq!(fire_due(&w.state, Utc::now() + Duration::days(10)).unwrap(), 0, "nothing fires");
    assert_eq!(w.notices(&w.dean, "process_escalated"), 0, "the dean is never asked");
    let inst = w.state.repository.get_process_instance(&id).unwrap().unwrap();
    assert_eq!(inst.status, ProcessStatus::Completed);
}

// 3. `Fail` ends the instance with the reason and runs its `on_fail` actions.
#[tokio::test(flavor = "multi_thread")]
async fn fail_ends_the_instance_with_the_reason_and_runs_on_fail() {
    let w = world().await;
    let reason = "Nobody reviewed the request within five days";
    let id = w.start(w.rule("fails", 5, None, vec![DueAction::Fail { reason: reason.into() }], vec![ActionType::UpdateRecordStatus { new_status: "Expired".into() }]));
    assert_eq!(fire_due(&w.state, Utc::now() + Duration::days(6)).unwrap(), 1);

    let inst = w.state.repository.get_process_instance(&id).unwrap().unwrap();
    assert_eq!(inst.status, ProcessStatus::Failed);
    assert_eq!(inst.waiting_step_id, None);
    assert!(inst.log.iter().any(|l| l.contains(reason)), "the log holds the reason: {:?}", inst.log);
    assert_eq!(w.record_status(&id), "Expired", "the step's on_fail actions ran");
    assert_eq!(w.decide(&w.rivera_t, &id).await.0, StatusCode::CONFLICT, "a failed instance is not decided");
}

// 4. `Remind` with `remind_every` of 1 day sends at most 5 reminders.
#[tokio::test(flavor = "multi_thread")]
async fn remind_every_day_sends_at_most_five_reminders() {
    let w = world().await;
    let id = w.start(w.rule("reminds", 1, Some(1), vec![DueAction::Remind], vec![]));
    let mut fired = 0;
    for day in 1..=10 {
        fired += fire_due(&w.state, Utc::now() + Duration::days(day) + Duration::hours(1)).unwrap();
    }
    assert_eq!(fired, 5, "five reminders and no more");
    assert_eq!(w.notices(&w.rivera, "process_overdue"), 5, "the approver was reminded five times");
    assert!(w.timers(&id).iter().all(|t| t.0 != "pending"), "no reminder is left waiting");
    let inst = w.state.repository.get_process_instance(&id).unwrap().unwrap();
    assert_eq!(inst.status, ProcessStatus::Waiting, "reminding decides nothing");
}

// 5. A timer is applied once even if two job threads run `fire_timers`.
#[tokio::test(flavor = "multi_thread")]
async fn a_timer_is_applied_once_when_two_threads_fire() {
    let w = world().await;
    let id = w.start(w.rule("raced", 3, None, vec![DueAction::Escalate { approver: ApproverSpec::Person { eppn: w.dean.clone() } }], vec![]));
    let at = Utc::now() + Duration::days(4);
    let handles: Vec<_> = (0..2)
        .map(|_| {
            let state = w.state.clone();
            std::thread::spawn(move || fire_due(&state, at).unwrap())
        })
        .collect();
    let total: usize = handles.into_iter().map(|h| h.join().unwrap()).sum();
    assert_eq!(total, 1, "one of the two threads applied the timer");
    assert_eq!(w.notices(&w.dean, "process_escalated"), 1, "the dean was told once");
    let inst = w.state.repository.get_process_instance(&id).unwrap().unwrap();
    assert_eq!(inst.escalated_to, vec![w.dean.clone()], "the dean was added once");
}

// 6. No `DueAction` variant approves or rejects. The match below has no wildcard, so a new variant fails to compile here
// until it is named, and the list is checked.
#[test]
fn no_due_action_approves_or_rejects() {
    fn kind(a: &DueAction) -> &'static str {
        match a {
            DueAction::Remind => "Remind",
            DueAction::Escalate { .. } => "Escalate",
            DueAction::EscalateUp => "EscalateUp",
            DueAction::Fail { .. } => "Fail",
        }
    }
    let all = [DueAction::Remind, DueAction::Escalate { approver: ApproverSpec::Role { name: "dean".into() } }, DueAction::EscalateUp, DueAction::Fail { reason: "late".into() }];
    let kinds: Vec<&str> = all.iter().map(kind).collect();
    assert_eq!(kinds, DueAction::KINDS, "every variant is listed");
    for k in kinds {
        let k = k.to_lowercase();
        assert!(!k.contains("approve") && !k.contains("reject") && !k.contains("decide"), "{k} decides");
    }
}

// 7. The console shows the next timer in words.
#[tokio::test(flavor = "multi_thread")]
async fn the_next_timer_is_shown_in_words() {
    let w = world().await;
    let id = w.start(w.rule("worded", 3, None, vec![DueAction::EscalateUp], vec![]));
    let fire_at = w.timers(&id)[0].1;
    let (s, rows) = send(&w.app, "GET", &format!("/api/v1/apps/{APP}/processes"), &w.admin, None).await;
    assert_eq!(s, StatusCode::OK, "{rows}");
    let row = rows.as_array().unwrap().iter().find(|r| r["id"] == id.as_str()).expect("the instance is listed");
    let words = row["next_timer"]["sentence"].as_str().unwrap_or_default();
    let day = fire_at.format("%-d %b").to_string();
    assert_eq!(words, format!("On {day}, this will be sent to the Chair of Science."), "{row}");

    let fail = w.start(w.rule("worded-fail", 2, None, vec![DueAction::Fail { reason: "Too late".into() }], vec![]));
    let (_, rows) = send(&w.app, "GET", &format!("/api/v1/apps/{APP}/processes"), &w.admin, None).await;
    let row = rows.as_array().unwrap().iter().find(|r| r["id"] == fail.as_str()).unwrap();
    assert!(row["next_timer"]["sentence"].as_str().unwrap_or_default().ends_with("this will fail: Too late."), "{row}");
}

// 8. Changing the platform time zone changes when new timers fire and not ones already set.
#[tokio::test(flavor = "multi_thread")]
async fn a_new_time_zone_moves_new_timers_and_not_old_ones() {
    let w = world().await;
    let (s, b) = send(&w.app, "PUT", "/api/v1/settings/platform.timezone", &w.admin, Some(json!("UTC"))).await;
    assert!(s.is_success(), "{b}");
    let before = w.start(w.rule("zone-before", 3, None, vec![DueAction::Remind], vec![]));
    let first = w.timers(&before)[0].1;
    assert_eq!((first.hour(), first.minute()), (0, 0), "due counts whole days: midnight in UTC");

    let (s, b) = send(&w.app, "PUT", "/api/v1/settings/platform.timezone", &w.admin, Some(json!("Pacific/Kiritimati"))).await;
    assert!(s.is_success(), "{b}");
    let after = w.start(w.rule("zone-after", 3, None, vec![DueAction::Remind], vec![]));
    let second = w.timers(&after)[0].1;
    assert_eq!((second.hour(), second.minute()), (10, 0), "midnight in Kiritimati (UTC+14) is 10:00 UTC");
    assert_eq!(w.timers(&before)[0].1, first, "the timer already set did not move");
}

// Added with the validation, after it: a rule whose timers make no sense is refused when it is saved, and says why.
#[tokio::test(flavor = "multi_thread")]
async fn a_rule_with_timers_that_make_no_sense_is_refused() {
    let w = world().await;
    let manifest = scaffoldry_engine::HostRouter::resolve_by_slug(&*w.state.engine.read().unwrap(), APP).unwrap().clone();
    let problems = |rule: AutomationRule| scaffoldry_server::service::processes::validate_rule(&w.state, &manifest, &rule);
    let fine = w.rule("fine", 3, Some(1), vec![DueAction::Remind, DueAction::EscalateUp], vec![ActionType::UpdateRecordStatus { new_status: "Expired".into() }]);
    assert!(problems(fine.clone()).is_empty(), "{:?}", problems(fine));

    let cases: Vec<(AutomationRule, &str)> = vec![
        (w.rule("zero", 0, None, vec![DueAction::Remind], vec![]), "due in 0 days"),
        (w.rule("long", 400, None, vec![DueAction::Remind], vec![]), "due in 400 days"),
        (w.rule("every", 3, Some(1), vec![DueAction::EscalateUp], vec![]), "no Remind"),
        (w.rule("hook", 3, None, vec![DueAction::Fail { reason: "late".into() }], vec![ActionType::WebhookDispatch { target_url: "https://example.org".into() }]), "webhook"),
        (w.rule("nobody", 3, None, vec![DueAction::Escalate { approver: ApproverSpec::Position { key: "provost".into(), from: Origin::Workspace, walk_up: false } }], vec![]), "escalates to someone it cannot find"),
    ];
    for (rule, words) in cases {
        let found = problems(rule.clone());
        assert!(found.iter().any(|p| p.contains(words)), "{}: expected '{words}' in {found:?}", rule.id);
    }

    let mut no_due = w.rule("no-due", 3, None, vec![DueAction::Remind], vec![]);
    let mut by_role = w.rule("by-role", 3, None, vec![DueAction::EscalateUp], vec![]);
    for (rule, role_only) in [(&mut no_due, false), (&mut by_role, true)] {
        if let StepKind::UserTask { due, approver, .. } = &mut rule.steps[0].kind {
            if role_only {
                *approver = Some(ApproverSpec::Role { name: "chair".into() });
            } else {
                *due = None;
            }
        }
    }
    assert!(problems(no_due).iter().any(|p| p.contains("Set due")), "on_due without due");
    assert!(problems(by_role).iter().any(|p| p.contains("needs a position")), "EscalateUp without a position");
}
