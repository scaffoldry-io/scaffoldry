//! External apps phase 2: signing in, consent, and tokens.
//!
//! An outside app is a registered client. The user signs in to it with the authorization code flow and PKCE (S256
//! only), sees the client, its operator and its ceiling, and allows or denies. The client trades the code for a
//! one-hour token. RFC 6749, RFC 7009, RFC 7636, RFC 8414, RFC 8707, RFC 9700 and RFC 9728.

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
const CALLBACK: &str = "https://tracker.example.edu/callback";

struct World {
    db: common::TestDb,
    app: Router,
    owner: String,
    platform: String,
    slug: String,
}

struct Reply {
    status: StatusCode,
    headers: axum::http::HeaderMap,
    body: Value,
}

async fn call(app: &Router, method: &str, uri: &str, token: Option<&str>, content_type: Option<&str>, body: Body, extra: &[(&str, &str)]) -> Reply {
    let mut b = Request::builder().method(method).uri(uri).header("host", HOST);
    if let Some(t) = token {
        b = b.header("authorization", t);
    }
    if let Some(ct) = content_type {
        b = b.header("content-type", ct);
    }
    for (k, v) in extra {
        b = b.header(*k, *v);
    }
    let resp = app.clone().oneshot(b.body(body).unwrap()).await.unwrap();
    let (status, headers) = (resp.status(), resp.headers().clone());
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    Reply { status, headers, body: serde_json::from_slice(&bytes).unwrap_or(Value::Null) }
}

async fn rest(app: &Router, method: &str, uri: &str, token: &str, body: Option<Value>) -> Reply {
    let bearer = format!("Bearer {token}");
    match body {
        Some(v) => call(app, method, uri, Some(&bearer), Some("application/json"), Body::from(v.to_string()), &[]).await,
        None => call(app, method, uri, Some(&bearer), None, Body::empty(), &[]).await,
    }
}

fn field(name: &str) -> Value {
    json!({ "name": name, "label": name, "field_type": "Text", "required": false, "ferpa_sensitive": false })
}

async fn tool(w: &World, token: &str, name: &str, args: Value) -> Value {
    let payload = json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": { "name": name, "arguments": args } });
    let r = rest(&w.app, "POST", "/api/mcp", token, Some(payload)).await;
    assert!(r.body["result"]["isError"] != true, "{name}: {}", r.body);
    r.body["result"]["structuredContent"].clone()
}

async fn world() -> World {
    let db = common::TestDb::new();
    let state = db.state();
    state.seed_demo().expect("seed");
    let app = build_app_with_state(state).expect("router");
    let tag = uuid::Uuid::new_v4().to_string()[..8].to_string();
    let w = World { db, app, owner: issue_test_token_and_user(OWNER), platform: issue_test_token_and_user(PLATFORM_ADMIN), slug: format!("oa-{tag}") };
    let manifest = json!({
        "slug": w.slug, "title": "Grants Office", "description": "d", "organization_code": "PHYS", "department": "physics",
        "workspace_id": WORKSPACE, "views": [], "pages": [],
        "tables": [{ "id": "proposals", "name": "Proposals", "slug": "proposals", "fields": [field("title"), field("status")] }]
    });
    let p = tool(&w, &w.owner, "propose_app_change", json!({ "workspace_id": WORKSPACE, "manifest": manifest, "summary": "An app" })).await;
    tool(&w, &w.owner, "decide_proposal", json!({ "proposal_id": p["proposal_id"], "decision": "approve", "rationale": "Set up", "digest": p["digest"] })).await;
    w
}

/// An approved client with a read-only ceiling. Returns its id and, for a confidential client, its secret.
async fn client(w: &World, kind: &str) -> (String, Option<String>) {
    let body = json!({
        "name": format!("Grant tracker {kind}"), "operator": "Physics Department", "kind": kind, "redirect_uris": [CALLBACK],
        "ceiling": { "apps": [{ "app_slug": w.slug, "tables": [{ "table_id": "proposals", "read": ["title"], "write": [], "create": false, "delete": false }] }] }
    });
    let made = rest(&w.app, "POST", "/api/v1/clients", &w.owner, Some(body)).await;
    assert_eq!(made.status, StatusCode::CREATED, "{}", made.body);
    let id = made.body["client_id"].as_str().unwrap().to_string();
    let decided = rest(&w.app, "POST", &format!("/api/v1/clients/{id}/decide"), &w.owner, Some(json!({ "decision": "approve", "rationale": "Reviewed" }))).await;
    assert_eq!(decided.status, StatusCode::OK, "{}", decided.body);
    (id, decided.body["client_secret"].as_str().map(str::to_string))
}

