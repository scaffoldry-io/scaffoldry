//! Proposal preview phase 3: a trial copy to try writes.
//!
//! A trial is the proposed app in a synthetic workspace, filled with synthetic records. It never holds a live record, and a
//! write in it changes nothing outside it. A person has a few at a time. A trial ends when its proposal is decided, or when
//! it has gone unopened for the number of days in the settings. NIST SP 800-53 CM-3, CM-4 and SA-11.

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::Router;
use http_body_util::BodyExt;
use scaffoldry_server::build_app_with_state;
use scaffoldry_server::service::identity::issue_test_token_and_user;
use scaffoldry_server::state::{CollaboratorRecord, ServerState};
use serde_json::{json, Value};
use tower::ServiceExt;

const WORKSPACE: &str = "ws-physics-optics";
const OWNER: &str = "einstein@physics.state.edu";
const PLATFORM_ADMIN: &str = "jordan.lee@state.edu";

struct World {
    _db: common::TestDb,
    app: Router,
    owner: String,
    second: String,
    viewer: String,
    platform: String,
    slug: String,
}

fn member(state: &ServerState, eppn: &str, role: &str) {
    let record = CollaboratorRecord {
        id: uuid::Uuid::new_v4().to_string(),
        workspace_id: WORKSPACE.to_string(),
        eppn: eppn.to_string(),
        name: eppn.to_string(),
        role: role.to_string(),
        scoped_affiliation: "faculty".to_string(),
        department: "physics".to_string(),
        added_at: String::new(),
        app_slug: String::new(),
    };
    // Stored, so a reload of the caches keeps the member.
    state.repository.upsert_collaborator(&record).expect("a member");
    state.collaborators.write().unwrap().entry(WORKSPACE.to_string()).or_default().push(CollaboratorRecord {
        id: uuid::Uuid::new_v4().to_string(),
        workspace_id: WORKSPACE.to_string(),
        eppn: eppn.to_string(),
        name: eppn.to_string(),
        role: role.to_string(),
        scoped_affiliation: "faculty".to_string(),
        department: "physics".to_string(),
        added_at: String::new(),
        app_slug: String::new(),
    });
}

