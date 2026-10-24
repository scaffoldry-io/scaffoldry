//! External apps phase 7: an outside app may use tokens from the institution's identity provider.
//!
//! A provider token whose `client_id` (RFC 9068) or `azp` claim names a registered, active client is treated exactly like
//! a phase 2 token: the person is the subject, and the client's ceiling applies. A token that names an unregistered,
//! pending or disabled client is refused, and so is one for an agent client, which signs in through OAuth sessions.

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::Router;
use base64::Engine;
use http_body_util::BodyExt;
use jsonwebtoken::{Algorithm, EncodingKey, Header};
use scaffoldry_server::build_app_with_state;
use scaffoldry_server::service::identity::issue_test_token_and_user;
use scaffoldry_server::state::CollaboratorRecord;
use serde_json::{json, Value};
use tower::ServiceExt;

const WORKSPACE: &str = "ws-physics-optics";
const OWNER: &str = "einstein@physics.state.edu";
const PLATFORM_ADMIN: &str = "jordan.lee@state.edu";
const ISSUER: &str = "https://idp.state.edu";
const AUDIENCE: &str = "scaffoldry";
const CALLBACK: &str = "https://tracker.example.edu/callback";

struct World {
    _db: common::TestDb,
    app: Router,
    owner: String,
    second: String,
    platform: String,
    slug: String,
    kid: String,
    pem: String,
}

async fn send(app: &Router, method: &str, uri: &str, token: &str, body: Option<Value>) -> (StatusCode, Value) {
    let mut b = Request::builder().method(method).uri(uri).header("authorization", format!("Bearer {token}"));
    let body = match body {
        Some(v) => {
            b = b.header("content-type", "application/json");
            Body::from(v.to_string())
        }
        None => Body::empty(),
    };
    let resp = app.clone().oneshot(b.body(body).unwrap()).await.unwrap();
    let status = resp.status();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
}

async fn tool(w: &World, token: &str, name: &str, args: Value) -> Result<Value, String> {
    let payload = json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": { "name": name, "arguments": args } });
    let (_, v) = send(&w.app, "POST", "/api/mcp", token, Some(payload)).await;
    if v["result"]["isError"] == true {
        return Err(v["result"]["structuredContent"]["code"].as_str().unwrap_or("").to_string());
    }
    Ok(v["result"]["structuredContent"].clone())
}

fn field(name: &str) -> Value {
    json!({ "name": name, "label": name, "field_type": "Text", "required": false, "ferpa_sensitive": false })
}

/// An RSA key made with the openssl command: its PEM and its public half as a JWK.
fn new_key(kid: &str) -> (String, Value) {
    let dir = std::env::temp_dir().join(format!("idp-key-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("key.pem");
    let made = std::process::Command::new("openssl").args(["genpkey", "-algorithm", "RSA", "-pkeyopt", "rsa_keygen_bits:2048", "-out"]).arg(&path).output().expect("openssl");
    assert!(made.status.success());
    let modulus = std::process::Command::new("openssl").args(["rsa", "-noout", "-modulus", "-in"]).arg(&path).output().expect("openssl");
    let text = String::from_utf8(modulus.stdout).unwrap();
    let hex = text.trim().strip_prefix("Modulus=").expect("a modulus");
    let bytes: Vec<u8> = (0..hex.len()).step_by(2).map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap()).collect();
    let n = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes);
    (std::fs::read_to_string(&path).unwrap(), json!({ "kty": "RSA", "kid": kid, "alg": "RS256", "use": "sig", "n": n, "e": "AQAB" }))
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
    let second = format!("second-{tag}@state.edu");
    member(&state, &second, "admin");
    let app = build_app_with_state(state).expect("router");
    let (pem, jwk) = new_key("k1");
    let w = World {
        _db: db,
        app,
        owner: issue_test_token_and_user(OWNER),
        second: issue_test_token_and_user(&second),
        platform: issue_test_token_and_user(PLATFORM_ADMIN),
        slug: format!("ip-{tag}"),
        kid: "k1".to_string(),
        pem,
    };
    for (key, value) in [("oidc.issuer", json!(ISSUER)), ("oidc.audience", json!(AUDIENCE)), ("oidc.jwks", json!({ "keys": [jwk] }))] {
        let (status, body) = send(&w.app, "PUT", &format!("/api/v1/settings/{key}"), &w.platform, Some(value)).await;
        assert_eq!(status, StatusCode::OK, "{key}: {body}");
    }
    let manifest = json!({
        "slug": w.slug, "title": "Grants Office", "description": "d", "organization_code": "PHYS", "department": "physics",
        "workspace_id": WORKSPACE, "views": [], "pages": [],
        "tables": [{ "id": "proposals", "name": "Proposals", "slug": "proposals", "fields": [field("title"), field("status"), field("notes")] }]
    });
    let p = tool(&w, &w.owner, "propose_app_change", json!({ "workspace_id": WORKSPACE, "manifest": manifest, "summary": "An app" })).await.unwrap();
    tool(&w, &w.owner, "decide_proposal", json!({ "proposal_id": p["proposal_id"], "decision": "approve", "rationale": "Set up", "digest": p["digest"] })).await.unwrap();
    for (title, status) in [("Telescope", "Draft"), ("Laser", "Final")] {
        let (code, row) = send(&w.app, "POST", &format!("/api/v1/apps/{}/tables/proposals/records", w.slug), &w.owner, Some(json!({ "data": { "title": title, "status": status, "notes": "private" } }))).await;
        assert_eq!(code, StatusCode::CREATED, "{row}");
    }
    w
}

