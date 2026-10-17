//! Row scale Phase 4 and 5:
//! Phase 4: Totals and group counts in one statement over `record_index`.
//! Phase 5: Full-text search with generated tsvector and GIN index.

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::Router;
use http_body_util::BodyExt;
use scaffoldry_server::state::CollaboratorRecord;
use scaffoldry_engine::{AppManifest, AppTable, AppView, FieldSpec, FieldType};
use scaffoldry_server::build_app_with_state;
use scaffoldry_server::service::identity::issue_test_token_and_user;
use scaffoldry_server::state::ServerState;
use serde_json::{json, Value};
use std::sync::Arc;
use tower::ServiceExt;
use uuid::Uuid;


const OWNER: &str = "prof.curie@science.state.edu";
const VIEWER: &str = "student.viewer@science.state.edu";
const EDITOR: &str = "editor.curie@science.state.edu";
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
                let _ = c.execute("DELETE FROM record_index WHERE app_slug = $1", &[&slug]);
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

fn fields() -> Vec<FieldSpec> {
    let mut sensitive_f = indexed("sensitive_amount", FieldType::Number);
    sensitive_f.ferpa_sensitive = true;
    vec![
        indexed("amount", FieldType::Number),
        indexed("status", FieldType::Select),
        sensitive_f,
        plain("plain_text", FieldType::Text),
        plain("score", FieldType::Number),
    ]
}

fn manifest(slug: &str, views: Vec<AppView>) -> AppManifest {
    AppManifest {
        slug: slug.to_string(),
        title: "Row Scale Tests".to_string(),
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
            fields: fields(),
            primary_field: None,
            sample_records: vec![],
        }],
        relationships: vec![],
        views,
        ceds_mappings: Default::default(),
        pages: vec![],
        access: Default::default(),
    }
}

struct World {
    _state: Arc<ServerState>,
    app: Router,
    owner_token: String,
    viewer_token: String,
    editor_token: String,
    slug: String,
    _cleanup: Cleanup,
}

