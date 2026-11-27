//! Data layer phase 5: the AI review.
//!
//! The review job reads a dataset's rows in batches. For a row whose content matches a category and that does not already carry it,
//! it writes a finding with the field, the category, a count and a few record ids. It stores no value. It changes no tag, floor,
//! label, grant or access. A reader sees a finding only for rows the phase 3 gate lets them read.

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::Router;
use http_body_util::BodyExt;
use scaffoldry_server::build_app_with_state;
use scaffoldry_server::jobs::{get_job, JobConfig, JobRunner, JobState, JOB_KINDS};
use scaffoldry_server::service::identity::issue_test_token_and_user;
use scaffoldry_server::state::{OrganizationNode, RoleRow, ServerState, WorkspaceRecord};
use serde_json::{json, Value};
use std::sync::Arc;
use tower::ServiceExt;
use uuid::Uuid;

const WS: &str = "ws-review";
const ROOT: &str = "00000000-0000-0000-0000-000000000001";
const ADMIN: &str = "jordan.lee@state.edu";
const COMPLIANCE: &str = "compliance.officer@state.edu";
const CHEM_COMPLIANCE: &str = "chem.compliance@science.state.edu";
const SSN: &str = "123-45-6789";
const TABLE: &str = "students";

struct World {
    db: common::TestDb,
    state: Arc<ServerState>,
    app: Router,
    admin: String,
    compliance: String,
    chem_compliance: String,
    physics: Uuid,
    chemistry: Uuid,
    slug: String,
}

