//! Compliance is a role, not a claim from the identity provider.
//!
//! A Platform Admin appoints a compliance officer on the root unit, through the appointment API,
//! and the grant is a ledger entry. A SCIM record that says `compliance@...` grants nothing, so the
//! registry cannot hand out access to the disclosure log or the other compliance routes.

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::Router;
use http_body_util::BodyExt;
use scaffoldry_server::build_app_with_state;
use scaffoldry_server::service::identity::{issue_test_token_and_user, resolve_user};
use scaffoldry_server::state::ServerState;
use serde_json::{json, Value};
use std::sync::Arc;
use tower::ServiceExt;
use uuid::Uuid;

const ADMIN: &str = "jordan.lee@state.edu";
const FACULTY: &str = "prof.curie@science.state.edu";

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

struct Cleanup(String);
impl Drop for Cleanup {
    fn drop(&mut self) {
        let (eppn, url) = (self.0.clone(), db_url());
        let _ = std::thread::spawn(move || {
            if let Ok(mut c) = postgres::Client::connect(&url, postgres::NoTls) {
                let _ = c.execute("DELETE FROM roles WHERE person_id IN (SELECT id FROM persons WHERE LOWER(eppn) = LOWER($1))", &[&eppn]);
            }
        })
        .join();
    }
}

struct World {
    app: Router,
    state: Arc<ServerState>,
    admin: String,
    faculty: String,
    officer: String,
    officer_eppn: String,
    root: Uuid,
    _cleanup: Cleanup,
}

