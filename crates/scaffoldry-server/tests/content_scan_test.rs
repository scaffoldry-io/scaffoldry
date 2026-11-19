//! Sensitive content phase 2: stored records are scanned as a job, a compliance officer reviews the findings, and authorized stores
//! say where a category may live. A finding never holds a matched value.

mod common;

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
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
    platform: String,
    compliance: String,
    editor: String,
    viewer: String,
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
        platform: issue_test_token_and_user("jordan.lee@state.edu"),
        compliance,
        editor: issue_test_token_and_user(&editor_e),
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


const SSN: &str = "123-45-6789";
const SSN_B: &str = "234-56-7890";
const SSN_C: &str = "345-67-8901";
const CARD: &str = "4111 1111 1111 1111";

/// Whether text holds the planted card number, as written or with its spaces removed. A bare "4111" is not enough: a
/// random id or a SHA-256 holds those four digits about once in every few thousand values.
fn holds_card(text: &str) -> bool {
    text.contains(CARD) || text.contains(&CARD.replace(' ', ""))
}

fn repo() -> PostgresRepository {
    PostgresRepository::connect(None).expect("database is reachable")
}

/// The runner is given the server's state, as it is in `main`, so a job reads the apps and the settings the server holds.
fn fast(w: &World) -> JobConfig {
    JobConfig { workers: 2, poll_ms: 20, backoff_secs: [0, 0, 0], state: Some(JobState(w.state.clone())), ..JobConfig::default() }
}

/// Puts a record straight in the table. `data` is the record's fields.
fn put(w: &World, table: &str, n: usize, data: Value) -> String {
    let (slug, table) = (w.slug.clone(), table.to_string());
    let id = format!("{slug}-{table}-{n:05}");
    let mut data = data;
    data["_table_id"] = json!(table);
    let rid = id.clone();
    db(move |c| {
        c.execute(
            "INSERT INTO dataset_records (id, app_slug, data, ceds_mapping, is_ferpa_sensitive, table_id, created_at) VALUES ($1, $2, $3, '{}'::jsonb, false, $4, NOW())",
            &[&rid, &slug, &data, &table],
        )
        .unwrap();
    });
    id
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

/// Asks for a scan of one table and runs the jobs until they are done.
async fn scan(w: &World, table: &str) -> Value {
    let (s, v) = send(&w.app, "POST", "/api/v1/admin/content-scans", &w.compliance, Some(json!({ "app_slug": w.slug, "table_id": table, "batch_size": 3 }))).await;
    assert_eq!(s, StatusCode::ACCEPTED, "{v}");
    let id: uuid::Uuid = v["job_id"].as_str().unwrap().parse().unwrap();
    let repo = repo();
    let runner = JobRunner::start(&repo, JOB_KINDS, fast(w)).unwrap();
    let job = wait_for_job(&repo, id);
    runner.stop();
    assert_eq!(job.state, "done", "{:?}", job.error);
    v
}

async fn findings(w: &World, query: &str) -> Vec<Value> {
    let (s, v) = send(&w.app, "GET", &format!("/api/v1/admin/content-findings?app_slug={}&{query}", w.slug), &w.compliance, None).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    v["findings"].as_array().unwrap().clone()
}

fn finding_of<'a>(rows: &'a [Value], field: &str) -> &'a Value {
    rows.iter().find(|f| f["field"] == field).unwrap_or_else(|| panic!("a finding for {field} in {rows:?}"))
}

async fn decide(w: &World, id: &str, decision: &str) -> (StatusCode, Value) {
    send(&w.app, "POST", &format!("/api/v1/admin/content-findings/{id}/decide"), &w.compliance, Some(json!({ "decision": decision, "reason": "Reviewed with the owner." }))).await
}

