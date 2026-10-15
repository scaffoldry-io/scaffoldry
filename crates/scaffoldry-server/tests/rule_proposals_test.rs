//! MCP apps phase 4a: process definitions and every direct app change are proposals.
//!
//! Each test uses its own app slug, so the tests share the database without touching anyone's rows.

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::Router;
use http_body_util::BodyExt;
use scaffoldry_server::build_app_with_state;
use scaffoldry_server::service::identity::issue_test_token_and_user;
use scaffoldry_server::state::{CollaboratorRecord, ServerState};
use serde_json::{json, Value};
use std::sync::Arc;
use tower::ServiceExt;
use uuid::Uuid;


/// An Internal workspace.
const WORKSPACE: &str = "ws-physics-optics";
const OWNER: &str = "einstein@physics.state.edu";
const EDITOR: &str = "editor.test@physics.state.edu";
const OTHER_ADMIN: &str = "jordan.lee@state.edu";

fn db_url() -> String {
    scaffoldry_server::testing::database_url()
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

struct World {
    state: Arc<ServerState>,
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
        app_slug: String::new(),
    });
    let app = build_app_with_state(state.clone()).expect("router");
    World {
        state,
        app,
        owner: issue_test_token_and_user(OWNER),
        editor: issue_test_token_and_user(EDITOR),
        admin: issue_test_token_and_user(OTHER_ADMIN),
        slug: format!("rules-{}", &Uuid::new_v4().to_string()[..8]),
    }
}

fn field(name: &str, t: &str, sensitive: bool) -> Value {
    json!({ "name": name, "label": name, "field_type": t, "required": false, "ferpa_sensitive": sensitive })
}

fn manifest(slug: &str, status_sensitive: bool) -> Value {
    json!({
        "slug": slug, "title": "Rules", "description": "d", "organization_code": "PHYS", "department": "physics",
        "views": [],
        "tables": [{ "id": "apps", "name": "Applications", "slug": "apps", "fields": [
            field("gpa", "Number", false), field("name", "Text", false), field("status", "Text", status_sensitive)
        ]}]
    })
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

async fn live_app(w: &World, status_sensitive: bool) {
    // The owner makes it through the direct route. A clean manifest is live at once.
    let (s, body) = rest(&w.app, "POST", &format!("/api/v1/workspaces/{WORKSPACE}/apps"), &w.owner, Some(manifest(&w.slug, status_sensitive))).await;
    if s == StatusCode::ACCEPTED {
        // A sensitive field in an Internal workspace needs a second person.
        let id = body["proposal_id"].as_str().unwrap().to_string();
        let (err, done) = decide(w, &w.admin, &id, "approve").await;
        assert!(!err, "{done}");
        return;
    }
    assert_eq!(s, StatusCode::CREATED, "{body}");
}

fn service_rule(id: &str, new_status: &str) -> Value {
    json!({
        "id": id, "name": format!("Rule {id}"), "enabled": true, "trigger": "RecordCreated",
        "predicates": [{ "field_name": "gpa", "operator": "GreaterThan", "expected_value": "3.85" }],
        "steps": [{ "id": "s1", "when": [], "kind": { "Service": { "action": { "UpdateRecordStatus": { "new_status": new_status } } } } }]
    })
}

fn task_rule(id: &str, approve_to: &str) -> Value {
    json!({
        "id": id, "name": format!("Task {id}"), "enabled": true, "trigger": "RecordCreated",
        "steps": [{ "id": "decide", "when": [], "kind": { "UserTask": {
            "role": "owner", "prompt": "Decide",
            "approve": [{ "UpdateRecordStatus": { "new_status": approve_to } }],
            "reject": [{ "UpdateRecordStatus": { "new_status": "Denied" } }],
            "approver": { "Role": { "name": "owner" } }
        }}}]
    })
}

async fn propose_rules(w: &World, token: &str, rules: Vec<Value>) -> (bool, Value) {
    tool(&w.app, token, "propose_app_change", json!({ "workspace_id": WORKSPACE, "app_slug": w.slug, "rules": rules, "summary": "A rule" })).await
}

async fn decide(w: &World, token: &str, id: &str, decision: &str) -> (bool, Value) {
    tool(&w.app, token, "decide_proposal", json!({ "proposal_id": id, "decision": decision, "rationale": "Reviewed" })).await
}

fn live_rules(w: &World) -> Vec<Value> {
    w.state.automations.read().unwrap().get(&w.slug).map(|l| l.iter().map(|r| serde_json::to_value(r).unwrap()).collect()).unwrap_or_default()
}

fn check<'a>(result: &'a Value, name: &str) -> &'a Value {
    result["checks"].as_array().unwrap().iter().find(|c| c["name"] == name).unwrap_or_else(|| panic!("no check {name} in {result}"))
}