fn challenge_of(verifier: &str) -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
}

const VERIFIER: &str = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";

fn params(client_id: &str, state: &str) -> Value {
    json!({
        "client_id": client_id, "redirect_uri": CALLBACK, "response_type": "code", "state": state,
        "code_challenge": challenge_of(VERIFIER), "code_challenge_method": "S256", "resource": ISSUER
    })
}

fn query_of(p: &Value) -> String {
    p.as_object().unwrap().iter().filter_map(|(k, v)| v.as_str().map(|s| format!("{k}={s}"))).collect::<Vec<_>>().join("&")
}

async fn consent(w: &World, token: &str, p: &Value) -> Reply {
    let bearer = format!("Bearer {token}");
    call(&w.app, "GET", &format!("/oauth/authorize?{}", query_of(p)), Some(&bearer), None, Body::empty(), &[]).await
}

async fn decide_consent(w: &World, token: &str, p: &Value, decision: &str) -> Reply {
    let mut body = p.clone();
    body["decision"] = json!(decision);
    let bearer = format!("Bearer {token}");
    call(&w.app, "POST", "/oauth/authorize", Some(&bearer), Some("application/json"), Body::from(body.to_string()), &[]).await
}

/// The `code` and `state` in the address a consent redirects to.
fn parts_of(redirect: &str) -> (Option<String>, Option<String>, Option<String>) {
    let query = redirect.split_once('?').map(|(_, q)| q).unwrap_or_default();
    let get = |name: &str| query.split('&').find_map(|p| p.strip_prefix(&format!("{name}="))).map(str::to_string);
    (get("code"), get("state"), get("error"))
}

async fn exchange(w: &World, form: &str, basic: Option<(&str, &str)>) -> Reply {
    let auth = basic.map(|(id, secret)| format!("Basic {}", base64::engine::general_purpose::STANDARD.encode(format!("{id}:{secret}"))));
    call(&w.app, "POST", "/oauth/token", auth.as_deref(), Some("application/x-www-form-urlencoded"), Body::from(form.to_string()), &[]).await
}

fn token_form(client_id: &str, code: &str, verifier: &str, redirect: &str) -> String {
    format!("grant_type=authorization_code&code={code}&redirect_uri={redirect}&client_id={client_id}&code_verifier={verifier}")
}

/// Signs in as `token`'s user and gets a code.
async fn code_for(w: &World, token: &str, client_id: &str) -> String {
    let allowed = decide_consent(w, token, &params(client_id, "xyz"), "allow").await;
    assert_eq!(allowed.status, StatusCode::OK, "{}", allowed.body);
    let (code, state, _) = parts_of(allowed.body["redirect_to"].as_str().expect("a redirect"));
    assert_eq!(state.as_deref(), Some("xyz"));
    code.expect("a code")
}

async fn token_for(w: &World, token: &str, client_id: &str) -> String {
    let code = code_for(w, token, client_id).await;
    let r = exchange(w, &token_form(client_id, &code, VERIFIER, CALLBACK), None).await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.body);
    r.body["access_token"].as_str().unwrap().to_string()
}

