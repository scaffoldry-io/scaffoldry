//! Access model phase 6: a list shows the rows a person's grants reach.
//!
//! A table that places its rows (`row_place`) shows each person the rows in the units their grants reach. A container
//! grant on the workspace shows every row. Several grants are a union. The list, a single record, and a count agree.

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

struct World {
    _db: TestDb,
    app: axum::Router,
    science: Uuid,
    physics: Uuid,
    lab: Uuid,
    chemistry: Uuid,
    slug: String,
}

/// science > physics > lab, science > chemistry. One row in each unit, titled with the unit's name.
async fn world() -> World {
    let db = TestDb::new();
    let state = db.state();
    let root = Uuid::parse_str(ROOT).unwrap();
    let science = unit(&state, "science", root);
    let physics = unit(&state, "physics", science);
    let lab = unit(&state, "lab", physics);
    let chemistry = unit(&state, "chemistry", science);
    workspace(&state, WS, science);
    appoint(&state, ADMIN_TWO, root, "platform_admin");
    let app = build_app_with_state(state).expect("router");
    let slug = format!("cases-{}", &Uuid::new_v4().to_string()[..8]);
    let manifest = json!({
        "slug": slug, "title": "Cases", "description": "d", "organization_code": "INST", "department": "science", "views": [],
        "tables": [{
            "id": "cases", "name": "Cases", "slug": "cases",
            "fields": [
                { "name": "title", "label": "Title", "field_type": "Text", "required": false, "ferpa_sensitive": false },
                { "name": "unit", "label": "Unit", "field_type": "Unit", "hierarchy": "academic", "required": false, "ferpa_sensitive": false },
                { "name": "ssn", "label": "SSN", "field_type": "Text", "required": false, "ferpa_sensitive": true, "indexed": true }
            ],
            "row_place": { "academic": "unit" }
        }]
    });
    let (s, body) = send(&app, "POST", &format!("/api/v1/workspaces/{WS}/apps"), ADMIN, Some(manifest)).await;
    // A protected field in an Internal workspace needs a second person to approve the app.
    if s == StatusCode::ACCEPTED {
        let id = body["proposal_id"].as_str().expect("a proposal id").to_string();
        let (s, done) = send(&app, "POST", &format!("/api/v1/proposals/{id}/decision"), ADMIN_TWO, Some(json!({ "decision": "approve", "rationale": "Reviewed" }))).await;
        assert_eq!(s, StatusCode::OK, "{done}");
    } else {
        assert_eq!(s, StatusCode::CREATED, "{body}");
    }
    for (name, id) in [("science", science), ("physics", physics), ("lab", lab), ("chemistry", chemistry)] {
        let (s, body) = send(&app, "POST", &format!("/api/v1/apps/{slug}/records"), ADMIN, Some(json!({ "data": { "_table_id": "cases", "title": name, "unit": id.to_string(), "ssn": format!("ssn-of-{name}") } }))).await;
        assert_eq!(s, StatusCode::CREATED, "{body}");
    }
    // The indexes of a new app are built by a job. The rows above wrote their index entries as they were saved, so the
    // fields are ready to filter and sort on.
    let slug_for_db = slug.clone();
    db.run(move |c| c.execute("UPDATE index_status SET state = 'ready' WHERE app_slug = $1", &[&slug_for_db]).unwrap());
    World { _db: db, app, science, physics, lab, chemistry, slug }
}

fn unit_grant(who: &str, anchor: Uuid, depth: Option<i64>) -> Value {
    json!({ "holder_kind": "person", "holder_id": who, "role": "viewer", "hierarchy": "academic", "anchor_kind": "unit", "anchor_id": anchor.to_string(), "depth": depth, "source": "request", "reason": "Test grant" })
}

async fn grant(w: &World, body: Value) {
    let (s, pending) = send(&w.app, "POST", "/api/v1/admin/access/grants", ADMIN, Some(body)).await;
    assert_eq!(s, StatusCode::ACCEPTED, "{pending}");
    let change = pending["change_id"].as_str().unwrap();
    let (s, done) = send(&w.app, "POST", &format!("/api/v1/admin/access/changes/{change}/decide"), ADMIN_TWO, Some(json!({ "decision": "confirm", "reason": "Reviewed" }))).await;
    assert_eq!(s, StatusCode::OK, "{done}");
}

