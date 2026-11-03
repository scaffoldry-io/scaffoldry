//! Notifications phase 3: preferences and digests.
//!
//! Each person chooses, per kind, `in_app`, `email`, `both`, `digest` or `off`. The institution's defaults, in the
//! setting `notifications.defaults`, apply to anyone with no choice of their own. A decision waiting on a person, a
//! change to their own access and an infected file can never be set below `in_app`. The `send_digests` job, at 07:00 in
//! `platform.timezone`, sends each person one mail with a line per pending notice, grouped by app: their `digest`
//! notices, and those the hourly limit held back.

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use common::TestDb;
use http_body_util::BodyExt;
use scaffoldry_server::build_app_with_state;
use scaffoldry_server::mail;
use scaffoldry_server::notifications::{self, AppRef, Count, Notice, PersonRef, RecordRef, Recipients, StepRef, TableRef};
use scaffoldry_server::service::identity::issue_test_token_and_user;
use scaffoldry_server::state::ServerState;
use serde_json::{json, Value};
use std::sync::Arc;
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

struct World {
    db: TestDb,
    state: Arc<ServerState>,
    app: axum::Router,
    slug: String,
}

fn put_settings(db: &TestDb, values: Vec<(&'static str, Value)>) {
    db.run(move |c| {
        for (k, v) in values {
            c.execute("INSERT INTO platform_settings (key, value, updated_by) VALUES ($1, $2, 'test') ON CONFLICT (key) DO UPDATE SET value = EXCLUDED.value", &[&k, &v]).unwrap();
        }
    });
}

/// Mail on (nothing is actually sent here), an app the owner can read, and the given institution defaults.
async fn world(defaults: Option<Value>) -> World {
    let db = TestDb::new();
    let mut settings = vec![
        ("mail.enabled", json!(true)),
        ("mail.smtp", json!({ "host": "mail.test", "port": 465, "security": "tls", "from": "desk@state.edu" })),
        ("platform.base_url", json!("https://desk.state.edu")),
        ("platform.timezone", json!("America/Chicago")),
    ];
    if let Some(d) = defaults {
        settings.push(("notifications.defaults", d));
    }
    put_settings(&db, settings);
    let state = db.state();
    state.seed_demo().expect("seed");
    let app = build_app_with_state(state.clone()).expect("router");
    let slug = format!("prefs-{}", &uuid::Uuid::new_v4().simple().to_string()[..8]);
    let manifest = json!({
        "slug": slug, "title": "Lab Bookings", "description": "", "organization_code": "PHYS", "department": "physics", "workspace_id": WORKSPACE, "views": [],
        "tables": [{ "id": "bookings", "name": "Bookings", "slug": "bookings", "primary_field": "title", "fields": [{ "name": "title", "label": "Title", "field_type": "Text", "required": false, "ferpa_sensitive": false }] }],
    });
    let (s, b) = send(&app, "POST", &format!("/api/v1/workspaces/{WORKSPACE}/apps"), OWNER, Some(manifest)).await;
    assert_eq!(s, StatusCode::CREATED, "{b}");
    World { db, state, app, slug }
}

async fn notify(w: &World, notice: Notice) -> usize {
    let st = w.state.clone();
    tokio::task::spawn_blocking(move || notifications::notify(&st, &Recipients::Person(OWNER.into()), Some(ADMIN), &notice)).await.unwrap().unwrap()
}

fn mention(w: &World) -> Notice {
    Notice::Mention { by: PersonRef(ADMIN.into()), app: AppRef(w.slug.clone()), table: TableRef("bookings".into()), record: RecordRef("r1".into()) }
}

fn waiting(w: &World) -> Notice {
    Notice::DecisionWaiting { app: AppRef(w.slug.clone()), table: TableRef("bookings".into()), step: StepRef("nothing".into()), waiting: Count(3) }
}

fn inbox(db: &TestDb) -> Vec<(String, String, bool)> {
    db.run(|c| {
        c.query("SELECT kind, email_state, read_at IS NOT NULL FROM notifications WHERE lower(recipient) = lower($1) ORDER BY created_at", &[&OWNER])
            .unwrap()
            .iter()
            .map(|r| (r.get(0), r.get(1), r.get(2)))
            .collect()
    })
}

fn outbox(db: &TestDb) -> Vec<(String, String, String)> {
    db.run(|c| c.query("SELECT to_address, subject, body FROM email_outbox ORDER BY created_at", &[]).unwrap().iter().map(|r| (r.get(0), r.get(1), r.get(2))).collect())
}

async fn choose(w: &World, kind: &str, channel: &str) -> (StatusCode, Value) {
    send(&w.app, "PUT", "/api/v1/me/notification-prefs", OWNER, Some(json!({ "kind": kind, "channel": channel }))).await
}

// 1. A person who chose `off` for `mention` gets no notice. For `decision_waiting`, `off` is refused with a message.
#[tokio::test]
async fn off_means_no_notice_and_a_waiting_decision_cannot_be_turned_off() {
    let w = world(None).await;
    let (s, b) = choose(&w, "mention", "off").await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_eq!(notify(&w, mention(&w)).await, 0, "no notice when the person turned it off");
    assert!(inbox(&w.db).is_empty());
    assert!(outbox(&w.db).is_empty());

    let (s, b) = choose(&w, "decision_waiting", "off").await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    let message = b["error"].as_str().unwrap_or_default();
    assert!(message.contains("cannot") && message.to_lowercase().contains("decision"), "the refusal says why: {message}");
    for kind in ["access_changed", "file_infected"] {
        assert_eq!(choose(&w, kind, "off").await.0, StatusCode::BAD_REQUEST, "{kind} is forced too");
    }
    let (s, _) = choose(&w, "no_such_kind", "email").await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    let (s, _) = choose(&w, "mention", "loudly").await;
    assert_eq!(s, StatusCode::BAD_REQUEST);

    // The list shows each kind in words, the person's channel, which are forced and why, and whether mail is on.
    let (s, b) = send(&w.app, "GET", "/api/v1/me/notification-prefs", OWNER, None).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_eq!(b["mail_enabled"], true);
    let row = |k: &str| b["kinds"].as_array().unwrap().iter().find(|r| r["kind"] == k).cloned().unwrap();
    assert_eq!(row("mention")["channel"], "off");
    assert!(row("mention")["label"].as_str().is_some_and(|l| !l.contains('_')), "plain words: {}", row("mention"));
    assert_eq!(row("decision_waiting")["forced"], true);
    assert!(row("decision_waiting")["forced_reason"].is_string());
    assert_eq!(row("mention")["forced"], false);
}

// One kind on each other channel: in the inbox only, mailed only, both, or held for the digest.
#[tokio::test]
async fn each_channel_does_what_it_says() {
    let w = world(None).await;
    choose(&w, "mention", "in_app").await;
    notify(&w, mention(&w)).await;
    assert_eq!(inbox(&w.db), vec![("mention".into(), "none".into(), false)], "in the inbox, not mailed");
    choose(&w, "mention", "email").await;
    notify(&w, mention(&w)).await;
    assert_eq!(inbox(&w.db)[1], ("mention".into(), "queued".into(), true), "mailed, and not left unread in the inbox");
    choose(&w, "mention", "both").await;
    notify(&w, mention(&w)).await;
    assert_eq!(inbox(&w.db)[2], ("mention".into(), "queued".into(), false));
    choose(&w, "mention", "digest").await;
    notify(&w, mention(&w)).await;
    assert_eq!(inbox(&w.db)[3], ("mention".into(), "digest".into(), false));
    assert_eq!(outbox(&w.db).len(), 2, "two mails: email and both");
}

// 3. The institution's default applies to a person with no choice of their own. A forced kind's default cannot be off.
#[tokio::test]
async fn the_institution_default_applies_to_a_person_with_no_row() {
    let w = world(Some(json!({ "mention": "in_app" }))).await;
    notify(&w, mention(&w)).await;
    assert_eq!(inbox(&w.db), vec![("mention".into(), "none".into(), false)], "the default: in the inbox, no mail");
    assert!(outbox(&w.db).is_empty());
    // With no default, a kind is both: inbox and mail.
    notify(&w, waiting(&w)).await;
    assert_eq!(inbox(&w.db)[1].1, "queued");

    let put = |v: Value| send(&w.app, "PUT", "/api/v1/settings/notifications.defaults", ADMIN, Some(v));
    assert_eq!(put(json!({ "decision_waiting": "off" })).await.0, StatusCode::BAD_REQUEST, "a forced kind cannot default to off");
    assert_eq!(put(json!({ "mention": "sometimes" })).await.0, StatusCode::BAD_REQUEST);
    assert_eq!(put(json!({ "nonsense": "off" })).await.0, StatusCode::BAD_REQUEST);
    assert_eq!(put(json!({ "mention": "digest", "view_shared": "off" })).await.0, StatusCode::OK);
}

// 2. `digest` produces one mail, at 07:00 in the institution's zone, with every pending line, grouped by app, and no
// record values. Notices held back by the hourly limit are in it too.
#[tokio::test]
async fn the_digest_is_one_mail_at_seven_in_the_institutions_zone_with_every_pending_line() {
    let w = world(Some(json!({ "mention": "digest", "decision_waiting": "digest" }))).await;
    // The schedule is 07:00 in America/Chicago.
    let repo = w.state.repository.clone();
    tokio::task::spawn_blocking(move || mail::register_digest_schedule(&repo)).await.unwrap().unwrap();
    let local: String = w.db.run(|c| c.query_one("SELECT to_char(next_run_at AT TIME ZONE 'America/Chicago', 'HH24:MI') FROM job_schedules WHERE kind = 'send_digests'", &[]).unwrap().get(0));
    assert_eq!(local, "07:00");

    notify(&w, mention(&w)).await;
    notify(&w, waiting(&w)).await;
    // One held back by the hourly limit, not by a choice.
    w.db.run(|c| {
        c.execute("INSERT INTO notifications (id, recipient, kind, app_slug, title, body, email_state) VALUES (gen_random_uuid(), $1, 'job_finished', NULL, 'Background work finished', 'Recalculating a field finished.', 'digest')", &[&OWNER]).unwrap();
        // A read one is not pending.
        c.execute("INSERT INTO notifications (id, recipient, kind, title, body, email_state, read_at) VALUES (gen_random_uuid(), $1, 'mention', 'Already read', 'Read', 'digest', NOW())", &[&OWNER]).unwrap();
    });
    assert!(outbox(&w.db).is_empty(), "nothing is mailed before the digest");

    let repo = w.state.repository.clone();
    let made = tokio::task::spawn_blocking(move || mail::send_digests(&repo)).await.unwrap().expect("the digest ran");
    assert_eq!(made["digests"], 1, "{made}");
    let mails = outbox(&w.db);
    assert_eq!(mails.len(), 1, "one mail");
    let (to, subject, body) = &mails[0];
    assert_eq!(to, OWNER);
    assert!(subject.contains('3'), "the subject counts the notices: {subject}");
    assert!(body.contains("Lab Bookings"), "grouped under the app's name: {body}");
    assert_eq!(body.matches("https://desk.state.edu/n/").count(), 3, "a line with a link for each pending notice: {body}");
    assert!(!body.contains("Already read"), "{body}");
    assert!(!body.contains("r1") && !body.contains("nothing"), "no record or step id: {body}");
    let lab = body.find("Lab Bookings").unwrap();
    let other = body.find("Background work finished").unwrap();
    assert!(lab < other || body[lab..].contains("Background work finished"), "grouped: {body}");
    // Sent once: a second run finds nothing pending.
    let repo = w.state.repository.clone();
    let again = tokio::task::spawn_blocking(move || mail::send_digests(&repo)).await.unwrap().unwrap();
    assert_eq!(again["digests"], 0);
    assert!(inbox(&w.db).iter().filter(|(_, s, read)| s == "digest" && !read).count() == 0, "the pending notices are marked as digested");
}
