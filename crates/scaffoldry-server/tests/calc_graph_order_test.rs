//! Calculation graph phase 2: a write recalculates what depends on it, in order, in the same
//! transaction, and hands a large fan-out to a job.

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::Router;
use http_body_util::BodyExt;
use scaffoldry_engine::{AppManifest, AppTable, FieldSpec, FieldType, LinkCardinality};
use scaffoldry_server::build_app_with_state;
use scaffoldry_server::jobs::{get_job, JobConfig, JobRunner, JOB_KINDS};
use scaffoldry_server::repository::PostgresRepository;
use scaffoldry_server::service::identity::issue_test_token_and_user;
use scaffoldry_server::state::ServerState;
use serde_json::{json, Value};
use std::sync::Arc;
use std::time::{Duration, Instant};
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
        let slug = self.0.clone();
        let url = db_url();
        let _ = std::thread::spawn(move || {
            if let Ok(mut c) = postgres::Client::connect(&url, postgres::NoTls) {
                let _ = c.execute("DELETE FROM record_links WHERE app_slug = $1", &[&slug]);
                let _ = c.execute("DELETE FROM record_history WHERE app_slug = $1", &[&slug]);
                let _ = c.execute("DELETE FROM jobs WHERE payload->>'app_slug' = $1", &[&slug]);
                let _ = c.execute("DELETE FROM dataset_records WHERE app_slug = $1", &[&slug]);
            }
        })
        .join();
    }
}

fn num(name: &str) -> FieldSpec {
    FieldSpec::simple(name, name, FieldType::Number, false, false)
}

fn formula(name: &str, expression: &str) -> FieldSpec {
    let mut f = FieldSpec::simple(name, name, FieldType::Formula, false, false);
    f.formula_expression = Some(expression.to_string());
    f
}

fn link(name: &str, target: &str, related: &str, c: LinkCardinality) -> FieldSpec {
    let mut f = FieldSpec::simple(name, name, FieldType::Link, false, false);
    f.link_target_table = Some(target.to_string());
    f.link_related_field = Some(related.to_string());
    f.link_cardinality = c;
    f
}

fn computed(name: &str, t: FieldType, via: &str, target: Option<&str>) -> FieldSpec {
    let mut f = FieldSpec::simple(name, name, t, false, false);
    f.computed_via = Some(via.to_string());
    f.target_display_field = target.map(String::from);
    f.rollup_function = Some("sum".to_string());
    f
}

fn table(id: &str, fields: Vec<FieldSpec>) -> AppTable {
    AppTable { id: id.into(), name: id.into(), slug: id.into(), description: None, icon: None, fields, primary_field: None, sample_records: vec![] }
}

/// `chain.a` is declared before `chain.b`, but reads it. `parents` roll up their children's amount.
/// A `student` rolls up the credits of its `course`.
fn manifest(slug: &str) -> AppManifest {
    AppManifest {
        slug: slug.into(),
        title: "Order".into(),
        description: String::new(),
        organization_code: "BIO".into(),
        department: "biology".into(),
        workspace_id: Some(WORKSPACE.into()),
        herm_capability_id: None,
        custom_domain: None,
        custom_domain_verified: false,
        tables: vec![
            table("chain", vec![num("c"), formula("a", "{b} + 1"), formula("b", "{c} * 2")]),
            table(
                "parents",
                vec![
                    FieldSpec::simple("name", "name", FieldType::Text, false, false),
                    link("children", "children", "parent", LinkCardinality::Many),
                    computed("total", FieldType::Rollup, "children", Some("amount")),
                    computed("n", FieldType::Count, "children", None),
                    formula("label", "{double} + {n}"),
                    formula("double", "{total} * 2"),
                ],
            ),
            table("children", vec![num("amount"), link("parent", "parents", "children", LinkCardinality::One)]),
            table("courses", vec![num("credits")]),
            table(
                "students",
                vec![
                    link("course", "courses", "students", LinkCardinality::One),
                    computed("course_credits", FieldType::Rollup, "course", Some("credits")),
                    formula("double_credits", "{course_credits} * 2"),
                ],
            ),
        ],
        relationships: vec![],
        views: vec![],
        ceds_mappings: Default::default(),
        pages: vec![],
        access: Default::default(),
    }
}

struct World {
    app: Router,
    token: String,
    slug: String,
    _cleanup: Cleanup,
}

