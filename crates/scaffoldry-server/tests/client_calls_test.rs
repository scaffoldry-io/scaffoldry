//! External apps phase 3: an outside app calls as the person, within its ceiling.
//!
//! A client token is a narrowed token whose scope is the ceiling. The person's own rights apply on top. Fields outside
//! the ceiling are dropped on reads and refused on writes. Routes a ceiling cannot express refuse with `client_scope`.
//! Every write records the client in its history.

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::Router;
use base64::Engine;
use http_body_util::BodyExt;
use scaffoldry_server::state::CollaboratorRecord;
use scaffoldry_server::build_app_with_state;
use scaffoldry_server::service::identity::issue_test_token_and_user;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tower::ServiceExt;

const WORKSPACE: &str = "ws-physics-optics";
const OWNER: &str = "einstein@physics.state.edu";
const HOST: &str = "api.test.edu";
const CALLBACK: &str = "https://tracker.example.edu/callback";
const VERIFIER: &str = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";

struct World {
    _db: common::TestDb,
    app: Router,
    owner: String,
    second: String,
    editor: String,
    viewer: String,
    slug: String,
}

async fn send(app: &Router, method: &str, uri: &str, token: &str, content_type: Option<&str>, body: Body) -> (StatusCode, Value) {
    let mut b = Request::builder().method(method).uri(uri).header("host", HOST).header("authorization", format!("Bearer {token}"));
    if let Some(ct) = content_type {
        b = b.header("content-type", ct);
    }
    let resp = app.clone().oneshot(b.body(body).unwrap()).await.unwrap();
    let status = resp.status();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
}

async fn rest(app: &Router, method: &str, uri: &str, token: &str, body: Option<Value>) -> (StatusCode, Value) {
    match body {
        Some(v) => send(app, method, uri, token, Some("application/json"), Body::from(v.to_string())).await,
        None => send(app, method, uri, token, None, Body::empty()).await,
    }
}

/// `Err` holds the error code.
async fn tool(w: &World, token: &str, name: &str, args: Value) -> Result<Value, String> {
    let payload = json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": { "name": name, "arguments": args } });
    let (_, v) = rest(&w.app, "POST", "/api/mcp", token, Some(payload)).await;
    if v["result"]["isError"] == true {
        return Err(v["result"]["structuredContent"]["code"].as_str().unwrap_or("").to_string());
    }
    Ok(v["result"]["structuredContent"].clone())
}

fn field(name: &str) -> Value {
    json!({ "name": name, "label": name, "field_type": "Text", "required": false, "ferpa_sensitive": false })
}

fn member(state: &scaffoldry_server::state::ServerState, eppn: &str, role: &str) {
    state.collaborators.write().unwrap().entry(WORKSPACE.to_string()).or_default().push(CollaboratorRecord {
        id: uuid::Uuid::new_v4().to_string(),
        workspace_id: WORKSPACE.to_string(),
        eppn: eppn.to_string(),
        name: eppn.to_string(),
        role: role.to_string(),
        scoped_affiliation: "faculty".to_string(),
        department: "physics".to_string(),
        added_at: String::new(),
        app_slug: String::new(),
    });
}

async fn world() -> World {
    let db = common::TestDb::new();
    let state = db.state();
    state.seed_demo().expect("seed");
    let tag = uuid::Uuid::new_v4().to_string()[..8].to_string();
    let (second, editor, viewer) = (format!("second-{tag}@state.edu"), format!("editor-{tag}@state.edu"), format!("viewer-{tag}@state.edu"));
    member(&state, &second, "admin");
    member(&state, &editor, "editor");
    member(&state, &viewer, "viewer");
    let app = build_app_with_state(state).expect("router");
    let w = World {
        _db: db,
        app,
        owner: issue_test_token_and_user(OWNER),
        second: issue_test_token_and_user(&second),
        editor: issue_test_token_and_user(&editor),
        viewer: issue_test_token_and_user(&viewer),
        slug: format!("cc-{tag}"),
    };
    let manifest = json!({
        "slug": w.slug, "title": "Grants Office", "description": "d", "organization_code": "PHYS", "department": "physics",
        "workspace_id": WORKSPACE, "views": [], "pages": [],
        "tables": [{ "id": "proposals", "name": "Proposals", "slug": "proposals", "fields": [field("title"), field("status"), field("notes")] }]
    });
    let p = tool(&w, &w.owner, "propose_app_change", json!({ "workspace_id": WORKSPACE, "manifest": manifest, "summary": "An app" })).await.unwrap();
    tool(&w, &w.owner, "decide_proposal", json!({ "proposal_id": p["proposal_id"], "decision": "approve", "rationale": "Set up", "digest": p["digest"] })).await.unwrap();
    w
}

