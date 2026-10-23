//! Access model phase 3: every object has coordinates. A workspace, an app, a table and a field sit at a unit in each
//! hierarchy, in a domain, and at a sensitivity. A child may narrow its container's and never widen it.

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::Router;
use common::TestDb;
use http_body_util::BodyExt;
use scaffoldry_server::build_app_with_state;
use scaffoldry_server::service::identity::issue_test_token_and_user;
use serde_json::{json, Value};
use tower::ServiceExt;
use uuid::Uuid;

const WORKSPACE: &str = "ws-physics-optics";
const OWNER: &str = "einstein@physics.state.edu";
const ADMIN: &str = "jordan.lee@state.edu";
const ROOT: &str = "00000000-0000-0000-0000-000000000001";

async fn send(app: &Router, method: &str, uri: &str, who: &str, body: Option<Value>) -> (StatusCode, Value) {
    let b = Request::builder().method(method).uri(uri).header("authorization", format!("Bearer {}", issue_test_token_and_user(who)));
    let req = match body {
        Some(v) => b.header("content-type", "application/json").body(Body::from(v.to_string())).unwrap(),
        None => b.body(Body::empty()).unwrap(),
    };
    let resp = app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
}

async fn tool(app: &Router, who: &str, name: &str, args: Value) -> (bool, Value) {
    let payload = json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": { "name": name, "arguments": args } });
    let (_, v) = send(app, "POST", "/api/mcp", who, Some(payload)).await;
    if let Some(err) = v.get("error") {
        return (true, err["message"].clone());
    }
    let result = &v["result"];
    if result["isError"] == true {
        return (true, result["content"][0]["text"].clone());
    }
    (false, result["structuredContent"].clone())
}

fn world(db: &TestDb) -> Router {
    let state = db.state();
    state.seed_demo().expect("seed");
    build_app_with_state(state).expect("router")
}

async fn make_unit(app: &Router, name: &str, parent: &str) -> String {
    let (s, body) = send(app, "POST", "/api/v1/orgs", ADMIN, Some(json!({ "name": name, "code": name, "org_type": "Department", "parent_id": parent }))).await;
    assert_eq!(s, StatusCode::CREATED, "{body}");
    body["id"].as_str().unwrap().to_string()
}

fn field(name: &str) -> Value {
    json!({ "name": name, "label": name, "field_type": "Text", "required": false, "ferpa_sensitive": false })
}

/// An app with one table `apps`. `app` and `table` are the coordinates each names, or null.
fn manifest(slug: &str, app: Value, table: Value, fields: Vec<Value>) -> Value {
    json!({
        "slug": slug, "title": "Coordinates", "description": "d", "organization_code": "PHYS", "department": "physics", "views": [],
        "coordinates": app,
        "tables": [{ "id": "apps", "name": "Applications", "slug": "apps", "coordinates": table, "fields": fields }]
    })
}

fn slug(prefix: &str) -> String {
    format!("{prefix}-{}", &Uuid::new_v4().to_string()[..8])
}

async fn create_app(app: &Router, m: Value) -> (StatusCode, Value) {
    send(app, "POST", &format!("/api/v1/workspaces/{WORKSPACE}/apps"), OWNER, Some(m)).await
}

// 2. A manifest whose table sits in a unit above its app's is refused, and the refusal names both units.
#[tokio::test]
async fn a_table_in_a_unit_above_its_apps_is_refused_and_both_units_are_named() {
    let db = TestDb::new();
    let app = world(&db);
    let science = make_unit(&app, "science", ROOT).await;
    let physics = make_unit(&app, "physics", &science).await;

    let (s, body) = create_app(&app, manifest(&slug("p3up"), json!({ "units": { "academic": physics } }), json!({ "units": { "academic": science } }), vec![field("name")])).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "the table's unit is above its app's: {body}");
    let text = body.to_string();
    assert!(text.contains(&physics) && text.contains(&science), "it names both units: {text}");
    assert!(text.contains("table") && text.contains("apps"), "and the table: {text}");
    assert!(text.contains("unit:academic"), "and the axis: {text}");

    // The other way round narrows, which is allowed.
    let (s, body) = create_app(&app, manifest(&slug("p3down"), json!({ "units": { "academic": science } }), json!({ "units": { "academic": physics } }), vec![field("name")])).await;
    assert_eq!(s, StatusCode::CREATED, "a table below its app's unit narrows it: {body}");
}

