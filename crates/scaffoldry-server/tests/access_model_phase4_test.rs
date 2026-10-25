//! Access model phase 4: one grant shape and one function, `role_at`.

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use common::TestDb;
use http_body_util::BodyExt;
use scaffoldry_core::ledger::DecisionType;
use scaffoldry_server::build_app_with_state;
use scaffoldry_server::service::identity::issue_test_token_and_user;
use scaffoldry_server::state::{CollaboratorRecord, OrganizationNode, RoleRow, ServerState, WorkspaceRecord};
use serde_json::{json, Value};
use tower::ServiceExt;
use uuid::Uuid;

const ADMIN: &str = "jordan.lee@state.edu";
const ADMIN_TWO: &str = "second.admin@state.edu";
const UNIT_ADMIN: &str = "einstein@state.edu";
const OTHER_UNIT_ADMIN: &str = "marie.curie@state.edu";
const CHAIR: &str = "sarah.connor@state.edu";
const NEW_CHAIR: &str = "marcus.vance@state.edu";
const READER: &str = "student.p4@state.edu";
const ROOT: &str = "00000000-0000-0000-0000-000000000001";

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

fn unit(state: &ServerState, name: &str, parent: Option<Uuid>) -> Uuid {
    let root = Uuid::parse_str(ROOT).unwrap();
    let id = Uuid::new_v4();
    let node = OrganizationNode { id, parent_id: Some(parent.unwrap_or(root)), name: name.to_string(), code: name.to_uppercase(), org_type: "Unit".to_string(), hierarchy_key: "academic".to_string(), synthetic: false };
    state.organizations.write().unwrap().insert(id, node.clone());
    state.persist_organization(&node).unwrap();
    id
}

fn appoint(state: &ServerState, eppn: &str, unit: Uuid, scoped_affiliation: &str, position: Option<&str>) {
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
            position_key: position.map(str::to_string),
        })
        .unwrap();
}

/// A workspace placed in `unit` with no members.
fn workspace(state: &ServerState, id: &str, unit: Uuid) {
    let now = chrono::Utc::now().to_rfc3339();
    let ws = WorkspaceRecord {
        id: id.to_string(),
        name: format!("Workspace {id}"),
        code: format!("WS-{id}"),
        organization: "INST".to_string(),
        department: "biology".to_string(),
        description: "Test".to_string(),
        icon: "dna".to_string(),
        lead: "Lead".to_string(),
        visibility: "departmental".to_string(),
        coordinates: Default::default(),
        data_classification: "restricted".to_string(),
        created_at: now,
        organization_id: Some(unit),
        synthetic: false,
    };
    state.workspaces.write().unwrap().insert(id.to_string(), ws.clone());
    state.repository.upsert_workspace(&ws).unwrap();
    state.collaborators.write().unwrap().insert(id.to_string(), Vec::<CollaboratorRecord>::new());
}

struct World {
    db: TestDb,
    app: axum::Router,
    science: Uuid,
    physics: Uuid,
    chemistry: Uuid,
}

fn world() -> World {
    let db = TestDb::new();
    let state = db.state();
    let science = unit(&state, "science", None);
    let physics = unit(&state, "physics", Some(science));
    let chemistry = unit(&state, "chemistry", Some(science));
    workspace(&state, "ws-physics", physics);
    workspace(&state, "ws-chemistry", chemistry);
    appoint(&state, ADMIN_TWO, Uuid::parse_str(ROOT).unwrap(), "platform_admin", None);
    appoint(&state, UNIT_ADMIN, science, "unit_admin", None);
    appoint(&state, OTHER_UNIT_ADMIN, physics, "unit_admin", None);
    let app = build_app_with_state(state).expect("router");
    World { db, app, science, physics, chemistry }
}

fn person_grant(who: &str, role: &str, anchor_kind: &str, anchor_id: &str) -> Value {
    json!({ "holder_kind": "person", "holder_id": who, "role": role, "hierarchy": "academic", "anchor_kind": anchor_kind, "anchor_id": anchor_id, "source": "request", "valid_until": (chrono::Utc::now() + chrono::Duration::days(30)).to_rfc3339(), "reason": "Test grant" })
}

