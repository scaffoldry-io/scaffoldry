//! Integrations phase 3: an API description for each app.
//!
//! `GET /api/v1/apps/{slug}/openapi.json` returns an OpenAPI 3.1 document for the caller, with one JSON Schema per
//! table. A field hidden from the caller is not in it. The MCP resource `scaffoldry://apps/{slug}/openapi` returns the
//! same document.

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::Router;
use http_body_util::BodyExt;
use scaffoldry_server::build_app_with_state;
use scaffoldry_server::service::identity::issue_test_token_and_user;
use serde_json::{json, Value};
use tower::ServiceExt;

const WORKSPACE: &str = "ws-physics-optics";
const OWNER: &str = "einstein@physics.state.edu";
const PLATFORM_ADMIN: &str = "jordan.lee@state.edu";

struct World {
    _db: common::TestDb,
    app: Router,
    owner: String,
    admin: String,
    slug: String,
}

struct Reply {
    status: StatusCode,
    etag: Option<String>,
    body: Value,
}

async fn get(app: &Router, uri: &str, token: &str, if_none_match: Option<&str>) -> Reply {
    let mut b = Request::builder().method("GET").uri(uri).header("authorization", format!("Bearer {token}"));
    if let Some(tag) = if_none_match {
        b = b.header("if-none-match", tag);
    }
    let resp = app.clone().oneshot(b.body(Body::empty()).unwrap()).await.unwrap();
    let status = resp.status();
    let etag = resp.headers().get("etag").and_then(|v| v.to_str().ok()).map(str::to_string);
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    Reply { status, etag, body: serde_json::from_slice(&bytes).unwrap_or(Value::Null) }
}

async fn rpc(w: &World, token: &str, method: &str, params: Value) -> Value {
    let payload = json!({ "jsonrpc": "2.0", "id": 1, "method": method, "params": params });
    let b = Request::builder().method("POST").uri("/api/mcp").header("authorization", format!("Bearer {token}")).header("content-type", "application/json");
    let resp = w.app.clone().oneshot(b.body(Body::from(payload.to_string())).unwrap()).await.unwrap();
    serde_json::from_slice(&resp.into_body().collect().await.unwrap().to_bytes()).unwrap_or(Value::Null)
}

async fn tool(w: &World, token: &str, name: &str, args: Value) -> Value {
    let v = rpc(w, token, "tools/call", json!({ "name": name, "arguments": args })).await;
    assert!(v["result"]["isError"] != true, "{name}: {v}");
    v["result"]["structuredContent"].clone()
}

fn field(name: &str, kind: &str) -> Value {
    json!({ "name": name, "label": name, "field_type": kind, "required": false, "ferpa_sensitive": false })
}

fn manifest(slug: &str, extra: &[Value]) -> Value {
    let mut status = field("status", "Select");
    status["select_options"] = json!([{ "id": "open", "label": "Open" }, { "id": "closed", "label": "Closed" }]);
    let mut tags = field("tags", "MultiSelect");
    tags["select_options"] = json!([{ "id": "a", "label": "Alpha" }, { "id": "b", "label": "Beta" }]);
    let mut title = field("title", "Text");
    title["required"] = json!(true);
    let mut score = field("score", "Number");
    score["constraints"] = json!({ "min": 0.0, "max": 10.0 });
    let mut related = field("related", "Link");
    related["link_target_table"] = json!("notes");
    let mut fields = vec![title, status, tags, score, field("due", "Date"), field("done", "Boolean"), field("seq", "Autonumber"), field("secret", "Text"), related, field("files", "Attachment")];
    fields.extend(extra.iter().cloned());
    json!({
        "slug": slug, "title": "Open API", "description": "An app", "organization_code": "PHYS", "department": "physics",
        "workspace_id": WORKSPACE, "views": [], "pages": [],
        "tables": [
            { "id": "cases", "name": "Cases", "slug": "cases", "fields": fields },
            { "id": "notes", "name": "Notes", "slug": "notes", "fields": [field("body", "Text")] }
        ],
        "access": { "rows": [], "columns": [{ "id": "hide-secret", "table_id": "cases", "fields": ["secret"], "mode": "hidden" }] }
    })
}