/// A second hierarchy with one root unit `Campus`, put in place by SQL because creating a hierarchy is phase 11.
fn second_hierarchy(db: &TestDb) -> String {
    let campus = Uuid::new_v4();
    db.run(move |c| {
        c.execute("INSERT INTO hierarchies (key, name, kind, source, created_by) VALUES ('sites', 'Sites', 'units', 'api', 'test')", &[]).unwrap();
        c.execute("INSERT INTO organizations (id, parent_id, name, code, org_type, hierarchy_key) VALUES ($1, NULL, 'Campus', 'CAMPUS', 'Campus', 'sites')", &[&campus]).unwrap();
        c.execute("INSERT INTO org_closure (hierarchy_key, ancestor, descendant, depth) VALUES ('sites', $1, $1, 0)", &[&campus]).unwrap();
        c.execute("UPDATE cache_versions SET version = version + 1 WHERE name = 'organizations'", &[]).unwrap();
    });
    campus.to_string()
}

// 3. A `Unit` field refuses a unit from another hierarchy, and its value is in `record_index`.
#[tokio::test]
async fn a_unit_field_refuses_a_unit_of_another_hierarchy_and_is_indexed() {
    let db = TestDb::new();
    let app = world(&db);
    let science = make_unit(&app, "science", ROOT).await;
    let campus = second_hierarchy(&db);

    let slug = slug("p3unit");
    let unit_field = json!({ "name": "unit", "label": "Unit", "field_type": "Unit", "hierarchy": "academic", "required": false, "ferpa_sensitive": false });
    let mut m = manifest(&slug, Value::Null, Value::Null, vec![field("name"), unit_field]);
    m["tables"][0]["row_place"] = json!({ "academic": "unit" });
    let (s, body) = create_app(&app, m).await;
    assert_eq!(s, StatusCode::CREATED, "{body}");

    let write = |unit: &str| {
        let (app, slug, unit) = (app.clone(), slug.clone(), unit.to_string());
        async move { send(&app, "POST", &format!("/api/v1/apps/{slug}/records"), OWNER, Some(json!({ "data": { "_table_id": "apps", "name": "Ada", "unit": unit } }))).await }
    };
    let (s, body) = write(&campus).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "a unit of the sites hierarchy is not a unit of academic: {body}");
    assert!(body.to_string().contains("unit"), "the field is named: {body}");
    let (s, body) = write(&Uuid::new_v4().to_string()).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "a unit that does not exist is refused: {body}");

    let (s, body) = write(&science).await;
    assert_eq!(s, StatusCode::CREATED, "{body}");
    let id = body["id"].as_str().unwrap().to_string();
    let (slug2, science2) = (slug.clone(), science.clone());
    let indexed: i64 = db.run(move |c| c.query_one("SELECT count(*) FROM record_index WHERE app_slug = $1 AND field = 'unit' AND record_id = $2 AND txt = $3", &[&slug2, &id, &science2]).unwrap().get(0));
    assert_eq!(indexed, 1, "the value is in record_index as text");

    // A field that names a hierarchy that does not exist, and a row place that names no such field, are refused.
    let mut bad = manifest(&self::slug("p3bad"), Value::Null, Value::Null, vec![field("name"), json!({ "name": "unit", "label": "Unit", "field_type": "Unit", "hierarchy": "nowhere", "required": false, "ferpa_sensitive": false })]);
    let (s, body) = create_app(&app, bad.clone()).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "a Unit field names a hierarchy that exists: {body}");
    bad["tables"][0]["fields"] = json!([field("name")]);
    bad["tables"][0]["row_place"] = json!({ "academic": "missing" });
    let (s, body) = create_app(&app, bad).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "a row place names a field of the table: {body}");
}

