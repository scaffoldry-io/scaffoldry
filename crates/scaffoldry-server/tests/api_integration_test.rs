use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::Router;
use http_body_util::BodyExt;
use scaffoldry_server::{build_app, build_app_with_state};
use scaffoldry_server::state::ServerState;
use serde_json::{json, Value};
use std::sync::Arc;
use tower::ServiceExt;

async fn reset_test_db() {
    tokio::task::spawn_blocking(|| {
        let db_url = std::env::var("DATABASE_URL").unwrap_or_else(|_| {
            "postgres://scaffoldry:scaffoldry_dev_password@127.0.0.1:5433/scaffoldry".to_string()
        });
        if let Ok(mut client) = postgres::Client::connect(&db_url, postgres::NoTls) {
            let _ = client.batch_execute("
                TRUNCATE TABLE organizations CASCADE;
                TRUNCATE TABLE workspaces CASCADE;
                TRUNCATE TABLE app_manifests CASCADE;
                TRUNCATE TABLE dataset_records CASCADE;
                TRUNCATE TABLE published_datasets CASCADE;
                TRUNCATE TABLE dataset_relationships CASCADE;
                TRUNCATE TABLE workflow_automations CASCADE;
                TRUNCATE TABLE process_instances CASCADE;
                TRUNCATE TABLE scim_users CASCADE;
                TRUNCATE TABLE scim_groups CASCADE;
                TRUNCATE TABLE roles CASCADE;
                TRUNCATE TABLE workspace_collaborators CASCADE;
            ");
        }
    }).await.unwrap();
}

fn authed_req() -> axum::http::request::Builder {
    let token = scaffoldry_server::service::identity::issue_test_token_and_user("jordan.lee@state.edu");
    Request::builder().header("authorization", format!("Bearer {token}"))
}

fn user_req(token: &str) -> axum::http::request::Builder {
    Request::builder().header("authorization", format!("Bearer {token}"))
}

async fn login_user(_app: &Router, eppn: &str) -> String {
    scaffoldry_server::service::identity::issue_test_token_and_user(eppn)
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
    let state = ServerState::new().expect("Failed to initialize state");
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

    // Add Einstein as collaborator
    let einstein_collab = json!({
        "eppn": "einstein@physics.state.edu",
        "role": "Editor",
        "scoped_affiliation": "student@physics.state.edu"
    });
    let _ = app.clone().oneshot(
        authed_req()
            .method("POST")
            .uri(format!("/api/v1/workspaces/{}/collaborators", ws_id))
            .header("content-type", "application/json")
            .body(Body::from(serde_json::to_vec(&einstein_collab).unwrap()))
            .unwrap(),
    ).await.unwrap();

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
    assert_eq!(published_app["custom_domain_verified"], false);

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
    let state = std::sync::Arc::new(ServerState::new().expect("Failed to initialize state"));
    state.seed_demo().expect("Failed to seed demo data");
    let app = build_app_with_state(state).expect("Failed to build router");

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

    // 1. GET /api/mcp is not an endpoint. The protocol is POST only.
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
    assert_eq!(resp.status(), StatusCode::METHOD_NOT_ALLOWED);

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
    assert!(!tools.iter().any(|t| t["name"] == "calculate_formula"), "the fake formula tool is gone");
    assert!(tools.iter().any(|t| t["name"] == "simulate_cedar_policy"));

    // 4. Tools Call - list_datasets
    let call_payload = json!({
        "jsonrpc": "2.0",
        "id": 3,
        "method": "tools/call",
        "params": { "name": "list_datasets", "arguments": {} }
    });
    let resp = app
        .clone()
        .oneshot(
            authed_req()
                .method("POST")
                .uri("/api/mcp")
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&call_payload).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = resp.into_body().collect().await.unwrap().to_bytes();
    let call_res: Value = serde_json::from_slice(&body).unwrap();
    assert!(call_res["result"]["structuredContent"].is_array());

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
    reset_test_db().await;
    let state = std::sync::Arc::new(ServerState::new().expect("Failed to initialize state"));
    state.seed_demo().expect("Failed to seed demo data");
    let app = build_app_with_state(state).expect("Failed to build router");

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

    // 3. POST the same automation id twice and GET the list. Length stays 1.
    let rule_payload = json!({
        "id": "rule-upsert-test",
        "app_slug": "physics-admissions-review",
        "name": "Upsert Rule",
        "description": "Rule upsert testing",
        "enabled": true,
        "trigger": "RecordCreated",
        "cedar_policy_guard": null,
        "predicates": [],
        "actions": []
    });

    let resp_post1 = app
        .clone()
        .oneshot(
            authed_req()
                .method("POST")
                .uri("/api/v1/apps/physics-admissions-review/automations")
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&rule_payload).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp_post1.status(), StatusCode::CREATED);

    let resp_post2 = app
        .clone()
        .oneshot(
            authed_req()
                .method("POST")
                .uri("/api/v1/apps/physics-admissions-review/automations")
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&rule_payload).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp_post2.status(), StatusCode::CREATED);

    let resp_list = app
        .clone()
        .oneshot(
            authed_req()
                .uri("/api/v1/apps/physics-admissions-review/automations")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp_list.status(), StatusCode::OK);
    let body = resp_list.into_body().collect().await.unwrap().to_bytes();
    let rules_list: Value = serde_json::from_slice(&body).unwrap();
    let matching_rules: Vec<_> = rules_list
        .as_array()
        .unwrap()
        .iter()
        .filter(|r| r["id"] == "rule-upsert-test")
        .collect();
    assert_eq!(matching_rules.len(), 1);
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
    assert_eq!(entries[0]["oscal_control_id"], "PL-02");

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

    // Add einstein as editor collaborator to workspace
    let collab_payload = json!({
        "eppn": "einstein@physics.state.edu",
        "name": "Albert Einstein",
        "role": "editor",
        "scoped_affiliation": "faculty",
        "department": "physics"
    });
    let _ = app.clone().oneshot(
        authed_req()
            .method("POST")
            .uri(format!("/api/v1/workspaces/{}/collaborators", ws_id))
            .header("content-type", "application/json")
            .body(Body::from(serde_json::to_vec(&collab_payload).unwrap()))
            .unwrap(),
    ).await.unwrap();

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
    let faculty_token = login_user(&app, "sarah.connor@state.edu").await;
    let faculty_auth = json!({
        "token": faculty_token.clone(),
        "user": {
            "eppn": "sarah.connor@state.edu",
            "affiliation": "faculty"
        },
        "is_impersonating": false
    });
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
    let admin_token = login_user(&app, "jordan.lee@state.edu").await;
    let admin_auth = json!({
        "token": admin_token.clone(),
        "user": {
            "affiliation": "central_admin"
        }
    });
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
    reset_test_db().await;
    let state = std::sync::Arc::new(ServerState::new().expect("Failed to initialize state"));
    state.seed_demo().expect("Failed to seed demo data");
    let app = build_app_with_state(state).expect("Failed to build router");

    let faculty_token = scaffoldry_server::service::identity::issue_test_token_and_user("sarah.connor@state.edu");

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





