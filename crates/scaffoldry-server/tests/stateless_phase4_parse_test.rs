//! Stateless server phase 4, test 4: with many apps, approving one makes another instance read one, not all.

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::Router;
use common::TestDb;
use http_body_util::BodyExt;
use scaffoldry_server::build_app_with_state;
use scaffoldry_server::service::identity::issue_test_token_and_user;
use serde_json::{json, Value};
use tower::ServiceExt;

const WORKSPACE: &str = "ws-physics-optics";
const OWNER: &str = "einstein@physics.state.edu";
const APPS: i64 = 500;

async fn send(app: &Router, method: &str, uri: &str, body: Option<Value>) -> (StatusCode, Value) {
    let token = issue_test_token_and_user(OWNER);
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

async fn tool(app: &Router, name: &str, args: Value) -> (bool, Value) {
    let payload = json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": { "name": name, "arguments": args } });
    let (_, v) = send(app, "POST", "/api/mcp", Some(payload)).await;
    let result = &v["result"];
    if result["isError"] == true {
        return (true, result["content"][0]["text"].clone());
    }
    (false, result["structuredContent"].clone())
}

fn manifest(slug: &str, extra: Option<&str>) -> Value {
    let mut fields = vec![
        json!({ "name": "name", "label": "name", "field_type": "Text", "required": false, "ferpa_sensitive": false }),
        json!({ "name": "status", "label": "status", "field_type": "Text", "required": false, "ferpa_sensitive": false }),
    ];
    if let Some(e) = extra {
        fields.push(json!({ "name": e, "label": e, "field_type": "Text", "required": false, "ferpa_sensitive": false }));
    }
    json!({
        "slug": slug, "title": "Bulk", "description": "d", "organization_code": "PHYS", "department": "physics",
        "workspace_id": WORKSPACE, "views": [],
        "tables": [{ "id": "apps", "name": "Applications", "slug": "apps", "fields": fields }]
    })
}

// 4. With 500 apps, approving one reloads one. The definitions parsed are counted.
#[tokio::test]
async fn approving_one_app_among_many_makes_another_instance_parse_one() {
    let db = TestDb::new();
    let state_a = db.state();
    state_a.seed_demo().expect("seed");

    // 500 stored apps, written the way a restore would write them.
    let template = manifest("template", None).to_string();
    db.run(move |c| {
        c.execute(
            "INSERT INTO app_manifests (slug, title, description, organization_code, department, workspace_id, manifest) \
             SELECT 'bulk-' || g, 'Bulk', 'd', 'PHYS', 'physics', 'ws-physics-optics', jsonb_set($1::text::jsonb, '{slug}', to_jsonb('bulk-' || g)) \
             FROM generate_series(1, 500) g",
            &[&template],
        )
        .unwrap();
        c.execute("UPDATE cache_versions SET version = version + 1 WHERE name = 'manifests'", &[]).unwrap();
    });

    let a = build_app_with_state(db.state()).expect("router");
    let state_b = db.state();
    let b = build_app_with_state(state_b.clone()).expect("router");
    let (s, body) = send(&b, "GET", "/api/v1/apps/bulk-250/schema", None).await;
    assert_eq!(s, StatusCode::OK, "B holds the stored apps: {body}");
    let _ = (&state_a, APPS);

    let (err, p) = tool(&a, "propose_app_change", json!({ "workspace_id": WORKSPACE, "manifest": manifest("bulk-250", Some("nickname")), "summary": "A field" })).await;
    assert!(!err, "{p}");
    let (err, done) = tool(&a, "decide_proposal", json!({ "proposal_id": p["proposal_id"], "decision": "approve", "rationale": "Reviewed", "digest": p["digest"] })).await;
    assert!(!err, "{done}");

    let before = state_b.repository.manifests_parsed();
    let (s, schema) = send(&b, "GET", "/api/v1/apps/bulk-250/schema", None).await;
    assert_eq!(s, StatusCode::OK, "{schema}");
    assert!(schema.to_string().contains("nickname"), "B shows the new field: {schema}");
    let parsed = state_b.repository.manifests_parsed() - before;
    assert_eq!(parsed, 1, "B read one definition, not {APPS}: it parsed {parsed}");
}