async fn world() -> World {
    let mut m = manifest("placeholder");
    // `courses.students` mirrors `students.course`.
    m.tables[3].fields.push(link("students", "students", "course", LinkCardinality::Many));
    let state = Arc::new(ServerState::new().expect("state"));
    state.seed_demo().expect("seed");
    let slug = format!("cg-{}", &Uuid::new_v4().to_string()[..8]);
    m.slug = slug.clone();
    state.engine.write().unwrap().register_manifest(m).unwrap();
    World { app: build_app_with_state(state).expect("router"), token: issue_test_token_and_user(OWNER), _cleanup: Cleanup(slug.clone()), slug }
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

async fn create(w: &World, table: &str, mut data: Value) -> Value {
    data["_table_id"] = json!(table);
    let (s, body) = send(&w.app, "POST", &format!("/api/v1/apps/{}/records", w.slug), &w.token, Some(json!({ "data": data }))).await;
    assert_eq!(s, StatusCode::CREATED, "{body}");
    body
}

async fn get(w: &World, id: &str) -> Value {
    send(&w.app, "GET", &format!("/api/v1/apps/{}/records/{id}", w.slug), &w.token, None).await.1
}

async fn patch(w: &World, id: &str, version: i64, data: Value) -> Value {
    let (s, body) = send(&w.app, "PATCH", &format!("/api/v1/apps/{}/records/{id}", w.slug), &w.token, Some(json!({ "data": data, "version": version }))).await;
    assert_eq!(s, StatusCode::OK, "{body}");
    body
}

/// A number, whether the server stored it as an integer or a float.
fn n(v: &Value) -> Option<i64> {
    v.as_f64().map(|f| f as i64)
}

fn id_of(v: &Value) -> String {
    v["id"].as_str().unwrap().to_string()
}

// 1. A write to `c` updates `b`, then `a`, though `a` is declared first.
#[tokio::test]
async fn a_write_updates_the_chain_in_dependency_order_not_field_order() {
    let _db = common::TestDb::new();
    let w = world().await;
    let rec = create(&w, "chain", json!({ "c": 5 })).await;
    assert_eq!((n(&rec["data"]["b"]), n(&rec["data"]["a"])), (Some(10), Some(11)), "{rec}");
    let after = patch(&w, &id_of(&rec), 1, json!({ "_table_id": "chain", "c": 7 })).await;
    assert_eq!((n(&after["data"]["b"]), n(&after["data"]["a"])), (Some(14), Some(15)), "{after}");
}

// 2. A parent's rollup updates when a child changes, then the formulas that read it, in order.
#[tokio::test]
async fn a_rollup_updates_then_the_formulas_that_read_it() {
    let _db = common::TestDb::new();
    let w = world().await;
    let parent = id_of(&create(&w, "parents", json!({ "name": "P" })).await);
    let child = create(&w, "children", json!({ "amount": 10, "parent": parent })).await;
    let p = get(&w, &parent).await;
    assert_eq!((n(&p["data"]["total"]), n(&p["data"]["double"]), n(&p["data"]["label"])), (Some(10), Some(20), Some(21)), "{p}");
    patch(&w, &id_of(&child), 1, json!({ "_table_id": "children", "amount": 25, "parent": parent })).await;
    let p = get(&w, &parent).await;
    assert_eq!((n(&p["data"]["total"]), n(&p["data"]["double"]), n(&p["data"]["label"])), (Some(25), Some(50), Some(51)), "{p}");
}

// 3. A computed change does not change a version, and is not in the owner's history.
#[tokio::test]
async fn a_computed_change_keeps_the_version_and_leaves_no_history() {
    let _db = common::TestDb::new();
    let w = world().await;
    let parent = id_of(&create(&w, "parents", json!({ "name": "P" })).await);
    let child = create(&w, "children", json!({ "amount": 10, "parent": parent })).await;
    let before = get(&w, &parent).await;
    patch(&w, &id_of(&child), 1, json!({ "_table_id": "children", "amount": 99, "parent": parent })).await;
    let after = get(&w, &parent).await;
    assert_eq!(n(&after["data"]["total"]), Some(99));
    assert_eq!(after["version"], before["version"], "the owner's version did not move");
    let (_, hist) = send(&w.app, "GET", &format!("/api/v1/apps/{}/records/{parent}/history", w.slug), &w.token, None).await;
    let totals = hist["entries"].as_array().unwrap().iter().filter(|e| e["changes"].get("total").is_some() || e["changes"].get("double").is_some()).count();
    assert_eq!(totals, 0, "no computed field is in the history: {hist}");
}

fn seed_students(slug: &str, course: &str, n: i64) {
    let (slug, course) = (slug.to_string(), course.to_string());
    db(move |c| {
        c.execute(
            "INSERT INTO dataset_records (id, app_slug, data, ceds_mapping, is_ferpa_sensitive, table_id) \
             SELECT $1::text || '-s-' || lpad(g::text, 6, '0'), $1::text, \
                    jsonb_build_object('_table_id', 'students', 'course', $2::text, 'course_credits', 3, 'double_credits', 6), \
                    '{}'::jsonb, false, 'students' FROM generate_series(1, $3::bigint) g",
            &[&slug, &course, &n],
        )
        .unwrap();
        c.execute(
            "INSERT INTO record_links (app_slug, field, from_record, to_record, ord) \
             SELECT $1::text, 'course', $1::text || '-s-' || lpad(g::text, 6, '0'), $2::text, 0 FROM generate_series(1, $3::bigint) g",
            &[&slug, &course, &n],
        )
        .unwrap();
    });
}

fn student_credits(slug: &str, n: i64) -> Vec<(i64, i64)> {
    let slug = slug.to_string();
    db(move |c| {
        c.query(
            "SELECT (data->>'course_credits')::float8::bigint, (data->>'double_credits')::float8::bigint FROM dataset_records WHERE app_slug = $1 AND table_id = 'students' ORDER BY id LIMIT $2",
            &[&slug, &n],
        )
        .unwrap()
        .iter()
        .map(|r| (r.get(0), r.get(1)))
        .collect()
    })
}

fn jobs_of(slug: &str) -> Vec<uuid::Uuid> {
    let slug = slug.to_string();
    db(move |c| c.query("SELECT id FROM jobs WHERE kind = 'recalc_owners' AND payload->>'app_slug' = $1", &[&slug]).unwrap().iter().map(|r| r.get(0)).collect())
}

fn fast() -> JobConfig {
    JobConfig { workers: 2, poll_ms: 20, backoff_secs: [0, 0, 0], ..JobConfig::default() }
}

fn wait_for_job(repo: &PostgresRepository, id: uuid::Uuid) -> scaffoldry_server::jobs::Job {
    let deadline = Instant::now() + Duration::from_secs(120);
    loop {
        let job = get_job(repo, id).expect("query").expect("job exists");
        if job.state == "done" || job.state == "failed" {
            return job;
        }
        assert!(Instant::now() < deadline, "job stuck in {}", job.state);
        std::thread::sleep(Duration::from_millis(50));
    }
}

// 4. Up to 200 owners are recomputed inline. More than that is a job.
#[tokio::test]
async fn a_small_fan_out_is_inline_and_a_large_one_is_a_job() {
    let _db = common::TestDb::new();
    let w = world().await;

    // 150 students: inline.
    let small = create(&w, "courses", json!({ "credits": 3 })).await;
    seed_students(&w.slug, &id_of(&small), 150);
    patch(&w, &id_of(&small), 1, json!({ "_table_id": "courses", "credits": 4 })).await;
    assert!(student_credits(&w.slug, 150).iter().all(|c| *c == (4, 8)), "all 150 updated before the write returned");
    assert!(jobs_of(&w.slug).is_empty(), "no job below the limit");

    // 500 students: a job, and nothing changed inline.
    let w2 = world().await;
    let big = create(&w2, "courses", json!({ "credits": 3 })).await;
    seed_students(&w2.slug, &id_of(&big), 500);
    patch(&w2, &id_of(&big), 1, json!({ "_table_id": "courses", "credits": 5 })).await;
    assert!(student_credits(&w2.slug, 500).iter().all(|c| *c == (3, 6)), "nothing is recomputed inline above the limit");
    let jobs = jobs_of(&w2.slug);
    assert_eq!(jobs.len(), 1, "one job names the changed record");

    let repo = PostgresRepository::connect(None).expect("database");
    let runner = JobRunner::start(&repo, JOB_KINDS, fast()).unwrap();
    let job = wait_for_job(&repo, jobs[0]);
    runner.stop();
    assert_eq!(job.state, "done", "{:?}", job.error);
    assert!(student_credits(&w2.slug, 500).iter().all(|c| *c == (5, 10)), "the job recomputed every owner, rollup then formula");
}
