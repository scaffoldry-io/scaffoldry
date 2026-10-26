//! Decision authority phase 2: the matrix in the console, with its effect shown first.

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::Router;
use common::TestDb;
use http_body_util::BodyExt;
use scaffoldry_server::build_app_with_state;
use scaffoldry_server::service::identity::{create_setup_token, issue_test_token_and_user};
use serde_json::{json, Value};
use tower::ServiceExt;
use uuid::Uuid;

const ADMIN: &str = "jordan.lee@state.edu";
const ADMIN2: &str = "second.admin@state.edu";
const PHYS_ADMIN: &str = "physics.admin@state.edu";
const PROPOSER: &str = "prop.oser@state.edu";
const H1: &str = "holder.one@state.edu";
const H2: &str = "holder.two@state.edu";
const FACULTY: &str = "prof.curie@science.state.edu";
const ROOT: &str = "00000000-0000-0000-0000-000000000001";

async fn send(app: &Router, method: &str, uri: &str, who: &str, body: Option<Value>) -> (StatusCode, Value) {
    send_token(app, method, uri, &issue_test_token_and_user(who), body).await
}

async fn send_token(app: &Router, method: &str, uri: &str, token: &str, body: Option<Value>) -> (StatusCode, Value) {
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

struct World {
    db: TestDb,
    app: Router,
    state: scaffoldry_server::state::SharedState,
    college: String,
    physics: String,
    chemistry: String,
    ws_physics: String,
    ws_chemistry: String,
    position: String,
}

async fn unit(app: &Router, name: &str, parent: &str) -> String {
    let (s, u) = send(app, "POST", "/api/v1/orgs", ADMIN, Some(json!({ "name": name, "code": name, "org_type": "Department", "parent_id": parent }))).await;
    assert_eq!(s, StatusCode::CREATED, "{u}");
    u["id"].as_str().unwrap().to_string()
}

async fn workspace(app: &Router, name: &str, unit: &str) -> String {
    let (s, w) = send(app, "POST", "/api/v1/workspaces", ADMIN, Some(json!({ "name": name, "code": name, "visibility": "restricted", "department": "physics", "organization_id": unit }))).await;
    assert_eq!(s, StatusCode::CREATED, "{w}");
    w["id"].as_str().unwrap().to_string()
}

/// root > college > (physics, chemistry), a workspace in each, and a position the college's holders fill.
async fn world(holders: &[&str]) -> World {
    let db = TestDb::new();
    let state = db.state();
    let app = build_app_with_state(state.clone()).expect("router");
    for who in [ADMIN2, PHYS_ADMIN, PROPOSER, H1, H2, FACULTY] {
        issue_test_token_and_user(who);
    }
    let tag = Uuid::new_v4().simple().to_string()[..8].to_string();
    let college = unit(&app, &format!("Col{tag}"), ROOT).await;
    let physics = unit(&app, &format!("Phy{tag}"), &college).await;
    let chemistry = unit(&app, &format!("Che{tag}"), &college).await;
    let ws_physics = workspace(&app, &format!("wp{tag}"), &physics).await;
    let ws_chemistry = workspace(&app, &format!("wc{tag}"), &chemistry).await;
    let position = format!("officer-{tag}");
    let (s, p) = send(&app, "POST", "/api/v1/admin/positions", ADMIN, Some(json!({ "key": position, "name": "Officer", "description": "Test", "org_types": ["Department"], "max_holders": 5, "reason": "Define" }))).await;
    assert_eq!(s, StatusCode::CREATED, "{p}");
    for h in holders {
        let (s, b) = send(&app, "POST", &format!("/api/v1/orgs/{college}/positions/{position}/holders"), ADMIN, Some(json!({ "eppn": h, "replace": false, "reason": "Assign" }))).await;
        assert_eq!(s, StatusCode::CREATED, "{b}");
    }
    // A second Platform Admin, and an Org Unit Admin of physics.
    let (s, b) = send(&app, "POST", &format!("/api/v1/orgs/{ROOT}/appointments"), ADMIN, Some(json!({ "eppn": ADMIN2, "scoped_affiliation": "platform_admin", "reason": "Second admin" }))).await;
    assert_eq!(s, StatusCode::CREATED, "{b}");
    let (s, b) = send(&app, "POST", &format!("/api/v1/orgs/{physics}/appointments"), ADMIN, Some(json!({ "eppn": PHYS_ADMIN, "scoped_affiliation": "unit_admin", "reason": "Physics admin" }))).await;
    assert_eq!(s, StatusCode::CREATED, "{b}");
    World { db, app, state, college, physics, chemistry, ws_physics, ws_chemistry, position }
}

impl World {
    fn officer(&self) -> Value {
        json!({ "Position": { "key": self.position, "from": "Workspace", "walk_up": true } })
    }

    fn row_change(&self, tier: &str, path: &str) -> Value {
        let mut row = json!({ "tier": tier, "path": path });
        if path == "approver" {
            row["approver"] = self.officer();
        }
        json!({ "kind": "row", "row": row })
    }

    async fn propose(&self, who: &str, unit: &str, change: Value) -> (StatusCode, Value) {
        send(&self.app, "POST", "/api/v1/admin/authority/changes", who, Some(json!({ "organization_id": unit, "change": change, "reason": "Tighten the rule" }))).await
    }

    async fn decide(&self, who: &str, id: &str, decision: &str) -> (StatusCode, Value) {
        send(&self.app, "POST", &format!("/api/v1/admin/authority/changes/{id}/decide"), who, Some(json!({ "decision": decision, "note": "Reviewed" }))).await
    }

    fn count(&self, table: &'static str) -> i64 {
        self.db.run(move |c| c.query_one(&format!("SELECT COUNT(*) FROM {table}"), &[]).unwrap().get(0))
    }
}

// 1. A change one Platform Admin proposes is not in force until another confirms it.
#[tokio::test]
async fn a_change_is_not_in_force_until_a_different_person_confirms_it() {
    let w = world(&[H1]).await;
    let (s, p) = w.propose(ADMIN, &w.college, w.row_change("Moderate", "approver")).await;
    assert_eq!(s, StatusCode::CREATED, "{p}");
    let id = p["change"]["id"].as_str().unwrap().to_string();
    assert_eq!(p["change"]["status"], "Pending");
    assert_eq!(w.count("authority_rows"), 0, "proposing applies nothing");

    let (s, b) = w.decide(ADMIN, &id, "confirm").await;
    assert_eq!(s, StatusCode::FORBIDDEN, "the proposer cannot confirm: {b}");
    assert_eq!(w.count("authority_rows"), 0);

    let (s, b) = w.decide(ADMIN2, &id, "confirm").await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_eq!(b["change"]["status"], "Confirmed");
    assert_eq!(w.count("authority_rows"), 1);
    let (_, shown) = send(&w.app, "GET", &format!("/api/v1/admin/authority?organization_id={}", w.physics), ADMIN, None).await;
    let moderate = shown["tiers"].as_array().unwrap().iter().find(|t| t["tier"] == "Moderate").unwrap();
    assert_eq!(moderate["path"], "approver", "{shown}");
    assert_eq!(moderate["source"]["organization_id"], w.college.as_str(), "{shown}");
    let (s, again) = w.decide(ADMIN2, &id, "confirm").await;
    assert_eq!(s, StatusCode::CONFLICT, "a decided change is not decided twice: {again}");
}

#[tokio::test]
async fn the_setup_identity_counts_as_a_different_person() {
    let w = world(&[H1]).await;
    let (s, p) = w.propose(ADMIN, &w.college, w.row_change("Moderate", "second_person")).await;
    assert_eq!(s, StatusCode::CREATED, "{p}");
    let id = p["change"]["id"].as_str().unwrap().to_string();
    // The setup identity exists only while no Platform Admin is appointed. Take the appointments away.
    w.db.run(|c| c.batch_execute("DELETE FROM roles WHERE scoped_affiliation = 'platform_admin'").unwrap_or(()));
    let token = create_setup_token(&w.state).expect("a setup token");
    let (s, b) = send_token(&w.app, "POST", &format!("/api/v1/admin/authority/changes/{id}/decide"), &token, Some(json!({ "decision": "confirm", "note": "Setup" }))).await;
    assert_eq!(s, StatusCode::OK, "{b}");
}

// 2. An Org Unit Admin proposes for their own unit and no other.
#[tokio::test]
async fn an_org_unit_admin_may_propose_for_their_unit_and_not_for_another_or_the_root() {
    let w = world(&[H1]).await;
    let (s, b) = w.propose(PHYS_ADMIN, &w.physics, w.row_change("Moderate", "approver")).await;
    assert_eq!(s, StatusCode::CREATED, "{b}");
    let (s, _) = w.propose(PHYS_ADMIN, &w.chemistry, w.row_change("Moderate", "approver")).await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    let (s, _) = w.propose(PHYS_ADMIN, ROOT, w.row_change("Moderate", "approver")).await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    let (s, _) = w.propose(FACULTY, &w.physics, w.row_change("Moderate", "approver")).await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    // They read their own unit and not another.
    let (s, _) = send(&w.app, "GET", &format!("/api/v1/admin/authority?organization_id={}", w.physics), PHYS_ADMIN, None).await;
    assert_eq!(s, StatusCode::OK);
    let (s, _) = send(&w.app, "GET", &format!("/api/v1/admin/authority?organization_id={}", w.chemistry), PHYS_ADMIN, None).await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    // A unit admin of a unit above may confirm a change for a unit below, and the proposer may not.
    let (_, p) = w.propose(PHYS_ADMIN, &w.physics, w.row_change("High", "second_person")).await;
    let id = p["change"]["id"].as_str().unwrap().to_string();
    let (s, _) = w.decide(PHYS_ADMIN, &id, "confirm").await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    let (s, b) = w.decide(ADMIN2, &id, "confirm").await;
    assert_eq!(s, StatusCode::OK, "{b}");
}

// 3. A change that loosens the built-in answer is refused.
#[tokio::test]
async fn a_change_that_would_loosen_the_built_in_answer_is_refused() {
    let w = world(&[H1]).await;
    for change in [
        json!({ "kind": "signal", "check_name": "sensitivity", "tier": "Low" }),
        w.row_change("Moderate", "direct"),
        w.row_change("High", "direct"),
    ] {
        let (s, b) = w.propose(ADMIN, &w.college, change).await;
        assert_eq!(s, StatusCode::BAD_REQUEST, "{b}");
        assert_eq!(b["code"], "authority_loosens", "{b}");
    }
    assert_eq!(w.count("authority_changes"), 0, "nothing was stored");
    // A Low row at direct is the built-in answer: allowed, with no effect.
    let (s, b) = w.propose(ADMIN, &w.college, w.row_change("Low", "direct")).await;
    assert_eq!(s, StatusCode::CREATED, "{b}");
    // Other bad bodies are plain bad requests that name the field.
    let (s, b) = w.propose(ADMIN, &w.college, json!({ "kind": "signal", "check_name": "stale", "tier": "High" })).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{b}");
    assert_ne!(b["code"], "authority_loosens");
    let (s, _) = w.propose(ADMIN, &w.college, json!({ "kind": "row", "row": { "tier": "Moderate", "path": "approver" } })).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "an approver row names an approver");
}

