//! Views phase 3: a million rows in a window.
//!
//! Skipping 500,000 rows with `OFFSET` over 1,000,000 took 3.0 s with the join and 0.4 s on the index alone (see
//! `docs/plans/views.md`), against a limit of 0.3 s. So a job samples the sort key of every 1,000th row into
//! `view_anchors`, and a jump starts at the nearest anchor. Windows, counts and the last page all run on the index,
//! and a person with a row scope gets windows and counts over their own scope only.

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::Router;
use http_body_util::BodyExt;
use scaffoldry_core::access::{Op, Operand, PersonAttr, Pred, RowOp, RowRule};
use scaffoldry_engine::{AppManifest, AppTable, FieldSpec, FieldType, SelectOption, SelectOptionColor};
use scaffoldry_server::build_app_with_state;
use scaffoldry_server::jobs::{JobConfig, JobRunner, JOB_KINDS};
use scaffoldry_server::service::identity::issue_test_token_and_user;
use scaffoldry_server::service::scope::set_rules;
use scaffoldry_server::state::{CollaboratorRecord, ServerState};
use serde_json::{json, Value};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tower::ServiceExt;
use uuid::Uuid;


const OWNER: &str = "prof.curie@science.state.edu";
const VIEWER: &str = "student.viewer@science.state.edu";
const WORKSPACE: &str = "ws-bio-lab";
const MILLION: i64 = 1_000_000;

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
        let (slug, url) = (self.0.clone(), db_url());
        let _ = std::thread::spawn(move || {
            if let Ok(mut c) = postgres::Client::connect(&url, postgres::NoTls) {
                let _ = c.execute("DELETE FROM view_anchors WHERE view_id IN (SELECT id FROM app_views WHERE app_slug = $1)", &[&slug]);
                let _ = c.execute("DELETE FROM view_anchor_builds WHERE view_id IN (SELECT id FROM app_views WHERE app_slug = $1)", &[&slug]);
                let _ = c.execute("DELETE FROM jobs WHERE payload->>'app_slug' = $1", &[&slug]);
                let _ = c.execute("DELETE FROM index_status WHERE app_slug = $1", &[&slug]);
                let _ = c.execute("DELETE FROM record_index WHERE app_slug = $1", &[&slug]);
                let _ = c.execute("DELETE FROM app_views WHERE app_slug = $1", &[&slug]);
                let _ = c.execute("DELETE FROM record_history WHERE app_slug = $1", &[&slug]);
                let _ = c.execute("DELETE FROM dataset_records WHERE app_slug = $1", &[&slug]);
                let _ = c.execute("DELETE FROM app_manifests WHERE slug = $1", &[&slug]);
            }
        })
        .join();
    }
}

fn indexed(name: &str, t: FieldType) -> FieldSpec {
    let mut f = FieldSpec::simple(name, name, t, false, false);
    f.indexed = true;
    f
}

fn manifest(slug: &str) -> AppManifest {
    let mut status = indexed("status", FieldType::Select);
    status.select_options = ["s1", "s2", "s3", "s4", "s5"]
        .iter()
        .enumerate()
        .map(|(i, id)| SelectOption { id: id.to_string(), label: id.to_string(), color: SelectOptionColor::Neutral, order: i as i32 })
        .collect();
    AppManifest {
        slug: slug.to_string(),
        title: "Registrar".to_string(),
        description: String::new(),
        organization_code: "BIO".to_string(),
        department: "biology".to_string(),
        workspace_id: Some(WORKSPACE.to_string()),
        herm_capability_id: None,
        custom_domain: None,
        custom_domain_verified: false,
        tables: vec![AppTable {
            id: "t1".to_string(),
            name: "Things".to_string(),
            slug: "things".to_string(),
            description: None,
            icon: None,
            fields: vec![indexed("amount", FieldType::Number), status, indexed("advisor", FieldType::Text)],
            primary_field: None,
            sample_records: vec![],
        }],
        relationships: vec![],
        views: vec![],
        ceds_mappings: Default::default(),
        pages: vec![],
        access: Default::default(),
    }
}

struct World {
    app: Router,
    state: Arc<ServerState>,
    slug: String,
    owner: String,
    viewer: String,
    _cleanup: Cleanup,
}

