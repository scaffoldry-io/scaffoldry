//! MCP apps phase 5: the business process desk is reachable from an agent. The same cases as the
//! REST process tests, run through `tools/call`.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::Router;
use http_body_util::BodyExt;
use scaffoldry_server::build_app_with_state;
use scaffoldry_server::service::identity::issue_test_token_and_user;
use scaffoldry_server::state::{CollaboratorRecord, ServerState};
use serde_json::{json, Value};
use std::sync::Arc;
use tokio::sync::Mutex;
use tower::ServiceExt;
use uuid::Uuid;

static LOCK: Mutex<()> = Mutex::const_new(());

const WORKSPACE: &str = "ws-physics-optics";
const OWNER: &str = "einstein@physics.state.edu";
const EDITOR: &str = "editor.test@physics.state.edu";
const OUTSIDER: &str = "sarah.connor@state.edu";

struct World {
    app: Router,
    owner: String,
    editor: String,
    outsider: String,
    slug: String,
}

async fn rest(app: &Router, method: &str, uri: &str, token: &str, body: Option<Value>) -> (StatusCode, Value) {
    let b = Request::builder().method(method).uri(uri).header("authorization", format!("Bearer {token}"));
    let req = match body {
        Some(v) => b.header("content-type", "application/json").body(Body::from(v.to_string())).unwrap(),
        None => b.body(Body::empty()).unwrap(),
    };
    let resp = app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
}

async fn tool(app: &Router, token: &str, name: &str, args: Value) -> (bool, Value) {
    let payload = json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": { "name": name, "arguments": args } });
    let (_, v) = rest(app, "POST", "/api/mcp", token, Some(payload)).await;
    if let Some(err) = v.get("error") {
        return (true, err["message"].clone());
    }
    let result = &v["result"];
    if result["isError"] == true {
        return (true, result["content"][0]["text"].clone());
    }
    (false, result["structuredContent"].clone())
}

/// An app with a rule that waits on the workspace owner, and a record the editor submitted.
/// Returns the world, the instance id, and the record id.
async fn world() -> (World, String, String) {
    let state = Arc::new(ServerState::new().expect("state"));
    state.seed_demo().expect("seed");
    state.collaborators.write().unwrap().entry(WORKSPACE.to_string()).or_default().push(CollaboratorRecord {
        id: format!("collab-{}", Uuid::new_v4()),
        workspace_id: WORKSPACE.to_string(),
        eppn: EDITOR.to_string(),
        name: "Edith Editor".to_string(),
        role: "editor".to_string(),
        scoped_affiliation: "faculty".to_string(),
        department: "physics".to_string(),
        added_at: String::new(),
    });
    let app = build_app_with_state(state).expect("router");
    let w = World {
        app,
        owner: issue_test_token_and_user(OWNER),
        editor: issue_test_token_and_user(EDITOR),
        outsider: issue_test_token_and_user(OUTSIDER),
        slug: format!("proc-{}", &Uuid::new_v4().to_string()[..8]),
    };
    let manifest = json!({
        "slug": w.slug, "title": "Process", "description": "d", "organization_code": "PHYS", "department": "physics", "views": [],
        "tables": [{ "id": "apps", "name": "Applications", "slug": "apps", "fields": [
            { "name": "name", "label": "Name", "field_type": "Text", "required": false, "ferpa_sensitive": false }
        ]}]
    });
    let (s, body) = rest(&w.app, "POST", &format!("/api/v1/workspaces/{WORKSPACE}/apps"), &w.owner, Some(manifest)).await;
    assert_eq!(s, StatusCode::CREATED, "{body}");
    let rule_id = format!("r-{}", &Uuid::new_v4().to_string()[..8]);
    let rule = json!({
        "id": rule_id, "name": "Review", "enabled": true, "trigger": "RecordCreated",
        "steps": [{ "id": "decide", "when": [], "kind": { "UserTask": {
            "role": "owner", "prompt": "Decide",
            "approve": [{ "UpdateRecordStatus": { "new_status": "Admitted" } }],
            "reject": [{ "UpdateRecordStatus": { "new_status": "Denied" } }],
            "approver": { "Role": { "name": "owner" } }
        }}}]
    });
    let (s, body) = rest(&w.app, "POST", &format!("/api/v1/apps/{}/automations", w.slug), &w.owner, Some(rule)).await;
    assert_eq!(s, StatusCode::CREATED, "{body}");
    let (s, created) = rest(&w.app, "POST", &format!("/api/v1/apps/{}/records", w.slug), &w.editor, Some(json!({ "data": { "_table_id": "apps", "name": "A" } }))).await;
    assert_eq!(s, StatusCode::CREATED, "{created}");
    let record_id = created["id"].as_str().unwrap().to_string();
    (w, format!("{rule_id}:{record_id}"), record_id)
}

