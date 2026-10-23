//! Row scale phase 6: the million-row proof.
//!
//! One test, `million_rows`, marked `#[ignore]` because it seeds 1,000,000 rows of thirty fields (five indexed). CI runs it
//! in release mode: `cargo test --release -- --ignored million_rows`. It runs each of nine operations twenty times through
//! the real routes, prints the median, the 95th percentile, and the slowest, and fails if the 95th percentile (the 19th of twenty) passes its limit. The slowest is still printed and recorded. Each read also has its
//! plan checked for a `Seq Scan` of the records. If a limit fails on the CI runner and the plan is right, the limit does not
//! move: the plan and the timings go into the record and the work stops.

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::Router;
use base64::Engine;
use http_body_util::BodyExt;
use scaffoldry_engine::{AppManifest, AppTable, FieldSpec, FieldType, FilterOperator, IndexColumn, SelectOption, SelectOptionColor};
use scaffoldry_server::build_app_with_state;
use scaffoldry_server::repository::{build_record_query, RecordQuery, ScopePlan, FIRST_PAGE_SQL, PAGE_SQL};
use scaffoldry_server::repository_index::{build_aggregate_sql, build_indexed_sql, AggregateFunction, AggregatePlan, Expr, IndexPhase, IndexedClause, IndexedQuery, IndexedSort};
use scaffoldry_server::repository::SqlParam;
use scaffoldry_server::service::identity::issue_test_token_and_user;
use scaffoldry_server::state::{CollaboratorRecord, ServerState};
use serde_json::{json, Value};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tower::ServiceExt;
use uuid::Uuid;

const OWNER: &str = "prof.curie@science.state.edu";
const WORKSPACE: &str = "ws-bio-lab";
const ROWS: i64 = 1_000_000;
const RUNS: usize = 20;

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
                let _ = c.execute("DELETE FROM record_index WHERE app_slug = $1", &[&slug]);
                let _ = c.execute("DELETE FROM record_history WHERE app_slug = $1", &[&slug]);
                let _ = c.execute("DELETE FROM dataset_records WHERE app_slug = $1", &[&slug]);
                let _ = c.execute("DELETE FROM app_manifests WHERE slug = $1", &[&slug]);
            }
        })
        .join();
    }
}

fn plain(name: &str, t: FieldType) -> FieldSpec {
    FieldSpec::simple(name, name, t, false, false)
}

fn indexed(name: &str, t: FieldType) -> FieldSpec {
    let mut f = plain(name, t);
    f.indexed = true;
    f
}

/// Thirty fields. Five are indexed: `amount`, `status`, `bucket`, `due` and `title`.
fn manifest(slug: &str) -> AppManifest {
    let mut status = indexed("status", FieldType::Select);
    status.select_options = ["s1", "s2", "s3", "s4", "s5"]
        .iter()
        .enumerate()
        .map(|(i, id)| SelectOption { id: id.to_string(), label: id.to_string(), color: SelectOptionColor::Neutral, order: i as i32 })
        .collect();
    let mut fields = vec![indexed("amount", FieldType::Number), status, indexed("bucket", FieldType::Number), indexed("due", FieldType::Date), indexed("title", FieldType::Text), plain("note", FieldType::Text)];
    fields.extend((1..=24).map(|i| plain(&format!("f{i:02}"), FieldType::Text)));
    assert_eq!(fields.len(), 30);
    AppManifest {
        slug: slug.to_string(),
        title: "Million".to_string(),
        description: String::new(),
        organization_code: "BIO".to_string(),
        department: "biology".to_string(),
        workspace_id: Some(WORKSPACE.to_string()),
        herm_capability_id: None,
        coordinates: None,
        custom_domain: None,
        custom_domain_verified: false,
        tables: vec![AppTable { id: "t1".to_string(), name: "Things".to_string(), slug: "things".to_string(), description: None, icon: None, fields, primary_field: None, sample_records: vec![] , coordinates: None, row_place: Default::default() }],
        relationships: vec![],
        views: vec![],
        ceds_mappings: Default::default(),
        pages: vec![],
        access: Default::default(),
    }
}

