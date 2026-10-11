//! Calculation graph phase 3: changing a definition is a job, and the field says so while it runs.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::Router;
use http_body_util::BodyExt;
use scaffoldry_engine::{AppManifest, AppTable, FieldSpec, FieldType};
use scaffoldry_server::build_app_with_state;
use scaffoldry_server::jobs::{claim_next, get_job, JobConfig, JobRunner, JOB_KINDS};
use scaffoldry_server::repository::PostgresRepository;
use scaffoldry_server::service::identity::issue_test_token_and_user;
use scaffoldry_server::state::ServerState;
use serde_json::{json, Value};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Mutex;
use tower::ServiceExt;
use uuid::Uuid;

static LOCK: Mutex<()> = Mutex::const_new(());

const OWNER: &str = "prof.curie@science.state.edu";
const SECOND: &str = "jordan.lee@state.edu";
const WORKSPACE: &str = "ws-bio-lab";
const BIG: i64 = 200_000;
const SMALL: i64 = 5_000;

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
        let slug = self.0.clone();
        let url = db_url();
        let _ = std::thread::spawn(move || {
            if let Ok(mut c) = postgres::Client::connect(&url, postgres::NoTls) {
                let _ = c.execute("DELETE FROM jobs WHERE payload->>'app_slug' = $1", &[&slug]);
                let _ = c.execute("DELETE FROM calc_status WHERE app_slug = $1", &[&slug]);
                let _ = c.execute("DELETE FROM record_history WHERE app_slug = $1", &[&slug]);
                let _ = c.execute("DELETE FROM dataset_records WHERE app_slug = $1", &[&slug]);
            }
        })
        .join();
    }
}

/// `doubled` is a formula over `score`. `code` is text, or a number after a conversion.
fn manifest(slug: &str, formula: &str, code: FieldType) -> AppManifest {
    let mut doubled = FieldSpec::simple("doubled", "doubled", FieldType::Formula, false, false);
    doubled.formula_expression = Some(formula.to_string());
    AppManifest {
        slug: slug.to_string(),
        title: "Recalc".to_string(),
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
            fields: vec![
                FieldSpec::simple("score", "score", FieldType::Number, false, false),
                doubled,
                FieldSpec::simple("code", "code", code, false, false),
            ],
            primary_field: None,
            sample_records: vec![],
        }],
        relationships: vec![],
        views: vec![],
        ceds_mappings: Default::default(),
        pages: vec![],
    }
}

struct World {
    app: Router,
    token: String,
    second: String,
    slug: String,
    _cleanup: Cleanup,
}