/// Proposes a grant as one platform admin and confirms it as the other.
async fn grant(w: &World, body: Value) -> i64 {
    let (s, pending) = send(&w.app, "POST", "/api/v1/admin/access/grants", ADMIN, Some(body)).await;
    assert_eq!(s, StatusCode::ACCEPTED, "{pending}");
    let change = pending["change_id"].as_str().unwrap();
    let (s, done) = send(&w.app, "POST", &format!("/api/v1/admin/access/changes/{change}/decide"), ADMIN_TWO, Some(json!({ "decision": "confirm", "reason": "Reviewed" }))).await;
    assert_eq!(s, StatusCode::OK, "{done}");
    done["grant_id"].as_i64().expect("the confirmed change names the grant")
}

async fn can_read(w: &World, who: &str, ws: &str) -> bool {
    let (s, _) = send(&w.app, "GET", &format!("/api/v1/workspaces/{ws}"), who, None).await;
    s == StatusCode::OK
}

/// The role the workspace page reports for the caller: `None` for a non-member, even when the page opens by visibility.
async fn role_in(w: &World, who: &str, ws: &str) -> Option<String> {
    let (s, body) = send(&w.app, "GET", &format!("/api/v1/workspaces/{ws}"), who, None).await;
    assert_eq!(s, StatusCode::OK, "{body}");
    body["user_role"].as_str().map(str::to_string)
}

// 7. A grant is not in force until a different person confirms it. The proposer's confirm is 403.
#[tokio::test]
async fn a_grant_is_not_in_force_until_a_different_person_confirms() {
    let w = world();
    let (s, pending) = send(&w.app, "POST", "/api/v1/admin/access/grants", ADMIN, Some(person_grant(READER, "viewer", "unit", &w.physics.to_string()))).await;
    assert_eq!(s, StatusCode::ACCEPTED, "every new grant widens, so it waits: {pending}");
    let change = pending["change_id"].as_str().unwrap().to_string();
    assert!(!can_read(&w, READER, "ws-physics").await, "a pending grant gives nothing");

    let (_, listed) = send(&w.app, "GET", "/api/v1/admin/access/grants?state=pending", ADMIN, None).await;
    assert!(listed["changes"].to_string().contains(&change), "the pending change is listed: {listed}");
    let (_, active) = send(&w.app, "GET", "/api/v1/admin/access/grants?state=active", ADMIN, None).await;
    assert_eq!(active["grants"].as_array().unwrap().len(), 0, "and no grant is active: {active}");

    let decide = format!("/api/v1/admin/access/changes/{change}/decide");
    let (s, body) = send(&w.app, "POST", &decide, ADMIN, Some(json!({ "decision": "confirm", "reason": "Mine" }))).await;
    assert_eq!(s, StatusCode::FORBIDDEN, "the proposer's own confirm is refused: {body}");
    assert!(!can_read(&w, READER, "ws-physics").await);

    let (s, body) = send(&w.app, "POST", &decide, ADMIN_TWO, Some(json!({ "decision": "confirm", "reason": "Reviewed" }))).await;
    assert_eq!(s, StatusCode::OK, "{body}");
    assert!(can_read(&w, READER, "ws-physics").await, "now it is in force");
    assert!(!can_read(&w, READER, "ws-chemistry").await, "and reaches only its anchor");
    let (s, _) = send(&w.app, "POST", &decide, ADMIN_TWO, Some(json!({ "decision": "confirm", "reason": "Again" }))).await;
    assert_eq!(s, StatusCode::CONFLICT, "a change is decided once");

    let (_, ledger) = send(&w.app, "GET", "/api/v1/governance/ledger", ADMIN, None).await;
    assert_eq!(ledger["entries"][0]["decision_type"], "AccessGrantChanged", "the confirmation is in the ledger: {ledger}");
    assert!(DecisionType::parse("CoordinatesChanged").is_some() && DecisionType::parse("HierarchyChanged").is_some());
}