// 4. A guard `deny_outside_domain` forbids `read_app` on an app in another domain.
#[tokio::test]
async fn a_guard_forbids_reading_an_app_in_another_domain() {
    let db = TestDb::new();
    let app = world(&db);
    let (research, teaching, grants) = (slug("p3res"), slug("p3tea"), slug("p3grn"));
    for (s, domain) in [(&research, "research"), (&teaching, "learning-and-teaching"), (&grants, "herm:2.2.3")] {
        let (code, body) = create_app(&app, manifest(s, json!({ "domain": domain }), Value::Null, vec![field("name")])).await;
        assert_eq!(code, StatusCode::CREATED, "{body}");
    }
    for s in [&research, &teaching, &grants] {
        let (code, body) = send(&app, "GET", &format!("/api/v1/apps/{s}/schema"), OWNER, None).await;
        assert_eq!(code, StatusCode::OK, "before the guard every app is readable: {body}");
    }

    let (code, body) = send(&app, "PUT", &format!("/api/v1/workspaces/{WORKSPACE}/guards"), ADMIN, Some(json!({ "rules": [{ "template": "deny_outside_domain", "domains": ["research"] }], "reason": "Research only" }))).await;
    assert_eq!(code, StatusCode::OK, "{body}");
    assert!(body.to_string().to_lowercase().contains("research"), "the guard reads as a sentence that names the domain: {body}");

    let (code, body) = send(&app, "GET", &format!("/api/v1/apps/{teaching}/schema"), OWNER, None).await;
    assert_eq!(code, StatusCode::FORBIDDEN, "an app in another domain is forbidden: {body}");
    let (code, body) = send(&app, "GET", &format!("/api/v1/apps/{research}/schema"), OWNER, None).await;
    assert_eq!(code, StatusCode::OK, "an app in the domain is allowed: {body}");
    let (code, body) = send(&app, "GET", &format!("/api/v1/apps/{grants}/schema"), OWNER, None).await;
    assert_eq!(code, StatusCode::OK, "an app in a domain below it is allowed: {body}");

    // A domain that does not exist cannot be named by a guard.
    let (code, body) = send(&app, "PUT", &format!("/api/v1/workspaces/{WORKSPACE}/guards"), ADMIN, Some(json!({ "rules": [{ "template": "deny_outside_domain", "domains": ["no-such-domain"] }], "reason": "Typo" }))).await;
    assert_eq!(code, StatusCode::BAD_REQUEST, "{body}");
}

// 4b. A guard `deny_above_sensitivity` forbids `read_app` on an app above the level it names.
#[tokio::test]
async fn a_guard_forbids_reading_an_app_above_a_sensitivity() {
    let db = TestDb::new();
    let app = world(&db);
    let (internal, restricted) = (slug("p3int"), slug("p3res"));
    for (s, level) in [(&internal, "Internal"), (&restricted, "Restricted")] {
        let (code, body) = create_app(&app, manifest(s, json!({ "sensitivity": level }), Value::Null, vec![field("name")])).await;
        assert_eq!(code, StatusCode::CREATED, "{body}");
    }
    let (code, body) = send(&app, "PUT", &format!("/api/v1/workspaces/{WORKSPACE}/guards"), ADMIN, Some(json!({ "rules": [{ "template": "deny_above_sensitivity", "sensitivity": "Internal" }], "reason": "Nothing above Internal" }))).await;
    assert_eq!(code, StatusCode::OK, "{body}");
    let (code, body) = send(&app, "GET", &format!("/api/v1/apps/{restricted}/schema"), OWNER, None).await;
    assert_eq!(code, StatusCode::FORBIDDEN, "an app above the level is forbidden: {body}");
    let (code, body) = send(&app, "GET", &format!("/api/v1/apps/{internal}/schema"), OWNER, None).await;
    assert_eq!(code, StatusCode::OK, "an app at the level is allowed: {body}");
    let (code, body) = send(&app, "PUT", &format!("/api/v1/workspaces/{WORKSPACE}/guards"), ADMIN, Some(json!({ "rules": [{ "template": "deny_above_sensitivity", "sensitivity": "Secret" }], "reason": "Typo" }))).await;
    assert_eq!(code, StatusCode::BAD_REQUEST, "a level off the ladder is refused: {body}");
}

fn check<'a>(proposal: &'a Value, name: &str) -> &'a Value {
    proposal["checks"].as_array().unwrap().iter().find(|c| c["name"] == name).unwrap_or_else(|| panic!("no check {name} in {proposal}"))
}