#[tokio::test]
async fn test_automation_retrigger_loop_guard() {
    let state = std::sync::Arc::new(scaffoldry_server::state::ServerState::new().expect("Failed to create ServerState"));

    let loop_rule = scaffoldry_core::AutomationRule {
        id: "rule-loop-guard".to_string(),
        app_slug: "test-slug".to_string(),
        name: "Loop Guard Rule".to_string(),
        description: "Rule watching update and updating status".to_string(),
        enabled: true,
        trigger: scaffoldry_core::TriggerEvent::RecordUpdated,
        cedar_policy_guard: None,
        predicates: vec![],
        actions: vec![scaffoldry_core::ActionType::UpdateRecordStatus {
            new_status: "Processed".to_string(),
        }],
        steps: vec![],
    };

    state.automations.write().unwrap().insert("test-slug".to_string(), vec![loop_rule]);

    let initial_rec = json!({
        "id": "rec-1",
        "status": "Pending"
    });
    state.records.write().unwrap().insert(
        "test-slug".to_string(),
        vec![scaffoldry_server::state::DatasetRecord {
            id: "rec-1".to_string(),
            app_slug: "test-slug".to_string(),
            data: initial_rec.clone(),
            ceds_mapping: Default::default(),
            is_ferpa_sensitive: false,
            created_at: "".to_string(),
        }],
    );

    let identity = scaffoldry_core::standards::eduperson::EduPersonIdentity {
        eppn: "dr.smith@university.edu".to_string(),
        realm: "university.edu".to_string(),
        affiliations: vec![scaffoldry_core::standards::eduperson::EduPersonAffiliation::Faculty],
    };

    let mut applied_ids = Vec::new();
    scaffoldry_server::service::records::run_automations(
        &state,
        "test-slug",
        scaffoldry_core::TriggerEvent::RecordUpdated,
        &initial_rec,
        &scaffoldry_server::service::records::Actor { identity: &identity, department: "biology" },
        0,
        &mut applied_ids,
    );

    // Rule was applied exactly once in the re-entry chain due to loop guard
    assert_eq!(applied_ids, vec!["rule-loop-guard".to_string()]);
    let records = state.records.read().unwrap();
    let updated = &records["test-slug"][0];
    assert_eq!(updated.data["status"], "Processed");
}


