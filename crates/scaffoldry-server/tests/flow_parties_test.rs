//! Workflows phase 2: a flow touching several workspaces needs approval from each. Moving sensitive data is flagged.
//!
//! A proposal that carries a flow has parties: the home workspace and every other workspace in its grants. Each party's
//! owners or admins approve that party. The flow goes live only when every party has approved, and the stale check
//! then covers every app the flow touches.

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

fn check<'a>(p: &'a Value, name: &str) -> &'a Value {
    p["checks"].as_array().unwrap().iter().find(|c| c["name"] == name).unwrap_or_else(|| panic!("no check {name} in {p}"))
}

fn live(w: &World, rule_id: &str) -> bool {
    w.state.automations.read().unwrap().get(&w.source).is_some_and(|l| l.iter().any(|r| r.id == rule_id))
}

fn ledger(w: &World, kind: &str, needle: &str) -> Vec<Value> {
    let (kind, needle) = (kind.to_string(), format!("%{needle}%"));
    w.db.run(move |c| c.query("SELECT payload FROM governance_ledger WHERE decision_type = $1 AND payload::text LIKE $2 ORDER BY sequence", &[&kind, &needle]).unwrap().iter().map(|r| r.get(0)).collect())
}

fn status_of(w: &World, id: &str) -> String {
    let id = uuid::Uuid::parse_str(id).unwrap();
    w.db.run(move |c| c.query_one("SELECT status FROM app_proposals WHERE id = $1", &[&id]).unwrap().get(0))
}

// 1. Two workspaces: two parties. One approval leaves it pending and not live; both make it live, with one ledger entry
// per party and one for the publication.
#[tokio::test(flavor = "multi_thread")]
async fn a_flow_across_two_workspaces_needs_both_parties() {
    let w = world().await;
    let rule_id = new_id();
    let p = propose(&w, OWNER, SOURCE_WS, &w.source, open_aid_file(&w, &rule_id, "\"Fall\"")).await.expect("proposed");
    let id = p["proposal_id"].as_str().unwrap().to_string();
    let parties: Vec<&str> = p["parties"].as_array().unwrap().iter().map(|x| x["workspace_id"].as_str().unwrap()).collect();
    assert_eq!(parties, vec![SOURCE_WS, AID_WS], "{p}");
    assert_eq!(check(&p, "cross_workspace")["outcome"], "fail", "flagged: {p}");
    assert_eq!(check(&p, "flow_grants")["outcome"], "pass");
    assert!(check(&p, "flow_grants")["messages"].as_array().unwrap().len() >= 2, "sentences per workspace: {p}");

    let (s, b) = decide(&w, AID_OWNER, &id, AID_WS, "approve").await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_eq!(b["status"], "Pending", "{b}");
    assert!(!live(&w, &rule_id), "not live with one party");
    let waiting: Vec<&str> = b["parties"].as_array().unwrap().iter().filter(|x| x["decision"].is_null()).map(|x| x["workspace_id"].as_str().unwrap()).collect();
    assert_eq!(waiting, vec![SOURCE_WS], "the proposer sees who is still waiting: {b}");

    let (s, b) = decide(&w, CO_OWNER, &id, SOURCE_WS, "approve").await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_eq!(b["status"], "Approved", "{b}");
    assert!(live(&w, &rule_id), "live after both");
    assert_eq!(ledger(&w, "FlowPartyApproved", &id).len(), 2, "one entry per party");
    let published = ledger(&w, "ProcessDefinitionChanged", &rule_id);
    assert_eq!(published.len(), 1);
    assert_eq!(published[0]["parties"].as_array().map(|a| a.len()), Some(2), "the publication lists the parties: {}", published[0]);
}

// 2. One party rejecting rejects the proposal.
#[tokio::test(flavor = "multi_thread")]
async fn any_party_rejecting_rejects_the_proposal() {
    let w = world().await;
    let rule_id = new_id();
    let p = propose(&w, OWNER, SOURCE_WS, &w.source, open_aid_file(&w, &rule_id, "\"Fall\"")).await.expect("proposed");
    let id = p["proposal_id"].as_str().unwrap().to_string();
    let (s, b) = decide(&w, CO_OWNER, &id, SOURCE_WS, "approve").await;
    assert_eq!(s, StatusCode::OK, "{b}");
    let (s, b) = decide(&w, AID_OWNER, &id, AID_WS, "reject").await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_eq!(status_of(&w, &id), "Rejected");
    assert!(!live(&w, &rule_id));
}

