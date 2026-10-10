//! Approvers phase 1, test 10: a record remembers who created it, on every write path.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use scaffoldry_server::build_app_with_state;
use scaffoldry_server::service::identity::issue_test_token_and_user;
use scaffoldry_server::state::ServerState;
use serde_json::{json, Value};
use std::sync::Arc;
use tower::ServiceExt;

async fn call(app: &axum::Router, method: &str, uri: &str, token: &str, body: Option<Value>) -> (StatusCode, Value) {
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

#[tokio::test]
async fn the_created_by_column_exists_after_migration() {
    let url = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://scaffoldry:scaffoldry_dev_password@127.0.0.1:5433/scaffoldry".to_string());
    let _ = ServerState::new().expect("state runs the migrations");
    let found = tokio::task::spawn_blocking(move || {
        let mut c = postgres::Client::connect(&url, postgres::NoTls).unwrap();
        let rows = c
            .query(
                "SELECT table_name, column_name FROM information_schema.columns \
                 WHERE (table_name = 'dataset_records' AND column_name = 'created_by') \
                    OR (table_name = 'roles' AND column_name = 'position_key') \
                    OR (table_name = 'position_types' AND column_name = 'max_holders')",
                &[],
            )
            .unwrap();
        rows.len()
    })
    .await
    .unwrap();
    assert_eq!(found, 3, "created_by, position_key and position_types must exist");
}

async fn app_with_demo_data() -> axum::Router {
    let state = Arc::new(ServerState::new().expect("state"));
    state.seed_demo().expect("seed");
    build_app_with_state(state).expect("router")
}

const SLUG: &str = "physics-admissions-review";
const AUTHOR: &str = "jordan.lee@state.edu";

#[tokio::test]
async fn a_record_created_over_rest_stores_the_callers_eppn() {
    let app = app_with_demo_data().await;
    let token = issue_test_token_and_user(AUTHOR);
    let (status, body) = call(&app, "POST", &format!("/api/v1/apps/{SLUG}/records"), &token, Some(json!({ "data": { "candidate_name": "Rest Path", "gpa": 3.9 } }))).await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    assert_eq!(body["created_by"], AUTHOR, "{body}");

    // The author survives a read.
    let id = body["id"].as_str().unwrap();
    let (_, read) = call(&app, "GET", &format!("/api/v1/apps/{SLUG}/records/{id}"), &token, None).await;
    assert_eq!(read["created_by"], AUTHOR, "{read}");
}

#[tokio::test]
async fn a_record_created_over_the_table_route_stores_the_callers_eppn() {
    let app = app_with_demo_data().await;
    let token = issue_test_token_and_user(AUTHOR);
    let (status, body) = call(&app, "POST", &format!("/api/v1/apps/{SLUG}/tables/main/records"), &token, Some(json!({ "data": { "candidate_name": "Table Path", "gpa": 3.5 } }))).await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    assert_eq!(body["created_by"], AUTHOR, "{body}");
}

#[tokio::test]
async fn a_record_created_over_mcp_stores_the_callers_eppn() {
    let app = app_with_demo_data().await;
    let token = issue_test_token_and_user(AUTHOR);
    let (status, body) = call(&app, "POST", "/api/mcp", &token, Some(json!({
        "jsonrpc": "2.0", "id": "1", "method": "tools/call",
        "params": { "name": "create_record", "arguments": { "app_slug": SLUG, "data": { "candidate_name": "Mcp Path", "gpa": 3.7 } } }
    }))).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(!body["result"]["isError"].as_bool().unwrap_or(false), "{body}");
    let text = body["result"]["content"][0]["text"].as_str().expect("tool text");
    let record: Value = serde_json::from_str(text).unwrap();
    assert_eq!(record["created_by"], AUTHOR, "{record}");
}
