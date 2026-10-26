//! Tools that did the same job were combined, so the list stays short enough for a client to load:
//! `get_page` lists the pages when it is given no page, `get_record` returns the record's history when it is asked for
//! it, and `governance_report` holds the three reports that were tools of their own.

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::Router;
use http_body_util::BodyExt;
use scaffoldry_server::build_app_with_state;
use scaffoldry_server::service::identity::issue_test_token_and_user;
use serde_json::{json, Value};
use tower::ServiceExt;

const OWNER: &str = "einstein@physics.state.edu";
const WORKSPACE: &str = "ws-physics-optics";

async fn rest(app: &Router, method: &str, uri: &str, who: &str, body: Option<Value>) -> (StatusCode, Value) {
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

/// The whole result of a tool call.
async fn call(app: &Router, name: &str, args: Value) -> Value {
    let payload = json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": { "name": name, "arguments": args } });
    rest(app, "POST", "/api/mcp", OWNER, Some(payload)).await.1["result"].clone()
}

async fn world() -> (common::TestDb, Router, String) {
    let db = common::TestDb::new();
    let state = db.state();
    state.seed_demo().expect("seed");
    let app = build_app_with_state(state).expect("router");
    let slug = format!("comb-{}", &uuid::Uuid::new_v4().simple().to_string()[..8]);
    let manifest = json!({
        "slug": slug, "title": "Applications", "description": "d", "organization_code": "PHYS", "department": "physics", "views": [],
        "tables": [{ "id": "apps", "name": "Applications", "slug": "apps", "fields": [{ "name": "name", "label": "name", "field_type": "Text", "required": false, "ferpa_sensitive": false }] }]
    });
    let (s, b) = rest(&app, "POST", &format!("/api/v1/workspaces/{WORKSPACE}/apps"), OWNER, Some(manifest)).await;
    assert_eq!(s, StatusCode::CREATED, "{b}");
    (db, app, slug)
}

#[tokio::test]
async fn the_combined_tools_are_listed_and_the_ones_they_replace_are_not() {
    let (_db, app, _) = world().await;
    let payload = json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/list" });
    let (_, listed) = rest(&app, "POST", "/api/mcp", OWNER, Some(payload)).await;
    let names: Vec<&str> = listed["result"]["tools"].as_array().unwrap().iter().filter_map(|t| t["name"].as_str()).collect();
    for gone in ["list_pages", "get_record_history", "verify_decision_ledger", "export_oscal_compliance", "get_framework_spec"] {
        assert!(!names.contains(&gone), "{gone} was combined into another tool: {names:?}");
    }
    for kept in ["get_page", "get_record", "governance_report"] {
        assert!(names.contains(&kept), "{kept}: {names:?}");
    }
}

#[tokio::test]
async fn get_page_without_a_page_lists_the_pages() {
    let (_db, app, slug) = world().await;
    let r = call(&app, "get_page", json!({ "app_slug": slug })).await;
    assert!(r["isError"].is_null(), "{r}");
    assert!(r["structuredContent"]["pages"].is_array(), "the list the old list_pages gave: {r}");
    let r = call(&app, "get_page", json!({ "app_slug": slug, "page_id": "nope" })).await;
    assert_eq!(r["isError"], true, "a page that is not there is still an error: {r}");
}

#[tokio::test]
async fn get_record_returns_the_history_when_it_is_asked_for_it() {
    let (_db, app, slug) = world().await;
    let made = call(&app, "create_record", json!({ "app_slug": slug, "data": { "_table_id": "apps", "name": "First" } })).await;
    let id = made["structuredContent"]["id"].as_str().or_else(|| made["structuredContent"]["record"]["id"].as_str()).unwrap_or_else(|| panic!("{made}")).to_string();
    let record = call(&app, "get_record", json!({ "app_slug": slug, "record_id": id })).await;
    assert!(record["structuredContent"]["version"].is_number(), "the record itself: {record}");
    assert!(record["structuredContent"]["entries"].is_null(), "no history unless asked: {record}");
    let history = call(&app, "get_record", json!({ "app_slug": slug, "record_id": id, "history": true, "limit": 10 })).await;
    assert!(history["structuredContent"]["entries"].is_array(), "{history}");
    // Paging arguments mean nothing without the history.
    let r = call(&app, "get_record", json!({ "app_slug": slug, "record_id": id, "limit": 10 })).await;
    assert_eq!(r["isError"], true, "{r}");
}

#[tokio::test]
async fn governance_report_holds_the_three_reports_and_refuses_a_fourth() {
    let (_db, app, _) = world().await;
    let ledger = call(&app, "governance_report", json!({ "report": "ledger" })).await;
    assert!(ledger["isError"].is_null(), "{ledger}");
    let oscal = call(&app, "governance_report", json!({ "report": "oscal" })).await;
    assert!(oscal["isError"].is_null(), "{oscal}");
    assert!(oscal["structuredContent"].to_string().contains("component-definition"), "{oscal}");
    let framework = call(&app, "governance_report", json!({ "report": "framework" })).await;
    assert!(framework["isError"].is_null(), "{framework}");
    assert!(framework["structuredContent"].to_string().contains("component_catalog"), "the catalog");
    let r = call(&app, "governance_report", json!({ "report": "gossip" })).await;
    assert_eq!(r["isError"], true, "{r}");
    let r = call(&app, "governance_report", json!({})).await;
    assert_eq!(r["isError"], true, "the report is named: {r}");
}