async fn world() -> World {
    let db = common::TestDb::new();
    let state = db.state();
    state.seed_demo().expect("seed");
    let tag = uuid::Uuid::new_v4().to_string()[..8].to_string();
    let (second, viewer) = (format!("second-{tag}@state.edu"), format!("viewer-{tag}@state.edu"));
    member(&state, &second, "admin");
    member(&state, &viewer, "viewer");
    let app = build_app_with_state(state).expect("router");
    World {
        _db: db,
        app,
        owner: issue_test_token_and_user(OWNER),
        second: issue_test_token_and_user(&second),
        viewer: issue_test_token_and_user(&viewer),
        platform: issue_test_token_and_user(PLATFORM_ADMIN),
        slug: format!("pv-{tag}"),
    }
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

async fn tool(w: &World, token: &str, name: &str, args: Value) -> Result<Value, String> {
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

fn field(name: &str) -> Value {
    json!({ "name": name, "label": name, "field_type": "Text", "required": false, "ferpa_sensitive": false })
}

/// A row rule that shows a `cases` row only when `title` is not `hidden`.
fn hide(title: &str) -> Value {
    json!({
        "id": format!("hide-{title}"), "table_id": "cases", "ops": ["read"],
        "visible_when": { "cmp": { "field": "title", "op": "ne", "value": { "literal": title } } },
        "except_roles": [], "message": null
    })
}

fn manifest(slug: &str, fields: &[&str], rows: Vec<Value>) -> Value {
    json!({
        "slug": slug, "title": "Preview", "description": "d", "organization_code": "PHYS", "department": "physics",
        "workspace_id": WORKSPACE, "views": [], "pages": [],
        "tables": [{ "id": "cases", "name": "Cases", "slug": "cases", "fields": fields.iter().map(|f| field(f)).collect::<Vec<_>>() }],
        "access": { "rows": rows, "columns": [] }
    })
}

async fn propose(w: &World, fields: &[&str], rows: Vec<Value>) -> Value {
    tool(w, &w.owner, "propose_app_change", json!({ "workspace_id": WORKSPACE, "manifest": manifest(&w.slug, fields, rows), "summary": "A change" })).await.expect("proposed")
}

/// Puts the app live with `title` and `note`, a rule that hides `secret`, and two rows.
async fn live(w: &World) {
    let p = propose(w, &["title", "note"], vec![hide("secret")]).await;
    tool(w, &w.owner, "decide_proposal", json!({ "proposal_id": p["proposal_id"], "decision": "approve", "rationale": "Set up", "digest": p["digest"] })).await.expect("live");
    for title in ["plain", "secret"] {
        tool(w, &w.owner, "create_record", json!({ "app_slug": w.slug, "data": { "_table_id": "cases", "title": title, "note": format!("note of {title}") } })).await.expect("record");
    }
}


async fn open_trial(w: &World, token: &str, proposal: &Value) -> (StatusCode, Value) {
    rest(&w.app, "POST", &format!("/api/v1/proposals/{}/trial", proposal["proposal_id"].as_str().unwrap()), token, None).await
}

async fn trial_records(w: &World, token: &str, slug: &str) -> (StatusCode, Vec<Value>) {
    let (s, body) = rest(&w.app, "GET", &format!("/api/v1/apps/{slug}/tables/cases/records?limit=1000"), token, None).await;
    (s, body["records"].as_array().cloned().unwrap_or_default())
}

/// How many records the live app holds, whatever the owner may see of them.
fn live_count(w: &World) -> i64 {
    let live = w.slug.clone();
    w._db.run(move |c| c.query_one("SELECT COUNT(*) FROM dataset_records WHERE app_slug = $1", &[&live]).unwrap().get(0))
}

async fn put_setting(w: &World, key: &str, value: Value) -> StatusCode {
    rest(&w.app, "PUT", &format!("/api/v1/settings/{key}"), &w.platform, Some(value)).await.0
}

// 1. A trial has the proposed tables and synthetic records, and no live record.
#[tokio::test]
async fn a_trial_has_the_proposed_tables_and_synthetic_records_and_no_live_record() {
    let w = world().await;
    live(&w).await;
    let p = propose(&w, &["title", "note", "extra"], vec![hide("secret")]).await;
    let (status, trial) = open_trial(&w, &w.owner, &p).await;
    assert_eq!(status, StatusCode::CREATED, "{trial}");
    let slug = trial["app_slug"].as_str().expect("a slug").to_string();
    assert!(slug.contains("~trial~") && slug != w.slug, "a trial has a slug of its own: {slug}");

    let (status, rows) = trial_records(&w, &w.owner, &slug).await;
    assert_eq!(status, StatusCode::OK);
    assert!(!rows.is_empty() && rows.len() <= 500, "{} records", rows.len());
    assert_eq!(trial["records"], rows.len(), "{trial}");
    // The proposed field is there, and no live value is.
    assert!(rows.iter().all(|r| r["data"].get("extra").is_some()), "every record carries the proposed field");
    for r in &rows {
        let title = r["data"]["title"].as_str().unwrap_or("");
        assert!(title != "plain" && title != "secret", "a live value reached the trial: {title}");
    }
    // Nothing in the trial was written by a person, and the live app still has its two rows.
    let writers: i64 = w._db.run({
        let slug = slug.clone();
        move |c| c.query_one("SELECT COUNT(*) FROM dataset_records WHERE app_slug = $1 AND created_by IS DISTINCT FROM 'synthetic@synthetic.invalid'", &[&slug]).unwrap().get(0)
    });
    assert_eq!(writers, 0, "every trial record is synthetic");
    assert_eq!(live_count(&w), 2, "the live app still has its two rows");
}

// 2. Writing in a trial changes nothing outside it.
#[tokio::test]
async fn writing_in_a_trial_changes_nothing_outside_it() {
    let w = world().await;
    live(&w).await;
    let p = propose(&w, &["title", "note", "extra"], vec![hide("secret")]).await;
    let (_, trial) = open_trial(&w, &w.owner, &p).await;
    let slug = trial["app_slug"].as_str().unwrap().to_string();
    let before_proposal = rest(&w.app, "GET", &format!("/api/v1/proposals/{}", p["proposal_id"].as_str().unwrap()), &w.owner, None).await.1;
    let live_before = w._db.run({
        let live = w.slug.clone();
        move |c| c.query_one("SELECT COUNT(*), COALESCE(MAX(version), 0)::bigint FROM dataset_records WHERE app_slug = $1", &[&live]).map(|r| (r.get::<_, i64>(0), r.get::<_, i64>(1))).unwrap()
    });

    let (status, made) = rest(&w.app, "POST", &format!("/api/v1/apps/{slug}/tables/cases/records"), &w.owner, Some(json!({ "data": { "title": "Mine", "note": "n", "extra": "e" } }))).await;
    assert_eq!(status, StatusCode::CREATED, "a person can write in a trial: {made}");
    let id = made["id"].as_str().unwrap().to_string();
    let (status, _) = rest(&w.app, "PATCH", &format!("/api/v1/apps/{slug}/tables/cases/records/{id}"), &w.owner, Some(json!({ "data": { "title": "Mine, changed" }, "version": 1 }))).await;
    assert_eq!(status, StatusCode::OK);
    let (status, _) = rest(&w.app, "DELETE", &format!("/api/v1/apps/{slug}/tables/cases/records/{id}"), &w.owner, None).await;
    assert!(status.is_success(), "{status}");

    let live_after = w._db.run({
        let live = w.slug.clone();
        move |c| c.query_one("SELECT COUNT(*), COALESCE(MAX(version), 0)::bigint FROM dataset_records WHERE app_slug = $1", &[&live]).map(|r| (r.get::<_, i64>(0), r.get::<_, i64>(1))).unwrap()
    });
    assert_eq!(live_before, live_after, "the live app's records did not change");
    assert_eq!(live_count(&w), 2, "the live app still has its two rows");
    let after_proposal = rest(&w.app, "GET", &format!("/api/v1/proposals/{}", p["proposal_id"].as_str().unwrap()), &w.owner, None).await.1;
    assert_eq!(before_proposal["status"], after_proposal["status"]);
    assert_eq!(before_proposal["digest"], after_proposal["digest"], "the proposal is what it was");
}

// 3. A fourth trial for the same person is refused with 429. Another person is not held back by it.
#[tokio::test]
async fn a_fourth_trial_for_the_same_person_is_refused_with_429() {
    let w = world().await;
    live(&w).await;
    let p = propose(&w, &["title", "note", "extra"], vec![hide("secret")]).await;
    for n in 0..3 {
        let (status, body) = open_trial(&w, &w.owner, &p).await;
        assert_eq!(status, StatusCode::CREATED, "trial {n}: {body}");
    }
    let (status, refused) = open_trial(&w, &w.owner, &p).await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS, "{refused}");
    let (_, mine) = rest(&w.app, "GET", "/api/v1/me/trials", &w.owner, None).await;
    assert_eq!(mine["limit"], 3, "{mine}");
    assert_eq!(mine["trials"].as_array().map(Vec::len), Some(3), "{mine}");
    // The second admin has their own three.
    let (status, theirs) = open_trial(&w, &w.second, &p).await;
    assert_eq!(status, StatusCode::CREATED, "{theirs}");
}

// 4. A trial is destroyed when its proposal is decided.
#[tokio::test]
async fn a_trial_is_destroyed_when_its_proposal_is_approved_or_rejected() {
    let w = world().await;
    live(&w).await;
    let approved = propose(&w, &["title", "note", "extra"], vec![hide("secret")]).await;
    let (_, trial) = open_trial(&w, &w.owner, &approved).await;
    let slug = trial["app_slug"].as_str().unwrap().to_string();
    assert_eq!(trial_records(&w, &w.owner, &slug).await.0, StatusCode::OK);

    tool(&w, &w.owner, "decide_proposal", json!({ "proposal_id": approved["proposal_id"], "decision": "approve", "rationale": "Reviewed", "digest": approved["digest"] })).await.expect("approved");
    assert_eq!(trial_records(&w, &w.owner, &slug).await.0, StatusCode::NOT_FOUND, "the trial is gone");
    let (_, mine) = rest(&w.app, "GET", "/api/v1/me/trials", &w.owner, None).await;
    assert_eq!(mine["trials"], json!([]), "{mine}");
    let records_left: i64 = w._db.run({
        let slug = slug.clone();
        move |c| c.query_one("SELECT COUNT(*) FROM dataset_records WHERE app_slug = $1", &[&slug]).unwrap().get(0)
    });
    assert_eq!(records_left, 0, "its records are destroyed, not hidden");

    // A rejection ends one too.
    let rejected = propose(&w, &["title", "note", "other"], vec![hide("secret")]).await;
    let (_, trial) = open_trial(&w, &w.owner, &rejected).await;
    let slug = trial["app_slug"].as_str().unwrap().to_string();
    tool(&w, &w.owner, "decide_proposal", json!({ "proposal_id": rejected["proposal_id"], "decision": "reject", "rationale": "No" })).await.expect("rejected");
    assert_eq!(trial_records(&w, &w.owner, &slug).await.0, StatusCode::NOT_FOUND);
}

// 5. The numbers are settings.
#[tokio::test]
async fn the_limits_are_settings_and_a_bad_value_is_refused() {
    let w = world().await;
    live(&w).await;
    let p = propose(&w, &["title", "note", "extra"], vec![hide("secret")]).await;
    assert_eq!(put_setting(&w, "preview.trials_per_person", json!(0)).await, StatusCode::BAD_REQUEST);
    assert_eq!(put_setting(&w, "preview.trial_records", json!(0)).await, StatusCode::BAD_REQUEST);
    assert_eq!(put_setting(&w, "preview.trial_days", json!("seven")).await, StatusCode::BAD_REQUEST);

    assert_eq!(put_setting(&w, "preview.trials_per_person", json!(1)).await, StatusCode::OK);
    assert_eq!(put_setting(&w, "preview.trial_records", json!(7)).await, StatusCode::OK);
    let (status, trial) = open_trial(&w, &w.owner, &p).await;
    assert_eq!(status, StatusCode::CREATED, "{trial}");
    assert!(trial["records"].as_u64().unwrap() <= 7, "{trial}");
    assert_eq!(open_trial(&w, &w.owner, &p).await.0, StatusCode::TOO_MANY_REQUESTS, "the limit is one now");
}

// 6. A trial unopened for the number of days in the settings is destroyed. One opened since is kept.
#[tokio::test]
async fn a_trial_unopened_for_the_set_days_is_destroyed_and_a_recent_one_is_kept() {
    let w = world().await;
    live(&w).await;
    let p = propose(&w, &["title", "note", "extra"], vec![hide("secret")]).await;
    let (_, old) = open_trial(&w, &w.owner, &p).await;
    let (_, recent) = open_trial(&w, &w.owner, &p).await;
    let old_id = old["trial_id"].as_str().unwrap().to_string();
    w._db.run({
        let id = uuid::Uuid::parse_str(&old_id).unwrap();
        move |c| c.execute("UPDATE proposal_trials SET last_opened_at = NOW() - INTERVAL '8 days' WHERE id = $1", &[&id]).unwrap()
    });

    let state = w._db.state();
    let gone = scaffoldry_server::service::trials::expire(&state).expect("expire");
    assert_eq!(gone, 1);
    assert_eq!(trial_records(&w, &w.owner, old["app_slug"].as_str().unwrap()).await.0, StatusCode::NOT_FOUND);
    assert_eq!(trial_records(&w, &w.owner, recent["app_slug"].as_str().unwrap()).await.0, StatusCode::OK);
    assert!(scaffoldry_server::jobs::JOB_KINDS.iter().any(|k| k.name == "expire_trials"), "a scheduled job runs it");
}

// 7. Opening a trial is for the people who may open a preview, and only while the proposal waits.
#[tokio::test]
async fn only_people_who_may_see_the_proposal_open_a_trial_and_only_while_it_waits() {
    let w = world().await;
    live(&w).await;
    let p = propose(&w, &["title", "note", "extra"], vec![hide("secret")]).await;
    assert_eq!(open_trial(&w, &w.viewer, &p).await.0, StatusCode::FORBIDDEN, "a viewer of the workspace does not decide it");
    assert_eq!(open_trial(&w, &w.second, &p).await.0, StatusCode::CREATED, "a person who may decide it does");
    tool(&w, &w.owner, "decide_proposal", json!({ "proposal_id": p["proposal_id"], "decision": "reject", "rationale": "No" })).await.expect("rejected");
    assert_eq!(open_trial(&w, &w.owner, &p).await.0, StatusCode::CONFLICT, "a decided proposal has no trial");
}

// 8. Opening a trial again starts its days over, and only its owner may.
#[tokio::test]
async fn opening_a_trial_again_starts_its_days_over_and_only_for_its_owner() {
    let w = world().await;
    live(&w).await;
    let p = propose(&w, &["title", "note", "extra"], vec![hide("secret")]).await;
    let (_, trial) = open_trial(&w, &w.owner, &p).await;
    let id = trial["trial_id"].as_str().unwrap().to_string();
    w._db.run({
        let id = uuid::Uuid::parse_str(&id).unwrap();
        move |c| c.execute("UPDATE proposal_trials SET last_opened_at = NOW() - INTERVAL '6 days' WHERE id = $1", &[&id]).unwrap()
    });
    let (status, again) = rest(&w.app, "POST", &format!("/api/v1/me/trials/{id}/open"), &w.owner, None).await;
    assert_eq!(status, StatusCode::OK, "{again}");
    // Six days old, then opened: it is not stale at eight days from the first opening.
    w._db.run({
        let id = uuid::Uuid::parse_str(&id).unwrap();
        move |c| c.execute("UPDATE proposal_trials SET created_at = NOW() - INTERVAL '20 days' WHERE id = $1", &[&id]).unwrap()
    });
    assert_eq!(scaffoldry_server::service::trials::expire(&w._db.state()).unwrap(), 0, "an old trial that was opened lately stays");
    let (status, _) = rest(&w.app, "POST", &format!("/api/v1/me/trials/{id}/open"), &w.second, None).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "another person has no such trial");
    let (status, _) = rest(&w.app, "POST", "/api/v1/me/trials/not-an-id/open", &w.owner, None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}