/// Rows 1..=ROWS. `amount` is the row number, `status` s1..s5 in turn, `bucket` is the row number modulo 1,000, so each bucket
/// holds 1,000 rows. `note` is `zebrafish` for ten rows and `standard` for the rest. The other twenty-four fields hold short text.
fn seed(slug: &str) -> Duration {
    let started = Instant::now();
    let s = slug.to_string();
    db(move |c| {
        let fillers: String = (1..=24).map(|i| format!("'f{i:02}', 'value-' || ((g * {i}) % 9973)")).collect::<Vec<_>>().join(", ");
        c.execute(
            &format!(
                "INSERT INTO dataset_records (id, app_slug, data, ceds_mapping, is_ferpa_sensitive, table_id, created_at) \
                 SELECT 'rec-' || $1 || '-' || lpad(g::text, 8, '0'), $1, \
                        jsonb_build_object('_table_id', 't1', 'amount', g, 'status', 's' || ((g % 5) + 1), 'bucket', g % 1000, \
                                           'due', to_char(TIMESTAMPTZ '2026-01-01 00:00:00+00' + (g % 3650) * interval '1 day', 'YYYY-MM-DD\"T\"HH24:MI:SS\"Z\"'), \
                                           'title', 'title-' || g, 'note', CASE WHEN g % 100000 = 42 THEN 'zebrafish' ELSE 'standard' END, {fillers}), \
                        '{{}}'::jsonb, false, 't1', TIMESTAMPTZ '2026-01-01 00:00:00+00' + g * interval '1 millisecond' \
                 FROM generate_series(1, $2::bigint) AS g"
            ),
            &[&s, &ROWS],
        )
        .expect("records");
        for (field, col, expr) in [
            ("amount", "num", "(data->>'amount')::float8"),
            ("status", "txt", "lower(data->>'status')"),
            ("bucket", "num", "(data->>'bucket')::float8"),
            ("due", "ts", "(data->>'due')::timestamptz"),
            ("title", "txt", "lower(data->>'title')"),
        ] {
            c.execute(
                &format!("INSERT INTO record_index (app_slug, table_id, field, record_id, {col}) SELECT app_slug, 't1', '{field}', id, {expr} FROM dataset_records WHERE app_slug = $1"),
                &[&s],
            )
            .expect("index");
        }
        // The planner reads statistics, and an index-only scan reads the visibility map. A real table has both by now.
        // VACUUM cannot share a transaction, so each is its own statement.
        c.batch_execute("VACUUM ANALYZE dataset_records").expect("vacuum records");
        c.batch_execute("VACUUM ANALYZE record_index").expect("vacuum index");
        // A table that has been loaded has been through its checkpoints. Without this the first writes pay for the load.
        c.batch_execute("CHECKPOINT").expect("checkpoint");
    });
    started.elapsed()
}

struct World {
    app: Router,
    state: Arc<ServerState>,
    slug: String,
    token: String,
    _cleanup: Cleanup,
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

fn encode(s: &str) -> String {
    s.bytes().map(|b| if b.is_ascii_alphanumeric() { (b as char).to_string() } else { format!("%{b:02X}") }).collect()
}

/// A page cursor for the row at `row`, as the server issues them: the row's creation time and id, and its sort value.
fn cursor_at(slug: &str, row: i64, key: Option<i64>) -> String {
    let s = slug.to_string();
    let created: String = db(move |c| c.query_one("SELECT (TIMESTAMPTZ '2026-01-01 00:00:00+00' + $1::bigint * interval '1 millisecond')::text", &[&row]).unwrap().get(0));
    let mut body = json!({ "created_at": created, "id": format!("rec-{s}-{row:08}") });
    if let Some(k) = key {
        body["key"] = json!(k);
    }
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(body.to_string())
}

#[derive(Clone)]
struct Timed {
    name: &'static str,
    limit: Duration,
    runs: Vec<Duration>,
}

impl Timed {
    fn median(&self) -> Duration {
        let mut r = self.runs.clone();
        r.sort();
        r[r.len() / 2]
    }
    fn slowest(&self) -> Duration {
        *self.runs.iter().max().unwrap()
    }
    /// The run the limit is checked against: the 95th percentile. With twenty runs it is the 19th, sorted from fastest to
    /// slowest. One slow run on a shared runner does not fail the proof. Two do.
    fn gate(&self) -> Duration {
        let mut r = self.runs.clone();
        r.sort();
        r[(r.len() * 95).div_ceil(100) - 1]
    }
    fn over_limit(&self) -> bool {
        self.gate() > self.limit
    }
}

/// Runs an operation `RUNS` times and keeps every duration. A failed response stops the proof.
async fn time<F, Fut>(name: &'static str, limit_ms: u64, mut op: F) -> Timed
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = (StatusCode, Value)>,
{
    // One run to warm the caches and the plan. It is not counted, and it must succeed.
    let (s, v) = op().await;
    assert!(s.is_success(), "{name}: {s} {v}");
    let mut runs = Vec::new();
    for _ in 0..RUNS {
        let started = Instant::now();
        let (s, v) = op().await;
        runs.push(started.elapsed());
        assert!(s.is_success(), "{name}: {s} {v}");
    }
    Timed { name, limit: Duration::from_millis(limit_ms), runs }
}

fn plan_of(w: &World, sql: &str, params: &[SqlParam]) -> String {
    w.state.repository.explain(sql, params).unwrap()
}

#[tokio::test]
#[ignore = "seeds 1,000,000 rows. CI runs it in release mode: cargo test --release -- --ignored million_rows"]
async fn million_rows() {
    let _db = common::TestDb::new();
    let slug = format!("million-{}", &Uuid::new_v4().to_string()[..8]);
    let state = Arc::new(ServerState::new().expect("state"));
    state.seed_demo().expect("seed");
    state.engine.write().unwrap().register_manifest(manifest(&slug)).unwrap();
    state.collaborators.write().unwrap().insert(
        WORKSPACE.to_string(),
        vec![CollaboratorRecord {
            id: Uuid::new_v4().to_string(),
            workspace_id: WORKSPACE.to_string(),
            eppn: OWNER.to_string(),
            name: OWNER.to_string(),
            role: "owner".to_string(),
            scoped_affiliation: "faculty".to_string(),
            department: "biology".to_string(),
            added_at: String::new(),
            app_slug: String::new(),
        }],
    );
    let seeded = seed(&slug);
    println!("million_rows: seeded {ROWS} rows of 30 fields (5 indexed) in {seeded:?}");
    let w = World { app: build_app_with_state(state.clone()).expect("router"), state, token: issue_test_token_and_user(OWNER), slug: slug.clone(), _cleanup: Cleanup(slug.clone()) };

    let base = format!("/api/v1/apps/{}/tables/t1", w.slug);
    let near_end = cursor_at(&w.slug, ROWS - 200, None);
    let sort_cursor = cursor_at(&w.slug, ROWS - 200, Some(ROWS - 200));
    let mut results: Vec<Timed> = Vec::new();
    // Each call owns what it sends, so the future it returns borrows nothing.
    let call = |method: &'static str, uri: String, body: Option<Value>| {
        let (app, token) = (w.app.clone(), w.token.clone());
        async move { send(&app, method, &uri, &token, body).await }
    };

    // 1. The first page, in the default order.
    results.push(time("First page, default order", 100, || call("GET", format!("{base}/records?limit=100"), None)).await);

    // 2. A page reached by cursor near the end.
    let uri = format!("{base}/records?limit=100&cursor={}", encode(&near_end));
    results.push(time("A page by cursor near the end", 100, || call("GET", uri.clone(), None)).await);

    // 3. Sort by an indexed number, a page near the end. 4. An indexed range filter returning 200 of 1,000 matches.
    let tool = |name: &'static str, args: Value| {
        let (app, token) = (w.app.clone(), w.token.clone());
        async move {
            let payload = json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": { "name": name, "arguments": args } });
            let (s, v) = send(&app, "POST", "/api/v1/mcp", &token, Some(payload)).await;
            if s.is_success() && v["result"]["isError"] == true {
                return (StatusCode::BAD_REQUEST, v["result"].clone());
            }
            (s, v)
        }
    };
    let sort_args = json!({ "app_slug": w.slug, "table_id": "t1", "sort": { "field_name": "amount", "direction": "Asc" }, "limit": 100, "cursor": sort_cursor });
    results.push(time("Sort by an indexed number, a page near the end", 200, || tool("list_records", sort_args.clone())).await);
    let range = json!({ "conjunction": "And", "clauses": [{ "id": "r", "field_name": "amount", "operator": "Between", "value": "500001..501000" }], "groups": [] });
    let range_args = json!({ "app_slug": w.slug, "table_id": "t1", "filter": range, "limit": 200 });
    results.push(time("Indexed range filter, 200 of 1,000 matches", 200, || tool("list_records", range_args.clone())).await);
    let (_, one) = tool("list_records", range_args.clone()).await;
    assert_eq!(one["result"]["structuredContent"]["records"].as_array().map(Vec::len), Some(200), "the filter returns a page of 200");
    assert!(one["result"]["structuredContent"]["next_cursor"].is_string(), "and there are more of the 1,000");

