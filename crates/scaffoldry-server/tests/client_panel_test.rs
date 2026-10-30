//! External apps phase 6: the clients panel. Disabling stops the next call, usage counts match the calls made, and the
//! panel lists what an owner needs to decide: workspaces, connected users, and the last call.

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


async fn get_usage(w: &World, token: &str, client_id: &str) -> (StatusCode, Value) {
    rest(&w.app, "GET", &format!("/api/v1/clients/{client_id}/usage"), token, None).await
}

fn today(usage: &Value) -> Value {
    usage["days"].as_array().expect("days").last().expect("a day").clone()
}

// 1. Disabling from the panel stops the client's next call.
#[tokio::test]
async fn disabling_from_the_panel_stops_the_clients_next_call() {
    let w = world().await;
    seed_row(&w).await;
    let client_id = client(&w, false).await;
    let token = sign_in(&w, &w.editor, &client_id).await;
    assert_eq!(rest(&w.app, "GET", &records_uri(&w), &token, None).await.0, StatusCode::OK);

    let (status, body) = rest(&w.app, "POST", &format!("/api/v1/clients/{client_id}/disable"), &w.owner, Some(json!({ "reason": "No longer used" }))).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let (_, listed) = rest(&w.app, "GET", "/api/v1/clients", &w.owner, None).await;
    let row = listed.as_array().unwrap().iter().find(|c| c["client_id"] == client_id.as_str()).expect("listed");
    assert_eq!(row["status"], "Disabled", "{row}");
    assert_eq!(rest(&w.app, "GET", &records_uri(&w), &token, None).await.0, StatusCode::UNAUTHORIZED, "the next call is refused");
}

// 2. Usage counts match the calls made.
#[tokio::test]
async fn usage_counts_match_the_calls_made_in_the_test() {
    let w = world().await;
    seed_row(&w).await;
    seed_row(&w).await;
    let client_id = client(&w, true).await;
    let token = sign_in(&w, &w.editor, &client_id).await;

    // Three reads of two records each. A create and an update. One call the ceiling refuses (a delete).
    for _ in 0..3 {
        let (status, list) = rest(&w.app, "GET", &records_uri(&w), &token, None).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(list["records"].as_array().map(Vec::len), Some(2));
    }
    let (status, created) = rest(&w.app, "POST", &records_uri(&w), &token, Some(json!({ "data": { "status": "New" } }))).await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    let id = created["id"].as_str().unwrap().to_string();
    let (status, _) = rest(&w.app, "PATCH", &format!("{}/{id}", records_uri(&w)), &token, Some(json!({ "data": { "status": "Done" }, "version": 1 }))).await;
    assert_eq!(status, StatusCode::OK);
    let (status, _) = rest(&w.app, "DELETE", &format!("{}/{id}", records_uri(&w)), &token, None).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "the ceiling does not allow delete");

    let (status, usage) = get_usage(&w, &w.owner, &client_id).await;
    assert_eq!(status, StatusCode::OK, "{usage}");
    assert_eq!(usage["days"].as_array().map(Vec::len), Some(30), "thirty days, quiet ones included");
    let day = today(&usage);
    assert_eq!(day["calls"], 6, "every call counts, including the refused one: {day}");
    assert_eq!(day["records_read"], 6, "{day}");
    assert_eq!(day["records_written"], 2, "{day}");
    // Every other day is empty.
    let others: i64 = usage["days"].as_array().unwrap().iter().rev().skip(1).map(|d| d["calls"].as_i64().unwrap()).sum();
    assert_eq!(others, 0);
}

// 3. The list names what an owner needs: workspaces, how many people connected, and the last call.
#[tokio::test]
async fn the_list_names_workspaces_connected_users_and_the_last_call() {
    let w = world().await;
    seed_row(&w).await;
    let client_id = client(&w, false).await;
    let (_, before) = rest(&w.app, "GET", &format!("/api/v1/clients/{client_id}"), &w.owner, None).await;
    assert_eq!(before["connected_users"], 0, "{before}");
    assert!(before["last_call_at"].is_null(), "never called: {before}");

    let one = sign_in(&w, &w.editor, &client_id).await;
    sign_in(&w, &w.viewer, &client_id).await;
    assert_eq!(rest(&w.app, "GET", &records_uri(&w), &one, None).await.0, StatusCode::OK);

    let (_, listed) = rest(&w.app, "GET", "/api/v1/clients", &w.owner, None).await;
    let row = listed.as_array().unwrap().iter().find(|c| c["client_id"] == client_id.as_str()).expect("listed").clone();
    assert_eq!(row["connected_users"], 2, "{row}");
    assert!(row["last_call_at"].as_str().is_some(), "{row}");
    let spaces = row["workspaces"].as_array().expect("workspaces");
    assert!(spaces.iter().any(|s| s["id"] == WORKSPACE), "{row}");
    assert!(spaces.iter().all(|s| s["name"].as_str().is_some_and(|n| !n.is_empty())), "{row}");
}

// 4. Usage is for the people who may see the client.
#[tokio::test]
async fn only_people_who_may_see_a_client_read_its_usage() {
    let w = world().await;
    let client_id = client(&w, false).await;
    let outsider = issue_test_token_and_user("outsider@elsewhere.edu");
    assert_eq!(get_usage(&w, &outsider, &client_id).await.0, StatusCode::NOT_FOUND);
    assert_eq!(get_usage(&w, &w.owner, &client_id).await.0, StatusCode::OK);
    assert_eq!(get_usage(&w, &w.owner, "scc_unknown").await.0, StatusCode::NOT_FOUND);
}
