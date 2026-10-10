use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::Router;
use http_body_util::BodyExt;
use scaffoldry_core::ledger::{DecisionType, LedgerEntry, LedgerHashInput, GENESIS_PREVIOUS_HASH};
use scaffoldry_server::{
    build_app_with_state,
    service::{
        admin::ADMIN_ROUTES,
        organizations::{is_platform_admin, unit_in_scope, OrgCaller},
    },
    state::{OrganizationNode, RoleRow, ServerState},
};
use serde_json::Value;
use std::sync::Arc;
use tower::ServiceExt;
use uuid::Uuid;

async fn send_req(
    app: &Router,
    method: &str,
    uri: &str,
    token: Option<&str>,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let mut b = Request::builder().method(method).uri(uri);
    if let Some(t) = token {
        b = b.header("authorization", format!("Bearer {t}"));
    }
    let req = match body {
        Some(v) => b
            .header("content-type", "application/json")
            .body(Body::from(serde_json::to_vec(&v).unwrap()))
            .unwrap(),
        None => b.body(Body::empty()).unwrap(),
    };
    let resp = app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let val = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    (status, val)
}

#[test]
fn test_decision_types_properties_and_pinned_hashes() {
    assert_eq!(DecisionType::ALL.len(), 22);

    for &dt in DecisionType::ALL {
        let s = dt.as_str();
        assert_eq!(s, format!("{dt:?}"), "as_str must equal Debug representation");
        assert_eq!(DecisionType::parse(s), Some(dt), "parse(as_str) must roundtrip");
    }

    // Pinned hashes for each of the original 14 variants
    let fixed_input = |dt: &'static DecisionType| LedgerHashInput {
        sequence: 1,
        timestamp_iso: "2026-10-10T00:00:00Z",
        previous_hash: GENESIS_PREVIOUS_HASH,
        principal: "admin.user@state.edu",
        organization_code: "DIV-TEST",
        app_slug: Some("test-slug"),
        decision_type: dt,
        oscal_control_id: "AC-02",
        rationale: "Fixed test rationale for pinning hashes",
        payload_hash: "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    };

    let pinned = [
        (&DecisionType::AppPublished, "fe0806f4832ffb7b77fd42dbc3704f44c470175dbed03b69eb5033b4ec761012"),
        (&DecisionType::VanityDnsBound, "1bf5149264a34fbe05a08dbd027d64e1eb06ff31c2ce7649043ef798d6b58894"),
        (&DecisionType::PolicyRevision, "f1c920d08262fe3b3659d003bcaf21e2bb5a1991c5541a1db514a8dc52015f42"),
        (&DecisionType::WorkflowRuleApproved, "5fef9ebfa0f07ca63bcb1e4e9efab5acc6b4d5ac5802c27ad44a29e0a33e1dd4"),
        (&DecisionType::AccessRoleGranted, "c07759de118085af71510a06abb1e24b0511d9c8fcd7d2cb0894333ce9d85b57"),
        (&DecisionType::DatasetAccessShared, "a2e7951ea20bb117ad170ab145a9bb6eaa94f717335113bfc841354b3f9c61d2"),
        (&DecisionType::StatutoryAttestation, "0d8674b7bc7c22e184542b50773565313d7890b79776ddef3ddd948be15eab9a"),
        (&DecisionType::ImpersonationSessionStarted, "76c0d6e7d17160eec0dd3bbce74b64c945f91f5c23260f704625acf9bb67008e"),
        (&DecisionType::ImpersonationSessionEnded, "2189acf7d60225711e9915bf6f48fe3e18cd8a7a7f39e5c25e6645442511cfd8"),
        (&DecisionType::WorkspaceCreated, "3854eba3c3e794092e680afe9d3783b871637902d71d5f14f7eef55a31e1e48e"),
        (&DecisionType::WorkspaceUpdated, "fb290694f5e68fdcce625eddf8a40f55e567a68aa786c533c539f668a3029d79"),
        (&DecisionType::WorkspaceMemberAdded, "fafe7dc9ca5614b8a594a85d953d92d0308e5a8b8f3bc1cf3b1c04ca6faf0f4a"),
        (&DecisionType::WorkspaceMemberRemoved, "4bb35d5cfbc40e64c890d0667ed4a56a99ce623c521ef5e6f14869708aa41cc5"),
        (&DecisionType::WorkspaceMemberRoleUpdated, "71cd9c5b921a6afd8f5bfc86aa6da8e61d8f904c75dc3d5a27646851376f0853"),
    ];

    for (variant, expected_hash) in pinned {
        let computed = LedgerEntry::compute_entry_hash(&fixed_input(variant));
        assert_eq!(computed, expected_hash, "Hash mismatch for {:?}", variant);
    }
}

