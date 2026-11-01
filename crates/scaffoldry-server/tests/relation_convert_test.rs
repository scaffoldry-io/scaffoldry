//! Record links, phase 5: converting a value join to a link.
//!
//! An app that joins one table to another with a `TableRelationship` matches the text in one field to the text in another.
//! Converting it turns the source field into a `Link` field and links each record to the one record its text matched.
//! A record whose text matched no record, or several, is left blank and its old text is kept in history. The conversion
//! is a proposal, flagged `data_loss` when any record would be left blank. NIST SP 800-53 CM-3 and SI-12.
//!
//! The fixture: 1,000 applications and 60 programs. 970 applications name a program code that exactly one program has,
//! 20 name a code no program has, and 10 name a code that two programs share.

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::Router;
use http_body_util::BodyExt;
use scaffoldry_server::build_app_with_state;
use scaffoldry_server::jobs::{get_job, JobConfig, JobRunner, JOB_KINDS};
use scaffoldry_server::service::identity::issue_test_token_and_user;
use scaffoldry_server::state::{CollaboratorRecord, ServerState};
use serde_json::{json, Value};
use std::sync::OnceLock;
use std::time::{Duration, Instant};
use tower::ServiceExt;

const WORKSPACE: &str = "ws-physics-optics";
const OWNER: &str = "einstein@physics.state.edu";
const EDITOR: &str = "relation.editor@physics.state.edu";
const ADMIN: &str = "jordan.lee@state.edu";
const RELATION: &str = "app-program";

