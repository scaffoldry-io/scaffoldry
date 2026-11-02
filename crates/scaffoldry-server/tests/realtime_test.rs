//! Real-time updates, phase 1: events.
//!
//! Each record write sends `pg_notify('scaffoldry_changes', ...)` in its own transaction, so a rolled-back write sends
//! nothing. One listener per server republishes each change to the open streams of that app. A stream is
//! `GET /api/v1/apps/{slug}/events`, server-sent events: `change`, `resync` and `ping`, and `closed` when the reader loses
//! access. A reader that falls 256 events behind is told to `resync`, and a writer never waits for a reader.

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::Router;
use http_body_util::BodyExt;
use scaffoldry_server::build_app_with_state;
use scaffoldry_server::service::identity::issue_test_token_and_user;
use scaffoldry_server::state::{CollaboratorRecord, ServerState};
use serde_json::{json, Value};
use std::time::{Duration, Instant};
use tower::ServiceExt;

const WORKSPACE: &str = "ws-physics-optics";
const OWNER: &str = "einstein@physics.state.edu";
const STRANGER: &str = "nobody.realtime@state.edu";

struct World {
    db: common::TestDb,
    state: std::sync::Arc<ServerState>,
    app: Router,
    owner: String,
    slug: String,
}

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

async fn world() -> World {
    std::env::set_var("SCAFFOLDRY_REALTIME_REAUTH_SECS", "1");
    let db = common::TestDb::new();
    let state = db.state();
    state.seed_demo().expect("seed");
    let tag = uuid::Uuid::new_v4().to_string()[..8].to_string();
    let app = build_app_with_state(state.clone()).expect("router");
    let w = World { db, state, app, owner: issue_test_token_and_user(OWNER), slug: format!("live-{tag}") };
    let manifest = json!({
        "slug": w.slug, "title": "Live", "description": "Records that change", "organization_code": "PHYS", "department": "physics",
        "workspace_id": WORKSPACE, "views": [],
        "tables": [{ "id": "cases", "name": "Cases", "slug": "cases", "fields": [
            { "name": "title", "label": "Title", "field_type": "Text", "required": false, "ferpa_sensitive": false }
        ]}],
    });
    let (s, b) = rest(&w.app, "POST", &format!("/api/v1/workspaces/{WORKSPACE}/apps"), &w.owner, Some(manifest)).await;
    assert_eq!(s, StatusCode::CREATED, "{b}");
    w
}

/// An open stream: the body of `GET /events`, read one server-sent event at a time.
struct Stream {
    body: Body,
    buf: String,
}

impl Stream {
    /// The next event, as `(name, data)`, or `None` after `wait`.
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

    /// The next event with this name, skipping others.
    async fn next_named(&mut self, name: &str, wait: Duration) -> Option<Value> {
        let deadline = Instant::now() + wait;
        while let Some(left) = deadline.checked_duration_since(Instant::now()) {
            match self.next(left).await {
                Some((n, d)) if n == name => return Some(d),
                Some(_) => continue,
                None => return None,
            }
        }
        None
    }
}

async fn open(app: &Router, slug: &str, token: &str) -> Result<Stream, (StatusCode, Value)> {
    let req = Request::builder().uri(format!("/api/v1/apps/{slug}/events")).header("authorization", format!("Bearer {token}")).body(Body::empty()).unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    if status != StatusCode::OK {
        let bytes = resp.into_body().collect().await.unwrap().to_bytes();
        return Err((status, serde_json::from_slice(&bytes).unwrap_or(Value::Null)));
    }
    assert_eq!(resp.headers().get("content-type").and_then(|v| v.to_str().ok()), Some("text/event-stream"));
    let mut s = Stream { body: resp.into_body(), buf: String::new() };
    // The stream pings once it is listening, so a write after this is seen.
    assert!(s.next_named("ping", Duration::from_secs(10)).await.is_some(), "the stream opens with a ping");
    Ok(s)
}

