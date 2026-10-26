//! Decision authority phase 1: tiers, the matrix, and one pure function. The answer decides who may approve a proposal.

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

async fn tool(app: &Router, who: &str, name: &str, args: Value) -> Value {
    let payload = json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": { "name": name, "arguments": args } });
    let (_, v) = send(app, "POST", "/api/mcp", who, Some(payload)).await;
    assert!(v["result"]["isError"] != true, "{name}: {v}");
    v["result"]["structuredContent"].clone()
}

struct World {
    db: TestDb,
    app: Router,
    unit: String,
    ws: String,
    position: String,
}

/// A department whose `records-officer` position is held by `holders`, and a workspace in it. The Platform Admin owns
/// the workspace. `PROPOSER` and every holder are editors in it.
async fn world(holders: &[&str]) -> World {
    let db = TestDb::new();
    let app = build_app_with_state(db.state()).expect("router");
    for who in [PROPOSER, H1, H2, OUTSIDER] {
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
    let (s, ws) = send(&app, "POST", "/api/v1/workspaces", ADMIN, Some(json!({ "name": code, "code": code, "visibility": "restricted", "department": "physics", "organization_id": unit }))).await;
    assert_eq!(s, StatusCode::CREATED, "{ws}");
    let ws = ws["id"].as_str().unwrap().to_string();
    for who in [PROPOSER, H1, H2] {
        let (s, b) = send(&app, "POST", &format!("/api/v1/workspaces/{ws}/collaborators"), ADMIN, Some(json!({ "eppn": who, "role": "editor" }))).await;
        assert_eq!(s, StatusCode::CREATED, "{b}");
    }
    World { db, app, unit, ws, position }
}

impl World {
    /// A row at `org` that makes Moderate and High changes need the officer position.
    fn require_officer_at(&self, org: &str) {
        let (org, spec) = (Uuid::parse_str(org).unwrap(), json!({ "Position": { "key": self.position, "from": "Workspace", "walk_up": false } }));
        self.db.run(move |c| {
            c.execute("INSERT INTO authority_rows (id, organization_id, tier, path, approver) VALUES ($1, $2, 'Moderate', 'approver', $3)", &[&Uuid::new_v4(), &org, &spec]).unwrap();
        });
    }

    /// A proposal whose manifest has a sensitive field, so `sensitivity` is raised. Returns its id and digest.
    async fn propose_sensitive(&self, who: &str) -> (String, String) {
        let slug = format!("auth-{}", &Uuid::new_v4().to_string()[..8]);
        let f = |n: &str, s: bool| json!({ "name": n, "label": n, "field_type": "Text", "required": false, "ferpa_sensitive": s });
        let manifest = json!({
            "slug": slug, "title": "Authority", "description": "d", "organization_code": "PHYS", "department": "physics",
            "workspace_id": self.ws, "views": [], "pages": [],
            "tables": [{ "id": "t", "name": "T", "slug": "t", "fields": [f("title", false), f("ssn", true)] }]
        });
        let p = tool(&self.app, who, "propose_app_change", json!({ "workspace_id": self.ws, "manifest": manifest, "summary": "An app" })).await;
        (p["proposal_id"].as_str().unwrap().to_string(), p["digest"].as_str().unwrap().to_string())
    }

    async fn approve(&self, who: &str, id: &str, digest: &str) -> (StatusCode, Value) {
        send(&self.app, "POST", &format!("/api/v1/proposals/{id}/decision"), who, Some(json!({ "decision": "approve", "rationale": "Reviewed", "digest": digest }))).await
    }

    fn status_of(&self, id: &str) -> String {
        let id: Uuid = id.parse().unwrap();
        self.db.run(move |c| c.query_one("SELECT status FROM app_proposals WHERE id = $1", &[&id]).unwrap().get(0))
    }
}

// The proposal carries what it needs, so the builder can be told why.
#[tokio::test]
async fn a_proposal_carries_its_tier_path_and_reasons() {
    let w = world(&[H1, H2]).await;
    let (id, _) = w.propose_sensitive(PROPOSER).await;
    let (_, p) = send(&w.app, "GET", &format!("/api/v1/proposals/{id}"), ADMIN, None).await;
    assert_eq!(p["authority"]["tier"], "Moderate", "{p}");
    assert_eq!(p["authority"]["path"], "second_person", "{p}");
    assert!(p["authority"]["reasons"].as_array().unwrap().iter().any(|r| r.as_str().unwrap().contains("sensitive")), "{p}");

    w.require_officer_at(ROOT);
    let (id, _) = w.propose_sensitive(PROPOSER).await;
    let (_, p) = send(&w.app, "GET", &format!("/api/v1/proposals/{id}"), ADMIN, None).await;
    assert_eq!(p["authority"]["path"], "approver", "{p}");
    assert_eq!(p["authority"]["approver"]["Position"]["key"], w.position.as_str(), "{p}");
    let who: Vec<&str> = p["authority"]["resolved"].as_array().unwrap().iter().map(|r| r["eppn"].as_str().unwrap()).collect();
    assert!(who.contains(&H1) && who.contains(&H2), "{p}");
    assert!(p["authority"]["reasons"].as_array().unwrap().iter().any(|r| r.as_str().unwrap().contains("requires a named approver")), "{p}");
}

// 6. On the approver path only the resolved people decide, and never the proposer.
#[tokio::test]
async fn on_the_approver_path_only_a_resolved_person_who_did_not_propose_may_decide() {
    let w = world(&[H1, H2]).await;
    w.require_officer_at(ROOT);
    let (id, digest) = w.propose_sensitive(H1).await;

    let (s, body) = w.approve(ADMIN, &id, &digest).await;
    assert_eq!(s, StatusCode::FORBIDDEN, "a workspace owner who is not in the set: {body}");
    let text = body.to_string();
    assert!(text.contains(H2), "the refusal names who can decide: {text}");
    assert!(!text.contains(H1), "and not the proposer: {text}");

    let (s, body) = w.approve(H1, &id, &digest).await;
    assert_eq!(s, StatusCode::FORBIDDEN, "a resolved holder who proposed it: {body}");

    let (s, _) = w.approve(OUTSIDER, &id, &digest).await;
    assert_eq!(s, StatusCode::FORBIDDEN, "someone with no part in it learns nothing");
    assert_eq!(w.status_of(&id), "Pending");

    let (s, body) = w.approve(H2, &id, &digest).await;
    assert_eq!(s, StatusCode::OK, "a resolved holder who did not propose: {body}");
    assert_eq!(w.status_of(&id), "Approved");
}

// 7. A vacant position: no_approver, listed as Unassigned, and still Pending.
#[tokio::test]
async fn a_vacant_approver_position_leaves_the_proposal_unassigned_and_pending() {
    let w = world(&[]).await;
    w.require_officer_at(ROOT);
    let (id, digest) = w.propose_sensitive(PROPOSER).await;
    let (_, p) = send(&w.app, "GET", &format!("/api/v1/proposals/{id}"), ADMIN, None).await;
    assert_eq!(p["authority"]["no_approver"], true, "{p}");

    let (s, listed) = send(&w.app, "GET", "/api/v1/admin/proposals?unassigned=true", ADMIN, None).await;
    assert_eq!(s, StatusCode::OK, "{listed}");
    assert!(listed["proposals"].as_array().unwrap().iter().any(|x| x["proposal_id"] == id.as_str()), "{listed}");

    for who in [ADMIN, H1, OUTSIDER] {
        let (s, body) = w.approve(who, &id, &digest).await;
        assert_eq!(s, StatusCode::FORBIDDEN, "{who}: {body}");
    }
    assert_eq!(w.status_of(&id), "Pending");

    // Once the position has a holder, it is no longer unassigned.
    let (s, b) = send(&w.app, "POST", &format!("/api/v1/orgs/{}/positions/{}/holders", w.unit, w.position), ADMIN, Some(json!({ "eppn": H2, "replace": false, "reason": "Fill" }))).await;
    assert_eq!(s, StatusCode::CREATED, "{b}");
    let (_, listed) = send(&w.app, "GET", "/api/v1/admin/proposals?unassigned=true", ADMIN, None).await;
    assert!(!listed["proposals"].as_array().unwrap().iter().any(|x| x["proposal_id"] == id.as_str()), "{listed}");
    let (s, _) = w.approve(H2, &id, &digest).await;
    assert_eq!(s, StatusCode::OK);
}

#[tokio::test]
async fn the_unassigned_list_is_for_a_platform_admin() {
    let w = world(&[]).await;
    let (s, _) = send(&w.app, "GET", "/api/v1/admin/proposals?unassigned=true", OUTSIDER, None).await;
    assert_eq!(s, StatusCode::FORBIDDEN);
}

// 8. The matrix is tightened after the proposal is made and before it is decided. The decision uses the new answer.
#[tokio::test]
async fn a_matrix_tightened_before_the_decision_governs_the_decision() {
    let w = world(&[H1, H2]).await;
    let (id, digest) = w.propose_sensitive(PROPOSER).await;
    let (_, p) = send(&w.app, "GET", &format!("/api/v1/proposals/{id}"), ADMIN, None).await;
    assert_eq!(p["authority"]["path"], "second_person", "when it was made, a second owner would have done");
    w.require_officer_at(&w.unit.clone());
    let (s, body) = w.approve(ADMIN, &id, &digest).await;
    assert_eq!(s, StatusCode::FORBIDDEN, "the owner no longer suffices: {body}");
    assert_eq!(w.status_of(&id), "Pending");
    let (s, body) = w.approve(H1, &id, &digest).await;
    assert_eq!(s, StatusCode::OK, "{body}");
}

// A looser row never loosens. With no row at all, a second owner decides, as before.
#[tokio::test]
async fn with_no_rows_a_second_owner_decides_and_the_proposer_cannot() {
    let w = world(&[]).await;
    let (id, digest) = w.propose_sensitive(PROPOSER).await;
    let (s, _) = w.approve(PROPOSER, &id, &digest).await;
    assert_eq!(s, StatusCode::FORBIDDEN, "the proposer never decides a flagged change");
    let unit = w.unit.clone();
    let uid = Uuid::parse_str(&unit).unwrap();
    w.db.run(move |c| {
        c.execute("INSERT INTO authority_rows (id, organization_id, tier, path) VALUES ($1, $2, 'Moderate', 'direct')", &[&Uuid::new_v4(), &uid]).unwrap();
    });
    let (s, _) = w.approve(PROPOSER, &id, &digest).await;
    assert_eq!(s, StatusCode::FORBIDDEN, "a row cannot loosen the floor");
    let (s, body) = w.approve(ADMIN, &id, &digest).await;
    assert_eq!(s, StatusCode::OK, "{body}");
}

// 9. Every flagged check has a sentence, and the server's list is the core's.
#[test]
fn the_flagged_checks_are_the_cores_and_each_has_a_sentence() {
    let core: Vec<&str> = scaffoldry_core::authority::FLAGGED.iter().map(|(n, _)| *n).collect();
    let server: Vec<&str> = scaffoldry_server::repository_proposals::FLAGGED_CHECKS.to_vec();
    assert_eq!(core, server);
}

// 10. approve_proposal is called from one place, and that place asks the requirement first.
#[test]
fn approve_proposal_is_called_from_one_place_and_that_place_asks_what_is_required_first() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut calls = Vec::new();
    let mut stack = vec![root];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(dir).unwrap().flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().is_some_and(|e| e == "rs") {
                let text = std::fs::read_to_string(&path).unwrap();
                for (i, line) in text.lines().enumerate() {
                    if line.contains(".approve_proposal(") {
                        calls.push((path.clone(), i, text.clone()));
                    }
                }
            }
        }
    }
    assert_eq!(calls.len(), 1, "approve_proposal is called from {} places", calls.len());
    let (path, line, text) = &calls[0];
    assert!(path.ends_with("service/proposals.rs"), "{path:?}");
    let before: String = text.lines().take(*line).collect::<Vec<_>>().join("\n");
    let decide_start = before.rfind("pub fn decide(").expect("the call is inside decide");
    assert!(before[decide_start..].contains("authority::requirement_for("), "decide asks for the requirement before it applies anything");
}

// A proposer may withdraw their own proposal, even as an editor who could not decide anyone's.
#[tokio::test]
async fn a_proposer_may_withdraw_their_own_proposal() {
    let w = world(&[]).await;
    let (id, _) = w.propose_sensitive(PROPOSER).await;
    let (s, body) = send(&w.app, "POST", &format!("/api/v1/proposals/{id}/decision"), PROPOSER, Some(json!({ "decision": "reject", "rationale": "Changed my mind" }))).await;
    assert_eq!(s, StatusCode::OK, "{body}");
    assert_eq!(w.status_of(&id), "Rejected");
    // Another editor cannot reject it.
    let (id, _) = w.propose_sensitive(PROPOSER).await;
    let (s, _) = send(&w.app, "POST", &format!("/api/v1/proposals/{id}/decision"), H1, Some(json!({ "decision": "reject", "rationale": "No" }))).await;
    assert_eq!(s, StatusCode::FORBIDDEN);
}
