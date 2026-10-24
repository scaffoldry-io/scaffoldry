//! External apps phase 8: AI tools sign in as themselves.
//!
//! An agent client is registered by a Platform Admin and has no ceiling: it acts within the person's own rights. At
//! consent the person picks what it may do. Its tokens come with a rotating refresh token inside a session that has an
//! end. The tool `begin_task` records what it is doing for the person. RFC 9700 section 4.14 and the MCP authorization
//! specification.

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::Router;
use base64::Engine;
use http_body_util::BodyExt;
use scaffoldry_server::build_app_with_state;
use scaffoldry_server::service::identity::issue_test_token_and_user;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tower::ServiceExt;

const WORKSPACE: &str = "ws-physics-optics";
const OWNER: &str = "einstein@physics.state.edu";
const PLATFORM_ADMIN: &str = "jordan.lee@state.edu";
const HOST: &str = "api.test.edu";
const ISSUER: &str = "https://api.test.edu";
const CALLBACK: &str = "https://tool.example.edu/callback";
const VERIFIER: &str = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";

struct World {
    db: common::TestDb,
    app: Router,
    owner: String,
    platform: String,
    slug: String,
}

async fn send(app: &Router, method: &str, uri: &str, token: &str, content_type: Option<&str>, body: Body) -> (StatusCode, Value) {
    let mut b = Request::builder().method(method).uri(uri).header("host", HOST);
    if !token.is_empty() {
        b = b.header("authorization", format!("Bearer {token}"));
    }
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

/// A tool call. `Err` holds the error code and the sentence.
async fn tool(app: &Router, token: &str, name: &str, args: Value) -> Result<Value, (String, String)> {
    let payload = json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": { "name": name, "arguments": args } });
    let (_, v) = rest(app, "POST", "/api/mcp", token, Some(payload)).await;
    if v["result"]["isError"] == true {
        let code = v["result"]["structuredContent"]["code"].as_str().unwrap_or("").to_string();
        return Err((code, v["result"]["content"][0]["text"].as_str().unwrap_or("").to_string()));
    }
    Ok(v["result"]["structuredContent"].clone())
}

fn field(name: &str) -> Value {
    json!({ "name": name, "label": name, "field_type": "Text", "required": false, "ferpa_sensitive": false })
}

async fn world() -> World {
    let db = common::TestDb::new();
    let state = db.state();
    state.seed_demo().expect("seed");
    let app = build_app_with_state(state).expect("router");
    let tag = uuid::Uuid::new_v4().to_string()[..8].to_string();
    let w = World { db, app, owner: issue_test_token_and_user(OWNER), platform: issue_test_token_and_user(PLATFORM_ADMIN), slug: format!("ag-{tag}") };
    let manifest = json!({
        "slug": w.slug, "title": "Grants Office", "description": "d", "organization_code": "PHYS", "department": "physics",
        "workspace_id": WORKSPACE, "views": [], "pages": [],
        "tables": [{ "id": "proposals", "name": "Proposals", "slug": "proposals", "fields": [field("title"), field("status")] }]
    });
    let p = tool(&w.app, &w.owner, "propose_app_change", json!({ "workspace_id": WORKSPACE, "manifest": manifest, "summary": "An app" })).await.unwrap();
    tool(&w.app, &w.owner, "decide_proposal", json!({ "proposal_id": p["proposal_id"], "decision": "approve", "rationale": "Set up", "digest": p["digest"] })).await.unwrap();
    w
}

/// An approved agent client, registered and decided by the Platform Admin.
async fn agent_client(w: &World) -> String {
    let body = json!({ "name": "Desk assistant", "operator": "Academic Computing", "kind": "agent", "redirect_uris": [CALLBACK] });
    let (status, made) = rest(&w.app, "POST", "/api/v1/clients", &w.platform, Some(body)).await;
    assert_eq!(status, StatusCode::CREATED, "{made}");
    let id = made["client_id"].as_str().unwrap().to_string();
    let (status, decided) = rest(&w.app, "POST", &format!("/api/v1/clients/{id}/decide"), &w.platform, Some(json!({ "decision": "approve", "rationale": "An allowed tool" }))).await;
    assert_eq!(status, StatusCode::OK, "{decided}");
    assert_eq!(decided["status"], "Active");
    assert!(decided.get("client_secret").is_none(), "an agent client is public");
    id
}

fn challenge() -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(Sha256::digest(VERIFIER.as_bytes()))
}

