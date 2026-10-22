//! A record write names fields the table has and sends values of the type the field stores. The MCP suite found three
//! gaps: an unknown field, text in a Number field, and an unknown export format were all accepted.

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use common::TestDb;
use http_body_util::BodyExt;
use scaffoldry_server::build_app_with_state;
use scaffoldry_server::service::identity::issue_test_token_and_user;
use serde_json::{json, Value};
use tower::ServiceExt;
use uuid::Uuid;

const WORKSPACE: &str = "ws-physics-optics";
const OWNER: &str = "einstein@physics.state.edu";

async fn send(app: &axum::Router, method: &str, uri: &str, body: Option<Value>) -> (StatusCode, Value) {
    let b = Request::builder().method(method).uri(uri).header("authorization", format!("Bearer {}", issue_test_token_and_user(OWNER)));
    let req = match body {
        Some(v) => b.header("content-type", "application/json").body(Body::from(v.to_string())).unwrap(),
        None => b.body(Body::empty()).unwrap(),
    };
    let resp = app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
}

async fn call(app: &axum::Router, tool: &str, args: Value) -> Value {
    let payload = json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": { "name": tool, "arguments": args } });
    send(app, "POST", "/api/v1/mcp", Some(payload)).await.1["result"].clone()
}

struct Fixture {
    app: axum::Router,
    slug: String,
    _db: TestDb,
}

async fn fixture() -> Fixture {
    let db = TestDb::new();
    let state = db.state();
    state.seed_demo().expect("seed");
    let app = build_app_with_state(state).expect("router");
    let slug = format!("val-{}", &Uuid::new_v4().to_string()[..8]);
    let manifest = json!({
        "slug": slug, "title": "Validation", "description": "d", "organization_code": "PHYS", "department": "physics", "views": [],
        "tables": [{ "id": "people", "name": "People", "slug": "people", "fields": [
            { "name": "name", "label": "Name", "field_type": "Text", "required": true, "ferpa_sensitive": false },
            { "name": "score", "label": "Score", "field_type": "Number", "required": false, "ferpa_sensitive": false },
            { "name": "active", "label": "Active", "field_type": "Boolean", "required": false, "ferpa_sensitive": false }
        ] }]
    });
    let (s, body) = send(&app, "POST", &format!("/api/v1/workspaces/{WORKSPACE}/apps"), Some(manifest)).await;
    assert_eq!(s, StatusCode::CREATED, "{body}");
    Fixture { app, slug, _db: db }
}

async fn create(f: &Fixture, mut data: Value) -> (StatusCode, Value) {
    data["_table_id"] = json!("people");
    send(&f.app, "POST", &format!("/api/v1/apps/{}/records", f.slug), Some(json!({ "data": data }))).await
}

async fn made(f: &Fixture) -> (String, i64) {
    let (s, body) = create(f, json!({ "name": "Ada", "score": 3 })).await;
    assert_eq!(s, StatusCode::CREATED, "{body}");
    (body["id"].as_str().unwrap().to_string(), body["version"].as_i64().unwrap_or(1))
}

#[tokio::test]
async fn a_field_the_table_does_not_have_is_refused_on_create_and_update() {
    let f = fixture().await;
    let (s, body) = create(&f, json!({ "name": "Bo", "nope": 1 })).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{body}");
    let text = body.to_string();
    assert!(text.contains("nope") && text.contains("people"), "the field and the table are named: {text}");

    let (id, version) = made(&f).await;
    let (s, body) = send(&f.app, "PATCH", &format!("/api/v1/apps/{}/records/{id}", f.slug), Some(json!({ "data": { "nope": 1 }, "version": version }))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "an update names a real field too: {body}");

    // Through the MCP tool the answer is a tool error with a hint.
    let r = call(&f.app, "create_record", json!({ "app_slug": f.slug, "data": { "_table_id": "people", "name": "Bo", "nope": 1 } })).await;
    assert_eq!((r["isError"].clone(), r["structuredContent"]["code"].clone()), (json!(true), json!("bad_request")), "{r}");
    assert!(r["structuredContent"]["hint"].as_str().is_some_and(|h| h.contains("describe_app")), "{r}");

    // Nothing was stored for the refused write.
    let (_, listed) = send(&f.app, "GET", &format!("/api/v1/apps/{}/tables/people/records", f.slug), None).await;
    assert_eq!(listed["records"].as_array().map(Vec::len), Some(1), "only the good record is there: {listed}");
}

#[tokio::test]
async fn a_value_of_the_wrong_type_is_refused() {
    let f = fixture().await;
    for (what, data) in [
        ("text in a Number field", json!({ "name": "Bo", "score": "high" })),
        ("a list in a Number field", json!({ "name": "Bo", "score": [1] })),
        ("a word in a Boolean field", json!({ "name": "Bo", "active": "maybe" })),
        ("a number in a Text field", json!({ "name": 7 })),
        ("a list in a Text field", json!({ "name": ["Bo"] })),
    ] {
        let (s, body) = create(&f, data).await;
        assert_eq!(s, StatusCode::BAD_REQUEST, "{what}: {body}");
    }
    let (s, body) = create(&f, json!({ "name": "Bo", "score": "high" })).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    assert!(body.to_string().contains("score"), "the field is named: {body}");
}

#[tokio::test]
async fn values_that_fit_are_still_accepted() {
    let f = fixture().await;
    for (what, data) in [
        ("a number", json!({ "name": "A", "score": 4.5 })),
        ("a number sent as digits", json!({ "name": "B", "score": "12" })),
        ("a boolean", json!({ "name": "C", "active": true })),
        ("a boolean sent as a word", json!({ "name": "D", "active": "true" })),
        ("nothing", json!({ "name": "E", "score": null, "active": null })),
    ] {
        let (s, body) = create(&f, data).await;
        assert_eq!(s, StatusCode::CREATED, "{what}: {body}");
    }
    // Clearing a field with null is allowed on update.
    let (id, version) = made(&f).await;
    let (s, body) = send(&f.app, "PATCH", &format!("/api/v1/apps/{}/records/{id}", f.slug), Some(json!({ "data": { "name": "Ada", "score": null }, "version": version }))).await;
    assert!(s.is_success(), "{body}");
}

#[tokio::test]
async fn an_export_format_that_does_not_exist_is_refused() {
    let f = fixture().await;
    for format in ["pdf", "docx", ""] {
        let r = call(&f.app, "export_records", json!({ "app_slug": f.slug, "table_id": "people", "format": format })).await;
        assert_eq!((r["isError"].clone(), r["structuredContent"]["code"].clone()), (json!(true), json!("bad_request")), "format '{format}': {r}");
    }
    for format in ["csv", "xlsx", "json", "jsonl"] {
        let r = call(&f.app, "export_records", json!({ "app_slug": f.slug, "table_id": "people", "format": format })).await;
        assert_ne!(r["isError"], json!(true), "format {format} is allowed: {r}");
    }
}
