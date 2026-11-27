//! Sensitive content phase 2, test 9: a scan of a million rows runs in batches. It records the total time and the longest single batch,
//! and a record write made while the scan runs completes, with its latency recorded.
//!
//! This seeds 1,000,000 rows, so it is in the heavy group of the CI run.

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::Router;
use http_body_util::BodyExt;
use scaffoldry_engine::{AppManifest, AppTable, FieldSpec, FieldType};
use scaffoldry_server::build_app_with_state;
use scaffoldry_server::jobs::{get_job, JobConfig, JobRunner, JobState, JOB_KINDS};
use scaffoldry_server::repository::PostgresRepository;
use scaffoldry_server::service::identity::issue_test_token_and_user;
use scaffoldry_server::state::{CollaboratorRecord, ServerState};
use serde_json::{json, Value};
use std::sync::Arc;
use tower::ServiceExt;
use uuid::Uuid;

const WORKSPACE: &str = "ws-bio-lab";
const COMPLIANCE: &str = "hold.officer@state.edu";

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
                let _ = c.execute("DELETE FROM data_labels WHERE app_slug LIKE $1", &[&like]);
                let _ = c.execute("DELETE FROM content_findings WHERE app_slug LIKE $1", &[&like]);
                let _ = c.execute("DELETE FROM content_expected WHERE app_slug LIKE $1", &[&like]);
                let _ = c.execute("DELETE FROM authorized_stores WHERE app_slug LIKE $1", &[&like]);
                let _ = c.execute("DELETE FROM app_manifests WHERE slug LIKE $1", &[&like]);
            }
        })
        .join();
    }
}

