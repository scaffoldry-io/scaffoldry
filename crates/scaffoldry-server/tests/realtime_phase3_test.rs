//! Real-time updates, phase 3: presence, and a channel for the person.
//!
//! Presence is who has an app's stream open. Each server keeps it in memory and hears every server's joins, pings and
//! leaves through `pg_notify('scaffoldry_presence', ...)`, so two people on two servers see each other. The desk pings
//! `POST /apps/{slug}/presence` while its stream is open. Someone silent past the timeout (45 seconds, 2 here) is
//! dropped. A stream is told `presence` events: the list on connect, then `joined` and `left`, each with a count.
//!
//! `GET /api/v1/me/events` carries the signed-in person's own events: `notification` (an id and a count), `job` (id,
//! state, percent) and `proposal` (id, status). Triggers on those tables send them, so every writer is covered. No
//! event carries a title, a name or a value.

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::Router;
use http_body_util::BodyExt;
use scaffoldry_server::build_app_with_state;
use scaffoldry_server::notifications::{self, Notice, PersonRef, Recipients};
use scaffoldry_server::service::identity::issue_test_token_and_user;
use scaffoldry_server::state::{CollaboratorRecord, ServerState};
use serde_json::{json, Value};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tower::ServiceExt;

const WORKSPACE: &str = "ws-physics-optics";
const OWNER: &str = "einstein@physics.state.edu";
const ADMIN: &str = "jordan.lee@state.edu";
const STRANGER: &str = "nobody.presence@state.edu";