fn authorize_body(client_id: &str, access: Option<&str>) -> Value {
    let mut body = json!({
        "client_id": client_id, "redirect_uri": CALLBACK, "response_type": "code", "state": "s", "code_challenge": challenge(),
        "code_challenge_method": "S256", "resource": ISSUER, "decision": "allow"
    });
    if let Some(a) = access {
        body["access"] = json!(a);
    }
    body
}

async fn token_form(w: &World, form: String) -> (StatusCode, Value) {
    send(&w.app, "POST", "/oauth/token", "", Some("application/x-www-form-urlencoded"), Body::from(form)).await
}

/// Signs `user` in to the agent client with the access they pick. Returns the whole token reply.
async fn sign_in(w: &World, user: &str, client_id: &str, access: &str) -> Value {
    let (status, allowed) = rest(&w.app, "POST", "/oauth/authorize", user, Some(authorize_body(client_id, Some(access)))).await;
    assert_eq!(status, StatusCode::OK, "{allowed}");
    let code = allowed["redirect_to"].as_str().unwrap().split_once('?').unwrap().1.split('&').find_map(|p| p.strip_prefix("code=")).unwrap().to_string();
    let (status, reply) = token_form(w, format!("grant_type=authorization_code&code={code}&redirect_uri={CALLBACK}&client_id={client_id}&code_verifier={VERIFIER}")).await;
    assert_eq!(status, StatusCode::OK, "{reply}");
    reply
}

fn records_uri(w: &World) -> String {
    format!("/api/v1/apps/{}/tables/proposals/records", w.slug)
}

fn sql(w: &World, statement: String) {
    let url = w.db.url().to_string();
    std::thread::spawn(move || {
        let mut c = postgres::Client::connect(&url, postgres::NoTls).unwrap();
        c.batch_execute(&statement).unwrap();
    })
    .join()
    .unwrap();
}

// 1. An MCP client discovers the metadata and completes the flow with PKCE.
#[tokio::test]
async fn an_mcp_client_discovers_the_metadata_and_completes_the_flow_with_pkce() {
    let w = world().await;
    let (_, resource) = rest(&w.app, "GET", "/.well-known/oauth-protected-resource", "", None).await;
    let servers = resource["authorization_servers"].as_array().expect("the resource names its authorization server");
    assert!(servers.iter().any(|s| s.as_str().is_some_and(|s| s.starts_with("http"))), "{resource}");
    let (_, meta) = rest(&w.app, "GET", "/.well-known/oauth-authorization-server", "", None).await;
    let grants: Vec<&str> = meta["grant_types_supported"].as_array().unwrap().iter().filter_map(Value::as_str).collect();
    assert!(grants.contains(&"authorization_code") && grants.contains(&"refresh_token"), "{meta}");
    assert_eq!(meta["code_challenge_methods_supported"], json!(["S256"]));

    // Only a Platform Admin registers an agent client.
    let body = json!({ "name": "Desk assistant", "operator": "Academic Computing", "kind": "agent", "redirect_uris": [CALLBACK] });
    let (status, refused) = rest(&w.app, "POST", "/api/v1/clients", &w.owner, Some(body)).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{refused}");
    let id = agent_client(&w).await;

    // The consent names the three choices as sentences.
    let query = format!("client_id={id}&redirect_uri={CALLBACK}&response_type=code&state=s&code_challenge={}&code_challenge_method=S256&resource={ISSUER}", challenge());
    let (status, consent) = rest(&w.app, "GET", &format!("/oauth/authorize?{query}"), &w.owner, None).await;
    assert_eq!(status, StatusCode::OK, "{consent}");
    let choices: Vec<&str> = consent["choices"].as_array().expect("choices").iter().filter_map(|c| c["id"].as_str()).collect();
    assert_eq!(choices, ["read", "propose", "write"]);
    assert!(consent["choices"][1]["sentence"].as_str().unwrap().contains("propose"));
    assert_eq!(consent["remembered"], false, "an agent session is chosen every time");
    // A choice outside the list is refused, and so is none.
    for access in [Some("admin"), None] {
        let (status, body) = rest(&w.app, "POST", "/oauth/authorize", &w.owner, Some(authorize_body(&id, access))).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{access:?}: {body}");
    }

    let reply = sign_in(&w, &w.owner, &id, "propose").await;
    assert_eq!(reply["expires_in"], 3600);
    assert!(reply["refresh_token"].as_str().is_some_and(|t| !t.is_empty()), "an agent client gets a refresh token: {reply}");
    let access = reply["access_token"].as_str().unwrap();
    let payload = json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/list" });
    let (_, listed) = rest(&w.app, "POST", "/api/mcp", access, Some(payload)).await;
    let names: Vec<&str> = listed["result"]["tools"].as_array().unwrap().iter().filter_map(|t| t["name"].as_str()).collect();
    for wanted in ["list_records", "propose_app_change", "begin_task"] {
        assert!(names.contains(&wanted), "{wanted} is in a propose session: {names:?}");
    }
    for banned in ["create_record", "update_record", "decide_proposal", "update_workspace"] {
        assert!(!names.contains(&banned), "{banned} is not: {names:?}");
    }
}

