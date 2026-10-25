//! Access model phase 10, item 1: supervision as a hierarchy. The SCIM manager attribute keeps a `people` hierarchy
//! with source `scim` as a tree of supervisory units.

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use common::TestDb;
use http_body_util::BodyExt;
use scaffoldry_server::build_app_with_state;
use scaffoldry_server::service::identity::issue_test_token_and_user;
use scaffoldry_server::state::{CollaboratorRecord, OrganizationNode, RoleRow, ServerState, WorkspaceRecord};
use serde_json::{json, Value};
use std::collections::BTreeSet;
use tower::ServiceExt;
use uuid::Uuid;

const ADMIN: &str = "jordan.lee@state.edu";
const ADMIN_TWO: &str = "second.admin@state.edu";
const ROOT: &str = "00000000-0000-0000-0000-000000000001";
const WS: &str = "ws-science";
const SCIM_TOKEN: &str = "test-scim-token";
const ENT: &str = "urn:ietf:params:scim:schemas:extension:enterprise:2.0:User";

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

fn unit(state: &ServerState, name: &str, parent: Uuid) -> Uuid {
    let id = Uuid::new_v4();
    let node = OrganizationNode {
        id,
        parent_id: Some(parent),
        name: name.to_string(),
        code: name.to_uppercase(),
        org_type: "Unit".to_string(),
        hierarchy_key: "academic".to_string(),
        synthetic: false,
    };
    state.organizations.write().unwrap().insert(id, node.clone());
    state.persist_organization(&node).unwrap();
    id
}

fn appoint(state: &ServerState, eppn: &str, unit: Uuid, scoped_affiliation: &str) {
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
            position_key: None,
        })
        .unwrap();
}

fn workspace(state: &ServerState, id: &str, unit: Uuid) {
    let ws = WorkspaceRecord {
        id: id.to_string(),
        name: format!("Workspace {id}"),
        code: format!("WS-{id}"),
        organization: "INST".to_string(),
        department: "science".to_string(),
        description: "Test".to_string(),
        icon: "dna".to_string(),
        lead: "Lead".to_string(),
        visibility: "departmental".to_string(),
        coordinates: Default::default(),
        data_classification: "restricted".to_string(),
        created_at: chrono::Utc::now().to_rfc3339(),
        organization_id: Some(unit),
        synthetic: false,
    };
    state.workspaces.write().unwrap().insert(id.to_string(), ws.clone());
    state.repository.upsert_workspace(&ws).unwrap();
    state.collaborators.write().unwrap().insert(id.to_string(), Vec::<CollaboratorRecord>::new());
}


async fn scim(app: &axum::Router, method: &str, uri: &str, body: Value) -> (StatusCode, Value) {
    let req = Request::builder().method(method).uri(uri).header("authorization", format!("Bearer {SCIM_TOKEN}")).header("content-type", "application/json").body(Body::from(body.to_string())).unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
}

fn user_body(name: &str, manager: Option<&str>) -> Value {
    let mut ent = json!({ "department": "science" });
    if let Some(m) = manager {
        ent["manager"] = json!({ "value": m });
    }
    json!({ "schemas": ["urn:ietf:params:scim:schemas:core:2.0:User"], "userName": format!("{name}@state.edu"), "name": { "formatted": name }, "active": true, ENT: ent })
}

async fn grant(app: &axum::Router, body: Value) {
    let (s, pending) = send(app, "POST", "/api/v1/admin/access/grants", ADMIN, Some(body)).await;
    assert_eq!(s, StatusCode::ACCEPTED, "{pending}");
    let change = pending["change_id"].as_str().unwrap();
    let (s, done) = send(app, "POST", &format!("/api/v1/admin/access/changes/{change}/decide"), ADMIN_TWO, Some(json!({ "decision": "confirm", "reason": "Reviewed" }))).await;
    assert_eq!(s, StatusCode::OK, "{done}");
}

async fn titles(app: &axum::Router, slug: &str, who: &str) -> BTreeSet<String> {
    let payload = json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": { "name": "list_records", "arguments": { "app_slug": slug, "table_id": "tickets", "limit": 200 } } });
    let (_, v) = send(app, "POST", "/api/mcp", who, Some(payload)).await;
    assert!(v["result"]["isError"] != true, "{who}: {v}");
    v["result"]["structuredContent"]["records"].as_array().unwrap_or(&vec![]).iter().map(|r| r["data"]["title"].as_str().unwrap().to_string()).collect()
}