#[tokio::test]
async fn test_phase_4_decide_approve_as_faculty_and_deny_student() {
    let state = std::sync::Arc::new(scaffoldry_server::state::ServerState::new().expect("Failed to create ServerState"));
    state.seed_demo().expect("Failed to seed demo data");
    let app = build_app_with_state(state.clone()).expect("Failed to build router");

    let app_slug = "physics-admissions";
    let manifest = scaffoldry_engine::AppManifest {
        slug: app_slug.to_string(),
        title: "Physics Admissions".to_string(),
        description: "Review flow".to_string(),
        organization_code: "PHYS".to_string(),
        department: "physics".to_string(),
        workspace_id: Some("ws-physics-optics".to_string()),
        herm_capability_id: None,
        custom_domain: None,
        custom_domain_verified: false,
        tables: vec![],
        relationships: vec![],
        views: vec![],
        ceds_mappings: Default::default(),
    };
    state.engine.write().unwrap().register_manifest(manifest).unwrap();
    state.collaborators.write().unwrap().entry("ws-physics-optics".to_string()).or_default().extend(vec![
        scaffoldry_server::state::CollaboratorRecord {
            id: "collab-curie-phys".to_string(),
            workspace_id: "ws-physics-optics".to_string(),
            eppn: "prof.curie@science.state.edu".to_string(),
            name: "Dr. Marie Curie".to_string(),
            role: "editor".to_string(),
            scoped_affiliation: "faculty".to_string(),
            department: "biology".to_string(),
            added_at: "".to_string(),
        },
        scaffoldry_server::state::CollaboratorRecord {
            id: "collab-smith-phys".to_string(),
            workspace_id: "ws-physics-optics".to_string(),
            eppn: "student.smith@science.state.edu".to_string(),
            name: "Alex Smith".to_string(),
            role: "editor".to_string(),
            scoped_affiliation: "student".to_string(),
            department: "biology".to_string(),
            added_at: "".to_string(),
        },
    ]);
    let rule_id = "rule-candidate-decision";
    let rec_id = "rec-cand-42";

    let rule = scaffoldry_core::AutomationRule {
        id: rule_id.to_string(),
        app_slug: app_slug.to_string(),
        name: "Candidate Review Flow".to_string(),
        description: "Review flow".to_string(),
        enabled: true,
        trigger: scaffoldry_core::TriggerEvent::RecordCreated,
        cedar_policy_guard: None,
        predicates: vec![],
        actions: vec![],
        steps: vec![
            scaffoldry_core::ProcessStep {
                id: "step-approval".to_string(),
                when: vec![],
                kind: scaffoldry_core::StepKind::UserTask {
                    role: "faculty".to_string(),
                    prompt: "Review candidate admission".to_string(),
                    approve: vec![scaffoldry_core::ActionType::UpdateRecordStatus {
                        new_status: "Approved".to_string(),
                    }],
                    reject: vec![scaffoldry_core::ActionType::UpdateRecordStatus {
                        new_status: "Rejected".to_string(),
                    }],
                },
            },
        ],
    };
    state.automations.write().unwrap().insert(app_slug.to_string(), vec![rule]);

    let initial_rec = json!({
        "id": rec_id,
        "status": "UnderReview"
    });
    state.records.write().unwrap().insert(
        app_slug.to_string(),
        vec![scaffoldry_server::state::DatasetRecord {
            id: rec_id.to_string(),
            app_slug: app_slug.to_string(),
            data: initial_rec.clone(),
            ceds_mapping: Default::default(),
            is_ferpa_sensitive: false,
            created_at: "".to_string(),
        }],
    );

    let instance_id = format!("{rule_id}:{rec_id}");
    let initial_instance = scaffoldry_core::ProcessInstance {
        id: instance_id.clone(),
        rule_id: rule_id.to_string(),
        app_slug: app_slug.to_string(),
        record_id: rec_id.to_string(),
        status: scaffoldry_core::ProcessStatus::Waiting,
        waiting_step_id: Some("step-approval".to_string()),
        role: Some("faculty".to_string()),
        prompt: Some("Review candidate admission".to_string()),
        log: vec![],
    };
    state.process_instances.write().unwrap().insert(instance_id.clone(), initial_instance);

    // 1. Student attempts decide -> 403 Forbidden
    let student_token = login_user(&app, "student.smith@science.state.edu").await;
    let resp = app
        .clone()
        .oneshot(
            user_req(&student_token)
                .method("POST")
                .uri(format!("/api/v1/apps/{app_slug}/processes/{instance_id}/decide"))
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&json!({"decision": "approve"})).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);

    // Instance must remain Waiting and record must not have changed
    {
        let instances = state.process_instances.read().unwrap();
        assert_eq!(instances[&instance_id].status, scaffoldry_core::ProcessStatus::Waiting);
        let records = state.records.read().unwrap();
        assert_eq!(records[app_slug][0].data["status"], "UnderReview");
    }

    // 2. Faculty decides approve -> 200 OK
    let faculty_token = login_user(&app, "prof.curie@science.state.edu").await;
    let resp = app
        .clone()
        .oneshot(
            user_req(&faculty_token)
                .method("POST")
                .uri(format!("/api/v1/apps/{app_slug}/processes/{instance_id}/decide"))
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&json!({"decision": "approve"})).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    // Instance status must become Completed and waiting_step_id cleared
    {
        let instances = state.process_instances.read().unwrap();
        let inst = &instances[&instance_id];
        assert_eq!(inst.status, scaffoldry_core::ProcessStatus::Completed);
        assert!(inst.waiting_step_id.is_none());
        let records = state.records.read().unwrap();
        assert_eq!(records[app_slug][0].data["status"], "Approved");
    }

    // Ledger must contain WorkflowRuleApproved entry with AC-03
    {
        let ledger = state.ledger.read().unwrap();
        assert!(ledger.iter().any(|entry| {
            entry.decision_type == scaffoldry_core::DecisionType::WorkflowRuleApproved
                && entry.oscal_control_id == "AC-03"
                && entry.rationale.contains(&instance_id)
        }));
    }
}

