//! MCP apps phase 8: pages and review screens inside the agent.
//!
//! Follows the MCP Apps extension, SEP-1865, specification 2026-01-26 (stable), from `modelcontextprotocol/ext-apps`.
//! A custom page and two built-in screens are `ui://` resources with the MIME type `text/html;profile=mcp-app`. A tool
//! links to its screen by `_meta.ui.resourceUri` on its definition. A view says `ui/initialize`, waits for the answer, and
//! says `ui/notifications/initialized` before it sends anything else. The harness in `tests/mcp_ui/harness.cjs` plays the
//! host around a document in jsdom, so the screens are run, not only read.

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::Router;
use http_body_util::BodyExt;
use scaffoldry_server::build_app_with_state;
use scaffoldry_server::service::identity::issue_test_token_and_user;
use scaffoldry_server::state::CollaboratorRecord;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use tower::ServiceExt;
use uuid::Uuid;

const WORKSPACE: &str = "ws-physics-optics";
const OWNER: &str = "einstein@physics.state.edu";
const VIEWER: &str = "viewer.p8@physics.state.edu";
const STRANGER: &str = "stranger.p8@elsewhere.example.edu";
const SOURCE: &str = "export default function Page() { return <p>AWARD-DASHBOARD-MARKER</p>; }";
const REVIEW_URI: &str = "ui://scaffoldry/proposal-review";
const DECISION_URI: &str = "ui://scaffoldry/decision";
const MIME: &str = "text/html;profile=mcp-app";

struct World {
    _db: common::TestDb,
    app: Router,
    owner: String,
    stranger: String,
    slug: String,
}

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

async fn world() -> World {
    let db = common::TestDb::new();
    let state = db.state();
    state.seed_demo().expect("seed");
    state.collaborators.write().unwrap().entry(WORKSPACE.to_string()).or_default().push(CollaboratorRecord {
        id: format!("collab-{}", Uuid::new_v4()),
        workspace_id: WORKSPACE.to_string(),
        eppn: VIEWER.to_string(),
        name: "Vera Viewer".to_string(),
        role: "viewer".to_string(),
        scoped_affiliation: "faculty".to_string(),
        department: "physics".to_string(),
        added_at: String::new(),
        app_slug: String::new(),
    });
    let app = build_app_with_state(state).expect("router");
    World { _db: db, app, owner: issue_test_token_and_user(OWNER), stranger: issue_test_token_and_user(STRANGER), slug: format!("ui-{}", &Uuid::new_v4().to_string()[..8]) }
}

fn sha(text: &str) -> String {
    Sha256::digest(text.as_bytes()).iter().map(|b| format!("{b:02x}")).collect()
}

async fn rpc(app: &Router, token: &str, method: &str, params: Value) -> Value {
    let body = json!({ "jsonrpc": "2.0", "id": 1, "method": method, "params": params });
    let req = Request::builder().method("POST").uri("/api/mcp").header("authorization", format!("Bearer {token}")).header("content-type", "application/json").body(Body::from(body.to_string())).unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    serde_json::from_slice(&resp.into_body().collect().await.unwrap().to_bytes()).unwrap()
}

/// Calls a tool. `Err` is the tool's own message.
async fn tool(w: &World, token: &str, name: &str, args: Value) -> Result<Value, String> {
    let v = rpc(&w.app, token, "tools/call", json!({ "name": name, "arguments": args })).await;
    if let Some(err) = v.get("error") {
        return Err(err["message"].as_str().unwrap_or("").to_string());
    }
    if v["result"]["isError"] == true {
        return Err(v["result"]["content"][0]["text"].as_str().unwrap_or("").to_string());
    }
    Ok(v["result"]["structuredContent"].clone())
}

fn field(name: &str, t: &str) -> Value {
    json!({ "name": name, "label": name, "field_type": t, "required": false, "ferpa_sensitive": false })
}

fn manifest(slug: &str, pages: Vec<Value>) -> Value {
    json!({
        "slug": slug, "title": "Pages", "description": "d", "organization_code": "PHYS", "department": "physics",
        "views": [], "tables": [{ "id": "grants", "name": "Grants", "slug": "grants", "fields": [field("title", "Text"), field("amount", "Number")] }],
        "pages": pages
    })
}

fn custom_page() -> Value {
    json!({ "id": "board", "title": "Board", "kind": { "Custom": { "source_sha256": sha(SOURCE), "grants": [
        { "table_id": "grants", "read": ["title"], "write": [], "create": false, "delete": false }
    ]}}})
}

