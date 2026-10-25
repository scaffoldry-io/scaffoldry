//! Access model phase 11: what the Access section of the console needs from the server. A hierarchy is created as a
//! change a different person confirms, and retired with a reason. A grant can be previewed before it is proposed. People
//! and objects can be searched. Every list pages in SQL.

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use common::TestDb;
use http_body_util::BodyExt;
use scaffoldry_server::build_app_with_state;
use scaffoldry_server::service::identity::issue_test_token_and_user;
use scaffoldry_server::state::{CollaboratorRecord, OrganizationNode, RoleRow, ServerState, WorkspaceRecord};
use serde_json::{json, Value};
use tower::ServiceExt;
use uuid::Uuid;

const ADMIN: &str = "jordan.lee@state.edu";
const ADMIN_TWO: &str = "second.admin@state.edu";
const READER: &str = "reader.p11@state.edu";
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
    let node = OrganizationNode { id, parent_id: Some(parent.unwrap_or(root)), name: name.to_string(), code: name.to_uppercase(), org_type: "Department".to_string(), hierarchy_key: "academic".to_string(), synthetic: false };
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
}

fn world() -> World {
    let db = TestDb::new();
    let state = db.state();
    let science = unit(&state, "science", None);
    appoint(&state, ADMIN_TWO, Uuid::parse_str(ROOT).unwrap(), "platform_admin", None);
    let app = build_app_with_state(state).expect("router");
    World { db, app, science }
}

fn in_days(n: i64) -> String {
    (chrono::Utc::now() + chrono::Duration::days(n)).to_rfc3339()
}

fn grant_body(w: &World, who: &str) -> Value {
    json!({ "holder_kind": "person", "holder_id": who, "role": "viewer", "hierarchy": "academic", "anchor_kind": "unit", "anchor_id": w.science.to_string(), "source": "request", "valid_until": in_days(30), "reason": "Test grant" })
}

async fn hierarchy_keys(w: &World, who: &str) -> Vec<String> {
    let (s, body) = send(&w.app, "GET", "/api/v1/admin/hierarchies", who, None).await;
    assert_eq!(s, StatusCode::OK, "{body}");
    body.as_array().unwrap().iter().map(|h| h["key"].as_str().unwrap().to_string()).collect()
}

// 1. A hierarchy is a change a different person confirms. Until then it does not exist.
#[tokio::test]
async fn a_hierarchy_is_not_usable_until_another_platform_admin_confirms_it() {
    let w = world();
    let body = json!({ "key": "supervision", "name": "Supervision", "kind": "people", "source": "scim", "reason": "Supervisors see their reports" });
    let (s, pending) = send(&w.app, "POST", "/api/v1/admin/hierarchies", ADMIN, Some(body.clone())).await;
    assert_eq!(s, StatusCode::ACCEPTED, "{pending}");
    let change = pending["change_id"].as_str().unwrap().to_string();
    assert!(!hierarchy_keys(&w, ADMIN).await.contains(&"supervision".to_string()), "a pending hierarchy is not listed as one that exists");

    // It is listed as a pending change.
    let (_, listed) = send(&w.app, "GET", "/api/v1/admin/access/grants?state=pending", ADMIN, None).await;
    assert!(listed["changes"].as_array().unwrap().iter().any(|c| c["change_id"] == change.as_str() && c["kind"] == "hierarchy"), "{listed}");

    // The proposer cannot confirm. A grant cannot be made in it.
    let (s, _) = send(&w.app, "POST", &format!("/api/v1/admin/access/changes/{change}/decide"), ADMIN, Some(json!({ "decision": "confirm", "reason": "Mine" }))).await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    let mut g = grant_body(&w, READER);
    g["hierarchy"] = json!("supervision");
    let (s, _) = send(&w.app, "POST", "/api/v1/admin/access/grants", ADMIN, Some(g.clone())).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "no such hierarchy yet");

    // A different person confirms. It exists, with a root unit.
    let (s, done) = send(&w.app, "POST", &format!("/api/v1/admin/access/changes/{change}/decide"), ADMIN_TWO, Some(json!({ "decision": "confirm", "reason": "Reviewed" }))).await;
    assert_eq!(s, StatusCode::OK, "{done}");
    assert!(hierarchy_keys(&w, ADMIN).await.contains(&"supervision".to_string()));
    let roots: i64 = w.db.run(|c| c.query_one("SELECT COUNT(*) FROM organizations WHERE hierarchy_key = 'supervision' AND parent_id IS NULL", &[]).unwrap().get(0));
    assert_eq!(roots, 1, "a new hierarchy has a root unit");
    let (_, ledger) = send(&w.app, "GET", "/api/v1/governance/ledger?limit=5", ADMIN, None).await;
    assert!(ledger.to_string().contains("HierarchyChanged"), "{ledger}");
}