async fn effective(w: &World, table: &str, field: &str) -> bool {
    let (s, v) = send(&w.app, "GET", &format!("/api/v1/admin/fields?app={}&table={table}", w.slug), &w.platform, None).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    v["fields"].as_array().unwrap().iter().find(|f| f["field"] == field).unwrap()["effective"].as_bool().unwrap()
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

// 1. A table with a planted SSN in a free-text field produces one finding with the right counts and no value.
#[tokio::test]
async fn a_planted_ssn_in_free_text_makes_one_finding_with_counts_and_no_value() {
    let _db = common::TestDb::new();
    let w = world().await;
    let one = put(&w, "notes", 1, json!({ "title": "a", "body": format!("call about {SSN} today") }));
    let two = put(&w, "notes", 2, json!({ "title": "b", "body": format!("{SSN_B} and also {SSN_C}") }));
    for n in 3..=7 {
        put(&w, "notes", n, json!({ "title": "c", "body": "nothing to see", "amount": 123456789 }));
    }
    scan(&w, "notes").await;
    let rows = findings(&w, "status=open").await;
    assert_eq!(rows.len(), 1, "one finding for one field and one detector: {rows:?}");
    let f = &rows[0];
    assert_eq!((f["field"].as_str(), f["detector_id"].as_str(), f["category"].as_str(), f["source"].as_str(), f["status"].as_str()), (Some("body"), Some("us_ssn"), Some("pii"), Some("scan"), Some("open")));
    assert_eq!((f["match_count"].as_i64(), f["record_count"].as_i64()), (Some(3), Some(2)), "three matches in two records: {f}");
    let samples: Vec<&str> = f["sample_record_ids"].as_array().unwrap().iter().map(|v| v.as_str().unwrap()).collect();
    assert!(samples.len() == 2 && samples.contains(&one.as_str()) && samples.contains(&two.as_str()), "{samples:?}");
    let all = format!("{rows:?}{}", everything_stored());
    for value in [SSN, SSN_B, SSN_C] {
        assert!(!all.contains(value), "{value} is nowhere in the findings, the ledger or the jobs");
    }
    // A second scan of the same data changes nothing but the time. The counts are the same and no second row is made.
    scan(&w, "notes").await;
    let again = findings(&w, "").await;
    assert_eq!(again.len(), 1);
    assert_eq!((again[0]["match_count"].as_i64(), again[0]["record_count"].as_i64()), (Some(3), Some(2)), "a repeat scan does not count twice");
}

// 2. A scan interrupted after 3 batches resumes from its cursor and ends with the same findings as an uninterrupted scan.
#[tokio::test]
async fn a_scan_interrupted_after_three_batches_resumes_and_matches_an_uninterrupted_scan() {
    let _db = common::TestDb::new();
    let w = world().await;
    // The same twenty records in two tables. The clean one is scanned straight through, and the other is cut off.
    for n in 0..20 {
        let body = if n % 3 == 0 { format!("note {n} holds {SSN}") } else { format!("note {n}") };
        put(&w, "notes", n, json!({ "title": "t", "body": body }));
        put(&w, "cards", n, json!({ "holder": "h", "card": if n % 4 == 0 { CARD.to_string() } else { "none".to_string() } }));
    }
    scan(&w, "notes").await;
    let straight = findings(&w, "").await;
    assert_eq!(finding_of(&straight, "body")["record_count"], 7);

    // The cards table: three batches of three, then the worker dies.
    let repo = repo();
    let scope = scaffoldry_server::content_scan::Scope { app_slug: w.slug.clone(), table_id: "cards".to_string() };
    let plan = scaffoldry_server::content_scan::plan(&w.state, &scope).unwrap();
    scaffoldry_server::content_scan::begin(&repo, &plan).unwrap();
    let mut after: Option<String> = None;
    for _ in 0..3 {
        let batch = scaffoldry_server::content_scan::scan_batch(&repo, &plan, after.as_deref(), 3, None).unwrap();
        assert_eq!(batch.read, 3);
        after = batch.last;
    }
    let (s, v) = send(&w.app, "POST", "/api/v1/admin/content-scans", &w.compliance, Some(json!({ "app_slug": w.slug, "table_id": "cards", "batch_size": 3 }))).await;
    assert_eq!(s, StatusCode::ACCEPTED, "{v}");
    let id: uuid::Uuid = v["job_id"].as_str().unwrap().parse().unwrap();
    // A ghost worker claimed the job and saved its checkpoint after three batches. The lock has run out.
    let cfg = JobConfig { lock_secs: 1, recovery_secs: 1, ..fast(&w) };
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
    db(move |c| {
        c.execute("UPDATE jobs SET checkpoint = $2, locked_until = NOW() - interval '1 second' WHERE id = $1", &[&id, &cp]).unwrap();
    });
    let runner = JobRunner::start(&repo, JOB_KINDS, cfg).unwrap();
    let job = wait_for_job(&repo, id);
    runner.stop();
    assert_eq!(job.state, "done", "{:?}", job.error);
    let resumed = findings(&w, "").await;
    let card = finding_of(&resumed, "card");
    assert_eq!((card["match_count"].as_i64(), card["record_count"].as_i64()), (Some(5), Some(5)), "the first nine records counted once, and the rest after: {card}");
}

// 3. `Confirm` raises the label with the category. A viewer who may not see sensitive fields is masked at once, before any
// recompute job runs.
#[tokio::test]
async fn confirming_a_finding_labels_the_field_with_its_category_and_masks_a_viewer_at_once() {
    let _db = common::TestDb::new();
    let w = world().await;
    put(&w, "notes", 1, json!({ "title": "a", "body": format!("ssn {SSN}") }));
    scan(&w, "notes").await;
    let id = finding_of(&findings(&w, "").await, "body")["id"].as_str().unwrap().to_string();
    let list = format!("/api/v1/apps/{}/tables/notes/records", w.slug);
    let (s, before) = send(&w.app, "GET", &list, &w.viewer, None).await;
    assert_eq!(s, StatusCode::OK, "{before}");
    assert!(before.to_string().contains("ssn"), "a viewer reads the body at first: {before}");
    assert!(!effective(&w, "notes", "body").await);

    let (s, v) = decide(&w, &id, "confirm").await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert!(effective(&w, "notes", "body").await, "the label protects the field");
    let (s, after) = send(&w.app, "GET", &list, &w.viewer, None).await;
    assert_eq!(s, StatusCode::OK, "{after}");
    assert!(!after.to_string().contains("ssn"), "a viewer no longer reads the body: {after}");
    let label: (Vec<String>, String) = db(|c| {
        let r = c.query_one("SELECT categories, note FROM data_labels WHERE field = 'body'", &[]).unwrap();
        (r.get(0), r.get(1))
    });
    assert_eq!(label.0, vec!["pii".to_string()]);
    assert!(label.1.contains("Reviewed with the owner."), "{label:?}");
    let ledger: i64 = db(|c| c.query_one("SELECT count(*) FROM governance_ledger WHERE decision_type = 'ContentFindingDecided'", &[]).unwrap().get(0));
    assert_eq!(ledger, 1);
    // A decision is made once.
    assert_eq!(decide(&w, &id, "confirm").await.0, StatusCode::CONFLICT);
    assert!(!everything_stored().contains(SSN));
}

// 4. `Dismiss` keeps the field unlabeled and the finding does not return on the next scan.
#[tokio::test]
async fn a_dismissed_finding_stays_dismissed_through_the_next_scan() {
    let _db = common::TestDb::new();
    let w = world().await;
    put(&w, "notes", 1, json!({ "title": "a", "body": format!("ssn {SSN}") }));
    scan(&w, "notes").await;
    let id = finding_of(&findings(&w, "").await, "body")["id"].as_str().unwrap().to_string();
    let (s, v) = decide(&w, &id, "dismiss").await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert!(!effective(&w, "notes", "body").await, "a dismissal labels nothing");
    put(&w, "notes", 2, json!({ "title": "b", "body": format!("another {SSN_B}") }));
    scan(&w, "notes").await;
    let rows = findings(&w, "").await;
    assert_eq!(rows.len(), 1, "no second finding");
    assert_eq!(rows[0]["status"], "dismissed", "it did not come back: {rows:?}");
    assert!(findings(&w, "status=open").await.is_empty());
}

// A dismissed finding stays dismissed until its detector changes. Then it is open again, and every table is scanned again.
#[tokio::test]
async fn changing_a_detector_reopens_what_was_dismissed_and_scans_again() {
    let _db = common::TestDb::new();
    let w = world().await;
    put(&w, "notes", 1, json!({ "title": "a", "body": format!("ssn {SSN}") }));
    scan(&w, "notes").await;
    let id = finding_of(&findings(&w, "").await, "body")["id"].as_str().unwrap().to_string();
    decide(&w, &id, "dismiss").await;
    let (_, v) = send(&w.app, "GET", "/api/v1/admin/sensitivity-settings", &w.compliance, None).await;
    let mut detectors = v["detectors"].clone();
    // A category and a detector other than the one that matched change nothing for it.
    let other = detectors.as_array().unwrap().iter().position(|d| d["id"] == "payment_card").unwrap();
    detectors[other]["action"] = json!("warn");
    assert_eq!(send(&w.app, "PUT", "/api/v1/settings/sensitivity.detectors", &w.platform, Some(detectors.clone())).await.0, StatusCode::OK);
    assert_eq!(findings(&w, "status=dismissed").await.len(), 1, "the SSN detector did not change");
    let changing = detectors.as_array().unwrap().iter().position(|d| d["id"] == "us_ssn").unwrap();
    detectors[changing]["action"] = json!("warn");
    assert_eq!(send(&w.app, "PUT", "/api/v1/settings/sensitivity.detectors", &w.platform, Some(detectors)).await.0, StatusCode::OK);
    assert_eq!(findings(&w, "status=open").await.len(), 1, "a changed detector sets its dismissals aside");
    let queued: i64 = db(|c| c.query_one("SELECT count(*) FROM jobs WHERE kind = 'scan_records'", &[]).unwrap().get(0));
    assert!(queued >= 2, "every table is scanned again: {queued}");
}

// 5. A field designated as an authorized store for `pci` is labeled at once. A card number in it raises no finding and counts as
// `expected`. The same number in another table raises a finding.
#[tokio::test]
async fn an_authorized_store_is_labeled_at_once_and_its_matches_are_expected_not_findings() {
    let _db = common::TestDb::new();
    let w = world().await;
    put(&w, "cards", 1, json!({ "holder": "ada", "card": CARD }));
    put(&w, "notes", 1, json!({ "title": "a", "body": format!("paid with {CARD}") }));
    let (s, v) = send(&w.app, "PUT", "/api/v1/admin/authorized-stores", &w.compliance, Some(json!({ "category": "pci", "scope": "field", "app_slug": w.slug, "table_id": "cards", "field": "card", "reason": "The payments table holds cards." }))).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert!(effective(&w, "cards", "card").await, "the field is labeled with the category at once, with no scan");
    scan(&w, "cards").await;
    scan(&w, "notes").await;
    let rows = findings(&w, "").await;
    assert_eq!(rows.len(), 1, "{rows:?}");
    assert_eq!((rows[0]["table_id"].as_str(), rows[0]["field"].as_str(), rows[0]["detector_id"].as_str()), (Some("notes"), Some("body"), Some("payment_card")));
    let (s, o) = send(&w.app, "GET", "/api/v1/admin/overview", &w.compliance, None).await;
    assert_eq!(s, StatusCode::OK, "{o}");
    assert_eq!(o["sensitive_content"]["expected"], 1, "the card in the store is expected: {o}");
    assert_eq!(o["sensitive_content"]["open"], 1);
    let (s, list) = send(&w.app, "GET", "/api/v1/admin/authorized-stores", &w.compliance, None).await;
    assert_eq!(s, StatusCode::OK, "{list}");
    assert_eq!(list["stores"][0]["category"], "pci");
    assert!(!holds_card(&everything_stored()), "no card number was stored anywhere");
}

// 6. Removing a designation leaves the label in place.
#[tokio::test]
async fn removing_a_designation_leaves_the_label() {
    let _db = common::TestDb::new();
    let w = world().await;
    let body = json!({ "category": "pci", "scope": "field", "app_slug": w.slug, "table_id": "cards", "field": "card", "reason": "Payments." });
    assert_eq!(send(&w.app, "PUT", "/api/v1/admin/authorized-stores", &w.compliance, Some(body.clone())).await.0, StatusCode::OK);
    let (s, v) = send(&w.app, "DELETE", "/api/v1/admin/authorized-stores", &w.compliance, Some(body.clone())).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let (_, list) = send(&w.app, "GET", "/api/v1/admin/authorized-stores", &w.compliance, None).await;
    assert!(list["stores"].as_array().unwrap().is_empty(), "{list}");
    assert!(effective(&w, "cards", "card").await, "the label stays until a person removes it");
    // The scope is checked: a field that is not in the app is refused.
    let mut bad = body;
    bad["field"] = json!("no_such_field");
    assert_eq!(send(&w.app, "PUT", "/api/v1/admin/authorized-stores", &w.compliance, Some(bad)).await.0, StatusCode::BAD_REQUEST);
}

// 7. The detector stats count confirmed and dismissed decisions correctly.
#[tokio::test]
async fn the_detector_stats_count_confirmed_and_dismissed() {
    let _db = common::TestDb::new();
    let w = world().await;
    put(&w, "notes", 1, json!({ "title": format!("id {SSN}"), "body": format!("ssn {SSN_B}") }));
    put(&w, "cards", 1, json!({ "holder": SSN_C, "card": "none" }));
    scan(&w, "notes").await;
    scan(&w, "cards").await;
    let rows = findings(&w, "").await;
    assert_eq!(rows.len(), 3, "{rows:?}");
    decide(&w, finding_of(&rows, "title")["id"].as_str().unwrap(), "confirm").await;
    decide(&w, finding_of(&rows, "body")["id"].as_str().unwrap(), "dismiss").await;
    let (s, v) = send(&w.app, "GET", "/api/v1/admin/detector-stats", &w.compliance, None).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let stat = v["detectors"].as_array().unwrap().iter().find(|d| d["detector_id"] == "us_ssn").unwrap();
    assert_eq!((stat["findings"].as_i64(), stat["confirmed"].as_i64(), stat["dismissed"].as_i64()), (Some(3), Some(1), Some(1)), "{stat}");
    assert_eq!(stat["dismissed_share"], 0.5, "a half of the decided findings were dismissed: {stat}");
}

// The categories, the detectors and the presets, read by a compliance officer.
#[tokio::test]
async fn a_compliance_officer_reads_the_categories_the_detectors_and_the_presets() {
    let _db = common::TestDb::new();
    let w = world().await;
    let (s, v) = send(&w.app, "GET", "/api/v1/admin/sensitivity-settings", &w.compliance, None).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert!(v["categories"].as_array().unwrap().iter().any(|c| c["id"] == "pci"), "{v}");
    assert!(v["detectors"].as_array().unwrap().iter().any(|d| d["id"] == "us_ssn"), "{v}");
    let presets = v["presets"].as_array().unwrap();
    assert!(presets.iter().any(|p| p["id"] == "pci" && p["switched_on"] == true), "pci is on in a new installation: {v}");
    assert!(presets.iter().any(|p| p["id"] == "phi" && p["switched_on"] == false), "phi is off: {v}");
}

// A reason sent with a setting is in the ledger entry, so a change of a detector's action carries why.
#[tokio::test]
async fn a_reason_sent_with_a_setting_is_in_its_ledger_entry() {
    let _db = common::TestDb::new();
    let w = world().await;
    let (_, v) = send(&w.app, "GET", "/api/v1/admin/sensitivity-settings", &w.compliance, None).await;
    let mut detectors = v["detectors"].clone();
    detectors[0]["action"] = json!("warn");
    let (s, v) = send(&w.app, "PUT", "/api/v1/settings/sensitivity.detectors?reason=Mostly%20dismissed", &w.platform, Some(detectors)).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let rationale: String = db(|c| c.query_one("SELECT rationale FROM governance_ledger WHERE decision_type = 'PolicyRevision' ORDER BY sequence DESC LIMIT 1", &[]).unwrap().get(0));
    assert!(rationale.contains("Reason: Mostly dismissed"), "{rationale}");
}

// 8. A faculty member who is not `compliance` gets 403 on every route.
#[tokio::test]
async fn only_a_platform_admin_or_a_compliance_officer_reaches_these_routes() {
    let _db = common::TestDb::new();
    let w = world().await;
    let faculty = issue_test_token_and_user("faculty.curie@state.edu");
    let zero = "00000000-0000-0000-0000-0000000000aa";
    let store = json!({ "category": "pci", "scope": "field", "app_slug": w.slug, "table_id": "cards", "field": "card", "reason": "x" });
    for who in [&faculty, &w.editor, &w.viewer] {
        for (method, uri, body) in [
            ("POST", "/api/v1/admin/content-scans".to_string(), Some(json!({ "all": true }))),
            ("GET", "/api/v1/admin/content-findings".to_string(), None),
            ("POST", format!("/api/v1/admin/content-findings/{zero}/decide"), Some(json!({ "decision": "dismiss", "reason": "x" }))),
            ("GET", "/api/v1/admin/authorized-stores".to_string(), None),
            ("PUT", "/api/v1/admin/authorized-stores".to_string(), Some(store.clone())),
            ("DELETE", "/api/v1/admin/authorized-stores".to_string(), Some(store.clone())),
            ("GET", "/api/v1/admin/detector-stats".to_string(), None),
            ("GET", "/api/v1/admin/sensitivity-settings".to_string(), None),
        ] {
            let (s, v) = send(&w.app, method, &uri, who, body).await;
            assert_eq!(s, StatusCode::FORBIDDEN, "{method} {uri}: {v}");
        }
    }
}

// 10. A grep for the planted value across the findings table and the ledger prints nothing. Test 1 and test 3 check it as they go. This one
// plants values in every kind of place a scan reads, and checks the whole database.
#[tokio::test]
async fn no_planted_value_is_anywhere_after_a_scan_a_confirm_and_a_designation() {
    let _db = common::TestDb::new();
    let w = world().await;
    put(&w, "notes", 1, json!({ "title": format!("t {SSN}"), "body": format!("{CARD} and {SSN_B}"), "amount": 4111111111111111u64 }));
    put(&w, "cards", 1, json!({ "holder": "h", "card": CARD }));
    send(&w.app, "PUT", "/api/v1/admin/authorized-stores", &w.compliance, Some(json!({ "category": "pci", "scope": "table", "app_slug": w.slug, "table_id": "cards", "reason": "Payments." }))).await;
    scan(&w, "notes").await;
    scan(&w, "cards").await;
    let rows = findings(&w, "").await;
    decide(&w, rows[0]["id"].as_str().unwrap(), "confirm").await;
    let seen = format!("{rows:?}{}", everything_stored());
    for value in [SSN, SSN_B, "4111 1111 1111 1111", "4111111111111111"] {
        assert!(!seen.contains(value), "{value} was stored");
    }
}

// Phase 4. 1. After `Confirm`, an old version that held the value shows `changed: true` and no value to a viewer who may not see the field.
#[tokio::test]
async fn a_confirmed_label_masks_an_old_version_in_history() {
    let _db = common::TestDb::new();
    let w = world().await;
    let (s, made) = send(&w.app, "POST", &format!("/api/v1/apps/{}/records", w.slug), &w.editor, Some(json!({ "data": { "_table_id": "notes", "title": "a", "body": format!("ssn {SSN}") } }))).await;
    assert_eq!(s, StatusCode::CREATED, "{made}");
    let id = made["id"].as_str().unwrap().to_string();
    let (s, v) = send(&w.app, "PATCH", &format!("/api/v1/apps/{}/records/{id}", w.slug), &w.editor, Some(json!({ "data": { "body": "removed" }, "version": 1 }))).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    scan(&w, "notes").await;
    let hist = |token: String| {
        let (app, slug, id) = (w.app.clone(), w.slug.clone(), id.clone());
        async move { send(&app, "GET", &format!("/api/v1/apps/{slug}/records/{id}/history"), &token, None).await }
    };
    // Before a person decides, nothing is hidden: the field is not labeled.
    let (s, before) = hist(w.viewer.clone()).await;
    assert_eq!(s, StatusCode::OK, "{before}");
    assert!(before.to_string().contains(SSN), "before Confirm the old value is in history: {before}");
    let finding = finding_of(&findings(&w, "status=open").await, "body")["id"].as_str().unwrap().to_string();
    assert_eq!(decide(&w, &finding, "confirm").await.0, StatusCode::OK);
    let (s, after) = hist(w.viewer.clone()).await;
    assert_eq!(s, StatusCode::OK, "{after}");
    assert!(!after.to_string().contains(SSN), "after Confirm the old value is not shown: {after}");
    assert!(after.to_string().contains("\"changed\":true"), "it says the field changed: {after}");
}

// 2. An export that includes a confirmed field writes one disclosure and one ledger entry. Nothing in the export code knew of the category.
#[tokio::test]
async fn an_export_of_a_confirmed_field_writes_one_disclosure_and_one_ledger_entry() {
    let _db = common::TestDb::new();
    let w = world().await;
    put(&w, "notes", 1, json!({ "title": "a", "body": format!("ssn {SSN}") }));
    let dir = std::env::temp_dir().join(format!("scaffoldry-files-{}", Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    std::env::set_var("SCAFFOLDRY_FILES_DIR", &dir);
    // The export route reads the stored app definition.
    let stored = w.state.engine.read().unwrap().list_manifests().into_iter().find(|m| m.slug == w.slug).unwrap();
    w.state.repository.upsert_app_manifest(&stored).unwrap();
    scan(&w, "notes").await;
    let slug = w.slug.clone();
    let count = move |sql: &'static str| -> i64 {
        let slug = slug.clone();
        db(move |c| c.query_one(sql, &[&slug]).unwrap().get(0))
    };
    let disclosures = |c: &dyn Fn(&'static str) -> i64| c("SELECT count(*) FROM disclosure_log WHERE app_slug = $1");
    let ledger = |c: &dyn Fn(&'static str) -> i64| c("SELECT count(*) FROM governance_ledger WHERE app_slug = $1 AND decision_type = 'DataExported'");
    let export = |token: String| {
        let (app, slug, state) = (w.app.clone(), w.slug.clone(), w.state.clone());
        async move {
            let (s, v) = send(&app, "POST", &format!("/api/v1/apps/{slug}/exports"), &token, Some(json!({ "table_id": "notes", "format": "csv" }))).await;
            assert_eq!(s, StatusCode::OK, "{v}");
            let job: uuid::Uuid = v["job_id"].as_str().unwrap().parse().unwrap();
            let runner = JobRunner::start(&state.repository, JOB_KINDS, JobConfig { poll_ms: 10, ..JobConfig::default() }).unwrap();
            for _ in 0..500 {
                if get_job(&state.repository, job).unwrap().is_some_and(|j| j.state == "done") {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
            runner.stop();
            let done = get_job(&state.repository, job).unwrap().unwrap();
            assert_eq!(done.state, "done", "{:?}", done.error);
            let req = Request::get(format!("/api/v1/apps/{slug}/exports/{}/download", v["export_id"].as_str().unwrap())).header(header::AUTHORIZATION, format!("Bearer {token}")).body(Body::empty()).unwrap();
            let resp = app.clone().oneshot(req).await.unwrap();
            assert_eq!(resp.status(), StatusCode::OK);
            String::from_utf8(resp.into_body().collect().await.unwrap().to_bytes().to_vec()).unwrap()
        }
    };
    export(w.editor.clone()).await;
    assert_eq!((disclosures(&count), ledger(&count)), (0, 0), "an unlabeled field is not a disclosure");
    let finding = finding_of(&findings(&w, "status=open").await, "body")["id"].as_str().unwrap().to_string();
    assert_eq!(decide(&w, &finding, "confirm").await.0, StatusCode::OK);
    export(w.editor.clone()).await;
    assert_eq!((disclosures(&count), ledger(&count)), (1, 1), "a confirmed field is one disclosure and one ledger entry");
}

// 3. The old name is gone from the code.
#[test]
fn the_old_function_name_is_gone() {
    fn walk(dir: &std::path::Path, hits: &mut Vec<String>) {
        for e in std::fs::read_dir(dir).unwrap().flatten() {
            let p = e.path();
            if p.is_dir() {
                if p.file_name().is_some_and(|n| n == "target") {
                    continue;
                }
                walk(&p, hits);
            } else if p.extension().is_some_and(|x| x == "rs") && std::fs::read_to_string(&p).is_ok_and(|t| t.contains(concat!("effective_", "ferpa_sensitive"))) {
                hits.push(p.display().to_string());
            }
        }
    }
    let mut hits = Vec::new();
    walk(&std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(".."), &mut hits);
    assert!(hits.is_empty(), "the old name is still in: {hits:?}");
}

// 4. The Overview counts match the findings table. 5. Deleting an app removes its findings.
#[tokio::test]
async fn the_overview_counts_match_the_findings_and_a_deleted_app_takes_them_with_it() {
    let _db = common::TestDb::new();
    let w = world().await;
    let (s, o) = send(&w.app, "GET", "/api/v1/admin/overview", &w.compliance, None).await;
    assert_eq!(s, StatusCode::OK, "{o}");
    assert!(o["sensitive_content"]["oldest_open"].is_null() && o["sensitive_content"]["last_scan"].is_null(), "{o}");
    let never_before = o["sensitive_content"]["tables_never_scanned"].as_i64().unwrap();
    put(&w, "notes", 1, json!({ "title": "a", "body": format!("ssn {SSN}") }));
    put(&w, "notes", 2, json!({ "title": "b", "body": format!("card {CARD}") }));
    scan(&w, "notes").await;
    let rows = findings(&w, "status=open").await;
    let (_, o) = send(&w.app, "GET", "/api/v1/admin/overview", &w.compliance, None).await;
    let c = &o["sensitive_content"];
    assert_eq!(c["open"].as_i64().unwrap(), rows.len() as i64, "{o}");
    assert!(c["oldest_open"].is_string() && c["last_scan"].is_string(), "{o}");
    assert_eq!(c["tables_never_scanned"].as_i64().unwrap(), never_before - 1, "one table was scanned: {o}");
    // An app is deleted the way a trial's app is, with what it held. The workspace named is not one that exists.
    let repo = scaffoldry_server::repository::PostgresRepository::connect(None).unwrap();
    let trial = scaffoldry_server::repository_trials::TrialRow { id: Uuid::new_v4(), proposal_id: Uuid::new_v4(), owner: "nobody@state.edu".into(), app_slug: w.slug.clone(), workspace_id: "ws-none".into(), records: 0, created_at: String::new(), last_opened_at: String::new() };
    repo.destroy_trial(&trial).unwrap();
    let slug = w.slug.clone();
    let left: i64 = db(move |c| c.query_one("SELECT count(*) FROM content_findings WHERE app_slug = $1", &[&slug]).unwrap().get(0));
    assert_eq!(left, 0, "the findings went with the app");
}
