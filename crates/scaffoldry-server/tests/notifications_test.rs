//! Notifications phase 1: the inbox.
//!
//! A notice carries names and links, never a record value. It is built only by `render` from a typed set of references
//! (an app, a table, a person, a step, a record id, a job, a count, a yes or no), so there is no field a caller can fill
//! with record content. `notify` resolves who is told, removes the actor and anyone inactive or on hold, and creates
//! nothing for someone who may not read the app the notice is about.

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use common::TestDb;
use http_body_util::BodyExt;
use scaffoldry_core::approver::{resolve_approvers, ApproverSpec, Delegation, Holding, Origin, Person, ResolveCtx, Unit};
use scaffoldry_server::build_app_with_state;
use scaffoldry_server::notifications::{self, AppRef, Count, Notice, PersonRef, Recipients, TableRef};
use scaffoldry_server::service::identity::issue_test_token_and_user;
use serde_json::{json, Value};
use tower::ServiceExt;

const ADMIN: &str = "jordan.lee@state.edu";
const OWNER: &str = "einstein@physics.state.edu";
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

fn inbox(db: &TestDb, who: &'static str) -> Vec<(String, String, String)> {
    db.run(move |c| {
        c.query("SELECT kind, title, body FROM notifications WHERE lower(recipient) = lower($1) ORDER BY created_at", &[&who])
            .unwrap()
            .iter()
            .map(|r| (r.get(0), r.get(1), r.get(2)))
            .collect()
    })
}

// 1. Approvers: a delegate is told as well, and the submitter and the actor are not. The resolver's answer, with a
// delegation in force, goes through the same filter `notify` uses.
#[test]
fn approvers_include_a_delegate_and_never_the_submitter_or_the_actor() {
    let now = 1_800_000_000;
    let person = |e: &str| Person { eppn: e.to_string(), display_name: e.to_string(), active: true, on_hold: false };
    let ctx = ResolveCtx {
        units: vec![Unit { id: "u1".into(), parent_id: None, org_type: "Department".into(), name: "Physics".into() }],
        holdings: vec![
            Holding { eppn: "chair@state.edu".into(), unit_id: "u1".into(), position_key: Some("chair".into()), is_primary: true },
            Holding { eppn: "submitter@state.edu".into(), unit_id: "u1".into(), position_key: Some("chair".into()), is_primary: true },
            Holding { eppn: "actor@state.edu".into(), unit_id: "u1".into(), position_key: Some("chair".into()), is_primary: true },
        ],
        people: vec![person("chair@state.edu"), person("submitter@state.edu"), person("actor@state.edu"), person("deputy@state.edu")],
        delegations: vec![Delegation { delegator: "chair@state.edu".into(), delegate: "deputy@state.edu".into(), position_key: None, starts_at: now - 10, ends_at: now + 10, revoked: false }],
        collaborators: vec![],
        now,
        submitter: Some("submitter@state.edu".into()),
        starter: None,
        workspace_unit: Some("u1".into()),
    };
    let spec = ApproverSpec::Position { key: "chair".into(), from: Origin::Workspace, walk_up: false };
    let resolved: Vec<String> = resolve_approvers(&spec, &ctx).approvers.into_iter().map(|a| a.eppn).collect();
    let told = notifications::recipients_of(resolved, Some("actor@state.edu"), Some("submitter@state.edu"), &ctx.people);
    assert!(told.iter().any(|e| e == "deputy@state.edu"), "the delegate is told: {told:?}");
    assert!(told.iter().any(|e| e == "chair@state.edu"), "{told:?}");
    assert!(!told.iter().any(|e| e == "submitter@state.edu"), "never the submitter: {told:?}");
    assert!(!told.iter().any(|e| e == "actor@state.edu"), "never the actor: {told:?}");
    // Someone inactive or on hold is not told, and nobody is told twice.
    let people = vec![person("a@state.edu"), Person { active: false, ..person("gone@state.edu") }, Person { on_hold: true, ..person("held@state.edu") }];
    let told = notifications::recipients_of(vec!["a@state.edu".into(), "A@state.edu".into(), "gone@state.edu".into(), "held@state.edu".into()], None, None, &people);
    assert_eq!(told, vec!["a@state.edu".to_string()]);
}

