//! MCP apps phase 4: an agent proposes an app change, with page source. A person approves it.
//!
//! Each test uses its own app slug, so the tests share the database without touching anyone's rows.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::Router;
use http_body_util::BodyExt;
use scaffoldry_server::build_app_with_state;
use scaffoldry_server::service::identity::issue_test_token_and_user;
use scaffoldry_server::state::{CollaboratorRecord, ServerState};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::sync::Arc;
use tokio::sync::Mutex;
use tower::ServiceExt;
use uuid::Uuid;

static LOCK: Mutex<()> = Mutex::const_new(());

/// An Internal workspace.
const WORKSPACE: &str = "ws-physics-optics";
const OWNER: &str = "einstein@physics.state.edu";
const EDITOR: &str = "editor.test@physics.state.edu";
const OTHER_ADMIN: &str = "jordan.lee@state.edu";

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

fn proposal_rows(slug: &str) -> i64 {
    let slug = slug.to_string();
    db(move |c| c.query_one("SELECT count(*) FROM app_proposals WHERE app_slug = $1", &[&slug]).unwrap().get(0))
}

fn sha(text: &str) -> String {
    Sha256::digest(text.as_bytes()).iter().map(|b| format!("{b:02x}")).collect()
}

struct World {
    app: Router,
    owner: String,
    editor: String,
    admin: String,
    slug: String,
}

async fn world() -> World {
    let state = Arc::new(ServerState::new().expect("state"));
    state.seed_demo().expect("seed");
    state.collaborators.write().unwrap().entry(WORKSPACE.to_string()).or_default().push(CollaboratorRecord {
        id: format!("collab-{}", Uuid::new_v4()),
        workspace_id: WORKSPACE.to_string(),
        eppn: EDITOR.to_string(),
        name: "Edith Editor".to_string(),
        role: "editor".to_string(),
        scoped_affiliation: "faculty".to_string(),
        department: "physics".to_string(),
        added_at: String::new(),
    });
    let app = build_app_with_state(state).expect("router");
    let slug = format!("prop-{}", &Uuid::new_v4().to_string()[..8]);
    World {
        app,
        owner: issue_test_token_and_user(OWNER),
        editor: issue_test_token_and_user(EDITOR),
        admin: issue_test_token_and_user(OTHER_ADMIN),
        slug,
    }
}

fn field(name: &str, label: &str, t: &str, sensitive: bool) -> Value {
    json!({ "name": name, "label": label, "field_type": t, "required": false, "ferpa_sensitive": sensitive })
}

fn manifest(slug: &str, fields: Vec<Value>) -> Value {
    json!({
        "slug": slug, "title": "Proposed", "description": "d", "organization_code": "PHYS", "department": "physics",
        "views": [],
        "tables": [{ "id": "grants", "name": "Grants", "slug": "grants", "fields": fields }]
    })
}

fn base_fields() -> Vec<Value> {
    vec![field("title", "Title", "Text", false), field("amount", "Amount", "Number", false), field("status", "Status", "Text", false)]
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

/// Calls a tool. Returns `(is_error, payload)`.
async fn tool(app: &Router, token: &str, name: &str, args: Value) -> (bool, Value) {
    let payload = json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": { "name": name, "arguments": args } });
    let (_, v) = rest(app, "POST", "/api/mcp", token, Some(payload)).await;
    if let Some(err) = v.get("error") {
        return (true, err["message"].clone());
    }
    let result = &v["result"];
    if result["isError"] == true {
        return (true, result["content"][0]["text"].clone());
    }
    (false, result["structuredContent"].clone())
}

/// A live app, made by the owner through the existing route.
async fn live_app(w: &World, fields: Vec<Value>) {
    let (s, body) = rest(&w.app, "POST", &format!("/api/v1/workspaces/{WORKSPACE}/apps"), &w.owner, Some(manifest(&w.slug, fields))).await;
    if s == StatusCode::ACCEPTED {
        // A sensitive field in an Internal workspace needs a second person.
        let id = body["proposal_id"].as_str().unwrap().to_string();
        let (err, done) = decide(w, &w.admin, &id, "approve").await;
        assert!(!err, "{done}");
        return;
    }
    assert_eq!(s, StatusCode::CREATED, "{body}");
}

async fn propose(w: &World, token: &str, m: Value, sources: Vec<String>) -> (bool, Value) {
    tool(&w.app, token, "propose_app_change", json!({ "workspace_id": WORKSPACE, "manifest": m, "summary": "A change", "page_sources": sources })).await
}

async fn decide(w: &World, token: &str, id: &str, decision: &str) -> (bool, Value) {
    tool(&w.app, token, "decide_proposal", json!({ "proposal_id": id, "decision": decision, "rationale": "Reviewed" })).await
}

fn check<'a>(result: &'a Value, name: &str) -> &'a Value {
    result["checks"].as_array().unwrap().iter().find(|c| c["name"] == name).unwrap_or_else(|| panic!("no check {name} in {result}"))
}

