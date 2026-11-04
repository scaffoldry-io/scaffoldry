//! Notifications phase 4: every producer calls `notify`.
//!
//! Each test triggers the event the way the platform does, then checks who was told, the kind, and that the notice
//! holds names and counts only: never the record's values, a comment's words or a rule's message text. Producers whose
//! briefs are not built yet (escalation and overdue steps, syncs, webhooks) have nothing to wire.

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use common::TestDb;
use http_body_util::BodyExt;
use scaffoldry_core::{ActionType, AutomationRule, ProcessStep, StepKind, TriggerEvent};
use scaffoldry_server::build_app_with_state;
use scaffoldry_server::service::identity::issue_test_token_and_user;
use scaffoldry_server::state::ServerState;
use serde_json::{json, Value};
use std::sync::Arc;
use std::time::Duration;
use tower::ServiceExt;

const ADMIN: &str = "jordan.lee@state.edu";
const OWNER: &str = "einstein@physics.state.edu";
const WORKSPACE: &str = "ws-physics-optics";
const ROOT: &str = "00000000-0000-0000-0000-000000000001";
/// A record value to look for where it must not be.
const VALUE: &str = "Confidential-VALUE-7731";

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
    let slug = format!("prod-{}", &uuid::Uuid::new_v4().simple().to_string()[..8]);
    let manifest = json!({
        "slug": slug, "title": "Lab Bookings", "description": "", "organization_code": "PHYS", "department": "physics", "workspace_id": WORKSPACE, "views": [],
        "tables": [{ "id": "bookings", "name": "Bookings", "slug": "bookings", "primary_field": "title", "fields": [
            { "name": "title", "label": "Title", "field_type": "Text", "required": false, "ferpa_sensitive": false },
            { "name": "status", "label": "Status", "field_type": "Text", "required": false, "ferpa_sensitive": false }
        ] }],
    });
    let (s, b) = send(&app, "POST", &format!("/api/v1/workspaces/{WORKSPACE}/apps"), OWNER, Some(manifest)).await;
    assert_eq!(s, StatusCode::CREATED, "{b}");
    World { db, state, app, slug }
}

/// `(kind, title, body)` of each notice `who` has, oldest first.
fn inbox(w: &World, who: &str) -> Vec<(String, String, String)> {
    let who = who.to_string();
    w.db.run(move |c| {
        c.query("SELECT kind, title, body FROM notifications WHERE lower(recipient) = lower($1) ORDER BY created_at", &[&who]).unwrap().iter().map(|r| (r.get(0), r.get(1), r.get(2))).collect()
    })
}

fn kinds(w: &World, who: &str) -> Vec<String> {
    inbox(w, who).into_iter().map(|n| n.0).collect()
}

/// No notice anywhere holds the record value or the given words.
fn clean(w: &World, words: &[&str]) {
    let all: Vec<String> = w.db.run(|c| c.query("SELECT title || ' ' || body FROM notifications", &[]).unwrap().iter().map(|r| r.get(0)).collect());
    for text in &all {
        assert!(!text.contains(VALUE), "a record value in a notice: {text}");
        for word in words {
            assert!(!text.contains(word), "{word:?} in a notice: {text}");
        }
    }
}

async fn record(w: &World, who: &str) -> String {
    let (s, b) = send(&w.app, "POST", &format!("/api/v1/apps/{}/records", w.slug), who, Some(json!({ "data": { "_table_id": "bookings", "title": VALUE, "status": "Submitted" } }))).await;
    assert!(s.is_success(), "{b}");
    b["id"].as_str().or_else(|| b["record"]["id"].as_str()).unwrap().to_string()
}

async fn collaborator(w: &World, who: &str, role: &str) {
    issue_test_token_and_user(who);
    let (s, b) = send(&w.app, "POST", &format!("/api/v1/workspaces/{WORKSPACE}/collaborators"), ADMIN, Some(json!({ "eppn": who, "role": role }))).await;
    assert!(s.is_success(), "{b}");
}

fn step_rule(slug: &str, kind: StepKind) -> AutomationRule {
    AutomationRule {
        home_workspace: None,
        grants: vec![],
        version: 0,
        id: format!("rule-{}", uuid::Uuid::new_v4().simple()),
        app_slug: slug.to_string(),
        name: "Booking review".to_string(),
        description: String::new(),
        enabled: true,
        trigger: TriggerEvent::RecordCreated,
        predicates: vec![],
        actions: vec![],
        steps: vec![ProcessStep { id: "review".to_string(), when: vec![], kind }],
    }
}

