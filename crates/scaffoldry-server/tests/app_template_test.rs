//! Import and export, phase 4: approved templates.
//!
//! A template is an approved bundle in an institution catalog. Publishing one needs a second person. Using one makes a
//! proposal like any other, except that what the template already contains was reviewed when it was published, so an
//! unchanged template into a workspace that can hold it is approved by the workspace owner in the same request.
//! NIST SP 800-53 CM-2, CM-3 and CM-14.

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::Router;
use http_body_util::BodyExt;
use scaffoldry_server::build_app_with_state;
use scaffoldry_server::service::identity::issue_test_token_and_user;
use scaffoldry_server::state::{CollaboratorRecord, OrganizationNode, RoleRow, ServerState, WorkspaceRecord};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tower::ServiceExt;
use uuid::Uuid;

const WORKSPACE: &str = "ws-physics-optics";
const FERPA: &str = "ws-tpl-ferpa";
const INTERNAL: &str = "ws-tpl-internal";
const OWNER: &str = "einstein@physics.state.edu";
const ADMIN: &str = "jordan.lee@state.edu";
const ADMIN_TWO: &str = "second.admin@state.edu";
const ROOT: &str = "00000000-0000-0000-0000-000000000001";
const PAGE: &str = "export default function Page() { return <p>TEMPLATE-PAGE</p>; }";

