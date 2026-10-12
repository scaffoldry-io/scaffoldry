//! Views phase 2: filter trees, grouping, footer totals, and color rules.
//!
//! The server filters, groups, totals and tones. The browser never does. A view still cannot widen access:
//! a color rule or a footer that reads a field the person cannot see is dropped for them.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::Router;
use http_body_util::BodyExt;
use scaffoldry_core::access::{ColumnMode, ColumnRule, Who};
use scaffoldry_engine::{AppManifest, AppTable, FieldSpec, FieldType, IndexColumn, SelectOption, SelectOptionColor};
use scaffoldry_server::build_app_with_state;
use scaffoldry_server::repository_index::{build_aggregate_sql, build_indexed_sql, AggregateFunction, AggregatePlan, Expr, IndexPhase, IndexedClause, IndexedQuery, IndexedSort};
use scaffoldry_server::service::identity::issue_test_token_and_user;
use scaffoldry_server::service::scope::set_column_rules;
use scaffoldry_server::state::{CollaboratorRecord, ServerState};
use serde_json::{json, Value};
use std::sync::Arc;
use tokio::sync::Mutex;
use tower::ServiceExt;
use uuid::Uuid;

/// The 200,000-row tests share the machine.
static LOCK: Mutex<()> = Mutex::const_new(());

const OWNER: &str = "prof.curie@science.state.edu";
const VIEWER: &str = "student.viewer@science.state.edu";
const WORKSPACE: &str = "ws-bio-lab";

