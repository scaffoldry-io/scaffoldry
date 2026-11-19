//! MCP guidance phase 4: five prompts are built from policy data when they are asked for. Changing the policy changes the
//! prompt with no code change. Each prompt names its tools and its stopping points, names only real tools, and carries no
//! record value. Prompts are not tools, so the tool list does not grow.

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use common::TestDb;
use http_body_util::BodyExt;
use scaffoldry_server::build_app_with_state;
use scaffoldry_server::service::identity::issue_test_token_and_user;
use scaffoldry_server::service::tools::TOOLS;
use serde_json::{json, Value};
use tower::ServiceExt;
use uuid::Uuid;

const WORKSPACE: &str = "ws-physics-optics";
const OWNER: &str = "einstein@physics.state.edu";
const OUTSIDER: &str = "marcus.vance@state.edu";
const ADMIN: &str = "jordan.lee@state.edu";
const ADMIN2: &str = "second.admin@state.edu";
const SECRET: &str = "SECRET-VALUE-4417";
const ROOT: &str = "00000000-0000-0000-0000-000000000001";

const FIVE: [&str; 5] = ["request_dataset_access", "review_a_change", "write_a_policy", "review_a_share_request", "explain_a_refusal"];

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

async fn rpc(app: &axum::Router, who: &str, method: &str, params: Value) -> Value {
    let payload = json!({ "jsonrpc": "2.0", "id": 1, "method": method, "params": params });
    send(app, "POST", "/api/v1/mcp", who, Some(payload)).await.1["result"].clone()
}

async fn prompt(app: &axum::Router, who: &str, name: &str, args: Value) -> String {
    let got = rpc(app, who, "prompts/get", json!({ "name": name, "arguments": args })).await;
    got["messages"][0]["content"]["text"].as_str().unwrap_or_else(|| panic!("{name} resolves: {got}")).to_string()
}

async fn fixture(db: &TestDb) -> (axum::Router, String) {
    let state = db.state();
    state.seed_demo().expect("seed");
    let app = build_app_with_state(state).expect("router");
    let slug = format!("g4-{}", &Uuid::new_v4().to_string()[..8]);
    let manifest = json!({
        "slug": slug, "title": "Guidance", "description": "d", "organization_code": "PHYS", "department": "physics", "views": [],
        "tables": [{ "id": "people", "name": "People", "slug": "people", "fields": [
            { "name": "name", "label": "Name", "field_type": "Text", "required": true, "ferpa_sensitive": false }
        ] }]
    });
    let (s, body) = send(&app, "POST", &format!("/api/v1/workspaces/{WORKSPACE}/apps"), OWNER, Some(manifest)).await;
    assert_eq!(s, StatusCode::CREATED, "{body}");
    let payload = json!({ "name": "create_record", "arguments": { "app_slug": slug, "data": { "_table_id": "people", "name": SECRET } } });
    let r = rpc(&app, OWNER, "tools/call", payload).await;
    assert!(r["structuredContent"]["id"].is_string(), "{r}");
    (app, slug)
}

/// The arguments that make each prompt as full as it gets.
fn args_for(name: &str, slug: &str) -> Value {
    match name {
        "request_dataset_access" => json!({ "dataset": slug, "role": "editor" }),
        "review_a_change" => json!({ "proposal_id": Uuid::new_v4().to_string() }),
        "write_a_policy" => json!({ "description": "Staff may not export student records" }),
        "review_a_share_request" => json!({ "app_slug": slug, "justification": "The board asked for a link" }),
        "explain_a_refusal" => json!({ "object_kind": "app", "object_id": slug, "role": "editor" }),
        _ => json!({}),
    }
}

