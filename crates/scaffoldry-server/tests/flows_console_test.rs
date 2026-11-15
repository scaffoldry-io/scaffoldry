//! Workflows phase 11: the Flows panel of the console. A Platform Admin sees every flow, who approved it for each workspace and
//! when, and can switch one off or on with a reason.

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use common::TestDb;
use http_body_util::BodyExt;
use scaffoldry_server::build_app_with_state;
use scaffoldry_server::service::identity::issue_test_token_and_user;
use scaffoldry_server::state::ServerState;
use serde_json::{json, Value};
use std::sync::Arc;
use tower::ServiceExt;

const ADMIN: &str = "jordan.lee@state.edu";
/// Owns the applicants' workspace (Internal) and proposes the flows.
const OWNER: &str = "einstein@physics.state.edu";
/// A second owner of the applicants' workspace.
const CO_OWNER: &str = "bohr@physics.state.edu";
const SOURCE_WS: &str = "ws-physics-optics";
/// Owns the aid office's workspace (Restricted).
const AID_OWNER: &str = "prof.curie@science.state.edu";
const AID_WS: &str = "ws-bio-lab";

async fn send(app: &axum::Router, method: &str, uri: &str, who: &str, body: Option<Value>) -> (StatusCode, Value) {
    let b = Request::builder().method(method).uri(uri).header("authorization", format!("Bearer {}", issue_test_token_and_user(who)));
    let req = match body {
        Some(v) => b.header("content-type", "application/json").body(Body::from(v.to_string())).unwrap(),
        None => b.body(Body::empty()).unwrap(),
    };
    let resp = app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
}

/// Calls an MCP tool. `Err` holds the error text.
async fn tool(app: &axum::Router, who: &str, name: &str, args: Value) -> Result<Value, String> {
    let payload = json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": { "name": name, "arguments": args } });
    let (_, v) = send(app, "POST", "/api/mcp", who, Some(payload)).await;
    if let Some(err) = v.get("error") {
        return Err(err["message"].to_string());
    }
    if v["result"]["isError"] == true {
        return Err(v["result"]["content"][0]["text"].to_string());
    }
    Ok(v["result"]["structuredContent"].clone())
}

fn field(name: &str, kind: &str) -> Value {
    json!({ "name": name, "label": name, "field_type": kind, "required": false, "ferpa_sensitive": false })
}

fn sensitive(name: &str) -> Value {
    let mut f = field(name, "Text");
    f["ferpa_sensitive"] = json!(true);
    f
}

struct World {
    db: TestDb,
    state: Arc<ServerState>,
    app: axum::Router,
    source: String,
    aid: String,
}

/// Makes an app through the direct route, and approves it as the platform admin if it needs a second person.
async fn make_app(app: &axum::Router, ws: &str, owner: &str, manifest: Value) {
    let (s, b) = send(app, "POST", &format!("/api/v1/workspaces/{ws}/apps"), owner, Some(manifest)).await;
    if s == StatusCode::ACCEPTED {
        let id = b["proposal_id"].as_str().unwrap().to_string();
        let digest = tool(app, ADMIN, "get_proposal", json!({ "proposal_id": id })).await.unwrap()["digest"].clone();
        tool(app, ADMIN, "decide_proposal", json!({ "proposal_id": id, "decision": "approve", "rationale": "Reviewed", "digest": digest })).await.expect("approved");
        return;
    }
    assert_eq!(s, StatusCode::CREATED, "{b}");
}

/// The applicants' app in the Internal workspace, with a sensitive `ssn`, and the aid app in the Restricted one.
async fn world() -> World {
    let db = TestDb::new();
    let state = db.state();
    state.seed_demo().expect("seed");
    let app = build_app_with_state(state.clone()).expect("router");
    let tag = &uuid::Uuid::new_v4().simple().to_string()[..8];
    let (source, aid) = (format!("apl-{tag}"), format!("aid-{tag}"));
    let mut email = field("email", "Text");
    email["indexed"] = json!(true);
    let mut link = field("student", "Link");
    link["link_target_table"] = json!("students");
    make_app(&app, SOURCE_WS, OWNER, json!({
        "slug": source, "title": "Admissions", "description": "", "organization_code": "PHYS", "department": "physics", "workspace_id": SOURCE_WS, "views": [],
        "tables": [{ "id": "applications", "name": "Applications", "slug": "applications", "primary_field": "name", "fields": [
            field("name", "Text"), field("email", "Text"), field("program", "Text"), field("status", "Text"), field("aid_file", "Text"), sensitive("ssn")
        ] }],
    })).await;
    make_app(&app, AID_WS, AID_OWNER, json!({
        "slug": aid, "title": "Financial Aid", "description": "", "organization_code": "BIO", "department": "biology", "workspace_id": AID_WS, "views": [],
        "tables": [
            { "id": "students", "name": "Students", "slug": "students", "primary_field": "name", "fields": [field("name", "Text"), email, sensitive("ssn")] },
            { "id": "awards", "name": "Awards", "slug": "awards", "primary_field": "term", "fields": [field("amount", "Number"), field("term", "Text"), field("tax_id", "Text"), link] },
        ],
    })).await;
    issue_test_token_and_user(CO_OWNER);
    let (s, b) = send(&app, "POST", &format!("/api/v1/workspaces/{SOURCE_WS}/collaborators"), ADMIN, Some(json!({ "eppn": CO_OWNER, "role": "owner" }))).await;
    assert!(s.is_success(), "{b}");
    World { db, state, app, source, aid }
}

