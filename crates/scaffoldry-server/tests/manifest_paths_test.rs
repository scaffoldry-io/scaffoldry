//! MCP apps phase 3, server side: every path that stores a manifest validates it, and the schema
//! is a resource an agent can read.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::Router;
use http_body_util::BodyExt;
use scaffoldry_server::build_app_with_state;
use scaffoldry_server::service::identity::issue_test_token_and_user;
use scaffoldry_server::state::ServerState;
use serde_json::{json, Value};
use std::sync::Arc;
use tower::ServiceExt;
use uuid::Uuid;

const OWNER: &str = "prof.curie@science.state.edu";
const WORKSPACE: &str = "ws-bio-lab";

async fn send(app: &Router, method: &str, uri: &str, token: &str, body: Option<Value>) -> (StatusCode, Value) {
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

fn manifest(slug: &str) -> Value {
    json!({
        "slug": slug, "title": "Paths", "description": "d", "organization_code": "BIO", "department": "biology",
        "views": [],
        "tables": [{ "id": "t1", "name": "T", "slug": "t", "fields": [
            { "name": "gpa", "label": "GPA", "field_type": "Number", "required": false, "ferpa_sensitive": false }
        ]}]
    })
}

async fn setup() -> (Router, String) {
    let state = Arc::new(ServerState::new().expect("state"));
    state.seed_demo().expect("seed");
    (build_app_with_state(state).expect("router"), issue_test_token_and_user(OWNER))
}

#[tokio::test]
async fn creating_an_app_with_an_invalid_manifest_lists_every_problem() {
    let (app, token) = setup().await;
    let mut bad = manifest(&format!("bad-{}", &Uuid::new_v4().to_string()[..6]));
    bad["tables"][0]["fields"][0]["name"] = json!("GPA");
    bad["tables"][0]["fields"].as_array_mut().unwrap().push(json!({
        "name": "total", "label": "Total", "field_type": "Formula", "required": false, "ferpa_sensitive": false,
        "formula_expression": "{ghost} + 1"
    }));
    let (s, body) = send(&app, "POST", &format!("/api/v1/workspaces/{WORKSPACE}/apps"), &token, Some(bad)).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{body}");
    let text = body["error"].as_str().unwrap();
    assert!(text.contains("GPA") && text.contains("ghost"), "both problems are named: {text}");
}

#[tokio::test]
async fn a_valid_manifest_is_created_and_a_new_app_cannot_claim_a_verified_domain() {
    let (app, token) = setup().await;
    let slug = format!("ok-{}", &Uuid::new_v4().to_string()[..6]);
    let mut m = manifest(&slug);
    m["custom_domain_verified"] = json!(true);
    let (s, body) = send(&app, "POST", &format!("/api/v1/workspaces/{WORKSPACE}/apps"), &token, Some(m)).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{body}");
    assert!(body["error"].as_str().unwrap().contains("custom_domain_verified"));

    let (s, body) = send(&app, "POST", &format!("/api/v1/workspaces/{WORKSPACE}/apps"), &token, Some(manifest(&slug))).await;
    assert_eq!(s, StatusCode::CREATED, "{body}");
    assert_eq!(body["custom_domain_verified"], false);
}

#[tokio::test]
async fn updating_an_app_validates_the_manifest_too() {
    let (app, token) = setup().await;
    let slug = format!("upd-{}", &Uuid::new_v4().to_string()[..6]);
    let (s, _) = send(&app, "POST", &format!("/api/v1/workspaces/{WORKSPACE}/apps"), &token, Some(manifest(&slug))).await;
    assert_eq!(s, StatusCode::CREATED);
    let (_, current) = send(&app, "GET", &format!("/api/v1/apps/{slug}"), &token, None).await;

    let mut bad = current.clone();
    bad["pages"] = json!([{ "id": "p", "title": "P", "kind": { "View": { "view_id": "ghost-view" } } }]);
    let (s, body) = send(&app, "PUT", &format!("/api/v1/apps/{slug}"), &token, Some(bad)).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{body}");
    assert!(body["error"].as_str().unwrap().contains("ghost-view"));

    let mut good = current;
    good["description"] = json!("changed");
    let (s, body) = send(&app, "PUT", &format!("/api/v1/apps/{slug}"), &token, Some(good)).await;
    assert_eq!(s, StatusCode::OK, "{body}");
}

#[tokio::test]
async fn the_schema_is_an_mcp_resource() {
    let (app, token) = setup().await;
    let (_, list) = send(&app, "POST", "/api/mcp", &token, Some(json!({ "jsonrpc": "2.0", "id": 1, "method": "resources/list" }))).await;
    assert!(list["result"]["resources"].as_array().unwrap().iter().any(|r| r["uri"] == "scaffoldry://schema/app-manifest"), "{list}");

    let (_, read) = send(&app, "POST", "/api/mcp", &token, Some(json!({ "jsonrpc": "2.0", "id": 2, "method": "resources/read", "params": { "uri": "scaffoldry://schema/app-manifest" } }))).await;
    let text = read["result"]["contents"][0]["text"].as_str().expect("the schema text");
    let schema: Value = serde_json::from_str(text).expect("it is JSON");
    assert_eq!(schema["$schema"], "https://json-schema.org/draft/2020-12/schema");
    assert!(schema["properties"]["pages"].is_object());
}