/// Proposes `manifest` and returns the proposal as `get_proposal` shows it.
async fn propose(w: &World, manifest: Value, summary: &str, sources: Vec<&str>) -> Value {
    let proposed = tool(w, &w.owner, "propose_app_change", json!({ "workspace_id": WORKSPACE, "manifest": manifest, "summary": summary, "page_sources": sources })).await.expect("propose");
    tool(w, &w.owner, "get_proposal", json!({ "proposal_id": proposed["proposal_id"] })).await.expect("get_proposal")
}

async fn publish_page(w: &World) {
    let p = propose(w, manifest(&w.slug, vec![custom_page()]), "Pages", vec![SOURCE]).await;
    tool(w, &w.owner, "decide_proposal", json!({ "proposal_id": p["proposal_id"], "decision": "approve", "rationale": "Reviewed", "digest": p["digest"] })).await.expect("approve");
}

fn page_uri(w: &World) -> String {
    format!("ui://scaffoldry/apps/{}/pages/board", w.slug)
}

fn node(scenario: &Value) -> Result<Value, String> {
    let dir = std::env::temp_dir().join(format!("mcp-ui-{}", Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("scenario.json");
    std::fs::write(&file, scenario.to_string()).unwrap();
    let harness = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/mcp_ui/harness.cjs");
    let out = std::process::Command::new("node").arg(&harness).arg(&file).arg(root()).output().expect("run node");
    if !out.status.success() {
        return Err(format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr)));
    }
    serde_json::from_slice(&out.stdout).map_err(|e| format!("{e}: {}", String::from_utf8_lossy(&out.stdout)))
}

fn screen_file(name: &str) -> String {
    std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("src/ui").join(name)).unwrap_or_else(|e| panic!("{name}: {e}"))
}

// 1. Each built-in screen is read as its file, with the MIME type, and holds no data.
#[tokio::test]
async fn the_built_in_screens_are_static_files_with_the_apps_mime_type() {
    let w = world().await;
    for (uri, file) in [(REVIEW_URI, "proposal-review.html"), (DECISION_URI, "decision.html")] {
        let v = rpc(&w.app, &w.owner, "resources/read", json!({ "uri": uri })).await;
        let content = &v["result"]["contents"][0];
        assert_eq!(content["uri"], uri, "{v}");
        assert_eq!(content["mimeType"], MIME, "{v}");
        let text = content["text"].as_str().expect("the document");
        assert_eq!(text, screen_file(file), "{uri} is its file, byte for byte");
        assert!(!text.contains("{{"), "{uri}: no template holes");
        for data in ["einstein", "@state.edu", "@physics", "Telescope"] {
            assert!(!text.contains(data), "{uri}: no data ({data})");
        }
        assert!(text.to_lowercase().starts_with("<!doctype html>"), "{uri}: a valid HTML5 document");
        assert!(!text.contains("innerHTML") && !text.contains("document.write"), "{uri}: it builds no HTML from data");
    }
    // They are listed, so a host can find them.
    let listed = rpc(&w.app, &w.owner, "resources/list", json!({})).await;
    let uris: Vec<&str> = listed["result"]["resources"].as_array().unwrap().iter().filter_map(|r| r["uri"].as_str()).collect();
    assert!(uris.contains(&REVIEW_URI) && uris.contains(&DECISION_URI), "{uris:?}");
}

// 2. A custom page's resource is the same string get_page returns.
#[tokio::test]
async fn a_custom_page_resource_is_the_string_get_page_returns() {
    let w = world().await;
    publish_page(&w).await;
    let page = tool(&w, &w.owner, "get_page", json!({ "app_slug": w.slug, "page_id": "board" })).await.expect("get_page");
    let v = rpc(&w.app, &w.owner, "resources/read", json!({ "uri": page_uri(&w) })).await;
    let content = &v["result"]["contents"][0];
    assert_eq!(content["mimeType"], MIME, "{v}");
    assert_eq!(content["text"], page["html"], "the resource and the tool say the same thing");
    assert_eq!(page["resource_uri"], page_uri(&w).as_str(), "get_page names its resource: {page}");
    let listed = rpc(&w.app, &w.owner, "resources/list", json!({})).await;
    assert!(listed["result"]["resources"].as_array().unwrap().iter().any(|r| r["uri"] == page_uri(&w).as_str() && r["mimeType"] == MIME), "{listed}");
}

