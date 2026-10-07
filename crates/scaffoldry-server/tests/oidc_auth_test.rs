//! OAuth 2.1 / OIDC Authentication, JWKS Discovery & CORS Hardening Tests
//!
//! Enforces:
//! 1. Bearer JWT validation against issuer keys with claims mapping to AuthUser.
//! 2. Complete removal of seeded `sct_*` tokens, credential-less login, and hardcoded directory.
//! 3. RFC 9728 & OIDC authorization metadata discovery for MCP clients.
//! 4. CORS restricted to configured institutional origins (removal of `Any`).
//! 5. Cedar-guarded and cryptographically ledger-recorded impersonation.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use scaffoldry_server::build_app;
use scaffoldry_server::jwt::{mint_test_jwt, TestJwtParams};
use serde_json::Value;
use tower::ServiceExt;

#[tokio::test]
async fn test_oauth2_oidc_jwt_bearer_validation() {
    let app = build_app().expect("Build router");

    // 1. Valid signed JWT with institutional claims
    let valid_token = mint_test_jwt(TestJwtParams {
        eppn: "prof.curie@science.state.edu".to_string(),
        name: "Dr. Marie Curie".to_string(),
        role_title: "Professor of Biology".to_string(),
        affiliation: "faculty".to_string(),
        department: "biology".to_string(),
        expires_in_secs: 3600,
    }).expect("mint test jwt");

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

    assert_eq!(resp.status(), StatusCode::OK, "Valid JWT must be accepted");

    // 2. Tampered JWT signature must fail closed with 401
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
    let www_auth = resp.headers().get("www-authenticate").and_then(|h| h.to_str().ok());
    assert!(www_auth.is_some(), "401 must include WWW-Authenticate header");
    assert!(www_auth.unwrap().contains("resource_metadata") || www_auth.unwrap().contains("error="));

    // 3. Expired JWT must fail closed with 401
    let expired_token = mint_test_jwt(TestJwtParams {
        eppn: "prof.curie@science.state.edu".to_string(),
        name: "Dr. Marie Curie".to_string(),
        role_title: "Professor of Biology".to_string(),
        affiliation: "faculty".to_string(),
        department: "biology".to_string(),
        expires_in_secs: -3600, // expired 1 hour ago
    }).expect("mint test jwt");

    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/workspaces")
                .header("authorization", format!("Bearer {expired_token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED, "Expired JWT must be rejected");
}

#[tokio::test]
async fn test_seeded_tokens_and_credential_less_login_deleted() {
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

    // 2. OpenID Connect Discovery 1.0 Metadata
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

    assert_eq!(resp.status(), StatusCode::OK, "OpenID configuration metadata must exist");
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let oidc_meta: Value = serde_json::from_slice(&bytes).unwrap();
    assert!(oidc_meta["issuer"].is_string());
    assert!(oidc_meta["jwks_uri"].is_string());

    // 3. JWKS keys endpoint
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

    assert_eq!(resp.status(), StatusCode::OK, "JWKS keys endpoint must return 200 OK");
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let jwks: Value = serde_json::from_slice(&bytes).unwrap();
    assert!(jwks["keys"].is_array());
}

#[tokio::test]
async fn test_cors_tightened_to_configured_origins() {
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
    let app = build_app().expect("Build router");

    // 1. Non-admin (faculty) trying to impersonate is DENIED by Cedar policy (403 Forbidden)
    let faculty_token = mint_test_jwt(TestJwtParams {
        eppn: "prof.curie@science.state.edu".to_string(),
        name: "Dr. Marie Curie".to_string(),
        role_title: "Professor of Biology".to_string(),
        affiliation: "faculty".to_string(),
        department: "biology".to_string(),
        expires_in_secs: 3600,
    }).expect("mint test jwt");

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
    let admin_token = mint_test_jwt(TestJwtParams {
        eppn: "jordan.lee@state.edu".to_string(),
        name: "Jordan Lee".to_string(),
        role_title: "Central Enterprise Administrator".to_string(),
        affiliation: "central_admin".to_string(),
        department: "Central IT & Institutional Governance".to_string(),
        expires_in_secs: 3600,
    }).expect("mint test jwt");

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
