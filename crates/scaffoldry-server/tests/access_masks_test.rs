//! Access model phase 12: a reader who is not cleared for a field sees its mask instead of nothing.
//!
//! One app, one table of people. The date of birth, the identifier and the salary are protected. Each names a mask.
//! A viewer is cleared for nothing protected, so the viewer sees masks. The owner is cleared and sees the values.

mod common;

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use axum::Router;
use http_body_util::BodyExt;
use scaffoldry_server::build_app_with_state;
use scaffoldry_server::jobs::{get_job, JobConfig, JobRunner, JOB_KINDS};
use scaffoldry_server::service::identity::issue_test_token_and_user;
use scaffoldry_server::state::CollaboratorRecord;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tower::ServiceExt;
use uuid::Uuid;

const WORKSPACE: &str = "ws-physics-optics";
const OWNER: &str = "einstein@physics.state.edu";
const VIEWER: &str = "viewer.p12@physics.state.edu";
const SECOND: &str = "jordan.lee@state.edu";

const SOURCE: &str = "export default function Page() { return <p>PEOPLE-MARKER</p>; }";

struct World {
    _db: common::TestDb,
    state: scaffoldry_server::state::SharedState,
    app: Router,
    slug: String,
    owner: String,
    viewer: String,
    second: String,
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

/// `Err` holds the error code and the message.
async fn tool(w: &World, token: &str, name: &str, args: Value) -> Result<Value, (String, String)> {
    let payload = json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": { "name": name, "arguments": args } });
    let (_, v) = rest(&w.app, "POST", "/api/mcp", token, Some(payload)).await;
    if let Some(err) = v.get("error") {
        return Err((String::new(), err["message"].as_str().unwrap_or("").to_string()));
    }
    if v["result"]["isError"] == true {
        let code = v["result"]["structuredContent"]["code"].as_str().unwrap_or("").to_string();
        return Err((code, v["result"]["content"][0]["text"].as_str().unwrap_or("").to_string()));
    }
    Ok(v["result"]["structuredContent"].clone())
}

fn sha(text: &str) -> String {
    Sha256::digest(text.as_bytes()).iter().map(|b| format!("{b:02x}")).collect()
}

fn plain(name: &str, t: &str) -> Value {
    json!({ "name": name, "label": name, "field_type": t, "required": false, "ferpa_sensitive": false })
}

fn protected(name: &str, t: &str, mask: Value) -> Value {
    json!({ "name": name, "label": name, "field_type": t, "required": false, "ferpa_sensitive": true, "mask": mask })
}

/// The manifest, with the masks the three protected fields name.
fn manifest(slug: &str, dob: Value, ssn: Value, salary: Value) -> Value {
    json!({
        "slug": slug, "title": "People", "description": "d", "organization_code": "PHYS", "department": "physics",
        "views": [{ "id": "all", "table_id": "people", "title": "All", "view_type": "Table", "fields": [plain("name", "Text")] }],
        "tables": [{ "id": "people", "name": "People", "slug": "people", "fields": [
            plain("name", "Text"), protected("dob", "Date", dob), protected("ssn", "Text", ssn), protected("salary", "Number", salary),
        ] }],
        "pages": [{ "id": "board", "title": "Board", "kind": { "Custom": { "source_sha256": sha(SOURCE), "grants": [
            { "table_id": "people", "read": ["name", "dob", "ssn", "salary"], "write": [], "create": false, "delete": false }
        ]}}}]
    })
}

fn year() -> Value {
    json!({ "kind": "year" })
}
fn last4() -> Value {
    json!({ "kind": "last4" })
}
fn band() -> Value {
    json!({ "kind": "band", "width": 10000 })
}

/// Proposes `manifest` as the owner. A mask that shows more than its category default needs a second person.
async fn publish(w: &World, m: Value) -> Result<Value, (String, String)> {
    let proposed = tool(w, &w.owner, "propose_app_change", json!({ "workspace_id": WORKSPACE, "manifest": m, "summary": "Masks", "page_sources": [SOURCE] })).await?;
    let id = proposed["proposal_id"].as_str().expect("a proposal id").to_string();
    for who in [&w.owner, &w.second] {
        let decided = tool(w, who, "decide_proposal", json!({ "proposal_id": id, "decision": "approve", "rationale": "Reviewed", "digest": proposed["digest"] })).await;
        if decided.is_ok() {
            return Ok(proposed);
        }
    }
    Err((String::new(), "nobody could approve it".to_string()))
}

async fn world() -> World {
    let db = common::TestDb::new();
    let dir = std::env::temp_dir().join(format!("scaffoldry-mask-files-{}", Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    std::env::set_var("SCAFFOLDRY_FILES_DIR", &dir);
    let state = db.state();
    state.seed_demo().expect("seed");
    state.collaborators.write().unwrap().entry(WORKSPACE.to_string()).or_default().push(CollaboratorRecord {
        id: format!("collab-{}", Uuid::new_v4()),
        workspace_id: WORKSPACE.to_string(),
        eppn: VIEWER.to_string(),
        name: "Vera Viewer".to_string(),
        role: "viewer".to_string(),
        scoped_affiliation: "faculty".to_string(),
        department: "physics".to_string(),
        added_at: String::new(),
        app_slug: String::new(),
    });
    let app = build_app_with_state(state.clone()).expect("router");
    let w = World {
        _db: db,
        state,
        app,
        slug: format!("mask-{}", &Uuid::new_v4().to_string()[..8]),
        owner: issue_test_token_and_user(OWNER),
        viewer: issue_test_token_and_user(VIEWER),
        second: issue_test_token_and_user(SECOND),
    };
    publish(&w, manifest(&w.slug, year(), last4(), band())).await.expect("the app is published");
    let made = tool(&w, &w.owner, "create_record", json!({ "app_slug": w.slug, "data": { "_table_id": "people", "name": "Ada Lovelace", "dob": "2004-12-31", "ssn": "123456789", "salary": 55250 } })).await;
    made.expect("a record");
    w
}

async fn one_row(w: &World, token: &str) -> Value {
    let page = tool(w, token, "list_records", json!({ "app_slug": w.slug, "table_id": "people", "limit": 10 })).await.expect("a list");
    let rows = page["records"].as_array().unwrap();
    assert_eq!(rows.len(), 1);
    rows[0]["data"].clone()
}

// 2. A viewer reads a protected date of birth whose field mask is `year` and gets the year. An editor gets the date.
#[tokio::test]
async fn a_viewer_sees_the_mask_and_the_owner_sees_the_value() {
    let w = world().await;
    let mine = one_row(&w, &w.viewer).await;
    assert_eq!(mine["dob"], "2004", "{mine}");
    assert_eq!(mine["ssn"], "*****6789");
    assert_eq!(mine["salary"], "50000 to under 60000");
    assert_eq!(mine["name"], "Ada Lovelace", "an unprotected field is as it was");
    let theirs = one_row(&w, &w.owner).await;
    assert_eq!(theirs["dob"], "2004-12-31");
    assert_eq!(theirs["ssn"], "123456789");
    assert_eq!(theirs["salary"], 55250);
    // One record read gives the same.
    let id = {
        let page = tool(&w, &w.owner, "list_records", json!({ "app_slug": w.slug, "table_id": "people" })).await.unwrap();
        page["records"][0]["id"].as_str().unwrap().to_string()
    };
    let got = tool(&w, &w.viewer, "get_record", json!({ "app_slug": w.slug, "record_id": id })).await.expect("a record");
    assert_eq!(got["data"]["dob"], "2004", "{got}");
}

// 3. Filtering, sorting, searching or totalling a masked field is `field_hidden`. A mask must not leak order.
#[tokio::test]
async fn a_masked_field_cannot_be_sorted_filtered_or_totalled() {
    let w = world().await;
    let base = json!({ "app_slug": w.slug, "table_id": "people" });
    let with = |extra: Value| {
        let mut a = base.clone();
        a.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
        a
    };
    let sorted = tool(&w, &w.viewer, "list_records", with(json!({ "sort": { "field_name": "dob", "direction": "Asc" } }))).await.unwrap_err();
    assert_eq!(sorted.0, "field_hidden", "{sorted:?}");
    let filtered = tool(&w, &w.viewer, "list_records", with(json!({ "filter": { "conjunction": "And", "clauses": [{ "id": "c", "field_name": "salary", "operator": "GreaterThan", "value": "50000" }] } }))).await.unwrap_err();
    assert_eq!(filtered.0, "field_hidden", "{filtered:?}");
    let totalled = tool(&w, &w.viewer, "aggregate_records", with(json!({ "function": "Sum", "field": "salary" }))).await.unwrap_err();
    assert_eq!(totalled.0, "field_hidden", "{totalled:?}");
    // The owner is cleared, so the same calls work.
    tool(&w, &w.owner, "list_records", with(json!({ "sort": { "field_name": "dob", "direction": "Asc" } }))).await.expect("the owner may sort");
}

fn check<'a>(proposal: &'a Value, name: &str) -> &'a Value {
    proposal["checks"].as_array().unwrap().iter().find(|c| c["name"] == name).unwrap_or_else(|| panic!("no check {name} in {proposal}"))
}

// 4. Setting a field's mask to show more than its category default raises `mask_weakened`.
#[tokio::test]
async fn a_mask_that_shows_more_than_the_category_default_raises_mask_weakened() {
    let w = world().await;
    let propose = |m: Value| tool(&w, &w.owner, "propose_app_change", json!({ "workspace_id": WORKSPACE, "manifest": m, "summary": "Masks", "page_sources": [SOURCE] }));
    let unchanged = propose(manifest(&w.slug, year(), last4(), band())).await.unwrap();
    assert_eq!(check(&unchanged, "mask_weakened")["outcome"], "pass", "a mask the app already had is not new: {unchanged}");
    let month = propose(manifest(&w.slug, json!({ "kind": "month" }), last4(), band())).await.unwrap();
    let c = check(&month, "mask_weakened");
    assert_eq!(c["outcome"], "fail", "{month}");
    assert!(c.to_string().contains("dob"), "and names the field: {c}");
    let hidden = propose(manifest(&w.slug, json!({ "kind": "hide" }), last4(), band())).await.unwrap();
    assert_eq!(check(&hidden, "mask_weakened")["outcome"], "pass", "hiding is what the category does already: {hidden}");
}

async fn run_export(w: &World, token: &str) -> String {
    let (s, v) = rest(&w.app, "POST", &format!("/api/v1/apps/{}/exports", w.slug), token, Some(json!({ "table_id": "people", "format": "csv" }))).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let job: Uuid = v["job_id"].as_str().unwrap().parse().unwrap();
    let runner = JobRunner::start(&w.state.repository, JOB_KINDS, JobConfig { poll_ms: 10, ..JobConfig::default() }).unwrap();
    for _ in 0..500 {
        if get_job(&w.state.repository, job).unwrap().is_some_and(|j| j.state == "done") {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    runner.stop();
    let req = Request::get(format!("/api/v1/apps/{}/exports/{}/download", w.slug, v["export_id"].as_str().unwrap())).header(header::AUTHORIZATION, format!("Bearer {token}")).body(Body::empty()).unwrap();
    let resp = w.app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    String::from_utf8(resp.into_body().collect().await.unwrap().to_bytes().to_vec()).unwrap()
}

// 5. An export and a page call apply the same mask as a list.
#[tokio::test]
async fn an_export_and_a_page_call_apply_the_same_mask_as_a_list() {
    let w = world().await;
    let csv = run_export(&w, &w.viewer).await;
    assert!(csv.contains("2004") && !csv.contains("2004-12-31"), "{csv}");
    assert!(csv.contains("*****6789") && !csv.contains("123456789"), "{csv}");
    assert!(csv.contains("50000 to under 60000") && !csv.contains("55250"), "{csv}");
    let full = run_export(&w, &w.owner).await;
    assert!(full.contains("2004-12-31") && full.contains("123456789") && full.contains("55250"), "the owner exports the values: {full}");

    let listed = tool(&w, &w.viewer, "page_call", json!({ "app_slug": w.slug, "page_id": "board", "op": "list", "table_id": "people" })).await.expect("a page call");
    let row = &listed["records"][0]["data"];
    assert_eq!((row["dob"].as_str(), row["ssn"].as_str(), row["salary"].as_str()), (Some("2004"), Some("*****6789"), Some("50000 to under 60000")), "{listed}");
}

// 6. A masked read is not a disclosure of the protected value. Only a read with no mask is.
#[tokio::test]
async fn a_masked_export_is_not_a_disclosure_and_an_unmasked_one_is() {
    let w = world().await;
    run_export(&w, &w.viewer).await;
    let logged = |w: &World| w.state.repository.list_disclosures(scaffoldry_server::repository_disclosure::DisclosureFilter { app: Some(w.slug.clone()), ..Default::default() }, 10, 0).unwrap();
    assert!(logged(&w).is_empty(), "the viewer saw masks only");
    run_export(&w, &w.owner).await;
    let after = logged(&w);
    assert_eq!(after.len(), 1);
    assert_eq!(after[0].fields, vec!["dob".to_string(), "salary".to_string(), "ssn".to_string()]);
}