// 2. A session consented as Read cannot write, and is told how the person can widen it.
#[tokio::test]
async fn a_read_session_cannot_write_and_the_refusal_says_how_to_widen_it() {
    let w = world().await;
    let id = agent_client(&w).await;
    let read = sign_in(&w, &w.owner, &id, "read").await["access_token"].as_str().unwrap().to_string();

    assert!(tool(&w.app, &read, "list_records", json!({ "app_slug": w.slug, "table_id": "proposals" })).await.is_ok());
    let (code, sentence) = tool(&w.app, &read, "create_record", json!({ "app_slug": w.slug, "data": { "_table_id": "proposals", "title": "x" } })).await.unwrap_err();
    assert_eq!(code, "client_scope");
    assert!(sentence.contains("Read") && sentence.to_lowercase().contains("ask the person"), "the refusal tells the agent what the person can do: {sentence}");
    let (status, body) = rest(&w.app, "POST", &records_uri(&w), &read, Some(json!({ "data": { "title": "x" } }))).await;
    assert_eq!((status, body["code"].as_str()), (StatusCode::FORBIDDEN, Some("client_scope")), "{body}");
    assert_eq!(rest(&w.app, "GET", &records_uri(&w), &read, None).await.0, StatusCode::OK);

    // Propose may propose and still not write. Write may write, and still not approve.
    let propose = sign_in(&w, &w.owner, &id, "propose").await["access_token"].as_str().unwrap().to_string();
    let (code, _) = tool(&w.app, &propose, "create_record", json!({ "app_slug": w.slug, "data": { "_table_id": "proposals", "title": "x" } })).await.unwrap_err();
    assert_eq!(code, "client_scope");
    let (code, _) = tool(&w.app, &propose, "propose_app_change", json!({})).await.unwrap_err();
    assert_ne!(code, "client_scope", "a propose session may call propose_app_change");
    let write = sign_in(&w, &w.owner, &id, "write").await["access_token"].as_str().unwrap().to_string();
    assert!(tool(&w.app, &write, "create_record", json!({ "app_slug": w.slug, "data": { "_table_id": "proposals", "title": "made by the tool" } })).await.is_ok());
    let (code, _) = tool(&w.app, &write, "decide_proposal", json!({ "proposal_id": "x", "decision": "approve", "rationale": "r", "digest": "d" })).await.unwrap_err();
    assert_eq!(code, "client_scope", "a tool never approves its own proposal");
}

