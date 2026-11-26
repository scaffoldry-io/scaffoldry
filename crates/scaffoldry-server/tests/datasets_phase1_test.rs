//! Data layer phase 1: an app table is a dataset with an id.
//!
//! A list of datasets returns it. A record read through the dataset id returns the same row as the app path.

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::Router;
use http_body_util::BodyExt;
use scaffoldry_engine::{AppManifest, AppTable, FieldSpec, FieldType};
use scaffoldry_server::build_app_with_state;
use scaffoldry_server::service::identity::issue_test_token_and_user;
use scaffoldry_server::state::ServerState;
use serde_json::{json, Value};
use std::sync::Arc;
use tower::ServiceExt;
use uuid::Uuid;

const WORKSPACE: &str = "ws-bio-lab";
const ADMIN: &str = "jordan.lee@state.edu";

fn table(id: &str) -> AppTable {
    AppTable {
        retired_fields: Vec::new(), key: None, unit_field: None,
        id: id.to_string(),
        name: id.to_string(),
        slug: id.to_string(),
        description: None,
        icon: None,
        fields: vec![FieldSpec::simple("name", "name", FieldType::Text, false, false)],
        primary_field: Some("name".to_string()),
        sample_records: vec![],
        coordinates: None,
        sealed: false,
        row_place: Default::default(),
    }
}

fn manifest(slug: &str) -> AppManifest {
    AppManifest {
        slug: slug.to_string(),
        title: "Advising".to_string(),
        description: String::new(),
        organization_code: "BIO".to_string(),
        department: "biology".to_string(),
        workspace_id: Some(WORKSPACE.to_string()),
        herm_capability_id: None,
        coordinates: None,
        sealed: false,
        integrations: Default::default(),
        kind: Default::default(),
        custom_domain: None,
        custom_domain_verified: false,
        tables: vec![table("students"), table("advisors")],
        relationships: vec![],
        views: vec![],
        ceds_mappings: Default::default(),
        pages: vec![],
        access: Default::default(),
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

struct World {
    app: Router,
    slug: String,
    admin: String,
}

fn world() -> World {
    let state = Arc::new(ServerState::new().expect("state"));
    state.seed_demo().expect("seed");
    let slug = format!("ds-{}", &Uuid::new_v4().to_string()[..8]);
    state.persist_app_manifest(manifest(&slug)).expect("the app is saved");
    World { app: build_app_with_state(state).expect("router"), slug, admin: issue_test_token_and_user(ADMIN) }
}

async fn dataset_of(w: &World, table: &str) -> Value {
    let (s, body) = send(&w.app, "GET", "/api/v1/data/datasets", &w.admin, None).await;
    assert_eq!(s, StatusCode::OK, "{body}");
    body["datasets"]
        .as_array()
        .expect("a datasets list")
        .iter()
        .find(|d| d["app_slug"] == json!(w.slug) && d["table_id"] == json!(table))
        .cloned()
        .unwrap_or_else(|| panic!("no dataset for {}/{table} in {body}", w.slug))
}

#[tokio::test]
async fn an_app_table_is_a_dataset_with_a_stable_id() {
    let _db = common::TestDb::new();
    let w = world();
    let first = dataset_of(&w, "students").await;
    let id = first["id"].as_str().expect("an id");
    assert!(id.starts_with("ds_"), "{id}");
    assert_eq!(first["name"], json!("students"));
    // Each table has its own id, and a second list returns the same ones.
    let other = dataset_of(&w, "advisors").await;
    assert_ne!(other["id"], first["id"]);
    assert_eq!(dataset_of(&w, "students").await["id"], first["id"]);
}

#[tokio::test]
async fn a_record_read_by_dataset_id_is_the_row_the_app_path_returns() {
    let _db = common::TestDb::new();
    let w = world();
    let (s, made) = send(&w.app, "POST", &format!("/api/v1/apps/{}/tables/students/records", w.slug), &w.admin, Some(json!({"name": "Ada"}))).await;
    assert!(s.is_success(), "{s} {made}");
    let rid = made["id"].as_str().expect("a record id").to_string();

    let ds = dataset_of(&w, "students").await;
    let (s1, via_app) = send(&w.app, "GET", &format!("/api/v1/apps/{}/tables/students/records/{rid}", w.slug), &w.admin, None).await;
    let (s2, via_dataset) = send(&w.app, "GET", &format!("/api/v1/data/datasets/{}/records/{rid}", ds["id"].as_str().unwrap()), &w.admin, None).await;
    assert_eq!((s1, s2), (StatusCode::OK, StatusCode::OK), "{via_dataset}");
    assert_eq!(via_dataset, via_app, "the two paths return the same row");

    // The same row is not in the other table's dataset.
    let other = dataset_of(&w, "advisors").await;
    let (s, _) = send(&w.app, "GET", &format!("/api/v1/data/datasets/{}/records/{rid}", other["id"].as_str().unwrap()), &w.admin, None).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn an_unknown_dataset_or_a_caller_without_the_app_gets_nothing() {
    let _db = common::TestDb::new();
    let w = world();
    let (s, _) = send(&w.app, "GET", "/api/v1/data/datasets/ds_nope/records/x", &w.admin, None).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    let (s, _) = send(&w.app, "GET", "/api/v1/data/datasets", "not-a-token", None).await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);

    let ds = dataset_of(&w, "students").await;
    let outsider = issue_test_token_and_user(&format!("outsider-{}@elsewhere.edu", &Uuid::new_v4().to_string()[..8]));
    let (s, body) = send(&w.app, "GET", "/api/v1/data/datasets", &outsider, None).await;
    assert_eq!(s, StatusCode::OK);
    assert!(!body["datasets"].as_array().unwrap().iter().any(|d| d["app_slug"] == json!(w.slug)), "the list shows only apps the caller may read");
    let (s, _) = send(&w.app, "GET", &format!("/api/v1/data/datasets/{}/records/x", ds["id"].as_str().unwrap()), &outsider, None).await;
    assert!(s == StatusCode::NOT_FOUND || s == StatusCode::FORBIDDEN, "{s}");
}

#[tokio::test]
async fn the_migration_gives_an_existing_app_table_a_dataset_and_can_run_again() {
    let _db = common::TestDb::new();
    let _state = ServerState::new().expect("state");
    let slug = format!("old-{}", &Uuid::new_v4().to_string()[..8]);
    let m = serde_json::to_value(manifest(&slug)).unwrap();
    let url = scaffoldry_server::testing::database_url();
    let rows = std::thread::spawn(move || {
        let mut c = postgres::Client::connect(&url, postgres::NoTls).expect("database");
        c.execute(
            "INSERT INTO app_manifests (slug, title, organization_code, department, manifest) VALUES ($1, 'Old', 'BIO', 'biology', $2)",
            &[&slug, &m],
        )
        .unwrap();
        let sql = include_str!("../../scaffoldry-core/migrations/0114_datasets.sql");
        c.batch_execute(sql).unwrap();
        c.batch_execute(sql).unwrap();
        c.query("SELECT table_id FROM datasets WHERE app_slug = $1 ORDER BY table_id", &[&slug])
            .unwrap()
            .iter()
            .map(|r| r.get::<_, String>(0))
            .collect::<Vec<_>>()
    })
    .join()
    .unwrap();
    assert_eq!(rows, ["advisors", "students"]);
}