async fn tool(w: &World, who: &str, name: &str, args: Value) -> Result<Value, String> {
    let payload = json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": { "name": name, "arguments": args } });
    let (_, v) = send(&w.app, "POST", "/api/mcp", who, Some(payload)).await;
    if let Some(err) = v.get("error") {
        return Err(err["message"].as_str().unwrap_or("").to_string());
    }
    if v["result"]["isError"] == true {
        return Err(v["result"]["content"][0]["text"].as_str().unwrap_or("").to_string());
    }
    Ok(v["result"]["structuredContent"].clone())
}

/// The titles a person's list shows.
async fn listed(w: &World, who: &str) -> BTreeSet<String> {
    let page = tool(w, who, "list_records", json!({ "app_slug": w.slug, "table_id": "cases", "limit": 200 })).await.expect("a list");
    page["records"].as_array().unwrap().iter().map(|r| r["data"]["title"].as_str().unwrap().to_string()).collect()
}

fn set(names: &[&str]) -> BTreeSet<String> {
    names.iter().map(|s| s.to_string()).collect()
}

// 1. A unit grant shows the rows in its subtree, and a depth limits it.
#[tokio::test]
async fn a_unit_grant_shows_the_rows_in_its_subtree_and_a_depth_limits_it() {
    let w = world().await;
    grant(&w, unit_grant("ada@state.edu", w.science, Some(1))).await;
    grant(&w, unit_grant("bo@state.edu", w.science, Some(0))).await;
    grant(&w, unit_grant("cy@state.edu", w.science, None)).await;
    assert_eq!(listed(&w, "ada@state.edu").await, set(&["science", "physics", "chemistry"]), "depth 1 stops above the lab");
    assert_eq!(listed(&w, "bo@state.edu").await, set(&["science"]), "depth 0 is the unit only");
    assert_eq!(listed(&w, "cy@state.edu").await, set(&["science", "physics", "lab", "chemistry"]), "no depth is the whole subtree");
}

// 2. A container grant on the workspace shows every row, and so does a platform admin.
#[tokio::test]
async fn a_container_grant_and_a_platform_admin_see_every_row() {
    let w = world().await;
    grant(&w, json!({ "holder_kind": "person", "holder_id": "dee@state.edu", "role": "viewer", "anchor_kind": "container", "anchor_id": format!("workspace:{WS}"), "source": "request", "reason": "Test grant" })).await;
    assert_eq!(listed(&w, "dee@state.edu").await, set(&["science", "physics", "lab", "chemistry"]));
    assert_eq!(listed(&w, ADMIN).await, set(&["science", "physics", "lab", "chemistry"]));
}

// 3. Several grants are a union. A grant that does not reach the workspace still adds its rows.
#[tokio::test]
async fn several_grants_are_a_union() {
    let w = world().await;
    grant(&w, unit_grant("eve@state.edu", w.science, Some(0))).await;
    grant(&w, unit_grant("eve@state.edu", w.physics, None)).await;
    assert_eq!(listed(&w, "eve@state.edu").await, set(&["science", "physics", "lab"]));
}

// 4. The list, one record, and a count agree.
#[tokio::test]
async fn the_list_a_single_record_and_a_count_agree() {
    let w = world().await;
    grant(&w, unit_grant("fay@state.edu", w.science, Some(1))).await;
    let page = tool(&w, "fay@state.edu", "list_records", json!({ "app_slug": w.slug, "table_id": "cases", "limit": 200 })).await.unwrap();
    let shown: Vec<String> = page["records"].as_array().unwrap().iter().map(|r| r["id"].as_str().unwrap().to_string()).collect();
    assert_eq!(shown.len(), 3);

    let all = tool(&w, ADMIN, "list_records", json!({ "app_slug": w.slug, "table_id": "cases", "limit": 200 })).await.unwrap();
    for r in all["records"].as_array().unwrap() {
        let id = r["id"].as_str().unwrap();
        let got = tool(&w, "fay@state.edu", "get_record", json!({ "app_slug": w.slug, "record_id": id })).await;
        assert_eq!(got.is_ok(), shown.iter().any(|s| s == id), "get_record agrees with the list for {}", r["data"]["title"]);
    }
    let count = tool(&w, "fay@state.edu", "aggregate_records", json!({ "app_slug": w.slug, "table_id": "cases", "function": "count" })).await.unwrap();
    assert_eq!(count["value"].as_f64(), Some(3.0), "{count}");
}