fn step(id: &str, action: Value) -> Value {
    json!({ "id": id, "when": [], "kind": { "Service": { "action": action } } })
}

/// The applicants' flow: find the student, create an award in the aid app, link it, and note it on the application.
fn open_aid_file(w: &World, rule_id: &str, award_term: &str) -> Value {
    json!({
        "id": rule_id, "name": "Open aid file", "enabled": true, "trigger": "RecordCreated",
        "predicates": [{ "field_name": "status", "operator": "Equals", "expected_value": "Admitted" }],
        "steps": [
            step("find_student", json!({ "FindRecord": { "id": "find_student", "app": w.aid, "table": "students", "field": "email", "equals": "{trigger.email}", "limit": 1 } })),
            step("create_award", json!({ "CreateRecord": { "id": "create_award", "app": w.aid, "table": "awards", "fields": [["amount", "1000"], ["term", award_term]] } })),
            step("link_student", json!({ "SetLink": { "target": { "Created": { "step": "create_award" } }, "link_field": "student", "to": { "Found": { "step": "find_student" } } } })),
            step("note_file", json!({ "UpdateRecord": { "target": "Trigger", "fields": [["aid_file", "{create_award.id}"]] } })),
        ]
    })
}

fn new_id() -> String {
    format!("flow-{}", &uuid::Uuid::new_v4().simple().to_string()[..10])
}

async fn propose(w: &World, who: &str, ws: &str, app: &str, rule: Value) -> Result<Value, String> {
    tool(&w.app, who, "propose_app_change", json!({ "workspace_id": ws, "app_slug": app, "rules": [rule], "summary": "Open an aid file on admission" })).await
}

/// Decides one party of a proposal through the REST route.
async fn decide(w: &World, who: &str, id: &str, party: &str, decision: &str) -> (StatusCode, Value) {
    let (_, p) = send(&w.app, "GET", &format!("/api/v1/proposals/{id}"), who, None).await;
    send(&w.app, "POST", &format!("/api/v1/proposals/{id}/decision"), who, Some(json!({ "decision": decision, "rationale": "Reviewed", "digest": p["digest"], "workspace_id": party }))).await
}

fn live(w: &World, rule_id: &str) -> bool {
    w.state.automations.read().unwrap().get(&w.source).is_some_and(|l| l.iter().any(|r| r.id == rule_id))
}

fn ledger(w: &World, kind: &str, needle: &str) -> Vec<Value> {
    let (kind, needle) = (kind.to_string(), format!("%{needle}%"));
    w.db.run(move |c| c.query("SELECT payload FROM governance_ledger WHERE decision_type = $1 AND payload::text LIKE $2 ORDER BY sequence", &[&kind, &needle]).unwrap().iter().map(|r| r.get(0)).collect())
}

const FACULTY: &str = "prof.curie@science.state.edu";

/// Makes a cross-workspace flow live: both parties approve.
async fn go_live(w: &World, rule: Value) -> String {
    let rule_id = rule["id"].as_str().unwrap().to_string();
    let p = propose(w, OWNER, SOURCE_WS, &w.source, rule).await.expect("proposed");
    let id = p["proposal_id"].as_str().unwrap().to_string();
    assert_eq!(decide(w, AID_OWNER, &id, AID_WS, "approve").await.0, StatusCode::OK);
    assert_eq!(decide(w, CO_OWNER, &id, SOURCE_WS, "approve").await.0, StatusCode::OK);
    assert!(live(w, &rule_id));
    rule_id
}

/// A flow in the applicants' app alone that reads the sensitive `ssn` of its own trigger.
fn reads_ssn(rule_id: &str) -> Value {
    json!({
        "id": rule_id, "name": "Copy ssn note", "enabled": true, "trigger": "RecordCreated", "predicates": [],
        "steps": [step("note", json!({ "UpdateRecord": { "target": "Trigger", "fields": [["program", "{trigger.ssn}"]] } }))]
    })
}

