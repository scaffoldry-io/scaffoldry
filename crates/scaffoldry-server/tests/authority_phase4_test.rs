//! Decision authority phase 4: the load on real decisions. Counts, names of approvers, and times. Never content.

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
const ADMIN2: &str = "second.admin@state.edu";
const UNIT_ADMIN: &str = "physics.admin@state.edu";
const H1: &str = "holder.one@state.edu";
const H2: &str = "holder.two@state.edu";
const FACULTY: &str = "prof.curie@science.state.edu";
const ROOT: &str = "00000000-0000-0000-0000-000000000001";
const PROPOSER: &str = "prop.oser@state.edu";

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

struct World {
    db: TestDb,
    app: Router,
    workspace: String,
}

/// root > college > physics, a workspace in physics, and a position at the college that `holders` fill. A row at the
/// college sends every Moderate change to that position.
async fn world(holders: &[&str]) -> World {
    let db = TestDb::new();
    let state = db.state();
    let app = build_app_with_state(state).expect("router");
    for who in [ADMIN2, UNIT_ADMIN, PROPOSER, H1, H2, FACULTY] {
        issue_test_token_and_user(who);
    }
    let tag = Uuid::new_v4().simple().to_string()[..8].to_string();
    let mut ids = Vec::new();
    let mut parent = ROOT.to_string();
    for (name, kind) in [("Col", "College"), ("Phy", "Department")] {
        let (s, u) = send(&app, "POST", "/api/v1/orgs", ADMIN, Some(json!({ "name": format!("{name}{tag}"), "code": format!("{name}{tag}"), "org_type": kind, "parent_id": parent }))).await;
        assert_eq!(s, StatusCode::CREATED, "{u}");
        parent = u["id"].as_str().unwrap().to_string();
        ids.push(parent.clone());
    }
    let (college, physics) = (ids[0].clone(), ids[1].clone());
    let (s, w) = send(&app, "POST", "/api/v1/workspaces", ADMIN, Some(json!({ "name": format!("w{tag}"), "code": format!("w{tag}"), "visibility": "restricted", "department": "physics", "organization_id": physics }))).await;
    assert_eq!(s, StatusCode::CREATED, "{w}");
    let workspace = w["id"].as_str().unwrap().to_string();
    let position = format!("officer-{tag}");
    let (s, p) = send(&app, "POST", "/api/v1/admin/positions", ADMIN, Some(json!({ "key": position, "name": "Officer", "description": "Test", "org_types": ["College"], "max_holders": 5, "reason": "Define" }))).await;
    assert_eq!(s, StatusCode::CREATED, "{p}");
    for h in holders {
        let (s, b) = send(&app, "POST", &format!("/api/v1/orgs/{college}/positions/{position}/holders"), ADMIN, Some(json!({ "eppn": h, "replace": false, "reason": "Assign" }))).await;
        assert_eq!(s, StatusCode::CREATED, "{b}");
    }
    let (s, b) = send(&app, "POST", &format!("/api/v1/orgs/{ROOT}/appointments"), ADMIN, Some(json!({ "eppn": ADMIN2, "scoped_affiliation": "platform_admin", "reason": "Second admin" }))).await;
    assert_eq!(s, StatusCode::CREATED, "{b}");
    let (s, b) = send(&app, "POST", &format!("/api/v1/orgs/{physics}/appointments"), ADMIN, Some(json!({ "eppn": UNIT_ADMIN, "scoped_affiliation": "unit_admin", "reason": "Physics admin" }))).await;
    assert_eq!(s, StatusCode::CREATED, "{b}");
    // Every Moderate change at the college goes to the position.
    let change = json!({ "kind": "row", "row": { "tier": "Moderate", "path": "approver", "approver": { "Position": { "key": position, "from": "Workspace", "walk_up": true } } } });
    let (s, p) = send(&app, "POST", "/api/v1/admin/authority/changes", ADMIN, Some(json!({ "organization_id": college, "change": change, "reason": "Tighten the rule" }))).await;
    assert_eq!(s, StatusCode::CREATED, "{p}");
    let id = p["change"]["id"].as_str().unwrap();
    let (s, b) = send(&app, "POST", &format!("/api/v1/admin/authority/changes/{id}/decide"), ADMIN2, Some(json!({ "decision": "confirm", "note": "Reviewed" }))).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    World { db, app, workspace }
}

