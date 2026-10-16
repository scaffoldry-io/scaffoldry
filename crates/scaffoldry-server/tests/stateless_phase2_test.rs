//! Stateless server phase 2: people, groups, tokens, sessions and appointments are read from the database.
//! Two servers share one database, so a change through one is seen by the other on the very next request.

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::Router;
use common::TestDb;
use http_body_util::BodyExt;
use scaffoldry_server::repository_people::UserFilter;
use scaffoldry_server::service::identity::issue_test_token_and_user;
use scaffoldry_server::service::people::{self, UserQuery};
use serde_json::{json, Value};
use std::time::{Duration, Instant};
use tower::ServiceExt;

const ADMIN: &str = "jordan.lee@state.edu";
const ROOT_UNIT: &str = "00000000-0000-0000-0000-000000000001";

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

async fn create_user(app: &Router, admin: &str, name: &str) -> String {
    let (status, body) = call(
        app,
        "POST",
        "/api/v1/admin/users",
        admin,
        Some(json!({
            "userName": name, "name": "Test Person", "email": name,
            "affiliation": "faculty", "department": "University", "title": "Lecturer", "reason": "Fixture"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    body["id"].as_str().or_else(|| body["user"]["id"].as_str()).unwrap_or_default().to_string()
}

fn seed_users(db: &TestDb, count: i64) {
    db.run(move |c| {
        c.execute(
            "INSERT INTO scim_users (id, user_name, active, payload) \
             SELECT 'u' || g, 'user' || g || '@state.edu', true, \
                    jsonb_build_object('id', 'u' || g, 'user_name', 'user' || g || '@state.edu', 'active', true, 'admin_hold', false, \
                                       'name', jsonb_build_object('formatted', 'User ' || g), \
                                       'emails', jsonb_build_array(jsonb_build_object('value', 'user' || g || '@state.edu')), \
                                       'roles', '[]'::jsonb) \
             FROM generate_series(1, $1::bigint) g",
            &[&count],
        )
        .unwrap();
        c.batch_execute("ANALYZE scim_users").unwrap();
    });
}

// 1. A token revoked through one instance is refused by the other on the next request.
#[tokio::test]
async fn a_token_revoked_through_one_instance_is_refused_by_the_other() {
    let db = TestDb::new();
    let (a, b) = (db.app(), db.second_app());
    let admin = issue_test_token_and_user(ADMIN);

    let (status, minted) = call(&a, "POST", "/api/v1/auth/tokens", &admin, Some(json!({ "label": "agent", "days": 5 }))).await;
    assert_eq!(status, StatusCode::CREATED, "{minted}");
    let (token, id) = (minted["token"].as_str().unwrap().to_string(), minted["id"].as_str().unwrap().to_string());

    let (status, _) = call(&b, "GET", "/api/v1/auth/tokens", &token, None).await;
    assert_eq!(status, StatusCode::OK, "the token works on the second instance");
    let (status, body) = call(&a, "DELETE", &format!("/api/v1/auth/tokens/{id}"), &admin, None).await;
    assert!(status.is_success(), "{body}");
    let (status, _) = call(&b, "GET", "/api/v1/auth/tokens", &token, None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "the second instance read the revocation");
}

// 2. A hold placed through one instance blocks the person on the other.
#[tokio::test]
async fn a_hold_placed_through_one_instance_blocks_the_person_on_the_other() {
    let db = TestDb::new();
    let (a, b) = (db.app(), db.second_app());
    let admin = issue_test_token_and_user(ADMIN);
    let who = "held.person@state.edu";
    let id = create_user(&a, &admin, who).await;
    let person = issue_test_token_and_user(who);

    let (status, _) = call(&b, "GET", "/api/v1/auth/tokens", &person, None).await;
    assert_eq!(status, StatusCode::OK);
    let (status, body) = call(&a, "POST", &format!("/api/v1/admin/users/{id}/hold"), &admin, Some(json!({ "hold": true, "reason": "Review" }))).await;
    assert!(status.is_success(), "{body}");
    let (status, _) = call(&b, "GET", "/api/v1/auth/tokens", &person, None).await;
    assert!(status == StatusCode::UNAUTHORIZED || status == StatusCode::FORBIDDEN, "the hold reached the second instance: {status}");
}

// 3. A user created through one instance is on the People page of the other.
#[tokio::test]
async fn a_user_created_through_one_instance_is_listed_by_the_other() {
    let db = TestDb::new();
    let (a, b) = (db.app(), db.second_app());
    let admin = issue_test_token_and_user(ADMIN);
    create_user(&a, &admin, "new.hire@state.edu").await;
    let (status, page) = call(&b, "GET", "/api/v1/admin/users?search=new.hire", &admin, None).await;
    assert_eq!(status, StatusCode::OK, "{page}");
    assert_eq!(page["users"].as_array().map(|u| u.len()), Some(1), "{page}");
}

// 4. Two instances fill a position of one seat at the same moment. One wins and one is refused.
#[tokio::test]
async fn two_instances_filling_a_one_seat_position_leave_one_holder() {
    let db = TestDb::new();
    let a = db.app();
    let admin = issue_test_token_and_user(ADMIN);
    let (one, two) = ("seat.one@state.edu", "seat.two@state.edu");
    create_user(&a, &admin, one).await;
    create_user(&a, &admin, two).await;
    let (status, unit) = call(&a, "POST", "/api/v1/orgs", &admin, Some(json!({ "name": "Physics", "code": "PHYS", "org_type": "Department", "parent_id": ROOT_UNIT }))).await;
    assert_eq!(status, StatusCode::CREATED, "{unit}");
    let unit = unit["id"].as_str().unwrap().to_string();
    let (status, body) = call(
        &a,
        "POST",
        "/api/v1/admin/positions",
        &admin,
        Some(json!({ "key": "chair", "name": "Chair", "description": "", "org_types": ["Department"], "max_holders": 1, "reason": "Define" })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");

    // Units and position types are cached collections that phase 3 moves to the mechanism, so the second instance
    // starts after they exist.
    let b = db.second_app();
    let uri = format!("/api/v1/orgs/{unit}/positions/chair/holders");
    for round in 0..5 {
        let (r1, r2) = tokio::join!(
            call(&a, "POST", &uri, &admin, Some(json!({ "eppn": one, "replace": false, "reason": "Race" }))),
            call(&b, "POST", &uri, &admin, Some(json!({ "eppn": two, "replace": false, "reason": "Race" }))),
        );
        let created = [r1.0, r2.0].iter().filter(|s| **s == StatusCode::CREATED).count();
        let refused = [r1.0, r2.0].iter().filter(|s| **s == StatusCode::CONFLICT).count();
        assert_eq!((created, refused), (1, 1), "round {round}: {:?} {:?}", r1, r2);
        let holders: i64 = db.run(|c| c.query_one("SELECT COUNT(*) FROM roles WHERE position_key = 'chair'", &[]).unwrap().get(0));
        assert_eq!(holders, 1, "round {round}: one seat, one holder");
        db.run(|c| c.batch_execute("DELETE FROM roles WHERE position_key = 'chair'").unwrap());
    }
}

/// A guard against a missing index at 50,000 rows, not a speed target.
const LIMIT: Duration = Duration::from_millis(250);

// 5. One user among 50,000 costs one statement. A People page costs three at most.
#[test]
fn one_user_among_fifty_thousand_is_one_statement_and_a_page_is_three() {
    let db = TestDb::new();
    seed_users(&db, 50_000);
    let state = db.state();
    state.refresh().unwrap();

    let before = state.repository.statements();
    let t = Instant::now();
    let found = state.repository.user_by_name("USER25000@state.edu").unwrap().expect("found, whatever the case");
    let one = t.elapsed();
    assert_eq!(found.id, "u25000");
    assert_eq!(state.repository.statements() - before, 1);
    println!("one user among 50,000: {one:?}");
    // The statement counts above are the exact claim. The time is a coarse guard against a missing index, which costs
    // seconds at this size. It is wide because the full suite runs many tests at once, and 20 ms and 50 ms both
    // failed there (28.8 ms and 66 ms) while passing alone in 1 to 13 ms.
    assert!(one < LIMIT, "{one:?}");

    let before = state.repository.statements();
    let t = Instant::now();
    let (rows, next) = people::list_users(&state, &UserQuery { limit: Some(50), ..Default::default() }).unwrap();
    let page = t.elapsed();
    assert_eq!(rows.len(), 50);
    assert!(next.is_some());
    assert!(state.repository.statements() - before <= 3, "{} statements", state.repository.statements() - before);
    println!("a People page of 50 among 50,000: {page:?}");
    assert!(page < LIMIT, "{page:?}");

    let t = Instant::now();
    let found = state.repository.search_users(&UserFilter { prefix: Some("user4999".into()), ..Default::default() }, 0, 51).unwrap();
    println!("a prefix search among 50,000: {:?}", t.elapsed());
    assert_eq!(found.len(), 11, "user4999 and user4999x");
}

// 6. The first page and the second page do not repeat or skip a person.
#[test]
fn pages_do_not_repeat_or_skip() {
    let db = TestDb::new();
    seed_users(&db, 130);
    let state = db.state();
    let mut seen = Vec::new();
    let mut cursor = None;
    loop {
        let (rows, next) = people::list_users(&state, &UserQuery { limit: Some(50), cursor: cursor.clone(), ..Default::default() }).unwrap();
        seen.extend(rows.iter().map(|r| r["user_name"].as_str().unwrap().to_string()));
        match next {
            Some(c) => cursor = Some(c),
            None => break,
        }
    }
    assert_eq!(seen.len(), 130);
    let mut sorted = seen.clone();
    sorted.sort_by_key(|s| s.to_lowercase());
    assert_eq!(seen, sorted, "in user name order");
    sorted.dedup();
    assert_eq!(sorted.len(), 130, "no repeats");
}

// 7. The resolver is given the holdings on the path it walks, not every holding in the university.
#[test]
fn the_resolver_loads_the_holdings_on_its_path_only() {
    let db = TestDb::new();
    db.run(|c| {
        c.batch_execute(&format!(
            "INSERT INTO position_types (key, name, description, org_types, max_holders, created_by) \
             VALUES ('chair', 'Chair', '', '[\"Department\"]', 1, 'test'); \
             INSERT INTO organizations (id, parent_id, name, code, org_type) \
             SELECT gen_random_uuid(), '{ROOT_UNIT}', 'Unit ' || g, 'U' || g, 'Department' FROM generate_series(1, 400) g; \
             INSERT INTO persons (id, first_name, last_name, email, eppn) \
             SELECT gen_random_uuid(), 'P', 'P', 'p' || g || '@state.edu', 'p' || g || '@state.edu' FROM generate_series(1, 400) g; \
             INSERT INTO roles (id, person_id, organization_id, role_title, scoped_affiliation, is_primary, source, position_key) \
             SELECT gen_random_uuid(), p.id, o.id, 'Chair', 'faculty', false, 'api', 'chair' \
             FROM (SELECT id, row_number() OVER (ORDER BY email) n FROM persons WHERE email LIKE 'p%@state.edu') p \
             JOIN (SELECT id, row_number() OVER (ORDER BY code) n FROM organizations WHERE code LIKE 'U%') o ON o.n = p.n;"
        ))
        .unwrap();
    });
    let state = db.state();
    let some_unit: uuid::Uuid = db.run(|c| c.query_one("SELECT id FROM organizations WHERE code = 'U7'", &[]).unwrap().get(0));
    let holdings = scaffoldry_server::service::approvers::path_holdings(&state, &[some_unit], &[]);
    assert!(!holdings.is_empty(), "the holder at the unit is there");
    assert!(holdings.len() < 10, "{} holdings of 400", holdings.len());
}

// 8. After this phase the server holds no person, group, appointment, session or token between requests.
#[test]
fn the_five_collections_are_gone_from_the_state() {
    let text = std::fs::read_to_string(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/state.rs")).unwrap();
    let start = text.find("pub struct ServerState {").unwrap();
    let body = &text[start..start + text[start..].find("\n}\n").unwrap()];
    for gone in ["pub users:", "pub groups:", "pub roles:", "pub sessions:", "pub api_tokens:"] {
        assert!(!body.contains(gone), "{gone} is still a field");
    }
}
