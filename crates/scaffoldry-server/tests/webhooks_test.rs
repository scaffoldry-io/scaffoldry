//! Integrations phase 4: webhooks behind the outbound allowlist and a proposal, with signing, retries and a call log.
//!
//! A local HTTP server in this process stands in for the receiver. It is reached through an injected resolver and an
//! allowlist entry, over plain http, which `webhooks.allow_http` allows for tests and local work only.

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

/// One request the receiver got.
#[derive(Clone, Debug)]
struct Got {
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

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
const FAIL: &str = "HTTP/1.1 500 Internal Server Error\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
const MOVED: &str = "HTTP/1.1 302 Found\r\nLocation: http://169.254.169.254/\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";

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

/// Records whose status is not Draft.
fn not_draft() -> Value {
    json!({ "conjunction": "And", "clauses": [{ "id": "c1", "field_name": "status", "operator": "NotEquals", "value": "Draft" }] })
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

fn checks_of(body: &Value) -> Vec<Value> {
    body["checks"].as_array().cloned().or_else(|| body["proposal"]["checks"].as_array().cloned()).unwrap_or_default()
}

// 1. A webhook whose host is not allowlisted fails `webhook_host` at proposal time. One with a missing secret fails too.
#[tokio::test(flavor = "multi_thread")]
async fn an_unlisted_host_or_a_missing_secret_fails_webhook_host() {
    let w = world(vec![OK]);
    // A failed check does not refuse the proposal: it is stored with the check failed, and a failed check blocks approval.
    let failing = |body: &Value| checks_of(body).into_iter().find(|c| c["name"] == "webhook_host").unwrap_or_else(|| panic!("a webhook_host check: {body}"));

    let away = hook(&w, json!({ "url": "https://elsewhere.example.net/in" }));
    let (s, b) = send(&w.app, "POST", &format!("/api/v1/workspaces/{WORKSPACE}/apps"), OWNER, Some(manifest(&slug(), vec![away]))).await;
    assert_eq!(s, StatusCode::ACCEPTED, "{b}");
    let c = failing(&b);
    assert_eq!(c["outcome"], "fail", "{c}");
    assert!(c.to_string().contains("elsewhere.example.net"), "{c}");

    let nosecret = hook(&w, json!({ "secret": "no-such-secret" }));
    let (s, b) = send(&w.app, "POST", &format!("/api/v1/workspaces/{WORKSPACE}/apps"), OWNER, Some(manifest(&slug(), vec![nosecret]))).await;
    assert_eq!(s, StatusCode::ACCEPTED, "{b}");
    let c = failing(&b);
    assert_eq!(c["outcome"], "fail", "{c}");
    assert!(c.to_string().contains("no-such-secret"), "{c}");

    // A webhook on the list, with its secret, passes.
    let (s, b) = send(&w.app, "POST", &format!("/api/v1/workspaces/{WORKSPACE}/apps"), OWNER, Some(manifest(&slug(), vec![hook(&w, json!({}))]))).await;
    assert!(s.is_success(), "{b}");
    assert!(checks_of(&b).iter().all(|c| c["name"] != "webhook_host" || c["outcome"] == "pass"), "{b}");
}

// 2. A matching write makes a delivery in its own transaction. A rolled-back write makes none.
#[tokio::test(flavor = "multi_thread")]
async fn a_write_makes_its_delivery_in_its_transaction() {
    let w = world(vec![OK]);
    let slug = make_app(&w, vec![hook(&w, json!({}))]).await;
    let (s, b) = create(&w, &slug, json!({ "name": "Ada" })).await;
    assert!(s.is_success(), "{b}");
    let list = deliveries(&w, &slug);
    assert_eq!(list.len(), 1, "{list:?}");
    assert_eq!(list[0]["state"], "queued");
    assert_eq!(list[0]["payload"]["event"], "record.created");

    // A delivery that cannot be stored fails the write with it: neither the record nor the delivery is kept.
    w.db.run(|c| {
        c.batch_execute(
            "CREATE FUNCTION refuse_boom() RETURNS trigger AS $$ BEGIN IF NEW.payload->'changed' ? 'boom' THEN RAISE EXCEPTION 'refused'; END IF; RETURN NEW; END $$ LANGUAGE plpgsql;
             CREATE TRIGGER refuse_boom BEFORE INSERT ON webhook_deliveries FOR EACH ROW EXECUTE FUNCTION refuse_boom();",
        )
        .unwrap()
    });
    let (s, _) = create(&w, &slug, json!({ "name": "Bo", "boom": "x" })).await;
    assert!(!s.is_success(), "the write fails with its delivery");
    assert_eq!(deliveries(&w, &slug).len(), 1, "no second delivery");
    let names: i64 = {
        let slug = slug.clone();
        w.db.run(move |c| c.query_one("SELECT COUNT(*) FROM dataset_records WHERE app_slug = $1 AND data->>'name' = 'Bo'", &[&slug]).unwrap().get(0))
    };
    assert_eq!(names, 0, "the record rolled back with it");
}

// 3. A delivery is signed. The signature verifies with the secret and the timestamp, and a changed body fails.
#[tokio::test(flavor = "multi_thread")]
async fn a_delivery_is_signed_and_verifies() {
    let w = world(vec![OK]);
    let slug = make_app(&w, vec![hook(&w, json!({}))]).await;
    create(&w, &slug, json!({ "name": "Ada" })).await;
    let sent = webhooks::deliver_due(&w.state, &client(&w), chrono::Utc::now()).unwrap();
    assert_eq!(sent, 1);
    let got = w.got.lock().unwrap()[0].clone();
    let sig = got.header("X-Scaffoldry-Signature").expect("a signature");
    assert!(sig.starts_with("t=") && sig.contains(",v1="), "{sig}");
    assert!(webhooks::verify(secret_value(), &sig, &got.body));
    let mut changed = got.body.clone();
    changed.push(b' ');
    assert!(!webhooks::verify(secret_value(), &sig, &changed), "a changed body fails");
    assert!(!webhooks::verify(format!("other-{}", uuid::Uuid::new_v4().simple()).as_bytes(), &sig, &got.body), "another secret fails");
    assert_eq!(got.header("X-Scaffoldry-Event-Id").as_deref(), got.json()["id"].as_str(), "the event id is the idempotency key");
    assert_eq!(deliveries(&w, &slug)[0]["state"], "delivered");
}

// 4. A 500 retries on the schedule and ends dead. A 200 after two failures ends delivered.
#[tokio::test(flavor = "multi_thread")]
async fn a_failure_retries_on_the_schedule_until_dead() {
    let w = world(vec![FAIL]);
    let slug = make_app(&w, vec![hook(&w, json!({}))]).await;
    create(&w, &slug, json!({ "name": "Ada" })).await;
    // A whole second, so the waits are measured exactly.
    let mut now = chrono::DateTime::from_timestamp(chrono::Utc::now().timestamp() + 1, 0).unwrap();
    let mut waits = Vec::new();
    for _ in 0..8 {
        webhooks::deliver_due(&w.state, &client(&w), now).unwrap();
        let d = &deliveries(&w, &slug)[0];
        if d["state"] == "dead" {
            break;
        }
        let next = d["next"].as_f64().unwrap();
        waits.push((next - now.timestamp() as f64).round() as i64);
        now = chrono::DateTime::from_timestamp(next as i64 + 1, 0).unwrap();
    }
    assert_eq!(waits, [60, 300, 1800, 7200, 21_600, 43_200, 86_400], "the stated delays");
    let d = &deliveries(&w, &slug)[0];
    assert_eq!(d["state"], "dead", "{d}");
    assert_eq!(d["attempts"], 8, "the first try and seven retries");
    assert_eq!(d["last_status"], 500);

    let w = world(vec![FAIL, FAIL, OK]);
    let slug = make_app(&w, vec![hook(&w, json!({}))]).await;
    create(&w, &slug, json!({ "name": "Ada" })).await;
    let mut now = chrono::Utc::now();
    for _ in 0..3 {
        webhooks::deliver_due(&w.state, &client(&w), now).unwrap();
        now += chrono::Duration::hours(1);
    }
    let d = &deliveries(&w, &slug)[0];
    assert_eq!(d["state"], "delivered", "{d}");
    assert_eq!(d["attempts"], 3);
}

// 5. Twenty failures in a row disable the webhook and tell the owner. Re-enabling clears the count.
#[tokio::test(flavor = "multi_thread")]
async fn twenty_failures_disable_the_webhook_until_an_owner_enables_it() {
    let w = world(vec![FAIL]);
    let slug = make_app(&w, vec![hook(&w, json!({}))]).await;
    for i in 0..20 {
        create(&w, &slug, json!({ "name": format!("N{i}") })).await;
    }
    webhooks::deliver_due(&w.state, &client(&w), chrono::Utc::now()).unwrap();
    let state = |w: &World| {
        let slug = slug.clone();
        w.db.run(move |c| {
            let r = c.query_one("SELECT consecutive_failures, disabled_at IS NOT NULL FROM webhook_state WHERE app_slug = $1 AND webhook_id = 'notify-housing'", &[&slug]).unwrap();
            (r.get::<_, i32>(0), r.get::<_, bool>(1))
        })
    };
    assert_eq!(state(&w), (20, true));
    let told: i64 = w.db.run(|c| c.query_one("SELECT COUNT(*) FROM notifications WHERE recipient = $1", &[&OWNER]).unwrap().get(0));
    assert!(told >= 1, "the owner is told");
    // A disabled webhook sends nothing more.
    let before = w.got.lock().unwrap().len();
    create(&w, &slug, json!({ "name": "late" })).await;
    webhooks::deliver_due(&w.state, &client(&w), chrono::Utc::now() + chrono::Duration::days(2)).unwrap();
    assert_eq!(w.got.lock().unwrap().len(), before);

    let (s, b) = send(&w.app, "POST", &format!("/api/v1/apps/{slug}/webhooks/notify-housing/enable"), OWNER, None).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_eq!(state(&w), (0, false));
}

// 6. Without values the payload has none. With them it has only the listed fields.
#[tokio::test(flavor = "multi_thread")]
async fn a_payload_holds_only_the_listed_values() {
    let w = world(vec![OK]);
    let slug = make_app(&w, vec![hook(&w, json!({})), hook(&w, json!({ "id": "with-values", "include_values": true, "fields": ["name"], "filter": not_draft() }))]).await;
    create(&w, &slug, json!({ "name": "Ada Okoro", "email": "ada@state.edu", "status": "New" })).await;
    let list = deliveries(&w, &slug);
    assert_eq!(list.len(), 2, "{list:?}");
    let plain = list.iter().find(|d| d["payload"].get("values").is_none()).expect("one without values");
    assert!(!plain.to_string().contains("Ada Okoro") && !plain.to_string().contains("ada@state.edu"), "no value in it: {plain}");
    let valued = list.iter().find(|d| d["payload"].get("values").is_some()).expect("one with values");
    assert_eq!(valued["payload"]["values"], json!({ "name": "Ada Okoro" }));
    assert!(!valued.to_string().contains("ada@state.edu"), "only the listed fields: {valued}");
    for key in ["id", "event", "app", "table", "record", "version", "at", "changed"] {
        assert!(valued["payload"].get(key).is_some(), "{key}: {valued}");
    }
}

// 7. A sensitive listed field raises `webhook_sensitive`, and the proposer cannot approve it alone.
#[tokio::test(flavor = "multi_thread")]
async fn a_sensitive_listed_field_needs_a_second_person() {
    let w = world(vec![OK]);
    let h = hook(&w, json!({ "include_values": true, "fields": ["ssn"] }));
    let (s, b) = send(&w.app, "POST", &format!("/api/v1/workspaces/{WORKSPACE}/apps"), OWNER, Some(manifest(&slug(), vec![h]))).await;
    assert_eq!(s, StatusCode::ACCEPTED, "{b}");
    let id = b["proposal_id"].as_str().unwrap().to_string();
    let (_, p) = send(&w.app, "GET", &format!("/api/v1/proposals/{id}"), OWNER, None).await;
    let checks = checks_of(&p);
    let sensitive = checks.iter().find(|c| c["name"] == "webhook_sensitive").unwrap_or_else(|| panic!("{p}"));
    assert_ne!(sensitive["outcome"], "pass", "{sensitive}");
    let scope = checks.iter().find(|c| c["name"] == "webhook_scope").unwrap_or_else(|| panic!("{p}"));
    assert_eq!(scope["outcome"], "warn", "values with no filter warn: {scope}");
    let (s, b) = send(&w.app, "POST", &format!("/api/v1/proposals/{id}/decision"), OWNER, Some(json!({ "decision": "approve", "rationale": "mine", "digest": p["digest"] }))).await;
    assert!(s.is_client_error(), "the proposer may not approve it alone: {s} {b}");
}

// 8. A redirect is a failure, and a private address is refused.
#[tokio::test(flavor = "multi_thread")]
async fn a_redirect_fails_and_a_private_address_is_refused() {
    let w = world(vec![MOVED]);
    let slug = make_app(&w, vec![hook(&w, json!({}))]).await;
    create(&w, &slug, json!({ "name": "Ada" })).await;
    webhooks::deliver_due(&w.state, &client(&w), chrono::Utc::now()).unwrap();
    let d = &deliveries(&w, &slug)[0];
    assert_eq!(d["state"], "queued", "it will retry: {d}");
    assert!(d["last_error"].as_str().unwrap_or_default().to_lowercase().contains("redirect"), "{d}");

    let w2 = world(vec![OK]);
    let slug2 = make_app(&w2, vec![hook(&w2, json!({}))]).await;
    create(&w2, &slug2, json!({ "name": "Ada" })).await;
    let private = {
        let s = w2.state.settings.read().unwrap();
        Client::new(Allow::from_settings(&s), Arc::new(To("10.1.2.3".parse().unwrap())))
    };
    webhooks::deliver_due(&w2.state, &private, chrono::Utc::now()).unwrap();
    let d = &deliveries(&w2, &slug2)[0];
    assert_eq!(d["state"], "queued");
    let err = d["last_error"].as_str().unwrap_or_default().to_lowercase();
    assert!(err.contains("10.1.2.3") || err.contains("not allowed") || err.contains("private"), "{d}");
    assert!(w2.got.lock().unwrap().is_empty(), "nothing reached a private address");
}

// 9. A process rule's `WebhookDispatch` must name an approved webhook of the same app.
#[tokio::test(flavor = "multi_thread")]
async fn a_rule_names_an_approved_webhook_of_its_app() {
    use scaffoldry_core::{ActionType, AutomationRule, ProcessStep, StepKind, TriggerEvent};
    use scaffoldry_engine::HostRouter;
    let w = world(vec![OK]);
    let slug = make_app(&w, vec![hook(&w, json!({}))]).await;
    let manifest = w.state.engine.read().unwrap().resolve_by_slug(&slug).cloned().unwrap();
    let rule = |id: &str| AutomationRule {
        id: "tell".into(),
        app_slug: slug.clone(),
        name: "Tell".into(),
        description: String::new(),
        enabled: true,
        trigger: TriggerEvent::RecordCreated,
        predicates: vec![],
        actions: vec![],
        version: 0,
        home_workspace: None,
        grants: vec![],
        steps: vec![ProcessStep { id: "send".into(), when: vec![], kind: StepKind::Service { action: ActionType::WebhookDispatch { webhook_id: id.into() } } }],
    };
    let bad = scaffoldry_server::service::processes::validate_rule(&w.state, &manifest, &rule("nope"));
    assert!(bad.iter().any(|p| p.contains("nope")), "{bad:?}");
    let good = scaffoldry_server::service::processes::validate_rule(&w.state, &manifest, &rule("notify-housing"));
    assert!(good.is_empty(), "{good:?}");
}

// 10. A sensitive delivery writes one disclosure row per webhook and hour, with a count.
#[tokio::test(flavor = "multi_thread")]
async fn sensitive_deliveries_are_one_disclosure_per_hour() {
    let w = world(vec![OK]);
    let slug = make_app(&w, vec![hook(&w, json!({ "include_values": true, "fields": ["name", "ssn"], "filter": not_draft() }))]).await;
    for i in 0..3 {
        create(&w, &slug, json!({ "name": format!("N{i}"), "ssn": format!("000-00-000{i}"), "status": "New" })).await;
    }
    assert_eq!(webhooks::deliver_due(&w.state, &client(&w), chrono::Utc::now()).unwrap(), 3);
    let rows: Vec<(i64, Option<String>)> = {
        let slug = slug.clone();
        w.db.run(move |c| c.query("SELECT record_count, recipient FROM disclosure_log WHERE app_slug = $1 AND kind = 'webhook'", &[&slug]).unwrap().iter().map(|r| (r.get(0), r.get(1))).collect())
    };
    assert_eq!(rows.len(), 1, "one row for the hour: {rows:?}");
    assert_eq!(rows[0].0, 3);
    assert_eq!(rows[0].1.as_deref(), Some("hooks.example.org"));
}

// 11. The call log keeps the last 200 deliveries per webhook after a purge.
#[tokio::test(flavor = "multi_thread")]
async fn the_call_log_keeps_200_per_webhook() {
    let w = world(vec![OK]);
    let slug = make_app(&w, vec![hook(&w, json!({}))]).await;
    {
        let slug = slug.clone();
        w.db.run(move |c| {
            c.execute(
                "INSERT INTO webhook_deliveries (id, app_slug, webhook_id, event_id, payload, state, attempts, created_at, delivered_at, last_status) \
                 SELECT gen_random_uuid(), $1, 'notify-housing', gen_random_uuid(), '{}'::jsonb, 'delivered', 1, NOW() - (g || ' minutes')::interval, NOW(), 200 FROM generate_series(1, 230) g",
                &[&slug],
            )
            .unwrap()
        });
    }
    let removed = webhooks::purge_deliveries(&w.state.repository).unwrap();
    assert_eq!(removed, 30);
    assert_eq!(deliveries(&w, &slug).len(), 200);
    let (s, log) = send(&w.app, "GET", &format!("/api/v1/apps/{slug}/webhooks/notify-housing/deliveries"), OWNER, None).await;
    assert_eq!(s, StatusCode::OK, "{log}");
    assert_eq!(log["deliveries"].as_array().unwrap().len(), 200);
    assert!(log["deliveries"][0].get("payload").is_none(), "the call log holds no payload");
}

// A live rule from before webhooks were approved with the app loses its URL step, with one ledger note. A rule that
// names an approved webhook by id is left alone.
#[tokio::test(flavor = "multi_thread")]
async fn old_url_webhook_steps_are_dropped_with_one_ledger_note() {
    let w = world(vec![OK]);
    let old = json!({
        "id": "old-hook", "app_slug": "legacy", "name": "Old", "enabled": true, "trigger": "RecordCreated", "predicates": [], "actions": [], "version": 1,
        "steps": [
            { "id": "keep", "when": [], "kind": { "Service": { "action": { "UpdateRecordStatus": { "new_status": "Open" } } } } },
            { "id": "send", "when": [], "kind": { "Service": { "action": { "WebhookDispatch": { "target_url": "https://example.org/x" } } } } },
            { "id": "by-id", "when": [], "kind": { "Service": { "action": { "WebhookDispatch": { "webhook_id": "notify-housing" } } } } }
        ],
    });
    w.db.run(move |c| c.execute("INSERT INTO workflow_automations (id, app_slug, name, rule_json, enabled) VALUES ('old-hook', 'legacy', 'Old', $1, TRUE)", &[&old]).unwrap());
    let before: i64 = w.db.run(|c| c.query_one("SELECT COUNT(*) FROM governance_ledger WHERE rationale LIKE '%webhook steps that named a URL%'", &[]).unwrap().get(0));
    assert_eq!(webhooks::drop_url_webhook_steps(&w.state.repository).unwrap(), 1);
    let after: Value = w.db.run(|c| c.query_one("SELECT rule_json FROM workflow_automations WHERE id = 'old-hook'", &[]).unwrap().get(0));
    let ids: Vec<&str> = after["steps"].as_array().unwrap().iter().map(|s| s["id"].as_str().unwrap()).collect();
    assert_eq!(ids, ["keep", "by-id"], "{after}");
    let noted: i64 = w.db.run(|c| c.query_one("SELECT COUNT(*) FROM governance_ledger WHERE rationale LIKE '%webhook steps that named a URL%'", &[]).unwrap().get(0));
    assert_eq!(noted - before, 1, "one counted note");
    assert_eq!(webhooks::drop_url_webhook_steps(&w.state.repository).unwrap(), 0, "a second boot drops nothing and writes nothing");
}
