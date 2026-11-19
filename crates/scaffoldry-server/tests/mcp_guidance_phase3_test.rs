//! MCP guidance phase 3: `request_access` with `explain: true` says in words why the caller can or cannot do the thing,
//! names the rule, files nothing, refuses to explain another person's access, and carries no record value. The tool
//! list is full, so the explanation is an argument of an existing tool and not a new tool.

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use common::TestDb;
use http_body_util::BodyExt;
use scaffoldry_server::build_app_with_state;
use scaffoldry_server::service::identity::issue_test_token_and_user;
use serde_json::{json, Value};
use tower::ServiceExt;
use uuid::Uuid;

const WORKSPACE: &str = "ws-physics-optics";
const OWNER: &str = "einstein@physics.state.edu";
const OUTSIDER: &str = "marcus.vance@state.edu";
const SECRET: &str = "SECRET-VALUE-9031";

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

async fn call(app: &axum::Router, who: &str, tool: &str, args: Value) -> Value {
    let payload = json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": { "name": tool, "arguments": args } });
    send(app, "POST", "/api/v1/mcp", who, Some(payload)).await.1["result"].clone()
}

async fn fixture(db: &TestDb) -> (axum::Router, String) {
    let state = db.state();
    state.seed_demo().expect("seed");
    let app = build_app_with_state(state).expect("router");
    let slug = format!("g3-{}", &Uuid::new_v4().to_string()[..8]);
    let manifest = json!({
        "slug": slug, "title": "Guidance", "description": "d", "organization_code": "PHYS", "department": "physics", "views": [],
        "tables": [{ "id": "people", "name": "People", "slug": "people", "fields": [
            { "name": "name", "label": "Name", "field_type": "Text", "required": true, "ferpa_sensitive": false }
        ] }]
    });
    let (s, body) = send(&app, "POST", &format!("/api/v1/workspaces/{WORKSPACE}/apps"), OWNER, Some(manifest)).await;
    assert_eq!(s, StatusCode::CREATED, "{body}");
    let r = call(&app, OWNER, "create_record", json!({ "app_slug": slug, "data": { "_table_id": "people", "name": SECRET } })).await;
    assert!(r["structuredContent"]["id"].is_string(), "{r}");
    (app, slug)
}

fn requests(db: &TestDb) -> i64 {
    db.run(|c| c.query_one("SELECT COUNT(*) FROM access_requests", &[]).unwrap().get(0))
}

#[tokio::test]
async fn explain_says_why_the_caller_is_refused_names_the_rule_and_files_nothing() {
    let db = TestDb::new();
    let (app, slug) = fixture(&db).await;
    let before = requests(&db);
    let args = json!({ "object_kind": "app", "object_id": slug, "role": "editor", "explain": true });
    let r = call(&app, OUTSIDER, "request_access", args).await;
    assert_ne!(r["isError"], json!(true), "{r}");
    let s = &r["structuredContent"];
    assert_eq!(s["allowed"], json!(false), "{s}");
    assert_eq!(s["filed"], json!(false), "{s}");
    assert!(s["rule"]["id"].as_str().is_some_and(|i| !i.is_empty()), "the explanation names the rule: {s}");
    let words = s["explanation"].as_str().unwrap_or("");
    assert!(words.split_whitespace().count() >= 8 && words.contains("request_access"), "the explanation is in words and says what to do: {s}");
    assert!(s["guidance"]["next"].as_str().is_some_and(|n| !n.contains("waits for a person")), "the filing guidance does not apply: {s}");
    assert_eq!(requests(&db), before, "an explanation files no access request");

    // The owner is allowed, and is told so in words.
    let args = json!({ "object_kind": "app", "object_id": slug, "role": "viewer", "explain": true });
    let r = call(&app, OWNER, "request_access", args).await;
    assert_eq!(r["structuredContent"]["allowed"], json!(true), "{r}");
    assert!(r["structuredContent"]["rule"]["id"].is_string(), "{r}");
    assert_eq!(requests(&db), before);
}

#[tokio::test]
async fn explain_refuses_another_persons_access_and_says_why() {
    let db = TestDb::new();
    let (app, slug) = fixture(&db).await;
    let before = requests(&db);
    // Even a person who owns the workspace may not ask about someone else. No steward role exists in code to allow it.
    for (asker, about) in [(OUTSIDER, OWNER), (OWNER, OUTSIDER)] {
        let args = json!({ "object_kind": "app", "object_id": slug, "role": "viewer", "explain": true, "person": about });
        let r = call(&app, asker, "request_access", args).await;
        assert_eq!(r["isError"], json!(true), "{r}");
        let s = &r["structuredContent"];
        assert_eq!(s["code"], "forbidden", "{s}");
        assert!(s["message"].as_str().is_some_and(|m| m.contains("steward")), "the refusal says no steward role exists: {s}");
        assert!(!s.to_string().contains(about), "the refusal does not repeat the other person: {s}");
    }
    // Naming yourself is fine, in any letter case.
    let args = json!({ "object_kind": "app", "object_id": slug, "role": "viewer", "explain": true, "person": OUTSIDER.to_uppercase() });
    let r = call(&app, OUTSIDER, "request_access", args).await;
    assert_ne!(r["isError"], json!(true), "{r}");
    assert_eq!(requests(&db), before);
}

#[tokio::test]
async fn an_explanation_carries_no_record_value() {
    let db = TestDb::new();
    let (app, slug) = fixture(&db).await;
    for who in [OWNER, OUTSIDER] {
        for role in ["viewer", "editor"] {
            let args = json!({ "object_kind": "app", "object_id": slug, "role": role, "explain": true, "days": 14 });
            let r = call(&app, who, "request_access", args).await;
            assert_ne!(r["isError"], json!(true), "{r}");
            assert!(!r.to_string().contains(SECRET), "{r}");
        }
    }
}

#[tokio::test]
async fn filing_still_needs_a_reason_and_days() {
    let db = TestDb::new();
    let (app, slug) = fixture(&db).await;
    let r = call(&app, OUTSIDER, "request_access", json!({ "object_kind": "app", "object_id": slug, "role": "viewer" })).await;
    assert_eq!(r["isError"], json!(true), "{r}");
}