#[tokio::test]
async fn test_phase_4_duplicate_trigger_does_not_insert_second_row() {
    let state = std::sync::Arc::new(scaffoldry_server::state::ServerState::new().expect("Failed to create ServerState"));

    let app_slug = "physics-dup-check";
    let rule_id = "rule-dup-check";
    let rec_id = "rec-dup-99";

    let rule = scaffoldry_core::AutomationRule {
        id: rule_id.to_string(),
        app_slug: app_slug.to_string(),
        name: "Dup Check Flow".to_string(),
        description: "User task flow".to_string(),
        enabled: true,
        trigger: scaffoldry_core::TriggerEvent::RecordCreated,
        cedar_policy_guard: None,
        predicates: vec![],
        actions: vec![],
        steps: vec![
            scaffoldry_core::ProcessStep {
                id: "step-user".to_string(),
                when: vec![],
                kind: scaffoldry_core::StepKind::UserTask {
                    role: "chair".to_string(),
                    prompt: "Sign off".to_string(),
                    approve: vec![],
                    reject: vec![],
                },
            },
        ],
    };
    state.automations.write().unwrap().insert(app_slug.to_string(), vec![rule]);

    let record_val = json!({
        "id": rec_id,
        "status": "New"
    });
    let identity = scaffoldry_core::standards::eduperson::EduPersonIdentity {
        eppn: "dr.smith@university.edu".to_string(),
        realm: "university.edu".to_string(),
        affiliations: vec![scaffoldry_core::standards::eduperson::EduPersonAffiliation::Faculty],
    };

    let mut applied_ids = Vec::new();
    scaffoldry_server::service::records::run_automations(
        &state,
        app_slug,
        scaffoldry_core::TriggerEvent::RecordCreated,
        &record_val,
        &scaffoldry_server::service::records::Actor { identity: &identity, department: "biology" },
        0,
        &mut applied_ids,
    );

    let instances_after_first = state.process_instances.read().unwrap().values().filter(|p| p.app_slug == app_slug).count();
    assert_eq!(instances_after_first, 1);

    // Trigger second time with fresh applied_ids
    let mut applied_ids_2 = Vec::new();
    scaffoldry_server::service::records::run_automations(
        &state,
        app_slug,
        scaffoldry_core::TriggerEvent::RecordCreated,
        &record_val,
        &scaffoldry_server::service::records::Actor { identity: &identity, department: "biology" },
        0,
        &mut applied_ids_2,
    );

    let instances_after_second = state.process_instances.read().unwrap().values().filter(|p| p.app_slug == app_slug).count();
    assert_eq!(instances_after_second, 1);
}