#[test]
fn test_is_platform_admin_and_unit_in_scope_agreement() {
    let root_id = Uuid::new_v4();
    let dept_id = Uuid::new_v4();

    let orgs = vec![
        OrganizationNode {
            id: root_id,
            parent_id: None,
            code: "INST-ROOT".to_string(),
            name: "State University".to_string(),
            org_type: "institution".to_string(),
        },
        OrganizationNode {
            id: dept_id,
            parent_id: Some(root_id),
            code: "DEPT-CS".to_string(),
            name: "Computer Science".to_string(),
            org_type: "department".to_string(),
        },
    ];

    let platform_admin_eppn = "admin.root@state.edu";
    let central_admin_eppn = "central.admin@state.edu";
    let regular_eppn = "faculty@state.edu";

    let roles = vec![
        RoleRow {
            id: Uuid::new_v4(),
            person_id: Uuid::new_v4(),
            organization_id: root_id,
            eppn: platform_admin_eppn.to_string(),
            scoped_affiliation: "platform_admin".to_string(),
            role_title: "Platform Administrator".to_string(),
            is_primary: true,
            source: "api".to_string(),
        },
    ];

    let pa_caller = OrgCaller {
        eppn: platform_admin_eppn.to_string(),
        affiliation: "faculty".to_string(),
    };
    let ca_caller = OrgCaller {
        eppn: central_admin_eppn.to_string(),
        affiliation: "central_admin".to_string(),
    };
    let reg_caller = OrgCaller {
        eppn: regular_eppn.to_string(),
        affiliation: "faculty".to_string(),
    };

    assert!(is_platform_admin(&pa_caller, &orgs, &roles));
    assert!(is_platform_admin(&ca_caller, &orgs, &roles));
    assert!(!is_platform_admin(&reg_caller, &orgs, &roles));

    assert!(unit_in_scope(&pa_caller, root_id, &orgs, &roles));
    assert!(unit_in_scope(&pa_caller, dept_id, &orgs, &roles));
    assert!(unit_in_scope(&ca_caller, root_id, &orgs, &roles));
    assert!(unit_in_scope(&ca_caller, dept_id, &orgs, &roles));
    assert!(!unit_in_scope(&reg_caller, root_id, &orgs, &roles));
    assert!(!unit_in_scope(&reg_caller, dept_id, &orgs, &roles));
}

#[tokio::test]
async fn test_admin_routes_enforce_platform_admin_and_auth() {
    let state = Arc::new(ServerState::new().expect("server state"));
    let app = build_app_with_state(state.clone()).expect("app router");

    // 1. ADMIN_ROUTES covers GET /admin/overview
    assert!(
        ADMIN_ROUTES.iter().any(|(m, p)| *m == "GET" && (*p == "/admin/overview" || *p == "/api/v1/admin/overview")),
        "ADMIN_ROUTES must cover GET /admin/overview"
    );

    let faculty_token = scaffoldry_server::service::identity::issue_test_token_and_user("faculty.curie@state.edu");

    for &(method, path) in ADMIN_ROUTES {
        let uri = if path.starts_with("/api/v1") {
            path.to_string()
        } else {
            format!("/api/v1{path}")
        };

        // No token => 401
        let (status_no_token, _) = send_req(&app, method, &uri, None, None).await;
        assert_eq!(
            status_no_token,
            StatusCode::UNAUTHORIZED,
            "Route {method} {uri} without token must return 401"
        );

        // Faculty caller => 403
        let (status_faculty, _) = send_req(&app, method, &uri, Some(&faculty_token), None).await;
        assert_eq!(
            status_faculty,
            StatusCode::FORBIDDEN,
            "Route {method} {uri} with faculty caller must return 403"
        );
    }
}

