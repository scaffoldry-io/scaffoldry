//! Agent coverage phase 2: the platform teaches the agent. A guide resource says how to build an app, and four prompts say
//! which tools to call in what order. The guide's code blocks are run as scenarios in `agent_scenarios.rs`.

mod common;

use axum::body::Body;
use axum::http::Request;
use http_body_util::BodyExt;
use scaffoldry_server::build_app_with_state;
use scaffoldry_server::service::identity::issue_test_token_and_user;
use scaffoldry_server::service::tools::TOOLS;
use scaffoldry_server::state::ServerState;
use serde_json::{json, Value};
use std::sync::Arc;
use tower::ServiceExt;

const GUIDE: &str = "scaffoldry://guide/building-apps";

async fn rpc(app: &axum::Router, method: &str, params: Value) -> Value {
    let token = issue_test_token_and_user("jordan.lee@state.edu");
    let req = Request::builder()
        .method("POST")
        .uri("/api/v1/mcp")
        .header("content-type", "application/json")
        .header("authorization", format!("Bearer {token}"))
        .body(Body::from(json!({ "jsonrpc": "2.0", "id": 1, "method": method, "params": params }).to_string()))
        .unwrap();
    let bytes = app.clone().oneshot(req).await.unwrap().into_body().collect().await.unwrap().to_bytes();
    serde_json::from_slice::<Value>(&bytes).unwrap_or(Value::Null)["result"].clone()
}

fn app() -> axum::Router {
    build_app_with_state(Arc::new(ServerState::new().expect("state"))).expect("router")
}

#[tokio::test]
async fn the_guide_is_a_listed_resource_and_reads_back_within_twelve_kilobytes() {
    let _db = common::TestDb::new();
    let app = app();
    let listed = rpc(&app, "resources/list", json!({})).await;
    let entry = listed["resources"].as_array().unwrap().iter().find(|r| r["uri"] == GUIDE).expect("the guide is listed");
    assert_eq!(entry["mimeType"], "text/markdown");
    let read = rpc(&app, "resources/read", json!({ "uri": GUIDE })).await;
    let text = read["contents"][0]["text"].as_str().expect("the guide has text");
    assert!(text.len() <= 12_000, "the guide is {} bytes, the limit is 12,000", text.len());
    for topic in ["describe", "propose", "approval", "second person", "cursor", "version", "job", "never"] {
        assert!(text.to_lowercase().contains(topic), "the guide covers '{topic}'");
    }
    // Everything the guide tells an agent to call exists.
    let called: Vec<String> = text.split('`').skip(1).step_by(2).filter(|s| s.chars().all(|c| c.is_ascii_lowercase() || c == '_') && s.contains('_')).map(str::to_string).collect();
    let names: Vec<&str> = TOOLS.iter().map(|t| t.name).collect();
    let unknown: Vec<&String> = called.iter().filter(|c| !names.contains(&c.as_str()) && !["next_cursor", "job_id", "ferpa_sensitive", "field_type", "ref_id", "record_id", "app_slug", "table_id", "version_conflict"].contains(&c.as_str())).collect();
    assert!(unknown.is_empty(), "the guide names things that are not tools: {unknown:?}");
}

#[tokio::test]
async fn four_prompts_list_and_resolve_to_a_plan_that_names_real_tools_in_order() {
    let _db = common::TestDb::new();
    let app = app();
    let listed = rpc(&app, "prompts/list", json!({})).await;
    let names: Vec<&str> = listed["prompts"].as_array().unwrap().iter().filter_map(|p| p["name"].as_str()).collect();
    for p in ["build_app_from_description", "import_spreadsheet", "design_approval_process", "explain_denial"] {
        assert!(names.contains(&p), "{p} is listed: {names:?}");
    }
    let tool_names: Vec<&str> = TOOLS.iter().map(|t| t.name).collect();
    for (prompt, args, first, then) in [
        ("build_app_from_description", json!({ "description": "Track scholarship applications" }), "list_workspaces", "propose_app_change"),
        ("import_spreadsheet", json!({ "app_slug": "scholarships", "columns": "name, amount" }), "describe_app", "create_record"),
        ("design_approval_process", json!({ "app_slug": "scholarships", "description": "The chair approves awards over 5000" }), "describe_app", "propose_app_change"),
        ("explain_denial", json!({ "error": "403 Forbidden: Cedar Policy restricts access to this app" }), "list_workspaces", "get_workspace"),
    ] {
        let got = rpc(&app, "prompts/get", json!({ "name": prompt, "arguments": args })).await;
        let text = got["messages"][0]["content"]["text"].as_str().unwrap_or_else(|| panic!("{prompt} resolves: {got}"));
        assert!(text.len() < 2_500, "{prompt} is a short template: {} bytes", text.len());
        let a = text.find(first).unwrap_or_else(|| panic!("{prompt} names {first}"));
        let b = text.find(then).unwrap_or_else(|| panic!("{prompt} names {then}"));
        assert!(a < b, "{prompt} calls {first} before {then}");
        // Every tool-looking word it names is a real tool.
        for word in text.split(|c: char| !(c.is_ascii_lowercase() || c == '_')).filter(|w| w.contains('_') && w.len() > 6) {
            if ["scaffoldry"].contains(&word) {
                continue;
            }
            let looks_like_tool = ["list_", "get_", "describe_", "propose_", "decide_", "create_", "update_", "delete_", "save_", "export_", "simulate_", "cancel_"].iter().any(|p| word.starts_with(p));
            assert!(!looks_like_tool || tool_names.contains(&word), "{prompt} names {word}, which is not a tool");
        }
    }
    // A prompt that is not listed resolves to nothing, and the two that already existed still work.
    let none = rpc(&app, "prompts/get", json!({ "name": "no_such_prompt" })).await;
    assert_eq!(none["messages"], json!([]));
    let old = rpc(&app, "prompts/get", json!({ "name": "governed_app_builder", "arguments": { "app_purpose": "x", "department": "y" } })).await;
    assert!(old["messages"][0]["content"]["text"].is_string());
}