    // 5. sum over the whole table. 6. count grouped by a five-value field.
    let sum_uri = format!("{base}/aggregate?function=sum&field=amount");
    results.push(time("sum over the whole table", 1_500, || call("GET", sum_uri.clone(), None)).await);
    let (_, sum) = call("GET", sum_uri.clone(), None).await;
    assert_eq!(sum["value"], 500_000_500_000.0_f64, "the sum of 1..=1,000,000");
    let count_uri = format!("{base}/aggregate?function=count&group_by=status");
    results.push(time("count grouped by a five-value field", 1_500, || call("GET", count_uri.clone(), None)).await);
    let (_, counts) = call("GET", count_uri.clone(), None).await;
    assert_eq!(counts["groups"].as_array().map(Vec::len), Some(5));
    assert!(counts["groups"].as_array().unwrap().iter().all(|g| g["value"] == 200_000.0), "{counts}");

    // 7. A search for a word in ten rows.
    let search_uri = format!("{base}/records?limit=200&search=zebrafish");
    results.push(time("Search for a word in ten rows", 200, || call("GET", search_uri.clone(), None)).await);
    let (_, found) = call("GET", search_uri.clone(), None).await;
    assert_eq!(found["records"].as_array().map(Vec::len), Some(10), "ten rows hold the word");

    // 8. Create one record with five indexed fields. 9. Update one indexed field.
    let mut n = 0i64;
    let create = || {
        n += 1;
        let body = json!({ "data": { "amount": ROWS + n, "status": "s3", "bucket": n % 1000, "due": "2027-01-01T00:00:00Z", "title": format!("new-{n}") } });
        call("POST", format!("{base}/records"), Some(body))
    };
    results.push(time("Create one record, five indexed fields", 50, create).await);
    let (s, made) = call("POST", format!("{base}/records"), Some(json!({ "data": { "amount": 1, "status": "s1", "bucket": 1, "due": "2027-01-01T00:00:00Z", "title": "to-update" } }))).await;
    assert_eq!(s, StatusCode::CREATED, "{made}");
    let (id, mut version) = (made["id"].as_str().unwrap().to_string(), made["version"].as_i64().unwrap_or(1));
    let mut bump = 0i64;
    let update = || {
        bump += 1;
        let body = json!({ "data": { "amount": 5_000_000 + bump }, "version": version });
        version += 1;
        call("PATCH", format!("{base}/records/{id}"), Some(body))
    };
    results.push(time("Update one indexed field", 50, update).await);

