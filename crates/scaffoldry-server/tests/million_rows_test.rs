//! Row scale phase 6: the million-row proof.
//!
//! One test, `million_rows`, marked `#[ignore]` because it seeds 1,000,000 rows of thirty fields (five indexed). CI runs it
//! in release mode: `cargo test --release -- --ignored million_rows`. It runs each of nine operations twenty times through
//! the real routes, prints the median and the slowest, and fails if the slowest passes its limit. Each read also has its
//! plan checked for a `Seq Scan` of the records. If a limit fails on the CI runner and the plan is right, the limit does not
//! move: the plan and the timings go into the record and the work stops.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::Router;
use base64::Engine;
use http_body_util::BodyExt;
use scaffoldry_engine::{AppManifest, AppTable, FieldSpec, FieldType, FilterOperator, IndexColumn, SelectOption, SelectOptionColor};
use scaffoldry_server::build_app_with_state;
use scaffoldry_server::repository::{build_record_query, RecordQuery, FIRST_PAGE_SQL, PAGE_SQL};
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
        custom_domain: None,
        custom_domain_verified: false,
        tables: vec![AppTable { id: "t1".to_string(), name: "Things".to_string(), slug: "things".to_string(), description: None, icon: None, fields, primary_field: None, sample_records: vec![] }],
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
    w.state.repository.as_deref().unwrap().explain(sql, params).unwrap()
}

#[tokio::test]
#[ignore = "seeds 1,000,000 rows. CI runs it in release mode: cargo test --release -- --ignored million_rows"]
async fn million_rows() {
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
    println!("\nmillion_rows: {ROWS} rows, 30 fields, 5 indexed. Median and slowest of {RUNS} runs through the routes.");
    println!("| Operation | Median | Slowest | Limit |");
    println!("| --- | --- | --- | --- |");
    for r in &results {
        println!("| {} | {:.1} ms | {:.1} ms | {} ms |", r.name, r.median().as_secs_f64() * 1000.0, r.slowest().as_secs_f64() * 1000.0, r.limit.as_millis());
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

    let slow: Vec<String> = results.iter().filter(|r| r.slowest() > r.limit).map(|r| format!("{}: slowest {:?} against {:?}", r.name, r.slowest(), r.limit)).collect();
    assert!(slow.is_empty(), "these passed their limit. The limit does not move. Paste the plan and the timings and stop:\n{}", slow.join("\n"));
}
