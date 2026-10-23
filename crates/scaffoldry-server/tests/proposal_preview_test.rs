//! Proposal preview phase 2: open the proposed app, read-only.
//!
//! A proposal can be opened before it is live. The preview token reaches one app, the proposed one, for 30 minutes.
//! It reads live records through the stricter of the live rules and the proposed rules. It writes nothing.
//! NIST SP 800-53 CM-3, CM-3(2) and CM-4.

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::Router;
use http_body_util::BodyExt;
use scaffoldry_server::build_app_with_state;
use scaffoldry_server::service::identity::issue_test_token_and_user;
use scaffoldry_server::state::{CollaboratorRecord, ServerState};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
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

async fn preview(w: &World, token: &str, proposal: &Value) -> (StatusCode, Value) {
    rest(&w.app, "POST", &format!("/api/v1/proposals/{}/preview", proposal["proposal_id"].as_str().unwrap()), token, None).await
}

/// The preview token and the slug it reaches.
async fn opened(w: &World, proposal: &Value) -> (String, String) {
    let (status, body) = preview(w, &w.owner, proposal).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    (body["token"].as_str().expect("a token").to_string(), body["app_slug"].as_str().expect("an app slug").to_string())
}

async fn titles(w: &World, token: &str, slug: &str) -> Vec<String> {
    let (s, body) = rest(&w.app, "GET", &format!("/api/v1/apps/{slug}/tables/cases/records"), token, None).await;
    assert_eq!(s, StatusCode::OK, "{body}");
    let mut out: Vec<String> = body["records"].as_array().unwrap().iter().map(|r| r["data"]["title"].as_str().unwrap_or("").to_string()).collect();
    out.sort();
    out
}

async fn schema_fields(w: &World, token: &str, slug: &str) -> Vec<String> {
    let (s, body) = rest(&w.app, "GET", &format!("/api/v1/apps/{slug}/schema"), token, None).await;
    assert_eq!(s, StatusCode::OK, "{body}");
    body["tables"][0]["fields"].as_array().unwrap().iter().map(|f| f["name"].as_str().unwrap_or("").to_string()).collect()
}

// 1. The proposer, a person who may decide, and a Platform Admin can open a preview. Another member gets 403.
#[tokio::test]
async fn the_proposer_and_the_people_who_may_decide_can_preview_and_others_cannot() {
    let w = world().await;
    live(&w).await;
    let p = propose(&w, &["title", "note", "extra"], vec![hide("secret")]).await;
    for (who, token) in [("the proposer", &w.owner), ("a person who may decide", &w.second), ("a Platform Admin", &w.platform)] {
        let (status, body) = preview(&w, token, &p).await;
        assert_eq!(status, StatusCode::OK, "{who}: {body}");
        assert_eq!(body["read_only"], true);
        assert_eq!(body["digest"], p["digest"], "the preview names the digest it shows");
    }
    let (status, body) = preview(&w, &w.viewer, &p).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "another workspace member: {body}");
}

// 2. A preview of a proposal that adds a field shows it, empty. One that removes a field hides it.
#[tokio::test]
async fn a_added_field_shows_empty_and_a_removed_field_is_hidden() {
    let w = world().await;
    live(&w).await;

    let added = propose(&w, &["title", "note", "extra"], vec![hide("secret")]).await;
    let (token, shadow) = opened(&w, &added).await;
    assert!(schema_fields(&w, &token, &shadow).await.contains(&"extra".to_string()), "the new field is in the proposed schema");
    let (_, body) = rest(&w.app, "GET", &format!("/api/v1/apps/{shadow}/tables/cases/records"), &token, None).await;
    let rows = body["records"].as_array().unwrap();
    assert_eq!(rows.len(), 1, "the secret row stays hidden: {body}");
    assert!(rows[0]["data"]["extra"].is_null() || rows[0]["data"].get("extra").is_none(), "the new field holds no value: {body}");
    assert_eq!(rows[0]["data"]["note"], "note of plain", "an existing field keeps its value");

    let removed = propose(&w, &["title"], vec![hide("secret")]).await;
    let (token, shadow) = opened(&w, &removed).await;
    assert!(!schema_fields(&w, &token, &shadow).await.contains(&"note".to_string()), "the removed field is not in the proposed schema");
    let (_, body) = rest(&w.app, "GET", &format!("/api/v1/apps/{shadow}/tables/cases/records"), &token, None).await;
    assert!(body["records"][0]["data"].get("note").is_none(), "the removed field is hidden: {body}");

    // The live app is untouched by either.
    assert_eq!(schema_fields(&w, &w.owner, &w.slug).await, vec!["title", "note"]);
}