    // The table, as the run printed it.
    println!("\nmillion_rows: {ROWS} rows, 30 fields, 5 indexed. Median, 95th percentile (the gate), and slowest of {RUNS} runs through the routes.");
    println!("| Operation | Median | p95 | Slowest | Limit |");
    println!("| --- | --- | --- | --- | --- |");
    for r in &results {
        println!(
            "| {} | {:.1} ms | {:.1} ms | {:.1} ms | {} ms |",
            r.name,
            r.median().as_secs_f64() * 1000.0,
            r.gate().as_secs_f64() * 1000.0,
            r.slowest().as_secs_f64() * 1000.0,
            r.limit.as_millis()
        );
    }
    println!();

    // Each read passes the Seq Scan check. The statement is the one the read runs.
    let t = "t1".to_string();
    let appslug = w.slug.clone();
    let mut plans: Vec<(&str, String)> = Vec::new();
    plans.push(("First page, default order", plan_of(&w, FIRST_PAGE_SQL, &[SqlParam::Text(appslug.clone()), SqlParam::Text(t.clone()), SqlParam::Int(101)])));
    let created: String = {
        let r = ROWS - 200;
        db(move |c| c.query_one("SELECT (TIMESTAMPTZ '2026-01-01 00:00:00+00' + $1::bigint * interval '1 millisecond')::text", &[&r]).unwrap().get(0))
    };
    plans.push((
        "A page by cursor near the end",
        plan_of(&w, PAGE_SQL, &[SqlParam::Text(appslug.clone()), SqlParam::Text(t.clone()), SqlParam::Text(created), SqlParam::Text(format!("rec-{}-{:08}", w.slug, ROWS - 200)), SqlParam::Int(101)]),
    ));
    let sort = IndexedSort { field: "amount".into(), column: IndexColumn::Num, descending: false };
    let query = |sort: IndexedSort, after, expr| IndexedQuery {
        app_slug: appslug.clone(),
        table_id: t.clone(),
        clauses: vec![],
        sort: Some(sort),
        after,
        limit: 101,
        scope: None,
        search: None,
        expr,
        offset: 0,
    };
    let after = Some((scaffoldry_server::repository_index::IndexedValue::Num((ROWS - 200) as f64), format!("rec-{}-{:08}", w.slug, ROWS - 200)));
    let (sql, params) = build_indexed_sql(&query(sort.clone(), after, None), IndexPhase::All);
    plans.push(("Sort by an indexed number, a page near the end", plan_of(&w, &sql, &params)));
    let leaf = Expr::Leaf(IndexedClause { field: "amount".into(), column: IndexColumn::Num, multi: false, op: FilterOperator::Between, value: "500001..501000".into() });
    let (sql, params) = build_indexed_sql(&query(sort, None, Some(Expr::And(vec![leaf]))), IndexPhase::All);
    plans.push(("Indexed range filter, 200 of 1,000 matches", plan_of(&w, &sql, &params)));
    let agg = |function, field: Option<&str>, group: Option<&str>| AggregatePlan {
        app_slug: appslug.clone(),
        table_id: t.clone(),
        function,
        field: field.map(str::to_string),
        driver: None,
        group_by: group.map(str::to_string),
        clauses: vec![],
        expr: None,
        scope: None,
        ..Default::default()
    };
    let (sql, params) = build_aggregate_sql(&agg(AggregateFunction::Sum, Some("amount"), None));
    plans.push(("sum over the whole table", plan_of(&w, &sql, &params)));
    let (sql, params) = build_aggregate_sql(&agg(AggregateFunction::Count, None, Some("status")));
    plans.push(("count grouped by a five-value field", plan_of(&w, &sql, &params)));
    let (sql, params) = build_record_query(
        &appslug,
        &RecordQuery { table_id: t.clone(), any: false, clauses: vec![], sort: None, after: None, limit: 201, scope: None, search: Some("zebrafish".into()), expr: None },
    );
    plans.push(("Search for a word in ten rows", plan_of(&w, &sql, &params)));

    let mut scanned = Vec::new();
    for (name, plan) in &plans {
        if plan.contains("Seq Scan on dataset_records") || (plan.contains("Seq Scan") && !name.starts_with("sum") && !name.starts_with("count")) {
            scanned.push(format!("{name}:\n{plan}"));
        }
    }
    // A total reads every value of its field, so it may read the whole index table. It never reads the records.
    for (name, plan) in plans.iter().filter(|(n, _)| n.starts_with("sum") || n.starts_with("count")) {
        println!("million_rows: plan for {name}:\n{plan}\n");
        assert!(!plan.contains("dataset_records"), "{name} reads the records:\n{plan}");
    }
    assert!(scanned.is_empty(), "these reads scan a table:\n{}", scanned.join("\n\n"));