async fn world() -> World {
    let state = Arc::new(ServerState::new().expect("state"));
    state.seed_demo().expect("seed");
    let slug = format!("rc-{}", &Uuid::new_v4().to_string()[..8]);
    World {
        app: build_app_with_state(state).expect("router"),
        token: issue_test_token_and_user(OWNER),
        second: issue_test_token_and_user(SECOND),
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

/// The whole result of a tool call, so a coded error can be read.
async fn tool_full(w: &World, token: &str, name: &str, args: Value) -> Value {
    let payload = json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": { "name": name, "arguments": args } });
    let (_, v) = send(&w.app, "POST", "/api/mcp", token, Some(payload)).await;
    v["result"].clone()
}

fn repo() -> PostgresRepository {
    PostgresRepository::connect(None).expect("database is reachable")
}

fn fast() -> JobConfig {
    JobConfig { workers: 2, poll_ms: 20, backoff_secs: [0, 0, 0], ..JobConfig::default() }
}

fn wait_for_job(repo: &PostgresRepository, id: Uuid) -> scaffoldry_server::jobs::Job {
    let deadline = Instant::now() + Duration::from_secs(300);
    loop {
        let job = get_job(repo, id).expect("query").expect("job exists");
        if job.state == "done" || job.state == "failed" {
            return job;
        }
        assert!(Instant::now() < deadline, "job stuck in {}", job.state);
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn job_of(slug: &str, kind: &str) -> Uuid {
    let (slug, kind) = (slug.to_string(), kind.to_string());
    db(move |c| c.query_one("SELECT id FROM jobs WHERE kind = $1 AND payload->>'app_slug' = $2 ORDER BY created_at DESC LIMIT 1", &[&kind, &slug]).expect("the job was queued").get(0))
}

fn calc_state(slug: &str, field: &str) -> Option<String> {
    let (slug, field) = (slug.to_string(), field.to_string());
    db(move |c| c.query_opt("SELECT state FROM calc_status WHERE app_slug = $1 AND table_id = 't1' AND field = $2", &[&slug, &field]).unwrap().map(|r| r.get(0)))
}

/// Rows `1..=n`. `doubled` holds what the old formula, `{score} * 2`, wrote. `code` is `n/a` on every 25th.
fn fill(slug: &str, n: i64) {
    let slug = slug.to_string();
    db(move |c| {
        c.execute(
            "INSERT INTO dataset_records (id, app_slug, data, ceds_mapping, is_ferpa_sensitive, table_id, created_at) \
             SELECT 'rec-' || $1::text || '-' || lpad(g::text, 8, '0'), $1::text, \
                    jsonb_build_object('_table_id', 't1', 'score', g, 'doubled', g * 2, 'code', CASE WHEN g % 25 = 0 THEN 'n/a' ELSE (g % 100)::text END), \
                    '{}'::jsonb, false, 't1', TIMESTAMPTZ '2026-01-01 00:00:00+00' + (g / 10) * interval '1 millisecond' \
             FROM generate_series(1, $2::bigint) AS g",
            &[&slug, &n],
        )
        .expect("fill");
        c.batch_execute("ANALYZE dataset_records").expect("analyze");
    });
}

async fn go_live(w: &World, rows: i64) {
    let live = manifest(&w.slug, "{score} * 2", FieldType::Text);
    let (s, body) = send(&w.app, "POST", &format!("/api/v1/workspaces/{WORKSPACE}/apps"), &w.token, Some(serde_json::to_value(&live).unwrap())).await;
    assert!(s == StatusCode::CREATED || s == StatusCode::CONFLICT, "{s} {body}");
    fill(&w.slug, rows);
}

/// Proposes a manifest and returns the proposal id.
async fn propose(w: &World, m: &AppManifest, summary: &str) -> Value {
    let r = tool_full(w, &w.token, "propose_app_change", json!({ "workspace_id": WORKSPACE, "manifest": serde_json::to_value(m).unwrap(), "summary": summary })).await;
    assert_ne!(r["isError"], true, "{r}");
    r["structuredContent"].clone()
}

async fn decide(w: &World, token: &str, proposal: &Value) -> Value {
    tool_full(w, token, "decide_proposal", json!({ "proposal_id": proposal["proposal_id"], "decision": "approve", "rationale": "ok" })).await
}

/// Goes live and approves the formula `{score} * 3`. Returns how long the approval took.
async fn approve_triple(w: &World, rows: i64) -> Duration {
    go_live(w, rows).await;
    let p = propose(w, &manifest(&w.slug, "{score} * 3", FieldType::Text), "Triple").await;
    let started = Instant::now();
    let done = decide(w, &w.token, &p).await;
    assert_ne!(done["isError"], true, "{done}");
    started.elapsed()
}

fn wrong_rows(slug: &str, factor: i64) -> i64 {
    let slug = slug.to_string();
    db(move |c| {
        c.query_one(
            "SELECT count(*) FROM dataset_records WHERE app_slug = $1 AND ((data->>'doubled')::float8 IS DISTINCT FROM (data->>'score')::float8 * $2::float8)",
            &[&slug, &(factor as f64)],
        )
        .unwrap()
        .get(0)
    })
}

// 1. A formula changes on a big table: the approval returns at once, the field is calculating, and a job makes every value right.
#[tokio::test]
async fn changing_a_formula_on_a_big_table_returns_at_once_and_a_job_recalculates_every_value() {
    let _turn = LOCK.lock().await;
    let w = world().await;
    let took = approve_triple(&w, BIG).await;
    println!("calc: the approval returned in {took:?} on {BIG} rows");
    assert!(took < Duration::from_secs(1), "the approval took {took:?}");
    assert_eq!(calc_state(&w.slug, "doubled").as_deref(), Some("calculating"));
    assert_eq!(wrong_rows(&w.slug, 3), BIG, "nothing is recalculated by the approval");

    // A page says which field is calculating and how far along.
    let id = job_of(&w.slug, "recalc_field");
    db(move |c| c.execute("UPDATE jobs SET progress_done = 126000, progress_total = 200000 WHERE id = $1", &[&id]).unwrap());
    let page = tool_full(&w, &w.token, "list_records", json!({ "app_slug": w.slug, "table_id": "t1", "limit": 3 })).await;
    assert_eq!(page["structuredContent"]["field_states"], json!({ "doubled": "calculating" }), "{page}");
    assert_eq!(page["structuredContent"]["field_progress"], json!({ "doubled": 63 }), "{page}");
    assert_eq!(page["structuredContent"]["records"][0]["data"]["doubled"], 2, "the old stored value is returned until the job reaches it");

    let repo = repo();
    let runner = JobRunner::start(&repo, JOB_KINDS, fast()).unwrap();
    let job = wait_for_job(&repo, id);
    runner.stop();
    assert_eq!(job.state, "done", "{:?}", job.error);
    assert_eq!(wrong_rows(&w.slug, 3), 0, "every value matches the new formula");
    assert_eq!(calc_state(&w.slug, "doubled").as_deref(), Some("ready"));
    let page = tool_full(&w, &w.token, "list_records", json!({ "app_slug": w.slug, "table_id": "t1", "limit": 3 })).await;
    assert!(page["structuredContent"].get("field_states").is_none(), "nothing is calculating now: {page}");
}

// 2. A filter or sort on a calculating field is refused with `calculating`.
#[tokio::test]
async fn a_filter_or_sort_on_a_calculating_field_is_refused() {
    let _turn = LOCK.lock().await;
    let w = world().await;
    approve_triple(&w, SMALL).await;
    let probe = json!({ "conjunction": "And", "clauses": [{ "id": "c", "field_name": "doubled", "operator": "GreaterThan", "value": "10" }] });
    let r = tool_full(&w, &w.token, "list_records", json!({ "app_slug": w.slug, "table_id": "t1", "filter": probe })).await;
    assert_eq!(r["isError"], true, "{r}");
    assert_eq!(r["structuredContent"]["code"], "calculating", "{r}");
    assert!(r["content"][0]["text"].as_str().unwrap().contains("still being calculated"), "{r}");
    let r = tool_full(&w, &w.token, "list_records", json!({ "app_slug": w.slug, "table_id": "t1", "sort": { "field_name": "doubled", "direction": "Desc" } })).await;
    assert_eq!(r["structuredContent"]["code"], "calculating", "{r}");
    // A field that is not calculating still sorts.
    let r = tool_full(&w, &w.token, "list_records", json!({ "app_slug": w.slug, "table_id": "t1", "limit": 2, "sort": { "field_name": "score", "direction": "Desc" } })).await;
    assert_ne!(r["isError"], true, "{r}");
}

// 3. A write during the job uses the new formula at once, and the job leaves that record alone.
#[tokio::test]
async fn a_record_written_during_the_job_is_right_and_the_job_does_not_touch_it() {
    let _turn = LOCK.lock().await;
    let w = world().await;
    approve_triple(&w, SMALL).await;
    let rid = format!("rec-{}-{:08}", w.slug, 77);
    let (s, body) = send(&w.app, "PATCH", &format!("/api/v1/apps/{}/records/{rid}", w.slug), &w.token, Some(json!({ "data": { "_table_id": "t1", "score": 1000, "code": "5" }, "version": 1 }))).await;
    assert_eq!(s, StatusCode::OK, "{body}");
    assert_eq!(body["data"]["doubled"].as_f64(), Some(3000.0), "the write used the new formula: {body}");
    let read = {
        let (slug, rid) = (w.slug.clone(), rid.clone());
        move || {
            let (slug, rid) = (slug.clone(), rid.clone());
            db(move |c| c.query_one("SELECT data::text, xmin::text FROM dataset_records WHERE app_slug = $1 AND id = $2", &[&slug, &rid]).map(|r| (r.get::<_, String>(0), r.get::<_, String>(1))).unwrap())
        }
    };
    let before = read();

    let repo = repo();
    let id = job_of(&w.slug, "recalc_field");
    let runner = JobRunner::start(&repo, JOB_KINDS, fast()).unwrap();
    wait_for_job(&repo, id);
    runner.stop();
    assert_eq!(read(), before, "the job did not rewrite the record");
    let wrong: i64 = {
        let slug = w.slug.clone();
        db(move |c| c.query_one("SELECT count(*) FROM dataset_records WHERE app_slug = $1 AND (data->>'doubled')::float8 IS DISTINCT FROM (data->>'score')::float8 * 3", &[&slug]).unwrap().get(0))
    };
    assert_eq!(wrong, 0, "and every other record is right");
}

// 4. A worker that died halfway: the job resumes from its checkpoint. Nothing is skipped, and the first half is not computed again.
#[tokio::test]
async fn a_killed_recalculation_resumes_from_its_checkpoint() {
    let _turn = LOCK.lock().await;
    let w = world().await;
    approve_triple(&w, SMALL).await;
    let repo = repo();
    let id = job_of(&w.slug, "recalc_field");
    let cfg = JobConfig { lock_secs: 1, recovery_secs: 1, ..fast() };
    {
        let (repo, cfg) = (repo.clone(), cfg.clone());
        let claimed = std::thread::spawn(move || {
            let mut ghost = repo.dedicated_client().unwrap();
            claim_next(&mut ghost, "ghost", JOB_KINDS, &cfg).unwrap().is_some()
        })
        .join()
        .unwrap();
        assert!(claimed, "the ghost claimed the job");
    }
    // It wrote the first half, saved its checkpoint, and died.
    let half = SMALL / 2;
    let (slug, last) = (w.slug.clone(), format!("rec-{}-{:08}", w.slug, half));
    db(move |c| {
        c.execute("UPDATE dataset_records SET data = jsonb_set(data, '{doubled}', to_jsonb((data->>'score')::int * 3)) WHERE app_slug = $1 AND id <= $2", &[&slug, &last]).unwrap();
        let cp = json!({ "after": last, "done": half });
        c.execute("UPDATE jobs SET checkpoint = $2, locked_until = NOW() - interval '1 second' WHERE id = $1", &[&id, &cp]).unwrap();
    });
    let first = {
        let (slug, rid) = (w.slug.clone(), format!("rec-{}-{:08}", w.slug, 1));
        db(move |c| c.query_one("SELECT xmin::text FROM dataset_records WHERE app_slug = $1 AND id = $2", &[&slug, &rid]).unwrap().get::<_, String>(0))
    };

    let runner = JobRunner::start(&repo, JOB_KINDS, cfg).unwrap();
    let job = wait_for_job(&repo, id);
    runner.stop();
    assert_eq!(job.state, "done", "{:?}", job.error);
    assert_eq!(wrong_rows(&w.slug, 3), 0, "no record is skipped");
    let after = {
        let (slug, rid) = (w.slug.clone(), format!("rec-{}-{:08}", w.slug, 1));
        db(move |c| c.query_one("SELECT xmin::text FROM dataset_records WHERE app_slug = $1 AND id = $2", &[&slug, &rid]).unwrap().get::<_, String>(0))
    };
    assert_eq!(after, first, "the first half was not computed again");
}

// 5. A preview of text to number counts what would blank, with 50 examples. The proposal is flagged. Approval blanks it, with history.
#[tokio::test]
async fn a_conversion_previews_flags_converts_with_history_and_can_be_undone() {
    let _turn = LOCK.lock().await;
    let w = world().await;
    go_live(&w, 3_000).await;
    let repo = repo();
    let runner = JobRunner::start(&repo, JOB_KINDS, fast()).unwrap();

    // 3,000 rows: every 25th is `n/a`, so 120 would become blank. The rest are numeric text.
    let (s, body) = send(&w.app, "POST", &format!("/api/v1/apps/{}/fields/convert-preview", w.slug), &w.token, Some(json!({ "table_id": "t1", "field": "code", "to_type": "Number" }))).await;
    assert_eq!(s, StatusCode::ACCEPTED, "{body}");
    let preview = wait_for_job(&repo, body["job_id"].as_str().unwrap().parse().unwrap());
    assert_eq!(preview.state, "done", "{:?}", preview.error);
    let result = preview.result.clone().unwrap();
    assert_eq!((result["blanks"].as_i64(), result["converts"].as_i64()), (Some(120), Some(2_880)), "{result}");
    assert_eq!(result["examples"]["blank"].as_array().unwrap().len(), 50);
    assert_eq!(result["examples"]["convert"].as_array().unwrap().len(), 50);
    assert_eq!(result["examples"]["blank"][0]["old"], "n/a");

    // Only an owner or admin may ask.
    let (s, _) = send(&w.app, "POST", &format!("/api/v1/apps/{}/fields/convert-preview", w.slug), &issue_test_token_and_user("outsider.student@state.edu"), Some(json!({ "table_id": "t1", "field": "code", "to_type": "Number" }))).await;
    assert!(s == StatusCode::FORBIDDEN || s == StatusCode::NOT_FOUND, "{s}");

    // The proposal is flagged for data loss, with the preview attached, so a second person must approve.
    let p = propose(&w, &manifest(&w.slug, "{score} * 2", FieldType::Number), "Code is a number").await;
    let shown = tool_full(&w, &w.token, "get_proposal", json!({ "proposal_id": p["proposal_id"] })).await;
    let checks = shown["structuredContent"]["checks"].as_array().cloned().unwrap_or_default();
    let loss = checks.iter().find(|c| c["name"] == "data_loss").unwrap_or_else(|| panic!("{shown}"));
    assert_eq!(loss["outcome"], "fail", "{loss}");
    assert!(loss["messages"].to_string().contains("120 values"), "{loss}");
    assert_eq!(loss["previews"][0]["blanks"], 120, "{loss}");
    let refused = decide(&w, &w.token, &p).await;
    assert_eq!(refused["isError"], true, "the proposer cannot approve a flagged change alone: {refused}");

    let done = decide(&w, &w.second, &p).await;
    assert_ne!(done["isError"], true, "{done}");
    let convert = job_of(&w.slug, "convert_field");
    let job = wait_for_job(&repo, convert);
    assert_eq!(job.state, "done", "{:?}", job.error);
    let batch = job.result.clone().unwrap()["batch_id"].as_str().unwrap().to_string();
    assert_eq!(calc_state(&w.slug, "code").as_deref(), Some("ready"));

    let slug = w.slug.clone();
    let counts = db(move |c| {
        let numbers: i64 = c.query_one("SELECT count(*) FROM dataset_records WHERE app_slug = $1 AND jsonb_typeof(data->'code') = 'number'", &[&slug]).unwrap().get(0);
        let blanks: i64 = c.query_one("SELECT count(*) FROM dataset_records WHERE app_slug = $1 AND NOT (data ? 'code')", &[&slug]).unwrap().get(0);
        (numbers, blanks)
    });
    assert_eq!(counts, (2_880, 120), "numbers where they converted, no value where they did not");
    let slug = w.slug.clone();
    let b = batch.clone();
    let history = db(move |c| {
        c.query(
            "SELECT actor_kind, changes -> 'code' ->> 'old', changes -> 'code' ->> 'new' FROM record_history WHERE app_slug = $1 AND batch_id::text = $2 AND (changes -> 'code' ->> 'old') = 'n/a' LIMIT 3",
            &[&slug, &b],
        )
        .unwrap()
        .iter()
        .map(|r| (r.get::<_, String>(0), r.get::<_, String>(1), r.get::<_, Option<String>>(2)))
        .collect::<Vec<_>>()
    });
    assert_eq!(history.len(), 3, "the old value of every blanked record is in history");
    assert!(history.iter().all(|h| h.0 == "system" && h.1 == "n/a" && h.2.is_none()), "{history:?}");

    // 6. Undoing the batch restores the old values.
    let (s, body) = send(&w.app, "POST", &format!("/api/v1/apps/{}/history/{batch}/undo", w.slug), &w.token, Some(json!({}))).await;
    assert_eq!(s, StatusCode::OK, "{body}");
    let slug = w.slug.clone();
    let restored = db(move |c| {
        let strings: i64 = c.query_one("SELECT count(*) FROM dataset_records WHERE app_slug = $1 AND jsonb_typeof(data->'code') = 'string'", &[&slug]).unwrap().get(0);
        let na: i64 = c.query_one("SELECT count(*) FROM dataset_records WHERE app_slug = $1 AND data->>'code' = 'n/a'", &[&slug]).unwrap().get(0);
        (strings, na)
    });
    assert_eq!(restored, (3_000, 120), "every value is the text it was");
    runner.stop();
}
