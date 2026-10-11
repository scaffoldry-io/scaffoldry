//! Row scale phase 3: computed values are written, not recomputed.
//!
//! Each test uses a private app with a fresh slug and deletes its rows when it ends.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::Router;
use http_body_util::BodyExt;
use scaffoldry_engine::{AppManifest, AppTable, FieldSpec, FieldType, LinkCardinality};
use scaffoldry_server::build_app_with_state;
use scaffoldry_server::repository::page_statements;
use scaffoldry_server::repository_links::lookup_statements;
use scaffoldry_server::service::identity::issue_test_token_and_user;
use scaffoldry_server::state::ServerState;
use serde_json::{json, Value};
use std::sync::Arc;
use tokio::sync::Mutex;
use tower::ServiceExt;
use uuid::Uuid;

static LOCK: Mutex<()> = Mutex::const_new(());

const OWNER: &str = "prof.curie@science.state.edu";
const WORKSPACE: &str = "ws-bio-lab";

fn db_url() -> String {
    std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://scaffoldry:scaffoldry_dev_password@127.0.0.1:5433/scaffoldry".to_string())
}

fn db<R: Send + 'static>(f: impl FnOnce(&mut postgres::Client) -> R + Send + 'static) -> R {
    let url = db_url();
    std::thread::spawn(move || {
        let mut c = postgres::Client::connect(&url, postgres::NoTls).expect("database");
        f(&mut c)
    })
    .join()
    .expect("database thread")
}

struct Cleanup(String);
impl Drop for Cleanup {
    fn drop(&mut self) {
        let slug = self.0.clone();
        let url = db_url();
        let _ = std::thread::spawn(move || {
            if let Ok(mut c) = postgres::Client::connect(&url, postgres::NoTls) {
                let _ = c.execute("DELETE FROM record_links WHERE app_slug = $1", &[&slug]);
                let _ = c.execute("DELETE FROM dataset_records WHERE app_slug = $1", &[&slug]);
            }
        })
        .join();
    }
}

fn text(name: &str) -> FieldSpec {
    FieldSpec::simple(name, name, FieldType::Text, false, false)
}

fn num(name: &str) -> FieldSpec {
    FieldSpec::simple(name, name, FieldType::Number, false, false)
}

fn link(name: &str, target: &str, cardinality: LinkCardinality, related: &str) -> FieldSpec {
    let mut f = FieldSpec::simple(name, name, FieldType::Link, false, false);
    f.link_target_table = Some(target.to_string());
    f.link_cardinality = cardinality;
    f.link_related_field = Some(related.to_string());
    f
}

fn computed(name: &str, t: FieldType, via: &str, target: Option<&str>, function: Option<&str>) -> FieldSpec {
    let mut f = FieldSpec::simple(name, name, t, false, false);
    f.computed_via = Some(via.to_string());
    f.target_display_field = target.map(String::from);
    f.rollup_function = function.map(String::from);
    f
}

fn table(id: &str, fields: Vec<FieldSpec>, primary: &str) -> AppTable {
    AppTable {
        id: id.to_string(),
        name: id.to_string(),
        slug: id.to_string(),
        description: None,
        icon: None,
        fields,
        primary_field: Some(primary.to_string()),
        sample_records: vec![],
    }
}

