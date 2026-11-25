//! MCP toolsets phase 1 (`docs/plans/mcp-toolsets.md`): every tool is in one toolset, and a token that names toolsets
//! lists and calls only those, plus `core`.

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::Router;
use http_body_util::BodyExt;
use scaffoldry_core::{AgentLevel, TokenScope, Toolset};
use scaffoldry_server::build_app_with_state;
use scaffoldry_server::service::identity::{hash_token, issue_test_token_and_user};
use scaffoldry_server::service::tools::TOOLS;
use scaffoldry_server::state::ApiToken;
use serde_json::{json, Value};
use std::collections::BTreeSet;
use tower::ServiceExt;

const ADMIN: &str = "jordan.lee@state.edu";
const MEMBER: &str = "prof.curie@science.state.edu";
const BYTE_BUDGET: usize = 24_000;
const TOOL_BUDGET: usize = 45;

struct World {
    _db: common::TestDb,
    app: Router,
    state: scaffoldry_server::state::SharedState,
}

fn world() -> World {
    let db = common::TestDb::new();
    let state = db.state();
    state.seed_demo().expect("seed");
    let app = build_app_with_state(state.clone()).expect("router");
    World { _db: db, app, state }
}

/// A token for `eppn` with this scope. The person is made first, so the token names a real person.
fn token_with(w: &World, eppn: &str, scope: Option<TokenScope>) -> String {
    let _ = issue_test_token_and_user(eppn);
    let raw = format!("raw-{}", uuid::Uuid::new_v4());
    w.state
        .persist_api_token(&ApiToken {
            token_hash: hash_token(&raw),
            id: uuid::Uuid::new_v4(),
            kind: "agent".to_string(),
            eppn: eppn.to_string(),
            label: "toolsets test".to_string(),
            original_admin: None,
            created_at: chrono::Utc::now(),
            expires_at: chrono::Utc::now() + chrono::Duration::days(1),
            last_used_at: None,
            revoked_at: None,
            app_slug: None,
            scope,
        })
        .expect("persisted");
    raw
}

/// A session at the Write level, with the toolsets a test names. Production sessions keep `None` in phase 1.
fn session(level: AgentLevel, toolsets: Option<Vec<Toolset>>) -> TokenScope {
    TokenScope { apps: Vec::new(), tables: Default::default(), tools: None, max_rows_per_call: None, level: Some(level), toolsets }
}

async fn post(w: &World, path: &str, token: &str, body: Value) -> (StatusCode, Value) {
    let req = Request::builder()
        .method("POST")
        .uri(path)
        .header("content-type", "application/json")
        .header("authorization", format!("Bearer {token}"))
        .body(Body::from(body.to_string()))
        .unwrap();
    let resp = w.app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
}

