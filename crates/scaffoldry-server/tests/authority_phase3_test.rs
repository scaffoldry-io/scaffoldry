//! Decision authority phase 3: the builder is told why, and what would change it.

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::Router;
use common::TestDb;
use http_body_util::BodyExt;
use scaffoldry_server::build_app_with_state;
use scaffoldry_server::service::identity::issue_test_token_and_user;
use serde_json::{json, Value};
use tower::ServiceExt;
use uuid::Uuid;

const ADMIN: &str = "jordan.lee@state.edu";
const PROPOSER: &str = "prop.oser@state.edu";
const H1: &str = "holder.one@state.edu";
const H2: &str = "holder.two@state.edu";
const UNIT_ADMIN: &str = "unit.admin@state.edu";
const OUTSIDER: &str = "prof.curie@science.state.edu";
const ROOT: &str = "00000000-0000-0000-0000-000000000001";

async fn send(app: &Router, method: &str, uri: &str, who: &str, body: Option<Value>) -> (StatusCode, Value) {
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

/// The whole MCP result of a tool call, so a refusal can be read as well as an answer.
async fn call(app: &Router, who: &str, tool: &str, args: Value) -> Value {
    let payload = json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": { "name": tool, "arguments": args } });
    send(app, "POST", "/api/mcp", who, Some(payload)).await.1["result"].clone()
}

struct World {
    db: TestDb,
    app: Router,
    unit: String,
    ws: String,
    position: String,
}

/// A department whose `officer` position is held by `holders`, and a workspace in it that only `members` belong to.
async fn world(holders: &[&str], members: &[&str]) -> World {
    let db = TestDb::new();
    let app = build_app_with_state(db.state()).expect("router");
    for who in [PROPOSER, H1, H2, UNIT_ADMIN, OUTSIDER] {
        issue_test_token_and_user(who);
    }
    let code = format!("A{}", &Uuid::new_v4().simple().to_string()[..8]);
    let (s, unit) = send(&app, "POST", "/api/v1/orgs", ADMIN, Some(json!({ "name": code, "code": code, "org_type": "Department", "parent_id": ROOT }))).await;
    assert_eq!(s, StatusCode::CREATED, "{unit}");
    let unit = unit["id"].as_str().unwrap().to_string();
    let position = format!("officer-{}", code.to_lowercase());
    let (s, p) = send(&app, "POST", "/api/v1/admin/positions", ADMIN, Some(json!({ "key": position, "name": "Officer", "description": "Test", "org_types": ["Department"], "max_holders": 5, "reason": "Define" }))).await;
    assert_eq!(s, StatusCode::CREATED, "{p}");
    for h in holders {
        let (s, b) = send(&app, "POST", &format!("/api/v1/orgs/{unit}/positions/{position}/holders"), ADMIN, Some(json!({ "eppn": h, "replace": false, "reason": "Assign" }))).await;
        assert_eq!(s, StatusCode::CREATED, "{b}");
    }
    let (s, b) = send(&app, "POST", &format!("/api/v1/orgs/{unit}/appointments"), ADMIN, Some(json!({ "eppn": UNIT_ADMIN, "scoped_affiliation": "unit_admin", "reason": "Unit admin" }))).await;
    assert_eq!(s, StatusCode::CREATED, "{b}");
    let (s, ws) = send(&app, "POST", "/api/v1/workspaces", ADMIN, Some(json!({ "name": code, "code": code, "visibility": "restricted", "department": "physics", "organization_id": unit }))).await;
    assert_eq!(s, StatusCode::CREATED, "{ws}");
    let ws = ws["id"].as_str().unwrap().to_string();
    for who in members {
        let (s, b) = send(&app, "POST", &format!("/api/v1/workspaces/{ws}/collaborators"), ADMIN, Some(json!({ "eppn": who, "role": "editor" }))).await;
        assert_eq!(s, StatusCode::CREATED, "{b}");
    }
    World { db, app, unit, ws, position }
}

impl World {
    fn require_officer(&self) {
        let (org, spec) = (Uuid::parse_str(ROOT).unwrap(), json!({ "Position": { "key": self.position, "from": "Workspace", "walk_up": false } }));
        self.db.run(move |c| {
            c.execute("INSERT INTO authority_rows (id, organization_id, tier, path, approver) VALUES ($1, $2, 'Moderate', 'approver', $3)", &[&Uuid::new_v4(), &org, &spec]).unwrap();
        });
    }

    fn manifest(&self, field: &str) -> Value {
        let slug = format!("auth-{}", &Uuid::new_v4().to_string()[..8]);
        let f = |n: &str, s: bool| json!({ "name": n, "label": n, "field_type": "Text", "required": false, "ferpa_sensitive": s });
        json!({
            "slug": slug, "title": "Advising", "description": "d", "organization_code": "PHYS", "department": "physics",
            "workspace_id": self.ws, "views": [], "pages": [],
            "tables": [{ "id": "t", "name": "T", "slug": "t", "fields": [f("title", false), f(field, true)] }]
        })
    }

    async fn propose(&self, who: &str, dry_run: bool) -> Value {
        let mut args = json!({ "workspace_id": self.ws, "manifest": self.manifest("date_of_birth"), "summary": "Add a date of birth field to the advising page" });
        if dry_run {
            args["dry_run"] = json!(true);
        }
        let r = call(&self.app, who, "propose_app_change", args).await;
        assert!(r["isError"] != true, "{r}");
        r["structuredContent"].clone()
    }

    fn count(&self, table: &'static str) -> i64 {
        self.db.run(move |c| c.query_one(&format!("SELECT COUNT(*) FROM {table}"), &[]).unwrap().get(0))
    }

    async fn approve(&self, who: &str, p: &Value) -> (StatusCode, Value) {
        send(&self.app, "POST", &format!("/api/v1/proposals/{}/decision", p["proposal_id"].as_str().unwrap()), who, Some(json!({ "decision": "approve", "rationale": "Reviewed", "digest": p["digest"] }))).await
    }
}

// 1. A dry run stores nothing and writes no ledger entry.
#[tokio::test]
async fn a_dry_run_stores_no_proposal_and_writes_no_ledger_entry() {
    let w = world(&[H1, H2], &[PROPOSER]).await;
    let tables = ["app_proposals", "governance_ledger", "app_page_sources", "app_manifests"];
    let before: Vec<i64> = tables.iter().map(|t| w.count(t)).collect();
    let dry = w.propose(PROPOSER, true).await;
    assert_eq!(dry["dry_run"], true, "{dry}");
    assert!(dry["proposal_id"].is_null(), "no proposal was made: {dry}");
    assert_eq!(dry["authority"]["tier"], "Moderate", "{dry}");
    assert_eq!(dry["authority"]["path"], "second_person");
    let after: Vec<i64> = tables.iter().map(|t| w.count(t)).collect();
    assert_eq!(before, after, "nothing was stored");
    // A real proposal made after it is still made.
    let real = w.propose(PROPOSER, false).await;
    assert!(real["proposal_id"].is_string(), "{real}");
    assert_eq!(w.count("app_proposals"), before[0] + 1);
}

// Scenario 1: the dry run answers Moderate, a second owner, and names the reason and the remedy.
#[tokio::test]
async fn a_dry_run_names_the_tier_the_path_the_reason_and_what_would_avoid_it() {
    let w = world(&[H1], &[PROPOSER]).await;
    let a = w.propose(PROPOSER, true).await["authority"].clone();
    assert_eq!(a["tier"], "Moderate");
    assert!(a["path_sentence"].as_str().unwrap().contains("second owner or admin"), "{a}");
    let reason = a["reasons_detail"].as_array().unwrap().iter().find(|r| r["check"] == "sensitivity").unwrap_or_else(|| panic!("a reason for the sensitivity check in {a}"));
    assert!(reason["sentence"].as_str().unwrap().contains("sensitive"), "{reason}");
    assert!(reason["remedy"].as_str().is_some_and(|r| !r.is_empty()), "the remedy: {reason}");
}

// Scenario 2: a unit whose matrix sends Moderate to the privacy officer names that position and its holder.
#[tokio::test]
async fn a_dry_run_in_a_unit_that_sends_moderate_to_an_officer_names_the_position_and_its_holder() {
    let w = world(&[H1], &[PROPOSER]).await;
    w.require_officer();
    let a = w.propose(PROPOSER, true).await["authority"].clone();
    assert_eq!(a["path"], "approver", "{a}");
    assert_eq!(a["approver"]["Position"]["key"], w.position.as_str());
    assert_eq!(a["resolved"][0]["eppn"], H1, "{a}");
    assert!(a["reasons_detail"].as_array().unwrap().iter().any(|r| r["sentence"].as_str().unwrap().contains("requires a named approver")), "{a}");
}

// Scenario 3: a vacant position is unassigned, and the answer says who to contact.
#[tokio::test]
async fn a_proposal_waiting_on_a_vacant_position_says_it_is_unassigned_and_who_to_contact() {
    let w = world(&[], &[PROPOSER]).await;
    w.require_officer();
    let dry = w.propose(PROPOSER, true).await["authority"].clone();
    assert_eq!(dry["no_approver"], true, "{dry}");
    assert!(dry["contact"].as_array().unwrap().iter().any(|c| c == UNIT_ADMIN), "the Org Unit Admins of the unit: {dry}");
    let real = w.propose(PROPOSER, false).await;
    assert_eq!(real["authority"]["no_approver"], true);
    let (_, got) = send(&w.app, "GET", &format!("/api/v1/proposals/{}", real["proposal_id"].as_str().unwrap()), ADMIN, None).await;
    assert!(got["authority"]["contact"].as_array().unwrap().iter().any(|c| c == UNIT_ADMIN), "{got}");
}

// 2. The names in `authority` are the people who then succeed at deciding.
#[tokio::test]
async fn the_people_named_are_the_people_who_can_then_decide() {
    let w = world(&[H1, H2], &[PROPOSER]).await;
    w.require_officer();
    let (a, b, c) = (w.propose(PROPOSER, false).await, w.propose(PROPOSER, false).await, w.propose(PROPOSER, false).await);
    let named: Vec<String> = a["authority"]["resolved"].as_array().unwrap().iter().map(|r| r["eppn"].as_str().unwrap().to_string()).collect();
    assert_eq!(named.len(), 2, "{a}");
    assert!(named.contains(&H1.to_string()) && named.contains(&H2.to_string()));
    let (s, body) = w.approve(&named[0], &a).await;
    assert_eq!(s, StatusCode::OK, "{} can decide: {body}", named[0]);
    let (s, body) = w.approve(&named[1], &b).await;
    assert_eq!(s, StatusCode::OK, "{} can decide: {body}", named[1]);
    let (s, _) = w.approve(ADMIN, &c).await;
    assert_eq!(s, StatusCode::FORBIDDEN, "someone not named cannot");
    let (s, _) = w.approve(PROPOSER, &c).await;
    assert_eq!(s, StatusCode::FORBIDDEN, "nor the proposer");
}

// 4 and scenario 4: the officer lists proposals from workspaces they are not a member of, with reasons and no record values.
#[tokio::test]
async fn an_officer_lists_the_proposals_they_may_decide_from_workspaces_they_are_not_in() {
    let w = world(&[H1, H2], &[PROPOSER]).await;
    w.require_officer();
    let p = w.propose(PROPOSER, false).await;
    let id = p["proposal_id"].as_str().unwrap();

    let (s, list) = send(&w.app, "GET", "/api/v1/me/proposals/to-decide", H1, None).await;
    assert_eq!(s, StatusCode::OK, "{list}");
    let item = list["proposals"].as_array().unwrap().iter().find(|x| x["proposal_id"] == id).unwrap_or_else(|| panic!("listed: {list}"));
    assert_eq!(item["path"], "approver");
    assert!(item["reasons"].as_array().is_some_and(|r| !r.is_empty()), "{item}");
    assert_eq!(item["proposed_by"], PROPOSER);
    let text = item.to_string();
    for forbidden in ["manifest", "ferpa_sensitive", "\"fields\"", "\"data\""] {
        assert!(!text.contains(forbidden), "no record or app content in the list: {forbidden} in {text}");
    }
    // Not for the proposer, a workspace owner outside the approver set, or someone with no part in it.
    for who in [PROPOSER, ADMIN, OUTSIDER] {
        let (_, other) = send(&w.app, "GET", "/api/v1/me/proposals/to-decide", who, None).await;
        assert!(!other["proposals"].as_array().unwrap().iter().any(|x| x["proposal_id"] == id), "{who} should not see it: {other}");
    }
    // The same through the tool.
    let r = call(&w.app, H1, "list_proposals", json!({ "to_decide": true })).await;
    assert!(r["isError"] != true, "{r}");
    assert!(r["structuredContent"]["proposals"].as_array().unwrap().iter().any(|x| x["proposal_id"] == id && x["reasons"].is_array()), "{r}");
    // A list for one workspace still works for someone who can read it, and not for someone who cannot.
    let r = call(&w.app, PROPOSER, "list_proposals", json!({ "workspace_id": w.ws })).await;
    assert!(r["isError"] != true, "{r}");
    let r = call(&w.app, OUTSIDER, "list_proposals", json!({ "workspace_id": w.ws })).await;
    assert_eq!(r["isError"], true, "{r}");
}

#[tokio::test]
async fn on_the_built_in_path_a_second_owner_lists_what_they_may_decide_and_the_proposer_does_not() {
    let w = world(&[], &[PROPOSER]).await;
    let p = w.propose(PROPOSER, false).await;
    let id = p["proposal_id"].as_str().unwrap();
    let (_, mine) = send(&w.app, "GET", "/api/v1/me/proposals/to-decide", PROPOSER, None).await;
    assert!(!mine["proposals"].as_array().unwrap().iter().any(|x| x["proposal_id"] == id), "{mine}");
    let (_, owner) = send(&w.app, "GET", "/api/v1/me/proposals/to-decide", ADMIN, None).await;
    assert!(owner["proposals"].as_array().unwrap().iter().any(|x| x["proposal_id"] == id), "{owner}");
    let anon = w.app.clone().oneshot(Request::builder().uri("/api/v1/me/proposals/to-decide").body(Body::empty()).unwrap()).await.unwrap();
    assert_eq!(anon.status(), StatusCode::UNAUTHORIZED);
    let _ = &w.unit;
}