#[tokio::test]
async fn list_waiting_decisions_shows_what_the_caller_may_decide_and_nothing_else() {
    let _turn = LOCK.lock().await;
    let (w, instance, _) = world().await;
    let (err, mine) = tool(&w.app, &w.owner, "list_waiting_decisions", json!({})).await;
    assert!(!err, "{mine}");
    assert!(mine.as_array().unwrap().iter().any(|i| i["id"] == instance.as_str()), "the owner decides it: {mine}");

    let (_, submitter) = tool(&w.app, &w.editor, "list_waiting_decisions", json!({})).await;
    assert!(!submitter.as_array().unwrap().iter().any(|i| i["id"] == instance.as_str()), "the submitter never decides their own");
    let (_, outsider) = tool(&w.app, &w.outsider, "list_waiting_decisions", json!({})).await;
    assert!(!outsider.as_array().unwrap().iter().any(|i| i["id"] == instance.as_str()), "a person who cannot read the app sees nothing of it");
}

#[tokio::test]
async fn decide_process_applies_the_step_and_an_instance_cannot_be_decided_twice() {
    let _turn = LOCK.lock().await;
    let (w, instance, record_id) = world().await;
    let args = |d: &str| json!({ "app_slug": w.slug, "instance_id": instance, "decision": d });

    let (err, msg) = tool(&w.app, &w.editor, "decide_process", args("approve")).await;
    assert!(err, "the submitter may not decide: {msg}");

    let (err, done) = tool(&w.app, &w.owner, "decide_process", args("approve")).await;
    assert!(!err, "{done}");
    assert_eq!(done["status"], "Completed");
    let (_, record) = rest(&w.app, "GET", &format!("/api/v1/apps/{}/records/{record_id}", w.slug), &w.owner, None).await;
    assert_eq!(record["data"]["status"], "Admitted", "{record}");

    let (err, msg) = tool(&w.app, &w.owner, "decide_process", args("approve")).await;
    assert!(err, "{msg}");
    assert!(msg.as_str().unwrap().contains("Waiting"), "{msg}");
}

#[tokio::test]
async fn decide_process_refuses_a_caller_who_cannot_read_the_app_and_a_bad_decision() {
    let _turn = LOCK.lock().await;
    let (w, instance, _) = world().await;
    let (err, _) = tool(&w.app, &w.outsider, "decide_process", json!({ "app_slug": w.slug, "instance_id": instance, "decision": "approve" })).await;
    assert!(err, "no read access to the app");
    let (err, msg) = tool(&w.app, &w.owner, "decide_process", json!({ "app_slug": w.slug, "instance_id": instance, "decision": "maybe" })).await;
    assert!(err);
    assert!(msg.as_str().unwrap().contains("approve"), "{msg}");
    let (err, _) = tool(&w.app, &w.owner, "decide_process", json!({ "app_slug": w.slug, "instance_id": "no-such:instance", "decision": "approve" })).await;
    assert!(err);
}

#[tokio::test]
async fn the_rest_route_and_the_tool_decide_the_same_way() {
    let _turn = LOCK.lock().await;
    let (w, instance, record_id) = world().await;
    let (s, body) = rest(&w.app, "POST", &format!("/api/v1/apps/{}/processes/{}/decide", w.slug, instance.replace(':', "%3A")), &w.editor, Some(json!({ "decision": "approve" }))).await;
    assert_eq!(s, StatusCode::FORBIDDEN, "{body}");
    let (s, body) = rest(&w.app, "POST", &format!("/api/v1/apps/{}/processes/{}/decide", w.slug, instance.replace(':', "%3A")), &w.owner, Some(json!({ "decision": "reject" }))).await;
    assert_eq!(s, StatusCode::OK, "{body}");
    assert_eq!(body["status"], "Rejected");
    let (_, record) = rest(&w.app, "GET", &format!("/api/v1/apps/{}/records/{record_id}", w.slug), &w.owner, None).await;
    assert_eq!(record["data"]["status"], "Denied");
}