async fn list(w: &World, token: &str) -> Vec<Value> {
    let (_, v) = post(w, "/api/v1/mcp", token, json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/list" })).await;
    v["result"]["tools"].as_array().cloned().unwrap_or_default()
}

fn names(tools: &[Value]) -> BTreeSet<String> {
    tools.iter().map(|t| t["name"].as_str().unwrap().to_string()).collect()
}

fn toolset_of(name: &str) -> Toolset {
    TOOLS.iter().find(|t| t.name == name).unwrap_or_else(|| panic!("{name} is a tool")).toolset
}

async fn call(w: &World, token: &str, name: &str, args: Value) -> Value {
    let (_, v) = post(w, "/api/v1/mcp", token, json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": { "name": name, "arguments": args } })).await;
    v["result"].clone()
}

#[test]
fn every_tool_is_in_exactly_one_toolset() {
    // One field cannot name two toolsets, so "exactly one" is that every tool has a name once and the sets add up.
    let mut seen = BTreeSet::new();
    for t in TOOLS {
        assert!(seen.insert(t.name), "{} is registered twice", t.name);
    }
    let total: usize = Toolset::ALL.iter().map(|s| TOOLS.iter().filter(|t| t.toolset == *s).count()).sum();
    assert_eq!(total, TOOLS.len());
    for s in Toolset::ALL {
        assert!(TOOLS.iter().any(|t| t.toolset == s), "{} has no tool", s.word());
    }
}

#[test]
fn the_toolsets_are_assigned_as_the_note_says() {
    let table: [(Toolset, &[&str]); 6] = [
        (Toolset::Core, &["begin_task", "list_workspaces", "get_workspace", "list_apps", "describe_app", "list_datasets", "get_agent_kit", "list_my_work", "complete_task", "get_job", "cancel_job", "request_access", "explain_access"]),
        (Toolset::Data, &["create_record", "list_records", "get_record", "update_record", "delete_record", "aggregate_records", "get_view_data", "list_views", "save_view", "delete_view", "export_records", "decide_process"]),
        (Toolset::Build, &["propose_app_change", "list_proposals", "get_proposal", "preview_proposal", "decide_proposal", "get_page", "page_call", "update_workspace", "manage_workspace_member"]),
        (Toolset::Access, &["draft_access_change", "access_suggestions"]),
        (Toolset::Governance, &["governance_report", "record_governance_decision", "simulate_cedar_policy", "list_library_policies", "read_library_snapshot", "find_library_rules", "propose_library_rules", "list_library_questions"]),
        (Toolset::Admin, &["admin_inventory"]),
    ];
    for (set, tools) in table {
        let mut have: Vec<&str> = TOOLS.iter().filter(|t| t.toolset == set).map(|t| t.name).collect();
        let mut want = tools.to_vec();
        have.sort();
        want.sort();
        assert_eq!(have, want, "{}", set.word());
    }
}

#[test]
fn a_scope_refuses_an_unknown_toolset_and_core() {
    let base = |toolsets: Value| json!({ "apps": ["a"], "tables": { "t": ["read"] }, "toolsets": toolsets });
    assert!(TokenScope::parse(&base(json!(["data", "governance"]))).is_ok());
    assert!(TokenScope::parse(&base(json!([]))).is_ok(), "no toolsets beyond core is a choice");
    assert!(TokenScope::parse(&base(json!(["telepathy"]))).is_err(), "an unknown name");
    let err = TokenScope::parse(&base(json!(["core"]))).unwrap_err();
    assert!(err.contains("core"), "the refusal says why: {err}");
    // Absent stays absent, and is not written back out.
    let none = TokenScope::parse(&json!({ "apps": ["a"], "tables": { "t": ["read"] } })).unwrap();
    assert_eq!(none.toolsets, None);
    assert!(serde_json::to_value(&none).unwrap().get("toolsets").is_none());
}

#[tokio::test]
async fn a_token_with_data_and_governance_lists_core_data_and_governance_only() {
    let w = world();
    let granted = session(AgentLevel::Write, Some(vec![Toolset::Data, Toolset::Governance]));
    let with = token_with(&w, ADMIN, Some(granted));
    let without = token_with(&w, ADMIN, Some(session(AgentLevel::Write, None)));
    let listed = names(&list(&w, &with).await);
    for name in &listed {
        assert!(matches!(toolset_of(name), Toolset::Core | Toolset::Data | Toolset::Governance), "{name} is outside the granted toolsets");
    }
    for name in ["list_workspaces", "list_records", "governance_report"] {
        assert!(listed.contains(name), "{name} is listed");
    }
    for name in ["propose_app_change", "draft_access_change", "admin_inventory"] {
        assert!(!listed.contains(name), "{name} is not listed");
    }
    // The toolsets only narrow. Everything else about the list is what the same session lists today.
    let today: BTreeSet<String> = names(&list(&w, &without).await).into_iter().filter(|n| matches!(toolset_of(n), Toolset::Core | Toolset::Data | Toolset::Governance)).collect();
    assert_eq!(listed, today);
    // An empty choice is core alone.
    let core = token_with(&w, ADMIN, Some(session(AgentLevel::Write, Some(Vec::new()))));
    assert!(names(&list(&w, &core).await).iter().all(|n| toolset_of(n) == Toolset::Core));
}

#[tokio::test]
async fn a_token_with_no_toolsets_lists_what_it_lists_today() {
    let w = world();
    let plain = issue_test_token_and_user(ADMIN);
    let all = names(&list(&w, &plain).await);
    assert_eq!(all, TOOLS.iter().map(|t| t.name.to_string()).collect::<BTreeSet<_>>(), "an administrator without a scope lists every tool");
    let session = token_with(&w, ADMIN, Some(session(AgentLevel::Read, None)));
    let listed = names(&list(&w, &session).await);
    for name in ["list_records", "governance_report", "begin_task", "get_proposal"] {
        assert!(listed.contains(name), "a read session still lists {name}");
    }
    for name in ["create_record", "propose_app_change", "decide_proposal"] {
        assert!(!listed.contains(name), "a read session still does not list {name}");
    }
}

#[tokio::test]
async fn a_call_outside_the_granted_toolsets_is_refused_with_required_toolset() {
    let w = world();
    let t = token_with(&w, ADMIN, Some(session(AgentLevel::Write, Some(vec![Toolset::Data]))));
    let result = call(&w, &t, "governance_report", json!({})).await;
    assert_eq!(result["isError"], true, "{result}");
    let body = &result["structuredContent"];
    assert_eq!(body["code"], "token_scope", "{body}");
    assert_eq!(body["required_toolset"], "governance", "{body}");
    assert!(body["request_path"].as_str().unwrap().to_lowercase().contains("reconnect"), "{body}");
    assert!(body["request_path"].as_str().unwrap().contains("governance"), "{body}");
    assert!(!body["hint"].as_str().unwrap().is_empty(), "{body}");
    // A tool in a granted toolset, and in core, is not stopped by this check.
    let ok = call(&w, &t, "list_workspaces", json!({})).await;
    assert_ne!(ok["isError"], true, "{ok}");
    let ok = call(&w, &t, "list_views", json!({ "app_slug": "no-such-app" })).await;
    assert_ne!(ok["structuredContent"]["required_toolset"], "data", "{ok}");
}

#[tokio::test]
async fn admin_is_never_listed_to_a_non_admin_even_when_asked_for() {
    let w = world();
    let asked = token_with(&w, MEMBER, Some(session(AgentLevel::Write, Some(vec![Toolset::Admin, Toolset::Data]))));
    let listed = names(&list(&w, &asked).await);
    assert!(!listed.contains("admin_inventory"), "{listed:?}");
    assert!(listed.contains("list_records"));
    // The call is refused by the person's own rights, not allowed by the toolset.
    let result = call(&w, &asked, "admin_inventory", json!({})).await;
    assert_eq!(result["isError"], true, "{result}");
    assert_ne!(result["structuredContent"]["required_toolset"], "admin");
    // An administrator who asks for it gets it, and one who does not, does not.
    let admin = token_with(&w, ADMIN, Some(session(AgentLevel::Read, Some(vec![Toolset::Admin]))));
    assert!(names(&list(&w, &admin).await).contains("admin_inventory"));
    let admin_without = token_with(&w, ADMIN, Some(session(AgentLevel::Read, Some(vec![Toolset::Data]))));
    assert!(!names(&list(&w, &admin_without).await).contains("admin_inventory"));
}

#[tokio::test]
async fn a_limited_token_keeps_its_own_list_and_toolsets_narrow_it_further() {
    let w = world();
    let limited = |toolsets: Option<Vec<Toolset>>| TokenScope {
        apps: vec!["a".to_string()],
        tables: [("t".to_string(), vec![scaffoldry_core::TokenOp::Read])].into_iter().collect(),
        tools: None,
        max_rows_per_call: None,
        level: None,
        toolsets,
    };
    let today = names(&list(&w, &token_with(&w, ADMIN, Some(limited(None)))).await);
    assert!(today.contains("list_records") && today.contains("describe_app"), "{today:?}");
    let core_only = names(&list(&w, &token_with(&w, ADMIN, Some(limited(Some(Vec::new()))))).await);
    assert_eq!(core_only, BTreeSet::from(["describe_app".to_string()]));
}

#[tokio::test]
async fn minting_a_token_accepts_toolsets_and_refuses_core() {
    let w = world();
    let owner = issue_test_token_and_user(ADMIN);
    let scope = |toolsets: Value| json!({ "label": "agent", "kind": "agent", "scope": { "apps": ["a"], "tables": { "t": ["read"] }, "toolsets": toolsets } });
    let (status, body) = post(&w, "/api/v1/auth/tokens", &owner, scope(json!(["data"]))).await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    assert_eq!(body["scope"]["toolsets"], json!(["data"]));
    for bad in [json!(["core"]), json!(["nope"])] {
        let (status, body) = post(&w, "/api/v1/auth/tokens", &owner, scope(bad)).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    }
}

#[tokio::test]
async fn the_largest_reachable_list_stays_inside_24_kb_and_45_tools() {
    let w = world();
    // The largest list is a Platform Admin with every toolset: no scope, so every toolset.
    let listed = list(&w, &issue_test_token_and_user(ADMIN)).await;
    let bytes = serde_json::to_vec(&listed).unwrap().len();
    eprintln!("largest reachable list: {} tools, {bytes} bytes", listed.len());
    assert!(bytes <= BYTE_BUDGET, "{bytes} bytes is over {BYTE_BUDGET}");
    assert!(listed.len() <= TOOL_BUDGET, "{} tools is over {TOOL_BUDGET}", listed.len());
    let sets: BTreeSet<&str> = listed.iter().map(|t| toolset_of(t["name"].as_str().unwrap()).word()).collect();
    assert_eq!(sets.len(), Toolset::ALL.len(), "every toolset is in the largest list");
}
