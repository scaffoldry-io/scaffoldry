//! Links phase 3: lookup, rollup, and count go through a link field.
//!
//! Each test uses a private app with a fresh slug and deletes its rows when it ends.

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::Router;
use http_body_util::BodyExt;
use scaffoldry_engine::{AppManifest, AppTable, FieldSpec, FieldType, LinkCardinality};
use scaffoldry_server::build_app_with_state;
use scaffoldry_server::repository::{PostgresRepository, SqlParam};
use scaffoldry_server::repository_links::{lookup_statements, LOOKUP_SQL, ROLLUP_SUM_SQL};
use scaffoldry_server::service::identity::issue_test_token_and_user;
use scaffoldry_server::state::ServerState;
use serde_json::{json, Value};
use std::sync::Arc;
use tower::ServiceExt;
use uuid::Uuid;


const VIEWER: &str = "marcus.vance@state.edu";
const OWNER: &str = "prof.curie@science.state.edu";
const WORKSPACE: &str = "ws-bio-lab";

fn db_url() -> String {
    scaffoldry_server::testing::database_url()
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

/// `parents.children` and `children.parent` mirror each other. A parent rolls up its children's
/// `amount` and counts them. A child looks up its parent's `name` and its parent's `notes`, which is sensitive.
fn manifest(slug: &str) -> AppManifest {
    let mut notes = text("notes");
    notes.ferpa_sensitive = true;
    AppManifest {
        slug: slug.to_string(),
        title: "Computing".to_string(),
        description: String::new(),
        organization_code: "BIO".to_string(),
        department: "biology".to_string(),
        workspace_id: Some(WORKSPACE.to_string()),
        herm_capability_id: None,
        custom_domain: None,
        custom_domain_verified: false,
        tables: vec![
            table(
                "parents",
                vec![
                    text("name"),
                    notes,
                    link("children", "children", LinkCardinality::Many, "parent"),
                    computed("total", FieldType::Rollup, "children", Some("amount"), Some("sum")),
                    computed("n", FieldType::Count, "children", None, None),
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
                    computed("parent_notes", FieldType::Lookup, "parent", Some("notes"), None),
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

async fn delete(w: &World, id: &str) -> (StatusCode, Value) {
    send(&w.app, "DELETE", &format!("/api/v1/apps/{}/records/{id}", w.slug), &w.token, None).await
}

async fn field_of(w: &World, id: &str, field: &str) -> Value {
    get(w, id).await["data"].get(field).cloned().unwrap_or(Value::Null)
}

fn as_f64(v: &Value) -> f64 {
    v.as_f64().unwrap_or(f64::NAN)
}

// 1. A parent with three children shows the sum. Adding, changing, moving, and removing each updates it.
#[tokio::test]
async fn a_rollup_and_a_count_follow_every_change_to_the_children() {
    let _db = common::TestDb::new();
    let w = world().await;
    let p1 = id_of(&create(&w, "parents", json!({ "name": "P1" })).await);
    let p2 = id_of(&create(&w, "parents", json!({ "name": "P2" })).await);
    let mut kids = Vec::new();
    for (title, amount) in [("a", 10), ("b", 20), ("c", 30)] {
        kids.push(id_of(&create(&w, "children", json!({ "title": title, "amount": amount, "parent": p1 })).await));
    }
    assert_eq!(as_f64(&field_of(&w, &p1, "total").await), 60.0, "three children");
    assert_eq!(as_f64(&field_of(&w, &p1, "n").await), 3.0);
    let version = get(&w, &p1).await["version"].clone();

    // Change a child's amount.
    let c = get(&w, &kids[2]).await;
    patch(&w, &kids[2], c["version"].as_i64().unwrap(), json!({ "title": "c", "amount": 50, "parent": p1 })).await;
    assert_eq!(as_f64(&field_of(&w, &p1, "total").await), 80.0, "a child changed");

    // Move a child to the other parent.
    let c = get(&w, &kids[2]).await;
    patch(&w, &kids[2], c["version"].as_i64().unwrap(), json!({ "title": "c", "amount": 50, "parent": p2 })).await;
    assert_eq!(as_f64(&field_of(&w, &p1, "total").await), 30.0, "the old parent lost the child");
    assert_eq!(as_f64(&field_of(&w, &p2, "total").await), 50.0, "the new parent gained it");
    assert_eq!(as_f64(&field_of(&w, &p1, "n").await), 2.0);
    assert_eq!(as_f64(&field_of(&w, &p2, "n").await), 1.0);

    // Remove a child.
    let (status, _) = delete(&w, &kids[1]).await;
    assert!(status.is_success());
    assert_eq!(as_f64(&field_of(&w, &p1, "total").await), 10.0, "a child was removed");
    assert_eq!(as_f64(&field_of(&w, &p1, "n").await), 1.0);

    // Link a child that has no parent yet, from the parent's side.
    let loose = id_of(&create(&w, "children", json!({ "title": "d", "amount": 5 })).await);
    let parent = get(&w, &p1).await;
    patch(&w, &p1, parent["version"].as_i64().unwrap(), json!({ "name": "P1", "children": [kids[0].clone(), loose] })).await;
    assert_eq!(as_f64(&field_of(&w, &p1, "total").await), 15.0, "linking from the parent's side");

    // The computed changes were not user edits: P2 still has the version it was made with, plus nothing.
    assert_eq!(get(&w, &p2).await["version"], 1);
    let _ = version;
}

async fn list(w: &World, token: &str, limit: usize) -> Vec<Value> {
    let payload = json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/call",
        "params": { "name": "list_records", "arguments": { "app_slug": w.slug, "table_id": "children", "limit": limit } } });
    let (_, v) = send(&w.app, "POST", "/api/mcp", token, Some(payload)).await;
    v["result"]["structuredContent"]["records"].as_array().cloned().unwrap_or_default()
}

// 2. A lookup on a page of 200 rows runs a fixed number of statements, however many rows the page has.
#[tokio::test]
async fn a_lookup_runs_one_statement_per_page_whatever_the_page_size() {
    let _db = common::TestDb::new();
    let w = world().await;
    let p = id_of(&create(&w, "parents", json!({ "name": "Dean" })).await);
    let q = id_of(&create(&w, "parents", json!({ "name": "Chair" })).await);
    for i in 0..200 {
        create(&w, "children", json!({ "title": format!("c{i}"), "amount": i, "parent": if i % 2 == 0 { &p } else { &q } })).await;
    }
    let before = lookup_statements();
    let page = list(&w, &w.token, 200).await;
    let big = lookup_statements() - before;
    assert_eq!(page.len(), 200);
    assert!(page.iter().all(|r| r["data"]["parent_name"] == "Dean" || r["data"]["parent_name"] == "Chair"), "every row has its lookup");

    let before = lookup_statements();
    let small = list(&w, &w.token, 5).await;
    let few = lookup_statements() - before;
    assert_eq!(small.len(), 5);
    assert_eq!(big, few, "the same number of statements for 200 rows and for 5");
    assert_eq!(big, 2, "one statement for each of the two lookup fields");
}

// 3. A hidden target field is absent from a lookup for a restricted person.
#[tokio::test]
async fn a_lookup_never_returns_a_field_the_reader_may_not_see() {
    let _db = common::TestDb::new();
    let w = world().await;
    let p = id_of(&create(&w, "parents", json!({ "name": "Dean", "notes": "advises on probation" })).await);
    create(&w, "children", json!({ "title": "x", "amount": 1, "parent": p })).await;

    let owner = list(&w, &w.token, 10).await;
    assert_eq!(owner[0]["data"]["parent_notes"], "advises on probation", "an editor sees it");
    assert_eq!(owner[0]["data"]["parent_name"], "Dean");

    let viewer = list(&w, &issue_test_token_and_user(VIEWER), 10).await;
    assert_eq!(viewer.len(), 1);
    assert!(viewer[0]["data"].get("parent_notes").is_none(), "a viewer does not: {}", viewer[0]);
    assert_eq!(viewer[0]["data"]["parent_name"], "Dean", "and still sees the field they may read");
    assert!(!viewer[0].to_string().contains("probation"));
}

fn fill_big(slug: &str) {
    let slug = slug.to_string();
    db(move |c| {
        c.execute(
            "INSERT INTO dataset_records (id, app_slug, data, ceds_mapping, is_ferpa_sensitive, table_id) \
             SELECT 'par-' || $1::text || '-' || g, $1::text, jsonb_build_object('_table_id', 'parents', 'name', 'P' || g), '{}', false, 'parents' \
             FROM generate_series(1, 1000) AS g",
            &[&slug],
        )
        .unwrap();
        c.execute(
            "INSERT INTO dataset_records (id, app_slug, data, ceds_mapping, is_ferpa_sensitive, table_id) \
             SELECT 'chi-' || $1::text || '-' || g, $1::text, \
                    jsonb_build_object('_table_id', 'children', 'title', 'c' || g, 'amount', g, 'parent', 'par-' || $1::text || '-' || ((g % 1000) + 1)), \
                    '{}', false, 'children' \
             FROM generate_series(1, 200000) AS g",
            &[&slug],
        )
        .unwrap();
        c.execute(
            "INSERT INTO record_index (app_slug, table_id, field, record_id, num) \
             SELECT $1::text, 'children', 'amount', id, (data->>'amount')::float8 FROM dataset_records WHERE app_slug = $1::text AND table_id = 'children'",
            &[&slug],
        )
        .unwrap();
        c.execute(
            "INSERT INTO record_links (app_slug, field, from_record, to_record, ord) \
             SELECT $1::text, 'parent', id, data->>'parent', 0 FROM dataset_records WHERE app_slug = $1::text AND table_id = 'children'",
            &[&slug],
        )
        .unwrap();
        c.execute(
            "INSERT INTO record_links (app_slug, field, from_record, to_record, ord) \
             SELECT $1::text, 'children', data->>'parent', id, 0 FROM dataset_records WHERE app_slug = $1::text AND table_id = 'children'",
            &[&slug],
        )
        .unwrap();
        c.batch_execute("ANALYZE dataset_records; ANALYZE record_index; ANALYZE record_links").unwrap();
    });
}

// 4. A rollup over 200,000 children of 1,000 parents updates one parent in under 200 ms.
#[tokio::test]
async fn updating_one_child_among_200000_updates_its_parent_in_under_200_ms() {
    let _db = common::TestDb::new();
    let w = world().await;
    fill_big(&w.slug);
    let child = format!("chi-{}-7", w.slug);
    let parent = format!("par-{}-8", w.slug);
    let current = get(&w, &child).await;
    let started = std::time::Instant::now();
    patch(&w, &child, current["version"].as_i64().unwrap(), json!({ "title": "c7", "amount": 1_000_000, "parent": parent })).await;
    let took = started.elapsed();
    println!("links: one child among 200000 updated its parent in {took:?}");
    assert!(took < std::time::Duration::from_millis(200), "took {took:?}");
    // Children 7, 1007, 2007 ... belong to this parent: 200 of them. Child 7 changed from 7 to 1,000,000.
    let expected: f64 = (0..200).map(|k| (7 + 1000 * k) as f64).sum::<f64>() - 7.0 + 1_000_000.0;
    assert_eq!(as_f64(&field_of(&w, &parent, "total").await), expected);
}

// 5. The plans of the aggregate and the lookup hold no sequential scan.
#[tokio::test]
async fn the_aggregate_and_the_lookup_plans_have_no_sequential_scan() {
    let _db = common::TestDb::new();
    let w = world().await;
    fill_big(&w.slug);
    let repo = PostgresRepository::connect(None).unwrap();
    let parent = format!("par-{}-8", w.slug);
    let rollup = repo.explain(ROLLUP_SUM_SQL, &[SqlParam::Text(parent), SqlParam::Text("children".into()), SqlParam::Text("amount".into())]).unwrap();
    assert!(!rollup.contains("Seq Scan"), "{rollup}");
    let owners: Vec<String> = (1..=200).map(|g| format!("chi-{}-{g}", w.slug)).collect();
    let lookup = repo.explain(LOOKUP_SQL, &[SqlParam::Texts(owners), SqlParam::Text("parent".into()), SqlParam::Text("name".into())]).unwrap();
    assert!(!lookup.contains("Seq Scan"), "{lookup}");
}