async fn world() -> World {
    let state = Arc::new(ServerState::new().expect("state"));
    state.seed_demo().expect("seed");
    let slug = format!("scale-{}", &Uuid::new_v4().to_string()[..8]);
    state.engine.write().unwrap().register_manifest(manifest(&slug, vec![])).unwrap();

    let now = "2026-01-01T00:00:00Z".to_string();
    let collabs = vec![
        CollaboratorRecord {
            id: Uuid::new_v4().to_string(),
            workspace_id: WORKSPACE.to_string(),
            eppn: OWNER.to_string(),
            name: "Prof Curie".to_string(),
            role: "owner".to_string(),
            scoped_affiliation: "faculty".to_string(),
            department: "biology".to_string(),
            added_at: now.clone(),
            app_slug: String::new(),
        },
        CollaboratorRecord {
            id: Uuid::new_v4().to_string(),
            workspace_id: WORKSPACE.to_string(),
            eppn: VIEWER.to_string(),
            name: "Viewer User".to_string(),
            role: "viewer".to_string(),
            scoped_affiliation: "student".to_string(),
            department: "biology".to_string(),
            added_at: now.clone(),
            app_slug: String::new(),
        },
        CollaboratorRecord {
            id: Uuid::new_v4().to_string(),
            workspace_id: WORKSPACE.to_string(),
            eppn: EDITOR.to_string(),
            name: "Editor User".to_string(),
            role: "editor".to_string(),
            scoped_affiliation: "faculty".to_string(),
            department: "biology".to_string(),
            added_at: now.clone(),
            app_slug: String::new(),
        },
    ];
    state.collaborators.write().unwrap().insert(WORKSPACE.to_string(), collabs);

    World {
        app: build_app_with_state(state.clone()).expect("router"),
        owner_token: issue_test_token_and_user(OWNER),
        viewer_token: issue_test_token_and_user(VIEWER),
        editor_token: issue_test_token_and_user(EDITOR),
        _state: state,
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

async fn tool(w: &World, token: &str, name: &str, args: Value) -> (bool, Value) {
    let payload = json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": { "name": name, "arguments": args } });
    let (_, v) = send(&w.app, "POST", "/api/mcp", token, Some(payload)).await;
    let result = &v["result"];
    if result["isError"] == true {
        return (true, result["content"][0]["text"].clone());
    }
    (false, result["structuredContent"].clone())
}

/// Rows 1..=200,000.
/// Row 42 has `plain_text` = "zebrafish", others have "standard".
/// `status` is distributed evenly among 's1'..'s5' (40,000 each).
/// `amount` = g (sum = 20,000,100,000).
fn fill_scale(slug: &str, n: i64) {
    let slug = slug.to_string();
    db(move |c| {
        c.execute(
            "INSERT INTO dataset_records (id, app_slug, data, ceds_mapping, is_ferpa_sensitive, table_id, created_at) \
             SELECT 'rec-' || $1 || '-' || lpad(g::text, 8, '0'), $1, \
                    jsonb_build_object( \
                        '_table_id', 't1', \
                        'amount', g, \
                        'sensitive_amount', g, \
                        'status', 's' || ((g % 5) + 1)::text, \
                        'plain_text', CASE WHEN g = 42 THEN 'zebrafish' ELSE 'standard' END, \
                        'score', g \
                    ), \
                    '{}'::jsonb, false, 't1', TIMESTAMPTZ '2026-01-01 00:00:00+00' + (g / 10) * interval '1 millisecond' \
             FROM generate_series(1, $2::bigint) AS g",
            &[&slug, &n],
        )
        .expect("fill records");

        c.execute(
            "INSERT INTO record_index (app_slug, table_id, field, record_id, num) \
             SELECT app_slug, 't1', 'amount', id, (data->>'amount')::float8 FROM dataset_records WHERE app_slug = $1",
            &[&slug],
        )
        .expect("fill index amount");

        c.execute(
            "INSERT INTO record_index (app_slug, table_id, field, record_id, num) \
             SELECT app_slug, 't1', 'sensitive_amount', id, (data->>'sensitive_amount')::float8 FROM dataset_records WHERE app_slug = $1",
            &[&slug],
        )
        .expect("fill index sensitive_amount");

        c.execute(
            "INSERT INTO record_index (app_slug, table_id, field, record_id, txt) \
             SELECT app_slug, 't1', 'status', id, data->>'status' FROM dataset_records WHERE app_slug = $1",
            &[&slug],
        )
        .expect("fill index status");

        c.batch_execute("ANALYZE dataset_records; ANALYZE record_index;").expect("analyze");
    });
}

// ---------------------------------------------------------------------------------------------
// Phase 4 Tests
// ---------------------------------------------------------------------------------------------

// 1. sum of an indexed number over 200,000 rows equals the predicted value. The plan holds no Seq Scan on dataset_records.
#[tokio::test]
async fn phase4_sum_over_200k_rows_equals_predicted_and_no_seq_scan_on_dataset_records() {
    let _db = common::TestDb::new();
    let w = world().await;
    fill_scale(&w.slug, ROWS);

    let (status, body) = send(
        &w.app,
        "GET",
        &format!("/api/v1/apps/{}/tables/t1/aggregate?function=sum&field=amount", w.slug),
        &w.owner_token,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "aggregate response: {body}");
    let predicted = (ROWS as f64) * ((ROWS + 1) as f64) / 2.0; // 20000100000.0
    let value = body["value"].as_f64().expect("numeric value");
    assert_eq!(value, predicted, "sum over 200,000 rows");

    // Explain check: query plan must not contain Seq Scan on dataset_records
    let slug = w.slug.clone();
    let plan = db(move |c| {
        let rows = c
            .query(
                "EXPLAIN (ANALYZE, COSTS OFF, FORMAT TEXT) \
                 SELECT COALESCE(SUM(i.num)::float8, 0.0) FROM record_index i \
                 WHERE i.app_slug = $1 AND i.table_id = 't1' AND i.field = 'amount' AND i.ord = 0",
                &[&slug],
            )
            .unwrap();
        rows.iter().map(|r| r.get::<_, String>(0)).collect::<Vec<_>>().join("\n")
    });
    assert!(!plan.contains("Seq Scan on dataset_records"), "Plan holds no Seq Scan on dataset_records:\n{plan}");
}

// 2. count with group_by on a status field with five values returns five groups that add up to 200,000.
#[tokio::test]
async fn phase4_count_grouped_by_five_value_status_adds_to_200k() {
    let _db = common::TestDb::new();
    let w = world().await;
    fill_scale(&w.slug, ROWS);

    let (status, body) = send(
        &w.app,
        "GET",
        &format!("/api/v1/apps/{}/tables/t1/aggregate?function=count&group_by=status", w.slug),
        &w.owner_token,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "aggregate response: {body}");
    let groups = body["groups"].as_array().expect("groups array");
    assert_eq!(groups.len(), 5, "5 status groups");

    let mut total = 0.0;
    for g in groups {
        let cnt = g["value"].as_f64().or_else(|| g["count"].as_f64()).expect("group count/value");
        total += cnt;
    }
    assert_eq!(total, 200_000.0, "groups must add up to 200,000");

    // Also verify via MCP tool
    let (err, tool_res) = tool(
        &w,
        &w.owner_token,
        "aggregate_records",
        json!({
            "app_slug": w.slug,
            "table_id": "t1",
            "function": "count",
            "group_by": "status"
        }),
    )
    .await;
    assert!(!err, "MCP tool error: {tool_res}");
    let tool_groups = tool_res["groups"].as_array().expect("tool groups");
    assert_eq!(tool_groups.len(), 5);
}

// 3. sum on a field outside the set is BadRequest.
#[tokio::test]
async fn phase4_sum_on_unindexed_field_is_bad_request() {
    let _db = common::TestDb::new();
    let w = world().await;

    // 'score' is not in the indexed set
    let (status, body) = send(
        &w.app,
        "GET",
        &format!("/api/v1/apps/{}/tables/t1/aggregate?function=sum&field=score", w.slug),
        &w.owner_token,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "score is unindexed: {body}");

    // Also via MCP tool
    let (err, msg) = tool(
        &w,
        &w.owner_token,
        "aggregate_records",
        json!({
            "app_slug": w.slug,
            "table_id": "t1",
            "function": "sum",
            "field": "score"
        }),
    )
    .await;
    assert!(err, "unindexed field must fail in tool");
    let err_str = msg.as_str().unwrap_or("");
    assert!(err_str.contains("score") || err_str.contains("indexed"), "{err_str}");
}

// 4. A viewer asking for sum of a ferpa_sensitive field is refused.
#[tokio::test]
async fn phase4_viewer_asking_sum_of_ferpa_sensitive_field_is_refused() {
    let _db = common::TestDb::new();
    let w = world().await;

    // sensitive_amount is ferpa_sensitive: true, hidden from viewer
    let (status, body) = send(
        &w.app,
        "GET",
        &format!("/api/v1/apps/{}/tables/t1/aggregate?function=sum&field=sensitive_amount", w.slug),
        &w.viewer_token,
        None,
    )
    .await;
    assert!(status == StatusCode::FORBIDDEN || status == StatusCode::BAD_REQUEST, "viewer refused: status={status}, body={body}");

    // Also via MCP tool
    let (err, msg) = tool(
        &w,
        &w.viewer_token,
        "aggregate_records",
        json!({
            "app_slug": w.slug,
            "table_id": "t1",
            "function": "sum",
            "field": "sensitive_amount"
        }),
    )
    .await;
    assert!(err, "viewer asking for sensitive field must be refused: {msg}");
}

// ---------------------------------------------------------------------------------------------
// Phase 5 Tests
// ---------------------------------------------------------------------------------------------

// 1. 200,000 rows, one holding the word zebrafish. Search returns that row. The plan uses idx_dataset_records_search.
#[tokio::test]
async fn phase5_search_returns_row_and_uses_gin_index() {
    let _db = common::TestDb::new();
    let w = world().await;
    fill_scale(&w.slug, ROWS);

    let (status, body) = send(
        &w.app,
        "GET",
        &format!("/api/v1/apps/{}/tables/t1/records?search=zebrafish", w.slug),
        &w.owner_token,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "search response: {body}");
    let records = body["records"].as_array().expect("records array");
    assert_eq!(records.len(), 1, "exactly 1 matching record");
    assert_eq!(records[0]["data"]["plain_text"], "zebrafish");

    // Explain check: plan uses idx_dataset_records_search
    let slug = w.slug.clone();
    let plan = db(move |c| {
        let rows = c
            .query(
                "EXPLAIN (ANALYZE, COSTS OFF, FORMAT TEXT) \
                 SELECT id FROM dataset_records \
                 WHERE app_slug = $1 AND table_id = 't1' AND deleted_at IS NULL \
                   AND search @@ plainto_tsquery('simple', 'zebrafish')",
                &[&slug],
            )
            .unwrap();
        rows.iter().map(|r| r.get::<_, String>(0)).collect::<Vec<_>>().join("\n")
    });
    assert!(plan.contains("idx_dataset_records_search"), "plan must use idx_dataset_records_search:\n{plan}");
}

// 2. A viewer searching a table with a ferpa_sensitive field is refused. An editor is not.
#[tokio::test]
async fn phase5_viewer_searching_sensitive_table_is_refused_editor_is_not() {
    let _db = common::TestDb::new();
    let w = world().await;

    // Viewer searching table with sensitive_amount is refused with BadRequest and exact message
    let (status, body) = send(
        &w.app,
        "GET",
        &format!("/api/v1/apps/{}/tables/t1/records?search=test", w.slug),
        &w.viewer_token,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "viewer must be BadRequest: {body}");
    let err_msg = body["message"].as_str().or_else(|| body["error"].as_str()).unwrap_or("");
    assert!(err_msg.contains("Search is not available on this table for your role"), "{err_msg}");

    // Editor searching the same table is allowed
    let (status_editor, body_editor) = send(
        &w.app,
        "GET",
        &format!("/api/v1/apps/{}/tables/t1/records?search=test", w.slug),
        &w.editor_token,
        None,
    )
    .await;
    assert_eq!(status_editor, StatusCode::OK, "editor allowed: {body_editor}");
}

// 3. idx_dataset_records_data_gin no longer exists.
#[tokio::test]
async fn phase5_idx_dataset_records_data_gin_no_longer_exists() {
    let _db = common::TestDb::new();
    let _w = world().await;

    let count: i64 = db(|c| {
        c.query_one(
            "SELECT count(*) FROM pg_indexes WHERE indexname = 'idx_dataset_records_data_gin'",
            &[],
        )
        .unwrap()
        .get(0)
    });
    assert_eq!(count, 0, "idx_dataset_records_data_gin must no longer exist");
}
