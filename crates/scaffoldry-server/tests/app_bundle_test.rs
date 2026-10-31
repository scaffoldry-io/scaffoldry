//! Import and export, phase 3: app bundles.
//!
//! A bundle is one JSON document that holds an app's definition: its manifest, its process rules, the source of its custom
//! pages, and optionally some sample records. It never holds a secret, a token, a member, a comment, history or an attachment.
//! Importing one creates a proposal. It never creates a live app. NIST SP 800-53 CM-2, CM-3 and SA-10.

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::Router;
use http_body_util::BodyExt;
use scaffoldry_server::build_app_with_state;
use scaffoldry_server::service::identity::issue_test_token_and_user;
use scaffoldry_server::state::{CollaboratorRecord, ServerState, WorkspaceRecord};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tower::ServiceExt;

const WORKSPACE: &str = "ws-physics-optics";
const TARGET: &str = "ws-bundle-target";
const OWNER: &str = "einstein@physics.state.edu";
const PAGE: &str = "export default function Page() { return <p>BUNDLE-PAGE</p>; }";

struct World {
    db: common::TestDb,
    app: Router,
    owner: String,
    slug: String,
    tag: String,
}

fn sha(text: &str) -> String {
    Sha256::digest(text.as_bytes()).iter().map(|b| format!("{b:02x}")).collect()
}

/// The same document with its keys in sorted order at every depth.
fn sorted(v: &Value) -> Value {
    match v {
        Value::Object(o) => {
            let mut keys: Vec<&String> = o.keys().collect();
            keys.sort();
            Value::Object(keys.into_iter().map(|k| (k.clone(), sorted(&o[k]))).collect())
        }
        Value::Array(a) => Value::Array(a.iter().map(sorted).collect()),
        other => other.clone(),
    }
}

/// The same document with its keys in the opposite order at every depth.
fn reversed(v: &Value) -> Value {
    match v {
        Value::Object(o) => {
            let mut keys: Vec<&String> = o.keys().collect();
            keys.sort();
            Value::Object(keys.into_iter().rev().map(|k| (k.clone(), reversed(&o[k]))).collect())
        }
        Value::Array(a) => Value::Array(a.iter().map(reversed).collect()),
        other => other.clone(),
    }
}

