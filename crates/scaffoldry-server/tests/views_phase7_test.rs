//! Views phase 7: forms, boards, and calendars.
//!
//! A form view is a subset of a table's fields, in an order, with conditions that show a field. The server runs the
//! conditions at submit and is authoritative. A board is a view grouped by a single select, paged a column at a time. A
//! calendar fetches a date range through the index and shows at most 500 events.

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::Router;
use http_body_util::BodyExt;
use scaffoldry_engine::{AppManifest, AppTable, FieldSpec, FieldType, SelectOption, SelectOptionColor};
use scaffoldry_engine::IndexColumn;
use scaffoldry_server::build_app_with_state;
use scaffoldry_server::repository_index::{build_indexed_sql, Expr, IndexPhase, IndexedClause, IndexedQuery, IndexedSort};
use scaffoldry_server::service::identity::issue_test_token_and_user;
use scaffoldry_server::state::{CollaboratorRecord, ServerState};
use serde_json::{json, Value};
use std::sync::Arc;
use tower::ServiceExt;
use uuid::Uuid;

const WORKSPACE: &str = "ws-bio-lab";

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
        let (tag, url) = (self.0.clone(), db_url());
        let _ = std::thread::spawn(move || {
            if let Ok(mut c) = postgres::Client::connect(&url, postgres::NoTls) {
                let like = format!("%{tag}%");
                let _ = c.execute("DELETE FROM jobs WHERE payload->>'app_slug' LIKE $1", &[&like]);
                let _ = c.execute("DELETE FROM index_status WHERE app_slug LIKE $1", &[&like]);
                let _ = c.execute("DELETE FROM record_index WHERE app_slug LIKE $1", &[&like]);
                let _ = c.execute("DELETE FROM app_views WHERE app_slug LIKE $1", &[&like]);
                let _ = c.execute("DELETE FROM workspace_collaborators WHERE workspace_id = $1 AND eppn LIKE $2", &[&WORKSPACE, &like]);
                let _ = c.execute("DELETE FROM record_history WHERE app_slug LIKE $1", &[&like]);
                let _ = c.execute("DELETE FROM dataset_records WHERE app_slug LIKE $1", &[&like]);
                let _ = c.execute("DELETE FROM app_manifests WHERE slug LIKE $1", &[&like]);
            }
        })
        .join();
    }
}

fn select(name: &str, options: &[&str]) -> FieldSpec {
    let mut f = FieldSpec::simple(name, name, FieldType::Select, false, false);
    f.select_options = options
        .iter()
        .enumerate()
        .map(|(i, id)| SelectOption { id: id.to_string(), label: id.to_string(), color: SelectOptionColor::Neutral, order: i as i32 })
        .collect();
    f
}

/// A field the table indexes. The test fills its index rows by hand, so no build job has to run.
fn indexed(mut f: FieldSpec) -> FieldSpec {
    f.indexed = true;
    f
}

/// Students with a program, an advisor, a stage and a gpa. With `many_indexed`, twenty more fields are
/// marked indexed and a twenty-first is not.
fn manifest(slug: &str) -> AppManifest {
    let students = vec![
        FieldSpec::simple("name", "Name", FieldType::Text, false, false),
        indexed(select("program", &["p0", "p1", "p2", "p3", "p4"])),
        indexed(FieldSpec::simple("due", "Due", FieldType::Date, false, false)),
    ];
    let requests = vec![
        select("kind", &["grant", "loan"]),
        FieldSpec::simple("amount", "Amount", FieldType::Number, true, false),
        FieldSpec::simple("note", "Note", FieldType::Text, false, false),
    ];
    let table = |id: &str, fields: Vec<FieldSpec>, primary: &str| AppTable {
        retired_fields: Vec::new(),
        id: id.to_string(),
        name: id.to_string(),
        slug: id.to_string(),
        description: None,
        icon: None,
        fields,
        primary_field: Some(primary.to_string()),
        sample_records: vec![],
        coordinates: None, sealed: false,
        row_place: Default::default(),
    };
    AppManifest {
        slug: slug.to_string(),
        title: "Registrar".to_string(),
        description: String::new(),
        organization_code: "BIO".to_string(),
        department: "biology".to_string(),
        workspace_id: Some(WORKSPACE.to_string()),
        herm_capability_id: None,
        coordinates: None, sealed: false, integrations: Default::default(), kind: Default::default(),
        custom_domain: None,
        custom_domain_verified: false,
        tables: vec![table("students", students, "name"), table("requests", requests, "kind")],
        relationships: vec![],
        views: vec![],
        ceds_mappings: Default::default(),
        pages: vec![],
        access: Default::default(),
    }
}

