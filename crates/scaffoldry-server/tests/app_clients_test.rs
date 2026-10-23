//! External apps phase 1: registered clients and their ceilings.
//!
//! An outside app is a registered client. A person proposes it with a ceiling: the apps, tables, fields and operations
//! it may ever use. An owner or admin decides. A ceiling never names a sensitive field. The ceiling is shown as
//! sentences. NIST SP 800-53 AC-6, CM-3 and AC-2.

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
use tower::ServiceExt;

const WORKSPACE: &str = "ws-physics-optics";
const OWNER: &str = "einstein@physics.state.edu";
const PLATFORM_ADMIN: &str = "jordan.lee@state.edu";

struct World {
    db: common::TestDb,
    app: Router,
    owner: String,
    second: String,
    editor: String,
    outsider: String,
    platform: String,
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

async fn tool(w: &World, token: &str, name: &str, args: Value) -> Value {
    let payload = json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": { "name": name, "arguments": args } });
    let (_, v) = rest(&w.app, "POST", "/api/mcp", token, Some(payload)).await;
    assert!(v["result"]["isError"] != true, "{name}: {v}");
    v["result"]["structuredContent"].clone()
}

fn field(name: &str, sensitive: bool) -> Value {
    json!({ "name": name, "label": name, "field_type": "Text", "required": false, "ferpa_sensitive": sensitive })
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
    let (second, editor) = (format!("second-{tag}@state.edu"), format!("editor-{tag}@state.edu"));
    member(&state, &second, "admin");
    member(&state, &editor, "editor");
    let app = build_app_with_state(state).expect("router");
    let w = World {
        db,
        app,
        owner: issue_test_token_and_user(OWNER),
        second: issue_test_token_and_user(&second),
        editor: issue_test_token_and_user(&editor),
        outsider: issue_test_token_and_user(&format!("outsider-{tag}@state.edu")),
        platform: issue_test_token_and_user(PLATFORM_ADMIN),
        slug: format!("cl-{tag}"),
    };
    let manifest = json!({
        "slug": w.slug, "title": "Grants Office", "description": "d", "organization_code": "PHYS", "department": "physics",
        "workspace_id": WORKSPACE, "views": [], "pages": [],
        "tables": [{ "id": "proposals", "name": "Proposals", "slug": "proposals", "fields": [field("title", false), field("status", false), field("ssn", true)] }]
    });
    let p = tool(&w, &w.owner, "propose_app_change", json!({ "workspace_id": WORKSPACE, "manifest": manifest, "summary": "An app" })).await;
    // An app with a sensitive field needs a second person to approve it.
    tool(&w, &w.second, "decide_proposal", json!({ "proposal_id": p["proposal_id"], "decision": "approve", "rationale": "Set up", "digest": p["digest"] })).await;
    w
}

fn ceiling(w: &World, read: &[&str], write: &[&str], create: bool) -> Value {
    json!({ "apps": [{ "app_slug": w.slug, "tables": [{ "table_id": "proposals", "read": read, "write": write, "create": create, "delete": false }] }] })
}

fn registration(w: &World, kind: &str, ceiling: Value) -> Value {
    json!({ "name": "Grant tracker", "operator": "Physics Department", "kind": kind, "redirect_uris": ["https://tracker.example.edu/callback"], "ceiling": ceiling })
}

async fn register(w: &World, token: &str, body: Value) -> (StatusCode, Value) {
    let _ = w;
    rest(&w.app, "POST", "/api/v1/clients", token, Some(body)).await
}

async fn decide(w: &World, token: &str, id: &str, decision: &str) -> (StatusCode, Value) {
    rest(&w.app, "POST", &format!("/api/v1/clients/{id}/decide"), token, Some(json!({ "decision": decision, "rationale": "Reviewed" }))).await
}

fn ledger_count(entries: &Value, kind: &str) -> usize {
    entries["entries"].as_array().unwrap().iter().filter(|e| e["decision_type"] == kind).count()
}

async fn ledger(w: &World) -> Value {
    rest(&w.app, "GET", "/api/v1/governance/ledger", &w.platform, None).await.1
}

// 1. A ceiling naming a sensitive field is refused.
#[tokio::test]
async fn a_ceiling_naming_a_sensitive_field_is_refused() {
    let w = world().await;
    for (read, write) in [(&["title", "ssn"][..], &[][..]), (&["title"][..], &["ssn"][..])] {
        let (status, body) = register(&w, &w.owner, registration(&w, "public", ceiling(&w, read, write, false))).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
        assert_eq!(body["code"], "client_sensitive_field", "{body}");
        assert!(body["error"].as_str().unwrap().contains("ssn"), "{body}");
    }
    // A field that does not exist is a plain bad request.
    let (status, body) = register(&w, &w.owner, registration(&w, "public", ceiling(&w, &["nope"], &[], false))).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_ne!(body["code"], "client_sensitive_field");
    let (_, listed) = rest(&w.app, "GET", "/api/v1/clients", &w.owner, None).await;
    assert!(listed.as_array().unwrap().is_empty(), "nothing was stored: {listed}");
}

// 2. A read-only ceiling is approved by its owner. A writing ceiling needs a second person.
#[tokio::test]
async fn a_writing_ceiling_needs_a_second_person() {
    let w = world().await;
    let (status, read_only) = register(&w, &w.owner, registration(&w, "public", ceiling(&w, &["title", "status"], &[], false))).await;
    assert_eq!(status, StatusCode::CREATED, "{read_only}");
    assert_eq!(read_only["status"], "Pending");
    let id = read_only["client_id"].as_str().unwrap().to_string();
    let (status, body) = decide(&w, &w.owner, &id, "approve").await;
    assert_eq!(status, StatusCode::OK, "the proposer owns the workspace: {body}");
    assert_eq!(body["status"], "Active");

    let (status, writing) = register(&w, &w.owner, registration(&w, "public", ceiling(&w, &["title"], &["status"], true))).await;
    assert_eq!(status, StatusCode::CREATED, "{writing}");
    let id = writing["client_id"].as_str().unwrap().to_string();
    let (status, body) = decide(&w, &w.owner, &id, "approve").await;
    assert_eq!(status, StatusCode::FORBIDDEN, "the proposer cannot approve a writing ceiling: {body}");
    let (_, now) = rest(&w.app, "GET", &format!("/api/v1/clients/{id}"), &w.owner, None).await;
    assert_eq!(now["status"], "Pending", "it is not in force");
    let (status, body) = decide(&w, &w.second, &id, "approve").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["status"], "Active");
    assert_eq!(body["decided_by"].as_str().unwrap().split('@').next().unwrap().split('-').next(), Some("second"));