async fn live_version(w: &World) -> (i64, Value) {
    let (err, d) = tool(&w.app, &w.owner, "describe_app", json!({ "app_slug": w.slug })).await;
    assert!(!err, "{d}");
    (d["version"].as_i64().unwrap(), d)
}

fn ledger_entries_for(proposal: &str) -> i64 {
    let id = proposal.to_string();
    db(move |c| {
        c.query_one("SELECT count(*) FROM governance_ledger WHERE decision_type = 'AppPublished' AND payload->>'proposal_id' = $1", &[&id])
            .unwrap()
            .get(0)
    })
}

// 1. A proposal changes no live app.
#[tokio::test]
async fn proposing_leaves_the_live_app_unchanged() {
    let _turn = LOCK.lock().await;
    let w = world().await;
    live_app(&w, base_fields()).await;
    let (v0, before) = live_version(&w).await;

    let mut fields = base_fields();
    fields.push(field("notes", "Notes", "Text", false));
    let (err, p) = propose(&w, &w.owner, manifest(&w.slug, fields), vec![]).await;
    assert!(!err, "{p}");
    assert!(p["proposal_id"].is_string());
    assert!(p["changes"].as_array().unwrap().iter().any(|c| c.as_str().unwrap().contains("notes")), "{p}");

    let (v1, after) = live_version(&w).await;
    assert_eq!(v0, v1);
    assert_eq!(before, after);
}

// 2. Approving writes the manifest, raises the version by one, and adds one ledger entry.
#[tokio::test]
async fn approving_writes_the_manifest_raises_the_version_and_adds_one_ledger_entry() {
    let _turn = LOCK.lock().await;
    let w = world().await;
    live_app(&w, base_fields()).await;
    let (v0, _) = live_version(&w).await;

    let mut fields = base_fields();
    fields.push(field("notes", "Notes", "Text", false));
    let (_, p) = propose(&w, &w.owner, manifest(&w.slug, fields), vec![]).await;
    let id = p["proposal_id"].as_str().unwrap().to_string();
    assert!(p["checks"].as_array().unwrap().iter().all(|c| c["outcome"] == "pass"), "a clean change: {p}");

    let (err, done) = decide(&w, &w.owner, &id, "approve").await;
    assert!(!err, "the proposer is an owner and nothing is flagged: {done}");
    let (v1, after) = live_version(&w).await;
    assert_eq!(v1, v0 + 1);
    assert!(after["tables"][0]["fields"].as_array().unwrap().iter().any(|f| f["name"] == "notes"));
    assert_eq!(ledger_entries_for(&id), 1);

    let (_, got) = tool(&w.app, &w.owner, "get_proposal", json!({ "proposal_id": id })).await;
    assert_eq!(got["status"], "Approved");
    assert_eq!(got["decided_by"], OWNER);
}