#[tokio::test]
async fn a_hierarchy_needs_a_valid_key_kind_source_and_reason() {
    let w = world();
    for bad in [
        json!({ "key": "Bad Key", "name": "X", "kind": "units", "source": "api", "reason": "r" }),
        json!({ "key": "academic", "name": "X", "kind": "units", "source": "api", "reason": "r" }),
        json!({ "key": "ok", "name": "X", "kind": "tree", "source": "api", "reason": "r" }),
        json!({ "key": "ok", "name": "X", "kind": "units", "source": "carrier-pigeon", "reason": "r" }),
        json!({ "key": "ok", "name": "X", "kind": "units", "source": "api" }),
        json!({ "key": "ok", "name": "", "kind": "units", "source": "api", "reason": "r" }),
    ] {
        let (s, body) = send(&w.app, "POST", "/api/v1/admin/hierarchies", ADMIN, Some(bad.clone())).await;
        assert_eq!(s, StatusCode::BAD_REQUEST, "{bad} -> {body}");
    }
    // Only a Platform Admin proposes.
    let (s, _) = send(&w.app, "POST", "/api/v1/admin/hierarchies", READER, Some(json!({ "key": "ok", "name": "X", "kind": "units", "source": "api", "reason": "r" }))).await;
    assert_eq!(s, StatusCode::FORBIDDEN);
}

// A retired hierarchy: its grants stop reaching, its units stay, and it is never deleted.
async fn make_hierarchy(w: &World, key: &str) {
    let body = json!({ "key": key, "name": key, "kind": "units", "source": "api", "reason": "A second tree" });
    let (_, pending) = send(&w.app, "POST", "/api/v1/admin/hierarchies", ADMIN, Some(body)).await;
    let change = pending["change_id"].as_str().unwrap().to_string();
    let (s, done) = send(&w.app, "POST", &format!("/api/v1/admin/access/changes/{change}/decide"), ADMIN_TWO, Some(json!({ "decision": "confirm", "reason": "Reviewed" }))).await;
    assert_eq!(s, StatusCode::OK, "{done}");
}