// 3. A preview shows no row the person could not see live, and applies a stricter proposed rule too.
#[tokio::test]
async fn a_preview_applies_the_stricter_of_the_live_and_the_proposed_rules() {
    let w = world().await;
    live(&w).await;
    assert_eq!(titles(&w, &w.owner, &w.slug).await, vec!["plain"], "live, the rule hides the secret row");

    // The proposal drops the rule. The preview still hides what the live rule hides.
    let loosened = propose(&w, &["title", "note"], vec![]).await;
    let (token, shadow) = opened(&w, &loosened).await;
    assert_eq!(titles(&w, &token, &shadow).await, vec!["plain"]);

    // The proposal hides another row. The preview hides it too.
    let stricter = propose(&w, &["title", "note"], vec![hide("plain")]).await;
    let (token, shadow) = opened(&w, &stricter).await;
    assert!(titles(&w, &token, &shadow).await.is_empty(), "both rules apply");
}

// 4. Every write under a preview token is refused, by REST and by tool, and no record changes.
#[tokio::test]
async fn every_write_under_a_preview_is_refused_and_nothing_changes() {
    let w = world().await;
    live(&w).await;
    let p = propose(&w, &["title", "note", "extra"], vec![hide("secret")]).await;
    let (token, shadow) = opened(&w, &p).await;

    let (status, body) = rest(&w.app, "POST", &format!("/api/v1/apps/{shadow}/records"), &token, Some(json!({ "data": { "_table_id": "cases", "title": "new" } }))).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert_eq!(body["code"], "preview_read_only", "{body}");
    let err = tool(&w, &token, "create_record", json!({ "app_slug": shadow, "data": { "_table_id": "cases", "title": "new" } })).await.unwrap_err();
    assert!(err.to_lowercase().contains("read-only") || err.to_lowercase().contains("preview"), "{err}");
    let (status, body) = rest(&w.app, "POST", &format!("/api/v1/proposals/{}/decision", p["proposal_id"].as_str().unwrap()), &token, Some(json!({ "decision": "approve", "rationale": "x", "digest": p["digest"] }))).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "a preview cannot approve what it shows: {body}");

    assert_eq!(titles(&w, &w.owner, &w.slug).await, vec!["plain"], "the live rows are unchanged");
    let (_, live_rows) = rest(&w.app, "GET", &format!("/api/v1/apps/{}/tables/cases/records", w.slug), &w.platform, None).await;
    assert_eq!(live_rows["records"].as_array().unwrap().len(), 2, "no record was added");
}