async fn send(app: &Router, method: &str, uri: &str, token: &str, body: Option<Value>) -> (StatusCode, Value) {
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

async fn world() -> World {
    let tag = Uuid::new_v4().to_string()[..8].to_string();
    let officer_eppn = format!("officer-{tag}@state.edu");
    let admin = issue_test_token_and_user(ADMIN);
    let faculty = issue_test_token_and_user(FACULTY);
    let officer = issue_test_token_and_user(&officer_eppn);
    let state = Arc::new(ServerState::new().expect("state"));
    state.seed_demo().expect("seed");
    let root = state.organizations.read().unwrap().values().find(|o| o.parent_id.is_none()).map(|o| o.id).expect("a root unit");
    World { app: build_app_with_state(state.clone()).expect("router"), state, admin, faculty, officer, root, _cleanup: Cleanup(officer_eppn.clone()), officer_eppn }
}

fn affiliation(w: &World) -> String {
    resolve_user(&w.officer_eppn, &w.state).expect("the officer resolves").affiliation
}

fn appoint(_w: &World, unit: Uuid) -> String {
    format!("/api/v1/orgs/{unit}/appointments")
}

// 1. A SCIM record that says compliance@... grants nothing. The registry cannot hand out this access.
#[tokio::test]
async fn a_scim_claim_of_compliance_grants_nothing() {
    let _db = common::TestDb::new();
    let w = world().await;
    let eppn = w.officer_eppn.clone();
    let updated = db(move |c| {
        c.execute(
            "UPDATE scim_users SET payload = jsonb_set(payload, '{roles}', '[{\"type\":\"eduPersonScopedAffiliation\",\"value\":\"compliance@state.edu\"}]'::jsonb) WHERE user_name = $1",
            &[&eppn],
        )
        .unwrap()
    });
    assert_eq!(updated, 1, "the SCIM record exists and now says compliance");
    assert_ne!(affiliation(&w), "compliance");
    let (s, _) = send(&w.app, "GET", "/api/v1/admin/disclosures", &w.officer, None).await;
    assert_eq!(s, StatusCode::FORBIDDEN, "the claim opens no compliance route");
}

// 2. A Platform Admin appoints a compliance officer on the root unit. The appointment is what makes them
//    compliance, it opens the compliance routes, and revoking it takes both away. Each is a ledger entry.
#[tokio::test]
async fn a_platform_admin_appoints_and_revokes_a_compliance_officer() {
    let _db = common::TestDb::new();
    let w = world().await;
    let ledger = |body: &Value, kind: &str| body["entries"].as_array().unwrap().iter().filter(|e| e["decision_type"] == kind).count();
    let (_, before) = send(&w.app, "GET", "/api/v1/governance/ledger", &w.admin, None).await;

    assert_ne!(affiliation(&w), "compliance");
    let (s, _) = send(&w.app, "GET", "/api/v1/admin/disclosures", &w.officer, None).await;
    assert_eq!(s, StatusCode::FORBIDDEN);

    let body = json!({ "eppn": w.officer_eppn, "scoped_affiliation": "compliance", "reason": "Named the institution's compliance officer" });
    let (s, role) = send(&w.app, "POST", &appoint(&w, w.root), &w.admin, Some(body)).await;
    assert_eq!(s, StatusCode::CREATED, "{role}");
    assert_eq!(role["role_title"], "Compliance Officer");
    assert_eq!(affiliation(&w), "compliance");
    let (s, body) = send(&w.app, "GET", "/api/v1/admin/disclosures", &w.officer, None).await;
    assert_eq!(s, StatusCode::OK, "{body}");

    let revoke = format!("/api/v1/orgs/{}/appointments/{}/compliance", w.root, w.officer_eppn);
    let (s, body) = send(&w.app, "DELETE", &revoke, &w.admin, None).await;
    assert_eq!(s, StatusCode::OK, "{body}");
    assert_ne!(affiliation(&w), "compliance");
    let (s, _) = send(&w.app, "GET", "/api/v1/admin/disclosures", &w.officer, None).await;
    assert_eq!(s, StatusCode::FORBIDDEN);

    let (_, after) = send(&w.app, "GET", "/api/v1/governance/ledger", &w.admin, None).await;
    assert_eq!(ledger(&after, "AccessRoleGranted"), ledger(&before, "AccessRoleGranted") + 1);
    assert_eq!(ledger(&after, "AccessRoleRevoked"), ledger(&before, "AccessRoleRevoked") + 1);
}

// 3. Only a Platform Admin appoints compliance, and only on the root unit.
#[tokio::test]
async fn only_a_platform_admin_appoints_compliance_and_only_on_the_root_unit() {
    let _db = common::TestDb::new();
    let w = world().await;
    let body = json!({ "eppn": w.officer_eppn, "scoped_affiliation": "compliance" });

    let (s, _) = send(&w.app, "POST", &appoint(&w, w.root), &w.faculty, Some(body.clone())).await;
    assert_eq!(s, StatusCode::FORBIDDEN, "faculty may not");
    assert_ne!(affiliation(&w), "compliance");

    // A compliance officer does not appoint another.
    let (s, _) = send(&w.app, "POST", &appoint(&w, w.root), &w.admin, Some(body.clone())).await;
    assert_eq!(s, StatusCode::CREATED);
    let other = json!({ "eppn": format!("other-{}@state.edu", Uuid::new_v4().simple()), "scoped_affiliation": "compliance" });
    let (s, _) = send(&w.app, "POST", &appoint(&w, w.root), &w.officer, Some(other)).await;
    assert_eq!(s, StatusCode::FORBIDDEN, "compliance is not a way to make more compliance");

    // Not on a unit below the root.
    let child = state_child(&w).await;
    let (s, msg) = send(&w.app, "POST", &appoint(&w, child), &w.admin, Some(body)).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{msg}");
}

/// A unit below the root, made through the API.
async fn state_child(w: &World) -> Uuid {
    let code = format!("C{}", &Uuid::new_v4().simple().to_string()[..6]);
    let (s, org) = send(
        &w.app,
        "POST",
        "/api/v1/orgs",
        &w.admin,
        Some(json!({ "name": format!("Unit {code}"), "code": code, "org_type": "College", "parent_id": w.root })),
    )
    .await;
    assert!(s.is_success(), "{org}");
    Uuid::parse_str(org["id"].as_str().expect("an id")).unwrap()
}

// 4. The other affiliations are as they were.
#[tokio::test]
async fn the_other_affiliations_are_as_they_were() {
    let _db = common::TestDb::new();
    let w = world().await;
    assert_eq!(resolve_user(FACULTY, &w.state).unwrap().affiliation, "faculty");
    assert_eq!(resolve_user(ADMIN, &w.state).unwrap().affiliation, "central_admin");
}
