//! Access rules phase 4: column rules, and denials that explain themselves.
//!
//! `gpa` is hidden from everyone but an admin. `name` is read-only for everyone but an admin.
//! `awards.amount` is hidden the same way, so a lookup of it must not carry it either.

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::Router;
use http_body_util::BodyExt;
use scaffoldry_core::access::{ColumnMode, ColumnRule, Who};
use scaffoldry_engine::{AppManifest, AppTable, FieldSpec, FieldType, LinkCardinality};
use scaffoldry_server::build_app_with_state;
use scaffoldry_server::service::identity::issue_test_token_and_user;
use scaffoldry_server::service::scope::set_column_rules;
use scaffoldry_server::state::{CollaboratorRecord, ServerState};
use serde_json::{json, Value};
use std::sync::Arc;
use tower::ServiceExt;
use uuid::Uuid;

const WORKSPACE: &str = "ws-bio-lab";
const ADMIN: &str = "jordan.lee@state.edu";
const MESSAGE: &str = "Financial details are visible to the scholarship office only.";

fn db_url() -> String {
    scaffoldry_server::testing::database_url()
}

struct Cleanup(String);
impl Drop for Cleanup {
    fn drop(&mut self) {
        let (tag, url) = (self.0.clone(), db_url());
        let _ = std::thread::spawn(move || {
            if let Ok(mut c) = postgres::Client::connect(&url, postgres::NoTls) {
                let like = format!("%{tag}%");
                let _ = c.execute("DELETE FROM workspace_collaborators WHERE workspace_id = $1 AND eppn LIKE $2", &[&WORKSPACE, &like]);
                let _ = c.execute("DELETE FROM record_links WHERE app_slug LIKE $1", &[&like]);
                let _ = c.execute("DELETE FROM record_history WHERE app_slug LIKE $1", &[&like]);
                let _ = c.execute("DELETE FROM dataset_records WHERE app_slug LIKE $1", &[&like]);
            }
        })
        .join();
    }
}

fn text(name: &str) -> FieldSpec {
    FieldSpec::simple(name, name, FieldType::Text, false, false)
}

fn table(id: &str, fields: Vec<FieldSpec>, primary: &str) -> AppTable {
    AppTable {
        id: id.into(),
        name: id.into(),
        slug: id.into(),
        description: None,
        icon: None,
        fields,
        primary_field: Some(primary.into()),
        sample_records: vec![],
        coordinates: None,
        row_place: Default::default(),
    }
}

fn manifest(slug: &str) -> AppManifest {
    let mut award = FieldSpec::simple("award", "award", FieldType::Link, false, false);
    award.link_target_table = Some("awards".into());
    award.link_cardinality = LinkCardinality::One;
    let mut award_amount = FieldSpec::simple("award_amount", "award_amount", FieldType::Lookup, false, false);
    award_amount.computed_via = Some("award".into());
    award_amount.target_display_field = Some("amount".into());
    AppManifest {
        slug: slug.into(),
        title: "Columns".into(),
        description: String::new(),
        organization_code: "BIO".into(),
        department: "biology".into(),
        workspace_id: Some(WORKSPACE.into()),
        herm_capability_id: None,
        coordinates: None,
        custom_domain: None,
        custom_domain_verified: false,
        tables: vec![
            table("students", vec![text("name"), FieldSpec::simple("gpa", "gpa", FieldType::Number, false, false), award, award_amount], "name"),
            table("awards", vec![text("name"), FieldSpec::simple("amount", "amount", FieldType::Number, false, false)], "name"),
        ],
        relationships: vec![],
        views: vec![],
        ceds_mappings: Default::default(),
        pages: vec![],
        access: Default::default(),
    }
}

fn rules() -> Vec<ColumnRule> {
    let admins = || Who { roles: vec!["admin".into(), "owner".into()], ..Default::default() };
    vec![
        ColumnRule { id: "gpa".into(), table_id: "students".into(), fields: vec!["gpa".into()], mode: ColumnMode::Hidden, unless: admins(), message: Some(MESSAGE.into()) },
        ColumnRule { id: "name".into(), table_id: "students".into(), fields: vec!["name".into()], mode: ColumnMode::ReadOnly, unless: admins(), message: None },
        ColumnRule { id: "amount".into(), table_id: "awards".into(), fields: vec!["amount".into()], mode: ColumnMode::Hidden, unless: admins(), message: None },
    ]
}