    let slow: Vec<String> = results
        .iter()
        .filter(|r| r.over_limit())
        .map(|r| format!("{}: p95 {:?} (slowest {:?}) against {:?}", r.name, r.gate(), r.slowest(), r.limit))
        .collect();
    assert!(slow.is_empty(), "these passed their limit at p95. The limit does not move. Paste the plan and the timings and stop:\n{}", slow.join("\n"));
}

// The gate is the 19th of 20 runs, sorted from fastest to slowest (the 95th percentile). The slowest is still printed
// and recorded. The limits do not move. One slow run on a shared runner does not fail the proof. Two do.
#[test]
fn the_gate_is_the_nineteenth_of_twenty_and_the_slowest_is_still_reported() {
    let ms = Duration::from_millis;
    let timed = |mut runs: Vec<Duration>| {
        runs.reverse();
        Timed { name: "x", limit: ms(50), runs }
    };
    // Eighteen fast runs, one at 30 ms, one outlier at 900 ms.
    let mut runs = vec![ms(10); 18];
    runs.push(ms(30));
    runs.push(ms(900));
    let one_outlier = timed(runs);
    assert_eq!(one_outlier.gate(), ms(30), "the 19th of 20 is the gate");
    assert_eq!(one_outlier.slowest(), ms(900), "the slowest is still reported");
    assert!(!one_outlier.over_limit(), "one slow run in twenty passes");

    // Two slow runs: the 19th is slow too.
    let mut runs = vec![ms(10); 18];
    runs.push(ms(900));
    runs.push(ms(900));
    assert!(timed(runs).over_limit(), "two slow runs in twenty fail");

    // The 19th run over the limit fails even when the slowest is the only other slow run.
    let mut runs = vec![ms(10); 18];
    runs.push(ms(51));
    runs.push(ms(52));
    assert!(timed(runs).over_limit(), "a gate over the limit fails");
}


// ---------------------------------------------------------------------------------------------------------------------
// Access model phase 6: lists at a million rows, for a person whose grants reach some of them.
// ---------------------------------------------------------------------------------------------------------------------

const GATE_WORKSPACE_UNIT: &str = "gate-ws";
const DIVISIONS: i64 = 20;
const LEAVES: i64 = 99;
const UNITS_PER_HIERARCHY: i64 = DIVISIONS * LEAVES;
const PEOPLE: i64 = 20_000;
const REACHED_PEOPLE: i64 = 5_000;

/// Three hierarchies of units and one of people. Each has a root, twenty divisions, and ninety-nine units in each division,
/// so a grant on five divisions reaches five hundred units.
fn seed_units() {
    db(|c| {
        for (key, kind) in [("gate_a", "units"), ("gate_b", "units"), ("gate_c", "units"), ("gate_p", "people")] {
            c.execute("INSERT INTO hierarchies (key, name, kind, source, created_by) VALUES ($1, $1, $2, 'api', 'test') ON CONFLICT DO NOTHING", &[&key, &kind]).unwrap();
            c.execute("INSERT INTO organizations (id, parent_id, name, code, org_type, hierarchy_key, synthetic) VALUES (md5($1 || 'root')::uuid, NULL, $1 || ' root', $1 || '-root', 'Unit', $1, false)", &[&key]).unwrap();
            c.execute(
                "INSERT INTO organizations (id, parent_id, name, code, org_type, hierarchy_key, synthetic) \
                 SELECT md5($1 || 'd' || d)::uuid, md5($1 || 'root')::uuid, $1 || ' division ' || d, $1 || '-d' || d, 'Unit', $1, false FROM generate_series(1, $2::int) d",
                &[&key, &(DIVISIONS as i32)],
            )
            .unwrap();
            c.execute(
                "INSERT INTO organizations (id, parent_id, name, code, org_type, hierarchy_key, synthetic) \
                 SELECT md5($1 || 'u' || d || '_' || u)::uuid, md5($1 || 'd' || d)::uuid, $1 || ' unit ' || d || '.' || u, $1 || '-u' || d || '-' || u, 'Unit', $1, false \
                 FROM generate_series(1, $2::int) d, generate_series(1, $3::int) u",
                &[&key, &(DIVISIONS as i32), &(LEAVES as i32)],
            )
            .unwrap();
            c.execute("INSERT INTO org_closure (hierarchy_key, ancestor, descendant, depth) SELECT $1::varchar, id, id, 0 FROM organizations WHERE hierarchy_key = $1::varchar", &[&key]).unwrap();
            c.execute("INSERT INTO org_closure (hierarchy_key, ancestor, descendant, depth) SELECT $1::varchar, parent_id, id, 1 FROM organizations WHERE hierarchy_key = $1::varchar AND parent_id IS NOT NULL", &[&key]).unwrap();
            c.execute(
                "INSERT INTO org_closure (hierarchy_key, ancestor, descendant, depth) SELECT $1::varchar, p.parent_id, o.id, 2 FROM organizations o JOIN organizations p ON o.parent_id = p.id \
                 WHERE o.hierarchy_key = $1::varchar AND p.parent_id IS NOT NULL",
                &[&key],
            )
            .unwrap();
        }
        // The workspace and the table sit in the academic hierarchy. A grant on the workspace's unit reaches the workspace and
        // not the table, so only the grants in the gate hierarchies decide which rows a person sees.
        let root: uuid::Uuid = "00000000-0000-0000-0000-000000000001".parse().unwrap();
        for (code, parent) in [(GATE_WORKSPACE_UNIT, "root"), ("gate-table", GATE_WORKSPACE_UNIT)] {
            let parent_id: uuid::Uuid = if parent == "root" { root } else { c.query_one("SELECT id FROM organizations WHERE code = $1", &[&parent]).unwrap().get(0) };
            c.execute("INSERT INTO organizations (id, parent_id, name, code, org_type, hierarchy_key, synthetic) VALUES (gen_random_uuid(), $1, $2, $2, 'Unit', 'academic', false)", &[&parent_id, &code]).unwrap();
            let id: uuid::Uuid = c.query_one("SELECT id FROM organizations WHERE code = $1", &[&code]).unwrap().get(0);
            c.execute("INSERT INTO org_closure (hierarchy_key, ancestor, descendant, depth) VALUES ('academic', $1, $1, 0) ON CONFLICT DO NOTHING", &[&id]).unwrap();
            c.execute("INSERT INTO org_closure (hierarchy_key, ancestor, descendant, depth) SELECT 'academic', ancestor, $1, depth + 1 FROM org_closure WHERE descendant = $2 ON CONFLICT DO NOTHING", &[&id, &parent_id]).unwrap();
        }
    });
}

