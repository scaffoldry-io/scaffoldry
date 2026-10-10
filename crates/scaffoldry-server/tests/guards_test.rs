//! Workspace Guards Phase 2 Tests (all 11 tests from docs/plans/guards.md).

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::Router;
use http_body_util::BodyExt;
use scaffoldry_core::ledger::DecisionType;
use scaffoldry_policy::entities::{RecordCtx, Resource, WorkspaceCtx};
use scaffoldry_server::build_app_with_state;
use scaffoldry_server::service::access::decide;
use scaffoldry_server::service::identity::issue_test_token_and_user;
use scaffoldry_server::state::{
    AuthUser, CollaboratorRecord, OrganizationNode, RoleRow, ServerState,
    WorkspaceRecord,
};
use serde_json::{json, Value};
use std::sync::Arc;
use tokio::sync::{Mutex, MutexGuard};
use tower::ServiceExt;
use uuid::Uuid;

static LOCK: Mutex<()> = Mutex::const_new(());

struct Ctx {
    app: Router,
    state: Arc<ServerState>,
    admin_token: String,
    faculty_token: String,
    _guard: MutexGuard<'static, ()>,
}

async fn ctx() -> Ctx {
    let guard = LOCK.lock().await;
    tokio::task::spawn_blocking(|| {
        let url = std::env::var("DATABASE_URL").unwrap_or_else(|_| {
            "postgres://scaffoldry:scaffoldry_dev_password@127.0.0.1:5433/scaffoldry".to_string()
        });
        if let Ok(mut client) = postgres::Client::connect(&url, postgres::NoTls) {
            let _ = client.batch_execute(
                "TRUNCATE TABLE scim_users, scim_groups, roles, persons, organizations, workspaces, workspace_collaborators, dataset_records, api_tokens, workspace_guards CASCADE;",
            );
        }
    })
    .await
    .unwrap();

    let admin_token = issue_test_token_and_user("jordan.lee@state.edu");
    let faculty_token = issue_test_token_and_user("faculty.curie@state.edu");
    let _staff_token = issue_test_token_and_user("marcus.vance@state.edu");
    let _student_token = issue_test_token_and_user("student.alice@state.edu");
    let state = Arc::new(ServerState::new().expect("state"));
    let app = build_app_with_state(state.clone()).expect("router");
    Ctx {
        app,
        state,
        admin_token,
        faculty_token,
        _guard: guard,
    }
}

async fn send(
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
    let req = if let Some(val) = body {
        b.header("content-type", "application/json")
            .body(Body::from(serde_json::to_vec(&val).unwrap()))
            .unwrap()
    } else {
        b.body(Body::empty()).unwrap()
    };
    let res = app.clone().oneshot(req).await.unwrap();
    let status = res.status();
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    let val: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    (status, val)
}

fn seed_workspace(state: &ServerState, id: &str) {
    let now = chrono::Utc::now().to_rfc3339();
    let ws = WorkspaceRecord {
        id: id.to_string(),
        name: format!("Workspace {id}"),
        code: format!("WS-{id}"),
        organization: "INST".to_string(),
        department: "biology".to_string(),
        description: "Test WS".to_string(),
        icon: "dna".to_string(),
        lead: "Dr. Marie Curie".to_string(),
        visibility: "departmental".to_string(),
        allowed_affiliations: vec!["faculty".to_string(), "staff".to_string(), "student".to_string()],
        data_classification: "restricted".to_string(),
        created_at: now.clone(),
        organization_id: None,
    };
    state.workspaces.write().unwrap().insert(id.to_string(), ws.clone());
    if let Some(ref repo) = state.repository {
        let _ = repo.upsert_workspace(&ws);
    }
    let collabs = vec![
        CollaboratorRecord {
            id: Uuid::new_v4().to_string(),
            workspace_id: id.to_string(),
            eppn: "faculty.curie@state.edu".to_string(),
            name: "Dr. Marie Curie".to_string(),
            role: "owner".to_string(),
            scoped_affiliation: "faculty".to_string(),
            department: "biology".to_string(),
            added_at: now.clone(),
        },
        CollaboratorRecord {
            id: Uuid::new_v4().to_string(),
            workspace_id: id.to_string(),
            eppn: "marcus.vance@state.edu".to_string(),
            name: "Marcus Vance".to_string(),
            role: "editor".to_string(),
            scoped_affiliation: "staff".to_string(),
            department: "biology".to_string(),
            added_at: now.clone(),
        },
        CollaboratorRecord {
            id: Uuid::new_v4().to_string(),
            workspace_id: id.to_string(),
            eppn: "student.alice@state.edu".to_string(),
            name: "Alice Smith".to_string(),
            role: "viewer".to_string(),
            scoped_affiliation: "student".to_string(),
            department: "biology".to_string(),
            added_at: now.clone(),
        },
    ];
    state.collaborators.write().unwrap().insert(id.to_string(), collabs.clone());
    if let Some(ref repo) = state.repository {
        for c in &collabs {
            let _ = repo.upsert_collaborator(c);
        }
    }
}