struct World {
    app: Router,
    slug: String,
    ann: String,
    olga: String,
    admin: String,
    _cleanup: Cleanup,
}

fn member(state: &ServerState, eppn: &str, role: &str) {
    state.collaborators.write().unwrap().entry(WORKSPACE.to_string()).or_default().push(CollaboratorRecord {
        id: Uuid::new_v4().to_string(),
        workspace_id: WORKSPACE.to_string(),
        eppn: eppn.to_string(),
        name: eppn.to_string(),
        role: role.to_string(),
        scoped_affiliation: "faculty".to_string(),
        department: "biology".to_string(),
        added_at: String::new(),
        app_slug: String::new(),
    });
}

async fn world() -> World {
    world_with(rules()).await
}

async fn world_with(rules: Vec<ColumnRule>) -> World {
    let state = Arc::new(ServerState::new().expect("state"));
    state.seed_demo().expect("seed");
    let tag = Uuid::new_v4().to_string()[..8].to_string();
    let slug = format!("col-{tag}");
    let (ann, olga) = (format!("ann-{tag}@state.edu"), format!("olga-{tag}@state.edu"));
    state.engine.write().unwrap().register_manifest(manifest(&slug)).unwrap();
    member(&state, &ann, "editor");
    member(&state, &olga, "admin");
    set_column_rules(&slug, rules, &state);
    World {
        app: build_app_with_state(state).expect("router"),
        ann: issue_test_token_and_user(&ann),
        olga: issue_test_token_and_user(&olga),
        admin: issue_test_token_and_user(ADMIN),
        _cleanup: Cleanup(tag),
        slug,
    }
}

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

