//! Integrations phase 2: a token that can be narrower than its owner.
//!
//! A token may carry a scope: apps, tables with operations, tools, and a row cap. It can never exceed its owner:
//! the effective right is the intersection. NIST SP 800-53 AC-6 (least privilege) and IA-5.

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::Router;
use http_body_util::BodyExt;
use scaffoldry_server::build_app_with_state;
use scaffoldry_server::service::identity::issue_test_token_and_user;
use scaffoldry_server::state::CollaboratorRecord;
use serde_json::{json, Value};
use tower::ServiceExt;

const WORKSPACE: &str = "ws-physics-optics";
const OWNER: &str = "einstein@physics.state.edu";

struct World {
    _db: common::TestDb,
    app: Router,
    owner: String,
    viewer: String,
    a: String,
    b: String,
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

/// `Err` holds the error code and the message.
async fn tool(w: &World, token: &str, name: &str, args: Value) -> Result<Value, (String, String)> {
    let payload = json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": { "name": name, "arguments": args } });
    let (_, v) = rest(&w.app, "POST", "/api/mcp", token, Some(payload)).await;
    if let Some(err) = v.get("error") {
        return Err((String::new(), err["message"].as_str().unwrap_or("").to_string()));
    }
    if v["result"]["isError"] == true {
        let code = v["result"]["structuredContent"]["code"].as_str().unwrap_or("").to_string();
        return Err((code, v["result"]["content"][0]["text"].as_str().unwrap_or("").to_string()));
    }
    Ok(v["result"]["structuredContent"].clone())
}

fn field(name: &str) -> Value {
    json!({ "name": name, "label": name, "field_type": "Text", "required": false, "ferpa_sensitive": false })
}

fn manifest(slug: &str) -> Value {
    json!({
        "slug": slug, "title": "Scoped", "description": "d", "organization_code": "PHYS", "department": "physics",
        "workspace_id": WORKSPACE, "views": [], "pages": [],
        "tables": [
            { "id": "cases", "name": "Cases", "slug": "cases", "fields": [field("title")] },
            { "id": "notes", "name": "Notes", "slug": "notes", "fields": [field("body")] }
        ]
    })
}

async fn make_app(w: &World, slug: &str) {
    let p = tool(w, &w.owner, "propose_app_change", json!({ "workspace_id": WORKSPACE, "manifest": manifest(slug), "summary": "An app" })).await.expect("proposed");
    tool(w, &w.owner, "decide_proposal", json!({ "proposal_id": p["proposal_id"], "decision": "approve", "rationale": "Set up", "digest": p["digest"] })).await.expect("live");
    for i in 0..5 {
        tool(w, &w.owner, "create_record", json!({ "app_slug": slug, "data": { "_table_id": "cases", "title": format!("case {i}") } })).await.expect("record");
    }
    tool(w, &w.owner, "create_record", json!({ "app_slug": slug, "data": { "_table_id": "notes", "body": "a note" } })).await.expect("note");
}

async fn world() -> World {
    let db = common::TestDb::new();
    let state = db.state();
    state.seed_demo().expect("seed");
    let tag = uuid::Uuid::new_v4().to_string()[..8].to_string();
    let viewer = format!("viewer-{tag}@state.edu");
    state.collaborators.write().unwrap().entry(WORKSPACE.to_string()).or_default().push(CollaboratorRecord {
        id: uuid::Uuid::new_v4().to_string(),
        workspace_id: WORKSPACE.to_string(),
        eppn: viewer.clone(),
        name: viewer.clone(),
        role: "viewer".to_string(),
        scoped_affiliation: "faculty".to_string(),
        department: "physics".to_string(),
        added_at: String::new(),
        app_slug: String::new(),
    });
    let app = build_app_with_state(state).expect("router");
    let w = World { _db: db, app, owner: issue_test_token_and_user(OWNER), viewer: issue_test_token_and_user(&viewer), a: format!("ts-a-{tag}"), b: format!("ts-b-{tag}") };
    make_app(&w, &w.a).await;
    make_app(&w, &w.b).await;
    w
}

/// A token for the holder of `who`, limited by `scope`. Null means the owner's full rights.
async fn mint(w: &World, who: &str, scope: Value) -> (StatusCode, Value) {
    let mut body = json!({ "label": "an agent", "kind": "agent" });
    if !scope.is_null() {
        body["scope"] = scope;
    }
    rest(&w.app, "POST", "/api/v1/auth/tokens", who, Some(body)).await
}

async fn scoped(w: &World, who: &str, scope: Value) -> String {
    let (status, body) = mint(w, who, scope).await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    body["token"].as_str().expect("a token").to_string()
}

fn count(v: &Value) -> usize {
    v["records"].as_array().map(Vec::len).unwrap_or(0)
}

// 1. A token scoped to one app is refused on another, with token_scope.
#[tokio::test]
async fn a_token_scoped_to_one_app_is_refused_on_another() {
    let w = world().await;
    let t = scoped(&w, &w.owner, json!({ "apps": [w.a], "tables": { "cases": ["read"] } })).await;
    let (status, body) = rest(&w.app, "GET", &format!("/api/v1/apps/{}/tables/cases/records", w.a), &t, None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let (status, body) = rest(&w.app, "GET", &format!("/api/v1/apps/{}/tables/cases/records", w.b), &t, None).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert_eq!(body["code"], "token_scope", "{body}");
    assert!(body["error"].as_str().unwrap().contains(&w.a), "the message says what the token may do: {body}");
    let (code, _) = tool(&w, &t, "list_records", json!({ "app_slug": w.b, "table_id": "cases" })).await.unwrap_err();
    assert_eq!(code, "token_scope");
    assert!(tool(&w, &t, "list_records", json!({ "app_slug": w.a, "table_id": "cases" })).await.is_ok());
}

// 2. A token scoped to read on one table cannot create or update there, and cannot read another table.
#[tokio::test]
async fn a_table_scope_limits_the_operations_and_the_tables() {
    let w = world().await;
    let t = scoped(&w, &w.owner, json!({ "apps": [w.a], "tables": { "cases": ["read"] } })).await;
    let first = tool(&w, &t, "list_records", json!({ "app_slug": w.a, "table_id": "cases" })).await.expect("a read");
    let id = first["records"][0]["id"].as_str().unwrap().to_string();
    let version = first["records"][0]["version"].clone();

    let (code, _) = tool(&w, &t, "create_record", json!({ "app_slug": w.a, "data": { "_table_id": "cases", "title": "new" } })).await.unwrap_err();
    assert_eq!(code, "token_scope", "create");
    let (code, _) = tool(&w, &t, "update_record", json!({ "app_slug": w.a, "record_id": id, "version": version, "data": { "title": "changed" } })).await.unwrap_err();
    assert_eq!(code, "token_scope", "update");
    let (code, _) = tool(&w, &t, "list_records", json!({ "app_slug": w.a, "table_id": "notes" })).await.unwrap_err();
    assert_eq!(code, "token_scope", "another table");
    let (status, body) = rest(&w.app, "POST", &format!("/api/v1/apps/{}/tables/cases/records", w.a), &t, Some(json!({ "data": { "title": "new" } }))).await;
    assert_eq!((status, body["code"].as_str()), (StatusCode::FORBIDDEN, Some("token_scope")), "{body}");
    let (status, body) = rest(&w.app, "GET", &format!("/api/v1/apps/{}/tables/notes/records", w.a), &t, None).await;
    assert_eq!((status, body["code"].as_str()), (StatusCode::FORBIDDEN, Some("token_scope")), "{body}");

    let (_, live) = rest(&w.app, "GET", &format!("/api/v1/apps/{}/tables/cases/records", w.a), &w.owner, None).await;
    assert_eq!(count(&live), 5, "nothing was added");
}

// 3. The effective right is the intersection with the owner's rights.
#[tokio::test]
async fn the_effective_right_is_the_intersection_with_the_owner() {
    let w = world().await;
    let (_, rows) = rest(&w.app, "GET", &format!("/api/v1/apps/{}/tables/cases/records", w.a), &w.owner, None).await;
    let id = rows["records"][0]["id"].as_str().unwrap().to_string();
    let version = rows["records"][0]["version"].clone();
    // A viewer can read. A token that may update, minted by the viewer, still cannot.
    let t = scoped(&w, &w.viewer, json!({ "apps": [w.a], "tables": { "cases": ["read", "update"] } })).await;
    let err = tool(&w, &t, "update_record", json!({ "app_slug": w.a, "record_id": id, "version": version, "data": { "title": "changed" } })).await.unwrap_err();
    assert_ne!(err.0, "token_scope", "the scope allows it, so the owner's rights are what refuse it: {err:?}");
    let (_, after) = rest(&w.app, "GET", &format!("/api/v1/apps/{}/records/{id}", w.a), &w.owner, None).await;
    assert_ne!(after["data"]["title"], "changed", "{after}");
}

// 4. A scoped token cannot call an admin tool and cannot mint a token.
#[tokio::test]
async fn a_scoped_token_cannot_call_admin_tools_or_mint_tokens() {
    let w = world().await;
    let t = scoped(&w, &w.owner, json!({ "apps": [w.a], "tables": { "cases": ["read", "create", "update", "delete"] } })).await;
    for (name, args) in [
        ("propose_app_change", json!({ "workspace_id": WORKSPACE, "manifest": manifest(&w.a), "summary": "x" })),
        ("decide_proposal", json!({ "proposal_id": uuid::Uuid::new_v4(), "decision": "reject", "rationale": "x" })),
        ("list_workspaces", json!({})),
        ("manage_workspace_member", json!({ "workspace_id": WORKSPACE, "eppn": "a@b.c", "action": "add", "role": "viewer" })),
    ] {
        let (code, _) = tool(&w, &t, name, args).await.unwrap_err();
        assert_eq!(code, "token_scope", "{name}");
    }
    let (status, body) = rest(&w.app, "POST", "/api/v1/auth/tokens", &t, Some(json!({ "label": "wider", "kind": "agent" }))).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    // A tool list that names an admin tool does not allow it. The tool is not a data tool.
    let (status, body) = mint(&w, &w.owner, json!({ "apps": [w.a], "tables": { "cases": ["read"] }, "tools": ["list_records", "propose_app_change"] })).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
}

// 5. max_rows_per_call caps a list.
#[tokio::test]
async fn max_rows_per_call_caps_a_list() {
    let w = world().await;
    let t = scoped(&w, &w.owner, json!({ "apps": [w.a], "tables": { "cases": ["read"] }, "max_rows_per_call": 2 })).await;
    let listed = tool(&w, &t, "list_records", json!({ "app_slug": w.a, "table_id": "cases", "limit": 50 })).await.expect("a read");
    assert_eq!(count(&listed), 2, "{listed}");
    let listed = tool(&w, &t, "list_records", json!({ "app_slug": w.a, "table_id": "cases" })).await.expect("a read");
    assert_eq!(count(&listed), 2, "no limit asked: {listed}");
    let (status, body) = rest(&w.app, "GET", &format!("/api/v1/apps/{}/tables/cases/records?limit=50", w.a), &t, None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(count(&body), 2, "{body}");
    let (_, body) = rest(&w.app, "GET", &format!("/api/v1/apps/{}/tables/cases/records", w.a), &t, None).await;
    assert_eq!(count(&body), 2, "{body}");
}

// 6. A null scope behaves as before.
#[tokio::test]
async fn a_token_with_no_scope_has_its_owners_rights() {
    let w = world().await;
    let t = scoped(&w, &w.owner, Value::Null).await;
    let listed = tool(&w, &t, "list_records", json!({ "app_slug": w.b, "table_id": "notes" })).await.expect("a read");
    assert_eq!(count(&listed), 1);
    assert!(tool(&w, &t, "create_record", json!({ "app_slug": w.a, "data": { "_table_id": "cases", "title": "ok" } })).await.is_ok());
}

// The scope is checked when the token is made, and the token list shows it.
#[tokio::test]
async fn a_bad_scope_is_refused_and_the_list_shows_the_scope() {
    let w = world().await;
    for bad in [
        json!({ "apps": [], "tables": {} }),
        json!({ "apps": [w.a], "tables": { "cases": ["read", "explode"] } }),
        json!({ "apps": [w.a], "tables": { "cases": ["read"] }, "tools": ["no_such_tool"] }),
        json!({ "apps": [w.a], "tables": { "cases": ["read"] }, "max_rows_per_call": 0 }),
    ] {
        let (status, body) = mint(&w, &w.owner, bad.clone()).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{bad}: {body}");
    }
    let scope = json!({ "apps": [w.a], "tables": { "cases": ["read", "create"] }, "max_rows_per_call": 10 });
    scoped(&w, &w.owner, scope).await;
    let (_, listed) = rest(&w.app, "GET", "/api/v1/auth/tokens", &w.owner, None).await;
    let mine = listed.as_array().unwrap().iter().find(|t| !t["scope"].is_null()).expect("a scoped token in the list");
    assert_eq!(mine["scope"]["apps"], json!([w.a]));
    assert_eq!(mine["scope"]["tables"]["cases"], json!(["read", "create"]));
}