async fn change(w: &World, extra: &[Value]) {
    let p = tool(w, &w.owner, "propose_app_change", json!({ "workspace_id": WORKSPACE, "manifest": manifest(&w.slug, extra), "summary": "A change" })).await;
    tool(w, &w.owner, "decide_proposal", json!({ "proposal_id": p["proposal_id"], "decision": "approve", "rationale": "Reviewed", "digest": p["digest"] })).await;
}

async fn world() -> World {
    let db = common::TestDb::new();
    let state = db.state();
    state.seed_demo().expect("seed");
    let app = build_app_with_state(state).expect("router");
    let w = World { _db: db, app, owner: issue_test_token_and_user(OWNER), admin: issue_test_token_and_user(PLATFORM_ADMIN), slug: format!("oa-{}", &uuid::Uuid::new_v4().to_string()[..8]) };
    change(&w, &[]).await;
    w
}

fn url(w: &World) -> String {
    format!("/api/v1/apps/{}/openapi.json", w.slug)
}

/// Whether `v` satisfies `schema`, for the parts of JSON Schema the description uses.
fn valid(root: &Value, schema: &Value, v: &Value) -> bool {
    if let Some(r) = schema["$ref"].as_str() {
        let target = r.trim_start_matches("#/").split('/').fold(root, |node, key| &node[key]);
        return valid(root, target, v);
    }
    if let Some(options) = schema["enum"].as_array() {
        if !options.contains(v) {
            return false;
        }
    }
    let type_ok = |t: &str| match t {
        "string" => v.is_string(),
        "number" => v.is_number(),
        "integer" => v.is_i64() || v.is_u64(),
        "boolean" => v.is_boolean(),
        "array" => v.is_array(),
        "object" => v.is_object(),
        "null" => v.is_null(),
        _ => true,
    };
    match &schema["type"] {
        Value::String(t) if !type_ok(t) => return false,
        Value::Array(ts) if !ts.iter().filter_map(Value::as_str).any(type_ok) => return false,
        _ => {}
    }
    if let (Some(n), Some(min)) = (v.as_f64(), schema["minimum"].as_f64()) {
        if n < min {
            return false;
        }
    }
    if let (Some(n), Some(max)) = (v.as_f64(), schema["maximum"].as_f64()) {
        if n > max {
            return false;
        }
    }
    if let (Some(items), Some(item_schema)) = (v.as_array(), schema.get("items")) {
        if !items.iter().all(|i| valid(root, item_schema, i)) {
            return false;
        }
    }
    if let Some(obj) = v.as_object() {
        if let Some(required) = schema["required"].as_array() {
            if !required.iter().filter_map(Value::as_str).all(|k| obj.contains_key(k)) {
                return false;
            }
        }
        if let Some(props) = schema["properties"].as_object() {
            if schema["additionalProperties"] == false && !obj.keys().all(|k| props.contains_key(k)) {
                return false;
            }
            if !obj.iter().all(|(k, val)| props.get(k).is_none_or(|s| valid(root, s, val))) {
                return false;
            }
        }
    }
    true
}

