//! Agent coverage phase 2: every tool error has a `code`, a `message`, and a `hint`: one sentence that tells an agent
//! what to do next and names a tool that exists.

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use common::TestDb;
use http_body_util::BodyExt;
use scaffoldry_server::build_app_with_state;
use scaffoldry_server::service::identity::issue_test_token_and_user;
use scaffoldry_server::service::tools::TOOLS;
use serde_json::{json, Value};
use std::collections::BTreeSet;
use tower::ServiceExt;
use uuid::Uuid;

const WORKSPACE: &str = "ws-physics-optics";
const OWNER: &str = "einstein@physics.state.edu";
const OUTSIDER: &str = "marcus.vance@state.edu";
const ADMIN: &str = "jordan.lee@state.edu";

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

/// The whole MCP result of a tool call.
async fn call(app: &axum::Router, who: &str, tool: &str, args: Value) -> Value {
    let payload = json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": { "name": tool, "arguments": args } });
    send(app, "POST", "/api/v1/mcp", who, Some(payload)).await.1["result"].clone()
}

/// An app with a required text field, a number, a select, and a sensitive field, and one record.
async fn fixture(app: &axum::Router) -> (String, String) {
    let slug = format!("hints-{}", &Uuid::new_v4().to_string()[..8]);
    let manifest = json!({
        "slug": slug, "title": "Hints", "description": "d", "organization_code": "PHYS", "department": "physics", "views": [],
        "tables": [{ "id": "people", "name": "People", "slug": "people", "fields": [
            { "name": "name", "label": "Name", "field_type": "Text", "required": true, "ferpa_sensitive": false },
            { "name": "score", "label": "Score", "field_type": "Number", "required": false, "ferpa_sensitive": false },
            { "name": "status", "label": "Status", "field_type": "Select", "required": false, "ferpa_sensitive": false,
              "select_options": [{ "id": "open", "label": "Open", "color": "info" }, { "id": "closed", "label": "Closed", "color": "neutral" }] },
            { "name": "ssn", "label": "SSN", "field_type": "Text", "required": false, "ferpa_sensitive": false }
        ] }]
    });
    let (s, body) = send(app, "POST", &format!("/api/v1/workspaces/{WORKSPACE}/apps"), OWNER, Some(manifest)).await;
    assert_eq!(s, StatusCode::CREATED, "{body}");
    let r = call(app, OWNER, "create_record", json!({ "app_slug": slug, "data": { "_table_id": "people", "name": "Ada", "score": 3 } })).await;
    let id = r["structuredContent"]["id"].as_str().unwrap_or_default().to_string();
    assert!(!id.is_empty(), "the fixture record: {r}");
    (slug, id)
}

fn tool_names_in(hint: &str) -> Vec<&'static str> {
    TOOLS.iter().map(|t| t.name).filter(|n| hint.contains(n)).collect()
}