fn set(names: &[&str]) -> BTreeSet<String> {
    names.iter().map(|s| s.to_string()).collect()
}

fn supervisor_of(db: &TestDb, eppn: &str) -> Vec<String> {
    let eppn = eppn.to_string();
    db.run(move |c| {
        c.query("SELECT o.name FROM roles r JOIN persons p ON p.id = r.person_id JOIN organizations o ON o.id = r.organization_id WHERE p.eppn = $1 AND r.position_key = 'supervisor' ORDER BY 1", &[&eppn]).unwrap().iter().map(|r| r.get(0)).collect()
    })
}

struct World {
    db: TestDb,
    app: axum::Router,
    slug: String,
}

/// A `supervisory` hierarchy that SCIM keeps, with its root, and a table whose rows sit under the person who asked.
async fn world() -> World {
    let db = TestDb::new();
    let state = db.state();
    db.run(|c| c.execute("INSERT INTO hierarchies (key, name, kind, source, created_by) VALUES ('supervisory', 'Supervisory', 'people', 'scim', 'test') ON CONFLICT (key) DO UPDATE SET kind = 'people', source = 'scim'", &[]).unwrap());
    let root = Uuid::parse_str(ROOT).unwrap();
    let science = unit(&state, "science", root);
    let physics = unit(&state, "physics", science);
    let sup_root = Uuid::new_v4();
    let node = OrganizationNode { id: sup_root, parent_id: None, name: "Supervision".to_string(), code: "SUPERVISION".to_string(), org_type: "Unit".to_string(), hierarchy_key: "supervisory".to_string(), synthetic: false };
    state.organizations.write().unwrap().insert(sup_root, node.clone());
    state.persist_organization(&node).unwrap();
    workspace(&state, WS, science);
    appoint(&state, ADMIN_TWO, root, "platform_admin");
    let app = build_app_with_state(state.clone()).expect("router");
    let slug = format!("tickets-{}", &Uuid::new_v4().to_string()[..8]);
    let manifest = json!({
        "slug": slug, "title": "Tickets", "description": "d", "organization_code": "INST", "department": "science", "views": [],
        "tables": [{
            "id": "tickets", "name": "Tickets", "slug": "tickets",
            "coordinates": { "units": { "academic": physics.to_string() } },
            "fields": [
                { "name": "title", "label": "Title", "field_type": "Text", "required": false, "ferpa_sensitive": false },
                { "name": "requester", "label": "Requester", "field_type": "Collaborator", "required": false, "ferpa_sensitive": false }
            ],
            "row_place": { "supervisory": "requester" }
        }]
    });
    let (s, body) = send(&app, "POST", &format!("/api/v1/workspaces/{WS}/apps"), ADMIN, Some(manifest)).await;
    assert_eq!(s, StatusCode::CREATED, "{body}");
    World { db, app, slug }
}

async fn open_app_and_see_supervised_rows(w: &World, who: &str, science_unit: Uuid) {
    // A grant at science reaches the workspace and not the table, so only the supervisory grant decides the rows.
    let g = json!({ "holder_kind": "person", "holder_id": who, "role": "viewer", "hierarchy": "academic", "anchor_kind": "unit", "anchor_id": science_unit.to_string(), "depth": 0, "source": "appointment", "reason": "Test grant" });
    grant(&w.app, g).await;
}

