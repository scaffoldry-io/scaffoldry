//! MCP apps phase 6: a custom page is rendered and bounded.
//!
//! `get_page` gives the document. `page_call` is the limit on what the document may do. Each test uses its own app slug.

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::Router;
use http_body_util::BodyExt;
use scaffoldry_server::build_app_with_state;
use scaffoldry_server::service::identity::issue_test_token_and_user;
use scaffoldry_server::state::CollaboratorRecord;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tower::ServiceExt;
use uuid::Uuid;

const WORKSPACE: &str = "ws-physics-optics";
const OWNER: &str = "einstein@physics.state.edu";
const VIEWER: &str = "viewer.p6@physics.state.edu";
const OTHER: &str = "jordan.lee@state.edu";

const SOURCE: &str = "export default function Page() { return <p>AWARD-DASHBOARD-MARKER</p>; }";

struct World {
    _db: common::TestDb,
    app: Router,
    owner: String,
    viewer: String,
    other: String,
    slug: String,
}

async fn world() -> World {
    // Each test has a database of its own, so tests that start together do not race to seed the demo data.
    let db = common::TestDb::new();
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
    let app = build_app_with_state(state).expect("router");
    World {
        _db: db,
        app,
        owner: issue_test_token_and_user(OWNER),
        viewer: issue_test_token_and_user(VIEWER),
        other: issue_test_token_and_user(OTHER),
        slug: format!("page-{}", &Uuid::new_v4().to_string()[..8]),
    }
}

fn sha(text: &str) -> String {
    Sha256::digest(text.as_bytes()).iter().map(|b| format!("{b:02x}")).collect()
}

async fn rest(app: &Router, method: &str, uri: &str, token: &str, body: Option<Value>) -> (StatusCode, Value) {
    let b = Request::builder().method(method).uri(uri).header("authorization", format!("Bearer {token}"));
    let req = match body {
        Some(v) => b.header("content-type", "application/json").body(Body::from(v.to_string())).unwrap(),
        None => b.body(Body::empty()).unwrap(),
    };
    let resp = app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let content_type = resp.headers().get("content-type").and_then(|v| v.to_str().ok()).unwrap_or("").to_string();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let mut v: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    if let Some(o) = v.as_object_mut() {
        o.insert("__content_type".into(), json!(content_type));
    }
    (status, v)
}

/// Calls a tool. `Err` is the tool's own message.
async fn tool(w: &World, token: &str, name: &str, args: Value) -> Result<Value, String> {
    let payload = json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": { "name": name, "arguments": args } });
    let (_, v) = rest(&w.app, "POST", "/api/mcp", token, Some(payload)).await;
    if let Some(err) = v.get("error") {
        return Err(err["message"].as_str().unwrap_or("").to_string());
    }
    let result = &v["result"];
    if result["isError"] == true {
        return Err(result["content"][0]["text"].as_str().unwrap_or("").to_string());
    }
    Ok(result["structuredContent"].clone())
}

fn field(name: &str, t: &str) -> Value {
    json!({ "name": name, "label": name, "field_type": t, "required": false, "ferpa_sensitive": false })
}

fn custom_page(source: &str, read: &[&str], write: &[&str], create: bool, delete: bool) -> Value {
    json!({ "id": "board", "title": "Board", "kind": { "Custom": { "source_sha256": sha(source), "grants": [
        { "table_id": "grants", "read": read, "write": write, "create": create, "delete": delete }
    ]}}})
}

fn manifest(slug: &str, pages: Vec<Value>) -> Value {
    json!({
        "slug": slug, "title": "Pages", "description": "d", "organization_code": "PHYS", "department": "physics",
        "views": [{ "id": "all", "table_id": "grants", "title": "All", "view_type": "Table", "fields": [field("title", "Text")] }],
        "tables": [
            { "id": "grants", "name": "Grants", "slug": "grants", "fields": [field("title", "Text"), field("amount", "Number"), field("status", "Text")] },
            { "id": "notes", "name": "Notes", "slug": "notes", "fields": [field("body", "Text")] }
        ],
        "pages": pages
    })
}