// 3. A caller with no role gets an error, and the URI is not in their list.
#[tokio::test]
async fn a_caller_with_no_role_gets_an_error_and_never_sees_the_page() {
    let w = world().await;
    publish_page(&w).await;
    let v = rpc(&w.app, &w.stranger, "resources/read", json!({ "uri": page_uri(&w) })).await;
    assert!(v.get("error").is_some(), "an error, not a document: {v}");
    assert!(!v.to_string().contains("AWARD-DASHBOARD-MARKER") && v.get("result").is_none(), "{v}");
    let listed = rpc(&w.app, &w.stranger, "resources/list", json!({})).await;
    assert!(!listed.to_string().contains(&w.slug), "the page is not listed: {listed}");
    // A page that does not exist looks the same.
    let missing = rpc(&w.app, &w.owner, "resources/read", json!({ "uri": format!("ui://scaffoldry/apps/{}/pages/nope", w.slug) })).await;
    assert!(missing.get("error").is_some(), "{missing}");
}

// A tool links to its screen the way the specification states: `_meta.ui.resourceUri` on its definition.
#[tokio::test]
async fn two_tools_link_to_their_screens_in_the_way_the_specification_states() {
    let w = world().await;
    let listed = rpc(&w.app, &w.owner, "tools/list", json!({})).await;
    let tools = listed["result"]["tools"].as_array().unwrap();
    let link = |name: &str| tools.iter().find(|t| t["name"] == name).map(|t| t["_meta"].clone()).unwrap_or(Value::Null);
    assert_eq!(link("get_proposal")["ui"]["resourceUri"], REVIEW_URI);
    assert_eq!(link("list_waiting_decisions")["ui"]["resourceUri"], DECISION_URI);
    assert!(link("get_proposal").get("ui/resourceUri").is_none(), "the flat key is deprecated, and is not used");
    // Both screens call tools, so those tools must stay visible to an app (the default is model and app).
    for name in ["decide_proposal", "decide_process", "get_record"] {
        assert!(tools.iter().any(|t| t["name"] == name), "{name} is listed");
        assert!(tools.iter().find(|t| t["name"] == name).unwrap()["_meta"].get("ui").is_none(), "{name} keeps the default visibility");
    }
}

// The handshake: ui/initialize, the answer, then ui/notifications/initialized. No tools/call before it is over.
#[tokio::test]
async fn the_bridge_finishes_the_handshake_before_a_page_calls_a_tool() {
    let bridge = std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("src/ui/bridge.js")).unwrap();
    let html = format!(
        "<!doctype html><html><body><script type=\"application/json\" id=\"page-meta\">{}</script><script>{bridge}</script></body></html>",
        json!({ "app": { "slug": "a", "page_id": "board", "title": "T" }, "grants": [], "format": "module" })
    );
    let out = node(&json!({ "kind": "bridge", "html": html })).expect("harness");
    assert_eq!(out["firstMethod"], "ui/initialize", "{out}");
    assert_eq!(out["initialize"]["params"]["protocolVersion"], "2026-01-26", "{out}");
    assert!(out["initialize"]["params"]["appCapabilities"].is_object(), "the capabilities are declared: {out}");
    assert_eq!(out["callsBeforeHandshake"], 0, "no tools/call before the handshake: {out}");
    assert_eq!(out["initializedSent"], true, "{out}");
    let calls = out["callsAfterHandshake"].as_array().unwrap();
    assert_eq!(calls.len(), 1, "the waiting call goes out once the handshake is over: {out}");
    assert_eq!(calls[0]["name"], "page_call");
}