/// `budgets` has `budget`, `spent`, and `remaining = {budget} - {spent}`. `parents` and `children` mirror each
/// other: a parent rolls up its children's `amount`, and a child looks up its parent's `name`, and has a formula
/// over that lookup.
fn manifest(slug: &str) -> AppManifest {
    let mut remaining = FieldSpec::simple("remaining", "remaining", FieldType::Formula, false, false);
    remaining.formula_expression = Some("{budget} - {spent}".to_string());
    let mut shout = FieldSpec::simple("parent_label", "parent_label", FieldType::Formula, false, false);
    shout.formula_expression = Some("{parent_name} + \"!\"".to_string());
    AppManifest {
        slug: slug.to_string(),
        title: "Written".to_string(),
        description: String::new(),
        organization_code: "BIO".to_string(),
        department: "biology".to_string(),
        workspace_id: Some(WORKSPACE.to_string()),
        herm_capability_id: None,
        custom_domain: None,
        custom_domain_verified: false,
        tables: vec![
            table("budgets", vec![text("name"), num("budget"), num("spent"), remaining], "name"),
            table(
                "parents",
                vec![
                    text("name"),
                    link("children", "children", LinkCardinality::Many, "parent"),
                    computed("total", FieldType::Rollup, "children", Some("amount"), Some("sum")),
                ],
                "name",
            ),
            table(
                "children",
                vec![
                    text("title"),
                    num("amount"),
                    link("parent", "parents", LinkCardinality::One, "children"),
                    computed("parent_name", FieldType::Lookup, "parent", Some("name"), None),
                    shout,
                ],
                "title",
            ),
        ],
        relationships: vec![],
        views: vec![],
        ceds_mappings: Default::default(),
        pages: vec![],
        access: Default::default(),
    }
}

struct World {
    app: Router,
    token: String,
    slug: String,
    _cleanup: Cleanup,
}

