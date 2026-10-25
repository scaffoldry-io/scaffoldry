//! Access model phase 10: provisioning from the systems of record. Validity, review, separation of duties.

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use common::TestDb;
use http_body_util::BodyExt;
use scaffoldry_server::build_app_with_state;
use scaffoldry_server::service::identity::issue_test_token_and_user;
use scaffoldry_server::state::{OrganizationNode, RoleRow, ServerState};
use serde_json::{json, Value};
use tower::ServiceExt;
use uuid::Uuid;

const ADMIN: &str = "jordan.lee@state.edu";
const ADMIN_TWO: &str = "second.admin@state.edu";
const READER: &str = "reader.p10@state.edu";
const ROOT: &str = "00000000-0000-0000-0000-000000000001";

async fn send(app: &axum::Router, method: &str, uri: &str, who: &str, body: Option<Value>) -> (StatusCode, Value) {
    let mut req = Request::builder().method(method).uri(uri).header("authorization", format!("Bearer {}", issue_test_token_and_user(who)));
    let body = match body {
        Some(v) => {
            req = req.header("content-type", "application/json");
            Body::from(v.to_string())
        }
        None => Body::empty(),
    };
    let resp = app.clone().oneshot(req.body(body).unwrap()).await.unwrap();
    let status = resp.status();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    (status, if bytes.is_empty() { Value::Null } else { serde_json::from_slice(&bytes).unwrap_or(Value::Null) })
}

fn unit(state: &ServerState, name: &str, parent: Option<Uuid>) -> Uuid {
    let root = Uuid::parse_str(ROOT).unwrap();
    let id = Uuid::new_v4();
    let node = OrganizationNode { id, parent_id: Some(parent.unwrap_or(root)), name: name.to_string(), code: name.to_uppercase(), org_type: "Department".to_string(), hierarchy_key: "academic".to_string(), synthetic: false };
    state.organizations.write().unwrap().insert(id, node.clone());
    state.persist_organization(&node).unwrap();
    id
}

fn appoint(state: &ServerState, eppn: &str, unit: Uuid, scoped_affiliation: &str, position: Option<&str>) {
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
            position_key: position.map(str::to_string),
        })
        .unwrap();
}


struct World {
    db: TestDb,
    app: axum::Router,
    science: Uuid,
}

fn world() -> World {
    let db = TestDb::new();
    let state = db.state();
    let science = unit(&state, "science", None);
    appoint(&state, ADMIN_TWO, Uuid::parse_str(ROOT).unwrap(), "platform_admin", None);
    let app = build_app_with_state(state).expect("router");
    World { db, app, science }
}

fn in_days(n: i64) -> String {
    (chrono::Utc::now() + chrono::Duration::days(n)).to_rfc3339()
}

fn grant_body(w: &World, who: &str, source: &str, valid_until: Option<String>) -> Value {
    let mut b = json!({ "holder_kind": "person", "holder_id": who, "role": "viewer", "hierarchy": "academic", "anchor_kind": "unit", "anchor_id": w.science.to_string(), "source": source, "reason": "Test grant" });
    if let Some(v) = valid_until {
        b["valid_until"] = json!(v);
    }
    b
}

async fn propose(w: &World, body: Value) -> (StatusCode, Value) {
    send(&w.app, "POST", "/api/v1/admin/access/grants", ADMIN, Some(body)).await
}

async fn grant(w: &World, body: Value) -> i64 {
    let (s, pending) = propose(w, body).await;
    assert_eq!(s, StatusCode::ACCEPTED, "{pending}");
    let change = pending["change_id"].as_str().unwrap();
    let (s, done) = send(&w.app, "POST", &format!("/api/v1/admin/access/changes/{change}/decide"), ADMIN_TWO, Some(json!({ "decision": "confirm", "reason": "Reviewed" }))).await;
    assert_eq!(s, StatusCode::OK, "{done}");
    done["grant_id"].as_i64().unwrap()
}

fn age_grant(w: &World, id: i64, days: i64) {
    w.db.run(move |c| {
        c.execute("UPDATE access_grants SET created_at = NOW() - make_interval(days => $2::int) WHERE id = $1", &[&id, &(days as i32)]).unwrap();
    });
}