// 1. The document for a fixture app has the expected paths and one schema per table.
#[tokio::test]
async fn the_document_has_the_paths_and_a_schema_for_each_table() {
    let w = world().await;
    let r = get(&w.app, &url(&w), &w.owner, None).await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.body);
    let doc = &r.body;
    assert_eq!(doc["openapi"], "3.1.0");
    assert_eq!(doc["components"]["securitySchemes"]["bearerAuth"]["scheme"], "bearer");
    assert_eq!(doc["security"], json!([{ "bearerAuth": [] }]));
    for table in ["cases", "notes"] {
        let base = format!("/apps/{}/tables/{table}", w.slug);
        let paths = &doc["paths"];
        assert!(paths[format!("{base}/records")]["get"].is_object() && paths[format!("{base}/records")]["post"].is_object(), "{table}: list and create");
        let one = &paths[format!("{base}/records/{{id}}")];
        assert!(one["get"].is_object() && one["patch"].is_object() && one["delete"].is_object(), "{table}: get, change, delete");
        assert!(paths[format!("{base}/aggregate")]["get"].is_object(), "{table}: aggregate");
        assert_eq!(doc["components"]["schemas"][table]["type"], "object", "{table}: one schema");
    }
    let status = &doc["components"]["schemas"]["cases"]["properties"]["status"];
    assert_eq!(status["enum"], json!(["open", "closed"]));
    assert_eq!(status["x-labels"], json!({ "open": "Open", "closed": "Closed" }));
    assert_eq!(doc["components"]["schemas"]["cases"]["properties"]["tags"]["type"], "array");
    assert_eq!(doc["components"]["schemas"]["cases"]["properties"]["related"]["items"]["type"], "string");
    assert_eq!(doc["components"]["schemas"]["cases"]["properties"]["files"]["items"]["required"], json!(["ref_id", "name"]));
    assert_eq!(doc["components"]["schemas"]["cases"]["properties"]["due"]["format"], "date");

    let nobody = get(&w.app, &format!("/api/v1/apps/nope-{}/openapi.json", w.slug), &w.owner, None).await;
    assert_eq!(nobody.status, StatusCode::NOT_FOUND);
}

// 2. A sample record validates against its schema. A record with an unknown select id does not.
#[tokio::test]
async fn a_sample_record_validates_and_an_unknown_select_id_does_not() {
    let w = world().await;
    let doc = get(&w.app, &url(&w), &w.admin, None).await.body;
    let schema = &doc["components"]["schemas"]["cases"];
    let good = json!({ "title": "A case", "status": "open", "tags": ["a"], "score": 4.5, "due": "2026-10-23", "done": false, "related": ["n1"], "files": [{ "ref_id": "r1", "name": "a.pdf" }] });
    assert!(valid(&doc, schema, &good), "{good}");
    assert!(!valid(&doc, schema, &json!({ "title": "A case", "status": "pending" })), "an unknown select id");
    assert!(!valid(&doc, schema, &json!({ "title": "A case", "tags": ["z"] })), "an unknown multi-select id");
    assert!(!valid(&doc, schema, &json!({ "status": "open" })), "a required field is missing");
    assert!(!valid(&doc, schema, &json!({ "title": "A case", "score": 11 })), "above the maximum");
    assert!(!valid(&doc, schema, &json!({ "title": "A case", "invented": 1 })), "an unknown field");
}

// 3. A hidden field is absent for a restricted person and present for an admin.
#[tokio::test]
async fn a_hidden_field_is_absent_for_the_restricted_and_present_for_an_admin() {
    let w = world().await;
    let owner = get(&w.app, &url(&w), &w.owner, None).await.body;
    let admin = get(&w.app, &url(&w), &w.admin, None).await.body;
    assert!(owner["components"]["schemas"]["cases"]["properties"].get("secret").is_none(), "the document must not reveal it: {owner}");
    assert!(!owner.to_string().contains("secret"), "no trace of the hidden field");
    assert!(admin["components"]["schemas"]["cases"]["properties"].get("secret").is_some());
}

// 4. Computed fields are readOnly.
#[tokio::test]
async fn computed_fields_are_read_only() {
    let w = world().await;
    let doc = get(&w.app, &url(&w), &w.admin, None).await.body;
    let props = &doc["components"]["schemas"]["cases"]["properties"];
    assert_eq!(props["seq"]["readOnly"], true, "{props}");
    assert!(props["title"].get("readOnly").is_none());
}