async fn create(w: &World, title: &str) -> Value {
    let (s, r) = rest(&w.app, "POST", &format!("/api/v1/apps/{}/records", w.slug), &w.owner, Some(json!({ "data": { "_table_id": "cases", "title": title } }))).await;
    assert_eq!(s, StatusCode::CREATED, "{r}");
    r
}

// 1. Two clients are connected. A write by one produces one `change` for the other, with the right ids and version.
// 3. An event's JSON has only the keys it names.
#[tokio::test]
async fn a_write_reaches_every_open_stream_with_its_ids_and_version_and_nothing_else() {
    let w = world().await;
    let mut a = open(&w.app, &w.slug, &w.owner).await.expect("a stream");
    let mut b = open(&w.app, &w.slug, &issue_test_token_and_user(OWNER)).await.expect("another stream");
    let made = create(&w, "First").await;
    for stream in [&mut a, &mut b] {
        let change = stream.next_named("change", Duration::from_secs(10)).await.expect("a change");
        assert_eq!(change["record"], made["id"], "{change}");
        assert_eq!(change["table"], "cases");
        assert_eq!(change["version"], made["version"]);
        assert_eq!(change["kind"], "create");
        let mut keys: Vec<&str> = change.as_object().unwrap().keys().map(String::as_str).collect();
        keys.sort();
        assert_eq!(keys, vec!["kind", "record", "table", "version"], "a change carries its four keys and no field value");
        assert!(stream.next_named("change", Duration::from_millis(500)).await.is_none(), "exactly one change for one write");
    }
    // An update is the next version.
    let id = made["id"].as_str().unwrap();
    let (s, _) = rest(&w.app, "PATCH", &format!("/api/v1/apps/{}/records/{id}", w.slug), &w.owner, Some(json!({ "version": made["version"], "data": { "_table_id": "cases", "title": "Second" } }))).await;
    assert_eq!(s, StatusCode::OK);
    let change = a.next_named("change", Duration::from_secs(10)).await.expect("the update");
    assert_eq!(change["kind"], "update");
    assert_eq!(change["version"], made["version"].as_i64().unwrap() + 1);
}

// 2. A client with no read access to the app is refused with 403. A client that loses access receives `closed`.
#[tokio::test]
async fn a_reader_without_access_is_refused_and_one_who_loses_it_is_closed() {
    let w = world().await;
    let stranger = issue_test_token_and_user(STRANGER);
    let refused = open(&w.app, &w.slug, &stranger).await.err().expect("refused");
    assert_eq!(refused.0, StatusCode::FORBIDDEN, "{:?}", refused.1);

    let member = format!("realtime.member.{}@physics.state.edu", &w.slug[5..]);
    let record = CollaboratorRecord {
        id: uuid::Uuid::new_v4().to_string(),
        workspace_id: WORKSPACE.to_string(),
        eppn: member.clone(),
        name: member.clone(),
        role: "viewer".to_string(),
        scoped_affiliation: "faculty".to_string(),
        department: "physics".to_string(),
        added_at: String::new(),
        app_slug: String::new(),
    };
    w.state.repository.upsert_collaborator(&record).unwrap();
    w.state.collaborators.write().unwrap().entry(WORKSPACE.to_string()).or_default().push(record);
    let token = issue_test_token_and_user(&member);
    let mut s = open(&w.app, &w.slug, &token).await.expect("a member reads");
    let m = member.clone();
    w.db.run(move |c| {
        c.execute("DELETE FROM workspace_collaborators WHERE workspace_id = $1 AND eppn = $2", &[&WORKSPACE, &m]).unwrap();
        c.execute("UPDATE cache_versions SET version = version + 1 WHERE name = 'workspaces'", &[]).unwrap();
    });
    assert!(s.next_named("closed", Duration::from_secs(70)).await.is_some(), "a reader who loses access is closed");
}