// 3. Two proposals on one base: the second approval is refused and its row is Superseded.
#[tokio::test]
async fn the_second_approval_on_one_base_version_is_refused_and_superseded() {
    let _turn = LOCK.lock().await;
    let w = world().await;
    live_app(&w, base_fields()).await;
    let mk = |extra: &str| {
        let mut f = base_fields();
        f.push(field(extra, extra, "Text", false));
        manifest(&w.slug, f)
    };
    let (_, a) = propose(&w, &w.owner, mk("first_extra"), vec![]).await;
    let (_, b) = propose(&w, &w.owner, mk("second_extra"), vec![]).await;
    let (a, b) = (a["proposal_id"].as_str().unwrap().to_string(), b["proposal_id"].as_str().unwrap().to_string());

    let (err, _) = decide(&w, &w.owner, &a, "approve").await;
    assert!(!err);
    let (err, msg) = decide(&w, &w.owner, &b, "approve").await;
    assert!(err, "{msg}");
    assert!(msg.as_str().unwrap().to_lowercase().contains("version"), "{msg}");
    let (_, got) = tool(&w.app, &w.owner, "get_proposal", json!({ "proposal_id": b })).await;
    assert_eq!(got["status"], "Superseded");
    assert_eq!(ledger_entries_for(&b), 0);
}

// 4. A sensitive field in an Internal workspace is flagged. The proposer cannot approve it.
#[tokio::test]
async fn a_sensitive_field_in_an_internal_workspace_needs_a_second_person() {
    let _turn = LOCK.lock().await;
    let w = world().await;
    live_app(&w, base_fields()).await;
    let mut fields = base_fields();
    fields.push(field("ssn", "SSN", "Text", true));
    let (_, p) = propose(&w, &w.owner, manifest(&w.slug, fields), vec![]).await;
    let id = p["proposal_id"].as_str().unwrap().to_string();
    assert_eq!(check(&p, "sensitivity")["outcome"], "fail", "{p}");

    let (err, msg) = decide(&w, &w.owner, &id, "approve").await;
    assert!(err, "the proposer may not approve a flagged change: {msg}");
    let (_, still) = tool(&w.app, &w.owner, "get_proposal", json!({ "proposal_id": id })).await;
    assert_eq!(still["status"], "Pending");

    let (err, done) = decide(&w, &w.admin, &id, "approve").await;
    assert!(!err, "a different admin may: {done}");
    assert_eq!(ledger_entries_for(&id), 1);
}

// 5. If the ledger insert fails, nothing is kept.
#[tokio::test]
async fn when_the_ledger_insert_fails_the_version_and_the_proposal_are_unchanged() {
    let _turn = LOCK.lock().await;
    let w = world().await;
    live_app(&w, base_fields()).await;
    let (v0, _) = live_version(&w).await;
    let mut fields = base_fields();
    fields.push(field("notes", "Notes", "Text", false));
    let (_, p) = propose(&w, &w.owner, manifest(&w.slug, fields), vec![]).await;
    let id = p["proposal_id"].as_str().unwrap().to_string();

    let tag = id.replace('-', "_");
    let (t1, id1) = (tag.clone(), id.clone());
    db(move |c| {
        c.batch_execute(&format!(
            "CREATE FUNCTION fail_ledger_{t1}() RETURNS trigger AS $$ BEGIN \
               IF NEW.payload->>'proposal_id' = '{id1}' THEN RAISE EXCEPTION 'ledger refused'; END IF; RETURN NEW; END; $$ LANGUAGE plpgsql; \
             CREATE TRIGGER fail_ledger_{t1} BEFORE INSERT ON governance_ledger FOR EACH ROW EXECUTE FUNCTION fail_ledger_{t1}();"
        ))
        .unwrap();
    });
    let (err, _) = decide(&w, &w.owner, &id, "approve").await;
    db(move |c| c.batch_execute(&format!("DROP TRIGGER fail_ledger_{tag} ON governance_ledger; DROP FUNCTION fail_ledger_{tag}();")).unwrap());

    assert!(err, "the approval failed");
    let (v1, _) = live_version(&w).await;
    assert_eq!(v0, v1, "the manifest version did not move");
    let (_, got) = tool(&w.app, &w.owner, "get_proposal", json!({ "proposal_id": id })).await;
    assert_eq!(got["status"], "Pending");
}

// 6. An editor can propose and cannot decide.
#[tokio::test]
async fn an_editor_can_propose_and_cannot_decide() {
    let _turn = LOCK.lock().await;
    let w = world().await;
    live_app(&w, base_fields()).await;
    let mut fields = base_fields();
    fields.push(field("notes", "Notes", "Text", false));
    let (err, p) = propose(&w, &w.editor, manifest(&w.slug, fields), vec![]).await;
    assert!(!err, "{p}");
    let id = p["proposal_id"].as_str().unwrap().to_string();
    let (err, _) = decide(&w, &w.editor, &id, "approve").await;
    assert!(err, "an editor may not decide");
    let (err, _) = decide(&w, &w.editor, &id, "reject").await;
    assert!(err, "or reject");
}