// 5. The token stops working when the proposal is decided. It lasts 30 minutes.
#[tokio::test]
async fn a_preview_ends_when_the_proposal_is_decided() {
    let w = world().await;
    live(&w).await;
    let p = propose(&w, &["title", "note", "extra"], vec![hide("secret")]).await;
    let (status, body) = preview(&w, &w.owner, &p).await;
    assert_eq!(status, StatusCode::OK);
    let token = body["token"].as_str().unwrap().to_string();
    let shadow = body["app_slug"].as_str().unwrap().to_string();

    let expires = chrono::DateTime::parse_from_rfc3339(body["expires_at"].as_str().unwrap()).unwrap();
    let minutes = (expires.with_timezone(&chrono::Utc) - chrono::Utc::now()).num_minutes();
    assert!((29..=30).contains(&minutes), "a preview lasts 30 minutes: {minutes}");

    let (status, _) = rest(&w.app, "GET", &format!("/api/v1/apps/{shadow}/schema"), &token, None).await;
    assert_eq!(status, StatusCode::OK, "it works while the proposal is pending");

    tool(&w, &w.owner, "decide_proposal", json!({ "proposal_id": p["proposal_id"], "decision": "approve", "rationale": "Reviewed", "digest": p["digest"] })).await.expect("approved");
    let (status, _) = rest(&w.app, "GET", &format!("/api/v1/apps/{shadow}/schema"), &token, None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "an approved proposal ends its previews");

    // A rejected one does too, and nothing can be opened on a decided proposal.
    let other = propose(&w, &["title", "note", "more"], vec![hide("secret")]).await;
    let (token, shadow) = opened(&w, &other).await;
    tool(&w, &w.owner, "decide_proposal", json!({ "proposal_id": other["proposal_id"], "decision": "reject", "rationale": "No" })).await.expect("rejected");
    let (status, _) = rest(&w.app, "GET", &format!("/api/v1/apps/{shadow}/schema"), &token, None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let (status, _) = preview(&w, &w.owner, &other).await;
    assert_eq!(status, StatusCode::CONFLICT, "a decided proposal cannot be previewed");
}

// 6. Opening a preview writes one ledger entry holding the proposal, the digest and the person.
#[tokio::test]
async fn opening_a_preview_writes_one_ledger_entry() {
    let w = world().await;
    live(&w).await;
    let p = propose(&w, &["title", "note", "extra"], vec![hide("secret")]).await;
    let count = |body: &Value| body["entries"].as_array().unwrap().iter().filter(|e| e["decision_type"] == "ProposalPreviewed").count();
    let (_, before) = rest(&w.app, "GET", "/api/v1/governance/ledger", &w.platform, None).await;
    opened(&w, &p).await;
    let (_, after) = rest(&w.app, "GET", "/api/v1/governance/ledger", &w.platform, None).await;
    assert_eq!(count(&after), count(&before) + 1);
    let entry = after["entries"].as_array().unwrap().iter().find(|e| e["decision_type"] == "ProposalPreviewed").expect("an entry");
    assert_eq!(entry["principal"], OWNER);
    assert!(entry["rationale"].as_str().unwrap().contains(p["proposal_id"].as_str().unwrap()), "{entry}");
    // The entry holds the hash of its payload, which names the proposal, the digest and the person.
    let payload = json!({ "proposal_id": p["proposal_id"], "digest": p["digest"], "previewer": OWNER, "app_slug": w.slug });
    assert_eq!(entry["payload_hash"], scaffoldry_core::LedgerEntry::compute_payload_hash(&payload), "{entry}");
}

// The shadow app is not an app: it is in no list, and a second instance serves the same token.
#[tokio::test]
async fn the_shadow_app_is_in_no_list_and_another_instance_serves_the_token() {
    let w = world().await;
    live(&w).await;
    let p = propose(&w, &["title", "note", "extra"], vec![hide("secret")]).await;
    let (token, shadow) = opened(&w, &p).await;

    let listed = tool(&w, &w.owner, "list_apps", json!({ "workspace_id": WORKSPACE })).await.unwrap().to_string();
    assert!(!listed.contains(&shadow) && !listed.contains("~preview~"), "no preview in the app list: {listed}");

    let b = w._db.second_app();
    let (status, body) = rest(&b, "GET", &format!("/api/v1/apps/{shadow}/tables/cases/records"), &token, None).await;
    assert_eq!(status, StatusCode::OK, "the second instance builds the preview from the proposal: {body}");
}

const PAGE_SOURCE: &str = "export default function Page() { return <p>PROPOSED-PAGE-MARKER</p>; }";

fn sha(text: &str) -> String {
    Sha256::digest(text.as_bytes()).iter().map(|b| format!("{b:02x}")).collect()
}

// 4 (pages). A page in the proposal renders from the proposal's source. Its reads go through the same rules and its
// writes are refused.
#[tokio::test]
async fn a_proposed_page_renders_from_the_proposal_and_its_writes_are_refused() {
    let w = world().await;
    live(&w).await;
    let mut m = manifest(&w.slug, &["title", "note"], vec![hide("secret")]);
    m["pages"] = json!([{ "id": "board", "title": "Board", "kind": { "Custom": { "source_sha256": sha(PAGE_SOURCE), "grants": [
        { "table_id": "cases", "read": ["title"], "write": ["title"], "create": true, "delete": false }
    ]}}}]);
    let p = tool(&w, &w.owner, "propose_app_change", json!({ "workspace_id": WORKSPACE, "manifest": m, "summary": "A page", "page_sources": [PAGE_SOURCE] })).await.expect("proposed");
    let (token, shadow) = opened(&w, &p).await;

    let live_page = tool(&w, &w.owner, "get_page", json!({ "app_slug": w.slug, "page_id": "board" })).await;
    assert!(live_page.is_err(), "the page is not live");
    let page = tool(&w, &token, "get_page", json!({ "app_slug": shadow, "page_id": "board" })).await.expect("the preview renders the page");
    assert!(page["html"].as_str().unwrap().contains("PROPOSED-PAGE-MARKER"), "the proposal's source is in the document");

    let listed = tool(&w, &token, "page_call", json!({ "app_slug": shadow, "page_id": "board", "op": "list", "table_id": "cases" })).await.expect("a read");
    let rows = listed["records"].as_array().unwrap();
    assert_eq!(rows.len(), 1, "the live rule still hides the secret row: {listed}");
    assert_eq!(rows[0]["data"]["title"], "plain");

    let err = tool(&w, &token, "page_call", json!({ "app_slug": shadow, "page_id": "board", "op": "create", "table_id": "cases", "data": { "title": "new" } })).await.unwrap_err();
    assert!(err.to_lowercase().contains("read-only") || err.to_lowercase().contains("preview"), "{err}");
    assert_eq!(titles(&w, &w.owner, &w.slug).await, vec!["plain"], "nothing was created");
}

// 7. The tool opens the preview and returns the app as proposed and each custom page rendered.
#[tokio::test]
async fn the_tool_returns_the_proposed_app_and_its_rendered_pages() {
    let w = world().await;
    live(&w).await;
    let mut m = manifest(&w.slug, &["title", "note", "extra"], vec![hide("secret")]);
    m["pages"] = json!([{ "id": "board", "title": "Board", "kind": { "Custom": { "source_sha256": sha(PAGE_SOURCE), "grants": [
        { "table_id": "cases", "read": ["title"], "write": [], "create": false, "delete": false }
    ]}}}]);
    let p = tool(&w, &w.owner, "propose_app_change", json!({ "workspace_id": WORKSPACE, "manifest": m, "summary": "A page", "page_sources": [PAGE_SOURCE] })).await.expect("proposed");
    let shown = tool(&w, &w.owner, "preview_proposal", json!({ "proposal_id": p["proposal_id"] })).await.expect("previewed");
    assert!(shown["token"].is_string() && shown["app_slug"].as_str().unwrap().contains("~preview~"), "{shown}");
    assert!(shown["app"].to_string().contains("extra"), "the app as proposed: {shown}");
    assert!(shown["pages"][0]["html"].as_str().unwrap().contains("PROPOSED-PAGE-MARKER"), "the rendered page: {shown}");
    let err = tool(&w, &w.viewer, "preview_proposal", json!({ "proposal_id": p["proposal_id"] })).await.unwrap_err();
    assert!(err.to_lowercase().contains("forbidden") || err.to_lowercase().contains("not"), "{err}");
}
