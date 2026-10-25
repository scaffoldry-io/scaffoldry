//! Access model phase 7: sealing, the explanation, and review both ways.

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
const ROOT: &str = "00000000-0000-0000-0000-000000000001";
const WS: &str = "ws-science";
const WS_LAB: &str = "ws-lab";
const SCIENCE_ADMIN: &str = "science.admin@state.edu";
const CHEMISTRY_ADMIN: &str = "chemistry.admin@state.edu";

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

fn manifest(slug: &str, cases_sealed: bool, notes_sealed: bool) -> Value {
    let field = |name: &str| json!({ "name": name, "label": name, "field_type": "Text", "required": false, "ferpa_sensitive": false });
    json!({
        "slug": slug, "title": "Lab", "description": "d", "organization_code": "INST", "department": "science", "views": [],
        "tables": [
            { "id": "cases", "name": "Cases", "slug": "cases", "sealed": cases_sealed, "fields": [field("title")] },
            { "id": "notes", "name": "Notes", "slug": "notes", "sealed": notes_sealed, "fields": [field("title")] }
        ]
    })
}

/// science > physics > lab, science > chemistry. The workspace sits at physics, a department of the college. The app has a sealed table `cases` and
/// an open table `notes`, one row of each.
async fn world() -> World {
    let db = TestDb::new();
    let state = db.state();
    let root = Uuid::parse_str(ROOT).unwrap();
    let science = unit(&state, "science", root);
    let physics = unit(&state, "physics", science);
    let lab = unit(&state, "lab", physics);
    let chemistry = unit(&state, "chemistry", science);
    workspace(&state, WS, physics);
    workspace(&state, WS_LAB, lab);
    appoint(&state, ADMIN_TWO, root, "platform_admin");
    appoint(&state, SCIENCE_ADMIN, science, "unit_admin");
    appoint(&state, CHEMISTRY_ADMIN, chemistry, "unit_admin");
    appoint(&state, "ann@state.edu", physics, "staff");
    appoint(&state, "bo@state.edu", lab, "staff");
    appoint(&state, "cy@state.edu", chemistry, "staff");
    appoint(&state, "dee@state.edu", physics, "staff");
    let app = build_app_with_state(state).expect("router");
    let slug = format!("lab-{}", &Uuid::new_v4().to_string()[..8]);
    let manifest = manifest(&slug, true, false);
    let (s, body) = send(&app, "POST", &format!("/api/v1/workspaces/{WS}/apps"), ADMIN, Some(manifest)).await;
    assert_eq!(s, StatusCode::CREATED, "{body}");
    for table in ["cases", "notes"] {
        let (s, body) = send(&app, "POST", &format!("/api/v1/apps/{slug}/records"), ADMIN, Some(json!({ "data": { "_table_id": table, "title": format!("a {table} row") } }))).await;
        assert_eq!(s, StatusCode::CREATED, "{body}");
    }
    World { _db: db, app, science, physics, lab, chemistry, slug }
}

fn unit_grant(who: &str, role: &str, anchor: Uuid) -> Value {
    json!({ "holder_kind": "person", "holder_id": who, "role": role, "hierarchy": "academic", "anchor_kind": "unit", "anchor_id": anchor.to_string(), "source": "request", "valid_until": (chrono::Utc::now() + chrono::Duration::days(30)).to_rfc3339(), "reason": "Test grant" })
}

fn container_grant(who: &str, role: &str, anchor: &str) -> Value {
    json!({ "holder_kind": "person", "holder_id": who, "role": role, "anchor_kind": "container", "anchor_id": anchor, "source": "request", "valid_until": (chrono::Utc::now() + chrono::Duration::days(30)).to_rfc3339(), "reason": "Test grant" })
}