// 1. A rules-only proposal leaves the live rules alone. Approving makes version 1 live.
#[tokio::test]
async fn a_rules_only_proposal_changes_nothing_until_approved_and_then_writes_version_one() {
    let _db = common::TestDb::new();
    let w = world().await;
    live_app(&w, false).await;
    let rule_id = format!("r-{}", &Uuid::new_v4().to_string()[..8]);

    let (err, p) = propose_rules(&w, &w.owner, vec![service_rule(&rule_id, "Approved")]).await;
    assert!(!err, "{p}");
    assert!(live_rules(&w).is_empty(), "nothing is live yet");
    let id = p["proposal_id"].as_str().unwrap().to_string();
    assert!(p["changes"].as_array().unwrap().iter().any(|c| c.as_str().unwrap().contains("set status to Approved")), "{p}");

    let (err, done) = decide(&w, &w.owner, &id, "approve").await;
    assert!(!err, "{done}");
    let live = live_rules(&w);
    assert_eq!(live.len(), 1);
    assert_eq!(live[0]["version"], 1);

    let (rid, pid) = (rule_id.clone(), id.clone());
    let (version, proposal): (i32, Option<Uuid>) = db(move |c| {
        let row = c.query_one("SELECT version, proposal_id FROM workflow_rule_versions WHERE rule_id = $1", &[&rid]).unwrap();
        (row.get(0), row.get(1))
    });
    assert_eq!(version, 1);
    assert_eq!(proposal.unwrap().to_string(), pid);
    let ledger: i64 = db(move |c| {
        c.query_one("SELECT count(*) FROM governance_ledger WHERE decision_type = 'ProcessDefinitionChanged' AND payload->>'proposal_id' = $1", &[&id])
            .unwrap()
            .get(0)
    });
    assert_eq!(ledger, 1);
}

// 2. A rule that sets a sensitive field is flagged.
#[tokio::test]
async fn a_rule_that_sets_a_sensitive_field_needs_a_second_person() {
    let _db = common::TestDb::new();
    let w = world().await;
    live_app(&w, true).await;
    let (err, p) = propose_rules(&w, &w.owner, vec![service_rule(&format!("r-{}", &Uuid::new_v4().to_string()[..8]), "Approved")]).await;
    assert!(!err, "{p}");
    assert_eq!(check(&p, "rule_effects")["outcome"], "fail", "{p}");
    let id = p["proposal_id"].as_str().unwrap().to_string();
    let (err, _) = decide(&w, &w.owner, &id, "approve").await;
    assert!(err, "the proposer may not approve it");
    let (err, done) = decide(&w, &w.admin, &id, "approve").await;
    assert!(!err, "{done}");
}

// 3. The direct route: 201 when clean, 202 when flagged.
#[tokio::test]
async fn posting_a_rule_is_201_when_clean_and_202_when_flagged() {
    let _db = common::TestDb::new();
    let w = world().await;
    live_app(&w, false).await;
    let rule_id = format!("r-{}", &Uuid::new_v4().to_string()[..8]);
    let (s, body) = rest(&w.app, "POST", &format!("/api/v1/apps/{}/automations", w.slug), &w.owner, Some(service_rule(&rule_id, "Approved"))).await;
    assert_eq!(s, StatusCode::CREATED, "{body}");
    assert_eq!(body["id"], rule_id.as_str());
    assert_eq!(body["version"], 1);
    let rid = rule_id.clone();
    let approved: i64 = db(move |c| {
        c.query_one("SELECT count(*) FROM app_proposals WHERE status = 'Approved' AND rules::text LIKE '%' || $1 || '%'", &[&rid]).unwrap().get(0)
    });
    assert_eq!(approved, 1, "an approved proposal row records it");

    // A flagged rule: the app's status field is sensitive.
    let w2 = world().await;
    live_app(&w2, true).await;
    let flagged_id = format!("r-{}", &Uuid::new_v4().to_string()[..8]);
    let (s, body) = rest(&w2.app, "POST", &format!("/api/v1/apps/{}/automations", w2.slug), &w2.owner, Some(service_rule(&flagged_id, "Approved"))).await;
    assert_eq!(s, StatusCode::ACCEPTED, "{body}");
    assert_eq!(body["status"], "Pending");
    assert!(body["proposal_id"].is_string());
    assert!(live_rules(&w2).is_empty(), "the rule is not live");
}

