//! MCP toolsets phase 2 (`docs/plans/mcp-toolsets.md`): consent offers the toolsets, an OAuth `scope` pre-ticks them,
//! the session stores the choice, a refreshed token keeps it, discovery lists them, and a call outside them says which
//! scope to ask for.

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::Router;
use base64::Engine;
use http_body_util::BodyExt;
use scaffoldry_core::Toolset;
use scaffoldry_server::build_app_with_state;
use scaffoldry_server::service::identity::issue_test_token_and_user;
use scaffoldry_server::service::tools::TOOLS;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use tower::ServiceExt;

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
}

struct Reply {
    status: StatusCode,
    www_authenticate: Option<String>,
    body: Value,
}

async fn send(app: &Router, method: &str, uri: &str, token: &str, body: Option<Value>) -> Reply {
    let mut b = Request::builder().method(method).uri(uri).header("host", HOST);
    if !token.is_empty() {
        b = b.header("authorization", format!("Bearer {token}"));
    }
    let payload = match body {
        Some(v) => {
            b = b.header("content-type", "application/json");
            Body::from(v.to_string())
        }
        None => Body::empty(),
    };
    let resp = app.clone().oneshot(b.body(payload).unwrap()).await.unwrap();
    let status = resp.status();
    let www_authenticate = resp.headers().get("www-authenticate").and_then(|v| v.to_str().ok()).map(str::to_string);
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    Reply { status, www_authenticate, body: serde_json::from_slice(&bytes).unwrap_or(Value::Null) }
}

async fn token_form(w: &World, form: String) -> Value {
    let req = Request::builder()
        .method("POST")
        .uri("/oauth/token")
        .header("host", HOST)
        .header("content-type", "application/x-www-form-urlencoded")
        .body(Body::from(form))
        .unwrap();
    let resp = w.app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    serde_json::from_slice(&resp.into_body().collect().await.unwrap().to_bytes()).unwrap()
}

fn world() -> World {
    let db = common::TestDb::new();
    let state = db.state();
    state.seed_demo().expect("seed");
    let app = build_app_with_state(state).expect("router");
    World { db, app, owner: issue_test_token_and_user(OWNER), platform: issue_test_token_and_user(PLATFORM_ADMIN) }
}

async fn agent_client(w: &World) -> String {
    let body = json!({ "name": "Desk assistant", "operator": "Academic Computing", "kind": "agent", "redirect_uris": [CALLBACK] });
    let made = send(&w.app, "POST", "/api/v1/clients", &w.platform, Some(body)).await;
    assert_eq!(made.status, StatusCode::CREATED, "{}", made.body);
    let id = made.body["client_id"].as_str().unwrap().to_string();
    let decided = send(&w.app, "POST", &format!("/api/v1/clients/{id}/decide"), &w.platform, Some(json!({ "decision": "approve", "rationale": "An allowed tool" }))).await;
    assert_eq!(decided.status, StatusCode::OK, "{}", decided.body);
    id
}

fn challenge() -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(Sha256::digest(VERIFIER.as_bytes()))
}

/// The consent screen's data for an agent client, with an optional OAuth `scope`.
async fn consent(w: &World, user: &str, client_id: &str, scope: Option<&str>) -> Value {
    let mut query = format!("client_id={client_id}&redirect_uri={CALLBACK}&response_type=code&state=s&code_challenge={}&code_challenge_method=S256&resource={ISSUER}", challenge());
    if let Some(s) = scope {
        query.push_str(&format!("&scope={}", s.replace(' ', "%20").replace(':', "%3A")));
    }
    let r = send(&w.app, "GET", &format!("/oauth/authorize?{query}"), user, None).await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.body);
    r.body
}