async fn me(w: &World, access_token: &str) -> StatusCode {
    rest(&w.app, "GET", "/api/v1/auth/me", access_token, None).await.status
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

// 1. A full flow with PKCE issues a token. The same code twice fails and revokes the first token.
#[tokio::test]
async fn a_full_flow_issues_a_token_and_a_second_use_of_the_code_revokes_it() {
    let w = world().await;
    let (id, _) = client(&w, "public").await;

    let shown = consent(&w, &w.owner, &params(&id, "xyz")).await;
    assert_eq!(shown.status, StatusCode::OK, "{}", shown.body);
    assert_eq!(shown.body["client"]["name"], "Grant tracker public");
    assert_eq!(shown.body["client"]["operator"], "Physics Department");
    assert_eq!(shown.body["client"]["callback_origin"], "https://tracker.example.edu");
    assert_eq!(shown.body["sentences"], json!(["Read the title of Proposals in Grants Office."]));
    assert_eq!(shown.body["remembered"], false);
    assert!(shown.headers.get("location").is_none(), "consent never redirects");

    let code = code_for(&w, &w.owner, &id).await;
    let issued = exchange(&w, &token_form(&id, &code, VERIFIER, CALLBACK), None).await;
    assert_eq!(issued.status, StatusCode::OK, "{}", issued.body);
    assert_eq!(issued.body["token_type"], "Bearer");
    assert_eq!(issued.body["expires_in"], 3600);
    assert_eq!(issued.headers.get("cache-control").and_then(|v| v.to_str().ok()), Some("no-store"));
    let access = issued.body["access_token"].as_str().unwrap().to_string();
    assert_eq!(me(&w, &access).await, StatusCode::OK, "the token works for its user");
    assert_eq!(rest(&w.app, "GET", "/api/v1/auth/me", &access, None).await.body["user"]["eppn"], OWNER);

    // The same code again fails, and the token it issued stops working.
    let again = exchange(&w, &token_form(&id, &code, VERIFIER, CALLBACK), None).await;
    assert_eq!(again.status, StatusCode::BAD_REQUEST);
    assert_eq!(again.body["error"], "invalid_grant");
    assert_eq!(me(&w, &access).await, StatusCode::UNAUTHORIZED, "the first token was revoked");

    // A client token reaches only the record routes its ceiling covers. Everything else says `client_scope`.
    let second = token_for(&w, &w.owner, &id).await;
    let apps = rest(&w.app, "GET", "/api/v1/me/apps", &second, None).await;
    assert_eq!(apps.status, StatusCode::FORBIDDEN, "{}", apps.body);
    assert_eq!(apps.body["code"], "client_scope");
}

// 2. A wrong verifier, a mismatched redirect address, a missing state or `plain` is refused.
#[tokio::test]
async fn a_bad_request_is_refused_and_a_bad_client_or_address_never_redirects() {
    let w = world().await;
    let (id, _) = client(&w, "public").await;

    // At the token endpoint.
    let code = code_for(&w, &w.owner, &id).await;
    let wrong_verifier = exchange(&w, &token_form(&id, &code, "not-the-verifier-not-the-verifier-not-the-verifier", CALLBACK), None).await;
    assert_eq!((wrong_verifier.status, wrong_verifier.body["error"].as_str()), (StatusCode::BAD_REQUEST, Some("invalid_grant")));
    let code = code_for(&w, &w.owner, &id).await;
    let other_redirect = exchange(&w, &token_form(&id, &code, VERIFIER, "https://evil.example.com/callback"), None).await;
    assert_eq!((other_redirect.status, other_redirect.body["error"].as_str()), (StatusCode::BAD_REQUEST, Some("invalid_grant")));
    let code = code_for(&w, &w.owner, &id).await;
    let (other_id, _) = client(&w, "public").await;
    let wrong_client = exchange(&w, &token_form(&other_id, &code, VERIFIER, CALLBACK), None).await;
    assert_eq!(wrong_client.status, StatusCode::BAD_REQUEST, "a code belongs to one client: {}", wrong_client.body);
    let unsupported = exchange(&w, "grant_type=password&username=a&password=b", None).await;
    assert_eq!((unsupported.status, unsupported.body["error"].as_str()), (StatusCode::BAD_REQUEST, Some("unsupported_grant_type")));

    // At the consent screen.
    let mut cases: Vec<(&str, Value)> = Vec::new();
    let mut p = params(&id, "xyz");
    p["code_challenge_method"] = json!("plain");
    cases.push(("plain", p));
    let mut p = params(&id, "xyz");
    p["state"] = json!("");
    cases.push(("an empty state", p));
    let mut p = params(&id, "xyz");
    p.as_object_mut().unwrap().remove("state");
    cases.push(("no state", p));
    let mut p = params(&id, "xyz");
    p.as_object_mut().unwrap().remove("code_challenge");
    cases.push(("no challenge", p));
    let mut p = params(&id, "xyz");
    p["response_type"] = json!("token");
    cases.push(("the implicit grant", p));
    let mut p = params(&id, "xyz");
    p["resource"] = json!("https://elsewhere.example.com");
    cases.push(("another resource", p));
    let mut p = params(&id, "xyz");
    p["redirect_uri"] = json!("https://evil.example.com/callback");
    cases.push(("an unregistered address", p));
    cases.push(("an unknown client", params("scc_nope", "xyz")));
    for (what, p) in cases {
        for reply in [consent(&w, &w.owner, &p).await, decide_consent(&w, &w.owner, &p, "allow").await] {
            assert_eq!(reply.status, StatusCode::BAD_REQUEST, "{what}: {}", reply.body);
            assert!(reply.headers.get("location").is_none() && reply.body.get("redirect_to").is_none(), "{what}: nothing redirects");
        }
    }

    // A person must be signed in.
    let anonymous = call(&w.app, "GET", &format!("/oauth/authorize?{}", query_of(&params(&id, "xyz"))), None, None, Body::empty(), &[]).await;
    assert_eq!(anonymous.status, StatusCode::UNAUTHORIZED);

    // Deny redirects with an error and the state.
    let denied = decide_consent(&w, &w.owner, &params(&id, "keep-me"), "deny").await;
    assert_eq!(denied.status, StatusCode::OK, "{}", denied.body);
    let (code, state, error) = parts_of(denied.body["redirect_to"].as_str().unwrap());
    assert_eq!((code, state.as_deref(), error.as_deref()), (None, Some("keep-me"), Some("access_denied")));
}

// 3. A token is refused after one hour, after the user disconnects, after the client is disabled, and after the user is put on hold.
#[tokio::test]
async fn a_token_stops_working_when_it_should() {
    let w = world().await;
    let (id, _) = client(&w, "public").await;

    // One hour.
    let hour = token_for(&w, &w.owner, &id).await;
    assert_eq!(me(&w, &hour).await, StatusCode::OK);
    sql(&w, "UPDATE api_tokens SET expires_at = NOW() - INTERVAL '1 second' WHERE kind = 'client'".to_string());
    assert_eq!(me(&w, &hour).await, StatusCode::UNAUTHORIZED, "after an hour");

    // The user disconnects.
    let linked = token_for(&w, &w.owner, &id).await;
    assert_eq!(me(&w, &linked).await, StatusCode::OK);
    let gone = rest(&w.app, "DELETE", &format!("/api/v1/me/connections/{id}"), &w.owner, None).await;
    assert_eq!(gone.status, StatusCode::OK, "{}", gone.body);
    assert_eq!(me(&w, &linked).await, StatusCode::UNAUTHORIZED, "after the user disconnects");

    // The client is disabled.
    let live = token_for(&w, &w.owner, &id).await;
    assert_eq!(me(&w, &live).await, StatusCode::OK);
    let off = rest(&w.app, "POST", &format!("/api/v1/clients/{id}/disable"), &w.owner, Some(json!({ "reason": "Done" }))).await;
    assert_eq!(off.status, StatusCode::OK, "{}", off.body);
    assert_eq!(me(&w, &live).await, StatusCode::UNAUTHORIZED, "after the client is disabled");
    let refused = consent(&w, &w.owner, &params(&id, "xyz")).await;
    assert_eq!(refused.status, StatusCode::BAD_REQUEST, "a disabled client cannot be signed in to");

    // The user is put on hold.
    let (id, _) = client(&w, "public").await;
    let name = format!("held-{}@state.edu", &uuid::Uuid::new_v4().to_string()[..8]);
    let created = rest(&w.app, "POST", "/api/v1/admin/users", &w.platform, Some(json!({
        "userName": name, "name": "Held Person", "email": name, "affiliation": "faculty", "department": "Physics", "title": "Lecturer", "reason": "Test fixture"
    }))).await;
    assert_eq!(created.status, StatusCode::CREATED, "{}", created.body);
    let user_token = issue_test_token_and_user(&name);
    let theirs = token_for(&w, &user_token, &id).await;
    assert_eq!(me(&w, &theirs).await, StatusCode::OK);
    let held = rest(&w.app, "POST", &format!("/api/v1/admin/users/{}/hold", created.body["user"]["id"].as_str().unwrap()), &w.platform, Some(json!({ "hold": true, "reason": "Under review" }))).await;
    assert_eq!(held.status, StatusCode::OK, "{}", held.body);
    assert_eq!(me(&w, &theirs).await, StatusCode::UNAUTHORIZED, "after the user is put on hold");
}

// 4. Consent is remembered until the ceiling changes. A changed ceiling asks again.
#[tokio::test]
async fn a_changed_ceiling_asks_for_consent_again() {
    let w = world().await;
    let (id, _) = client(&w, "public").await;
    assert_eq!(consent(&w, &w.owner, &params(&id, "xyz")).await.body["remembered"], false);
    code_for(&w, &w.owner, &id).await;
    assert_eq!(consent(&w, &w.owner, &params(&id, "xyz")).await.body["remembered"], true, "remembered for this user and client");
    let someone_else = issue_test_token_and_user("someone.else@state.edu");
    assert_eq!(consent(&w, &someone_else, &params(&id, "xyz")).await.body["remembered"], false, "not for another user");

    sql(&w, format!(
        "UPDATE app_clients SET ceiling = jsonb_set(ceiling, '{{apps,0,tables,0,read}}', '[\"title\",\"status\"]') WHERE client_id = '{id}'"
    ));
    let asked = consent(&w, &w.owner, &params(&id, "xyz")).await;
    assert_eq!(asked.body["remembered"], false, "the ceiling changed: {}", asked.body);
    assert_eq!(asked.body["sentences"], json!(["Read the title and status of Proposals in Grants Office."]));
}

// 5. The metadata documents carry the fields their RFCs require.
#[tokio::test]
async fn the_metadata_documents_have_the_required_fields() {
    let w = world().await;
    let server = call(&w.app, "GET", "/.well-known/oauth-authorization-server", None, None, Body::empty(), &[]).await;
    assert_eq!(server.status, StatusCode::OK);
    let m = &server.body;
    // RFC 8414 section 2: issuer, response_types_supported, and the endpoints a code flow needs.
    assert_eq!(m["issuer"], ISSUER, "the issuer is the address the document was read from");
    assert!(m["issuer"].as_str().unwrap().starts_with("https://") && !m["issuer"].as_str().unwrap().contains('?') && !m["issuer"].as_str().unwrap().ends_with('/'));
    assert_eq!(m["response_types_supported"], json!(["code"]));
    assert_eq!(m["grant_types_supported"], json!(["authorization_code", "refresh_token"]));
    assert_eq!(m["code_challenge_methods_supported"], json!(["S256"]));
    for (key, path) in [("authorization_endpoint", "/oauth/authorize"), ("token_endpoint", "/oauth/token"), ("revocation_endpoint", "/oauth/revoke")] {
        assert_eq!(m[key], format!("{ISSUER}{path}"), "{key}");
    }
    assert!(m["token_endpoint_auth_methods_supported"].as_array().unwrap().iter().any(|v| v == "client_secret_basic"));
    assert!(m["token_endpoint_auth_methods_supported"].as_array().unwrap().iter().any(|v| v == "none"));

    // RFC 9728 section 2: resource, and the authorization servers that issue tokens for it.
    let resource = call(&w.app, "GET", "/.well-known/oauth-protected-resource", None, None, Body::empty(), &[]).await;
    assert_eq!(resource.status, StatusCode::OK);
    let r = &resource.body;
    assert!(r["resource"].as_str().unwrap().starts_with(ISSUER), "{r}");
    assert!(r["authorization_servers"].as_array().unwrap().iter().any(|a| a == ISSUER), "this server issues tokens for itself: {r}");
    assert_eq!(r["bearer_methods_supported"], json!(["header"]));
}

// A confidential client authenticates with its secret, and the client can revoke its own token.
#[tokio::test]
async fn a_confidential_client_authenticates_and_may_revoke_its_own_tokens() {
    let w = world().await;
    let (id, secret) = client(&w, "confidential").await;
    let secret = secret.expect("a secret");

    let code = code_for(&w, &w.owner, &id).await;
    let no_secret = exchange(&w, &token_form(&id, &code, VERIFIER, CALLBACK), None).await;
    assert_eq!((no_secret.status, no_secret.body["error"].as_str()), (StatusCode::UNAUTHORIZED, Some("invalid_client")));
    let wrong = exchange(&w, &token_form(&id, &code, VERIFIER, CALLBACK), Some((&id, "scs_wrong"))).await;
    assert_eq!(wrong.status, StatusCode::UNAUTHORIZED);
    let code = code_for(&w, &w.owner, &id).await;
    let ok = exchange(&w, &token_form(&id, &code, VERIFIER, CALLBACK), Some((&id, &secret))).await;
    assert_eq!(ok.status, StatusCode::OK, "{}", ok.body);
    let access = ok.body["access_token"].as_str().unwrap().to_string();
    assert_eq!(me(&w, &access).await, StatusCode::OK);

    // Another client cannot revoke it. The reply is the same either way.
    let (other, _) = client(&w, "public").await;
    let revoke = |client_id: String, secret: Option<String>, token: String| {
        let app = w.app.clone();
        async move {
            let auth = secret.map(|s| format!("Basic {}", base64::engine::general_purpose::STANDARD.encode(format!("{client_id}:{s}"))));
            let form = if auth.is_some() { format!("token={token}") } else { format!("token={token}&client_id={client_id}") };
            call(&app, "POST", "/oauth/revoke", auth.as_deref(), Some("application/x-www-form-urlencoded"), Body::from(form), &[]).await
        }
    };
    assert_eq!(revoke(other, None, access.clone()).await.status, StatusCode::OK);
    assert_eq!(me(&w, &access).await, StatusCode::OK, "another client's request changed nothing");
    assert_eq!(revoke(id.clone(), Some(secret.clone()), access.clone()).await.status, StatusCode::OK);
    assert_eq!(me(&w, &access).await, StatusCode::UNAUTHORIZED, "revoked by its client");
    assert_eq!(revoke(id, Some(secret), "scf_not_a_token".to_string()).await.status, StatusCode::OK, "an unknown token is not an error (RFC 7009)");
}

// A user lists and disconnects their clients, and a disconnection is a ledger entry.
#[tokio::test]
async fn a_user_lists_and_disconnects_their_clients() {
    let w = world().await;
    let (id, _) = client(&w, "public").await;
    assert_eq!(rest(&w.app, "GET", "/api/v1/me/connections", &w.owner, None).await.body, json!([]));
    token_for(&w, &w.owner, &id).await;
    let listed = rest(&w.app, "GET", "/api/v1/me/connections", &w.owner, None).await;
    let row = &listed.body[0];
    assert_eq!((row["client_id"].as_str(), row["name"].as_str(), row["operator"].as_str()), (Some(id.as_str()), Some("Grant tracker public"), Some("Physics Department")), "{}", listed.body);
    assert_eq!(row["sentences"], json!(["Read the title of Proposals in Grants Office."]));
    let theirs = rest(&w.app, "GET", "/api/v1/me/connections", &issue_test_token_and_user("someone.else@state.edu"), None).await;
    assert_eq!(theirs.body, json!([]), "a person sees only their own");

    let count = |body: &Value| body["entries"].as_array().unwrap().iter().filter(|e| e["decision_type"] == "ClientDisconnected").count();
    let before = rest(&w.app, "GET", "/api/v1/governance/ledger", &w.platform, None).await.body;
    assert_eq!(rest(&w.app, "DELETE", "/api/v1/me/connections/scc_unknown", &w.owner, None).await.status, StatusCode::NOT_FOUND);
    assert_eq!(count(&rest(&w.app, "GET", "/api/v1/governance/ledger", &w.platform, None).await.body), count(&before), "a refusal writes nothing");
    assert_eq!(rest(&w.app, "DELETE", &format!("/api/v1/me/connections/{id}"), &w.owner, None).await.status, StatusCode::OK);
    assert_eq!(count(&rest(&w.app, "GET", "/api/v1/governance/ledger", &w.platform, None).await.body), count(&before) + 1);
    assert_eq!(rest(&w.app, "GET", "/api/v1/me/connections", &w.owner, None).await.body, json!([]));
    assert_eq!(consent(&w, &w.owner, &params(&id, "xyz")).await.body["remembered"], false, "consent is forgotten");
}

// The origins of an active client's redirect addresses may call the server from a browser.
#[tokio::test]
async fn a_client_origin_may_call_from_a_browser() {
    let w = world().await;
    let preflight = |origin: &'static str| {
        let app = w.app.clone();
        async move {
            call(&app, "OPTIONS", "/oauth/token", None, None, Body::empty(), &[("origin", origin), ("access-control-request-method", "POST"), ("access-control-request-headers", "content-type")]).await
        }
    };
    let allow = |r: &Reply| r.headers.get("access-control-allow-origin").and_then(|v| v.to_str().ok()).map(str::to_string);
    assert_eq!(allow(&preflight("https://tracker.example.edu").await), None, "not before a client is registered");
    let (id, _) = client(&w, "public").await;
    assert_eq!(allow(&preflight("https://tracker.example.edu").await).as_deref(), Some("https://tracker.example.edu"));
    assert_eq!(allow(&preflight("https://other.example.edu").await), None, "only the registered origin");
    rest(&w.app, "POST", &format!("/api/v1/clients/{id}/disable"), &w.owner, Some(json!({ "reason": "Done" }))).await;
    assert_eq!(allow(&preflight("https://tracker.example.edu").await), None, "not once the client is disabled");
}