// A rejected change grants nothing.
#[tokio::test]
async fn a_rejected_grant_grants_nothing() {
    let w = world();
    let (_, pending) = send(&w.app, "POST", "/api/v1/admin/access/grants", ADMIN, Some(person_grant(READER, "viewer", "unit", &w.physics.to_string()))).await;
    let change = pending["change_id"].as_str().unwrap();
    let (s, body) = send(&w.app, "POST", &format!("/api/v1/admin/access/changes/{change}/decide"), ADMIN_TWO, Some(json!({ "decision": "reject", "reason": "No" }))).await;
    assert_eq!(s, StatusCode::OK, "{body}");
    assert!(!can_read(&w, READER, "ws-physics").await);
}

// 6. An Org Unit Admin cannot anchor a grant above their unit or grant owner.
#[tokio::test]
async fn an_org_unit_admin_cannot_anchor_above_their_unit_or_grant_owner() {
    let w = world();
    let post = |body: Value| {
        let app = w.app.clone();
        async move { send(&app, "POST", "/api/v1/admin/access/grants", UNIT_ADMIN, Some(body)).await }
    };
    let (s, body) = post(person_grant(READER, "viewer", "unit", ROOT)).await;
    assert_eq!((s, body["code"].as_str()), (StatusCode::FORBIDDEN, Some("grant_outside_scope")), "above their unit: {body}");
    let (s, body) = post(person_grant(READER, "owner", "unit", &w.physics.to_string())).await;
    assert_eq!((s, body["code"].as_str()), (StatusCode::FORBIDDEN, Some("grant_outside_scope")), "owner: {body}");
    let (s, body) = post(person_grant(READER, "viewer", "unit", &w.chemistry.to_string())).await;
    assert_eq!(s, StatusCode::ACCEPTED, "inside their own subtree is allowed, and waits: {body}");
    // A unit admin of Physics has no reach over Chemistry.
    let (s, body) = send(&w.app, "POST", "/api/v1/admin/access/grants", OTHER_UNIT_ADMIN, Some(person_grant(READER, "viewer", "unit", &w.chemistry.to_string()))).await;
    assert_eq!((s, body["code"].as_str()), (StatusCode::FORBIDDEN, Some("grant_outside_scope")), "a sibling's subtree: {body}");
    // Someone with no right at all.
    let (s, _) = send(&w.app, "POST", "/api/v1/admin/access/grants", READER, Some(person_grant(READER, "viewer", "unit", &w.physics.to_string()))).await;
    assert_eq!(s, StatusCode::FORBIDDEN);
}

// A grant to everyone below a root is access for everyone, which a workspace's visibility already says.
#[tokio::test]
async fn a_grant_to_a_whole_hierarchy_is_too_broad_unless_it_is_one_container_viewer() {
    let w = world();
    let to_root = |role: &str, anchor_kind: &str, anchor_id: &str| {
        let hierarchy = if anchor_kind == "container" { Value::Null } else { json!("academic") };
        json!({ "holder_kind": "unit", "holder_id": ROOT, "role": role, "hierarchy": hierarchy, "anchor_kind": anchor_kind, "anchor_id": anchor_id, "source": "request", "valid_until": (chrono::Utc::now() + chrono::Duration::days(30)).to_rfc3339(), "reason": "All" })
    };
    let (s, body) = send(&w.app, "POST", "/api/v1/admin/access/grants", ADMIN, Some(to_root("viewer", "unit", &w.physics.to_string()))).await;
    assert_eq!((s, body["code"].as_str()), (StatusCode::BAD_REQUEST, Some("grant_too_broad")), "{body}");
    let (s, body) = send(&w.app, "POST", "/api/v1/admin/access/grants", ADMIN, Some(to_root("editor", "container", "workspace:ws-physics"))).await;
    assert_eq!((s, body["code"].as_str()), (StatusCode::BAD_REQUEST, Some("grant_too_broad")), "an editor role: {body}");
    let (s, body) = send(&w.app, "POST", "/api/v1/admin/access/grants", ADMIN, Some(to_root("viewer", "container", "workspace:ws-physics"))).await;
    assert_eq!(s, StatusCode::ACCEPTED, "one container and viewer is allowed: {body}");
}