#[tokio::test]
async fn test_phase2_organization_api_and_scoping() {
    reset_test_db().await;
    let state = Arc::new(ServerState::new().expect("ServerState::new"));
    let app = build_app_with_state(state.clone()).unwrap();

    let root_id = uuid::Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap();

    // 1. central_admin creates a College and Departments
    let college_payload = json!({
        "name": "College of Sciences",
        "code": "SCI-API",
        "org_type": "College",
        "parent_id": root_id.to_string(),
    });
    let resp = app.clone().oneshot(
        authed_req()
            .method("POST")
            .uri("/api/v1/orgs")
            .header("content-type", "application/json")
            .body(Body::from(serde_json::to_vec(&college_payload).unwrap()))
            .unwrap(),
    ).await.unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);
    let college_body: Value = serde_json::from_slice(&resp.into_body().collect().await.unwrap().to_bytes()).unwrap();
    let college_id = college_body["id"].as_str().unwrap().to_string();

    let dept_phys_payload = json!({
        "name": "Department of Physics",
        "code": "PHYS-API",
        "org_type": "Department",
        "parent_id": college_id,
    });
    let resp = app.clone().oneshot(
        authed_req()
            .method("POST")
            .uri("/api/v1/orgs")
            .header("content-type", "application/json")
            .body(Body::from(serde_json::to_vec(&dept_phys_payload).unwrap()))
            .unwrap(),
    ).await.unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);
    let phys_body: Value = serde_json::from_slice(&resp.into_body().collect().await.unwrap().to_bytes()).unwrap();
    let dept_phys_id = phys_body["id"].as_str().unwrap().to_string();

    // Appoint Physics Chair
    let appoint_payload = json!({
        "eppn": "chair.physics@state.edu",
        "scoped_affiliation": "unit_admin",
    });
    let resp = app.clone().oneshot(
        authed_req()
            .method("POST")
            .uri(format!("/api/v1/orgs/{dept_phys_id}/appointments"))
            .header("content-type", "application/json")
            .body(Body::from(serde_json::to_vec(&appoint_payload).unwrap()))
            .unwrap(),
    ).await.unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);

    // 2. Department Org Unit Admin POST of a sibling under the college returns 403
    let chair_token = scaffoldry_server::service::identity::issue_test_token_and_user("chair.physics@state.edu");
    let chair_req = Request::builder().header("authorization", format!("Bearer {chair_token}"));

    let sibling_dept_payload = json!({
        "name": "Department of Biology",
        "code": "BIO-NEW",
        "org_type": "Department",
        "parent_id": college_id,
    });
    let resp = app.clone().oneshot(
        chair_req
            .method("POST")
            .uri("/api/v1/orgs")
            .header("content-type", "application/json")
            .body(Body::from(serde_json::to_vec(&sibling_dept_payload).unwrap()))
            .unwrap(),
    ).await.unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);

    // 3. Faculty collaborator on one workspace does not receive a sibling department's workspace
    // Create workspace in PHYS
    let ws_phys_payload = json!({
        "name": "Physics Quantum Lab",
        "code": "PQL",
        "organization_id": dept_phys_id,
    });
    let resp = app.clone().oneshot(
        authed_req()
            .method("POST")
            .uri("/api/v1/workspaces")
            .header("content-type", "application/json")
            .body(Body::from(serde_json::to_vec(&ws_phys_payload).unwrap()))
            .unwrap(),
    ).await.unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);
    let ws_phys: Value = serde_json::from_slice(&resp.into_body().collect().await.unwrap().to_bytes()).unwrap();
    let ws_phys_id = ws_phys["id"].as_str().unwrap();

    // Add faculty to Physics Quantum Lab
    let faculty_collab_payload = json!({
        "eppn": "prof.optics@state.edu",
        "role": "editor",
    });
    let resp = app.clone().oneshot(
        authed_req()
            .method("POST")
            .uri(format!("/api/v1/workspaces/{ws_phys_id}/collaborators"))
            .header("content-type", "application/json")
            .body(Body::from(serde_json::to_vec(&faculty_collab_payload).unwrap()))
            .unwrap(),
    ).await.unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);

    // Faculty requests list_workspaces
    let faculty_token = scaffoldry_server::service::identity::issue_test_token_and_user("prof.optics@state.edu");
    let faculty_req = Request::builder().header("authorization", format!("Bearer {faculty_token}"));

    let resp = app.clone().oneshot(
        faculty_req
            .method("GET")
            .uri("/api/v1/workspaces")
            .body(Body::empty())
            .unwrap(),
    ).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let ws_list: Value = serde_json::from_slice(&resp.into_body().collect().await.unwrap().to_bytes()).unwrap();
    let list_arr = ws_list.as_array().unwrap();
    assert!(list_arr.iter().any(|w| w["id"] == ws_phys_id));
    // Sibling or restricted workspaces where faculty is not collaborator are not present
    assert!(!list_arr.iter().any(|w| w["id"] == "ws-bio-lab"));
}

