//! Tests for MCP as the authoritative API and MCP Apps resources.
//! Each capability tested via MCP JSON-RPC 2.0 with allowed and denied cases per role.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::Router;
use http_body_util::BodyExt;
use serde_json::{json, Value};
use tower::ServiceExt;

async fn reset_test_db() {
    tokio::task::spawn_blocking(|| {
        let db_url = std::env::var("DATABASE_URL").unwrap_or_else(|_| {
            "postgres://scaffoldry:scaffoldry_dev_password@127.0.0.1:5433/scaffoldry".to_string()
        });
        if let Ok(mut client) = postgres::Client::connect(&db_url, postgres::NoTls) {
            let _ = client.batch_execute("
                TRUNCATE TABLE organizations CASCADE;
                TRUNCATE TABLE workspaces CASCADE;
                TRUNCATE TABLE app_manifests CASCADE;
                TRUNCATE TABLE dataset_records CASCADE;
                TRUNCATE TABLE published_datasets CASCADE;
                TRUNCATE TABLE dataset_relationships CASCADE;
                TRUNCATE TABLE workflow_automations CASCADE;
                TRUNCATE TABLE process_instances CASCADE;
                TRUNCATE TABLE scim_users CASCADE;
                TRUNCATE TABLE scim_groups CASCADE;
                TRUNCATE TABLE roles CASCADE;
                TRUNCATE TABLE workspace_collaborators CASCADE;
            ");
        }
    }).await.unwrap();
}

async fn login_user(_app: &Router, eppn: &str) -> String {
    scaffoldry_server::service::identity::issue_test_token_and_user(eppn)
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
    reset_test_db().await;
    let state = std::sync::Arc::new(scaffoldry_server::state::ServerState::new().expect("state"));
    state.seed_demo().expect("seed");
    let app = scaffoldry_server::build_app_with_state(state).expect("Failed to build router");

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
    reset_test_db().await;
    let state = std::sync::Arc::new(scaffoldry_server::state::ServerState::new().expect("state"));
    state.seed_demo().expect("seed");
    let state_for_setup = state.clone();
    let app = scaffoldry_server::build_app_with_state(state).expect("Failed to build router");

    let einstein_token = login_user(&app, "einstein@physics.state.edu").await;
    let curie_token = login_user(&app, "prof.curie@science.state.edu").await;

    // A physics app in the physics workspace, registered directly. Agents propose apps in phase 4.
    {
        let manifest = scaffoldry_engine::AppManifest {
            slug: "quantum-research-grants".to_string(),
            title: "Quantum Research Grants".to_string(),
            description: String::new(),
            organization_code: "PHYS".to_string(),
            department: "physics".to_string(),
            workspace_id: Some("ws-physics-optics".to_string()),
            herm_capability_id: None,
            custom_domain: None,
            custom_domain_verified: false,
            tables: vec![],
            relationships: vec![],
            views: vec![],
            ceds_mappings: std::collections::HashMap::new(),
        };
        let mut engine = state_for_setup.engine.write().unwrap();
        engine.register_manifest(manifest).expect("register app");
    }

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
    let state = std::sync::Arc::new(scaffoldry_server::state::ServerState::new().expect("state"));
    state.seed_demo().expect("seed");
    let app = scaffoldry_server::build_app_with_state(state).expect("Failed to build router");
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


// ---------------------------------------------------------------------------------------------
// Phase 1: a tool registry with one gate
// ---------------------------------------------------------------------------------------------

use scaffoldry_server::service::tools::{Scope, TOOLS};

/// A valid-looking argument for each required property of a tool, so a call fails only for the
/// reason under test.
fn dummy_args(tool: &scaffoldry_server::service::tools::Tool, target: (&str, &str)) -> Value {
    let schema = (tool.input_schema)();
    let mut args = serde_json::Map::new();
    if let Some(required) = schema["required"].as_array() {
        for name in required.iter().filter_map(|v| v.as_str()) {
            let kind = schema["properties"][name]["type"].as_str().unwrap_or("string");
            let value = match kind {
                "object" => json!({}),
                "array" => json!([]),
                "boolean" => json!(false),
                "integer" | "number" => json!(1),
                _ => json!("x"),
            };
            args.insert(name.to_string(), value);
        }
    }
    args.insert(target.0.to_string(), json!(target.1));
    Value::Object(args)
}

async fn seeded_app() -> Router {
    reset_test_db().await;
    let state = std::sync::Arc::new(scaffoldry_server::state::ServerState::new().expect("state"));
    state.seed_demo().expect("seed");
    scaffoldry_server::build_app_with_state(state).expect("router")
}

#[tokio::test]
async fn tools_list_returns_exactly_the_registry_with_read_only_hints() {
    let app = seeded_app().await;
    let token = login_user(&app, "jordan.lee@state.edu").await;
    let (status, res) = mcp_call(&app, &token, "tools/list", None).await;
    assert_eq!(status, StatusCode::OK);
    let listed = res["result"]["tools"].as_array().unwrap();

    let mut listed_names: Vec<&str> = listed.iter().map(|t| t["name"].as_str().unwrap()).collect();
    let mut registry_names: Vec<&str> = TOOLS.iter().map(|t| t.name).collect();
    listed_names.sort();
    registry_names.sort();
    assert_eq!(listed_names, registry_names);

    for tool in TOOLS {
        let entry = listed.iter().find(|t| t["name"] == tool.name).unwrap();
        assert_eq!(entry["annotations"]["readOnlyHint"], tool.read_only, "{}", tool.name);
        assert_eq!(entry["inputSchema"], (tool.input_schema)(), "{}", tool.name);
    }

    // The fake tools are gone.
    for gone in ["query_dataset", "create_app_proposal", "calculate_formula", "get_governance_posture"] {
        assert!(!listed_names.contains(&gone), "{gone} must be removed");
    }
}

#[tokio::test]
async fn every_app_and_workspace_tool_refuses_a_caller_with_no_role() {
    let app = seeded_app().await;
    // Sarah Connor is computer science faculty. She has no role in the bio lab or the physics lab.
    let sarah = login_user(&app, "sarah.connor@state.edu").await;

    let mut checked = 0;
    for tool in TOOLS {
        let target = match tool.scope {
            Scope::App(_) => ("app_slug", "physics-admissions-review"),
            Scope::Workspace(_) => ("workspace_id", "ws-bio-lab"),
            _ => continue,
        };
        checked += 1;
        let (status, res) = mcp_call(
            &app,
            &sarah,
            "tools/call",
            Some(json!({ "name": tool.name, "arguments": dummy_args(tool, target) })),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{}", tool.name);
        assert_eq!(
            res["result"]["isError"], true,
            "{} must refuse a caller with no role: {res}",
            tool.name
        );
    }
    assert!(checked >= 4, "the registry has app and workspace tools to check");
}

#[tokio::test]
async fn a_scoped_tool_without_its_target_is_a_tool_error_that_names_it() {
    let app = seeded_app().await;
    let token = login_user(&app, "jordan.lee@state.edu").await;
    let (_, res) = mcp_call(
        &app,
        &token,
        "tools/call",
        Some(json!({ "name": "create_record", "arguments": { "data": {} } })),
    )
    .await;
    assert_eq!(res["result"]["isError"], true);
    assert!(res["result"]["content"][0]["text"].as_str().unwrap().contains("app_slug"));
}

#[tokio::test]
async fn an_unknown_argument_is_a_tool_error_that_names_it() {
    let app = seeded_app().await;
    let token = login_user(&app, "jordan.lee@state.edu").await;
    let (status, res) = mcp_call(
        &app,
        &token,
        "tools/call",
        Some(json!({ "name": "list_datasets", "arguments": { "departmnt": "Biology" } })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(res["result"]["isError"], true);
    assert!(res["result"]["content"][0]["text"].as_str().unwrap().contains("departmnt"));
}

#[tokio::test]
async fn an_unknown_tool_is_a_tool_error() {
    let app = seeded_app().await;
    let token = login_user(&app, "jordan.lee@state.edu").await;
    let (_, res) = mcp_call(
        &app,
        &token,
        "tools/call",
        Some(json!({ "name": "query_dataset", "arguments": { "dataset_id": "courses" } })),
    )
    .await;
    assert_eq!(res["result"]["isError"], true);
}

#[tokio::test]
async fn a_result_is_structured_content_and_one_matching_text_block() {
    let app = seeded_app().await;
    let token = login_user(&app, "jordan.lee@state.edu").await;
    let (_, res) = mcp_call(
        &app,
        &token,
        "tools/call",
        Some(json!({ "name": "list_datasets", "arguments": {} })),
    )
    .await;
    assert_eq!(res["result"]["isError"], Value::Null);
    let content = res["result"]["content"].as_array().unwrap();
    assert_eq!(content.len(), 1);
    assert_eq!(content[0]["type"], "text");
    let from_text: Value = serde_json::from_str(content[0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(from_text, res["result"]["structuredContent"]);
}

async fn raw_post(app: &Router, token: &str, payload: Value) -> (StatusCode, Vec<u8>) {
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
    (status, resp.into_body().collect().await.unwrap().to_bytes().to_vec())
}

#[tokio::test]
async fn a_notification_is_202_with_no_body() {
    let app = seeded_app().await;
    let token = login_user(&app, "jordan.lee@state.edu").await;
    let (status, body) =
        raw_post(&app, &token, json!({ "jsonrpc": "2.0", "method": "notifications/initialized" })).await;
    assert_eq!(status, StatusCode::ACCEPTED);
    assert!(body.is_empty(), "a notification gets no body");
}

#[tokio::test]
async fn get_mcp_is_405() {
    let app = seeded_app().await;
    let token = login_user(&app, "jordan.lee@state.edu").await;
    for path in ["/api/mcp", "/api/v1/mcp"] {
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(path)
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::METHOD_NOT_ALLOWED, "{path}");
    }
}

#[tokio::test]
async fn initialize_answers_with_a_version_it_speaks() {
    let app = seeded_app().await;
    let token = login_user(&app, "jordan.lee@state.edu").await;
    for (asked, expected) in [
        (Some("2024-11-05"), "2024-11-05"),
        (Some("2025-06-18"), "2025-06-18"),
        (Some("1999-01-01"), "2025-06-18"),
        (None, "2025-06-18"),
    ] {
        let params = match asked {
            Some(v) => json!({ "protocolVersion": v }),
            None => json!({}),
        };
        let (_, res) = mcp_call(&app, &token, "initialize", Some(params)).await;
        assert_eq!(res["result"]["protocolVersion"], expected, "asked {asked:?}");
    }
}

#[tokio::test]
async fn simulate_cedar_policy_calls_the_engine_and_needs_a_platform_admin() {
    let app = seeded_app().await;
    let admin = login_user(&app, "jordan.lee@state.edu").await;
    let faculty = login_user(&app, "sarah.connor@state.edu").await;
    let args = json!({
        "eppn": "student.smith@science.state.edu",
        "affiliation": "student",
        "action": "export",
        "ferpa_sensitive": true
    });

    // A faculty member is refused: the tool is for Platform Admins.
    let (_, res) = mcp_call(
        &app,
        &faculty,
        "tools/call",
        Some(json!({ "name": "simulate_cedar_policy", "arguments": args })),
    )
    .await;
    assert_eq!(res["result"]["isError"], true);

    let (_, res) = mcp_call(
        &app,
        &admin,
        "tools/call",
        Some(json!({ "name": "simulate_cedar_policy", "arguments": args })),
    )
    .await;
    assert_eq!(res["result"]["isError"], Value::Null, "{res}");
    let out = &res["result"]["structuredContent"];
    assert_eq!(out["decision"], "Deny");

    // The reasons are the engine's own, not a made-up policy name.
    let engine = scaffoldry_policy::ScaffoldryPolicyEngine::default_institutional_engine().unwrap();
    let identity = scaffoldry_core::standards::eduperson::EduPersonIdentity::parse(
        "student.smith@science.state.edu",
        vec!["student@science.state.edu"],
    )
    .unwrap();
    let expected = engine
        .authorize_record_action(&identity, "science", "export", "simulation", "science", true)
        .unwrap();
    let expected_reasons: Vec<Value> = expected.reasons.iter().map(|r| json!(r)).collect();
    assert!(!expected_reasons.is_empty(), "a forbid decided this");
    assert_eq!(out["reasons"], Value::Array(expected_reasons));
    assert!(out["deciding_policy"]["description"].as_str().is_some_and(|d| !d.is_empty()));
}

#[tokio::test]
async fn the_cedar_resource_is_the_source_the_engine_was_built_from() {
    let app = seeded_app().await;
    let token = login_user(&app, "jordan.lee@state.edu").await;
    let (_, res) = mcp_call(&app, &token, "resources/read", Some(json!({ "uri": "policies://cedar" }))).await;
    let text = res["result"]["contents"][0]["text"].as_str().unwrap();
    let engine = scaffoldry_policy::ScaffoldryPolicyEngine::default_institutional_engine().unwrap();
    assert_eq!(text, engine.policy_source());
    assert!(text.contains("@description"), "it is the real set, with descriptions");

    // The fake OSCAL resource is gone. export_oscal_compliance is the real one.
    let (_, listed) = mcp_call(&app, &token, "resources/list", None).await;
    let uris: Vec<&str> = listed["result"]["resources"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["uri"].as_str().unwrap())
        .collect();
    assert!(!uris.contains(&"compliance://oscal"));
    assert!(uris.contains(&"policies://cedar"));
}

#[tokio::test]
async fn a_recorded_decision_is_attributed_to_the_session_not_to_an_argument() {
    let app = seeded_app().await;
    let faculty = login_user(&app, "sarah.connor@state.edu").await;
    let admin = login_user(&app, "jordan.lee@state.edu").await;

    // Naming another principal is refused outright, and the error says why.
    let (_, refused) = mcp_call(
        &app,
        &faculty,
        "tools/call",
        Some(json!({
            "name": "record_governance_decision",
            "arguments": {
                "principal": "someone.else@state.edu",
                "organization_code": "CS",
                "decision_type": "PolicyRevision",
                "oscal_control_id": "CM-03",
                "rationale": "Attribution test"
            }
        })),
    )
    .await;
    assert_eq!(refused["result"]["isError"], true);
    assert!(refused["result"]["content"][0]["text"].as_str().unwrap().contains("principal"));

    let (_, res) = mcp_call(
        &app,
        &faculty,
        "tools/call",
        Some(json!({
            "name": "record_governance_decision",
            "arguments": {
                "organization_code": "CS",
                "decision_type": "PolicyRevision",
                "oscal_control_id": "CM-03",
                "rationale": "Attribution test"
            }
        })),
    )
    .await;
    assert_eq!(res["result"]["isError"], Value::Null, "{res}");
    let sequence = res["result"]["structuredContent"]["sequence"].as_u64().unwrap();

    let ledger = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/governance/ledger")
                .header("authorization", format!("Bearer {admin}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let bytes = ledger.into_body().collect().await.unwrap().to_bytes();
    let ledger: Value = serde_json::from_slice(&bytes).unwrap();
    let entry = ledger["entries"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["sequence"].as_u64() == Some(sequence))
        .expect("the entry is in the ledger");
    assert_eq!(entry["principal"], "sarah.connor@state.edu");

    // A student may not record decisions, the same rule as the REST route.
    let student = login_user(&app, "student.smith@science.state.edu").await;
    let (_, res) = mcp_call(
        &app,
        &student,
        "tools/call",
        Some(json!({
            "name": "record_governance_decision",
            "arguments": {
                "organization_code": "PHYS",
                "decision_type": "PolicyRevision",
                "oscal_control_id": "CM-03",
                "rationale": "Should be refused"
            }
        })),
    )
    .await;
    assert_eq!(res["result"]["isError"], true);
}