async fn flows(w: &World, query: &str) -> Vec<Value> {
    let (s, v) = send(&w.app, "GET", &format!("/api/v1/admin/flows{query}"), ADMIN, None).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    v["flows"].as_array().unwrap().clone()
}

fn row<'a>(rows: &'a [Value], id: &str) -> &'a Value {
    rows.iter().find(|r| r["rule_id"] == id).unwrap_or_else(|| panic!("a row for {id} in {rows:?}"))
}

// 1. The panel lists a cross-workspace flow with both parties and their approval times.
#[tokio::test(flavor = "multi_thread")]
async fn the_panel_lists_a_cross_workspace_flow_with_both_parties_and_when_each_approved() {
    let w = world().await;
    let id = go_live(&w, open_aid_file(&w, &new_id(), "\"Fall\"")).await;
    let rows = flows(&w, "").await;
    let r = row(&rows, &id);
    assert_eq!(r["crosses_workspaces"], true, "{r}");
    assert_eq!(r["state"], "Active");
    assert_eq!(r["home_workspace"], SOURCE_WS);
    let parties = r["parties"].as_array().unwrap();
    assert_eq!(parties.len(), 2, "{r}");
    let aid = parties.iter().find(|p| p["workspace_id"] == AID_WS).unwrap();
    let home = parties.iter().find(|p| p["workspace_id"] == SOURCE_WS).unwrap();
    assert_eq!(aid["approved_by"], AID_OWNER, "{r}");
    assert_eq!(home["approved_by"], CO_OWNER, "{r}");
    assert!(aid["approved_at"].is_string() && home["approved_at"].is_string(), "each approval has a time: {r}");
    assert!(r["grants"].as_str().unwrap().contains("may"), "grants in sentences: {r}");
    assert!(r["last_run"].is_null() && r["runs_last_7_days"] == 0, "no run yet: {r}");
    // A run that happens shows as the last run.
    let (s, b) = send(&w.app, "POST", &format!("/api/v1/apps/{}/records", w.source), OWNER, Some(json!({ "data": { "_table_id": "applications", "name": "Ada", "email": "ada@state.edu", "program": "PhD", "status": "Admitted" } }))).await;
    assert!(s.is_success(), "{b}");
    let rows = flows(&w, "").await;
    assert!(row(&rows, &id)["last_run"]["at"].is_string(), "{rows:?}");
    assert_eq!(row(&rows, &id)["runs_last_7_days"], 1);
    let (_, o) = send(&w.app, "GET", "/api/v1/admin/overview", ADMIN, None).await;
    assert!(o["flows"]["total"].as_i64().unwrap() >= 1 && o["flows"]["cross_workspace"].as_i64().unwrap() >= 1, "the Overview counts flows: {o}");
}

// 2. `Disable` writes a settings ledger entry with the reason and stops the next run. `Enable` puts it back.
#[tokio::test(flavor = "multi_thread")]
async fn disable_writes_a_ledger_entry_and_stops_the_next_run() {
    let w = world().await;
    let id = go_live(&w, open_aid_file(&w, &new_id(), "\"Fall\"")).await;
    let (s, v) = send(&w.app, "POST", &format!("/api/v1/admin/flows/{id}/disable"), ADMIN, Some(json!({}))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "a reason is required: {v}");
    let (s, v) = send(&w.app, "POST", &format!("/api/v1/admin/flows/{id}/disable"), ADMIN, Some(json!({ "reason": "Aid office asked for a pause." }))).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let entries = ledger(&w, "PolicyRevision", &id);
    assert_eq!(entries.len(), 1, "one settings ledger entry: {entries:?}");
    let rationale: String = { let id = id.clone(); w.db.run(move |c| c.query_one("SELECT rationale FROM governance_ledger WHERE decision_type = 'PolicyRevision' AND payload::text LIKE $1 ORDER BY sequence DESC LIMIT 1", &[&format!("%{id}%")]).unwrap().get(0)) };
    assert!(rationale.contains("Aid office asked for a pause."), "the reason is in the entry: {rationale}");
    assert_eq!(row(&flows(&w, "").await, &id)["state"], "Disabled");
    let (s, b) = send(&w.app, "POST", &format!("/api/v1/apps/{}/records", w.source), OWNER, Some(json!({ "data": { "_table_id": "applications", "name": "Ada", "email": "ada@state.edu", "program": "PhD", "status": "Admitted" } }))).await;
    assert!(s.is_success(), "{b}");
    let slug = w.aid.clone();
    let awards: i64 = w.db.run(move |c| c.query_one("SELECT COUNT(*) FROM dataset_records WHERE app_slug = $1 AND table_id = 'awards'", &[&slug]).unwrap().get(0));
    assert_eq!(awards, 0, "the disabled flow made nothing");
    let (s, v) = send(&w.app, "POST", &format!("/api/v1/admin/flows/{id}/enable"), ADMIN, Some(json!({ "reason": "The pause is over." }))).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(row(&flows(&w, "").await, &id)["state"], "Active");
    let (s, _) = send(&w.app, "POST", "/api/v1/admin/flows/nope/disable", ADMIN, Some(json!({ "reason": "x" }))).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
}