/// The hash a bundle carries: SHA-256 of the document without its own `sha256`, as compact JSON with the keys sorted.
fn seal(mut bundle: Value) -> Value {
    bundle.as_object_mut().expect("a bundle is an object").remove("sha256");
    let hash = sha(&serde_json::to_string(&sorted(&bundle)).unwrap());
    bundle["sha256"] = json!(hash);
    bundle
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

async fn tool(w: &World, name: &str, args: Value) -> Result<Value, String> {
    let payload = json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": { "name": name, "arguments": args } });
    let (_, v) = rest(&w.app, "POST", "/api/mcp", &w.owner, Some(payload)).await;
    if let Some(err) = v.get("error") {
        return Err(err["message"].as_str().unwrap_or("").to_string());
    }
    if v["result"]["isError"] == true {
        return Err(v["result"]["content"][0]["text"].as_str().unwrap_or("").to_string());
    }
    Ok(v["result"]["structuredContent"].clone())
}

fn member(state: &ServerState, workspace: &str, eppn: &str, role: &str) {
    let record = CollaboratorRecord {
        id: uuid::Uuid::new_v4().to_string(),
        workspace_id: workspace.to_string(),
        eppn: eppn.to_string(),
        name: eppn.to_string(),
        role: role.to_string(),
        scoped_affiliation: "faculty".to_string(),
        department: "physics".to_string(),
        added_at: String::new(),
        app_slug: String::new(),
    };
    state.repository.upsert_collaborator(&record).expect("a member");
    state.collaborators.write().unwrap().entry(workspace.to_string()).or_default().push(record);
}

async fn world() -> World {
    let db = common::TestDb::new();
    let state = db.state();
    state.seed_demo().expect("seed");
    // A second workspace of the same owner, for the import.
    let mut ws: WorkspaceRecord = state.workspaces.read().unwrap().get(WORKSPACE).cloned().expect("the demo workspace");
    ws.id = TARGET.to_string();
    ws.name = "Bundle target".to_string();
    ws.code = "BTGT".to_string();
    state.repository.upsert_workspace(&ws).expect("a workspace");
    state.workspaces.write().unwrap().insert(TARGET.to_string(), ws);
    member(&state, TARGET, OWNER, "owner");
    let tag = uuid::Uuid::new_v4().to_string()[..8].to_string();
    let app = build_app_with_state(state).expect("router");
    World { db, app, owner: issue_test_token_and_user(OWNER), slug: format!("bundle-src-{tag}"), tag }
}

fn field(name: &str) -> Value {
    json!({ "name": name, "label": name, "field_type": "Text", "required": false, "ferpa_sensitive": false })
}

/// The source app: two tables, a custom page, a view.
fn manifest(slug: &str) -> Value {
    json!({
        "slug": slug, "title": "Bundle source", "description": "An app to copy", "organization_code": "PHYS", "department": "physics",
        "workspace_id": WORKSPACE,
        "views": [{ "id": "all", "table_id": "cases", "title": "All cases", "view_type": "Table", "fields": [field("title")] }],
        "tables": [
            { "id": "cases", "name": "Cases", "slug": "cases", "fields": [field("title"), field("note")] },
            { "id": "people", "name": "People", "slug": "people", "fields": [field("name")] },
        ],
        "pages": [{ "id": "board", "title": "Board", "kind": { "Custom": { "source_sha256": sha(PAGE), "grants": [
            { "table_id": "cases", "read": ["title"], "write": [], "create": false, "delete": false }
        ]}}}]
    })
}

fn rule(id: &str) -> Value {
    json!({
        "id": id, "name": format!("Rule {id}"), "enabled": true, "trigger": "RecordCreated",
        "predicates": [{ "field_name": "title", "operator": "Equals", "expected_value": "urgent" }],
        "steps": [{ "id": "s1", "when": [], "kind": { "Service": { "action": { "UpdateRecordStatus": { "new_status": "Open" } } } } }]
    })
}

/// Puts the source app live, with a rule and three records.
async fn live(w: &World) {
    live_as(w, &w.slug.clone(), &format!("urgent-{}", w.tag)).await;
}

async fn live_as(w: &World, slug: &str, rule_id: &str) {
    live_manifest(w, manifest(slug), rule_id).await;
}

/// Puts this manifest live with a rule, and adds three records to its `cases` table.
async fn live_manifest(w: &World, manifest: Value, rule_id: &str) {
    let slug = manifest["slug"].as_str().expect("a slug").to_string();
    let p = tool(w, "propose_app_change", json!({
        "workspace_id": WORKSPACE, "manifest": manifest, "rules": [rule(rule_id)],
        "summary": "Set up the source app", "page_sources": [PAGE],
    }))
    .await
    .expect("proposed");
    tool(w, "decide_proposal", json!({ "proposal_id": p["proposal_id"], "decision": "approve", "rationale": "Set up", "digest": p["digest"] })).await.expect("live");
    for title in ["one", "two", "three"] {
        tool(w, "create_record", json!({ "app_slug": slug, "data": { "_table_id": "cases", "title": title, "note": format!("note {title}") } })).await.expect("record");
    }
}

async fn export(w: &World, sample: bool) -> (StatusCode, Value) {
    rest(&w.app, "POST", &format!("/api/v1/apps/{}/bundle", w.slug), &w.owner, Some(json!({ "sample_data": sample }))).await
}

async fn import(w: &World, bundle: Value, extra: Value) -> (StatusCode, Value) {
    let mut body = json!({ "bundle": bundle });
    if let Some(o) = extra.as_object() {
        for (k, v) in o {
            body[k] = v.clone();
        }
    }
    rest(&w.app, "POST", &format!("/api/v1/workspaces/{TARGET}/bundle-import"), &w.owner, Some(body)).await
}

fn proposal_count(w: &World) -> i64 {
    w.db.run(|c| c.query_one("SELECT COUNT(*) FROM app_proposals", &[]).unwrap().get(0))
}

fn walk(v: &Value, f: &mut impl FnMut(&str, &Value)) {
    match v {
        Value::Object(o) => {
            for (k, child) in o {
                f(k, child);
                walk(child, f);
            }
        }
        Value::Array(a) => a.iter().for_each(|c| walk(c, f)),
        _ => {}
    }
}

// 1. Export then import into another workspace yields a proposal whose change list matches the source app.
#[tokio::test]
async fn export_then_import_yields_a_proposal_that_matches_the_source_app() {
    let w = world().await;
    live(&w).await;
    let (s, bundle) = export(&w, false).await;
    assert_eq!(s, StatusCode::OK, "{bundle}");
    assert_eq!(bundle["bundle_version"], 1);

    let before = proposal_count(&w);
    let (s, p) = import(&w, bundle.clone(), json!({})).await;
    assert_eq!(s, StatusCode::CREATED, "{p}");
    assert_eq!(proposal_count(&w), before + 1, "one proposal, and nothing else");
    assert_eq!(p["workspace_id"], TARGET, "{p}");

    // The change list describes the source app: its title, its two tables, its page and its rule.
    let source = &bundle["manifest"];
    let sentence = format!("A new app \"{}\" with {} table(s) and {} page(s).", source["title"].as_str().unwrap(), source["tables"].as_array().unwrap().len(), source["pages"].as_array().unwrap().len());
    let changes = p["changes"].to_string();
    assert!(p["changes"].as_array().unwrap().iter().any(|c| *c == json!(sentence)), "the change list says {sentence}: {changes}");
    assert!(changes.contains("Board") && changes.contains("Rule"), "and names its page and its rule: {changes}");
    // The stored proposal holds the source app's tables and fields, whatever slug it was given.
    let stored: Value = w.db.run({
        let id = uuid::Uuid::parse_str(p["proposal_id"].as_str().unwrap()).unwrap();
        move |c| c.query_one("SELECT manifest FROM app_proposals WHERE id = $1", &[&id]).unwrap().get(0)
    });
    assert_eq!(stored["tables"], source["tables"], "the proposal holds the tables of the source app");
    assert_eq!(stored["pages"], source["pages"], "and its pages");
    assert_eq!(stored["workspace_id"], TARGET, "in the workspace it was imported into");
    // It is a proposal. No app of that name is live in the target workspace.
    let new_slug = p["app_slug"].as_str().expect("an app slug").to_string();
    let (s, _) = rest(&w.app, "GET", &format!("/api/v1/apps/{new_slug}"), &w.owner, None).await;
    assert_eq!(s, StatusCode::NOT_FOUND, "importing a bundle makes no live app");
    let failed: Vec<&Value> = p["checks"].as_array().unwrap().iter().filter(|c| c["status"] == "fail").collect();
    assert!(failed.is_empty(), "no check fails for a bundle exported from a live app: {failed:?}");
}

// 2. A bundle with a changed byte fails the hash check and creates nothing.
#[tokio::test]
async fn a_bundle_with_a_changed_byte_fails_the_hash_check_and_creates_nothing() {
    let w = world().await;
    live(&w).await;
    let (_, mut bundle) = export(&w, false).await;
    bundle["manifest"]["title"] = json!("Bundle sourcf");
    let before = proposal_count(&w);
    let (s, body) = import(&w, bundle, json!({})).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{body}");
    assert!(body["error"].as_str().unwrap_or("").contains("damaged"), "the refusal says the bundle is damaged: {body}");
    assert_eq!(proposal_count(&w), before, "nothing was created");
}

// 3. A bundle holds no secret, token, member, comment or history. A scan of the output proves it.
#[tokio::test]
async fn a_bundle_holds_no_secret_token_member_comment_or_history() {
    let w = world().await;
    live(&w).await;
    let member_address = format!("bundle-member-{}@state.edu", w.tag);
    let state = w.db.state();
    member(&state, WORKSPACE, &member_address, "viewer");
    let (_, rec) = rest(&w.app, "GET", &format!("/api/v1/apps/{}/tables/cases/records?limit=1", w.slug), &w.owner, None).await;
    let record_id = rec["records"][0]["id"].as_str().expect("a record").to_string();
    let comment = format!("COMMENT-MARKER-{}", w.tag);
    let (s, c) = rest(&w.app, "POST", &format!("/api/v1/apps/{}/records/{record_id}/comments", w.slug), &w.owner, Some(json!({ "body": comment }))).await;
    assert!(s.is_success(), "{c}");
    let label = format!("TOKEN-LABEL-{}", w.tag);
    let (s, t) = rest(&w.app, "POST", "/api/v1/auth/tokens", &w.owner, Some(json!({ "label": label, "kind": "agent" }))).await;
    assert!(s.is_success(), "{t}");
    let raw_token = t["token"].as_str().unwrap_or("").to_string();

    for sample in [false, true] {
        let (s, bundle) = export(&w, sample).await;
        assert_eq!(s, StatusCode::OK, "{bundle}");
        let text = bundle.to_string();
        for marker in [member_address.as_str(), comment.as_str(), label.as_str(), raw_token.as_str(), OWNER] {
            assert!(!marker.is_empty() && !text.contains(marker), "the bundle (sample data: {sample}) holds {marker}");
        }
        walk(&bundle, &mut |key, _| {
            for banned in ["secret", "token", "password", "members", "collaborators", "comments", "history", "attachments"] {
                assert!(!key.to_lowercase().contains(banned), "the bundle (sample data: {sample}) has a key named {key}");
            }
        });
    }
    // Without the sample option, no record is in the bundle at all.
    let (_, plain) = export(&w, false).await;
    assert!(plain.get("sample_data").is_none_or(|v| v.is_null()) || plain["sample_data"] == json!({}), "no sample data unless asked: {plain}");
}

// 4. A slug collision suggests a free slug. Ids are rewritten without collision.
#[tokio::test]
async fn a_slug_collision_suggests_a_free_slug_and_colliding_ids_are_rewritten() {
    let w = world().await;
    live(&w).await;
    let (_, bundle) = export(&w, false).await;
    // The source app is live under this slug, and its rule holds its id, so both collide with the bundle.
    let (s, p) = import(&w, bundle.clone(), json!({})).await;
    assert_eq!(s, StatusCode::CREATED, "{p}");
    let new_slug = p["app_slug"].as_str().unwrap().to_string();
    assert_ne!(new_slug, w.slug, "the proposal is not for the live app's slug: {p}");
    assert_eq!(p["bundle"]["suggested_slug"], json!(new_slug), "{p}");
    assert_eq!(p["bundle"]["original_slug"], json!(w.slug), "{p}");
    let rewritten = p["bundle"]["rewritten_ids"].to_string();
    assert!(rewritten.contains("urgent-"), "the rule id that belonged to another app was rewritten: {p}");
    let failed: Vec<&Value> = p["checks"].as_array().unwrap().iter().filter(|c| c["status"] == "fail").collect();
    assert!(failed.is_empty(), "the rewritten rule does not collide: {failed:?}");

    // A second import finds the first slug taken in a proposal, and suggests another.
    let (s, again) = import(&w, bundle, json!({})).await;
    assert_eq!(s, StatusCode::CREATED, "{again}");
    let second = again["app_slug"].as_str().unwrap().to_string();
    assert!(second != new_slug && second != w.slug, "a free slug each time: {new_slug} then {second}");

    // A person may choose the slug. A taken one is still refused.
    let (s, body) = import(&w, export(&w, false).await.1, json!({ "slug": w.slug })).await;
    assert_eq!(s, StatusCode::CREATED, "{body}");
    assert_ne!(body["app_slug"], json!(w.slug), "a taken slug is replaced by a free one: {body}");
}

// 5. A bundle's webhook arrives disabled, and a bundle never carries a secret.
#[tokio::test]
async fn a_bundles_webhook_arrives_disabled_and_a_secret_is_refused() {
    let w = world().await;
    live(&w).await;
    let (_, mut bundle) = export(&w, false).await;
    bundle["integrations"] = json!({ "webhooks": [{ "name": "notify", "url": "https://hooks.example.edu/notify", "events": ["record.created"], "enabled": true, "secret_name": "notify-secret" }] });
    let (s, p) = import(&w, seal(bundle.clone()), json!({})).await;
    assert_eq!(s, StatusCode::CREATED, "{p}");
    let hooks = p["bundle"]["webhooks"].as_array().expect("the webhooks are listed");
    assert_eq!(hooks.len(), 1, "{p}");
    assert_eq!(hooks[0]["enabled"], false, "a webhook in a bundle arrives disabled: {p}");
    assert_eq!(hooks[0]["secret_name"], "notify-secret", "the secret is named, so the importer must supply it: {p}");
    assert!(p["changes"].to_string().contains("notify"), "the change list says a webhook is waiting: {p}");

    // A bundle that carries a secret itself is refused outright.
    bundle["integrations"]["webhooks"][0]["secret"] = json!("s3cr3t-value");
    let before = proposal_count(&w);
    let (s, body) = import(&w, seal(bundle), json!({})).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{body}");
    assert!(body["error"].as_str().unwrap_or("").contains("secret"), "{body}");
    assert_eq!(proposal_count(&w), before, "nothing was created");
}

// 6. A custom page keeps its source hash, and a modified source fails the page check.
#[tokio::test]
async fn a_custom_page_keeps_its_source_hash_and_a_modified_source_fails() {
    let w = world().await;
    live(&w).await;
    let (_, bundle) = export(&w, false).await;
    assert!(bundle["page_sources"].get(sha(PAGE)).is_some(), "the page source is keyed by its hash: {}", bundle["page_sources"]);
    assert_eq!(bundle["manifest"]["pages"][0]["kind"]["Custom"]["source_sha256"], json!(sha(PAGE)));
    let (s, p) = import(&w, bundle.clone(), json!({})).await;
    assert_eq!(s, StatusCode::CREATED, "{p}");
    assert!(p["checks"].to_string().contains("page_source"), "the page check ran: {p}");

    // The source is changed and the bundle sealed again, so only the page check can catch it.
    let mut edited = bundle.clone();
    edited["page_sources"][sha(PAGE)] = json!("export default function Page() { return <p>CHANGED</p>; }");
    let before = proposal_count(&w);
    let (s, body) = import(&w, seal(edited), json!({})).await;
    assert_ne!(s, StatusCode::CREATED, "a modified page source is not accepted: {body}");
    assert!(body.to_string().contains("page_source") || body.to_string().contains("source"), "{body}");
    assert_eq!(proposal_count(&w), before, "nothing was created");
}

// 7. Sample data: at most 5,000 records, and only when asked.
#[tokio::test]
async fn sample_data_is_only_in_the_bundle_when_asked_and_is_capped() {
    let w = world().await;
    live(&w).await;
    let (_, with) = export(&w, true).await;
    let rows = with["sample_data"]["cases"].as_array().expect("sample records of the cases table");
    assert_eq!(rows.len(), 3, "{with}");
    assert!(rows.iter().all(|r| r.get("title").is_some()), "{with}");
    // A bundle with more than 5,000 sample records is refused on import.
    let mut big = with.clone();
    big["sample_data"] = json!({ "cases": (0..5001).map(|i| json!({ "title": format!("row {i}") })).collect::<Vec<_>>() });
    let before = proposal_count(&w);
    let (s, body) = import(&w, seal(big), json!({})).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{body}");
    assert!(body["error"].as_str().unwrap_or("").contains("5,000"), "{body}");
    assert_eq!(proposal_count(&w), before);
}

// 8. Only an owner or admin of the app's workspace may export.
#[tokio::test]
async fn only_an_owner_or_admin_may_export() {
    let w = world().await;
    live(&w).await;
    let stranger = issue_test_token_and_user(&format!("stranger-{}@state.edu", w.tag));
    let (s, _) = rest(&w.app, "POST", &format!("/api/v1/apps/{}/bundle", w.slug), &stranger, Some(json!({}))).await;
    assert_eq!(s, StatusCode::FORBIDDEN);
}

// 9. The exported document is the committed example, and the schema in governance/schema validates that example.
#[tokio::test]
async fn the_exported_document_is_the_committed_example() {
    let w = world().await;
    live_as(&w, "bundle-example", "flag-urgent").await;
    let (s, doc) = rest(&w.app, "POST", "/api/v1/apps/bundle-example/bundle", &w.owner, Some(json!({ "sample_data": true }))).await;
    assert_eq!(s, StatusCode::OK, "{doc}");
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../governance/fixtures/app-bundle.example.json");
    if std::env::var_os("UPDATE_GOLDEN").is_some() {
        std::fs::write(&path, serde_json::to_string_pretty(&doc).unwrap() + "\n").expect("write the example");
    }
    let committed: Value = serde_json::from_str(&std::fs::read_to_string(&path).expect("governance/fixtures/app-bundle.example.json")).expect("the example is JSON");
    assert_eq!(doc, committed, "the export changed. Run with UPDATE_GOLDEN=1 to write the new example, and read the diff");
}

// 10. The hash does not depend on the order of the keys, so a tool that sorts or reorders them does not damage a bundle.
#[tokio::test]
async fn the_hash_does_not_depend_on_the_order_of_the_keys() {
    let w = world().await;
    live(&w).await;
    let (_, bundle) = export(&w, true).await;
    for reordered in [sorted(&bundle), reversed(&bundle)] {
        assert_ne!(reordered.to_string(), bundle.to_string(), "the keys really are in another order");
        let (s, p) = import(&w, reordered, json!({})).await;
        assert_eq!(s, StatusCode::CREATED, "{p}");
    }
}

// 11. Sample data follows the exporter's row rules: a row they may not read is not in the bundle.
#[tokio::test]
async fn sample_data_leaves_out_rows_the_exporter_may_not_read() {
    let w = world().await;
    let mut m = manifest(&w.slug);
    m["access"] = json!({ "rows": [{
        "id": "hide-two", "table_id": "cases", "ops": ["read"],
        "visible_when": { "cmp": { "field": "title", "op": "ne", "value": { "literal": "two" } } },
        "except_roles": [], "message": null
    }], "columns": [] });
    live_manifest(&w, m, &format!("urgent-{}", w.tag)).await;
    let (s, bundle) = export(&w, true).await;
    assert_eq!(s, StatusCode::OK, "{bundle}");
    let titles: Vec<&str> = bundle["sample_data"]["cases"].as_array().expect("sample rows").iter().filter_map(|r| r["title"].as_str()).collect();
    assert_eq!(titles, vec!["one", "three"], "the row the rule hides is not exported: {bundle}");
    assert!(!bundle.to_string().contains("note two"), "nor any of its values");
}
