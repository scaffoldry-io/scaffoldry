use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::Router;
use http_body_util::BodyExt;
use scaffoldry_server::{build_app, build_app_with_state};
use scaffoldry_server::state::ServerState;
use serde_json::{json, Value};
use std::sync::Arc;
use tower::ServiceExt;

fn authed_req() -> axum::http::request::Builder {
    let token = scaffoldry_server::jwt::mint_test_jwt(scaffoldry_server::jwt::TestJwtParams {
        eppn: "jordan.lee@state.edu".to_string(),
        name: "Jordan Lee".to_string(),
        role_title: "Central Enterprise Administrator".to_string(),
        affiliation: "central_admin".to_string(),
        department: "Central IT & Institutional Governance".to_string(),
        expires_in_secs: 3600,
    });
    Request::builder().header("authorization", format!("Bearer {token}"))
}

fn user_req(token: &str) -> axum::http::request::Builder {
    Request::builder().header("authorization", format!("Bearer {token}"))
}

async fn login_user(app: &Router, eppn: &str) -> String {
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/auth/token")
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&json!({ "eppn": eppn })).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK, "login failed for {eppn}");
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let body: Value = serde_json::from_slice(&bytes).unwrap();
    body["token"].as_str().unwrap().to_string()
}

#[tokio::test]
async fn test_health_check() {
    let app = build_app().expect("Failed to build router");

    let response = app
        .oneshot(
            Request::builder()
                .uri("/healthz")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = response.into_body().collect().await.unwrap().to_bytes();
    let val: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(val["status"], "healthy");
}

#[tokio::test]
async fn test_scim_2_provisioning_workflow() {
    let mut state = ServerState::new().expect("Failed to initialize state");
    state.scim_token = Some("test-scim-token".to_string());
    let app = build_app_with_state(Arc::new(state)).expect("Failed to build router");

    let scim_req = || Request::builder().header("authorization", "Bearer test-scim-token");

    // 1. ServiceProviderConfig
    let resp = app
        .clone()
        .oneshot(
            scim_req()
                .uri("/scim/v2/ServiceProviderConfig")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = resp.into_body().collect().await.unwrap().to_bytes();
    let config: Value = serde_json::from_slice(&body).unwrap();
    assert!(config["schemas"].as_array().unwrap().contains(&json!("urn:ietf:params:scim:schemas:core:2.0:ServiceProviderConfig")));

    // 2. ResourceTypes
    let resp = app
        .clone()
        .oneshot(
            scim_req()
                .uri("/scim/v2/ResourceTypes")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    // 3. Create User with Enterprise and eduPerson attributes
    let user_payload = json!({
        "schemas": [
            "urn:ietf:params:scim:schemas:core:2.0:User",
            "urn:ietf:params:scim:schemas:extension:enterprise:2.0:User"
        ],
        "userName": "dr.smith@university.edu",
        "name": {
            "formatted": "Dr. Sarah Smith",
            "familyName": "Smith",
            "givenName": "Sarah"
        },
        "active": true,
        "emails": [
            { "value": "dr.smith@university.edu", "primary": true, "type": "work" }
        ],
        "roles": [
            { "value": "faculty@university.edu", "type": "eduPersonScopedAffiliation" }
        ],
        "urn:ietf:params:scim:schemas:extension:enterprise:2.0:User": {
            "department": "Physics",
            "organization": "University College"
        }
    });

    let resp = app
        .clone()
        .oneshot(
            scim_req()
                .method("POST")
                .uri("/scim/v2/Users")
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&user_payload).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);
    let body = resp.into_body().collect().await.unwrap().to_bytes();
    let created_user: Value = serde_json::from_slice(&body).unwrap();
    let user_id = created_user["id"].as_str().unwrap().to_string();
    assert_eq!(created_user["userName"], "dr.smith@university.edu");

    // 4. Get User by ID
    let resp = app
        .clone()
        .oneshot(
            scim_req()
                .uri(format!("/scim/v2/Users/{}", user_id))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    // 5. Create Group
    let group_payload = json!({
        "schemas": ["urn:ietf:params:scim:schemas:core:2.0:Group"],
        "displayName": "Physics Faculty Council",
        "members": [
            { "value": user_id, "display": "Dr. Sarah Smith" }
        ]
    });
    let resp = app
        .clone()
        .oneshot(
            scim_req()
                .method("POST")
                .uri("/scim/v2/Groups")
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&group_payload).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);
}

#[tokio::test]
async fn test_workspaces_and_dataset_collaboration_lifecycle() {
    let app = build_app().expect("Failed to build router");

    // 1. Create Workspace
    let ws_payload = json!({
        "name": "Department of Physics",
        "code": "PHYS",
        "organization": "College of Arts & Sciences"
    });
    let resp = app
        .clone()
        .oneshot(
            authed_req()
                .method("POST")
                .uri("/api/v1/workspaces")
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&ws_payload).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);
    let body = resp.into_body().collect().await.unwrap().to_bytes();
    let created_ws: Value = serde_json::from_slice(&body).unwrap();
    let ws_id = created_ws["id"].as_str().unwrap().to_string();

    // 2. Add Collaborator
    let collab_payload = json!({
        "eppn": "colleague@university.edu",
        "role": "Editor",
        "scoped_affiliation": "faculty@university.edu"
    });
    let resp = app
        .clone()
        .oneshot(
            authed_req()
                .method("POST")
                .uri(format!("/api/v1/workspaces/{}/collaborators", ws_id))
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&collab_payload).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);

    // 3. Create Application / Dataset Manifest
    let app_payload = json!({
        "slug": "physics-grants",
        "title": "Physics Research Grants",
        "description": "Tracking departmental grant proposals and FERPA-sensitive awards",
        "organization_code": "PHYS",
        "department": "physics",
        "herm_capability_id": "RES-01-GRANTS",
        "views": [
            {
                "id": "main-table",
                "title": "All Proposals",
                "view_type": "Table",
                "fields": [
                    { "name": "proposal_title", "label": "Proposal Title", "field_type": "Text", "required": true, "ferpa_sensitive": false },
                    { "name": "student_pi", "label": "Student Co-PI", "field_type": "Text", "required": false, "ferpa_sensitive": true },
                    { "name": "amount", "label": "Requested Amount", "field_type": "Number", "required": true, "ferpa_sensitive": false }
                ]
            }
        ],
        "ceds_mappings": {
            "student_pi": "000033"
        }
    });
    let resp = app
        .clone()
        .oneshot(
            authed_req()
                .method("POST")
                .uri(format!("/api/v1/workspaces/{}/apps", ws_id))
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&app_payload).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);

    // 4. Publish App with Vanity DNS
    let publish_payload = json!({
        "custom_domain": "grants.physics.scaffoldry.internal"
    });
    let resp = app
        .clone()
        .oneshot(
            authed_req()
                .method("POST")
                .uri("/api/v1/apps/physics-grants/publish")
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&publish_payload).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = resp.into_body().collect().await.unwrap().to_bytes();
    let published_app: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(published_app["custom_domain"], "grants.physics.scaffoldry.internal");
    assert_eq!(published_app["custom_domain_verified"], true);

    // 5. Insert Record with FERPA and CEDS evaluation
    let einstein_token = login_user(&app, "einstein@physics.state.edu").await;
    let record_payload = json!({
        "data": {
            "proposal_title": "Quantum Lattice Simulation",
            "student_pi": "Alice Walker",
            "amount": 150000
        }
    });
    let resp = app
        .clone()
        .oneshot(
            user_req(&einstein_token)
                .method("POST")
                .uri("/api/v1/apps/physics-grants/records")
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&record_payload).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);
    let body = resp.into_body().collect().await.unwrap().to_bytes();
    let record: Value = serde_json::from_slice(&body).unwrap();
    let record_id = record["id"].as_str().unwrap().to_string();
    assert_eq!(record["is_ferpa_sensitive"], true);
    assert_eq!(record["ceds_mapping"]["student_pi"], "000033");

    // 6. Query Records
    let resp = app
        .clone()
        .oneshot(
            authed_req()
                .uri("/api/v1/apps/physics-grants/records")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = resp.into_body().collect().await.unwrap().to_bytes();
    let records_list: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(records_list["total"], 1);

    // 7. Update Record
    let update_payload = json!({
        "data": {
            "proposal_title": "Quantum Lattice Simulation (Revised)",
            "student_pi": "Alice Walker",
            "amount": 175000
        }
    });
    let resp = app
        .clone()
        .oneshot(
            authed_req()
                .method("PATCH")
                .uri(format!("/api/v1/apps/physics-grants/records/{}", record_id))
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&update_payload).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    // 8. Delete Record
    let resp = app
        .clone()
        .oneshot(
            authed_req()
                .method("DELETE")
                .uri(format!("/api/v1/apps/physics-grants/records/{}", record_id))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NO_CONTENT);
}

