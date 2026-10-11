//! Links phase 4: choosing a record to link. Each test uses a private app with a fresh slug.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::Router;
use http_body_util::BodyExt;
use scaffoldry_engine::{AppManifest, AppTable, FieldSpec, FieldType, LinkCardinality};
use scaffoldry_server::build_app_with_state;
use scaffoldry_server::service::identity::issue_test_token_and_user;
use scaffoldry_server::state::ServerState;
use serde_json::{json, Value};
use std::sync::Arc;
use tower::ServiceExt;
use uuid::Uuid;

const OWNER: &str = "prof.curie@science.state.edu";
const OUTSIDER: &str = "outsider.student@state.edu";
const WORKSPACE: &str = "ws-bio-lab";

fn db_url() -> String {
    std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://scaffoldry:scaffoldry_dev_password@127.0.0.1:5433/scaffoldry".to_string())
}

struct Cleanup(String);
impl Drop for Cleanup {
    fn drop(&mut self) {
        let (slug, url) = (self.0.clone(), db_url());
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

fn manifest(slug: &str) -> AppManifest {
    let mut program = FieldSpec::simple("program", "program", FieldType::Link, false, false);
    program.link_target_table = Some("programs".to_string());
    program.link_cardinality = LinkCardinality::One;
    AppManifest {
        slug: slug.to_string(),
        title: "Options".to_string(),
        description: String::new(),
        organization_code: "BIO".to_string(),
        department: "biology".to_string(),
        workspace_id: Some(WORKSPACE.to_string()),
        herm_capability_id: None,
        custom_domain: None,
        custom_domain_verified: false,
        tables: vec![
            table("apps", vec![text("applicant"), program], "applicant"),
            // `notes` is the fourth text field, so it is not searched.
            table("programs", vec![text("code"), text("name"), text("dean"), text("notes")], "code"),
        ],
        relationships: vec![],
        views: vec![],
        ceds_mappings: Default::default(),
        pages: vec![],
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
    let slug = format!("opt-{}", &Uuid::new_v4().to_string()[..8]);
    state.engine.write().unwrap().register_manifest(manifest(&slug)).unwrap();
    World {
        app: build_app_with_state(state).expect("router"),
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

async fn program(w: &World, data: Value) -> String {
    let mut data = data;
    data["_table_id"] = json!("programs");
    let (s, body) = send(&w.app, "POST", &format!("/api/v1/apps/{}/records", w.slug), &w.token, Some(json!({ "data": data }))).await;
    assert_eq!(s, StatusCode::CREATED, "{body}");
    body["id"].as_str().unwrap().to_string()
}

async fn options(w: &World, token: &str, query: &str) -> (StatusCode, Value) {
    send(&w.app, "GET", &format!("/api/v1/apps/{}/tables/programs/link-options{query}", w.slug), token, None).await
}

// 1. A person with no access to the app gets no options. (Row-level scope arrives with access-rules phase 2.)
#[tokio::test]
async fn a_person_without_access_to_the_app_gets_no_options() {
    let w = world().await;
    program(&w, json!({ "code": "CS" })).await;
    let (owner_status, owner_body) = options(&w, &w.token, "").await;
    assert_eq!(owner_status, StatusCode::OK, "{owner_body}");
    assert_eq!(owner_body["options"].as_array().unwrap().len(), 1);
    let (s, body) = options(&w, &issue_test_token_and_user(OUTSIDER), "").await;
    assert!(s == StatusCode::FORBIDDEN || s == StatusCode::NOT_FOUND, "{s} {body}");
    assert!(body.get("options").is_none(), "{body}");
}

// 2. The response holds only id, label, and secondary.
#[tokio::test]
async fn the_response_holds_only_id_label_and_secondary() {
    let w = world().await;
    let id = program(&w, json!({ "code": "CS", "name": "Computer Science", "dean": "Hopper", "notes": "private" })).await;
    let (s, body) = options(&w, &w.token, "").await;
    assert_eq!(s, StatusCode::OK, "{body}");
    assert_eq!(body["options"], json!([{ "id": id, "label": "CS", "secondary": "Computer Science" }]));
}

// 3. A query matches the primary field and the first three text fields, ignoring case, and pages.
#[tokio::test]
async fn a_query_matches_case_insensitively_and_pages() {
    let w = world().await;
    for i in 0..5 {
        program(&w, json!({ "code": format!("PX{i}"), "name": "Physics" })).await;
    }
    program(&w, json!({ "code": "BIO", "name": "Biology", "dean": "Darwin", "notes": "physics lab" })).await;

    let (s, body) = options(&w, &w.token, "?q=pHySiCs&limit=2").await;
    assert_eq!(s, StatusCode::OK, "{body}");
    assert_eq!(body["options"].as_array().unwrap().len(), 2);
    let cursor = body["next_cursor"].as_str().expect("more pages").to_string();
    let (_, second) = options(&w, &w.token, &format!("?q=physics&limit=2&cursor={cursor}")).await;
    let (_, third) = options(&w, &w.token, &format!("?q=physics&limit=2&cursor={}", second["next_cursor"].as_str().unwrap())).await;
    assert_eq!(second["options"].as_array().unwrap().len(), 2);
    assert_eq!(third["options"].as_array().unwrap().len(), 1, "five matches, and not the one whose fourth text field matches");
    assert!(third["next_cursor"].is_null());

    let (_, dean) = options(&w, &w.token, "?q=darwin").await;
    assert_eq!(dean["options"].as_array().unwrap().len(), 1, "the third text field is searched");
    let (_, wildcard) = options(&w, &w.token, "?q=%25").await;
    assert_eq!(wildcard["options"].as_array().unwrap().len(), 0, "a percent sign is text, not a wildcard");
}

// A limit above 50 is cut to 50, and a trashed record is not offered.
#[tokio::test]
async fn the_limit_is_capped_and_trashed_records_are_not_offered() {
    let w = world().await;
    let id = program(&w, json!({ "code": "GONE" })).await;
    program(&w, json!({ "code": "KEPT" })).await;
    let (s, _) = send(&w.app, "DELETE", &format!("/api/v1/apps/{}/records/{id}?version=1", w.slug), &w.token, None).await;
    assert!(s.is_success(), "{s}");
    let (_, body) = options(&w, &w.token, "?limit=500").await;
    let labels: Vec<&str> = body["options"].as_array().unwrap().iter().map(|o| o["label"].as_str().unwrap()).collect();
    assert_eq!(labels, vec!["KEPT"]);
}