    // Someone who is not an owner or admin cannot decide. An editor may propose.
    let (status, mine) = register(&w, &w.editor, registration(&w, "public", ceiling(&w, &["title"], &[], false))).await;
    assert_eq!(status, StatusCode::CREATED, "an editor may propose: {mine}");
    let id = mine["client_id"].as_str().unwrap().to_string();
    assert_eq!(decide(&w, &w.editor, &id, "approve").await.0, StatusCode::FORBIDDEN, "an editor does not own the workspace");
    assert_eq!(decide(&w, &w.outsider, &id, "approve").await.0, StatusCode::NOT_FOUND, "a stranger cannot tell it exists");
    assert_eq!(decide(&w, &w.owner, &id, "approve").await.0, StatusCode::OK, "an owner other than the proposer");
    assert_eq!(register(&w, &w.outsider, registration(&w, "public", ceiling(&w, &["title"], &[], false))).await.0, StatusCode::FORBIDDEN, "an outsider may not propose");

    // A client is decided once.
    assert_eq!(decide(&w, &w.owner, &id, "reject").await.0, StatusCode::CONFLICT);
}

// 3. A redirect URI with a wildcard, a query string, or http (other than allowed localhost) is refused.
#[tokio::test]
async fn redirect_uris_are_exact_https_addresses() {
    let w = world().await;
    let bad = [
        "https://*.example.edu/callback",
        "https://tracker.example.edu/callback?next=/home",
        "https://tracker.example.edu/callback#frag",
        "http://tracker.example.edu/callback",
        "not a url",
        "https://tracker.example.edu/*",
        "",
    ];
    for uri in bad {
        let mut body = registration(&w, "public", ceiling(&w, &["title"], &[], false));
        body["redirect_uris"] = json!([uri]);
        let (status, reply) = register(&w, &w.owner, body).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{uri:?}: {reply}");
    }
    let mut none = registration(&w, "public", ceiling(&w, &["title"], &[], false));
    none["redirect_uris"] = json!([]);
    assert_eq!(register(&w, &w.owner, none).await.0, StatusCode::BAD_REQUEST, "a client needs a redirect URI");

    let mut local = registration(&w, "public", ceiling(&w, &["title"], &[], false));
    local["redirect_uris"] = json!(["http://localhost:5173/callback"]);
    assert_eq!(register(&w, &w.owner, local.clone()).await.0, StatusCode::BAD_REQUEST, "localhost is off by default");
    let (status, _) = rest(&w.app, "PUT", "/api/v1/settings/oauth.allow_localhost", &w.platform, Some(json!(true))).await;
    assert_eq!(status, StatusCode::OK, "the setting exists");
    assert_eq!(register(&w, &w.owner, local).await.0, StatusCode::CREATED, "localhost is allowed once the setting is true");
    let mut http_other = registration(&w, "public", ceiling(&w, &["title"], &[], false));
    http_other["redirect_uris"] = json!(["http://tracker.example.edu/callback"]);
    assert_eq!(register(&w, &w.owner, http_other).await.0, StatusCode::BAD_REQUEST, "only localhost may be http");
}