// 3. A record written in a session names the person, the client, and the task sentence.
#[tokio::test]
async fn a_record_written_in_a_session_names_the_person_the_client_and_the_task() {
    let w = world().await;
    let id = agent_client(&w).await;
    let write = sign_in(&w, &w.owner, &id, "write").await["access_token"].as_str().unwrap().to_string();
    let too_long = "x".repeat(201);
    let (code, _) = tool(&w.app, &write, "begin_task", json!({ "task": too_long })).await.unwrap_err();
    assert_eq!(code, "bad_request", "a task is at most 200 characters");
    tool(&w.app, &write, "begin_task", json!({ "task": "Enter the spring grant proposals" })).await.expect("begin_task");

    let made = tool(&w.app, &write, "create_record", json!({ "app_slug": w.slug, "data": { "_table_id": "proposals", "title": "Telescope" } })).await.unwrap();
    let record = made["id"].as_str().expect("the new record").to_string();
    let (_, history) = rest(&w.app, "GET", &format!("/api/v1/apps/{}/records/{record}/history", w.slug), &w.owner, None).await;
    let text = history.to_string();
    assert!(text.contains(OWNER), "the person: {text}");
    assert!(text.contains(&id), "the client: {text}");
    assert!(text.contains("Enter the spring grant proposals"), "the task: {text}");
}

// 4. A reused refresh token revokes the session. A session ends at the configured hours.
#[tokio::test]
async fn a_refresh_token_rotates_a_reused_one_ends_the_session_and_a_session_has_an_end() {
    let w = world().await;
    let id = agent_client(&w).await;
    let first = sign_in(&w, &w.owner, &id, "read").await;
    let refresh = |token: &str| format!("grant_type=refresh_token&refresh_token={token}&client_id={id}");

    let (status, second) = token_form(&w, refresh(first["refresh_token"].as_str().unwrap())).await;
    assert_eq!(status, StatusCode::OK, "{second}");
    assert_ne!(second["refresh_token"], first["refresh_token"], "it rotates on every use");
    let new_access = second["access_token"].as_str().unwrap();
    assert_eq!(rest(&w.app, "GET", &records_uri(&w), new_access, None).await.0, StatusCode::OK);

    // The old refresh token is used again: the whole session ends, including the newest access token.
    let (status, reuse) = token_form(&w, refresh(first["refresh_token"].as_str().unwrap())).await;
    assert_eq!((status, reuse["error"].as_str()), (StatusCode::BAD_REQUEST, Some("invalid_grant")), "{reuse}");
    assert_eq!(rest(&w.app, "GET", &records_uri(&w), new_access, None).await.0, StatusCode::UNAUTHORIZED, "the session was revoked");
    let (status, _) = token_form(&w, refresh(second["refresh_token"].as_str().unwrap())).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "the newest refresh token died with the session");

    // The session has an end, set by access.agent_session_hours (1 to 24, 12 by default).
    for bad in [json!(0), json!(25), json!("12")] {
        let (status, _) = rest(&w.app, "PUT", "/api/v1/settings/access.agent_session_hours", &w.platform, Some(bad.clone())).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{bad}");
    }
    let (status, _) = rest(&w.app, "PUT", "/api/v1/settings/access.agent_session_hours", &w.platform, Some(json!(2))).await;
    assert_eq!(status, StatusCode::OK);
    let third = sign_in(&w, &w.owner, &id, "read").await;
    let hours = |w: &World| {
        let url = w.db.url().to_string();
        std::thread::spawn(move || {
            let mut c = postgres::Client::connect(&url, postgres::NoTls).unwrap();
            let row = c.query_one("SELECT EXTRACT(EPOCH FROM (MAX(expires_at) - MAX(started_at)))::float8 / 3600 FROM oauth_sessions WHERE revoked_at IS NULL", &[]).unwrap();
            row.get::<_, f64>(0)
        })
        .join()
        .unwrap()
    };
    assert!((hours(&w) - 2.0).abs() < 0.01, "the session lasts the configured hours");
    sql(&w, "UPDATE oauth_sessions SET expires_at = NOW() - INTERVAL '1 minute' WHERE revoked_at IS NULL".to_string());
    assert_eq!(rest(&w.app, "GET", &records_uri(&w), third["access_token"].as_str().unwrap(), None).await.0, StatusCode::UNAUTHORIZED, "the access token ends with the session");
    let (status, _) = token_form(&w, refresh(third["refresh_token"].as_str().unwrap())).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "so does the refresh token");
}

