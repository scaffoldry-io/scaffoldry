//! Lifecycle, phase 2: retired fields and archived apps and workspaces.
//!
//! Removing a field does not delete its values. It retires the field: hidden everywhere, refused on write, out of the
//! index and out of every formula, while the values stay in each record's JSON. A proposal brings it back, values and
//! all. An archived app or workspace is read-only and hidden from the default lists, and it is kept. NIST SP 800-53
//! SI-12 (information retention) and AU-11.
//!
//! The brief's fourth test, "the retired-value strip runs only after `keep_retired_days` and not under a hold", waits on
//! retention policies, `purge::destroy` and legal holds (lifecycle phases 3 and 4). None of them exists yet, so a retired
//! field's values are kept until those phases strip them.

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::Router;
use http_body_util::BodyExt;
use scaffoldry_server::build_app_with_state;
use scaffoldry_server::jobs::{JobConfig, JobRunner, JOB_KINDS};
use scaffoldry_server::service::identity::issue_test_token_and_user;
use scaffoldry_server::state::{CollaboratorRecord, ServerState};
use serde_json::{json, Value};
use std::sync::OnceLock;
use std::time::{Duration, Instant};
use tower::ServiceExt;

const WORKSPACE: &str = "ws-physics-optics";
const OWNER: &str = "einstein@physics.state.edu";
const EDITOR: &str = "retire.editor@physics.state.edu";
const ADMIN: &str = "jordan.lee@state.edu";