// 4. The confidential client's secret is in one response and nowhere else. The table holds its hash.
#[tokio::test]
async fn the_secret_is_shown_once_and_stored_as_a_hash() {
    let w = world().await;
    let (_, created) = register(&w, &w.owner, registration(&w, "confidential", ceiling(&w, &["title"], &[], false))).await;
    let id = created["client_id"].as_str().unwrap().to_string();
    assert!(created.get("client_secret").is_none(), "no secret before approval: {created}");
    let (_, approved) = decide(&w, &w.second, &id, "approve").await;
    let secret = approved["client_secret"].as_str().expect("the secret, once").to_string();
    assert!(secret.len() >= 32);

    for uri in [format!("/api/v1/clients/{id}"), "/api/v1/clients".to_string()] {
        let (_, body) = rest(&w.app, "GET", &uri, &w.owner, None).await;
        assert!(!body.to_string().contains(&secret), "{uri} does not return the secret");
        assert!(!body.to_string().contains("secret_hash"), "{uri} does not return the hash");
    }
    let ledger = ledger(&w).await.to_string();
    assert!(!ledger.contains(&secret), "the ledger does not hold it");

    let digest: String = Sha256::digest(secret.as_bytes()).iter().map(|b| format!("{b:02x}")).collect();
    let (url, wanted) = (w.db.url().to_string(), id.clone());
    let (hash, whole): (Option<String>, String) = std::thread::spawn(move || {
        let mut c = postgres::Client::connect(&url, postgres::NoTls).unwrap();
        let row = c.query_one("SELECT secret_hash, row_to_json(app_clients)::text FROM app_clients WHERE client_id = $1", &[&wanted]).unwrap();
        (row.get(0), row.get(1))
    })
    .join()
    .unwrap();
    assert_eq!(hash.as_deref().map(str::trim), Some(digest.as_str()), "the table holds the SHA-256 of the secret");
    assert!(!whole.contains(&secret), "the secret is in no column");

    // A public client has none.
    let (_, public) = register(&w, &w.owner, registration(&w, "public", ceiling(&w, &["title"], &[], false))).await;
    let (_, approved) = decide(&w, &w.owner, public["client_id"].as_str().unwrap(), "approve").await;
    assert!(approved.get("client_secret").is_none(), "{approved}");
}