/// A client with the given ceiling, decided by a second person when it writes. Returns its id.
async fn client(w: &World, kind: &str, read: &[&str], write: &[&str], create: bool) -> String {
    let mut body = json!({ "name": "Lab tracker", "operator": "Physics", "kind": kind, "redirect_uris": [CALLBACK] });
    if kind != "agent" {
        body["ceiling"] = json!({ "apps": [{ "app_slug": w.slug, "tables": [{ "table_id": "proposals", "read": read, "write": write, "create": create, "delete": false }] }] });
    }
    let maker = if kind == "agent" { &w.platform } else { &w.owner };
    let (status, made) = send(&w.app, "POST", "/api/v1/clients", maker, Some(body)).await;
    assert_eq!(status, StatusCode::CREATED, "{made}");
    let id = made["client_id"].as_str().unwrap().to_string();
    let decider = if kind == "agent" { &w.platform } else if write.is_empty() && !create { &w.owner } else { &w.second };
    let (status, decided) = send(&w.app, "POST", &format!("/api/v1/clients/{id}/decide"), decider, Some(json!({ "decision": "approve", "rationale": "Reviewed" }))).await;
    assert_eq!(status, StatusCode::OK, "{decided}");
    id
}

fn jwt(w: &World, claims: Value) -> String {
    let mut header = Header::new(Algorithm::RS256);
    header.kid = Some(w.kid.clone());
    jsonwebtoken::encode(&header, &claims, &EncodingKey::from_rsa_pem(w.pem.as_bytes()).unwrap()).unwrap()
}

fn claims(sub: &str, client_claim: Option<(&str, &str)>) -> Value {
    let mut c = json!({ "iss": ISSUER, "aud": AUDIENCE, "sub": sub, "exp": chrono::Utc::now().timestamp() + 600 });
    if let Some((name, id)) = client_claim {
        c[name] = json!(id);
    }
    c
}

fn records_uri(w: &World) -> String {
    format!("/api/v1/apps/{}/tables/proposals/records", w.slug)
}