// 5. If-None-Match returns 304 when nothing changed and 200 after a manifest change.
#[tokio::test]
async fn if_none_match_returns_304_until_the_manifest_changes() {
    let w = world().await;
    let first = get(&w.app, &url(&w), &w.owner, None).await;
    let tag = first.etag.clone().expect("an ETag");
    let again = get(&w.app, &url(&w), &w.owner, Some(&tag)).await;
    assert_eq!(again.status, StatusCode::NOT_MODIFIED);
    assert_eq!(again.etag.as_deref(), Some(tag.as_str()));
    assert_eq!(get(&w.app, &url(&w), &w.owner, Some("\"other\"")).await.status, StatusCode::OK);

    change(&w, &[field("extra", "Text")]).await;
    let after = get(&w.app, &url(&w), &w.owner, Some(&tag)).await;
    assert_eq!(after.status, StatusCode::OK, "the app changed");
    assert_ne!(after.etag.as_deref(), Some(tag.as_str()));
    assert!(after.body["components"]["schemas"]["cases"]["properties"].get("extra").is_some());

    // The tag belongs to what the caller sees. An admin's document differs from an owner's.
    let admin = get(&w.app, &url(&w), &w.admin, None).await;
    assert_ne!(admin.etag, after.etag);
}

// The MCP resource is the same document, and the template is listed.
#[tokio::test]
async fn the_mcp_resource_returns_the_same_document() {
    let w = world().await;
    let templates = rpc(&w, &w.owner, "resources/templates/list", json!({})).await;
    assert_eq!(templates["result"]["resourceTemplates"][0]["uriTemplate"], "scaffoldry://apps/{slug}/openapi");
    let uri = format!("scaffoldry://apps/{}/openapi", w.slug);
    let read = rpc(&w, &w.owner, "resources/read", json!({ "uri": uri })).await;
    let text = read["result"]["contents"][0]["text"].as_str().expect("the document");
    let over_mcp: Value = serde_json::from_str(text).unwrap();
    let over_rest = get(&w.app, &url(&w), &w.owner, None).await.body;
    assert_eq!(over_mcp, over_rest);
    let missing = rpc(&w, &w.owner, "resources/read", json!({ "uri": "scaffoldry://apps/not-an-app/openapi" })).await;
    assert_eq!(missing["result"]["isError"], true, "{missing}");
}

// The document for the fixture app is checked in at governance/fixtures/openapi/fixture.openapi.json. The CI gate
// `governance/scripts/validate-openapi.py` validates it, and the examples beside it, with the jsonschema package.
// This test keeps the file the same as what the server builds. To change it on purpose:
// `UPDATE_OPENAPI_FIXTURE=1 cargo test -p scaffoldry-server --test openapi_test the_checked_in_fixture`
#[tokio::test]
async fn the_checked_in_fixture_is_what_the_server_builds() {
    let db = common::TestDb::new();
    let state = db.state();
    state.seed_demo().expect("seed");
    let app = build_app_with_state(state).expect("router");
    let w = World { _db: db, app, owner: issue_test_token_and_user(OWNER), admin: issue_test_token_and_user(PLATFORM_ADMIN), slug: "fixture-app".to_string() };
    change(&w, &[]).await;
    let built = get(&w.app, &url(&w), &w.admin, None).await.body;
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../governance/fixtures/openapi/fixture.openapi.json");
    let text = serde_json::to_string_pretty(&built).unwrap() + "\n";
    if std::env::var("UPDATE_OPENAPI_FIXTURE").is_ok() {
        std::fs::write(&path, &text).unwrap();
    }
    let stored = std::fs::read_to_string(&path).expect("the fixture is checked in");
    assert_eq!(stored, text, "the checked-in fixture differs from the document the server builds. Regenerate it with UPDATE_OPENAPI_FIXTURE=1");
}
