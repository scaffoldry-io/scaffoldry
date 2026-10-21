//! Stateless server phase 1: one cache mechanism, an honest boot, and a database for each test.

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::Router;
use common::TestDb;
use http_body_util::BodyExt;
use scaffoldry_server::service::identity::issue_test_token_and_user;
use scaffoldry_server::state::ServerState;
use serde_json::{json, Value};
use tower::ServiceExt;

const ADMIN: &str = "jordan.lee@state.edu";

async fn call(
    app: &Router,
    method: &str,
    uri: &str,
    token: &str,
    body: Option<Value>,
) -> (StatusCode, Value) {
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

// 1. A setting changed through one instance is read through the other on the next request.
#[tokio::test]
async fn a_setting_changed_through_one_instance_is_read_through_the_other() {
    let db = TestDb::new();
    let (a, b) = (db.app(), db.second_app());
    let admin = issue_test_token_and_user(ADMIN);

    let (_, before) = call(&b, "GET", "/api/v1/settings", &admin, None).await;
    assert_eq!(before["tokens.max_days"], 90, "{before}");
    let (status, saved) = call(
        &a,
        "PUT",
        "/api/v1/settings/tokens.max_days",
        &admin,
        Some(json!(30)),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{saved}");
    let (_, after) = call(&b, "GET", "/api/v1/settings", &admin, None).await;
    assert_eq!(
        after["tokens.max_days"], 30,
        "the second instance reloaded the setting: {after}"
    );
}

// 2. With nothing changed, refresh asks the database one question.
#[test]
fn refresh_runs_one_statement_when_nothing_changed() {
    let db = TestDb::new();
    let state = db.state();
    state.refresh().expect("refresh");
    let before = state.repository.statements();
    state.refresh().expect("refresh");
    state.refresh().expect("refresh");
    assert_eq!(
        state.repository.statements() - before,
        2,
        "one statement for each refresh"
    );
}

// 3. A bump for a name that is not in the table is an error, and the transaction rolls back.
#[test]
fn a_bump_for_an_unknown_name_is_an_error_and_rolls_back() {
    let db = TestDb::new();
    let state = db.state();
    let version = |db: &TestDb| -> i64 {
        db.run(|c| {
            c.query_one(
                "SELECT version FROM cache_versions WHERE name = 'settings'",
                &[],
            )
            .unwrap()
            .get(0)
        })
    };
    let start = version(&db);
    let result = state.repository.with_client(|c| {
        let mut tx = c.transaction()?;
        scaffoldry_server::cache::bump(&mut tx, "settings")?;
        scaffoldry_server::cache::bump(&mut tx, "no_such_collection")?;
        tx.commit()?;
        Ok(())
    });
    assert!(
        result.is_err(),
        "a name that is not on the closed list is refused"
    );
    assert_eq!(
        version(&db),
        start,
        "the first bump went with the transaction"
    );

    let committed = state.repository.with_client(|c| {
        let mut tx = c.transaction()?;
        scaffoldry_server::cache::bump(&mut tx, "settings")?;
        tx.commit()?;
        Ok(())
    });
    assert!(committed.is_ok());
    assert_eq!(version(&db), start + 1);
}

// 4. A boot read that fails stops the server.
#[test]
fn a_database_where_the_organizations_cannot_be_read_does_not_start() {
    let db = TestDb::new();
    drop(db.state());
    db.run(|c| {
        c.batch_execute("ALTER TABLE organizations RENAME TO organizations_away")
            .unwrap()
    });
    let refused = ServerState::connect(db.url());
    assert!(
        refused.is_err(),
        "a server must not start with an empty organization tree"
    );
    db.run(|c| {
        c.batch_execute("ALTER TABLE organizations_away RENAME TO organizations")
            .unwrap()
    });
    assert!(
        ServerState::connect(db.url()).is_ok(),
        "and it starts when the table is back"
    );
}

// 5. A request fails with storage_unavailable when the cache versions cannot be read.
#[tokio::test]
async fn a_request_is_503_when_the_cache_versions_cannot_be_read() {
    let db = TestDb::new();
    let app = db.app();
    let token = issue_test_token_and_user(ADMIN);
    let (status, _) = call(&app, "GET", "/api/v1/settings", &token, None).await;
    assert_eq!(status, StatusCode::OK);
    db.run(|c| {
        c.batch_execute("ALTER TABLE cache_versions RENAME TO cache_versions_away")
            .unwrap()
    });
    let (status, body) = call(&app, "GET", "/api/v1/settings", &token, None).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{body}");
    assert_eq!(body["code"], "storage_unavailable");
    db.run(|c| {
        c.batch_execute("ALTER TABLE cache_versions_away RENAME TO cache_versions")
            .unwrap()
    });
    let (status, _) = call(&app, "GET", "/api/v1/settings", &token, None).await;
    assert_eq!(status, StatusCode::OK, "the next request tries again");
}

// 6. Two tests that create the same organization code at once do not meet.
#[test]
fn two_tests_may_create_the_same_organization_code_at_once() {
    let make = || {
        std::thread::spawn(|| {
            let db = TestDb::new();
            let state = db.state();
            let org = scaffoldry_server::state::OrganizationNode {
                id: uuid::Uuid::new_v4(),
                parent_id: Some(
                    uuid::Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(),
                ),
                code: "PHYS".to_string(),
                name: "Physics".to_string(),
                org_type: "department".to_string(),
                hierarchy_key: "academic".to_string(),
                synthetic: false,
            };
            state
                .persist_organization(&org)
                .expect("PHYS is free in a database of its own");
        })
    };
    let (one, two) = (make(), make());
    one.join().expect("first");
    two.join().expect("second");
}

// 7. The cost of refresh. The median of 1,000 calls is printed for the pull request. Above 2 ms the phase stops.
#[test]
fn refresh_costs_under_two_milliseconds() {
    let db = TestDb::new();
    let state = db.state();
    state.refresh().unwrap();
    let mut times: Vec<std::time::Duration> = (0..1000)
        .map(|_| {
            let t = std::time::Instant::now();
            state.refresh().unwrap();
            t.elapsed()
        })
        .collect();
    times.sort();
    let median = times[500];
    println!("refresh median over 1000 calls: {median:?}");
    assert!(
        median < std::time::Duration::from_millis(2),
        "median {median:?}"
    );
}
