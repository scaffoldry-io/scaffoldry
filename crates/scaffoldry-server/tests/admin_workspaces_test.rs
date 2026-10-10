//! Admin console phase 3: Workspaces and apps inventory, updates, and ownership transfers.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::Router;
use http_body_util::BodyExt;
use scaffoldry_engine::AppManifest;
use scaffoldry_server::build_app_with_state;
use scaffoldry_server::service::admin::ADMIN_ROUTES;
use scaffoldry_server::service::identity::issue_test_token_and_user;
use scaffoldry_server::state::{CollaboratorRecord, DatasetRecord, OrganizationNode, ServerState, WorkspaceRecord};
use serde_json::{json, Value};
use std::sync::Arc;
use tokio::sync::{Mutex, MutexGuard};
use tower::ServiceExt;
use uuid::Uuid;

static LOCK: Mutex<()> = Mutex::const_new(());

struct Ctx {
    app: Router,
    state: Arc<ServerState>,
    admin: String,
    faculty: String,
    _guard: MutexGuard<'static, ()>,
}

async fn ctx() -> Ctx {
    let guard = LOCK.lock().await;
    tokio::task::spawn_blocking(|| {
        let url = std::env::var("DATABASE_URL")
            .unwrap_or_else(|_| "postgres://scaffoldry:scaffoldry_dev_password@127.0.0.1:5433/scaffoldry".to_string());
        if let Ok(mut client) = postgres::Client::connect(&url, postgres::NoTls) {
            let _ = client.batch_execute(
                "TRUNCATE TABLE scim_users, scim_groups, roles, persons, organizations, workspaces, workspace_collaborators, dataset_records, api_tokens CASCADE;",
            );
        }
    })
    .await
    .unwrap();

    let admin = issue_test_token_and_user("jordan.lee@state.edu");
    let faculty = issue_test_token_and_user("faculty.curie@state.edu");
    let state = Arc::new(ServerState::new().expect("state"));
    let app = build_app_with_state(state.clone()).expect("router");
    Ctx { app, state, admin, faculty, _guard: guard }
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

async fn last_ledger_entry(c: &Ctx) -> Value {
    let (_, body) = send(&c.app, "GET", "/api/v1/governance/ledger", Some(&c.admin), None).await;
    body["entries"].as_array().unwrap().last().unwrap().clone()
}

#[tokio::test]
async fn every_workspace_route_is_in_admin_routes_and_refuses_a_faculty_caller() {
    let c = ctx().await;
    let required = [
        ("GET", "/admin/workspaces"),
        ("GET", "/admin/workspaces/{id}"),
        ("PATCH", "/admin/workspaces/{id}"),
        ("POST", "/admin/workspaces/{id}/transfer-ownership"),
        ("GET", "/admin/apps"),
    ];

    for (method, path) in required {
        assert!(
            ADMIN_ROUTES.iter().any(|(m, p)| *m == method && *p == path),
            "ADMIN_ROUTES must list {method} {path}"
        );
    }

    for &(method, path) in ADMIN_ROUTES {
        let uri = format!("/api/v1{}", path.replace("{id}", "00000000-0000-0000-0000-0000000000aa"));
        let (no_token_status, _) = send(&c.app, method, &uri, None, None).await;
        assert_eq!(no_token_status, StatusCode::UNAUTHORIZED, "Route {method} {uri} without token must be 401");

        let (faculty_status, _) = send(&c.app, method, &uri, Some(&c.faculty), None).await;
        assert_eq!(faculty_status, StatusCode::FORBIDDEN, "Route {method} {uri} with faculty caller must be 403");
    }
}

#[tokio::test]
async fn test_workspace_fixture_counts_and_single_statement_for_page() {
    let c = ctx().await;
    let ws_id = format!("ws-{}", Uuid::new_v4());
    let ws = WorkspaceRecord {
        id: ws_id.clone(),
        name: "Genome Research Workspace".to_string(),
        code: "GRW".to_string(),
        organization: "Biology Department".to_string(),
        department: "Biology".to_string(),
        description: "Genome workspace for testing".to_string(),
        icon: "dna".to_string(),
        lead: "Lead Scientist".to_string(),
        visibility: "departmental".to_string(),
        allowed_affiliations: vec!["faculty".to_string()],
        data_classification: "Internal".to_string(),
        cedar_policy_guard: None,
        created_at: "2026-10-10T00:00:00Z".to_string(),
        organization_id: None,
    };

    if let Some(ref repo) = c.state.repository {
        repo.upsert_workspace(&ws).unwrap();
    }
    c.state.workspaces.write().unwrap().insert(ws_id.clone(), ws.clone());

    let app1_slug = format!("app-seq-{}", &Uuid::new_v4().to_string()[..6]);
    let app2_slug = format!("app-align-{}", &Uuid::new_v4().to_string()[..6]);

    let m1 = AppManifest {
        slug: app1_slug.clone(),
        title: "Sequencing Tool".to_string(),
        description: "DNA sequencing app".to_string(),
        organization_code: "BIO".to_string(),
        department: "Biology".to_string(),
        workspace_id: Some(ws_id.clone()),
        herm_capability_id: None,
        custom_domain: None,
        custom_domain_verified: false,
        tables: vec![],
        relationships: vec![],
        views: vec![],
        ceds_mappings: std::collections::HashMap::new(),
    };
    let mut m2 = m1.clone();
    m2.slug = app2_slug.clone();
    m2.title = "Alignment Tool".to_string();

    c.state.engine.write().unwrap().register_manifest(m1.clone()).unwrap();
    c.state.engine.write().unwrap().register_manifest(m2.clone()).unwrap();
    if let Some(ref repo) = c.state.repository {
        repo.upsert_app_manifest(&m1).unwrap();
        repo.upsert_app_manifest(&m2).unwrap();
    }

    // Insert 5 records: 3 in app1, 2 in app2
    for i in 0..3 {
        let rec = DatasetRecord {
            id: format!("rec-1-{i}"),
            app_slug: app1_slug.clone(),
            data: json!({ "seq_id": i, "content": "ATCG" }),
            ceds_mapping: json!({}),
            is_ferpa_sensitive: false,
            created_at: "2026-10-10T01:00:00Z".to_string(),
            created_by: None,
        };
        if let Some(ref repo) = c.state.repository {
            repo.upsert_record(&rec).unwrap();
        }
        c.state.records.write().unwrap().entry(app1_slug.clone()).or_default().push(rec);
    }
    for i in 0..2 {
        let rec = DatasetRecord {
            id: format!("rec-2-{i}"),
            app_slug: app2_slug.clone(),
            data: json!({ "align_id": i, "score": 99 }),
            ceds_mapping: json!({}),
            is_ferpa_sensitive: false,
            created_at: "2026-10-10T02:00:00Z".to_string(),
            created_by: None,
        };
        if let Some(ref repo) = c.state.repository {
            repo.upsert_record(&rec).unwrap();
        }
        c.state.records.write().unwrap().entry(app2_slug.clone()).or_default().push(rec);
    }

    let (status, body) = send(&c.app, "GET", &format!("/api/v1/admin/workspaces?search={ws_id}"), Some(&c.admin), None).await;
    assert_eq!(status, StatusCode::OK);
    let rows = body["rows"].as_array().expect("rows array");
    assert_eq!(rows.len(), 1);
    let row = &rows[0];
    assert_eq!(row["id"], ws_id);
    assert_eq!(row["app_count"], 2);
    assert_eq!(row["record_count"], 5);

    // Also verify GET /admin/workspaces/{id}
    let (detail_status, detail_body) = send(&c.app, "GET", &format!("/api/v1/admin/workspaces/{ws_id}"), Some(&c.admin), None).await;
    assert_eq!(detail_status, StatusCode::OK);
    assert_eq!(detail_body["id"], ws_id);
    assert_eq!(detail_body["app_count"], 2);
    assert_eq!(detail_body["record_count"], 5);
    let apps = detail_body["apps"].as_array().expect("apps list");
    assert_eq!(apps.len(), 2);
}

#[tokio::test]
async fn test_patch_moving_workspace_to_another_unit() {
    let c = ctx().await;
    let old_unit_id = Uuid::new_v4();
    let new_unit_id = Uuid::new_v4();

    let old_unit = OrganizationNode {
        id: old_unit_id,
        parent_id: None,
        name: "Old Science Div".to_string(),
        code: "OSD".to_string(),
        org_type: "Division".to_string(),
    };
    let new_unit = OrganizationNode {
        id: new_unit_id,
        parent_id: None,
        name: "New Engineering Div".to_string(),
        code: "NED".to_string(),
        org_type: "Division".to_string(),
    };
    if let Some(ref repo) = c.state.repository {
        repo.upsert_organization(&old_unit).unwrap();
        repo.upsert_organization(&new_unit).unwrap();
    }
    c.state.organizations.write().unwrap().insert(old_unit_id, old_unit);
    c.state.organizations.write().unwrap().insert(new_unit_id, new_unit);

    let ws_id = format!("ws-{}", Uuid::new_v4());
    let ws = WorkspaceRecord {
        id: ws_id.clone(),
        name: "Transfer Unit Workspace".to_string(),
        code: "TUW".to_string(),
        organization: "Old Science Div".to_string(),
        department: "Science".to_string(),
        description: "Moving test".to_string(),
        icon: "box".to_string(),
        lead: "Lead".to_string(),
        visibility: "departmental".to_string(),
        allowed_affiliations: vec![],
        data_classification: "Internal".to_string(),
        cedar_policy_guard: None,
        created_at: "2026-10-10T00:00:00Z".to_string(),
        organization_id: Some(old_unit_id),
    };
    if let Some(ref repo) = c.state.repository {
        repo.upsert_workspace(&ws).unwrap();
    }
    c.state.workspaces.write().unwrap().insert(ws_id.clone(), ws);

    // Faculty caller is 403
    let (faculty_status, _) = send(
        &c.app,
        "PATCH",
        &format!("/api/v1/admin/workspaces/{ws_id}"),
        Some(&c.faculty),
        Some(json!({ "organization_id": new_unit_id, "reason": "Moving unit" })),
    )
    .await;
    assert_eq!(faculty_status, StatusCode::FORBIDDEN);

    // Unknown unit is 400
    let unknown_unit = Uuid::new_v4();
    let (unknown_status, _) = send(
        &c.app,
        "PATCH",
        &format!("/api/v1/admin/workspaces/{ws_id}"),
        Some(&c.admin),
        Some(json!({ "organization_id": unknown_unit, "reason": "Bad move" })),
    )
    .await;
    assert_eq!(unknown_status, StatusCode::BAD_REQUEST);

    // Success PATCH
    let (status, updated) = send(
        &c.app,
        "PATCH",
        &format!("/api/v1/admin/workspaces/{ws_id}"),
        Some(&c.admin),
        Some(json!({ "organization_id": new_unit_id, "reason": "Reorganizing into Engineering" })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(updated["organization_id"], new_unit_id.to_string());

    // Verify ledger entry holds both ids
    let entry = last_ledger_entry(&c).await;
    assert_eq!(entry["decision_type"], "WorkspaceUpdated");
    assert_eq!(entry["principal"], "jordan.lee@state.edu");
    
    // Check payload stored in database
    let payload = c.state.repository.as_ref().unwrap().with_client(|client| {
        let row = client.query_one(
            "SELECT payload FROM governance_ledger WHERE decision_type = 'WorkspaceUpdated' ORDER BY sequence DESC LIMIT 1",
            &[],
        )?;
        let p: Value = row.get(0);
        Ok(p)
    }).unwrap();
    let payload_str = payload.to_string();
    assert!(payload_str.contains(&old_unit_id.to_string()), "Ledger must hold old organization_id: {payload_str}");
    assert!(payload_str.contains(&new_unit_id.to_string()), "Ledger must hold new organization_id: {payload_str}");
}

#[tokio::test]
async fn test_bad_classification_and_visibility_is_400() {
    let c = ctx().await;
    let ws_id = format!("ws-{}", Uuid::new_v4());
    let ws = WorkspaceRecord {
        id: ws_id.clone(),
        name: "Validation Workspace".to_string(),
        code: "VAL".to_string(),
        organization: "IT".to_string(),
        department: "IT".to_string(),
        description: "Validation test".to_string(),
        icon: "shield".to_string(),
        lead: "Lead".to_string(),
        visibility: "departmental".to_string(),
        allowed_affiliations: vec![],
        data_classification: "Internal".to_string(),
        cedar_policy_guard: None,
        created_at: "2026-10-10T00:00:00Z".to_string(),
        organization_id: None,
    };
    c.state.workspaces.write().unwrap().insert(ws_id.clone(), ws);

    // Bad classification
    let (status, _) = send(
        &c.app,
        "PATCH",
        &format!("/api/v1/admin/workspaces/{ws_id}"),
        Some(&c.admin),
        Some(json!({ "data_classification": "Top Secret Ultra", "reason": "Bad classification" })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    // Bad visibility
    let (status_vis, _) = send(
        &c.app,
        "PATCH",
        &format!("/api/v1/admin/workspaces/{ws_id}"),
        Some(&c.admin),
        Some(json!({ "visibility": "world-public", "reason": "Bad visibility" })),
    )
    .await;
    assert_eq!(status_vis, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn test_transfer_ownership_to_new_user_and_held_user_rejection() {
    let c = ctx().await;
    let ws_id = format!("ws-{}", Uuid::new_v4());
    let old_owner_eppn = "alice.oldowner@state.edu".to_string();
    let new_owner_eppn = "bob.newowner@state.edu".to_string();
    let held_user_eppn = "charlie.held@state.edu".to_string();

    let _ = issue_test_token_and_user(&old_owner_eppn);
    let _ = issue_test_token_and_user(&new_owner_eppn);

    // Create held user via /admin/users and hold them
    let (create_status, created_held) = send(
        &c.app,
        "POST",
        "/api/v1/admin/users",
        Some(&c.admin),
        Some(json!({
            "userName": held_user_eppn,
            "name": "Charlie Held",
            "email": held_user_eppn,
            "affiliation": "faculty",
            "department": "Physics",
            "reason": "Creating test user"
        })),
    )
    .await;
    assert_eq!(create_status, StatusCode::CREATED);
    let held_id = created_held["user"]["id"].as_str().unwrap();

    let (hold_status, _) = send(
        &c.app,
        "POST",
        &format!("/api/v1/admin/users/{held_id}/hold"),
        Some(&c.admin),
        Some(json!({ "hold": true, "reason": "Security review" })),
    )
    .await;
    assert_eq!(hold_status, StatusCode::OK);

    // Create workspace with old_owner as owner
    let ws = WorkspaceRecord {
        id: ws_id.clone(),
        name: "Ownership Transfer Workspace".to_string(),
        code: "OTW".to_string(),
        organization: "Engineering".to_string(),
        department: "Engineering".to_string(),
        description: "Ownership test".to_string(),
        icon: "key".to_string(),
        lead: "Alice Oldowner".to_string(),
        visibility: "restricted".to_string(),
        allowed_affiliations: vec![],
        data_classification: "Restricted".to_string(),
        cedar_policy_guard: None,
        created_at: "2026-10-10T00:00:00Z".to_string(),
        organization_id: None,
    };
    let initial_owner = CollaboratorRecord {
        id: Uuid::new_v4().to_string(),
        workspace_id: ws_id.clone(),
        eppn: old_owner_eppn.clone(),
        name: "Alice Oldowner".to_string(),
        role: "owner".to_string(),
        scoped_affiliation: "faculty".to_string(),
        department: "Engineering".to_string(),
        added_at: "2026-10-10T00:00:00Z".to_string(),
    };
    if let Some(ref repo) = c.state.repository {
        repo.upsert_workspace(&ws).unwrap();
        repo.upsert_collaborator(&initial_owner).unwrap();
    }
    c.state.workspaces.write().unwrap().insert(ws_id.clone(), ws);
    c.state.collaborators.write().unwrap().insert(ws_id.clone(), vec![initial_owner]);

    // 1. Transferring to a held user is 400
    let (held_transfer_status, _) = send(
        &c.app,
        "POST",
        &format!("/api/v1/admin/workspaces/{ws_id}/transfer-ownership"),
        Some(&c.admin),
        Some(json!({ "eppn": held_user_eppn, "reason": "Transfer to suspended user" })),
    )
    .await;
    assert_eq!(held_transfer_status, StatusCode::BAD_REQUEST);

    // 2. Transferring to valid user
    let (transfer_status, _) = send(
        &c.app,
        "POST",
        &format!("/api/v1/admin/workspaces/{ws_id}/transfer-ownership"),
        Some(&c.admin),
        Some(json!({ "eppn": new_owner_eppn, "reason": "Leadership transition" })),
    )
    .await;
    assert_eq!(transfer_status, StatusCode::OK);

    // Verify collaborator roles: new owner is owner, old owner is admin, exactly 1 owner
    let collabs = c.state.collaborators.read().unwrap().get(&ws_id).cloned().unwrap();
    let new_owner_collab = collabs.iter().find(|c| c.eppn == new_owner_eppn).expect("new owner in collabs");
    assert_eq!(new_owner_collab.role, "owner");
    let old_owner_collab = collabs.iter().find(|c| c.eppn == old_owner_eppn).expect("old owner in collabs");
    assert_eq!(old_owner_collab.role, "admin");
    let owner_count = collabs.iter().filter(|c| c.role == "owner").count();
    assert_eq!(owner_count, 1, "The workspace always keeps one owner");

    // Verify ledger entry
    let entry = last_ledger_entry(&c).await;
    assert_eq!(entry["decision_type"], "WorkspaceMemberRoleUpdated");
    assert_eq!(entry["principal"], "jordan.lee@state.edu");
    assert_eq!(entry["oscal_control_id"], "AC-02");
    assert_eq!(entry["rationale"], "Leadership transition");
}

#[tokio::test]
async fn test_apps_inventory_list_and_filters() {
    let c = ctx().await;
    let ws_id = format!("ws-{}", Uuid::new_v4());
    let ws = WorkspaceRecord {
        id: ws_id.clone(),
        name: "Course Catalog Workspace".to_string(),
        code: "CCW".to_string(),
        organization: "Registrar".to_string(),
        department: "Registrar".to_string(),
        description: "Catalog workspace".to_string(),
        icon: "book".to_string(),
        lead: "Lead".to_string(),
        visibility: "institutional".to_string(),
        allowed_affiliations: vec![],
        data_classification: "Public".to_string(),
        cedar_policy_guard: None,
        created_at: "2026-10-10T00:00:00Z".to_string(),
        organization_id: None,
    };
    if let Some(ref repo) = c.state.repository {
        repo.upsert_workspace(&ws).unwrap();
    }
    c.state.workspaces.write().unwrap().insert(ws_id.clone(), ws);

    let slug = format!("app-{}", &Uuid::new_v4().to_string()[..6]);

    let manifest = AppManifest {
        slug: slug.clone(),
        title: "Course Catalog App".to_string(),
        description: "Institutional catalog".to_string(),
        organization_code: "REG".to_string(),
        department: "Registrar".to_string(),
        workspace_id: Some(ws_id.clone()),
        herm_capability_id: None,
        custom_domain: None,
        custom_domain_verified: false,
        tables: vec![],
        relationships: vec![],
        views: vec![],
        ceds_mappings: std::collections::HashMap::new(),
    };
    c.state.engine.write().unwrap().register_manifest(manifest.clone()).unwrap();
    if let Some(ref repo) = c.state.repository {
        repo.upsert_app_manifest(&manifest).unwrap();
    }

    let (status, body) = send(&c.app, "GET", &format!("/api/v1/admin/apps?search={slug}"), Some(&c.admin), None).await;
    assert_eq!(status, StatusCode::OK);
    let rows = body["rows"].as_array().expect("rows array");
    assert_eq!(rows.len(), 1);
    let row = &rows[0];
    assert_eq!(row["slug"], slug);
    assert_eq!(row["title"], "Course Catalog App");
    assert_eq!(row["workspace"], ws_id);
    assert!(row.get("version").is_some());
    assert!(row.get("table_count").is_some());
    assert!(row.get("page_count").is_some());
    assert!(row.get("custom_page_count").is_some());
    assert!(row.get("record_count_per_table").is_some());
    assert!(row.get("updated").is_some());
}
