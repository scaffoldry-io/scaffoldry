//! Tests for MCP as the authoritative API and MCP Apps resources.
//! Each capability tested via MCP JSON-RPC 2.0 with allowed and denied cases per role.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::Router;
use http_body_util::BodyExt;
use scaffoldry_server::build_app;
use serde_json::{json, Value};
use tower::ServiceExt;

async fn login_user(app: &Router, eppn: &str) -> String {
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/auth/token")
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&json!({ "eppn": eppn })).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let body: Value = serde_json::from_slice(&bytes).unwrap();
    body["token"].as_str().unwrap().to_string()
}

async fn mcp_call(
    app: &Router,
    token: &str,
    method: &str,
    params: Option<Value>,
) -> (StatusCode, Value) {
    let payload = json!({
        "jsonrpc": "2.0",
        "id": "test-call",
        "method": method,
        "params": params.unwrap_or(json!({}))
    });
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/mcp")
                .header("authorization", format!("Bearer {token}"))
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&payload).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = resp.status();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let val: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    (status, val)
}

#[tokio::test]
async fn test_mcp_workspace_tools_allow_and_deny_by_role() {
    let app = build_app().expect("Failed to build router");

    // Sarah Connor is member/owner of ws-cs-research, but not ws-bio-lab
    let sarah_token = login_user(&app, "sarah.connor@state.edu").await;
    // Marcus Vance is viewer in ws-bio-lab
    let marcus_token = login_user(&app, "marcus.vance@state.edu").await;
    // Marie Curie is owner in ws-bio-lab
    let curie_token = login_user(&app, "prof.curie@science.state.edu").await;

    // 1. list_workspaces: Sarah only sees her workspaces
    let (status, res) = mcp_call(
        &app,
        &sarah_token,
        "tools/call",
        Some(json!({
            "name": "list_workspaces",
            "arguments": {}
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let content = &res["result"]["content"][0]["text"];
    let workspaces: Value = serde_json::from_str(content.as_str().unwrap()).unwrap();
    let ids: Vec<&str> = workspaces.as_array().unwrap().iter().map(|w| w["id"].as_str().unwrap()).collect();
    assert!(ids.contains(&"ws-cs-research"));
    assert!(!ids.contains(&"ws-bio-lab"));

    // 2. get_workspace: Sarah denied on ws-bio-lab
    let (status, res) = mcp_call(
        &app,
        &sarah_token,
        "tools/call",
        Some(json!({
            "name": "get_workspace",
            "arguments": { "workspace_id": "ws-bio-lab" }
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(res["result"]["isError"].as_bool().unwrap_or(false) || !res["error"].is_null(),
        "Sarah must be denied access to ws-bio-lab: {res:?}");

    // 3. get_workspace: Curie allowed on ws-bio-lab
    let (status, res) = mcp_call(
        &app,
        &curie_token,
        "tools/call",
        Some(json!({
            "name": "get_workspace",
            "arguments": { "workspace_id": "ws-bio-lab" }
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(res["result"]["isError"], Value::Null);
    let content = &res["result"]["content"][0]["text"];
    assert!(content.as_str().unwrap().contains("ws-bio-lab"));

    // 4. update_workspace: Marcus (viewer) is denied manage_workspace
    let (status, res) = mcp_call(
        &app,
        &marcus_token,
        "tools/call",
        Some(json!({
            "name": "update_workspace",
            "arguments": {
                "workspace_id": "ws-bio-lab",
                "description": "Unauthorized modification"
            }
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(res["result"]["isError"].as_bool().unwrap_or(false) || !res["error"].is_null(),
        "Marcus (viewer) must be denied updating ws-bio-lab");

    // 5. update_workspace: Curie (owner) is allowed
    let (status, res) = mcp_call(
        &app,
        &curie_token,
        "tools/call",
        Some(json!({
            "name": "update_workspace",
            "arguments": {
                "workspace_id": "ws-bio-lab",
                "description": "Authorized owner modification"
            }
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(res["result"]["isError"], Value::Null);

    // 6. manage_workspace_member: Marcus (viewer) denied adding a collaborator
    let (status, res) = mcp_call(
        &app,
        &marcus_token,
        "tools/call",
        Some(json!({
            "name": "manage_workspace_member",
            "arguments": {
                "workspace_id": "ws-bio-lab",
                "action": "add",
                "eppn": "intruder@state.edu",
                "role": "Viewer"
            }
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(res["result"]["isError"].as_bool().unwrap_or(false) || !res["error"].is_null());

    // 7. manage_workspace_member: Curie (owner) allowed adding a collaborator
    let (status, res) = mcp_call(
        &app,
        &curie_token,
        "tools/call",
        Some(json!({
            "name": "manage_workspace_member",
            "arguments": {
                "workspace_id": "ws-bio-lab",
                "action": "add",
                "eppn": "new.postdoc@science.state.edu",
                "role": "Editor",
                "name": "New Postdoc"
            }
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(res["result"]["isError"], Value::Null);
}

#[tokio::test]
async fn test_mcp_record_tools_allow_and_deny_by_department() {
    let app = build_app().expect("Failed to build router");

    let einstein_token = login_user(&app, "einstein@physics.state.edu").await;
    let curie_token = login_user(&app, "prof.curie@science.state.edu").await;

    // First create a physics app as admin
    let admin_token = login_user(&app, "jordan.lee@state.edu").await;
    let (status, _ws) = mcp_call(
        &app,
        &admin_token,
        "tools/call",
        Some(json!({
            "name": "create_app_proposal",
            "arguments": {
                "title": "Quantum Research Grants",
                "department": "physics",
                "organization_code": "PHYS",
                "fields": [
                    { "name": "title", "label": "Title", "type": "Text" }
                ]
            }
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    // Curie (biology faculty) denied write on physics app
    let (status, res) = mcp_call(
        &app,
        &curie_token,
        "tools/call",
        Some(json!({
            "name": "create_record",
            "arguments": {
                "app_slug": "quantum-research-grants",
                "data": { "title": "Cross-dept submission" }
            }
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(res["result"]["isError"].as_bool().unwrap_or(false) || !res["error"].is_null(),
        "Curie from biology must be denied creating records in physics app");

    // Einstein (physics) allowed write on physics app
    let (status, res) = mcp_call(
        &app,
        &einstein_token,
        "tools/call",
        Some(json!({
            "name": "create_record",
            "arguments": {
                "app_slug": "quantum-research-grants",
                "data": { "title": "Valid Physics Proposal" }
            }
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(res["result"]["isError"], Value::Null);
}

#[tokio::test]
async fn test_mcp_apps_ui_resources() {
    let app = build_app().expect("Failed to build router");
    let admin_token = login_user(&app, "jordan.lee@state.edu").await;

    // 1. Read UI resource for workspace security settings
    let (status, res) = mcp_call(
        &app,
        &admin_token,
        "resources/read",
        Some(json!({
            "uri": "ui://workspaces/ws-bio-lab/settings"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(res["result"]["isError"], Value::Null);
    let contents = &res["result"]["contents"][0];
    assert_eq!(contents["mimeType"], "text/html;profile=mcp-app");
    assert!(contents["text"].as_str().unwrap().contains("Workspace Security"));

    // 2. Read UI resource for decision ledger view
    let (status, res) = mcp_call(
        &app,
        &admin_token,
        "resources/read",
        Some(json!({
            "uri": "ui://governance/decision-ledger"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(res["result"]["isError"], Value::Null);
    let contents = &res["result"]["contents"][0];
    assert_eq!(contents["mimeType"], "text/html;profile=mcp-app");
    assert!(contents["text"].as_str().unwrap().contains("Decision Ledger"));
}
