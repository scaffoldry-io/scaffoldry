//! Live data phases 1 and 2: records live in PostgreSQL only, boot does not clobber stored rows,
//! and a save carries a version.
//!
//! Each test builds its own `ServerState`, sometimes two on the same database, to stand for a
//! restart. Record ids are fresh, so the tests share the database without clearing it.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::Router;
use http_body_util::BodyExt;
use scaffoldry_server::build_app_with_state;
use scaffoldry_server::service::identity::issue_test_token_and_user;
use scaffoldry_server::state::ServerState;
use serde_json::{json, Value};
use std::sync::Arc;
use tokio::sync::Mutex;
use tower::ServiceExt;

/// The tests rename seeded rows, so they take turns.
static LOCK: Mutex<()> = Mutex::const_new(());

const SLUG: &str = "physics-admissions-review";
const ADMIN: &str = "jordan.lee@state.edu";

fn new_state() -> Arc<ServerState> {
    Arc::new(ServerState::new().expect("state"))
}

async fn seeded() -> (Arc<ServerState>, Router, String) {
    let state = new_state();
    state.seed_demo().expect("seed");
    let app = build_app_with_state(state.clone()).expect("router");
    (state, app, issue_test_token_and_user(ADMIN))
}

async fn call(app: &Router, method: &str, uri: &str, token: &str, body: Option<Value>) -> (StatusCode, Value) {
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

async fn create(app: &Router, token: &str, title: &str) -> Value {
    let (s, body) = call(app, "POST", &format!("/api/v1/apps/{SLUG}/records"), token, Some(json!({ "data": { "title": title, "gpa": 3.1 } }))).await;
    assert_eq!(s, StatusCode::CREATED, "{body}");
    body
}

fn record_uri(id: &str) -> String {
    format!("/api/v1/apps/{SLUG}/records/{id}")
}

// ---- Phase 1 ------------------------------------------------------------------------------

#[tokio::test]
async fn seeding_twice_does_not_change_a_workspace_or_dataset_someone_edited() {
    let _turn = LOCK.lock().await;
    let (state, _app, _token) = seeded().await;

    let mut ws = state.repository.as_ref().unwrap().get_workspace("ws-bio-lab").unwrap().expect("seeded workspace");
    let original = ws.name.clone();
    ws.name = "Renamed by an administrator".to_string();
    state.persist_workspace(ws).unwrap();

    // A second seed, and then a fresh process.
    state.seed_demo().expect("seed again");
    let restarted = new_state();
    restarted.seed_demo().expect("seed after restart");

    let stored = restarted.repository.as_ref().unwrap().get_workspace("ws-bio-lab").unwrap().unwrap();
    assert_eq!(stored.name, "Renamed by an administrator", "the stored row is left as stored");
    assert_eq!(state.workspaces.read().unwrap()["ws-bio-lab"].name, "Renamed by an administrator");
    assert_eq!(restarted.workspaces.read().unwrap()["ws-bio-lab"].name, "Renamed by an administrator");

    // Put it back for the other tests.
    let mut back = stored;
    back.name = original;
    restarted.persist_workspace(back).unwrap();
}

#[tokio::test]
async fn a_record_created_by_one_process_is_read_by_the_next() {
    let _turn = LOCK.lock().await;
    let (_state, app, token) = seeded().await;
    let created = create(&app, &token, "Survives a restart").await;
    let id = created["id"].as_str().unwrap();

    let (_s2, app2, token2) = seeded().await;
    let (status, read) = call(&app2, "GET", &record_uri(id), &token2, None).await;
    assert_eq!(status, StatusCode::OK, "{read}");
    assert_eq!(read["data"]["title"], "Survives a restart");
    assert_eq!(read["created_by"], ADMIN);

    let (_, list) = call(&app2, "GET", &format!("/api/v1/apps/{SLUG}/records"), &token2, None).await;
    assert!(list["records"].as_array().unwrap().iter().any(|r| r["id"] == id), "the list reads the table");
}

#[tokio::test]
async fn a_cell_edit_and_a_delete_survive_a_restart() {
    let _turn = LOCK.lock().await;
    let (_state, app, token) = seeded().await;
    let edited = create(&app, &token, "Before").await;
    let removed = create(&app, &token, "To remove").await;
    let (edit_id, remove_id) = (edited["id"].as_str().unwrap(), removed["id"].as_str().unwrap());

    let (s, _) = call(&app, "PATCH", &record_uri(edit_id), &token, Some(json!({ "data": { "title": "After", "gpa": 3.1 }, "version": 1 }))).await;
    assert_eq!(s, StatusCode::OK);
    let (s, _) = call(&app, "DELETE", &record_uri(remove_id), &token, None).await;
    assert!(s == StatusCode::OK || s == StatusCode::NO_CONTENT, "delete: {s}");

    let (_s2, app2, token2) = seeded().await;
    let (_, read) = call(&app2, "GET", &record_uri(edit_id), &token2, None).await;
    assert_eq!(read["data"]["title"], "After", "the edit reached PostgreSQL");
    let (s, _) = call(&app2, "GET", &record_uri(remove_id), &token2, None).await;
    assert_eq!(s, StatusCode::NOT_FOUND, "the delete reached PostgreSQL");
}

// ---- Phase 2 ------------------------------------------------------------------------------

#[tokio::test]
async fn a_record_carries_a_version_that_starts_at_one() {
    let _turn = LOCK.lock().await;
    let (_state, app, token) = seeded().await;
    let created = create(&app, &token, "Versioned").await;
    assert_eq!(created["version"], 1);
    let id = created["id"].as_str().unwrap();
    let (_, read) = call(&app, "GET", &record_uri(id), &token, None).await;
    assert_eq!(read["version"], 1);
    let (_, list) = call(&app, "GET", &format!("/api/v1/apps/{SLUG}/records"), &token, None).await;
    let row = list["records"].as_array().unwrap().iter().find(|r| r["id"] == id).unwrap().clone();
    assert_eq!(row["version"], 1);
}

#[tokio::test]
async fn a_save_with_the_returned_version_succeeds_and_bumps_it_by_one() {
    let _turn = LOCK.lock().await;
    let (_state, app, token) = seeded().await;
    let created = create(&app, &token, "First").await;
    let id = created["id"].as_str().unwrap();

    let (s, saved) = call(&app, "PATCH", &record_uri(id), &token, Some(json!({ "data": { "title": "Second", "gpa": 3.1 }, "version": 1 }))).await;
    assert_eq!(s, StatusCode::OK, "{saved}");
    assert_eq!(saved["version"], 2);
    let (s, again) = call(&app, "PATCH", &record_uri(id), &token, Some(json!({ "data": { "title": "Third", "gpa": 3.1 }, "version": saved["version"] }))).await;
    assert_eq!(s, StatusCode::OK, "{again}");
    assert_eq!(again["version"], 3);
}

#[tokio::test]
async fn two_saves_with_the_same_version_get_one_success_and_one_conflict() {
    let _turn = LOCK.lock().await;
    let (_state, app, token) = seeded().await;
    let created = create(&app, &token, "Original").await;
    let id = created["id"].as_str().unwrap();

    let (s1, _) = call(&app, "PATCH", &record_uri(id), &token, Some(json!({ "data": { "title": "Winner", "gpa": 3.1 }, "version": 1 }))).await;
    assert_eq!(s1, StatusCode::OK);
    let (s2, conflict) = call(&app, "PATCH", &record_uri(id), &token, Some(json!({ "data": { "title": "Loser", "gpa": 3.1 }, "version": 1 }))).await;
    assert_eq!(s2, StatusCode::CONFLICT, "{conflict}");
    assert_eq!(conflict["code"], "version_conflict");
    assert_eq!(conflict["version"], 2, "the body names the current version so the caller can reload");

    let (_, read) = call(&app, "GET", &record_uri(id), &token, None).await;
    assert_eq!(read["data"]["title"], "Winner", "the stored field keeps the first write");
    assert_eq!(read["version"], 2);
}

#[tokio::test]
async fn a_save_without_a_version_is_a_bad_request_and_writes_nothing() {
    let _turn = LOCK.lock().await;
    let (_state, app, token) = seeded().await;
    let created = create(&app, &token, "Untouched").await;
    let id = created["id"].as_str().unwrap();

    let (s, body) = call(&app, "PATCH", &record_uri(id), &token, Some(json!({ "data": { "title": "Blind overwrite", "gpa": 3.1 } }))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{body}");
    let (_, read) = call(&app, "GET", &record_uri(id), &token, None).await;
    assert_eq!(read["data"]["title"], "Untouched");
    assert_eq!(read["version"], 1);
}

#[tokio::test]
async fn an_app_save_is_versioned_in_the_same_way() {
    let _turn = LOCK.lock().await;
    let (_state, app, token) = seeded().await;
    let (s, current) = call(&app, "GET", &format!("/api/v1/apps/{SLUG}"), &token, None).await;
    assert_eq!(s, StatusCode::OK);
    let version = current["version"].as_i64().expect("the app reports its version");

    let mut first = current.clone();
    first["description"] = json!("Saved by the first builder");
    first["version"] = json!(version);
    let (s, saved) = call(&app, "PUT", &format!("/api/v1/apps/{SLUG}"), &token, Some(first)).await;
    assert_eq!(s, StatusCode::OK, "{saved}");
    assert_eq!(saved["version"], version + 1);

    let mut second = current.clone();
    second["description"] = json!("Saved by the second builder");
    second["version"] = json!(version);
    let (s, conflict) = call(&app, "PUT", &format!("/api/v1/apps/{SLUG}"), &token, Some(second)).await;
    assert_eq!(s, StatusCode::CONFLICT, "{conflict}");
    assert_eq!(conflict["version"], version + 1);

    let mut blind = current.clone();
    blind.as_object_mut().unwrap().remove("version");
    let (s, _) = call(&app, "PUT", &format!("/api/v1/apps/{SLUG}"), &token, Some(blind)).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "a missing version is not a silent overwrite");

    let (_, after) = call(&app, "GET", &format!("/api/v1/apps/{SLUG}"), &token, None).await;
    assert_eq!(after["description"], "Saved by the first builder", "the JSON is not merged");
}
