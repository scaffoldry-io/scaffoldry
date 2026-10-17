//! Platform admin is a role a person holds on the root unit. Nothing on a person's record grants it.
//! The setup identity is the one way in before any admin exists, and it ends when the first admin is appointed.

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use scaffoldry_server::build_app_with_state;
use scaffoldry_server::service::identity::{create_setup_token, resolve_user};
use tower::ServiceExt;
use scaffoldry_server::state::{OrganizationNode, RoleRow, ScimUser, ServerState};
use serde_json::json;
use std::sync::Arc;
use uuid::Uuid;

const SETUP: &str = "setup@scaffoldry.local";

fn state() -> Arc<ServerState> {
    Arc::new(ServerState::new().expect("state"))
}

fn root(state: &ServerState) -> Uuid {
    let id = Uuid::new_v4();
    state
        .persist_organization(&OrganizationNode { id, parent_id: None, name: "Institution".into(), code: format!("INST-{}", id.simple()), org_type: "Institution".into() })
        .unwrap();
    id
}

fn appoint_platform_admin(state: &ServerState, root: Uuid, eppn: &str) {
    state
        .persist_role(&RoleRow {
            id: Uuid::new_v4(),
            person_id: Uuid::new_v4(),
            eppn: eppn.to_string(),
            organization_id: root,
            role_title: "Platform Administrator".into(),
            scoped_affiliation: "platform_admin".into(),
            is_primary: true,
            source: "api".into(),
            position_key: None,
        })
        .unwrap();
}

fn scim_user(name: &str, title: &str, affiliation: &str) -> ScimUser {
    ScimUser {
        id: format!("u-{name}"),
        user_name: name.to_string(),
        name: json!({ "formatted": name }),
        active: true,
        admin_hold: false,
        emails: vec![],
        roles: vec![json!({ "type": "eduPersonScopedAffiliation", "value": format!("{affiliation}@state.edu") })],
        enterprise_extension: None,
        title: Some(title.to_string()),
    }
}

// A record that says central_admin, by title or by affiliation, grants nothing.
#[test]
fn a_record_that_says_central_admin_grants_nothing() {
    let _db = common::TestDb::new();
    let state = state();
    root(&state);
    state.persist_scim_user(scim_user("claimant@state.edu", "central_admin", "central_admin")).unwrap();
    let who = resolve_user("claimant@state.edu", &state).expect("resolved");
    assert_ne!(who.affiliation, "central_admin");
}

// The setup identity is an admin while there is no admin, can appoint the first one, and is nothing after.
#[test]
fn the_setup_identity_ends_when_the_first_admin_is_appointed() {
    let _db = common::TestDb::new();
    let state = state();
    let root = root(&state);

    let before = resolve_user(SETUP, &state).expect("the setup identity works on first boot");
    assert_eq!(before.affiliation, "central_admin");

    appoint_platform_admin(&state, root, "first.admin@state.edu");

    assert!(resolve_user(SETUP, &state).is_none(), "the setup identity must not outlive the first admin");
    let first = resolve_user("first.admin@state.edu", &state).expect("the first admin resolves");
    assert_eq!(first.affiliation, "central_admin");
}

// The count of platform admins is the people who hold the role, not people whose title says so.
#[test]
fn platform_admin_names_counts_held_roles_only() {
    let _db = common::TestDb::new();
    let state = state();
    let root = root(&state);
    state.persist_scim_user(scim_user("titled@state.edu", "central_admin", "faculty")).unwrap();
    appoint_platform_admin(&state, root, "holder@state.edu");

    let names = state.repository.platform_admin_names().unwrap();
    assert!(names.iter().any(|n| n == "holder@state.edu"), "{names:?}");
    assert!(!names.iter().any(|n| n == "titled@state.edu"), "a title is not a role: {names:?}");
}

// First boot, through the API: the setup token appoints the first admin, and then the token no longer works.
#[tokio::test]
async fn first_boot_the_setup_token_appoints_the_first_admin_and_then_stops_working() {
    let _db = common::TestDb::new();
    let state = state();
    let root = root(&state);
    state.persist_scim_user(scim_user("first.admin@state.edu", "Registrar", "staff")).unwrap();
    let token = create_setup_token(&state).expect("a setup token");
    let app = build_app_with_state(state.clone()).expect("router");

    let send = |method: &'static str, uri: String, body: Option<serde_json::Value>| {
        let app = app.clone();
        let token = token.clone();
        async move {
            let mut b = Request::builder().method(method).uri(uri).header("authorization", format!("Bearer {token}"));
            let req = match body {
                Some(v) => b.header("content-type", "application/json").body(Body::from(v.to_string())).unwrap(),
                None => {
                    b = b.header("accept", "application/json");
                    b.body(Body::empty()).unwrap()
                }
            };
            let resp = app.oneshot(req).await.unwrap();
            let status = resp.status();
            let bytes = resp.into_body().collect().await.unwrap().to_bytes();
            (status, String::from_utf8_lossy(&bytes).to_string())
        }
    };

    let (status, _) = send("GET", "/api/v1/auth/me".into(), None).await;
    assert_eq!(status, StatusCode::OK, "the setup token works on first boot");

    let body = json!({ "eppn": "first.admin@state.edu", "scoped_affiliation": "platform_admin", "reason": "First administrator" });
    let (status, text) = send("POST", format!("/api/v1/orgs/{root}/appointments"), Some(body)).await;
    assert_eq!(status, StatusCode::CREATED, "the setup identity appoints the first admin: {text}");

    let (status, _) = send("GET", "/api/v1/auth/me".into(), None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "the setup token stops working once an admin exists");
}