/// An approved client. It reads `title` and `status`, writes `status`, and may create. `notes` is outside it.
async fn client(w: &World, writes: bool) -> String {
    let (write, create) = if writes { (json!(["status"]), true) } else { (json!([]), false) };
    let body = json!({
        "name": "Grant tracker", "operator": "Physics Department", "kind": "public", "redirect_uris": [CALLBACK],
        "ceiling": { "apps": [{ "app_slug": w.slug, "tables": [{ "table_id": "proposals", "read": ["title", "status"], "write": write, "create": create, "delete": false }] }] }
    });
    let (status, made) = rest(&w.app, "POST", "/api/v1/clients", &w.owner, Some(body)).await;
    assert_eq!(status, StatusCode::CREATED, "{made}");
    let id = made["client_id"].as_str().unwrap().to_string();
    let (status, decided) = rest(&w.app, "POST", &format!("/api/v1/clients/{id}/decide"), &w.second, Some(json!({ "decision": "approve", "rationale": "Reviewed" }))).await;
    assert_eq!(status, StatusCode::OK, "{decided}");
    id
}

/// Signs `user` in to the client and returns the access token.
async fn sign_in(w: &World, user: &str, client_id: &str) -> String {
    let challenge = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(Sha256::digest(VERIFIER.as_bytes()));
    let body = json!({
        "client_id": client_id, "redirect_uri": CALLBACK, "response_type": "code", "state": "s", "code_challenge": challenge,
        "code_challenge_method": "S256", "resource": "https://api.test.edu", "decision": "allow"
    });
    let (status, allowed) = rest(&w.app, "POST", "/oauth/authorize", user, Some(body)).await;
    assert_eq!(status, StatusCode::OK, "{allowed}");
    let redirect = allowed["redirect_to"].as_str().unwrap();
    let code = redirect.split_once('?').unwrap().1.split('&').find_map(|p| p.strip_prefix("code=")).unwrap().to_string();
    let form = format!("grant_type=authorization_code&code={code}&redirect_uri={CALLBACK}&client_id={client_id}&code_verifier={VERIFIER}");
    let (status, token) = send(&w.app, "POST", "/oauth/token", "", Some("application/x-www-form-urlencoded"), Body::from(form)).await;
    assert_eq!(status, StatusCode::OK, "{token}");
    token["access_token"].as_str().unwrap().to_string()
}

fn records_uri(w: &World) -> String {
    format!("/api/v1/apps/{}/tables/proposals/records", w.slug)
}

async fn seed_row(w: &World) -> String {
    let (status, row) = rest(&w.app, "POST", &records_uri(w), &w.owner, Some(json!({ "data": { "title": "Telescope", "status": "Draft", "notes": "Private" } }))).await;
    assert_eq!(status, StatusCode::CREATED, "{row}");
    row["id"].as_str().unwrap().to_string()
}

// 1. A viewer's client token cannot write, even when the ceiling allows writes.
#[tokio::test]
async fn a_viewers_client_token_cannot_write_even_when_the_ceiling_allows_it() {
    let w = world().await;
    let id = seed_row(&w).await;
    let client_id = client(&w, true).await;
    let token = sign_in(&w, &w.viewer, &client_id).await;

    let (status, list) = rest(&w.app, "GET", &records_uri(&w), &token, None).await;
    assert_eq!(status, StatusCode::OK, "a viewer reads through the ceiling: {list}");
    assert_eq!(list["records"].as_array().map(Vec::len), Some(1));
    let (status, body) = rest(&w.app, "POST", &records_uri(&w), &token, Some(json!({ "data": { "title": "New" } }))).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "the ceiling allows creating and the viewer's rights do not: {body}");
    let (status, body) = rest(&w.app, "PATCH", &format!("/api/v1/apps/{}/records/{id}", w.slug), &token, Some(json!({ "data": { "status": "Final" }, "version": 1 }))).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
}

// 2. An editor's client token cannot write or read a field outside the ceiling.
#[tokio::test]
async fn an_editors_client_token_cannot_touch_a_field_outside_the_ceiling() {
    let w = world().await;
    let id = seed_row(&w).await;
    let client_id = client(&w, true).await;
    let token = sign_in(&w, &w.editor, &client_id).await;

    let (_, list) = rest(&w.app, "GET", &records_uri(&w), &token, None).await;
    let data = &list["records"][0]["data"];
    assert_eq!(data["title"], "Telescope");
    assert!(data.get("notes").is_none(), "notes is outside the ceiling's read: {data}");
    let (_, one) = rest(&w.app, "GET", &format!("{}/{id}", records_uri(&w)), &token, None).await;
    assert!(one["data"].get("notes").is_none(), "{one}");

    // title is readable but not writable. notes is neither.
    for field in ["title", "notes"] {
        let (status, body) = rest(&w.app, "PATCH", &format!("{}/{id}", records_uri(&w)), &token, Some(json!({ "data": { field: "changed" }, "version": 1 }))).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{field}: {body}");
    }
    let (status, body) = rest(&w.app, "PATCH", &format!("{}/{id}", records_uri(&w)), &token, Some(json!({ "data": { "status": "Final" }, "version": 1 }))).await;
    assert_eq!(status, StatusCode::OK, "status is in the ceiling's write: {body}");

    // The person herself still sees notes. The ceiling narrowed the client, not the person.
    let (_, own) = rest(&w.app, "GET", &format!("{}/{id}", records_uri(&w)), &w.editor, None).await;
    assert_eq!(own["data"]["notes"], "Private");

    // Delete needs its flag.
    let (status, body) = rest(&w.app, "DELETE", &format!("{}/{id}", records_uri(&w)), &token, None).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
}