// A row looser than the institution's is stored and has no effect.
#[tokio::test]
async fn a_row_the_institution_already_exceeds_has_no_effect_and_says_so() {
    let w = world(&[H1]).await;
    let (_, p) = w.propose(ADMIN, ROOT, w.row_change("Moderate", "approver")).await;
    w.decide(ADMIN2, p["change"]["id"].as_str().unwrap(), "confirm").await;
    let (_, p) = w.propose(PHYS_ADMIN, &w.physics, w.row_change("Moderate", "second_person")).await;
    let (s, b) = w.decide(ADMIN, p["change"]["id"].as_str().unwrap(), "confirm").await;
    assert_eq!(s, StatusCode::OK, "stored: {b}");
    let (_, shown) = send(&w.app, "GET", &format!("/api/v1/admin/authority?organization_id={}", w.physics), ADMIN, None).await;
    let rows = shown["rows"].as_array().unwrap();
    let unit_row = rows.iter().find(|r| r["organization_id"] == w.physics.as_str()).expect("the unit row is shown");
    assert_eq!(unit_row["has_effect"], false, "{shown}");
    let root_row = rows.iter().find(|r| r["organization_id"] == ROOT).unwrap();
    assert_eq!(root_row["has_effect"], true);
    let moderate = shown["tiers"].as_array().unwrap().iter().find(|t| t["tier"] == "Moderate").unwrap();
    assert_eq!(moderate["source"]["organization_id"], ROOT, "{shown}");
}