struct World {
    db: common::TestDb,
    app: Router,
    owner: String,
    admin: String,
    admin_two: String,
    slug: String,
    tag: String,
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

fn member(state: &ServerState, workspace: &str, eppn: &str, role: &str) {
    let record = CollaboratorRecord {
        id: Uuid::new_v4().to_string(),
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

fn workspace(state: &ServerState, id: &str, classification: &str, unit: Option<Uuid>) {
    let mut ws: WorkspaceRecord = state.workspaces.read().unwrap().get(WORKSPACE).cloned().expect("the demo workspace");
    ws.id = id.to_string();
    ws.name = format!("Workspace {id}");
    ws.code = id.to_uppercase();
    ws.data_classification = classification.to_string();
    if let Some(u) = unit {
        ws.organization_id = Some(u);
    }
    state.repository.upsert_workspace(&ws).expect("a workspace");
    state.workspaces.write().unwrap().insert(id.to_string(), ws);
    member(state, id, OWNER, "owner");
}

fn appoint(state: &ServerState, eppn: &str, unit: Uuid, scoped_affiliation: &str) {
    state
        .persist_role(&RoleRow {
            id: Uuid::new_v4(),
            person_id: Uuid::new_v4(),
            eppn: eppn.to_string(),
            organization_id: unit,
            role_title: scoped_affiliation.to_string(),
            scoped_affiliation: scoped_affiliation.to_string(),
            is_primary: false,
            source: "test".to_string(),
            position_key: None,
        })
        .unwrap();
}

fn unit(state: &ServerState, name: &str) -> Uuid {
    let id = Uuid::new_v4();
    let node = OrganizationNode {
        id,
        parent_id: Some(Uuid::parse_str(ROOT).unwrap()),
        name: name.to_string(),
        code: name.to_uppercase(),
        org_type: "Department".to_string(),
        hierarchy_key: "academic".to_string(),
        synthetic: false,
    };
    state.organizations.write().unwrap().insert(id, node.clone());
    state.persist_organization(&node).unwrap();
    id
}

async fn world() -> World {
    let db = common::TestDb::new();
    let state = db.state();
    state.seed_demo().expect("seed");
    workspace(&state, FERPA, "FERPA Sensitive", None);
    workspace(&state, INTERNAL, "Internal", None);
    appoint(&state, ADMIN_TWO, Uuid::parse_str(ROOT).unwrap(), "platform_admin");
    let tag = uuid::Uuid::new_v4().to_string()[..8].to_string();
    let app = build_app_with_state(state).expect("router");
    World {
        db,
        app,
        owner: issue_test_token_and_user(OWNER),
        admin: issue_test_token_and_user(ADMIN),
        admin_two: issue_test_token_and_user(ADMIN_TWO),
        slug: format!("tpl-src-{tag}"),
        tag,
    }
}

fn field(name: &str, sensitive: bool) -> Value {
    if sensitive {
        json!({ "name": name, "label": name, "field_type": "Text", "required": false, "ferpa_sensitive": true, "mask": { "kind": "last4" } })
    } else {
        json!({ "name": name, "label": name, "field_type": "Text", "required": false, "ferpa_sensitive": false })
    }
}

/// The source app: a table with a sensitive field, and a custom page that reads it.
fn manifest(slug: &str) -> Value {
    json!({
        "slug": slug, "title": "Student intake", "description": "Collects a student's name and number", "organization_code": "PHYS", "department": "physics",
        "workspace_id": WORKSPACE, "views": [],
        "tables": [{ "id": "students", "name": "Students", "slug": "students", "fields": [field("name", false), field("note", false), field("ssn", true)] }],
        "pages": [{ "id": "roster", "title": "Roster", "kind": { "Custom": { "source_sha256": sha(PAGE), "grants": [
            { "table_id": "students", "read": ["name", "ssn"], "write": [], "create": false, "delete": false }
        ]}}}]
    })
}

/// Puts the source app live. A sensitive field is a flagged change, so the second administrator approves it.
async fn live(w: &World) {
    let p = tool_as(w, &w.owner, "propose_app_change", json!({
        "workspace_id": WORKSPACE, "manifest": manifest(&w.slug), "summary": "The source app", "page_sources": [PAGE],
    }))
    .await
    .expect("proposed");
    tool_as(w, &w.admin_two, "decide_proposal", json!({ "proposal_id": p["proposal_id"], "decision": "approve", "rationale": "Set up", "digest": p["digest"] })).await.expect("live");
}

async fn bundle_of_source(w: &World) -> Value {
    let (s, b) = rest(&w.app, "POST", &format!("/api/v1/apps/{}/bundle", w.slug), &w.owner, Some(json!({}))).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    b
}

/// Publishes the source app as a template: proposed by one administrator and approved by the other.
async fn published(w: &World, extra: Value) -> String {
    live(w).await;
    let mut body = json!({ "bundle": bundle_of_source(w).await, "name": "Student intake", "description": "Collects a student's name and number", "category": "Admissions" });
    if let Some(o) = extra.as_object() {
        for (k, v) in o {
            body[k] = v.clone();
        }
    }
    let (s, t) = rest(&w.app, "POST", "/api/v1/templates", &w.admin, Some(body)).await;
    assert_eq!(s, StatusCode::CREATED, "{t}");
    let id = t["template_id"].as_str().expect("a template id").to_string();
    let (s, d) = rest(&w.app, "POST", &format!("/api/v1/templates/{id}/decision"), &w.admin_two, Some(json!({ "decision": "approve", "rationale": "Reviewed" }))).await;
    assert_eq!(s, StatusCode::OK, "{d}");
    id
}

async fn instantiate(w: &World, who: &str, workspace: &str, template: &str, body: Value) -> (StatusCode, Value) {
    rest(&w.app, "POST", &format!("/api/v1/workspaces/{workspace}/templates/{template}/instantiate"), who, Some(body)).await
}

fn check<'a>(p: &'a Value, name: &str) -> Option<&'a Value> {
    p["checks"].as_array()?.iter().find(|c| c["name"] == name)
}

// 1. Publishing needs a second person. A single admin cannot publish alone.
#[tokio::test]
async fn publishing_needs_a_second_person() {
    let w = world().await;
    live(&w).await;
    let (s, t) = rest(&w.app, "POST", "/api/v1/templates", &w.admin, Some(json!({ "bundle": bundle_of_source(&w).await, "name": "Student intake" }))).await;
    assert_eq!(s, StatusCode::CREATED, "{t}");
    assert_eq!(t["status"], "pending", "a published template waits for a second person: {t}");
    let id = t["template_id"].as_str().unwrap().to_string();

    // Not in the catalog yet, and the publisher cannot approve their own.
    let (_, list) = rest(&w.app, "GET", "/api/v1/templates", &w.owner, None).await;
    assert!(!list.to_string().contains(&id), "a pending template is not offered: {list}");
    let (s, own) = rest(&w.app, "POST", &format!("/api/v1/templates/{id}/decision"), &w.admin, Some(json!({ "decision": "approve", "rationale": "Mine" }))).await;
    assert!(s == StatusCode::FORBIDDEN || s == StatusCode::CONFLICT, "the publisher cannot approve alone: {s} {own}");

    // The other administrator can. A ledger entry records each step.
    let (s, done) = rest(&w.app, "POST", &format!("/api/v1/templates/{id}/decision"), &w.admin_two, Some(json!({ "decision": "approve", "rationale": "Reviewed" }))).await;
    assert_eq!(s, StatusCode::OK, "{done}");
    let (_, list) = rest(&w.app, "GET", "/api/v1/templates", &w.owner, None).await;
    assert!(list.to_string().contains(&id), "an approved template is offered: {list}");
    let entries: i64 = w.db.run(|c| c.query_one("SELECT COUNT(*) FROM governance_ledger WHERE decision_type LIKE 'Template%'", &[]).unwrap().get(0));
    assert!(entries >= 2, "publishing and approving are in the ledger ({entries})");

    // Someone who is not an administrator cannot publish at all.
    let (s, _) = rest(&w.app, "POST", "/api/v1/templates", &w.owner, Some(json!({ "bundle": bundle_of_source(&w).await, "name": "Mine" }))).await;
    assert_eq!(s, StatusCode::FORBIDDEN);
}

// 2. An unchanged template into a matching workspace is approved in the same request by an owner.
#[tokio::test]
async fn an_unchanged_template_into_a_matching_workspace_is_approved_by_the_owner_at_once() {
    let w = world().await;
    let id = published(&w, json!({})).await;
    let slug = format!("tpl-use-{}", w.tag);

    // The same content proposed by hand needs a second person: that is what the template saves.
    let hand_slug = format!("by-hand-{}", w.tag);
    let mut by_hand_manifest = manifest(&hand_slug);
    by_hand_manifest["workspace_id"] = json!(FERPA);
    let by_hand = tool_as(&w, &w.owner, "propose_app_change", json!({ "workspace_id": FERPA, "manifest": by_hand_manifest, "summary": "By hand", "page_sources": [PAGE] })).await.expect("proposed");
    let refused = tool_as(&w, &w.owner, "decide_proposal", json!({ "proposal_id": by_hand["proposal_id"], "decision": "approve", "rationale": "Mine", "digest": by_hand["digest"] })).await;
    assert!(refused.is_err(), "without the template the owner cannot approve it alone");

    let (s, p) = instantiate(&w, &w.owner, FERPA, &id, json!({ "slug": slug, "approve": true })).await;
    assert_eq!(s, StatusCode::CREATED, "{p}");
    assert_eq!(p["approved"], true, "the owner's approval stood in the same request: {p}");
    assert_eq!(check(&p, "template_pristine").map(|c| c["outcome"].clone()), Some(json!("pass")), "{p}");
    let (s, app) = rest(&w.app, "GET", &format!("/api/v1/apps/{slug}"), &w.owner, None).await;
    assert_eq!(s, StatusCode::OK, "the app is live: {app}");
    for name in ["page_sensitivity", "mask_weakened", "access_loosened"] {
        assert_ne!(check(&p, name).map(|c| c["outcome"].clone()), Some(json!("fail")), "{name} is not raised again for what the template holds: {p}");
    }
}

// 3. The same template into a lower-classification workspace raises `sensitivity`.
#[tokio::test]
async fn the_same_template_into_a_lower_classification_workspace_raises_sensitivity() {
    let w = world().await;
    let id = published(&w, json!({})).await;
    let (s, p) = instantiate(&w, &w.owner, INTERNAL, &id, json!({ "slug": format!("tpl-low-{}", w.tag), "approve": true })).await;
    assert_eq!(s, StatusCode::CREATED, "{p}");
    assert_eq!(check(&p, "sensitivity").map(|c| c["outcome"].clone()), Some(json!("fail")), "{p}");
    assert_eq!(p["approved"], false, "the owner cannot approve past a failed check alone: {p}");
    let (s, _) = rest(&w.app, "GET", &format!("/api/v1/apps/tpl-low-{}", w.tag), &w.owner, None).await;
    assert_eq!(s, StatusCode::NOT_FOUND, "nothing is live");
}

// 4. Changing one field raises the checks for that field only.
#[tokio::test]
async fn changing_one_field_raises_the_checks_for_that_field_only() {
    let w = world().await;
    let id = published(&w, json!({})).await;

    // A plain field is changed. Nothing about the sensitive field or the page is raised again.
    let mut plain = manifest(&format!("tpl-plain-{}", w.tag));
    plain["tables"][0]["fields"][1]["label"] = json!("Remarks");
    let (s, p) = instantiate(&w, &w.owner, FERPA, &id, json!({ "slug": format!("tpl-plain-{}", w.tag), "manifest": plain, "approve": true })).await;
    assert_eq!(s, StatusCode::CREATED, "{p}");
    assert_eq!(check(&p, "template_pristine").map(|c| c["outcome"].clone()), Some(json!("warn")), "a change is named, not hidden: {p}");
    assert!(check(&p, "template_pristine").unwrap()["messages"].to_string().contains("note"), "it names the field that changed: {p}");
    assert_ne!(check(&p, "page_sensitivity").map(|c| c["outcome"].clone()), Some(json!("fail")), "{p}");
    assert_eq!(p["approved"], true, "a harmless change to a plain field still goes through: {p}");

    // The sensitive field's mask is loosened. That field's check is raised, and the owner cannot approve it alone.
    let mut loose = manifest(&format!("tpl-loose-{}", w.tag));
    loose["tables"][0]["fields"][2]["mask"] = json!({ "kind": "initials" });
    let (s, p) = instantiate(&w, &w.owner, FERPA, &id, json!({ "slug": format!("tpl-loose-{}", w.tag), "manifest": loose, "approve": true })).await;
    assert_eq!(s, StatusCode::CREATED, "{p}");
    assert_eq!(check(&p, "mask_weakened").map(|c| c["outcome"].clone()), Some(json!("fail")), "the changed field's check is raised: {p}");
    assert!(check(&p, "mask_weakened").unwrap()["messages"].to_string().contains("ssn"), "{p}");
    assert_eq!(p["approved"], false, "{p}");
}

// 5. A retired template cannot be instantiated.
#[tokio::test]
async fn a_retired_template_cannot_be_instantiated() {
    let w = world().await;
    let id = published(&w, json!({})).await;
    let (s, r) = rest(&w.app, "POST", &format!("/api/v1/templates/{id}/retire"), &w.admin, Some(json!({ "reason": "Out of date" }))).await;
    assert_eq!(s, StatusCode::OK, "{r}");
    let (s, p) = instantiate(&w, &w.owner, FERPA, &id, json!({ "slug": format!("tpl-old-{}", w.tag) })).await;
    assert!(s == StatusCode::NOT_FOUND || s == StatusCode::CONFLICT, "{s} {p}");
    assert!(p["error"].as_str().unwrap_or("").contains("retired"), "the refusal says it is retired: {p}");
    let (_, list) = rest(&w.app, "GET", "/api/v1/templates", &w.owner, None).await;
    assert!(!list.to_string().contains(&id), "a retired template is no longer offered: {list}");
}

// 6. A unit-limited template is invisible outside its unit.
#[tokio::test]
async fn a_unit_limited_template_is_invisible_outside_its_unit() {
    let w = world().await;
    let state = w.db.state();
    let science = unit(&state, "science");
    let id = published(&w, json!({ "unit_id": science.to_string() })).await;

    let (_, outside) = rest(&w.app, "GET", "/api/v1/templates", &w.owner, None).await;
    assert!(!outside.to_string().contains(&id), "someone outside the unit does not see it: {outside}");
    let (s, p) = instantiate(&w, &w.owner, FERPA, &id, json!({ "slug": format!("tpl-out-{}", w.tag) })).await;
    assert_eq!(s, StatusCode::NOT_FOUND, "nor can they use it: {p}");

    let insider = "insider@state.edu";
    appoint(&state, insider, science, "faculty");
    let (_, inside) = rest(&w.app, "GET", "/api/v1/templates", &issue_test_token_and_user(insider), None).await;
    assert!(inside.to_string().contains(&id), "someone placed in the unit does see it: {inside}");
    let (_, all) = rest(&w.app, "GET", "/api/v1/templates", &w.admin, None).await;
    assert!(all.to_string().contains(&id), "a Platform Admin sees it: {all}");
}

// 7. A template says what it contains in sentences.
#[tokio::test]
async fn a_template_says_what_it_contains_in_sentences() {
    let w = world().await;
    let id = published(&w, json!({})).await;
    let (_, list) = rest(&w.app, "GET", "/api/v1/templates", &w.owner, None).await;
    let row = list["templates"].as_array().unwrap().iter().find(|t| t["id"] == id.as_str()).cloned().expect("the template is listed");
    assert_eq!(row["name"], "Student intake");
    assert_eq!(row["category"], "Admissions");
    let contents = row["contents"].to_string();
    for word in ["Students", "ssn", "Roster"] {
        assert!(contents.contains(word), "the contents name {word}: {contents}");
    }
    assert!(row["needs_classification"].is_string(), "and say what classification it needs: {row}");
}

// 8. A damaged bundle cannot be published.
#[tokio::test]
async fn a_damaged_bundle_cannot_be_published() {
    let w = world().await;
    live(&w).await;
    let mut bundle = bundle_of_source(&w).await;
    bundle["manifest"]["title"] = json!("Changed");
    let (s, body) = rest(&w.app, "POST", "/api/v1/templates", &w.admin, Some(json!({ "bundle": bundle, "name": "Bad" }))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{body}");
    assert!(body["error"].as_str().unwrap_or("").contains("damaged"), "{body}");
}