#[tokio::test]
async fn test_overview_counts_match_fixture() {
    let state = Arc::new(ServerState::new().expect("server state"));

    // 1. Setup 2 units: root and a child department
    let root_id = Uuid::new_v4();
    let dept_id = Uuid::new_v4();
    {
        let mut orgs = state.organizations.write().unwrap();
        orgs.clear();
        orgs.insert(
            root_id,
            OrganizationNode {
                id: root_id,
                parent_id: None,
                code: "INST-ROOT".to_string(),
                name: "State University".to_string(),
                org_type: "institution".to_string(),
            },
        );
        orgs.insert(
            dept_id,
            OrganizationNode {
                id: dept_id,
                parent_id: Some(root_id),
                code: "DEPT-MATH".to_string(),
                name: "Mathematics".to_string(),
                org_type: "department".to_string(),
            },
        );
    }

    // 2. Setup 3 users: 1 central admin (active), 1 on hold, 1 regular active
    let admin_eppn = "admin.lead@state.edu";
    let on_hold_eppn = "student.hold@state.edu";
    let active_eppn = "faculty.user@state.edu";

    {
        let mut users = state.users.write().unwrap();
        users.clear();
        users.insert(
            "u-admin".to_string(),
            scaffoldry_server::state::ScimUser {
                id: "u-admin".to_string(),
                user_name: admin_eppn.to_string(),
                name: serde_json::json!({"formatted": "Admin Lead"}),
                active: true,
                admin_hold: false,
                emails: vec![serde_json::json!({"value": admin_eppn, "primary": true})],
                roles: vec![],
                enterprise_extension: None,
                title: Some("central_admin".to_string()),
            },
        );
        users.insert(
            "u-hold".to_string(),
            scaffoldry_server::state::ScimUser {
                id: "u-hold".to_string(),
                user_name: on_hold_eppn.to_string(),
                name: serde_json::json!({"formatted": "Student Hold"}),
                active: true,
                admin_hold: true,
                emails: vec![serde_json::json!({"value": on_hold_eppn, "primary": true})],
                roles: vec![],
                enterprise_extension: None,
                title: Some("student".to_string()),
            },
        );
        users.insert(
            "u-active".to_string(),
            scaffoldry_server::state::ScimUser {
                id: "u-active".to_string(),
                user_name: active_eppn.to_string(),
                name: serde_json::json!({"formatted": "Faculty User"}),
                active: true,
                admin_hold: false,
                emails: vec![serde_json::json!({"value": active_eppn, "primary": true})],
                roles: vec![],
                enterprise_extension: None,
                title: Some("faculty".to_string()),
            },
        );
    }

    // Assign platform_admin role on root org to admin_eppn
    state.roles.write().unwrap().push(RoleRow {
        id: Uuid::new_v4(),
        person_id: Uuid::new_v4(),
        organization_id: root_id,
        eppn: admin_eppn.to_string(),
        scoped_affiliation: "platform_admin".to_string(),
        role_title: "Platform Administrator".to_string(),
        is_primary: true,
        source: "api".to_string(),
    });

    // 3. Setup 1 workspace
    {
        let mut workspaces = state.workspaces.write().unwrap();
        workspaces.clear();
        workspaces.insert(
            "ws-fixture-1".to_string(),
            scaffoldry_server::state::WorkspaceRecord {
                id: "ws-fixture-1".to_string(),
                name: "Research Analytics".to_string(),
                code: "analytics".to_string(),
                organization: "State University".to_string(),
                department: "Mathematics".to_string(),
                description: "Research data analytics workspace".to_string(),
                icon: "chart".to_string(),
                lead: admin_eppn.to_string(),
                visibility: "restricted".to_string(),
                allowed_affiliations: vec![],
                data_classification: "Internal".to_string(),
                cedar_policy_guard: None,
                created_at: "2026-10-10T00:00:00Z".to_string(),
                organization_id: Some(dept_id),
            },
        );
    }

    // Issue token for admin_eppn
    let admin_token = "sct_fixture_admin_token";
    let token_hash = scaffoldry_server::service::identity::hash_token(admin_token);
    state.api_tokens.write().unwrap().insert(
        token_hash.clone(),
        scaffoldry_server::state::ApiToken {
            id: Uuid::new_v4(),
            token_hash,
            kind: "agent".to_string(),
            eppn: admin_eppn.to_string(),
            label: "Admin Fixture Token".to_string(),
            original_admin: None,
            created_at: chrono::Utc::now(),
            expires_at: chrono::Utc::now() + chrono::Duration::days(1),
            last_used_at: None,
            revoked_at: None,
        },
    );

    let app = build_app_with_state(state).expect("app router");
    let (status, body) = send_req(&app, "GET", "/api/v1/admin/overview", Some(admin_token), None).await;

    assert_eq!(status, StatusCode::OK, "Overview must return 200 OK for admin");

    // Assert counts match fixture: three users, one on hold, two units, one workspace
    assert_eq!(body["organization"]["unit_count"], 2, "Two units");
    assert_eq!(body["workspaces"]["workspace_count"], 1, "One workspace");
    assert_eq!(body["people"]["active"], 2, "Two active users");
    assert_eq!(body["people"]["on_hold"], 1, "One user on hold");
    assert_eq!(body["people"]["inactive"], 0, "Zero inactive users");
    assert_eq!(
        body["people"]["active"].as_i64().unwrap()
            + body["people"]["on_hold"].as_i64().unwrap()
            + body["people"]["inactive"].as_i64().unwrap(),
        3,
        "Total of three users in fixture"
    );

    // Assert processes and server info
    assert_eq!(body["processes"]["waiting_instance_count"], 0);
    assert!(body["server"]["version"].is_string());
}
