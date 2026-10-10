//! Approvers phase 2: position types, holders and vacancies.
//!
//! The tests share one database, so they run one at a time behind a lock and reset the
//! people, organization and position tables first. The ledger is append-only and is not reset.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::Router;
use http_body_util::BodyExt;
use scaffoldry_core::ledger::DecisionType;
use scaffoldry_server::build_app_with_state;
use scaffoldry_server::service::admin::ADMIN_ROUTES;
use scaffoldry_server::service::identity::issue_test_token_and_user;
use scaffoldry_server::state::ServerState;
use serde_json::{json, Value};
use std::sync::Arc;
use tokio::sync::{Mutex, MutexGuard};
use tower::ServiceExt;
use uuid::Uuid;

static LOCK: Mutex<()> = Mutex::const_new(());

const ROOT_UNIT: &str = "00000000-0000-0000-0000-000000000001";
const SCIM_TOKEN: &str = "test-scim-token";

struct Ctx {
    app: Router,
    admin: String,
    _guard: MutexGuard<'static, ()>,
}

async fn ctx() -> Ctx {
    let guard = LOCK.lock().await;
    tokio::task::spawn_blocking(|| {
        let url = std::env::var("DATABASE_URL")
            .unwrap_or_else(|_| "postgres://scaffoldry:scaffoldry_dev_password@127.0.0.1:5433/scaffoldry".to_string());
        if let Ok(mut client) = postgres::Client::connect(&url, postgres::NoTls) {
            let _ = client.batch_execute(
                "TRUNCATE TABLE scim_users, scim_groups, roles, persons, organizations, workspaces, api_tokens, position_types CASCADE;",
            );
        }
    })
    .await
    .unwrap();
    let admin = issue_test_token_and_user("jordan.lee@state.edu");
    let state = Arc::new(ServerState::new().expect("state"));
    let app = build_app_with_state(state).expect("router");
    Ctx { app, admin, _guard: guard }
}

async fn send(app: &Router, method: &str, uri: &str, token: Option<&str>, body: Option<Value>) -> (StatusCode, Value) {
    let mut b = Request::builder().method(method).uri(uri);
    if let Some(t) = token {
        b = b.header("authorization", format!("Bearer {t}"));
    }
    let req = match body {
        Some(v) => b.header("content-type", "application/json").body(Body::from(serde_json::to_vec(&v).unwrap())).unwrap(),
        None => b.body(Body::empty()).unwrap(),
    };
    let resp = app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
}

fn unique(prefix: &str) -> String {
    format!("{prefix}-{}@state.edu", &Uuid::new_v4().to_string()[..8])
}

async fn ledger_len(c: &Ctx) -> u64 {
    let (_, body) = send(&c.app, "GET", "/api/v1/governance/ledger", Some(&c.admin), None).await;
    body["total_entries"].as_u64().unwrap()
}

/// The last `n` ledger entries, oldest first.
async fn last_entries(c: &Ctx, n: usize) -> Vec<Value> {
    let (_, body) = send(&c.app, "GET", "/api/v1/governance/ledger", Some(&c.admin), None).await;
    let all = body["entries"].as_array().unwrap();
    all[all.len() - n..].to_vec()
}