impl World {
    /// `count` proposals made `days_ago` days ago. `raised` fails `sensitivity`, which is Moderate and so goes to the position.
    fn seed(&self, count: i32, days_ago: i32, raised: bool, summary: &'static str) {
        let ws = self.workspace.clone();
        let outcome = if raised { "fail" } else { "pass" };
        let checks = json!({ "checks": [{ "name": "sensitivity", "outcome": outcome, "messages": ["SECRET-MESSAGE"] }], "changes": [] });
        self.db.run(move |c| {
            c.execute(
                "INSERT INTO app_proposals (id, workspace_id, app_slug, manifest, checks, summary, proposed_by, status, created_at) \
                 SELECT gen_random_uuid(), $1, 'seed', '{\"secret\": \"SECRET-FIELD\"}', $2, $3, $4, 'Pending', NOW() - make_interval(days => $5, hours => 1) FROM generate_series(1, $6)",
                &[&ws, &checks, &summary, &PROPOSER, &days_ago, &count],
            )
            .unwrap();
        });
    }

    async fn load(&self, who: &str) -> (StatusCode, Value) {
        send(&self.app, "GET", "/api/v1/admin/authority/load", who, None).await
    }
}

// 1. Paths, and the load each approver carries in each week, from stored proposals and the matrix as it is now.
#[tokio::test]
async fn paths_and_items_per_approver_per_week_come_from_stored_proposals() {
    let w = world(&[H1, H2]).await;
    w.seed(3, 1, true, "This week");
    w.seed(1, 15, true, "Two weeks ago");
    w.seed(2, 3, false, "Nothing raised");
    w.seed(5, 120, true, "Outside the window");
    let (s, b) = w.load(ADMIN).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_eq!(b["window_days"], 90);
    assert_eq!(b["proposals"], 6, "{b}");
    assert_eq!(b["paths"], json!({ "direct": 2, "second_person": 0, "approver": 4 }), "{b}");
    assert_eq!(b["unassigned"], 0);
    let approvers = b["approvers"].as_array().unwrap();
    assert_eq!(approvers.len(), 2, "{b}");
    for a in approvers {
        // Index 0 is the last seven days. Thirteen weeks cover the window, quiet ones included.
        assert_eq!(a["weeks"], json!([3, 0, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]), "{a}");
        // Sorted: eleven zeros, 1, 3. Nearest rank: the 7th is 0, the 13th is 3.
        assert_eq!((a["median"].as_u64(), a["p95"].as_u64(), a["largest"].as_u64()), (Some(0), Some(3), Some(3)), "{a}");
    }
    let names: Vec<&str> = approvers.iter().map(|a| a["eppn"].as_str().unwrap()).collect();
    assert!(names.contains(&H1) && names.contains(&H2), "{names:?}");
}

// 2. A proposal nobody can decide is counted, and counted as unassigned.
#[tokio::test]
async fn a_proposal_with_no_one_to_decide_it_is_unassigned() {
    let w = world(&[]).await;
    w.seed(4, 2, true, "Nobody holds the position");
    w.seed(1, 2, false, "Direct");
    let (s, b) = w.load(ADMIN).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_eq!(b["paths"]["approver"], 4, "{b}");
    assert_eq!(b["unassigned"], 4, "{b}");
    assert_eq!(b["approvers"], json!([]), "{b}");
}

