//! MCP apps phase 2: an agent reads and writes records through six data tools, a page at a time.
//!
//! The app is private to each run, so the tests share the database without touching anyone's rows.

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::Router;
use http_body_util::BodyExt;
use scaffoldry_engine::{AppManifest, AppTable, AppView, FieldSpec, FieldType, ViewType};
use scaffoldry_server::build_app_with_state;
use scaffoldry_server::service::identity::issue_test_token_and_user;
use scaffoldry_server::state::ServerState;
use serde_json::{json, Value};
use std::collections::HashSet;
use std::sync::Arc;
use tower::ServiceExt;
use uuid::Uuid;


const OWNER: &str = "prof.curie@science.state.edu";
const VIEWER: &str = "marcus.vance@state.edu";
const WORKSPACE: &str = "ws-bio-lab";

fn db_url() -> String {
    scaffoldry_server::testing::database_url()
}

/// Direct database work on its own thread. The synchronous client cannot start inside the async
/// test runtime.
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

struct Fixture {
    app: Router,
    slug: String,
    owner: String,
    viewer: String,
    _cleanup: Cleanup,
}

/// An app with one table, `t1`, in the biology workspace. `ssn` is marked FERPA sensitive.
async fn fixture() -> Fixture {
    let state = Arc::new(ServerState::new().expect("state"));
    state.seed_demo().expect("seed");
    let slug = format!("tools-{}", &Uuid::new_v4().to_string()[..8]);
    let fields = || {
        vec![
            FieldSpec::simple("title", "Title", FieldType::Text, false, false),
            FieldSpec::simple("score", "Score", FieldType::Number, false, false),
            FieldSpec::simple("ssn", "SSN", FieldType::Text, false, true),
        ]
    };
    state
        .engine
        .write()
        .unwrap()
        .register_manifest(AppManifest {
            slug: slug.clone(),
            title: "Tools".to_string(),
            description: String::new(),
            organization_code: "BIO".to_string(),
            department: "biology".to_string(),
            workspace_id: Some(WORKSPACE.to_string()),
            herm_capability_id: None,
            custom_domain: None,
            custom_domain_verified: false,
            views: vec![AppView::table("main", "Main", ViewType::Table, fields())],
            pages: vec![],
        access: Default::default(),
            ceds_mappings: Default::default(),
            tables: vec![AppTable {
                id: "t1".to_string(),
                name: "Things".to_string(),
                slug: "things".to_string(),
                description: None,
                icon: None,
                fields: fields(),
                primary_field: Some("title".to_string()),
                sample_records: vec![],
            }],
            relationships: vec![],
        })
        .unwrap();
    let app = build_app_with_state(state).expect("router");
    Fixture {
        app,
        owner: issue_test_token_and_user(OWNER),
        viewer: issue_test_token_and_user(VIEWER),
        _cleanup: Cleanup(slug.clone()),
        slug,
    }
}