async fn create_unit(c: &Ctx, name: &str, org_type: &str, parent: &str) -> String {
    let (status, body) = send(
        &c.app,
        "POST",
        "/api/v1/orgs",
        Some(&c.admin),
        Some(json!({ "name": name, "code": &name[..4].to_uppercase(), "org_type": org_type, "parent_id": parent })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "create unit: {body}");
    body["id"].as_str().unwrap().to_string()
}

/// A user the platform knows, placed in `department` by name.
async fn create_user(c: &Ctx, user_name: &str, department: &str) {
    let (status, _) = send(
        &c.app,
        "POST",
        "/api/v1/admin/users",
        Some(&c.admin),
        Some(json!({
            "userName": user_name, "name": "Test Person", "email": user_name,
            "affiliation": "faculty", "department": department, "title": "Lecturer", "reason": "Fixture"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "creating a test user failed");
}

async fn create_position(c: &Ctx, key: &str, org_types: Value, max_holders: u32) -> (StatusCode, Value) {
    send(
        &c.app,
        "POST",
        "/api/v1/admin/positions",
        Some(&c.admin),
        Some(json!({
            "key": key, "name": format!("Position {key}"), "description": "A test position",
            "org_types": org_types, "max_holders": max_holders, "reason": "Define the position"
        })),
    )
    .await
}

async fn assign(c: &Ctx, unit: &str, key: &str, eppn: &str, replace: bool, token: &str) -> (StatusCode, Value) {
    send(
        &c.app,
        "POST",
        &format!("/api/v1/orgs/{unit}/positions/{key}/holders"),
        Some(token),
        Some(json!({ "eppn": eppn, "replace": replace, "reason": "Assignment test" })),
    )
    .await
}

fn holders_of(listing: &Value, key: &str) -> Vec<String> {
    listing["positions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["key"] == key)
        .map(|p| p["holders"].as_array().unwrap().iter().map(|h| h["eppn"].as_str().unwrap().to_string()).collect())
        .unwrap_or_default()
}

// 1. Create a position type, assign a holder at a department, read it back.
#[tokio::test]
async fn a_holder_assigned_at_a_department_reads_back_from_the_unit() {
    let c = ctx().await;
    let dept = create_unit(&c, "Physics", "Department", ROOT_UNIT).await;
    let chair = unique("p1chair");
    create_user(&c, &chair, "Physics").await;

    let (status, created) = create_position(&c, "chair", json!(["Department"]), 1).await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    assert_eq!(created["position"]["key"], "chair");

    let (status, body) = assign(&c, &dept, "chair", &chair, false, &c.admin).await;
    assert_eq!(status, StatusCode::CREATED, "{body}");

    let (status, listing) = send(&c.app, "GET", &format!("/api/v1/orgs/{dept}/positions"), Some(&c.admin), None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(holders_of(&listing, "chair"), std::slice::from_ref(&chair));

    let (_, all) = send(&c.app, "GET", "/api/v1/admin/positions", Some(&c.admin), None).await;
    let row = all["positions"].as_array().unwrap().iter().find(|p| p["key"] == "chair").unwrap();
    assert_eq!(row["holder_count"], 1);
}

// 2. A position that applies to a College cannot be held at a Department.
#[tokio::test]
async fn a_position_cannot_be_held_at_a_unit_type_it_does_not_apply_to() {
    let c = ctx().await;
    let dept = create_unit(&c, "Chemistry", "Department", ROOT_UNIT).await;
    let person = unique("p2");
    create_user(&c, &person, "Chemistry").await;
    assert_eq!(create_position(&c, "dean", json!(["College"]), 1).await.0, StatusCode::CREATED);

    let (status, _) = assign(&c, &dept, "dean", &person, false, &c.admin).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

// 3. A full position is 409 position_full. With replace the old holder is gone.
#[tokio::test]
async fn a_full_position_is_refused_unless_replace_vacates_the_old_holder() {
    let c = ctx().await;
    let dept = create_unit(&c, "Geology", "Department", ROOT_UNIT).await;
    let (old, new) = (unique("p3old"), unique("p3new"));
    create_user(&c, &old, "Geology").await;
    create_user(&c, &new, "Geology").await;
    create_position(&c, "chair", json!(["Department"]), 1).await;
    assert_eq!(assign(&c, &dept, "chair", &old, false, &c.admin).await.0, StatusCode::CREATED);

    let (status, body) = assign(&c, &dept, "chair", &new, false, &c.admin).await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(body["code"], "position_full", "{body}");

    let before = ledger_len(&c).await;
    let (status, body) = assign(&c, &dept, "chair", &new, true, &c.admin).await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    assert_eq!(ledger_len(&c).await, before + 2, "a revoke entry and a grant entry");
    let entries = last_entries(&c, 2).await;
    assert_eq!(entries[0]["decision_type"], "AccessRoleRevoked");
    assert_eq!(entries[1]["decision_type"], "AccessRoleGranted");

    let (_, listing) = send(&c.app, "GET", &format!("/api/v1/orgs/{dept}/positions"), Some(&c.admin), None).await;
    assert_eq!(holders_of(&listing, "chair"), [new]);
}

// 4. A scim holding cannot be vacated. Re-syncing removes and re-adds it.
#[tokio::test]
async fn a_scim_holding_cannot_be_vacated_and_a_resync_restores_it() {
    let c = ctx().await;
    let dept = create_unit(&c, "Astronomy", "Department", ROOT_UNIT).await;
    create_position(&c, "chair", json!(["Department"]), 1).await;
    let user = unique("p4scim");
    let payload = json!({
        "userName": user, "active": true,
        "urn:ietf:params:scim:schemas:extension:enterprise:2.0:User": { "department": "ASTR" },
        "roles": [{ "type": "scaffoldry-position", "value": "chair" }]
    });
    let (status, created) = send(&c.app, "POST", "/scim/v2/Users", Some(SCIM_TOKEN), Some(payload.clone())).await;
    assert_eq!(status, StatusCode::CREATED);

    let (_, listing) = send(&c.app, "GET", &format!("/api/v1/orgs/{dept}/positions"), Some(&c.admin), None).await;
    assert_eq!(holders_of(&listing, "chair"), std::slice::from_ref(&user));

    let (status, _) = send(&c.app, "DELETE", &format!("/api/v1/orgs/{dept}/positions/chair/holders/{user}"), Some(&c.admin), Some(json!({ "reason": "Try it" }))).await;
    assert_eq!(status, StatusCode::CONFLICT, "the registry owns it");

    // Re-sync without the role removes the holding. With it, the holding is back.
    let id = created["id"].as_str().unwrap();
    let mut without = payload.clone();
    without["roles"] = json!([]);
    send(&c.app, "PUT", &format!("/scim/v2/Users/{id}"), Some(SCIM_TOKEN), Some(without)).await;
    let (_, listing) = send(&c.app, "GET", &format!("/api/v1/orgs/{dept}/positions"), Some(&c.admin), None).await;
    assert!(holders_of(&listing, "chair").is_empty());
    send(&c.app, "PUT", &format!("/scim/v2/Users/{id}"), Some(SCIM_TOKEN), Some(payload)).await;
    let (_, listing) = send(&c.app, "GET", &format!("/api/v1/orgs/{dept}/positions"), Some(&c.admin), None).await;
    assert_eq!(holders_of(&listing, "chair"), [user]);
}

// 5. A job title never creates a holding. An unknown key inserts nothing and the write is 201.
#[tokio::test]
async fn a_title_or_an_unknown_key_grants_nothing() {
    let c = ctx().await;
    let dept = create_unit(&c, "Biology", "Department", ROOT_UNIT).await;
    create_position(&c, "chair", json!(["Department"]), 1).await;
    let enterprise = json!({ "department": "BIOL" });
    let by_title = json!({
        "userName": unique("p5title"), "active": true, "title": "Department Chair",
        "urn:ietf:params:scim:schemas:extension:enterprise:2.0:User": enterprise
    });
    assert_eq!(send(&c.app, "POST", "/scim/v2/Users", Some(SCIM_TOKEN), Some(by_title)).await.0, StatusCode::CREATED);
    let unknown = json!({
        "userName": unique("p5key"), "active": true,
        "urn:ietf:params:scim:schemas:extension:enterprise:2.0:User": enterprise,
        "roles": [{ "type": "scaffoldry-position", "value": "no-such-position" }]
    });
    assert_eq!(send(&c.app, "POST", "/scim/v2/Users", Some(SCIM_TOKEN), Some(unknown)).await.0, StatusCode::CREATED);

    let (_, listing) = send(&c.app, "GET", &format!("/api/v1/orgs/{dept}/positions"), Some(&c.admin), None).await;
    assert!(holders_of(&listing, "chair").is_empty(), "{listing}");
}

// 6. Vacancies lists exactly the units with no holder.
#[tokio::test]
async fn vacancies_list_exactly_the_units_with_no_holder() {
    let c = ctx().await;
    let filled = create_unit(&c, "Mathematics", "Department", ROOT_UNIT).await;
    let empty = create_unit(&c, "Statistics", "Department", ROOT_UNIT).await;
    create_unit(&c, "Humanities", "College", ROOT_UNIT).await;
    let person = unique("p6");
    create_user(&c, &person, "Mathematics").await;
    create_position(&c, "chair", json!(["Department"]), 1).await;
    assign(&c, &filled, "chair", &person, false, &c.admin).await;

    let (status, body) = send(&c.app, "GET", "/api/v1/admin/positions/vacancies", Some(&c.admin), None).await;
    assert_eq!(status, StatusCode::OK);
    let rows = body["vacancies"].as_array().unwrap();
    assert_eq!(rows.len(), 1, "{body}");
    assert_eq!(rows[0]["unit"]["id"], empty);
    assert_eq!(rows[0]["position"]["key"], "chair");
}

// 7. An Org Unit Admin in scope can assign inside their unit and is 403 outside it.
#[tokio::test]
async fn an_org_unit_admin_assigns_inside_their_unit_only() {
    let c = ctx().await;
    let mine = create_unit(&c, "Music", "Department", ROOT_UNIT).await;
    let theirs = create_unit(&c, "Drama", "Department", ROOT_UNIT).await;
    let admin_user = unique("p7admin");
    let person = unique("p7person");
    create_user(&c, &admin_user, "Music").await;
    create_user(&c, &person, "Music").await;
    create_position(&c, "chair", json!(["Department"]), 1).await;
    let (status, _) = send(
        &c.app, "POST", &format!("/api/v1/orgs/{mine}/appointments"), Some(&c.admin),
        Some(json!({ "eppn": admin_user, "scoped_affiliation": "unit_admin", "reason": "Make a unit admin" })),
    ).await;
    assert_eq!(status, StatusCode::CREATED);
    let unit_admin = issue_test_token_and_user(&admin_user);

    assert_eq!(assign(&c, &mine, "chair", &person, false, &unit_admin).await.0, StatusCode::CREATED);
    assert_eq!(assign(&c, &theirs, "chair", &person, false, &unit_admin).await.0, StatusCode::FORBIDDEN);
}

// Position type rules.
#[tokio::test]
async fn position_types_are_validated_patched_and_audited() {
    let c = ctx().await;
    let dept = create_unit(&c, "History", "Department", ROOT_UNIT).await;
    let person = unique("p8");
    create_user(&c, &person, "History").await;

    for bad in [
        json!([]),
        json!(["Wizardry"]),
    ] {
        assert_eq!(create_position(&c, "bad", bad, 1).await.0, StatusCode::BAD_REQUEST);
    }
    assert_eq!(create_position(&c, "toomany", json!(["Department"]), 0).await.0, StatusCode::BAD_REQUEST);
    assert_eq!(create_position(&c, "toomany", json!(["Department"]), 51).await.0, StatusCode::BAD_REQUEST);
    assert_eq!(create_position(&c, "Bad Key!", json!(["Department"]), 1).await.0, StatusCode::BAD_REQUEST);

    let before = ledger_len(&c).await;
    assert_eq!(create_position(&c, "chair", json!(["Department"]), 1).await.0, StatusCode::CREATED);
    assert_eq!(ledger_len(&c).await, before + 1);
    let entry = last_entries(&c, 1).await.remove(0);
    assert_eq!(entry["decision_type"], "PositionChanged");
    assert_eq!(entry["principal"], "jordan.lee@state.edu");
    assert_eq!(entry["rationale"], "Define the position");
    assert_eq!(create_position(&c, "chair", json!(["Department"]), 1).await.0, StatusCode::CONFLICT, "duplicate key");

    // A write with no reason is refused and writes nothing.
    let before = ledger_len(&c).await;
    let (status, _) = send(&c.app, "PATCH", "/api/v1/admin/positions/chair", Some(&c.admin), Some(json!({ "name": "Chair" }))).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(ledger_len(&c).await, before);

    assign(&c, &dept, "chair", &person, false, &c.admin).await;
    // org_types cannot change while holders exist. A rename can.
    let patch = |body: Value| {
        let app = c.app.clone();
        let admin = c.admin.clone();
        async move { send(&app, "PATCH", "/api/v1/admin/positions/chair", Some(&admin), Some(body)).await }
    };
    assert_eq!(patch(json!({ "org_types": ["College"], "reason": "Change" })).await.0, StatusCode::CONFLICT);
    let (status, renamed) = patch(json!({ "name": "Department Chair", "reason": "Rename" })).await;
    assert_eq!(status, StatusCode::OK, "{renamed}");
    assert_eq!(renamed["position"]["name"], "Department Chair");
    assert_eq!(patch(json!({ "max_holders": 0, "reason": "Bad" })).await.0, StatusCode::BAD_REQUEST);
    assert_eq!(send(&c.app, "PATCH", "/api/v1/admin/positions/nope", Some(&c.admin), Some(json!({ "name": "X", "reason": "r" }))).await.0, StatusCode::NOT_FOUND);

    // Retiring keeps the holder. A retired position takes no new holder and is not offered.
    let (status, retired) = patch(json!({ "retired": true, "reason": "No longer used" })).await;
    assert_eq!(status, StatusCode::OK, "{retired}");
    let (_, listing) = send(&c.app, "GET", &format!("/api/v1/orgs/{dept}/positions"), Some(&c.admin), None).await;
    assert!(listing["positions"].as_array().unwrap().iter().all(|p| p["key"] != "chair"), "retired positions are not offered");
    let (_, all) = send(&c.app, "GET", "/api/v1/admin/positions", Some(&c.admin), None).await;
    let row = all["positions"].as_array().unwrap().iter().find(|p| p["key"] == "chair").unwrap();
    assert_eq!(row["holder_count"], 1, "retiring does not remove holders");
    assert_eq!(row["retired"], true);
}

#[tokio::test]
async fn a_holder_must_be_an_active_user_and_can_be_vacated() {
    let c = ctx().await;
    let dept = create_unit(&c, "Legal", "Department", ROOT_UNIT).await;
    create_position(&c, "chair", json!(["Department"]), 2).await;
    let (status, _) = assign(&c, &dept, "chair", "ghost@state.edu", false, &c.admin).await;
    assert!(status == StatusCode::BAD_REQUEST || status == StatusCode::NOT_FOUND, "an unknown user is refused");

    let person = unique("p9");
    create_user(&c, &person, "Legal").await;
    assert_eq!(assign(&c, &dept, "chair", &person, false, &c.admin).await.0, StatusCode::CREATED);
    assert_eq!(assign(&c, &dept, "chair", &person, false, &c.admin).await.0, StatusCode::CONFLICT, "already a holder");

    let before = ledger_len(&c).await;
    let uri = format!("/api/v1/orgs/{dept}/positions/chair/holders/{person}");
    let (status, _) = send(&c.app, "DELETE", &uri, Some(&c.admin), Some(json!({}))).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "a reason is required");
    assert_eq!(ledger_len(&c).await, before);
    let (status, _) = send(&c.app, "DELETE", &uri, Some(&c.admin), Some(json!({ "reason": "Stepped down" }))).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(last_entries(&c, 1).await[0]["decision_type"], "AccessRoleRevoked");
    let (status, _) = send(&c.app, "DELETE", &uri, Some(&c.admin), Some(json!({ "reason": "Again" }))).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

// Every admin route is registered and guarded.
#[tokio::test]
async fn every_position_route_is_in_admin_routes_and_refuses_a_faculty_caller() {
    let c = ctx().await;
    for (method, path) in [
        ("GET", "/admin/positions"),
        ("POST", "/admin/positions"),
        ("PATCH", "/admin/positions/{key}"),
        ("GET", "/admin/positions/vacancies"),
    ] {
        assert!(ADMIN_ROUTES.iter().any(|(m, p)| *m == method && *p == path), "ADMIN_ROUTES must list {method} {path}");
    }
    let faculty = issue_test_token_and_user("faculty.curie@state.edu");
    for &(method, path) in ADMIN_ROUTES {
        let uri = format!("/api/v1{}", path.replace("{id}", &Uuid::new_v4().to_string()).replace("{key}", "chair"));
        assert_eq!(send(&c.app, method, &uri, None, None).await.0, StatusCode::UNAUTHORIZED, "{method} {uri} with no token");
        assert_eq!(send(&c.app, method, &uri, Some(&faculty), None).await.0, StatusCode::FORBIDDEN, "{method} {uri} as faculty");
    }
}

#[test]
fn the_decision_types_for_positions_and_delegations_exist() {
    for name in ["PositionChanged", "DelegationChanged"] {
        let parsed = DecisionType::parse(name).unwrap_or_else(|| panic!("{name} is in DecisionType::ALL"));
        assert_eq!(parsed.as_str(), name);
    }
}
