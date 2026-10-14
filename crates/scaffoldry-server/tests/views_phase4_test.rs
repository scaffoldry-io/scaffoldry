//! Views phase 4: summary views and the aggregate route's additions.
//!
//! A summary view is a read-only table of groups and aggregates, computed by the aggregate route over the index, at most 200
//! groups with a flag for more. The route gains a date `bucket` in the institution's time zone, a second `group_by`, and
//! `count_distinct`.

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::Router;
use http_body_util::BodyExt;
use scaffoldry_engine::{AppManifest, AppTable, FieldSpec, FieldType, SelectOption, SelectOptionColor};
use scaffoldry_server::build_app_with_state;
use scaffoldry_server::service::identity::issue_test_token_and_user;
use scaffoldry_server::state::{CollaboratorRecord, ServerState};
use serde_json::{json, Value};
use std::sync::Arc;
use tower::ServiceExt;
use uuid::Uuid;


const OWNER: &str = "prof.curie@science.state.edu";
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

/// If a test stalls, say what the database is doing instead of waiting for the runner to give up. A test that blocks on the
/// database cannot be timed out from inside, so a thread watches the clock, prints every statement that is still running with
/// its generic plan, and ends the process.
struct Watchdog(std::sync::Arc<std::sync::atomic::AtomicBool>);

impl Watchdog {
    fn arm(what: &'static str, seconds: u64) -> Self {
        let done = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let flag = done.clone();
        std::thread::spawn(move || {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(seconds);
            while std::time::Instant::now() < deadline {
                if flag.load(std::sync::atomic::Ordering::SeqCst) {
                    return;
                }
                std::thread::sleep(std::time::Duration::from_millis(500));
            }
            let report = db(|c| {
                let rows = c
                    .query(
                        "SELECT pid, EXTRACT(EPOCH FROM now() - query_start)::int, state, COALESCE(wait_event_type, ''), COALESCE(wait_event, ''), query \
                         FROM pg_stat_activity WHERE datname = current_database() AND pid <> pg_backend_pid() AND backend_type = 'client backend' AND state <> 'idle' ORDER BY query_start",
                        &[],
                    )
                    .unwrap_or_default();
                let mut out = String::new();
                for r in rows {
                    let (pid, secs, state, wt, we, q): (i32, i32, String, String, String, String) = (r.get(0), r.get(1), r.get(2), r.get(3), r.get(4), r.get(5));
                    out.push_str(&format!("pid {pid}, {secs} s, {state}, waiting on {wt}/{we}:\n{q}\n"));
                    if q.trim_start().to_uppercase().starts_with("SELECT") || q.trim_start().to_uppercase().starts_with("INSERT") {
                        match c.query(&format!("EXPLAIN (GENERIC_PLAN) {q}"), &[]) {
                            Ok(plan) => out.push_str(&plan.iter().map(|p| p.get::<_, String>(0)).collect::<Vec<_>>().join("\n")),
                            Err(e) => out.push_str(&format!("(no plan: {e})")),
                        }
                        out.push('\n');
                    }
                }
                out
            });
            // Straight to stderr: `eprintln!` is captured by the test harness, which would print nothing before the exit.
            let _ = std::io::Write::write_all(&mut std::io::stderr(), format!("WATCHDOG: {what} ran for {seconds} s. The database is running:\n{report}\n").as_bytes());
            std::process::exit(101);
        });
        Watchdog(done)
    }
}