fn files_dir() {
    static DIR: OnceLock<std::path::PathBuf> = OnceLock::new();
    DIR.get_or_init(|| {
        let dir = std::env::temp_dir().join(format!("scaffoldry-relations-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        std::env::set_var("SCAFFOLDRY_FILES_DIR", &dir);
        dir
    });
}

struct World {
    db: common::TestDb,
    state: std::sync::Arc<ServerState>,
    app: Router,
    owner: String,
    editor: String,
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

async fn tool_as(w: &World, token: &str, name: &str, args: Value) -> Result<Value, String> {
    let payload = json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": { "name": name, "arguments": args } });
    let (_, v) = rest(&w.app, "POST", "/api/mcp", token, Some(payload)).await;
    if let Some(err) = v.get("error") {
        return Err(err["message"].as_str().unwrap_or("").to_string());
    }
    if v["result"]["isError"] == true {
        return Err(v["result"]["content"][0]["text"].as_str().unwrap_or("").to_string());
    }
    Ok(v["result"]["structuredContent"].clone())
}

fn text(name: &str) -> Value {
    json!({ "name": name, "label": name, "field_type": "Text", "required": false, "ferpa_sensitive": false })
}

fn manifest(slug: &str) -> Value {
    json!({
        "slug": slug, "title": "Admissions", "description": "Applications and their programs", "organization_code": "PHYS", "department": "physics",
        "workspace_id": WORKSPACE, "views": [],
        "tables": [
            { "id": "programs", "name": "Programs", "slug": "programs", "fields": [text("code"), text("name")] },
            { "id": "applications", "name": "Applications", "slug": "applications", "fields": [text("title"), text("program_code")] },
        ],
        "relationships": [{
            "id": RELATION, "name": "Application program", "source_table_id": "applications", "target_table_id": "programs",
            "source_field": "program_code", "target_field": "code", "relationship_type": "many_to_one", "display_field": "name"
        }],
    })
}

async fn world() -> World {
    files_dir();
    let db = common::TestDb::new();
    let state = db.state();
    state.seed_demo().expect("seed");
    let record = CollaboratorRecord {
        id: uuid::Uuid::new_v4().to_string(),
        workspace_id: WORKSPACE.to_string(),
        eppn: EDITOR.to_string(),
        name: EDITOR.to_string(),
        role: "editor".to_string(),
        scoped_affiliation: "faculty".to_string(),
        department: "physics".to_string(),
        added_at: String::new(),
        app_slug: String::new(),
    };
    state.repository.upsert_collaborator(&record).expect("a member");
    state.collaborators.write().unwrap().entry(WORKSPACE.to_string()).or_default().push(record);
    let tag = uuid::Uuid::new_v4().to_string()[..8].to_string();
    let app = build_app_with_state(state.clone()).expect("router");
    let w = World {
        db,
        state,
        app,
        owner: issue_test_token_and_user(OWNER),
        editor: issue_test_token_and_user(EDITOR),
        admin: issue_test_token_and_user(ADMIN),
        slug: format!("adm-{tag}"),
    };
    let (s, b) = rest(&w.app, "POST", &format!("/api/v1/workspaces/{WORKSPACE}/apps"), &w.owner, Some(manifest(&w.slug))).await;
    assert_eq!(s, StatusCode::CREATED, "{b}");
    fill(&w);
    w
}

/// 60 programs: 50 with a code of their own and 5 codes held by two programs each. 1,000 applications: 970 that name a code
/// one program has, 20 that name a code nobody has, and 10 that name a code two programs share.
fn fill(w: &World) {
    let slug = w.slug.clone();
    w.db.run(move |c| {
        c.execute(
            "INSERT INTO dataset_records (id, app_slug, data, ceds_mapping, is_ferpa_sensitive, table_id) \
             SELECT 'prog-' || $1 || '-' || lpad(g::text, 3, '0'), $1, \
                    jsonb_build_object('_table_id', 'programs', 'code', CASE WHEN g <= 50 THEN 'U' || lpad(g::text, 2, '0') ELSE 'D' || ((g - 51) % 5) END, 'name', 'Program ' || g), \
                    '{}'::jsonb, false, 'programs' \
             FROM generate_series(1, 60) g",
            &[&slug],
        )
        .unwrap();
        c.execute(
            "INSERT INTO dataset_records (id, app_slug, data, ceds_mapping, is_ferpa_sensitive, table_id, created_at) \
             SELECT 'appl-' || $1 || '-' || lpad(g::text, 4, '0'), $1, \
                    jsonb_build_object('_table_id', 'applications', 'title', 'Application ' || g, \
                      'program_code', CASE WHEN g <= 970 THEN 'U' || lpad(((g % 50) + 1)::text, 2, '0') WHEN g <= 990 THEN 'X' || g ELSE 'D' || (g % 5) END), \
                    '{}'::jsonb, false, 'applications', TIMESTAMPTZ '2026-01-01 00:00:00+00' + g * interval '1 millisecond' \
             FROM generate_series(1, 1000) g",
            &[&slug],
        )
        .unwrap();
    });
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

fn job_of(w: &World, kind: &str) -> uuid::Uuid {
    let (slug, kind) = (w.slug.clone(), kind.to_string());
    let deadline = Instant::now() + Duration::from_secs(120);
    loop {
        let (s, k) = (slug.clone(), kind.clone());
        let found: Option<uuid::Uuid> = w.db.run(move |c| c.query_opt("SELECT id FROM jobs WHERE kind = $1 AND payload->>'app_slug' = $2 ORDER BY created_at DESC LIMIT 1", &[&k, &s]).unwrap().map(|r| r.get(0)));
        if let Some(id) = found {
            return id;
        }
        assert!(Instant::now() < deadline, "no {kind} job was queued");
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// Runs the preview and returns its result.
async fn preview(w: &World) -> Value {
    let (s, b) = rest(&w.app, "POST", &format!("/api/v1/apps/{}/relations/{RELATION}/convert-preview", w.slug), &w.owner, Some(json!({}))).await;
    assert_eq!(s, StatusCode::ACCEPTED, "{b}");
    let job = wait_job(w, b["job_id"].as_str().expect("a job id").parse().unwrap());
    assert_eq!(job.state, "done", "{:?}", job.error);
    job.result.expect("a result")
}

fn check<'a>(p: &'a Value, name: &str) -> &'a Value {
    p["checks"].as_array().and_then(|l| l.iter().find(|c| c["name"] == name)).unwrap_or_else(|| panic!("no check {name}: {p}"))
}

fn status_of(w: &World, id: &str) -> String {
    w.state.repository.get_proposal(id.parse().unwrap()).unwrap().expect("the proposal").status
}

/// The programs each application's old join returned, for the applications whose text matched exactly one program.
fn old_join(w: &World) -> Vec<(String, String)> {
    let slug = w.slug.clone();
    w.db.run(move |c| {
        c.query(
            "SELECT a.id, min(p.data->>'name') FROM dataset_records a \
             JOIN dataset_records p ON p.app_slug = a.app_slug AND p.table_id = 'programs' AND p.data->>'code' = a.data->>'program_code' \
             WHERE a.app_slug = $1 AND a.table_id = 'applications' GROUP BY a.id HAVING count(*) = 1 ORDER BY a.id",
            &[&slug],
        )
        .unwrap()
        .iter()
        .map(|r| (r.get(0), r.get(1)))
        .collect()
    })
}

/// Proposes the conversion, has a second person approve it, and waits for the conversion job. Returns the proposal.
async fn convert(w: &World) -> Value {
    let (s, p) = rest(&w.app, "POST", &format!("/api/v1/apps/{}/relations/{RELATION}/convert", w.slug), &w.owner, Some(json!({}))).await;
    assert_eq!(s, StatusCode::CREATED, "{p}");
    let id = p["proposal_id"].as_str().expect("a proposal").to_string();
    // The proposer cannot approve a flagged change alone.
    let refused = tool_as(w, &w.owner, "decide_proposal", json!({ "proposal_id": id, "decision": "approve", "rationale": "Mine", "digest": p["digest"] })).await;
    assert!(refused.is_err(), "a flagged change needs a second person");
    // A change that loses data waits for a snapshot, which the snapshot job takes.
    tool_as(w, &w.admin, "decide_proposal", json!({ "proposal_id": id, "decision": "approve", "rationale": "Reviewed the unmatched list", "digest": p["digest"] }))
        .await
        .expect("approved");
    let deadline = Instant::now() + Duration::from_secs(120);
    while status_of(w, &id) != "Approved" {
        assert!(Instant::now() < deadline, "the proposal stayed {}", status_of(w, &id));
        std::thread::sleep(Duration::from_millis(50));
    }
    let convert = wait_job(w, job_of(w, "convert_relation_to_link"));
    assert_eq!(convert.state, "done", "{:?}", convert.error);
    p
}

// 1. The preview reports exactly the counts of the fixture, with 50 examples at most of each kind that does not match.
#[tokio::test]
async fn the_preview_counts_matched_unmatched_and_ambiguous_records() {
    let w = world().await;
    let runner = JobRunner::start(&w.state.repository, JOB_KINDS, fast()).unwrap();
    let result = preview(&w).await;
    assert_eq!((result["scanned"].clone(), result["matched"].clone(), result["unmatched"].clone(), result["ambiguous"].clone()), (json!(1000), json!(970), json!(20), json!(10)), "{result}");
    let unmatched = result["examples"]["unmatched"].as_array().expect("examples of unmatched records");
    let ambiguous = result["examples"]["ambiguous"].as_array().expect("examples of ambiguous records");
    assert_eq!((unmatched.len(), ambiguous.len()), (20, 10), "every one, since there are fewer than 50");
    assert!(unmatched.iter().all(|e| e["id"].as_str().is_some_and(|i| i.starts_with("appl-")) && e["value"].as_str().is_some_and(|v| v.starts_with('X'))), "{unmatched:?}");
    assert!(ambiguous.iter().all(|e| e["matches"].as_array().is_some_and(|m| m.len() == 2) && e["value"].as_str().is_some_and(|v| v.starts_with('D'))), "{ambiguous:?}");

    // Only an owner or admin may ask.
    let (s, _) = rest(&w.app, "POST", &format!("/api/v1/apps/{}/relations/{RELATION}/convert-preview", w.slug), &w.editor, Some(json!({}))).await;
    assert_eq!(s, StatusCode::FORBIDDEN, "an editor cannot");
    let (s, _) = rest(&w.app, "POST", &format!("/api/v1/apps/{}/relations/no-such-relation/convert-preview", w.slug), &w.owner, Some(json!({}))).await;
    assert_eq!(s, StatusCode::NOT_FOUND, "a relationship the app does not have");
    runner.stop();
}

// 2. Approval links 970 and leaves 30 blank. History holds the old text for the 30.
#[tokio::test]
async fn approval_links_the_matched_and_leaves_the_rest_blank_with_their_text_in_history() {
    let w = world().await;
    let runner = JobRunner::start(&w.state.repository, JOB_KINDS, fast()).unwrap();
    preview(&w).await;
    let p = convert(&w).await;
    let loss = check(&p, "data_loss");
    assert_eq!(loss["outcome"], "fail", "records that match nothing, or several, are lost: {loss}");
    assert!(loss["messages"].to_string().contains("20") && loss["messages"].to_string().contains("10"), "the check says how many: {loss}");

    let slug = w.slug.clone();
    let (linked, blank, one_target): (i64, i64, i64) = w.db.run(move |c| {
        let r = c
            .query_one(
                "SELECT count(*) FILTER (WHERE jsonb_typeof(data->'program_code') = 'array' AND jsonb_array_length(data->'program_code') = 1), \
                        count(*) FILTER (WHERE NOT (data ? 'program_code')), \
                        (SELECT count(DISTINCT from_record) FROM record_links WHERE app_slug = $1 AND field = 'program_code') \
                 FROM dataset_records WHERE app_slug = $1 AND table_id = 'applications'",
                &[&slug],
            )
            .unwrap();
        (r.get(0), r.get(1), r.get(2))
    });
    assert_eq!((linked, blank, one_target), (970, 30, 970), "970 are linked and indexed, 30 are blank");

    let slug = w.slug.clone();
    let kept: i64 = w.db.run(move |c| {
        c.query_one(
            "SELECT count(*) FROM record_history WHERE app_slug = $1 AND changes -> 'program_code' ->> 'old' ~ '^[XD]' AND changes -> 'program_code' ->> 'new' IS NULL",
            &[&slug],
        )
        .unwrap()
        .get(0)
    });
    assert_eq!(kept, 30, "history holds the old text of the 30 that were left blank");
    runner.stop();
}

// 3. The converted app passes the validator with no relationship warning.
#[tokio::test]
async fn the_converted_app_has_no_value_join_left() {
    let w = world().await;
    let runner = JobRunner::start(&w.state.repository, JOB_KINDS, fast()).unwrap();
    preview(&w).await;
    convert(&w).await;
    let stored = w.state.repository.get_app_manifest(&w.slug).unwrap().expect("the app");
    assert!(stored.relationships.is_empty(), "the value join is gone");
    assert!(scaffoldry_engine::manifest_warnings(&stored).is_empty(), "{:?}", scaffoldry_engine::manifest_warnings(&stored));
    assert!(scaffoldry_engine::validate_manifest(&stored).is_ok(), "{:?}", scaffoldry_engine::validate_manifest(&stored));
    let field = stored.tables.iter().find(|t| t.id == "applications").and_then(|t| t.fields.iter().find(|f| f.name == "program_code")).expect("the field");
    assert_eq!(field.field_type, scaffoldry_engine::FieldType::Link);
    assert_eq!(field.link_target_table.as_deref(), Some("programs"));
    runner.stop();
}

// 4. A converted app's links lead to what the old join returned, for the 970.
#[tokio::test]
async fn the_links_lead_to_the_programs_the_old_join_returned() {
    let w = world().await;
    let before = old_join(&w);
    assert_eq!(before.len(), 970, "the old join found one program for 970 applications");
    let runner = JobRunner::start(&w.state.repository, JOB_KINDS, fast()).unwrap();
    preview(&w).await;
    convert(&w).await;
    let slug = w.slug.clone();
    let after: Vec<(String, String)> = w.db.run(move |c| {
        c.query(
            "SELECT l.from_record, p.data->>'name' FROM record_links l JOIN dataset_records p ON p.id = l.to_record \
             WHERE l.app_slug = $1 AND l.field = 'program_code' ORDER BY l.from_record",
            &[&slug],
        )
        .unwrap()
        .iter()
        .map(|r| (r.get(0), r.get(1)))
        .collect()
    });
    assert_eq!(after, before, "every link leads to the program the join returned");
    runner.stop();
}

// 5. A conversion proposed before any preview is flagged and says to run one.
#[tokio::test]
async fn a_conversion_with_no_preview_is_flagged_and_says_to_run_one() {
    let w = world().await;
    let (s, p) = rest(&w.app, "POST", &format!("/api/v1/apps/{}/relations/{RELATION}/convert", w.slug), &w.owner, Some(json!({}))).await;
    assert_eq!(s, StatusCode::CREATED, "{p}");
    let loss = check(&p, "data_loss");
    assert_eq!(loss["outcome"], "fail", "{loss}");
    assert!(loss["messages"].to_string().contains("convert-preview"), "it says which route to run first: {loss}");
    let (s, _) = rest(&w.app, "POST", &format!("/api/v1/apps/{}/relations/{RELATION}/convert", w.slug), &w.editor, Some(json!({}))).await;
    assert_eq!(s, StatusCode::FORBIDDEN, "an editor cannot propose the conversion");
}