#[tokio::test]
async fn a_retired_hierarchy_stops_reaching_and_keeps_its_units() {
    let w = world();
    make_hierarchy(&w, "campus").await;
    let root: String = w.db.run(|c| c.query_one("SELECT id::text FROM organizations WHERE hierarchy_key = 'campus' AND parent_id IS NULL", &[]).unwrap().get(0));
    // A grant anchored at the new hierarchy's root.
    let mut g = grant_body(&w, READER);
    g["hierarchy"] = json!("campus");
    g["anchor_id"] = json!(root);
    let (_, pending) = send(&w.app, "POST", "/api/v1/admin/access/grants", ADMIN, Some(g)).await;
    let change = pending["change_id"].as_str().unwrap().to_string();
    send(&w.app, "POST", &format!("/api/v1/admin/access/changes/{change}/decide"), ADMIN_TWO, Some(json!({ "decision": "confirm", "reason": "Reviewed" }))).await;
    let held = |app: axum::Router| async move {
        let (_, body) = send(&app, "GET", &format!("/api/v1/admin/access/people/{READER}"), ADMIN, None).await;
        body["grants"].as_array().map(|g| g.len()).unwrap_or(0)
    };
    assert_eq!(held(w.app.clone()).await, 1);

    // A reason is required.
    let (s, _) = send(&w.app, "POST", "/api/v1/admin/hierarchies/campus/retire", ADMIN, Some(json!({}))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    let (s, body) = send(&w.app, "POST", "/api/v1/admin/hierarchies/campus/retire", ADMIN, Some(json!({ "reason": "No longer used" }))).await;
    assert_eq!(s, StatusCode::OK, "{body}");
    assert_eq!(held(w.app.clone()).await, 0, "the grant no longer reaches");
    let (_, list) = send(&w.app, "GET", "/api/v1/admin/hierarchies", ADMIN, None).await;
    let row = list.as_array().unwrap().iter().find(|h| h["key"] == "campus").unwrap();
    assert_eq!(row["status"], "Retired");
    let units: i64 = w.db.run(|c| c.query_one("SELECT COUNT(*) FROM organizations WHERE hierarchy_key = 'campus'", &[]).unwrap().get(0));
    assert_eq!(units, 1, "its units stay for audit");
    // The academic hierarchy cannot be retired, and a retired one cannot be retired twice.
    let (s, _) = send(&w.app, "POST", "/api/v1/admin/hierarchies/academic/retire", ADMIN, Some(json!({ "reason": "Oops" }))).await;
    assert_eq!(s, StatusCode::CONFLICT);
    let (s, _) = send(&w.app, "POST", "/api/v1/admin/hierarchies/campus/retire", ADMIN, Some(json!({ "reason": "Again" }))).await;
    assert_eq!(s, StatusCode::CONFLICT);
}

// 2. A grant can be previewed, with its impact, before it is proposed. Nothing is stored.
#[tokio::test]
async fn a_grant_is_previewed_with_its_impact_and_nothing_is_stored() {
    let w = world();
    issue_test_token_and_user(READER);
    let (s, preview) = send(&w.app, "POST", "/api/v1/admin/access/grants/preview", ADMIN, Some(grant_body(&w, READER))).await;
    assert_eq!(s, StatusCode::OK, "{preview}");
    assert!(preview["impact"]["per_role"].is_object() || preview["impact"]["gain"].is_array(), "{preview}");
    let (_, pending) = send(&w.app, "GET", "/api/v1/admin/access/grants?state=pending", ADMIN, None).await;
    assert!(pending["changes"].as_array().unwrap().is_empty(), "a preview stores nothing");
    // A grant that would be refused is refused in the preview, with the same words.
    let mut bad = grant_body(&w, READER);
    bad["valid_until"] = json!(in_days(400));
    let (s, body) = send(&w.app, "POST", "/api/v1/admin/access/grants/preview", ADMIN, Some(bad)).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{body}");
    // Not for a person with no right to give access.
    let (s, _) = send(&w.app, "POST", "/api/v1/admin/access/grants/preview", READER, Some(grant_body(&w, READER))).await;
    assert_eq!(s, StatusCode::FORBIDDEN);
}

// People and objects can be searched, a page at a time.
#[tokio::test]
async fn people_and_objects_are_searched_a_page_at_a_time() {
    let w = world();
    for n in 0..5 {
        issue_test_token_and_user(&format!("finder{n}@state.edu"));
    }
    let (s, page) = send(&w.app, "GET", "/api/v1/admin/access/people?q=finder&limit=2", ADMIN, None).await;
    assert_eq!(s, StatusCode::OK, "{page}");
    assert_eq!(page["people"].as_array().unwrap().len(), 2);
    let cursor = page["next_cursor"].as_str().expect("more pages").to_string();
    let (_, page2) = send(&w.app, "GET", &format!("/api/v1/admin/access/people?q=finder&limit=10&cursor={cursor}"), ADMIN, None).await;
    assert_eq!(page2["people"].as_array().unwrap().len(), 3);
    assert!(page2["next_cursor"].is_null());
    let (s, _) = send(&w.app, "GET", "/api/v1/admin/access/people?q=finder", READER, None).await;
    assert_eq!(s, StatusCode::FORBIDDEN);

    let (s, found) = send(&w.app, "GET", "/api/v1/admin/access/objects?q=biology", ADMIN, None).await;
    assert_eq!(s, StatusCode::OK, "{found}");
    assert!(found["objects"].as_array().unwrap().iter().all(|o| o["container"].as_str().is_some() && o["title"].as_str().is_some()), "{found}");
}

// 5. Every list pages in SQL. With 10,000 grants each page comes back in under 200 ms.
#[tokio::test]
async fn every_list_returns_a_page_of_ten_thousand_grants_in_under_200_ms() {
    let w = world();
    let unit = w.science.to_string();
    w.db.run(move |c| {
        c.execute(
            "INSERT INTO access_grants (holder_kind, holder_id, role, hierarchy_key, anchor_kind, anchor_id, source, reason, created_by, created_at) \
             SELECT 'person', 'person' || g || '@state.edu', 'viewer', 'academic', 'unit', $1, 'appointment', 'Bulk', 'test', NOW() - interval '400 days' FROM generate_series(1, 10000) g",
            &[&unit],
        )
        .unwrap();
        c.execute(
            "INSERT INTO persons (id, first_name, last_name, email, eppn) SELECT gen_random_uuid(), 'P', g::text, 'person' || g || '@state.edu', 'person' || g || '@state.edu' FROM generate_series(1, 10000) g ON CONFLICT DO NOTHING",
            &[],
        )
        .unwrap();
        c.execute(
            "INSERT INTO scim_users (id, user_name, active, payload, updated_at) SELECT 'bulk-' || g, 'person' || g || '@state.edu', true, jsonb_build_object('id', 'bulk-' || g, 'userName', 'person' || g || '@state.edu', 'active', true, 'name', jsonb_build_object('formatted', 'Person ' || g)), NOW() FROM generate_series(1, 10000) g ON CONFLICT DO NOTHING",
            &[],
        )
        .unwrap();
    });
    let mut times = Vec::new();
    // The token is made once. Making one costs a database round trip of its own, which is not the page's.
    let token = issue_test_token_and_user(ADMIN);
    let get = |uri: &'static str| {
        let app = w.app.clone();
        let token = token.clone();
        async move {
            let req = Request::builder().method("GET").uri(uri).header("authorization", format!("Bearer {token}")).body(Body::empty()).unwrap();
            let resp = app.oneshot(req).await.unwrap();
            let status = resp.status();
            let bytes = resp.into_body().collect().await.unwrap().to_bytes();
            (status, String::from_utf8_lossy(&bytes).to_string())
        }
    };
    for uri in [
        "/api/v1/admin/access/grants?state=active&limit=50",
        "/api/v1/admin/access/reviews?limit=50",
        "/api/v1/admin/access/people?q=person&limit=50",
        "/api/v1/admin/access/receipts?limit=50",
        "/api/v1/admin/hierarchies",
    ] {
        // One warm-up, then the median of five.
        get(uri).await;
        let mut runs = Vec::new();
        for _ in 0..5 {
            let t = std::time::Instant::now();
            let (s, body) = get(uri).await;
            runs.push(t.elapsed());
            assert_eq!(s, StatusCode::OK, "{uri}: {body}");
        }
        runs.sort();
        let median = runs[2];
        eprintln!("PAGE {uri}: median {median:?}");
        times.push((uri, median));
    }
    for (uri, t) in times {
        assert!(t < std::time::Duration::from_millis(200), "{uri} took {t:?}");
    }
    // The reviews list pages: 10,000 grants are due, and a page holds 50 with a cursor for the next.
    let (_, page) = send(&w.app, "GET", "/api/v1/admin/access/reviews?limit=50", ADMIN, None).await;
    assert_eq!(page["due"].as_array().unwrap().len(), 50);
    assert!(page["next_cursor"].is_string(), "{page}");
}

