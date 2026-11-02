//! Lifecycle, phase 3: retention policies, the nightly purge, and a certificate of destruction.
//!
//! Delete is trash. Destruction happens only here, by policy, after the ledger says so, and leaves a certificate: the
//! counts and the SHA-256 of the sorted ids, which proves what went without keeping it. The disclosure log outlives the
//! record. NIST SP 800-53 SI-12, MP-6, AU-11 and AU-10. Legal holds (phase 4) are not built, so nothing is held yet.

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::Router;
use http_body_util::BodyExt;
use scaffoldry_server::build_app_with_state;
use scaffoldry_server::jobs::{enqueue, get_job, JobConfig, JobRunner, JOB_KINDS};
use scaffoldry_server::service::identity::issue_test_token_and_user;
use scaffoldry_server::state::ServerState;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::time::{Duration, Instant};
use tower::ServiceExt;

const WORKSPACE: &str = "ws-physics-optics";
const CLASSIFICATION: &str = "Internal";
const OWNER: &str = "einstein@physics.state.edu";
const ADMIN: &str = "jordan.lee@state.edu";
const COMPLIANCE: &str = "elena.rodriguez@state.edu";

struct World {
    db: common::TestDb,
    state: std::sync::Arc<ServerState>,
    app: Router,
    owner: String,
    admin: String,
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

fn text(name: &str) -> Value {
    json!({ "name": name, "label": name, "field_type": "Text", "required": false, "ferpa_sensitive": false })
}

fn manifest(slug: &str, fields: Vec<Value>) -> Value {
    json!({
        "slug": slug, "title": "Retention source", "description": "Records that age out", "organization_code": "PHYS", "department": "physics",
        "workspace_id": WORKSPACE, "views": [],
        "tables": [{ "id": "cases", "name": "Cases", "slug": "cases", "primary_field": "title", "fields": fields }],
    })
}

async fn world() -> World {
    let db = common::TestDb::new();
    let state = db.state();
    state.seed_demo().expect("seed");
    let tag = uuid::Uuid::new_v4().to_string()[..8].to_string();
    let app = build_app_with_state(state.clone()).expect("router");
    let w = World { db, state, app, owner: issue_test_token_and_user(OWNER), admin: issue_test_token_and_user(ADMIN), slug: format!("keep-{tag}") };
    let fields = vec![text("title"), json!({ "name": "other", "label": "Other", "field_type": "Link", "required": false, "ferpa_sensitive": false, "link_target_table": "cases" })];
    let (s, b) = rest(&w.app, "POST", &format!("/api/v1/workspaces/{WORKSPACE}/apps"), &w.owner, Some(manifest(&w.slug, fields))).await;
    assert_eq!(s, StatusCode::CREATED, "{b}");
    w
}

fn fast() -> JobConfig {
    JobConfig { workers: 2, poll_ms: 20, backoff_secs: [0, 0, 0], ..JobConfig::default() }
}

fn wait_job(w: &World, id: uuid::Uuid) -> scaffoldry_server::jobs::Job {
    let deadline = Instant::now() + Duration::from_secs(240);
    loop {
        let job = get_job(&w.state.repository, id).unwrap().expect("the job");
        if job.state == "done" || job.state == "failed" {
            return job;
        }
        assert!(Instant::now() < deadline, "the job is stuck in {}", job.state);
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// Runs the nightly job once, now, and waits for it.
fn purge_now(w: &World) -> scaffoldry_server::jobs::Job {
    let runner = JobRunner::start(&w.state.repository, JOB_KINDS, fast()).unwrap();
    let id = enqueue(&w.state.repository, "purge_expired", json!({}), "test@scaffoldry.internal", None).expect("queued");
    let job = wait_job(w, id);
    runner.stop();
    job
}

/// `n` records trashed `days` ago, with ids `<prefix>-0001` and on.
fn trashed(w: &World, prefix: &str, n: i64, days: i64) -> Vec<String> {
    let ids = trashed_ids(prefix, n);
    let (slug, prefix) = (w.slug.clone(), prefix.to_string());
    w.db.run(move |c| {
        c.execute(
            "INSERT INTO dataset_records (id, app_slug, data, ceds_mapping, is_ferpa_sensitive, table_id, created_at, deleted_at, deleted_by) \
             SELECT $3 || '-' || lpad(g::text, 4, '0'), $1, jsonb_build_object('_table_id', 'cases', 'title', 'T' || g), '{}'::jsonb, false, 'cases', \
                    NOW() - make_interval(days => $4::int + 10), NOW() - make_interval(days => $4::int), 'einstein@physics.state.edu' \
             FROM generate_series(1, $2::bigint) AS g",
            &[&slug, &n, &prefix, &(days as i32)],
        )
        .unwrap();
    });
    ids
}

fn present(w: &World, ids: &[String]) -> i64 {
    let ids = ids.to_vec();
    w.db.run(move |c| c.query_one("SELECT count(*) FROM dataset_records WHERE id = ANY($1)", &[&ids]).unwrap().get(0))
}

fn certificates(w: &World) -> Vec<Value> {
    let slug = w.slug.clone();
    w.db.run(move |c| {
        c.query("SELECT counts, ids_sha256, held_skipped FROM purge_certificates WHERE app_slug = $1 ORDER BY run_at", &[&slug])
            .unwrap()
            .iter()
            .map(|r| json!({ "counts": r.get::<_, Value>(0), "ids_sha256": r.get::<_, String>(1).trim().to_string(), "held_skipped": r.get::<_, i32>(2) }))
            .collect()
    })
}

fn sha_of(ids: &[String]) -> String {
    let mut sorted = ids.to_vec();
    sorted.sort();
    Sha256::digest(sorted.join("\n").as_bytes()).iter().map(|b| format!("{b:02x}")).collect()
}

/// The policy that covers the demo workspace's classification, changed through the API.
async fn set_class_policy(w: &World, changes: Value) -> (StatusCode, Value) {
    let (_, list) = rest(&w.app, "GET", "/api/v1/admin/retention/policies", &w.admin, None).await;
    let policy = list["policies"].as_array().and_then(|l| l.iter().find(|p| p["scope_kind"] == "classification" && p["classification"] == CLASSIFICATION)).cloned().expect("a seeded policy");
    let mut body = policy.clone();
    for (k, v) in changes.as_object().unwrap() {
        body[k] = v.clone();
    }
    body["reason"] = json!("The records officer set this");
    rest(&w.app, "PUT", &format!("/api/v1/admin/retention/policies/{}", policy["id"].as_str().unwrap()), &w.admin, Some(body)).await
}

// 1. A record trashed 366 days ago with `keep_trash_days` 365 is destroyed by the job. One trashed 364 days ago is not.
#[tokio::test]
async fn trash_past_its_days_is_destroyed_and_trash_within_them_is_kept() {
    let w = world().await;
    let (s, b) = set_class_policy(&w, json!({ "keep_trash_days": 365 })).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    let old = trashed(&w, "old", 1, 366);
    let young = trashed(&w, "young", 1, 364);
    let job = purge_now(&w);
    assert_eq!(job.state, "done", "{job:?}");
    assert_eq!(present(&w, &old), 0, "366 days in trash is destroyed");
    assert_eq!(present(&w, &young), 1, "364 days in trash is kept");
}

// 2. Destruction removes the record, index rows, link rows, comments, history, and file references, and leaves the
//    disclosure log.
#[tokio::test]
async fn destruction_removes_everything_the_record_had_and_leaves_the_disclosure_log() {
    let w = world().await;
    set_class_policy(&w, json!({ "keep_trash_days": 365 })).await;
    let base = format!("/api/v1/apps/{}", w.slug);
    let (_, keep) = rest(&w.app, "POST", &format!("{base}/records"), &w.owner, Some(json!({ "data": { "_table_id": "cases", "title": "Keep" } }))).await;
    let keep_id = keep["id"].as_str().unwrap().to_string();
    let (s, gone) = rest(&w.app, "POST", &format!("{base}/records"), &w.owner, Some(json!({ "data": { "_table_id": "cases", "title": "Gone", "other": [keep_id] } }))).await;
    assert_eq!(s, StatusCode::CREATED, "{gone}");
    let id = gone["id"].as_str().unwrap().to_string();
    let (s, c) = rest(&w.app, "POST", &format!("{base}/records/{id}/comments"), &w.owner, Some(json!({ "body": "A note" }))).await;
    assert!(s.is_success(), "{s} {c}");
    let (s, b) = rest(&w.app, "DELETE", &format!("{base}/records/{id}"), &w.owner, None).await;
    assert!(s.is_success(), "{s} {b}");
    let (slug, rid) = (w.slug.clone(), id.clone());
    w.db.run(move |c| {
        c.execute("UPDATE dataset_records SET deleted_at = NOW() - interval '366 days' WHERE id = $1", &[&rid]).unwrap();
        c.execute("INSERT INTO files (sha256, size, media_type) VALUES (repeat('a', 64), 1, 'text/plain') ON CONFLICT DO NOTHING", &[]).unwrap();
        c.execute(
            "INSERT INTO file_refs (id, sha256, app_slug, table_id, record_id, field, name, uploaded_by, attached_at) VALUES ($1, repeat('a', 64), $2, 'cases', $3, 'doc', 'a.txt', 'x', NOW())",
            &[&uuid::Uuid::new_v4(), &slug, &rid],
        )
        .unwrap();
        let d = uuid::Uuid::new_v4();
        c.execute("INSERT INTO disclosure_log (id, actor, kind, app_slug, record_count, fields) VALUES ($1, 'x', 'export', $2, 1, '{title}')", &[&d, &slug]).unwrap();
        c.execute("INSERT INTO disclosure_records (disclosure_id, record_id) VALUES ($1, $2)", &[&d, &rid]).unwrap();
    });
    let count = |sql: &'static str| {
        let rid = id.clone();
        w.db.run(move |c| c.query_one(sql, &[&rid]).unwrap().get::<_, i64>(0))
    };
    assert!(count("SELECT count(*) FROM record_history WHERE record_id = $1") > 0, "the record has history before");
    assert!(count("SELECT count(*) FROM trash_links WHERE record_id = $1 OR other_record = $1") > 0, "the trash remembers its link");

    assert_eq!(purge_now(&w).state, "done");
    for (what, sql) in [
        ("the record", "SELECT count(*) FROM dataset_records WHERE id = $1"),
        ("its index rows", "SELECT count(*) FROM record_index WHERE record_id = $1"),
        ("its link rows", "SELECT count(*) FROM record_links WHERE from_record = $1 OR to_record = $1"),
        ("its remembered links", "SELECT count(*) FROM trash_links WHERE record_id = $1 OR other_record = $1"),
        ("its comments", "SELECT count(*) FROM record_comments WHERE record_id = $1"),
        ("its history", "SELECT count(*) FROM record_history WHERE record_id = $1"),
        ("its file references", "SELECT count(*) FROM file_refs WHERE record_id = $1"),
    ] {
        assert_eq!(count(sql), 0, "{what} is destroyed");
    }
    assert_eq!(count("SELECT count(*) FROM disclosure_records WHERE record_id = $1"), 1, "the fact of a disclosure outlives the record");
    assert_eq!(present(&w, &[keep_id]), 1, "the record it linked to is not touched");
}

// 3. The ledger entry is written before the first delete. Make the ledger append fail and nothing is destroyed.
#[tokio::test]
async fn nothing_is_destroyed_when_the_ledger_cannot_record_it() {
    let w = world().await;
    set_class_policy(&w, json!({ "keep_trash_days": 365 })).await;
    let old = trashed(&w, "old", 3, 400);
    w.db.run(|c| {
        c.batch_execute(
            "CREATE FUNCTION refuse_purge_entry() RETURNS trigger AS $$ BEGIN \
               IF NEW.decision_type = 'RecordsPurged' THEN RAISE EXCEPTION 'the ledger is down'; END IF; RETURN NEW; END; $$ LANGUAGE plpgsql; \
             CREATE TRIGGER refuse_purge_entry BEFORE INSERT ON governance_ledger FOR EACH ROW EXECUTE FUNCTION refuse_purge_entry();",
        )
        .unwrap();
    });
    let job = purge_now(&w);
    assert_eq!(job.state, "failed", "{job:?}");
    assert_eq!(present(&w, &old), 3, "no entry, no destruction");
    assert!(certificates(&w).is_empty());
    let entries: i64 = w.db.run(|c| c.query_one("SELECT count(*) FROM governance_ledger WHERE decision_type = 'RecordsPurged'", &[]).unwrap().get(0));
    assert_eq!(entries, 0);
}

// 4. The certificate's hash equals the hash of the sorted ids that the test destroyed, and the ids are not stored.
#[tokio::test]
async fn the_certificate_proves_which_records_went_without_keeping_them() {
    let w = world().await;
    set_class_policy(&w, json!({ "keep_trash_days": 365 })).await;
    let mut ids = trashed(&w, "zeta", 4, 500);
    ids.extend(trashed(&w, "alpha", 3, 500));
    assert_eq!(purge_now(&w).state, "done");
    let certs = certificates(&w);
    assert_eq!(certs.len(), 1, "{certs:?}");
    assert_eq!(certs[0]["counts"]["records"], 7, "{certs:?}");
    assert_eq!(certs[0]["ids_sha256"], sha_of(&ids), "the hash is of the sorted ids, one per line");
    assert_eq!(certs[0]["held_skipped"], 0);
    let pending: i64 = w.db.run(|c| c.query_one("SELECT count(*) FROM purge_pending", &[]).unwrap().get(0));
    assert_eq!(pending, 0, "no list of destroyed ids is kept");

    let (s, list) = rest(&w.app, "GET", "/api/v1/admin/purges", &w.admin, None).await;
    assert_eq!(s, StatusCode::OK, "{list}");
    let row = list["certificates"].as_array().and_then(|l| l.iter().find(|c| c["app_slug"] == w.slug.as_str())).cloned().expect("listed");
    let (s, one) = rest(&w.app, "GET", &format!("/api/v1/admin/purges/{}", row["id"].as_str().unwrap()), &issue_test_token_and_user(COMPLIANCE), None).await;
    assert_eq!(s, StatusCode::OK, "compliance reads a certificate: {one}");
    assert_eq!(one["ids_sha256"], sha_of(&ids));
    assert!(!one.to_string().contains("alpha-0001"), "the certificate names no record: {one}");
    let (s, _) = rest(&w.app, "GET", "/api/v1/admin/purges", &w.owner, None).await;
    assert_eq!(s, StatusCode::FORBIDDEN, "an app owner does not read certificates");
}

// 5. A workspace policy overrides the classification policy.
#[tokio::test]
async fn a_workspace_policy_overrides_the_classification_policy() {
    let w = world().await;
    set_class_policy(&w, json!({ "keep_trash_days": 365 })).await;
    let (s, p) = rest(
        &w.app,
        "POST",
        "/api/v1/admin/retention/policies",
        &w.admin,
        Some(json!({ "name": "Optics lab", "scope_kind": "workspace", "workspace_id": WORKSPACE, "keep_trash_days": 30, "keep_retired_days": 365, "on_expiry": "trash", "basis": "Lab schedule 4.2", "reason": "Shorter for the lab" })),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{p}");
    let month = trashed(&w, "month", 1, 31);
    assert_eq!(purge_now(&w).state, "done");
    assert_eq!(present(&w, &month), 0, "the workspace's 30 days apply, not the classification's 365");
    let changed: i64 = w.db.run(|c| c.query_one("SELECT count(*) FROM governance_ledger WHERE decision_type = 'RetentionPolicyChanged'", &[]).unwrap().get(0));
    assert!(changed >= 2, "each policy change is in the ledger");
}

// 6. `on_expiry` of `purge` with the placeholder basis is refused.
#[tokio::test]
async fn purging_on_expiry_needs_a_real_basis() {
    let w = world().await;
    let (s, b) = set_class_policy(&w, json!({ "on_expiry": "purge", "max_record_age_days": 3650 })).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{b}");
    assert!(b["error"].as_str().unwrap_or("").contains("basis"), "{b}");
    let (s, b) = set_class_policy(&w, json!({ "on_expiry": "purge", "max_record_age_days": 3650, "basis": "  " })).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{b}");
    let (s, b) = set_class_policy(&w, json!({ "on_expiry": "purge", "max_record_age_days": 3650, "basis": "General schedule GRS 5.2, item 020" })).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    // While a placeholder is left, the overview says so.
    let (_, list) = rest(&w.app, "GET", "/api/v1/admin/retention/policies", &w.admin, None).await;
    assert_eq!(list["placeholder"], true, "the other classifications still have the placeholder: {list}");
    // Only a Platform Admin or compliance edits a policy.
    let (s, _) = rest(&w.app, "GET", "/api/v1/admin/retention/policies", &w.owner, None).await;
    assert_eq!(s, StatusCode::FORBIDDEN);
}

// 7. The preview counts equal what a real run then destroys.
#[tokio::test]
async fn the_preview_counts_what_the_run_then_destroys() {
    let w = world().await;
    set_class_policy(&w, json!({ "keep_trash_days": 365 })).await;
    trashed(&w, "a", 5, 200);
    trashed(&w, "b", 2, 50);
    let runner = JobRunner::start(&w.state.repository, JOB_KINDS, fast()).unwrap();
    let (_, list) = rest(&w.app, "GET", "/api/v1/admin/retention/policies", &w.admin, None).await;
    let mut proposed = list["policies"].as_array().unwrap().iter().find(|p| p["classification"] == CLASSIFICATION).cloned().unwrap();
    proposed["keep_trash_days"] = json!(100);
    let (s, queued) = rest(&w.app, "POST", "/api/v1/admin/retention/preview", &w.admin, Some(json!({ "policy": proposed }))).await;
    assert_eq!(s, StatusCode::ACCEPTED, "{queued}");
    let preview = wait_job(&w, queued["job_id"].as_str().unwrap().parse().unwrap());
    runner.stop();
    assert_eq!(preview.state, "done", "{preview:?}");
    let result = preview.result.clone().unwrap_or(Value::Null);
    let row = result["apps"].as_array().and_then(|l| l.iter().find(|a| a["app_slug"] == w.slug.as_str())).cloned().expect("the app is previewed");
    assert_eq!(row["records"], 5, "{row}");
    assert_eq!(row["held"], 0, "{row}");
    assert_eq!(present(&w, &trashed_ids("a", 5)), 5, "a preview destroys nothing");

    set_class_policy(&w, json!({ "keep_trash_days": 100 })).await;
    assert_eq!(purge_now(&w).state, "done");
    let certs = certificates(&w);
    assert_eq!(certs.last().unwrap()["counts"]["records"], row["records"], "{certs:?}");
    assert_eq!(certs.last().unwrap()["counts"]["history"], row["history"], "{certs:?}");
    assert_eq!(certs.last().unwrap()["counts"]["files"], row["files"], "{certs:?}");
}

fn trashed_ids(prefix: &str, n: i64) -> Vec<String> {
    (1..=n).map(|i| format!("{prefix}-{i:04}")).collect()
}

// 8. A job that dies halfway resumes and the final counts and certificate are correct, with no id destroyed twice.
#[tokio::test]
async fn a_run_that_stops_halfway_resumes_and_certifies_every_record_once() {
    let w = world().await;
    set_class_policy(&w, json!({ "keep_trash_days": 365 })).await;
    let ids = trashed(&w, "bulk", 2_500, 400);
    // The run stops after its first batch of 1,000, as a job killed mid-run does.
    let state = w.state.clone();
    tokio::task::spawn_blocking(move || scaffoldry_server::purge::run(&state, Some(1))).await.unwrap().expect("the first part runs");
    assert_eq!(present(&w, &ids), 1_500, "one batch of 1,000 went");
    assert!(certificates(&w).is_empty(), "no certificate until the app is done");

    assert_eq!(purge_now(&w).state, "done");
    assert_eq!(present(&w, &ids), 0);
    let certs = certificates(&w);
    assert_eq!(certs.len(), 1, "one certificate for the app: {certs:?}");
    assert_eq!(certs[0]["counts"]["records"], 2_500, "{certs:?}");
    assert_eq!(certs[0]["ids_sha256"], sha_of(&ids));
    let entries: i64 = w.db.run(|c| c.query_one("SELECT count(*) FROM governance_ledger WHERE decision_type = 'RecordsPurged'", &[]).unwrap().get(0));
    assert_eq!(entries, 1, "the resumed run does not write a second entry");
}

// 9. Every `DELETE FROM` on records, history, links, comments, and file references is inside `purge::destroy`.
#[test]
fn every_destroying_delete_is_in_the_purge_module() {
    // Each exception names where it is and why it is not destruction.
    const ALLOWED: &[(&str, &str, &str)] = &[
        ("repository_links.rs", "record_links", "the link table is an index of the record's own JSON, rewritten in the same transaction as the record"),
        ("repository_links.rs", "trash_links", "a restore takes back the links the trash remembered"),
        ("repository_files.rs", "file_refs WHERE attached_at IS NULL", "an upload no record ever took is not a record's file"),
    ];
    const TABLES: &[&str] = &["dataset_records", "record_history", "record_links", "trash_links", "record_comments", "file_refs"];
    let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut found = Vec::new();
    let mut stack = vec![src];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            let name = path.file_name().unwrap().to_string_lossy().to_string();
            if name == "purge.rs" || !name.ends_with(".rs") {
                continue;
            }
            let body = std::fs::read_to_string(&path).unwrap();
            for line in body.lines() {
                let Some(at) = line.find("DELETE FROM ") else { continue };
                let rest = &line[at + "DELETE FROM ".len()..];
                if let Some(table) = TABLES.iter().find(|t| rest.starts_with(*t)) {
                    if !ALLOWED.iter().any(|(file, what, _)| *file == name && rest.starts_with(what)) {
                        found.push(format!("{name}: {}", line.trim()));
                    }
                    let _ = table;
                }
            }
        }
    }
    assert!(found.is_empty(), "a delete of a record's data outside purge.rs: {found:#?}");
}

// Lifecycle phase 2, test 4, which waited for this phase: a retired field's values are stripped after
// `keep_retired_days`. The hold half of that test waits for phase 4.
#[tokio::test]
async fn retired_values_are_stripped_after_keep_retired_days_and_not_before() {
    let w = world().await;
    set_class_policy(&w, json!({ "keep_retired_days": 90 })).await;
    let base = format!("/api/v1/apps/{}", w.slug);
    let (_, r) = rest(&w.app, "POST", &format!("{base}/records"), &w.owner, Some(json!({ "data": { "_table_id": "cases", "title": "A" } }))).await;
    let id = r["id"].as_str().unwrap().to_string();
    // Two retired fields: one retired 91 days ago, one 89. The values sit in the record as a retired field's do.
    let slug = w.slug.clone();
    let rid = id.clone();
    w.db.run(move |c| {
        c.execute(
            "UPDATE app_manifests SET manifest = jsonb_set(manifest, '{tables,0,retired_fields}', jsonb_build_array( \
                jsonb_build_object('name', 'old_note', 'label', 'Old', 'field_type', 'Text', 'required', false, 'ferpa_sensitive', false, 'retired_at', to_char((NOW() - interval '91 days') AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS\"Z\"')), \
                jsonb_build_object('name', 'new_note', 'label', 'New', 'field_type', 'Text', 'required', false, 'ferpa_sensitive', false, 'retired_at', to_char((NOW() - interval '89 days') AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS\"Z\"')))) \
             WHERE slug = $1",
            &[&slug],
        )
        .unwrap();
        c.execute("UPDATE dataset_records SET data = data || '{\"old_note\": \"o\", \"new_note\": \"n\"}'::jsonb WHERE id = $1", &[&rid]).unwrap();
        c.execute("UPDATE cache_versions SET version = version + 1 WHERE name = 'manifests'", &[]).unwrap();
    });
    assert_eq!(purge_now(&w).state, "done");
    let rid = id.clone();
    let data: Value = w.db.run(move |c| c.query_one("SELECT data FROM dataset_records WHERE id = $1", &[&rid]).unwrap().get(0));
    assert!(data.get("old_note").is_none(), "91 days retired with 90 kept: stripped: {data}");
    assert_eq!(data["new_note"], "n", "89 days retired: kept: {data}");
    assert_eq!(data["title"], "A");
    let stored = w.state.repository.get_app_manifest(&w.slug).unwrap().unwrap();
    let left: Vec<&str> = stored.tables[0].retired_fields.iter().map(|f| f.name.as_str()).collect();
    assert_eq!(left, vec!["new_note"], "a stripped field can no longer be restored, so it leaves the list");
    assert_eq!(certificates(&w).last().unwrap()["counts"]["fields"], 1);
}