async fn world() -> World {
    let state = Arc::new(ServerState::new().expect("state"));
    state.seed_demo().expect("seed");
    let slug = format!("lnk-{}", &Uuid::new_v4().to_string()[..8]);
    state.engine.write().unwrap().register_manifest(manifest(&slug)).unwrap();
    World {
        app: build_app_with_state(state.clone()).expect("router"),
        token: issue_test_token_and_user(OWNER),
        _cleanup: Cleanup(slug.clone()),
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

async fn try_create(w: &World, table: &str, mut data: Value) -> (StatusCode, Value) {
    data["_table_id"] = json!(table);
    send(&w.app, "POST", &format!("/api/v1/apps/{}/records", w.slug), &w.token, Some(json!({ "data": data }))).await
}

async fn create(w: &World, table: &str, data: Value) -> Value {
    let (s, body) = try_create(w, table, data).await;
    assert_eq!(s, StatusCode::CREATED, "{body}");
    body
}

async fn try_patch(w: &World, id: &str, version: i64, data: Value) -> (StatusCode, Value) {
    send(&w.app, "PATCH", &format!("/api/v1/apps/{}/records/{id}", w.slug), &w.token, Some(json!({ "data": data, "version": version }))).await
}

async fn patch(w: &World, id: &str, version: i64, data: Value) -> Value {
    let (s, body) = try_patch(w, id, version, data).await;
    assert_eq!(s, StatusCode::OK, "{body}");
    body
}

fn id_of(v: &Value) -> String {
    v["id"].as_str().unwrap().to_string()
}

async fn get(w: &World, id: &str) -> Value {
    let (s, body) = send(&w.app, "GET", &format!("/api/v1/apps/{}/records/{id}", w.slug), &w.token, None).await;
    assert_eq!(s, StatusCode::OK, "{body}");
    body
}



async fn stored(w: &World, id: &str, field: &str) -> Value {
    let (slug, id, field) = (w.slug.clone(), id.to_string(), field.to_string());
    db(move |c| {
        let row = c.query_one("SELECT data -> $3::text FROM dataset_records WHERE app_slug = $1::text AND id = $2::text", &[&slug, &id, &field]).unwrap();
        row.get::<_, Option<Value>>(0).unwrap_or(Value::Null)
    })
}

fn as_f64(v: &Value) -> f64 {
    v.as_f64().unwrap_or(f64::NAN)
}

// 1. A formula is stored when the record is written, and stored again when it changes.
#[tokio::test]
async fn a_formula_is_written_into_the_record_and_follows_an_update() {
    let _turn = LOCK.lock().await;
    let w = world().await;
    let b = create(&w, "budgets", json!({ "name": "Grant", "budget": 100, "spent": 30 })).await;
    let id = id_of(&b);
    assert_eq!(as_f64(&stored(&w, &id, "remaining").await), 70.0, "the stored data holds the result");

    patch(&w, &id, 1, json!({ "name": "Grant", "budget": 100, "spent": 40 })).await;
    assert_eq!(as_f64(&stored(&w, &id, "remaining").await), 60.0, "and the stored result changes with spent");
}

// 2. A parent's stored rollup changes with its children, and its version does not.
#[tokio::test]
async fn a_rollup_is_stored_in_the_parent_and_the_parents_version_does_not_move() {
    let _turn = LOCK.lock().await;
    let w = world().await;
    let p1 = id_of(&create(&w, "parents", json!({ "name": "P1" })).await);
    let p2 = id_of(&create(&w, "parents", json!({ "name": "P2" })).await);
    let child = id_of(&create(&w, "children", json!({ "title": "a", "amount": 10, "parent": p1 })).await);
    assert_eq!(as_f64(&stored(&w, &p1, "total").await), 10.0, "adding a child raised the stored value");
    assert_eq!(get(&w, &p1).await["version"], 1, "and did not raise the version");

    let c = get(&w, &child).await;
    patch(&w, &child, c["version"].as_i64().unwrap(), json!({ "title": "a", "amount": 10, "parent": p2 })).await;
    assert_eq!(as_f64(&stored(&w, &p1, "total").await), 0.0, "moved away");
    assert_eq!(as_f64(&stored(&w, &p2, "total").await), 10.0, "moved to");
    assert_eq!(get(&w, &p2).await["version"], 1);

    let (status, _) = send(&w.app, "DELETE", &format!("/api/v1/apps/{}/records/{child}", w.slug), &w.token, None).await;
    assert!(status.is_success());
    assert_eq!(as_f64(&stored(&w, &p2, "total").await), 0.0, "deleted");
}

// 3. A page of 200 rows with one lookup runs two statements: the page and the lookup.
#[tokio::test]
async fn a_page_of_200_rows_with_a_lookup_runs_exactly_two_statements() {
    let _turn = LOCK.lock().await;
    let w = world().await;
    let p = id_of(&create(&w, "parents", json!({ "name": "Dean" })).await);
    for i in 0..200 {
        create(&w, "children", json!({ "title": format!("c{i}"), "amount": i, "parent": p })).await;
    }
    let (page_before, lookup_before) = (page_statements(), lookup_statements());
    let payload = json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/call",
        "params": { "name": "list_records", "arguments": { "app_slug": w.slug, "table_id": "children", "limit": 200 } } });
    let (_, v) = send(&w.app, "POST", "/api/mcp", &w.token, Some(payload)).await;
    let rows = v["result"]["structuredContent"]["records"].as_array().cloned().unwrap_or_default();
    assert_eq!(rows.len(), 200);
    assert!(rows.iter().all(|r| r["data"]["parent_name"] == "Dean"));
    let (pages, lookups) = (page_statements() - page_before, lookup_statements() - lookup_before);
    assert_eq!((pages, lookups), (1, 1), "one statement for the page and one for the lookup");

    // A formula over a lookup is not stored. It is added to the rows of the page.
    assert!(rows.iter().all(|r| r["data"]["parent_label"] == "Dean!"), "{}", rows[0]);
    assert_eq!(stored(&w, &id_of(&rows[0]), "parent_label").await, Value::Null, "and nowhere else");
}

// 4. A value a caller sends for a computed field is replaced.
#[tokio::test]
async fn a_value_sent_for_a_computed_field_is_replaced_by_the_computed_one() {
    let _turn = LOCK.lock().await;
    let w = world().await;
    let b = create(&w, "budgets", json!({ "name": "Grant", "budget": 100, "spent": 30, "remaining": 999_999 })).await;
    let id = id_of(&b);
    assert_eq!(as_f64(&stored(&w, &id, "remaining").await), 70.0, "a created record");
    let saved = patch(&w, &id, 1, json!({ "name": "Grant", "budget": 100, "spent": 30, "remaining": -5 })).await;
    assert_eq!(as_f64(&saved["data"]["remaining"]), 70.0, "an updated record");

    let p = id_of(&create(&w, "parents", json!({ "name": "P", "total": 12345 })).await);
    assert_eq!(as_f64(&stored(&w, &p, "total").await), 0.0, "a rollup too");
}
