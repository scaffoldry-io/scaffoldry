//! UX standards phase 2: every error has a code, and a denial names the policy that made it.

mod common;

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
        (ServiceError::StaleVersion { message: "Stale".into(), version: 4 }, "version_conflict", StatusCode::CONFLICT),
        (ServiceError::TooLarge("Big".into()), "too_large", StatusCode::PAYLOAD_TOO_LARGE),
        (ServiceError::FileTooLarge("Too big".into()), "file_too_large", StatusCode::PAYLOAD_TOO_LARGE),
        (ServiceError::FileTypeBlocked("Blocked".into()), "file_type_blocked", StatusCode::BAD_REQUEST),
        (ServiceError::ScannerUnavailable("No scanner".into()), "scanner_unavailable", StatusCode::SERVICE_UNAVAILABLE),
        (ServiceError::StorageUnavailable("No database".into()), "storage_unavailable", StatusCode::SERVICE_UNAVAILABLE),
        (ServiceError::IndexBuilding("Preparing".into()), "index_building", StatusCode::CONFLICT),
        (ServiceError::Calculating("Calculating".into()), "calculating", StatusCode::CONFLICT),
        (ServiceError::QuotaExceeded("Busy".into()), "quota_exceeded", StatusCode::TOO_MANY_REQUESTS),
        (ServiceError::TooManyIndexes { message: "Too many".into(), unused: vec!["a".into()] }, "too_many_indexes", StatusCode::BAD_REQUEST),
        (ServiceError::ViewAsReadOnly("Read only".into()), "view_as_read_only", StatusCode::FORBIDDEN),
        (ServiceError::ImpersonationNotSynthetic("Real".into()), "impersonation_not_synthetic", StatusCode::FORBIDDEN),
        (ServiceError::GrantConflict("Both".into()), "grant_conflict", StatusCode::CONFLICT),
        (ServiceError::CitationNotFound("No such passage".into()), "citation_not_found", StatusCode::BAD_REQUEST),
        (ServiceError::AuthorityLoosens("Too loose".into()), "authority_loosens", StatusCode::BAD_REQUEST),
        (ServiceError::ConstraintViolated("Linked".into()), "constraint_violated", StatusCode::CONFLICT),
        (
            ServiceError::ConstraintViolation {
                message: "field 'score' violates constraint 'min'".into(),
                field: "score".into(),
                constraint: "min".into(),
            },
            "constraint_violated",
            StatusCode::BAD_REQUEST,
        ),
        (
            ServiceError::FieldHidden { message: "Hidden".into(), reason: "A rule hides it".into(), remedy: "Ask an owner".into() },
            "field_hidden",
            StatusCode::FORBIDDEN,
        ),
        (
            ServiceError::FieldReadOnly { message: "Read only".into(), fields: vec!["gpa".into()], reason: "A rule".into(), remedy: "Ask an owner".into() },
            "field_read_only",
            StatusCode::FORBIDDEN,
        ),
        (ServiceError::UndoConflict { message: "Moved on".into(), conflicts: vec![] }, "undo_conflict", StatusCode::CONFLICT),
        (ServiceError::GrantOutsideScope("Outside".into()), "grant_outside_scope", StatusCode::FORBIDDEN),
        (ServiceError::GrantTooBroad("Broad".into()), "grant_too_broad", StatusCode::BAD_REQUEST),
        (ServiceError::ProposalChanged("Changed".into()), "proposal_changed", StatusCode::CONFLICT),
        (ServiceError::PreviewReadOnly("Read only".into()), "preview_read_only", StatusCode::FORBIDDEN),
        (ServiceError::TokenScope("Scoped".into()), "token_scope", StatusCode::FORBIDDEN),
        (ServiceError::ClientScope("Ceiling".into()), "client_scope", StatusCode::FORBIDDEN),
        (ServiceError::ClientSensitiveField("Sensitive".into()), "client_sensitive_field", StatusCode::BAD_REQUEST),
        (ServiceError::Internal("Oops".into()), "internal", StatusCode::INTERNAL_SERVER_ERROR),
    ];
    for (e, _, _) in &samples {
        match e {
            ServiceError::Unauthorized(_)
            | ServiceError::Forbidden { .. }
            | ServiceError::GrantOutsideScope(_)
            | ServiceError::GrantTooBroad(_)
            | ServiceError::ProposalChanged(_)
            | ServiceError::PreviewReadOnly(_)
            | ServiceError::TokenScope(_)
            | ServiceError::ClientScope(_)
            | ServiceError::ClientSensitiveField(_)
            | ServiceError::NotFound(_)
            | ServiceError::BadRequest(_)
            | ServiceError::Invalid { .. }
            | ServiceError::Conflict(_)
            | ServiceError::PositionFull(_)
            | ServiceError::StaleVersion { .. }
            | ServiceError::TooLarge(_)
            | ServiceError::FileTooLarge(_)
            | ServiceError::FileTypeBlocked(_)
            | ServiceError::ScannerUnavailable(_)
            | ServiceError::StorageUnavailable(_)
            | ServiceError::IndexBuilding(_)
            | ServiceError::Calculating(_)
            | ServiceError::ViewAsReadOnly(_)
            | ServiceError::ImpersonationNotSynthetic(_)
            | ServiceError::GrantConflict(_)
            | ServiceError::CitationNotFound(_)
            | ServiceError::AuthorityLoosens(_)
            | ServiceError::QuotaExceeded(_)
            | ServiceError::TooManyIndexes { .. }
            | ServiceError::ConstraintViolated(_)
            | ServiceError::ConstraintViolation { .. }
            | ServiceError::FieldHidden { .. }
            | ServiceError::FieldReadOnly { .. }
            | ServiceError::UndoConflict { .. }
            | ServiceError::Internal(_) => {}
        }
    }
    samples
}

#[tokio::test]
async fn every_service_error_variant_serializes_with_a_code_from_the_closed_list() {
    let _db = common::TestDb::new();
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
    let _db = common::TestDb::new();
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
    let _db = common::TestDb::new();
    let error = ServiceError::Forbidden {
        message: "Not allowed".into(),
        reasons: vec!["policy11".into()],
        diagnostics: vec![],
        policy: Some(Box::new(PolicyRef { id: "policy11".into(), description: "Only members may open restricted workspaces.".into(), rule: None })),
        action: None,
    };
    let (_, body) = body_of(error.into_response()).await;
    assert_eq!(body["policy"]["id"], "policy11");
    assert_eq!(body["policy"]["description"], "Only members may open restricted workspaces.");
    assert_eq!(body["reasons"][0], "policy11", "the existing reasons field is kept");
}

#[tokio::test]
async fn a_cedar_denial_over_http_holds_the_policy_id_and_a_description() {
    let _db = common::TestDb::new();
    let db_url = scaffoldry_server::testing::database_url();
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
    let _db = common::TestDb::new();
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