/// Signs in as `user` with the access and toolsets they pick. `None` toolsets sends no `toolsets` field at all.
async fn sign_in(w: &World, user: &str, client_id: &str, access: &str, toolsets: Option<&[&str]>) -> Value {
    let mut body = json!({
        "client_id": client_id, "redirect_uri": CALLBACK, "response_type": "code", "state": "s", "code_challenge": challenge(),
        "code_challenge_method": "S256", "resource": ISSUER, "decision": "allow", "access": access
    });
    if let Some(t) = toolsets {
        body["toolsets"] = json!(t);
    }
    let allowed = send(&w.app, "POST", "/oauth/authorize", user, Some(body)).await;
    assert_eq!(allowed.status, StatusCode::OK, "{}", allowed.body);
    let code = allowed.body["redirect_to"].as_str().unwrap().split_once('?').unwrap().1.split('&').find_map(|p| p.strip_prefix("code=")).unwrap().to_string();
    token_form(w, format!("grant_type=authorization_code&code={code}&redirect_uri={CALLBACK}&client_id={client_id}&code_verifier={VERIFIER}")).await
}

async fn refresh(w: &World, client_id: &str, refresh_token: &str) -> Value {
    token_form(w, format!("grant_type=refresh_token&refresh_token={refresh_token}&client_id={client_id}")).await
}

async fn rpc(w: &World, token: &str, method: &str, params: Value) -> Reply {
    send(&w.app, "POST", "/api/mcp", token, Some(json!({ "jsonrpc": "2.0", "id": 1, "method": method, "params": params }))).await
}

async fn listed(w: &World, token: &str) -> Vec<Value> {
    rpc(w, token, "tools/list", json!({})).await.body["result"]["tools"].as_array().cloned().unwrap_or_default()
}

fn sets_of(tools: &[Value]) -> BTreeSet<&'static str> {
    tools.iter().map(|t| TOOLS.iter().find(|x| x.name == t["name"].as_str().unwrap()).unwrap().toolset.word()).collect()
}

fn access_of(reply: &Value) -> &str {
    reply["access_token"].as_str().unwrap()
}

fn sql(w: &World, statement: &str) {
    let (url, statement) = (w.db.url().to_string(), statement.to_string());
    std::thread::spawn(move || postgres::Client::connect(&url, postgres::NoTls).unwrap().batch_execute(&statement).unwrap()).join().unwrap();
}

fn ids(consent: &Value) -> Vec<&str> {
    consent["toolsets"].as_array().expect("consent lists toolsets").iter().filter_map(|t| t["id"].as_str()).collect()
}

fn selected(consent: &Value) -> Vec<&str> {
    consent["toolsets"].as_array().unwrap().iter().filter(|t| t["selected"] == true).filter_map(|t| t["id"].as_str()).collect()
}

#[tokio::test]
async fn consent_lists_the_toolsets_with_a_sentence_each_and_hides_admin_from_others() {
    let w = world();
    let id = agent_client(&w).await;
    let c = consent(&w, &w.owner, &id, None).await;
    assert_eq!(ids(&c), ["core", "data", "build", "access", "governance"], "admin is not offered to a member");
    for t in c["toolsets"].as_array().unwrap() {
        assert!(t["sentence"].as_str().is_some_and(|s| !s.is_empty() && !s.contains('\n')), "{t}");
    }
    let core = &c["toolsets"][0];
    assert_eq!(core["always_on"], true);
    assert_eq!(core["selected"], true);
    assert_eq!(selected(&c), ["core", "data"], "with no scope, the default is ticked");

    let admin = consent(&w, &w.platform, &id, None).await;
    assert_eq!(ids(&admin), ["core", "data", "build", "access", "governance", "admin"]);
}

