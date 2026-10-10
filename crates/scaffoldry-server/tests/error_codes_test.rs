//! UX standards phase 2: every error has a code, and a denial names the policy that made it.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::response::IntoResponse;
use http_body_util::BodyExt;
use scaffoldry_policy::PolicyRef;
use scaffoldry_server::service::{ServiceError, ERROR_CODES};
use scaffoldry_server::state::ServerState;
use scaffoldry_server::build_app_with_state;
use serde_json::Value;
use std::collections::BTreeMap;
use std::sync::Arc;
use tower::ServiceExt;

async fn body_of(resp: axum::response::Response) -> (StatusCode, Value) {
    let status = resp.status();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
}

/// One sample of every variant, with the code and status it must produce. The `match` has no
/// wildcard, so adding a variant without a row here is a compile error.
fn samples() -> Vec<(ServiceError, &'static str, StatusCode)> {
    let samples = vec![
        (ServiceError::Unauthorized("Sign in".into()), "unauthorized", StatusCode::UNAUTHORIZED),
        (ServiceError::forbidden("No"), "forbidden", StatusCode::FORBIDDEN),
        (ServiceError::NotFound("Gone".into()), "not_found", StatusCode::NOT_FOUND),
        (ServiceError::BadRequest("Bad".into()), "bad_request", StatusCode::BAD_REQUEST),
        (
            ServiceError::Invalid {
                message: "Fix the form".into(),
                fields: BTreeMap::from([("reason".to_string(), "required".to_string())]),
            },
            "bad_request",
            StatusCode::BAD_REQUEST,
        ),
        (ServiceError::Conflict("Changed".into()), "version_conflict", StatusCode::CONFLICT),
        (ServiceError::PositionFull("Full".into()), "position_full", StatusCode::CONFLICT),
        (ServiceError::TooLarge("Big".into()), "too_large", StatusCode::PAYLOAD_TOO_LARGE),
        (ServiceError::Internal("Oops".into()), "internal", StatusCode::INTERNAL_SERVER_ERROR),
    ];
    for (e, _, _) in &samples {
        match e {
            ServiceError::Unauthorized(_)
            | ServiceError::Forbidden { .. }
            | ServiceError::NotFound(_)
            | ServiceError::BadRequest(_)
            | ServiceError::Invalid { .. }
            | ServiceError::Conflict(_)
            | ServiceError::PositionFull(_)
            | ServiceError::TooLarge(_)
            | ServiceError::Internal(_) => {}
        }
    }
    samples
}

#[tokio::test]
async fn every_service_error_variant_serializes_with_a_code_from_the_closed_list() {
    for (error, code, status) in samples() {
        let (got_status, body) = body_of(error.into_response()).await;
        assert_eq!(got_status, status);
        assert_eq!(body["code"], code);
        assert!(ERROR_CODES.contains(&body["code"].as_str().unwrap()), "{code} is in the closed list");
        assert!(body["error"].as_str().is_some_and(|s| !s.is_empty()), "the plain sentence is present");
    }
}

#[tokio::test]
async fn invalid_input_names_the_bad_fields_and_only_it_does() {
    let (_, body) = body_of(
        ServiceError::Invalid {
            message: "Fix the form".into(),
            fields: BTreeMap::from([("reason".to_string(), "required".to_string())]),
        }
        .into_response(),
    )
    .await;
    assert_eq!(body["fields"]["reason"], "required");

    let (_, plain) = body_of(ServiceError::BadRequest("Bad".into()).into_response()).await;
    assert!(plain.get("fields").is_none(), "no empty fields object");
    assert!(plain.get("policy").is_none(), "no policy on a non-denial");
}

#[tokio::test]
async fn a_forbidden_error_carries_the_policy_that_decided() {
    let error = ServiceError::Forbidden {
        message: "Not allowed".into(),
        reasons: vec!["policy11".into()],
        diagnostics: vec![],
        policy: Some(PolicyRef { id: "policy11".into(), description: "Only members may open restricted workspaces.".into() }),
    };
    let (_, body) = body_of(error.into_response()).await;
    assert_eq!(body["policy"]["id"], "policy11");
    assert_eq!(body["policy"]["description"], "Only members may open restricted workspaces.");
    assert_eq!(body["reasons"][0], "policy11", "the existing reasons field is kept");
}

#[tokio::test]
async fn a_cedar_denial_over_http_holds_the_policy_id_and_a_description() {
    let db_url = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://scaffoldry:scaffoldry_dev_password@127.0.0.1:5433/scaffoldry".to_string());
    tokio::task::spawn_blocking(move || {
        if let Ok(mut client) = postgres::Client::connect(&db_url, postgres::NoTls) {
            let _ = client.batch_execute(
                "TRUNCATE TABLE organizations CASCADE; TRUNCATE TABLE workspaces CASCADE; \
                 TRUNCATE TABLE workspace_collaborators CASCADE; TRUNCATE TABLE roles CASCADE;",
            );
        }
    })
    .await
    .unwrap();
    let state = Arc::new(ServerState::new().expect("state"));
    state.seed_demo().expect("seed");
    let app = build_app_with_state(state).expect("router");
    let faculty = scaffoldry_server::service::identity::issue_test_token_and_user("sarah.connor@state.edu");

    // A faculty member who is not a member of the restricted lab workspace.
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/api/v1/workspaces/ws-bio-lab")
                .header("authorization", format!("Bearer {faculty}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let (status, body) = body_of(resp).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(body["code"], "forbidden");
    assert!(body["policy"]["id"].as_str().is_some_and(|s| !s.is_empty()), "the policy id is named: {body}");
    assert!(
        body["policy"]["description"].as_str().is_some_and(|s| !s.trim().is_empty()),
        "the description is a sentence a person can read"
    );
}

/// The rule that no handler builds error JSON by hand. Every error goes through `ServiceError`.
/// It catches `json!({"error"`, `json!({ "error"`, and the spelling split across lines.
#[test]
fn no_handler_builds_error_json_by_hand() {
    fn hand_built(text: &str) -> Vec<usize> {
        let mut found = Vec::new();
        let mut from = 0;
        while let Some(at) = text[from..].find("json!({") {
            let start = from + at + "json!({".len();
            if text[start..].trim_start().starts_with("\"error\"") {
                found.push(text[..start].matches('\n').count() + 1);
            }
            from = start;
        }
        found
    }
    fn walk(dir: &std::path::Path, hits: &mut Vec<String>) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                walk(&path, hits);
            } else if path.extension().is_some_and(|e| e == "rs") {
                let text = std::fs::read_to_string(&path).unwrap();
                for line in hand_built(&text) {
                    hits.push(format!("{}:{}", path.display(), line));
                }
            }
        }
    }
    // The detector itself must catch every spelling, or the test proves nothing.
    assert_eq!(hand_built("json!({\"error\": 1})").len(), 1);
    assert_eq!(hand_built("json!({ \"error\": 1})").len(), 1);
    assert_eq!(hand_built("json!({\n    \"error\": 1})").len(), 1);
    assert_eq!(hand_built("json!({\"errors\": 1})").len(), 0);

    let mut hits = Vec::new();
    walk(&std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src"), &mut hits);
    assert!(hits.is_empty(), "hand-built error JSON remains:\n{}", hits.join("\n"));
}