impl Drop for Watchdog {
    fn drop(&mut self) {
        self.0.store(true, std::sync::atomic::Ordering::SeqCst);
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
            fields: vec![indexed("amount", FieldType::Number), indexed("bucket", FieldType::Number), status, indexed("due", FieldType::Date)],
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
    _cleanup: Cleanup,
}

/// Rows 1..=`rows`. `amount` is the row number, `bucket` the row number modulo 1,000 (so 1,000 groups of `rows / 1,000`), and
/// `status` s1..s5 in turn. `due` is the first of January 2026 plus the row number in days.
async fn world(rows: i64) -> World {
    let slug = format!("vp4-{}", &Uuid::new_v4().to_string()[..8]);
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
    let s = slug.clone();
    db(move |c| {
        c.execute(
            "INSERT INTO dataset_records (id, app_slug, data, ceds_mapping, is_ferpa_sensitive, table_id, created_at) \
             SELECT 'rec-' || $1 || '-' || lpad(g::text, 8, '0'), $1, \
                    jsonb_build_object('_table_id', 't1', 'amount', g, 'bucket', g % 1000, 'status', 's' || ((g % 5) + 1), \
                                       'due', to_char(TIMESTAMPTZ '2026-01-01 00:00:00+00' + g * interval '1 day', 'YYYY-MM-DD\"T\"HH24:MI:SS\"Z\"')), \
                    '{}'::jsonb, false, 't1', TIMESTAMPTZ '2026-01-01 00:00:00+00' + g * interval '1 millisecond' \
             FROM generate_series(1, $2::bigint) AS g",
            &[&s, &rows],
        )
        .expect("records");
        index_rows(c, &s, None);
        c.batch_execute("ANALYZE dataset_records").unwrap();
        c.batch_execute("ANALYZE record_index").unwrap();
    });
    World { app: build_app_with_state(state.clone()).expect("router"), state, owner: issue_test_token_and_user(OWNER), _cleanup: Cleanup(slug.clone()), slug }
}

/// The index rows of the app's records: all of them, or only those whose id starts with one of `prefixes`.
fn index_rows(c: &mut postgres::Client, slug: &str, prefixes: Option<Vec<&str>>) {
    let like: Vec<String> = prefixes.unwrap_or_else(|| vec![""]).into_iter().map(|p| format!("{p}%")).collect();
    for (field, col, expr) in [
        ("amount", "num", "(data->>'amount')::float8"),
        ("bucket", "num", "(data->>'bucket')::float8"),
        ("status", "txt", "lower(data->>'status')"),
        ("due", "ts", "(data->>'due')::timestamptz"),
    ] {
        c.execute(
            &format!("INSERT INTO record_index (app_slug, table_id, field, record_id, {col}) SELECT app_slug, 't1', '{field}', id, {expr} FROM dataset_records WHERE app_slug = $1 AND id LIKE ANY($2)"),
            &[&slug, &like],
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

fn encode(s: &str) -> String {
    s.bytes().map(|b| if b.is_ascii_alphanumeric() { (b as char).to_string() } else { format!("%{b:02X}") }).collect()
}

async fn aggregate(w: &World, query: &str) -> (StatusCode, Value) {
    send(&w.app, "GET", &format!("/api/v1/apps/{}/tables/t1/aggregate?{query}", w.slug), &w.owner, None).await
}

async fn make_view(w: &World, kind: &str, name: &str, config: Value) -> String {
    let body = json!({ "name": name, "kind": kind, "ownership": "collaborative", "table_id": "t1", "config": config });
    let (s, v) = send(&w.app, "POST", &format!("/api/v1/apps/{}/views", w.slug), &w.owner, Some(body)).await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    v["id"].as_str().unwrap().to_string()
}

async fn run(w: &World, id: &str) -> (StatusCode, Value) {
    send(&w.app, "GET", &format!("/api/v1/apps/{}/views/{id}/data", w.slug), &w.owner, None).await
}

fn equals(field: &str, value: &str) -> String {
    encode(&json!({ "conjunction": "And", "clauses": [{ "id": "c", "field_name": field, "operator": "Equals", "value": value }] }).to_string())
}

// 1. A summary view over 200,000 rows matches a direct aggregate and has 200 groups at most.
#[tokio::test]
async fn a_summary_view_over_200000_rows_matches_a_direct_aggregate_and_holds_200_groups_at_most() {
    let _db = common::TestDb::new();
    let _dog = Watchdog::arm("a_summary_view_over_200000_rows_matches_a_direct_aggregate_and_holds_200_groups_at_most", 240);
    let w = world(200_000).await;
    let view = make_view(
        &w,
        "summary",
        "By bucket",
        json!({ "group_by": ["bucket"], "aggregations": [{ "fn": "count" }, { "field": "amount", "fn": "sum" }, { "field": "status", "fn": "count_distinct" }] }),
    )
    .await;
    let (s, body) = run(&w, &view).await;
    assert_eq!(s, StatusCode::OK, "{body}");
    let summary = &body["summary"];
    let groups = summary["groups"].as_array().expect("groups");
    assert_eq!(groups.len(), 200, "1,000 buckets, 200 shown");
    assert_eq!(summary["has_more"], true, "and the flag says there are more");
    assert_eq!(summary["columns"], json!(["bucket", "count", "sum of amount", "distinct status"]));

    // Each shown group equals a direct aggregate over a filter on that group.
    for g in groups.iter().step_by(40) {
        let key = g["key"][0].as_str().unwrap();
        let [count, sum, distinct] = [&g["values"][0], &g["values"][1], &g["values"][2]];
        let (_, direct) = aggregate(&w, &format!("function=count&filter={}", equals("bucket", key))).await;
        assert_eq!(direct["value"], *count, "count of bucket {key}");
        let (_, direct) = aggregate(&w, &format!("function=sum&field=amount&filter={}", equals("bucket", key))).await;
        assert_eq!(direct["value"], *sum, "sum of bucket {key}");
        let (_, direct) = aggregate(&w, &format!("function=count_distinct&field=status&filter={}", equals("bucket", key))).await;
        assert_eq!(direct["value"], *distinct, "distinct status of bucket {key}");
    }
    assert_eq!(groups[0]["values"][0], 200.0, "each bucket holds 200 of the 200,000 rows");

    // A few groups fit, and the flag is off.
    let small = make_view(&w, "summary", "By status", json!({ "group_by": ["status"], "aggregations": [{ "fn": "count" }, { "field": "amount", "fn": "sum" }] })).await;
    let (_, body) = run(&w, &small).await;
    assert_eq!(body["summary"]["groups"].as_array().unwrap().len(), 5);
    assert_eq!(body["summary"]["has_more"], false);
    assert!(body["summary"]["groups"].as_array().unwrap().iter().all(|g| g["values"][0] == 40_000.0));
}

// 2. A date bucket by month in the institution's time zone puts a record in the right month across a boundary.
#[tokio::test]
async fn a_date_bucket_by_month_in_the_institutions_zone_puts_a_record_in_the_right_month() {
    let _db = common::TestDb::new();
    let _dog = Watchdog::arm("a_date_bucket_by_month_in_the_institutions_zone_puts_a_record_in_the_right_month", 120);
    let w = world(0).await;
    w.state.settings.write().unwrap().insert("platform.timezone".to_string(), json!("America/New_York"));
    let s = w.slug.clone();
    db(move |c| {
        // New York is on daylight time (UTC-4) from 8 March 2026.
        for (n, due) in [
            ("a", "2026-03-01T03:30:00Z"), // 22:30 on 28 February, local
            ("b", "2026-03-01T05:30:00Z"), // 00:30 on 1 March, local
            ("c", "2026-03-31T23:30:00Z"), // 19:30 on 31 March, local
            ("d", "2026-04-01T03:30:00Z"), // 23:30 on 31 March, local
            ("e", "2026-04-01T04:30:00Z"), // 00:30 on 1 April, local
        ] {
            c.execute(
                "INSERT INTO dataset_records (id, app_slug, data, ceds_mapping, is_ferpa_sensitive, table_id) VALUES ($1, $2, jsonb_build_object('_table_id', 't1', 'amount', 1, 'due', $3::text), '{}', false, 't1')",
                &[&format!("b-{n}-{s}"), &s, &due],
            )
            .unwrap();
        }
        index_rows(c, &s, Some(vec!["b-a-", "b-b-", "b-c-", "b-d-", "b-e-"]));
    });
    let keys = |v: &Value| -> Vec<(String, f64)> {
        let mut out: Vec<(String, f64)> = v["groups"].as_array().unwrap().iter().map(|g| (g["key"].as_str().unwrap().to_string(), g["value"].as_f64().unwrap())).collect();
        out.sort_by(|a, b| a.0.cmp(&b.0));
        out
    };
    let (st, month) = aggregate(&w, "function=count&group_by=due&bucket=month").await;
    assert_eq!(st, StatusCode::OK, "{month}");
    assert_eq!(keys(&month), vec![("2026-02-01".to_string(), 1.0), ("2026-03-01".to_string(), 3.0), ("2026-04-01".to_string(), 1.0)], "a: February. b, c, d: March. e: April");
    let (_, quarter) = aggregate(&w, "function=count&group_by=due&bucket=quarter").await;
    assert_eq!(keys(&quarter), vec![("2026-01-01".to_string(), 4.0), ("2026-04-01".to_string(), 1.0)]);
    let (_, year) = aggregate(&w, "function=count&group_by=due&bucket=year").await;
    assert_eq!(keys(&year), vec![("2026-01-01".to_string(), 5.0)]);
    let (_, day) = aggregate(&w, "function=count&group_by=due&bucket=day").await;
    assert_eq!(day["groups"].as_array().unwrap().len(), 4, "a is 28 Feb, b is 1 Mar, c and d are 31 Mar, e is 1 Apr");
    let (_, week) = aggregate(&w, "function=count&group_by=due&bucket=week").await;
    assert_eq!(keys(&week).iter().map(|(k, _)| k.as_str()).collect::<Vec<_>>(), vec!["2026-02-23", "2026-03-30"], "a is the week of Monday 23 Feb. b is 1 March, a Sunday, so still that week. c, d and e are the week of Monday 30 March");

    // In UTC the same records fall differently, so the zone is what decided it.
    w.state.settings.write().unwrap().insert("platform.timezone".to_string(), json!("UTC"));
    let (_, utc) = aggregate(&w, "function=count&group_by=due&bucket=month").await;
    assert_eq!(keys(&utc), vec![("2026-03-01".to_string(), 3.0), ("2026-04-01".to_string(), 2.0)]);

    // A bucket on a field that is not a date is refused, and so is an unknown bucket.
    let (st, _) = aggregate(&w, "function=count&group_by=status&bucket=month").await;
    assert_eq!(st, StatusCode::BAD_REQUEST);
    let (st, _) = aggregate(&w, "function=count&group_by=due&bucket=fortnight").await;
    assert_eq!(st, StatusCode::BAD_REQUEST);
}

// 3. count_distinct, and a second group_by.
#[tokio::test]
async fn count_distinct_and_a_second_group_by_agree_with_the_rows() {
    let _db = common::TestDb::new();
    let _dog = Watchdog::arm("count_distinct_and_a_second_group_by_agree_with_the_rows", 120);
    let w = world(1_000).await;
    let (s, v) = aggregate(&w, "function=count_distinct&field=status").await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["value"], 5.0);
    let (_, v) = aggregate(&w, "function=count_distinct&field=bucket&group_by=status").await;
    let groups = v["groups"].as_array().unwrap();
    assert_eq!(groups.len(), 5);
    assert!(groups.iter().all(|g| g["value"] == 200.0), "each status holds 200 distinct buckets of 1,000 rows: {v}");

    // 1,000 rows: bucket is the row number modulo 1,000, status is the row number modulo 5 plus one, so a bucket has one status.
    let (s, two) = aggregate(&w, "function=count&group_by=status&group_by2=bucket").await;
    assert_eq!(s, StatusCode::OK, "{two}");
    let groups = two["groups"].as_array().unwrap();
    assert_eq!(groups.len(), 200, "capped at 200 of the 1,000 pairs");
    assert_eq!(two["has_more"], true);
    assert!(groups.iter().all(|g| g["key"].is_string() && g["key2"].is_string() && g["value"] == 1.0), "{two}");

    // Three of the five statuses and ten buckets: pairs add up to the rows.
    let (_, pairs) = aggregate(&w, &format!("function=count&group_by=status&group_by2=bucket&filter={}", encode(&json!({ "conjunction": "And", "clauses": [{ "id": "c", "field_name": "bucket", "operator": "Between", "value": "0..9" }], "groups": [] }).to_string()))).await;
    let total: f64 = pairs["groups"].as_array().unwrap().iter().map(|g| g["value"].as_f64().unwrap()).sum();
    assert_eq!(total, 10.0, "ten rows have a bucket from 0 to 9");
}

// 4. A summary over a field the person cannot see is refused, and a summary needs a group.
#[tokio::test]
async fn a_summary_view_needs_a_group_and_an_aggregate_of_a_field_that_exists() {
    let _db = common::TestDb::new();
    let _dog = Watchdog::arm("a_summary_view_needs_a_group_and_an_aggregate_of_a_field_that_exists", 120);
    let w = world(10).await;
    for (what, config) in [
        ("no group", json!({ "aggregations": [{ "fn": "count" }] })),
        ("an unknown function", json!({ "group_by": ["status"], "aggregations": [{ "fn": "median", "field": "amount" }] })),
        ("an unknown field", json!({ "group_by": ["status"], "aggregations": [{ "fn": "sum", "field": "nope" }] })),
        ("a sum with no field", json!({ "group_by": ["status"], "aggregations": [{ "fn": "sum" }] })),
        ("no aggregation", json!({ "group_by": ["status"], "aggregations": [] })),
    ] {
        let body = json!({ "name": what, "kind": "summary", "table_id": "t1", "config": config });
        let (s, v) = send(&w.app, "POST", &format!("/api/v1/apps/{}/views", w.slug), &w.owner, Some(body)).await;
        assert_eq!(s, StatusCode::BAD_REQUEST, "{what}: {v}");
    }
}
