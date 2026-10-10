//! Admin console phase 2: People.
//!
//! The tests share one database, so they run one at a time behind a lock and reset the people
//! tables first. The ledger is append-only and is never reset.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::Router;
use http_body_util::BodyExt;
use scaffoldry_server::service::admin::ADMIN_ROUTES;
use scaffoldry_server::service::identity::{hash_token, issue_test_token_and_user};
use scaffoldry_server::state::{ApiToken, ServerState};
use scaffoldry_server::build_app_with_state;
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
    state: Arc<ServerState>,
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
                "TRUNCATE TABLE scim_users, scim_groups, roles, persons, organizations, workspaces, api_tokens CASCADE;",
            );
        }
    })
    .await
    .unwrap();
    let admin = issue_test_token_and_user("jordan.lee@state.edu");
    let state = Arc::new(ServerState::new().expect("state"));
    let app = build_app_with_state(state.clone()).expect("router");
    Ctx { app, state, admin, _guard: guard }
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

async fn create_user(c: &Ctx, user_name: &str, affiliation: &str) -> Value {
    let (status, body) = send(
        &c.app,
        "POST",
        "/api/v1/admin/users",
        Some(&c.admin),
        Some(json!({
            "userName": user_name,
            "name": "Test Person",
            "email": user_name,
            "affiliation": affiliation,
            "department": "Biology",
            "title": "Lecturer",
            "reason": "Test fixture"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "create {user_name}: {body}");
    body["user"].clone()
}

async fn ledger_len(c: &Ctx) -> u64 {
    let (_, body) = send(&c.app, "GET", "/api/v1/governance/ledger", Some(&c.admin), None).await;
    body["total_entries"].as_u64().unwrap()
}

/// The newest ledger entry.
async fn last_entry(c: &Ctx) -> Value {
    let (_, body) = send(&c.app, "GET", "/api/v1/governance/ledger", Some(&c.admin), None).await;
    body["entries"].as_array().unwrap().last().unwrap().clone()
}

fn api_token(eppn: &str, kind: &str, raw: &str) -> ApiToken {
    ApiToken {
        token_hash: hash_token(raw),
        id: Uuid::new_v4(),
        kind: kind.to_string(),
        eppn: eppn.to_string(),
        label: format!("test {kind}"),
        original_admin: None,
        created_at: chrono::Utc::now(),
        expires_at: chrono::Utc::now() + chrono::Duration::days(1),
        last_used_at: None,
        revoked_at: None,
    }
}

// 1. Paging, order, and the row shape.
#[tokio::test]
async fn users_page_by_cursor_in_user_name_order_with_the_full_row() {
    let c = ctx().await;
    let prefix = format!("t1{}", &Uuid::new_v4().to_string()[..6]);
    for letter in ["c", "a", "b"] {
        create_user(&c, &format!("{prefix}-{letter}@state.edu"), "faculty").await;
    }

    let (status, page1) = send(&c.app, "GET", &format!("/api/v1/admin/users?search={prefix}&limit=2"), Some(&c.admin), None).await;
    assert_eq!(status, StatusCode::OK);
    let rows = page1["users"].as_array().unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0]["user_name"], format!("{prefix}-a@state.edu"));
    assert_eq!(rows[1]["user_name"], format!("{prefix}-b@state.edu"));
    let cursor = page1["next_cursor"].as_str().expect("a cursor for the next page").to_string();

    let (_, page2) = send(
        &c.app,
        "GET",
        &format!("/api/v1/admin/users?search={prefix}&limit=2&cursor={cursor}"),
        Some(&c.admin),
        None,
    )
    .await;
    let rows2 = page2["users"].as_array().unwrap();
    assert_eq!(rows2.len(), 1);
    assert_eq!(rows2[0]["user_name"], format!("{prefix}-c@state.edu"));
    assert_eq!(page2["next_cursor"], Value::Null);

    // The row carries what the table needs.
    for key in [
        "id", "user_name", "display_name", "email", "affiliation", "units", "active", "hold",
        "platform_admin", "unit_admin_of", "active_agent_tokens", "latest_token_use",
    ] {
        assert!(rows[0].get(key).is_some(), "row is missing {key}: {}", rows[0]);
    }
    assert_eq!(rows[0]["affiliation"], "faculty");
    assert_eq!(rows[0]["active"], true);
    assert_eq!(rows[0]["hold"], false);
}

#[tokio::test]
async fn users_filter_by_active_hold_affiliation_and_search_matches_the_start_of_a_name() {
    let c = ctx().await;
    let prefix = format!("t1f{}", &Uuid::new_v4().to_string()[..6]);
    let student = create_user(&c, &format!("{prefix}-student@state.edu"), "student").await;
    create_user(&c, &format!("{prefix}-staff@state.edu"), "staff").await;

    let (_, held) = send(
        &c.app,
        "POST",
        &format!("/api/v1/admin/users/{}/hold", student["id"].as_str().unwrap()),
        Some(&c.admin),
        Some(json!({ "hold": true, "reason": "Under review" })),
    )
    .await;
    assert_eq!(held["user"]["hold"], true);

    let list = |q: String| {
        let app = c.app.clone();
        let admin = c.admin.clone();
        async move { send(&app, "GET", &format!("/api/v1/admin/users?{q}"), Some(&admin), None).await.1 }
    };
    assert_eq!(list(format!("search={prefix}&hold=true")).await["users"].as_array().unwrap().len(), 1);
    assert_eq!(list(format!("search={prefix}&hold=false")).await["users"].as_array().unwrap().len(), 1);
    assert_eq!(list(format!("search={prefix}&affiliation=student")).await["users"].as_array().unwrap().len(), 1);
    assert_eq!(list(format!("search={prefix}&active=false")).await["users"].as_array().unwrap().len(), 0);
    // Search matches the start of the name, not the middle.
    assert_eq!(list("search=student".to_string()).await["users"].as_array().unwrap().len(), 0);
    assert_eq!(list(format!("search={}", prefix.to_uppercase())).await["users"].as_array().unwrap().len(), 2);
}

// 2. SCIM updates a user it already has.
#[tokio::test]
async fn scim_post_for_an_existing_user_name_updates_the_row_and_keeps_its_id_and_hold() {
    let c = ctx().await;
    let user_name = unique("t2");
    let payload = json!({ "userName": user_name, "active": true, "name": { "formatted": "First Name" } });

    let (s1, first) = send(&c.app, "POST", "/scim/v2/Users", Some(SCIM_TOKEN), Some(payload.clone())).await;
    assert_eq!(s1, StatusCode::CREATED);
    let (s2, second) = send(
        &c.app,
        "POST",
        "/scim/v2/Users",
        Some(SCIM_TOKEN),
        Some(json!({ "userName": user_name, "active": true, "name": { "formatted": "Second Name" } })),
    )
    .await;
    assert_eq!(s2, StatusCode::OK, "the second push is an update");
    assert_eq!(first["id"], second["id"], "the id is kept");
    assert_eq!(second["name"]["formatted"], "Second Name");

    let (_, list) = send(&c.app, "GET", &format!("/api/v1/admin/users?search={user_name}"), Some(&c.admin), None).await;
    assert_eq!(list["users"].as_array().unwrap().len(), 1, "one user, not two");

    // A hold survives another push.
    let id = first["id"].as_str().unwrap();
    send(&c.app, "POST", &format!("/api/v1/admin/users/{id}/hold"), Some(&c.admin), Some(json!({ "hold": true, "reason": "Hold test" }))).await;
    let (s3, _) = send(&c.app, "POST", "/scim/v2/Users", Some(SCIM_TOKEN), Some(payload)).await;
    assert_eq!(s3, StatusCode::OK);
    let (_, detail) = send(&c.app, "GET", &format!("/api/v1/admin/users/{id}"), Some(&c.admin), None).await;
    assert_eq!(detail["user"]["hold"], true, "SCIM never clears a hold");
}

// 3. A hold stops the user's tokens at the next request.
#[tokio::test]
async fn a_hold_stops_the_users_tokens_and_scim_cannot_lift_it() {
    let c = ctx().await;
    let user_name = unique("t3");
    let user = create_user(&c, &user_name, "faculty").await;
    let id = user["id"].as_str().unwrap();
    let token = issue_test_token_and_user(&user_name);

    let (status, _) = send(&c.app, "GET", "/api/v1/workspaces", Some(&token), None).await;
    assert_eq!(status, StatusCode::OK, "before the hold");

    let (status, _) = send(&c.app, "POST", &format!("/api/v1/admin/users/{id}/hold"), Some(&c.admin), Some(json!({ "hold": true, "reason": "Pending review" }))).await;
    assert_eq!(status, StatusCode::OK);
    let (status, _) = send(&c.app, "GET", "/api/v1/workspaces", Some(&token), None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "on hold");

    // A SCIM PUT, even one that says admin_hold is false, does not lift it.
    let (status, _) = send(
        &c.app,
        "PUT",
        &format!("/scim/v2/Users/{id}"),
        Some(SCIM_TOKEN),
        Some(json!({ "userName": user_name, "active": true, "admin_hold": false })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (status, _) = send(&c.app, "GET", "/api/v1/workspaces", Some(&token), None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "still on hold after SCIM");

    let (status, _) = send(&c.app, "POST", &format!("/api/v1/admin/users/{id}/hold"), Some(&c.admin), Some(json!({ "hold": false, "reason": "Review finished" }))).await;
    assert_eq!(status, StatusCode::OK);
    let (status, _) = send(&c.app, "GET", "/api/v1/workspaces", Some(&token), None).await;
    assert_eq!(status, StatusCode::OK, "released");
}

// 4. Revoking tokens leaves the SCIM credential alone.
#[tokio::test]
async fn revoking_tokens_revokes_agent_and_impersonation_tokens_and_not_scim() {
    let c = ctx().await;
    let user_name = unique("t4");
    let user = create_user(&c, &user_name, "faculty").await;
    let id = user["id"].as_str().unwrap();

    let impersonation = issue_test_token_and_user(&user_name);
    let agent_raw = format!("agent-{}", Uuid::new_v4());
    let scim_raw = format!("scim-{}", Uuid::new_v4());
    c.state.persist_api_token(&api_token(&user_name, "agent", &agent_raw)).unwrap();
    c.state.persist_api_token(&api_token(&user_name, "scim", &scim_raw)).unwrap();
    assert_eq!(send(&c.app, "GET", "/api/v1/workspaces", Some(&agent_raw), None).await.0, StatusCode::OK);

    let (status, body) = send(&c.app, "POST", &format!("/api/v1/admin/users/{id}/revoke-tokens"), Some(&c.admin), Some(json!({ "reason": "Lost laptop" }))).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["revoked"], 2, "the agent token and the impersonation token");

    assert_eq!(send(&c.app, "GET", "/api/v1/workspaces", Some(&agent_raw), None).await.0, StatusCode::UNAUTHORIZED);
    assert_eq!(send(&c.app, "GET", "/api/v1/workspaces", Some(&impersonation), None).await.0, StatusCode::UNAUTHORIZED);
    let scim_row = c.state.get_api_token(&hash_token(&scim_raw)).expect("the scim token still exists");
    assert!(scim_row.revoked_at.is_none(), "the scim token is untouched");

    let entry = last_entry(&c).await;
    assert_eq!(entry["decision_type"], "TokenRevoked");
}

// 5. Appointments.
async fn create_dept(c: &Ctx, name: &str, parent: &str) -> String {
    let (status, body) = send(
        &c.app,
        "POST",
        "/api/v1/orgs",
        Some(&c.admin),
        Some(json!({ "name": name, "code": &name[..4].to_uppercase(), "org_type": "Department", "parent_id": parent })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    body["id"].as_str().unwrap().to_string()
}

async fn appoint(c: &Ctx, unit: &str, eppn: &str, affiliation: &str, token: &str) -> (StatusCode, Value) {
    send(
        &c.app,
        "POST",
        &format!("/api/v1/orgs/{unit}/appointments"),
        Some(token),
        Some(json!({ "eppn": eppn, "scoped_affiliation": affiliation, "reason": "Appointment test" })),
    )
    .await
}

#[tokio::test]
async fn appointments_can_be_revoked_by_the_right_people_and_not_when_scim_owns_them() {
    let c = ctx().await;
    let dept = create_dept(&c, "Chemistry", ROOT_UNIT).await;
    let other = create_dept(&c, "Geology", ROOT_UNIT).await;

    let chair = unique("t5chair");
    let deputy = unique("t5deputy");
    create_user(&c, &chair, "faculty").await;
    create_user(&c, &deputy, "faculty").await;
    assert_eq!(appoint(&c, &dept, &chair, "unit_admin", &c.admin).await.0, StatusCode::CREATED);
    assert_eq!(appoint(&c, &dept, &deputy, "unit_admin", &c.admin).await.0, StatusCode::CREATED);
    let chair_token = issue_test_token_and_user(&chair);

    // An Org Unit Admin outside the unit is refused. One inside the unit may revoke a unit_admin.
    let outsider = unique("t5out");
    create_user(&c, &outsider, "faculty").await;
    assert_eq!(appoint(&c, &other, &outsider, "unit_admin", &c.admin).await.0, StatusCode::CREATED);
    let outsider_token = issue_test_token_and_user(&outsider);
    let revoke = |unit: String, eppn: String, aff: &'static str, token: String| {
        let app = c.app.clone();
        async move { send(&app, "DELETE", &format!("/api/v1/orgs/{unit}/appointments/{eppn}/{aff}"), Some(&token), None).await }
    };
    assert_eq!(revoke(dept.clone(), deputy.clone(), "unit_admin", outsider_token).await.0, StatusCode::FORBIDDEN);
    let before = ledger_len(&c).await;
    let (status, body) = revoke(dept.clone(), deputy.clone(), "unit_admin", chair_token.clone()).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(ledger_len(&c).await, before + 1, "one ledger entry");
    assert_eq!(last_entry(&c).await["decision_type"], "AccessRoleRevoked");

    // An Org Unit Admin may not revoke a platform_admin appointment.
    let (status, _) = revoke(ROOT_UNIT.to_string(), "jordan.lee@state.edu".to_string(), "platform_admin", chair_token).await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    // An appointment that SCIM owns is refused with 409, even for a Platform Admin.
    let (status, _) = revoke(ROOT_UNIT.to_string(), "jordan.lee@state.edu".to_string(), "platform_admin", c.admin.clone()).await;
    assert_eq!(status, StatusCode::CONFLICT);

    // Revoking something that is not there is 404.
    let (status, _) = revoke(dept, "nobody@state.edu".to_string(), "unit_admin", c.admin.clone()).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn platform_admin_appointments_are_root_only_and_platform_admin_only() {
    let c = ctx().await;
    let dept = create_dept(&c, "Physics", ROOT_UNIT).await;
    let chair = unique("t5pa");
    create_user(&c, &chair, "faculty").await;
    assert_eq!(appoint(&c, &dept, &chair, "unit_admin", &c.admin).await.0, StatusCode::CREATED);
    let chair_token = issue_test_token_and_user(&chair);
    let newcomer = unique("t5new");
    create_user(&c, &newcomer, "staff").await;

    // Not on a department.
    assert_eq!(appoint(&c, &dept, &newcomer, "platform_admin", &c.admin).await.0, StatusCode::BAD_REQUEST);
    // Not from an Org Unit Admin.
    assert_eq!(appoint(&c, ROOT_UNIT, &newcomer, "platform_admin", &chair_token).await.0, StatusCode::FORBIDDEN);
    // From a Platform Admin on the root unit, with one ledger entry.
    let before = ledger_len(&c).await;
    let (status, body) = appoint(&c, ROOT_UNIT, &newcomer, "platform_admin", &c.admin).await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    assert_eq!(ledger_len(&c).await, before + 1);
    let entry = last_entry(&c).await;
    assert_eq!(entry["decision_type"], "AccessRoleGranted");
    assert_eq!(entry["principal"], "jordan.lee@state.edu");

    // The API appointment can be revoked, because the root still has another platform_admin.
    let (status, _) = send(
        &c.app,
        "DELETE",
        &format!("/api/v1/orgs/{ROOT_UNIT}/appointments/{newcomer}/platform_admin"),
        Some(&c.admin),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
}

#[test]
fn the_last_platform_admin_appointment_cannot_be_revoked() {
    use scaffoldry_server::service::people::{revoke_check, RevokeRefusal};
    use scaffoldry_server::state::RoleRow;
    let root = Uuid::new_v4();
    let role = |eppn: &str, aff: &str, source: &str| RoleRow {
        id: Uuid::new_v4(),
        person_id: Uuid::new_v4(),
        eppn: eppn.to_string(),
        organization_id: root,
        role_title: "t".to_string(),
        scoped_affiliation: aff.to_string(),
        is_primary: true,
        source: source.to_string(),
    };
    let only = role("a@x.edu", "platform_admin", "api");
    assert_eq!(revoke_check(&only, std::slice::from_ref(&only), root), Err(RevokeRefusal::LastPlatformAdmin));

    let second = role("b@x.edu", "platform_admin", "api");
    assert_eq!(revoke_check(&only, &[only.clone(), second], root), Ok(()));

    let scim = role("c@x.edu", "unit_admin", "scim");
    assert_eq!(revoke_check(&scim, std::slice::from_ref(&scim), root), Err(RevokeRefusal::OwnedByRegistry));
}

// 6. Every admin route is registered, guarded, and checks the admin before it reads a body.
#[tokio::test]
async fn every_people_route_is_in_admin_routes_and_refuses_a_faculty_caller() {
    let c = ctx().await;
    let wanted = [
        ("GET", "/admin/users"),
        ("GET", "/admin/users/{id}"),
        ("POST", "/admin/users"),
        ("POST", "/admin/users/{id}/hold"),
        ("POST", "/admin/users/{id}/revoke-tokens"),
        ("GET", "/admin/groups"),
    ];
    for (method, path) in wanted {
        assert!(
            ADMIN_ROUTES.iter().any(|(m, p)| *m == method && *p == path),
            "ADMIN_ROUTES must list {method} {path}"
        );
    }

    let faculty = issue_test_token_and_user("faculty.curie@state.edu");
    let fixed_id = Uuid::new_v4().to_string();
    for &(method, path) in ADMIN_ROUTES {
        let uri = format!("/api/v1{}", path.replace("{id}", &fixed_id));
        assert_eq!(send(&c.app, method, &uri, None, None).await.0, StatusCode::UNAUTHORIZED, "{method} {uri} with no token");
        assert_eq!(send(&c.app, method, &uri, Some(&faculty), None).await.0, StatusCode::FORBIDDEN, "{method} {uri} as faculty");
    }
}

// 7. Every write is one ledger entry in the admin's name, with the stated reason.
#[tokio::test]
async fn each_write_adds_one_ledger_entry_with_the_reason_and_the_matching_type() {
    let c = ctx().await;
    let user_name = unique("t7");

    let before = ledger_len(&c).await;
    let user = create_user(&c, &user_name, "staff").await;
    let id = user["id"].as_str().unwrap().to_string();
    assert_eq!(ledger_len(&c).await, before + 1);
    let entry = last_entry(&c).await;
    assert_eq!(entry["decision_type"], "UserAccessChanged");
    assert_eq!(entry["principal"], "jordan.lee@state.edu");
    assert_eq!(entry["rationale"], "Test fixture");

    let before = ledger_len(&c).await;
    send(&c.app, "POST", &format!("/api/v1/admin/users/{id}/hold"), Some(&c.admin), Some(json!({ "hold": true, "reason": "Because of an inquiry" }))).await;
    assert_eq!(ledger_len(&c).await, before + 1);
    let entry = last_entry(&c).await;
    assert_eq!(entry["decision_type"], "UserAccessChanged");
    assert_eq!(entry["rationale"], "Because of an inquiry");

    let before = ledger_len(&c).await;
    send(&c.app, "POST", &format!("/api/v1/admin/users/{id}/revoke-tokens"), Some(&c.admin), Some(json!({ "reason": "Routine rotation" }))).await;
    assert_eq!(ledger_len(&c).await, before + 1);
    assert_eq!(last_entry(&c).await["decision_type"], "TokenRevoked");

    // A write with no reason is refused, and writes nothing.
    let before = ledger_len(&c).await;
    for body in [json!({ "hold": false }), json!({ "hold": false, "reason": "   " })] {
        let (status, _) = send(&c.app, "POST", &format!("/api/v1/admin/users/{id}/hold"), Some(&c.admin), Some(body)).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }
    assert_eq!(ledger_len(&c).await, before);
    let (_, detail) = send(&c.app, "GET", &format!("/api/v1/admin/users/{id}"), Some(&c.admin), None).await;
    assert_eq!(detail["user"]["hold"], true, "a refused write changes nothing");
}

#[tokio::test]
async fn creating_a_user_that_exists_is_a_conflict_and_input_is_checked() {
    let c = ctx().await;
    let name = unique("t7dup");
    create_user(&c, &name, "staff").await;
    let again = json!({ "userName": name, "name": "Dup", "email": name, "affiliation": "staff", "department": "X", "title": "Y", "reason": "Again" });
    assert_eq!(send(&c.app, "POST", "/api/v1/admin/users", Some(&c.admin), Some(again)).await.0, StatusCode::CONFLICT);

    let bad_affiliation = json!({ "userName": unique("t7bad"), "name": "B", "email": "b@x.edu", "affiliation": "wizard", "department": "X", "title": "Y", "reason": "Test" });
    assert_eq!(send(&c.app, "POST", "/api/v1/admin/users", Some(&c.admin), Some(bad_affiliation)).await.0, StatusCode::BAD_REQUEST);
    let no_name = json!({ "name": "B", "affiliation": "staff", "reason": "Test" });
    assert_eq!(send(&c.app, "POST", "/api/v1/admin/users", Some(&c.admin), Some(no_name)).await.0, StatusCode::BAD_REQUEST);
    assert_eq!(send(&c.app, "POST", "/api/v1/admin/users/not-a-user/hold", Some(&c.admin), Some(json!({ "hold": true, "reason": "x" }))).await.0, StatusCode::NOT_FOUND);
}

// The detail response and groups.
#[tokio::test]
async fn user_detail_shows_appointments_tokens_without_hashes_and_recent_ledger_entries() {
    let c = ctx().await;
    let dept = create_dept(&c, "Astronomy", ROOT_UNIT).await;
    let name = unique("t8");
    let user = create_user(&c, &name, "faculty").await;
    let id = user["id"].as_str().unwrap();
    assert_eq!(appoint(&c, &dept, &name, "unit_admin", &c.admin).await.0, StatusCode::CREATED);
    c.state.persist_api_token(&api_token(&name, "agent", &format!("raw-{}", Uuid::new_v4()))).unwrap();

    let (status, detail) = send(&c.app, "GET", &format!("/api/v1/admin/users/{id}"), Some(&c.admin), None).await;
    assert_eq!(status, StatusCode::OK);
    let appointments = detail["appointments"].as_array().unwrap();
    assert_eq!(appointments.len(), 1);
    assert_eq!(appointments[0]["scoped_affiliation"], "unit_admin");
    assert_eq!(appointments[0]["source"], "api");
    let tokens = detail["tokens"].as_array().unwrap();
    assert_eq!(tokens.len(), 1);
    assert!(!detail.to_string().contains("token_hash"), "no token hash leaves the server");
    assert!(detail["workspace_memberships"].is_array());
    assert!(detail["ledger"].as_array().unwrap().len() <= 20);

    let (_, listed) = send(&c.app, "GET", &format!("/api/v1/admin/users?search={name}"), Some(&c.admin), None).await;
    assert_eq!(listed["users"][0]["active_agent_tokens"], 1);
    assert_eq!(listed["users"][0]["unit_admin_of"].as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn groups_are_listed_read_only_with_a_member_count() {
    let c = ctx().await;
    let (status, _) = send(&c.app, "POST", "/scim/v2/Groups", Some(SCIM_TOKEN), Some(json!({ "displayName": "Biology Faculty" }))).await;
    assert_eq!(status, StatusCode::CREATED);
    let (status, body) = send(&c.app, "GET", "/api/v1/admin/groups", Some(&c.admin), None).await;
    assert_eq!(status, StatusCode::OK);
    let group = body["groups"].as_array().unwrap().iter().find(|g| g["name"] == "Biology Faculty").expect("the group");
    assert_eq!(group["member_count"], 0);
}