fn page(sha256: &str, read: &[&str], write: &[&str]) -> Value {
    json!({ "id": "award-dashboard", "title": "Award dashboard", "kind": { "Custom": { "source_sha256": sha256, "grants": [
        { "table_id": "grants", "read": read, "write": write, "create": false, "delete": false }
    ]}}})
}

// 7. A page hash with no stored source returns the page_source error and stores no row.
#[tokio::test]
async fn a_page_whose_source_was_not_sent_is_refused_and_stores_no_row() {
    let _turn = LOCK.lock().await;
    let w = world().await;
    live_app(&w, base_fields()).await;
    let base_rows = proposal_rows(&w.slug);
    let mut m = manifest(&w.slug, base_fields());
    m["pages"] = json!([page(&sha("never sent"), &["title"], &[])]);
    let (err, msg) = propose(&w, &w.owner, m, vec![]).await;
    assert!(err, "{msg}");
    assert!(msg.as_str().unwrap().contains("page_source"), "{msg}");
    assert_eq!(proposal_rows(&w.slug), base_rows, "no proposal row was stored");
}

#[tokio::test]
async fn a_source_with_a_closing_script_tag_or_over_256_kb_is_refused() {
    let _turn = LOCK.lock().await;
    let w = world().await;
    live_app(&w, base_fields()).await;
    let base_rows = proposal_rows(&w.slug);
    for bad in ["export default () => '</ScRiPt><script>alert(1)</script>'".to_string(), "x".repeat(256 * 1024 + 1)] {
        let mut m = manifest(&w.slug, base_fields());
        m["pages"] = json!([page(&sha(&bad), &["title"], &[])]);
        let (err, msg) = propose(&w, &w.owner, m, vec![bad]).await;
        assert!(err, "{msg}");
        assert!(msg.as_str().unwrap().contains("page_source"), "{msg}");
    }
    assert_eq!(proposal_rows(&w.slug), base_rows, "no proposal row was stored");
}

// 8. A page granted read on a sensitive field fails page_sensitivity.
#[tokio::test]
async fn a_page_that_reads_a_sensitive_field_is_flagged_and_the_proposer_cannot_approve() {
    let _turn = LOCK.lock().await;
    let w = world().await;
    let mut fields = base_fields();
    fields.push(field("ssn", "Social security number", "Text", true));
    live_app(&w, fields.clone()).await;

    let source = "export default function Page() { return null }";
    let mut m = manifest(&w.slug, fields);
    m["pages"] = json!([page(&sha(source), &["title", "ssn", "status"], &["status"])]);
    let (err, p) = propose(&w, &w.owner, m, vec![source.to_string()]).await;
    assert!(!err, "{p}");
    assert_eq!(check(&p, "page_sensitivity")["outcome"], "fail", "{p}");
    let changes = p["changes"].as_array().unwrap().iter().map(|c| c.as_str().unwrap().to_string()).collect::<Vec<_>>().join("\n");
    assert!(changes.contains("Award dashboard") && changes.to_lowercase().contains("social security number"), "{changes}");
    assert!(changes.contains("may not create or delete records"), "{changes}");

    let id = p["proposal_id"].as_str().unwrap().to_string();
    let (err, _) = decide(&w, &w.owner, &id, "approve").await;
    assert!(err, "the proposer may not approve it");
    let (err, done) = decide(&w, &w.admin, &id, "approve").await;
    assert!(!err, "{done}");
}