#[tokio::test]
async fn test_cedar_policy_and_governance_endpoints() {
    let app = build_app().expect("Failed to build router");

    // 1. List Policies
    let resp = app
        .clone()
        .oneshot(
            authed_req()
                .uri("/api/v1/policies")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    // 2. Simulate Cedar Policy: Permitted departmental write
    let sim_payload = json!({
        "eppn": "prof.test@physics.edu",
        "affiliation": "faculty",
        "action": "write",
        "app_slug": "physics-grants",
        "department": "physics",
        "is_ferpa_sensitive": false
    });
    let resp = app
        .clone()
        .oneshot(
            authed_req()
                .method("POST")
                .uri("/api/v1/policies/simulate")
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&sim_payload).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = resp.into_body().collect().await.unwrap().to_bytes();
    let sim_result: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(sim_result["decision"], "Allow");

    // 3. Simulate Cedar Policy: Forbidden FERPA export without compliance affiliation
    let sim_forbidden = json!({
        "eppn": "student.reviewer@physics.edu",
        "affiliation": "student",
        "action": "export",
        "app_slug": "physics-grants",
        "department": "physics",
        "is_ferpa_sensitive": true
    });
    let resp = app
        .clone()
        .oneshot(
            authed_req()
                .method("POST")
                .uri("/api/v1/policies/simulate")
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&sim_forbidden).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = resp.into_body().collect().await.unwrap().to_bytes();
    let sim_result: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(sim_result["decision"], "Deny");

    // 4. OSCAL Governance Catalog Export
    let resp = app
        .clone()
        .oneshot(
            authed_req()
                .uri("/api/v1/governance/oscal")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = resp.into_body().collect().await.unwrap().to_bytes();
    let oscal: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(oscal["schema_version"], "1.1.2");
}

#[tokio::test]
async fn test_published_datasets_and_relationships_api() {
    let app = build_app().expect("Failed to build router");

    // 1. List Seeded Published Datasets
    let resp = app
        .clone()
        .oneshot(
            authed_req()
                .uri("/api/v1/datasets")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = resp.into_body().collect().await.unwrap().to_bytes();
    let list: Value = serde_json::from_slice(&body).unwrap();
    assert!(list["total"].as_u64().unwrap() >= 4);

    // 2. Query Specific Dataset with Pre-configured Relationships
    let resp = app
        .clone()
        .oneshot(
            authed_req()
                .uri("/api/v1/datasets/courses")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = resp.into_body().collect().await.unwrap().to_bytes();
    let course_ds: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(course_ds["name"], "University Course Catalog");
    assert!(course_ds["relationships"].as_array().unwrap().len() >= 2);

    // 3. Publish a New Departmental Dataset
    let new_ds = json!({
        "id": "physics_laboratories",
        "name": "Physics Research Laboratories",
        "description": "Departmental laboratory spaces, equipment rosters, and faculty supervisors",
        "department": "Physics",
        "organization": "College of Arts & Sciences",
        "sensitivity_level": "Directory",
        "herm_capability_id": "RES-03-LABS",
        "fields": [
            { "name": "lab_id", "label": "Laboratory ID", "field_type": "Text", "required": true, "ferpa_sensitive": false },
            { "name": "room_number", "label": "Room Number", "field_type": "Text", "required": true, "ferpa_sensitive": false },
            { "name": "supervisor_eppn", "label": "Faculty Supervisor", "field_type": "Relation", "required": true, "ferpa_sensitive": false }
        ],
        "sample_data": [
            { "lab_id": "LAB-PHY-101", "room_number": "Curie Hall 304", "supervisor_eppn": "dr.smith@university.edu" }
        ]
    });
    let resp = app
        .clone()
        .oneshot(
            authed_req()
                .method("POST")
                .uri("/api/v1/datasets")
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&new_ds).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);

    // 4. Create Cross-Dataset Relationship (physics_laboratories.supervisor_eppn -> faculty.eppn)
    let rel_payload = json!({
        "name": "Laboratory Faculty Supervisor",
        "target_dataset_id": "faculty",
        "source_field": "supervisor_eppn",
        "target_field": "eppn",
        "display_field": "full_name",
        "relationship_type": "OneToMany"
    });
    let resp = app
        .clone()
        .oneshot(
            authed_req()
                .method("POST")
                .uri("/api/v1/datasets/physics_laboratories/relationships")
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&rel_payload).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);
    let body = resp.into_body().collect().await.unwrap().to_bytes();
    let created_rel: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(created_rel["target_dataset_id"], "faculty");

    // 5. Query Relationships for physics_laboratories
    let resp = app
        .clone()
        .oneshot(
            authed_req()
                .uri("/api/v1/datasets/physics_laboratories/relationships")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = resp.into_body().collect().await.unwrap().to_bytes();
    let rel_query: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(rel_query["total"], 1);
}

#[tokio::test]
async fn test_mcp_server_protocol_tools_and_resources() {
    let app = build_app().expect("Failed to build router");

    // 1. GET /api/mcp overview
    let resp = app
        .clone()
        .oneshot(
            authed_req()
                .uri("/api/mcp")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = resp.into_body().collect().await.unwrap().to_bytes();
    let overview: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(overview["protocol"], "Model Context Protocol (MCP)");
    assert_eq!(overview["protocol_version"], "2024-11-05");

    // 2. Initialize
    let init_payload = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {}
    });
    let resp = app
        .clone()
        .oneshot(
            authed_req()
                .method("POST")
                .uri("/api/mcp")
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&init_payload).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = resp.into_body().collect().await.unwrap().to_bytes();
    let init_res: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(init_res["result"]["serverInfo"]["name"], "scaffoldry-sovereign-mcp");

    // 3. Tools List
    let tools_payload = json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "tools/list"
    });
    let resp = app
        .clone()
        .oneshot(
            authed_req()
                .method("POST")
                .uri("/api/mcp")
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&tools_payload).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = resp.into_body().collect().await.unwrap().to_bytes();
    let tools_res: Value = serde_json::from_slice(&body).unwrap();
    let tools = tools_res["result"]["tools"].as_array().unwrap();
    assert!(tools.iter().any(|t| t["name"] == "list_datasets"));
    assert!(tools.iter().any(|t| t["name"] == "calculate_formula"));
    assert!(tools.iter().any(|t| t["name"] == "simulate_cedar_policy"));

    // 4. Tools Call - calculate_formula
    let calc_payload = json!({
        "jsonrpc": "2.0",
        "id": 3,
        "method": "tools/call",
        "params": {
            "name": "calculate_formula",
            "arguments": {
                "formula": "SUM",
                "values": [12.5, 27.5, 60.0]
            }
        }
    });
    let resp = app
        .clone()
        .oneshot(
            authed_req()
                .method("POST")
                .uri("/api/mcp")
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&calc_payload).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = resp.into_body().collect().await.unwrap().to_bytes();
    let calc_res: Value = serde_json::from_slice(&body).unwrap();
    let text = calc_res["result"]["content"][0]["text"].as_str().unwrap();
    let parsed_calc: Value = serde_json::from_str(text).unwrap();
    assert_eq!(parsed_calc["result"], 100.0);

    // 5. Resources List
    let res_payload = json!({
        "jsonrpc": "2.0",
        "id": 4,
        "method": "resources/list"
    });
    let resp = app
        .clone()
        .oneshot(
            authed_req()
                .method("POST")
                .uri("/api/mcp")
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&res_payload).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = resp.into_body().collect().await.unwrap().to_bytes();
    let resources_res: Value = serde_json::from_slice(&body).unwrap();
    let resources = resources_res["result"]["resources"].as_array().unwrap();
    assert!(resources.iter().any(|r| r["uri"] == "datasets://catalog"));
    assert!(resources.iter().any(|r| r["uri"] == "policies://cedar"));
}