// 2. A person who cannot read the app gets no notice about it.
#[tokio::test]
async fn a_person_who_cannot_read_the_app_gets_no_notice_about_it() {
    let db = TestDb::new();
    let state = db.state();
    state.seed_demo().expect("seed");
    let app = build_app_with_state(state.clone()).expect("router");
    let slug = format!("notify-{}", &uuid::Uuid::new_v4().simple().to_string()[..8]);
    let manifest = json!({
        "slug": slug, "title": "Lab Bookings", "description": "", "organization_code": "PHYS", "department": "physics", "workspace_id": WORKSPACE, "views": [],
        "tables": [{ "id": "bookings", "name": "Bookings", "slug": "bookings", "primary_field": "title", "fields": [{ "name": "title", "label": "Title", "field_type": "Text", "required": false, "ferpa_sensitive": false }] }],
    });
    let (s, b) = send(&app, "POST", &format!("/api/v1/workspaces/{WORKSPACE}/apps"), OWNER, Some(manifest)).await;
    assert_eq!(s, StatusCode::CREATED, "{b}");
    let stranger = "stranger.n1@state.edu";
    issue_test_token_and_user(stranger);
    let notice = Notice::ProposalPending { by: PersonRef(ADMIN.into()), app: AppRef(slug.clone()) };
    let st = state.clone();
    let made = tokio::task::spawn_blocking(move || notifications::notify(&st, &Recipients::Person(stranger.into()), Some(ADMIN), &notice)).await.unwrap().unwrap();
    assert_eq!(made, 0, "nothing for someone who cannot read the app");
    assert!(inbox(&db, stranger).is_empty());
    let notice = Notice::ProposalPending { by: PersonRef(ADMIN.into()), app: AppRef(slug.clone()) };
    let st = state.clone();
    let made = tokio::task::spawn_blocking(move || notifications::notify(&st, &Recipients::Person(OWNER.into()), Some(ADMIN), &notice)).await.unwrap().unwrap();
    assert_eq!(made, 1);
    let got = inbox(&db, OWNER);
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].0, "proposal_pending");
    assert!(got[0].1.contains("Lab Bookings"), "the title names the app: {got:?}");
    // The actor is never told about their own act.
    let notice = Notice::ProposalPending { by: PersonRef(OWNER.into()), app: AppRef(slug) };
    let st = state.clone();
    let made = tokio::task::spawn_blocking(move || notifications::notify(&st, &Recipients::Person(OWNER.into()), Some(OWNER), &notice)).await.unwrap().unwrap();
    assert_eq!(made, 0);
}

// 3. No kind's arguments hold free text. Every field is a reference the server resolves to a name, a count, or a yes
// or no. The list below is generated from the same macro that defines the kinds, so it cannot drift from them.
#[test]
fn no_kind_takes_free_text() {
    let allowed = ["AppRef", "TableRef", "PersonRef", "RecordRef", "StepRef", "JobRef", "Count", "Flag"];
    assert_eq!(notifications::FIELDS.len(), 16, "one entry per kind");
    for (kind, fields) in notifications::FIELDS {
        assert!(notifications::KINDS.contains(kind), "{kind}");
        for (name, ty) in fields.iter() {
            assert!(allowed.contains(ty), "{kind}.{name} is a {ty}, which is not a reference");
        }
    }
    for kind in ["mention", "decision_waiting", "decision_made", "process_escalated", "process_overdue", "proposal_pending", "proposal_decided", "job_finished", "import_finished", "sync_failed", "webhook_disabled", "file_infected", "assigned", "delegation_received", "access_changed", "view_shared"] {
        assert!(notifications::KINDS.contains(&kind), "{kind}");
    }
    // A reference that names nothing the server holds renders as a plain word, never as what was passed.
    let db = TestDb::new();
    let state = db.state();
    let secret = "Diagnosis: severe";
    let n = Notice::ImportFinished { app: AppRef(secret.into()), table: TableRef(secret.into()), rows: Count(3), refused: Count(0) };
    let (title, body, _) = notifications::render(&state, &n);
    assert!(!title.contains(secret) && !body.contains(secret), "{title} / {body}");
    let n = Notice::DelegationReceived { by: PersonRef(secret.into()) };
    let (title, body, _) = notifications::render(&state, &n);
    assert!(!title.contains(secret) && !body.contains(secret), "{title} / {body}");
}