async fn grant(w: &World, body: Value) -> i64 {
    let (s, pending) = send(&w.app, "POST", "/api/v1/admin/access/grants", ADMIN, Some(body)).await;
    assert_eq!(s, StatusCode::ACCEPTED, "{pending}");
    let change = pending["change_id"].as_str().unwrap();
    let (s, done) = send(&w.app, "POST", &format!("/api/v1/admin/access/changes/{change}/decide"), ADMIN_TWO, Some(json!({ "decision": "confirm", "reason": "Reviewed" }))).await;
    assert_eq!(s, StatusCode::OK, "{done}");
    done["grant_id"].as_i64().expect("the confirmed change names the grant")
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

/// How many rows of `table` the person's list shows. An error (no role in the app at all) shows none.
async fn rows_of(w: &World, who: &str, table: &str) -> usize {
    match tool(w, who, "list_records", json!({ "app_slug": w.slug, "table_id": table, "limit": 200 })).await {
        Ok(page) => page["records"].as_array().map_or(0, Vec::len),
        Err(_) => 0,
    }
}

// 1. A sealed table hides its rows from a dean's college grant and from the workspace's editors. A grant anchored at the
// table still reaches.
#[tokio::test]
async fn a_sealed_table_hides_its_rows_from_the_grants_above_it() {
    let w = world().await;
    let table = format!("table:{}/cases", w.slug);
    grant(&w, unit_grant("dean@state.edu", "viewer", w.science)).await;
    grant(&w, container_grant("editor@state.edu", "editor", &format!("workspace:{WS}"))).await;
    grant(&w, container_grant("keeper@state.edu", "viewer", &format!("app:{}", w.slug))).await;
    grant(&w, container_grant("keeper@state.edu", "viewer", &table)).await;

    assert_eq!(rows_of(&w, "dean@state.edu", "notes").await, 1, "the dean reads the open table");
    assert_eq!(rows_of(&w, "dean@state.edu", "cases").await, 0, "and not the sealed one");
    assert_eq!(rows_of(&w, "editor@state.edu", "notes").await, 1);
    assert_eq!(rows_of(&w, "editor@state.edu", "cases").await, 0, "the workspace's editors are above the seal");
    assert_eq!(rows_of(&w, "keeper@state.edu", "cases").await, 1, "a grant anchored at the table reaches its rows");
    assert_eq!(rows_of(&w, ADMIN, "cases").await, 1, "a Platform Admin is not stopped");
}

async fn coordinates(w: &World, who: &str, body: Value) -> (StatusCode, Value) {
    send(&w.app, "PUT", &format!("/api/v1/workspaces/{WS}/coordinates"), who, Some(body)).await
}

// 2. Sealing applies at once. Unsealing is not in force until a different person confirms it.
#[tokio::test]
async fn unsealing_is_not_in_force_until_a_different_person_confirms_it() {
    let w = world().await;
    grant(&w, unit_grant("dean@state.edu", "viewer", w.science)).await;
    assert_eq!(rows_of(&w, "dean@state.edu", "notes").await, 1);

    let (s, body) = coordinates(&w, ADMIN, json!({ "sealed": true, "reason": "Closed for a review" })).await;
    assert_eq!(s, StatusCode::OK, "sealing narrows, so it applies at once: {body}");
    assert_eq!(rows_of(&w, "dean@state.edu", "notes").await, 0, "the dean's grant is above the sealed workspace");

    let (s, body) = coordinates(&w, ADMIN, json!({ "reason": "Review is over" })).await;
    assert_eq!(s, StatusCode::ACCEPTED, "unsealing widens: {body}");
    assert_eq!(rows_of(&w, "dean@state.edu", "notes").await, 0, "still sealed while the change waits");
    let change = body["change_id"].as_str().unwrap().to_string();
    let uri = format!("/api/v1/workspaces/{WS}/coordinates/changes/{change}/decide");
    let (s, _) = send(&w.app, "POST", &uri, ADMIN, Some(json!({ "decision": "confirm" }))).await;
    assert_eq!(s, StatusCode::FORBIDDEN, "the proposer cannot confirm");
    let (s, body) = send(&w.app, "POST", &uri, ADMIN_TWO, Some(json!({ "decision": "confirm" }))).await;
    assert_eq!(s, StatusCode::OK, "{body}");
    assert_eq!(rows_of(&w, "dean@state.edu", "notes").await, 1, "now it is in force");
}

fn check<'a>(proposal: &'a Value, name: &str) -> &'a Value {
    proposal["checks"].as_array().unwrap().iter().find(|c| c["name"] == name).unwrap_or_else(|| panic!("no check {name} in {proposal}"))
}