// 5. Lowering a table's sensitivity raises `coordinates_widened` and needs a second person.
#[tokio::test]
async fn lowering_a_tables_sensitivity_raises_coordinates_widened_and_needs_a_second_person() {
    let db = TestDb::new();
    let app = world(&db);
    let slug = slug("p3lower");
    let (code, body) = create_app(&app, manifest(&slug, Value::Null, json!({ "sensitivity": "Restricted" }), vec![field("name")])).await;
    assert_eq!(code, StatusCode::CREATED, "{body}");

    // Raising it, or leaving it alone, passes.
    let (err, same) = tool(&app, OWNER, "propose_app_change", json!({ "workspace_id": WORKSPACE, "manifest": manifest(&slug, Value::Null, json!({ "sensitivity": "Restricted" }), vec![field("name"), field("extra")]), "summary": "A field" })).await;
    assert!(!err, "{same}");
    assert_eq!(check(&same, "coordinates_widened")["outcome"], "pass", "a change that leaves the coordinates alone passes");

    // Lowering it fails the check.
    let (err, lowered) = tool(&app, OWNER, "propose_app_change", json!({ "workspace_id": WORKSPACE, "manifest": manifest(&slug, Value::Null, json!({ "sensitivity": "Internal" }), vec![field("name")]), "summary": "Lower it" })).await;
    assert!(!err, "{lowered}");
    let c = check(&lowered, "coordinates_widened");
    assert_eq!(c["outcome"], "fail", "lowering a table's sensitivity raises the check: {lowered}");
    assert!(c.to_string().contains("Restricted") && c.to_string().contains("Internal"), "and says from what to what: {c}");
    let id = lowered["proposal_id"].as_str().unwrap().to_string();

    // Removing it is lowering it too.
    let (err, removed) = tool(&app, OWNER, "propose_app_change", json!({ "workspace_id": WORKSPACE, "manifest": manifest(&slug, Value::Null, Value::Null, vec![field("name")]), "summary": "Remove it" })).await;
    assert!(!err, "{removed}");
    assert_eq!(check(&removed, "coordinates_widened")["outcome"], "fail", "a removed coordinate falls back to the container's, which is lower");

    // The proposer cannot approve. Another person can.
    let (err, msg) = tool(&app, OWNER, "decide_proposal", json!({ "proposal_id": id, "decision": "approve", "rationale": "Mine", "digest": lowered["digest"] })).await;
    assert!(err, "the proposer is refused: {msg}");
    assert!(msg.as_str().unwrap_or("").contains("second person") || msg.as_str().unwrap_or("").contains("proposer"), "{msg}");
    let (err, done) = tool(&app, ADMIN, "decide_proposal", json!({ "proposal_id": id, "decision": "approve", "rationale": "Reviewed", "digest": lowered["digest"] })).await;
    assert!(!err, "a different person approves: {done}");
}

async fn coordinates(app: &Router, who: &str) -> Value {
    let (s, body) = send(app, "GET", &format!("/api/v1/workspaces/{WORKSPACE}/coordinates"), who, None).await;
    assert_eq!(s, StatusCode::OK, "{body}");
    body
}