// 5. Every registration, decision and disablement writes one ledger entry, and a refusal writes none.
#[tokio::test]
async fn every_registration_decision_and_disablement_is_one_ledger_entry() {
    let w = world().await;
    let before = ledger(&w).await;
    let (_, created) = register(&w, &w.owner, registration(&w, "public", ceiling(&w, &["title"], &["status"], false))).await;
    let id = created["client_id"].as_str().unwrap().to_string();
    let after_register = ledger(&w).await;
    assert_eq!(ledger_count(&after_register, "ClientProposed"), ledger_count(&before, "ClientProposed") + 1);

    // Refused decisions write nothing.
    assert_eq!(decide(&w, &w.owner, &id, "approve").await.0, StatusCode::FORBIDDEN);
    assert_eq!(decide(&w, &w.outsider, &id, "approve").await.0, StatusCode::NOT_FOUND);
    assert_eq!(ledger_count(&ledger(&w).await, "ClientDecided"), ledger_count(&before, "ClientDecided"));

    assert_eq!(decide(&w, &w.second, &id, "approve").await.0, StatusCode::OK);
    let after_decide = ledger(&w).await;
    assert_eq!(ledger_count(&after_decide, "ClientDecided"), ledger_count(&before, "ClientDecided") + 1);

    // An outsider cannot disable it. An owner can.
    let disable = |token: String| {
        let (app, uri) = (w.app.clone(), format!("/api/v1/clients/{id}/disable"));
        async move { rest(&app, "POST", &uri, &token, Some(json!({ "reason": "No longer used" }))).await }
    };
    assert_eq!(disable(w.outsider.clone()).await.0, StatusCode::NOT_FOUND);
    assert_eq!(ledger_count(&ledger(&w).await, "ClientDisabled"), ledger_count(&before, "ClientDisabled"));
    let (status, body) = disable(w.owner.clone()).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["status"], "Disabled");
    assert_eq!(ledger_count(&ledger(&w).await, "ClientDisabled"), ledger_count(&before, "ClientDisabled") + 1);
    // Disabling again is a conflict and writes nothing.
    assert_eq!(disable(w.owner.clone()).await.0, StatusCode::CONFLICT);
    assert_eq!(ledger_count(&ledger(&w).await, "ClientDisabled"), ledger_count(&before, "ClientDisabled") + 1);
}

// The ceiling is shown as sentences, by one function. The proposer and the owners see it. A stranger does not.
#[tokio::test]
async fn the_ceiling_is_shown_as_sentences_to_the_people_who_may_see_it() {
    let w = world().await;
    let (_, created) = register(&w, &w.owner, registration(&w, "public", ceiling(&w, &["title", "status"], &["status"], true))).await;
    let id = created["client_id"].as_str().unwrap().to_string();
    let sentences: Vec<&str> = created["sentences"].as_array().unwrap().iter().filter_map(Value::as_str).collect();
    assert_eq!(
        sentences,
        [
            "Read the title and status of Proposals in Grants Office.",
            "Change the status of Proposals in Grants Office.",
            "Create Proposals in Grants Office."
        ],
        "{created}"
    );
    let (status, one) = rest(&w.app, "GET", &format!("/api/v1/clients/{id}"), &w.second, None).await;
    assert_eq!(status, StatusCode::OK, "an admin of the workspace sees it: {one}");
    assert_eq!(one["sentences"], created["sentences"]);
    assert_eq!(rest(&w.app, "GET", &format!("/api/v1/clients/{id}"), &w.platform, None).await.0, StatusCode::OK);
    assert_eq!(rest(&w.app, "GET", &format!("/api/v1/clients/{id}"), &w.outsider, None).await.0, StatusCode::NOT_FOUND, "a stranger cannot tell it exists");
    let (_, theirs) = rest(&w.app, "GET", "/api/v1/clients", &w.outsider, None).await;
    assert!(theirs.as_array().unwrap().is_empty());
}
