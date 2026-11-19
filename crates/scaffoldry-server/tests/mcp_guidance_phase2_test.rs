//! MCP guidance phase 2: every error code has a hint that names a next step, a refusal names the rule and the request
//! path, and the main results carry a small `guidance` block. None of it holds a record value.

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use common::TestDb;
use http_body_util::BodyExt;
use scaffoldry_server::build_app_with_state;
use scaffoldry_server::service::hints::{hint_for, hint_for_code};
use scaffoldry_server::service::identity::issue_test_token_and_user;
use scaffoldry_server::service::tools::TOOLS;
use scaffoldry_server::service::{ServiceError, ERROR_CODES};
use serde_json::{json, Value};
use tower::ServiceExt;
use uuid::Uuid;

const WORKSPACE: &str = "ws-physics-optics";
const OWNER: &str = "einstein@physics.state.edu";
const OUTSIDER: &str = "marcus.vance@state.edu";
const SECRET: &str = "SECRET-VALUE-4417";

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

async fn fixture(app: &axum::Router) -> (String, String) {
    let slug = format!("g2-{}", &Uuid::new_v4().to_string()[..8]);
    let manifest = json!({
        "slug": slug, "title": "Guidance", "description": "d", "organization_code": "PHYS", "department": "physics", "views": [],
        "tables": [{ "id": "people", "name": "People", "slug": "people", "fields": [
            { "name": "name", "label": "Name", "field_type": "Text", "required": true, "ferpa_sensitive": false }
        ] }]
    });
    let (s, body) = send(app, "POST", &format!("/api/v1/workspaces/{WORKSPACE}/apps"), OWNER, Some(manifest)).await;
    assert_eq!(s, StatusCode::CREATED, "{body}");
    let r = call(app, OWNER, "create_record", json!({ "app_slug": slug, "data": { "_table_id": "people", "name": SECRET } })).await;
    let id = r["structuredContent"]["id"].as_str().unwrap_or_default().to_string();
    assert!(!id.is_empty(), "{r}");
    (slug, id)
}

/// A next step is a tool, a route or resource, or a concrete action an agent or a person can take.
fn names_a_next_step(hint: &str) -> bool {
    const ACTIONS: [&str; 14] = ["Sign in", "Ask ", "Wait", "Retry", "Choose", "Stop", "Exit", "Change it at", "Send ", "Give it", "Make ", "Remove ", "Use ", "Open "];
    TOOLS.iter().any(|t| hint.contains(t.name)) || hint.contains("scaffoldry://") || hint.contains("/api/v1/") || ACTIONS.iter().any(|a| hint.contains(a))
}

#[test]
fn every_error_code_has_a_hint_that_names_a_next_step() {
    let mut problems = Vec::new();
    for code in ERROR_CODES {
        match hint_for_code(code) {
            None => problems.push(format!("{code}: no hint")),
            Some(h) => {
                if h.trim().is_empty() {
                    problems.push(format!("{code}: empty hint"));
                }
                if !names_a_next_step(h) {
                    problems.push(format!("{code}: the hint names no next step: {h}"));
                }
                if !h.ends_with('.') || h.contains('\n') || h.chars().count() > 220 {
                    problems.push(format!("{code}: not one short sentence: {h}"));
                }
            }
        }
    }
    assert!(problems.is_empty(), "{}", problems.join("\n"));
    assert!(hint_for_code("not_a_code").is_none(), "an unknown code has no hint, so the test above cannot pass by default");
}

#[test]
fn a_hint_never_carries_text_from_the_error_message() {
    let variants = vec![
        ServiceError::NotFound(format!("Record {SECRET} not found")),
        ServiceError::BadRequest(format!("Unknown argument '{SECRET}'")),
        ServiceError::forbidden(format!("{SECRET} may not")),
        ServiceError::Conflict(SECRET.into()),
        ServiceError::Archived(SECRET.into()),
        ServiceError::RateLimited(SECRET.into()),
        ServiceError::ShareNotAllowed(SECRET.into()),
        ServiceError::SensitiveContentBlocked(SECRET.into()),
        ServiceError::MasterKeyMissing(SECRET.into()),
        ServiceError::Internal(SECRET.into()),
        ServiceError::ConstraintViolated(SECRET.into()),
        ServiceError::TooManyIndexes { message: SECRET.into(), unused: vec![SECRET.into()] },
    ];
    for e in variants {
        let h = hint_for(&e);
        assert!(!h.is_empty() && !h.contains(SECRET), "{}: {h}", e.code());
    }
}