/// Calls a tool. Returns `(is_error, payload)`: the structured result, or the error text.
async fn tool(app: &Router, token: &str, name: &str, args: Value) -> (bool, Value) {
    let payload = json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": { "name": name, "arguments": args } });
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/mcp")
                .header("authorization", format!("Bearer {token}"))
                .header("content-type", "application/json")
                .body(Body::from(payload.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let v: Value = serde_json::from_slice(&bytes).unwrap();
    if let Some(err) = v.get("error") {
        return (true, err["message"].clone());
    }
    let result = &v["result"];
    if result["isError"] == true {
        return (true, result["content"][0]["text"].clone());
    }
    (false, result["structuredContent"].clone())
}

/// Fills the table with `n` rows through SQL, `created_at` ten rows to the millisecond.
fn fill(slug: &str, n: i64) {
    let slug = slug.to_string();
    db(move |c| {
        c.execute(
            "INSERT INTO dataset_records (id, app_slug, data, ceds_mapping, is_ferpa_sensitive, table_id, created_at) \
             SELECT 'rec-' || $1 || '-' || lpad(g::text, 6, '0'), $1, \
                    jsonb_build_object('_table_id', 't1', 'title', 'row ' || g, 'score', g, 'ssn', '000-00-' || lpad(g::text, 4, '0')), \
                    '{}'::jsonb, false, 't1', \
                    TIMESTAMPTZ '2026-01-01 00:00:00+00' + (g / 10) * interval '1 millisecond' \
             FROM generate_series(1, $2::bigint) AS g",
            &[&slug, &n],
        )
        .expect("fill");
    });
}

fn ids(page: &Value) -> Vec<String> {
    page["records"].as_array().unwrap().iter().map(|r| r["id"].as_str().unwrap().to_string()).collect()
}

// 1. 450 records, limit 200: 200, 200, 50 and no row twice.
#[tokio::test]
async fn four_hundred_fifty_records_come_back_in_pages_of_200_200_50_with_no_row_twice() {
    let _db = common::TestDb::new();
    let f = fixture().await;
    fill(&f.slug, 450);

    let mut seen = HashSet::new();
    let mut sizes = vec![];
    let mut cursor = Value::Null;
    loop {
        let mut args = json!({ "app_slug": f.slug, "table_id": "t1", "limit": 200 });
        if !cursor.is_null() {
            args["cursor"] = cursor.clone();
        }
        let (err, page) = tool(&f.app, &f.owner, "list_records", args).await;
        assert!(!err, "{page}");
        sizes.push(page["records"].as_array().unwrap().len());
        for id in ids(&page) {
            assert!(seen.insert(id), "a row came back twice");
        }
        cursor = page["next_cursor"].clone();
        if cursor.is_null() {
            break;
        }
        assert!(sizes.len() < 5, "the pages do not end");
    }
    assert_eq!(sizes, vec![200, 200, 50]);
    assert_eq!(seen.len(), 450);
}

#[tokio::test]
async fn a_limit_above_200_is_a_tool_error_and_every_row_carries_id_and_version() {
    let _db = common::TestDb::new();
    let f = fixture().await;
    fill(&f.slug, 5);
    let (err, msg) = tool(&f.app, &f.owner, "list_records", json!({ "app_slug": f.slug, "table_id": "t1", "limit": 201 })).await;
    assert!(err, "{msg}");
    assert!(msg.as_str().unwrap().contains("limit"), "{msg}");

    // With no limit the default is a bounded page, and each row names its version.
    let (err, page) = tool(&f.app, &f.owner, "list_records", json!({ "app_slug": f.slug, "table_id": "t1" })).await;
    assert!(!err, "{page}");
    let row = &page["records"][0];
    assert!(row["id"].is_string());
    assert_eq!(row["version"], 1);
}

// 2. A filter on a field that is not in the manifest is a tool error.
#[tokio::test]
async fn a_filter_on_a_field_the_manifest_does_not_have_is_a_tool_error() {
    let _db = common::TestDb::new();
    let f = fixture().await;
    fill(&f.slug, 5);
    let hostile = "title'; DROP TABLE dataset_records; --";
    for field in ["nope", hostile] {
        let filter = json!({ "conjunction": "And", "clauses": [{ "id": "c", "field_name": field, "operator": "Equals", "value": "x" }] });
        let (err, msg) = tool(&f.app, &f.owner, "list_records", json!({ "app_slug": f.slug, "table_id": "t1", "filter": filter })).await;
        assert!(err, "{msg}");
        assert!(msg.as_str().unwrap().contains("not a field"), "{msg}");
    }
    let sort = json!({ "field_name": "nope", "direction": "Asc" });
    let (err, msg) = tool(&f.app, &f.owner, "list_records", json!({ "app_slug": f.slug, "table_id": "t1", "sort": sort })).await;
    assert!(err, "{msg}");
    // The table is still there.
    let n: i64 = db(|c| c.query_one("SELECT count(*) FROM dataset_records", &[]).unwrap().get(0));
    assert!(n >= 5);
}

#[tokio::test]
async fn filter_and_sort_work_on_a_manifest_field_and_page_by_cursor() {
    let _db = common::TestDb::new();
    let f = fixture().await;
    fill(&f.slug, 30);

    // score > 20, sorted by score descending: 30 down to 21, ten rows, in two pages of six and four.
    let filter = json!({ "conjunction": "And", "clauses": [{ "id": "c", "field_name": "score", "operator": "GreaterThan", "value": "20" }] });
    let sort = json!({ "field_name": "score", "direction": "Desc" });
    let (err, first) = tool(&f.app, &f.owner, "list_records", json!({ "app_slug": f.slug, "table_id": "t1", "filter": filter, "sort": sort, "limit": 6 })).await;
    assert!(!err, "{first}");
    let scores: Vec<i64> = first["records"].as_array().unwrap().iter().map(|r| r["data"]["score"].as_i64().unwrap()).collect();
    assert_eq!(scores, vec![30, 29, 28, 27, 26, 25], "numbers sort as numbers, not as text");
    let (err, second) = tool(
        &f.app,
        &f.owner,
        "list_records",
        json!({ "app_slug": f.slug, "table_id": "t1", "filter": filter, "sort": sort, "limit": 6, "cursor": first["next_cursor"] }),
    )
    .await;
    assert!(!err, "{second}");
    let scores: Vec<i64> = second["records"].as_array().unwrap().iter().map(|r| r["data"]["score"].as_i64().unwrap()).collect();
    assert_eq!(scores, vec![24, 23, 22, 21]);
    assert!(second["next_cursor"].is_null());

    // Contains is case-insensitive.
    let contains = json!({ "conjunction": "And", "clauses": [{ "id": "c", "field_name": "title", "operator": "Contains", "value": "ROW 1" }] });
    let (err, page) = tool(&f.app, &f.owner, "list_records", json!({ "app_slug": f.slug, "table_id": "t1", "filter": contains })).await;
    assert!(!err, "{page}");
    // row 1, row 10..19
    assert_eq!(page["records"].as_array().unwrap().len(), 11);
}

#[tokio::test]
async fn a_filter_or_sort_on_a_table_over_10000_rows_is_refused_with_the_stated_text() {
    let _db = common::TestDb::new();
    let f = fixture().await;
    fill(&f.slug, 10_001);
    let sort = json!({ "field_name": "score", "direction": "Asc" });
    let (err, msg) = tool(&f.app, &f.owner, "list_records", json!({ "app_slug": f.slug, "table_id": "t1", "sort": sort })).await;
    assert!(err);
    assert!(msg.as_str().unwrap().contains("This table is too large to filter or sort on a field that is not indexed."), "{msg}");
    // The plain paging still works.
    let (err, page) = tool(&f.app, &f.owner, "list_records", json!({ "app_slug": f.slug, "table_id": "t1", "limit": 5 })).await;
    assert!(!err, "{page}");
}

// 3. A stale version is a tool error and the row is unchanged.
#[tokio::test]
async fn update_with_a_stale_version_is_a_tool_error_and_the_row_is_unchanged() {
    let _db = common::TestDb::new();
    let f = fixture().await;
    fill(&f.slug, 3);
    let (_, page) = tool(&f.app, &f.owner, "list_records", json!({ "app_slug": f.slug, "table_id": "t1" })).await;
    let id = page["records"][0]["id"].as_str().unwrap().to_string();

    let (err, saved) = tool(&f.app, &f.owner, "update_record", json!({ "app_slug": f.slug, "record_id": id, "version": 1, "data": { "title": "Winner" } })).await;
    assert!(!err, "{saved}");
    assert_eq!(saved["version"], 2);
    assert_eq!(saved["data"]["title"], "Winner");

    let (err, msg) = tool(&f.app, &f.owner, "update_record", json!({ "app_slug": f.slug, "record_id": id, "version": 1, "data": { "title": "Loser" } })).await;
    assert!(err);
    assert!(msg.as_str().unwrap().contains("current version is 2"), "{msg}");

    let (_, row) = tool(&f.app, &f.owner, "get_record", json!({ "app_slug": f.slug, "record_id": id })).await;
    assert_eq!(row["data"]["title"], "Winner");
    assert_eq!(row["version"], 2);
}

// 4. A FERPA field is absent for a viewer.
#[tokio::test]
async fn a_ferpa_field_is_absent_from_every_read_for_a_viewer_and_present_for_an_editor() {
    let _db = common::TestDb::new();
    let f = fixture().await;
    fill(&f.slug, 3);

    let (err, page) = tool(&f.app, &f.viewer, "list_records", json!({ "app_slug": f.slug, "table_id": "t1" })).await;
    assert!(!err, "{page}");
    for row in page["records"].as_array().unwrap() {
        assert!(row["data"].get("ssn").is_none(), "the viewer saw ssn: {row}");
        assert!(row["data"].get("title").is_some());
    }
    let id = page["records"][0]["id"].as_str().unwrap().to_string();
    let (err, one) = tool(&f.app, &f.viewer, "get_record", json!({ "app_slug": f.slug, "record_id": id })).await;
    assert!(!err, "{one}");
    assert!(one["data"].get("ssn").is_none());

    // A filter cannot be used to probe a field the viewer may not read.
    let probe = json!({ "conjunction": "And", "clauses": [{ "id": "c", "field_name": "ssn", "operator": "Equals", "value": "000-00-0001" }] });
    let (err, msg) = tool(&f.app, &f.viewer, "list_records", json!({ "app_slug": f.slug, "table_id": "t1", "filter": probe })).await;
    assert!(err, "{msg}");

    let (_, page) = tool(&f.app, &f.owner, "list_records", json!({ "app_slug": f.slug, "table_id": "t1" })).await;
    assert!(page["records"][0]["data"].get("ssn").is_some(), "the owner sees it");

    // The REST route is the same service function.
    let resp = f
        .app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/api/v1/apps/{}/records/{}", f.slug, id))
                .header("authorization", format!("Bearer {}", f.viewer))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let rest: Value = serde_json::from_slice(&bytes).unwrap();
    assert!(rest["data"].get("ssn").is_none(), "REST leaked it: {rest}");
}

#[tokio::test]
async fn a_viewer_cannot_write_and_delete_needs_the_version() {
    let _db = common::TestDb::new();
    let f = fixture().await;
    fill(&f.slug, 2);
    let (_, page) = tool(&f.app, &f.owner, "list_records", json!({ "app_slug": f.slug, "table_id": "t1" })).await;
    let id = page["records"][0]["id"].as_str().unwrap().to_string();

    let (err, _) = tool(&f.app, &f.viewer, "update_record", json!({ "app_slug": f.slug, "record_id": id, "version": 1, "data": { "title": "x" } })).await;
    assert!(err, "a viewer may not write");
    let (err, _) = tool(&f.app, &f.viewer, "delete_record", json!({ "app_slug": f.slug, "record_id": id, "version": 1 })).await;
    assert!(err, "a viewer may not delete");

    let (err, msg) = tool(&f.app, &f.owner, "delete_record", json!({ "app_slug": f.slug, "record_id": id, "version": 9 })).await;
    assert!(err, "{msg}");
    assert!(msg.as_str().unwrap().contains("current version is 1"), "{msg}");
    let (err, gone) = tool(&f.app, &f.owner, "delete_record", json!({ "app_slug": f.slug, "record_id": id, "version": 1 })).await;
    assert!(!err, "{gone}");
    assert_eq!(gone["deleted"], true);
    let (err, _) = tool(&f.app, &f.owner, "get_record", json!({ "app_slug": f.slug, "record_id": id })).await;
    assert!(err, "the row is gone");
}

#[tokio::test]
async fn list_apps_and_describe_app_name_the_app_and_its_version() {
    let _db = common::TestDb::new();
    let f = fixture().await;
    let (err, apps) = tool(&f.app, &f.owner, "list_apps", json!({ "workspace_id": WORKSPACE })).await;
    assert!(!err, "{apps}");
    let mine = apps.as_array().unwrap().iter().find(|a| a["slug"] == f.slug.as_str()).expect("listed");
    assert_eq!(mine["title"], "Tools");
    assert_eq!(mine["tables"], json!(["Things"]));

    let (err, described) = tool(&f.app, &f.viewer, "describe_app", json!({ "app_slug": f.slug })).await;
    assert!(!err, "{described}");
    assert_eq!(described["slug"], f.slug.as_str());
    assert!(described["version"].is_number());
    assert!(described["tables"][0]["fields"].is_array());
}

// No caller text reaches the statement. Field names and values are bound parameters.
#[test]
fn the_statement_holds_no_text_the_caller_sent() {
    let _db = common::TestDb::new();
    use scaffoldry_engine::FilterOperator;
    use scaffoldry_server::repository::{build_record_query, RecordQuery, SqlParam};
    let hostile = "x'; DROP TABLE dataset_records; --";
    let q = RecordQuery {
        table_id: "t1".to_string(),
        any: true,
        clauses: vec![
            (hostile.to_string(), FilterOperator::Equals, hostile.to_string()),
            (hostile.to_string(), FilterOperator::GreaterThan, "5".to_string()),
        ],
        sort: Some((hostile.to_string(), true)),
        after: None,
        limit: 10,
        scope: None,
        search: None, expr: None,
    };
    let (sql, params) = build_record_query("app", &q);
    assert!(!sql.contains("DROP") && !sql.contains(hostile), "{sql}");
    assert!(params.contains(&SqlParam::Text(hostile.to_string())));
}

// No tool returns more than 256 KB of rows.
#[tokio::test]
async fn a_page_stops_short_of_256_kb_and_says_where_to_continue() {
    let _db = common::TestDb::new();
    let f = fixture().await;
    let slug = f.slug.clone();
    db(move |c| {
        c.execute(
            "INSERT INTO dataset_records (id, app_slug, data, ceds_mapping, is_ferpa_sensitive, table_id) \
             SELECT 'big-' || $1 || '-' || g, $1, jsonb_build_object('_table_id', 't1', 'title', repeat('x', 20000)), '{}'::jsonb, false, 't1' \
             FROM generate_series(1, 30) g",
            &[&slug],
        )
        .unwrap();
    });
    let (err, page) = tool(&f.app, &f.owner, "list_records", json!({ "app_slug": f.slug, "table_id": "t1", "limit": 200 })).await;
    assert!(!err, "{page}");
    let n = page["records"].as_array().unwrap().len();
    assert!(n < 30 && n > 0, "{n} rows");
    assert!(serde_json::to_string(&page["records"]).unwrap().len() <= 256 * 1024);
    assert!(page["next_cursor"].is_string());
}