async fn rest(app: &Router, method: &str, uri: &str, token: &str, body: Option<Value>) -> (StatusCode, Value) {
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

struct Stream {
    body: Body,
    buf: String,
}

impl Stream {
    async fn next(&mut self, wait: Duration) -> Option<(String, Value)> {
        let deadline = Instant::now() + wait;
        loop {
            if let Some(end) = self.buf.find("\n\n") {
                let block: String = self.buf.drain(..end + 2).collect();
                let mut name = "message".to_string();
                let mut data = String::new();
                for line in block.lines() {
                    if let Some(v) = line.strip_prefix("event:") {
                        name = v.trim().to_string();
                    } else if let Some(v) = line.strip_prefix("data:") {
                        data.push_str(v.trim());
                    }
                }
                return Some((name, serde_json::from_str(&data).unwrap_or(Value::Null)));
            }
            let left = deadline.checked_duration_since(Instant::now())?;
            match tokio::time::timeout(left, self.body.frame()).await {
                Ok(Some(Ok(frame))) => {
                    if let Some(bytes) = frame.data_ref() {
                        self.buf.push_str(&String::from_utf8_lossy(bytes));
                    }
                }
                _ => return None,
            }
        }
    }

    async fn next_named(&mut self, name: &str, wait: Duration) -> Option<Value> {
        self.next_matching(name, wait, |_| true).await
    }

    /// The next event with this name whose data passes `test`, skipping others.
    async fn next_matching(&mut self, name: &str, wait: Duration, test: impl Fn(&Value) -> bool) -> Option<Value> {
        let deadline = Instant::now() + wait;
        while let Some(left) = deadline.checked_duration_since(Instant::now()) {
            match self.next(left).await {
                Some((n, d)) if n == name && test(&d) => return Some(d),
                Some(_) => continue,
                None => return None,
            }
        }
        None
    }

    /// Every event that arrives within `wait`.
    async fn drain(&mut self, wait: Duration) -> Vec<(String, Value)> {
        let deadline = Instant::now() + wait;
        let mut got = Vec::new();
        while let Some(left) = deadline.checked_duration_since(Instant::now()) {
            match self.next(left).await {
                Some(e) => got.push(e),
                None => break,
            }
        }
        got
    }
}

async fn open_at(app: &Router, path: &str, token: &str) -> Result<Stream, (StatusCode, Value)> {
    let req = Request::builder().uri(path).header("authorization", format!("Bearer {token}")).body(Body::empty()).unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    if status != StatusCode::OK {
        let bytes = resp.into_body().collect().await.unwrap().to_bytes();
        return Err((status, serde_json::from_slice(&bytes).unwrap_or(Value::Null)));
    }
    let mut s = Stream { body: resp.into_body(), buf: String::new() };
    assert!(s.next_named("ping", Duration::from_secs(10)).await.is_some(), "the stream opens with a ping");
    Ok(s)
}

async fn open(app: &Router, slug: &str, token: &str) -> Result<Stream, (StatusCode, Value)> {
    open_at(app, &format!("/api/v1/apps/{slug}/events"), token).await
}

async fn me(app: &Router, token: &str) -> Stream {
    open_at(app, "/api/v1/me/events", token).await.expect("the person's own stream")
}

fn keys(v: &Value) -> Vec<String> {
    let mut k: Vec<String> = v.as_object().map(|o| o.keys().cloned().collect()).unwrap_or_default();
    k.sort();
    k
}

struct World {
    db: common::TestDb,
    one: Arc<ServerState>,
    two: Arc<ServerState>,
    app_one: Router,
    app_two: Router,
    slug: String,
    member: String,
}

/// Two servers on one database, an app, and a second person who may read it.
async fn world() -> World {
    std::env::set_var("SCAFFOLDRY_PRESENCE_TIMEOUT_SECS", "2");
    let db = common::TestDb::new();
    let one = db.state();
    one.seed_demo().expect("seed");
    let app_one = build_app_with_state(one.clone()).expect("router");
    let tag = uuid::Uuid::new_v4().simple().to_string()[..8].to_string();
    let slug = format!("here-{tag}");
    let manifest = json!({
        "slug": slug, "title": "Here", "description": "", "organization_code": "PHYS", "department": "physics", "workspace_id": WORKSPACE, "views": [],
        "tables": [{ "id": "cases", "name": "Cases", "slug": "cases", "fields": [{ "name": "title", "label": "Title", "field_type": "Text", "required": false, "ferpa_sensitive": false }] }],
    });
    let (s, b) = rest(&app_one, "POST", &format!("/api/v1/workspaces/{WORKSPACE}/apps"), &issue_test_token_and_user(OWNER), Some(manifest)).await;
    assert_eq!(s, StatusCode::CREATED, "{b}");
    let member = format!("presence.member.{tag}@physics.state.edu");
    one.repository
        .upsert_collaborator(&CollaboratorRecord {
            id: uuid::Uuid::new_v4().to_string(),
            workspace_id: WORKSPACE.to_string(),
            eppn: member.clone(),
            name: "Marie Curie".to_string(),
            role: "viewer".to_string(),
            scoped_affiliation: "faculty".to_string(),
            department: "physics".to_string(),
            added_at: String::new(),
            app_slug: String::new(),
        })
        .unwrap();
    db.run(|c| {
        c.execute("UPDATE cache_versions SET version = version + 1 WHERE name = 'workspaces'", &[]).unwrap();
    });
    one.refresh().unwrap();
    let two = db.state();
    let app_two = build_app_with_state(two.clone()).expect("router");
    World { db, one, two, app_one, app_two, slug, member }
}

/// Pings presence for `stream` every 500 ms until dropped, as the desk does every 15 seconds.
fn keep_pinging(app: Router, slug: String, token: String, stream: String) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        loop {
            let (s, b) = rest(&app, "POST", &format!("/api/v1/apps/{slug}/presence"), &token, Some(json!({ "stream": stream }))).await;
            assert_eq!(s, StatusCode::NO_CONTENT, "{b}");
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
    })
}