// Sealing a table in a manifest narrows and passes. Unsealing one raises `coordinates_widened`, and says which.
#[tokio::test]
async fn unsealing_a_table_in_a_manifest_needs_a_second_person() {
    let w = world().await;
    let propose = |m: Value| tool(&w, ADMIN, "propose_app_change", json!({ "workspace_id": WS, "manifest": m, "summary": "Change the seals" }));
    let sealed_more = propose(manifest(&w.slug, true, true)).await.expect("a proposal");
    assert_eq!(check(&sealed_more, "coordinates_widened")["outcome"], "pass", "sealing narrows: {sealed_more}");
    let unsealed = propose(manifest(&w.slug, false, false)).await.expect("a proposal");
    let c = check(&unsealed, "coordinates_widened");
    assert_eq!(c["outcome"], "fail", "{unsealed}");
    assert!(c.to_string().contains("cases") && c.to_string().contains("unsealed"), "and names the table: {c}");
}

// A grant may anchor at a table that exists, and only at one.
#[tokio::test]
async fn a_grant_anchors_at_a_table_that_exists() {
    let w = world().await;
    let (s, body) = send(&w.app, "POST", "/api/v1/admin/access/grants", ADMIN, Some(container_grant("x@state.edu", "viewer", &format!("table:{}/ghost", w.slug)))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{body}");
    assert!(body["error"].as_str().unwrap_or("").contains("table:<slug>/<table id>"), "{body}");
}

async fn explain(w: &World, who: &str, person: &str, object: &str) -> (StatusCode, Value) {
    send(&w.app, "GET", &format!("/api/v1/access/explain?person={person}&object={object}"), who, None).await
}

// 3. For a denied person, `explain` names the axis that failed. For an allowed person it names the grant id and the closure
// path from the anchor to the object's unit. A sealed container is named when it stopped a grant.
#[tokio::test]
async fn explain_names_the_axis_that_failed_and_the_path_that_reached() {
    let w = world().await;
    let ada = grant(&w, unit_grant("ada@state.edu", "viewer", w.science)).await;
    let eve = grant(&w, unit_grant("eve@state.edu", "viewer", w.chemistry)).await;
    grant(&w, unit_grant("dean@state.edu", "viewer", w.science)).await;
    let lab = format!("workspace:{WS_LAB}");

    let (s, allowed) = explain(&w, ADMIN, "ada@state.edu", &lab).await;
    assert_eq!(s, StatusCode::OK, "{allowed}");
    assert_eq!(allowed["role"], "viewer");
    let reached = &allowed["reached"][0];
    assert_eq!(reached["grant_id"], ada, "{allowed}");
    let path: Vec<&str> = reached["path"].as_array().unwrap().iter().map(|p| p.as_str().unwrap()).collect();
    assert_eq!(path, [w.science.to_string(), w.physics.to_string(), w.lab.to_string()], "from the anchor down to the object's unit");
    assert!(allowed["missed"].as_array().unwrap().is_empty());

    let (s, denied) = explain(&w, ADMIN, "eve@state.edu", &lab).await;
    assert_eq!(s, StatusCode::OK, "{denied}");
    assert_eq!(denied["role"], Value::Null);
    assert!(denied["reached"].as_array().unwrap().is_empty());
    assert_eq!(denied["missed"][0]["grant_id"], eve);
    assert_eq!(denied["missed"][0]["axis"], "anchor", "chemistry is not above the lab: {denied}");

    let open = format!("table:{}/notes", w.slug);
    let (s, reaching) = explain(&w, ADMIN, "dean@state.edu", &open).await;
    assert_eq!(s, StatusCode::OK, "{reaching}");
    assert_eq!(reaching["role"], "viewer", "an open table sits where its workspace sits: {reaching}");
    assert_eq!(reaching["reached"][0]["path"][0], w.science.to_string());

    let sealed = format!("table:{}/cases", w.slug);
    let (s, stopped) = explain(&w, ADMIN, "dean@state.edu", &sealed).await;
    assert_eq!(s, StatusCode::OK, "{stopped}");
    assert_eq!(stopped["role"], Value::Null);
    assert_eq!(stopped["missed"][0]["axis"], "sealed", "{stopped}");
    assert_eq!(stopped["missed"][0]["sealed_by"], sealed.as_str());
    assert!(stopped["policy"].is_object() && stopped["guards"].is_array() && stopped["rules"].is_array(), "the answer also carries the policy, guards and rules: {stopped}");

    // Who may ask: the person, a Platform Admin, an Org Unit Admin whose subtree holds the object. No one else.
    assert_eq!(explain(&w, "eve@state.edu", "eve@state.edu", &lab).await.0, StatusCode::OK, "a person may ask about themselves");
    assert_eq!(explain(&w, "eve@state.edu", "ada@state.edu", &lab).await.0, StatusCode::FORBIDDEN);
    assert_eq!(explain(&w, SCIENCE_ADMIN, "ada@state.edu", &lab).await.0, StatusCode::OK, "science holds the lab");
    assert_eq!(explain(&w, CHEMISTRY_ADMIN, "ada@state.edu", &lab).await.0, StatusCode::FORBIDDEN, "chemistry does not");
}

// 6. `explain_access` explains the caller's own access, and refuses to explain another person to an agent token.
#[tokio::test]
async fn explain_access_refuses_to_explain_another_person_to_an_agent_token() {
    let w = world().await;
    grant(&w, unit_grant("ada@state.edu", "viewer", w.science)).await;
    grant(&w, unit_grant("eve@state.edu", "viewer", w.chemistry)).await;
    let (s, minted) = send(&w.app, "POST", "/api/v1/auth/tokens", "ada@state.edu", Some(json!({ "label": "an agent", "kind": "agent" }))).await;
    assert_eq!(s, StatusCode::CREATED, "{minted}");
    let token = minted["token"].as_str().unwrap().to_string();
    let call = |args: Value| {
        let app = w.app.clone();
        let token = token.clone();
        async move {
            let payload = json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": { "name": "explain_access", "arguments": args } });
            let req = Request::builder().method("POST").uri("/api/mcp").header("authorization", format!("Bearer {token}")).header("content-type", "application/json").body(Body::from(payload.to_string())).unwrap();
            let resp = app.oneshot(req).await.unwrap();
            let bytes = resp.into_body().collect().await.unwrap().to_bytes();
            serde_json::from_slice::<Value>(&bytes).unwrap()
        }
    };
    let lab = format!("workspace:{WS_LAB}");
    let own = call(json!({ "object": lab })).await;
    assert_ne!(own["result"]["isError"], true, "{own}");
    assert_eq!(own["result"]["structuredContent"]["role"], "viewer", "{own}");
    let other = call(json!({ "object": lab, "person": "eve@state.edu" })).await;
    assert_eq!(other["result"]["isError"], true, "{other}");
    assert!(other["result"]["content"][0]["text"].as_str().unwrap().contains("your own"), "{other}");
}

fn ids(list: &Value, key: &str) -> Vec<String> {
    let mut out: Vec<String> = list.as_array().unwrap().iter().map(|x| x[key].as_str().unwrap().to_string()).collect();
    out.sort();
    out.dedup();
    out
}

// 4. The per-object review lists a college grant on a department's table. The per-person review counts it.
#[tokio::test]
async fn the_reviews_list_a_college_grant_on_a_departments_table_and_count_it() {
    let w = world().await;
    let dean = grant(&w, unit_grant("dean@state.edu", "viewer", w.science)).await;
    let member = grant(&w, container_grant("member@state.edu", "editor", &format!("workspace:{WS}"))).await;
    grant(&w, unit_grant("eve@state.edu", "viewer", w.chemistry)).await;

    let (s, object) = send(&w.app, "GET", &format!("/api/v1/admin/access/objects/table/{}/notes", w.slug), ADMIN, None).await;
    assert_eq!(s, StatusCode::OK, "{object}");
    let listed: Vec<i64> = object["grants"].as_array().unwrap().iter().filter_map(|g| g["grant_id"].as_i64()).collect();
    assert!(listed.contains(&dean), "the college grant reaches the department's table: {object}");
    assert!(listed.contains(&member), "so does a grant on its workspace: {object}");
    assert_eq!(listed.len(), 2, "and the chemistry grant does not: {object}");
    let college = object["grants"].as_array().unwrap().iter().find(|g| g["grant_id"] == dean).unwrap();
    assert_eq!(college["path"][0], w.science.to_string(), "the review says how it reached: {college}");

    let sealed = send(&w.app, "GET", &format!("/api/v1/admin/access/objects/table/{}/cases", w.slug), ADMIN, None).await.1;
    assert!(sealed["grants"].as_array().unwrap().is_empty(), "nothing above the sealed table reaches it: {sealed}");

    let (s, person) = send(&w.app, "GET", "/api/v1/admin/access/people/dean@state.edu", ADMIN, None).await;
    assert_eq!(s, StatusCode::OK, "{person}");
    let mine = person["grants"].as_array().unwrap().iter().find(|g| g["grant_id"] == dean).unwrap_or_else(|| panic!("the grant is in the person's review: {person}"));
    assert_eq!(mine["reaches"]["tables"], 1, "one table, the open one: {mine}");
    assert_eq!(mine["reaches"]["workspaces"], 2, "both workspaces are under the college: {mine}");
    assert_eq!(person["by_hierarchy"]["academic"]["tables"], 1, "{person}");

    // Only the people who administer may look.
    assert_eq!(send(&w.app, "GET", "/api/v1/admin/access/people/dean@state.edu", "dean@state.edu", None).await.0, StatusCode::FORBIDDEN);
    assert_eq!(send(&w.app, "GET", &format!("/api/v1/admin/access/objects/workspace/{WS}"), "dean@state.edu", None).await.0, StatusCode::FORBIDDEN);
}

fn changes(impact: &Value, side: &str) -> Vec<(String, Option<String>, Option<String>)> {
    impact[side].as_array().unwrap().iter().map(|c| (c["person"].as_str().unwrap().to_string(), c["before"].as_str().map(str::to_string), c["after"].as_str().map(str::to_string))).collect()
}

// 5. The impact of a pending grant lists exactly the people who gain, and changes nothing.
#[tokio::test]
async fn the_impact_of_a_pending_grant_lists_exactly_the_people_who_gain() {
    let w = world().await;
    let app = format!("app:{}", w.slug);
    grant(&w, container_grant("dee@state.edu", "admin", &format!("workspace:{WS}"))).await;
    grant(&w, container_grant("bo@state.edu", "viewer", &app)).await;
    // Everyone appointed in physics or below would become an editor of the app.
    let body = json!({ "holder_kind": "unit", "holder_id": w.physics.to_string(), "role": "editor", "anchor_kind": "container", "anchor_id": app, "source": "request", "valid_until": (chrono::Utc::now() + chrono::Duration::days(30)).to_rfc3339(), "reason": "Test grant" });
    let (s, pending) = send(&w.app, "POST", "/api/v1/admin/access/grants", ADMIN, Some(body)).await;
    assert_eq!(s, StatusCode::ACCEPTED, "{pending}");
    let change = pending["change_id"].as_str().unwrap().to_string();

    let (s, impact) = send(&w.app, "POST", &format!("/api/v1/admin/access/changes/{change}/impact"), ADMIN, None).await;
    assert_eq!(s, StatusCode::OK, "{impact}");
    // By hand: ann is appointed in physics and holds nothing, so she gains editor. bo is in the lab, under physics, and holds
    // viewer on the app, so he goes from viewer to editor. dee is already an admin, so nothing changes for her. cy is in
    // chemistry, which is not under physics. The grant is on the app, so it reaches the app and its open table. The sealed
    // table is below the app, and the app is above the seal.
    assert_eq!(ids(&impact["gain"], "person"), ["ann@state.edu", "bo@state.edu"], "{impact}");
    let notes = format!("table:{}/notes", w.slug);
    let mut gained = changes(&impact, "gain");
    gained.sort();
    let editor = Some("editor".to_string());
    assert_eq!(
        gained,
        vec![
            ("ann@state.edu".to_string(), None, editor.clone()),
            ("ann@state.edu".to_string(), None, editor.clone()),
            ("bo@state.edu".to_string(), Some("viewer".to_string()), editor.clone()),
            ("bo@state.edu".to_string(), Some("viewer".to_string()), editor),
        ],
        "{impact}"
    );
    assert_eq!(ids(&impact["gain"], "object"), [app.clone(), notes], "{impact}");
    assert!(impact["lose"].as_array().unwrap().is_empty(), "{impact}");
    assert_eq!(impact["per_role"]["editor"]["gain"], 4, "{impact}");

    // It changed nothing.
    assert_eq!(rows_of(&w, "ann@state.edu", "notes").await, 0, "ann has no role yet");
    let (_, pending_list) = send(&w.app, "GET", "/api/v1/admin/access/grants?state=pending", ADMIN, None).await;
    assert_eq!(pending_list["changes"].as_array().unwrap().len(), 1, "the change is still pending");
    assert_eq!(send(&w.app, "POST", &format!("/api/v1/admin/access/changes/{change}/impact"), "dean@state.edu", None).await.0, StatusCode::FORBIDDEN);
}

// 5. The impact of a pending unseal lists who the grants above the workspace let back in.
#[tokio::test]
async fn the_impact_of_a_pending_unseal_lists_who_gets_back_in() {
    let w = world().await;
    grant(&w, unit_grant("dean@state.edu", "viewer", w.science)).await;
    grant(&w, container_grant("keeper@state.edu", "viewer", &format!("workspace:{WS}"))).await;
    let (s, body) = coordinates(&w, ADMIN, json!({ "sealed": true, "reason": "Closed for a review" })).await;
    assert_eq!(s, StatusCode::OK, "{body}");
    let (s, pending) = coordinates(&w, ADMIN, json!({ "reason": "Review is over" })).await;
    assert_eq!(s, StatusCode::ACCEPTED, "{pending}");
    let change = pending["change_id"].as_str().unwrap();

    let (s, impact) = send(&w.app, "POST", &format!("/api/v1/admin/access/changes/{change}/impact"), ADMIN, None).await;
    assert_eq!(s, StatusCode::OK, "{impact}");
    // The seal stops every grant above the workspace: the dean's, and the college's unit administrator, whose grant comes from
    // an appointment. The keeper holds a grant at the workspace, which a seal never stops.
    assert_eq!(ids(&impact["gain"], "person"), ["dean@state.edu", SCIENCE_ADMIN], "{impact}");
    let roles: Vec<(&str, &str)> = ["dean@state.edu", SCIENCE_ADMIN].iter().map(|p| (*p, impact["gain"].as_array().unwrap().iter().find(|c| c["person"] == *p).unwrap()["after"].as_str().unwrap())).collect();
    assert_eq!(roles, [("dean@state.edu", "viewer"), (SCIENCE_ADMIN, "admin")], "{impact}");
    assert!(impact["gain"].as_array().unwrap().iter().all(|c| c["before"].is_null()), "{impact}");
    assert!(impact["lose"].as_array().unwrap().is_empty(), "{impact}");
}