#[tokio::test]
async fn test_phase5_scim_academic_and_position_sync() {
    let state = ServerState::new().expect("Failed to initialize state");
    let state_arc = Arc::new(state);
    let app = build_app_with_state(state_arc.clone()).expect("Failed to build router");

    let scim_req = || Request::builder().header("authorization", "Bearer test-scim-token");
    let root_id = uuid::Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap();

    // 0. Create department PHYS under root
    let phys_payload = json!({
        "name": "Department of Physics",
        "code": "PHYS",
        "org_type": "Department",
        "parent_id": root_id.to_string(),
    });
    let resp = app.clone().oneshot(
        authed_req()
            .method("POST")
            .uri("/api/v1/orgs")
            .header("content-type", "application/json")
            .body(Body::from(serde_json::to_vec(&phys_payload).unwrap()))
            .unwrap(),
    ).await.unwrap();
    let dept_phys_id = if resp.status() == StatusCode::CREATED {
        let phys_body: Value = serde_json::from_slice(&resp.into_body().collect().await.unwrap().to_bytes()).unwrap();
        phys_body["id"].as_str().unwrap().to_string()
    } else {
        let orgs = state_arc.organizations.read().unwrap();
        orgs.values().find(|o| o.code == "PHYS").unwrap().id.to_string()
    };

    // 1 & 2: Enterprise department equal to existing unit code, roles faculty@university.edu, title "Department Chair"
    let user_payload = json!({
        "userName": "prof.planck@university.edu",
        "title": "Department Chair",
        "name": { "formatted": "Prof. Max Planck" },
        "active": true,
        "roles": [
            { "value": "faculty@university.edu", "type": "eduPersonScopedAffiliation" }
        ],
        "urn:ietf:params:scim:schemas:extension:enterprise:2.0:User": {
            "department": "PHYS"
        }
    });

    let resp = app.clone().oneshot(
        scim_req()
            .method("POST")
            .uri("/scim/v2/Users")
            .header("content-type", "application/json")
            .body(Body::from(serde_json::to_vec(&user_payload).unwrap()))
            .unwrap(),
    ).await.unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);
    let user_res: Value = serde_json::from_slice(&resp.into_body().collect().await.unwrap().to_bytes()).unwrap();
    let user_id = user_res["id"].as_str().unwrap().to_string();

    // Check members: faculty row exists with is_primary = true and source = scim, but NO unit_admin
    let resp = app.clone().oneshot(
        authed_req()
            .method("GET")
            .uri(format!("/api/v1/orgs/{dept_phys_id}/members"))
            .body(Body::empty())
            .unwrap(),
    ).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let members: Value = serde_json::from_slice(&resp.into_body().collect().await.unwrap().to_bytes()).unwrap();
    let member_arr = members.as_array().unwrap();

    let faculty_row = member_arr.iter().find(|m| m["eppn"] == "prof.planck@university.edu" && m["scoped_affiliation"] == "faculty");
    assert!(faculty_row.is_some(), "Faculty row must exist");
    let f = faculty_row.unwrap();
    assert_eq!(f["is_primary"], true);
    assert_eq!(f["source"], "scim");

    let admin_row = member_arr.iter().find(|m| m["eppn"] == "prof.planck@university.edu" && m["scoped_affiliation"] == "unit_admin");
    assert!(admin_row.is_none(), "Title Department Chair must NOT grant unit_admin");

    // 3: Same user plus { "value": "unit_admin", "type": "scaffoldry" } does insert unit_admin on that department
    let mut update_payload = user_payload.clone();
    update_payload["roles"] = json!([
        { "value": "faculty@university.edu", "type": "eduPersonScopedAffiliation" },
        { "value": "unit_admin", "type": "scaffoldry" }
    ]);

    let resp = app.clone().oneshot(
        scim_req()
            .method("PUT")
            .uri(format!("/scim/v2/Users/{user_id}"))
            .header("content-type", "application/json")
            .body(Body::from(serde_json::to_vec(&update_payload).unwrap()))
            .unwrap(),
    ).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let resp = app.clone().oneshot(
        authed_req()
            .method("GET")
            .uri(format!("/api/v1/orgs/{dept_phys_id}/members"))
            .body(Body::empty())
            .unwrap(),
    ).await.unwrap();
    let members: Value = serde_json::from_slice(&resp.into_body().collect().await.unwrap().to_bytes()).unwrap();
    let member_arr = members.as_array().unwrap();
    let admin_row = member_arr.iter().find(|m| m["eppn"] == "prof.planck@university.edu" && m["scoped_affiliation"] == "unit_admin");
    assert!(admin_row.is_some(), "unit_admin must be inserted when scaffoldry role is provided");
    assert_eq!(admin_row.unwrap()["source"], "scim");

    // Also insert an api appointment
    let appoint_payload = json!({
        "eppn": "api.chair@state.edu",
        "scoped_affiliation": "unit_admin",
    });
    let resp = app.clone().oneshot(
        authed_req()
            .method("POST")
            .uri(format!("/api/v1/orgs/{dept_phys_id}/appointments"))
            .header("content-type", "application/json")
            .body(Body::from(serde_json::to_vec(&appoint_payload).unwrap()))
            .unwrap(),
    ).await.unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);

    // 4: PUT removing the scaffoldry role drops the scim admin row and keeps api appointment
    let mut remove_admin_payload = user_payload.clone();
    remove_admin_payload["roles"] = json!([
        { "value": "faculty@university.edu", "type": "eduPersonScopedAffiliation" }
    ]);
    let resp = app.clone().oneshot(
        scim_req()
            .method("PUT")
            .uri(format!("/scim/v2/Users/{user_id}"))
            .header("content-type", "application/json")
            .body(Body::from(serde_json::to_vec(&remove_admin_payload).unwrap()))
            .unwrap(),
    ).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let resp = app.clone().oneshot(
        authed_req()
            .method("GET")
            .uri(format!("/api/v1/orgs/{dept_phys_id}/members"))
            .body(Body::empty())
            .unwrap(),
    ).await.unwrap();
    let members: Value = serde_json::from_slice(&resp.into_body().collect().await.unwrap().to_bytes()).unwrap();
    let member_arr = members.as_array().unwrap();
    let planck_admin = member_arr.iter().find(|m| m["eppn"] == "prof.planck@university.edu" && m["scoped_affiliation"] == "unit_admin");
    assert!(planck_admin.is_none(), "SCIM unit_admin must be removed");
    let api_admin = member_arr.iter().find(|m| m["eppn"] == "api.chair@state.edu" && m["scoped_affiliation"] == "unit_admin");
    assert!(api_admin.is_some(), "API appointment must be kept");
    assert_eq!(api_admin.unwrap()["source"], "api");

    // 5: GET members outside scope returns 403
    let outsider_token = scaffoldry_server::service::identity::issue_test_token_and_user("outsider.student@state.edu");
    let resp = app.clone().oneshot(
        Request::builder()
            .header("authorization", format!("Bearer {outsider_token}"))
            .method("GET")
            .uri(format!("/api/v1/orgs/{dept_phys_id}/members"))
            .body(Body::empty())
            .unwrap(),
    ).await.unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
}