// 1. Two people in an app, on two servers, see each other. One closes the stream and the other sees `left` within a
// second. A silent client is dropped after the timeout.
#[tokio::test]
async fn two_people_on_two_servers_see_each_other_and_a_silent_one_is_dropped() {
    let w = world().await;
    let owner = issue_test_token_and_user(OWNER);
    let member = issue_test_token_and_user(&w.member);

    let mut a = open(&w.app_one, &w.slug, &owner).await.expect("the owner's stream on server one");
    let first = a.next_named("presence", Duration::from_secs(5)).await.expect("the list on connect");
    let stream_a = first["stream"].as_str().expect("the stream's own id, for pings").to_string();
    assert!(first["present"].is_array() && first["count"].is_number(), "{first}");
    let pinger = keep_pinging(w.app_one.clone(), w.slug.clone(), owner.clone(), stream_a);

    let mut b = open(&w.app_two, &w.slug, &member).await.expect("the member's stream on server two");
    let is = |who: &str, what: &str| {
        let (who, what) = (who.to_string(), what.to_string());
        move |d: &Value| d[&what]["address"] == who.as_str()
    };
    let joined = a.next_matching("presence", Duration::from_secs(5), is(&w.member, "joined")).await.expect("the owner sees the member join");
    assert!(joined["joined"]["name"].as_str().is_some_and(|n| !n.is_empty()), "a name in words: {joined}");
    assert_eq!(joined["count"], 2, "{joined}");
    // Server two had not heard the owner join. The owner's next ping tells it.
    let seen = b.next_matching("presence", Duration::from_secs(5), |d| {
        d["joined"]["address"] == OWNER || d["present"].as_array().is_some_and(|l| l.iter().any(|p| p["address"] == OWNER))
    });
    assert!(seen.await.is_some(), "the member sees the owner");

    // Closing a stream is `left` within a second.
    drop(b);
    let t = Instant::now();
    let left = a.next_matching("presence", Duration::from_secs(5), is(&w.member, "left")).await.expect("the owner sees the member leave");
    assert!(t.elapsed() <= Duration::from_secs(1), "left within a second, not {:?}", t.elapsed());
    assert_eq!(left["count"], 1, "{left}");

    // A stream that never pings is dropped after the timeout, though it stays open.
    let _silent = open(&w.app_two, &w.slug, &member).await.expect("a silent stream");
    a.next_matching("presence", Duration::from_secs(5), is(&w.member, "joined")).await.expect("joined");
    let t = Instant::now();
    a.next_matching("presence", Duration::from_secs(8), is(&w.member, "left")).await.expect("the silent one is dropped");
    assert!(t.elapsed() >= Duration::from_millis(1500), "not before the timeout: {:?}", t.elapsed());
    // The owner, who pings, is still there.
    assert!(a.next_matching("presence", Duration::from_secs(3), is(OWNER, "left")).await.is_none(), "a person who pings stays");
    pinger.abort();
    let _ = (&w.one, &w.two, &w.db);
}

// 2. A person without read access never receives the presence list: not on the app's stream, which refuses them, not
// on their own stream, and not by pinging.
#[tokio::test]
async fn a_person_without_read_access_never_receives_the_presence_list() {
    let w = world().await;
    let stranger = issue_test_token_and_user(STRANGER);
    let refused = open(&w.app_one, &w.slug, &stranger).await.err().expect("refused");
    assert_eq!(refused.0, StatusCode::FORBIDDEN, "{:?}", refused.1);
    let (s, _) = rest(&w.app_one, "POST", &format!("/api/v1/apps/{}/presence", w.slug), &stranger, Some(json!({ "stream": uuid::Uuid::new_v4() }))).await;
    assert_eq!(s, StatusCode::FORBIDDEN, "nobody may appear in an app they cannot read");
    let mut own = me(&w.app_one, &stranger).await;
    let _a = open(&w.app_one, &w.slug, &issue_test_token_and_user(OWNER)).await.expect("the owner's stream");
    let _b = open(&w.app_two, &w.slug, &issue_test_token_and_user(&w.member)).await.expect("the member's stream");
    let got = own.drain(Duration::from_secs(2)).await;
    assert!(got.iter().all(|(n, _)| n == "ping"), "the stranger's own stream carries nothing about the app: {got:?}");
    let text = format!("{got:?}");
    assert!(!text.contains(OWNER) && !text.contains(&w.member), "{text}");
}

