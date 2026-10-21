//! Stateless server phase 3: units, position types, workspaces, guards, datasets, and relationships are read through
//! the cache mechanism. A change through one instance is read through another on the next request.
//!
//! `labels` is on the closed list but has no table and no writer yet, so nothing can be cached or tested for it.

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::Router;
use common::TestDb;
use http_body_util::BodyExt;
use scaffoldry_server::service::identity::issue_test_token_and_user;
use scaffoldry_server::state::OrganizationNode;
use serde_json::{json, Value};
use tower::ServiceExt;
use uuid::Uuid;

const ADMIN: &str = "jordan.lee@state.edu";
const STUDENT: &str = "student.p3@state.edu";
const ROOT: &str = "00000000-0000-0000-0000-000000000001";

async fn send(
    app: &Router,
    method: &str,
    uri: &str,
    who: &str,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let token = issue_test_token_and_user(who);
    let b = Request::builder()
        .method(method)
        .uri(uri)
        .header("authorization", format!("Bearer {token}"));
    let req = match body {
        Some(v) => b
            .header("content-type", "application/json")
            .body(Body::from(v.to_string()))
            .unwrap(),
        None => b.body(Body::empty()).unwrap(),
    };
    let resp = app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

fn names_of(list: &Value, key: &str) -> Vec<String> {
    let items = list.get("items").or_else(|| list.get(key)).unwrap_or(list);
    items
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|x| {
                    x.get("code")
                        .or_else(|| x.get("key"))
                        .or_else(|| x.get("id"))
                        .and_then(|v| v.as_str())
                        .map(String::from)
                })
                .collect()
        })
        .unwrap_or_default()
}

