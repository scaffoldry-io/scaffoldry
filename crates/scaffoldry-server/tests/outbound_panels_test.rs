//! Integrations phase 5: the console panels' routes. Outbound access, webhooks across apps, and the Overview counts.
//!
//! Every route is a Platform Admin's. A faculty caller is 403 on all of them.

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::Router;
use http_body_util::BodyExt;
use scaffoldry_server::build_app_with_state;
use scaffoldry_server::net::egress::{Allow, Client, Resolver};
use scaffoldry_server::service::identity::issue_test_token_and_user;
use scaffoldry_server::state::ServerState;
use scaffoldry_server::webhooks;
use serde_json::{json, Value};
use std::io::{Read, Write};
use std::net::{IpAddr, TcpListener};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use tower::ServiceExt;

const ADMIN: &str = "jordan.lee@state.edu";
const OWNER: &str = "einstein@physics.state.edu";
const CO_OWNER: &str = "bohr@physics.state.edu";
const WORKSPACE: &str = "ws-physics-optics";
const SECRET_NAME: &str = "housing-hook";

/// The signing secret of the test webhook, made fresh for each run so no key is written in the source.
fn secret_value() -> &'static [u8] {
    static SECRET: std::sync::OnceLock<Vec<u8>> = std::sync::OnceLock::new();
    SECRET.get_or_init(|| format!("whsec-{}", uuid::Uuid::new_v4().simple()).into_bytes())
}

/// Every name resolves to `ip`.
struct To(IpAddr);
impl Resolver for To {
    fn lookup(&self, _host: &str, _port: u16) -> std::io::Result<Vec<IpAddr>> {
        Ok(vec![self.0])
    }
}

/// One request the receiver got. These tests only count them.
#[derive(Clone, Debug)]
#[allow(dead_code)]
struct Got {
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

#[allow(dead_code)]
impl Got {
    fn header(&self, name: &str) -> Option<String> {
        self.headers.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.clone())
    }
    fn json(&self) -> Value {
        serde_json::from_slice(&self.body).unwrap()
    }
}

/// A receiver that answers with `replies` in turn (the last one repeats) and keeps what it got.
fn receiver(replies: Vec<&'static str>) -> (u16, Arc<Mutex<Vec<Got>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let got = Arc::new(Mutex::new(Vec::new()));
    let kept = got.clone();
    std::thread::spawn(move || {
        for (i, stream) in listener.incoming().enumerate() {
            let Ok(mut s) = stream else { continue };
            let mut buf = Vec::new();
            let mut chunk = [0u8; 8192];
            loop {
                let n = s.read(&mut chunk).unwrap_or(0);
                if n == 0 {
                    break;
                }
                buf.extend_from_slice(&chunk[..n]);
                if let Some(end) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                    let head = String::from_utf8_lossy(&buf[..end]).to_string();
                    let len = head.lines().find_map(|l| l.to_ascii_lowercase().strip_prefix("content-length:").map(|v| v.trim().parse::<usize>().unwrap_or(0))).unwrap_or(0);
                    while buf.len() < end + 4 + len {
                        let n = s.read(&mut chunk).unwrap_or(0);
                        if n == 0 {
                            break;
                        }
                        buf.extend_from_slice(&chunk[..n]);
                    }
                    let headers = head.lines().skip(1).filter_map(|l| l.split_once(':').map(|(k, v)| (k.trim().to_string(), v.trim().to_string()))).collect();
                    kept.lock().unwrap().push(Got { headers, body: buf[end + 4..].to_vec() });
                    break;
                }
            }
            let reply = replies.get(i).or(replies.last()).copied().unwrap_or("HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
            let _ = s.write_all(reply.as_bytes());
        }
    });
    (port, got)
}

const OK: &str = "HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok";