#[tokio::test]
async fn the_five_prompts_are_listed_and_each_has_the_five_parts() {
    let db = TestDb::new();
    let (app, slug) = fixture(&db).await;
    let listed = rpc(&app, ADMIN, "prompts/list", json!({})).await;
    let names: Vec<&str> = listed["prompts"].as_array().unwrap().iter().filter_map(|p| p["name"].as_str()).collect();
    for p in FIVE {
        assert!(names.contains(&p), "{p} is listed: {names:?}");
    }
    // The prompts that were there before are still listed.
    for p in ["governed_app_builder", "ferpa_risk_assessment", "tidy_access", "explain_denial"] {
        assert!(names.contains(&p), "{p} is still listed");
    }
    for who in [ADMIN, OUTSIDER] {
        for name in FIVE {
            let text = prompt(&app, who, name, args_for(name, &slug)).await;
            assert!(text.len() < 9_000, "{name} is {} bytes", text.len());
            for part in ["Task:", "Context:", "Tools you may use:", "Stop and ask the person:", "Output shape:"] {
                assert!(text.contains(part), "{name} has '{part}': {text}");
            }
            assert!(text.contains(who), "{name} says who the agent acts for");
            let stops = text.split("Stop and ask the person:").nth(1).unwrap_or("");
            assert!(stops.to_lowercase().contains("never") || stops.to_lowercase().contains("propose"), "{name} says nothing is applied by the agent: {text}");
        }
    }
    // Asking with no arguments still works, and an unknown prompt still resolves to nothing.
    for name in FIVE {
        prompt(&app, ADMIN, name, json!({})).await;
    }
    let none = rpc(&app, ADMIN, "prompts/get", json!({ "name": "no_such_prompt" })).await;
    assert_eq!(none["messages"], json!([]));
}

#[tokio::test]
async fn a_prompt_names_only_tools_that_exist() {
    let db = TestDb::new();
    let (app, slug) = fixture(&db).await;
    let tools: Vec<&str> = TOOLS.iter().map(|t| t.name).collect();
    for who in [ADMIN, OUTSIDER] {
        for name in FIVE {
            let text = prompt(&app, who, name, args_for(name, &slug)).await;
            // Every word in backticks is a tool.
            let ticked: Vec<&str> = text.split('`').skip(1).step_by(2).collect();
            assert!(!ticked.is_empty(), "{name} names its tools");
            for word in &ticked {
                assert!(tools.contains(word), "{name} names `{word}`, which is not a tool");
            }
            // A word that looks like a tool name is one.
            for word in text.split(|c: char| !(c.is_ascii_lowercase() || c == '_')).filter(|w| w.contains('_') && w.len() > 6) {
                let looks_like_tool = ["list_", "get_", "describe_", "propose_", "decide_", "create_", "update_", "delete_", "save_", "export_", "simulate_", "cancel_", "explain_", "draft_", "preview_"]
                    .iter()
                    .any(|p| word.starts_with(p));
                assert!(!looks_like_tool || tools.contains(&word), "{name} names {word}, which is not a tool");
            }
        }
    }
}

#[tokio::test]
async fn the_tool_list_is_the_same_size_after_the_prompts() {
    let total: usize = TOOLS.len();
    let bytes: usize = TOOLS.iter().map(|t| t.name.len() + t.description.len()).sum();
    assert!(total > 0 && bytes > 0);
    let db = TestDb::new();
    let (app, _) = fixture(&db).await;
    let listed = rpc(&app, ADMIN, "tools/list", json!({})).await;
    let names: Vec<&str> = listed["tools"].as_array().unwrap().iter().filter_map(|t| t["name"].as_str()).collect();
    for p in FIVE {
        assert!(!names.contains(&p), "{p} is a prompt and not a tool");
    }
}

#[tokio::test]
async fn no_prompt_carries_a_record_value() {
    let db = TestDb::new();
    let (app, slug) = fixture(&db).await;
    for who in [OWNER, OUTSIDER, ADMIN] {
        for name in FIVE {
            let text = prompt(&app, who, name, args_for(name, &slug)).await;
            assert!(!text.contains(SECRET), "{name} for {who} carries a record value");
        }
    }
}