// 5. A grant below the workspace does not open the app. Rows are narrowed for people who are in, never widened.
#[tokio::test]
async fn a_grant_below_the_workspace_does_not_open_the_app() {
    let w = world().await;
    grant(&w, unit_grant("hal@state.edu", w.chemistry, None)).await;
    let err = tool(&w, "hal@state.edu", "list_records", json!({ "app_slug": w.slug, "table_id": "cases", "limit": 200 })).await;
    assert!(err.is_err(), "hal reaches chemistry rows only, and the workspace is above chemistry");
}

// ---------------------------------------------------------------------------------------------------------------------
// Clearance
// ---------------------------------------------------------------------------------------------------------------------

fn cleared_grant(who: &str, role: &str, anchor: Uuid, clearance: Option<&str>) -> Value {
    json!({ "holder_kind": "person", "holder_id": who, "role": role, "hierarchy": "academic", "anchor_kind": "unit", "anchor_id": anchor.to_string(), "clearance": clearance, "source": "request", "reason": "Test grant" })
}

/// The `ssn` values a list shows, by title. A title with no `ssn` key shows `None`.
async fn ssns(w: &World, who: &str, filter: Option<Value>) -> Result<std::collections::BTreeMap<String, Option<String>>, String> {
    let mut args = json!({ "app_slug": w.slug, "table_id": "cases", "limit": 200 });
    if let Some(f) = filter {
        args["filter"] = f;
    }
    let page = tool(w, who, "list_records", args).await?;
    Ok(page["records"].as_array().unwrap().iter().map(|r| (r["data"]["title"].as_str().unwrap().to_string(), r["data"]["ssn"].as_str().map(str::to_string))).collect())
}

fn unit_is(unit: Uuid) -> Value {
    json!({ "conjunction": "And", "clauses": [{ "id": "c1", "field_name": "unit", "operator": "Equals", "value": unit.to_string() }] })
}

// 3. Cleared for protected fields in physics and not in chemistry: shown on a physics-only list, hidden on a list that
// also holds chemistry rows.
#[tokio::test]
async fn a_field_is_shown_on_a_list_only_if_every_grant_that_contributes_rows_clears_it() {
    let w = world().await;
    grant(&w, cleared_grant("ida@state.edu", "viewer", w.science, Some("Restricted"))).await;
    grant(&w, cleared_grant("ida@state.edu", "viewer", w.chemistry, None)).await;
    // The science grant reaches the workspace. Ida reads physics through it, cleared.
    let physics_only = ssns(&w, "ida@state.edu", Some(unit_is(w.physics))).await.unwrap();
    assert_eq!(physics_only.keys().cloned().collect::<Vec<_>>(), ["physics"]);
    assert_eq!(physics_only["physics"].as_deref(), Some("ssn-of-physics"), "a physics-only list shows the field");

    let everything = ssns(&w, "ida@state.edu", None).await.unwrap();
    assert!(everything.len() >= 4, "{everything:?}");
    assert!(everything.values().all(|v| v.is_none()), "chemistry contributes rows and is not cleared, so the field is hidden: {everything:?}");
}

#[tokio::test]
async fn a_viewer_is_cleared_for_nothing_protected_and_an_editor_for_everything() {
    let w = world().await;
    grant(&w, cleared_grant("viv@state.edu", "viewer", w.science, None)).await;
    grant(&w, cleared_grant("ed@state.edu", "editor", w.science, None)).await;
    grant(&w, cleared_grant("lo@state.edu", "editor", w.science, Some("Internal"))).await;
    assert!(ssns(&w, "viv@state.edu", None).await.unwrap().values().all(|v| v.is_none()), "a viewer, as before");
    assert!(ssns(&w, "ed@state.edu", None).await.unwrap().values().all(|v| v.is_some()), "an editor, as before");
    assert!(ssns(&w, "lo@state.edu", None).await.unwrap().values().all(|v| v.is_none()), "a grant may lower an editor's clearance");
}