// 1. A verified token for a registered client lists rows within the ceiling.
#[tokio::test]
async fn a_verified_token_for_a_registered_client_lists_rows_within_the_ceiling() {
    let w = world().await;
    let id = client(&w, "public", &["title"], &[], false).await;
    // Either claim names the client: `client_id` (RFC 9068) or `azp` (OpenID Connect).
    for claim in ["client_id", "azp"] {
        let token = jwt(&w, claims(OWNER, Some((claim, &id))));
        let (status, list) = send(&w.app, "GET", &records_uri(&w), &token, None).await;
        assert_eq!(status, StatusCode::OK, "{claim}: {list}");
        let rows = list["records"].as_array().expect("records");
        assert_eq!(rows.len(), 2, "{claim}");
        for row in rows {
            assert!(row["data"].get("title").is_some(), "{claim}: the ceiling reads title");
            assert!(row["data"].get("notes").is_none() && row["data"].get("status").is_none(), "{claim}: nothing outside the ceiling: {row}");
        }
        // The ceiling's limits apply, whoever signed in: a write is refused as the phase 2 token is.
        let (status, body) = send(&w.app, "POST", &records_uri(&w), &token, Some(json!({ "data": { "title": "x" } }))).await;
        assert_eq!((status, body["code"].as_str()), (StatusCode::FORBIDDEN, Some("client_scope")), "{claim}: {body}");
        let (status, body) = send(&w.app, "GET", "/api/v1/clients", &token, None).await;
        assert_eq!((status, body["code"].as_str()), (StatusCode::FORBIDDEN, Some("client_scope")), "{claim}: an admin route: {body}");
    }
    // The tool list holds only what the ceiling allows.
    let token = jwt(&w, claims(OWNER, Some(("client_id", &id))));
    let (_, listed) = send(&w.app, "POST", "/api/mcp", &token, Some(json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/list" }))).await;
    let names: Vec<&str> = listed["result"]["tools"].as_array().unwrap().iter().filter_map(|t| t["name"].as_str()).collect();
    assert!(names.contains(&"list_records") && !names.contains(&"create_record") && !names.contains(&"decide_proposal"), "{names:?}");
    // A token that names no client is the person's own session, as in phase 10.
    let (status, own) = send(&w.app, "GET", &records_uri(&w), &jwt(&w, claims(OWNER, None)), None).await;
    assert_eq!(status, StatusCode::OK);
    assert!(own["records"][0]["data"].get("notes").is_some(), "the person reads notes: {own}");
}

// 2. A verified token naming an unregistered, pending, disabled or agent client is refused.
#[tokio::test]
async fn a_verified_token_naming_a_client_that_cannot_be_used_is_refused() {
    let w = world().await;
    for (what, id) in [("an unregistered client", "scc_does_not_exist".to_string())] {
        let (status, _) = send(&w.app, "GET", &records_uri(&w), &jwt(&w, claims(OWNER, Some(("client_id", &id)))), None).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "{what}");
    }
    // Pending: registered and not decided.
    let body = json!({ "name": "Pending", "operator": "Physics", "kind": "public", "redirect_uris": [CALLBACK],
        "ceiling": { "apps": [{ "app_slug": w.slug, "tables": [{ "table_id": "proposals", "read": ["title"], "write": [], "create": false, "delete": false }] }] } });
    let (_, made) = send(&w.app, "POST", "/api/v1/clients", &w.owner, Some(body)).await;
    let pending = made["client_id"].as_str().unwrap().to_string();
    let (status, _) = send(&w.app, "GET", &records_uri(&w), &jwt(&w, claims(OWNER, Some(("azp", &pending)))), None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "a pending client");
    // Disabled.
    let active = client(&w, "public", &["title"], &[], false).await;
    let token = jwt(&w, claims(OWNER, Some(("client_id", &active))));
    assert_eq!(send(&w.app, "GET", &records_uri(&w), &token, None).await.0, StatusCode::OK);
    let (status, _) = send(&w.app, "POST", &format!("/api/v1/clients/{active}/disable"), &w.platform, Some(json!({ "reason": "Retired" }))).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(send(&w.app, "GET", &records_uri(&w), &token, None).await.0, StatusCode::UNAUTHORIZED, "a disabled client, on the next call");
    // An agent client signs in through OAuth sessions, where the person picks the access. A provider token cannot stand in.
    let agent = client(&w, "agent", &[], &[], false).await;
    let (status, _) = send(&w.app, "GET", &records_uri(&w), &jwt(&w, claims(OWNER, Some(("client_id", &agent)))), None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "an agent client");
}

// 3. A token with the wrong audience is refused, whichever client it names.
#[tokio::test]
async fn a_token_with_the_wrong_audience_is_refused() {
    let w = world().await;
    let id = client(&w, "public", &["title"], &[], false).await;
    let mut wrong = claims(OWNER, Some(("client_id", &id)));
    wrong["aud"] = json!("some-other-service");
    assert_eq!(send(&w.app, "GET", &records_uri(&w), &jwt(&w, wrong), None).await.0, StatusCode::UNAUTHORIZED);
}

// A write through the provider path names the client in its history, as a phase 2 token does.
#[tokio::test]
async fn a_write_through_a_provider_token_names_the_client_in_its_history() {
    let w = world().await;
    let id = client(&w, "public", &["title", "status"], &["status"], true).await;
    let token = jwt(&w, claims(OWNER, Some(("client_id", &id))));
    let (status, made) = send(&w.app, "POST", &records_uri(&w), &token, Some(json!({ "data": { "status": "Made by the app" } }))).await;
    assert_eq!(status, StatusCode::CREATED, "{made}");
    let record = made["id"].as_str().unwrap();
    let (_, history) = send(&w.app, "GET", &format!("/api/v1/apps/{}/records/{record}/history", w.slug), &w.owner, None).await;
    assert!(history.to_string().contains(&id), "history names the client: {history}");
}