// 3. The filter `Reads sensitive data` lists only flows with a sensitive field in `read`. `Crosses workspaces` lists only the others' complement.
#[tokio::test(flavor = "multi_thread")]
async fn the_filters_list_only_what_they_name() {
    let w = world().await;
    let cross = go_live(&w, open_aid_file(&w, &new_id(), "\"Fall\"")).await;
    let sid = new_id();
    let p = propose(&w, OWNER, SOURCE_WS, &w.source, reads_ssn(&sid)).await.expect("proposed");
    let pid = p["proposal_id"].as_str().unwrap().to_string();
    let (s, b) = decide(&w, CO_OWNER, &pid, SOURCE_WS, "approve").await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert!(live(&w, &sid), "a flow in one workspace needs one party: {b}");
    let all = flows(&w, "").await;
    assert!(all.iter().any(|r| r["rule_id"] == cross.as_str()) && all.iter().any(|r| r["rule_id"] == sid.as_str()));
    let sensitive = flows(&w, "?reads_sensitive=true").await;
    assert!(sensitive.iter().any(|r| r["rule_id"] == sid.as_str()), "the flow that reads ssn is listed: {sensitive:?} / {all:?}");
    assert!(sensitive.iter().all(|r| r["reads_sensitive"] == true), "only flows that read a sensitive field: {sensitive:?}");
    assert!(!sensitive.iter().any(|r| r["rule_id"] == cross.as_str()), "the aid flow reads no sensitive field");
    let crossing = flows(&w, "?crosses_workspaces=true").await;
    assert!(crossing.iter().any(|r| r["rule_id"] == cross.as_str()) && crossing.iter().all(|r| r["crosses_workspaces"] == true), "{crossing:?}");
    assert!(!crossing.iter().any(|r| r["rule_id"] == sid.as_str()));
}

// 4. A faculty caller is 403 on every route here.
#[tokio::test(flavor = "multi_thread")]
async fn a_faculty_caller_is_refused_on_every_route() {
    let w = world().await;
    let id = go_live(&w, open_aid_file(&w, &new_id(), "\"Fall\"")).await;
    assert_eq!(send(&w.app, "GET", "/api/v1/admin/flows", FACULTY, None).await.0, StatusCode::FORBIDDEN);
    for verb in ["disable", "enable"] {
        let (s, _) = send(&w.app, "POST", &format!("/api/v1/admin/flows/{id}/{verb}"), FACULTY, Some(json!({ "reason": "I want to." }))).await;
        assert_eq!(s, StatusCode::FORBIDDEN, "{verb}");
    }
    assert!(!flow_disabled(&w, &id), "nothing changed");
}

fn flow_disabled(w: &World, id: &str) -> bool {
    w.state.settings.read().unwrap().get("flows.disabled").and_then(Value::as_array).is_some_and(|l| l.iter().any(|v| v == id))
}

// A flow the breaker turned off shows as `Tripped`, and the Overview counts it. An administrator's `Disable` shows over it.
#[tokio::test(flavor = "multi_thread")]
async fn a_flow_the_breaker_turned_off_is_tripped() {
    let w = world().await;
    let id = go_live(&w, open_aid_file(&w, &new_id(), "\"Fall\"")).await;
    let (_, tripped) = w.state.repository.flow_failed(&id, 1).unwrap();
    assert!(tripped, "one failure trips a breaker set to one");
    assert_eq!(row(&flows(&w, "").await, &id)["state"], "Tripped");
    assert_eq!(flows(&w, "?state=Tripped").await.len(), 1);
    assert!(flows(&w, "?state=Active").await.iter().all(|r| r["rule_id"] != id.as_str()));
    let (_, o) = send(&w.app, "GET", "/api/v1/admin/overview", ADMIN, None).await;
    assert!(o["flows"]["tripped"].as_i64().unwrap() >= 1, "{o}");
}