// 3. The explanation names each unit on the path. The console shows names, not ids.
#[tokio::test]
async fn an_explanation_names_the_units_on_the_path() {
    let db = TestDb::new();
    let state = db.state();
    let science = unit(&state, "science", None);
    let physics = unit(&state, "physics", Some(science));
    workspace(&state, "ws-physics", physics);
    appoint(&state, ADMIN_TWO, Uuid::parse_str(ROOT).unwrap(), "platform_admin", None);
    let app = build_app_with_state(state).expect("router");
    let g = json!({ "holder_kind": "person", "holder_id": READER, "role": "viewer", "hierarchy": "academic", "anchor_kind": "unit", "anchor_id": science.to_string(), "source": "request", "valid_until": in_days(30), "reason": "Test grant" });
    let (_, pending) = send(&app, "POST", "/api/v1/admin/access/grants", ADMIN, Some(g)).await;
    let change = pending["change_id"].as_str().unwrap();
    send(&app, "POST", &format!("/api/v1/admin/access/changes/{change}/decide"), ADMIN_TWO, Some(json!({ "decision": "confirm", "reason": "Reviewed" }))).await;
    let (s, body) = send(&app, "GET", &format!("/api/v1/access/explain?person={READER}&object=workspace:ws-physics"), ADMIN, None).await;
    assert_eq!(s, StatusCode::OK, "{body}");
    let reached = body["reached"].as_array().unwrap();
    assert_eq!(reached.len(), 1, "{body}");
    assert_eq!(reached[0]["path_names"], json!(["science", "physics"]), "{body}");
    assert_eq!(reached[0]["anchor_name"], "science", "{body}");
}