async fn setting(app: &axum::Router, key: &str, value: Value) {
    let (s, b) = send(app, "PUT", &format!("/api/v1/settings/{key}"), ADMIN, Some(value)).await;
    assert_eq!(s, StatusCode::OK, "{b}");
}

#[tokio::test]
async fn a_changed_setting_changes_the_request_prompts() {
    let db = TestDb::new();
    let (app, slug) = fixture(&db).await;
    let before = prompt(&app, OUTSIDER, "request_dataset_access", args_for("request_dataset_access", &slug)).await;
    assert!(before.contains("90 days"), "{before}");
    setting(&app, "access.request_max_days", json!(45)).await;
    let after = prompt(&app, OUTSIDER, "request_dataset_access", args_for("request_dataset_access", &slug)).await;
    assert_ne!(before, after);
    assert!(after.contains("45 days") && !after.contains("90 days"), "{after}");
}

#[tokio::test]
async fn a_changed_request_rule_changes_the_prompts_that_describe_it() {
    let db = TestDb::new();
    let (app, slug) = fixture(&db).await;
    let before_request = prompt(&app, OUTSIDER, "request_dataset_access", args_for("request_dataset_access", &slug)).await;
    let before_refusal = prompt(&app, OUTSIDER, "explain_a_refusal", args_for("explain_a_refusal", &slug)).await;
    assert!(before_request.contains("A viewer request"), "{before_request}");
    assert!(before_refusal.contains("A viewer request"), "{before_refusal}");

    issue_test_token_and_user(ADMIN2);
    let (s, b) = send(&app, "POST", &format!("/api/v1/orgs/{ROOT}/appointments"), ADMIN, Some(json!({ "eppn": ADMIN2, "scoped_affiliation": "platform_admin", "reason": "A second administrator" }))).await;
    assert_eq!(s, StatusCode::CREATED, "{b}");
    let rule = json!({ "max_role": "editor", "max_sensitivity": "Internal", "visibilities": ["departmental"], "own_unit": false, "max_days": 14 });
    let (s, proposed) = send(&app, "POST", "/api/v1/admin/access/request-rules", ADMIN, Some(json!({ "organization_id": ROOT, "reason": "Editors for two weeks", "rule": rule }))).await;
    assert_eq!(s, StatusCode::ACCEPTED, "{proposed}");
    let change = proposed["change_id"].as_str().unwrap();
    let (s, done) = send(&app, "POST", &format!("/api/v1/admin/access/changes/{change}/decide"), ADMIN2, Some(json!({ "decision": "confirm", "reason": "Second person" }))).await;
    assert_eq!(s, StatusCode::OK, "{done}");

    let after_request = prompt(&app, OUTSIDER, "request_dataset_access", args_for("request_dataset_access", &slug)).await;
    let after_refusal = prompt(&app, OUTSIDER, "explain_a_refusal", args_for("explain_a_refusal", &slug)).await;
    assert_ne!(before_request, after_request);
    assert_ne!(before_refusal, after_refusal);
    assert!(after_request.contains("An editor request") && after_refusal.contains("An editor request"), "{after_request}");
}

#[tokio::test]
async fn a_changed_share_setting_changes_the_share_prompt() {
    let db = TestDb::new();
    let (app, slug) = fixture(&db).await;
    setting(&app, "sharing.allowed_scopes", json!([])).await;
    let off = prompt(&app, ADMIN, "review_a_share_request", args_for("review_a_share_request", &slug)).await;
    setting(&app, "sharing.allowed_scopes", json!(["institution"])).await;
    let on = prompt(&app, ADMIN, "review_a_share_request", args_for("review_a_share_request", &slug)).await;
    assert_ne!(off, on);
    assert!(off.contains("turned off") || off.contains("none"), "{off}");
    assert!(on.contains("institution"), "{on}");
}

