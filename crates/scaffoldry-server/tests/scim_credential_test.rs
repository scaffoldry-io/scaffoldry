//! A server ships with no provisioning credential. An operator mints one, and revoking it holds across a restart.

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::Router;
use http_body_util::BodyExt;
use scaffoldry_server::service::identity::issue_test_token_and_user;
use serde_json::{json, Value};
use tower::ServiceExt;

const ADMIN: &str = "jordan.lee@state.edu";

async fn call(app: &Router, method: &str, uri: &str, token: &str, body: Option<Value>) -> (StatusCode, Value) {
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

// 1. A new installation holds no provisioning credential, and the string the tests once used opens nothing.
#[tokio::test]
async fn a_new_installation_has_no_provisioning_credential() {
    let db = common::TestDb::bare();
    let app = db.app();
    let count: i64 = db.run(|c| c.query_one("SELECT COUNT(*) FROM api_tokens WHERE kind = 'scim'", &[]).unwrap().get(0));
    assert_eq!(count, 0, "a server must not create a provisioning credential of its own");
    let (status, _) = call(&app, "GET", "/scim/v2/Users", "test-scim-token", None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

// 2. A Platform Admin mints one. It works until it is revoked, and a restart does not bring it back.
#[tokio::test]
async fn a_minted_credential_works_until_revoked_and_stays_revoked_after_a_restart() {
    let db = common::TestDb::bare();
    let app = db.app();
    let admin = issue_test_token_and_user(ADMIN);
    let (status, made) = call(&app, "POST", "/api/v1/auth/tokens", &admin, Some(json!({ "label": "Campus IdP", "kind": "scim", "days": 30 }))).await;
    assert_eq!(status, StatusCode::CREATED, "{made}");
    let (id, token) = (made["id"].as_str().unwrap().to_string(), made["token"].as_str().unwrap().to_string());
    assert_eq!(call(&app, "GET", "/scim/v2/Users", &token, None).await.0, StatusCode::OK);

    let (status, body) = call(&app, "DELETE", &format!("/api/v1/auth/tokens/{id}"), &admin, None).await;
    assert!(status.is_success(), "{status}: {body}");
    assert_eq!(call(&app, "GET", "/scim/v2/Users", &token, None).await.0, StatusCode::UNAUTHORIZED);

    let restarted = db.app();
    assert_eq!(call(&restarted, "GET", "/scim/v2/Users", &token, None).await.0, StatusCode::UNAUTHORIZED, "a restart must not undo a revocation");
}

// 3. Nobody but a Platform Admin mints one.
#[tokio::test]
async fn only_a_platform_admin_mints_a_provisioning_credential() {
    let db = common::TestDb::bare();
    let app = db.app();
    let faculty = issue_test_token_and_user("prof.curie@science.state.edu");
    let (status, body) = call(&app, "POST", "/api/v1/auth/tokens", &faculty, Some(json!({ "label": "Mine", "kind": "scim" }))).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
}

// 4. No source file carries a credential.
#[test]
fn no_source_file_carries_a_credential() {
    fn walk(dir: &std::path::Path, hits: &mut Vec<String>) {
        for entry in std::fs::read_dir(dir).unwrap().flatten() {
            let path = entry.path();
            if path.is_dir() {
                walk(&path, hits);
            } else if path.extension().is_some_and(|e| e == "rs") {
                let text = std::fs::read_to_string(&path).unwrap_or_default();
                if text.contains("test-scim-token") {
                    hits.push(path.display().to_string());
                }
            }
        }
    }
    let crates = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    let mut hits = Vec::new();
    for entry in std::fs::read_dir(&crates).unwrap().flatten() {
        let src = entry.path().join("src");
        if src.is_dir() {
            walk(&src, &mut hits);
        }
    }
    assert!(hits.is_empty(), "a credential is written into: {hits:?}");
}

// 5. An installation that already holds the old credential loses it when it upgrades. The row is kept, revoked.
#[tokio::test]
async fn an_upgrade_revokes_the_credential_older_servers_created() {
    let db = common::TestDb::bare();
    let app = db.app();
    let legacy = scaffoldry_server::service::identity::hash_token("test-scim-token");
    let h = legacy.clone();
    db.run(move |c| {
        c.execute(
            "INSERT INTO api_tokens (token_hash, id, kind, eppn, label, created_at, expires_at) \
             VALUES ($1, gen_random_uuid(), 'scim', 'jordan.lee@state.edu', 'SCIM Provisioning Credential', NOW(), NOW() + INTERVAL '365 days')",
            &[&h],
        )
        .unwrap();
    });
    assert_eq!(call(&app, "GET", "/scim/v2/Users", "test-scim-token", None).await.0, StatusCode::OK, "an older installation still opens to the string");

    let file = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../scaffoldry-core/migrations/0075_revoke_default_scim_credential.sql");
    let sql = std::fs::read_to_string(&file).unwrap_or_else(|e| panic!("{}: {e}", file.display()));
    db.run(move |c| c.batch_execute(&sql).unwrap());

    assert_eq!(call(&app, "GET", "/scim/v2/Users", "test-scim-token", None).await.0, StatusCode::UNAUTHORIZED);
    let row: (bool,) = db.run(move |c| {
        let r = c.query_one("SELECT revoked_at IS NOT NULL FROM api_tokens WHERE token_hash = $1", &[&legacy]).unwrap();
        (r.get(0),)
    });
    assert!(row.0, "the row stays as a record, revoked");
    // Running it again changes nothing.
    let sql = std::fs::read_to_string(&file).unwrap();
    db.run(move |c| c.batch_execute(&sql).unwrap());
}