// 3. A sensitive field moving into a lower-classification workspace fails `classification_downgrade` and stores nothing.
#[tokio::test(flavor = "multi_thread")]
async fn moving_a_sensitive_field_to_a_lower_classification_fails() {
    let w = world().await;
    // From the aid app (Restricted) into the applicants' app (Internal).
    let rule = json!({
        "id": new_id(), "name": "Copy the tax id back", "enabled": true, "trigger": "RecordCreated",
        "steps": [step("copy", json!({ "CreateRecord": { "id": "copy", "app": w.source, "table": "applications", "fields": [["name", "{trigger.name}"], ["program", "{trigger.ssn}"]] } }))]
    });
    let before: i64 = w.db.run(|c| c.query_one("SELECT COUNT(*) FROM app_proposals", &[]).unwrap().get(0));
    let err = propose(&w, AID_OWNER, AID_WS, &w.aid, rule).await.expect_err("refused");
    assert!(err.contains("classification_downgrade"), "{err}");
    let after: i64 = w.db.run(|c| c.query_one("SELECT COUNT(*) FROM app_proposals", &[]).unwrap().get(0));
    assert_eq!(before, after, "nothing stored");
}

// 4. A flagged `data_movement` cannot be approved by the proposer for the source party. Another owner of the source
// workspace can approve it.
#[tokio::test(flavor = "multi_thread")]
async fn the_proposer_cannot_approve_the_source_of_moving_data() {
    let w = world().await;
    let rule_id = new_id();
    // The applicant's ssn (Internal) goes into the aid app (Restricted): no downgrade, but data moves.
    let p = propose(&w, OWNER, SOURCE_WS, &w.source, open_aid_file(&w, &rule_id, "{trigger.ssn}")).await.expect("proposed");
    assert_eq!(check(&p, "data_movement")["outcome"], "fail", "flagged: {p}");
    let id = p["proposal_id"].as_str().unwrap().to_string();
    let (s, b) = decide(&w, OWNER, &id, SOURCE_WS, "approve").await;
    assert_eq!(s, StatusCode::FORBIDDEN, "{b}");
    let (s, b) = decide(&w, CO_OWNER, &id, SOURCE_WS, "approve").await;
    assert_eq!(s, StatusCode::OK, "{b}");
}

// 5. Another party's app changes before the final approval: the check is stale, the proposal is `Superseded`, and nothing
// is live.
#[tokio::test(flavor = "multi_thread")]
async fn a_change_in_another_partys_app_supersedes_the_proposal() {
    let w = world().await;
    let rule_id = new_id();
    let p = propose(&w, OWNER, SOURCE_WS, &w.source, open_aid_file(&w, &rule_id, "\"Fall\"")).await.expect("proposed");
    let id = p["proposal_id"].as_str().unwrap().to_string();
    let (s, b) = decide(&w, CO_OWNER, &id, SOURCE_WS, "approve").await;
    assert_eq!(s, StatusCode::OK, "{b}");

    // The aid office changes its app.
    let manifest = {
        use scaffoldry_engine::HostRouter;
        serde_json::to_value(w.state.engine.read().unwrap().resolve_by_slug(&w.aid).cloned().expect("aid app")).unwrap()
    };
    let mut changed = manifest.clone();
    changed["description"] = json!("Changed by the aid office");
    let made = tool(&w.app, AID_OWNER, "propose_app_change", json!({ "workspace_id": AID_WS, "manifest": changed, "summary": "A new description" })).await.expect("proposed");
    let (_, other) = send(&w.app, "GET", &format!("/api/v1/proposals/{}", made["proposal_id"].as_str().unwrap()), AID_OWNER, None).await;
    let (s, b) = send(&w.app, "POST", &format!("/api/v1/proposals/{}/decision", made["proposal_id"].as_str().unwrap()), AID_OWNER, Some(json!({ "decision": "approve", "rationale": "Ours", "digest": other["digest"] }))).await;
    assert_eq!(s, StatusCode::OK, "{b}");

    let (s, b) = decide(&w, AID_OWNER, &id, AID_WS, "approve").await;
    assert_eq!(s, StatusCode::CONFLICT, "{b}");
    assert_eq!(status_of(&w, &id), "Superseded");
    assert!(!live(&w, &rule_id));
}

