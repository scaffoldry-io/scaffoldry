//! The MCP endpoint's protocol behaviour, the parts every client leans on and a tool test would never touch.

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use scaffoldry_server::build_app_with_state;
use scaffoldry_server::service::identity::issue_test_token_and_user;
use scaffoldry_server::state::ServerState;
use serde_json::{json, Value};
use std::sync::Arc;
use tower::ServiceExt;

async fn post(body: &str, token: Option<&str>) -> (StatusCode, Value) {
    let state = Arc::new(ServerState::new().expect("state"));
    let app = build_app_with_state(state).expect("router");
    let mut b = Request::builder().method("POST").uri("/api/v1/mcp").header("content-type", "application/json");
    if let Some(t) = token {
        b = b.header("authorization", format!("Bearer {t}"));
    }
    let resp = app.oneshot(b.body(Body::from(body.to_string())).unwrap()).await.unwrap();
    let status = resp.status();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
}

// The MCP spec: a ping is answered with an empty result.
#[tokio::test]
async fn ping_is_answered_with_an_empty_result() {
    let _db = common::TestDb::new();
    let token = issue_test_token_and_user("jordan.lee@state.edu");
    let (status, body) = post(&json!({ "jsonrpc": "2.0", "id": 7, "method": "ping" }).to_string(), Some(&token)).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["id"], 7);
    assert_eq!(body["result"], json!({}), "{body}");
    assert!(body.get("error").is_none(), "{body}");
}

// JSON-RPC: a body that is not JSON is a parse error with code -32700 and a null id, not a plain-text 400.
#[tokio::test]
async fn malformed_json_is_a_json_rpc_parse_error() {
    let _db = common::TestDb::new();
    let token = issue_test_token_and_user("jordan.lee@state.edu");
    let (_, body) = post("{not json", Some(&token)).await;
    assert_eq!(body["jsonrpc"], "2.0", "{body}");
    assert_eq!(body["error"]["code"], -32700, "{body}");
    assert!(body["id"].is_null(), "{body}");
}

// A body that is JSON but not a request is -32600.
#[tokio::test]
async fn json_that_is_not_a_request_is_an_invalid_request() {
    let _db = common::TestDb::new();
    let token = issue_test_token_and_user("jordan.lee@state.edu");
    let (_, body) = post(&json!({ "hello": "world" }).to_string(), Some(&token)).await;
    assert_eq!(body["error"]["code"], -32600, "{body}");
}

// Without a token nothing is answered, and the status says so.
#[tokio::test]
async fn a_call_with_no_token_is_refused() {
    let _db = common::TestDb::new();
    let (status, _) = post(&json!({ "jsonrpc": "2.0", "id": 1, "method": "ping" }).to_string(), None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

// The MCP spec: structuredContent is a JSON object. A tool whose result is a list must not return the bare list,
// because a client validates it and throws the result away. The text content still carries the plain result.
#[tokio::test]
async fn a_list_result_comes_back_as_an_object_and_the_text_keeps_the_list() {
    let _db = common::TestDb::new();
    let token = issue_test_token_and_user("jordan.lee@state.edu");
    let (status, body) = post(
        &json!({ "jsonrpc": "2.0", "id": 3, "method": "tools/call", "params": { "name": "list_workspaces", "arguments": {} } }).to_string(),
        Some(&token),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let result = &body["result"];
    assert!(result["isError"].is_null() || result["isError"] == false, "{body}");
    assert!(result["structuredContent"].is_object(), "structuredContent must be an object: {}", result["structuredContent"]);
    assert!(result["structuredContent"]["items"].is_array(), "a list goes under items: {}", result["structuredContent"]);
    let text: Value = serde_json::from_str(result["content"][0]["text"].as_str().unwrap()).unwrap();
    assert!(text.is_array(), "the text is the plain list: {text}");
    assert_eq!(text.as_array().unwrap().len(), result["structuredContent"]["items"].as_array().unwrap().len());
}