async fn wait_for(w: &World, who: &str, kind: &str) -> bool {
    for _ in 0..100 {
        if kinds(w, who).iter().any(|k| k == kind) {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    false
}

// mention: the mentioned person, never the comment's words.
#[tokio::test]
async fn a_mention_tells_the_mentioned_person() {
    let w = world().await;
    let reader = format!("reader-{}@physics.state.edu", &w.slug[5..]);
    collaborator(&w, &reader, "viewer").await;
    let id = record(&w, OWNER).await;
    let (s, b) = send(&w.app, "POST", &format!("/api/v1/apps/{}/records/{id}/comments", w.slug), OWNER, Some(json!({ "body": format!("Secret-Comment-Words @[{reader}] please look") }))).await;
    assert!(s.is_success(), "{b}");
    assert!(b["mentions"].as_array().is_some_and(|m| !m.is_empty()), "the comment kept the mention: {b}");
    assert!(kinds(&w, &reader).contains(&"mention".to_string()), "{:?}", inbox(&w, &reader));
    assert!(kinds(&w, OWNER).is_empty(), "never the author");
    clean(&w, &["Secret-Comment-Words"]);
}

// decision_waiting: the step's approvers. decision_made: the record's creator and the starter.
#[tokio::test]
async fn a_waiting_step_tells_its_approvers_and_a_decision_tells_the_creator() {
    let w = world().await;
    let approver = scaffoldry_core::approver::ApproverSpec::Person { eppn: OWNER.to_string() };
    let rule = step_rule(&w.slug, StepKind::UserTask {
        role: String::new(),
        prompt: "Approve this booking".to_string(),
        approve: vec![ActionType::UpdateRecordStatus { new_status: "Approved".into() }],
        reject: vec![],
        approver: Some(approver), due: None, remind_every: None, on_due: vec![], on_fail: vec![],
    });
    w.state.automations.write().unwrap().entry(w.slug.clone()).or_default().push(rule);
    let id = record(&w, ADMIN).await;
    assert_eq!(kinds(&w, OWNER), vec!["decision_waiting".to_string()], "the approver is told");
    let instance: String = {
        let slug = w.slug.clone();
        w.db.run(move |c| c.query_one("SELECT id FROM process_instances WHERE app_slug = $1", &[&slug]).unwrap().get(0))
    };
    let (s, b) = send(&w.app, "POST", &format!("/api/v1/apps/{}/processes/{instance}/decide", w.slug), OWNER, Some(json!({ "decision": "approve" }))).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_eq!(kinds(&w, ADMIN), vec!["decision_made".to_string()], "the creator, who also started it, is told once");
    let (_, title, body) = inbox(&w, ADMIN).pop().unwrap();
    assert!(title.contains("Lab Bookings") && body.to_lowercase().contains("approved"), "{title} / {body}");
    assert!(!body.contains(&id));
    clean(&w, &[]);
}

// A Notify effect for a position reaches its holders and their delegates, with no message text.
#[tokio::test]
async fn a_notify_effect_for_a_position_reaches_its_holders_and_delegates() {
    let w = world().await;
    let (holder, deputy) = (format!("holder-{}@physics.state.edu", &w.slug[5..]), format!("deputy-{}@physics.state.edu", &w.slug[5..]));
    for who in [&holder, &deputy] {
        let (s, b) = send(&w.app, "POST", "/api/v1/admin/users", ADMIN, Some(json!({ "userName": who, "name": "Test Person", "email": who, "affiliation": "faculty", "department": "Physics", "title": "Lecturer", "reason": "Fixture" }))).await;
        assert_eq!(s, StatusCode::CREATED, "{b}");
        collaborator(&w, who, "viewer").await;
    }
    let unit: String = {
        let ws = w.state.workspaces.read().unwrap().get(WORKSPACE).and_then(|x| x.organization_id).map(|u| u.to_string());
        match ws {
            Some(u) => u,
            None => {
                let (s, b) = send(&w.app, "POST", "/api/v1/orgs", ADMIN, Some(json!({ "name": "Optics", "code": "OPTX", "org_type": "Department", "parent_id": ROOT }))).await;
                assert_eq!(s, StatusCode::CREATED, "{b}");
                b["id"].as_str().unwrap().to_string()
            }
        }
    };
    let key = format!("head{}", &w.slug[5..]);
    let (s, b) = send(&w.app, "POST", "/api/v1/admin/positions", ADMIN, Some(json!({ "key": key, "name": "Head", "org_types": ["Department", "College", "Institution"], "max_holders": 1, "reason": "Fixture" }))).await;
    assert_eq!(s, StatusCode::CREATED, "{b}");
    let (s, b) = send(&w.app, "POST", &format!("/api/v1/orgs/{unit}/positions/{key}/holders"), ADMIN, Some(json!({ "eppn": holder, "reason": "Fixture" }))).await;
    assert_eq!(s, StatusCode::CREATED, "{b}");
    let now = chrono::Utc::now();
    let (s, b) = send(&w.app, "POST", "/api/v1/me/delegations", &holder, Some(json!({ "delegate": deputy, "starts_at": (now - chrono::Duration::hours(1)).to_rfc3339(), "ends_at": (now + chrono::Duration::days(3)).to_rfc3339(), "reason": "Away" }))).await;
    assert_eq!(s, StatusCode::CREATED, "{b}");
    // delegation_received: the delegate.
    assert!(kinds(&w, &deputy).contains(&"delegation_received".to_string()), "{:?}", inbox(&w, &deputy));

    let rule = step_rule(&w.slug, StepKind::Service { action: ActionType::NotifyCollaborator { role: key.clone(), message_template: "Secret-Rule-Message".to_string() } });
    w.state.automations.write().unwrap().entry(w.slug.clone()).or_default().push(rule);
    record(&w, ADMIN).await;
    assert!(kinds(&w, &holder).contains(&"process_notice".to_string()), "{:?}", inbox(&w, &holder));
    assert!(kinds(&w, &deputy).contains(&"process_notice".to_string()), "the delegate too: {:?}", inbox(&w, &deputy));
    clean(&w, &["Secret-Rule-Message"]);
}

// proposal_pending: the workspace's owners and admins other than the proposer. proposal_decided: the proposer.
#[tokio::test]
async fn a_proposal_tells_the_owners_and_its_decision_tells_the_proposer() {
    let w = world().await;
    let editor = format!("editor-{}@physics.state.edu", &w.slug[5..]);
    collaborator(&w, &editor, "editor").await;
    let caller = scaffoldry_server::service::identity::resolve_user(&editor, &w.state).expect("the editor");
    let mut manifest = serde_json::to_value({
        use scaffoldry_engine::HostRouter;
        w.state.engine.read().unwrap().resolve_by_slug(&w.slug).unwrap().clone()
    })
    .unwrap();
    manifest["description"] = json!("A clearer description");
    let input = scaffoldry_server::service::proposals::ProposeInput { workspace_id: WORKSPACE.into(), app_slug: Some(w.slug.clone()), manifest: Some(manifest), rules: vec![], summary: "Describe it".into(), page_sources: vec![], notes: vec![] };
    let st = w.state.clone();
    let made = tokio::task::spawn_blocking(move || scaffoldry_server::service::proposals::propose(&caller, input, &st)).await.unwrap().expect("proposed");
    let id = made["id"].as_str().or_else(|| made["proposal"]["id"].as_str()).expect("an id").to_string();
    assert!(kinds(&w, OWNER).contains(&"proposal_pending".to_string()), "the owner is told: {:?}", inbox(&w, OWNER));
    let about = |who: &str| kinds(&w, who).into_iter().filter(|k| k.starts_with("proposal_")).collect::<Vec<_>>();
    assert!(about(&editor).is_empty(), "never the proposer: {:?}", inbox(&w, &editor));

    let decider = scaffoldry_server::service::identity::resolve_user(OWNER, &w.state).unwrap();
    let st = w.state.clone();
    tokio::task::spawn_blocking(move || scaffoldry_server::service::proposals::decide(&decider, &id, "reject", "Not now", None, &st)).await.unwrap().expect("decided");
    assert_eq!(about(&editor), vec!["proposal_decided".to_string()], "the proposer is told");
    clean(&w, &["A clearer description"]);
}

// job_finished: the job's owner, and nobody else.
#[tokio::test]
async fn a_finished_job_tells_its_owner() {
    let w = world().await;
    let job = scaffoldry_server::jobs::enqueue(&w.state.repository, "purge_notifications", json!({}), OWNER, None).expect("queued");
    let cfg = scaffoldry_server::jobs::JobConfig {
        workers: 1,
        poll_ms: 20,
        on_finish: Some(scaffoldry_server::notifications::job_hook(w.state.clone())),
        state: None,
        ..scaffoldry_server::jobs::JobConfig::default()
    };
    let _runner = scaffoldry_server::jobs::JobRunner::start(&w.state.repository, scaffoldry_server::jobs::JOB_KINDS, cfg).expect("runner");
    assert!(wait_for(&w, OWNER, "job_finished").await, "the owner is told: {:?}", inbox(&w, OWNER));
    let (_, title, _) = inbox(&w, OWNER).pop().unwrap();
    assert!(title.contains("Clearing old notices") || title.to_lowercase().contains("finished"), "{title}");
    assert!(!title.contains(&job.to_string()));
    assert!(kinds(&w, ADMIN).is_empty());
}

// access_changed: the person whose access to a workspace was granted or removed.
#[tokio::test]
async fn a_person_given_or_losing_access_is_told() {
    let w = world().await;
    let who = format!("newcomer-{}@physics.state.edu", &w.slug[5..]);
    collaborator(&w, &who, "viewer").await;
    assert_eq!(kinds(&w, &who), vec!["access_changed".to_string()], "granted");
    let (s, b) = send(&w.app, "DELETE", &format!("/api/v1/workspaces/{WORKSPACE}/collaborators/{who}"), ADMIN, None).await;
    assert!(s.is_success(), "{b}");
    assert_eq!(kinds(&w, &who).len(), 2, "removed: {:?}", inbox(&w, &who));
}

// Only `notify` and the mail module write to the outbox: no producer mails anyone directly.
#[test]
fn only_notify_writes_to_the_outbox() {
    let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut found = Vec::new();
    fn walk(dir: &std::path::Path, found: &mut Vec<String>) {
        for e in std::fs::read_dir(dir).unwrap().flatten() {
            let p = e.path();
            if p.is_dir() {
                walk(&p, found);
            } else if p.extension().is_some_and(|x| x == "rs") {
                let text = std::fs::read_to_string(&p).unwrap();
                if text.contains(concat!("INSERT INTO ", "email_outbox")) || text.contains(concat!("mail::", "queue(")) {
                    found.push(p.display().to_string());
                }
            }
        }
    }
    walk(&src, &mut found);
    let outside: Vec<&String> = found.iter().filter(|f| !f.ends_with("src/mail.rs") && !f.ends_with("src/notifications.rs") && !f.ends_with("src/service/mail.rs")).collect();
    assert!(outside.is_empty(), "mail written outside notify and the mail module: {outside:?}");
}

// assigned: a collaborator field that adds someone tells that person, on create and on update, once.
#[tokio::test]
async fn a_collaborator_field_that_adds_someone_tells_them() {
    let w = world().await;
    let slug = format!("{}-c", w.slug);
    let manifest = json!({
        "slug": slug, "title": "Case Work", "description": "", "organization_code": "PHYS", "department": "physics", "workspace_id": WORKSPACE, "views": [],
        "tables": [{ "id": "cases", "name": "Cases", "slug": "cases", "primary_field": "title", "fields": [
            { "name": "title", "label": "Title", "field_type": "Text", "required": false, "ferpa_sensitive": false },
            { "name": "owners", "label": "Owners", "field_type": "Collaborator", "required": false, "ferpa_sensitive": false }
        ] }],
    });
    let (s, b) = send(&w.app, "POST", &format!("/api/v1/workspaces/{WORKSPACE}/apps"), OWNER, Some(manifest)).await;
    assert_eq!(s, StatusCode::CREATED, "{b}");
    let (one, two) = (format!("one-{}@physics.state.edu", &w.slug[5..]), format!("two-{}@physics.state.edu", &w.slug[5..]));
    for who in [&one, &two] {
        let (s, b) = send(&w.app, "POST", "/api/v1/admin/users", ADMIN, Some(json!({ "userName": who, "name": "Test Person", "email": who, "affiliation": "faculty", "department": "Physics", "title": "Lecturer", "reason": "Fixture" }))).await;
        assert_eq!(s, StatusCode::CREATED, "{b}");
        collaborator(&w, who, "viewer").await;
    }
    let assigned = |who: &str| kinds(&w, who).into_iter().filter(|k| k == "assigned").count();
    let (s, b) = send(&w.app, "POST", &format!("/api/v1/apps/{slug}/records"), OWNER, Some(json!({ "data": { "_table_id": "cases", "title": VALUE, "owners": one } }))).await;
    assert!(s.is_success(), "{b}");
    assert_eq!(assigned(&one), 1, "{:?}", inbox(&w, &one));
    let id = b["id"].as_str().or_else(|| b["record"]["id"].as_str()).unwrap().to_string();
    let version = b["version"].as_i64().or_else(|| b["record"]["version"].as_i64()).unwrap_or(1);
    let (s, b) = send(&w.app, "PATCH", &format!("/api/v1/apps/{slug}/records/{id}"), OWNER, Some(json!({ "version": version, "data": { "_table_id": "cases", "title": VALUE, "owners": two } }))).await;
    assert!(s.is_success(), "{b}");
    assert_eq!(assigned(&two), 1, "the newly added person: {:?}", inbox(&w, &two));
    assert_eq!(assigned(&one), 1, "not told twice");
    clean(&w, &[]);
}
