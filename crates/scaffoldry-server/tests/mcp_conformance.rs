//! Agent coverage phase 2: the MCP endpoint holds to the protocol, every tool describes itself truthfully, and the tool
//! list stays small. A client reads `tools/list` into its context on every session, so these are product limits.

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use scaffoldry_server::build_app_with_state;
use scaffoldry_core::{AgentLevel, TokenScope, Toolset};
use scaffoldry_server::service::identity::{hash_token, issue_test_token_and_user};
use scaffoldry_server::service::tools::TOOLS;
use scaffoldry_server::state::{ApiToken, ServerState};
use serde_json::{json, Value};
use std::sync::Arc;
use tower::ServiceExt;

const ADMIN: &str = "jordan.lee@state.edu";

async fn raw(app: &axum::Router, body: &str) -> (StatusCode, Vec<u8>) {
    let token = issue_test_token_and_user(ADMIN);
    let req = Request::builder()
        .method("POST")
        .uri("/api/v1/mcp")
        .header("content-type", "application/json")
        .header("authorization", format!("Bearer {token}"))
        .body(Body::from(body.to_string()))
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    (status, resp.into_body().collect().await.unwrap().to_bytes().to_vec())
}

async fn rpc(app: &axum::Router, body: Value) -> Value {
    let (_, bytes) = raw(app, &body.to_string()).await;
    serde_json::from_slice(&bytes).unwrap_or(Value::Null)
}

fn app() -> axum::Router {
    build_app_with_state(Arc::new(ServerState::new().expect("state"))).expect("router")
}