// 4. A rolled-back write sends nothing.
#[tokio::test]
async fn a_rolled_back_write_sends_nothing() {
    let w = world().await;
    let mut s = open(&w.app, &w.slug, &w.owner).await.expect("a stream");
    let slug = w.slug.clone();
    w.db.run(move |c| {
        let mut tx = c.transaction().unwrap();
        tx.execute(
            "SELECT pg_notify('scaffoldry_changes', json_build_object('app', $1::text, 'table', 'cases', 'record', 'ghost', 'version', 1, 'kind', 'create')::text)",
            &[&slug],
        )
        .unwrap();
        tx.rollback().unwrap();
    });
    // A real write after it arrives, and is the only one.
    let made = create(&w, "Real").await;
    let change = s.next_named("change", Duration::from_secs(10)).await.expect("the real change");
    assert_eq!(change["record"], made["id"], "the rolled-back notice never came: {change}");
}

// 5. A client reading slowly is sent `resync` after 256 events, and the writer's latency is unchanged.
#[tokio::test]
async fn a_slow_reader_is_told_to_resync_and_does_not_slow_the_writer() {
    let w = world().await;
    let mut baseline = Vec::new();
    for i in 0..20 {
        let t = Instant::now();
        create(&w, &format!("warm {i}")).await;
        baseline.push(t.elapsed());
    }
    let mut slow = open(&w.app, &w.slug, &w.owner).await.expect("a stream that will not read");
    let mut timed = Vec::new();
    for i in 0..300 {
        let t = Instant::now();
        create(&w, &format!("burst {i}")).await;
        timed.push(t.elapsed());
    }
    baseline.sort();
    timed.sort();
    let (base, with) = (baseline[baseline.len() / 2], timed[timed.len() / 2]);
    // The tolerance: the median write with a stalled reader is within 2x the median without one, plus 20 ms of noise.
    assert!(with <= base * 2 + Duration::from_millis(20), "a stalled reader slowed the writer: {base:?} without, {with:?} with");
    assert!(slow.next_named("resync", Duration::from_secs(20)).await.is_some(), "the reader that fell behind is told to resync");
}

// 6. Killing the listener's connection makes every stream receive `resync` after it reconnects.
#[tokio::test]
async fn a_lost_listener_connection_tells_every_stream_to_resync() {
    let w = world().await;
    let mut a = open(&w.app, &w.slug, &w.owner).await.expect("a stream");
    let mut b = open(&w.app, &w.slug, &issue_test_token_and_user(OWNER)).await.expect("another stream");
    let killed: i64 = w.db.run(|c| {
        c.query_one("SELECT count(pg_terminate_backend(pid)) FROM pg_stat_activity WHERE application_name = 'scaffoldry-realtime' AND datname = current_database()", &[])
            .unwrap()
            .get(0)
    });
    assert_eq!(killed, 1, "one listener connection for this server");
    for s in [&mut a, &mut b] {
        assert!(s.next_named("resync", Duration::from_secs(30)).await.is_some(), "each stream is told to resync");
    }
    // And the stream works again.
    let made = create(&w, "After").await;
    assert_eq!(a.next_named("change", Duration::from_secs(10)).await.expect("a change")["record"], made["id"]);
}

// 7. The twenty-first stream for one user is `429`.
#[tokio::test]
async fn the_twenty_first_stream_for_one_person_is_refused() {
    let w = world().await;
    let mut held = Vec::new();
    for _ in 0..20 {
        held.push(open(&w.app, &w.slug, &w.owner).await.expect("within the limit"));
    }
    let refused = open(&w.app, &w.slug, &w.owner).await.err().expect("the 21st is refused");
    assert_eq!(refused.0, StatusCode::TOO_MANY_REQUESTS, "{:?}", refused.1);
    assert_eq!(refused.1["code"], "rate_limited");
    drop(held.pop());
    // A closed stream frees its place.
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        match open(&w.app, &w.slug, &w.owner).await {
            Ok(_) => break,
            Err(e) if Instant::now() < deadline && e.0 == StatusCode::TOO_MANY_REQUESTS => tokio::time::sleep(Duration::from_millis(100)).await,
            Err(e) => panic!("a freed place is not reused: {e:?}"),
        }
    }
}