#[tokio::test]
async fn an_http_error_body_gains_a_hint_and_keeps_its_old_fields() {
    let _db = TestDb::new();
    let body = ServiceError::NotFound("Gone".into()).body();
    assert_eq!(body["error"], "Gone");
    assert_eq!(body["code"], "not_found");
    assert!(body["hint"].as_str().is_some_and(|h| !h.is_empty()), "{body}");
}

#[tokio::test]
async fn a_refusal_names_the_rule_and_the_request_path() {
    let db = TestDb::new();
    let state = db.state();
    state.seed_demo().expect("seed");
    let app = build_app_with_state(state).expect("router");
    let (slug, _) = fixture(&app).await;
    let r = call(&app, OUTSIDER, "create_record", json!({ "app_slug": slug, "data": { "_table_id": "people", "name": "Bo" } })).await;
    assert_eq!(r["isError"], json!(true), "{r}");
    let s = &r["structuredContent"];
    assert_eq!(s["code"], "forbidden", "{r}");
    let rule = &s["rule"];
    assert!(rule["id"].as_str().is_some_and(|i| !i.is_empty()), "the refusal names the rule that refused: {s}");
    assert!(s["request_path"].as_str().is_some_and(|p| p.contains("request_access") && p.contains("/api/v1/access/requests")), "{s}");
    assert!(s["request"]["object_id"].is_string(), "the filled request is still there: {s}");
    // A client that shows only the text still learns the next step from the hint.
    assert!(r["content"][1]["text"].as_str().unwrap_or("").contains("request_access"), "{r}");
}

#[tokio::test]
async fn a_refusal_with_no_request_path_says_who_to_ask() {
    let db = TestDb::new();
    let state = db.state();
    state.seed_demo().expect("seed");
    let app = build_app_with_state(state).expect("router");
    let r = call(&app, OWNER, "simulate_cedar_policy", json!({ "eppn": OWNER, "action": "read" })).await;
    let s = &r["structuredContent"];
    assert_eq!(s["code"], "forbidden", "{r}");
    assert!(s["rule"]["id"].as_str().is_some_and(|i| !i.is_empty()), "{s}");
    assert!(s["request_path"].as_str().is_some_and(|p| !p.is_empty()), "{s}");
}

#[tokio::test]
async fn the_main_results_carry_a_small_guidance_block_with_no_record_value() {
    let db = TestDb::new();
    let state = db.state();
    state.seed_demo().expect("seed");
    let app = build_app_with_state(state).expect("router");
    let (slug, rec) = fixture(&app).await;
    let draft = json!({ "slug": format!("d-{}", &Uuid::new_v4().to_string()[..8]), "title": "Draft", "description": "d", "organization_code": "PHYS", "department": "physics", "views": [],
        "tables": [{ "id": "t", "name": "T", "slug": "t", "fields": [{ "name": "a", "label": "A", "field_type": "Text", "required": false, "ferpa_sensitive": false }] }] });
    let cases: Vec<(&str, Value)> = vec![
        ("list_workspaces", json!({})),
        ("list_apps", json!({ "workspace_id": WORKSPACE })),
        ("describe_app", json!({ "app_slug": slug })),
        ("list_records", json!({ "app_slug": slug, "table_id": "people" })),
        ("get_record", json!({ "app_slug": slug, "record_id": rec })),
        ("create_record", json!({ "app_slug": slug, "data": { "_table_id": "people", "name": SECRET } })),
        ("propose_app_change", json!({ "workspace_id": WORKSPACE, "manifest": draft, "summary": "A draft" })),
    ];
    for (tool, args) in cases {
        let r = call(&app, OWNER, tool, args).await;
        assert_ne!(r["isError"], json!(true), "{tool}: {r}");
        let g = &r["structuredContent"]["guidance"];
        let next = g["next"].as_str().unwrap_or("");
        assert!(!next.is_empty() && names_a_next_step(next), "{tool}: guidance.next names a next step: {g}");
        assert!(g["avoid"].as_str().is_some_and(|a| !a.is_empty()), "{tool}: guidance.avoid: {g}");
        assert!(g.to_string().len() <= 500, "{tool}: the block stays small: {}", g.to_string().len());
        assert!(!g.to_string().contains(SECRET), "{tool}: no record value in guidance");
    }
    // A tool that is not a main result does not carry one.
    let r = call(&app, OWNER, "get_job", json!({ "job_id": Uuid::new_v4().to_string() })).await;
    assert!(r["structuredContent"]["guidance"].is_null());
}
