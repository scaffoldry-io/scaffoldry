//! Row scale phase 2: the pivot index. Sort and filter on an indexed field use `record_index`.
//!
//! The big-table tests fill a private app with generated rows and delete them when they end. The app
//! slug is fresh each run, so the tests share the database without touching anyone's data.

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::Router;
use http_body_util::BodyExt;
use scaffoldry_engine::{AppManifest, AppTable, AppView, FieldSpec, FieldType, FilterOperator, SortDirection, SortRule, ViewType};
use scaffoldry_server::build_app_with_state;
use scaffoldry_server::repository::{build_indexed_sql, IndexPhase, IndexedClause, IndexedQuery, IndexedSort, IndexedValue};
use scaffoldry_server::service::identity::issue_test_token_and_user;
use scaffoldry_server::state::ServerState;
use serde_json::{json, Value};
use std::collections::HashSet;
use std::sync::Arc;
use tower::ServiceExt;
use uuid::Uuid;


const OWNER: &str = "prof.curie@science.state.edu";
const WORKSPACE: &str = "ws-bio-lab";
const ROWS: i64 = 200_000;

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
                let _ = c.execute("DELETE FROM dataset_records WHERE app_slug = $1", &[&slug]);
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

fn plain(name: &str, t: FieldType) -> FieldSpec {
    FieldSpec::simple(name, name, t, false, false)
}

/// `amount` and `stamp` and `tags` are indexed. `plain_text` is not.
fn fields() -> Vec<FieldSpec> {
    vec![
        indexed("amount", FieldType::Number),
        indexed("stamp", FieldType::Date),
        indexed("tags", FieldType::MultiSelect),
        plain("plain_text", FieldType::Text),
        plain("score", FieldType::Number),
    ]
}

fn manifest(slug: &str, views: Vec<AppView>) -> AppManifest {
    AppManifest {
        slug: slug.to_string(),
        title: "Index".to_string(),
        description: String::new(),
        organization_code: "BIO".to_string(),
        department: "biology".to_string(),
        workspace_id: Some(WORKSPACE.to_string()),
        herm_capability_id: None,
        coordinates: None,
        custom_domain: None,
        custom_domain_verified: false,
        tables: vec![AppTable {
            id: "t1".to_string(),
            name: "Things".to_string(),
            slug: "things".to_string(),
            description: None,
            icon: None,
            fields: fields(),
            primary_field: None,
            sample_records: vec![],
            coordinates: None,
            row_place: Default::default(),
        }],
        relationships: vec![],
        views,
        ceds_mappings: Default::default(),
        pages: vec![],
        access: Default::default(),
    }
}

struct World {
    state: Arc<ServerState>,
    app: Router,
    token: String,
    slug: String,
    _cleanup: Cleanup,
}