/// 20,000 people, appointed at units of the people hierarchy. The first 5,000 sit in divisions 1 to 5.
fn seed_people() {
    db(|c| {
        c.execute(
            "INSERT INTO persons (id, first_name, last_name, email, eppn, synthetic) \
             SELECT md5('person' || i)::uuid, 'P', i::text, 'person' || i || '@gate.example', 'person' || i || '@gate.example', false FROM generate_series(0, $1::int - 1) i",
            &[&(PEOPLE as i32)],
        )
        .unwrap();
        c.execute(
            "INSERT INTO roles (id, person_id, organization_id, role_title, scoped_affiliation, is_primary, source) \
             SELECT gen_random_uuid(), md5('person' || i)::uuid, \
                    md5('gate_p' || 'u' || (k / $3::int + 1) || '_' || (k % $3::int + 1))::uuid, 'staff', 'staff', true, 'test' \
             FROM (SELECT i, CASE WHEN i < $1::int THEN i % ($2::int * $3::int) ELSE $2::int * $3::int + (i % (($4::int - $2::int) * $3::int)) END AS k FROM generate_series(0, $5::int - 1) i) x",
            &[&(REACHED_PEOPLE as i32), &5i32, &(LEAVES as i32), &(DIVISIONS as i32), &(PEOPLE as i32)],
        )
        .unwrap();
    });
}

fn grant_divisions(who: &str, hierarchy: &str, divisions: std::ops::RangeInclusive<i64>) {
    let (who, hierarchy) = (who.to_string(), hierarchy.to_string());
    db(move |c| {
        for d in divisions {
            c.execute(
                "INSERT INTO access_grants (holder_kind, holder_id, role, hierarchy_key, anchor_kind, anchor_id, depth, source, reason, created_by) \
                 VALUES ('person', $1::varchar, 'viewer', $2::varchar, 'unit', md5($2::text || 'd' || $3::int)::uuid::text, NULL, 'request', 'gate', 'test')",
                &[&who, &hierarchy, &(d as i32)],
            )
            .unwrap();
        }
    });
}

fn grant_workspace(who: &str) {
    let who = who.to_string();
    db(move |c| {
        c.execute(
            "INSERT INTO access_grants (holder_kind, holder_id, role, hierarchy_key, anchor_kind, anchor_id, depth, source, reason, created_by) \
             SELECT 'person', $1, 'viewer', 'academic', 'unit', id::text, 0, 'request', 'gate', 'test' FROM organizations WHERE code = $2",
            &[&who, &GATE_WORKSPACE_UNIT],
        )
        .unwrap();
    });
}

fn gate_manifest(slug: &str) -> AppManifest {
    let mut m = manifest(slug);
    let table = &mut m.tables[0];
    for (name, hierarchy) in [("u1", "gate_a"), ("u2", "gate_b"), ("u3", "gate_c")] {
        let mut f = indexed(name, FieldType::Unit);
        f.hierarchy = Some(hierarchy.to_string());
        table.fields.push(f);
    }
    table.fields.push(indexed("owner", FieldType::Collaborator { allow_multiple: false }));
    for (hierarchy, field) in [("gate_a", "u1"), ("gate_b", "u2"), ("gate_c", "u3"), ("gate_p", "owner")] {
        table.row_place.insert(hierarchy.to_string(), field.to_string());
    }
    let ws_unit = db(|c| c.query_one("SELECT id FROM organizations WHERE code = 'gate-table'", &[]).unwrap().get::<_, uuid::Uuid>(0));
    table.coordinates = Some(scaffoldry_core::coordinates::Coordinates { units: [("academic".to_string(), ws_unit.to_string())].into(), ..Default::default() });
    m
}