// 4. A body over 500 characters is cut at a word boundary.
#[test]
fn a_long_body_is_cut_at_a_word() {
    let words = "approval ".repeat(80);
    let cut = notifications::cut(&words, 500);
    assert!(cut.chars().count() <= 500, "{}", cut.chars().count());
    assert!(cut.ends_with('…'));
    let kept = cut.trim_end_matches('…');
    assert!(kept.ends_with("approval"), "cut at a word: {kept:?}");
    assert_eq!(notifications::cut("short", 500), "short");
}

// 5. `read` marks only the caller's own. Another person's id is ignored.
#[tokio::test]
async fn read_marks_only_the_callers_own() {
    let db = TestDb::new();
    let state = db.state();
    state.seed_demo().expect("seed");
    let app = build_app_with_state(state.clone()).expect("router");
    let (me, other) = ("reader.n1@state.edu", "other.n1@state.edu");
    for who in [me, other] {
        issue_test_token_and_user(who);
        let st = state.clone();
        let n = Notice::DelegationReceived { by: PersonRef(ADMIN.into()) };
        let made = tokio::task::spawn_blocking(move || notifications::notify(&st, &Recipients::Person(who.into()), Some(ADMIN), &n)).await.unwrap().unwrap();
        assert_eq!(made, 1);
    }
    let (s, count) = send(&app, "GET", "/api/v1/me/notifications/count", me, None).await;
    assert_eq!(s, StatusCode::OK, "{count}");
    assert_eq!(count["unread"], 1);
    let (_, list) = send(&app, "GET", "/api/v1/me/notifications", me, None).await;
    let mine = list["notifications"][0]["id"].as_str().unwrap().to_string();
    assert_eq!(list["notifications"][0]["read"], false);
    assert!(list["notifications"][0]["link"].is_object(), "{list}");
    let (_, theirs) = send(&app, "GET", "/api/v1/me/notifications", other, None).await;
    let theirs = theirs["notifications"][0]["id"].as_str().unwrap().to_string();
    let (s, b) = send(&app, "POST", "/api/v1/me/notifications/read", me, Some(json!({ "ids": [mine, theirs] }))).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_eq!(b["marked"], 1, "only my own");
    let (_, c) = send(&app, "GET", "/api/v1/me/notifications/count", other, None).await;
    assert_eq!(c["unread"], 1, "theirs is still unread");
    let (_, unread) = send(&app, "GET", "/api/v1/me/notifications?unread=true", me, None).await;
    assert_eq!(unread["notifications"].as_array().unwrap().len(), 0);
    let (s, b) = send(&app, "POST", "/api/v1/me/notifications/read", other, Some(json!({ "all": true }))).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_eq!(b["marked"], 1);
}

// 6. The purge job removes a 91-day-old read notice and keeps an unread one.
#[tokio::test]
async fn the_purge_removes_old_read_notices_and_keeps_unread_ones() {
    let db = TestDb::new();
    let state = db.state();
    let who = "purge.n1@state.edu";
    let (old_read, old_unread, recent_read, ancient_unread) = (uuid::Uuid::new_v4(), uuid::Uuid::new_v4(), uuid::Uuid::new_v4(), uuid::Uuid::new_v4());
    db.run(move |c| {
        for (id, days, read) in [(old_read, 91, true), (old_unread, 91, false), (recent_read, 10, true), (ancient_unread, 366, false)] {
            c.execute(
                "INSERT INTO notifications (id, recipient, kind, title, body, created_at, read_at) \
                 VALUES ($1, $2, 'job_finished', 't', 'b', NOW() - make_interval(days => $3), CASE WHEN $4 THEN NOW() - make_interval(days => $3) END)",
                &[&id, &who, &days, &read],
            )
            .unwrap();
        }
    });
    let repo = state.repository.clone();
    let removed = tokio::task::spawn_blocking(move || notifications::purge(&repo)).await.unwrap().unwrap();
    assert_eq!(removed, 2);
    let left: Vec<uuid::Uuid> = db.run(move |c| c.query("SELECT id FROM notifications WHERE recipient = $1 ORDER BY id", &[&who]).unwrap().iter().map(|r| r.get(0)).collect());
    let mut want = vec![old_unread, recent_read];
    want.sort();
    assert_eq!(left, want);
    assert!(scaffoldry_server::jobs::JOB_KINDS.iter().any(|k| k.name == "purge_notifications"));
}