// Bad input is refused by name.
#[tokio::test]
async fn a_malformed_grant_is_refused() {
    let w = world();
    let mut cases = vec![];
    let mut bad = person_grant(READER, "viewer", "unit", &w.physics.to_string());
    bad["role"] = json!("superuser");
    cases.push(bad);
    let mut bad = person_grant(READER, "viewer", "unit", &w.physics.to_string());
    bad["anchor_kind"] = json!("everywhere");
    cases.push(bad);
    let mut bad = person_grant(READER, "viewer", "unit", &Uuid::new_v4().to_string());
    bad["reason"] = json!("Unknown unit");
    cases.push(bad);
    let mut bad = person_grant(READER, "viewer", "unit", &w.physics.to_string());
    bad["domain"] = json!("no-such-domain");
    cases.push(bad);
    let mut bad = person_grant(READER, "viewer", "unit", &w.physics.to_string());
    bad["reason"] = json!("");
    cases.push(bad);
    let mut bad = person_grant(READER, "viewer", "unit", &w.physics.to_string());
    bad["hierarchy"] = json!("nowhere");
    cases.push(bad);
    for body in cases {
        let (s, resp) = send(&w.app, "POST", "/api/v1/admin/access/grants", ADMIN, Some(body.clone())).await;
        assert_eq!(s, StatusCode::BAD_REQUEST, "{body} gave {resp}");
    }
}