async fn due_ids(w: &World, who: &str) -> Vec<i64> {
    let (s, body) = send(&w.app, "GET", "/api/v1/admin/access/reviews", who, None).await;
    assert_eq!(s, StatusCode::OK, "{body}");
    body["due"].as_array().unwrap().iter().filter_map(|g| g["id"].as_i64()).collect()
}

// 2. A requested grant needs a valid_until, at most the setting.
#[tokio::test]
async fn a_requested_grant_needs_a_valid_until_within_the_maximum() {
    let w = world();
    let (s, body) = propose(&w, grant_body(&w, READER, "request", None)).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{body}");
    let (s, body) = propose(&w, grant_body(&w, READER, "request", Some(in_days(181)))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{body}");
    assert!(body.to_string().contains("180"), "the refusal names the maximum: {body}");
    let (s, body) = propose(&w, grant_body(&w, READER, "request", Some(in_days(179)))).await;
    assert_eq!(s, StatusCode::ACCEPTED, "{body}");
    // The maximum is a setting, from 1 to 365 days.
    let (s, _) = send(&w.app, "PUT", "/api/v1/settings/access.person_grant_max_days", ADMIN, Some(json!(0))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    let (s, _) = send(&w.app, "PUT", "/api/v1/settings/access.person_grant_max_days", ADMIN, Some(json!(30))).await;
    assert_eq!(s, StatusCode::OK);
    let (s, body) = propose(&w, grant_body(&w, READER, "request", Some(in_days(60)))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{body}");
    // A grant from another source needs no end date.
    let (s, body) = propose(&w, grant_body(&w, READER, "appointment", None)).await;
    assert_eq!(s, StatusCode::ACCEPTED, "{body}");
}

// 3. A grant past access.review_days appears in the review list. Keeping it removes it and writes a ledger entry.
#[tokio::test]
async fn an_old_grant_is_due_and_keeping_it_resets_the_clock() {
    let w = world();
    let id = grant(&w, grant_body(&w, READER, "appointment", None)).await;
    assert!(due_ids(&w, ADMIN).await.is_empty(), "a new grant is not due");
    age_grant(&w, id, 400);
    assert_eq!(due_ids(&w, ADMIN).await, vec![id]);

    let (s, body) = send(&w.app, "POST", &format!("/api/v1/admin/access/grants/{id}/review"), ADMIN, Some(json!({ "decision": "keep", "reason": "Still needed" }))).await;
    assert_eq!(s, StatusCode::OK, "{body}");
    assert!(due_ids(&w, ADMIN).await.is_empty(), "keeping resets the clock");
    let (_, ledger) = send(&w.app, "GET", "/api/v1/governance/ledger?limit=5", ADMIN, None).await;
    assert!(ledger.to_string().contains("AccessGrantReviewed"), "{ledger}");

    // Revoking in a review ends the grant.
    age_grant(&w, id, 800);
    let (s, body) = send(&w.app, "POST", &format!("/api/v1/admin/access/grants/{id}/review"), ADMIN, Some(json!({ "decision": "revoke", "reason": "No longer needed" }))).await;
    assert_eq!(s, StatusCode::OK, "{body}");
    let (_, active) = send(&w.app, "GET", &format!("/api/v1/admin/access/grants?state=active&holder={READER}"), ADMIN, None).await;
    assert!(active["grants"].as_array().unwrap().is_empty(), "{active}");
}

#[tokio::test]
async fn a_person_cannot_review_a_grant_they_hold() {
    let w = world();
    let id = grant(&w, grant_body(&w, ADMIN_TWO, "appointment", None)).await;
    age_grant(&w, id, 400);
    let (s, body) = send(&w.app, "POST", &format!("/api/v1/admin/access/grants/{id}/review"), ADMIN_TWO, Some(json!({ "decision": "keep", "reason": "Mine" }))).await;
    assert_eq!(s, StatusCode::FORBIDDEN, "{body}");
    assert_eq!(due_ids(&w, ADMIN).await, vec![id], "still due");
}

#[tokio::test]
async fn the_review_days_setting_moves_the_line() {
    let w = world();
    let id = grant(&w, grant_body(&w, READER, "appointment", None)).await;
    age_grant(&w, id, 40);
    assert!(due_ids(&w, ADMIN).await.is_empty());
    let (s, _) = send(&w.app, "PUT", "/api/v1/settings/access.review_days", ADMIN, Some(json!(10))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "30 to 730");
    let (s, _) = send(&w.app, "PUT", "/api/v1/settings/access.review_days", ADMIN, Some(json!(30))).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(due_ids(&w, ADMIN).await, vec![id]);
}

// 4. Completing a conflicting pair is 409 grant_conflict, and an existing conflict is listed in the review.
async fn make_position(w: &World, key: &str) {
    let (s, body) = send(&w.app, "POST", "/api/v1/admin/positions", ADMIN, Some(json!({ "key": key, "name": format!("Position {key}"), "description": "Test", "org_types": ["Department"], "max_holders": 5, "reason": "Define" }))).await;
    assert_eq!(s, StatusCode::CREATED, "{body}");
}

async fn set_conflicts(w: &World, pairs: Value) {
    let (s, body) = send(&w.app, "PUT", "/api/v1/settings/access.conflicts", ADMIN, Some(pairs)).await;
    assert_eq!(s, StatusCode::OK, "{body}");
}

#[tokio::test]
async fn completing_a_conflicting_pair_is_refused() {
    let w = world();
    make_position(&w, "dean").await;
    make_position(&w, "auditor").await;
    set_conflicts(&w, json!([
        { "a": { "position": "dean" }, "b": { "position": "auditor" } },
        { "a": { "position": "dean" }, "b": { "role": "admin", "domain": "research" } }
    ])).await;
    issue_test_token_and_user(READER);
    let unit = w.science.to_string();
    let assign = |key: &'static str| {
        let (app, unit) = (w.app.clone(), unit.clone());
        async move {
            send(&app, "POST", &format!("/api/v1/orgs/{unit}/positions/{key}/holders"), ADMIN, Some(json!({ "eppn": READER, "replace": false, "reason": "Assignment" }))).await
        }
    };
    let (s, body) = assign("dean").await;
    assert_eq!(s, StatusCode::CREATED, "{body}");
    // Two positions.
    let (s, body) = assign("auditor").await;
    assert_eq!(s, StatusCode::CONFLICT, "{body}");
    assert_eq!(body["code"], "grant_conflict", "{body}");
    assert!(body["error"].as_str().unwrap().contains("dean") && body["error"].as_str().unwrap().contains("auditor"), "names the pair: {body}");
    // A position and a role on a domain.
    let mut g = grant_body(&w, READER, "appointment", None);
    g["role"] = json!("admin");
    g["domain"] = json!("research");
    let (s, body) = propose(&w, g).await;
    assert_eq!(s, StatusCode::CONFLICT, "{body}");
    assert_eq!(body["code"], "grant_conflict", "{body}");
    // A role on another domain is no conflict.
    let mut g = grant_body(&w, READER, "appointment", None);
    g["role"] = json!("admin");
    g["domain"] = json!("engagement");
    let (s, body) = propose(&w, g).await;
    assert_eq!(s, StatusCode::ACCEPTED, "{body}");
}

#[tokio::test]
async fn an_existing_conflict_is_listed_in_the_review() {
    let w = world();
    make_position(&w, "dean").await;
    issue_test_token_and_user(READER);
    let unit = w.science.to_string();
    let (s, body) = send(&w.app, "POST", &format!("/api/v1/orgs/{unit}/positions/dean/holders"), ADMIN, Some(json!({ "eppn": READER, "replace": false, "reason": "Assignment" }))).await;
    assert_eq!(s, StatusCode::CREATED, "{body}");
    let mut g = grant_body(&w, READER, "appointment", None);
    g["role"] = json!("admin");
    g["domain"] = json!("research");
    grant(&w, g).await;
    // The pair is declared after both are held.
    set_conflicts(&w, json!([{ "a": { "position": "dean" }, "b": { "role": "admin", "domain": "research" } }])).await;
    let (s, body) = send(&w.app, "GET", "/api/v1/admin/access/reviews", ADMIN, None).await;
    assert_eq!(s, StatusCode::OK, "{body}");
    let found = body["conflicts"].as_array().unwrap();
    assert_eq!(found.len(), 1, "{body}");
    assert_eq!(found[0]["person"], READER);
}
