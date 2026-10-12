//! Field types phase 3: long text.
//!
//! Acceptance tests:
//! 1. Validator: LongText cannot be indexed.
//! 2. Validator: LongText cannot be used for sort or filter in views.
//! 3. Record creation: A 100,001-character value is 400.
//! 4. Record creation: A 100,000-character value is accepted.
//! 5. Record retrieval: LongText value is preserved and retrievable.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::Router;
use http_body_util::BodyExt;
use scaffoldry_engine::{
    validate_manifest, AppManifest, AppTable, AppView, CompoundFilter, FieldSpec, FieldType,
    FilterClause, FilterConjunction, FilterOperator, SortDirection, SortRule, ViewType,
};
use scaffoldry_server::build_app_with_state;
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

struct Cleanup(String);
impl Drop for Cleanup {
    fn drop(&mut self) {
        let slug = self.0.clone();
        let url = db_url();
        let _ = std::thread::spawn(move || {
            if let Ok(mut c) = postgres::Client::connect(&url, postgres::NoTls) {
                let _ = c.execute("DELETE FROM jobs WHERE payload->>'app_slug' = $1", &[&slug]);
                let _ = c.execute("DELETE FROM record_index WHERE app_slug = $1", &[&slug]);
                let _ = c.execute("DELETE FROM record_history WHERE app_slug = $1", &[&slug]);
                let _ = c.execute("DELETE FROM dataset_records WHERE app_slug = $1", &[&slug]);
                let _ = c.execute("DELETE FROM app_proposals WHERE app_slug = $1", &[&slug]);
                let _ = c.execute("DELETE FROM app_manifests WHERE slug = $1", &[&slug]);
            }
        })
        .join();
    }
}

struct World {
    app: Router,
    token: String,
    state: Arc<ServerState>,
    slug: String,
    _cleanup: Cleanup,
}

async fn world() -> World {
    let state = Arc::new(ServerState::new().expect("state"));
    state.seed_demo().expect("seed");
    let slug = format!("ft3-{}", &Uuid::new_v4().to_string()[..8]);
    World {
        app: build_app_with_state(state.clone()).expect("router"),
        token: issue_test_token_and_user(OWNER),
        state,
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

fn manifest_with_fields(slug: &str, fields: Vec<FieldSpec>) -> AppManifest {
    AppManifest {
        slug: slug.to_string(),
        title: "Field Types Phase 3 Test".to_string(),
        description: String::new(),
        organization_code: "BIO".to_string(),
        department: "biology".to_string(),
        workspace_id: Some(WORKSPACE.to_string()),
        herm_capability_id: None,
        custom_domain: None,
        custom_domain_verified: false,
        tables: vec![AppTable {
            id: "t1".to_string(),
            name: "Notes".to_string(),
            slug: "notes".to_string(),
            description: None,
            icon: None,
            fields,
            primary_field: Some("title".to_string()),
            sample_records: vec![],
        }],
        views: vec![],
        relationships: vec![],
        pages: vec![],
        ceds_mappings: Default::default(),
        access: Default::default(),
    }
}

#[tokio::test]
async fn test_manifest_rejects_indexed_long_text() {
    let mut field = FieldSpec::simple("body", "Body", FieldType::LongText, false, false);
    field.indexed = true;
    let m = manifest_with_fields("test-indexed-lt", vec![
        FieldSpec::simple("title", "Title", FieldType::Text, true, false),
        field,
    ]);

    let res = validate_manifest(&m);
    assert!(res.is_err(), "Manifest with indexed LongText must be rejected");
    let errs = res.unwrap_err();
    assert!(
        errs.iter().any(|e| e.contains("cannot be indexed")),
        "Error message must specify that LongText cannot be indexed"
    );
}

#[tokio::test]
async fn test_manifest_rejects_sort_and_filter_on_long_text() {
    let mut m = manifest_with_fields("test-sort-filter-lt", vec![
        FieldSpec::simple("title", "Title", FieldType::Text, true, false),
        FieldSpec::simple("body", "Body", FieldType::LongText, false, false),
    ]);

    let mut view = AppView::table("v1", "Table View", ViewType::Grid, vec![]);
    view.table_id = Some("t1".to_string());
    view.sort_rules.push(SortRule {
        id: "s1".to_string(),
        field_name: "body".to_string(),
        direction: SortDirection::Asc,
    });
    view.filters = Some(CompoundFilter {
        conjunction: FilterConjunction::And,
        clauses: vec![FilterClause {
            id: "c1".to_string(),
            field_name: "body".to_string(),
            operator: FilterOperator::Contains,
            value: "foo".to_string(),
        }],
    });
    m.views.push(view);

    let res = validate_manifest(&m);
    assert!(res.is_err(), "Manifest with sort/filter on LongText must be rejected");
    let errs = res.unwrap_err();
    assert!(
        errs.iter().any(|e| e.contains("cannot be used for sort")),
        "Error message must state LongText cannot be used for sort"
    );
    assert!(
        errs.iter().any(|e| e.contains("cannot be used for filter")),
        "Error message must state LongText cannot be used for filter"
    );
}

#[tokio::test]
async fn test_long_text_100001_chars_is_400() {
    let _guard = LOCK.lock().await;
    let w = world().await;
    let m = manifest_with_fields(&w.slug, vec![
        FieldSpec::simple("title", "Title", FieldType::Text, true, false),
        FieldSpec::simple("notes", "Notes", FieldType::LongText, false, false),
    ]);
    w.state.persist_app_manifest(m).expect("manifest");

    // 1. 100,001 characters should return 400
    let too_long = "a".repeat(100_001);
    let (status, resp) = send(
        &w.app,
        "POST",
        &format!("/api/v1/apps/{}/records", w.slug),
        &w.token,
        Some(json!({
            "data": {
                "_table_id": "t1",
                "title": "Too long doc",
                "notes": too_long,
            }
        })),
    ).await;

    assert_eq!(status, StatusCode::BAD_REQUEST, "100,001 chars must return 400 Bad Request");
    let err_str = resp.to_string();
    assert!(err_str.contains("bad_request") || err_str.contains("100,000") || err_str.contains("notes"));

    // 2. 100,000 characters should be accepted
    let exact_limit = "b".repeat(100_000);
    let (ok_status, ok_resp) = send(
        &w.app,
        "POST",
        &format!("/api/v1/apps/{}/records", w.slug),
        &w.token,
        Some(json!({
            "data": {
                "_table_id": "t1",
                "title": "Exact limit doc",
                "notes": exact_limit,
            }
        })),
    ).await;

    assert_eq!(ok_status, StatusCode::CREATED, "100,000 chars must be accepted with 201 Created");
    let rec_id = ok_resp["id"].as_str().expect("record id");

    // Retrieve the record and verify the 100,000 characters
    let (get_status, get_resp) = send(
        &w.app,
        "GET",
        &format!("/api/v1/apps/{}/records/{}", w.slug, rec_id),
        &w.token,
        None,
    ).await;

    assert_eq!(get_status, StatusCode::OK);
    assert_eq!(get_resp["data"]["notes"].as_str().unwrap().len(), 100_000);
}