// 3. 500 decided proposals with known times give hand-computed answers.
#[tokio::test]
async fn the_median_and_95th_percentile_of_the_time_to_decide_match_the_hand_count() {
    let w = world(&[H1]).await;
    let ws = w.workspace.clone();
    w.db.run(move |c| {
        // The i-th proposal was decided i minutes after it was made, ten days ago.
        c.execute(
            "INSERT INTO app_proposals (id, workspace_id, app_slug, manifest, checks, summary, proposed_by, status, decided_by, created_at, decided_at) \
             SELECT gen_random_uuid(), $1, 'seed', '{}', '{\"checks\": [], \"changes\": []}', 'Seed', 'prop.oser@state.edu', 'Approved', 'x@y.edu', \
                    NOW() - INTERVAL '10 days', NOW() - INTERVAL '10 days' + make_interval(mins => g) FROM generate_series(1, 500) g",
            &[&ws],
        )
        .unwrap();
        // Not counted: decided long ago, and still waiting.
        c.execute(
            "INSERT INTO app_proposals (id, workspace_id, app_slug, manifest, checks, summary, proposed_by, status, decided_by, created_at, decided_at) \
             VALUES (gen_random_uuid(), $1, 'seed', '{}', '{\"checks\": [], \"changes\": []}', 'Old', 'p@y.edu', 'Approved', 'x@y.edu', NOW() - INTERVAL '200 days', NOW() - INTERVAL '100 days')",
            &[&ws],
        )
        .unwrap();
    });
    w.seed(1, 1, false, "Still waiting");
    let (s, b) = w.load(ADMIN).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_eq!(b["decision_time"]["decided"], 500, "{b}");
    // Nearest rank over 500 values of 60, 120, ... 30000 seconds: the 250th and the 475th.
    assert_eq!(b["decision_time"]["median_seconds"], 250 * 60, "{b}");
    assert_eq!(b["decision_time"]["p95_seconds"], 475 * 60, "{b}");
}

#[tokio::test]
async fn with_nothing_decided_the_times_are_null() {
    let w = world(&[H1]).await;
    let (s, b) = w.load(ADMIN).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_eq!(b["decision_time"], json!({ "decided": 0, "median_seconds": null, "p95_seconds": null }), "{b}");
    assert_eq!(b["paths"], json!({ "direct": 0, "second_person": 0, "approver": 0 }));
}

// 4. Counts, names of approvers, and times. Never a record value or any proposal content.
#[tokio::test]
async fn the_answer_holds_no_content_of_any_proposal() {
    let w = world(&[H1]).await;
    w.seed(2, 1, true, "SECRET-SUMMARY");
    let (s, b) = w.load(ADMIN).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    let text = b.to_string();
    for secret in ["SECRET-SUMMARY", "SECRET-FIELD", "SECRET-MESSAGE", PROPOSER, "seed"] {
        assert!(!text.contains(secret), "{secret} leaked: {text}");
    }
    let mut keys: Vec<&str> = b.as_object().unwrap().keys().map(String::as_str).collect();
    keys.sort_unstable();
    assert_eq!(keys, ["approvers", "decision_time", "paths", "proposals", "unassigned", "window_days"]);
    let mut keys: Vec<&str> = b["approvers"][0].as_object().unwrap().keys().map(String::as_str).collect();
    keys.sort_unstable();
    assert_eq!(keys, ["display_name", "eppn", "largest", "median", "p95", "weeks"]);
}

// 5. Platform Admin or compliance, and no one else.
#[tokio::test]
async fn only_a_platform_admin_or_compliance_reads_the_load() {
    let w = world(&[H1]).await;
    for who in [FACULTY, UNIT_ADMIN, PROPOSER] {
        let (s, b) = w.load(who).await;
        assert_eq!(s, StatusCode::FORBIDDEN, "{who}: {b}");
    }
    let (s, _) = w.load(ADMIN2).await;
    assert_eq!(s, StatusCode::OK);
    let (s, _) = send(&w.app, "GET", "/api/v1/admin/authority/load", "nobody@nowhere.edu", None).await;
    assert!(s == StatusCode::FORBIDDEN || s == StatusCode::UNAUTHORIZED, "{s}");
}