/// Proposes the app with these pages and approves it as the owner.
async fn publish(w: &World, pages: Vec<Value>, sources: Vec<&str>) -> Result<(), String> {
    let proposed = tool(
        w,
        &w.owner,
        "propose_app_change",
        json!({ "workspace_id": WORKSPACE, "manifest": manifest(&w.slug, pages), "summary": "Pages", "page_sources": sources }),
    )
    .await?;
    let id = proposed["proposal_id"].as_str().ok_or("a proposal id")?.to_string();
    tool(w, &w.owner, "decide_proposal", json!({ "proposal_id": id, "decision": "approve", "rationale": "Reviewed", "digest": proposed["digest"] })).await?;
    Ok(())
}

async fn add_record(w: &World, title: &str, amount: i64, status: &str) -> String {
    let made = tool(
        w,
        &w.owner,
        "create_record",
        json!({ "app_slug": w.slug, "data": { "_table_id": "grants", "title": title, "amount": amount, "status": status } }),
    )
    .await
    .expect("a record");
    made["id"].as_str().expect("an id").to_string()
}

async fn page_call(w: &World, token: &str, args: Value) -> Result<Value, String> {
    let mut args = args;
    args["app_slug"] = json!(w.slug);
    args["page_id"] = json!("board");
    tool(w, token, "page_call", args).await
}

async fn record_data(w: &World, id: &str) -> Value {
    tool(w, &w.owner, "get_record", json!({ "app_slug": w.slug, "record_id": id })).await.expect("the record")["data"].clone()
}

// 1. get_page returns a document with the stored source and the policy, and nothing of the caller.
#[tokio::test]
async fn get_page_returns_the_source_and_the_policy_and_not_the_caller() {
    let w = world().await;
    publish(&w, vec![custom_page(SOURCE, &["title"], &[], false, false)], vec![SOURCE]).await.expect("published");
    add_record(&w, "Secret Title 77", 5, "open").await;

    let page = tool(&w, &w.owner, "get_page", json!({ "app_slug": w.slug, "page_id": "board" })).await.expect("get_page");
    let html = page["html"].as_str().expect("html");
    assert!(html.contains("AWARD-DASHBOARD-MARKER"), "the stored source is in the document");
    assert!(html.contains("Content-Security-Policy"), "the policy is in the document");
    assert!(html.contains("default-src 'none'") && html.contains("connect-src 'none'"));
    assert!(!html.contains("unsafe-eval"), "the policy allows no eval");
    assert!(!html.contains(OWNER) && !html.to_lowercase().contains("einstein"), "the caller is not in the document");
    assert!(!html.contains("Secret Title 77"), "no record value is in the document");
    assert!(!html.contains("Bearer") && !html.contains(&w.owner), "no token is in the document");
    for marker in ["@@META@@", "@@SOURCE@@", "@@REACT@@", "@@REACT_DOM@@", "@@SUCRASE@@", "@@BRIDGE@@", "@@LOADER@@"] {
        assert!(!html.contains(marker), "{marker} was filled");
    }
    let first = html.find("Content-Security-Policy").unwrap();
    assert!(html[..first].matches("<meta").count() == 1, "the policy is the first element in the head");
}

#[tokio::test]
async fn a_page_that_names_a_marker_is_not_filled_twice_and_a_left_angle_bracket_is_escaped() {
    let w = world().await;
    let tricky = "export default function P() { return <p>@@LOADER@@ @@BRIDGE@@</p>; }";
    publish(&w, vec![custom_page(tricky, &["title"], &[], false, false)], vec![tricky]).await.expect("published");
    let page = tool(&w, &w.owner, "get_page", json!({ "app_slug": w.slug, "page_id": "board" })).await.expect("get_page");
    let html = page["html"].as_str().unwrap();
    assert_eq!(html.matches("window.scaffoldry = {").count(), 1, "the bridge is in the document once");
    assert!(html.contains("@@LOADER@@"), "the page's own text is kept as written");
    assert!(html.contains("\\u003cp>"), "a left angle bracket in the source is escaped");
}