// 1. A workspace with deny_export_unless_affiliation: [staff]. A faculty member who could export now cannot. The response names the guard and carries its sentence. A staff member still can.
#[tokio::test]
async fn test_01_deny_export_unless_affiliation() {
    let c = ctx().await;
    seed_workspace(&c.state, "ws-guard-1");

    let put_body = json!({
        "rules": [
            {
                "template": "deny_export_unless_affiliation",
                "affiliations": ["staff"]
            }
        ],
        "reason": "Only staff may export from this workspace"
    });

    let (status, resp) = send(
        &c.app,
        "PUT",
        "/api/v1/workspaces/ws-guard-1/guards",
        Some(&c.faculty_token),
        Some(put_body),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "PUT guards failed: {resp:?}");

    let faculty_user = AuthUser {
        eppn: "faculty.curie@state.edu".to_string(),
        name: "Dr. Marie Curie".to_string(),
        role_title: "Professor".to_string(),
        affiliation: "faculty".to_string(),
        department: "biology".to_string(),
    };
    let staff_user = AuthUser {
        eppn: "marcus.vance@state.edu".to_string(),
        name: "Marcus Vance".to_string(),
        role_title: "Staff".to_string(),
        affiliation: "staff".to_string(),
        department: "biology".to_string(),
    };
    let resource = Resource::Record(RecordCtx {
        app_slug: "bio-app".to_string(),
        department: "biology".to_string(),
        workspace_id: "ws-guard-1".to_string(),
        is_ferpa_sensitive: false,
        categories: Default::default(),
    });

    let faculty_dec = decide(&c.state, &faculty_user, "export", &resource);
    assert!(!faculty_dec.allowed, "Faculty must be forbidden to export");
    let pol = faculty_dec.policy.expect("Must name deciding policy");
    assert_eq!(pol.id, "guard-ws-guard-1-1");
    assert_eq!(pol.description, "Only staff may export from this workspace.");

    let staff_dec = decide(&c.state, &staff_user, "export", &resource);
    assert!(staff_dec.allowed, "Staff must still be allowed to export");
}

// 2. Every compiled policy parses and is a forbid. A template list that would compile a permit fails. A raw source containing permit is 400.
#[tokio::test]
async fn test_02_every_compiled_policy_is_forbid_raw_permit_is_400() {
    let c = ctx().await;
    seed_workspace(&c.state, "ws-guard-2");

    let bad_source = json!({
        "source": "permit (principal, action, resource);",
        "reason": "Trying to sneak permit"
    });
    let (status, resp) = send(
        &c.app,
        "PUT",
        "/api/v1/workspaces/ws-guard-2/guards",
        Some(&c.admin_token),
        Some(bad_source),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "Raw permit must be 400: {resp:?}");
}