async fn listed(app: &axum::Router) -> Vec<Value> {
    rpc(app, json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/list" })).await["result"]["tools"].as_array().cloned().unwrap_or_default()
}

// --- the protocol ---------------------------------------------------------------------------------------------------

#[tokio::test]
async fn initialize_negotiates_the_version_and_offers_the_highest_for_one_it_does_not_speak() {
    let _db = common::TestDb::new();
    let app = app();
    let ask = |v: Value| {
        let app = app.clone();
        async move { rpc(&app, json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": { "protocolVersion": v, "capabilities": {}, "clientInfo": { "name": "t", "version": "1" } } })).await }
    };
    assert_eq!(ask(json!("2024-11-05")).await["result"]["protocolVersion"], "2024-11-05", "an older version it speaks is kept");
    assert_eq!(ask(json!("2025-06-18")).await["result"]["protocolVersion"], "2025-06-18");
    assert_eq!(ask(json!("1999-01-01")).await["result"]["protocolVersion"], "2025-06-18", "one it does not speak gets the highest it does");
    assert_eq!(ask(json!(42)).await["result"]["protocolVersion"], "2025-06-18", "a version that is not a string gets the highest");
    let full = ask(json!("2025-06-18")).await;
    for key in ["capabilities", "serverInfo"] {
        assert!(full["result"].get(key).is_some(), "initialize answers with {key}: {full}");
    }
}

#[tokio::test]
async fn a_notification_gets_202_and_no_body() {
    let _db = common::TestDb::new();
    let app = app();
    for method in ["notifications/initialized", "notifications/cancelled", "notifications/anything"] {
        let (status, body) = raw(&app, &json!({ "jsonrpc": "2.0", "method": method }).to_string()).await;
        assert_eq!(status, StatusCode::ACCEPTED, "{method}");
        assert!(body.is_empty(), "{method} gets no body");
    }
}

#[tokio::test]
async fn unknown_methods_and_invalid_requests_get_the_right_json_rpc_codes() {
    let _db = common::TestDb::new();
    let app = app();
    let r = rpc(&app, json!({ "jsonrpc": "2.0", "id": 5, "method": "no/such/method" })).await;
    assert_eq!((r["error"]["code"].as_i64(), r["id"].as_i64()), (Some(-32601), Some(5)), "method not found names the id: {r}");
    let r = rpc(&app, json!({ "jsonrpc": "1.0", "id": 6, "method": "ping" })).await;
    assert_eq!(r["error"]["code"], -32600, "a wrong jsonrpc version is an invalid request: {r}");
    let r = rpc(&app, json!({ "jsonrpc": "2.0", "id": 7 })).await;
    assert_eq!(r["error"]["code"], -32600, "no method is an invalid request: {r}");
    let (_, bytes) = raw(&app, "{not json").await;
    let r: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(r["error"]["code"], -32700, "{r}");
    assert!(r["id"].is_null(), "a parse error names no id");
    // A tool that is not registered is a tool error the agent can read, not a protocol failure.
    let r = rpc(&app, json!({ "jsonrpc": "2.0", "id": 8, "method": "tools/call", "params": { "name": "no_such_tool", "arguments": {} } })).await;
    assert_eq!(r["result"]["isError"], true, "{r}");
}

// --- every tool describes itself -----------------------------------------------------------------------------------

/// A small structural check of a JSON Schema 2020-12 object schema, enough for what a tool may use.
fn check_schema(schema: &Value, path: &str) -> Result<(), String> {
    let obj = schema.as_object().ok_or(format!("{path}: a schema is an object"))?;
    if let Some(t) = obj.get("type") {
        let ok = |s: &str| ["object", "array", "string", "number", "integer", "boolean", "null"].contains(&s);
        let valid = match t {
            Value::String(s) => ok(s),
            Value::Array(a) => !a.is_empty() && a.iter().all(|x| x.as_str().is_some_and(ok)),
            _ => false,
        };
        if !valid {
            return Err(format!("{path}: type {t} is not a JSON Schema type"));
        }
    }
    if let Some(props) = obj.get("properties") {
        for (k, v) in props.as_object().ok_or(format!("{path}: properties is an object"))? {
            check_schema(v, &format!("{path}.{k}"))?;
        }
    }
    if let Some(req) = obj.get("required") {
        let names: Vec<&str> = req.as_array().ok_or(format!("{path}: required is an array"))?.iter().filter_map(Value::as_str).collect();
        for n in names {
            if !obj.get("properties").and_then(|p| p.get(n)).is_some() {
                return Err(format!("{path}: required names {n}, which is not a property"));
            }
        }
    }
    if let Some(e) = obj.get("enum") {
        if e.as_array().is_none_or(|a| a.is_empty()) {
            return Err(format!("{path}: enum is a non-empty array"));
        }
    }
    if let Some(items) = obj.get("items") {
        check_schema(items, &format!("{path}[]"))?;
    }
    Ok(())
}

/// Whether `value` fits `schema`, for the keywords a tool uses.
fn fits(schema: &Value, value: &Value, path: &str) -> Result<(), String> {
    if let Some(t) = schema["type"].as_str() {
        let ok = match t {
            "object" => value.is_object(),
            "array" => value.is_array(),
            "string" => value.is_string(),
            "integer" => value.is_i64() || value.is_u64(),
            "number" => value.is_number(),
            "boolean" => value.is_boolean(),
            _ => true,
        };
        if !ok {
            return Err(format!("{path}: expected {t}, got {value}"));
        }
    }
    if let Some(e) = schema["enum"].as_array() {
        if !e.contains(value) {
            return Err(format!("{path}: {value} is not one of {e:?}"));
        }
    }
    if let (Some(props), Some(obj)) = (schema["properties"].as_object(), value.as_object()) {
        for (k, v) in obj {
            match props.get(k) {
                Some(p) => fits(p, v, &format!("{path}.{k}"))?,
                None if schema["additionalProperties"] == json!(false) => return Err(format!("{path}: {k} is not an allowed property")),
                None => {}
            }
        }
        for r in schema["required"].as_array().into_iter().flatten().filter_map(Value::as_str) {
            if obj.get(r).is_none_or(Value::is_null) {
                return Err(format!("{path}: {r} is required"));
            }
        }
    }
    if let (Some(items), Some(arr)) = (schema.get("items"), value.as_array()) {
        for (i, v) in arr.iter().enumerate() {
            fits(items, v, &format!("{path}[{i}]"))?;
        }
    }
    Ok(())
}

const VERBS: &[&str] = &[
    "List", "Read", "Get", "Create", "Update", "Delete", "Change", "Add", "Submit", "Ask", "Append", "Verify", "Generate", "Export", "Propose", "Approve", "Cancel", "Save", "Count", "Move", "Return",
    "Retrieve", "Aggregate", "Find", "Show", "Enqueue", "Draft",
];

#[tokio::test]
async fn every_tool_lists_a_valid_closed_schema_with_an_example_that_validates() {
    let _db = common::TestDb::new();
    let app = app();
    let tools = listed(&app).await;
    assert_eq!(tools.len(), TOOLS.len());
    let mut problems = Vec::new();
    for t in &tools {
        let name = t["name"].as_str().unwrap();
        let schema = &t["inputSchema"];
        if schema["type"] != "object" {
            problems.push(format!("{name}: inputSchema is not an object schema"));
        }
        if let Err(e) = check_schema(schema, name) {
            problems.push(e);
        }
        if schema["additionalProperties"] != json!(false) {
            problems.push(format!("{name}: inputSchema must set additionalProperties to false"));
        }
        let examples = schema["examples"].as_array().cloned().unwrap_or_default();
        if examples.is_empty() {
            problems.push(format!("{name}: no example"));
        }
        for (i, ex) in examples.iter().enumerate() {
            if let Err(e) = fits(schema, ex, &format!("{name} example {i}")) {
                problems.push(e);
            }
        }
        // An example is a valid call for the tool's own gate, so what the list shows is what works.
    }
    assert!(problems.is_empty(), "{} problem(s):\n{}", problems.len(), problems.join("\n"));
}

#[tokio::test]
async fn the_example_checker_rejects_a_wrong_example() {
    let schema = json!({ "type": "object", "properties": { "n": { "type": "integer" } }, "required": ["n"], "additionalProperties": false });
    assert!(fits(&schema, &json!({ "n": 1 }), "t").is_ok());
    assert!(fits(&schema, &json!({ "n": "one" }), "t").is_err(), "wrong type");
    assert!(fits(&schema, &json!({}), "t").is_err(), "missing required");
    assert!(fits(&schema, &json!({ "n": 1, "x": 2 }), "t").is_err(), "extra property");
    assert!(check_schema(&json!({ "type": "object", "properties": { "a": { "type": "text" } } }), "t").is_err(), "not a JSON Schema type");
    assert!(check_schema(&json!({ "type": "object", "properties": {}, "required": ["a"] }), "t").is_err(), "required names no property");
}

/// (name, read only, destructive, idempotent). Read-only tools carry no other hint.
const EXPECTED: &[(&str, bool, bool, bool)] = &[
    ("list_workspaces", true, false, false),
    ("get_workspace", true, false, false),
    ("update_workspace", false, false, true),
    ("manage_workspace_member", false, true, false),
    ("create_record", false, false, false),
    ("list_datasets", true, false, false),
    ("simulate_cedar_policy", true, false, false),
    ("record_governance_decision", false, false, false),
    ("governance_report", true, false, false),
    ("list_apps", true, false, false),
    ("describe_app", true, false, false),
    ("aggregate_records", true, false, false),
    ("list_records", true, false, false),
    ("get_view_data", true, false, false),
    ("get_record", true, false, false),
    ("get_page", true, false, false),
    ("page_call", false, true, false),
    ("update_record", false, false, true),
    ("delete_record", false, true, true),
    ("propose_app_change", false, false, false),
    ("list_proposals", true, false, false),
    ("get_proposal", true, false, false),
    ("decide_proposal", false, false, false),
    ("preview_proposal", false, false, false),
    ("get_agent_kit", true, false, false),
    ("explain_access", true, false, false),
    ("list_my_work", true, false, false),
    ("complete_task", false, false, false),
    ("decide_process", false, false, false),
    ("list_views", true, false, false),
    ("save_view", false, false, true),
    ("delete_view", false, true, true),
    ("get_job", true, false, false),
    ("cancel_job", false, true, true),
    ("begin_task", false, false, true),
    ("request_access", false, false, false),
    ("draft_access_change", false, false, false),
    ("access_suggestions", true, false, false),
    ("export_records", false, false, false),
    ("list_library_policies", true, false, false),
    ("read_library_snapshot", true, false, false),
    ("find_library_rules", true, false, false),
    ("propose_library_rules", false, false, false),
    ("list_library_questions", true, false, false),
    ("admin_inventory", true, false, false),
];

#[tokio::test]
async fn annotations_are_truthful_for_every_tool() {
    let _db = common::TestDb::new();
    let app = app();
    let tools = listed(&app).await;
    let mut names: Vec<&str> = TOOLS.iter().map(|t| t.name).collect();
    let mut expected: Vec<&str> = EXPECTED.iter().map(|e| e.0).collect();
    names.sort();
    expected.sort();
    assert_eq!(names, expected, "the table lists every tool and no other: a new tool needs a row");
    for (name, read_only, destructive, idempotent) in EXPECTED {
        let t = tools.iter().find(|t| t["name"] == *name).unwrap();
        let a = &t["annotations"];
        assert_eq!(a["readOnlyHint"], json!(read_only), "{name} readOnlyHint");
        assert_eq!(TOOLS.iter().find(|x| x.name == *name).unwrap().read_only, *read_only, "{name}: read_only in the registry");
        if *read_only {
            assert!(a.get("destructiveHint").is_none() && a.get("idempotentHint").is_none(), "{name}: a read-only tool carries no other hint: {a}");
        } else {
            assert_eq!(a["destructiveHint"], json!(destructive), "{name} destructiveHint");
            assert_eq!(a["idempotentHint"], json!(idempotent), "{name} idempotentHint");
        }
    }
}

#[tokio::test]
async fn every_description_starts_with_a_verb_and_stays_short() {
    let _db = common::TestDb::new();
    let app = app();
    for t in listed(&app).await {
        let name = t["name"].as_str().unwrap();
        let d = t["description"].as_str().unwrap();
        let first = d.split_whitespace().next().unwrap_or("").trim_matches(|c: char| !c.is_alphabetic());
        let verb = first.trim_end_matches('s');
        assert!(VERBS.iter().any(|v| *v == first || *v == verb), "{name}: the description starts with '{first}', not a verb: {d}");
        assert!(d.chars().count() <= 400, "{name}: the description is {} characters, the limit is 400", d.chars().count());
    }
}

// --- the budget ---------------------------------------------------------------------------------------------------

/// A client loads the whole list into its context on every session. About four bytes make a token, so 24 KB is about
/// 6,000 tokens. A change that needs more raises these numbers here, in the open, and says why.
/// The cap applies to every list an AGENT can receive. An agent token holds `core` plus at most 3 chosen toolsets, so
/// the largest agent list is a Platform Admin session with the 3 biggest toolsets. A desk sign-in token belongs to a
/// person, lists everything and is not capped. Its size is only printed (`docs/plans/mcp-toolsets.md`, phase 3).
const BYTE_BUDGET: usize = 24_000;
const TOOL_BUDGET: usize = 45;
const MAX_CHOSEN_TOOLSETS: usize = 3;

/// The byte limit of each toolset, measured as its entries in `tools/list` (a JSON array, as `serde_json` writes it).
/// A new tool goes inside its toolset's limit. Raising a limit is a decision, made here, in the open.
const TOOLSET_LIMITS: [(Toolset, usize); 6] = [
    (Toolset::Core, 5_000),
    (Toolset::Data, 7_600),
    (Toolset::Build, 6_200),
    (Toolset::Governance, 5_000),
    (Toolset::Access, 2_000),
    (Toolset::Admin, 1_500),
];

fn limit_of(set: Toolset) -> usize {
    TOOLSET_LIMITS.iter().find(|(s, _)| *s == set).map(|(_, l)| *l).expect("every toolset has a limit")
}

fn over_budget(tools: &[Value]) -> Option<String> {
    let bytes = serde_json::to_vec(tools).unwrap().len();
    if bytes > BYTE_BUDGET {
        return Some(format!("the tool list is {bytes} bytes, over the {BYTE_BUDGET} budget"));
    }
    if tools.len() > TOOL_BUDGET {
        return Some(format!("there are {} tools, over the {TOOL_BUDGET} budget", tools.len()));
    }
    None
}

fn bytes_of(tools: &[&Value]) -> usize {
    serde_json::to_vec(tools).unwrap().len()
}

fn in_set(tool: &Value, set: Toolset) -> bool {
    TOOLS.iter().any(|r| r.name == tool["name"].as_str().unwrap() && r.toolset == set)
}

/// A Platform Admin agent session at the Write level that was granted `sets` besides `core`.
fn agent_list_token(state: &scaffoldry_server::state::SharedState, sets: &[Toolset]) -> String {
    let _ = issue_test_token_and_user(ADMIN);
    let raw = format!("raw-{}", uuid::Uuid::new_v4());
    let scope = TokenScope { apps: Vec::new(), tables: Default::default(), tools: None, max_rows_per_call: None, level: Some(AgentLevel::Write), toolsets: Some(sets.to_vec()) };
    state
        .persist_api_token(&ApiToken {
            token_hash: hash_token(&raw),
            id: uuid::Uuid::new_v4(),
            kind: "agent".to_string(),
            eppn: ADMIN.to_string(),
            label: "budget test".to_string(),
            original_admin: None,
            created_at: chrono::Utc::now(),
            expires_at: chrono::Utc::now() + chrono::Duration::days(1),
            last_used_at: None,
            revoked_at: None,
            app_slug: None,
            scope: Some(scope),
        })
        .expect("persisted");
    raw
}

async fn list_with(app: &axum::Router, token: &str) -> Vec<Value> {
    let req = Request::builder()
        .method("POST")
        .uri("/api/v1/mcp")
        .header("content-type", "application/json")
        .header("authorization", format!("Bearer {token}"))
        .body(Body::from(json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/list" }).to_string()))
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    serde_json::from_slice::<Value>(&bytes).unwrap()["result"]["tools"].as_array().cloned().unwrap_or_default()
}

/// Every choice of up to 3 toolsets beyond `core`, as a list of toolsets.
fn choices() -> Vec<Vec<Toolset>> {
    let others: Vec<Toolset> = Toolset::ALL.into_iter().filter(|t| *t != Toolset::Core).collect();
    let mut out = Vec::new();
    for mask in 0u32..(1 << others.len()) {
        if mask.count_ones() as usize <= MAX_CHOSEN_TOOLSETS {
            out.push(others.iter().enumerate().filter(|(i, _)| mask & (1 << i) != 0).map(|(_, t)| *t).collect());
        }
    }
    out
}

#[tokio::test]
async fn the_largest_agent_list_stays_inside_the_budget_and_a_fixture_tool_pushes_it_over() {
    let db = common::TestDb::new();
    let state = db.state();
    let app = build_app_with_state(state.clone()).expect("router");
    let mut largest: Vec<Value> = Vec::new();
    for sets in choices() {
        let tools = list_with(&app, &agent_list_token(&state, &sets)).await;
        if serde_json::to_vec(&tools).unwrap().len() > serde_json::to_vec(&largest).unwrap().len() {
            largest = tools;
        }
    }
    eprintln!("largest agent list: {} tools, {} bytes", largest.len(), serde_json::to_vec(&largest).unwrap().len());
    assert_eq!(over_budget(&largest), None);

    let mut fat = largest.clone();
    fat.push(json!({ "name": "fixture", "description": "x".repeat(BYTE_BUDGET), "inputSchema": { "type": "object" } }));
    assert!(over_budget(&fat).unwrap().contains("bytes"), "a big tool is over the byte budget");
    // Small tools on their own, so the count is what goes over and the bytes cannot, however near the real list is to its budget.
    let many: Vec<Value> = (0..=TOOL_BUDGET).map(|_| json!({ "name": "f", "description": "d", "inputSchema": { "type": "object" } })).collect();
    assert!(over_budget(&many).unwrap().contains("tools"), "too many tools is over the count budget");
}

/// A desk sign-in token belongs to a person. It lists everything and is not capped, so this only prints its size.
#[tokio::test]
async fn the_desk_list_prints_its_size_and_is_not_capped() {
    let _db = common::TestDb::new();
    let app = app();
    let tools = listed(&app).await;
    eprintln!("desk list: {} tools, {} bytes (not capped: a desk token is a person, not an agent)", tools.len(), serde_json::to_vec(&tools).unwrap().len());
    assert_eq!(tools.len(), TOOLS.len(), "a desk token lists every tool");
}

/// The listed bytes of each toolset, each held to its limit. Printed so a change shows what it costs
/// (`cargo test -- --nocapture`).
#[tokio::test]
async fn each_toolset_stays_inside_its_own_limit() {
    let _db = common::TestDb::new();
    let app = app();
    let tools = listed(&app).await;
    let mut sum = 0;
    for set in Toolset::ALL {
        let mine: Vec<&Value> = tools.iter().filter(|t| in_set(t, set)).collect();
        let bytes = bytes_of(&mine);
        let limit = limit_of(set);
        eprintln!("toolset {:<10} {:>2} tools {:>6} bytes of {:>6} ({} free)", set.word(), mine.len(), bytes, limit, limit.saturating_sub(bytes));
        assert!(!mine.is_empty(), "{} lists no tool", set.word());
        assert!(bytes <= limit, "{} is {bytes} bytes, over its limit of {limit}", set.word());
        sum += mine.len();
    }
    assert_eq!(sum, tools.len(), "every listed tool is in a toolset");
}

/// The limits hold the cap by arithmetic: `core`, the 3 largest limits beyond it, and the JSON separators fit in 24,000.
#[test]
fn the_limits_add_up_to_inside_the_budget() {
    let mut others: Vec<usize> = TOOLSET_LIMITS.iter().filter(|(s, _)| *s != Toolset::Core).map(|(_, l)| *l).collect();
    others.sort_unstable_by(|a, b| b.cmp(a));
    let biggest: usize = others.iter().take(MAX_CHOSEN_TOOLSETS).sum();
    // The two brackets of the array, and a comma for each of the tools at most.
    let separators = 2 + TOOL_BUDGET;
    let total = limit_of(Toolset::Core) + biggest + separators;
    assert!(total <= BYTE_BUDGET, "core plus the 3 largest limits plus separators is {total}, over {BYTE_BUDGET}");
}

/// Every real grant is inside the cap: `core` plus any choice of up to 3 toolsets, as a Platform Admin agent session lists it.
#[tokio::test]
async fn every_agent_combination_stays_inside_24_kb_and_45_tools() {
    let db = common::TestDb::new();
    let state = db.state();
    let app = build_app_with_state(state.clone()).expect("router");
    let mut seen = 0;
    for sets in choices() {
        let tools = list_with(&app, &agent_list_token(&state, &sets)).await;
        let bytes = serde_json::to_vec(&tools).unwrap().len();
        let label: Vec<&str> = sets.iter().map(|s| s.word()).collect();
        assert!(bytes <= BYTE_BUDGET, "core + {label:?} lists {bytes} bytes, over {BYTE_BUDGET}");
        assert!(tools.len() <= TOOL_BUDGET, "core + {label:?} lists {} tools, over {TOOL_BUDGET}", tools.len());
        seen += 1;
    }
    assert_eq!(seen, 26, "1 + 5 + 10 + 10 choices of up to 3 of the 5 toolsets beyond core");
}
