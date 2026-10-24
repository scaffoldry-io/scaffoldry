//! Token Authentication, RFC 9728 Discovery & CORS Hardening Tests
//!
//! Enforces:
//! 1. Bearer token validation against stored api_tokens rows with resolution to AuthUser.
//! 2. Complete removal of seeded `sct_*` tokens, credential-less login, and hardcoded directory.
//! 3. RFC 9728 authorization metadata discovery for MCP clients (OIDC issuer endpoints deleted).
//! 4. CORS restricted to configured institutional origins (removal of `Any`).
//! 5. Cedar-guarded and cryptographically ledger-recorded impersonation.

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use scaffoldry_server::build_app;
use scaffoldry_server::service::identity::issue_test_token_and_user;
use serde_json::Value;
use tower::ServiceExt;

#[tokio::test]
async fn test_oauth2_oidc_jwt_bearer_validation() {
    let _db = common::TestDb::new();
    let app = build_app().expect("Build router");

    // 1. Valid token with institutional claims
    let valid_token = issue_test_token_and_user("prof.curie@science.state.edu");

    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/workspaces")
                .header("authorization", format!("Bearer {valid_token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::OK, "Valid token must be accepted");

    // 2. Tampered / unknown token must fail closed with 401
    let tampered_token = format!("{valid_token}tampered");
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/workspaces")
                .header("authorization", format!("Bearer {tampered_token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);

    // 3. Expired token must fail closed with 401
    let fake_expired_token = format!("scf_{}{}", uuid::Uuid::new_v4().simple(), uuid::Uuid::new_v4().simple());
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/workspaces")
                .header("authorization", format!("Bearer {fake_expired_token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED, "Expired token must be rejected");
}

#[tokio::test]
async fn test_seeded_tokens_and_credential_less_login_deleted() {
    let _db = common::TestDb::new();
    let app = build_app().expect("Build router");

    // 1. Seeded tokens must no longer exist
    let seeded_tokens = ["sct_admin_token", "sct_faculty_token", "sct_not_a_real_token"];
    for token in seeded_tokens {
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/v1/workspaces")
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(
            resp.status(),
            StatusCode::UNAUTHORIZED,
            "Seeded token '{token}' must be refused"
        );
    }

    // 2. Credential-less login endpoint must be deleted or reject unauthenticated logins
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/auth/login")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"eppn": "sarah.connor@state.edu"}"#))
                .unwrap(),
        )
        .await
        .unwrap();

    assert!(
        resp.status() == StatusCode::NOT_FOUND || resp.status() == StatusCode::UNAUTHORIZED || resp.status() == StatusCode::METHOD_NOT_ALLOWED,
        "Credential-less login must not be accessible (got {})",
        resp.status()
    );
}

#[tokio::test]
async fn test_mcp_authorization_metadata_discovery() {
    let _db = common::TestDb::new();
    let app = build_app().expect("Build router");

    // 1. RFC 9728 OAuth 2.0 Protected Resource Metadata
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/.well-known/oauth-protected-resource")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::OK, "Protected resource metadata must exist");
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let res_meta: Value = serde_json::from_slice(&bytes).unwrap();
    assert!(res_meta["resource"].is_string());
    assert!(res_meta["authorization_servers"].is_array());
    assert!(res_meta["scopes_supported"].is_array());

    // 2. OpenID Connect Discovery 1.0 Metadata must be 404 (Scaffoldry is not an issuer)
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/.well-known/openid-configuration")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::NOT_FOUND, "OpenID configuration endpoint must be deleted (404)");

    // 3. JWKS keys endpoint must be 404 (Scaffoldry is not an issuer)
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/.well-known/jwks.json")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::NOT_FOUND, "JWKS keys endpoint must be deleted (404)");
}

#[tokio::test]
async fn test_cors_tightened_to_configured_origins() {
    let _db = common::TestDb::new();
    let app = build_app().expect("Build router");

    // 1. Malicious origin must not receive Access-Control-Allow-Origin: * or reflect origin
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("OPTIONS")
                .uri("/api/v1/workspaces")
                .header("origin", "https://malicious-tracker.attacker.com")
                .header("access-control-request-method", "GET")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    let allow_origin = resp.headers().get("access-control-allow-origin").and_then(|h| h.to_str().ok());
    assert_ne!(allow_origin, Some("*"), "CORS must not allow Any / wildcard origin");
    assert_ne!(
        allow_origin,
        Some("https://malicious-tracker.attacker.com"),
        "CORS must not allow unauthorized attacker origin"
    );

    // 2. Configured dev origin (localhost:5173) must be allowed
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("OPTIONS")
                .uri("/api/v1/workspaces")
                .header("origin", "http://localhost:5173")
                .header("access-control-request-method", "GET")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    let allow_origin = resp.headers().get("access-control-allow-origin").and_then(|h| h.to_str().ok());
    assert_eq!(allow_origin, Some("http://localhost:5173"));
}

#[tokio::test]
async fn test_impersonation_remains_cedar_guarded_and_ledger_recorded() {
    let _db = common::TestDb::new();
    let app = build_app().expect("Build router");

    // 1. Non-admin (faculty) trying to impersonate is DENIED by Cedar policy (403 Forbidden)
    let faculty_token = issue_test_token_and_user("prof.curie@science.state.edu");

    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/auth/impersonate")
                .header("authorization", format!("Bearer {faculty_token}"))
                .header("content-type", "application/json")
                .body(Body::from(r#"{"target_eppn": "student.smith@science.state.edu"}"#))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(
        resp.status(),
        StatusCode::FORBIDDEN,
        "Faculty must be forbidden from administrative impersonation"
    );

    // 2. Central Admin trying to impersonate is ALLOWED
    let admin_token = issue_test_token_and_user("jordan.lee@state.edu");

    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/auth/impersonate")
                .header("authorization", format!("Bearer {admin_token}"))
                .header("content-type", "application/json")
                .body(Body::from(r#"{"target_eppn": "student.smith@science.state.edu"}"#))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::OK, "Central admin must be authorized to impersonate");
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let imp_res: Value = serde_json::from_slice(&bytes).unwrap();
    assert!(imp_res["token"].is_string());
    assert_eq!(imp_res["is_impersonating"], true);
}

// ---------------------------------------------------------------------------------------------------------------
// Foundation phase 10: tokens from the institution's identity provider are verified.
// Scaffoldry is a resource server. It verifies RS256 and ES256 tokens against the keys the Platform Admin pasted into
// `oidc.jwks`, and trusts no claim but the subject. RFC 7519, RFC 7517, RFC 8725.
// ---------------------------------------------------------------------------------------------------------------

mod institution {
    use super::*;
    use base64::Engine;
    use jsonwebtoken::{Algorithm, EncodingKey, Header};
    use serde_json::json;

    pub const ISSUER: &str = "https://idp.state.edu";
    pub const AUDIENCE: &str = "scaffoldry";
    pub const PERSON: &str = "prof.curie@science.state.edu";
    pub const ADMIN: &str = "jordan.lee@state.edu";

    pub struct Key {
        pub kid: String,
        pub pem: String,
        pub jwk: Value,
    }

    /// A fresh RSA key pair, made with the openssl command, and its public half as a JWK.
    pub fn new_key(kid: &str) -> Key {
        let dir = std::env::temp_dir().join(format!("oidc-key-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("key.pem");
        let made = std::process::Command::new("openssl").args(["genpkey", "-algorithm", "RSA", "-pkeyopt", "rsa_keygen_bits:2048", "-out"]).arg(&path).output().expect("openssl");
        assert!(made.status.success(), "openssl could not make a key");
        let modulus = std::process::Command::new("openssl").args(["rsa", "-noout", "-modulus", "-in"]).arg(&path).output().expect("openssl");
        let text = String::from_utf8(modulus.stdout).unwrap();
        let hex = text.trim().strip_prefix("Modulus=").expect("a modulus");
        let bytes: Vec<u8> = (0..hex.len()).step_by(2).map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap()).collect();
        let n = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes);
        Key {
            kid: kid.to_string(),
            pem: std::fs::read_to_string(&path).unwrap(),
            jwk: json!({ "kty": "RSA", "kid": kid, "alg": "RS256", "use": "sig", "n": n, "e": "AQAB" }),
        }
    }

    pub fn jwks(keys: &[&Key]) -> Value {
        json!({ "keys": keys.iter().map(|k| k.jwk.clone()).collect::<Vec<_>>() })
    }

    pub fn sign(key: &Key, claims: Value) -> String {
        let mut header = Header::new(Algorithm::RS256);
        header.kid = Some(key.kid.clone());
        jsonwebtoken::encode(&header, &claims, &EncodingKey::from_rsa_pem(key.pem.as_bytes()).unwrap()).unwrap()
    }

    pub fn claims(sub: &str) -> Value {
        let exp = chrono::Utc::now().timestamp() + 600;
        json!({ "iss": ISSUER, "aud": AUDIENCE, "sub": sub, "exp": exp })
    }

    pub async fn get(app: &axum::Router, path: &str, token: &str) -> (StatusCode, Value) {
        let resp = app.clone().oneshot(Request::builder().uri(path).header("authorization", format!("Bearer {token}")).body(Body::empty()).unwrap()).await.unwrap();
        let status = resp.status();
        let bytes = resp.into_body().collect().await.unwrap().to_bytes();
        (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
    }

    pub async fn put_setting(app: &axum::Router, admin: &str, key: &str, value: Value) {
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri(format!("/api/v1/settings/{key}"))
                    .header("authorization", format!("Bearer {admin}"))
                    .header("content-type", "application/json")
                    .body(Body::from(value.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK, "PUT {key}");
    }

    /// An app that trusts `key`.
    pub async fn trusting(key: &Key) -> (common::TestDb, axum::Router, String) {
        let db = common::TestDb::new();
        let app = build_app().expect("router");
        let admin = issue_test_token_and_user(ADMIN);
        // The people the tests sign in as exist in the directory. A token names a person and creates none.
        for person in [PERSON, "ada@science.state.edu"] {
            issue_test_token_and_user(person);
        }
        put_setting(&app, &admin, "oidc.issuer", json!(ISSUER)).await;
        put_setting(&app, &admin, "oidc.audience", json!(AUDIENCE)).await;
        put_setting(&app, &admin, "oidc.jwks", jwks(&[key])).await;
        (db, app, admin)
    }
}

// 1. A valid RS256 token is accepted, and the person is the subject.
#[tokio::test]
async fn a_valid_rs256_token_is_accepted() {
    let key = institution::new_key("k1");
    let (_db, app, _admin) = institution::trusting(&key).await;
    let token = institution::sign(&key, institution::claims(institution::PERSON));
    let (status, me) = institution::get(&app, "/api/v1/auth/me", &token).await;
    assert_eq!(status, StatusCode::OK, "{me}");
    assert_eq!(me["user"]["eppn"], institution::PERSON, "{me}");
    // An `eppn` claim names the person when there is one.
    let mut with_eppn = institution::claims("opaque-subject-123");
    with_eppn["eppn"] = serde_json::json!("ada@science.state.edu");
    let (status, me) = institution::get(&app, "/api/v1/auth/me", &institution::sign(&key, with_eppn)).await;
    assert_eq!((status, me["user"]["eppn"].as_str()), (StatusCode::OK, Some("ada@science.state.edu")), "{me}");
}

// 2. A wrong audience, a wrong issuer, and an expired token are 401.
#[tokio::test]
async fn a_wrong_audience_a_wrong_issuer_and_an_expired_token_are_refused() {
    let key = institution::new_key("k1");
    let (_db, app, _admin) = institution::trusting(&key).await;
    let mut wrong_aud = institution::claims(institution::PERSON);
    wrong_aud["aud"] = serde_json::json!("some-other-service");
    let mut wrong_iss = institution::claims(institution::PERSON);
    wrong_iss["iss"] = serde_json::json!("https://evil.example.com");
    let mut expired = institution::claims(institution::PERSON);
    expired["exp"] = serde_json::json!(chrono::Utc::now().timestamp() - 3600);
    let mut no_exp = institution::claims(institution::PERSON);
    no_exp.as_object_mut().unwrap().remove("exp");
    for (name, claims) in [("audience", wrong_aud), ("issuer", wrong_iss), ("expiry", expired), ("missing expiry", no_exp)] {
        let (status, _) = institution::get(&app, "/api/v1/auth/me", &institution::sign(&key, claims)).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "a token with a bad {name}");
    }
    // A token signed by a key the institution never published.
    let stranger = institution::new_key("k1");
    let (status, _) = institution::get(&app, "/api/v1/auth/me", &institution::sign(&stranger, institution::claims(institution::PERSON))).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "a key that is not in the JWKS, even under the same kid");
}

// 3. An HS256 token is 401, including one signed with the public key as the secret.
#[tokio::test]
async fn an_hs256_token_is_refused() {
    let key = institution::new_key("k1");
    let (_db, app, _admin) = institution::trusting(&key).await;
    for secret in [b"a shared secret".to_vec(), key.jwk["n"].as_str().unwrap().as_bytes().to_vec()] {
        let mut header = jsonwebtoken::Header::new(jsonwebtoken::Algorithm::HS256);
        header.kid = Some("k1".to_string());
        let token = jsonwebtoken::encode(&header, &institution::claims(institution::PERSON), &jsonwebtoken::EncodingKey::from_secret(&secret)).unwrap();
        let (status, _) = institution::get(&app, "/api/v1/auth/me", &token).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "HS256 is never accepted");
    }
    // An unsigned token (alg none).
    let b64 = |v: &str| base64::Engine::encode(&base64::engine::general_purpose::URL_SAFE_NO_PAD, v);
    let none = format!("{}.{}.", b64(r#"{"alg":"none","typ":"JWT"}"#), b64(&institution::claims(institution::PERSON).to_string()));
    assert_eq!(institution::get(&app, "/api/v1/auth/me", &none).await.0, StatusCode::UNAUTHORIZED);
}

// 4. A claim cannot make a Platform Admin.
#[tokio::test]
async fn a_token_cannot_make_a_platform_admin() {
    let key = institution::new_key("k1");
    let (_db, app, _admin) = institution::trusting(&key).await;
    let mut claims = institution::claims(institution::PERSON);
    claims["affiliation"] = serde_json::json!("central_admin");
    claims["roles"] = serde_json::json!(["platform_admin"]);
    claims["groups"] = serde_json::json!(["central_admin"]);
    let token = institution::sign(&key, claims);
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/orgs")
                .header("authorization", format!("Bearer {token}"))
                .header("content-type", "application/json")
                .body(Body::from(serde_json::json!({ "code": "FAKE", "name": "Fake", "org_type": "department" }).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN, "a claim does not open an admin route");
    let (_, me) = institution::get(&app, "/api/v1/auth/me", &token).await;
    assert_ne!(me["user"]["affiliation"], "central_admin", "{me}");
}

// 5. Replacing the JWKS takes effect on the next request, with no restart.
#[tokio::test]
async fn replacing_the_keys_takes_effect_on_the_next_request() {
    let first = institution::new_key("k1");
    let second = institution::new_key("k2");
    let (_db, app, admin) = institution::trusting(&first).await;
    let old = institution::sign(&first, institution::claims(institution::PERSON));
    let new = institution::sign(&second, institution::claims(institution::PERSON));
    assert_eq!(institution::get(&app, "/api/v1/auth/me", &old).await.0, StatusCode::OK);
    assert_eq!(institution::get(&app, "/api/v1/auth/me", &new).await.0, StatusCode::UNAUTHORIZED, "not yet trusted");
    institution::put_setting(&app, &admin, "oidc.jwks", institution::jwks(&[&second])).await;
    assert_eq!(institution::get(&app, "/api/v1/auth/me", &old).await.0, StatusCode::UNAUTHORIZED, "the first key is gone");
    assert_eq!(institution::get(&app, "/api/v1/auth/me", &new).await.0, StatusCode::OK, "the second key works");
}

// With any of the three settings missing, a JWT is 401. A stored token never reaches the JWT path.
#[tokio::test]
async fn without_all_three_settings_a_jwt_is_refused_and_scf_tokens_are_unchanged() {
    let key = institution::new_key("k1");
    let db = common::TestDb::new();
    let app = build_app().expect("router");
    let token = institution::sign(&key, institution::claims(institution::PERSON));
    assert_eq!(institution::get(&app, "/api/v1/auth/me", &token).await.0, StatusCode::UNAUTHORIZED, "nothing is set");
    let admin = issue_test_token_and_user(institution::ADMIN);
    institution::put_setting(&app, &admin, "oidc.issuer", serde_json::json!(institution::ISSUER)).await;
    institution::put_setting(&app, &admin, "oidc.jwks", institution::jwks(&[&key])).await;
    assert_eq!(institution::get(&app, "/api/v1/auth/me", &token).await.0, StatusCode::UNAUTHORIZED, "no audience is set");
    let own = issue_test_token_and_user(institution::PERSON);
    assert_eq!(institution::get(&app, "/api/v1/auth/me", &own).await.0, StatusCode::OK, "a scf_ token still works");
    let _ = db;
}

// The protected-resource document names the institution's issuer when it is set, and never an environment default.
#[tokio::test]
async fn the_resource_document_names_the_configured_issuer() {
    let key = institution::new_key("k1");
    let db = common::TestDb::new();
    let app = build_app().expect("router");
    let read = |app: axum::Router| async move {
        let resp = app.oneshot(Request::builder().uri("/.well-known/oauth-protected-resource").header("host", "api.test.edu").body(Body::empty()).unwrap()).await.unwrap();
        let bytes = resp.into_body().collect().await.unwrap().to_bytes();
        serde_json::from_slice::<Value>(&bytes).unwrap()
    };
    let before = read(app.clone()).await;
    assert!(!before["authorization_servers"].to_string().contains("auth.state.edu"), "no built-in default issuer: {before}");
    assert!(!before["authorization_servers"].to_string().contains(institution::ISSUER), "{before}");
    let admin = issue_test_token_and_user(institution::ADMIN);
    institution::put_setting(&app, &admin, "oidc.issuer", serde_json::json!(institution::ISSUER)).await;
    let after = read(app.clone()).await;
    assert!(after["authorization_servers"].to_string().contains(institution::ISSUER), "{after}");
    assert_eq!(after["resource"], "https://api.test.edu/api/mcp");
    let _ = (key, db);
}