// 9. One character changes the hash, and the change list reports the page as changed.
#[tokio::test]
async fn one_changed_character_is_a_new_hash_and_a_changed_page() {
    let _turn = LOCK.lock().await;
    let w = world().await;
    live_app(&w, base_fields()).await;
    let first = "export default function Page() { return 'a' }";
    let mut m = manifest(&w.slug, base_fields());
    m["pages"] = json!([page(&sha(first), &["title"], &[])]);
    let (_, p) = propose(&w, &w.owner, m.clone(), vec![first.to_string()]).await;
    let id = p["proposal_id"].as_str().unwrap().to_string();
    let (err, done) = decide(&w, &w.owner, &id, "approve").await;
    assert!(!err, "{done}");

    let second = "export default function Page() { return 'b' }";
    assert_ne!(sha(first), sha(second));
    m["pages"] = json!([page(&sha(second), &["title"], &[])]);
    let (err, p2) = propose(&w, &w.owner, m, vec![second.to_string()]).await;
    assert!(!err, "{p2}");
    let changes = p2["changes"].as_array().unwrap().iter().map(|c| c.as_str().unwrap().to_string()).collect::<Vec<_>>().join("\n");
    assert!(changes.contains("Award dashboard") && changes.to_lowercase().contains("changed"), "{changes}");
}

#[tokio::test]
async fn a_schema_failure_stores_no_row_and_returns_every_error() {
    let _turn = LOCK.lock().await;
    let w = world().await;
    live_app(&w, base_fields()).await;
    let base_rows = proposal_rows(&w.slug);
    let mut fields = base_fields();
    fields.push(field("Bad Name", "Bad", "Text", false));
    fields.push(json!({ "name": "total", "label": "Total", "field_type": "Formula", "required": false, "ferpa_sensitive": false, "formula_expression": "{ghost}" }));
    let (err, msg) = propose(&w, &w.owner, manifest(&w.slug, fields), vec![]).await;
    assert!(err, "{msg}");
    let text = msg.as_str().unwrap();
    assert!(text.contains("schema") && text.contains("Bad Name") && text.contains("ghost"), "{text}");
    assert_eq!(proposal_rows(&w.slug), base_rows, "no proposal row was stored");
}

#[tokio::test]
async fn removing_a_field_that_has_records_is_flagged_data_loss() {
    let _turn = LOCK.lock().await;
    let w = world().await;
    live_app(&w, base_fields()).await;
    let (s, _) = rest(&w.app, "POST", &format!("/api/v1/apps/{}/records", w.slug), &w.owner, Some(json!({ "data": { "_table_id": "grants", "title": "Has data" } }))).await;
    assert_eq!(s, StatusCode::CREATED);

    let fewer = vec![field("title", "Title", "Text", false)];
    let (_, p) = propose(&w, &w.owner, manifest(&w.slug, fewer), vec![]).await;
    assert_eq!(check(&p, "data_loss")["outcome"], "fail", "{p}");
    let id = p["proposal_id"].as_str().unwrap().to_string();
    let (err, _) = decide(&w, &w.owner, &id, "approve").await;
    assert!(err, "the proposer may not approve data loss alone");
}

#[tokio::test]
async fn list_shows_pending_first_and_a_rejection_changes_nothing() {
    let _turn = LOCK.lock().await;
    let w = world().await;
    live_app(&w, base_fields()).await;
    let (v0, _) = live_version(&w).await;
    let mk = |n: &str| {
        let mut f = base_fields();
        f.push(field(n, n, "Text", false));
        manifest(&w.slug, f)
    };
    let (_, a) = propose(&w, &w.owner, mk("one"), vec![]).await;
    let (_, b) = propose(&w, &w.owner, mk("two"), vec![]).await;
    let (a, b) = (a["proposal_id"].as_str().unwrap().to_string(), b["proposal_id"].as_str().unwrap().to_string());
    let (err, _) = decide(&w, &w.owner, &a, "reject").await;
    assert!(!err);

    let (err, listed) = tool(&w.app, &w.owner, "list_proposals", json!({ "workspace_id": WORKSPACE })).await;
    assert!(!err, "{listed}");
    // The proposal that made the app is in the list too. It is approved, so it sorts last.
    let rows: Vec<&Value> = listed.as_array().unwrap().iter().filter(|r| r["app_slug"] == w.slug.as_str() && r["status"] != "Approved").collect();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0]["id"], b.as_str(), "pending first");
    assert_eq!(rows[1]["status"], "Rejected");
    let (v1, _) = live_version(&w).await;
    assert_eq!(v0, v1);
    // A decided proposal cannot be decided again.
    let (err, _) = decide(&w, &w.owner, &a, "approve").await;
    assert!(err);
}
