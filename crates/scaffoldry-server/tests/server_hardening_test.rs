//! Tests for server resilience and hardening (Slice B: H2, H4, H5)
//! 1. Production boot refusal on default/missing JWT secret
//! 2. Body limit enforcement (413 Payload Too Large)
//! 3. Graceful error handling on poisoned locks without panicking

use axum::body::Body;
use axum::http::{Request, StatusCode};
use scaffoldry_server::{build_app, build_app_with_state, state::ServerState};
use serde_json::json;
use std::sync::Arc;
use tokio::sync::Mutex;
use tower::ServiceExt;

static TEST_LOCK: Mutex<()> = Mutex::const_new(());

#[tokio::test]
async fn test_boot_with_unreachable_database_url_returns_error() {
    let _lock = TEST_LOCK.lock().await;
    std::env::set_var("DATABASE_URL", "postgres://scaffoldry:wrong@127.0.0.1:5439/nonexistent");
    let res = scaffoldry_server::repository::PostgresRepository::connect(Some("postgres://scaffoldry:wrong@127.0.0.1:5439/nonexistent"));
    assert!(res.is_err(), "PostgresRepository::connect with unreachable DATABASE_URL must return an error");
    std::env::remove_var("DATABASE_URL");
}

#[tokio::test]
async fn test_default_body_limit_enforced() {
    let _lock = TEST_LOCK.lock().await;
    let app = build_app().expect("router");

    let admin_token = scaffoldry_server::service::identity::issue_test_token_and_user("jordan.lee@state.edu");

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
    let _lock = TEST_LOCK.lock().await;
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

    let admin_token = scaffoldry_server::service::identity::issue_test_token_and_user("jordan.lee@state.edu");

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

#[tokio::test]
async fn test_expired_session_rejected_with_401() {
    let _lock = TEST_LOCK.lock().await;
    let state = Arc::new(ServerState::new().expect("state"));

    let expired_token = scaffoldry_server::service::identity::generate_raw_token();
    let expired_hash = scaffoldry_server::service::identity::hash_token(&expired_token);
    let expired_row = scaffoldry_server::state::ApiToken {
        token_hash: expired_hash,
        id: uuid::Uuid::new_v4(),
        kind: "agent".to_string(),
        eppn: "expired.user@state.edu".to_string(),
        label: "Expired".to_string(),
        original_admin: None,
        created_at: chrono::Utc::now() - chrono::Duration::hours(2),
        expires_at: chrono::Utc::now() - chrono::Duration::hours(1),
        last_used_at: None,
        revoked_at: None,
    };
    let _ = state.persist_api_token(&expired_row);

    let app = build_app_with_state(state).expect("app");

    let req = Request::builder()
        .method("GET")
        .uri("/api/v1/auth/me")
        .header("authorization", format!("Bearer {expired_token}"))
        .body(Body::empty())
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(
        resp.status(),
        StatusCode::UNAUTHORIZED,
        "Session older than 1 hour must be rejected with 401 Unauthorized"
    );
}

fn get_db_url() -> String {
    std::env::var("DATABASE_URL").unwrap_or_else(|_| {
        "postgres://scaffoldry:scaffoldry_dev_password@127.0.0.1:5433/scaffoldry".to_string()
    })
}

#[tokio::test]
async fn test_tamper_detected_on_nonsense_decision_type() {
    let _lock = TEST_LOCK.lock().await;
    tokio::task::spawn_blocking(|| {
        let mut client = match postgres::Client::connect(&get_db_url(), postgres::NoTls) {
            Ok(c) => c,
            Err(e) => {
                eprintln!("Skipping test: PostgreSQL not reachable ({e})");
                return;
            }
        };

        let repo = scaffoldry_server::repository::PostgresRepository::connect(Some(&get_db_url()))
            .expect("repo connect");

        let last_row = client.query_one(
            "SELECT sequence, entry_hash FROM governance_ledger ORDER BY sequence DESC LIMIT 1",
            &[],
        ).expect("query last ledger row");
        let last_seq: i64 = last_row.get(0);
        let last_hash: String = last_row.get(1);
        let next_seq = last_seq + 1;

        // Clean up if previous run left sequence next_seq
        let _ = client.execute("DROP TRIGGER IF EXISTS governance_ledger_no_change ON governance_ledger", &[]);
        let _ = client.execute("DELETE FROM governance_ledger WHERE sequence = $1", &[&next_seq]);
        let _ = client.execute(
            "CREATE TRIGGER governance_ledger_no_change BEFORE UPDATE OR DELETE OR TRUNCATE ON governance_ledger FOR EACH STATEMENT EXECUTE FUNCTION governance_ledger_immutable()",
            &[],
        );

        // Insert a row with decision_type = Nonsense
        let insert_res = client.execute(
            "INSERT INTO governance_ledger (sequence, entry_hash, previous_hash, timestamp_iso,              principal, organization_code, app_slug, decision_type, oscal_control_id, rationale,              payload_hash, payload)              VALUES ($1, 'fakehash', $2, '2026-01-01T00:00:00Z',              'test_principal', 'TEST', NULL, 'Nonsense', 'AC-01', 'Test rationale', '', '{}')",
            &[&next_seq, &last_hash],
        );
        assert!(insert_res.is_ok(), "Insert nonsense row");

        let verify_res = repo.verify_and_initialize_ledger(false);

        // Clean up inserted row immediately
        let _ = client.execute("DROP TRIGGER IF EXISTS governance_ledger_no_change ON governance_ledger", &[]);
        let _ = client.execute("DELETE FROM governance_ledger WHERE sequence = $1", &[&next_seq]);
        let _ = client.execute(
            "CREATE TRIGGER governance_ledger_no_change BEFORE UPDATE OR DELETE OR TRUNCATE ON governance_ledger FOR EACH STATEMENT EXECUTE FUNCTION governance_ledger_immutable()",
            &[],
        );

        match verify_res {
            Err(scaffoldry_server::repository::RepositoryError::TamperDetected(msg)) => {
                assert!(
                    msg.contains("Nonsense"),
                    "TamperDetected should mention Nonsense, got: {msg}"
                );
            }
            other => panic!("Expected RepositoryError::TamperDetected, got {:?}", other),
        }
    }).await.unwrap();
}

#[tokio::test]
async fn test_workspace_update_fails_when_ledger_fails() {
    let _lock = TEST_LOCK.lock().await;
    tokio::task::spawn_blocking(|| {
        let mut client = match postgres::Client::connect(&get_db_url(), postgres::NoTls) {
            Ok(c) => c,
            Err(e) => {
                eprintln!("Skipping test: PostgreSQL not reachable ({e})");
                return;
            }
        };

        let state = Arc::new(ServerState::new().expect("state"));

        let fail_principal = "fail_ledger_test_user@state.edu";
        let caller = scaffoldry_server::state::AuthUser {
            eppn: fail_principal.to_string(),
            name: "Fail Test User".to_string(),
            role_title: "Central Admin".to_string(),
            affiliation: "central_admin".to_string(),
            department: "Central IT".to_string(),
        };

        let initial_ws = state
            .workspaces
            .read()
            .unwrap()
            .get("ws-campus-compliance")
            .cloned()
            .expect("ws-campus-compliance should exist");
        let original_visibility = initial_ws.visibility.clone();
        let new_visibility = if original_visibility == "public" {
            "restricted".to_string()
        } else {
            "public".to_string()
        };

        // Add constraint to make ledger insert fail on fail_principal
        let _ = client.execute("ALTER TABLE governance_ledger DROP CONSTRAINT IF EXISTS fail_workspace_update_ledger", &[]);
        client
            .execute(
                "ALTER TABLE governance_ledger ADD CONSTRAINT fail_workspace_update_ledger CHECK (principal != 'fail_ledger_test_user@state.edu')",
                &[],
            )
            .expect("add check constraint");

        let update_payload = scaffoldry_server::service::workspaces::UpdateWorkspacePayload {
            name: None,
            description: None,
            department: None,
            visibility: Some(new_visibility.clone()),
            allowed_affiliations: None,
            data_classification: None,
            icon: None,
            cedar_policy_guard: None,
        };

        let res = scaffoldry_server::service::workspaces::update_workspace(
            &caller,
            "ws-campus-compliance",
            update_payload,
            &state,
        );

        // Clean up constraint immediately
        let _ = client.execute("ALTER TABLE governance_ledger DROP CONSTRAINT IF EXISTS fail_workspace_update_ledger", &[]);

        assert!(res.is_err(), "update_workspace MUST fail when ledger append fails");

        let stored_ws = state
            .workspaces
            .read()
            .unwrap()
            .get("ws-campus-compliance")
            .cloned()
            .unwrap();
        assert_eq!(
            stored_ws.visibility, original_visibility,
            "Stored workspace visibility must remain unchanged when ledger write fails"
        );
    }).await.unwrap();
}