fn db_url() -> String {
    std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://scaffoldry:scaffoldry_dev_password@127.0.0.1:5433/scaffoldry".to_string())
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

fn select(name: &str, options: &[&str]) -> FieldSpec {
    let mut f = indexed(name, FieldType::Select);
    f.select_options = options
        .iter()
        .enumerate()
        .map(|(i, id)| SelectOption { id: id.to_string(), label: id.to_string(), color: SelectOptionColor::Neutral, order: i as i32 })
        .collect();
    f
}

fn fields() -> Vec<FieldSpec> {
    vec![
        indexed("amount", FieldType::Number),
        select("status", &["s1", "s2", "s3", "s4", "s5"]),
        select("program", &["p0", "p1", "p2"]),
        indexed("due", FieldType::Date),
        FieldSpec::simple("note", "Note", FieldType::Text, false, false),
    ]
}

fn manifest(slug: &str) -> AppManifest {
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
            fields: fields(),
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

/// Rows 1..=`rows`. `amount` is the row number. `status` is s1..s5 and `program` p0..p2 in turn. `due` is the first
/// of January 2026 plus the row number in days. `note` is `n` and the row number, and is not indexed.
async fn world(rows: i64) -> World {
    let slug = format!("vp2-{}", &Uuid::new_v4().to_string()[..8]);
    let state = Arc::new(ServerState::new().expect("state"));
    state.seed_demo().expect("seed");
    state.engine.write().unwrap().register_manifest(manifest(&slug)).unwrap();
    state.collaborators.write().unwrap().insert(WORKSPACE.to_string(), vec![collaborator(OWNER, "owner"), collaborator(VIEWER, "viewer")]);

    let s = slug.clone();
    db(move |c| {
        c.execute(
            "INSERT INTO dataset_records (id, app_slug, data, ceds_mapping, is_ferpa_sensitive, table_id, created_at) \
             SELECT 'rec-' || $1 || '-' || lpad(g::text, 8, '0'), $1, \
                    jsonb_build_object('_table_id', 't1', 'amount', g, 'status', 's' || ((g % 5) + 1), 'program', 'p' || (g % 3), \
                                       'due', to_char(TIMESTAMPTZ '2026-01-01 00:00:00+00' + g * interval '1 day', 'YYYY-MM-DD\"T\"HH24:MI:SS\"Z\"'), \
                                       'note', 'n' || g), \
                    '{}'::jsonb, false, 't1', TIMESTAMPTZ '2026-01-01 00:00:00+00' + g * interval '1 millisecond' \
             FROM generate_series(1, $2::bigint) AS g",
            &[&s, &rows],
        )
        .expect("records");
        index_all(c, &s);
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

/// The index rows of every record of the app that has none.
fn index_all(c: &mut postgres::Client, slug: &str) {
    for (field, col, expr) in [
        ("amount", "num", "(data->>'amount')::float8"),
        ("status", "txt", "lower(data->>'status')"),
        ("program", "txt", "lower(data->>'program')"),
        ("due", "ts", "(data->>'due')::timestamptz"),
    ] {
        c.execute(
            &format!("INSERT INTO record_index (app_slug, table_id, field, record_id, {col}) SELECT app_slug, 't1', '{field}', id, {expr} FROM dataset_records d WHERE app_slug = $1 AND NOT EXISTS (SELECT 1 FROM record_index x WHERE x.record_id = d.id AND x.field = '{field}')"),
            &[&slug],
        )
        .expect("index");
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
    let (_, v) = send(&w.app, "POST", "/api/v1/mcp", token, Some(payload)).await;
    let result = &v["result"];
    if result["isError"] == true {
        return (true, result["content"][0]["text"].clone());
    }
    (false, result["structuredContent"].clone())
}

fn clause(field: &str, op: &str, value: &str) -> Value {
    json!({ "id": format!("{field}-{op}"), "field_name": field, "operator": op, "value": value })
}

fn group(conjunction: &str, clauses: Vec<Value>, groups: Vec<Value>) -> Value {
    json!({ "conjunction": conjunction, "clauses": clauses, "groups": groups })
}

fn encode(s: &str) -> String {
    s.bytes().map(|b| if b.is_ascii_alphanumeric() { (b as char).to_string() } else { format!("%{b:02X}") }).collect()
}

/// The count over a filter, by the aggregate tool.
async fn count(w: &World, filter: Value) -> f64 {
    let (err, v) = tool(w, &w.owner, "aggregate_records", json!({ "app_slug": w.slug, "table_id": "t1", "function": "count", "filter": filter })).await;
    assert!(!err, "{v}");
    v["value"].as_f64().expect("a count")
}

/// The rows a filter returns, by the list tool.
async fn rows(w: &World, filter: Value) -> Vec<Value> {
    let (err, v) = tool(w, &w.owner, "list_records", json!({ "app_slug": w.slug, "table_id": "t1", "filter": filter, "limit": 200 })).await;
    assert!(!err, "{v}");
    v["records"].as_array().unwrap().iter().map(|r| r["data"].clone()).collect()
}

async fn make_view(w: &World, token: &str, name: &str, config: Value) -> Value {
    let body = json!({ "name": name, "kind": "grid", "ownership": "collaborative", "table_id": "t1", "config": config });
    let (s, v) = send(&w.app, "POST", &format!("/api/v1/apps/{}/views", w.slug), token, Some(body)).await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    v
}

async fn run(w: &World, token: &str, id: &str, query: &str) -> (StatusCode, Value) {
    send(&w.app, "GET", &format!("/api/v1/apps/{}/views/{id}/data{query}", w.slug), token, None).await
}

// 1. A nested filter, (status = s1 OR status = s2) AND amount > 5, returns the right rows over 200,000, and its
//    plan has no sequential scan.
#[tokio::test]
async fn a_nested_filter_returns_the_right_rows_and_its_plan_has_no_sequential_scan() {
    let _turn = LOCK.lock().await;
    let w = world(200_000).await;
    let filter = group("And", vec![clause("amount", "GreaterThan", "5")], vec![group("Or", vec![clause("status", "Equals", "s1"), clause("status", "Equals", "s2")], vec![])]);

    // status is s1 when the row number is a multiple of 5, s2 one above it. Rows 1 and 5 are not above 5.
    assert_eq!(count(&w, filter.clone()).await, 79_998.0);
    let page = rows(&w, filter).await;
    assert_eq!(page.len(), 200);
    assert!(page.iter().all(|r| r["amount"].as_f64().unwrap() > 5.0 && ["s1", "s2"].contains(&r["status"].as_str().unwrap())), "every row satisfies the tree");

    // The statement a page runs over the index.
    let leaf = |field: &str, column, op, value: &str| Expr::Leaf(IndexedClause { field: field.into(), column, multi: false, op, value: value.into() });
    use scaffoldry_engine::FilterOperator::{Equals, GreaterThan};
    let q = IndexedQuery {
        app_slug: w.slug.clone(),
        table_id: "t1".into(),
        clauses: vec![],
        sort: Some(IndexedSort { field: "amount".into(), column: IndexColumn::Num, descending: false }),
        after: None,
        limit: 51,
        scope: None,
        search: None,
        expr: Some(Expr::And(vec![
            leaf("amount", IndexColumn::Num, GreaterThan, "5"),
            Expr::Or(vec![leaf("status", IndexColumn::Txt, Equals, "s1"), leaf("status", IndexColumn::Txt, Equals, "s2")]),
        ])),
    };
    let (sql, params) = build_indexed_sql(&q, IndexPhase::All);
    let plan = w.state.repository.as_deref().unwrap().explain(&sql, &params).unwrap();
    assert!(!plan.contains("Seq Scan"), "the plan reads the index:\n{plan}");
}

// 2. In, Between, Before and After read the index on indexed fields and the record on the rest.
#[tokio::test]
async fn in_between_before_and_after_filter_on_indexed_and_unindexed_fields() {
    let w = world(2_000).await;
    let one = |c: Value| group("And", vec![c], vec![]);

    assert_eq!(count(&w, one(clause("status", "In", r#"["s1","s2"]"#))).await, 800.0);
    assert_eq!(count(&w, one(clause("amount", "Between", "10..20"))).await, 11.0);
    assert_eq!(count(&w, one(clause("due", "Before", "2026-01-11"))).await, 9.0, "rows 1 to 9 fall on 2 to 10 January");
    assert_eq!(count(&w, one(clause("due", "After", "2026-01-11"))).await, 1_990.0, "row 10 is on the 11th at midnight, which is not after it");
    assert_eq!(count(&w, one(clause("due", "Between", "2026-01-02..2026-01-04"))).await, 3.0);

    // `note` is not indexed. The table is small, so the tree reads the records.
    let tree = group("And", vec![clause("amount", "GreaterThan", "7")], vec![group("Or", vec![clause("note", "Equals", "n7"), clause("note", "Equals", "n8")], vec![])]);
    let got = rows(&w, tree).await;
    assert_eq!(got.iter().map(|r| r["note"].as_str().unwrap()).collect::<Vec<_>>(), vec!["n8"]);
    let got = rows(&w, group("And", vec![clause("note", "In", "n1, n2, n3"), clause("amount", "Between", "2..3")], vec![])).await;
    assert_eq!(got.len(), 2);
    let got = rows(&w, group("And", vec![clause("note", "Contains", "n1"), clause("due", "Before", "2026-01-11")], vec![])).await;
    assert_eq!(got.iter().map(|r| r["note"].as_str().unwrap().to_string()).collect::<Vec<_>>(), vec!["n1"], "a date reads from the record too");
}

// 3. A filter nests two levels and holds fifty clauses at most. A view that breaks either is refused.
#[tokio::test]
async fn a_filter_that_nests_too_deep_is_refused_everywhere_it_can_be_given() {
    let w = world(50).await;
    let leaf = || group("And", vec![clause("amount", "GreaterThan", "1")], vec![]);
    let too_deep = group("And", vec![], vec![group("Or", vec![], vec![group("And", vec![], vec![leaf()])])]);

    let (err, msg) = tool(&w, &w.owner, "list_records", json!({ "app_slug": w.slug, "table_id": "t1", "filter": too_deep })).await;
    assert!(err && msg.as_str().unwrap().contains("two levels"), "{msg}");

    let body = json!({ "name": "Deep", "kind": "grid", "table_id": "t1", "config": { "filters": too_deep } });
    let (s, v) = send(&w.app, "POST", &format!("/api/v1/apps/{}/views", w.slug), &w.owner, Some(body)).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{v}");
}

// 4. Relative dates are resolved on the server. A row from now is in the last 7 days, and a row from ten days ago is not.
#[tokio::test]
async fn a_relative_date_is_resolved_by_the_server_clock() {
    let w = world(10).await;
    let slug = w.slug.clone();
    db(move |c| {
        for (id, days) in [("recent", 0i32), ("old", 10)] {
            c.execute(
                "INSERT INTO dataset_records (id, app_slug, data, ceds_mapping, is_ferpa_sensitive, table_id) \
                 VALUES ($1, $2, jsonb_build_object('_table_id', 't1', 'amount', 1000, 'due', to_char(now() - $3::int4 * interval '1 day', 'YYYY-MM-DD\"T\"HH24:MI:SS\"Z\"')), '{}', false, 't1')",
                &[&format!("{id}-{slug}"), &slug, &days],
            )
            .unwrap();
        }
        index_all(c, &slug);
    });
    let got = rows(&w, group("And", vec![clause("due", "Between", "last_7_days"), clause("amount", "GreaterThan", "999")], vec![])).await;
    assert_eq!(got.len(), 1, "only the row from now");
    let got = rows(&w, group("And", vec![clause("due", "Between", "last_30_days"), clause("amount", "GreaterThan", "999")], vec![])).await;
    assert_eq!(got.len(), 2);
    // And through the record JSON, on a field the tree reads that is not indexed.
    let got = rows(&w, group("And", vec![clause("note", "IsEmpty", ""), clause("due", "Between", "today")], vec![])).await;
    assert_eq!(got.len(), 1);
}

// 5. Grouping by two fields returns groups with counts that add up to the filtered total. Opening a group pages only its rows.
#[tokio::test]
async fn grouping_by_two_fields_counts_to_the_total_and_a_group_pages_only_its_rows() {
    let w = world(2_000).await;
    let view = make_view(
        &w,
        &w.owner,
        "By program and status",
        json!({ "group_by": ["program", "status"], "filters": group("And", vec![clause("amount", "GreaterThan", "100")], vec![]), "sort_rules": [{ "id": "s", "field_name": "amount", "direction": "Asc" }] }),
    )
    .await;
    let id = view["id"].as_str().unwrap();
    let items = |v: &Value| -> Vec<(String, f64)> { v["groups"]["items"].as_array().unwrap().iter().map(|g| (g["key"].as_str().unwrap().to_string(), g["count"].as_f64().unwrap())).collect() };

    let (s, top) = run(&w, &w.owner, id, "").await;
    assert_eq!(s, StatusCode::OK, "{top}");
    assert_eq!(top["groups"]["field"], "program");
    assert_eq!(top["groups"]["level"], 0);
    let programs = items(&top);
    assert_eq!(programs.iter().map(|(k, _)| k.as_str()).collect::<Vec<_>>(), vec!["p0", "p1", "p2"]);
    assert_eq!(programs.iter().map(|(_, n)| n).sum::<f64>(), 1_900.0, "the groups add up to the filtered total");

    let mut leaves = Vec::new();
    for (program, n) in &programs {
        let path = encode(&json!([program]).to_string());
        let (_, level1) = run(&w, &w.owner, id, &format!("?group_path={path}")).await;
        assert_eq!(level1["groups"]["field"], "status");
        let statuses = items(&level1);
        assert_eq!(statuses.iter().map(|(_, c)| c).sum::<f64>(), *n, "the second level adds up to its parent");
        leaves.extend(statuses.into_iter().map(|(status, c)| (program.clone(), status, c)));
    }

    // Open one group and walk its pages.
    let (program, status, expected) = leaves.iter().find(|(p, s, _)| p == "p1" && s == "s3").cloned().unwrap();
    let path = encode(&json!([program, status]).to_string());
    let mut seen = 0.0;
    let mut cursor: Option<String> = None;
    loop {
        let q = format!("?group_path={path}&limit=40{}", cursor.as_ref().map(|c| format!("&cursor={}", encode(c))).unwrap_or_default());
        let (s, page) = run(&w, &w.owner, id, &q).await;
        assert_eq!(s, StatusCode::OK, "{page}");
        let records = page["records"].as_array().unwrap();
        assert!(records.iter().all(|r| r["data"]["program"] == "p1" && r["data"]["status"] == "s3" && r["data"]["amount"].as_f64().unwrap() > 100.0), "only the group's rows");
        seen += records.len() as f64;
        match page["next_cursor"].as_str() {
            Some(c) => cursor = Some(c.to_string()),
            None => break,
        }
    }
    assert_eq!(seen, expected, "the pages hold the whole group and nothing else");

    let too_long = encode(&json!(["p1", "s3", "x"]).to_string());
    let (s, _) = run(&w, &w.owner, id, &format!("?group_path={too_long}")).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "the path names more levels than the view groups by");
}

// 6. A footer sum over 200,000 rows is correct, and the statement is one, over the index and not the records.
#[tokio::test]
async fn a_footer_sum_over_200000_rows_is_correct_and_runs_as_one_statement() {
    let _turn = LOCK.lock().await;
    let w = world(200_000).await;
    let view = make_view(&w, &w.owner, "Totals", json!({ "column_summary": [["amount", "sum"], ["status", "unique"], ["program", "filled"], ["due", "empty"]] })).await;
    let (s, page) = run(&w, &w.owner, view["id"].as_str().unwrap(), "?limit=5").await;
    assert_eq!(s, StatusCode::OK, "{page}");
    assert_eq!(page["footers"]["amount"], json!({ "function": "sum", "value": 20_000_100_000.0 }));
    assert_eq!(page["footers"]["status"]["value"], 5.0, "five distinct statuses");
    assert_eq!(page["footers"]["program"]["value"], 200_000.0);
    assert_eq!(page["footers"]["due"]["value"], 0.0);

    // The statement: one SELECT over the index.
    let plan = AggregatePlan {
        app_slug: w.slug.clone(),
        table_id: "t1".into(),
        function: AggregateFunction::Sum,
        field: Some("amount".into()),
        driver: None,
        group_by: None,
        clauses: vec![],
        expr: None,
        scope: None,
    };
    let (sql, params) = build_aggregate_sql(&plan);
    assert_eq!(sql.matches("SELECT").count(), 2, "one statement: the sum, in a wrapper that turns no rows into zero\n{sql}");
    let explained = w.state.repository.as_deref().unwrap().explain(&sql, &params).unwrap();
    // A total reads every value of the field, so it may scan the index table. It never reads the records.
    assert!(!explained.contains("dataset_records"), "{explained}");

    // The footer is on the first page only.
    let cursor = page["next_cursor"].as_str().unwrap().to_string();
    let (_, second) = run(&w, &w.owner, view["id"].as_str().unwrap(), &format!("?limit=5&cursor={}", encode(&cursor))).await;
    assert!(second.get("footers").is_none());
}

// 7. The other footer choices, and a footer on a field the person cannot see.
#[tokio::test]
async fn footers_offer_the_other_choices_and_none_for_a_hidden_field() {
    let w = world(2_000).await;
    let view = |field: &str, f: &str| json!({ "column_summary": [[field, f]] });
    for (f, expected) in [("count", 2_000.0), ("avg", 1_000.5), ("min", 1.0), ("max", 2_000.0), ("sum", 2_001_000.0)] {
        let v = make_view(&w, &w.owner, &format!("{f} of amount"), view("amount", f)).await;
        let (_, page) = run(&w, &w.owner, v["id"].as_str().unwrap(), "?limit=1").await;
        assert_eq!(page["footers"]["amount"]["value"], expected, "{f}");
    }
    // A sum of a text-like field is not offered.
    let v = make_view(&w, &w.owner, "sum of status", view("status", "sum")).await;
    let (_, page) = run(&w, &w.owner, v["id"].as_str().unwrap(), "?limit=1").await;
    assert_eq!(page["footers"]["status"], json!({ "function": "sum", "value": null, "unsupported": true }));

    // A footer on a hidden field offers none.
    set_column_rules(
        &w.slug,
        vec![ColumnRule {
            id: "hide-amount".into(),
            table_id: "t1".into(),
            fields: vec!["amount".into()],
            mode: ColumnMode::Hidden,
            unless: Who { roles: vec!["admin".into(), "owner".into()], ..Default::default() },
            message: None,
        }],
        &w.state,
    );
    let v = make_view(&w, &w.owner, "sum again", view("amount", "sum")).await;
    let (s, page) = run(&w, &w.viewer, v["id"].as_str().unwrap(), "?limit=1").await;
    assert_eq!(s, StatusCode::OK, "{page}");
    assert!(page["footers"].get("amount").is_none(), "{page}");
    let (_, page) = run(&w, &w.owner, v["id"].as_str().unwrap(), "?limit=1").await;
    assert!(page["footers"].get("amount").is_some(), "the owner still has it");
}

fn rule(id: &str, label: &str, when: Value, tone: &str, target: &str, field: Option<&str>) -> Value {
    let mut r = json!({ "id": id, "label": label, "when": when, "tone": tone, "target": target });
    if let Some(f) = field {
        r["field"] = json!(f);
    }
    r
}

fn labels(row: &Value) -> Vec<String> {
    row["data"]["_tone"].as_array().map(|a| a.iter().map(|t| t["label"].as_str().unwrap().to_string()).collect()).unwrap_or_default()
}

fn row_with_amount(page: &Value, amount: f64) -> Value {
    page["records"].as_array().unwrap().iter().find(|r| r["data"]["amount"].as_f64() == Some(amount)).cloned().unwrap_or_else(|| panic!("no row with amount {amount}"))
}

// 8. A color rule marks the right rows. A row that matches two rules shows both labels. A rule that reads a field the
//    person cannot see is dropped for them.
#[tokio::test]
async fn a_color_rule_marks_the_right_rows_and_a_rule_on_a_hidden_field_is_dropped() {
    let w = world(200).await;
    let when = |f: &str, op: &str, v: &str| group("And", vec![clause(f, op, v)], vec![]);
    let view = make_view(
        &w,
        &w.owner,
        "Toned",
        json!({
            "sort_rules": [{ "id": "s", "field_name": "amount", "direction": "Asc" }],
            "color_rules": [
                rule("r1", "At risk", when("status", "Equals", "s1"), "danger", "row", None),
                rule("r2", "Large", when("amount", "GreaterThan", "100"), "warning", "field", Some("amount")),
                rule("r3", "Program one", when("program", "Equals", "p1"), "info", "row", None),
                // On a redacted row `program` is absent, so without the drop this would mark every row for the viewer.
                rule("r4", "No program", when("program", "IsEmpty", ""), "neutral", "row", None),
            ]
        }),
    )
    .await;
    let id = view["id"].as_str().unwrap();
    let (s, page) = run(&w, &w.owner, id, "?limit=200").await;
    assert_eq!(s, StatusCode::OK, "{page}");

    // Row 115 is s1 (115 is a multiple of 5), is above 100, and is in p1 (115 % 3 = 1).
    let both = row_with_amount(&page, 115.0);
    assert_eq!(labels(&both), vec!["At risk", "Large", "Program one"], "a row that matches several rules shows every label, in the view's order");
    let tones = both["data"]["_tone"].as_array().unwrap();
    assert_eq!((tones[0]["tone"].as_str(), tones[0]["target"].as_str()), (Some("danger"), Some("row")));
    assert_eq!((tones[1]["tone"].as_str(), tones[1]["target"].as_str(), tones[1]["field"].as_str()), (Some("warning"), Some("field"), Some("amount")));
    assert_eq!(labels(&row_with_amount(&page, 5.0)), vec!["At risk"], "row 5 is s1, small, and in p2");
    assert!(labels(&row_with_amount(&page, 2.0)).is_empty(), "row 2 matches nothing");
    assert!(row_with_amount(&page, 2.0)["data"].get("_tone").is_none());

    // The viewer cannot see `program`. The rule that reads it is dropped, and the others still apply.
    set_column_rules(
        &w.slug,
        vec![ColumnRule {
            id: "hide-program".into(),
            table_id: "t1".into(),
            fields: vec!["program".into()],
            mode: ColumnMode::Hidden,
            unless: Who { roles: vec!["admin".into(), "owner".into()], ..Default::default() },
            message: None,
        }],
        &w.state,
    );
    let (s, page) = run(&w, &w.viewer, id, "?limit=200").await;
    assert_eq!(s, StatusCode::OK, "a color rule does not stop the view: {page}");
    let both = row_with_amount(&page, 115.0);
    assert_eq!(labels(&both), vec!["At risk", "Large"], "the rule on the hidden field is dropped for the viewer");
    assert!(both["data"].get("program").is_none());
    let (_, page) = run(&w, &w.owner, id, "?limit=200").await;
    assert_eq!(labels(&row_with_amount(&page, 115.0)).len(), 3, "the owner still has all three");
}

// 9. A view with too many groups, color rules or a bad footer is refused, and a color rule needs a label.
#[tokio::test]
async fn a_view_that_asks_for_too_much_is_refused() {
    let w = world(20).await;
    let when = group("And", vec![clause("amount", "GreaterThan", "1")], vec![]);
    let many: Vec<Value> = (0..11).map(|i| rule(&format!("r{i}"), "L", when.clone(), "info", "row", None)).collect();
    let bad = [
        ("eleven color rules", json!({ "color_rules": many })),
        ("four group levels", json!({ "group_by": ["program", "status", "due", "amount"] })),
        ("a color rule with no label", json!({ "color_rules": [rule("r", " ", when.clone(), "info", "row", None)] })),
        ("a field rule that names no field", json!({ "color_rules": [rule("r", "L", when.clone(), "info", "field", None)] })),
        ("a footer that is not a choice", json!({ "column_summary": [["amount", "median"]] })),
        ("a footer on a field that is not there", json!({ "column_summary": [["nope", "sum"]] })),
        ("a rule that reads a field that is not there", json!({ "color_rules": [rule("r", "L", group("And", vec![clause("nope", "Equals", "x")], vec![]), "info", "row", None)] })),
    ];
    for (what, config) in bad {
        let body = json!({ "name": what, "kind": "grid", "table_id": "t1", "config": config });
        let (s, v) = send(&w.app, "POST", &format!("/api/v1/apps/{}/views", w.slug), &w.owner, Some(body)).await;
        assert_eq!(s, StatusCode::BAD_REQUEST, "{what}: {v}");
    }
}