// The proposal screen renders what the host gives it as text, and approves with the digest it was shown.
#[tokio::test]
async fn the_proposal_review_screen_shows_the_proposal_as_text_and_approves_with_its_digest() {
    let w = world().await;
    let hostile = "<img src=x onerror=alert(1)> Add a grants page";
    let proposal = propose(&w, manifest(&w.slug, vec![custom_page()]), hostile, vec![SOURCE]).await;
    let html = screen_file("proposal-review.html");
    let scenario = |press: Vec<&str>, rationale: &str| {
        json!({
            "kind": "screen", "html": html, "input": { "proposal_id": proposal["proposal_id"] },
            "result": { "content": [{ "type": "text", "text": "proposal" }], "structuredContent": proposal },
            "rationale": rationale, "press": press, "reply": { "structuredContent": { "status": "Approved" } }
        })
    };
    let out = node(&scenario(vec!["Approve"], "Checked the page grants")).expect("harness");
    assert_eq!(out["firstMethod"], "ui/initialize", "{out}");
    assert_eq!(out["initializedSent"], true);
    assert_eq!(out["callsBeforeResult"], 0, "the screen asks for nothing before it has the result: {out}");
    let text = out["text"].as_str().unwrap();
    assert!(text.contains("<img src=x onerror=alert(1)>"), "the summary is shown as text, tags and all: {text}");
    assert_eq!(out["imgCount"], 0, "no element came from the data: {out}");
    assert_eq!(out["inlineHandlers"], 0, "{out}");
    for needed in ["Approve", "Reject"] {
        assert!(out["buttons"].as_array().unwrap().iter().any(|b| b == needed), "{needed}: {out}");
    }
    assert!(text.contains(proposal["changes"][0].as_str().unwrap_or("a change")) || proposal["changes"].as_array().is_none_or(|c| c.is_empty()), "the changes are listed: {text}");
    let call = &out["clicks"][0]["call"];
    assert_eq!(call["name"], "decide_proposal", "{out}");
    assert_eq!(call["arguments"]["proposal_id"], proposal["proposal_id"]);
    assert_eq!(call["arguments"]["decision"], "approve");
    assert_eq!(call["arguments"]["digest"], proposal["digest"], "the digest it was shown: {out}");
    assert_eq!(call["arguments"]["rationale"], "Checked the page grants");
    assert!(out["textAfter"].as_str().unwrap().contains("Approved"), "{out}");

    // Reject names no digest. An approval with no reason is not sent.
    let rejected = node(&scenario(vec!["Reject"], "Not now")).expect("harness");
    assert_eq!(rejected["clicks"][0]["call"]["arguments"]["decision"], "reject", "{rejected}");
    let silent = node(&scenario(vec!["Approve"], "")).expect("harness");
    assert_eq!(silent["clicks"][0]["call"], Value::Null, "no call without a reason: {silent}");
    assert_eq!(silent["calls"].as_array().map(Vec::len), Some(0), "{silent}");
}

// The decision screen shows the prompt and the record's fields as text, and decides the step.
#[tokio::test]
async fn the_decision_screen_shows_the_prompt_and_record_and_decides_the_step() {
    let items = json!([{
        "id": "rule-1:rec-9", "rule_id": "rule-1", "app_slug": "scholarships", "record_id": "rec-9", "status": "Waiting",
        "prompt": "Approve <b>this</b> award?", "role": "chair", "approvers": [{ "eppn": "chair@x.edu", "display_name": "Chair", "via": "Named" }]
    }]);
    let scenario = json!({
        "kind": "screen", "html": screen_file("decision.html"), "input": {},
        "result": { "content": [], "structuredContent": { "items": items } },
        "record": { "id": "rec-9", "data": { "applicant": "Ada <i>Lovelace</i>", "amount": 4000 } },
        "press": ["Approve"], "reply": { "structuredContent": { "status": "Completed" } }
    });
    let out = node(&scenario).expect("harness");
    assert_eq!(out["firstMethod"], "ui/initialize", "{out}");
    let text = out["text"].as_str().unwrap();
    assert!(text.contains("Approve <b>this</b> award?"), "the prompt is text: {text}");
    assert!(text.contains("Ada <i>Lovelace</i>") && text.contains("4000"), "the record's fields: {text}");
    assert_eq!(out["imgCount"], 0);
    let reads: Vec<&Value> = out["calls"].as_array().unwrap().iter().filter(|c| c["name"] == "get_record").collect();
    assert_eq!(reads.len(), 1, "it reads the record once: {out}");
    assert_eq!(reads[0]["arguments"]["record_id"], "rec-9");
    let call = &out["clicks"][0]["call"];
    assert_eq!(call["name"], "decide_process", "{out}");
    assert_eq!(call["arguments"], json!({ "app_slug": "scholarships", "instance_id": "rule-1:rec-9", "decision": "approve" }));
    // An empty list says so.
    let none = node(&json!({ "kind": "screen", "html": screen_file("decision.html"), "input": {}, "result": { "content": [], "structuredContent": { "items": [] } } })).expect("harness");
    assert!(none["text"].as_str().unwrap().to_lowercase().contains("nothing"), "{none}");
}
