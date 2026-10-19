//! Stateless server phase 6: readiness, a clean stop, and a restart that changes no answer.

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::Router;
use common::TestDb;
use http_body_util::BodyExt;
use scaffoldry_server::build_app_with_state;
use scaffoldry_server::service::identity::issue_test_token_and_user;
use serde_json::{json, Value};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tower::ServiceExt;

const ADMIN: &str = "jordan.lee@state.edu";
const OWNER: &str = "einstein@physics.state.edu";
const WORKSPACE: &str = "ws-physics-optics";

async fn send(app: &Router, method: &str, uri: &str, who: Option<&str>, body: Option<Value>) -> (StatusCode, Value) {
    let mut b = Request::builder().method(method).uri(uri);
    if let Some(who) = who {
        b = b.header("authorization", format!("Bearer {}", issue_test_token_and_user(who)));
    }
    let req = match body {
        Some(v) => b.header("content-type", "application/json").body(Body::from(v.to_string())).unwrap(),
        None => b.body(Body::empty()).unwrap(),
    };
    let resp = app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
}

// 3. `/readyz` is 503 while the database is unreachable and 200 after it returns, with no restart. `/healthz` checks nothing.
#[tokio::test]
async fn readyz_follows_the_database_and_healthz_does_not() {
    let db = TestDb::new();
    let app = db.app();
    let (s, body) = send(&app, "GET", "/readyz", None, None).await;
    assert_eq!(s, StatusCode::OK, "a server on a reachable database is ready: {body}");

    db.cut_off();
    let (s, body) = send(&app, "GET", "/readyz", None, None).await;
    assert_eq!(s, StatusCode::SERVICE_UNAVAILABLE, "not ready while the database cannot be read: {body}");
    assert_eq!(body["code"], "storage_unavailable", "{body}");
    let (s, body) = send(&app, "GET", "/healthz", None, None).await;
    assert_eq!(s, StatusCode::OK, "healthz checks nothing, so it stays up: {body}");

    db.restore();
    let (s, body) = send(&app, "GET", "/readyz", None, None).await;
    assert_eq!(s, StatusCode::OK, "ready again with no restart: {body}");
}

/// One HTTP exchange over a socket. Returns what the server sent, or an empty string if it sent nothing.
async fn raw(addr: std::net::SocketAddr, request: &str) -> String {
    let mut stream = TcpStream::connect(addr).await.expect("connect");
    stream.write_all(request.as_bytes()).await.unwrap();
    let mut out = Vec::new();
    let _ = tokio::time::timeout(Duration::from_secs(10), stream.read_to_end(&mut out)).await;
    String::from_utf8_lossy(&out).to_string()
}

fn status_of(response: &str) -> Option<u16> {
    response.strip_prefix("HTTP/1.1 ")?.split(' ').next()?.parse().ok()
}