fn collaborator(eppn: &str, role: &str) -> CollaboratorRecord {
    CollaboratorRecord {
        id: Uuid::new_v4().to_string(),
        workspace_id: WORKSPACE.to_string(),
        eppn: eppn.to_string(),
        name: eppn.to_string(),
        role: role.to_string(),
        scoped_affiliation: "faculty".to_string(),
        department: "biology".to_string(),
        added_at: String::new(),
    }
}

/// Rows 1..=`rows`. `amount` is the row number, so row `n` (counting from zero) is at `amount = n + 1` when the view
/// sorts by amount. `status` is s1..s5 in turn. Every tenth row is advised by the viewer.
async fn world(rows: i64) -> World {
    let slug = format!("vp3-{}", &Uuid::new_v4().to_string()[..8]);
    let state = Arc::new(ServerState::new().expect("state"));
    state.seed_demo().expect("seed");
    state.engine.write().unwrap().register_manifest(manifest(&slug)).unwrap();
    state.collaborators.write().unwrap().insert(WORKSPACE.to_string(), vec![collaborator(OWNER, "owner"), collaborator(VIEWER, "viewer")]);

    let s = slug.clone();
    db(move |c| {
        c.execute(
            "INSERT INTO dataset_records (id, app_slug, data, ceds_mapping, is_ferpa_sensitive, table_id, created_at) \
             SELECT 'rec-' || $1 || '-' || lpad(g::text, 8, '0'), $1, \
                    jsonb_build_object('_table_id', 't1', 'amount', g, 'status', 's' || ((g % 5) + 1), \
                                       'advisor', CASE WHEN g % 10 = 0 THEN $3::text ELSE 'adv' || g || '@state.edu' END), \
                    '{}'::jsonb, false, 't1', TIMESTAMPTZ '2026-01-01 00:00:00+00' + g * interval '1 millisecond' \
             FROM generate_series(1, $2::bigint) AS g",
            &[&s, &rows, &VIEWER],
        )
        .expect("records");
        for (field, col, expr) in [("amount", "num", "(data->>'amount')::float8"), ("status", "txt", "lower(data->>'status')"), ("advisor", "txt", "lower(data->>'advisor')")] {
            c.execute(
                &format!("INSERT INTO record_index (app_slug, table_id, field, record_id, {col}) SELECT app_slug, 't1', '{field}', id, {expr} FROM dataset_records WHERE app_slug = $1"),
                &[&s],
            )
            .expect("index");
        }
        c.batch_execute("ANALYZE dataset_records; ANALYZE record_index;").expect("analyze");
    });

    World {
        app: build_app_with_state(state.clone()).expect("router"),
        state,
        owner: issue_test_token_and_user(OWNER),
        viewer: issue_test_token_and_user(VIEWER),
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

async fn make_view(w: &World, name: &str, config: Value) -> String {
    let body = json!({ "name": name, "kind": "grid", "ownership": "collaborative", "table_id": "t1", "config": config });
    let (s, v) = send(&w.app, "POST", &format!("/api/v1/apps/{}/views", w.slug), &w.owner, Some(body)).await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    v["id"].as_str().unwrap().to_string()
}

async fn run(w: &World, token: &str, id: &str, query: &str) -> (StatusCode, Value) {
    send(&w.app, "GET", &format!("/api/v1/apps/{}/views/{id}/data{query}", w.slug), token, None).await
}

fn by_amount() -> Value {
    json!({ "sort_rules": [{ "id": "s", "field_name": "amount", "direction": "Asc" }] })
}

/// The `amount` of each row of a response, in order.
fn amounts(v: &Value) -> Vec<i64> {
    v["records"].as_array().unwrap().iter().map(|r| r["data"]["amount"].as_i64().unwrap()).collect()
}

fn fast() -> JobConfig {
    JobConfig { workers: 2, poll_ms: 20, backoff_secs: [0, 0, 0], ..JobConfig::default() }
}

/// Waits for the anchors of a view to be built.
fn wait_for_anchors(w: &World, view: &str) {
    let view: Uuid = view.parse().unwrap();
    let started = Instant::now();
    loop {
        let built: i64 = db(move |c| c.query_one("SELECT count(*) FROM view_anchor_builds WHERE view_id = $1", &[&view]).unwrap().get(0));
        if built > 0 {
            return;
        }
        assert!(started.elapsed() < Duration::from_secs(120), "the anchors were not built in two minutes for {}", w.slug);
        std::thread::sleep(Duration::from_millis(100));
    }
}

// 1 to 4. A million rows: the strategy meets its limit at depth, a jump to the middle returns the right rows, an exact
//    count on a hard filter says at least inside the timeout, and the last page is the final 200 rows in order.
#[tokio::test]
async fn a_million_rows_open_in_a_window_by_anchor_and_the_last_page_and_count_hold() {
    let _db = common::TestDb::new();
    let w = world(MILLION).await;
    let view = make_view(&w, "By amount", by_amount()).await;

    // Before the anchors exist a deep jump still answers, by skipping on the index, and it asks for the anchors.
    let (s, first) = run(&w, &w.owner, &view, "?offset=500000&limit=200").await;
    assert_eq!(s, StatusCode::OK, "{first}");
    assert_eq!(first["window"]["strategy"], "offset");
    assert_eq!(amounts(&first), (500_001..=500_200).collect::<Vec<i64>>(), "the rows are right even without anchors");

    let runner = JobRunner::start(&w.state.repository, JOB_KINDS, fast()).unwrap();
    wait_for_anchors(&w, &view);
    runner.stop();
    let anchors: i64 = {
        let v: Uuid = view.parse().unwrap();
        db(move |c| c.query_one("SELECT count(*) FROM view_anchors WHERE view_id = $1", &[&v]).unwrap().get(0))
    };
    assert_eq!(anchors, 1_000, "one anchor for every 1,000th row of 1,000,000");

    // 1. The chosen strategy meets its limit at 1,000,000 rows. The unanchored skip measured 3.0 s.
    // The fastest of three, so one slow moment on a shared runner is not read as the strategy being slow.
    let mut took = Duration::MAX;
    let mut mid = Value::Null;
    for _ in 0..3 {
        let started = Instant::now();
        let (s, v) = run(&w, &w.owner, &view, "?offset=500000&limit=200").await;
        took = took.min(started.elapsed());
        assert_eq!(s, StatusCode::OK, "{v}");
        mid = v;
    }
    assert_eq!(mid["window"]["strategy"], "anchor");
    println!("views phase 3: a window at row 500,000 of {MILLION} took {took:?} by anchor");
    assert!(took < Duration::from_millis(300), "a jump took {took:?}, and the limit is 300 ms");

    // 2. A jump to 50 percent returns the right rows. So do the edges of an anchor and the far end.
    assert_eq!(amounts(&mid), (500_001..=500_200).collect::<Vec<i64>>());
    for offset in [0usize, 1_999, 2_000, 2_001, 123_456, 999_800] {
        let (s, v) = run(&w, &w.owner, &view, &format!("?offset={offset}&limit=200")).await;
        assert_eq!(s, StatusCode::OK, "{v}");
        let from = offset as i64 + 1;
        assert_eq!(amounts(&v), (from..from + 200).collect::<Vec<i64>>(), "offset {offset}");
    }
    let (s, over) = run(&w, &w.owner, &view, "?offset=1000001&limit=200").await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{over}");

    // 4. The last page is the final 200 rows, in order.
    let (s, last) = run(&w, &w.owner, &view, "?last=true&limit=200").await;
    assert_eq!(s, StatusCode::OK, "{last}");
    assert_eq!(last["window"]["strategy"], "reverse");
    assert_eq!(amounts(&last), (999_801..=1_000_000).collect::<Vec<i64>>());

    // A descending view reads the same rows from the other end, and its last page is the front of the table.
    let down = make_view(&w, "Down", json!({ "sort_rules": [{ "id": "s", "field_name": "amount", "direction": "Desc" }] })).await;
    let (_, top) = run(&w, &w.owner, &down, "?offset=0&limit=3").await;
    assert_eq!(amounts(&top), vec![1_000_000, 999_999, 999_998]);
    let (_, bottom) = run(&w, &w.owner, &down, "?last=true&limit=3").await;
    assert_eq!(amounts(&bottom), vec![3, 2, 1]);

    // 3. A count that cannot finish in time says at least, within the timeout. Here every row matches a tree that needs
    //    a probe for each, and the exact count is given one millisecond.
    let hard = make_view(
        &w,
        "Hard",
        json!({
            "sort_rules": [{ "id": "s", "field_name": "amount", "direction": "Asc" }],
            "filters": { "conjunction": "And", "clauses": [], "groups": [{ "conjunction": "Or", "clauses": [
                { "id": "a", "field_name": "status", "operator": "In", "value": "[\"s1\",\"s2\",\"s3\",\"s4\",\"s5\"]" },
                { "id": "b", "field_name": "amount", "operator": "Between", "value": "1..1000000" }
            ] }] }
        }),
    )
    .await;
    w.state.count_timeout_ms.store(1, std::sync::atomic::Ordering::SeqCst);
    let started = Instant::now();
    let (s, counted) = run(&w, &w.owner, &hard, "?offset=0&limit=1&count=exact").await;
    assert_eq!(s, StatusCode::OK, "{counted}");
    assert_eq!(counted["count"], json!({ "at_least": 250_000 }), "{counted}");
    assert!(started.elapsed() < Duration::from_secs(5), "the answer took {:?}", started.elapsed());

    // With the default timeout an easy count is exact.
    w.state.count_timeout_ms.store(2_000, std::sync::atomic::Ordering::SeqCst);
    let (_, exact) = run(&w, &w.owner, &view, "?offset=0&limit=1&count=exact").await;
    assert_eq!(exact["count"], json!({ "value": 1_000_000, "exact": true }));
}

// 5. A window of a view with no sort cannot be placed, and a count needs `exact`.
#[tokio::test]
async fn a_window_needs_a_sort_and_a_count_needs_exact() {
    let _db = common::TestDb::new();
    let w = world(50).await;
    let unsorted = make_view(&w, "Plain", json!({})).await;
    let (s, v) = run(&w, &w.owner, &unsorted, "?offset=10&limit=5").await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{v}");
    let sorted = make_view(&w, "Sorted", by_amount()).await;
    let (s, v) = run(&w, &w.owner, &sorted, "?offset=10&limit=5&count=roughly").await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{v}");
    let (s, v) = run(&w, &w.owner, &sorted, "?offset=10&limit=5&count=exact").await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(amounts(&v), vec![11, 12, 13, 14, 15]);
    assert_eq!(v["count"], json!({ "value": 50, "exact": true }));
}

// 6. A scoped person gets counts and windows over their own scope only. They never use the anchors of the whole table.
#[tokio::test]
async fn a_scoped_person_gets_counts_and_windows_over_their_own_scope_only() {
    let _db = common::TestDb::new();
    let w = world(2_000).await;
    set_rules(
        &w.slug,
        vec![RowRule {
            id: "advisees".into(),
            table_id: "t1".into(),
            ops: vec![RowOp::Read],
            visible_when: Pred::Cmp { field: "advisor".into(), op: Op::Eq, value: Operand::Person(PersonAttr::Address) },
            except_roles: vec!["admin".into(), "owner".into()],
            message: None,
        }],
        &w.state,
    );
    let view = make_view(&w, "By amount", by_amount()).await;

    // Every tenth row is the viewer's: 200 of 2,000, at amounts 10, 20, 30, ...
    let (s, mine) = run(&w, &w.viewer, &view, "?offset=0&limit=500&count=exact").await;
    assert_eq!(s, StatusCode::OK, "{mine}");
    assert_eq!(mine["count"], json!({ "value": 200, "exact": true }), "the count is theirs, not the table's");
    assert_eq!(amounts(&mine), (1..=200).map(|n| n * 10).collect::<Vec<i64>>());

    let (_, tail) = run(&w, &w.viewer, &view, "?offset=190&limit=50").await;
    assert_eq!(amounts(&tail), (191..=200).map(|n| n * 10).collect::<Vec<i64>>(), "a window past the front is in their own rows");
    assert_ne!(tail["window"]["strategy"], "anchor");

    let (_, last) = run(&w, &w.viewer, &view, "?last=true&limit=3").await;
    assert_eq!(amounts(&last), vec![1_980, 1_990, 2_000]);

    // A person with no scope sees every row.
    let (_, all) = run(&w, &w.owner, &view, "?offset=1995&limit=50&count=exact").await;
    assert_eq!(all["count"], json!({ "value": 2_000, "exact": true }));
    assert_eq!(amounts(&all), vec![1_996, 1_997, 1_998, 1_999, 2_000]);
}