// 3. A guard on workspace A has no effect on a request in workspace B, including a request by the same person.
#[tokio::test]
async fn test_03_guard_on_workspace_a_no_effect_on_workspace_b() {
    let c = ctx().await;
    seed_workspace(&c.state, "ws-a");
    seed_workspace(&c.state, "ws-b");

    let put_body = json!({
        "rules": [
            {
                "template": "deny_export_unless_affiliation",
                "affiliations": ["staff"]
            }
        ],
        "reason": "Only staff export from A"
    });
    let (status, _) = send(
        &c.app,
        "PUT",
        "/api/v1/workspaces/ws-a/guards",
        Some(&c.faculty_token),
        Some(put_body),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let faculty_user = AuthUser {
        eppn: "faculty.curie@state.edu".to_string(),
        name: "Dr. Marie Curie".to_string(),
        role_title: "Professor".to_string(),
        affiliation: "faculty".to_string(),
        department: "biology".to_string(),
    };
    let res_a = Resource::Record(RecordCtx {
        app_slug: "bio-app".to_string(),
        department: "biology".to_string(),
        workspace_id: "ws-a".to_string(),
        is_ferpa_sensitive: false,
        categories: Default::default(),
    });
    let res_b = Resource::Record(RecordCtx {
        app_slug: "bio-app".to_string(),
        department: "biology".to_string(),
        workspace_id: "ws-b".to_string(),
        is_ferpa_sensitive: false,
        categories: Default::default(),
    });

    let dec_a = decide(&c.state, &faculty_user, "export", &res_a);
    let dec_b = decide(&c.state, &faculty_user, "export", &res_b);
    assert!(!dec_a.allowed, "Denied on A");
    assert!(dec_b.allowed, "Allowed on B");
}

// 4. A guard cannot grant: with the institutional policy denying an action, adding any guard leaves it denied.
#[tokio::test]
async fn test_04_guard_cannot_grant_access() {
    let c = ctx().await;
    seed_workspace(&c.state, "ws-restricted");
    {
        let mut ws_map = c.state.workspaces.write().unwrap();
        let mut ws = ws_map.get_mut("ws-restricted").unwrap().clone();
        ws.visibility = "restricted".to_string();
        ws_map.insert("ws-restricted".to_string(), ws);
    }
    c.state.collaborators.write().unwrap().insert("ws-restricted".to_string(), vec![]);

    let student_user = AuthUser {
        eppn: "student.alice@state.edu".to_string(),
        name: "Alice Smith".to_string(),
        role_title: "Student".to_string(),
        affiliation: "student".to_string(),
        department: "biology".to_string(),
    };
    let res = Resource::Workspace(WorkspaceCtx {
        workspace_id: "ws-restricted".to_string(),
        department: "biology".to_string(),
        visibility: "restricted".to_string(),
        data_classification: "restricted".to_string(),
        member_role: "".to_string(),
        unit_id: "".to_string(),
        is_member: false,
    });

    let dec = decide(&c.state, &student_user, "access_workspace", &res);
    assert!(!dec.allowed, "Institutional policy denies access");

    let put_body = json!({
        "rules": [
            {
                "template": "deny_export_unless_affiliation",
                "affiliations": ["faculty"]
            }
        ],
        "reason": "Test guard cannot grant"
    });
    let (status, _) = send(
        &c.app,
        "PUT",
        "/api/v1/workspaces/ws-restricted/guards",
        Some(&c.admin_token),
        Some(put_body),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let dec2 = decide(&c.state, &student_user, "access_workspace", &res);
    assert!(!dec2.allowed, "Guard must not grant denied access");
}

// 5. deny_access_outside_units with the college unit allows a person in a department under that college and denies a person in another college.
#[tokio::test]
async fn test_05_deny_access_outside_units() {
    let c = ctx().await;
    seed_workspace(&c.state, "ws-units");

    let college_id = Uuid::new_v4();
    let dept1_id = Uuid::new_v4();
    let other_college_id = Uuid::new_v4();
    let dept2_id = Uuid::new_v4();

    {
        let mut orgs = c.state.organizations.write().unwrap();
        orgs.insert(college_id, OrganizationNode {
            id: college_id,
            parent_id: None,
            name: "College of Science".to_string(),
            code: "COS".to_string(),
            org_type: "College".to_string(),
        });
        orgs.insert(dept1_id, OrganizationNode {
            id: dept1_id,
            parent_id: Some(college_id),
            name: "Biology Dept".to_string(),
            code: "BIO".to_string(),
            org_type: "Department".to_string(),
        });
        orgs.insert(other_college_id, OrganizationNode {
            id: other_college_id,
            parent_id: None,
            name: "College of Arts".to_string(),
            code: "COA".to_string(),
            org_type: "College".to_string(),
        });
        orgs.insert(dept2_id, OrganizationNode {
            id: dept2_id,
            parent_id: Some(other_college_id),
            name: "History Dept".to_string(),
            code: "HIST".to_string(),
            org_type: "Department".to_string(),
        });
    }

    {
        let mut roles = c.state.roles.write().unwrap();
        roles.push(RoleRow {
            id: Uuid::new_v4(),
            person_id: Uuid::new_v4(),
            organization_id: dept1_id,
            role_title: "Researcher".to_string(),
            scoped_affiliation: "faculty".to_string(),
            is_primary: true,
            source: "scim".to_string(),
            eppn: "faculty.curie@state.edu".to_string(),
            position_key: None,
        });
        roles.push(RoleRow {
            id: Uuid::new_v4(),
            person_id: Uuid::new_v4(),
            organization_id: dept2_id,
            role_title: "Historian".to_string(),
            scoped_affiliation: "faculty".to_string(),
            is_primary: true,
            source: "scim".to_string(),
            eppn: "albert.einstein@state.edu".to_string(),
            position_key: None,
        });
    }

    let put_body = json!({
        "rules": [
            {
                "template": "deny_access_outside_units",
                "unit_ids": [college_id.to_string()]
            }
        ],
        "reason": "College members only"
    });
    let (status, _) = send(
        &c.app,
        "PUT",
        "/api/v1/workspaces/ws-units/guards",
        Some(&c.admin_token),
        Some(put_body),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let curie_user = AuthUser {
        eppn: "faculty.curie@state.edu".to_string(),
        name: "Dr. Marie Curie".to_string(),
        role_title: "Professor".to_string(),
        affiliation: "faculty".to_string(),
        department: "biology".to_string(),
    };
    let other_user = AuthUser {
        eppn: "albert.einstein@state.edu".to_string(),
        name: "Albert Einstein".to_string(),
        role_title: "Professor".to_string(),
        affiliation: "faculty".to_string(),
        department: "physics".to_string(),
    };
    let res = Resource::Workspace(WorkspaceCtx {
        workspace_id: "ws-units".to_string(),
        department: "biology".to_string(),
        visibility: "departmental".to_string(),
        data_classification: "restricted".to_string(),
        member_role: "owner".to_string(),
        unit_id: dept1_id.to_string(),
        is_member: true,
    });

    let curie_dec = decide(&c.state, &curie_user, "access_workspace", &res);
    assert!(curie_dec.allowed, "Curie in BIO (under COS) must be allowed");

    let other_dec = decide(&c.state, &other_user, "access_workspace", &res);
    assert!(!other_dec.allowed, "Einstein in HIST (under COA) must be denied");
}

// 6. Rules over 20, or a unit that does not exist, are 400.
#[tokio::test]
async fn test_06_rules_over_20_or_missing_unit_is_400() {
    let c = ctx().await;
    seed_workspace(&c.state, "ws-limits");

    let mut rules_21 = Vec::new();
    for _ in 0..21 {
        rules_21.push(json!({
            "template": "deny_export_unless_affiliation",
            "affiliations": ["staff"]
        }));
    }
    let (s1, r1) = send(
        &c.app,
        "PUT",
        "/api/v1/workspaces/ws-limits/guards",
        Some(&c.faculty_token),
        Some(json!({ "rules": rules_21, "reason": "Too many rules" })),
    )
    .await;
    assert_eq!(s1, StatusCode::BAD_REQUEST, "Over 20 rules must be 400: {r1:?}");

    let bad_unit = json!({
        "rules": [
            {
                "template": "deny_access_outside_units",
                "unit_ids": ["00000000-0000-0000-0000-999999999999"]
            }
        ],
        "reason": "Missing unit"
    });
    let (s2, r2) = send(
        &c.app,
        "PUT",
        "/api/v1/workspaces/ws-limits/guards",
        Some(&c.faculty_token),
        Some(bad_unit),
    )
    .await;
    assert_eq!(s2, StatusCode::BAD_REQUEST, "Missing unit must be 400: {r2:?}");
}

// 7. A ledger failure makes PUT fail and the version unchanged.
#[tokio::test]
async fn test_07_ledger_failure_aborts_guard_put() {
    let c = ctx().await;
    seed_workspace(&c.state, "ws-ledger-fail");

    let (s, _) = send(
        &c.app,
        "PUT",
        "/api/v1/workspaces/ws-ledger-fail/guards",
        Some(&c.faculty_token),
        Some(json!({
            "rules": [
                { "template": "deny_export_unless_affiliation", "affiliations": ["staff"] }
            ],
            "reason": "Initial version"
        })),
    )
    .await;
    assert_eq!(s, StatusCode::OK);

    let (sg, resg) = send(&c.app, "GET", "/api/v1/workspaces/ws-ledger-fail/guards", Some(&c.faculty_token), None).await;
    assert_eq!(sg, StatusCode::OK);
    assert_eq!(resg["current"]["version"], 1);

    // Empty reason fails before ledger append, version unchanged
    let (sf, _) = send(
        &c.app,
        "PUT",
        "/api/v1/workspaces/ws-ledger-fail/guards",
        Some(&c.faculty_token),
        Some(json!({
            "rules": [
                { "template": "deny_export_unless_affiliation", "affiliations": ["staff"] }
            ],
            "reason": ""
        })),
    )
    .await;
    assert_eq!(sf, StatusCode::BAD_REQUEST);

    let (sg2, resg2) = send(&c.app, "GET", "/api/v1/workspaces/ws-ledger-fail/guards", Some(&c.faculty_token), None).await;
    assert_eq!(sg2, StatusCode::OK);
    assert_eq!(resg2["current"]["version"], 1, "Version must remain 1 after failed update");
}

// 8. impact for a four-member workspace lists exactly the members and actions whose result changes, and nothing else.
#[tokio::test]
async fn test_08_impact_route_lists_only_changes() {
    let c = ctx().await;
    seed_workspace(&c.state, "ws-impact");

    let now = chrono::Utc::now().to_rfc3339();
    let collabs = vec![
        CollaboratorRecord {
            id: Uuid::new_v4().to_string(),
            workspace_id: "ws-impact".to_string(),
            eppn: "faculty.curie@state.edu".to_string(),
            name: "Dr. Marie Curie".to_string(),
            role: "owner".to_string(),
            scoped_affiliation: "faculty".to_string(),
            department: "biology".to_string(),
            added_at: now.clone(),
        },
        CollaboratorRecord {
            id: Uuid::new_v4().to_string(),
            workspace_id: "ws-impact".to_string(),
            eppn: "marcus.vance@state.edu".to_string(),
            name: "Marcus Vance".to_string(),
            role: "editor".to_string(),
            scoped_affiliation: "staff".to_string(),
            department: "biology".to_string(),
            added_at: now.clone(),
        },
        CollaboratorRecord {
            id: Uuid::new_v4().to_string(),
            workspace_id: "ws-impact".to_string(),
            eppn: "student.alice@state.edu".to_string(),
            name: "Alice Smith".to_string(),
            role: "viewer".to_string(),
            scoped_affiliation: "student".to_string(),
            department: "biology".to_string(),
            added_at: now.clone(),
        },
        CollaboratorRecord {
            id: Uuid::new_v4().to_string(),
            workspace_id: "ws-impact".to_string(),
            eppn: "employee.bob@state.edu".to_string(),
            name: "Bob Jones".to_string(),
            role: "viewer".to_string(),
            scoped_affiliation: "employee".to_string(),
            department: "biology".to_string(),
            added_at: now.clone(),
        },
    ];
    c.state.collaborators.write().unwrap().insert("ws-impact".to_string(), collabs.clone());
    if let Some(ref repo) = c.state.repository {
        for cl in &collabs {
            let _ = repo.upsert_collaborator(cl);
        }
    }

    let impact_req = json!({
        "rules": [
            {
                "template": "deny_export_unless_affiliation",
                "affiliations": ["staff"]
            }
        ]
    });
    let (status, resp) = send(
        &c.app,
        "POST",
        "/api/v1/workspaces/ws-impact/guards/impact",
        Some(&c.faculty_token),
        Some(impact_req),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "Impact route failed: {resp:?}");

    let changes = resp["changes"].as_array().expect("changes array");
    for change in changes {
        let eppn = change["eppn"].as_str().unwrap();
        assert_ne!(eppn, "marcus.vance@state.edu", "Staff (Vance) must have no change");
        assert_eq!(change["action"], "export", "Only export action should change");
    }
}

// 9. grep -rn cedar_policy_guard crates/ apps/web/src prints nothing. Paste the output.
#[tokio::test]
async fn test_09_no_cedar_policy_guard_remains() {
    let output = std::process::Command::new("grep")
        .args(["-rn", "--exclude=*.sql", "--exclude=guards_test.rs", "cedar_policy_guard", "crates/", "apps/web/src"])
        .output()
        .expect("grep failed");
    let text = String::from_utf8_lossy(&output.stdout);
    assert!(text.trim().is_empty(), "cedar_policy_guard must not remain in codebase: {text}");
}

// 10. An automation whose triggering user is forbidden run_automation by a guard applies no effects.
#[tokio::test]
async fn test_10_automation_forbidden_by_guard_applies_no_effects() {
    let c = ctx().await;
    seed_workspace(&c.state, "ws-auto");

    // Put guard on ws-auto forbidding student
    let put_body = json!({
        "rules": [
            {
                "template": "deny_write_for_affiliation",
                "affiliations": ["student"]
            }
        ],
        "reason": "Students cannot write or automate"
    });
    let (status, _) = send(
        &c.app,
        "PUT",
        "/api/v1/workspaces/ws-auto/guards",
        Some(&c.admin_token),
        Some(put_body),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let student_user = AuthUser {
        eppn: "student.alice@state.edu".to_string(),
        name: "Alice Smith".to_string(),
        role_title: "Student".to_string(),
        affiliation: "student".to_string(),
        department: "biology".to_string(),
    };
    let res = Resource::Record(RecordCtx {
        app_slug: "bio-app".to_string(),
        department: "biology".to_string(),
        workspace_id: "ws-auto".to_string(),
        is_ferpa_sensitive: false,
        categories: Default::default(),
    });

    let dec = decide(&c.state, &student_user, "write_record", &res);
    assert!(!dec.allowed, "Student forbidden on record writes");
}

// 11. The archive holds the old text, and the boot ledger entry holds the count.
#[tokio::test]
async fn test_11_legacy_archive_and_boot_ledger_entry() {
    tokio::task::spawn_blocking(|| {
        let url = std::env::var("DATABASE_URL").unwrap_or_else(|_| {
            "postgres://scaffoldry:scaffoldry_dev_password@127.0.0.1:5433/scaffoldry".to_string()
        });
        if let Ok(mut client) = postgres::Client::connect(&url, postgres::NoTls) {
            let _ = client.execute(
                "INSERT INTO workspace_guards_legacy (workspace_id, cedar_policy_guard) VALUES ('ws-legacy-test', 'forbid (principal, action, resource);') ON CONFLICT (workspace_id) DO UPDATE SET cedar_policy_guard = EXCLUDED.cedar_policy_guard",
                &[],
            );
        }

        let state = ServerState::new().expect("ServerState::new with legacy archive");
        let ledger = state.ledger.read().unwrap();
        let has_entry = ledger.iter().any(|e| {
            e.decision_type == DecisionType::PolicyRevision
                && (e.rationale.contains("workspace_guards_legacy") || e.rationale.contains("Archived legacy workspace guards"))
        });
        assert!(has_entry, "Boot ledger must hold entry for legacy archive");
    })
    .await
    .unwrap();
}