/// The same rows as `seed`, each placed in a unit of three hierarchies and owned by a person.
fn seed_placed(slug: &str) {
    let s = slug.to_string();
    db(move |c| {
        c.execute(
            "UPDATE dataset_records SET data = data || jsonb_build_object( \
                 'u1', md5('gate_a' || 'u' || (((substring(id from '[0-9]+$')::bigint - 1) % $2) / $3 + 1) || '_' || (((substring(id from '[0-9]+$')::bigint - 1) % $2) % $3 + 1))::uuid::text, \
                 'u2', md5('gate_b' || 'u' || ((((substring(id from '[0-9]+$')::bigint) * 7) % $2) / $3 + 1) || '_' || ((((substring(id from '[0-9]+$')::bigint) * 7) % $2) % $3 + 1))::uuid::text, \
                 'u3', md5('gate_c' || 'u' || ((((substring(id from '[0-9]+$')::bigint) * 13) % $2) / $3 + 1) || '_' || ((((substring(id from '[0-9]+$')::bigint) * 13) % $2) % $3 + 1))::uuid::text, \
                 'owner', 'person' || (substring(id from '[0-9]+$')::bigint % $4) || '@gate.example') \
             WHERE app_slug = $1",
            &[&s, &UNITS_PER_HIERARCHY, &LEAVES, &PEOPLE],
        )
        .expect("place the rows");
        for field in ["u1", "u2", "u3", "owner"] {
            c.execute(
                &format!("INSERT INTO record_index (app_slug, table_id, field, record_id, txt) SELECT app_slug, 't1', '{field}', id, lower(data->>'{field}') FROM dataset_records WHERE app_slug = $1"),
                &[&s],
            )
            .expect("index the placement");
        }
        c.batch_execute("VACUUM ANALYZE dataset_records").expect("vacuum records");
        c.batch_execute("VACUUM ANALYZE record_index").expect("vacuum index");
        c.batch_execute("CHECKPOINT").expect("checkpoint");
    });
}