async fn make_unit(app: &Router, code: &str, parent: &str) -> String {
    let (s, body) = send(
        app,
        "POST",
        "/api/v1/orgs",
        ADMIN,
        Some(json!({ "name": code, "code": code, "org_type": "Department", "parent_id": parent })),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{body}");
    body["id"].as_str().unwrap().to_string()
}

async fn make_workspace(app: &Router, code: &str, visibility: &str) -> String {
    let (s, body) = send(app, "POST", "/api/v1/workspaces", ADMIN, Some(json!({ "name": code, "code": code, "visibility": visibility, "department": "biology" }))).await;
    assert_eq!(s, StatusCode::CREATED, "{body}");
    body["id"].as_str().unwrap().to_string()
}

// 1a. A unit created through A is listed by B on its next request.
#[tokio::test]
async fn a_unit_created_through_one_instance_is_listed_by_the_other() {
    let db = TestDb::new();
    let (a, b) = (db.app(), db.second_app());
    let (_, before) = send(&b, "GET", "/api/v1/orgs", ADMIN, None).await;
    assert!(!names_of(&before, "orgs").contains(&"ASTRO".to_string()));
    make_unit(&a, "ASTRO", ROOT).await;
    let (s, after) = send(&b, "GET", "/api/v1/orgs", ADMIN, None).await;
    assert_eq!(s, StatusCode::OK, "{after}");
    assert!(
        names_of(&after, "orgs").contains(&"ASTRO".to_string()),
        "B lists the unit A created: {after}"
    );
}

// 1b. A position type created through A is listed by B.
#[tokio::test]
async fn a_position_type_created_through_one_instance_is_listed_by_the_other() {
    let db = TestDb::new();
    let (a, b) = (db.app(), db.second_app());
    let (_, before) = send(&b, "GET", "/api/v1/admin/positions", ADMIN, None).await;
    assert!(!names_of(&before, "positions").contains(&"dean".to_string()));
    let (s, body) = send(&a, "POST", "/api/v1/admin/positions", ADMIN, Some(json!({ "key": "dean", "name": "Dean", "org_types": ["Department"], "max_holders": 1, "reason": "A college has a dean" }))).await;
    assert_eq!(s, StatusCode::CREATED, "{body}");
    let (_, after) = send(&b, "GET", "/api/v1/admin/positions", ADMIN, None).await;
    assert!(
        names_of(&after, "positions").contains(&"dean".to_string()),
        "B lists the position type A created: {after}"
    );
}

// 1c. A workspace created through A is read by B.
#[tokio::test]
async fn a_workspace_created_through_one_instance_is_read_by_the_other() {
    let db = TestDb::new();
    let (a, b) = (db.app(), db.second_app());
    let (_, _) = send(&b, "GET", "/api/v1/workspaces", ADMIN, None).await;
    let id = make_workspace(&a, "P3WS1", "departmental").await;
    let (s, body) = send(&b, "GET", &format!("/api/v1/workspaces/{id}"), ADMIN, None).await;
    assert_eq!(s, StatusCode::OK, "B reads the workspace A created: {body}");
}

// 1d. A dataset published through A is read by B.
#[tokio::test]
async fn a_dataset_published_through_one_instance_is_read_by_the_other() {
    let db = TestDb::new();
    let (a, b) = (db.app(), db.second_app());
    let (_, _) = send(&b, "GET", "/api/v1/datasets", ADMIN, None).await;
    let (s, body) = send(
        &a,
        "POST",
        "/api/v1/datasets",
        ADMIN,
        Some(json!({ "id": "p3_courses", "name": "P3 Courses", "department": "biology" })),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{body}");
    let (s, got) = send(&b, "GET", "/api/v1/datasets/p3_courses", ADMIN, None).await;
    assert_eq!(s, StatusCode::OK, "B reads the dataset A published: {got}");
}

// 1e. A dataset relationship made through A is read by B.
#[tokio::test]
async fn a_relationship_made_through_one_instance_is_read_by_the_other() {
    let db = TestDb::new();
    let (a, b) = (db.app(), db.second_app());
    for id in ["p3_left", "p3_right"] {
        let (s, body) = send(
            &a,
            "POST",
            "/api/v1/datasets",
            ADMIN,
            Some(json!({ "id": id, "name": id, "department": "biology" })),
        )
        .await;
        assert_eq!(s, StatusCode::CREATED, "{body}");
    }
    let (_, _) = send(
        &b,
        "GET",
        "/api/v1/datasets/p3_left/relationships",
        ADMIN,
        None,
    )
    .await;
    let (s, body) = send(&a, "POST", "/api/v1/datasets/p3_left/relationships", ADMIN, Some(json!({ "id": "p3_rel", "target_dataset_id": "p3_right", "source_field": "id", "target_field": "id" }))).await;
    assert_eq!(s, StatusCode::CREATED, "{body}");
    let (s, listed) = send(
        &b,
        "GET",
        "/api/v1/datasets/p3_left/relationships",
        ADMIN,
        None,
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{listed}");
    assert!(
        listed.to_string().contains("p3_rel"),
        "B lists the relationship A made: {listed}"
    );
}

// 2. A guard added through A denies the next matching request through B. B started after the workspace and its member
//    existed, so what it must pick up is the guard.
#[tokio::test]
async fn a_guard_added_through_one_instance_denies_the_next_request_through_the_other() {
    let db = TestDb::new();
    let a = db.app();
    let ws = make_workspace(&a, "P3GUARD", "restricted").await;
    let (s, body) = send(
        &a,
        "POST",
        &format!("/api/v1/workspaces/{ws}/collaborators"),
        ADMIN,
        Some(json!({ "eppn": STUDENT, "role": "viewer" })),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{body}");
    let other = make_unit(&a, "P3OTHER", ROOT).await;
    let b = db.second_app();
    let (s, body) = send(
        &b,
        "GET",
        &format!("/api/v1/workspaces/{ws}"),
        STUDENT,
        None,
    )
    .await;
    assert_eq!(
        s,
        StatusCode::OK,
        "before the guard a member reads it through B: {body}"
    );

    let (s, body) = send(&a, "PUT", &format!("/api/v1/workspaces/{ws}/guards"), ADMIN, Some(json!({ "rules": [{ "template": "deny_access_outside_units", "unit_ids": [other] }], "reason": "Units only" }))).await;
    assert_eq!(s, StatusCode::OK, "{body}");
    let (s, body) = send(
        &b,
        "GET",
        &format!("/api/v1/workspaces/{ws}"),
        STUDENT,
        None,
    )
    .await;
    assert_eq!(
        s,
        StatusCode::FORBIDDEN,
        "the guard A added denies the request B gets next: {body}"
    );
}

// 3. A collaborator removed through A loses access through B on the next request.
#[tokio::test]
async fn a_collaborator_removed_through_one_instance_loses_access_through_the_other() {
    let db = TestDb::new();
    let a = db.app();
    let ws = make_workspace(&a, "P3MEMBER", "restricted").await;
    let (s, body) = send(
        &a,
        "POST",
        &format!("/api/v1/workspaces/{ws}/collaborators"),
        ADMIN,
        Some(json!({ "eppn": STUDENT, "role": "viewer" })),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{body}");
    let b = db.second_app();
    let (s, body) = send(
        &b,
        "GET",
        &format!("/api/v1/workspaces/{ws}"),
        STUDENT,
        None,
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{body}");

    let (s, body) = send(
        &a,
        "DELETE",
        &format!("/api/v1/workspaces/{ws}/collaborators/{STUDENT}"),
        ADMIN,
        None,
    )
    .await;
    assert!(s.is_success(), "{body}");
    let (s, body) = send(
        &b,
        "GET",
        &format!("/api/v1/workspaces/{ws}"),
        STUDENT,
        None,
    )
    .await;
    assert_ne!(
        s,
        StatusCode::OK,
        "B no longer lets the removed member in: {body}"
    );
}

// 4. A repository write that fails leaves memory unchanged.
#[test]
fn a_write_that_fails_leaves_memory_unchanged() {
    let db = TestDb::new();
    let state = db.state();
    let orphan = OrganizationNode {
        id: Uuid::new_v4(),
        parent_id: Some(Uuid::new_v4()),
        name: "Orphan".into(),
        code: "ORPHAN".into(),
        org_type: "Department".into(),
        hierarchy_key: "academic".into(),
        synthetic: false,
    };
    let before = state.organizations.read().unwrap().len();
    let result = state.persist_organization(&orphan);
    assert!(
        result.is_err(),
        "a parent that does not exist is a constraint violation"
    );
    assert_eq!(
        state.organizations.read().unwrap().len(),
        before,
        "memory did not take the unit the database refused"
    );
    assert!(!state.organizations.read().unwrap().contains_key(&orphan.id));
}
