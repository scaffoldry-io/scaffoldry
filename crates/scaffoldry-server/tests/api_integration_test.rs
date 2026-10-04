use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use scaffoldry_server::build_app;
use serde_json::{json, Value};
use tower::ServiceExt;

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
    let app = build_app().expect("Failed to build router");

    // 1. ServiceProviderConfig
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
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
            Request::builder()
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
            Request::builder()
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
            Request::builder()
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
            Request::builder()
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
            Request::builder()
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
            Request::builder()
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
            Request::builder()
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
            Request::builder()
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
    let record_payload = json!({
        "caller_eppn": "dr.smith@physics.university.edu",
        "caller_affiliation": "faculty",
        "data": {
            "proposal_title": "Quantum Lattice Simulation",
            "student_pi": "Alice Walker",
            "amount": 150000
        }
    });
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
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
            Request::builder()
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
            Request::builder()
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
            Request::builder()
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
            Request::builder()
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
            Request::builder()
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
            Request::builder()
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
            Request::builder()
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
            Request::builder()
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
            Request::builder()
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
            Request::builder()
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
            Request::builder()
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
            Request::builder()
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
            Request::builder()
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
            Request::builder()
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
            Request::builder()
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
            Request::builder()
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
            Request::builder()
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
            Request::builder()
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
            Request::builder()
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
            Request::builder()
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
    assert_eq!(ledger_res["total_entries"].as_u64().unwrap(), 4);
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
            Request::builder()
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
    assert_eq!(append_res["entry"]["sequence"].as_u64().unwrap(), 4);
    assert_eq!(append_res["entry"]["oscal_control_id"], "AC-03");

    // 3. POST /api/v1/governance/ledger/verify - cryptographic verification of entire chain
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
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
    assert_eq!(verify_res["total_entries"].as_u64().unwrap(), 5);

    // 4. GET /api/v1/governance/oscal/export - export official NIST OSCAL 1.1.2 JSON
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
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
    assert_eq!(impl_reqs.len(), 5);

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
            Request::builder()
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