// 3. A job's progress produces `job` events for its owner only, with an id, a state and a percent. A proposal's
// decision produces a `proposal` event for its proposer.
#[tokio::test]
async fn job_and_proposal_events_go_to_their_owner_only() {
    let w = world().await;
    let mut mine = me(&w.app_one, &issue_test_token_and_user(OWNER)).await;
    let mut theirs = me(&w.app_two, &issue_test_token_and_user(&w.member)).await;
    let job = uuid::Uuid::new_v4();
    w.db.run(move |c| {
        c.execute("INSERT INTO jobs (id, kind, created_by, progress_total) VALUES ($1, 'recalc_field', $2, 10)", &[&job, &OWNER]).unwrap();
        c.execute("UPDATE jobs SET state = 'running' WHERE id = $1", &[&job]).unwrap();
        c.execute("UPDATE jobs SET progress_done = 5 WHERE id = $1", &[&job]).unwrap();
        // A heartbeat that changes neither state nor percent sends nothing.
        c.execute("UPDATE jobs SET locked_until = NOW() WHERE id = $1", &[&job]).unwrap();
        c.execute("UPDATE jobs SET state = 'done', progress_done = 10 WHERE id = $1", &[&job]).unwrap();
    });
    let mut seen = Vec::new();
    for _ in 0..4 {
        let e = mine.next_named("job", Duration::from_secs(5)).await.expect("a job event for its owner");
        assert_eq!(keys(&e), vec!["id", "percent", "state"], "ids and numbers only: {e}");
        assert_eq!(e["id"], job.to_string());
        seen.push((e["state"].as_str().unwrap_or("").to_string(), e["percent"].as_i64()));
    }
    assert_eq!(
        seen,
        vec![("queued".into(), Some(0)), ("running".into(), Some(0)), ("running".into(), Some(50)), ("done".into(), Some(100))],
        "one event per change of state or percent"
    );
    assert!(mine.next_named("job", Duration::from_millis(500)).await.is_none(), "the heartbeat sent nothing");

    let proposal = uuid::Uuid::new_v4();
    let slug = w.slug.clone();
    w.db.run(move |c| {
        c.execute(
            "INSERT INTO app_proposals (id, workspace_id, app_slug, manifest, checks, summary, proposed_by) VALUES ($1, $2, $3, '{}', '{}', 'A change', $4)",
            &[&proposal, &WORKSPACE, &slug, &OWNER],
        )
        .unwrap();
        c.execute("UPDATE app_proposals SET status = 'Approved', decided_by = $2 WHERE id = $1", &[&proposal, &ADMIN]).unwrap();
    });
    let e = mine.next_matching("proposal", Duration::from_secs(5), |d| d["status"] == "Approved").await.expect("the decision");
    assert_eq!(keys(&e), vec!["id", "status"], "{e}");
    assert_eq!(e["id"], proposal.to_string());

    let other = theirs.drain(Duration::from_secs(1)).await;
    assert!(other.iter().all(|(n, _)| n == "ping"), "someone else's work is not on this person's stream: {other:?}");
}

// 4. A new notification produces one `notification` event, with its id and the unread count, and no text. Marking it
// read sends the new count.
#[tokio::test]
async fn a_new_notification_is_one_event_with_an_id_and_a_count_and_no_text() {
    let w = world().await;
    let token = issue_test_token_and_user(OWNER);
    let mut mine = me(&w.app_two, &token).await;
    let st = w.one.clone();
    let made = tokio::task::spawn_blocking(move || {
        notifications::notify(&st, &Recipients::Person(OWNER.into()), Some(ADMIN), &Notice::DelegationReceived { by: PersonRef(ADMIN.into()) })
    })
    .await
    .unwrap()
    .unwrap();
    assert_eq!(made, 1);
    let id: uuid::Uuid = w.db.run(|c| c.query_one("SELECT id FROM notifications WHERE lower(recipient) = lower($1)", &[&OWNER]).unwrap().get(0));
    let e = mine.next_named("notification", Duration::from_secs(5)).await.expect("a notification event");
    assert_eq!(keys(&e), vec!["count", "id"], "an id and a count, no title or body: {e}");
    assert_eq!(e["id"], id.to_string());
    assert_eq!(e["count"], 1);
    assert!(mine.next_named("notification", Duration::from_millis(500)).await.is_none(), "one event for one notice");

    let (s, _) = rest(&w.app_one, "POST", "/api/v1/me/notifications/read", &token, Some(json!({ "all": true }))).await;
    assert_eq!(s, StatusCode::OK);
    let e = mine.next_named("notification", Duration::from_secs(5)).await.expect("the new count");
    assert_eq!(e["count"], 0, "{e}");
    assert!(e["id"].is_null(), "{e}");
}

// The person's stream needs a session.
#[tokio::test]
async fn the_persons_stream_needs_a_session() {
    let w = world().await;
    let resp = w.app_one.clone().oneshot(Request::builder().uri("/api/v1/me/events").body(Body::empty()).unwrap()).await.unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}