// 6. With `flows.cross_workspace_enabled` false, a proposal with two parties fails.
#[tokio::test(flavor = "multi_thread")]
async fn cross_workspace_flows_can_be_turned_off() {
    let w = world().await;
    let (s, b) = send(&w.app, "PUT", "/api/v1/settings/flows.cross_workspace_enabled", ADMIN, Some(json!(false))).await;
    assert!(s.is_success(), "{b}");
    let err = propose(&w, OWNER, SOURCE_WS, &w.source, open_aid_file(&w, &new_id(), "\"Fall\"")).await.expect_err("refused");
    assert!(err.contains("cross_workspace"), "{err}");
}

// 7. A flow id in `flows.disabled` stops its next run, and the setting change is in the ledger.
#[tokio::test(flavor = "multi_thread")]
async fn a_disabled_flow_does_not_run() {
    let w = world().await;
    let rule_id = new_id();
    let p = propose(&w, OWNER, SOURCE_WS, &w.source, open_aid_file(&w, &rule_id, "\"Fall\"")).await.expect("proposed");
    let id = p["proposal_id"].as_str().unwrap().to_string();
    assert_eq!(decide(&w, AID_OWNER, &id, AID_WS, "approve").await.0, StatusCode::OK);
    assert_eq!(decide(&w, CO_OWNER, &id, SOURCE_WS, "approve").await.0, StatusCode::OK);
    assert!(live(&w, &rule_id));

    let (s, b) = send(&w.app, "PUT", "/api/v1/settings/flows.disabled", ADMIN, Some(json!([rule_id]))).await;
    assert!(s.is_success(), "{b}");
    assert!(!ledger(&w, "PolicyRevision", "flows.disabled").is_empty(), "the setting change is in the ledger");

    let (s, b) = send(&w.app, "POST", &format!("/api/v1/apps/{}/records", w.source), OWNER, Some(json!({ "data": { "_table_id": "applications", "name": "Ada", "email": "ada@state.edu", "program": "PhD", "status": "Admitted" } }))).await;
    assert!(s.is_success(), "{b}");
    let slug = w.aid.clone();
    let awards: i64 = w.db.run(move |c| c.query_one("SELECT COUNT(*) FROM dataset_records WHERE app_slug = $1 AND table_id = 'awards'", &[&slug]).unwrap().get(0));
    assert_eq!(awards, 0, "the disabled flow made nothing");
}

// 8. The aid office's owner sees only the sentences about their app.
#[tokio::test(flavor = "multi_thread")]
async fn each_party_sees_what_concerns_it() {
    let w = world().await;
    let p = propose(&w, OWNER, SOURCE_WS, &w.source, open_aid_file(&w, &new_id(), "\"Fall\"")).await.expect("proposed");
    let id = p["proposal_id"].as_str().unwrap().to_string();
    let (s, view) = send(&w.app, "GET", &format!("/api/v1/proposals/{id}"), AID_OWNER, None).await;
    assert_eq!(s, StatusCode::OK, "a party's owner may read the proposal: {view}");
    let changes: Vec<String> = view["changes"].as_array().unwrap().iter().map(|c| c.as_str().unwrap().to_string()).collect();
    assert!(!changes.is_empty());
    let text = changes.join(" ");
    assert!(text.contains("Financial Aid / Awards") && text.contains("Awards: amount, term, student"), "what it creates and sets here: {text}");
    assert!(text.contains("Applications: status, email"), "what it reads from elsewhere: {text}");
    assert!(!text.contains("aid_file") && !text.contains("is added"), "nothing about the other app's changes: {text}");

    let (_, full) = send(&w.app, "GET", &format!("/api/v1/proposals/{id}"), OWNER, None).await;
    assert!(full["changes"].as_array().unwrap().iter().any(|c| c.as_str().unwrap().contains("aid_file")), "the home owner sees all: {full}");
}