// 4. A webhook step is 400 with the step id, and stores no proposal.
#[tokio::test]
async fn a_webhook_step_is_refused_with_its_id_and_stores_no_proposal() {
    let _db = common::TestDb::new();
    let w = world().await;
    live_app(&w, false).await;
    let base_rows = proposal_rows(&w.slug);
    let mut rule = service_rule(&format!("r-{}", &Uuid::new_v4().to_string()[..8]), "Approved");
    rule["steps"] = json!([{ "id": "hook-step", "when": [], "kind": { "Service": { "action": { "WebhookDispatch": { "target_url": "https://example.org/x" } } } } }]);
    let (s, body) = rest(&w.app, "POST", &format!("/api/v1/apps/{}/automations", w.slug), &w.owner, Some(rule)).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{body}");
    assert!(body["error"].as_str().unwrap().contains("hook-step"), "{body}");
    assert_eq!(proposal_rows(&w.slug), base_rows, "no proposal row was stored");
}

// 5. A rule naming a field that does not exist is 400 and names the field.
#[tokio::test]
async fn a_rule_naming_a_missing_field_is_refused_and_names_it() {
    let _db = common::TestDb::new();
    let w = world().await;
    live_app(&w, false).await;
    let mut rule = service_rule(&format!("r-{}", &Uuid::new_v4().to_string()[..8]), "Approved");
    rule["predicates"] = json!([{ "field_name": "no_such_field", "operator": "Equals", "expected_value": "x" }]);
    let (s, body) = rest(&w.app, "POST", &format!("/api/v1/apps/{}/automations", w.slug), &w.owner, Some(rule)).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{body}");
    assert!(body["error"].as_str().unwrap().contains("no_such_field"), "{body}");
}

// 6. Two proposals for one rule on one base version.
#[tokio::test]
async fn the_second_approval_for_one_rule_version_is_refused_and_superseded() {
    let _db = common::TestDb::new();
    let w = world().await;
    live_app(&w, false).await;
    let rule_id = format!("r-{}", &Uuid::new_v4().to_string()[..8]);
    let (_, a) = propose_rules(&w, &w.owner, vec![service_rule(&rule_id, "Approved")]).await;
    let (_, b) = propose_rules(&w, &w.owner, vec![service_rule(&rule_id, "Admitted")]).await;
    let (a, b) = (a["proposal_id"].as_str().unwrap().to_string(), b["proposal_id"].as_str().unwrap().to_string());
    let (err, _) = decide(&w, &w.owner, &a, "approve").await;
    assert!(!err);
    let (err, msg) = decide(&w, &w.owner, &b, "approve").await;
    assert!(err, "{msg}");
    let (_, got) = tool(&w.app, &w.owner, "get_proposal", json!({ "proposal_id": b })).await;
    assert_eq!(got["status"], "Superseded");
}