#[tokio::test]
#[ignore = "seeds 1,000,000 rows. CI runs it in release mode: cargo test --release -- --ignored million_rows"]
async fn million_rows_with_grants() {
    let _db = common::TestDb::new();
    seed_units();
    seed_people();
    let slug = format!("gate-{}", &Uuid::new_v4().to_string()[..8]);
    // Three people. Each reaches the workspace through one grant that does not reach the table, then:
    //   alex: five divisions of one hierarchy (500 units).
    //   bea:  five divisions of each of three hierarchies.
    //   cy:   five divisions of the people hierarchy: 5,000 addresses.
    let (alex, bea, cy) = ("alex@gate.example", "bea@gate.example", "cy@gate.example");
    for who in [alex, bea, cy] {
        grant_workspace(who);
    }
    grant_divisions(alex, "gate_a", 1..=5);
    for h in ["gate_a", "gate_b", "gate_c"] {
        grant_divisions(bea, h, 1..=5);
    }
    grant_divisions(cy, "gate_p", 1..=5);

    let state = Arc::new(ServerState::new().expect("state"));
    state.seed_demo().expect("seed");
    {
        let ws_unit = db(|c| c.query_one("SELECT id FROM organizations WHERE code = $1", &[&GATE_WORKSPACE_UNIT]).unwrap().get::<_, uuid::Uuid>(0));
        let mut workspaces = state.workspaces.write().unwrap();
        let ws = workspaces.get_mut(WORKSPACE).expect("the demo workspace");
        ws.organization_id = Some(ws_unit);
        state.repository.upsert_workspace(ws).unwrap();
    }
    state.engine.write().unwrap().register_manifest(gate_manifest(&slug)).unwrap();
    let seeded = seed(&slug);
    seed_placed(&slug);
    state.refresh().expect("refresh");
    if std::env::var_os("GATE_SEED_ONLY").is_some() {
        println!("million_rows_with_grants: seeded only, slug {slug}");
        std::mem::forget(Cleanup(slug.clone()));
        return;
    }
    println!("million_rows_with_grants: seeded {ROWS} rows placed in 3 hierarchies and owned by 20,000 people in {:?}", seeded);
    let w = World { app: build_app_with_state(state.clone()).expect("router"), state, token: String::new(), slug: slug.clone(), _cleanup: Cleanup(slug.clone()) };

    let base = format!("/api/v1/apps/{}/tables/t1", w.slug);
    let near_end = cursor_at(&w.slug, ROWS - 200, None);
    let call = |who: &'static str, method: &'static str, uri: String| {
        let (app, token) = (w.app.clone(), issue_test_token_and_user(who));
        async move { send(&app, method, &uri, &token, None).await }
    };
    // Each person sees the share their grants reach. The check makes the timings mean something.
    for (who, rows) in [(alex, 100), (bea, 100), (cy, 100)] {
        let (s, page) = call(who, "GET", format!("{base}/records?limit={rows}")).await;
        assert_eq!(s, StatusCode::OK, "{who}: {page}");
        assert_eq!(page["records"].as_array().map(Vec::len), Some(rows), "{who} sees a full page");
    }
    let (_, everything) = call(alex, "GET", format!("{base}/aggregate?function=count")).await;
    let alex_rows = everything["value"].as_f64().expect("a count");
    assert!(alex_rows > 200_000.0 && alex_rows < 300_000.0, "alex reaches about a quarter of the rows: {alex_rows}");

    let mut results: Vec<Timed> = Vec::new();
    results.push(time("First page, a reach of 500 units in one hierarchy", 100, || call(alex, "GET", format!("{base}/records?limit=100"))).await);
    results.push(time("First page, reaches in three hierarchies at once", 150, || call(bea, "GET", format!("{base}/records?limit=100"))).await);
    results.push(time("First page, a people reach of 5,000 addresses", 200, || call(cy, "GET", format!("{base}/records?limit=100"))).await);
    let by_cursor = format!("{base}/records?limit=100&cursor={}", encode(&near_end));
    results.push(time("A page near the end by cursor, a reach of 500 units", 100, || call(alex, "GET", by_cursor.clone())).await);
    let grouped = format!("{base}/aggregate?function=count&group_by=status");
    results.push(time("Grouped count, a reach of 500 units", 1_500, || call(alex, "GET", grouped.clone())).await);

    println!("\nmillion_rows_with_grants: {ROWS} rows. Median, 95th percentile, and slowest of {RUNS} runs through the routes. The gate is the slowest.");
    println!("| Operation | Median | p95 | Slowest | Limit |");
    println!("| --- | --- | --- | --- | --- |");
    for r in &results {
        println!("| {} | {:.1} ms | {:.1} ms | {:.1} ms | {} ms |", r.name, r.median().as_secs_f64() * 1000.0, r.gate().as_secs_f64() * 1000.0, r.slowest().as_secs_f64() * 1000.0, r.limit.as_millis());
    }
    println!();

    // The statements the reads run, for the first page of each person: no scan of the records.
    let appslug = w.slug.clone();
    let fields = gate_manifest(&slug).tables[0].fields.clone();
    let units_of = |h: &str, divs: std::ops::RangeInclusive<i64>| -> Vec<String> {
        let h = h.to_string();
        db(move |c| {
            let mut out = Vec::new();
            for d in divs {
                for u in 1..=LEAVES {
                    out.push(c.query_one("SELECT md5($1 || 'u' || $2::int || '_' || $3::int)::uuid::text", &[&h, &(d as i32), &(u as i32)]).unwrap().get::<_, String>(0));
                }
            }
            out
        })
    };
    let in_set = |field: &str, ids: Vec<String>| scaffoldry_core::access::Pred::Cmp {
        field: field.to_string(),
        op: scaffoldry_core::access::Op::In,
        value: scaffoldry_core::access::Operand::Literal(Value::Array(ids.into_iter().map(Value::String).collect())),
    };
    let scoped = |pred| RecordQuery { table_id: "t1".into(), any: false, clauses: vec![], sort: None, after: None, limit: 101, scope: Some(ScopePlan { pred, indexed: fields.clone() }), search: None, expr: None };
    let owners: Vec<String> = (0..REACHED_PEOPLE).map(|i| format!("person{i}@gate.example")).collect();
    let mut plans: Vec<(&str, String)> = Vec::new();
    for (name, pred) in [
        ("500 units in one hierarchy", in_set("u1", units_of("gate_a", 1..=5))),
        ("three hierarchies", scaffoldry_core::access::Pred::Or(vec![in_set("u1", units_of("gate_a", 1..=5)), in_set("u2", units_of("gate_b", 1..=5)), in_set("u3", units_of("gate_c", 1..=5))])),
        ("5,000 addresses", in_set("owner", owners)),
    ] {
        let (sql, params) = build_record_query(&appslug, &scoped(pred));
        plans.push((name, plan_of(&w, &sql, &params)));
    }
    // The grouped count, over the 500 units of one hierarchy.
    let agg = AggregatePlan {
        app_slug: appslug.clone(),
        table_id: "t1".into(),
        function: AggregateFunction::Count,
        field: None,
        driver: None,
        group_by: Some("status".into()),
        clauses: vec![],
        expr: None,
        scope: Some(ScopePlan { pred: in_set("u1", units_of("gate_a", 1..=5)), indexed: fields.clone() }),
        ..Default::default()
    };
    let (sql, params) = build_aggregate_sql(&agg);
    plans.push(("grouped count over 500 units", plan_of(&w, &sql, &params)));
    for (name, plan) in &plans {
        println!("million_rows_with_grants: plan for {name}:\n{plan}\n");
    }
    let scanned: Vec<String> = plans.iter().filter(|(_, p)| p.contains("Seq Scan on dataset_records")).map(|(n, p)| format!("{n}:\n{p}")).collect();
    assert!(scanned.is_empty(), "these reads scan the records:\n{}", scanned.join("\n\n"));

    let slow: Vec<String> = results.iter().filter(|r| r.slowest() > r.limit).map(|r| format!("{}: slowest {:?} (p95 {:?}) against {:?}", r.name, r.slowest(), r.gate(), r.limit)).collect();
    assert!(slow.is_empty(), "these passed their limit. The limit does not move. Paste the plan and the timings and stop:\n{}", slow.join("\n"));
}