// 5. A person put on hold loses the session on the next call. Disconnecting and disabling end it too.
#[tokio::test]
async fn a_hold_a_disconnect_and_a_disable_end_the_session_at_once() {
    let w = world().await;
    let id = agent_client(&w).await;

    // A person on hold.
    let name = format!("held-{}@state.edu", &uuid::Uuid::new_v4().to_string()[..8]);
    let (status, created) = rest(&w.app, "POST", "/api/v1/admin/users", &w.platform, Some(json!({ "userName": name, "name": "Held Person", "email": name, "affiliation": "faculty", "department": "Physics", "title": "Lecturer", "reason": "Test" }))).await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    let held = issue_test_token_and_user(&name);
    let session = sign_in(&w, &held, &id, "read").await;
    let access = session["access_token"].as_str().unwrap().to_string();
    assert_eq!(rest(&w.app, "GET", "/api/v1/auth/me", &access, None).await.0, StatusCode::OK);
    let (status, _) = rest(&w.app, "POST", &format!("/api/v1/admin/users/{}/hold", created["user"]["id"].as_str().unwrap()), &w.platform, Some(json!({ "hold": true, "reason": "Under review" }))).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(rest(&w.app, "GET", "/api/v1/auth/me", &access, None).await.0, StatusCode::UNAUTHORIZED, "a hold ends the session");
    let (status, _) = token_form(&w, format!("grant_type=refresh_token&refresh_token={}&client_id={id}", session["refresh_token"].as_str().unwrap())).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "and it cannot be refreshed");

    // Disconnecting.
    let mine = sign_in(&w, &w.owner, &id, "read").await;
    let (status, _) = rest(&w.app, "DELETE", &format!("/api/v1/me/connections/{id}"), &w.owner, None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(rest(&w.app, "GET", "/api/v1/auth/me", mine["access_token"].as_str().unwrap(), None).await.0, StatusCode::UNAUTHORIZED);
    let (status, _) = token_form(&w, format!("grant_type=refresh_token&refresh_token={}&client_id={id}", mine["refresh_token"].as_str().unwrap())).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "a disconnect ends the session, refresh token included");

    // Disabling.
    let again = sign_in(&w, &w.owner, &id, "read").await;
    let (status, body) = rest(&w.app, "POST", &format!("/api/v1/clients/{id}/disable"), &w.platform, Some(json!({ "reason": "Retired" }))).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(rest(&w.app, "GET", "/api/v1/auth/me", again["access_token"].as_str().unwrap(), None).await.0, StatusCode::UNAUTHORIZED);
    let (status, _) = token_form(&w, format!("grant_type=refresh_token&refresh_token={}&client_id={id}", again["refresh_token"].as_str().unwrap())).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "a disabled client is unknown to the token endpoint");
}

// An app client gets no refresh token, and cannot ask for one.
#[tokio::test]
async fn an_app_client_has_no_refresh_token() {
    let w = world().await;
    let body = json!({
        "name": "Tracker", "operator": "Physics", "kind": "public", "redirect_uris": [CALLBACK],
        "ceiling": { "apps": [{ "app_slug": w.slug, "tables": [{ "table_id": "proposals", "read": ["title"], "write": [], "create": false, "delete": false }] }] }
    });
    let (_, made) = rest(&w.app, "POST", "/api/v1/clients", &w.owner, Some(body)).await;
    let id = made["client_id"].as_str().unwrap().to_string();
    rest(&w.app, "POST", &format!("/api/v1/clients/{id}/decide"), &w.owner, Some(json!({ "decision": "approve", "rationale": "Reviewed" }))).await;
    let (_, allowed) = rest(&w.app, "POST", "/oauth/authorize", &w.owner, Some(authorize_body(&id, None))).await;
    let code = allowed["redirect_to"].as_str().unwrap().split_once('?').unwrap().1.split('&').find_map(|p| p.strip_prefix("code=")).unwrap().to_string();
    let (status, reply) = token_form(&w, format!("grant_type=authorization_code&code={code}&redirect_uri={CALLBACK}&client_id={id}&code_verifier={VERIFIER}")).await;
    assert_eq!(status, StatusCode::OK, "{reply}");
    assert!(reply.get("refresh_token").is_none(), "{reply}");
}
