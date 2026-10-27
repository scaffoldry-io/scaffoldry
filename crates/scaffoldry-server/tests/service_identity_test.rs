//! External apps phase 5: a confidential client's own service identity, `client:{client_id}`.
//!
//! Its rights are grants a second person approved. `client_credentials` issues a one-hour token for it. The ceiling
//! still applies, so what it reads is what its grants and its ceiling both allow. Writes carry the client and
//! `Automation`. Disabling the client stops it on the next call.

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::Router;
use base64::Engine;
use http_body_util::BodyExt;
use scaffoldry_server::build_app_with_state;
use scaffoldry_server::service::identity::issue_test_token_and_user;
use scaffoldry_server::state::{CollaboratorRecord, RoleRow, ServerState};
use serde_json::{json, Value};
use tower::ServiceExt;
use uuid::Uuid;

const WORKSPACE: &str = "ws-physics-optics";
const OWNER: &str = "einstein@physics.state.edu";
const ADMIN: &str = "jordan.lee@state.edu";
const ADMIN_TWO: &str = "second.admin@state.edu";
const HOST: &str = "api.test.edu";
const CALLBACK: &str = "https://nightly.example.edu/callback";
const ROOT: &str = "00000000-0000-0000-0000-000000000001";

struct World {
    _db: common::TestDb,
    app: Router,
    owner: String,
    second: String,
    admin: String,
    admin_two: String,
    slug: String,
}

