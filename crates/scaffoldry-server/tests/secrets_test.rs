//! Integrations, phase 1: an encrypted secrets store.
//!
//! A secret is AES-256-GCM ciphertext under a master key the operator keeps in a file, with a fresh nonce per write and
//! the secret's id as associated data. The API writes a secret and the server uses it. Nothing ever reads one back: not
//! a response, a ledger entry, an error or a debug print. NIST SP 800-53 SC-12, SC-28, IA-5(7); NIST SP 800-38D.

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::Router;
use base64::Engine;
use http_body_util::BodyExt;
use scaffoldry_core::ledger::DecisionType;
use scaffoldry_server::build_app_with_state;
use scaffoldry_server::secrets::{self, MasterKey};
use scaffoldry_server::service::identity::issue_test_token_and_user;
use scaffoldry_server::state::ServerState;
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::Arc;
use tower::ServiceExt;

const ADMIN: &str = "jordan.lee@state.edu";
const OWNER: &str = "einstein@physics.state.edu";
/// A value no other part of the system could produce, to look for anywhere it must not be.
const VALUE: &str = "tok-7Hq2-UNMISTAKABLE-SECRET-VALUE-91xZ";

/// A key file with 32 random bytes in base64, in its own directory.
fn key_file() -> PathBuf {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(uuid::Uuid::new_v4().as_bytes());
    bytes.extend_from_slice(uuid::Uuid::new_v4().as_bytes());
    let dir = std::env::temp_dir().join(format!("scaffoldry-key-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("master.key");
    std::fs::write(&path, base64::engine::general_purpose::STANDARD.encode(bytes)).unwrap();
    path
}

struct World {
    db: common::TestDb,
    state: Arc<ServerState>,
    app: Router,
    admin: String,
    key: Option<PathBuf>,
}

fn world_with(key: Option<PathBuf>) -> World {
    let db = common::TestDb::new();
    let mut state = ServerState::connect(db.url()).expect("server");
    state.master_key_file = key.clone();
    let state = Arc::new(state);
    state.seed_demo().expect("seed");
    let app = build_app_with_state(state.clone()).expect("router");
    World { db, state, app, admin: issue_test_token_and_user(ADMIN), key }
}

fn world() -> World {
    world_with(Some(key_file()))
}

async fn rest(app: &Router, method: &str, uri: &str, token: &str, body: Option<Value>) -> (StatusCode, Value, String) {
    let b = Request::builder().method(method).uri(uri).header("authorization", format!("Bearer {token}"));
    let req = match body {
        Some(v) => b.header("content-type", "application/json").body(Body::from(v.to_string())).unwrap(),
        None => b.body(Body::empty()).unwrap(),
    };
    let resp = app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let text = String::from_utf8_lossy(&bytes).to_string();
    (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null), text)
}

async fn put(w: &World, name: &str, value: &str) -> (StatusCode, Value, String) {
    rest(&w.app, "PUT", &format!("/api/v1/admin/secrets/{name}"), &w.admin, Some(json!({ "value": value, "reason": "The registrar's workflow tool" }))).await
}

// 6. A secret round-trips.
#[tokio::test]
async fn a_secret_written_through_the_api_is_usable_by_the_server_and_never_read_back() {
    let w = world();
    let (s, b, _) = put(&w, "registrar_hook", VALUE).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    let state = w.state.clone();
    let secret = tokio::task::spawn_blocking(move || secrets::get(&state, "registrar_hook")).await.unwrap().expect("the server can use it");
    assert_eq!(secret.expose(), VALUE.as_bytes());

    // The list holds names and dates. A value is never in it.
    let (s, list, text) = rest(&w.app, "GET", "/api/v1/admin/secrets", &w.admin, None).await;
    assert_eq!(s, StatusCode::OK);
    let row = &list["secrets"][0];
    assert_eq!(row["name"], "registrar_hook");
    assert_eq!(row["references"], 0);
    assert!(row["created_at"].is_string() && row["created_by"].is_string());
    assert!(!text.contains(VALUE));

    // Replacing keeps the name, changes the value, and records when.
    let (s, _, _) = put(&w, "registrar_hook", "second-value").await;
    assert_eq!(s, StatusCode::OK);
    let state = w.state.clone();
    let again = tokio::task::spawn_blocking(move || secrets::get(&state, "registrar_hook")).await.unwrap().unwrap();
    assert_eq!(again.expose(), b"second-value");
    let (_, list, _) = rest(&w.app, "GET", "/api/v1/admin/secrets", &w.admin, None).await;
    assert!(list["secrets"][0]["rotated_at"].is_string());
}

#[test]
fn sealing_uses_a_fresh_nonce_and_the_id_as_associated_data() {
    let key = MasterKey::from_file(&key_file()).unwrap();
    let id = uuid::Uuid::new_v4();
    let (ct1, n1) = secrets::seal(&key, id, VALUE.as_bytes());
    let (ct2, n2) = secrets::seal(&key, id, VALUE.as_bytes());
    assert_eq!(n1.len(), 12, "a 96-bit nonce");
    assert_ne!(n1, n2, "a fresh nonce per write");
    assert_ne!(ct1, ct2);
    assert_eq!(secrets::open(&key, id, &ct1, &n1).unwrap().expose(), VALUE.as_bytes());
    // Another row's id, or another key, does not open it.
    assert!(secrets::open(&key, uuid::Uuid::new_v4(), &ct1, &n1).is_err());
    let other = MasterKey::from_file(&key_file()).unwrap();
    assert!(secrets::open(&other, id, &ct1, &n1).is_err());
}

// 6. A ciphertext copied to another row fails to decrypt.
#[tokio::test]
async fn a_ciphertext_copied_to_another_row_does_not_decrypt() {
    let w = world();
    assert_eq!(put(&w, "first_secret", VALUE).await.0, StatusCode::OK);
    assert_eq!(put(&w, "second_secret", "something-else").await.0, StatusCode::OK);
    w.db.run(|c| {
        c.execute(
            "UPDATE secrets SET ciphertext = f.ciphertext, nonce = f.nonce FROM secrets f WHERE secrets.name = 'second_secret' AND f.name = 'first_secret'",
            &[],
        )
        .unwrap();
    });
    let state = w.state.clone();
    let err = tokio::task::spawn_blocking(move || secrets::get(&state, "second_secret").map(|_| ())).await.unwrap().unwrap_err();
    assert!(!err.to_string().contains(VALUE));
}

// 6. A wrong key fails boot when secrets exist. A missing key does not stop a server that holds none.
#[tokio::test]
async fn a_missing_or_wrong_key_stops_boot_only_when_secrets_exist() {
    let w = world();
    let repo = w.state.repository.clone();
    let key = w.key.clone().unwrap();
    let (r, k) = (repo.clone(), key.clone());
    assert!(tokio::task::spawn_blocking(move || secrets::boot_check(&r, Some(&k))).await.unwrap().is_ok(), "no secrets yet");
    let r = repo.clone();
    assert!(tokio::task::spawn_blocking(move || secrets::boot_check(&r, None)).await.unwrap().is_ok(), "no secrets, no key: the server starts");

    assert_eq!(put(&w, "registrar_hook", VALUE).await.0, StatusCode::OK);
    let (r, k) = (repo.clone(), key.clone());
    assert!(tokio::task::spawn_blocking(move || secrets::boot_check(&r, Some(&k))).await.unwrap().is_ok());
    let r = repo.clone();
    let missing = tokio::task::spawn_blocking(move || secrets::boot_check(&r, None)).await.unwrap().unwrap_err();
    assert!(missing.contains("SCAFFOLDRY_MASTER_KEY_FILE"), "{missing}");
    let (r, wrong) = (repo.clone(), key_file());
    let wrong = tokio::task::spawn_blocking(move || secrets::boot_check(&r, Some(&wrong))).await.unwrap().unwrap_err();
    assert!(!wrong.contains(VALUE), "{wrong}");
    let (r, gone) = (repo.clone(), PathBuf::from("/nonexistent/master.key"));
    assert!(tokio::task::spawn_blocking(move || secrets::boot_check(&r, Some(&gone))).await.unwrap().is_err());
}

#[tokio::test]
async fn without_a_key_the_secrets_routes_say_so() {
    let w = world_with(None);
    for (method, uri, body) in [
        ("GET", "/api/v1/admin/secrets", None),
        ("PUT", "/api/v1/admin/secrets/registrar_hook", Some(json!({ "value": VALUE, "reason": "Set" }))),
    ] {
        let (s, b, text) = rest(&w.app, method, uri, &w.admin, body).await;
        assert_eq!(s, StatusCode::SERVICE_UNAVAILABLE, "{method} {uri}: {b}");
        assert_eq!(b["code"], "master_key_missing");
        assert!(!text.contains(VALUE));
    }
}

#[tokio::test]
async fn rotating_the_master_key_reencrypts_every_secret() {
    let w = world();
    assert_eq!(put(&w, "first_secret", VALUE).await.0, StatusCode::OK);
    assert_eq!(put(&w, "second_secret", "two").await.0, StatusCode::OK);
    let (repo, old, new) = (w.state.repository.clone(), w.key.clone().unwrap(), key_file());
    let (o, n) = (old.clone(), new.clone());
    let count = tokio::task::spawn_blocking(move || secrets::rotate_master_key(&repo, &o, &n)).await.unwrap().expect("rotated");
    assert_eq!(count, 2);
    let repo = w.state.repository.clone();
    let (n, o) = (new.clone(), old.clone());
    assert!(tokio::task::spawn_blocking(move || secrets::boot_check(&repo, Some(&n))).await.unwrap().is_ok());
    let repo = w.state.repository.clone();
    assert!(tokio::task::spawn_blocking(move || secrets::boot_check(&repo, Some(&o))).await.unwrap().is_err(), "the old key no longer opens them");
    // A rotation from a key that does not open them changes nothing.
    let (repo, wrong, n) = (w.state.repository.clone(), key_file(), new.clone());
    assert!(tokio::task::spawn_blocking(move || secrets::rotate_master_key(&repo, &wrong, &n)).await.unwrap().is_err());
}

// 7. No response, ledger entry, error message or debug print contains a secret value.
#[tokio::test]
async fn a_secret_value_appears_in_no_response_ledger_entry_or_error() {
    let w = world();
    let mut seen = Vec::new();
    let (_, _, text) = put(&w, "registrar_hook", VALUE).await;
    seen.push(text);
    let (_, _, text) = put(&w, "registrar_hook", VALUE).await;
    seen.push(text);
    // Refusals: a bad name, no reason, no value, someone who is not a Platform Admin.
    let (s, _, text) = rest(&w.app, "PUT", "/api/v1/admin/secrets/Bad-Name", &w.admin, Some(json!({ "value": VALUE, "reason": "x" }))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    seen.push(text);
    let (s, _, text) = rest(&w.app, "PUT", "/api/v1/admin/secrets/no_reason", &w.admin, Some(json!({ "value": VALUE }))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    seen.push(text);
    let owner = issue_test_token_and_user(OWNER);
    let (s, _, text) = rest(&w.app, "PUT", "/api/v1/admin/secrets/registrar_hook", &owner, Some(json!({ "value": VALUE, "reason": "x" }))).await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    seen.push(text);
    let (s, _, text) = rest(&w.app, "GET", "/api/v1/admin/secrets", &owner, None).await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    seen.push(text);
    let (_, _, text) = rest(&w.app, "GET", "/api/v1/admin/secrets", &w.admin, None).await;
    seen.push(text);
    let (_, _, text) = rest(&w.app, "DELETE", "/api/v1/admin/secrets/registrar_hook", &w.admin, Some(json!({ "reason": "Retired" }))).await;
    seen.push(text);
    for t in &seen {
        assert!(!t.contains(VALUE), "a response held the value");
    }
    // The ledger names the secret and holds no value.
    let rows: Vec<String> = w.db.run(|c| {
        c.query("SELECT row_to_json(l)::text FROM governance_ledger l WHERE decision_type = 'SecretChanged'", &[]).unwrap().iter().map(|r| r.get(0)).collect()
    });
    assert_eq!(rows.len(), 3, "two writes and a delete");
    assert!(rows.iter().all(|r| r.contains("registrar_hook") && !r.contains(VALUE)));
    let all: String = w.db.run(|c| c.query_one("SELECT coalesce(string_agg(row_to_json(l)::text, ''), '') FROM governance_ledger l", &[]).unwrap().get(0));
    assert!(!all.contains(VALUE));
    // A debug print of a secret, and of a failure, shows no value.
    let key = MasterKey::from_file(w.key.as_ref().unwrap()).unwrap();
    let id = uuid::Uuid::new_v4();
    let (ct, n) = secrets::seal(&key, id, VALUE.as_bytes());
    let secret = secrets::open(&key, id, &ct, &n).unwrap();
    assert!(!format!("{secret:?}").contains(VALUE));
    assert!(!format!("{key:?}").contains(&std::fs::read_to_string(w.key.as_ref().unwrap()).unwrap()));
    let err = secrets::open(&key, uuid::Uuid::new_v4(), &ct, &n).unwrap_err();
    assert!(!format!("{err:?} {err}").contains(VALUE));
    assert!(DecisionType::ALL.contains(&DecisionType::SecretChanged));
}

// 9. Deleting a secret that something references is 409 and names the references. A column named `secret` on any
// table names a secret: that is how the later briefs (webhooks, connections) refer to one.
#[tokio::test]
async fn a_secret_in_use_cannot_be_deleted_and_the_refusal_names_what_uses_it() {
    let w = world();
    assert_eq!(put(&w, "sis_password", VALUE).await.0, StatusCode::OK);
    let user = uuid::Uuid::new_v4();
    w.db.run(move |c| {
        c.batch_execute("CREATE TABLE nightly_feeds (id UUID PRIMARY KEY, secret VARCHAR(64))").unwrap();
        c.execute("INSERT INTO nightly_feeds (id, secret) VALUES ($1, 'sis_password')", &[&user]).unwrap();
    });
    let (_, list, _) = rest(&w.app, "GET", "/api/v1/admin/secrets", &w.admin, None).await;
    assert_eq!(list["secrets"][0]["references"], 1);
    let (s, b, _) = rest(&w.app, "DELETE", "/api/v1/admin/secrets/sis_password", &w.admin, Some(json!({ "reason": "Retired" }))).await;
    assert_eq!(s, StatusCode::CONFLICT, "{b}");
    let message = b["error"].as_str().unwrap_or_default();
    assert!(message.contains("nightly_feeds") && message.contains(&user.to_string()), "{message}");

    // Once nothing uses it, it can go.
    w.db.run(|c| {
        c.batch_execute("DELETE FROM nightly_feeds").unwrap();
    });
    let (s, _, _) = rest(&w.app, "DELETE", "/api/v1/admin/secrets/sis_password", &w.admin, Some(json!({ "reason": "Retired" }))).await;
    assert_eq!(s, StatusCode::OK);
    let (_, list, _) = rest(&w.app, "GET", "/api/v1/admin/secrets", &w.admin, None).await;
    assert_eq!(list["secrets"].as_array().unwrap().len(), 0);
    let (s, _, _) = rest(&w.app, "DELETE", "/api/v1/admin/secrets/sis_password", &w.admin, Some(json!({ "reason": "Again" }))).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn the_outbound_allowlist_is_a_checked_setting() {
    let w = world();
    let (s, b, _) = rest(&w.app, "PUT", "/api/v1/settings/egress.allow", &w.admin, Some(json!([{ "host": "hooks.example.edu", "ports": [443] }]))).await;
    assert!(s.is_success(), "{b}");
    let (s, _, _) = rest(&w.app, "PUT", "/api/v1/settings/egress.allow", &w.admin, Some(json!([{ "cidr": "10.0.0.0/99", "ports": [443] }]))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    let (s, _, _) = rest(&w.app, "PUT", "/api/v1/settings/egress.allow", &w.admin, Some(json!({ "host": "not-a-list" }))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
}