// 6. A workspace's widened coordinate is not in force until a different person confirms it.
#[tokio::test]
async fn a_widened_workspace_coordinate_waits_for_a_different_person() {
    let db = TestDb::new();
    let app = world(&db);
    let uri = format!("/api/v1/workspaces/{WORKSPACE}/coordinates");

    // Nothing named: the root domain and Internal.
    let before = coordinates(&app, OWNER).await;
    assert_eq!(before["effective"]["sensitivity"], "Internal", "{before}");
    assert_eq!(before["effective"]["domain"], "institution", "{before}");

    // Narrowing takes effect at once.
    let (s, body) = send(&app, "PUT", &uri, OWNER, Some(json!({ "sensitivity": "Restricted", "domain": "research", "reason": "Grant data" }))).await;
    assert_eq!(s, StatusCode::OK, "narrowing is applied: {body}");
    let now = coordinates(&app, OWNER).await;
    assert_eq!((now["effective"]["sensitivity"].as_str(), now["effective"]["domain"].as_str()), (Some("Restricted"), Some("research")), "{now}");

    // Widening waits.
    let (s, pending) = send(&app, "PUT", &uri, OWNER, Some(json!({ "sensitivity": "Directory", "domain": "research", "reason": "Open it up" }))).await;
    assert_eq!(s, StatusCode::ACCEPTED, "a widening change is stored as pending: {pending}");
    let change = pending["change_id"].as_str().expect("the pending change has an id").to_string();
    let still = coordinates(&app, OWNER).await;
    assert_eq!(still["effective"]["sensitivity"], "Restricted", "it is not in force: {still}");
    assert!(still["pending"].to_string().contains(&change), "but it is listed: {still}");

    // The person who proposed it cannot confirm it.
    let decide = format!("{uri}/changes/{change}/decide");
    let (s, body) = send(&app, "POST", &decide, OWNER, Some(json!({ "decision": "confirm" }))).await;
    assert_eq!(s, StatusCode::FORBIDDEN, "the proposer's own confirmation is refused: {body}");
    assert_eq!(coordinates(&app, OWNER).await["effective"]["sensitivity"], "Restricted");

    // Someone who may not change the coordinates cannot confirm either.
    let (s, body) = send(&app, "POST", &decide, "sarah.connor@state.edu", Some(json!({ "decision": "confirm" }))).await;
    assert!(s == StatusCode::FORBIDDEN || s == StatusCode::NOT_FOUND, "an outsider is refused: {s} {body}");

    // A different person who may confirms it, and it is in force.
    let (s, body) = send(&app, "POST", &decide, ADMIN, Some(json!({ "decision": "confirm" }))).await;
    assert_eq!(s, StatusCode::OK, "{body}");
    assert_eq!(coordinates(&app, OWNER).await["effective"]["sensitivity"], "Directory", "now it is in force");
    let (s, body) = send(&app, "POST", &decide, ADMIN, Some(json!({ "decision": "confirm" }))).await;
    assert_eq!(s, StatusCode::CONFLICT, "a change is decided once: {body}");

    // A reject leaves things as they are.
    let (s, pending) = send(&app, "PUT", &uri, OWNER, Some(json!({ "sensitivity": "Public", "domain": "research", "reason": "Wider still" }))).await;
    assert_eq!(s, StatusCode::ACCEPTED, "{pending}");
    let change = pending["change_id"].as_str().unwrap().to_string();
    let (s, body) = send(&app, "POST", &format!("{uri}/changes/{change}/decide"), ADMIN, Some(json!({ "decision": "reject" }))).await;
    assert_eq!(s, StatusCode::OK, "{body}");
    assert_eq!(coordinates(&app, OWNER).await["effective"]["sensitivity"], "Directory");

    // A value that is not on the ladder, or a domain that does not exist, is refused.
    let (s, _) = send(&app, "PUT", &uri, OWNER, Some(json!({ "sensitivity": "Secret", "reason": "x" }))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    let (s, _) = send(&app, "PUT", &uri, OWNER, Some(json!({ "domain": "no-such-domain", "reason": "x" }))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    // A person with no right over the workspace may not ask.
    let (s, _) = send(&app, "PUT", &uri, "sarah.connor@state.edu", Some(json!({ "sensitivity": "Restricted", "reason": "x" }))).await;
    assert_eq!(s, StatusCode::FORBIDDEN);
}

// The domains are the HERM ones. A capability the standards file holds that the seed does not is a failure.
#[test]
fn every_herm_capability_is_a_domain_under_its_herm_domain() {
    let db = TestDb::new();
    let rows: Vec<(String, Option<String>)> = db.run(|c| c.query("SELECT key, parent_key FROM access_domains", &[]).unwrap().iter().map(|r| (r.get(0), r.get(1))).collect());
    let parent_of = |key: &str| rows.iter().find(|(k, _)| k == key).map(|(_, p)| p.clone());
    assert_eq!(parent_of("institution"), Some(None), "one root");
    assert_eq!(rows.iter().filter(|(_, p)| p.is_none()).count(), 1, "and only one");
    use scaffoldry_core::standards::herm::{HermCapability, HermDomain};
    let key_of = |d: HermDomain| match d {
        HermDomain::LearningAndTeaching => "learning-and-teaching",
        HermDomain::Research => "research",
        HermDomain::Engagement => "engagement",
        HermDomain::StrategyAndGovernance => "strategy-and-governance",
        HermDomain::EnablingCapabilities => "enabling-capabilities",
    };
    for id in ["1.2.3", "2.2.3", "5.3.1"] {
        let cap = HermCapability::resolve(id).expect("a capability the standards file holds");
        let key = format!("herm:{id}");
        assert_eq!(parent_of(&key), Some(Some(key_of(cap.domain).to_string())), "{key} sits under its HERM domain");
        assert_eq!(parent_of(key_of(cap.domain)), Some(Some("institution".to_string())), "{} sits under the root", key_of(cap.domain));
    }
}

// `allowed_affiliations` was stored and never read by a decision. It is gone, so nothing stored goes unread.
#[tokio::test]
async fn allowed_affiliations_is_gone_from_the_column_and_the_api() {
    let db = TestDb::new();
    let app = world(&db);
    let column: i64 = db.run(|c| c.query_one("SELECT count(*) FROM information_schema.columns WHERE table_name = 'workspaces' AND column_name = 'allowed_affiliations'", &[]).unwrap().get(0));
    assert_eq!(column, 0, "the column is dropped");
    let (s, body) = send(&app, "GET", &format!("/api/v1/workspaces/{WORKSPACE}"), ADMIN, None).await;
    assert_eq!(s, StatusCode::OK, "{body}");
    assert!(!body.to_string().contains("allowed_affiliations"), "and the API does not return it: {body}");
}