#[tokio::test]
async fn twenty_five_common_mistakes_each_get_a_code_and_a_hint_that_names_a_real_tool() {
    let db = TestDb::new();
    let state = db.state();
    state.seed_demo().expect("seed");
    let app = build_app_with_state(state).expect("router");
    let (slug, rec) = fixture(&app).await;
    let missing = Uuid::new_v4().to_string();
    // A real proposal, so a bad decision on it is the mistake and not a missing id.
    let draft = json!({ "slug": format!("draft-{}", &Uuid::new_v4().to_string()[..8]), "title": "Draft", "description": "d", "organization_code": "PHYS", "department": "physics", "views": [],
        "tables": [{ "id": "t", "name": "T", "slug": "t", "fields": [{ "name": "a", "label": "A", "field_type": "Text", "required": false, "ferpa_sensitive": false }] }] });
    let proposed = call(&app, OWNER, "propose_app_change", json!({ "workspace_id": WORKSPACE, "manifest": draft, "summary": "A draft" })).await;
    let real_proposal = proposed["structuredContent"]["proposal_id"].as_str().unwrap_or(&missing).to_string();

    // (what went wrong, who, tool, arguments, the code an agent branches on)
    let cases: Vec<(&str, &str, &str, Value, &str)> = vec![
        ("a tool that does not exist", OWNER, "no_such_tool", json!({}), "not_found"),
        ("an argument the tool does not take", OWNER, "list_records", json!({ "app_slug": slug, "table_id": "people", "colour": "red" }), "bad_request"),
        ("a required argument left out", OWNER, "get_record", json!({ "app_slug": slug }), "bad_request"),
        ("an argument of the wrong type", OWNER, "list_records", json!({ "app_slug": slug, "table_id": "people", "limit": "ten" }), "bad_request"),
        ("an app that does not exist", OWNER, "describe_app", json!({ "app_slug": "no-such-app" }), "not_found"),
        ("a workspace that does not exist", OWNER, "get_workspace", json!({ "workspace_id": "ws-nope" }), "not_found"),
        ("a table that does not exist", OWNER, "list_records", json!({ "app_slug": slug, "table_id": "nope" }), "not_found"),
        ("a record that does not exist", OWNER, "get_record", json!({ "app_slug": slug, "record_id": missing }), "not_found"),
        ("a sort on a field that does not exist", OWNER, "list_records", json!({ "app_slug": slug, "table_id": "people", "sort": { "field_name": "nope", "direction": "Asc" } }), "bad_request"),
        ("a required field left out", OWNER, "create_record", json!({ "app_slug": slug, "data": { "_table_id": "people", "score": 1 } }), "constraint_violated"),
        ("a select value that is not an option id", OWNER, "create_record", json!({ "app_slug": slug, "data": { "_table_id": "people", "name": "Bo", "status": "Open" } }), "bad_request"),
        ("a total by a function that does not exist", OWNER, "aggregate_records", json!({ "app_slug": slug, "table_id": "people", "function": "median", "field": "score" }), "bad_request"),
        ("an update with a stale version", OWNER, "update_record", json!({ "app_slug": slug, "record_id": rec, "version": 99, "data": { "score": 4 } }), "version_conflict"),
        ("a delete with a stale version", OWNER, "delete_record", json!({ "app_slug": slug, "record_id": rec, "version": 99 }), "version_conflict"),
        ("an update to a record that does not exist", OWNER, "update_record", json!({ "app_slug": slug, "record_id": missing, "version": 1, "data": { "score": 4 } }), "not_found"),
        ("a write to an app the person is not in", OUTSIDER, "create_record", json!({ "app_slug": slug, "data": { "_table_id": "people", "name": "Bo" } }), "forbidden"),
        ("a workspace the person is not in", OUTSIDER, "update_workspace", json!({ "workspace_id": WORKSPACE, "name": "Mine" }), "forbidden"),
        ("a proposal that does not exist", OWNER, "decide_proposal", json!({ "proposal_id": missing, "decision": "approve", "rationale": "ok" }), "not_found"),
        ("a decision that is not approve or reject", OWNER, "decide_proposal", json!({ "proposal_id": real_proposal, "decision": "maybe", "rationale": "ok" }), "bad_request"),
        ("an app manifest that is not valid", OWNER, "propose_app_change", json!({ "workspace_id": WORKSPACE, "summary": "Bad", "manifest": { "slug": "X", "title": "Bad" } }), "bad_request"),
        ("a change with no manifest and no app", OWNER, "propose_app_change", json!({ "workspace_id": WORKSPACE, "summary": "Nothing" }), "bad_request"),
        ("a job that does not exist", OWNER, "get_job", json!({ "job_id": missing }), "not_found"),
        ("a filter operator that does not exist", OWNER, "list_records", json!({ "app_slug": slug, "table_id": "people", "filter": { "conjunction": "And", "clauses": [{ "id": "1", "field_name": "name", "operator": "Sounds", "value": "A" }] } }), "bad_request"),
        ("a sum over a field that is not a number", OWNER, "aggregate_records", json!({ "app_slug": slug, "table_id": "people", "function": "sum", "field": "name" }), "bad_request"),
        ("a view that is missing its kind", OWNER, "save_view", json!({ "app_slug": slug, "name": "No kind", "table_id": "people" }), "bad_request"),
        ("a view that does not exist", OWNER, "delete_view", json!({ "app_slug": slug, "view_id": missing }), "not_found"),
        ("an admin-only tool for a faculty member", OWNER, "simulate_cedar_policy", json!({ "eppn": OWNER, "action": "read" }), "forbidden"),
        ("a view that has no data to read", OWNER, "get_view_data", json!({ "app_slug": slug, "view_id": missing }), "not_found"),
        ("a process step that does not exist", OWNER, "decide_process", json!({ "app_slug": slug, "instance_id": missing, "decision": "approve" }), "not_found"),
        ("a member action that does not exist", OWNER, "manage_workspace_member", json!({ "workspace_id": WORKSPACE, "action": "promote", "eppn": "x@y.edu" }), "bad_request"),
    ];
    assert!(cases.len() >= 25);

    let mut problems = Vec::new();
    let mut distinct = BTreeSet::new();
    for (what, who, tool, args, code) in &cases {
        let r = call(&app, who, tool, args.clone()).await;
        let s = &r["structuredContent"];
        if r["isError"] != json!(true) {
            problems.push(format!("{what}: not an error: {r}"));
            continue;
        }
        if s["code"] != json!(code) {
            problems.push(format!("{what}: expected code {code}, got {} ({})", s["code"], s["message"]));
        }
        let hint = s["hint"].as_str().unwrap_or("");
        if hint.is_empty() {
            problems.push(format!("{what}: no hint ({})", s["message"]));
            continue;
        }
        if tool_names_in(hint).is_empty() {
            problems.push(format!("{what}: the hint names no real tool: {hint}"));
        }
        if !hint.ends_with('.') || hint.contains('\n') || hint.chars().count() > 200 {
            problems.push(format!("{what}: the hint is not one short sentence: {hint}"));
        }
        // A client that shows only the content sees the message, then the hint.
        let shown: Vec<&str> = r["content"].as_array().into_iter().flatten().filter_map(|c| c["text"].as_str()).collect();
        if shown.len() != 2 || shown[0] != s["message"].as_str().unwrap_or("") || shown[1] != hint {
            problems.push(format!("{what}: the hint is in the structured part only, so a client that shows text misses it"));
        }
        distinct.insert(hint.to_string());
    }
    assert!(problems.is_empty(), "{} of {} mistakes:\n{}", problems.len(), cases.len(), problems.join("\n"));
    assert!(distinct.len() >= 8, "hints should fit the mistake, not repeat one sentence: only {} distinct", distinct.len());
    let _ = ADMIN;
}

#[tokio::test]
async fn a_hint_never_repeats_a_record_value_or_a_secret() {
    let db = TestDb::new();
    let state = db.state();
    state.seed_demo().expect("seed");
    let app = build_app_with_state(state).expect("router");
    let (slug, _) = fixture(&app).await;
    let r = call(&app, OWNER, "create_record", json!({ "app_slug": slug, "data": { "_table_id": "people", "name": "Bo", "status": "123-45-6789" } })).await;
    assert_eq!(r["isError"], json!(true), "{r}");
    let hint = r["structuredContent"]["hint"].as_str().unwrap_or("");
    assert!(!hint.is_empty() && !hint.contains("123-45-6789"), "{hint}");
}