#[tokio::test]
async fn an_oauth_scope_pre_ticks_a_toolset_and_unknown_values_are_ignored() {
    let w = world();
    let id = agent_client(&w).await;
    let c = consent(&w, &w.owner, &id, Some("mcp:governance mcp:telepathy records:read mcp:core")).await;
    assert_eq!(selected(&c), ["core", "governance"], "only the named toolset is ticked, plus core");
    let both = consent(&w, &w.owner, &id, Some("mcp:data mcp:build")).await;
    assert_eq!(selected(&both), ["core", "data", "build"]);
    // A member who asks for admin gets no tick, and no row.
    let asked = consent(&w, &w.owner, &id, Some("mcp:admin")).await;
    assert!(!ids(&asked).contains(&"admin"));
    assert_eq!(selected(&asked), ["core", "data"], "nothing valid was asked, so the default stands");
    let admin = consent(&w, &w.platform, &id, Some("mcp:admin")).await;
    assert_eq!(selected(&admin), ["core", "admin"]);
}

#[tokio::test]
async fn a_new_session_that_picks_nothing_gets_core_and_data() {
    let w = world();
    let id = agent_client(&w).await;
    for picked in [None, Some(&[][..])] {
        let reply = sign_in(&w, &w.owner, &id, "write", picked).await;
        let sets = sets_of(&listed(&w, access_of(&reply)).await);
        assert_eq!(sets, BTreeSet::from(["core", "data"]), "picked {picked:?}");
    }
}

#[tokio::test]
async fn the_session_stores_the_choice_and_a_refreshed_token_keeps_it() {
    let w = world();
    let id = agent_client(&w).await;
    let reply = sign_in(&w, &w.owner, &id, "write", Some(&["governance", "build"])).await;
    let first = sets_of(&listed(&w, access_of(&reply)).await);
    assert_eq!(first, BTreeSet::from(["core", "governance", "build"]));

    let again = refresh(&w, &id, reply["refresh_token"].as_str().unwrap()).await;
    assert_ne!(access_of(&again), access_of(&reply));
    assert_eq!(sets_of(&listed(&w, access_of(&again)).await), first, "the refreshed token keeps the toolsets");

    let stored: Vec<String> = {
        let url = w.db.url().to_string();
        std::thread::spawn(move || {
            let mut c = postgres::Client::connect(&url, postgres::NoTls).unwrap();
            c.query_one("SELECT toolsets FROM oauth_sessions ORDER BY started_at DESC LIMIT 1", &[]).unwrap().get(0)
        })
        .join()
        .unwrap()
    };
    assert_eq!(stored, ["build", "governance"]);
}

#[tokio::test]
async fn a_member_cannot_get_admin_and_an_unknown_toolset_is_refused() {
    let w = world();
    let id = agent_client(&w).await;
    let reply = sign_in(&w, &w.owner, &id, "write", Some(&["data", "admin"])).await;
    assert!(!sets_of(&listed(&w, access_of(&reply)).await).contains("admin"));

    let body = json!({
        "client_id": id, "redirect_uri": CALLBACK, "response_type": "code", "state": "s", "code_challenge": challenge(),
        "code_challenge_method": "S256", "resource": ISSUER, "decision": "allow", "access": "read", "toolsets": ["telepathy"]
    });
    let r = send(&w.app, "POST", "/oauth/authorize", &w.owner, Some(body)).await;
    assert_eq!(r.status, StatusCode::BAD_REQUEST, "{}", r.body);
}

#[tokio::test]
async fn a_session_from_before_phase_2_keeps_every_toolset() {
    let w = world();
    let id = agent_client(&w).await;
    let reply = sign_in(&w, &w.owner, &id, "write", Some(&["data"])).await;
    sql(&w, "UPDATE oauth_sessions SET toolsets = NULL");
    let again = refresh(&w, &id, reply["refresh_token"].as_str().unwrap()).await;
    let sets = sets_of(&listed(&w, access_of(&again)).await);
    assert!(["core", "data", "build", "governance"].iter().all(|s| sets.contains(s)), "{sets:?}");
}