// 2. Instance A is stopped while B serves. Every request sent to B succeeds, and a request in flight on A completes.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn stopping_one_instance_drops_nothing_on_it_and_costs_the_other_nothing() {
    let db = TestDb::new();
    let (a, b) = (db.app(), db.second_app());
    let (la, lb) = (tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap(), tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap());
    let (addr_a, addr_b) = (la.local_addr().unwrap(), lb.local_addr().unwrap());
    let (stop_a, stop_a_rx) = tokio::sync::oneshot::channel::<()>();
    let (stop_b, stop_b_rx) = tokio::sync::oneshot::channel::<()>();
    let grace = Duration::from_secs(30);
    let server_a = tokio::spawn(scaffoldry_server::serve::serve_until(la, a, async move { let _ = stop_a_rx.await; }, grace));
    let server_b = tokio::spawn(scaffoldry_server::serve::serve_until(lb, b, async move { let _ = stop_b_rx.await; }, grace));

    // A request on A that has begun and is not finished: its headers and the first half of its body are sent. It is a
    // write, so finishing it is visible: the dataset exists afterwards.
    let admin = issue_test_token_and_user(ADMIN);
    let body = r#"{"id":"p6_stop","name":"Held","department":"biology"}"#;
    let (first, rest) = body.split_at(20);
    let mut in_flight = TcpStream::connect(addr_a).await.unwrap();
    in_flight
        .write_all(format!("POST /api/v1/datasets HTTP/1.1\r\nHost: a\r\nAuthorization: Bearer {admin}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{first}", body.len()).as_bytes())
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(300)).await;

    // B answers every request the whole time.
    let token = issue_test_token_and_user(ADMIN);
    let hammer = tokio::spawn(async move {
        let mut seen = Vec::new();
        for _ in 0..40 {
            let r = raw(addr_b, &format!("GET /api/v1/orgs HTTP/1.1\r\nHost: b\r\nAuthorization: Bearer {token}\r\nConnection: close\r\n\r\n")).await;
            seen.push(status_of(&r));
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        seen
    });

    let _ = stop_a.send(());
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(!server_a.is_finished(), "A is still waiting on the request it holds");

    // The held request finishes. It gets an answer, not a reset.
    in_flight.write_all(rest.as_bytes()).await.unwrap();
    let mut out = Vec::new();
    let _ = tokio::time::timeout(Duration::from_secs(10), in_flight.read_to_end(&mut out)).await;
    let answer = String::from_utf8_lossy(&out).to_string();
    assert_eq!(status_of(&answer), Some(201), "the request in flight on A completed: {answer:?}");

    let ended = tokio::time::timeout(Duration::from_secs(10), server_a).await.expect("A stops once its requests are done");
    assert!(ended.unwrap().is_ok());
    assert!(TcpStream::connect(addr_a).await.is_err(), "A takes no new connections");

    let seen = hammer.await.unwrap();
    assert!(seen.iter().all(|s| *s == Some(200)), "every request to B succeeded: {seen:?}");
    let _ = stop_b.send(());
    let _ = server_b.await;
}

// A server that holds a request past the grace period is stopped anyway.
#[tokio::test]
async fn a_request_held_past_the_grace_period_does_not_hold_the_stop() {
    let db = TestDb::new();
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = l.local_addr().unwrap();
    let (stop, stop_rx) = tokio::sync::oneshot::channel::<()>();
    let server = tokio::spawn(scaffoldry_server::serve::serve_until(l, db.app(), async move { let _ = stop_rx.await; }, Duration::from_millis(500)));
    let admin = issue_test_token_and_user(ADMIN);
    let mut held = TcpStream::connect(addr).await.unwrap();
    held.write_all(format!("POST /api/v1/datasets HTTP/1.1\r\nHost: a\r\nAuthorization: Bearer {admin}\r\nContent-Type: application/json\r\nContent-Length: 50\r\n\r\n{{").as_bytes()).await.unwrap();
    tokio::time::sleep(Duration::from_millis(200)).await;
    let _ = stop.send(());
    let ended = tokio::time::timeout(Duration::from_secs(5), server).await;
    assert!(ended.is_ok(), "the server stopped after the grace period although a request never finished");
}