async fn rest(app: &Router, method: &str, uri: &str, token: &str, body: Option<Value>) -> (StatusCode, Value) {
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

fn unit(state: &ServerState, name: &str, parent: Uuid) -> Uuid {
    let id = Uuid::new_v4();
    let node = OrganizationNode { id, parent_id: Some(parent), name: name.to_string(), code: name.to_uppercase(), org_type: "Unit".to_string(), hierarchy_key: "academic".to_string(), synthetic: false };
    state.organizations.write().unwrap().insert(id, node.clone());
    state.persist_organization(&node).unwrap();
    id
}

fn appoint(state: &ServerState, eppn: &str, unit: Uuid, scoped_affiliation: &str, position: Option<&str>) {
    state
        .persist_role(&RoleRow {
            id: Uuid::new_v4(),
            person_id: Uuid::new_v4(),
            eppn: eppn.to_string(),
            organization_id: unit,
            role_title: scoped_affiliation.to_string(),
            scoped_affiliation: scoped_affiliation.to_string(),
            is_primary: false,
            source: "test".to_string(),
            position_key: position.map(str::to_string),
        })
        .unwrap();
}

fn workspace(state: &ServerState, unit: Uuid) {
    let ws = WorkspaceRecord {
        id: WS.to_string(),
        name: "Review".to_string(),
        code: "WS-REVIEW".to_string(),
        organization: "INST".to_string(),
        department: "science".to_string(),
        description: "Test".to_string(),
        icon: "dna".to_string(),
        lead: "Lead".to_string(),
        visibility: "departmental".to_string(),
        coordinates: Default::default(),
        data_classification: "restricted".to_string(),
        created_at: chrono::Utc::now().to_rfc3339(),
        organization_id: Some(unit),
        synthetic: false,
    };
    state.workspaces.write().unwrap().insert(WS.to_string(), ws.clone());
    state.repository.upsert_workspace(&ws).unwrap();
    state.collaborators.write().unwrap().insert(WS.to_string(), Vec::new());
}

/// `pii` is signalled by the SSN detector and asks nothing of a reader. `hr` asks for a registrar at the row's unit, so a dataset
/// floor of `hr` is what separates the readers of Chemistry rows from the readers of Physics rows.
fn categories() -> Value {
    json!([
        { "id": "pii", "name": "PII", "protected": true, "detectors": ["us_ssn"] },
        { "id": "hr", "name": "HR", "protected": false, "detectors": [], "requires": [{ "position": "registrar", "reach": "unit" }] }
    ])
}

async fn world() -> World {
    let db = common::TestDb::new();
    let state = db.state();
    let root = Uuid::parse_str(ROOT).unwrap();
    let science = unit(&state, "science", root);
    let physics = unit(&state, "physics", science);
    let chemistry = unit(&state, "chemistry", science);
    workspace(&state, science);
    db.run(|c| c.execute("INSERT INTO position_types (key, name, description, org_types, max_holders, created_by) VALUES ('registrar', 'Registrar', '', '[]', 10, 'test') ON CONFLICT DO NOTHING", &[]).unwrap());
    appoint(&state, COMPLIANCE, root, "compliance", None);
    appoint(&state, CHEM_COMPLIANCE, root, "compliance", None);
    appoint(&state, CHEM_COMPLIANCE, chemistry, "faculty", Some("registrar"));
    state.settings.write().unwrap().insert("sensitivity.categories".to_string(), categories());
    let app = build_app_with_state(state.clone()).expect("router");
    let slug = format!("students-{}", &Uuid::new_v4().to_string()[..8]);
    let w = World {
        db,
        state,
        app,
        admin: issue_test_token_and_user(ADMIN),
        compliance: issue_test_token_and_user(COMPLIANCE),
        chem_compliance: issue_test_token_and_user(CHEM_COMPLIANCE),
        physics,
        chemistry,
        slug,
    };
    let manifest = json!({
        "slug": w.slug, "title": "Students", "description": "Students by unit", "organization_code": "PHYSICS", "department": "science", "workspace_id": WS, "views": [],
        "tables": [{
            "id": TABLE, "name": "Students", "slug": TABLE, "primary_field": "name",
            "fields": [
                { "name": "name", "label": "Name", "field_type": "Text", "required": false, "ferpa_sensitive": false },
                { "name": "note", "label": "Note", "field_type": "LongText", "required": false, "ferpa_sensitive": false },
                { "name": "unit", "label": "Unit", "field_type": "Unit", "hierarchy": "academic", "required": false, "ferpa_sensitive": false }
            ],
            "unit_field": "unit"
        }]
    });
    let (s, b) = rest(&w.app, "POST", &format!("/api/v1/workspaces/{WS}/apps"), &w.admin, Some(manifest)).await;
    assert_eq!(s, StatusCode::CREATED, "{b}");
    let slug = w.slug.clone();
    w.db.run(move |c| c.execute("UPDATE index_status SET state = 'ready' WHERE app_slug = $1", &[&slug]).unwrap());
    w
}

/// Puts a row straight in the table, so no write finding is made beside the review's. Returns its id.
fn put(w: &World, n: usize, note: &str, unit: Option<Uuid>, tags: &[&str]) -> String {
    let (slug, id) = (w.slug.clone(), format!("{}-{n:06}", w.slug));
    let mut data = json!({ "_table_id": TABLE, "name": format!("student {n}"), "note": note });
    if let Some(u) = unit {
        data["unit"] = json!(u.to_string());
    }
    if !tags.is_empty() {
        data["_tags"] = json!(tags);
    }
    let rid = id.clone();
    w.db.run(move |c| {
        c.execute(
            "INSERT INTO dataset_records (id, app_slug, data, ceds_mapping, is_ferpa_sensitive, table_id, created_at, unit_id) \
             VALUES ($1, $2, $3, '{}'::jsonb, false, $4, TIMESTAMPTZ '2026-01-01 00:00:00+00' + $5::int * interval '1 millisecond', $6)",
            &[&rid, &slug, &data, &TABLE, &(n as i32), &unit],
        )
        .unwrap();
    });
    id
}

fn runner(w: &World) -> JobConfig {
    JobConfig { workers: 2, poll_ms: 20, backoff_secs: [0, 0, 0], state: Some(JobState(w.state.clone())), ..JobConfig::default() }
}

fn wait_for_job(w: &World, id: Uuid) -> scaffoldry_server::jobs::Job {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(300);
    loop {
        let job = get_job(&w.state.repository, id).expect("query").expect("job exists");
        if job.state == "done" || job.state == "failed" {
            return job;
        }
        assert!(std::time::Instant::now() < deadline, "job stuck in {}", job.state);
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
}

/// Asks for a review of the table, by the route that asks for a scan, and runs jobs until it is done.
async fn review_with(w: &World, batch: i64) -> scaffoldry_server::jobs::Job {
    let (s, v) = rest(&w.app, "POST", "/api/v1/admin/content-scans", &w.compliance, Some(json!({ "app_slug": w.slug, "table_id": TABLE, "review": true, "batch_size": batch }))).await;
    assert_eq!(s, StatusCode::ACCEPTED, "{v}");
    let id: Uuid = v["job_id"].as_str().unwrap().parse().unwrap();
    let job = {
        let r = JobRunner::start(&w.state.repository, JOB_KINDS, runner(w)).unwrap();
        let job = wait_for_job(w, id);
        r.stop();
        job
    };
    assert_eq!(job.state, "done", "{:?}", job.error);
    assert_eq!(job.kind, "review_records", "the route queued a review, not a scan");
    job
}

async fn review(w: &World) -> scaffoldry_server::jobs::Job {
    review_with(w, 3).await
}

/// The findings a reader is shown, by the existing route.
async fn listed(w: &World, token: &str) -> (StatusCode, Value) {
    rest(&w.app, "GET", &format!("/api/v1/admin/content-findings?app_slug={}&limit=200", w.slug), token, None).await
}

async fn reviewed(w: &World, token: &str) -> Vec<Value> {
    let (s, v) = listed(w, token).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    v["findings"].as_array().unwrap().iter().filter(|f| f["source"] == "review").cloned().collect()
}

async fn set_floor(w: &World, floor: Value) {
    let (s, v) = rest(&w.app, "GET", "/api/v1/data/datasets", &w.admin, None).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let id = v["datasets"].as_array().unwrap().iter().find(|d| d["app_slug"] == json!(w.slug) && d["table_id"] == json!(TABLE)).expect("a dataset")["id"].as_str().unwrap().to_string();
    let (s, b) = rest(&w.app, "PUT", &format!("/api/v1/admin/datasets/{id}/label-floor"), &w.admin, Some(json!({ "floor": floor, "reason": "HR data is held here" }))).await;
    assert_eq!(s, StatusCode::OK, "{b}");
}

/// A fingerprint of everything that decides who reads what: rows with their tags and units, floors, labels, and every grant table.
fn access_snapshot(w: &World) -> String {
    w.db.run(|c| {
        let names: Vec<String> = c
            .query(
                "SELECT table_name FROM information_schema.tables WHERE table_schema = 'public' AND table_type = 'BASE TABLE' \
                 AND (table_name LIKE '%grant%' OR table_name IN ('dataset_records', 'datasets', 'data_labels', 'record_audience', 'roles', 'position_holders', 'authorized_stores')) ORDER BY 1",
                &[],
            )
            .unwrap()
            .iter()
            .map(|r| r.get(0))
            .collect();
        assert!(names.len() >= 5, "the snapshot covers the access tables: {names:?}");
        let mut out = String::new();
        for n in names {
            let h: String = c.query_one(&format!("SELECT coalesce(md5(string_agg(t::text, ',' ORDER BY t::text)), '') FROM {n} t"), &[]).unwrap().get(0);
            out.push_str(&format!("{n}={h};"));
        }
        out
    })
}

/// Everything the database holds that a value could have leaked into, besides the rows themselves.
fn everything_stored(w: &World) -> String {
    w.db.run(|c| {
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

fn ids_of(f: &Value) -> Vec<String> {
    f["sample_record_ids"].as_array().unwrap().iter().map(|v| v.as_str().unwrap().to_string()).collect()
}

// 1. A matching row gets a finding with the field, the category, a count and ids, and no value is stored anywhere.
#[tokio::test(flavor = "multi_thread")]
async fn a_matching_row_gets_a_finding_with_field_category_count_and_ids_and_no_value() {
    let w = world().await;
    let hit = put(&w, 1, &format!("ssn on file {SSN}"), Some(w.chemistry), &[]);
    let also = put(&w, 2, &format!("again {SSN} and {SSN}"), Some(w.physics), &[]);
    put(&w, 3, "nothing here", Some(w.chemistry), &[]);

    let job = review(&w).await;
    assert_eq!(job.result.as_ref().unwrap()["records"], 3, "every row was read");

    let found = reviewed(&w, &w.admin).await;
    assert_eq!(found.len(), 1, "{found:?}");
    let f = &found[0];
    assert_eq!((f["field"].as_str(), f["category"].as_str(), f["status"].as_str()), (Some("note"), Some("pii"), Some("open")));
    assert_eq!((f["match_count"].as_i64(), f["record_count"].as_i64()), (Some(3), Some(2)), "three matches in two rows: {f}");
    let mut ids = ids_of(f);
    ids.sort();
    assert_eq!(ids, vec![hit, also]);
    // The value is in the rows, which it has to be, and nowhere a finding, a job or the ledger could keep it.
    assert!(!everything_stored(&w).contains(SSN), "a planted value was stored beside the rows");
    assert!(!everything_stored(&w).contains(&SSN.replace('-', "")));
    // A repeat of the review counts the same rows again as one finding, not two.
    review(&w).await;
    let again = reviewed(&w, &w.admin).await;
    assert_eq!(again.len(), 1);
    assert_eq!((again[0]["match_count"].as_i64(), again[0]["record_count"].as_i64()), (Some(3), Some(2)));
}

// 2. No tag, floor, grant or access changes.
#[tokio::test(flavor = "multi_thread")]
async fn the_review_changes_no_tag_floor_label_grant_or_access() {
    let w = world().await;
    put(&w, 1, &format!("ssn {SSN}"), Some(w.chemistry), &[]);
    put(&w, 2, "plain", Some(w.physics), &["pii"]);
    put(&w, 3, &format!("ssn {SSN}"), None, &[]);
    set_floor(&w, json!(["hr"])).await;
    let before = access_snapshot(&w);
    let flags_before: Vec<(String, bool, Vec<String>)> = {
        let slug = w.slug.clone();
        w.db.run(move |c| c.query("SELECT id, is_ferpa_sensitive, tags FROM dataset_records WHERE app_slug = $1 ORDER BY id", &[&slug]).unwrap().iter().map(|r| (r.get(0), r.get(1), r.get(2))).collect())
    };
    let ledger_before: i64 = w.db.run(|c| c.query_one("SELECT count(*) FROM governance_ledger WHERE decision_type IN ('DataLabelChanged', 'ContentFindingDecided')", &[]).unwrap().get(0));

    review(&w).await;
    assert_eq!(reviewed(&w, &w.admin).await.len(), 1, "the review did find something");

    assert_eq!(access_snapshot(&w), before, "the review left every access table as it was");
    let slug = w.slug.clone();
    let flags_after: Vec<(String, bool, Vec<String>)> = w.db.run(move |c| c.query("SELECT id, is_ferpa_sensitive, tags FROM dataset_records WHERE app_slug = $1 ORDER BY id", &[&slug]).unwrap().iter().map(|r| (r.get(0), r.get(1), r.get(2))).collect());
    assert_eq!(flags_after, flags_before, "no row was tagged or flagged");
    let ledger_after: i64 = w.db.run(|c| c.query_one("SELECT count(*) FROM governance_ledger WHERE decision_type IN ('DataLabelChanged', 'ContentFindingDecided')", &[]).unwrap().get(0));
    assert_eq!(ledger_after, ledger_before, "no label was changed");
}

// 3. A row that does not match gets no finding. A row that already carries the category, as a tag or by the floor, gets none either.
#[tokio::test(flavor = "multi_thread")]
async fn a_row_that_does_not_match_or_already_carries_the_category_gets_no_finding() {
    let w = world().await;
    put(&w, 1, "no numbers at all", Some(w.chemistry), &[]);
    put(&w, 2, "a phone-like 555-1234 and 12-34-5678", Some(w.chemistry), &[]);
    // Matches, but the row is tagged with the category already.
    put(&w, 3, &format!("ssn {SSN}"), Some(w.chemistry), &["pii"]);
    review(&w).await;
    assert!(reviewed(&w, &w.admin).await.is_empty(), "no finding for a clean row or a row already tagged");

    // A row that matches and is not tagged does get one, so the empty list above is the review's answer and not a broken job.
    put(&w, 4, &format!("ssn {SSN}"), Some(w.chemistry), &[]);
    review(&w).await;
    let found = reviewed(&w, &w.admin).await;
    assert_eq!(found.len(), 1);
    assert_eq!(found[0]["record_count"], 1, "the tagged row was not counted: {found:?}");

    // A dataset floor of the category covers every row. Nothing is proposed for any of them, and the finding of before is withdrawn.
    set_floor(&w, json!(["pii"])).await;
    review(&w).await;
    assert!(reviewed(&w, &w.admin).await.is_empty(), "the floor already raises every row of the dataset");
}

// 4. A killed job resumes from its checkpoint, and no finding is counted twice.
#[tokio::test(flavor = "multi_thread")]
async fn a_killed_review_resumes_from_its_checkpoint_with_no_duplicate_findings() {
    let w = world().await;
    for n in 0..20 {
        put(&w, n, &if n % 3 == 0 { format!("ssn {SSN}") } else { "plain".to_string() }, Some(w.chemistry), &[]);
    }
    // Three batches of three, then the worker dies.
    let repo = w.state.repository.clone();
    let scope = scaffoldry_server::content_scan::Scope { app_slug: w.slug.clone(), table_id: TABLE.to_string() };
    let plan = scaffoldry_server::content_review::plan(&w.state, &scope).unwrap();
    scaffoldry_server::content_review::begin(&repo, &plan).unwrap();
    let mut after: Option<String> = None;
    for _ in 0..3 {
        let batch = scaffoldry_server::content_review::review_batch(&repo, &plan, after.as_deref(), 3, None).unwrap();
        assert_eq!(batch.read, 3);
        after = batch.last;
    }
    let (s, v) = rest(&w.app, "POST", "/api/v1/admin/content-scans", &w.compliance, Some(json!({ "app_slug": w.slug, "table_id": TABLE, "review": true, "batch_size": 3 }))).await;
    assert_eq!(s, StatusCode::ACCEPTED, "{v}");
    let id: Uuid = v["job_id"].as_str().unwrap().parse().unwrap();
    // A ghost worker claimed the job and saved its checkpoint after three batches. Its lock has run out.
    let cfg = JobConfig { lock_secs: 1, recovery_secs: 1, ..runner(&w) };
    {
        let (repo, cfg) = (repo.clone(), cfg.clone());
        let claimed = std::thread::spawn(move || {
            let mut ghost = repo.dedicated_client().unwrap();
            scaffoldry_server::jobs::claim_next(&mut ghost, "ghost", JOB_KINDS, &cfg).unwrap().is_some()
        })
        .join()
        .unwrap();
        assert!(claimed);
    }
    let cp = json!({ "after": after, "done": 9 });
    w.db.run(move |c| {
        c.execute("UPDATE jobs SET checkpoint = $2, locked_until = NOW() - interval '1 second' WHERE id = $1", &[&id, &cp]).unwrap();
    });
    let r = JobRunner::start(&repo, JOB_KINDS, cfg).unwrap();
    let job = wait_for_job(&w, id);
    r.stop();
    assert_eq!(job.state, "done", "{:?}", job.error);
    let found = reviewed(&w, &w.admin).await;
    assert_eq!(found.len(), 1, "one finding, not one for each attempt: {found:?}");
    assert_eq!((found[0]["match_count"].as_i64(), found[0]["record_count"].as_i64()), (Some(7), Some(7)), "the first nine rows counted once, and the rest after: {}", found[0]);
    let rows: i64 = w.db.run(|c| c.query_one("SELECT count(*) FROM content_findings WHERE source = 'review'", &[]).unwrap().get(0));
    assert_eq!(rows, 1);
}

// 5. A reader who may not read a row never sees its finding or its id.
#[tokio::test(flavor = "multi_thread")]
async fn a_reader_who_may_not_read_a_row_never_sees_its_finding_or_its_id() {
    let w = world().await;
    let chem = put(&w, 1, &format!("ssn {SSN}"), Some(w.chemistry), &[]);
    let phys = put(&w, 2, &format!("ssn {SSN}"), Some(w.physics), &[]);
    let phys_only = put(&w, 3, &format!("ssn {SSN}"), Some(w.physics), &[]);
    // The floor makes every row ask for a registrar at its own unit.
    set_floor(&w, json!(["hr"])).await;
    review(&w).await;

    // An administrator is not held back and sees all three rows.
    let all = reviewed(&w, &w.admin).await;
    assert_eq!(all.len(), 1);
    let mut ids = ids_of(&all[0]);
    ids.sort();
    let mut want = vec![chem.clone(), phys.clone(), phys_only.clone()];
    want.sort();
    assert_eq!(ids, want);

    // A compliance officer who is the registrar of Chemistry reads one row, and is shown that row and no other id.
    let (s, body) = listed(&w, &w.chem_compliance).await;
    assert_eq!(s, StatusCode::OK, "{body}");
    let seen = reviewed(&w, &w.chem_compliance).await;
    assert_eq!(seen.len(), 1, "{seen:?}");
    assert_eq!(ids_of(&seen[0]), vec![chem.clone()]);
    for hidden in [&phys, &phys_only] {
        assert!(!body.to_string().contains(hidden.as_str()), "a Physics id was shown to the Chemistry registrar: {body}");
    }
    assert!(seen[0]["match_count"].is_null() && seen[0]["record_count"].is_null(), "a count would say how many rows were left out: {}", seen[0]);

    // A compliance officer who holds no position reads none of the rows, and is shown no finding and no id.
    let (s, body) = listed(&w, &w.compliance).await;
    assert_eq!(s, StatusCode::OK, "{body}");
    assert!(reviewed(&w, &w.compliance).await.is_empty(), "{body}");
    for id in [&chem, &phys, &phys_only] {
        assert!(!body.to_string().contains(id.as_str()), "an id was shown to a reader who reads no row: {body}");
    }
}

// 6. At scale: 200,000 rows, in batches of a thousand, within the limits the content scan keeps.
#[tokio::test(flavor = "multi_thread")]
async fn a_review_of_two_hundred_thousand_rows_stays_inside_the_scan_budgets() {
    const ROWS: i64 = 200_000;
    let w = world().await;
    let slug = w.slug.clone();
    w.db.run(move |c| {
        c.execute(
            "INSERT INTO dataset_records (id, app_slug, data, ceds_mapping, is_ferpa_sensitive, table_id, created_at) \
             SELECT $1 || '-' || lpad(g::text, 8, '0'), $1, \
                    jsonb_build_object('_table_id', 'students', 'name', 'student ' || g, \
                                       'note', CASE WHEN g % 1000 = 0 THEN 'ssn 123-45-6789 for student ' || g ELSE 'ordinary text for student ' || g END), \
                    '{}'::jsonb, false, 'students', TIMESTAMPTZ '2026-01-01 00:00:00+00' + g * interval '1 millisecond' \
             FROM generate_series(1, $2::bigint) g",
            &[&slug, &ROWS],
        )
        .unwrap();
        c.batch_execute("ANALYZE dataset_records").unwrap();
    });
    let started = std::time::Instant::now();
    let job = review_with(&w, 1_000).await;
    let total = started.elapsed();
    let result = job.result.clone().unwrap();
    let longest = result["longest_batch_ms"].as_i64().unwrap();
    println!("review, {ROWS} rows: total {:.1} s, {} batches, longest single batch {longest} ms", total.as_secs_f64(), result["batches"]);
    assert_eq!(result["records"], ROWS, "every row was read: {result}");
    assert!(result["batches"].as_i64().unwrap() >= ROWS / 1_000, "in batches of a thousand: {result}");
    assert!(longest < 2_000, "no batch held a lock for two seconds: {longest} ms");
    let found = reviewed(&w, &w.admin).await;
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(found[0]["record_count"], ROWS / 1_000, "one row in a thousand names an SSN");
    assert!(ids_of(&found[0]).len() <= 5, "a few ids and no more");
    assert!(!everything_stored(&w).contains(SSN));
}