// 3. A client token is refused on admin and proposal routes with client_scope, whoever signed in.
#[tokio::test]
async fn a_client_token_is_refused_on_admin_and_proposal_routes_with_client_scope() {
    let w = world().await;
    let client_id = client(&w, false).await;
    let token = sign_in(&w, &w.owner, &client_id).await;
    for (method, uri) in [
        ("GET", "/api/v1/governance/ledger".to_string()),
        ("GET", "/api/v1/clients".to_string()),
        ("GET", format!("/api/v1/workspaces/{WORKSPACE}/proposals")),
        ("POST", format!("/api/v1/workspaces/{WORKSPACE}/proposals")),
    ] {
        let (status, body) = rest(&w.app, method, &uri, &token, if method == "POST" { Some(json!({})) } else { None }).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{method} {uri}: {body}");
        assert_eq!(body["code"], "client_scope", "{method} {uri}: {body}");
    }
    // The owner's own token still reaches them.
    assert_eq!(rest(&w.app, "GET", "/api/v1/clients", &w.owner, None).await.0, StatusCode::OK);
}

// 4. A record written through a client has the client id in its history.
#[tokio::test]
async fn a_record_written_through_a_client_names_the_client_in_its_history() {
    let w = world().await;
    let id = seed_row(&w).await;
    let client_id = client(&w, true).await;
    let token = sign_in(&w, &w.editor, &client_id).await;
    let (status, body) = rest(&w.app, "PATCH", &format!("{}/{id}", records_uri(&w)), &token, Some(json!({ "data": { "status": "Final" }, "version": 1 }))).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let (status, created) = rest(&w.app, "POST", &records_uri(&w), &token, Some(json!({ "data": { "status": "Made by the app" } }))).await;
    assert_eq!(status, StatusCode::CREATED, "create writes only what the ceiling lets it write: {created}");
    let new_id = created["id"].as_str().expect("created").to_string();

    for record in [&id, &new_id] {
        let (_, history) = rest(&w.app, "GET", &format!("/api/v1/apps/{}/records/{record}/history", w.slug), &w.owner, None).await;
        let text = history.to_string();
        assert!(text.contains(&client_id), "history for {record} names the client: {text}");
    }
    // A write the person makes herself does not.
    let (_, history) = rest(&w.app, "GET", &format!("/api/v1/apps/{}/records/{id}/history", w.slug), &w.owner, None).await;
    let first = history["entries"].as_array().or_else(|| history.as_array()).and_then(|e| e.last()).cloned().unwrap_or(Value::Null);
    assert!(!first.to_string().contains(&client_id), "the owner's own create has no client: {first}");
}

// 5. The tool list for a client token holds only tools the ceiling allows.
#[tokio::test]
async fn the_tool_list_for_a_client_token_holds_only_what_the_ceiling_allows() {
    let w = world().await;
    let read_only = client(&w, false).await;
    let token = sign_in(&w, &w.editor, &read_only).await;
    let payload = json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/list" });
    let (_, v) = rest(&w.app, "POST", "/api/mcp", &token, Some(payload.clone())).await;
    let names: Vec<&str> = v["result"]["tools"].as_array().expect("tools").iter().filter_map(|t| t["name"].as_str()).collect();
    assert!(names.contains(&"list_records"), "{names:?}");
    for banned in ["create_record", "update_record", "delete_record", "propose_app_change", "decide_proposal", "create_agent_token"] {
        assert!(!names.contains(&banned), "{banned} is outside a read-only ceiling: {names:?}");
    }
    assert_eq!(tool(&w, &token, "create_record", json!({ "app_slug": w.slug, "data": { "_table_id": "proposals", "title": "x" } })).await.unwrap_err(), "client_scope");
    assert_eq!(tool(&w, &token, "decide_proposal", json!({ "proposal_id": "x", "decision": "approve", "rationale": "r", "digest": "d" })).await.unwrap_err(), "client_scope");

    // The owner's list is the full one.
    let (_, full) = rest(&w.app, "POST", "/api/mcp", &w.owner, Some(payload)).await;
    assert!(full["result"]["tools"].as_array().unwrap().len() > names.len());
}