// 4. Filtering or sorting on a field that a contributing grant does not clear is `field_hidden`.
#[tokio::test]
async fn filtering_or_sorting_on_a_field_a_contributing_grant_does_not_clear_is_field_hidden() {
    let w = world().await;
    grant(&w, cleared_grant("ida@state.edu", "viewer", w.science, Some("Restricted"))).await;
    grant(&w, cleared_grant("ida@state.edu", "viewer", w.chemistry, None)).await;
    let by_ssn = json!({ "conjunction": "And", "clauses": [{ "id": "c1", "field_name": "ssn", "operator": "Equals", "value": "ssn-of-physics" }] });
    let err = tool(&w, "ida@state.edu", "list_records", json!({ "app_slug": w.slug, "table_id": "cases", "filter": by_ssn })).await.unwrap_err();
    assert!(err.contains("'ssn'") && err.contains("cannot see"), "{err}");
    let err = tool(&w, "ida@state.edu", "list_records", json!({ "app_slug": w.slug, "table_id": "cases", "sort": { "field_name": "ssn", "direction": "Asc" } })).await.unwrap_err();
    assert!(err.to_lowercase().contains("ssn"), "{err}");
    // Someone cleared everywhere may.
    grant(&w, cleared_grant("ed@state.edu", "editor", w.science, None)).await;
    let ok = tool(&w, "ed@state.edu", "list_records", json!({ "app_slug": w.slug, "table_id": "cases", "sort": { "field_name": "ssn", "direction": "Asc" } })).await;
    assert!(ok.is_ok(), "{ok:?}");
}

// ---------------------------------------------------------------------------------------------------------------------
// A hierarchy of people
// ---------------------------------------------------------------------------------------------------------------------

/// A unit in the `supervisory` hierarchy, which holds people.
fn supervisory_unit(state: &ServerState, name: &str, parent: Option<Uuid>) -> Uuid {
    let id = Uuid::new_v4();
    let node = OrganizationNode {
        id,
        parent_id: parent,
        name: name.to_string(),
        code: name.to_uppercase(),
        org_type: "Unit".to_string(),
        hierarchy_key: "supervisory".to_string(),
        synthetic: false,
    };
    state.organizations.write().unwrap().insert(id, node.clone());
    state.persist_organization(&node).unwrap();
    id
}

// Rows placed by a person are reached through the people appointed at the units a grant reaches.
#[tokio::test]
async fn a_people_hierarchy_reaches_the_rows_placed_by_the_people_appointed_in_the_units() {
    let db = TestDb::new();
    let state = db.state();
    db.run(|c| c.execute("INSERT INTO hierarchies (key, name, kind, source, created_by) VALUES ('supervisory', 'Supervisory', 'people', 'api', 'test') ON CONFLICT DO NOTHING", &[]).unwrap());
    let root = Uuid::parse_str(ROOT).unwrap();
    let science = unit(&state, "science", root);
    let physics = unit(&state, "physics", science);
    workspace(&state, WS, science);
    appoint(&state, ADMIN_TWO, root, "platform_admin");
    let boss = supervisory_unit(&state, "boss", None);
    let team_one = supervisory_unit(&state, "team-one", Some(boss));
    let team_two = supervisory_unit(&state, "team-two", Some(boss));
    // A Collaborator field accepts only people the directory knows.
    for who in ["p1", "p2", "p3"] {
        let user: scaffoldry_server::state::ScimUser = serde_json::from_value(json!({ "id": Uuid::new_v4().to_string(), "userName": format!("{who}@state.edu"), "active": true })).expect("a SCIM user");
        state.repository.upsert_scim_user(&user).unwrap();
    }
    appoint(&state, "p1@state.edu", team_one, "staff");
    appoint(&state, "p2@state.edu", team_two, "staff");
    appoint(&state, "p3@state.edu", boss, "staff");
    let app = build_app_with_state(state).expect("router");
    let slug = format!("tickets-{}", &Uuid::new_v4().to_string()[..8]);
    // The table sits in physics. Zed's grant at science reaches the workspace and not the table, so only his supervisory
    // grants decide which rows he sees.
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
    for who in ["p1", "p2", "p3"] {
        let (s, body) = send(&app, "POST", &format!("/api/v1/apps/{slug}/records"), ADMIN, Some(json!({ "data": { "_table_id": "tickets", "title": who, "requester": format!("{who}@state.edu") } }))).await;
        assert_eq!(s, StatusCode::CREATED, "{body}");
    }
    let slug_for_db = slug.clone();
    db.run(move |c| c.execute("UPDATE index_status SET state = 'ready' WHERE app_slug = $1", &[&slug_for_db]).unwrap());
    let w = World { _db: db, app, science, physics, lab: physics, chemistry: physics, slug };

    grant(&w, unit_grant("zed@state.edu", science, Some(0))).await;
    let sup = |who: &str, anchor: Uuid, depth: Option<i64>| json!({ "holder_kind": "person", "holder_id": who, "role": "viewer", "hierarchy": "supervisory", "anchor_kind": "unit", "anchor_id": anchor.to_string(), "depth": depth, "source": "request", "reason": "Test grant" });
    grant(&w, sup("zed@state.edu", team_one, None)).await;
    grant(&w, sup("amy@state.edu", boss, None)).await;
    grant(&w, unit_grant("amy@state.edu", science, Some(0))).await;
    grant(&w, sup("bob@state.edu", boss, Some(0))).await;
    grant(&w, unit_grant("bob@state.edu", science, Some(0))).await;

    let titles = |who: &'static str| {
        let w = &w;
        async move {
            let page = tool(w, who, "list_records", json!({ "app_slug": w.slug, "table_id": "tickets", "limit": 200 })).await.expect("a list");
            page["records"].as_array().unwrap().iter().map(|r| r["data"]["title"].as_str().unwrap().to_string()).collect::<BTreeSet<_>>()
        }
    };
    assert_eq!(titles("zed@state.edu").await, set(&["p1"]), "team one holds p1");
    assert_eq!(titles("amy@state.edu").await, set(&["p1", "p2", "p3"]), "the boss's subtree holds everyone");
    assert_eq!(titles("bob@state.edu").await, set(&["p3"]), "depth 0 is the boss's own unit, which holds p3");
}