fn manifest(slug: &str) -> AppManifest {
    let notes = vec![
        FieldSpec::simple("title", "Title", FieldType::Text, false, false),
        FieldSpec::simple("body", "Body", FieldType::LongText, false, false),
        FieldSpec::simple("amount", "Amount", FieldType::Number, false, false),
    ];
    let cards = vec![FieldSpec::simple("holder", "Holder", FieldType::Text, false, false), FieldSpec::simple("card", "Card", FieldType::Text, false, false)];
    let table = |id: &str, fields: Vec<FieldSpec>, primary: &str| AppTable {
        retired_fields: Vec::new(), key: None, unit_field: None, unit_required: false,
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
        tables: vec![table("notes", notes, "title"), table("cards", cards, "holder")],
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
    compliance: String,
    editor: String,
    _cleanup: Cleanup,
}

async fn world() -> World {
    let tag = Uuid::new_v4().to_string()[..8].to_string();
    let slug = format!("dg-{tag}");
    let (editor_e, viewer_e) = (format!("editor-{tag}@state.edu"), format!("viewer-{tag}@state.edu"));
    let state = Arc::new(ServerState::new().expect("state"));
    state.seed_demo().expect("seed");
    state.engine.write().unwrap().register_manifest(manifest(&slug)).unwrap();
    collaborator(&state, &editor_e, "editor");
    collaborator(&state, &viewer_e, "viewer");
    // Issuing the token makes the person. The role is put on them after.
    let compliance = issue_test_token_and_user(COMPLIANCE);
    db(|c| {
        c.execute(
            "INSERT INTO roles (id, person_id, organization_id, role_title, scoped_affiliation, is_primary, source) \
             SELECT gen_random_uuid(), p.id, o.id, 'Compliance Officer', 'compliance', true, 'api' \
             FROM persons p, organizations o WHERE LOWER(p.eppn) = LOWER($1) AND o.parent_id IS NULL ON CONFLICT DO NOTHING",
            &[&COMPLIANCE],
        )
        .unwrap();
    });
    World {
        app: build_app_with_state(state.clone()).expect("router"),
        state: state.clone(),
        compliance,
        editor: issue_test_token_and_user(&editor_e),
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


const SSN: &str = "123-45-6789";

fn repo() -> PostgresRepository {
    PostgresRepository::connect(None).expect("database is reachable")
}

/// The runner is given the server's state, as it is in `main`, so a job reads the apps and the settings the server holds.
fn fast(w: &World) -> JobConfig {
    JobConfig { workers: 2, poll_ms: 20, backoff_secs: [0, 0, 0], state: Some(JobState(w.state.clone())), ..JobConfig::default() }
}


fn wait_for_job(repo: &PostgresRepository, id: uuid::Uuid) -> scaffoldry_server::jobs::Job {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(120);
    loop {
        let job = get_job(repo, id).expect("query").expect("job exists");
        if job.state == "done" || job.state == "failed" {
            return job;
        }
        assert!(std::time::Instant::now() < deadline, "job stuck in {}", job.state);
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
}


async fn findings(w: &World, query: &str) -> Vec<Value> {
    let (s, v) = send(&w.app, "GET", &format!("/api/v1/admin/content-findings?app_slug={}&{query}", w.slug), &w.compliance, None).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    v["findings"].as_array().unwrap().clone()
}




/// Everything the database holds that a value could have leaked into, as one string.
fn everything_stored() -> String {
    db(|c| {
        let mut out = String::new();
        for sql in [
            "SELECT coalesce(string_agg(row_to_json(f)::text, ' '), '') FROM content_findings f",
            "SELECT coalesce(string_agg(row_to_json(e)::text, ' '), '') FROM content_expected e",
            "SELECT coalesce(string_agg(l.rationale || ' ' || l.payload::text, ' '), '') FROM governance_ledger l",
            "SELECT coalesce(string_agg(j.payload::text || ' ' || coalesce(j.result::text, '') || ' ' || coalesce(j.log::text, '') || ' ' || coalesce(j.checkpoint::text, ''), ' '), '') FROM jobs j",
        ] {
            out.push_str(&c.query_one(sql, &[]).unwrap().get::<_, String>(0));
            out.push(' ');
        }
        out
    })
}


const ROWS: i64 = 1_000_000;

/// A million notes. One in a thousand names an SSN, so the scan has something to find and the findings stay few.
fn fill(w: &World) {
    let slug = w.slug.clone();
    db(move |c| {
        c.execute(
            "INSERT INTO dataset_records (id, app_slug, data, ceds_mapping, is_ferpa_sensitive, table_id, created_at) \
             SELECT $1 || '-' || lpad(g::text, 8, '0'), $1, \
                    jsonb_build_object('_table_id', 'notes', 'title', 'note ' || g, \
                                       'body', CASE WHEN g % 1000 = 0 THEN 'ssn 123-45-6789 for note ' || g ELSE 'ordinary text for note ' || g END), \
                    '{}'::jsonb, false, 'notes', TIMESTAMPTZ '2026-01-01 00:00:00+00' + g * interval '1 millisecond' \
             FROM generate_series(1, $2::bigint) g",
            &[&slug, &ROWS],
        )
        .unwrap();
        c.batch_execute("ANALYZE dataset_records").unwrap();
    });
}

#[ignore = "seeds 1,000,000 rows. CI runs it in release mode in the million-rows job"]
#[tokio::test]
async fn a_scan_of_a_million_rows_runs_in_batches_and_a_write_completes_while_it_runs() {
    let _db = common::TestDb::new();
    let w = world().await;
    fill(&w);

    let (s, v) = send(&w.app, "POST", "/api/v1/admin/content-scans", &w.compliance, Some(json!({ "app_slug": w.slug, "table_id": "notes" }))).await;
    assert_eq!(s, StatusCode::ACCEPTED, "{v}");
    let id: uuid::Uuid = v["job_id"].as_str().unwrap().parse().unwrap();
    let repo = repo();
    let started = std::time::Instant::now();
    let runner = JobRunner::start(&repo, JOB_KINDS, fast(&w)).unwrap();

    // While the scan runs, write a record every half second through the real route and time each write.
    let mut latencies = Vec::new();
    let mut n = 0;
    loop {
        let job = get_job(&repo, id).unwrap().unwrap();
        if job.state == "done" || job.state == "failed" {
            break;
        }
        let t = std::time::Instant::now();
        let (s, v) = send(&w.app, "POST", &format!("/api/v1/apps/{}/records", w.slug), &w.editor, Some(json!({ "data": { "_table_id": "notes", "title": format!("live {n}"), "body": "written during the scan" } }))).await;
        assert_eq!(s, StatusCode::CREATED, "a write completes while the scan runs: {v}");
        latencies.push(t.elapsed());
        n += 1;
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    }
    let job = wait_for_job(&repo, id);
    runner.stop();
    let total = started.elapsed();
    assert_eq!(job.state, "done", "{:?}", job.error);

    let result = job.result.clone().unwrap_or(Value::Null);
    let longest = result["longest_batch_ms"].as_i64().unwrap();
    latencies.sort();
    let worst_write = latencies.last().copied().unwrap_or_default();
    println!(
        "content scan, {ROWS} rows: total {:.1} s, {} batches, longest single batch {longest} ms, {} writes during the scan, median write {:?}, slowest write {:?}",
        total.as_secs_f64(),
        result["batches"],
        latencies.len(),
        latencies.get(latencies.len() / 2).copied().unwrap_or_default(),
        worst_write
    );
    assert!(result["records"].as_i64().unwrap() >= ROWS, "every row was read: {result}");
    assert!(result["batches"].as_i64().unwrap() >= ROWS / 1_000, "in batches of a thousand: {result}");
    assert!(longest < 2_000, "no batch held a lock for two seconds: {longest} ms");
    assert!(!latencies.is_empty(), "the scan ran long enough to write during it");
    assert!(worst_write < std::time::Duration::from_secs(2), "a write was not held up by the scan: {worst_write:?}");
    let rows = findings(&w, "").await;
    assert_eq!(rows.len(), 1, "{rows:?}");
    assert_eq!(rows[0]["record_count"], ROWS / 1_000, "one record in a thousand names an SSN");
    assert!(!everything_stored().contains(SSN));
}
