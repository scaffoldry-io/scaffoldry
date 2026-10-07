//! Tests for server resilience and hardening (Slice B: H2, H4, H5)
//! 1. Production boot refusal on default/missing JWT secret
//! 2. Body limit enforcement (413 Payload Too Large)
//! 3. Graceful error handling on poisoned locks without panicking

use axum::body::Body;
use axum::http::{Request, StatusCode};
use scaffoldry_server::{build_app, build_app_with_state, state::ServerState};
use serde_json::json;
use std::sync::{Arc, Mutex};
use tower::ServiceExt;

static TEST_LOCK: Mutex<()> = Mutex::new(());

#[tokio::test]
async fn test_production_boot_refuses_default_jwt_secret() {
    let _lock = TEST_LOCK.lock().unwrap();

    // 1. In production with no JWT secret set, build_app must refuse to boot
    std::env::set_var("SCAFFOLDRY_ENV", "production");
    std::env::remove_var("SCAFFOLDRY_JWT_SECRET");

    let res = build_app();
    assert!(res.is_err(), "build_app must fail in production if SCAFFOLDRY_JWT_SECRET is missing");
    let err_str = res.err().unwrap().to_string();
    assert!(
        err_str.contains("SCAFFOLDRY_JWT_SECRET"),
        "Error message must specify SCAFFOLDRY_JWT_SECRET requirement: {err_str}"
    );

    // 2. In production with default secret explicitly provided, build_app must also refuse
    std::env::set_var("SCAFFOLDRY_JWT_SECRET", scaffoldry_server::jwt::DEFAULT_SECRET);
    let res = build_app();
    assert!(res.is_err(), "build_app must fail in production if default secret is used");

    // 3. In production with valid custom secret, build_app succeeds
    std::env::set_var("SCAFFOLDRY_JWT_SECRET", "super-secure-production-secret-lattice-entropy-2026-xyz");
    let res = build_app();
    assert!(res.is_ok(), "build_app must succeed in production with valid custom secret");

    // Clean up environment variables
    std::env::remove_var("SCAFFOLDRY_ENV");
    std::env::remove_var("SCAFFOLDRY_JWT_SECRET");
}

#[tokio::test]
async fn test_default_body_limit_enforced() {
    let _lock = TEST_LOCK.lock().unwrap();
    let app = build_app().expect("router");

    let admin_token = scaffoldry_server::jwt::mint_test_jwt(scaffoldry_server::jwt::TestJwtParams {
        eppn: "jordan.lee@state.edu".to_string(),
        name: "Jordan Lee".to_string(),
        role_title: "Central Admin".to_string(),
        affiliation: "central_admin".to_string(),
        department: "Central IT".to_string(),
        expires_in_secs: 3600,
    }).expect("mint jwt");

    // Create a 3 MB payload (exceeds default 2 MB limit)
    let large_blob = "x".repeat(3 * 1024 * 1024);
    let large_payload = json!({
        "slug": "oversized-app",
        "title": "Oversized App",
        "department": "IT",
        "description": large_blob
    });

    let body_bytes = serde_json::to_vec(&large_payload).unwrap();
    let req = Request::builder()
        .method("PUT")
        .uri("/api/v1/apps/oversized-app")
        .header("authorization", format!("Bearer {admin_token}"))
        .header("content-type", "application/json")
        .body(Body::from(body_bytes))
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(
        resp.status(),
        StatusCode::PAYLOAD_TOO_LARGE,
        "Payload exceeding 2MB must be rejected with 413 Payload Too Large"
    );
}

#[tokio::test]
async fn test_lock_poison_does_not_panic_server() {
    let state = Arc::new(ServerState::new().expect("state"));

    // Intentionally poison the datasets lock
    let state_clone = state.clone();
    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || {
        let _guard = state_clone.datasets.write().unwrap();
        panic!("intentional poison for resilience testing");
    }));

    assert!(state.datasets.is_poisoned(), "Lock should be poisoned");

    // Build app with the poisoned state
    let app = build_app_with_state(state).expect("app");

    let admin_token = scaffoldry_server::jwt::mint_test_jwt(scaffoldry_server::jwt::TestJwtParams {
        eppn: "jordan.lee@state.edu".to_string(),
        name: "Jordan Lee".to_string(),
        role_title: "Central Admin".to_string(),
        affiliation: "central_admin".to_string(),
        department: "Central IT".to_string(),
        expires_in_secs: 3600,
    }).expect("mint jwt");

    let req = Request::builder()
        .method("GET")
        .uri("/api/v1/datasets")
        .header("authorization", format!("Bearer {admin_token}"))
        .body(Body::empty())
        .unwrap();

    // The server MUST NOT panic; it must return 500 Internal Server Error cleanly
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(
        resp.status(),
        StatusCode::INTERNAL_SERVER_ERROR,
        "Request touching poisoned lock must return 500 without crashing the server"
    );
}