// A scope that names many units is read whole by an aggregate, so the statement is planned for a large set.
#[test]
fn a_scope_with_many_units_is_large_and_a_small_or_empty_one_is_not() {
    use scaffoldry_core::access::{Op, Operand, Pred};
    use scaffoldry_server::repository::{ScopePlan, LARGE_SCOPE_VALUES};
    let members = |n: usize| Pred::Cmp { field: "unit".into(), op: Op::In, value: Operand::Literal(Value::Array((0..n).map(|i| json!(format!("u{i}"))).collect())) };
    let plan = |pred| ScopePlan { pred, indexed: vec![] };
    assert!(!plan(members(LARGE_SCOPE_VALUES - 1)).is_large());
    assert!(plan(members(LARGE_SCOPE_VALUES)).is_large());
    // The values of every comparison count, across an OR.
    assert!(plan(Pred::Or(vec![members(LARGE_SCOPE_VALUES / 2), members(LARGE_SCOPE_VALUES / 2 + 1)])).is_large());
    assert!(!plan(Pred::True).is_large());
}

/// Statements the database has run for this test's database so far: committed transactions, which is one per statement here.
fn statements(w: &World) -> i64 {
    w._db.run(|c| {
        c.batch_execute("SELECT pg_stat_clear_snapshot()").unwrap();
        c.query_one("SELECT (xact_commit + xact_rollback)::bigint FROM pg_stat_database WHERE datname = current_database()", &[]).unwrap().get(0)
    })
}

// The statements a list runs do not grow with the number of grants the caller holds.
#[tokio::test]
async fn a_list_runs_the_same_statements_for_one_grant_and_for_four() {
    let w = world().await;
    grant(&w, unit_grant("one@state.edu", w.science, None)).await;
    for anchor in [w.science, w.physics, w.chemistry, w.lab] {
        grant(&w, unit_grant("four@state.edu", anchor, None)).await;
    }
    let mut used = Vec::new();
    for who in ["one@state.edu", "four@state.edu"] {
        let _ = listed(&w, who).await;
        // The database counts a session's transactions when the session ends, so let the pooled connections settle.
        std::thread::sleep(std::time::Duration::from_millis(1500));
        let before = statements(&w);
        let _ = listed(&w, who).await;
        std::thread::sleep(std::time::Duration::from_millis(1500));
        used.push(statements(&w) - before);
    }
    assert!(used[1] <= used[0] + 2, "one grant ran {} statements, four ran {}", used[0], used[1]);
}