// 7. An instance keeps the version it started under.
#[tokio::test]
async fn a_waiting_instance_is_decided_by_the_version_it_started_under() {
    let _db = common::TestDb::new();
    let w = world().await;
    live_app(&w, false).await;
    let rule_id = format!("r-{}", &Uuid::new_v4().to_string()[..8]);

    // Version 1 approves to Admitted.
    let (_, p1) = propose_rules(&w, &w.owner, vec![task_rule(&rule_id, "Admitted")]).await;
    let (err, _) = decide(&w, &w.owner, p1["proposal_id"].as_str().unwrap(), "approve").await;
    assert!(!err);

    // A record is created by the editor. The rule starts an instance on version 1.
    let (s, created) = rest(&w.app, "POST", &format!("/api/v1/apps/{}/records", w.slug), &w.editor, Some(json!({ "data": { "_table_id": "apps", "name": "A", "gpa": 3.9, "status": "New" } }))).await;
    assert_eq!(s, StatusCode::CREATED, "{created}");
    let record_id = created["id"].as_str().unwrap().to_string();
    let instance_id = format!("{rule_id}:{record_id}");
    let started = w.state.process_instances.read().unwrap().get(&instance_id).cloned().expect("an instance is waiting");
    assert_eq!(started.rule_version, 1);

    // Version 2 approves to Denied-for-test "Changed".
    let (_, p2) = propose_rules(&w, &w.owner, vec![task_rule(&rule_id, "Changed")]).await;
    let (err, done) = decide(&w, &w.owner, p2["proposal_id"].as_str().unwrap(), "approve").await;
    assert!(!err, "{done}");
    assert_eq!(live_rules(&w)[0]["version"], 2);

    // The owner decides the instance. The version 1 action is applied.
    let (s, body) = rest(&w.app, "POST", &format!("/api/v1/apps/{}/processes/{}/decide", w.slug, instance_id.replace(':', "%3A")), &w.owner, Some(json!({ "decision": "approve" }))).await;
    assert_eq!(s, StatusCode::OK, "{body}");
    let (_, record) = rest(&w.app, "GET", &format!("/api/v1/apps/{}/records/{record_id}", w.slug), &w.owner, None).await;
    assert_eq!(record["data"]["status"], "Admitted", "{record}");
}

// 8. The direct update of an app.
#[tokio::test]
async fn updating_an_app_is_forbidden_for_an_editor_and_writes_one_proposal_and_one_ledger_entry_for_an_owner() {
    let _db = common::TestDb::new();
    let w = world().await;
    live_app(&w, false).await;
    let (_, current) = rest(&w.app, "GET", &format!("/api/v1/apps/{}", w.slug), &w.owner, None).await;

    let mut edit = current.clone();
    edit["description"] = json!("By an editor");
    let (s, _) = rest(&w.app, "PUT", &format!("/api/v1/apps/{}", w.slug), &w.editor, Some(edit)).await;
    assert_eq!(s, StatusCode::FORBIDDEN);

    let slug = w.slug.clone();
    let before: (i64, i64) = {
        let slug = slug.clone();
        db(move |c| {
            let p: i64 = c.query_one("SELECT count(*) FROM app_proposals WHERE app_slug = $1", &[&slug]).unwrap().get(0);
            let l: i64 = c.query_one("SELECT count(*) FROM governance_ledger WHERE app_slug = $1 AND decision_type = 'AppPublished'", &[&slug]).unwrap().get(0);
            (p, l)
        })
    };
    let mut clean = current.clone();
    clean["description"] = json!("By the owner");
    let (s, body) = rest(&w.app, "PUT", &format!("/api/v1/apps/{}", w.slug), &w.owner, Some(clean)).await;
    assert_eq!(s, StatusCode::OK, "{body}");
    assert_eq!(body["description"], "By the owner");
    let after: (i64, i64) = db(move |c| {
        let p: i64 = c.query_one("SELECT count(*) FROM app_proposals WHERE app_slug = $1", &[&slug]).unwrap().get(0);
        let l: i64 = c.query_one("SELECT count(*) FROM governance_ledger WHERE app_slug = $1 AND decision_type = 'AppPublished'", &[&slug]).unwrap().get(0);
        (p, l)
    });
    assert_eq!((after.0 - before.0, after.1 - before.1), (1, 1));
}

// 9. A proposal cannot verify a domain.
#[tokio::test]
async fn a_proposal_that_sets_custom_domain_verified_is_refused() {
    let _db = common::TestDb::new();
    let w = world().await;
    live_app(&w, false).await;
    let mut m = manifest(&w.slug, false);
    let domain = format!("{}.example.edu", w.slug);
    m["custom_domain"] = json!(domain);
    m["custom_domain_verified"] = json!(true);
    let (err, msg) = tool(&w.app, &w.owner, "propose_app_change", json!({ "workspace_id": WORKSPACE, "manifest": m, "summary": "Verify" })).await;
    assert!(err, "{msg}");
    assert!(msg.as_str().unwrap().contains("custom_domain_verified"), "{msg}");

    // Publishing sets the domain, not verified.
    let (s, body) = rest(&w.app, "POST", &format!("/api/v1/apps/{}/publish", w.slug), &w.owner, Some(json!({ "custom_domain": domain }))).await;
    assert_eq!(s, StatusCode::OK, "{body}");
    assert_eq!(body["custom_domain"], domain.as_str());
    assert_eq!(body["custom_domain_verified"], false);
}