#[tokio::test]
async fn discovery_lists_the_toolset_scopes_and_keeps_the_old_ones() {
    let w = world();
    let r = send(&w.app, "GET", "/.well-known/oauth-protected-resource", "", None).await;
    let scopes: Vec<&str> = r.body["scopes_supported"].as_array().unwrap().iter().filter_map(Value::as_str).collect();
    for old in ["mcp:read", "mcp:write", "workspaces:read", "workspaces:write", "records:read", "records:write", "governance:audit"] {
        assert!(scopes.contains(&old), "{old} stays");
    }
    for new in ["mcp:data", "mcp:build", "mcp:access", "mcp:governance", "mcp:admin"] {
        assert!(scopes.contains(&new), "{new} is listed");
    }
    assert!(!scopes.contains(&"mcp:core"), "core is always on, so it is not asked for");
}

#[tokio::test]
async fn a_call_outside_the_toolsets_names_the_scope_to_ask_for() {
    let w = world();
    let id = agent_client(&w).await;
    let reply = sign_in(&w, &w.owner, &id, "write", None).await;
    let r = rpc(&w, access_of(&reply), "tools/call", json!({ "name": "governance_report", "arguments": {} })).await;
    assert_eq!(r.www_authenticate.as_deref(), Some(r#"Bearer error="insufficient_scope", scope="mcp:governance""#));
    assert_eq!(r.body["result"]["isError"], true, "the JSON-RPC body is unchanged");
    assert_eq!(r.body["result"]["structuredContent"]["required_toolset"], "governance");
    assert_eq!(r.body["result"]["structuredContent"]["code"], "token_scope");

    // A call inside the toolsets carries no such header.
    let ok = rpc(&w, access_of(&reply), "tools/call", json!({ "name": "list_workspaces", "arguments": {} })).await;
    assert!(ok.www_authenticate.is_none());
}

#[tokio::test]
async fn instructions_name_the_missing_toolsets_and_stay_short() {
    let w = world();
    let id = agent_client(&w).await;
    let reply = sign_in(&w, &w.owner, &id, "write", None).await;
    let text = rpc(&w, access_of(&reply), "initialize", json!({ "protocolVersion": "2025-06-18" })).await.body["result"]["instructions"].as_str().unwrap().to_string();
    let line = text.lines().find(|l| l.contains("toolset")).unwrap_or_else(|| panic!("a line names the toolsets: {text}"));
    for missing in ["build", "access", "governance"] {
        assert!(line.contains(missing), "{line}");
    }
    assert!(!line.contains("data") && !line.contains("admin"), "{line}");
    assert!(line.to_lowercase().contains("reconnect"), "{line}");
    assert!(text.len() <= 1800, "{} bytes", text.len());

    // A session with every toolset has nothing to say, and nor does a plain desk token.
    let all = sign_in(&w, &w.platform, &id, "write", Some(&["data", "build", "access", "governance", "admin"])).await;
    let text = rpc(&w, access_of(&all), "initialize", json!({})).await.body["result"]["instructions"].as_str().unwrap().to_string();
    assert!(!text.contains("toolset"), "{text}");
    let desk = rpc(&w, &w.owner, "initialize", json!({})).await.body["result"]["instructions"].as_str().unwrap().to_string();
    assert!(!desk.contains("toolset"));
}

#[tokio::test]
async fn the_default_list_is_core_and_data_and_fits_the_budget() {
    let w = world();
    let id = agent_client(&w).await;
    let reply = sign_in(&w, &w.owner, &id, "write", None).await;
    let tools = listed(&w, access_of(&reply)).await;
    let bytes: usize = tools.iter().map(|t| serde_json::to_vec(t).unwrap().len()).sum();
    println!("default core+data list: {} tools, {bytes} bytes", tools.len());
    let want: BTreeSet<&str> = TOOLS.iter().filter(|t| matches!(t.toolset, Toolset::Core | Toolset::Data)).map(|t| t.name).collect();
    let got: BTreeSet<&str> = tools.iter().filter_map(|t| t["name"].as_str()).collect();
    assert!(got.is_subset(&want) && got.contains("create_record") && got.contains("begin_task"), "{got:?}");
    assert!(bytes < 13_000, "the default list is about 12 KB: {bytes}");
}