// 1. A SCIM user whose manager changes moves to the new manager's unit, and the old manager's `holder_unit` grant stops
//    reaching their records.
#[tokio::test]
async fn a_user_whose_manager_changes_moves_to_the_new_managers_unit() {
    let w = world().await;
    let science = Uuid::parse_str(&w.db.run(|c| c.query_one("SELECT id::text FROM organizations WHERE name = 'science'", &[]).unwrap().get::<_, String>(0))).unwrap();
    let (_, a) = scim(&w.app, "POST", "/scim/v2/Users", user_body("mgr-a", None)).await;
    let (_, b) = scim(&w.app, "POST", "/scim/v2/Users", user_body("mgr-b", None)).await;
    let (a_id, b_id) = (a["id"].as_str().unwrap().to_string(), b["id"].as_str().unwrap().to_string());
    let (s, emp) = scim(&w.app, "POST", "/scim/v2/Users", user_body("emp", Some(&a_id))).await;
    assert_eq!(s, StatusCode::CREATED, "{emp}");
    let emp_id = emp["id"].as_str().unwrap().to_string();

    let (s, body) = send(&w.app, "POST", &format!("/api/v1/apps/{}/records", w.slug), ADMIN, Some(json!({ "data": { "_table_id": "tickets", "title": "emp ticket", "requester": "emp@state.edu" } }))).await;
    assert_eq!(s, StatusCode::CREATED, "{body}");
    let slug = w.slug.clone();
    w.db.run(move |c| c.execute("UPDATE index_status SET state = 'ready' WHERE app_slug = $1", &[&slug]).unwrap());

    // Supervisors see the rows in their own unit's subtree.
    let g = json!({ "holder_kind": "position", "holder_id": "supervisor@*", "role": "viewer", "hierarchy": "supervisory", "anchor_kind": "holder_unit", "source": "rule", "reason": "Supervisors see their reports' rows" });
    grant(&w.app, g).await;
    for who in ["mgr-a@state.edu", "mgr-b@state.edu"] {
        open_app_and_see_supervised_rows(&w, who, science).await;
    }

    assert_eq!(supervisor_of(&w.db, "mgr-a@state.edu"), vec!["Reports of mgr-a"], "the manager holds supervisor at their unit");
    assert_eq!(titles(&w.app, &w.slug, "mgr-a@state.edu").await, set(&["emp ticket"]), "A supervises emp");
    assert!(titles(&w.app, &w.slug, "mgr-b@state.edu").await.is_empty(), "B does not");

    // The manager changes.
    let (s, body) = scim(&w.app, "PUT", &format!("/scim/v2/Users/{emp_id}"), user_body("emp", Some(&b_id))).await;
    assert_eq!(s, StatusCode::OK, "{body}");
    assert!(titles(&w.app, &w.slug, "mgr-a@state.edu").await.is_empty(), "A no longer reaches emp's records");
    assert_eq!(titles(&w.app, &w.slug, "mgr-b@state.edu").await, set(&["emp ticket"]), "B now does");

    // A person with no manager hangs from the hierarchy's root.
    let (s, body) = scim(&w.app, "PUT", &format!("/scim/v2/Users/{emp_id}"), user_body("emp", None)).await;
    assert_eq!(s, StatusCode::OK, "{body}");
    assert!(titles(&w.app, &w.slug, "mgr-b@state.edu").await.is_empty());
}

// A manager's manager makes a chain of units.
#[tokio::test]
async fn a_chain_of_managers_is_a_tree_of_units() {
    let w = world().await;
    let (_, boss) = scim(&w.app, "POST", "/scim/v2/Users", user_body("boss", None)).await;
    let (_, lead) = scim(&w.app, "POST", "/scim/v2/Users", user_body("lead", Some(boss["id"].as_str().unwrap()))).await;
    let (s, _) = scim(&w.app, "POST", "/scim/v2/Users", user_body("worker", Some(lead["id"].as_str().unwrap()))).await;
    assert_eq!(s, StatusCode::CREATED);
    let parents: Vec<(String, Option<String>)> = w.db.run(|c| {
        c.query("SELECT o.name, p.name FROM organizations o LEFT JOIN organizations p ON p.id = o.parent_id WHERE o.hierarchy_key = 'supervisory' ORDER BY 1", &[]).unwrap().iter().map(|r| (r.get(0), r.get(1))).collect()
    });
    assert_eq!(parents, vec![
        ("Reports of boss".to_string(), Some("Supervision".to_string())),
        ("Reports of lead".to_string(), Some("Reports of boss".to_string())),
        ("Supervision".to_string(), None),
    ]);
}

// Without such a hierarchy nothing is built.
#[tokio::test]
async fn nothing_is_built_without_a_scim_people_hierarchy() {
    let db = TestDb::new();
    let state = db.state();
    let app = build_app_with_state(state).expect("router");
    let (_, boss) = scim(&app, "POST", "/scim/v2/Users", user_body("boss2", None)).await;
    scim(&app, "POST", "/scim/v2/Users", user_body("worker2", Some(boss["id"].as_str().unwrap()))).await;
    let n: i64 = db.run(|c| c.query_one("SELECT COUNT(*) FROM organizations WHERE code LIKE 'SUP-%'", &[]).unwrap().get(0));
    assert_eq!(n, 0);
}