async fn send(app: &Router, method: &str, uri: &str, auth: Option<String>, content_type: Option<&str>, body: Body) -> (StatusCode, Value) {
    let mut b = Request::builder().method(method).uri(uri).header("host", HOST);
    if let Some(a) = auth {
        b = b.header("authorization", a);
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
    let auth = Some(format!("Bearer {token}"));
    match body {
        Some(v) => send(app, method, uri, auth, Some("application/json"), Body::from(v.to_string())).await,
        None => send(app, method, uri, auth, None, Body::empty()).await,
    }
}

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

fn collaborator(state: &ServerState, eppn: &str, role: &str) {
    state.collaborators.write().unwrap().entry(WORKSPACE.to_string()).or_default().push(CollaboratorRecord {
        id: Uuid::new_v4().to_string(),
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
    let tag = Uuid::new_v4().to_string()[..8].to_string();
    let second = format!("second-{tag}@state.edu");
    collaborator(&state, &second, "admin");
    state
        .persist_role(&RoleRow {
            id: Uuid::new_v4(),
            person_id: Uuid::new_v4(),
            eppn: ADMIN_TWO.to_string(),
            organization_id: Uuid::parse_str(ROOT).unwrap(),
            role_title: "platform_admin".to_string(),
            scoped_affiliation: "platform_admin".to_string(),
            is_primary: false,
            source: "test".to_string(),
            position_key: None,
        })
        .expect("appoint");
    let app = build_app_with_state(state).expect("router");
    let w = World {
        _db: db,
        app,
        owner: issue_test_token_and_user(OWNER),
        second: issue_test_token_and_user(&second),
        admin: issue_test_token_and_user(ADMIN),
        admin_two: issue_test_token_and_user(ADMIN_TWO),
        slug: format!("si-{tag}"),
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

/// A confidential client with a service identity. It reads `title` and `status`, and writes `status` when `writes`.
/// `notes` is outside it. Returns its id and its secret.
async fn service_client(w: &World, writes: bool) -> (String, String) {
    let (write, create) = if writes { (json!(["status"]), true) } else { (json!([]), false) };
    let body = json!({
        "name": "Nightly summary", "operator": "Physics Department", "kind": "confidential", "redirect_uris": [CALLBACK], "service_identity": true,
        "ceiling": { "apps": [{ "app_slug": w.slug, "tables": [{ "table_id": "proposals", "read": ["title", "status"], "write": write, "create": create, "delete": false }] }] }
    });
    let (status, made) = rest(&w.app, "POST", "/api/v1/clients", &w.owner, Some(body)).await;
    assert_eq!(status, StatusCode::CREATED, "{made}");
    assert_eq!(made["service_identity"], true, "{made}");
    let id = made["client_id"].as_str().unwrap().to_string();
    let (status, decided) = rest(&w.app, "POST", &format!("/api/v1/clients/{id}/decide"), &w.second, Some(json!({ "decision": "approve", "rationale": "Reviewed" }))).await;
    assert_eq!(status, StatusCode::OK, "{decided}");
    (id, decided["client_secret"].as_str().expect("a confidential client gets a secret").to_string())
}

fn basic(id: &str, secret: &str) -> String {
    format!("Basic {}", base64::engine::general_purpose::STANDARD.encode(format!("{id}:{secret}")))
}

async fn client_credentials(w: &World, id: &str, secret: &str) -> (StatusCode, Value) {
    send(&w.app, "POST", "/oauth/token", Some(basic(id, secret)), Some("application/x-www-form-urlencoded"), Body::from("grant_type=client_credentials")).await
}

async fn token_of(w: &World, id: &str, secret: &str) -> String {
    let (status, body) = client_credentials(w, id, secret).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["token_type"], "Bearer");
    assert_eq!(body["expires_in"], 3600);
    assert!(body.get("refresh_token").is_none(), "a service identity gets no refresh token: {body}");
    body["access_token"].as_str().unwrap().to_string()
}

/// A grant to the service identity, proposed by one Platform Admin and confirmed by another.
async fn grant(w: &World, client_id: &str, role: &str) {
    let body = json!({ "holder_kind": "person", "holder_id": format!("client:{client_id}"), "role": role, "anchor_kind": "container", "anchor_id": format!("app:{}", w.slug), "source": "client", "reason": "The nightly summary reads proposals" });
    let (status, pending) = rest(&w.app, "POST", "/api/v1/admin/access/grants", &w.admin, Some(body)).await;
    assert_eq!(status, StatusCode::ACCEPTED, "{pending}");
    let change = pending["change_id"].as_str().unwrap();
    let (status, done) = rest(&w.app, "POST", &format!("/api/v1/admin/access/changes/{change}/decide"), &w.admin_two, Some(json!({ "decision": "confirm", "reason": "Reviewed" }))).await;
    assert_eq!(status, StatusCode::OK, "{done}");
}

fn records_uri(w: &World) -> String {
    format!("/api/v1/apps/{}/tables/proposals/records", w.slug)
}

async fn seed_row(w: &World) -> String {
    let (status, row) = rest(&w.app, "POST", &records_uri(w), &w.owner, Some(json!({ "data": { "title": "Telescope", "status": "Draft", "notes": "Private" } }))).await;
    assert_eq!(status, StatusCode::CREATED, "{row}");
    row["id"].as_str().unwrap().to_string()
}

// 1. It reads only what its grants and the ceiling both allow.
#[tokio::test]
async fn a_service_identity_reads_only_what_its_grants_and_the_ceiling_both_allow() {
    let w = world().await;
    let id = seed_row(&w).await;
    let (client_id, secret) = service_client(&w, false).await;
    let token = token_of(&w, &client_id, &secret).await;

    // No grant yet: the ceiling alone gives it nothing.
    let (status, body) = rest(&w.app, "GET", &records_uri(&w), &token, None).await;
    assert!(status == StatusCode::FORBIDDEN || body["records"].as_array().is_some_and(Vec::is_empty), "{status}: {body}");

    // A grant a second person approved gives it the viewer role on the app. The ceiling still drops `notes`.
    grant(&w, &client_id, "viewer").await;
    let (status, list) = rest(&w.app, "GET", &records_uri(&w), &token, None).await;
    assert_eq!(status, StatusCode::OK, "{list}");
    let data = &list["records"][0]["data"];
    assert_eq!(data["title"], "Telescope");
    assert!(data.get("notes").is_none(), "notes is outside the ceiling: {data}");

    // A write is refused: the grant is a viewer's, and the ceiling is read only.
    let (status, body) = rest(&w.app, "PATCH", &format!("{}/{id}", records_uri(&w)), &token, Some(json!({ "data": { "status": "Final" }, "version": 1 }))).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");

    // An editor grant widens the rights and not the ceiling: a read-only ceiling still refuses the write.
    grant(&w, &client_id, "editor").await;
    let (status, body) = rest(&w.app, "PATCH", &format!("{}/{id}", records_uri(&w)), &token, Some(json!({ "data": { "status": "Final" }, "version": 1 }))).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "the ceiling allows no write: {body}");

    // The person who owns the workspace still sees every field.
    let (_, own) = rest(&w.app, "GET", &format!("{}/{id}", records_uri(&w)), &w.owner, None).await;
    assert_eq!(own["data"]["notes"], "Private");
}

// 2. Only a confidential client with a service identity uses `client_credentials`.
#[tokio::test]
async fn a_public_client_cannot_use_client_credentials() {
    let w = world().await;
    // A public client cannot even ask for a service identity.
    let body = json!({ "name": "Browser app", "operator": "Physics", "kind": "public", "redirect_uris": [CALLBACK], "service_identity": true, "ceiling": { "apps": [{ "app_slug": w.slug, "tables": [{ "table_id": "proposals", "read": ["title"], "write": [], "create": false, "delete": false }] }] } });
    let (status, refused) = rest(&w.app, "POST", "/api/v1/clients", &w.owner, Some(body.clone())).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{refused}");

    // A public client without one is refused at the token endpoint.
    let mut plain = body;
    plain["service_identity"] = json!(false);
    let (_, made) = rest(&w.app, "POST", "/api/v1/clients", &w.owner, Some(plain)).await;
    let public_id = made["client_id"].as_str().unwrap().to_string();
    rest(&w.app, "POST", &format!("/api/v1/clients/{public_id}/decide"), &w.owner, Some(json!({ "decision": "approve", "rationale": "Reviewed" }))).await;
    let form = format!("grant_type=client_credentials&client_id={public_id}");
    let (status, refused) = send(&w.app, "POST", "/oauth/token", None, Some("application/x-www-form-urlencoded"), Body::from(form)).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{refused}");
    assert_eq!(refused["error"], "unauthorized_client", "{refused}");

    // A confidential client that did not ask for one is refused too, and a wrong secret is not accepted.
    let body = json!({ "name": "Server app", "operator": "Physics", "kind": "confidential", "redirect_uris": [CALLBACK], "ceiling": { "apps": [{ "app_slug": w.slug, "tables": [{ "table_id": "proposals", "read": ["title"], "write": [], "create": false, "delete": false }] }] } });
    let (_, made) = rest(&w.app, "POST", "/api/v1/clients", &w.owner, Some(body)).await;
    let server_id = made["client_id"].as_str().unwrap().to_string();
    let (_, decided) = rest(&w.app, "POST", &format!("/api/v1/clients/{server_id}/decide"), &w.owner, Some(json!({ "decision": "approve", "rationale": "Reviewed" }))).await;
    let secret = decided["client_secret"].as_str().unwrap().to_string();
    let (status, refused) = client_credentials(&w, &server_id, &secret).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{refused}");
    assert_eq!(refused["error"], "unauthorized_client", "{refused}");

    let (client_id, real_secret) = service_client(&w, false).await;
    let (status, refused) = client_credentials(&w, &client_id, "scs_wrong").await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "{refused}");
    assert_eq!(refused["error"], "invalid_client");
    assert_eq!(client_credentials(&w, &client_id, &real_secret).await.0, StatusCode::OK);
}

// 3. Disabling the client stops its service identity on the next call.
#[tokio::test]
async fn disabling_the_client_stops_its_service_identity_on_the_next_call() {
    let w = world().await;
    seed_row(&w).await;
    let (client_id, secret) = service_client(&w, false).await;
    grant(&w, &client_id, "viewer").await;
    let token = token_of(&w, &client_id, &secret).await;
    assert_eq!(rest(&w.app, "GET", &records_uri(&w), &token, None).await.0, StatusCode::OK);

    let (status, body) = rest(&w.app, "POST", &format!("/api/v1/clients/{client_id}/disable"), &w.owner, Some(json!({ "reason": "No longer used" }))).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(rest(&w.app, "GET", &records_uri(&w), &token, None).await.0, StatusCode::UNAUTHORIZED, "the token that worked a moment ago does not");
    let (status, refused) = client_credentials(&w, &client_id, &secret).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "{refused}");
}

// 4. A write by a service identity names the client and is an automation.
#[tokio::test]
async fn a_write_by_a_service_identity_names_the_client_and_is_an_automation() {
    let w = world().await;
    let id = seed_row(&w).await;
    let (client_id, secret) = service_client(&w, true).await;
    grant(&w, &client_id, "editor").await;
    let token = token_of(&w, &client_id, &secret).await;
    let (status, body) = rest(&w.app, "PATCH", &format!("{}/{id}", records_uri(&w)), &token, Some(json!({ "data": { "status": "Final" }, "version": 1 }))).await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let (_, history) = rest(&w.app, "GET", &format!("/api/v1/apps/{}/records/{id}/history", w.slug), &w.owner, None).await;
    let entry = history["entries"].as_array().unwrap().iter().find(|e| e["action"] == "update").expect("the update is in the history");
    assert_eq!(entry["actor_kind"], "automation", "{entry}");
    assert_eq!(entry["actor"], format!("client:{client_id}"), "the actor is the service identity: {entry}");
    assert!(entry["token_label"].as_str().is_some_and(|l| l.contains(&client_id)), "the history names the client: {entry}");
}

// 5. Making one needs a second person, at registration and at the grant.
#[tokio::test]
async fn a_service_identity_needs_a_second_person() {
    let w = world().await;
    // The owner proposes a service client for a read-only ceiling on one workspace. Owning the workspace is not enough.
    let body = json!({
        "name": "Nightly summary", "operator": "Physics", "kind": "confidential", "redirect_uris": [CALLBACK], "service_identity": true,
        "ceiling": { "apps": [{ "app_slug": w.slug, "tables": [{ "table_id": "proposals", "read": ["title"], "write": [], "create": false, "delete": false }] }] }
    });
    let (_, made) = rest(&w.app, "POST", "/api/v1/clients", &w.owner, Some(body)).await;
    let id = made["client_id"].as_str().unwrap().to_string();
    let (status, refused) = rest(&w.app, "POST", &format!("/api/v1/clients/{id}/decide"), &w.owner, Some(json!({ "decision": "approve", "rationale": "Mine" }))).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{refused}");
    let (status, _) = rest(&w.app, "POST", &format!("/api/v1/clients/{id}/decide"), &w.second, Some(json!({ "decision": "approve", "rationale": "Reviewed" }))).await;
    assert_eq!(status, StatusCode::OK);

    // A grant to it waits for a different person, and the proposer cannot confirm it.
    let grant_body = json!({ "holder_kind": "person", "holder_id": format!("client:{id}"), "role": "viewer", "anchor_kind": "container", "anchor_id": format!("app:{}", w.slug), "source": "client", "reason": "Reads proposals" });
    let (status, pending) = rest(&w.app, "POST", "/api/v1/admin/access/grants", &w.admin, Some(grant_body)).await;
    assert_eq!(status, StatusCode::ACCEPTED, "{pending}");
    let change = pending["change_id"].as_str().unwrap();
    let (status, refused) = rest(&w.app, "POST", &format!("/api/v1/admin/access/changes/{change}/decide"), &w.admin, Some(json!({ "decision": "confirm", "reason": "Mine" }))).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{refused}");

    // A grant to a client that has no service identity is refused.
    let (_, plain) = rest(&w.app, "POST", "/api/v1/clients", &w.owner, Some(json!({ "name": "Browser", "operator": "Physics", "kind": "public", "redirect_uris": [CALLBACK], "ceiling": { "apps": [] } }))).await;
    let plain_id = plain["client_id"].as_str().unwrap_or("scc_none");
    let body = json!({ "holder_kind": "person", "holder_id": format!("client:{plain_id}"), "role": "viewer", "anchor_kind": "container", "anchor_id": format!("app:{}", w.slug), "source": "client", "reason": "No" });
    let (status, refused) = rest(&w.app, "POST", "/api/v1/admin/access/grants", &w.admin, Some(body)).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{refused}");
}

// 6. The server says it supports `client_credentials` only when a client has a service identity.
#[tokio::test]
async fn the_metadata_names_client_credentials_only_when_a_client_has_one() {
    let w = world().await;
    let meta = |app: Router| async move { send(&app, "GET", "/.well-known/oauth-authorization-server", None, None, Body::empty()).await.1 };
    let before = meta(w.app.clone()).await;
    assert_eq!(before["grant_types_supported"], json!(["authorization_code", "refresh_token"]), "{before}");
    service_client(&w, false).await;
    let after = meta(w.app.clone()).await;
    assert_eq!(after["grant_types_supported"], json!(["authorization_code", "refresh_token", "client_credentials"]), "{after}");
}
