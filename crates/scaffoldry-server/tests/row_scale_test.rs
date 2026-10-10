//! Row scale phase 1: a record knows its table, and a list is one indexed page.
//!
//! The big-table tests fill a private app with generated rows, then delete them. The app slug is
//! fresh each run, so the tests share the database without touching anyone's data.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::Router;
use http_body_util::BodyExt;
use scaffoldry_server::build_app_with_state;
use scaffoldry_server::repository::{FIRST_PAGE_SQL, PAGE_SQL};
use scaffoldry_server::service::identity::issue_test_token_and_user;
use scaffoldry_server::state::ServerState;
use serde_json::{json, Value};
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::Mutex;
use tower::ServiceExt;
use uuid::Uuid;

static LOCK: Mutex<()> = Mutex::const_new(());

const ROWS: i64 = 200_000;
const SEEDED_APP: &str = "physics-admissions-review";

fn db_url() -> String {
    std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://scaffoldry:scaffoldry_dev_password@127.0.0.1:5433/scaffoldry".to_string())
}

/// Runs direct database work on its own thread. The synchronous client cannot start inside the
/// async test runtime.
fn db<R: Send + 'static>(f: impl FnOnce(&mut postgres::Client) -> R + Send + 'static) -> R {
    let url = db_url();
    std::thread::spawn(move || {
        let mut c = postgres::Client::connect(&url, postgres::NoTls).expect("database");
        f(&mut c)
    })
    .join()
    .expect("database thread")
}

/// Removes the generated rows when the test ends, even if it fails.
struct Cleanup(String);
impl Drop for Cleanup {
    fn drop(&mut self) {
        let slug = self.0.clone();
        let url = db_url();
        let _ = std::thread::spawn(move || {
            if let Ok(mut c) = postgres::Client::connect(&url, postgres::NoTls) {
                let _ = c.execute("DELETE FROM dataset_records WHERE app_slug = $1", &[&slug]);
            }
        })
        .join();
    }
}