fn key_file() -> PathBuf {
    use base64::Engine;
    let mut bytes = Vec::new();
    bytes.extend_from_slice(uuid::Uuid::new_v4().as_bytes());
    bytes.extend_from_slice(uuid::Uuid::new_v4().as_bytes());
    let dir = std::env::temp_dir().join(format!("scaffoldry-key-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("master.key");
    std::fs::write(&path, base64::engine::general_purpose::STANDARD.encode(bytes)).unwrap();
    path
}

struct World {
    db: common::TestDb,
    state: Arc<ServerState>,
    app: Router,
    port: u16,
    got: Arc<Mutex<Vec<Got>>>,
}

async fn send(app: &Router, method: &str, uri: &str, who: &str, body: Option<Value>) -> (StatusCode, Value) {
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

/// A server with a master key, a stored signing secret, a receiver answering `replies`, and the allowlist letting the
/// receiver's host and port through.
fn world(replies: Vec<&'static str>) -> World {
    let db = common::TestDb::new();
    let mut state = ServerState::connect(db.url()).expect("server");
    let key = key_file();
    state.master_key_file = Some(key.clone());
    let state = Arc::new(state);
    state.seed_demo().expect("seed");
    let app = build_app_with_state(state.clone()).expect("router");
    let master = scaffoldry_server::secrets::MasterKey::from_file(&key).unwrap();
    scaffoldry_server::secrets::put(&state.repository, &master, SECRET_NAME, secret_value(), ADMIN).unwrap();
    let (port, got) = receiver(replies);
    {
        let mut s = state.settings.write().unwrap();
        s.insert("egress.allow".into(), json!([{ "host": "hooks.example.org", "ports": [port] }, { "cidr": "127.0.0.1/32", "ports": [port] }]));
        s.insert("webhooks.allow_http".into(), json!(true));
    }
    World { db, state, app, port, got }
}

fn client(w: &World) -> Client {
    let settings = w.state.settings.read().unwrap();
    Client::new(Allow::from_settings(&settings), Arc::new(To("127.0.0.1".parse().unwrap())))
}

fn field(name: &str, kind: &str) -> Value {
    json!({ "name": name, "label": name, "field_type": kind, "required": false, "ferpa_sensitive": false })
}

fn hook(w: &World, extra: Value) -> Value {
    let mut h = json!({
        "id": "notify-housing", "name": "Notify housing", "url": format!("http://hooks.example.org:{}/in", w.port), "secret": SECRET_NAME,
        "events": ["RecordCreated", "RecordUpdated"], "tables": ["applications"], "include_values": false, "fields": [],
    });
    if let (Some(h), Some(e)) = (h.as_object_mut(), extra.as_object()) {
        h.extend(e.clone());
    }
    h
}

fn manifest(slug: &str, webhooks: Vec<Value>) -> Value {
    let mut ssn = field("ssn", "Text");
    ssn["ferpa_sensitive"] = json!(true);
    json!({
        "slug": slug, "title": "Housing", "description": "", "organization_code": "PHYS", "department": "physics", "workspace_id": WORKSPACE, "views": [],
        "tables": [{ "id": "applications", "name": "Applications", "slug": "applications", "primary_field": "name", "fields": [field("name", "Text"), field("email", "Text"), field("status", "Text"), field("boom", "Text"), ssn] }],
        "integrations": { "webhooks": webhooks },
    })
}

fn slug() -> String {
    format!("housing-{}", &uuid::Uuid::new_v4().simple().to_string()[..8])
}

/// Creates the app through the route. A pending proposal (202) is approved by a second owner.
async fn make_app(w: &World, webhooks: Vec<Value>) -> String {
    let slug = slug();
    let (s, b) = send(&w.app, "POST", &format!("/api/v1/workspaces/{WORKSPACE}/apps"), OWNER, Some(manifest(&slug, webhooks))).await;
    if s == StatusCode::ACCEPTED {
        let id = b["proposal_id"].as_str().unwrap().to_string();
        issue_test_token_and_user(CO_OWNER);
        let (s2, b2) = send(&w.app, "POST", &format!("/api/v1/workspaces/{WORKSPACE}/collaborators"), ADMIN, Some(json!({ "eppn": CO_OWNER, "role": "owner" }))).await;
        assert!(s2.is_success(), "{b2}");
        let (s3, b3) = send(&w.app, "GET", &format!("/api/v1/proposals/{id}"), CO_OWNER, None).await;
        assert!(s3.is_success(), "{b3}");
        let (s4, b4) = send(&w.app, "POST", &format!("/api/v1/proposals/{id}/decision"), CO_OWNER, Some(json!({ "decision": "approve", "rationale": "Reviewed", "digest": b3["digest"] }))).await;
        assert!(s4.is_success(), "{b4}");
    } else {
        assert_eq!(s, StatusCode::CREATED, "{b}");
    }
    slug
}

async fn create(w: &World, slug: &str, data: Value) -> (StatusCode, Value) {
    let mut d = data;
    d["_table_id"] = json!("applications");
    send(&w.app, "POST", &format!("/api/v1/apps/{slug}/records"), OWNER, Some(json!({ "data": d }))).await
}

fn deliveries(w: &World, slug: &str) -> Vec<Value> {
    let slug = slug.to_string();
    w.db.run(move |c| {
        c.query("SELECT json_build_object('state', state, 'attempts', attempts, 'payload', payload, 'last_status', last_status, 'last_error', last_error, 'next', EXTRACT(EPOCH FROM next_attempt_at)) FROM webhook_deliveries WHERE app_slug = $1 ORDER BY created_at", &[&slug])
            .unwrap()
            .iter()
            .map(|r| r.get(0))
            .collect()
    })
}


const FACULTY: &str = OWNER;

fn ledger_count(w: &World, kind: &str) -> i64 {
    let kind = kind.to_string();
    w.db.run(move |c| c.query_one("SELECT COUNT(*) FROM governance_ledger WHERE decision_type = $1", &[&kind]).unwrap().get(0))
}

async fn app_with_hook(w: &World) -> String {
    make_app(w, vec![hook(w, json!({}))]).await
}

// 1. A faculty caller is 403 on every route in this phase.
#[tokio::test(flavor = "multi_thread")]
async fn a_faculty_caller_is_forbidden_on_every_route() {
    let w = world(vec![OK]);
    let slug = app_with_hook(&w).await;
    let routes: Vec<(&str, String, Option<Value>)> = vec![
        ("GET", "/api/v1/admin/egress".into(), None),
        ("PUT", "/api/v1/admin/egress/allow".into(), Some(json!({ "allow": [], "reason": "Tidy" }))),
        ("POST", "/api/v1/admin/egress/test".into(), Some(json!({ "host": "10.0.0.5", "port": 443 }))),
        ("GET", "/api/v1/admin/webhooks".into(), None),
        ("GET", format!("/api/v1/admin/webhooks/{slug}/notify-housing/deliveries"), None),
        ("POST", format!("/api/v1/admin/webhooks/{slug}/notify-housing/disable"), Some(json!({ "reason": "Stop" }))),
        ("POST", format!("/api/v1/admin/webhooks/{slug}/notify-housing/enable"), Some(json!({ "reason": "Go" }))),
    ];
    for (method, uri, body) in routes {
        let (s, b) = send(&w.app, method, &uri, FACULTY, body).await;
        assert_eq!(s, StatusCode::FORBIDDEN, "{method} {uri}: {b}");
    }
}

// 2. `Test` on a private address says it is blocked and why. A name that is not on the list says so, without a lookup.
#[tokio::test(flavor = "multi_thread")]
async fn test_on_a_private_address_says_it_is_blocked_and_why() {
    let w = world(vec![OK]);
    let (s, b) = send(&w.app, "POST", "/api/v1/admin/egress/test", ADMIN, Some(json!({ "host": "10.0.0.5", "port": 443 }))).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_eq!(b["allowed"], false, "{b}");
    let said = b["explanation"].as_str().unwrap();
    assert!(said.contains("private"), "{said}");
    assert!(said.to_lowercase().contains("blocked") || said.to_lowercase().contains("refused"), "{said}");

    let (s, b) = send(&w.app, "POST", "/api/v1/admin/egress/test", ADMIN, Some(json!({ "host": "unlisted.example.net", "port": 443 }))).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_eq!(b["allowed"], false, "{b}");
    assert!(b["explanation"].as_str().unwrap().contains("not on the outbound allowlist"), "{b}");

    let (s, _) = send(&w.app, "POST", "/api/v1/admin/egress/test", ADMIN, Some(json!({ "host": "", "port": 443 }))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
}

// 3. The panel shows a webhook's failures and its recent deliveries.
#[tokio::test(flavor = "multi_thread")]
async fn the_panel_shows_a_webhooks_failures_and_recent_deliveries() {
    let w = world(vec![OK]);
    let slug = app_with_hook(&w).await;
    create(&w, &slug, json!({ "name": "Ada" })).await;
    let (s, b) = send(&w.app, "GET", "/api/v1/admin/webhooks", ADMIN, None).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    let row = b["webhooks"].as_array().unwrap().iter().find(|r| r["app"] == slug.as_str()).unwrap_or_else(|| panic!("a row for the app: {b}")).clone();
    assert_eq!(row["id"], "notify-housing");
    assert_eq!(row["name"], "Notify housing");
    assert_eq!(row["host"], "hooks.example.org");
    assert_eq!(row["events"], json!(["RecordCreated", "RecordUpdated"]));
    assert_eq!(row["state"], "Active");
    assert_eq!(row["consecutive_failures"], 0);
    assert!(row["last_success"].is_null());
    assert!(!row.to_string().contains(&String::from_utf8_lossy(secret_value()).to_string()), "no secret value");

    let s2 = slug.clone();
    w.db.run(move |c| {
        c.execute("INSERT INTO webhook_state (app_slug, webhook_id, consecutive_failures) VALUES ($1, 'notify-housing', 3) ON CONFLICT (app_slug, webhook_id) DO UPDATE SET consecutive_failures = 3", &[&s2]).unwrap();
        c.execute("UPDATE webhook_deliveries SET last_status = 500, last_error = 'The receiver answered 500', attempts = 2 WHERE app_slug = $1", &[&s2]).unwrap();
    });
    let (_, b) = send(&w.app, "GET", "/api/v1/admin/webhooks", ADMIN, None).await;
    let row = b["webhooks"].as_array().unwrap().iter().find(|r| r["app"] == slug.as_str()).unwrap().clone();
    assert_eq!(row["state"], "Failing", "{row}");
    assert_eq!(row["consecutive_failures"], 3);

    let (s, b) = send(&w.app, "GET", &format!("/api/v1/admin/webhooks/{slug}/notify-housing/deliveries"), ADMIN, None).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    let d = &b["deliveries"][0];
    assert_eq!(d["last_status"], 500, "{b}");
    assert_eq!(d["last_error"], "The receiver answered 500");
}

// 4. Disabling a webhook stops new deliveries, keeps the queued ones in the log as `dead` with a reason, and writes a
// ledger entry. Enabling asks for a reason too.
#[tokio::test(flavor = "multi_thread")]
async fn disabling_stops_deliveries_marks_the_queued_ones_dead_and_writes_the_ledger() {
    let w = world(vec![OK]);
    let slug = app_with_hook(&w).await;
    create(&w, &slug, json!({ "name": "Ada" })).await;
    let uri = format!("/api/v1/admin/webhooks/{slug}/notify-housing/disable");
    let (s, _) = send(&w.app, "POST", &uri, ADMIN, Some(json!({}))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "a reason is required");
    assert_eq!(ledger_count(&w, "WebhookChanged"), 0);

    let (s, b) = send(&w.app, "POST", &uri, ADMIN, Some(json!({ "reason": "The receiver is being replaced" }))).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    let list = deliveries(&w, &slug);
    assert_eq!(list[0]["state"], "dead", "{list:?}");
    assert!(list[0]["last_error"].as_str().unwrap().contains("The receiver is being replaced"), "{list:?}");
    assert_eq!(ledger_count(&w, "WebhookChanged"), 1);

    // A write after that queues nothing, and nothing is sent.
    create(&w, &slug, json!({ "name": "Bo" })).await;
    assert_eq!(deliveries(&w, &slug).len(), 1, "no new delivery while the webhook is off");
    assert_eq!(webhooks::deliver_due(&w.state, &client(&w), chrono::Utc::now()).unwrap(), 0);
    assert!(w.got.lock().unwrap().is_empty(), "nothing reached the receiver");
    let (_, b) = send(&w.app, "GET", "/api/v1/admin/webhooks", ADMIN, None).await;
    assert_eq!(b["webhooks"].as_array().unwrap().iter().find(|r| r["app"] == slug.as_str()).unwrap()["state"], "Disabled");

    let up = format!("/api/v1/admin/webhooks/{slug}/notify-housing/enable");
    let (s, _) = send(&w.app, "POST", &up, ADMIN, Some(json!({}))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "a reason is required");
    let (s, b) = send(&w.app, "POST", &up, ADMIN, Some(json!({ "reason": "The new receiver is ready" }))).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_eq!(ledger_count(&w, "WebhookChanged"), 2);
    let (_, b) = send(&w.app, "GET", "/api/v1/admin/webhooks", ADMIN, None).await;
    assert_eq!(b["webhooks"].as_array().unwrap().iter().find(|r| r["app"] == slug.as_str()).unwrap()["state"], "Active");
}

// 5. The allowlist editor saves the whole list with a reason, and the change is a settings ledger entry.
#[tokio::test(flavor = "multi_thread")]
async fn the_allowlist_saves_with_a_reason_and_a_ledger_entry() {
    let w = world(vec![OK]);
    let (s, b) = send(&w.app, "GET", "/api/v1/admin/egress", ADMIN, None).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert!(b["allow"].as_array().unwrap().iter().any(|e| e["host"] == "hooks.example.org"), "{b}");

    let next = json!([{ "host": "registrar.example.edu", "ports": [443] }]);
    let (s, _) = send(&w.app, "PUT", "/api/v1/admin/egress/allow", ADMIN, Some(json!({ "allow": next }))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "a reason is required");
    let (s, _) = send(&w.app, "PUT", "/api/v1/admin/egress/allow", ADMIN, Some(json!({ "allow": [{ "host": "*.example.edu", "ports": [443] }], "reason": "Wide" }))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "an entry that does not parse is refused");

    let before = ledger_count(&w, "PolicyRevision");
    let (s, b) = send(&w.app, "PUT", "/api/v1/admin/egress/allow", ADMIN, Some(json!({ "allow": next, "reason": "The registrar feed moves here" }))).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_eq!(ledger_count(&w, "PolicyRevision"), before + 1);
    let rationale: String = w.db.run(|c| c.query_one("SELECT rationale FROM governance_ledger WHERE decision_type = 'PolicyRevision' ORDER BY sequence DESC LIMIT 1", &[]).unwrap().get(0));
    assert!(rationale.contains("The registrar feed moves here") && rationale.contains("egress.allow"), "{rationale}");
    let (_, b) = send(&w.app, "GET", "/api/v1/admin/egress", ADMIN, None).await;
    assert_eq!(b["allow"], next);
}

// 6. The Overview counts failing and disabled webhooks, and dead deliveries in the last day.
#[tokio::test(flavor = "multi_thread")]
async fn the_overview_counts_failing_and_disabled_webhooks_and_dead_deliveries() {
    let w = world(vec![OK]);
    let slug = app_with_hook(&w).await;
    create(&w, &slug, json!({ "name": "Ada" })).await;
    let (_, b) = send(&w.app, "GET", "/api/v1/admin/overview", ADMIN, None).await;
    assert_eq!(b["outbound"], json!({ "webhooks_failing": 0, "webhooks_disabled": 0, "deliveries_dead_last_day": 0 }), "{b}");
    send(&w.app, "POST", &format!("/api/v1/admin/webhooks/{slug}/notify-housing/disable"), ADMIN, Some(json!({ "reason": "Maintenance" }))).await;
    let (_, b) = send(&w.app, "GET", "/api/v1/admin/overview", ADMIN, None).await;
    assert_eq!(b["outbound"], json!({ "webhooks_failing": 0, "webhooks_disabled": 1, "deliveries_dead_last_day": 1 }), "{b}");
}