/// Calls an MCP tool and returns the whole result, so a coded error can be read.
async fn tool(w: &World, token: &str, name: &str, args: Value) -> Value {
    let payload = json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": { "name": name, "arguments": args } });
    let resp = w
        .app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/mcp")
                .header("authorization", format!("Bearer {token}"))
                .header("content-type", "application/json")
                .body(Body::from(payload.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    serde_json::from_slice::<Value>(&bytes).unwrap()["result"].clone()
}

async fn create(w: &World, table: &str, data: Value) -> Value {
    let mut data = data;
    data["_table_id"] = json!(table);
    let (s, body) = send(&w.app, "POST", &format!("/api/v1/apps/{}/records", w.slug), &w.admin, Some(json!({ "data": data }))).await;
    assert_eq!(s, StatusCode::CREATED, "{body}");
    body
}

/// An award worth 5,000 and a student with a 3.9 who holds it.
async fn student(w: &World) -> (String, String) {
    let award = create(w, "awards", json!({ "name": "Merit", "amount": 5000 })).await["id"].as_str().unwrap().to_string();
    let student = create(w, "students", json!({ "name": "Sam", "gpa": 3.9, "award": award })).await["id"].as_str().unwrap().to_string();
    (student, award)
}

// 1. A hidden field is absent from list, get, history, and the field list, for a restricted person.
//    It is present for a person in `unless` and for a Platform Admin.
#[tokio::test]
async fn a_hidden_field_is_absent_for_a_restricted_person_and_present_for_the_exempt() {
    let _db = common::TestDb::new();
    let w = world().await;
    let (student, _) = student(&w).await;
    let base = format!("/api/v1/apps/{}", w.slug);
    let (s, _) = send(&w.app, "PATCH", &format!("{base}/records/{student}"), &w.admin, Some(json!({ "data": { "name": "Sam", "gpa": 4.0 }, "version": 1 }))).await;
    assert_eq!(s, StatusCode::OK);

    for (who, token, sees) in [("ann", &w.ann, false), ("olga", &w.olga, true), ("admin", &w.admin, true)] {
        let (_, one) = send(&w.app, "GET", &format!("{base}/records/{student}"), token, None).await;
        assert_eq!(one["data"].get("gpa").is_some(), sees, "{who} get: {one}");
        let (_, list) = send(&w.app, "GET", &format!("{base}/tables/students/records"), token, None).await;
        assert_eq!(list["records"][0]["data"].get("gpa").is_some(), sees, "{who} list");
        let (_, hist) = send(&w.app, "GET", &format!("{base}/records/{student}/history"), token, None).await;
        let changes = &hist["entries"][0]["changes"];
        let old_or_new = changes["gpa"].get("new").is_some();
        assert_eq!(old_or_new, sees, "{who} history: {changes}");
        assert!(sees || changes["gpa"]["changed"] == true || changes.get("gpa").is_none(), "{who}: masked, not shown: {changes}");
        let (_, schema) = send(&w.app, "GET", &format!("{base}/schema"), token, None).await;
        let in_schema = schema["tables"].as_array().unwrap().iter().filter(|t| t["id"] == "students").any(|t| t["fields"].as_array().unwrap().iter().any(|f| f["name"] == "gpa"));
        assert_eq!(in_schema, sees, "{who} schema");
        let described = tool(&w, token, "describe_app", json!({ "app_slug": w.slug })).await;
        let in_described = described["structuredContent"]["tables"].as_array().unwrap().iter().filter(|t| t["id"] == "students").any(|t| t["fields"].as_array().unwrap().iter().any(|f| f["name"] == "gpa"));
        assert_eq!(in_described, sees, "{who} describe_app");
    }
}

// 2. A filter or sort on a hidden field is field_hidden, with a reason and a remedy.
#[tokio::test]
async fn a_filter_or_sort_on_a_hidden_field_is_refused() {
    let _db = common::TestDb::new();
    let w = world().await;
    student(&w).await;
    let probe = json!({ "conjunction": "And", "clauses": [{ "id": "c", "field_name": "gpa", "operator": "GreaterThan", "value": "3" }] });
    let r = tool(&w, &w.ann, "list_records", json!({ "app_slug": w.slug, "table_id": "students", "filter": probe })).await;
    assert_eq!(r["isError"], true, "{r}");
    assert_eq!(r["structuredContent"]["code"], "field_hidden", "{r}");
    assert!(r["structuredContent"]["reason"].as_str().is_some_and(|s| !s.is_empty()), "{r}");
    let r = tool(&w, &w.ann, "list_records", json!({ "app_slug": w.slug, "table_id": "students", "sort": { "field_name": "gpa", "direction": "Desc" } })).await;
    assert_eq!(r["structuredContent"]["code"], "field_hidden", "{r}");
    let r = tool(&w, &w.olga, "list_records", json!({ "app_slug": w.slug, "table_id": "students", "sort": { "field_name": "gpa", "direction": "Desc" } })).await;
    assert_ne!(r["isError"], true, "a person in unless may sort: {r}");
}

// 3. A write to a read-only field is field_read_only and names it.
#[tokio::test]
async fn a_write_to_a_read_only_field_is_refused_naming_it() {
    let _db = common::TestDb::new();
    let w = world().await;
    let (student, _) = student(&w).await;
    let base = format!("/api/v1/apps/{}/records/{student}", w.slug);
    let (s, body) = send(&w.app, "PATCH", &base, &w.ann, Some(json!({ "data": { "name": "Renamed" }, "version": 1 }))).await;
    assert_eq!(s, StatusCode::FORBIDDEN, "{body}");
    assert_eq!(body["code"], "field_read_only", "{body}");
    assert_eq!(body["fields"], json!({ "name": "read_only" }), "{body}");
    assert!(body["error"].as_str().unwrap().contains("name"), "{body}");
    // Sending the field back unchanged is not a write to it.
    let (_, current) = send(&w.app, "GET", &base, &w.ann, None).await;
    let (s, body) = send(&w.app, "PATCH", &base, &w.ann, Some(json!({ "data": { "name": current["data"]["name"] }, "version": current["version"] }))).await;
    assert_eq!(s, StatusCode::OK, "{body}");
    // Creating a record that sets it is refused too.
    let (s, body) = send(&w.app, "POST", &format!("/api/v1/apps/{}/records", w.slug), &w.ann, Some(json!({ "data": { "_table_id": "students", "name": "New" } }))).await;
    assert_eq!(body["code"], "field_read_only", "{s} {body}");
    // A person in unless can.
    let (s, body) = send(&w.app, "PATCH", &base, &w.olga, Some(json!({ "data": { "name": "Renamed", "gpa": 3.9 }, "version": 2 }))).await;
    assert!(s.is_success(), "{s} {body}");
}

// 4. A rule's message is the reason. A generated remedy names who can help.
#[tokio::test]
async fn the_message_is_the_reason_and_the_remedy_names_who_to_ask() {
    let _db = common::TestDb::new();
    let w = world().await;
    let probe = json!({ "conjunction": "And", "clauses": [{ "id": "c", "field_name": "gpa", "operator": "GreaterThan", "value": "3" }] });
    let r = tool(&w, &w.ann, "list_records", json!({ "app_slug": w.slug, "table_id": "students", "filter": probe })).await;
    assert_eq!(r["structuredContent"]["reason"], MESSAGE, "{r}");
    let remedy = r["structuredContent"]["remedy"].as_str().unwrap();
    assert!(remedy.contains("admin or owner role") && remedy.contains("owner of this workspace"), "{remedy}");
    // A rule with no message gets a generated sentence.
    let (student, _) = student(&w).await;
    let (_, body) = send(&w.app, "PATCH", &format!("/api/v1/apps/{}/records/{student}", w.slug), &w.ann, Some(json!({ "data": { "name": "Renamed" }, "version": 1 }))).await;
    assert!(body["reason"].as_str().unwrap().contains("'name'"), "{body}");
    assert!(body["remedy"].as_str().unwrap().starts_with("Ask "), "{body}");
    // A row outside scope stays "not found" with no reason: not a field denial at all.
    let (s, body) = send(&w.app, "GET", &format!("/api/v1/apps/{}/records/{}", w.slug, "no-such"), &w.ann, None).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    assert!(body.get("reason").is_none() && body.get("remedy").is_none(), "{body}");
}

// 5. A lookup through a link never returns a hidden field of the target table.
#[tokio::test]
async fn a_lookup_never_returns_a_hidden_field_of_the_target() {
    let _db = common::TestDb::new();
    let w = world().await;
    student(&w).await;
    // Lookups are filled when a page is read.
    let list = format!("/api/v1/apps/{}/tables/students/records", w.slug);
    let (_, ann) = send(&w.app, "GET", &list, &w.ann, None).await;
    assert_eq!(ann["records"].as_array().unwrap().len(), 1);
    assert!(ann["records"][0]["data"].get("award_amount").is_none_or(|v| v.is_null()), "{ann}");
    assert!(!ann.to_string().contains("5000"), "{ann}");
    let (_, olga) = send(&w.app, "GET", &list, &w.olga, None).await;
    assert_eq!(olga["records"][0]["data"]["award_amount"], 5000, "{olga}");
}

// 6. A Platform Admin always sees everything, even when the rule excepts no one.
#[tokio::test]
async fn a_platform_admin_is_never_held_back() {
    let _db = common::TestDb::new();
    let none = vec![ColumnRule { id: "all".into(), table_id: "students".into(), fields: vec!["gpa".into(), "name".into()], mode: ColumnMode::Hidden, unless: Who::default(), message: None }];
    let w = world_with(none).await;
    let (student, _) = student(&w).await;
    let (_, one) = send(&w.app, "GET", &format!("/api/v1/apps/{}/records/{student}", w.slug), &w.admin, None).await;
    assert!(one["data"].get("gpa").is_some() && one["data"].get("name").is_some(), "{one}");
    let (_, ann) = send(&w.app, "GET", &format!("/api/v1/apps/{}/records/{student}", w.slug), &w.ann, None).await;
    assert!(ann["data"].get("gpa").is_none() && ann["data"].get("name").is_none(), "an editor is held back: {ann}");
    let (_, olga) = send(&w.app, "GET", &format!("/api/v1/apps/{}/records/{student}", w.slug), &w.olga, None).await;
    assert!(olga["data"].get("gpa").is_none(), "an empty `unless` excepts no workspace role: {olga}");
    let (s, _) = send(&w.app, "PATCH", &format!("/api/v1/apps/{}/records/{student}", w.slug), &w.admin, Some(json!({ "data": { "name": "Changed", "gpa": 2.0 }, "version": 1 }))).await;
    assert_eq!(s, StatusCode::OK, "a Platform Admin may change what a rule hides from others");
}

// The rules serialize in the shape the published schema describes.
#[test]
fn a_column_rule_round_trips_in_the_schema_shape() {
    let _db = common::TestDb::new();
    let schema: Value = serde_json::from_str(&std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../governance/schema/access-column-rules.schema.json")).unwrap()).unwrap();
    let required = schema["items"]["required"].as_array().unwrap();
    for rule in rules() {
        let json = serde_json::to_value(&rule).unwrap();
        for key in required {
            assert!(json.get(key.as_str().unwrap()).is_some(), "missing {key}");
        }
        let modes = schema["items"]["properties"]["mode"]["enum"].as_array().unwrap();
        assert!(modes.contains(&json["mode"]), "{}", json["mode"]);
        assert_eq!(serde_json::from_value::<ColumnRule>(json).unwrap(), rule);
    }
}