#[tokio::test]
async fn get_page_for_a_view_page_is_a_bad_request_and_the_rest_route_returns_json() {
    let w = world().await;
    let view = json!({ "id": "list", "title": "List", "kind": { "View": { "view_id": "all" } } });
    publish(&w, vec![view, custom_page(SOURCE, &["title"], &[], false, false)], vec![SOURCE]).await.expect("published");

    let err = tool(&w, &w.owner, "get_page", json!({ "app_slug": w.slug, "page_id": "list" })).await.unwrap_err();
    assert!(err.to_lowercase().contains("view"), "{err}");

    let (status, body) = rest(&w.app, "GET", &format!("/api/v1/apps/{}/pages/board", w.slug), &w.owner, None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(body["__content_type"].as_str().unwrap().starts_with("application/json"), "the document is inside JSON and never the response itself");
    assert!(body["html"].as_str().unwrap().contains("AWARD-DASHBOARD-MARKER"));
    let (status, _) = rest(&w.app, "GET", &format!("/api/v1/apps/{}/pages/list", w.slug), &w.owner, None).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, _) = rest(&w.app, "GET", &format!("/api/v1/apps/{}/pages/nope", w.slug), &w.owner, None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn list_pages_names_each_page_its_kind_and_its_grants() {
    let w = world().await;
    let view = json!({ "id": "list", "title": "List", "kind": { "View": { "view_id": "all" } } });
    publish(&w, vec![view, custom_page(SOURCE, &["title"], &["title"], true, false)], vec![SOURCE]).await.expect("published");
    let listed = tool(&w, &w.owner, "get_page", json!({ "app_slug": w.slug })).await.expect("the pages");
    let pages = listed["pages"].as_array().expect("pages");
    assert_eq!(pages.len(), 2);
    let board = pages.iter().find(|p| p["id"] == "board").unwrap();
    assert_eq!(board["kind"], "custom");
    assert_eq!(board["grants"][0]["table_id"], "grants");
    assert_eq!(board["grants"][0]["create"], true);
    assert_eq!(pages.iter().find(|p| p["id"] == "list").unwrap()["kind"], "view");
}

// 2. page_call list on a table with no grant is 403.
#[tokio::test]
async fn page_call_on_a_table_with_no_grant_is_forbidden() {
    let w = world().await;
    publish(&w, vec![custom_page(SOURCE, &["title"], &[], false, false)], vec![SOURCE]).await.expect("published");
    let (status, body) = rest(
        &w.app,
        "POST",
        &format!("/api/v1/apps/{}/pages/board/call", w.slug),
        &w.owner,
        Some(json!({ "op": "list", "table_id": "notes" })),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    let err = page_call(&w, &w.owner, json!({ "op": "list", "table_id": "notes" })).await.unwrap_err();
    assert!(err.contains("notes"), "{err}");
}

// 3. A page granted read: ["title"] receives id, version, and title only.
#[tokio::test]
async fn a_page_receives_only_the_fields_it_may_read() {
    let w = world().await;
    publish(&w, vec![custom_page(SOURCE, &["title"], &[], false, false)], vec![SOURCE]).await.expect("published");
    let id = add_record(&w, "Visible", 9000, "hidden-status").await;

    let listed = page_call(&w, &w.owner, json!({ "op": "list", "table_id": "grants" })).await.expect("list");
    let row = &listed["records"][0];
    let mut keys: Vec<&str> = row.as_object().unwrap().keys().map(String::as_str).collect();
    keys.sort();
    assert_eq!(keys, ["data", "id", "version"], "{row}");
    let data: Vec<&str> = row["data"].as_object().unwrap().keys().map(String::as_str).collect();
    assert_eq!(data, ["title"], "{row}");
    assert_eq!(row["data"]["title"], "Visible");
    assert!(!listed.to_string().contains("9000") && !listed.to_string().contains("hidden-status") && !listed.to_string().contains(OWNER));

    let got = page_call(&w, &w.owner, json!({ "op": "get", "table_id": "grants", "record_id": id })).await.expect("get");
    assert_eq!(got["data"], json!({ "title": "Visible" }), "{got}");
}

// 4. An update that sets a field outside write is refused and the row is unchanged.
#[tokio::test]
async fn an_update_outside_the_write_list_is_refused_and_the_row_is_unchanged() {
    let w = world().await;
    publish(&w, vec![custom_page(SOURCE, &["title", "status"], &["status"], false, false)], vec![SOURCE]).await.expect("published");
    let id = add_record(&w, "Keep", 1, "open").await;

    let err = page_call(&w, &w.owner, json!({ "op": "update", "table_id": "grants", "record_id": id, "version": 1, "data": { "title": "Changed" } })).await.unwrap_err();
    assert!(err.contains("title"), "{err}");
    let err = page_call(&w, &w.owner, json!({ "op": "update", "table_id": "grants", "record_id": id, "version": 1, "data": { "status": "closed", "amount": 99 } })).await.unwrap_err();
    assert!(err.contains("amount"), "one field outside the list refuses the whole change: {err}");
    let data = record_data(&w, &id).await;
    assert_eq!(data["title"], "Keep");
    assert_eq!(data["status"], "open", "nothing was applied");

    let saved = page_call(&w, &w.owner, json!({ "op": "update", "table_id": "grants", "record_id": id, "version": 1, "data": { "status": "closed" } })).await.expect("a granted field");
    assert_eq!(saved["data"], json!({ "status": "closed", "title": "Keep" }), "the answer is cut to what the page may read: {saved}");
    assert_eq!(record_data(&w, &id).await["status"], "closed");
}

// 5. A viewer is not widened by a page that grants write.
#[tokio::test]
async fn a_viewer_cannot_write_through_a_page_that_grants_write() {
    let w = world().await;
    publish(&w, vec![custom_page(SOURCE, &["title", "status"], &["status"], true, true)], vec![SOURCE]).await.expect("published");
    let id = add_record(&w, "Locked", 1, "open").await;

    let seen = page_call(&w, &w.viewer, json!({ "op": "list", "table_id": "grants" })).await.expect("a viewer reads through the page");
    assert_eq!(seen["records"].as_array().unwrap().len(), 1);

    for args in [
        json!({ "op": "update", "table_id": "grants", "record_id": id, "version": 1, "data": { "status": "hacked" } }),
        json!({ "op": "create", "table_id": "grants", "data": { "status": "new" } }),
        json!({ "op": "remove", "table_id": "grants", "record_id": id, "version": 1 }),
    ] {
        assert!(page_call(&w, &w.viewer, args.clone()).await.is_err(), "refused: {args}");
    }
    let data = record_data(&w, &id).await;
    assert_eq!(data["status"], "open");
    let (status, _) = rest(
        &w.app,
        "POST",
        &format!("/api/v1/apps/{}/pages/board/call", w.slug),
        &w.viewer,
        Some(json!({ "op": "update", "table_id": "grants", "record_id": id, "version": 1, "data": { "status": "hacked" } })),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn create_and_remove_need_their_own_grant_and_create_is_held_to_the_write_list() {
    let w = world().await;
    publish(&w, vec![custom_page(SOURCE, &["title", "status"], &["title", "status"], false, false)], vec![SOURCE]).await.expect("published");
    let id = add_record(&w, "Stay", 1, "open").await;
    let err = page_call(&w, &w.owner, json!({ "op": "create", "table_id": "grants", "data": { "title": "x" } })).await.unwrap_err();
    assert!(err.contains("create"), "{err}");
    let err = page_call(&w, &w.owner, json!({ "op": "remove", "table_id": "grants", "record_id": id, "version": 1 })).await.unwrap_err();
    assert!(err.contains("delete") || err.contains("remove"), "{err}");
    assert_eq!(record_data(&w, &id).await["title"], "Stay");

    // The same page with both rights.
    let w = world().await;
    publish(&w, vec![custom_page(SOURCE, &["title", "status"], &["title"], true, true)], vec![SOURCE]).await.expect("published");
    let err = page_call(&w, &w.owner, json!({ "op": "create", "table_id": "grants", "data": { "title": "x", "amount": 5 } })).await.unwrap_err();
    assert!(err.contains("amount"), "a create sets only what the write list names: {err}");
    let made = page_call(&w, &w.owner, json!({ "op": "create", "table_id": "grants", "data": { "title": "From a page" } })).await.expect("create");
    assert_eq!(made["data"], json!({ "title": "From a page" }));
    let id = made["id"].as_str().unwrap().to_string();
    page_call(&w, &w.owner, json!({ "op": "remove", "table_id": "grants", "record_id": id, "version": 1 })).await.expect("remove");
    let gone = page_call(&w, &w.owner, json!({ "op": "get", "table_id": "grants", "record_id": id })).await;
    assert!(gone.is_err(), "the record is gone");
}

#[tokio::test]
async fn page_call_on_a_view_page_or_an_unknown_page_is_refused() {
    let w = world().await;
    let view = json!({ "id": "list", "title": "List", "kind": { "View": { "view_id": "all" } } });
    publish(&w, vec![view, custom_page(SOURCE, &["title"], &[], false, false)], vec![SOURCE]).await.expect("published");
    for page in ["list", "ghost"] {
        let err = tool(&w, &w.owner, "page_call", json!({ "app_slug": w.slug, "page_id": page, "op": "list", "table_id": "grants" })).await.unwrap_err();
        assert!(!err.is_empty());
    }
    let err = page_call(&w, &w.owner, json!({ "op": "explode", "table_id": "grants" })).await.unwrap_err();
    assert!(err.contains("op"), "{err}");
}

// 12. A setting is the person's own, and 64 KB is the most one person keeps for a page.
#[tokio::test]
async fn a_setting_is_the_persons_own_and_is_bounded() {
    let w = world().await;
    publish(&w, vec![custom_page(SOURCE, &["title"], &[], false, false)], vec![SOURCE]).await.expect("published");

    page_call(&w, &w.owner, json!({ "op": "settings_set", "key": "layout", "value": { "columns": 3 } })).await.expect("set");
    let mine = page_call(&w, &w.owner, json!({ "op": "settings_get", "key": "layout" })).await.expect("get");
    assert_eq!(mine["value"], json!({ "columns": 3 }));
    let theirs = page_call(&w, &w.other, json!({ "op": "settings_get", "key": "layout" })).await.expect("get");
    assert_eq!(theirs["value"], Value::Null, "another person sees nothing of it");
    page_call(&w, &w.other, json!({ "op": "settings_set", "key": "layout", "value": "theirs" })).await.expect("set");
    assert_eq!(page_call(&w, &w.owner, json!({ "op": "settings_get", "key": "layout" })).await.unwrap()["value"], json!({ "columns": 3 }), "one person's write does not touch another's");

    let big = "x".repeat(65 * 1024);
    let err = page_call(&w, &w.owner, json!({ "op": "settings_set", "key": "big", "value": big })).await.unwrap_err();
    assert!(err.contains("64"), "{err}");
    // Two values that fit alone and not together.
    let half = "y".repeat(40 * 1024);
    page_call(&w, &w.owner, json!({ "op": "settings_set", "key": "a", "value": half })).await.expect("first half");
    let err = page_call(&w, &w.owner, json!({ "op": "settings_set", "key": "b", "value": half })).await.unwrap_err();
    assert!(err.contains("64"), "the limit is per person per page, not per key: {err}");
    // A key is replaced, not added to.
    page_call(&w, &w.owner, json!({ "op": "settings_set", "key": "a", "value": half })).await.expect("the same key again");
}

// 9 and 10. Bundles: size and declared libraries.
#[tokio::test]
async fn a_bundle_must_declare_libraries_on_the_license_list() {
    let w = world().await;
    let bundle = "(function(){ document.getElementById('root').textContent = 'hi'; })();";
    let page = |deps: Value| {
        let mut p = custom_page(bundle, &["title"], &[], false, false);
        p["kind"]["Custom"]["format"] = json!("bundle");
        p["kind"]["Custom"]["dependencies"] = deps;
        p
    };
    let propose = |deps: Value| {
        let (w, p) = (&w, page(deps));
        async move {
            tool(w, &w.owner, "propose_app_change", json!({ "workspace_id": WORKSPACE, "manifest": manifest(&w.slug, vec![p]), "summary": "Bundle", "page_sources": [bundle] })).await
        }
    };
    let outcome = |result: &Value| result["checks"].as_array().unwrap().iter().find(|c| c["name"] == "page_dependencies").cloned().unwrap_or_else(|| panic!("no page_dependencies check in {result}"));

    let ok = propose(json!([{ "name": "chart.js", "version": "4.4.0", "license": "MIT" }])).await.expect("proposed");
    assert_eq!(outcome(&ok)["outcome"], "pass", "{ok}");

    let bad_license = propose(json!([{ "name": "mapthing", "version": "1.0.0", "license": "AGPL-3.0" }])).await.expect("proposed");
    let check = outcome(&bad_license);
    assert_eq!(check["outcome"], "fail", "{bad_license}");
    assert!(check.to_string().contains("AGPL-3.0") && check.to_string().contains("mapthing"));

    let none = propose(json!([])).await.expect("proposed");
    let check = outcome(&none);
    assert_eq!(check["outcome"], "fail", "a bundle that declares nothing fails: {none}");

    // A failed check is a flagged check: the proposer cannot approve their own bundle.
    let id = bad_license["proposal_id"].as_str().unwrap().to_string();
    let err = tool(&w, &w.owner, "decide_proposal", json!({ "proposal_id": id, "decision": "approve", "rationale": "x", "digest": "0".repeat(64) })).await.unwrap_err();
    assert!(!err.is_empty(), "a flagged proposal needs a second person");
}

#[tokio::test]
async fn a_bundle_over_two_megabytes_is_refused_and_a_module_stays_at_256_kb() {
    let w = world().await;
    let make = |source: &str, format: &str| {
        let mut p = custom_page(source, &["title"], &[], false, false);
        p["kind"]["Custom"]["format"] = json!(format);
        p["kind"]["Custom"]["dependencies"] = json!([{ "name": "x", "version": "1", "license": "MIT" }]);
        p
    };
    let big = format!("/*{}*/", "a".repeat(2 * 1024 * 1024 + 10));
    let err = tool(&w, &w.owner, "propose_app_change", json!({ "workspace_id": WORKSPACE, "manifest": manifest(&w.slug, vec![make(&big, "bundle")]), "summary": "Big", "page_sources": [big] })).await.unwrap_err();
    assert!(err.contains("page_source") && err.contains("2097152"), "{err}");

    let fits = format!("/*{}*/", "a".repeat(1024 * 1024));
    let ok = tool(&w, &w.owner, "propose_app_change", json!({ "workspace_id": WORKSPACE, "manifest": manifest(&w.slug, vec![make(&fits, "bundle")]), "summary": "Fits", "page_sources": [fits] })).await;
    assert!(ok.is_ok(), "a 1 MB bundle is accepted: {ok:?}");

    let module_too_big = format!("export default function P(){{return null}} /*{}*/", "a".repeat(300 * 1024));
    let err = tool(&w, &w.owner, "propose_app_change", json!({ "workspace_id": WORKSPACE, "manifest": manifest(&w.slug, vec![make(&module_too_big, "module")]), "summary": "Big", "page_sources": [module_too_big] })).await.unwrap_err();
    assert!(err.contains("262144"), "{err}");
}

#[tokio::test]
async fn a_bundle_is_rendered_as_a_bundle() {
    let w = world().await;
    let bundle = "(function(){ document.getElementById('root').textContent = 'BUNDLE-MARKER'; })();";
    let mut page = custom_page(bundle, &["title"], &[], false, false);
    page["kind"]["Custom"]["format"] = json!("bundle");
    page["kind"]["Custom"]["dependencies"] = json!([{ "name": "mini", "version": "1.0.0", "license": "MIT" }]);
    publish(&w, vec![page], vec![bundle]).await.expect("published");
    let html = tool(&w, &w.owner, "get_page", json!({ "app_slug": w.slug, "page_id": "board" })).await.expect("get_page")["html"].as_str().unwrap().to_string();
    assert!(html.contains("BUNDLE-MARKER"));
    assert!(html.contains("\"format\":\"bundle\""), "the loader is told to run it as written");
    let listed = tool(&w, &w.owner, "get_page", json!({ "app_slug": w.slug })).await.unwrap();
    assert_eq!(listed["pages"][0]["format"], "bundle");
    assert_eq!(listed["pages"][0]["dependencies"][0]["name"], "mini");
}

// A filter or a sort on a field the page may not read would let it find the value by asking.
#[tokio::test]
async fn a_filter_or_sort_on_a_field_the_page_may_not_read_is_refused() {
    let w = world().await;
    publish(&w, vec![custom_page(SOURCE, &["title"], &[], false, false)], vec![SOURCE]).await.expect("published");
    add_record(&w, "A", 100, "open").await;
    let filter = |field: &str| json!({ "conjunction": "And", "clauses": [{ "id": "c1", "field_name": field, "operator": "GreaterThan", "value": "50" }] });
    let err = page_call(&w, &w.owner, json!({ "op": "list", "table_id": "grants", "filter": filter("amount") })).await.unwrap_err();
    assert!(err.contains("amount"), "{err}");
    let nested = json!({ "conjunction": "And", "clauses": [], "groups": [filter("amount")] });
    assert!(page_call(&w, &w.owner, json!({ "op": "list", "table_id": "grants", "filter": nested })).await.is_err(), "a nested group is read too");
    let err = page_call(&w, &w.owner, json!({ "op": "list", "table_id": "grants", "sort": { "field_name": "amount", "direction": "Asc" } })).await.unwrap_err();
    assert!(err.contains("amount"), "{err}");
    let ok = page_call(&w, &w.owner, json!({ "op": "list", "table_id": "grants", "filter": filter("title"), "sort": { "field_name": "title", "direction": "Asc" } })).await;
    assert!(ok.is_ok(), "a field the page may read can be filtered and sorted: {ok:?}");
}

// A grant names a table. A record id from another table is not reachable through it.
#[tokio::test]
async fn a_record_of_another_table_is_not_reachable_by_its_id() {
    let w = world().await;
    publish(&w, vec![custom_page(SOURCE, &["title", "status"], &["status"], false, true)], vec![SOURCE]).await.expect("published");
    let note = tool(&w, &w.owner, "create_record", json!({ "app_slug": w.slug, "data": { "_table_id": "notes", "body": "private" } })).await.expect("a note")["id"].as_str().unwrap().to_string();
    for args in [
        json!({ "op": "get", "table_id": "grants", "record_id": note }),
        json!({ "op": "update", "table_id": "grants", "record_id": note, "version": 1, "data": { "status": "x" } }),
        json!({ "op": "remove", "table_id": "grants", "record_id": note, "version": 1 }),
    ] {
        let err = page_call(&w, &w.owner, args.clone()).await.unwrap_err();
        assert!(!err.contains("private"), "{err}");
    }
    assert_eq!(record_data(&w, &note).await["body"], "private", "the note is untouched");
}