fn collaborator(state: &ServerState, eppn: &str, role: &str) {
    state.collaborators.write().unwrap().entry(WORKSPACE.to_string()).or_default().push(CollaboratorRecord {
        id: Uuid::new_v4().to_string(),
        workspace_id: WORKSPACE.to_string(),
        eppn: eppn.to_string(),
        name: eppn.to_string(),
        role: role.to_string(),
        scoped_affiliation: "faculty".to_string(),
        department: "biology".to_string(),
        added_at: String::new(),
        app_slug: String::new(),
    });
}

struct World {
    app: Router,
    state: Arc<ServerState>,
    slug: String,
    admin: String,
    viewer: String,
    _cleanup: Cleanup,
}

/// `march` students due in March 2026, and `april` due on 5 April. Every fifth is in each of five programs.
async fn world(march: i64, april: i64) -> World {
    let tag = Uuid::new_v4().to_string()[..8].to_string();
    let slug = format!("av-{tag}");
    let (admin_e, editor_e, viewer_e) = (format!("admin-{tag}@state.edu"), format!("editor-{tag}@state.edu"), format!("viewer-{tag}@state.edu"));
    let state = Arc::new(ServerState::new().expect("state"));
    state.seed_demo().expect("seed");
    state.engine.write().unwrap().register_manifest(manifest(&slug)).unwrap();
    collaborator(&state, &admin_e, "admin");
    collaborator(&state, &editor_e, "editor");
    collaborator(&state, &viewer_e, "viewer");

    let s = slug.clone();
    db(move |c| {
        c.execute(
            "INSERT INTO dataset_records (id, app_slug, data, ceds_mapping, is_ferpa_sensitive, table_id, created_at) \
             SELECT $1::text || lpad(g::text, 8, '0'), $1::text, \
                    jsonb_build_object('_table_id', 'students', 'name', 'student-' || g, 'program', 'p' || (g % 5), \
                                       'due', to_char(CASE WHEN g <= $2::bigint THEN TIMESTAMPTZ '2026-03-01 00:00:00+00' + (g % 28) * interval '1 day' ELSE TIMESTAMPTZ '2026-04-05 00:00:00+00' END, 'YYYY-MM-DD\"T\"HH24:MI:SS\"Z\"')), \
                    '{}'::jsonb, false, 'students', TIMESTAMPTZ '2026-01-01 00:00:00+00' + g * interval '1 millisecond' \
             FROM generate_series(1, $2::bigint + $3::bigint) g",
            &[&s, &march, &april],
        )
        .unwrap();
        for (field, col, expr) in [("program", "txt", "lower(data->>'program')"), ("due", "ts", "(data->>'due')::timestamptz")] {
            c.execute(
                &format!("INSERT INTO record_index (app_slug, table_id, field, record_id, {col}) SELECT app_slug, 'students', '{field}', id, {expr} FROM dataset_records WHERE app_slug = $1"),
                &[&s],
            )
            .unwrap();
        }
        c.batch_execute("ANALYZE dataset_records").unwrap();
        c.batch_execute("ANALYZE record_index").unwrap();
    });

    World {
        app: build_app_with_state(state.clone()).expect("router"),
        state,
        admin: issue_test_token_and_user(&admin_e),
        viewer: issue_test_token_and_user(&viewer_e),
        slug,
        _cleanup: Cleanup(tag),
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


fn views_path(w: &World) -> String {
    format!("/api/v1/apps/{}/views", w.slug)
}

async fn make(w: &World, name: &str, kind: &str, table: &str, config: Value) -> (StatusCode, Value) {
    send(&w.app, "POST", &views_path(w), &w.admin, Some(json!({ "name": name, "kind": kind, "ownership": "collaborative", "table_id": table, "config": config }))).await
}

fn id_of(v: &Value) -> String {
    v["id"].as_str().or_else(|| v["view"]["id"].as_str()).unwrap().to_string()
}

fn request_form() -> Value {
    json!({
        "form_fields": [
            { "name": "kind", "label": "What kind of help?" },
            { "name": "amount", "label": "How much?", "help": "In dollars", "show_when": [{ "field": "kind", "op": "eq", "value": "grant" }] },
            { "name": "note", "label": "Tell us more", "show_when": [{ "field": "kind", "op": "ne", "value": "grant" }] },
        ],
        "success_message": "Thank you. We have your request.",
    })
}

async fn form(w: &World) -> String {
    let (s, v) = make(w, "Request help", "form", "requests", request_form()).await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    id_of(&v)
}

async fn submit(w: &World, token: &str, view: &str, data: Value) -> (StatusCode, Value) {
    send(&w.app, "POST", &format!("{}/{view}/submit", views_path(w)), token, Some(json!({ "data": data }))).await
}

fn stored(w: &World) -> Vec<Value> {
    let slug = w.slug.clone();
    db(move |c| c.query("SELECT data FROM dataset_records WHERE app_slug = $1 AND table_id = 'requests' ORDER BY created_at", &[&slug]).unwrap().iter().map(|r| r.get(0)).collect())
}

// 1. A hidden required field is not required on submit. The server rejects a submit that violates a condition the client
// skipped. A form is submitted through the ordinary create path, so the ordinary rules apply.
#[tokio::test]
async fn the_server_runs_the_form_conditions_at_submit() {
    let _db = common::TestDb::new();
    let w = world(1, 0).await;
    let view = form(&w).await;

    // `amount` is required by the table. It shows for a grant, so a grant without it is refused.
    let (s, v) = submit(&w, &w.viewer, &view, json!({ "kind": "grant" })).await;
    assert_eq!(s, StatusCode::FORBIDDEN, "a viewer cannot write, as everywhere: {v}");
    let (s, v) = submit(&w, &w.admin, &view, json!({ "kind": "grant" })).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{v}");
    assert!(v.to_string().contains("amount"), "it names the field: {v}");

    // For a loan `amount` is hidden, so its required rule does not apply.
    let (s, v) = submit(&w, &w.admin, &view, json!({ "kind": "loan", "note": "Emergency" })).await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    assert_eq!(v["success_message"], "Thank you. We have your request.");
    assert_eq!(v["record"]["data"]["kind"], "loan");
    assert!(v["record"]["data"].get("amount").is_none(), "{v}");

    // A value for a hidden field is not taken: the client skipped a condition it should have kept.
    let (s, v) = submit(&w, &w.admin, &view, json!({ "kind": "loan", "amount": 5000 })).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{v}");
    assert!(v.to_string().contains("amount") && v.to_string().contains("not shown"), "it says why: {v}");

    // A shown field goes through, and a field the form does not have is refused.
    let (s, v) = submit(&w, &w.admin, &view, json!({ "kind": "grant", "amount": 250 })).await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    let (s, v) = submit(&w, &w.admin, &view, json!({ "kind": "grant", "amount": 250, "program": "p1" })).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{v}");
    assert_eq!(stored(&w).len(), 2, "only the two good submits are stored");
    // A value the table's own rules refuse is refused: `kind` is a select with two options.
    let (s, _) = submit(&w, &w.admin, &view, json!({ "kind": "gift", "amount": 1 })).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
}

// 1b. A form is checked when it is saved: its fields are fields of the table, and a condition names an earlier field.
#[tokio::test]
async fn a_form_that_names_a_missing_field_or_a_later_one_is_refused() {
    let _db = common::TestDb::new();
    let w = world(1, 0).await;
    let bad = |fields: Value| json!({ "form_fields": fields });
    let cases = [
        ("a field the table lacks", bad(json!([{ "name": "ghost" }]))),
        ("a field twice", bad(json!([{ "name": "kind" }, { "name": "kind" }]))),
        ("a condition on a later field", bad(json!([{ "name": "amount", "show_when": [{ "field": "kind", "op": "eq", "value": "grant" }] }, { "name": "kind" }]))),
        ("a condition on itself", bad(json!([{ "name": "kind", "show_when": [{ "field": "kind", "op": "is_empty" }] }]))),
        ("an operator that is not one", bad(json!([{ "name": "kind" }, { "name": "amount", "show_when": [{ "field": "kind", "op": "like", "value": "g" }] }]))),
        ("no fields", bad(json!([]))),
    ];
    for (what, config) in cases {
        let (s, v) = make(&w, "Bad form", "form", "requests", config).await;
        assert_eq!(s, StatusCode::BAD_REQUEST, "{what}: {v}");
    }
}

// 2. A board of 10,000 cards loads only the first page of each column, and the counts match the aggregate.
#[tokio::test]
async fn a_board_pages_each_column_and_counts_match_the_aggregate() {
    let _db = common::TestDb::new();
    let w = world(10_000, 0).await;
    let (s, board) = make(&w, "By program", "kanban", "students", json!({ "kanban_column_field": "program", "column_limits": { "p1": 1500 } })).await;
    assert_eq!(s, StatusCode::CREATED, "{board}");
    let id = id_of(&board);

    let (s, columns) = send(&w.app, "GET", &format!("{}/{id}/data", views_path(&w)), &w.admin, None).await;
    assert_eq!(s, StatusCode::OK, "{columns}");
    let items = columns["groups"]["items"].as_array().unwrap();
    assert_eq!(items.len(), 5, "one column for each option: {columns}");
    assert_eq!(items.iter().map(|g| g["count"].as_f64().unwrap()).sum::<f64>(), 10_000.0);
    let (_, agg) = send(&w.app, "GET", &format!("/api/v1/apps/{}/tables/students/aggregate?function=count&group_by=program", w.slug), &w.admin, None).await;
    for g in items {
        let same = agg["groups"].as_array().unwrap().iter().find(|a| a["key"] == g["key"]).unwrap_or_else(|| panic!("a group in the aggregate: {agg}"));
        assert_eq!(same["value"], g["count"], "the board's count is the aggregate's for {}", g["key"]);
    }

    // A column opens with its first 50 cards and a cursor for the rest. The next page does not repeat them.
    let q = |cursor: &str| format!("{}/{id}/data?group_path=%5B%22p2%22%5D&limit=50{cursor}", views_path(&w));
    let (_, first) = send(&w.app, "GET", &q(""), &w.admin, None).await;
    let cards = first["records"].as_array().unwrap();
    assert_eq!(cards.len(), 50, "only the first page: {}", first.to_string().len());
    assert!(cards.iter().all(|c| c["data"]["program"] == "p2"));
    let cursor = first["next_cursor"].as_str().expect("more cards in the column");
    let (_, second) = send(&w.app, "GET", &q(&format!("&cursor={cursor}")), &w.admin, None).await;
    let ids: std::collections::HashSet<&str> = cards.iter().chain(second["records"].as_array().unwrap()).map(|c| c["id"].as_str().unwrap()).collect();
    assert_eq!(ids.len(), 100, "two pages of fifty, with no card twice");
}

// 4. A calendar month with 2,000 events shows 500 and a count of the rest, and the query reads the index.
#[tokio::test]
async fn a_calendar_range_shows_five_hundred_events_and_counts_the_rest() {
    let _db = common::TestDb::new();
    let w = world(2_000, 100).await;
    let (s, cal) = make(&w, "Due dates", "calendar", "students", json!({ "calendar_date_field": "due" })).await;
    assert_eq!(s, StatusCode::CREATED, "{cal}");
    let id = id_of(&cal);

    let (s, march) = send(&w.app, "GET", &format!("{}/{id}/data?from=2026-03-01&to=2026-03-31", views_path(&w)), &w.admin, None).await;
    assert_eq!(s, StatusCode::OK, "{march}");
    assert_eq!(march["records"].as_array().unwrap().len(), 500, "at most 500 events");
    assert_eq!(march["calendar"], json!({ "shown": 500, "total": 2000, "more": 1500 }), "{march}");
    // The events are in date order, so the page is the start of the month.
    let dates: Vec<&str> = march["records"].as_array().unwrap().iter().map(|r| r["data"]["due"].as_str().unwrap()).collect();
    assert!(dates.windows(2).all(|p| p[0] <= p[1]), "in date order");
    assert!(dates.iter().all(|d| d.starts_with("2026-03")), "only March");

    // A range with a few events shows them all and says there are no more. The April rows are not in March.
    let (_, april) = send(&w.app, "GET", &format!("{}/{id}/data?from=2026-04-01&to=2026-04-30", views_path(&w)), &w.admin, None).await;
    assert_eq!(april["calendar"], json!({ "shown": 100, "total": 100, "more": 0 }), "{april}");
    // A range that is not a range is a bad request.
    let (s, _) = send(&w.app, "GET", &format!("{}/{id}/data?from=2026-04-01", views_path(&w)), &w.admin, None).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    let (s, _) = send(&w.app, "GET", &format!("{}/{id}/data?from=2026-05-01&to=2026-04-01", views_path(&w)), &w.admin, None).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);

    // The statement behind a range reads the index.
    use scaffoldry_engine::FilterOperator::Between;
    let q = IndexedQuery {
        app_slug: w.slug.clone(),
        table_id: "students".into(),
        clauses: vec![],
        sort: Some(IndexedSort { field: "due".into(), column: IndexColumn::Ts, descending: false }),
        after: None,
        limit: 501,
        scope: None,
        search: None,
        expr: Some(Expr::Leaf(IndexedClause { field: "due".into(), column: IndexColumn::Ts, multi: false, op: Between, value: "2026-03-01..2026-03-31".into() })),
        offset: 0,
    };
    let (sql, params) = build_indexed_sql(&q, IndexPhase::All);
    let plan = w.state.repository.explain(&sql, &params).unwrap();
    // The date range is answered by the index on the date column, and no scan reads the whole index table. (The records table
    // is small enough here for the planner to read it whole. At a size where that matters, it fetches by key.)
    assert!(plan.contains("idx_record_index_ts") && !plan.contains("Seq Scan on record_index"), "the range reads the index:\n{plan}");
}