/// 200,000 rows in table `t1`, ten at a time sharing one `created_at` so the id breaks ties.
fn fill(slug: &str) {
    let slug = slug.to_string();
    db(move |c| {
    c.execute(
        "INSERT INTO dataset_records (id, app_slug, data, ceds_mapping, is_ferpa_sensitive, table_id, created_at) \
         SELECT 'rec-' || lpad(g::text, 8, '0'), $1, \
                jsonb_build_object('_table_id', 't1', 'n', g), '{}'::jsonb, false, 't1', \
                TIMESTAMPTZ '2026-01-01 00:00:00+00' + (g / 10) * interval '1 millisecond' \
         FROM generate_series(1, $2::bigint) AS g",
        &[&slug, &ROWS],
    )
    .expect("fill");
    c.batch_execute("ANALYZE dataset_records").expect("analyze");
    });
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

async fn seeded() -> (Arc<ServerState>, Router, String) {
    let state = Arc::new(ServerState::new().expect("state"));
    state.seed_demo().expect("seed");
    let app = build_app_with_state(state.clone()).expect("router");
    (state, app, issue_test_token_and_user("jordan.lee@state.edu"))
}

// 1. The column follows the JSON.
#[tokio::test]
async fn a_record_created_with_a_table_id_has_it_in_the_column() {
    let _turn = LOCK.lock().await;
    let (_state, app, token) = seeded().await;
    let table = format!("rs-{}", &Uuid::new_v4().to_string()[..8]);
    let (s, created) = call(&app, "POST", &format!("/api/v1/apps/{SEEDED_APP}/tables/{table}/records"), &token, Some(json!({ "data": { "title": "Tagged" } }))).await;
    assert_eq!(s, StatusCode::CREATED, "{created}");
    let id = created["id"].as_str().unwrap().to_string();
    assert_eq!(created["data"]["_table_id"], table.as_str(), "the key stays in the JSON");

    let (id1, table1) = (id.clone(), table.clone());
    let stored: String = db(move |c| c.query_one("SELECT table_id FROM dataset_records WHERE id = $1", &[&id1]).unwrap().get(0));
    assert_eq!(stored, table1);

    // An edit keeps the column in step with the JSON.
    let (s, _) = call(&app, "PATCH", &format!("/api/v1/apps/{SEEDED_APP}/tables/{table}/records/{id}"), &token, Some(json!({ "data": { "title": "Edited" }, "version": 1 }))).await;
    assert_eq!(s, StatusCode::OK);
    let id2 = id.clone();
    let after: String = db(move |c| c.query_one("SELECT table_id FROM dataset_records WHERE id = $1", &[&id2]).unwrap().get(0));
    assert_eq!(after, table);
    db(move |c| c.execute("DELETE FROM dataset_records WHERE id = $1", &[&id]).unwrap());
}

#[tokio::test]
async fn existing_rows_are_backfilled_from_the_json() {
    let _turn = LOCK.lock().await;
    let (state, _app, _token) = seeded().await;
    let _ = state;
    let slug = format!("rowscale-bf-{}", &Uuid::new_v4().to_string()[..8]);
    let _cleanup = Cleanup(slug.clone());
    let s2 = slug.clone();
    let t: String = db(move |c| {
        // A row as an older server wrote it: the table is only in the JSON.
        c.execute(
            "INSERT INTO dataset_records (id, app_slug, data, ceds_mapping, is_ferpa_sensitive) VALUES ($1, $2, $3, '{}', false)",
            &[&format!("old-{s2}"), &s2, &json!({ "_table_id": "legacy", "x": 1 })],
        )
        .unwrap();
        // The migration's backfill statement.
        c.execute(
            "UPDATE dataset_records SET table_id = COALESCE(data->>'_table_id', '') WHERE table_id = '' AND app_slug = $1",
            &[&s2],
        )
        .unwrap();
        c.query_one("SELECT table_id FROM dataset_records WHERE app_slug = $1", &[&s2]).unwrap().get(0)
    });
    assert_eq!(t, "legacy");
}

// 2. Paging walks the table in (created_at, id) order with no repeats.
#[tokio::test]
async fn paging_by_cursor_yields_distinct_rows_in_created_at_then_id_order() {
    let _turn = LOCK.lock().await;
    let (state, _app, _token) = seeded().await;
    let slug = format!("rowscale-{}", &Uuid::new_v4().to_string()[..8]);
    let _cleanup = Cleanup(slug.clone());
    fill(&slug);

    let first = state.list_records_page(&slug, "t1", 200, None).unwrap();
    assert_eq!(first.len(), 200);
    let cursor = first.last().map(|r| (r.created_at.clone(), r.id.clone()));
    let second = state.list_records_page(&slug, "t1", 200, cursor).unwrap();
    assert_eq!(second.len(), 200);

    let ids: Vec<String> = first.iter().chain(second.iter()).map(|r| r.id.clone()).collect();
    let distinct: std::collections::HashSet<&String> = ids.iter().collect();
    assert_eq!(ids.len(), 400);
    assert_eq!(distinct.len(), 400, "no row repeats across the page boundary");

    // The same 400 as the database orders them.
    let s3 = slug.clone();
    let truth: Vec<String> = db(move |c| {
        c.query(
            "SELECT id FROM dataset_records WHERE app_slug = $1 AND table_id = 't1' ORDER BY created_at, id LIMIT 400",
            &[&s3],
        )
        .unwrap()
        .iter()
        .map(|r| r.get(0))
        .collect()
    });
    assert_eq!(ids, truth);
}

// 3. The statements do not scan the table.
#[tokio::test]
async fn the_page_statements_hold_no_sequential_scan() {
    let _turn = LOCK.lock().await;
    let (_state, _app, _token) = seeded().await;
    let slug = format!("rowscale-{}", &Uuid::new_v4().to_string()[..8]);
    let _cleanup = Cleanup(slug.clone());
    fill(&slug);

    let s4 = slug.clone();
    let (first_plan, next_plan) = db(move |c| {
        let plan = |sql: &str, c: &mut postgres::Client, params: &[&(dyn postgres::types::ToSql + Sync)]| -> String {
            c.query(&format!("EXPLAIN {sql}"), params)
                .unwrap()
                .iter()
                .map(|r| r.get::<_, String>(0))
                .collect::<Vec<_>>()
                .join("\n")
        };
        let limit: i64 = 200;
        let first = plan(FIRST_PAGE_SQL, c, &[&s4, &"t1", &limit]);
        let after = "2026-01-01 00:00:05+00".to_string();
        let after_id = "rec-00050000".to_string();
        let next = plan(PAGE_SQL, c, &[&s4, &"t1", &after, &after_id, &limit]);
        (first, next)
    });
    assert!(!first_plan.contains("Seq Scan"), "first page:\n{first_plan}");
    assert!(first_plan.contains("idx_dataset_records_page"), "the paging index is used:\n{first_plan}");
    assert!(!next_plan.contains("Seq Scan"), "next page:\n{next_plan}");
    assert!(next_plan.contains("idx_dataset_records_page"), "the paging index is used:\n{next_plan}");
}

// 4. A page from the middle comes back fast.
#[tokio::test]
async fn a_page_from_the_middle_of_the_table_returns_in_under_200_ms() {
    let _turn = LOCK.lock().await;
    let (state, _app, _token) = seeded().await;
    let slug = format!("rowscale-{}", &Uuid::new_v4().to_string()[..8]);
    let _cleanup = Cleanup(slug.clone());
    fill(&slug);

    // Walk to about the middle by cursor, the way a client would.
    let mut cursor = None;
    for _ in 0..500 {
        let page = state.list_records_page(&slug, "t1", 200, cursor.clone()).unwrap();
        cursor = page.last().map(|r| (r.created_at.clone(), r.id.clone()));
    }
    // 500 pages of 200 is 100,000 rows in. Time the next one, after one warm-up.
    let _ = state.list_records_page(&slug, "t1", 200, cursor.clone()).unwrap();
    let started = Instant::now();
    let page = state.list_records_page(&slug, "t1", 200, cursor).unwrap();
    let elapsed = started.elapsed();
    println!("row_scale: page of {} rows from the middle of {ROWS} rows took {elapsed:?}", page.len());
    assert_eq!(page.len(), 200);
    assert!(elapsed.as_millis() < 200, "took {elapsed:?}");
}

// The REST list is a page with a cursor, and carries no total.
#[tokio::test]
async fn the_table_list_route_pages_by_cursor_without_a_total() {
    let _turn = LOCK.lock().await;
    let (_state, app, token) = seeded().await;
    let table = format!("rs-{}", &Uuid::new_v4().to_string()[..8]);
    let mut ids = Vec::new();
    for n in 0..3 {
        let (s, created) = call(&app, "POST", &format!("/api/v1/apps/{SEEDED_APP}/tables/{table}/records"), &token, Some(json!({ "data": { "n": n } }))).await;
        assert_eq!(s, StatusCode::CREATED, "{created}");
        ids.push(created["id"].as_str().unwrap().to_string());
    }

    let uri = format!("/api/v1/apps/{SEEDED_APP}/tables/{table}/records");
    let (s, page1) = call(&app, "GET", &format!("{uri}?limit=2"), &token, None).await;
    assert_eq!(s, StatusCode::OK, "{page1}");
    assert_eq!(page1["records"].as_array().unwrap().len(), 2);
    assert!(page1.get("total").is_none(), "counting every row is the cost this phase removes: {page1}");
    let cursor = page1["next_cursor"].as_str().expect("a cursor for the next page").to_string();

    let (_, page2) = call(&app, "GET", &format!("{uri}?limit=2&cursor={cursor}"), &token, None).await;
    assert_eq!(page2["records"].as_array().unwrap().len(), 1);
    assert_eq!(page2["next_cursor"], Value::Null);

    let got: Vec<&str> = page1["records"].as_array().unwrap().iter().chain(page2["records"].as_array().unwrap().iter()).map(|r| r["id"].as_str().unwrap()).collect();
    for id in &ids {
        assert!(got.contains(&id.as_str()), "{id} is on a page");
    }
    assert_eq!(got.len(), 3);

    let (s, bad) = call(&app, "GET", &format!("{uri}?cursor=not-a-cursor"), &token, None).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{bad}");
    db(move |c| {
        for id in ids {
            c.execute("DELETE FROM dataset_records WHERE id = $1", &[&id]).unwrap();
        }
    });
}