async fn world() -> World {
    let state = Arc::new(ServerState::new().expect("state"));
    state.seed_demo().expect("seed");
    let slug = format!("idx-{}", &Uuid::new_v4().to_string()[..8]);
    state.engine.write().unwrap().register_manifest(manifest(&slug, vec![])).unwrap();
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

async fn tool(w: &World, name: &str, args: Value) -> (bool, Value) {
    let payload = json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": { "name": name, "arguments": args } });
    let (_, v) = send(&w.app, "POST", "/api/mcp", &w.token, Some(payload)).await;
    let result = &v["result"];
    if result["isError"] == true {
        return (true, result["content"][0]["text"].clone());
    }
    (false, result["structuredContent"].clone())
}

/// Rows `1..=n`. `amount` is the series number. Rows whose number is in `blank` (from the start) have none.
/// `record_index` is filled for `amount` in the same statement shape the write path produces.
fn fill(slug: &str, n: i64, blank: i64) {
    let slug = slug.to_string();
    db(move |c| {
        c.execute(
            "INSERT INTO dataset_records (id, app_slug, data, ceds_mapping, is_ferpa_sensitive, table_id, created_at) \
             SELECT 'rec-' || $1 || '-' || lpad(g::text, 8, '0'), $1, \
                    CASE WHEN g <= $3 THEN jsonb_build_object('_table_id', 't1', 'plain_text', 'x', 'score', g) \
                         ELSE jsonb_build_object('_table_id', 't1', 'amount', g, 'plain_text', 'x', 'score', g) END, \
                    '{}'::jsonb, false, 't1', TIMESTAMPTZ '2026-01-01 00:00:00+00' + (g / 10) * interval '1 millisecond' \
             FROM generate_series(1, $2::bigint) AS g",
            &[&slug, &n, &blank],
        )
        .expect("fill");
        c.execute(
            "INSERT INTO record_index (app_slug, table_id, field, record_id, num) \
             SELECT app_slug, 't1', 'amount', id, (data->>'amount')::float8 FROM dataset_records WHERE app_slug = $1",
            &[&slug],
        )
        .expect("fill index");
        c.batch_execute("ANALYZE dataset_records; ANALYZE record_index").expect("analyze");
    });
}

fn index_rows(slug: &str, field: &str) -> i64 {
    let (slug, field) = (slug.to_string(), field.to_string());
    db(move |c| c.query_one("SELECT count(*) FROM record_index WHERE app_slug = $1 AND field = $2", &[&slug, &field]).unwrap().get(0))
}

async fn create(w: &World, data: Value) -> Value {
    let mut d = data;
    d["_table_id"] = json!("t1");
    let (s, body) = send(&w.app, "POST", &format!("/api/v1/apps/{}/records", w.slug), &w.token, Some(json!({ "data": d }))).await;
    assert_eq!(s, StatusCode::CREATED, "{body}");
    body
}

type IndexSnapshotRow = (String, String, i16, Option<String>, Option<String>, Option<String>);

// 1. A write maintains the index: one row, and an update leaves one row with the new value.
#[tokio::test]
async fn a_write_keeps_one_index_row_per_indexed_field_and_an_update_replaces_it() {
    let _db = common::TestDb::new();
    let w = world().await;
    let created = create(&w, json!({ "amount": 42, "plain_text": "x" })).await;
    let id = created["id"].as_str().unwrap().to_string();

    let rows = |id: String| -> Vec<(String, Option<f64>)> {
        db(move |c| {
            c.query("SELECT field, num FROM record_index WHERE record_id = $1 ORDER BY field", &[&id])
                .unwrap()
                .iter()
                .map(|r| (r.get(0), r.get(1)))
                .collect()
        })
    };
    // `amount`, plus blank rows for the indexed fields the record does not have.
    let first = rows(id.clone());
    assert!(first.contains(&("amount".to_string(), Some(42.0))), "{first:?}");
    assert_eq!(first.iter().filter(|(f, _)| f == "amount").count(), 1);
    assert!(first.iter().any(|(f, n)| f == "stamp" && n.is_none()), "a blank value still gets a row: {first:?}");
    assert!(!first.iter().any(|(f, _)| f == "plain_text" || f == "score"), "only indexed fields: {first:?}");

    let (s, saved) = send(
        &w.app,
        "PATCH",
        &format!("/api/v1/apps/{}/records/{id}", w.slug),
        &w.token,
        Some(json!({ "data": { "_table_id": "t1", "amount": 77, "plain_text": "x" }, "version": 1 })),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{saved}");
    let second = rows(id.clone());
    assert_eq!(second.iter().filter(|(f, _)| f == "amount").count(), 1, "still one row");
    assert!(second.contains(&("amount".to_string(), Some(77.0))), "{second:?}");

    // A delete takes the rows with it.
    let (s, _) = send(&w.app, "DELETE", &format!("/api/v1/apps/{}/records/{id}", w.slug), &w.token, None).await;
    assert!(s == StatusCode::OK || s == StatusCode::NO_CONTENT);
    assert!(rows(id).is_empty());
}

// 2. Rebuilding from nothing gives the rows the writes gave.
#[tokio::test]
async fn rebuild_index_gives_back_exactly_the_rows_the_writes_made() {
    let _db = common::TestDb::new();
    let w = world().await;
    create(&w, json!({ "amount": 1, "stamp": "2026-03-04", "tags": ["Red", "Blue"] })).await;
    create(&w, json!({ "amount": "12.5", "stamp": "2026-03-04T10:00:00Z", "tags": [] })).await;
    create(&w, json!({ "plain_text": "no indexed values" })).await;
    create(&w, json!({ "amount": null, "tags": ["Green"] })).await;

    let snapshot = |slug: String| -> Vec<IndexSnapshotRow> {
        db(move |c| {
            let mut rows: Vec<_> = c
                .query(
                    "SELECT d.data->>'amount', i.field, i.ord, i.num::text, i.txt, i.ts::text \
                     FROM record_index i JOIN dataset_records d ON d.id = i.record_id WHERE i.app_slug = $1",
                    &[&slug],
                )
                .unwrap()
                .iter()
                .map(|r| (r.get::<_, Option<String>>(0).unwrap_or_default(), r.get(1), r.get(2), r.get(3), r.get(4), r.get(5)))
                .collect();
            rows.sort();
            rows
        })
    };
    let before = snapshot(w.slug.clone());
    assert!(before.len() >= 12, "three indexed fields on four records, and a row per tag: {before:?}");

    let slug = w.slug.clone();
    db(move |c| c.execute("DELETE FROM record_index WHERE app_slug = $1", &[&slug]).map(|_| ()).unwrap());
    assert!(snapshot(w.slug.clone()).is_empty());
    let repo = &w.state.repository;
    let n = repo.rebuild_index(&w.slug, "t1", &fields().into_iter().filter(|f| f.indexed).collect::<Vec<_>>()).unwrap();
    assert_eq!(n as usize, before.len());
    assert_eq!(snapshot(w.slug.clone()), before);
}

fn desc_query(slug: &str) -> IndexedQuery {
    IndexedQuery {
        table_id: "t1".to_string(),
        clauses: vec![],
        sort: Some(IndexedSort { field: "amount".to_string(), column: scaffoldry_engine::IndexColumn::Num, descending: true }),
        after: None,
        limit: 201,
        app_slug: slug.to_string(),
        scope: None,
        search: None,
        expr: None,
        offset: 0,
    }
}

// 3. Sort descending by an indexed number: three pages, 600 distinct ids in order, and no Seq Scan.
#[tokio::test]
async fn sorting_descending_by_an_indexed_number_pages_by_cursor_and_never_scans_the_table() {
    let _db = common::TestDb::new();
    let w = world().await;
    fill(&w.slug, ROWS, 0);

    let mut seen: Vec<String> = Vec::new();
    let mut amounts: Vec<i64> = Vec::new();
    let mut cursor = Value::Null;
    for page in 0..3 {
        let mut args = json!({ "app_slug": w.slug, "table_id": "t1", "sort": { "field_name": "amount", "direction": "Desc" }, "limit": 200 });
        if !cursor.is_null() {
            args["cursor"] = cursor.clone();
        }
        let (err, p) = tool(&w, "list_records", args).await;
        assert!(!err, "page {page}: {p}");
        for r in p["records"].as_array().unwrap() {
            seen.push(r["id"].as_str().unwrap().to_string());
            amounts.push(r["data"]["amount"].as_i64().unwrap());
        }
        cursor = p["next_cursor"].clone();
    }
    assert_eq!(seen.len(), 600);
    assert_eq!(seen.iter().collect::<HashSet<_>>().len(), 600, "distinct ids");
    assert_eq!(amounts[0], ROWS);
    assert!(amounts.windows(2).all(|p| p[0] > p[1]), "descending, in order");
    assert_eq!(amounts[599], ROWS - 599);

    // The statements do not scan either table.
    let repo = &w.state.repository;
    for phase in [IndexPhase::NonBlank, IndexPhase::Blank] {
        let (sql, params) = build_indexed_sql(&desc_query(&w.slug), phase);
        let plan = repo.explain(&sql, &params).unwrap();
        assert!(!plan.contains("Seq Scan"), "{phase:?}:\n{plan}");
    }
    let mut mid = desc_query(&w.slug);
    mid.after = Some((IndexedValue::Num(100_000.0), "rec-x".to_string()));
    let (sql, params) = build_indexed_sql(&mid, IndexPhase::NonBlank);
    let plan = repo.explain(&sql, &params).unwrap();
    assert!(!plan.contains("Seq Scan"), "a page from the middle:\n{plan}");
    assert!(plan.contains("idx_record_index_num"), "the number index is used:\n{plan}");
}

// 4. A thousand blank values: paging to the end returns every record once, blanks last.
#[tokio::test]
async fn blank_values_sort_last_in_both_directions_and_every_record_comes_back_once() {
    let _db = common::TestDb::new();
    let w = world().await;
    let total = 20_000;
    fill(&w.slug, total, 1_000);

    for dir in ["Asc", "Desc"] {
        let mut ids = Vec::new();
        let mut blanks_seen = false;
        let mut cursor = Value::Null;
        for _ in 0..150 {
            let mut args = json!({ "app_slug": w.slug, "table_id": "t1", "sort": { "field_name": "amount", "direction": dir }, "limit": 200 });
            if !cursor.is_null() {
                args["cursor"] = cursor.clone();
            }
            let (err, p) = tool(&w, "list_records", args).await;
            assert!(!err, "{dir}: {p}");
            for r in p["records"].as_array().unwrap() {
                let blank = r["data"].get("amount").is_none();
                assert!(blanks_seen <= blank || !blank, "{dir}: a value after a blank");
                if blanks_seen {
                    assert!(blank, "{dir}: a value came after a blank");
                }
                blanks_seen |= blank;
                ids.push(r["id"].as_str().unwrap().to_string());
            }
            cursor = p["next_cursor"].clone();
            if cursor.is_null() {
                break;
            }
        }
        assert_eq!(ids.len() as i64, total, "{dir}: every record");
        assert_eq!(ids.iter().collect::<HashSet<_>>().len() as i64, total, "{dir}: each once");
        assert!(blanks_seen, "{dir}: the blanks came, last");
    }
}

// 5. A filter on an indexed field returns the predicted rows, and the plan holds no Seq Scan.
#[tokio::test]
async fn a_filter_on_an_indexed_field_returns_the_predicted_rows_without_scanning() {
    let _db = common::TestDb::new();
    let w = world().await;
    fill(&w.slug, ROWS, 0);

    let filter = json!({ "conjunction": "And", "clauses": [{ "id": "c", "field_name": "amount", "operator": "GreaterThan", "value": "199000" }] });
    let mut got = Vec::new();
    let mut cursor = Value::Null;
    loop {
        let mut args = json!({ "app_slug": w.slug, "table_id": "t1", "filter": filter, "limit": 200 });
        if !cursor.is_null() {
            args["cursor"] = cursor.clone();
        }
        let (err, p) = tool(&w, "list_records", args).await;
        assert!(!err, "{p}");
        got.extend(p["records"].as_array().unwrap().iter().map(|r| r["data"]["amount"].as_i64().unwrap()));
        cursor = p["next_cursor"].clone();
        if cursor.is_null() {
            break;
        }
    }
    assert_eq!(got.len(), 1_000, "amounts 199001 to 200000");
    assert_eq!(got.iter().copied().min(), Some(199_001));
    assert_eq!(got.iter().copied().max(), Some(200_000));

    let repo = &w.state.repository;
    let query = IndexedQuery {
        table_id: "t1".to_string(),
        clauses: vec![IndexedClause {
            field: "amount".to_string(),
            column: scaffoldry_engine::IndexColumn::Num,
            multi: false,
            op: FilterOperator::GreaterThan,
            value: "199000".to_string(),
        }],
        sort: None,
        after: None,
        limit: 201,
        app_slug: w.slug.clone(),
        scope: None,
        search: None,
        expr: None,
        offset: 0,
    };
    let (sql, params) = build_indexed_sql(&query, IndexPhase::All);
    let plan = repo.explain(&sql, &params).unwrap();
    assert!(!plan.contains("Seq Scan"), "{plan}");

    // A filter and a sort on different fields: each clause is an EXISTS on the index.
    let both = IndexedQuery {
        sort: Some(IndexedSort { field: "amount".to_string(), column: scaffoldry_engine::IndexColumn::Num, descending: false }),
        ..query
    };
    let (sql, params) = build_indexed_sql(&both, IndexPhase::All);
    let plan2 = repo.explain(&sql, &params).unwrap();
    assert!(!plan2.contains("Seq Scan"), "{plan2}");
}

// 6. A filter on a field that is not indexed, on a big table, is refused and names the field.
#[tokio::test]
async fn a_filter_on_a_field_outside_the_index_is_refused_on_a_big_table_and_names_the_field() {
    let _db = common::TestDb::new();
    let w = world().await;
    fill(&w.slug, ROWS, 0);
    let filter = json!({ "conjunction": "And", "clauses": [{ "id": "c", "field_name": "plain_text", "operator": "Equals", "value": "x" }] });
    let (err, msg) = tool(&w, "list_records", json!({ "app_slug": w.slug, "table_id": "t1", "filter": filter })).await;
    assert!(err, "{msg}");
    let text = msg.as_str().unwrap();
    assert!(text.contains("plain_text") && text.contains("indexed"), "{text}");
    // Contains and Or are the small-table path too.
    let contains = json!({ "conjunction": "And", "clauses": [{ "id": "c", "field_name": "amount", "operator": "Contains", "value": "1" }] });
    let (err, _) = tool(&w, "list_records", json!({ "app_slug": w.slug, "table_id": "t1", "filter": contains })).await;
    assert!(err);
    // A second sort field cannot be asked for.
    let (err, _) = tool(&w, "list_records", json!({ "app_slug": w.slug, "table_id": "t1", "sort": [{ "field_name": "amount", "direction": "Asc" }, { "field_name": "stamp", "direction": "Asc" }] })).await;
    assert!(err, "one sort field");
}

// 7. Approving a manifest that adds a view sorting on a new field queues the build of its index rows.
#[tokio::test]
async fn approving_a_view_that_sorts_on_a_new_field_gets_its_index_rows_from_a_job() {
    let _db = common::TestDb::new();
    let w = world().await;
    // Make the app live through the proposals path, so the stored manifest exists.
    let live = manifest(&w.slug, vec![]);
    let (s, body) = send(&w.app, "POST", &format!("/api/v1/workspaces/{WORKSPACE}/apps"), &w.token, Some(serde_json::to_value(&live).unwrap())).await;
    assert!(s == StatusCode::CREATED || s == StatusCode::CONFLICT, "{s} {body}");
    fill(&w.slug, ROWS, 0);
    assert_eq!(index_rows(&w.slug, "score"), 0, "score is not indexed yet");

    let mut view = AppView::table("by-score", "By score", ViewType::Table, vec![]);
    view.table_id = Some("t1".to_string());
    view.sort_rules = vec![SortRule { id: "s".into(), field_name: "score".into(), direction: SortDirection::Asc }];
    let proposed = manifest(&w.slug, vec![view]);
    let (err, p) = tool(
        &w,
        "propose_app_change",
        json!({ "workspace_id": WORKSPACE, "manifest": serde_json::to_value(&proposed).unwrap(), "summary": "Sort by score" }),
    )
    .await;
    assert!(!err, "{p}");
    let started = std::time::Instant::now();
    let (err, done) = tool(&w, "decide_proposal", json!({ "proposal_id": p["proposal_id"], "decision": "approve", "rationale": "ok", "digest": p["digest"] })).await;
    assert!(!err, "{done}");
    println!("row_scale: the approval returned in {:?} on {ROWS} rows", started.elapsed());
    // Since jobs phase 3 the approval only queues the build. A job writes the rows.
    assert_eq!(index_rows(&w.slug, "score"), 0, "the approval copies nothing");
    let repo = scaffoldry_server::repository::PostgresRepository::connect(None).unwrap();
    let runner = scaffoldry_server::jobs::JobRunner::start(
        &repo,
        scaffoldry_server::jobs::JOB_KINDS,
        scaffoldry_server::jobs::JobConfig { workers: 2, poll_ms: 20, ..Default::default() },
    )
    .unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(120);
    while index_rows(&w.slug, "score") < ROWS {
        assert!(std::time::Instant::now() < deadline, "the build job did not finish");
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    runner.stop();
    assert_eq!(index_rows(&w.slug, "score"), ROWS, "the field joined the set, so the job wrote its rows");
    assert_eq!(index_rows(&w.slug, "amount"), ROWS, "a field that stayed is left alone");

    // And the sort now takes the index.
    let (err, page) = tool(&w, "list_records", json!({ "app_slug": w.slug, "table_id": "t1", "sort": { "field_name": "score", "direction": "Desc" }, "limit": 3 })).await;
    assert!(!err, "{page}");
    assert_eq!(page["records"][0]["data"]["score"], ROWS);
}
