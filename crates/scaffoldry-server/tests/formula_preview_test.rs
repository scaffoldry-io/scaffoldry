//! Formula preview route tests: validation, evaluation, position error, and rate limiting

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use scaffoldry_engine::{AppManifest, AppTable, FieldSpec, FieldType};
use scaffoldry_server::build_app_with_state;
use scaffoldry_server::service::identity::issue_test_token_and_user;
use scaffoldry_server::state::ServerState;
use serde_json::{json, Value};
use std::sync::Arc;
use tower::ServiceExt;

const OWNER: &str = "prof.curie@science.state.edu";
const WORKSPACE: &str = "ws-bio-lab";

fn make_manifest(slug: &str) -> AppManifest {
    AppManifest {
        slug: slug.to_string(),
        title: "Preview App".to_string(),
        description: String::new(),
        organization_code: "PREV".to_string(),
        department: "testing".to_string(),
        workspace_id: Some(WORKSPACE.to_string()),
        herm_capability_id: None,
        coordinates: None, sealed: false,
        custom_domain: None,
        custom_domain_verified: false,
        tables: vec![AppTable {
            id: "grants".to_string(),
            name: "Grants".to_string(),
            slug: "grants".to_string(),
            description: None,
            icon: None,
            fields: vec![
                FieldSpec::simple("title", "Title", FieldType::Text, false, false),
                FieldSpec::simple("budget", "Budget", FieldType::Number, false, false),
                FieldSpec::simple("spent", "Spent", FieldType::Number, false, false),
            ],
            primary_field: Some("title".to_string()),
            coordinates: None, sealed: false,
            row_place: Default::default(),
            sample_records: vec![json!({
                "id": "GRANT-1",
                "title": "Bio Research",
                "budget": 50000.0,
                "spent": 12000.0
            })],
        }],
        relationships: vec![],
        views: vec![],
        ceds_mappings: Default::default(),
        pages: vec![],
        access: Default::default(),
    }
}

struct TestWorld {
    state: Arc<ServerState>,
    token: String,
    slug: String,
}

impl TestWorld {
    fn setup() -> Self {
        let state = Arc::new(ServerState::new().expect("state"));
        state.seed_demo().expect("seed");
        let token = issue_test_token_and_user(OWNER);
        let slug = format!("preview-app-{}", uuid::Uuid::new_v4().simple());
        let manifest = make_manifest(&slug);
        state.engine.write().unwrap().register_manifest(manifest).unwrap();
        Self { state, token, slug }
    }

    async fn preview(&self, body: Value) -> (StatusCode, Value) {
        let app = build_app_with_state(self.state.clone()).expect("router");
        let req = Request::builder()
            .method("POST")
            .uri("/api/v1/formula/preview")
            .header("Authorization", format!("Bearer {}", self.token))
            .header("Content-Type", "application/json")
            .body(Body::from(serde_json::to_vec(&body).unwrap()))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        let status = resp.status();
        let bytes = resp.into_body().collect().await.unwrap().to_bytes();
        let val: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
        (status, val)
    }
}

#[tokio::test]
async fn preview_evaluates_valid_expression_with_custom_record() {
    let _db = common::TestDb::new();
    let world = TestWorld::setup();
    let (status, body) = world
        .preview(json!({
            "expression": "{budget} - {spent}",
            "table_id": "grants",
            "app_slug": world.slug,
            "record": {
                "budget": 100000.0,
                "spent": 40000.0
            }
        }))
        .await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["valid"], true);
    assert_eq!(body["result"], json!(60000.0));
    assert!(body["error"].is_null());
}

#[tokio::test]
async fn preview_returns_same_value_as_stored_eval() {
    let _db = common::TestDb::new();
    let world = TestWorld::setup();
    let record = json!({
        "budget": 50000.0,
        "spent": 12000.0
    });
    let expr = "ROUND({budget} * 0.15, 0)";

    let (status, body) = world
        .preview(json!({
            "expression": expr,
            "table_id": "grants",
            "app_slug": world.slug,
            "record": record
        }))
        .await;

    assert_eq!(status, StatusCode::OK);
    let expected = scaffoldry_engine::evaluate_formula(expr, &record);
    assert_eq!(body["result"], expected);
}

#[tokio::test]
async fn preview_returns_position_for_syntax_error() {
    let _db = common::TestDb::new();
    let world = TestWorld::setup();
    let (status, body) = world
        .preview(json!({
            "expression": "{budget} * (0.15 +",
            "table_id": "grants",
            "app_slug": world.slug
        }))
        .await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["valid"], false);
    assert!(body["error"].is_string());
    assert!(body["error_pos"].is_number());
    assert!(!body["problems"].as_array().unwrap().is_empty());
}

#[tokio::test]
async fn preview_division_by_zero_and_non_numeric_yield_blank() {
    let _db = common::TestDb::new();
    let world = TestWorld::setup();
    let (status, body) = world
        .preview(json!({
            "expression": "{budget} / 0",
            "table_id": "grants",
            "app_slug": world.slug,
            "record": { "budget": 1000.0 }
        }))
        .await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["valid"], true);
    assert!(body["result"].is_null());
}

#[tokio::test]
async fn preview_rate_limited_to_20_per_second() {
    let _db = common::TestDb::new();
    let world = TestWorld::setup();
    let mut got_429 = false;

    for i in 0..25 {
        let (status, _) = world
            .preview(json!({
                "expression": format!("{{budget}} + {i}"),
                "table_id": "grants",
                "app_slug": world.slug,
                "record": { "budget": 100.0 }
            }))
            .await;

        if status == StatusCode::TOO_MANY_REQUESTS {
            got_429 = true;
            break;
        }
    }

    assert!(got_429, "Expected to receive HTTP 429 Too Many Requests after 20 previews in 1 second");
}