// 11. A user task with no approver today saves with a warning.
#[tokio::test]
async fn a_user_task_that_resolves_to_nobody_saves_with_a_warning() {
    let _db = common::TestDb::new();
    let w = world().await;
    live_app(&w, false).await;
    let mut rule = task_rule(&format!("r-{}", &Uuid::new_v4().to_string()[..8]), "Admitted");
    rule["steps"][0]["kind"]["UserTask"]["approver"] = json!({ "Role": { "name": "nobody-holds-this-role" } });
    let (err, p) = propose_rules(&w, &w.owner, vec![rule]).await;
    assert!(!err, "a warning does not block: {p}");
    let approver = check(&p, "rule_approver");
    assert_eq!(approver["outcome"], "warn", "{p}");
    assert!(approver["messages"][0].as_str().unwrap().contains("nobody can decide"), "{p}");
    // A warning is not a flagged check, so the owner may approve their own proposal.
    let (err, done) = decide(&w, &w.owner, p["proposal_id"].as_str().unwrap(), "approve").await;
    assert!(!err, "{done}");
}

#[tokio::test]
async fn a_rule_that_sets_what_its_own_condition_reads_on_every_update_warns_of_a_loop() {
    let _db = common::TestDb::new();
    let w = world().await;
    live_app(&w, false).await;
    let mut rule = service_rule(&format!("r-{}", &Uuid::new_v4().to_string()[..8]), "Approved");
    rule["trigger"] = json!("RecordUpdated");
    rule["predicates"] = json!([{ "field_name": "status", "operator": "NotEquals", "expected_value": "Approved" }]);
    let (err, p) = propose_rules(&w, &w.owner, vec![rule]).await;
    assert!(!err, "{p}");
    assert_eq!(check(&p, "rule_loop")["outcome"], "warn", "{p}");
}

#[tokio::test]
async fn the_proposals_routes_list_get_and_decide_and_the_overview_counts_pending() {
    let _db = common::TestDb::new();
    let w = world().await;
    live_app(&w, false).await;
    let (_, p) = propose_rules(&w, &w.owner, vec![service_rule(&format!("r-{}", &Uuid::new_v4().to_string()[..8]), "Approved")]).await;
    let id = p["proposal_id"].as_str().unwrap().to_string();

    let (s, list) = rest(&w.app, "GET", &format!("/api/v1/workspaces/{WORKSPACE}/proposals"), &w.owner, None).await;
    assert_eq!(s, StatusCode::OK, "{list}");
    assert!(list.as_array().unwrap().iter().any(|r| r["id"] == id.as_str()));
    let (s, got) = rest(&w.app, "GET", &format!("/api/v1/proposals/{id}"), &w.owner, None).await;
    assert_eq!(s, StatusCode::OK);
    assert!(got["changes"].as_array().is_some());
    let (s, _) = rest(&w.app, "GET", &format!("/api/v1/workspaces/{WORKSPACE}/proposals"), &w.editor, None).await;
    assert_eq!(s, StatusCode::FORBIDDEN, "an editor does not see the Proposals tab's data");

    let (s, overview) = rest(&w.app, "GET", "/api/v1/admin/overview", &w.admin, None).await;
    assert_eq!(s, StatusCode::OK, "{overview}");
    assert!(overview["proposals"]["pending"].as_i64().unwrap() >= 1);

    let (s, done) = rest(&w.app, "POST", &format!("/api/v1/proposals/{id}/decision"), &w.owner, Some(json!({ "decision": "reject", "rationale": "No" }))).await;
    assert_eq!(s, StatusCode::OK, "{done}");
    assert_eq!(done["status"], "Rejected");
}