fn seed_proposals(w: &World, n: usize, old: usize) {
    let (a, b) = (w.ws_physics.clone(), w.ws_chemistry.clone());
    w.db.run(move |c| {
        for i in 0..n {
            let ws = if i % 2 == 0 { &a } else { &b };
            let raised = i % 4 < 2;
            let name = if raised { "sensitivity" } else { "stale" };
            let outcome = if raised { "fail" } else { "pass" };
            let path = if raised { "second_person" } else { "direct" };
            let checks = json!({ "checks": [{ "name": name, "outcome": outcome, "messages": [] }], "changes": [], "authority": { "path": path } });
            let age = if i < old { "120 days" } else { "1 day" };
            c.execute(
                &format!("INSERT INTO app_proposals (id, workspace_id, app_slug, manifest, checks, summary, proposed_by, status, created_at) VALUES ($1, $2, 'seed', '{{}}', $3, $4, 'prop.oser@state.edu', 'Pending', NOW() - INTERVAL '{age}')"),
                &[&Uuid::new_v4(), ws, &checks, &format!("Seed {i}")],
            )
            .unwrap();
        }
    });
}

// 4. The impact route counts what would change and changes nothing.
#[tokio::test]
async fn the_impact_of_a_change_is_counted_before_and_after_and_nothing_is_stored() {
    let w = world(&[H1, H2]).await;
    seed_proposals(&w, 210, 10);
    // 200 proposals in the window: half raise a flagged check, half raise nothing.
    let tables = ["authority_rows", "authority_signals", "authority_changes", "app_proposals", "governance_ledger"];
    let before: Vec<i64> = tables.iter().map(|t| w.count(t)).collect();
    let (s, impact) = send(&w.app, "POST", "/api/v1/admin/authority/impact", ADMIN, Some(json!({ "organization_id": w.college, "change": w.row_change("Moderate", "approver") }))).await;
    assert_eq!(s, StatusCode::OK, "{impact}");
    assert_eq!(impact["proposals"], 200, "only the last 90 days: {impact}");
    assert_eq!(impact["before"], json!({ "direct": 100, "second_person": 100, "approver": 0 }), "{impact}");
    assert_eq!(impact["after"], json!({ "direct": 100, "second_person": 0, "approver": 100 }), "{impact}");
    let load = impact["load"].as_array().unwrap();
    for h in [H1, H2] {
        let row = load.iter().find(|l| l["eppn"] == h).unwrap_or_else(|| panic!("{h} in {impact}"));
        assert_eq!((row["before"].as_i64(), row["after"].as_i64()), (Some(0), Some(100)), "{row}");
    }
    assert!(impact["unassigned"].as_array().unwrap().is_empty(), "{impact}");
    let after: Vec<i64> = tables.iter().map(|t| w.count(t)).collect();
    assert_eq!(before, after, "the impact route stored nothing");
    // A unit admin gets the same answer for their own unit, counted only below it.
    let (s, own) = send(&w.app, "POST", "/api/v1/admin/authority/impact", PHYS_ADMIN, Some(json!({ "organization_id": w.physics, "change": w.row_change("Moderate", "approver") }))).await;
    assert_eq!(s, StatusCode::OK, "{own}");
    assert_eq!(own["proposals"], 100, "only physics' workspace: {own}");
}