/// Approving a change that loses data takes a snapshot first, and a snapshot needs a files directory.
fn files_dir() {
    static DIR: OnceLock<std::path::PathBuf> = OnceLock::new();
    DIR.get_or_init(|| {
        let dir = std::env::temp_dir().join(format!("scaffoldry-retire-{}", uuid::Uuid::new_v4()));
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

async fn rest(
    app: &Router,
    method: &str,
    uri: &str,
    token: &str,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let b = Request::builder()
        .method(method)
        .uri(uri)
        .header("authorization", format!("Bearer {token}"));
    let req = match body {
        Some(v) => b
            .header("content-type", "application/json")
            .body(Body::from(v.to_string()))
            .unwrap(),
        None => b.body(Body::empty()).unwrap(),
    };
    let resp = app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

async fn tool_as(w: &World, token: &str, name: &str, args: Value) -> Result<Value, String> {
    let payload = json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": { "name": name, "arguments": args } });
    let (_, v) = rest(&w.app, "POST", "/api/mcp", token, Some(payload)).await;
    if let Some(err) = v.get("error") {
        return Err(err["message"].as_str().unwrap_or("").to_string());
    }
    if v["result"]["isError"] == true {
        return Err(v["result"]["content"][0]["text"]
            .as_str()
            .unwrap_or("")
            .to_string());
    }
    Ok(v["result"]["structuredContent"].clone())
}

fn manifest(slug: &str, fields: Vec<Value>) -> Value {
    json!({
        "slug": slug, "title": "Retire source", "description": "An app whose fields change", "organization_code": "PHYS", "department": "physics",
        "workspace_id": WORKSPACE, "views": [],
        "tables": [{ "id": "cases", "name": "Cases", "slug": "cases", "primary_field": "title", "fields": fields }],
    })
}

fn text(name: &str) -> Value {
    json!({ "name": name, "label": name, "field_type": "Text", "required": false, "ferpa_sensitive": false })
}

fn indexed(name: &str) -> Value {
    let mut f = text(name);
    f["indexed"] = json!(true);
    f
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
    state
        .repository
        .upsert_collaborator(&record)
        .expect("a member");
    state
        .collaborators
        .write()
        .unwrap()
        .entry(WORKSPACE.to_string())
        .or_default()
        .push(record);
    let tag = uuid::Uuid::new_v4().to_string()[..8].to_string();
    let app = build_app_with_state(state.clone()).expect("router");
    World {
        db,
        state,
        app,
        owner: issue_test_token_and_user(OWNER),
        editor: issue_test_token_and_user(EDITOR),
        admin: issue_test_token_and_user(ADMIN),
        slug: format!("retire-{tag}"),
    }
}

async fn live_app(w: &World, fields: Vec<Value>) {
    let (s, b) = rest(
        &w.app,
        "POST",
        &format!("/api/v1/workspaces/{WORKSPACE}/apps"),
        &w.owner,
        Some(manifest(&w.slug, fields)),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{b}");
}

async fn create(w: &World, title: &str, note: &str) -> Value {
    let (s, b) = rest(
        &w.app,
        "POST",
        &format!("/api/v1/apps/{}/records", w.slug),
        &w.owner,
        Some(json!({ "data": { "_table_id": "cases", "title": title, "note": note } })),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{b}");
    b
}

fn fast() -> JobConfig {
    JobConfig {
        workers: 2,
        poll_ms: 20,
        backoff_secs: [0, 0, 0],
        ..JobConfig::default()
    }
}

fn status_of(w: &World, id: &str) -> String {
    let id: uuid::Uuid = id.parse().unwrap();
    w.state
        .repository
        .get_proposal(id)
        .unwrap()
        .expect("the proposal")
        .status
}

fn wait_approved(w: &World, id: &str) {
    let deadline = Instant::now() + Duration::from_secs(120);
    while !status_of(w, id).eq_ignore_ascii_case("approved") {
        assert!(
            Instant::now() < deadline,
            "the proposal is stuck in {}",
            status_of(w, id)
        );
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// The stored JSON of every record of the app, by title.
fn stored(w: &World) -> Vec<Value> {
    let slug = w.slug.clone();
    w.db.run(move |c| {
        c.query(
            "SELECT data FROM dataset_records WHERE app_slug = $1 ORDER BY data->>'title'",
            &[&slug],
        )
        .unwrap()
        .iter()
        .map(|r| r.get(0))
        .collect()
    })
}

fn index_rows(w: &World, field: &str) -> (i64, i64) {
    let (slug, field) = (w.slug.clone(), field.to_string());
    w.db.run(move |c| {
        let rows: i64 = c
            .query_one(
                "SELECT count(*) FROM record_index WHERE app_slug = $1 AND field = $2",
                &[&slug, &field],
            )
            .unwrap()
            .get(0);
        let status: i64 = c
            .query_one(
                "SELECT count(*) FROM index_status WHERE app_slug = $1 AND field = $2",
                &[&slug, &field],
            )
            .unwrap()
            .get(0);
        (rows, status)
    })
}

async fn records(w: &World) -> Vec<Value> {
    let (s, b) = rest(
        &w.app,
        "GET",
        &format!("/api/v1/apps/{}/tables/cases/records", w.slug),
        &w.owner,
        None,
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{b}");
    b["records"].as_array().cloned().unwrap_or_default()
}

async fn table_field_names(w: &World) -> Vec<String> {
    let (s, b) = rest(
        &w.app,
        "GET",
        &format!("/api/v1/apps/{}", w.slug),
        &w.owner,
        None,
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{b}");
    let manifest = if b.get("tables").is_some() {
        b.clone()
    } else {
        b["manifest"].clone()
    };
    manifest["tables"][0]["fields"]
        .as_array()
        .map(|l| {
            l.iter()
                .filter_map(|f| f["name"].as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

// 1. A retired field is hidden, unwritable, unindexed, and its values are still in the JSON. Restoring it brings the
//    values back.
#[tokio::test]
async fn a_retired_field_is_hidden_unwritable_and_unindexed_and_its_values_stay_until_it_is_restored(
) {
    let w = world().await;
    let runner = JobRunner::start(&w.state.repository, JOB_KINDS, fast()).unwrap();
    live_app(&w, vec![text("title"), indexed("note")]).await;
    let first = create(&w, "A", "kept a").await;
    create(&w, "B", "kept b").await;
    let deadline = Instant::now() + Duration::from_secs(60);
    while index_rows(&w, "note").0 < 2 {
        assert!(Instant::now() < deadline, "the note index never filled");
        std::thread::sleep(Duration::from_millis(50));
    }

    // Removing the field is flagged data loss, and the check says the values are kept.
    let p = tool_as(&w, &w.owner, "propose_app_change", json!({ "workspace_id": WORKSPACE, "manifest": manifest(&w.slug, vec![text("title")]), "summary": "Retire note" }))
        .await
        .expect("proposed");
    let loss = p["checks"]
        .as_array()
        .and_then(|l| l.iter().find(|c| c["name"] == "data_loss"))
        .cloned()
        .unwrap_or(Value::Null);
    assert_eq!(loss["outcome"], "fail", "{p}");
    assert!(
        loss.to_string()
            .contains("The values stay for 90 days and can be restored."),
        "{loss}"
    );
    let id = p["proposal_id"].as_str().unwrap().to_string();
    tool_as(&w, &w.admin, "decide_proposal", json!({ "proposal_id": id, "decision": "approve", "rationale": "Reviewed", "digest": p["digest"] })).await.expect("approved");
    wait_approved(&w, &id);

    // Hidden: not in the definition anyone reads, not on any record.
    assert_eq!(table_field_names(&w).await, vec!["title".to_string()]);
    for r in records(&w).await {
        assert!(
            r["data"].get("note").is_none() && r.get("note").is_none(),
            "a retired value is shown: {r}"
        );
    }
    let (_, one) = rest(
        &w.app,
        "GET",
        &format!(
            "/api/v1/apps/{}/records/{}",
            w.slug,
            first["id"].as_str().unwrap()
        ),
        &w.owner,
        None,
    )
    .await;
    assert!(one["data"].get("note").is_none(), "{one}");

    // Unwritable, with a sentence that names the field and the way back.
    let rid = first["id"].as_str().unwrap();
    let version = one["version"].clone();
    let (s, b) = rest(&w.app, "PATCH", &format!("/api/v1/apps/{}/records/{rid}", w.slug), &w.owner, Some(json!({ "version": version, "data": { "_table_id": "cases", "title": "A", "note": "new" } }))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{b}");
    assert!(
        b["error"]
            .as_str()
            .unwrap_or("")
            .contains("'note' is retired"),
        "{b}"
    );

    // A write of the other fields keeps the retired value in the JSON.
    let (s, b) = rest(
        &w.app,
        "PATCH",
        &format!("/api/v1/apps/{}/records/{rid}", w.slug),
        &w.owner,
        Some(json!({ "version": version, "data": { "_table_id": "cases", "title": "A2" } })),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{b}");
    let kept: Vec<Option<String>> = stored(&w)
        .iter()
        .map(|d| d["note"].as_str().map(str::to_string))
        .collect();
    assert_eq!(
        kept,
        vec![Some("kept a".to_string()), Some("kept b".to_string())],
        "the values stay in the JSON"
    );

    // Unindexed: the rows and the status are gone.
    assert_eq!(index_rows(&w, "note"), (0, 0));

    // Restoring is a proposal. It is not data loss, so the owner applies it directly, and the values come back.
    let (s, r) = rest(
        &w.app,
        "POST",
        &format!("/api/v1/apps/{}/tables/cases/fields/note/restore", w.slug),
        &w.owner,
        Some(json!({ "reason": "We still need it" })),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{r}");
    assert_eq!(r["approved"], true, "{r}");
    assert_eq!(
        table_field_names(&w).await,
        vec!["title".to_string(), "note".to_string()]
    );
    let mut notes: Vec<String> = records(&w)
        .await
        .iter()
        .filter_map(|r| r["data"]["note"].as_str().map(str::to_string))
        .collect();
    notes.sort();
    assert_eq!(notes, vec!["kept a".to_string(), "kept b".to_string()]);

    // A field that was never retired cannot be restored.
    let (s, b) = rest(
        &w.app,
        "POST",
        &format!("/api/v1/apps/{}/tables/cases/fields/title/restore", w.slug),
        &w.owner,
        Some(json!({ "reason": "No" })),
    )
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{b}");
    // An editor does not change the definition.
    let (s, _) = rest(
        &w.app,
        "POST",
        &format!("/api/v1/apps/{}/tables/cases/fields/note/restore", w.slug),
        &w.editor,
        Some(json!({ "reason": "No" })),
    )
    .await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    runner.stop();
}

// 2. A formula over a retired field fails validation.
#[tokio::test]
async fn a_formula_over_a_retired_field_fails_validation() {
    let w = world().await;
    let runner = JobRunner::start(&w.state.repository, JOB_KINDS, fast()).unwrap();
    live_app(&w, vec![text("title"), text("note")]).await;
    create(&w, "A", "x").await;
    let p = tool_as(&w, &w.owner, "propose_app_change", json!({ "workspace_id": WORKSPACE, "manifest": manifest(&w.slug, vec![text("title")]), "summary": "Retire note" })).await.expect("proposed");
    let id = p["proposal_id"].as_str().unwrap().to_string();
    tool_as(&w, &w.admin, "decide_proposal", json!({ "proposal_id": id, "decision": "approve", "rationale": "Reviewed", "digest": p["digest"] })).await.expect("approved");
    wait_approved(&w, &id);

    let formula = json!({ "name": "shout", "label": "Shout", "field_type": "Formula", "required": false, "ferpa_sensitive": false, "formula_expression": "UPPER({note})" });
    let err = tool_as(&w, &w.owner, "propose_app_change", json!({ "workspace_id": WORKSPACE, "manifest": manifest(&w.slug, vec![text("title"), formula]), "summary": "Shout" }))
        .await
        .expect_err("a formula over a retired field is refused");
    assert!(err.contains("retired field 'note'"), "{err}");
    runner.stop();
}

async fn app_listed(w: &World, token: &str, query: &str) -> bool {
    let (s, b) = rest(
        &w.app,
        "GET",
        &format!("/api/v1/me/apps{query}"),
        token,
        None,
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{b}");
    b["apps"]
        .as_array()
        .is_some_and(|l| l.iter().any(|a| a["slug"] == w.slug.as_str()))
}

async fn workspace_listed(w: &World, token: &str, query: &str) -> bool {
    let (s, b) = rest(
        &w.app,
        "GET",
        &format!("/api/v1/workspaces{query}"),
        token,
        None,
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{b}");
    let list = b
        .as_array()
        .cloned()
        .or_else(|| b["workspaces"].as_array().cloned())
        .unwrap_or_default();
    list.iter().any(|x| x["id"] == WORKSPACE)
}

fn ledger_count(w: &World, kind: &str) -> i64 {
    let kind = kind.to_string();
    w.db.run(move |c| {
        c.query_one(
            "SELECT count(*) FROM governance_ledger WHERE decision_type = $1",
            &[&kind],
        )
        .unwrap()
        .get(0)
    })
}

// 3. Archiving an app refuses writes with `archived` and hides it from the default list. Unarchiving restores it.
#[tokio::test]
async fn an_archived_app_is_read_only_and_hidden_and_unarchiving_restores_it() {
    let w = world().await;
    live_app(&w, vec![text("title"), text("note")]).await;
    let first = create(&w, "A", "x").await;
    assert!(app_listed(&w, &w.owner, "").await);

    // A reason is required, and an editor may not archive.
    let (s, _) = rest(
        &w.app,
        "POST",
        &format!("/api/v1/apps/{}/archive", w.slug),
        &w.owner,
        Some(json!({})),
    )
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    let (s, _) = rest(
        &w.app,
        "POST",
        &format!("/api/v1/apps/{}/archive", w.slug),
        &w.editor,
        Some(json!({ "reason": "Done" })),
    )
    .await;
    assert_eq!(s, StatusCode::FORBIDDEN);

    let (s, b) = rest(
        &w.app,
        "POST",
        &format!("/api/v1/apps/{}/archive", w.slug),
        &w.owner,
        Some(json!({ "reason": "The programme closed" })),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_eq!(ledger_count(&w, "AppArchived"), 1);

    // Read-only, with the code, and kept: it still reads.
    let (s, b) = rest(
        &w.app,
        "POST",
        &format!("/api/v1/apps/{}/records", w.slug),
        &w.owner,
        Some(json!({ "data": { "_table_id": "cases", "title": "B" } })),
    )
    .await;
    assert_eq!(s, StatusCode::CONFLICT, "{b}");
    assert_eq!(b["code"], "archived", "{b}");
    let (s, b) = rest(
        &w.app,
        "DELETE",
        &format!(
            "/api/v1/apps/{}/records/{}",
            w.slug,
            first["id"].as_str().unwrap()
        ),
        &w.owner,
        None,
    )
    .await;
    assert_eq!(b["code"], "archived", "{s} {b}");
    let p = tool_as(&w, &w.owner, "propose_app_change", json!({ "workspace_id": WORKSPACE, "manifest": manifest(&w.slug, vec![text("title"), text("note"), text("more")]), "summary": "More" })).await;
    assert!(
        p.as_ref().err().is_some_and(|e| e.contains("archived")),
        "{p:?}"
    );
    assert_eq!(
        records(&w).await.len(),
        1,
        "an archived app is kept and reads"
    );

    // Hidden from the default list, listed when asked for.
    assert!(!app_listed(&w, &w.owner, "").await);
    assert!(app_listed(&w, &w.owner, "?include_archived=true").await);

    let (s, b) = rest(
        &w.app,
        "POST",
        &format!("/api/v1/apps/{}/unarchive", w.slug),
        &w.owner,
        Some(json!({ "reason": "It reopened" })),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_eq!(ledger_count(&w, "AppUnarchived"), 1);
    assert!(app_listed(&w, &w.owner, "").await);
    create(&w, "B", "y").await;
}

#[tokio::test]
async fn an_archived_workspace_is_read_only_and_hidden_and_only_its_owner_or_a_platform_admin_archives_it(
) {
    let w = world().await;
    live_app(&w, vec![text("title"), text("note")]).await;
    assert!(workspace_listed(&w, &w.owner, "").await);
    let (s, _) = rest(
        &w.app,
        "POST",
        &format!("/api/v1/workspaces/{WORKSPACE}/archive"),
        &w.editor,
        Some(json!({ "reason": "Done" })),
    )
    .await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    let (s, b) = rest(
        &w.app,
        "POST",
        &format!("/api/v1/workspaces/{WORKSPACE}/archive"),
        &w.admin,
        Some(json!({ "reason": "The lab closed" })),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_eq!(ledger_count(&w, "WorkspaceArchived"), 1);

    // Its apps refuse writes, and the workspace is not in the default list.
    let (s, b) = rest(
        &w.app,
        "POST",
        &format!("/api/v1/apps/{}/records", w.slug),
        &w.owner,
        Some(json!({ "data": { "_table_id": "cases", "title": "B" } })),
    )
    .await;
    assert_eq!(
        (s, b["code"].clone()),
        (StatusCode::CONFLICT, json!("archived")),
        "{b}"
    );
    assert!(!workspace_listed(&w, &w.owner, "").await);
    assert!(workspace_listed(&w, &w.owner, "?include_archived=true").await);

    let (s, b) = rest(
        &w.app,
        "POST",
        &format!("/api/v1/workspaces/{WORKSPACE}/unarchive"),
        &w.owner,
        Some(json!({ "reason": "Reopened" })),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_eq!(ledger_count(&w, "WorkspaceUnarchived"), 1);
    assert!(workspace_listed(&w, &w.owner, "").await);
    create(&w, "B", "y").await;
}