// 4. A restarted instance serves the same answers as before the restart for one read of every cached collection.
//    `labels` has no table and no reader of its own yet (see `stateless_phase3_test.rs`). It is read through the schema.
#[tokio::test]
async fn a_restarted_instance_answers_every_cached_read_as_before() {
    let db = TestDb::new();
    let first = {
        let state = db.state();
        state.seed_demo().expect("seed");
        build_app_with_state(state).expect("router")
    };
    // Put something in each cached collection through the running instance.
    let (s, ws) = send(&first, "POST", "/api/v1/workspaces", Some(ADMIN), Some(json!({ "name": "P6WS", "code": "P6WS", "visibility": "restricted", "department": "biology" }))).await;
    assert_eq!(s, StatusCode::CREATED, "{ws}");
    let ws_id = ws["id"].as_str().unwrap().to_string();
    let (s, b) = send(&first, "POST", "/api/v1/orgs", Some(ADMIN), Some(json!({ "name": "P6", "code": "P6", "org_type": "Department", "parent_id": "00000000-0000-0000-0000-000000000001" }))).await;
    assert_eq!(s, StatusCode::CREATED, "{b}");
    let (s, b) = send(&first, "POST", "/api/v1/admin/positions", Some(ADMIN), Some(json!({ "key": "p6dean", "name": "Dean", "org_types": ["Department"], "max_holders": 1, "reason": "A college has a dean" }))).await;
    assert_eq!(s, StatusCode::CREATED, "{b}");
    let (s, b) = send(&first, "POST", "/api/v1/datasets", Some(ADMIN), Some(json!({ "id": "p6_a", "name": "A", "department": "biology" }))).await;
    assert_eq!(s, StatusCode::CREATED, "{b}");
    let (s, b) = send(&first, "POST", "/api/v1/datasets", Some(ADMIN), Some(json!({ "id": "p6_b", "name": "B", "department": "biology" }))).await;
    assert_eq!(s, StatusCode::CREATED, "{b}");
    let (s, b) = send(&first, "POST", "/api/v1/datasets/p6_a/relationships", Some(ADMIN), Some(json!({ "id": "p6_rel", "target_dataset_id": "p6_b", "source_field": "id", "target_field": "id" }))).await;
    assert_eq!(s, StatusCode::CREATED, "{b}");
    let (s, b) = send(&first, "PUT", &format!("/api/v1/workspaces/{ws_id}/guards"), Some(ADMIN), Some(json!({ "rules": [{ "template": "deny_access_outside_units", "unit_ids": ["00000000-0000-0000-0000-000000000001"] }], "reason": "Units only" }))).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    let (s, b) = send(&first, "PUT", "/api/v1/settings/process.stale_days", Some(ADMIN), Some(json!(21))).await;
    assert!(s.is_success(), "{b}");
    let slug = "p6-app";
    let manifest = json!({
        "slug": slug, "title": "P6", "description": "d", "organization_code": "PHYS", "department": "physics", "views": [],
        "tables": [{ "id": "apps", "name": "Applications", "slug": "apps", "fields": [
            { "name": "name", "label": "Name", "field_type": "Text", "required": false, "ferpa_sensitive": false }
        ]}]
    });
    let (s, b) = send(&first, "POST", &format!("/api/v1/workspaces/{WORKSPACE}/apps"), Some(OWNER), Some(manifest)).await;
    assert_eq!(s, StatusCode::CREATED, "{b}");
    let rule = json!({
        "id": "p6-rule", "name": "Rule", "enabled": true, "trigger": "RecordCreated",
        "steps": [{ "id": "s1", "when": [], "kind": { "Service": { "action": { "UpdateRecordStatus": { "new_status": "Approved" } } } } }]
    });
    let (s, b) = send(&first, "POST", &format!("/api/v1/apps/{slug}/automations"), Some(OWNER), Some(rule)).await;
    assert_eq!(s, StatusCode::CREATED, "{b}");

    // One read of each cached collection, as one user sees it.
    let reads: Vec<(&str, String, &str)> = vec![
        ("settings", "/api/v1/settings".into(), ADMIN),
        ("position_types", "/api/v1/admin/positions".into(), ADMIN),
        ("organizations", "/api/v1/orgs".into(), ADMIN),
        ("workspaces", format!("/api/v1/workspaces/{ws_id}"), ADMIN),
        ("guards", format!("/api/v1/workspaces/{ws_id}/guards"), ADMIN),
        ("datasets", "/api/v1/datasets/p6_a".into(), ADMIN),
        ("relationships", "/api/v1/datasets/p6_a/relationships".into(), ADMIN),
        ("manifests (and labels, read through the schema)", format!("/api/v1/apps/{slug}/schema"), OWNER),
        ("rules", format!("/api/v1/apps/{slug}/automations"), OWNER),
        ("policy", "/api/v1/policies".into(), ADMIN),
    ];
    let mut before = Vec::new();
    for (name, uri, who) in &reads {
        let (s, body) = send(&first, "GET", uri, Some(who), None).await;
        assert_eq!(s, StatusCode::OK, "{name}: {body}");
        before.push(body);
    }

    // A restart: a server that did not exist before reads the same database.
    let restarted = db.second_app();
    for ((name, uri, who), was) in reads.iter().zip(before) {
        let (s, now) = send(&restarted, "GET", uri, Some(who), None).await;
        assert_eq!(s, StatusCode::OK, "{name} after the restart: {now}");
        assert_eq!(now, was, "{name} answers differently after a restart");
    }
}