// 5. A vacant position leaves proposals with no approver, and the route names them.
#[tokio::test]
async fn the_impact_names_the_proposals_that_would_have_no_approver() {
    let w = world(&[]).await;
    seed_proposals(&w, 40, 0);
    let (s, impact) = send(&w.app, "POST", "/api/v1/admin/authority/impact", ADMIN, Some(json!({ "organization_id": w.college, "change": w.row_change("Moderate", "approver") }))).await;
    assert_eq!(s, StatusCode::OK, "{impact}");
    let un = impact["unassigned"].as_array().unwrap();
    assert_eq!(un.len(), 20, "{impact}");
    assert!(un[0]["proposal_id"].is_string() && un[0]["summary"].as_str().unwrap().starts_with("Seed"), "{un:?}");
}

// 6. One ledger entry per confirmed change, written before the rows. A failing ledger write changes nothing.
#[tokio::test]
async fn a_confirmed_change_is_one_ledger_entry_written_first_and_a_failed_entry_changes_nothing() {
    let w = world(&[H1]).await;
    let (_, p) = w.propose(ADMIN, &w.college, w.row_change("Moderate", "approver")).await;
    let id = p["change"]["id"].as_str().unwrap().to_string();
    w.db.run(|c| {
        c.batch_execute(
            "CREATE FUNCTION fail_authority_ledger() RETURNS trigger AS $$ BEGIN RAISE EXCEPTION 'no ledger'; END $$ LANGUAGE plpgsql;
             CREATE TRIGGER fail_authority_ledger BEFORE INSERT ON governance_ledger FOR EACH ROW WHEN (NEW.decision_type = 'AuthorityChanged') EXECUTE FUNCTION fail_authority_ledger();",
        )
        .unwrap();
    });
    let (s, _) = w.decide(ADMIN2, &id, "confirm").await;
    assert!(s.is_server_error(), "{s}");
    assert_eq!(w.count("authority_rows"), 0, "no row changed");
    let status: String = w.db.run(|c| c.query_one("SELECT status FROM authority_changes", &[]).unwrap().get(0));
    assert_eq!(status, "Pending");
    w.db.run(|c| c.batch_execute("DROP TRIGGER fail_authority_ledger ON governance_ledger; DROP FUNCTION fail_authority_ledger();").unwrap());

    let (s, b) = w.decide(ADMIN2, &id, "confirm").await;
    assert_eq!(s, StatusCode::OK, "{b}");
    let entries: Vec<Value> = w.db.run(|c| c.query("SELECT payload FROM governance_ledger WHERE decision_type = 'AuthorityChanged'", &[]).unwrap().iter().map(|r| r.get::<_, Value>(0)).collect());
    assert_eq!(entries.len(), 1, "{entries:?}");
    assert_eq!(entries[0]["organization_id"], w.college.as_str());
    assert_eq!(entries[0]["proposed_by"], ADMIN);
    assert_eq!(entries[0]["decided_by"], ADMIN2);
    assert_eq!(entries[0]["change"]["row"]["tier"], "Moderate");
}

#[tokio::test]
async fn rejecting_a_change_applies_nothing_and_the_list_shows_who_decided() {
    let w = world(&[H1]).await;
    let (_, p) = w.propose(ADMIN, &w.college, w.row_change("Moderate", "approver")).await;
    let id = p["change"]["id"].as_str().unwrap().to_string();
    let (s, b) = w.decide(ADMIN2, &id, "reject").await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_eq!(w.count("authority_rows"), 0);
    let (s, list) = send(&w.app, "GET", &format!("/api/v1/admin/authority/changes?organization_id={}", w.college), ADMIN, None).await;
    assert_eq!(s, StatusCode::OK, "{list}");
    let c = &list["changes"][0];
    assert_eq!((c["status"].as_str(), c["proposed_by"].as_str(), c["decided_by"].as_str()), (Some("Rejected"), Some(ADMIN), Some(ADMIN2)), "{list}");
}