// Revoke narrows, so it applies at once. Grants are never deleted.
#[tokio::test]
async fn revoking_a_grant_applies_at_once_and_keeps_the_row() {
    let w = world();
    let id = grant(&w, person_grant(READER, "viewer", "unit", &w.physics.to_string())).await;
    assert!(can_read(&w, READER, "ws-physics").await);
    let (s, body) = send(&w.app, "POST", &format!("/api/v1/admin/access/grants/{id}/revoke"), ADMIN, Some(json!({ "reason": "Done" }))).await;
    assert_eq!(s, StatusCode::OK, "{body}");
    assert!(!can_read(&w, READER, "ws-physics").await, "revoked at once");
    let (_, revoked) = send(&w.app, "GET", "/api/v1/admin/access/grants?state=revoked", ADMIN, None).await;
    assert_eq!(revoked["grants"][0]["revoked_by"], ADMIN, "{revoked}");
    let count: i64 = w.db.run(|c| c.query_one("SELECT count(*) FROM access_grants", &[]).unwrap().get(0));
    assert_eq!(count, 1, "the row is still there");
    let (s, _) = send(&w.app, "POST", &format!("/api/v1/admin/access/grants/{id}/revoke"), ADMIN, Some(json!({ "reason": "Again" }))).await;
    assert_eq!(s, StatusCode::CONFLICT, "a revoked grant is revoked once");
    // A reason is required.
    let id = grant(&w, person_grant(READER, "viewer", "unit", &w.physics.to_string())).await;
    let (s, _) = send(&w.app, "POST", &format!("/api/v1/admin/access/grants/{id}/revoke"), ADMIN, Some(json!({}))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
}

// 2. "Every chair@* gets viewer on their own unit's subtree" gives the Physics chair viewer on a Physics workspace and
//    nothing on Chemistry. Appointing a new chair moves the access with no other change.
#[tokio::test]
async fn one_chair_grant_serves_every_department_and_follows_the_appointment() {
    let w = world();
    let (s, body) = send(&w.app, "POST", "/api/v1/admin/positions", ADMIN, Some(json!({ "key": "chair", "name": "Chair", "description": "A department chair", "org_types": ["Department"], "max_holders": 1, "reason": "Define the position" }))).await;
    assert_eq!(s, StatusCode::CREATED, "{body}");
    appoint(&w.db.state(), CHAIR, w.physics, "faculty", Some("chair"));
    assert_eq!(role_in(&w, CHAIR, "ws-physics").await, None, "before the grant, holding the post gives no role");
    grant(&w, json!({ "holder_kind": "position", "holder_id": "chair@*", "role": "viewer", "hierarchy": "academic", "anchor_kind": "holder_unit", "source": "rule", "reason": "Chairs see their department" })).await;

    assert_eq!(role_in(&w, CHAIR, "ws-physics").await.as_deref(), Some("viewer"), "the Physics chair is a viewer of the Physics workspace");
    assert_eq!(role_in(&w, CHAIR, "ws-chemistry").await, None, "and has no role on Chemistry");

    appoint(&w.db.state(), NEW_CHAIR, w.chemistry, "faculty", Some("chair"));
    assert_eq!(role_in(&w, NEW_CHAIR, "ws-chemistry").await.as_deref(), Some("viewer"), "a new chair in Chemistry gets Chemistry with no other change");
    assert_eq!(role_in(&w, NEW_CHAIR, "ws-physics").await, None);
}

// 9. A workspace guard forbids read_app to a person who holds a matching unit grant.
#[tokio::test]
async fn a_guard_forbids_read_app_to_a_person_who_holds_a_unit_grant() {
    let w = world();
    let state = w.db.state();
    let mut ssn = scaffoldry_engine::FieldSpec::simple("name", "Name", scaffoldry_engine::FieldType::Text, false, false);
    ssn.categories = vec![];
    let manifest: scaffoldry_engine::AppManifest = serde_json::from_value(json!({
        "slug": "p4-app", "title": "App", "description": "", "organization_code": "PHYS", "department": "physics", "workspace_id": "ws-physics",
        "tables": [{ "id": "t", "name": "T", "slug": "t", "fields": [{ "name": "name", "label": "Name", "field_type": "Text", "required": false, "ferpa_sensitive": false }] }],
        "views": [], "pages": []
    }))
    .unwrap();
    state.engine.write().unwrap().register_manifest(manifest).unwrap();
    let app = build_app_with_state(state).expect("router");
    let w = World { app, ..w };
    grant(&w, person_grant(READER, "editor", "unit", &w.science.to_string())).await;
    let (s, body) = send(&w.app, "GET", "/api/v1/apps/p4-app/schema", READER, None).await;
    assert_eq!(s, StatusCode::OK, "a unit grant lets them read the app: {body}");

    let other = Uuid::new_v4();
    let (s, body) = send(&w.app, "PUT", "/api/v1/workspaces/ws-physics/guards", ADMIN, Some(json!({ "rules": [{ "template": "deny_access_outside_units", "unit_ids": [other.to_string()] }], "reason": "Elsewhere only" }))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "a guard names a unit that exists: {body}");
    let arts = unit(&w.db.state(), "arts", None);
    let (s, body) = send(&w.app, "PUT", "/api/v1/workspaces/ws-physics/guards", ADMIN, Some(json!({ "rules": [{ "template": "deny_access_outside_units", "unit_ids": [arts.to_string()] }], "reason": "Arts only" }))).await;
    assert_eq!(s, StatusCode::OK, "{body}");
    let (s, body) = send(&w.app, "GET", "/api/v1/apps/p4-app/schema", READER, None).await;
    assert_eq!(s, StatusCode::FORBIDDEN, "the guard forbids them and a forbid wins: {body}");
}

// The published schema describes the grant the API returns.
#[tokio::test]
async fn the_listed_grant_has_the_fields_the_published_schema_requires() {
    let w = world();
    grant(&w, person_grant(READER, "viewer", "unit", &w.physics.to_string())).await;
    let schema: Value = serde_json::from_str(&std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../governance/schema/access-grant.schema.json")).unwrap()).unwrap();
    let (_, listed) = send(&w.app, "GET", "/api/v1/admin/access/grants?state=active", ADMIN, None).await;
    let row = &listed["grants"][0];
    for key in schema["required"].as_array().unwrap() {
        assert!(row.get(key.as_str().unwrap()).is_some(), "the grant is missing {key}: {row}");
    }
    let kinds: Vec<&str> = schema["properties"]["anchor_kind"]["enum"].as_array().unwrap().iter().filter_map(Value::as_str).collect();
    assert_eq!(kinds, ["container", "unit", "holder_unit"]);
}