#[tokio::test]
async fn test_app_workflow_automations_and_simulation() {
    let app = build_app().expect("Failed to build router");

    // 1. List automations for seeded app
    let resp = app
        .clone()
        .oneshot(
            authed_req()
                .uri("/api/v1/apps/physics-admissions-review/automations")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = resp.into_body().collect().await.unwrap().to_bytes();
    let rules: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(rules.as_array().unwrap().len(), 1);
    assert_eq!(rules[0]["name"], "Honors Fellowship Notification");

    // 2. Simulate workflow execution
    let sim_payload = json!({
        "event": {
            "StatusChanged": {
                "to_status": "Approved"
            }
        },
        "record": {
            "applicant_name": "Eleanor Vance",
            "gpa": 3.95,
            "status": "Approved"
        },
        "principal": "dr.smith@university.edu"
    });

    let resp = app
        .clone()
        .oneshot(
            authed_req()
                .method("POST")
                .uri("/api/v1/apps/physics-admissions-review/automations/simulate")
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&sim_payload).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = resp.into_body().collect().await.unwrap().to_bytes();
    let sim_res: Value = serde_json::from_slice(&body).unwrap();
    let results = sim_res.as_array().unwrap();
    assert_eq!(results.len(), 1);
    assert!(results[0]["trigger_matched"].as_bool().unwrap());
    assert!(results[0]["conditions_met"].as_bool().unwrap());
    assert!(results[0]["cedar_authorized"].as_bool().unwrap());
    assert_eq!(results[0]["actions_executed"].as_array().unwrap().len(), 2);
}

#[tokio::test]
async fn test_governance_decision_ledger_and_oscal_export() {
    let app = build_app().expect("Failed to build router");

    // 1. GET /api/v1/governance/ledger - verify initial seeded entries and chain validity
    let resp = app
        .clone()
        .oneshot(
            authed_req()
                .uri("/api/v1/governance/ledger")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = resp.into_body().collect().await.unwrap().to_bytes();
    let ledger_res: Value = serde_json::from_slice(&body).unwrap();
    assert!(ledger_res["chain_valid"].as_bool().unwrap());
    let initial_count = ledger_res["total_entries"].as_u64().unwrap();
    assert!(initial_count >= 4);
    let entries = ledger_res["entries"].as_array().unwrap();
    assert_eq!(entries[0]["sequence"].as_u64().unwrap(), 0);
    assert_eq!(entries[0]["oscal_control_id"], "CM-03");

    // 2. POST /api/v1/governance/ledger/append - append an approval block
    let append_payload = json!({
        "principal": "prof.curie@science.state.edu",
        "organization_code": "DIV-SCIENCES",
        "app_slug": "physics-admissions-review",
        "decision_type": "WorkflowRuleApproved",
        "oscal_control_id": "AC-03",
        "rationale": "Faculty board approved honors admissions trigger rule",
        "payload": {
            "rule": "auto-physics-honors-admit",
            "approver": "prof.curie@science.state.edu"
        }
    });

    let resp = app
        .clone()
        .oneshot(
            authed_req()
                .method("POST")
                .uri("/api/v1/governance/ledger/append")
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&append_payload).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);
    let body = resp.into_body().collect().await.unwrap().to_bytes();
    let append_res: Value = serde_json::from_slice(&body).unwrap();
    assert!(append_res["success"].as_bool().unwrap());
    let appended_seq = append_res["entry"]["sequence"].as_u64().unwrap();
    assert!(appended_seq >= initial_count);
    assert_eq!(append_res["entry"]["oscal_control_id"], "AC-03");

    // 3. POST /api/v1/governance/ledger/verify - cryptographic verification of entire chain
    let resp = app
        .clone()
        .oneshot(
            authed_req()
                .method("POST")
                .uri("/api/v1/governance/ledger/verify")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = resp.into_body().collect().await.unwrap().to_bytes();
    let verify_res: Value = serde_json::from_slice(&body).unwrap();
    assert!(verify_res["verified"].as_bool().unwrap());
    assert!(verify_res["total_entries"].as_u64().unwrap() > appended_seq);

    // 4. GET /api/v1/governance/oscal/export - export official NIST OSCAL 1.1.2 JSON
    let resp = app
        .clone()
        .oneshot(
            authed_req()
                .uri("/api/v1/governance/oscal/export")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = resp.into_body().collect().await.unwrap().to_bytes();
    let oscal_res: Value = serde_json::from_slice(&body).unwrap();
    let comp_def = &oscal_res["component-definition"];
    assert_eq!(comp_def["metadata"]["oscal-version"], "1.1.2");
    let components = comp_def["components"].as_array().unwrap();
    assert_eq!(components[0]["type"], "software");
    let impl_reqs = components[0]["control-implementations"][0]["implemented-requirements"]
        .as_array()
        .unwrap();
    assert!(impl_reqs.len() > appended_seq as usize);

    // 5. Test MCP tools: verify_decision_ledger over JSON-RPC 2.0
    let mcp_verify_payload = json!({
        "jsonrpc": "2.0",
        "id": "test-mcp-verify-1",
        "method": "tools/call",
        "params": {
            "name": "verify_decision_ledger",
            "arguments": {}
        }
    });

    let resp = app
        .clone()
        .oneshot(
            authed_req()
                .method("POST")
                .uri("/api/mcp")
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&mcp_verify_payload).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = resp.into_body().collect().await.unwrap().to_bytes();
    let mcp_res: Value = serde_json::from_slice(&body).unwrap();
    let text = mcp_res["result"]["content"][0]["text"].as_str().unwrap();
    assert!(text.contains("ALL_BLOCKS_VALID"));
}

#[tokio::test]
async fn test_framework_specification_api_and_mcp_integration() {
    let app = build_app().expect("Failed to build router");

    // 1. GET /api/v1/framework/spec - verify JSON Schema and catalog
    let resp = app
        .clone()
        .oneshot(
            authed_req()
                .uri("/api/v1/framework/spec")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = resp.into_body().collect().await.unwrap().to_bytes();
    let spec: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(spec["framework_version"], "1.0.0");
    assert_eq!(spec["standards_alignment"]["tabular_standard"], "TanStack Table v8 Headless Architecture");
    let components = spec["component_catalog"].as_array().unwrap();
    assert_eq!(components.len(), 7);
    assert_eq!(components[0]["type"], "stat-metric");
    assert_eq!(components[1]["type"], "tabular-grid");
    assert!(spec["relational_architecture"]["multi_table_supported"].as_bool().unwrap());

    // 2. MCP JSON-RPC 2.0 tools/call get_framework_spec
    let mcp_tool_payload = json!({
        "jsonrpc": "2.0",
        "id": "test-mcp-framework-1",
        "method": "tools/call",
        "params": {
            "name": "get_framework_spec",
            "arguments": {}
        }
    });

    let resp = app
        .clone()
        .oneshot(
            authed_req()
                .method("POST")
                .uri("/api/mcp")
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&mcp_tool_payload).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = resp.into_body().collect().await.unwrap().to_bytes();
    let mcp_res: Value = serde_json::from_slice(&body).unwrap();
    let content_text = mcp_res["result"]["content"][0]["text"].as_str().unwrap();
    assert!(content_text.contains("TanStack Table v8 Headless Architecture"));

    // 3. MCP JSON-RPC 2.0 resources/read scaffoldry://framework/component-spec
    let mcp_resource_payload = json!({
        "jsonrpc": "2.0",
        "id": "test-mcp-framework-2",
        "method": "resources/read",
        "params": {
            "uri": "scaffoldry://framework/component-spec"
        }
    });

    let resp = app
        .clone()
        .oneshot(
            authed_req()
                .method("POST")
                .uri("/api/mcp")
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&mcp_resource_payload).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = resp.into_body().collect().await.unwrap().to_bytes();
    let mcp_res: Value = serde_json::from_slice(&body).unwrap();
    let content_text = mcp_res["result"]["contents"][0]["text"].as_str().unwrap();
    assert!(content_text.contains("stat-metric"));
}

#[tokio::test]
async fn test_standardized_rest_data_and_metadata_api_parity() {
    let app = build_app().expect("Failed to build router");

    // 1. Create a workspace
    let ws_payload = json!({
        "name": "Collaborative Research Base",
        "code": "COLLAB-BASE",
        "organization": "University Systems"
    });
    let resp = app
        .clone()
        .oneshot(
            authed_req()
                .method("POST")
                .uri("/api/v1/workspaces")
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&ws_payload).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);
    let body = resp.into_body().collect().await.unwrap().to_bytes();
    let ws_res: Value = serde_json::from_slice(&body).unwrap();
    let ws_id = ws_res["id"].as_str().unwrap();

    // 2. Create multi-table app in workspace
    let app_payload = json!({
        "slug": "collab-allocations",
        "title": "Collaborative Research Allocations",
        "description": "Multi-table relational allocations and budget tracking base",
        "organization_code": "COLLAB-BASE",
        "department": "physics",
        "tables": [
            {
                "id": "proposals",
                "name": "Research Proposals",
                "slug": "proposals",
                "fields": [
                    { "name": "title", "label": "Title", "field_type": "Text", "required": true, "ferpa_sensitive": false },
                    { "name": "amount", "label": "Budget", "field_type": "Currency", "required": true, "ferpa_sensitive": false },
                    { "name": "status", "label": "Status", "field_type": "Select", "required": false, "ferpa_sensitive": false }
                ]
            },
            {
                "id": "allocations",
                "name": "Disbursements",
                "slug": "allocations",
                "fields": [
                    { "name": "disbursement_id", "label": "ID", "field_type": "Text", "required": true, "ferpa_sensitive": false },
                    { "name": "proposal_id", "label": "Proposal", "field_type": "Text", "required": true, "ferpa_sensitive": false },
                    { "name": "amount", "label": "Amount", "field_type": "Currency", "required": true, "ferpa_sensitive": false }
                ]
            }
        ],
        "relationships": [
            {
                "id": "rel_proposals_allocations",
                "name": "Proposal Disbursements",
                "source_table_id": "proposals",
                "target_table_id": "allocations",
                "source_field": "title",
                "target_field": "proposal_id",
                "relationship_type": "one_to_many",
                "display_field": "amount"
            }
        ],
        "views": [
            {
                "id": "view-all-proposals",
                "table_id": "proposals",
                "title": "All Proposals",
                "view_type": "Table",
                "fields": [
                    { "name": "title", "label": "Title", "field_type": "Text", "required": true, "ferpa_sensitive": false },
                    { "name": "amount", "label": "Budget", "field_type": "Currency", "required": true, "ferpa_sensitive": false },
                    { "name": "status", "label": "Status", "field_type": "Select", "required": false, "ferpa_sensitive": false }
                ]
            }
        ]
    });

    let resp = app
        .clone()
        .oneshot(
            authed_req()
                .method("POST")
                .uri(format!("/api/v1/workspaces/{}/apps", ws_id))
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&app_payload).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);

    // 3. Test GET /api/v1/apps/collab-allocations/schema
    let resp = app
        .clone()
        .oneshot(
            authed_req()
                .uri("/api/v1/apps/collab-allocations/schema")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = resp.into_body().collect().await.unwrap().to_bytes();
    let schema_res: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(schema_res["app_slug"], "collab-allocations");
    assert_eq!(schema_res["tables"].as_array().unwrap().len(), 2);
    assert_eq!(schema_res["relationships"].as_array().unwrap().len(), 1);

    // 4. Test GET /api/v1/apps/collab-allocations/tables/proposals/schema
    let resp = app
        .clone()
        .oneshot(
            authed_req()
                .uri("/api/v1/apps/collab-allocations/tables/proposals/schema")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = resp.into_body().collect().await.unwrap().to_bytes();
    let table_schema: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(table_schema["table"]["id"], "proposals");
    assert_eq!(table_schema["relationships"].as_array().unwrap().len(), 1);
    assert_eq!(table_schema["views"].as_array().unwrap().len(), 1);

    // 5. POST records to proposals table
    let einstein_token = login_user(&app, "einstein@physics.state.edu").await;
    let rec1 = json!({
        "data": {
            "title": "Quantum Photonics",
            "amount": 250000,
            "status": "Approved"
        }
    });
    let resp = app
        .clone()
        .oneshot(
            user_req(&einstein_token)
                .method("POST")
                .uri("/api/v1/apps/collab-allocations/tables/proposals/records")
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&rec1).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);
    let body = resp.into_body().collect().await.unwrap().to_bytes();
    let r1_res: Value = serde_json::from_slice(&body).unwrap();
    let r1_id = r1_res["id"].as_str().unwrap().to_string();
    assert_eq!(r1_res["data"]["title"], "Quantum Photonics");
    assert_eq!(r1_res["data"]["_table_id"], "proposals");

    let rec2 = json!({
        "data": {
            "title": "Autonomous Marine Robotics",
            "amount": 100000,
            "status": "Review"
        }
    });
    let resp = app
        .clone()
        .oneshot(
            user_req(&einstein_token)
                .method("POST")
                .uri("/api/v1/apps/collab-allocations/tables/proposals/records")
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&rec2).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);

    let rec3 = json!({
        "data": {
            "title": "Deep Biosphere Metagenomics",
            "amount": 400000,
            "status": "Approved"
        }
    });
    let resp = app
        .clone()
        .oneshot(
            user_req(&einstein_token)
                .method("POST")
                .uri("/api/v1/apps/collab-allocations/tables/proposals/records")
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&rec3).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);

    // 6. GET /api/v1/apps/collab-allocations/tables/proposals/records
    let resp = app
        .clone()
        .oneshot(
            authed_req()
                .uri("/api/v1/apps/collab-allocations/tables/proposals/records")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = resp.into_body().collect().await.unwrap().to_bytes();
    let list_res: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(list_res["total"], 3);
    assert_eq!(list_res["records"].as_array().unwrap().len(), 3);

    // 7. GET with filter_by_formula: {amount} > 200000
    let resp = app
        .clone()
        .oneshot(
            authed_req()
                .uri("/api/v1/apps/collab-allocations/tables/proposals/records?filter_by_formula=%7Bamount%7D%20%3E%20200000")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = resp.into_body().collect().await.unwrap().to_bytes();
    let filtered_res: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(filtered_res["total"], 2);

    // 8. GET with sort_field=amount and sort_direction=desc
    let resp = app
        .clone()
        .oneshot(
            authed_req()
                .uri("/api/v1/apps/collab-allocations/tables/proposals/records?sort_field=amount&sort_direction=desc")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = resp.into_body().collect().await.unwrap().to_bytes();
    let sorted_res: Value = serde_json::from_slice(&body).unwrap();
    let sorted_records = sorted_res["records"].as_array().unwrap();
    assert_eq!(sorted_records[0]["data"]["title"], "Deep Biosphere Metagenomics");
    assert_eq!(sorted_records[1]["data"]["title"], "Quantum Photonics");
    assert_eq!(sorted_records[2]["data"]["title"], "Autonomous Marine Robotics");

    // 9. GET with pagination (page_size=2, offset=0)
    let resp = app
        .clone()
        .oneshot(
            authed_req()
                .uri("/api/v1/apps/collab-allocations/tables/proposals/records?page_size=2&offset=0")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = resp.into_body().collect().await.unwrap().to_bytes();
    let paged_res: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(paged_res["total"], 3);
    assert_eq!(paged_res["records"].as_array().unwrap().len(), 2);
    assert_eq!(paged_res["offset"], 2);

    // 10. GET single record by ID
    let resp = app
        .clone()
        .oneshot(
            authed_req()
                .uri(format!("/api/v1/apps/collab-allocations/tables/proposals/records/{}", r1_id))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = resp.into_body().collect().await.unwrap().to_bytes();
    let single_res: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(single_res["id"], r1_id);
    assert_eq!(single_res["data"]["title"], "Quantum Photonics");

    // 11. PATCH record by ID
    let update_payload = json!({
        "data": {
            "title": "Quantum Photonics & Computing",
            "amount": 275000,
            "status": "Approved"
        }
    });
    let resp = app
        .clone()
        .oneshot(
            authed_req()
                .method("PATCH")
                .uri(format!("/api/v1/apps/collab-allocations/tables/proposals/records/{}", r1_id))
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&update_payload).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = resp.into_body().collect().await.unwrap().to_bytes();
    let patched_res: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(patched_res["data"]["title"], "Quantum Photonics & Computing");
    assert_eq!(patched_res["data"]["_table_id"], "proposals");

    // 12. DELETE record by ID
    let resp = app
        .clone()
        .oneshot(
            authed_req()
                .method("DELETE")
                .uri(format!("/api/v1/apps/collab-allocations/tables/proposals/records/{}", r1_id))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NO_CONTENT);

    // Verify record is gone
    let resp = app
        .clone()
        .oneshot(
            authed_req()
                .uri(format!("/api/v1/apps/collab-allocations/tables/proposals/records/{}", r1_id))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn test_authentication_authorization_and_impersonation() {
    let app = build_app().expect("Failed to build router");

    // 1. Directory listing endpoint is deleted (returns 404)
    let resp = app
        .clone()
        .oneshot(
            authed_req()
                .uri("/api/v1/auth/directory")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);

    // 2. Token issue as Faculty (Dr. Sarah Connor)
    let faculty_login_payload = json!({
        "eppn": "sarah.connor@state.edu"
    });
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/auth/token")
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&faculty_login_payload).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = resp.into_body().collect().await.unwrap().to_bytes();
    let faculty_auth: Value = serde_json::from_slice(&body).unwrap();
    let faculty_token = faculty_auth["token"].as_str().unwrap().to_string();
    assert_eq!(faculty_auth["user"]["eppn"], "sarah.connor@state.edu");
    assert_eq!(faculty_auth["user"]["affiliation"], "faculty");
    assert_eq!(faculty_auth["is_impersonating"], false);

    // 3. GET /api/v1/auth/me with Bearer token
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/auth/me")
                .header("authorization", format!("Bearer {}", faculty_token))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = resp.into_body().collect().await.unwrap().to_bytes();
    let me_res: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(me_res["user"]["name"], "Dr. Sarah Connor");

    // 4. Faculty attempts impersonation -> FORBIDDEN (403) via Cedar policy
    let imp_payload = json!({
        "target_eppn": "marcus.vance@state.edu"
    });
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/auth/impersonate")
                .header("authorization", format!("Bearer {}", faculty_token))
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&imp_payload).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);

    // 5. Token issue as Central Admin (Jordan Lee)
    let admin_login_payload = json!({
        "eppn": "jordan.lee@state.edu"
    });
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/auth/token")
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&admin_login_payload).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = resp.into_body().collect().await.unwrap().to_bytes();
    let admin_auth: Value = serde_json::from_slice(&body).unwrap();
    let admin_token = admin_auth["token"].as_str().unwrap().to_string();
    assert_eq!(admin_auth["user"]["affiliation"], "central_admin");

    // 6. Central Admin initiates impersonation of Dr. Sarah Connor
    let imp_payload = json!({
        "target_eppn": "sarah.connor@state.edu"
    });
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/auth/impersonate")
                .header("authorization", format!("Bearer {}", admin_token))
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&imp_payload).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = resp.into_body().collect().await.unwrap().to_bytes();
    let imp_res: Value = serde_json::from_slice(&body).unwrap();
    let imp_token = imp_res["token"].as_str().unwrap().to_string();
    assert_eq!(imp_res["is_impersonating"], true);
    assert_eq!(imp_res["user"]["eppn"], "sarah.connor@state.edu");
    assert_eq!(imp_res["original_admin"]["eppn"], "jordan.lee@state.edu");

    // 7. GET /api/v1/auth/me during active impersonation
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/auth/me")
                .header("authorization", format!("Bearer {}", imp_token))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = resp.into_body().collect().await.unwrap().to_bytes();
    let active_imp: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(active_imp["is_impersonating"], true);
    assert_eq!(active_imp["user"]["eppn"], "sarah.connor@state.edu");

    // 8. Stop impersonation and restore original admin session
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/auth/stop-impersonate")
                .header("authorization", format!("Bearer {}", imp_token))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = resp.into_body().collect().await.unwrap().to_bytes();
    let restore_res: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(restore_res["is_impersonating"], false);
    assert_eq!(restore_res["user"]["eppn"], "jordan.lee@state.edu");

    // 9. Verify cryptographic decision ledger audit entries
    let resp = app
        .clone()
        .oneshot(
            authed_req()
                .uri("/api/v1/governance/ledger")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = resp.into_body().collect().await.unwrap().to_bytes();
    let ledger_res: Value = serde_json::from_slice(&body).unwrap();
    let ledger_entries = ledger_res["entries"].as_array().unwrap();
    assert!(ledger_entries.iter().any(|e| {
        e["decision_type"] == "ImpersonationSessionStarted" && e["oscal_control_id"] == "AC-02"
    }));
    assert!(ledger_entries.iter().any(|e| {
        e["decision_type"] == "ImpersonationSessionEnded" && e["oscal_control_id"] == "AC-02"
    }));
}

#[tokio::test]
async fn test_workspace_sharing_security_and_configuration() {
    let app = build_app().expect("Failed to build router");

    let faculty_token = scaffoldry_server::jwt::mint_test_jwt(scaffoldry_server::jwt::TestJwtParams {
        eppn: "sarah.connor@state.edu".to_string(),
        name: "Dr. Sarah Connor".to_string(),
        role_title: "Department Chair & Professor".to_string(),
        affiliation: "faculty".to_string(),
        department: "Computer Science".to_string(),
        expires_in_secs: 3600,
    });

    // 1. Faculty caller (Dr. Sarah Connor) lists workspaces:
    // Only sees workspaces where they are member/owner (ws-cs-research)
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/workspaces")
                .header("authorization", format!("Bearer {faculty_token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = resp.into_body().collect().await.unwrap().to_bytes();
    let faculty_ws: Vec<Value> = serde_json::from_slice(&body).unwrap();
    assert!(faculty_ws.iter().any(|w| w["id"] == "ws-cs-research"));
    assert!(!faculty_ws.iter().any(|w| w["id"] == "ws-bio-lab"));
    assert!(!faculty_ws.iter().any(|w| w["id"] == "ws-physics-optics"));

    // 2. Faculty caller tries to access restricted ws-bio-lab -> 403 Forbidden
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/workspaces/ws-bio-lab")
                .header("authorization", format!("Bearer {faculty_token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    let body = resp.into_body().collect().await.unwrap().to_bytes();
    let err_res: Value = serde_json::from_slice(&body).unwrap();
    assert!(err_res["error"].as_str().unwrap().contains("Cedar Policy restricts access"));

    // 3. Central admin accesses ws-bio-lab -> 200 OK
    let resp = app
        .clone()
        .oneshot(
            authed_req()
                .uri("/api/v1/workspaces/ws-bio-lab")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = resp.into_body().collect().await.unwrap().to_bytes();
    let admin_ws: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(admin_ws["id"], "ws-bio-lab");

    // 4. Non-owner viewer attempts to add collaborator -> 403 Forbidden
    let new_member_payload = json!({
        "eppn": "guest@state.edu",
        "role": "Viewer",
        "name": "Guest Researcher"
    });
    let marcus_token = login_user(&app, "marcus.vance@state.edu").await;
    let resp = app
        .clone()
        .oneshot(
            user_req(&marcus_token)
                .method("POST")
                .uri("/api/v1/workspaces/ws-bio-lab/collaborators")
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&new_member_payload).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);

    // 5. Owner adds collaborator -> 201 Created
    let curie_token = login_user(&app, "prof.curie@science.state.edu").await;
    let resp = app
        .clone()
        .oneshot(
            user_req(&curie_token)
                .method("POST")
                .uri("/api/v1/workspaces/ws-bio-lab/collaborators")
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&new_member_payload).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);

    // 6. Owner updates workspace configuration -> 200 OK
    let update_payload = json!({
        "description": "Updated genomic protocols and bio-specimen tracking",
        "data_classification": "FERPA Sensitive"
    });
    let resp = app
        .clone()
        .oneshot(
            user_req(&curie_token)
                .method("PUT")
                .uri("/api/v1/workspaces/ws-bio-lab")
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&update_payload).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = resp.into_body().collect().await.unwrap().to_bytes();
    let updated_ws: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(updated_ws["data_classification"], "FERPA Sensitive");

    // 7. Verify audit ledger entries for workspace operations
    let resp = app
        .clone()
        .oneshot(
            authed_req()
                .uri("/api/v1/governance/ledger")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = resp.into_body().collect().await.unwrap().to_bytes();
    let ledger_res: Value = serde_json::from_slice(&body).unwrap();
    let entries = ledger_res["entries"].as_array().unwrap();
    assert!(entries.iter().any(|e| {
        e["decision_type"] == "WorkspaceMemberAdded" && e["oscal_control_id"] == "AC-02"
    }));
    assert!(entries.iter().any(|e| {
        e["decision_type"] == "WorkspaceUpdated" && e["oscal_control_id"] == "AC-03"
    }));
}