#[tokio::test]
async fn a_new_active_cedar_policy_changes_the_policy_prompt() {
    let db = TestDb::new();
    let (app, _) = fixture(&db).await;
    let before = prompt(&app, ADMIN, "write_a_policy", json!({ "description": "x" })).await;
    assert!(!before.contains("no-affiliate-read-g4"), "{before}");
    issue_test_token_and_user(ADMIN2);
    let (s, b) = send(&app, "POST", &format!("/api/v1/orgs/{ROOT}/appointments"), ADMIN, Some(json!({ "eppn": ADMIN2, "scoped_affiliation": "platform_admin", "reason": "A second administrator" }))).await;
    assert_eq!(s, StatusCode::CREATED, "{b}");
    let (_, active) = send(&app, "GET", "/api/v1/admin/policy/active", ADMIN, None).await;
    let rule = "@id(\"no-affiliate-read-g4\") @oscal(\"ac-3\") @description(\"Affiliates may not read\")\nforbid (principal, action == Action::\"read\", resource) when { principal.scoped_affiliation == \"affiliate\" };";
    let source = format!("{}\n{rule}\n", active["source"].as_str().unwrap());
    let (s, d) = send(&app, "POST", "/api/v1/admin/policy/versions", ADMIN, Some(json!({ "source": source, "note": "A change" }))).await;
    assert_eq!(s, StatusCode::CREATED, "{d}");
    let id = d["id"].as_str().unwrap();
    let (s, b) = send(&app, "POST", &format!("/api/v1/admin/policy/versions/{id}/activate"), ADMIN2, Some(json!({ "reason": "Reviewed" }))).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    let after = prompt(&app, ADMIN, "write_a_policy", json!({ "description": "x" })).await;
    assert_ne!(before, after);
    assert!(after.contains("no-affiliate-read-g4") && after.contains("Affiliates may not read"), "{after}");
}

#[tokio::test]
async fn a_changed_authority_table_changes_the_review_prompt() {
    let db = TestDb::new();
    let (app, _) = fixture(&db).await;
    let before = prompt(&app, ADMIN, "review_a_change", json!({})).await;
    issue_test_token_and_user(ADMIN2);
    let (s, b) = send(&app, "POST", &format!("/api/v1/orgs/{ROOT}/appointments"), ADMIN, Some(json!({ "eppn": ADMIN2, "scoped_affiliation": "platform_admin", "reason": "A second administrator" }))).await;
    assert_eq!(s, StatusCode::CREATED, "{b}");
    let (s, p) = send(&app, "POST", "/api/v1/admin/authority/changes", ADMIN, Some(json!({ "organization_id": ROOT, "change": { "kind": "signal", "check_name": "access_loosened", "tier": "High" }, "reason": "Tighten" }))).await;
    assert_eq!(s, StatusCode::CREATED, "{p}");
    let id = p["change"]["id"].as_str().unwrap();
    let (s, b) = send(&app, "POST", &format!("/api/v1/admin/authority/changes/{id}/decide"), ADMIN2, Some(json!({ "decision": "confirm", "note": "Reviewed" }))).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    let after = prompt(&app, ADMIN, "review_a_change", json!({})).await;
    assert_ne!(before, after);
    let line = |t: &str| t.lines().find(|l| l.contains("access_loosened")).unwrap_or("").to_string();
    assert!(line(&before).contains("Moderate") && line(&after).contains("High"), "{} / {}", line(&before), line(&after));
}

#[tokio::test]
async fn the_policy_prompt_offers_the_simulator_only_to_a_platform_admin() {
    let db = TestDb::new();
    let (app, _) = fixture(&db).await;
    let admin = prompt(&app, ADMIN, "write_a_policy", json!({})).await;
    let other = prompt(&app, OUTSIDER, "write_a_policy", json!({})).await;
    assert!(admin.contains("- `simulate_cedar_policy`"), "{admin}");
    assert!(!other.contains("- `simulate_cedar_policy`"), "{other}");
    assert!(other.contains("Do not use:") && other.contains("`simulate_cedar_policy`"), "{other}");
}
