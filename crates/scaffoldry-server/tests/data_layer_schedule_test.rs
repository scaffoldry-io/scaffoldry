//! Data layer phase 6b: a label change runs on a date.
//!
//! A steward approves a decided change for a date, at least the minimum notice away. The server builds a communications plan from the
//! evaluation, and nothing is sent until the steward approves it. Then each person on the plan gets a recorded notice, and a read of the
//! dataset's records returns `upcoming_changes` to the readers who will lose access. An affected person may ask for an exception during
//! the notice period. An approved one is a grant with an expiry, and the phase 3 gate honours it after the date. On the date a job applies
//! the change through the phase 6 path. The steward may withdraw the change before then. The clock is never waited for: the job takes
//! the time it should act at.

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::Router;
use http_body_util::BodyExt;
use scaffoldry_server::build_app_with_state;
use scaffoldry_server::service::identity::issue_test_token_and_user;
use scaffoldry_server::state::{CollaboratorRecord, OrganizationNode, PositionType, RoleRow, ServerState, WorkspaceRecord};
use serde_json::{json, Value};
use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::OnceLock;
use tower::ServiceExt;
use uuid::Uuid;

const WS: &str = "ws-schedule";
const OWNER: &str = "olga.owner@science.state.edu";
const ROOT: &str = "00000000-0000-0000-0000-000000000001";
const ADMIN: &str = "jordan.lee@state.edu";
const COMPLIANCE: &str = "compliance.officer@state.edu";
const CHEM_REG: &str = "chem.registrar@science.state.edu";
const PHYS_REG: &str = "phys.registrar@science.state.edu";
const SCIENCE_REG: &str = "science.registrar@science.state.edu";
const OUTSIDER: &str = "nora.editor@science.state.edu";
const STEWARD: &str = "sam.steward@science.state.edu";
const STEWARD2: &str = "sue.steward@science.state.edu";
const SECOND: &str = "second.admin@state.edu";

fn files_dir() {
    static DIR: OnceLock<PathBuf> = OnceLock::new();
    DIR.get_or_init(|| {
        let dir = std::env::temp_dir().join(format!("scaffoldry-schedule-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        std::env::set_var("SCAFFOLDRY_FILES_DIR", &dir);
        dir
    });
}

struct World {
    db: common::TestDb,
    state: std::sync::Arc<ServerState>,
    app: Router,
    admin: String,
    chem: String,
    outsider: String,
    steward: String,
    steward2: String,
    science: String,
    phys: String,
    owner: String,
    physics: Uuid,
    chemistry: Uuid,
    slug: String,
}

async fn rest(app: &Router, method: &str, uri: &str, token: &str, body: Option<Value>) -> (StatusCode, Value) {
    let b = Request::builder().method(method).uri(uri).header("authorization", format!("Bearer {token}"));
    let req = match body {
        Some(v) => b.header("content-type", "application/json").body(Body::from(v.to_string())).unwrap(),
        None => b.body(Body::empty()).unwrap(),
    };
    let resp = app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
}

fn unit(state: &ServerState, name: &str, parent: Uuid) -> Uuid {
    let id = Uuid::new_v4();
    let node = OrganizationNode { id, parent_id: Some(parent), name: name.to_string(), code: name.to_uppercase(), org_type: "Unit".to_string(), hierarchy_key: "academic".to_string(), synthetic: false };
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

fn member_of(state: &ServerState, eppn: &str, role: &str) {
    let record = CollaboratorRecord {
        id: Uuid::new_v4().to_string(),
        workspace_id: WS.to_string(),
        eppn: eppn.to_string(),
        name: eppn.to_string(),
        role: role.to_string(),
        scoped_affiliation: "faculty".to_string(),
        department: "science".to_string(),
        added_at: String::new(),
        app_slug: String::new(),
    };
    state.repository.upsert_collaborator(&record).expect("a member");
    state.collaborators.write().unwrap().entry(WS.to_string()).or_default().push(record);
}

fn workspace(state: &ServerState, unit: Uuid) {
    workspace_named(state, WS, unit);
}

fn workspace_named(state: &ServerState, id: &str, unit: Uuid) {
    let ws = WorkspaceRecord {
        id: id.to_string(),
        name: "Schedule".to_string(),
        code: "WS-SCHEDULE".to_string(),
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
    state.collaborators.write().unwrap().insert(id.to_string(), Vec::new());
}

/// `phi` is protected and needs a registrar at or above the row's unit. `open` asks nothing and is not protected.
fn categories() -> Value {
    json!([
        { "id": "phi", "name": "PHI", "protected": true, "detectors": [], "requires": [{ "position": "registrar", "reach": "unit_and_below" }] },
        { "id": "open", "name": "Open", "protected": false, "detectors": [] }
    ])
}

/// science > physics, science > chemistry. The workspace sits at science and the app's own unit is physics.
///
/// The people the evaluation counts: three registrars (chemistry, physics, science), a compliance officer, two stewards of the
/// dataset (a position held at science), and one editor with no position. That is seven. The second administrator is a Platform
/// Admin and is not counted: a Platform Admin reads every row.
async fn world() -> World {
    files_dir();
    let db = common::TestDb::new();
    let state = db.state();
    let root = Uuid::parse_str(ROOT).unwrap();
    let science = unit(&state, "science", root);
    let physics = unit(&state, "physics", science);
    let chemistry = unit(&state, "chemistry", science);
    workspace(&state, science);
    for (key, name) in [("registrar", "Registrar"), ("data_steward", "Data steward")] {
        state
            .persist_position_type(&PositionType {
                key: key.to_string(),
                name: name.to_string(),
                description: String::new(),
                org_types: vec![],
                max_holders: 10,
                retired_at: None,
                created_by: "test".to_string(),
                created_at: chrono::Utc::now().to_rfc3339(),
            })
            .unwrap();
    }
    for who in [CHEM_REG, PHYS_REG, SCIENCE_REG, OUTSIDER, STEWARD, STEWARD2] {
        member_of(&state, who, "editor");
    }
    member_of(&state, OWNER, "owner");
    appoint(&state, CHEM_REG, chemistry, "faculty", Some("registrar"));
    appoint(&state, PHYS_REG, physics, "faculty", Some("registrar"));
    appoint(&state, SCIENCE_REG, science, "faculty", Some("registrar"));
    appoint(&state, STEWARD, science, "faculty", Some("data_steward"));
    appoint(&state, STEWARD2, science, "faculty", Some("data_steward"));
    appoint(&state, COMPLIANCE, root, "compliance", None);
    appoint(&state, SECOND, root, "platform_admin", None);
    state.settings.write().unwrap().insert("sensitivity.categories".to_string(), categories());
    let app = build_app_with_state(state.clone()).expect("router");
    let slug = format!("students-{}", &Uuid::new_v4().to_string()[..8]);
    World {
        db,
        state: state.clone(),
        app,
        admin: issue_test_token_and_user(ADMIN),
        chem: issue_test_token_and_user(CHEM_REG),
        outsider: issue_test_token_and_user(OUTSIDER),
        steward: issue_test_token_and_user(STEWARD),
        steward2: issue_test_token_and_user(STEWARD2),
        science: issue_test_token_and_user(SCIENCE_REG),
        phys: issue_test_token_and_user(PHYS_REG),
        owner: issue_test_token_and_user(OWNER),
        physics,
        chemistry,
        slug,
    }
}

fn manifest(slug: &str) -> Value {
    json!({
        "slug": slug, "title": "Students", "description": "Students by unit", "organization_code": "PHYSICS", "department": "science", "workspace_id": WS, "views": [],
        "tables": [{
            "id": "students", "name": "Students", "slug": "students", "primary_field": "name",
            "fields": [
                { "name": "name", "label": "Name", "field_type": "Text", "required": false, "ferpa_sensitive": false },
                { "name": "score", "label": "Score", "field_type": "Number", "required": false, "ferpa_sensitive": false },
                { "name": "unit", "label": "Unit", "field_type": "Unit", "hierarchy": "academic", "required": false, "ferpa_sensitive": false }
            ],
            "unit_field": "unit"
        }]
    })
}

/// A live app whose dataset names `data_steward` as its steward position.
async fn live_app(w: &World) -> String {
    let (s, b) = rest(&w.app, "POST", &format!("/api/v1/workspaces/{WS}/apps"), &w.admin, Some(manifest(&w.slug))).await;
    assert_eq!(s, StatusCode::CREATED, "{b}");
    let slug = w.slug.clone();
    w.db.run(move |c| c.execute("UPDATE index_status SET state = 'ready' WHERE app_slug = $1", &[&slug]).unwrap());
    let id = dataset_id(w).await;
    let (s, b) = rest(&w.app, "PUT", &format!("/api/v1/admin/datasets/{id}/discoverability"), &w.admin, Some(json!({ "steward_position": "data_steward", "reason": "The stewards of the student data" }))).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    id
}

async fn dataset_id(w: &World) -> String {
    let (s, body) = rest(&w.app, "GET", "/api/v1/data/datasets", &w.admin, None).await;
    assert_eq!(s, StatusCode::OK, "{body}");
    body["datasets"].as_array().unwrap().iter().find(|d| d["app_slug"] == json!(w.slug) && d["table_id"] == json!("students")).expect("a dataset")["id"].as_str().unwrap().to_string()
}

async fn row(w: &World, name: &str, unit: Uuid, tags: Option<Value>) -> String {
    let mut data = json!({ "_table_id": "students", "name": name, "score": 1, "unit": unit.to_string() });
    if let Some(t) = tags {
        data["_tags"] = t;
    }
    let (s, b) = rest(&w.app, "POST", &format!("/api/v1/apps/{}/records", w.slug), &w.admin, Some(json!({ "data": data }))).await;
    assert_eq!(s, StatusCode::CREATED, "{b}");
    b["id"].as_str().unwrap().to_string()
}

async fn listed(w: &World, token: &str) -> BTreeSet<String> {
    let (s, body) = rest(&w.app, "GET", &format!("/api/v1/apps/{}/tables/students/records?limit=100", w.slug), token, None).await;
    assert_eq!(s, StatusCode::OK, "{body}");
    body["records"].as_array().unwrap().iter().map(|r| r["data"]["name"].as_str().unwrap().to_string()).collect()
}

fn set(names: &[&str]) -> BTreeSet<String> {
    names.iter().map(|n| n.to_string()).collect()
}

/// Proposes a change to tags by the route. `who` is the token of the proposer.
async fn propose_tags(w: &World, dataset: &str, who: &str, add: &[&str], remove: &[&str], records: &[&str]) -> (StatusCode, Value) {
    rest(
        &w.app,
        "POST",
        &format!("/api/v1/data/datasets/{dataset}/label-changes"),
        who,
        Some(json!({ "kind": "tags", "add": add, "remove": remove, "records": records, "reason": "These rows carry health information" })),
    )
    .await
}

async fn propose_floor(w: &World, dataset: &str, who: &str, floor: &[&str]) -> (StatusCode, Value) {
    rest(&w.app, "POST", &format!("/api/v1/data/datasets/{dataset}/label-changes"), who, Some(json!({ "kind": "floor", "floor": floor, "reason": "The dataset holds health information" }))).await
}

async fn decide(w: &World, who: &str, id: &str, body: Value) -> (StatusCode, Value) {
    rest(&w.app, "POST", &format!("/api/v1/data/label-changes/{id}/decide"), who, Some(body)).await
}

/// Everything that decides who reads what, as a fingerprint: rows with their tags, the floor, and the labels. The ledger is not in it,
/// because scheduling and approving a plan write entries and change nobody's access.
fn fingerprint(w: &World) -> String {
    w.db.run(|c| {
        let rows: String = c.query_one("SELECT coalesce(md5(string_agg(t::text, ',' ORDER BY t::text)), '') FROM (SELECT id, data, tags, version, is_ferpa_sensitive FROM dataset_records) t", &[]).unwrap().get(0);
        let floors: String = c.query_one("SELECT coalesce(md5(string_agg(label_floor::text, ',' ORDER BY id)), '') FROM datasets", &[]).unwrap().get(0);
        let labels: String = c.query_one("SELECT coalesce(md5(string_agg(t::text, ',' ORDER BY t::text)), '') FROM data_labels t", &[]).unwrap().get(0);
        format!("{rows}|{floors}|{labels}")
    })
}

fn tags_of_row(w: &World, id: &str) -> Vec<String> {
    let id = id.to_string();
    w.db.run(move |c| c.query_one("SELECT tags FROM dataset_records WHERE id = $1", &[&id]).unwrap().get(0))
}

fn at(days: i64) -> chrono::DateTime<chrono::Utc> {
    chrono::Utc::now() + chrono::Duration::days(days)
}

/// Runs the job's work with the clock at `now`, the way the job does at the time it was told.
async fn run_due(w: &World, now: chrono::DateTime<chrono::Utc>) -> usize {
    let state = w.db.state();
    tokio::task::spawn_blocking(move || scaffoldry_server::service::label_changes::schedule::apply_due(&state, now)).await.unwrap().unwrap()
}

async fn plan_approve(w: &World, who: &str, id: &str) -> (StatusCode, Value) {
    rest(&w.app, "POST", &format!("/api/v1/data/label-changes/{id}/plan/approve"), who, Some(json!({ "reason": "The plan tells the right people" }))).await
}

async fn withdraw(w: &World, who: &str, id: &str) -> (StatusCode, Value) {
    rest(&w.app, "POST", &format!("/api/v1/data/label-changes/{id}/withdraw"), who, Some(json!({ "reason": "The change is not needed" }))).await
}

struct Scene {
    change: String,
    p1: String,
    c1: String,
    effective: chrono::DateTime<chrono::Utc>,
}

/// Two rows (physics and chemistry) and a proposal to tag both `phi`. It is approved for a date 40 days away, and the plan waits.
async fn scheduled(w: &World) -> Scene {
    let dataset = live_app(w).await;
    let p1 = row(w, "Zed Secretname", w.physics, None).await;
    let c1 = row(w, "Quin Hiddenvalue", w.chemistry, None).await;
    let (s, made) = propose_tags(w, &dataset, &w.steward, &["phi"], &[], &[&p1, &c1]).await;
    assert_eq!(s, StatusCode::CREATED, "{made}");
    let change = made["id"].as_str().unwrap().to_string();
    let effective = at(40);
    let (s, b) = decide(w, &w.steward2, &change, json!({ "decision": "approve", "reason": "Notice is given before it applies", "effective_at": effective.to_rfc3339() })).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_eq!(b["status"], "Scheduled", "{b}");
    Scene { change, p1, c1, effective }
}

fn notices(w: &World, kind: &str) -> Vec<(String, String, String)> {
    let kind = kind.to_string();
    w.db.run(move |c| c.query("SELECT lower(recipient), title, body FROM notifications WHERE kind = $1 ORDER BY recipient", &[&kind]).unwrap().iter().map(|r| (r.get(0), r.get(1), r.get(2))).collect())
}

/// The people on the plan: person, whether a notice is recorded, and whether it was delivered.
fn audience(w: &World, change: &str) -> Vec<(String, Vec<String>, bool, bool)> {
    let id = Uuid::parse_str(change).unwrap();
    w.db.run(move |c| {
        c.query("SELECT person, audiences, notified_at IS NOT NULL, delivered FROM label_change_audience WHERE change_id = $1 ORDER BY person", &[&id])
            .unwrap()
            .iter()
            .map(|r| (r.get(0), r.get(1), r.get(2), r.get(3)))
            .collect()
    })
}

async fn records_body(w: &World, token: &str) -> Value {
    let (s, body) = rest(&w.app, "GET", &format!("/api/v1/apps/{}/tables/students/records?limit=100", w.slug), token, None).await;
    assert_eq!(s, StatusCode::OK, "{body}");
    body
}

async fn ask_exception(w: &World, who: &str, change: &str, expires: chrono::DateTime<chrono::Utc>) -> (StatusCode, Value) {
    rest(&w.app, "POST", &format!("/api/v1/data/label-changes/{change}/exceptions"), who, Some(json!({ "justification": "My office needs these rows to close the term", "expires_at": expires.to_rfc3339() }))).await
}

async fn decide_exception(w: &World, who: &str, change: &str, exception: &str, decision: &str) -> (StatusCode, Value) {
    rest(&w.app, "POST", &format!("/api/v1/data/label-changes/{change}/exceptions/{exception}/decide"), who, Some(json!({ "decision": decision, "reason": "Their work needs it until the term ends" }))).await
}

// 1. A scheduled change alters no one's access before its date.
#[tokio::test(flavor = "multi_thread")]
async fn a_scheduled_change_alters_no_ones_access_before_its_date() {
    let w = world().await;
    let before = {
        // The fingerprint before anything is proposed, with the two rows in place.
        let dataset = live_app(&w).await;
        row(&w, "Zed Secretname", w.physics, None).await;
        row(&w, "Quin Hiddenvalue", w.chemistry, None).await;
        let _ = dataset;
        fingerprint(&w)
    };
    let all = set(&["Zed Secretname", "Quin Hiddenvalue"]);
    assert_eq!(listed(&w, &w.outsider).await, all);

    // Scheduling it, approving its plan and running the job every day until the day before change nothing.
    let dataset = dataset_id(&w).await;
    let ids: Vec<String> = w.db.run(|c| c.query("SELECT id FROM dataset_records ORDER BY id", &[]).unwrap().iter().map(|r| r.get(0)).collect());
    let refs: Vec<&str> = ids.iter().map(String::as_str).collect();
    let (s, made) = propose_tags(&w, &dataset, &w.steward, &["phi"], &[], &refs).await;
    assert_eq!(s, StatusCode::CREATED, "{made}");
    let change = made["id"].as_str().unwrap().to_string();
    let effective = at(40);
    let (s, b) = decide(&w, &w.steward2, &change, json!({ "decision": "approve", "reason": "Notice is given before it applies", "effective_at": effective.to_rfc3339() })).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_eq!(b["status"], "Scheduled");
    assert_eq!(fingerprint(&w), before, "scheduling changed a row, a floor or a label");
    assert_eq!(listed(&w, &w.outsider).await, all);

    let (s, b) = plan_approve(&w, &w.steward2, &change).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    for day in [0, 1, 20, 39] {
        assert_eq!(run_due(&w, at(day)).await, 0, "nothing is due on day {day}");
        assert_eq!(fingerprint(&w), before, "the job changed something on day {day}");
        assert_eq!(listed(&w, &w.outsider).await, all, "the outsider reads everything on day {day}");
    }
    let (s, read) = rest(&w.app, "GET", &format!("/api/v1/data/label-changes/{change}"), &w.steward, None).await;
    assert_eq!(s, StatusCode::OK, "{read}");
    assert_eq!(read["change"]["status"], "Scheduled");
    // The evaluation now offers the date.
    let options: Vec<(String, bool)> = read["evaluation"]["options"].as_array().unwrap().iter().map(|o| (o["id"].as_str().unwrap().to_string(), o["available"].as_bool().unwrap())).collect();
    assert!(options.contains(&("date".to_string(), true)), "{options:?}");
}

// 2. On the date, a person without an exception loses access and a person with one keeps it. The gate honours the grant.
#[tokio::test(flavor = "multi_thread")]
async fn on_the_date_those_without_an_exception_lose_access_and_those_with_one_keep_it() {
    let w = world().await;
    let sc = scheduled(&w).await;
    let all = set(&["Zed Secretname", "Quin Hiddenvalue"]);

    // An exception is not possible before the plan is approved: the notice period has not begun.
    let (s, b) = ask_exception(&w, &w.outsider, &sc.change, sc.effective + chrono::Duration::days(30)).await;
    assert_eq!(s, StatusCode::CONFLICT, "{b}");
    let (s, b) = plan_approve(&w, &w.steward2, &sc.change).await;
    assert_eq!(s, StatusCode::OK, "{b}");

    // The outsider asks, with a justification and an expiry. A person the change does not affect cannot ask.
    let expires = sc.effective + chrono::Duration::days(30);
    let (s, b) = ask_exception(&w, &w.science, &sc.change, expires).await;
    assert_eq!(s, StatusCode::NOT_FOUND, "the science registrar keeps every row, so there is nothing to ask for: {b}");
    let (s, b) = ask_exception(&w, &w.outsider, &sc.change, sc.effective - chrono::Duration::days(1)).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "an exception that ends before the change begins is no exception: {b}");
    let (s, b) = rest(&w.app, "POST", &format!("/api/v1/data/label-changes/{}/exceptions", sc.change), &w.outsider, Some(json!({ "expires_at": expires.to_rfc3339() }))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "a justification is required: {b}");
    let (s, asked) = ask_exception(&w, &w.outsider, &sc.change, expires).await;
    assert_eq!(s, StatusCode::CREATED, "{asked}");
    assert_eq!(asked["status"], "Requested");
    let xid = asked["id"].as_str().unwrap().to_string();
    // Only the steward decides it.
    let (s, _) = decide_exception(&w, &w.outsider, &sc.change, &xid, "approve").await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    let (s, done) = decide_exception(&w, &w.steward, &sc.change, &xid, "approve").await;
    assert_eq!(s, StatusCode::OK, "{done}");
    assert_eq!(done["status"], "Approved");
    // It is a grant with an expiry, through the grant table.
    let grant = done["grant_id"].as_i64().expect("a grant id");
    let (holder, until, source): (String, Option<chrono::DateTime<chrono::Utc>>, String) =
        w.db.run(move |c| c.query_one("SELECT holder_id, valid_until, source FROM access_grants WHERE id = $1", &[&grant]).map(|r| (r.get(0), r.get(1), r.get(2))).unwrap());
    assert_eq!(holder, OUTSIDER);
    assert!(until.is_some_and(|u| (u - expires).num_seconds().abs() < 2), "the grant ends when the exception does: {until:?}");
    assert_eq!(source, "request");

    // The steward2's own exception stays unasked. Before the date everybody still reads everything.
    assert_eq!(run_due(&w, sc.effective - chrono::Duration::seconds(1)).await, 0);
    assert_eq!(listed(&w, &w.outsider).await, all);
    assert_eq!(listed(&w, &w.steward2).await, all);

    // On the date the change applies once, through the phase 6 path.
    assert_eq!(run_due(&w, sc.effective + chrono::Duration::seconds(1)).await, 1);
    assert_eq!(run_due(&w, sc.effective + chrono::Duration::seconds(2)).await, 0, "it applies once");
    assert_eq!(tags_of_row(&w, &sc.p1), vec!["phi".to_string()]);
    assert_eq!(tags_of_row(&w, &sc.c1), vec!["phi".to_string()]);
    assert_eq!(listed(&w, &w.steward2).await, set(&[]), "a person without an exception loses access");
    assert_eq!(listed(&w, &w.outsider).await, all, "a person with an exception keeps it");
    assert_eq!(listed(&w, &w.science).await, all, "the science registrar met the requirement all along");
    assert_eq!(listed(&w, &w.chem).await, set(&["Quin Hiddenvalue"]), "a registrar reads the rows of their own unit");
    assert_eq!(listed(&w, &w.phys).await, set(&["Zed Secretname"]));
    // The exception keeps the person's read, and nothing more: the owner has none and loses access.
    assert_eq!(listed(&w, &w.owner).await, set(&[]));

    // One DataLabelChanged entry applied it, and it names the change.
    let change = sc.change.clone();
    let applied: i64 = w.db.run(move |c| {
        c.query_one("SELECT count(*) FROM governance_ledger WHERE decision_type = 'DataLabelChanged' AND payload->>'change' = $1 AND payload->>'setting' = 'row_tags'", &[&change]).unwrap().get(0)
    });
    assert_eq!(applied, 1);
    let (s, read) = rest(&w.app, "GET", &format!("/api/v1/data/label-changes/{}", sc.change), &w.steward, None).await;
    assert_eq!(s, StatusCode::OK, "{read}");
    assert_eq!(read["change"]["status"], "Applied");

    // A grant that has run out stops the exception: the gate reads the grant's own dates.
    let grant_id = grant;
    w.db.run(move |c| c.execute("UPDATE access_grants SET valid_until = NOW() - INTERVAL '1 minute' WHERE id = $1", &[&grant_id]).unwrap());
    assert_eq!(listed(&w, &w.outsider).await, set(&[]), "the exception ended with its grant");
}

// 3. Every person on the plan has a recorded notice after the plan is approved, and none before.
#[tokio::test(flavor = "multi_thread")]
async fn every_person_on_the_plan_has_a_recorded_notice_after_approval_and_none_before() {
    let w = world().await;
    let sc = scheduled(&w).await;

    // The plan is built from the evaluation. The steward reads it. Nobody is told yet.
    let (s, read) = rest(&w.app, "GET", &format!("/api/v1/data/label-changes/{}", sc.change), &w.steward, None).await;
    assert_eq!(s, StatusCode::OK, "{read}");
    let plan = &read["schedule"]["plan"];
    let ids: Vec<&str> = plan["audiences"].as_array().unwrap().iter().map(|a| a["id"].as_str().unwrap()).collect();
    assert_eq!(ids, vec!["lose_access", "owners", "stewards", "compliance"], "{plan}");
    assert_eq!(plan["notice_days_min"], 30);
    assert!(plan["notice_days"].as_i64().unwrap() >= 40, "{plan}");
    assert_eq!(plan["timeline"][0]["action"], "first_notice");
    assert!(read["schedule"]["plan_approved_at"].is_null());
    let people = audience(&w, &sc.change);
    assert!(!people.is_empty());
    assert!(people.iter().all(|(_, _, sent, _)| !sent), "no notice is recorded before the plan is approved: {people:?}");
    assert!(notices(&w, "label_change_scheduled").is_empty(), "nothing was sent");
    // A person who is not a steward cannot approve it.
    let (s, _) = plan_approve(&w, &w.outsider, &sc.change).await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    assert!(notices(&w, "label_change_scheduled").is_empty());

    let (s, b) = plan_approve(&w, &w.steward2, &sc.change).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_eq!(b["notices"]["audience"], people.len());
    assert_eq!(b["notices"]["sent"], people.len());

    // The audience is built, not typed: who loses, the owner of the workspace, the stewards and the compliance office.
    let people = audience(&w, &sc.change);
    let named: Vec<&str> = people.iter().map(|(p, _, _, _)| p.as_str()).collect();
    for who in [OUTSIDER, CHEM_REG, PHYS_REG, STEWARD, STEWARD2, OWNER, COMPLIANCE] {
        assert!(named.contains(&who), "{who} is on the plan: {named:?}");
    }
    assert!(!named.contains(&SCIENCE_REG), "the science registrar loses nothing and is not on the plan: {named:?}");
    assert!(!named.contains(&ADMIN), "a Platform Admin reads every row and is not on the plan: {named:?}");
    let of = |p: &str| people.iter().find(|(x, _, _, _)| x == p).unwrap().1.clone();
    assert!(of(OUTSIDER).contains(&"lose_access".to_string()) && !of(OUTSIDER).contains(&"stewards".to_string()));
    assert!(of(OWNER).contains(&"owners".to_string()));
    assert!(of(STEWARD).contains(&"stewards".to_string()) && of(STEWARD).contains(&"lose_access".to_string()));
    assert!(of(COMPLIANCE).contains(&"compliance".to_string()));

    // Each has a recorded notice, and each has one in the inbox.
    assert!(people.iter().all(|(_, _, sent, delivered)| *sent && *delivered), "every person has a recorded, delivered notice: {people:?}");
    let inbox = notices(&w, "label_change_scheduled");
    for who in &named {
        assert_eq!(inbox.iter().filter(|(r, _, _)| r == who).count(), 1, "{who} has one notice in the inbox");
    }
    assert_eq!(inbox.len(), named.len());
    let lose = inbox.iter().find(|(r, _, _)| r == OUTSIDER).unwrap();
    assert!(lose.2.contains("lose") && lose.2.contains("exception"), "{lose:?}");
    let owner = inbox.iter().find(|(r, _, _)| r == OWNER).unwrap();
    assert!(owner.2.contains("Students"), "{owner:?}");

    // Approving it again does not tell anyone twice.
    let (s, b) = plan_approve(&w, &w.steward2, &sc.change).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_eq!(notices(&w, "label_change_scheduled").len(), named.len());
}

// 4. A read returns `upcoming_changes` to an affected reader and not to an unaffected one.
#[tokio::test(flavor = "multi_thread")]
async fn a_read_returns_upcoming_changes_to_an_affected_reader_and_not_to_an_unaffected_one() {
    let w = world().await;
    let sc = scheduled(&w).await;
    // Nothing is shown while the plan waits: nobody has been told.
    assert!(records_body(&w, &w.outsider).await.get("upcoming_changes").is_none());
    let (s, b) = plan_approve(&w, &w.steward2, &sc.change).await;
    assert_eq!(s, StatusCode::OK, "{b}");

    let body = records_body(&w, &w.outsider).await;
    let up = body["upcoming_changes"].as_array().expect("an upcoming_changes block");
    assert_eq!(up.len(), 1, "{body}");
    assert_eq!(up[0]["id"], json!(sc.change));
    assert_eq!(up[0]["you_lose_access"], true);
    assert!(up[0]["effective_at"].as_str().unwrap().starts_with(&sc.effective.format("%Y-%m-%d").to_string()), "{}", up[0]);
    assert_eq!(up[0]["adds"], json!(["phi"]));
    assert!(up[0]["days_left"].as_i64().unwrap() >= 39);
    assert!(up[0]["exception"].is_null());
    assert!(up[0]["what"].as_str().unwrap().contains("PHI"), "{}", up[0]);
    // The block is small and says nothing of another person or a row.
    let text = json!(up).to_string();
    assert!(text.len() < 1_200, "the block is small: {}", text.len());
    for unwanted in [STEWARD, STEWARD2, CHEM_REG, OWNER, COMPLIANCE, sc.p1.as_str(), sc.c1.as_str(), "Secretname", "Hiddenvalue"] {
        assert!(!text.contains(unwanted), "a name or row the reader may not learn is in the block: {text}");
    }
    // The records are still all there.
    assert_eq!(body["records"].as_array().unwrap().len(), 2);

    // The science registrar is not affected, and an administrator is not either.
    assert!(records_body(&w, &w.science).await.get("upcoming_changes").is_none());
    assert!(records_body(&w, &w.admin).await.get("upcoming_changes").is_none());
    // After an exception, the block says so.
    let (s, asked) = ask_exception(&w, &w.outsider, &sc.change, sc.effective + chrono::Duration::days(10)).await;
    assert_eq!(s, StatusCode::CREATED, "{asked}");
    let body = records_body(&w, &w.outsider).await;
    assert_eq!(body["upcoming_changes"][0]["exception"]["status"], "Requested", "{body}");
    // When the change is applied, there is nothing upcoming.
    run_due(&w, sc.effective + chrono::Duration::seconds(1)).await;
    assert!(records_body(&w, &w.steward2).await.get("upcoming_changes").is_none());
}

// 5. Withdrawing cancels the effective job, and the audience is told.
#[tokio::test(flavor = "multi_thread")]
async fn withdrawing_cancels_the_change_and_tells_the_audience() {
    let w = world().await;
    let sc = scheduled(&w).await;
    let before = fingerprint(&w);
    let (s, b) = plan_approve(&w, &w.steward2, &sc.change).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    let (s, asked) = ask_exception(&w, &w.outsider, &sc.change, sc.effective + chrono::Duration::days(10)).await;
    assert_eq!(s, StatusCode::CREATED, "{asked}");
    let xid = asked["id"].as_str().unwrap().to_string();
    let (s, done) = decide_exception(&w, &w.steward, &sc.change, &xid, "approve").await;
    assert_eq!(s, StatusCode::OK, "{done}");
    let grant = done["grant_id"].as_i64().unwrap();
    let told = audience(&w, &sc.change).len();

    // Only a steward withdraws it, and it needs a reason.
    let (s, _) = withdraw(&w, &w.outsider, &sc.change).await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    let (s, _) = rest(&w.app, "POST", &format!("/api/v1/data/label-changes/{}/withdraw", sc.change), &w.steward, Some(json!({}))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    let (s, b) = withdraw(&w, &w.steward, &sc.change).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_eq!(b["status"], "Withdrawn");
    assert_eq!(b["told"], told);

    // Nothing changes on the date. The job finds nothing to do.
    assert_eq!(run_due(&w, sc.effective + chrono::Duration::days(1)).await, 0);
    assert_eq!(fingerprint(&w), before);
    assert_eq!(listed(&w, &w.steward2).await, set(&["Zed Secretname", "Quin Hiddenvalue"]));
    assert!(tags_of_row(&w, &sc.p1).is_empty());
    // The audience was told, each person once, and the notice is recorded.
    let told_again = notices(&w, "label_change_withdrawn");
    assert_eq!(told_again.len(), told, "{told_again:?}");
    let recorded: i64 = {
        let id = Uuid::parse_str(&sc.change).unwrap();
        w.db.run(move |c| c.query_one("SELECT count(*) FROM label_change_audience WHERE change_id = $1 AND withdrawn_notified_at IS NOT NULL", &[&id]).unwrap().get(0))
    };
    assert_eq!(recorded as usize, told);
    // The exception granted for a change that will not happen is revoked, and a pending one is closed.
    let revoked: bool = w.db.run(move |c| c.query_one("SELECT revoked_at IS NOT NULL FROM access_grants WHERE id = $1", &[&grant]).unwrap().get(0));
    assert!(revoked);
    // It can no longer be asked for, decided, or withdrawn again. No upcoming block remains.
    let (s, _) = ask_exception(&w, &w.chem, &sc.change, sc.effective + chrono::Duration::days(10)).await;
    assert_eq!(s, StatusCode::CONFLICT);
    let (s, _) = withdraw(&w, &w.steward, &sc.change).await;
    assert_eq!(s, StatusCode::CONFLICT);
    let (s, _) = plan_approve(&w, &w.steward, &sc.change).await;
    assert_eq!(s, StatusCode::CONFLICT);
    assert!(records_body(&w, &w.outsider).await.get("upcoming_changes").is_none());
}

// 5b. A change whose plan was never approved told nobody, so withdrawing it tells nobody. It does not apply on its date either.
#[tokio::test(flavor = "multi_thread")]
async fn a_change_with_no_approved_plan_does_not_apply_and_withdrawing_it_tells_nobody() {
    let w = world().await;
    let sc = scheduled(&w).await;
    assert_eq!(run_due(&w, sc.effective + chrono::Duration::days(1)).await, 0, "no notice was given, so it does not apply");
    assert_eq!(listed(&w, &w.outsider).await, set(&["Zed Secretname", "Quin Hiddenvalue"]));
    let (s, b) = withdraw(&w, &w.steward, &sc.change).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_eq!(b["told"], 0);
    assert!(notices(&w, "label_change_withdrawn").is_empty() && notices(&w, "label_change_scheduled").is_empty());
}

// 6. A notice period shorter than the minimum is refused. Lowering protection may use a shorter minimum. A steward may lengthen it.
#[tokio::test(flavor = "multi_thread")]
async fn a_notice_period_shorter_than_the_minimum_is_refused() {
    let w = world().await;
    let dataset = live_app(&w).await;
    let p1 = row(&w, "Zed Secretname", w.physics, None).await;
    let t1 = row(&w, "Tagged", w.physics, Some(json!(["phi"]))).await;
    let (s, made) = propose_tags(&w, &dataset, &w.steward, &["phi"], &[], &[&p1]).await;
    assert_eq!(s, StatusCode::CREATED, "{made}");
    let change = made["id"].as_str().unwrap().to_string();
    let before = fingerprint(&w);
    let on = |days: i64| json!({ "decision": "approve", "reason": "Notice is given", "effective_at": at(days).to_rfc3339() });

    // phi is protected, and raising it takes 30 days by default.
    for days in [1, 5, 29] {
        let (s, b) = decide(&w, &w.steward2, &change, on(days)).await;
        assert_eq!(s, StatusCode::BAD_REQUEST, "{days} days is shorter than the minimum: {b}");
        assert!(b.to_string().contains("30"), "the refusal names the minimum: {b}");
    }
    let (s, b) = decide(&w, &w.steward2, &change, json!({ "decision": "approve", "reason": "Notice is given", "effective_at": at(-3).to_rfc3339() })).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "a date in the past: {b}");
    let (s, b) = decide(&w, &w.steward2, &change, json!({ "decision": "approve", "reason": "Notice is given", "effective_at": "next tuesday" })).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "not a date: {b}");
    let (s, b) = decide(&w, &w.steward2, &change, json!({ "decision": "approve", "reason": "Notice is given", "effective_at": at(400).to_rfc3339() })).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "more than a year ahead: {b}");
    let (s, b) = decide(&w, &w.steward2, &change, json!({ "decision": "approve", "reason": "Notice is given", "effective_at": at(40).to_rfc3339(), "open_work": "now" })).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "a date and `now` together: {b}");
    let (s, read) = rest(&w.app, "GET", &format!("/api/v1/data/label-changes/{change}"), &w.steward, None).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(read["change"]["status"], "Pending", "a refused date left the change pending");
    assert_eq!(fingerprint(&w), before);

    // The institution may set a longer minimum. The setting is validated, and the new minimum holds.
    let put = |v: Value| rest(&w.app, "PUT", "/api/v1/settings/sensitivity.notice_days", &w.admin, Some(v));
    let (s, b) = put(json!({ "protected": { "raise": 0 } })).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{b}");
    let (s, b) = put(json!({ "restricted": { "raise": 60 } })).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{b}");
    let (s, b) = put(json!({ "protected": { "raise": 60, "lower": 20 } })).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    let (s, b) = decide(&w, &w.steward2, &change, on(45)).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "45 days is under the new minimum of 60: {b}");
    assert!(b.to_string().contains("60"), "{b}");
    let (s, b) = put(json!({})).await;
    assert_eq!(s, StatusCode::OK, "{b}");

    // The steward may lengthen it: 90 days is accepted.
    let (s, b) = decide(&w, &w.steward2, &change, on(90)).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_eq!(b["status"], "Scheduled");
    assert!(b["notice_days"].as_i64().unwrap() >= 90 && b["notice_days_min"] == 30, "{b}");

    // Lowering protection takes two approvers and may use a shorter minimum: 14 days for a protected category.
    let (s, low) = propose_tags(&w, &dataset, &w.steward, &[], &["phi"], &[&t1]).await;
    assert_eq!(s, StatusCode::CREATED, "{low}");
    let low = low["id"].as_str().unwrap().to_string();
    let (s, b) = decide(&w, &w.steward2, &low, on(10)).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "10 days is under 14: {b}");
    assert!(b.to_string().contains("14"), "{b}");
    let (s, b) = decide(&w, &w.steward2, &low, on(15)).await;
    assert_eq!(s, StatusCode::OK, "the first of two approvers: {b}");
    assert_eq!(b["status"], "Pending");
    // The first approver's date is kept. The second says nothing about a date, and the change is held for the first one's date, not applied.
    let (s, kept) = rest(&w.app, "GET", &format!("/api/v1/data/label-changes/{low}"), &w.steward, None).await;
    assert_eq!(s, StatusCode::OK, "{kept}");
    assert!(kept["change"]["effective_at"].is_string(), "the date waits on the change: {kept}");
    let (s, b) = decide(&w, &w.admin, &low, json!({ "decision": "approve", "reason": "A second person agrees" })).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_eq!(b["status"], "Scheduled", "an approver who gives no date does not apply it at once: {b}");
    assert_eq!(b["notice_days_min"], 14);
    assert_eq!(b["effective_at"], kept["change"]["effective_at"]);
    assert!(tags_of_row(&w, &t1).contains(&"phi".to_string()), "nothing was removed yet");
}

// 7. No notice and no upcoming_changes block holds a record value.
#[tokio::test(flavor = "multi_thread")]
async fn no_notice_and_no_block_holds_a_record_value() {
    let w = world().await;
    let sc = scheduled(&w).await;
    let (s, b) = plan_approve(&w, &w.steward2, &sc.change).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    let (s, asked) = ask_exception(&w, &w.outsider, &sc.change, sc.effective + chrono::Duration::days(10)).await;
    assert_eq!(s, StatusCode::CREATED, "{asked}");
    let (s, b) = withdraw(&w, &w.steward, &sc.change).await;
    assert_eq!(s, StatusCode::OK, "{b}");

    let values = ["Zed", "Secretname", "Quin", "Hiddenvalue"];
    let mut everything = String::new();
    for kind in ["label_change_scheduled", "label_change_withdrawn"] {
        let found = notices(&w, kind);
        assert!(!found.is_empty(), "{kind} was sent");
        for (who, title, body) in found {
            everything.push_str(&format!("{who} {title} {body}\n"));
        }
    }
    let rows: String = w.db.run(|c| c.query("SELECT to_jsonb(n)::text FROM notifications n WHERE kind LIKE 'label_change%'", &[]).unwrap().iter().map(|r| r.get::<_, String>(0)).collect::<Vec<_>>().join("\n"));
    everything.push_str(&rows);
    let (_, read) = (0, rest(&w.app, "GET", &format!("/api/v1/data/label-changes/{}", sc.change), &w.steward, None).await.1);
    everything.push_str(&read["schedule"].to_string());
    for v in values {
        assert!(!everything.contains(v), "{v} is in a notice or the plan");
    }
    for id in [&sc.p1, &sc.c1] {
        assert!(!rows.contains(id.as_str()), "a notice names a row id");
    }
}

// 8. The notice kinds are forced into the inbox: a person cannot turn off a change to their own access.
#[tokio::test(flavor = "multi_thread")]
async fn a_person_cannot_turn_off_a_notice_of_a_change_to_their_access() {
    let w = world().await;
    let (s, b) = rest(&w.app, "PUT", "/api/v1/me/notification-prefs", &w.outsider, Some(json!({ "kind": "label_change_scheduled", "channel": "off" }))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{b}");
    let sc = scheduled(&w).await;
    let (s, b) = plan_approve(&w, &w.steward2, &sc.change).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert!(notices(&w, "label_change_scheduled").iter().any(|(r, _, _)| r == OUTSIDER));
}

// 9. A change to the floor works the same way: an exception keeps the person's read of every row, and only the added category is waived.
#[tokio::test(flavor = "multi_thread")]
async fn an_exception_to_a_floor_change_keeps_the_read_and_waives_only_what_the_floor_adds() {
    let w = world().await;
    let dataset = live_app(&w).await;
    row(&w, "Zed Secretname", w.physics, None).await;
    row(&w, "Quin Hiddenvalue", w.chemistry, None).await;
    let all = set(&["Zed Secretname", "Quin Hiddenvalue"]);
    let (s, made) = propose_floor(&w, &dataset, &w.steward, &["phi"]).await;
    assert_eq!(s, StatusCode::CREATED, "{made}");
    let change = made["id"].as_str().unwrap().to_string();
    let effective = at(40);
    let (s, b) = decide(&w, &w.steward2, &change, json!({ "decision": "approve", "reason": "Notice first", "effective_at": effective.to_rfc3339() })).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_eq!(b["plan"]["change"], json!({ "kind": "floor", "adds": ["phi"], "removes": [] }));
    let (s, b) = plan_approve(&w, &w.steward2, &change).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    let (s, asked) = ask_exception(&w, &w.outsider, &change, effective + chrono::Duration::days(20)).await;
    assert_eq!(s, StatusCode::CREATED, "{asked}");
    let (s, done) = decide_exception(&w, &w.steward, &change, asked["id"].as_str().unwrap(), "approve").await;
    assert_eq!(s, StatusCode::OK, "{done}");
    // One decision declined: the owner asks too, and the steward says no.
    let (s, asked) = ask_exception(&w, &w.owner, &change, effective + chrono::Duration::days(20)).await;
    assert_eq!(s, StatusCode::CREATED, "{asked}");
    let (s, no) = decide_exception(&w, &w.steward, &change, asked["id"].as_str().unwrap(), "decline").await;
    assert_eq!(s, StatusCode::OK, "{no}");
    assert_eq!(no["status"], "Declined");
    let (s, b) = decide_exception(&w, &w.steward, &change, asked["id"].as_str().unwrap(), "approve").await;
    assert_eq!(s, StatusCode::CONFLICT, "a decided exception is decided: {b}");
    // The same person may ask again after a refusal, and an open request lapses on the date.
    let (s, again) = ask_exception(&w, &w.owner, &change, effective + chrono::Duration::days(20)).await;
    assert_eq!(s, StatusCode::CREATED, "{again}");
    let (s, list) = rest(&w.app, "GET", &format!("/api/v1/data/label-changes/{change}/exceptions"), &w.steward, None).await;
    assert_eq!(s, StatusCode::OK, "{list}");
    assert_eq!(list["exceptions"].as_array().unwrap().len(), 3);

    assert_eq!(run_due(&w, effective + chrono::Duration::seconds(1)).await, 1);
    assert_eq!(listed(&w, &w.outsider).await, all, "the exception keeps the read");
    assert_eq!(listed(&w, &w.owner).await, set(&[]), "the request nobody decided lapsed, and the owner lost access");
    assert_eq!(listed(&w, &w.steward2).await, set(&[]));
    assert_eq!(listed(&w, &w.science).await, all);
    let (s, list) = rest(&w.app, "GET", &format!("/api/v1/data/label-changes/{change}/exceptions"), &w.steward, None).await;
    assert_eq!(s, StatusCode::OK);
    let states: Vec<&str> = list["exceptions"].as_array().unwrap().iter().map(|e| e["status"].as_str().unwrap()).collect();
    assert_eq!(states, vec!["Approved", "Declined", "Lapsed"], "{list}");
}

// 10. An exception to a change that tags named rows is refused when other rows already carry the category, because it would open them.
#[tokio::test(flavor = "multi_thread")]
async fn an_exception_that_would_open_other_rows_of_the_category_is_not_granted() {
    let w = world().await;
    let dataset = live_app(&w).await;
    let p1 = row(&w, "Zed Secretname", w.physics, None).await;
    row(&w, "Already tagged", w.physics, Some(json!(["phi"]))).await;
    let (s, made) = propose_tags(&w, &dataset, &w.steward, &["phi"], &[], &[&p1]).await;
    assert_eq!(s, StatusCode::CREATED, "{made}");
    let change = made["id"].as_str().unwrap().to_string();
    let effective = at(40);
    let (s, b) = decide(&w, &w.steward2, &change, json!({ "decision": "approve", "reason": "Notice first", "effective_at": effective.to_rfc3339() })).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    let (s, b) = plan_approve(&w, &w.steward2, &change).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    let (s, asked) = ask_exception(&w, &w.outsider, &change, effective + chrono::Duration::days(20)).await;
    assert_eq!(s, StatusCode::CREATED, "{asked}");
    let (s, b) = decide_exception(&w, &w.steward, &change, asked["id"].as_str().unwrap(), "approve").await;
    assert_eq!(s, StatusCode::CONFLICT, "{b}");
    assert!(b.to_string().contains("already carry"), "{b}");
    let grants: i64 = w.db.run(|c| c.query_one("SELECT count(*) FROM access_grants WHERE reason LIKE 'Exception to label change%'", &[]).unwrap().get(0));
    assert_eq!(grants, 0, "no grant was made");
}

// 11. An exception keeps what the person could read before the change, and nothing more. A person reads only the physics rows of one
// table, through a grant at the physics unit. The grant an exception records must open no other row and no other table.
#[tokio::test(flavor = "multi_thread")]
async fn an_exception_keeps_only_what_the_person_could_read_before() {
    const CLERK: &str = "carl.clerk@science.state.edu";
    let w = world().await;
    // The clerk holds a role (so the plan counts them) and a viewer grant at the physics unit. They are no member of the workspace.
    appoint(&w.state, CLERK, w.physics, "staff", None);
    let clerk = issue_test_token_and_user(CLERK);
    let g = json!({ "holder_kind": "person", "holder_id": CLERK, "role": "viewer", "hierarchy": "academic", "anchor_kind": "unit", "anchor_id": w.physics.to_string(), "depth": 0, "source": "request", "valid_until": (chrono::Utc::now() + chrono::Duration::days(120)).to_rfc3339(), "reason": "Reads physics" });
    let (s, pending) = rest(&w.app, "POST", "/api/v1/admin/access/grants", &w.admin, Some(g)).await;
    assert_eq!(s, StatusCode::ACCEPTED, "{pending}");
    let second = issue_test_token_and_user(SECOND);
    let (s, b) = rest(&w.app, "POST", &format!("/api/v1/admin/access/changes/{}/decide", pending["change_id"].as_str().unwrap()), &second, Some(json!({ "decision": "confirm", "reason": "Reviewed" }))).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    w.state.refresh().unwrap();
    // Two tables with a unit column each, in one app of a workspace that sits at physics.
    workspace_named(&w.state, "ws-physics-only", w.physics);
    let mut m = manifest(&w.slug);
    m["workspace_id"] = json!("ws-physics-only");
    m["tables"][0]["row_place"] = json!({ "academic": "unit" });
    let mut other = m["tables"][0].clone();
    other["id"] = json!("other");
    other["name"] = json!("Other");
    other["slug"] = json!("other");
    m["tables"].as_array_mut().unwrap().push(other);
    let (s, b) = rest(&w.app, "POST", "/api/v1/workspaces/ws-physics-only/apps", &w.admin, Some(m)).await;
    assert_eq!(s, StatusCode::CREATED, "{b}");
    let slug = w.slug.clone();
    w.db.run(move |c| c.execute("UPDATE index_status SET state = 'ready' WHERE app_slug = $1", &[&slug]).unwrap());
    let dataset = dataset_id(&w).await;
    let (s, b) = rest(&w.app, "PUT", &format!("/api/v1/admin/datasets/{dataset}/discoverability"), &w.admin, Some(json!({ "steward_position": "data_steward", "reason": "The stewards of the student data" }))).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    row(&w, "Physics person", w.physics, None).await;
    row(&w, "Chemistry person", w.chemistry, None).await;
    for (name, unit) in [("Other physics", w.physics), ("Other chemistry", w.chemistry)] {
        let data = json!({ "_table_id": "other", "name": name, "score": 1, "unit": unit.to_string() });
        let (s, b) = rest(&w.app, "POST", &format!("/api/v1/apps/{}/records", w.slug), &w.admin, Some(json!({ "data": data }))).await;
        assert_eq!(s, StatusCode::CREATED, "{b}");
    }
    let names_in = |body: &Value| -> BTreeSet<String> { body["records"].as_array().map(|r| r.iter().map(|x| x["data"]["name"].as_str().unwrap().to_string()).collect()).unwrap_or_default() };
    let read = |table: &'static str, token: String| {
        let (app, slug) = (w.app.clone(), w.slug.clone());
        async move { rest(&app, "GET", &format!("/api/v1/apps/{slug}/tables/{table}/records?limit=100"), &token, None).await }
    };
    // Before anything: the clerk reads only physics rows, in both tables.
    let (s, b) = read("students", clerk.clone()).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    let (before_t, before_u) = (names_in(&b), names_in(&read("other", clerk.clone()).await.1));
    assert_eq!(before_t, set(&["Physics person"]), "the clerk reads the physics rows of the table");
    assert!(!before_u.contains("Other chemistry"), "{before_u:?}");

    // A change raises the floor of `students` with `phi`. The clerk loses the rows, asks, and the steward approves.
    let (s, made) = rest(&w.app, "POST", &format!("/api/v1/data/datasets/{dataset}/label-changes"), &w.steward, Some(json!({ "kind": "floor", "floor": ["phi"], "reason": "The dataset holds health information" }))).await;
    assert_eq!(s, StatusCode::CREATED, "{made}");
    let change = made["id"].as_str().unwrap().to_string();
    let effective = at(40);
    let (s, b) = decide(&w, &w.steward2, &change, json!({ "decision": "approve", "reason": "Notice first", "effective_at": effective.to_rfc3339() })).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    let (s, b) = plan_approve(&w, &w.steward2, &change).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    let (s, asked) = ask_exception(&w, &clerk, &change, effective + chrono::Duration::days(20)).await;
    assert_eq!(s, StatusCode::CREATED, "{asked}");
    let (s, done) = decide_exception(&w, &w.steward, &change, asked["id"].as_str().unwrap(), "approve").await;
    assert_eq!(s, StatusCode::OK, "{done}");
    // The grant the exception records opens nothing, before or after the date.
    assert_eq!(names_in(&read("students", clerk.clone()).await.1), before_t, "the exception grant widened the table before the date");
    assert_eq!(names_in(&read("other", clerk.clone()).await.1), before_u, "the exception grant widened the other table before the date");
    assert_eq!(run_due(&w, effective + chrono::Duration::seconds(1)).await, 1);
    assert_eq!(names_in(&read("students", clerk.clone()).await.1), before_t, "the exception keeps the physics rows, and the chemistry rows stay closed");
    assert_eq!(names_in(&read("other", clerk.clone()).await.1), before_u, "the other table stays as closed as it was");
    // The single-row routes agree.
    let ids: Vec<(String, String)> = w.db.run(|c| c.query("SELECT id, data->>'name' FROM dataset_records", &[]).unwrap().iter().map(|r| (r.get(0), r.get(1))).collect());
    for (id, name) in ids {
        let (s, b) = rest(&w.app, "GET", &format!("/api/v1/apps/{}/records/{id}", w.slug), &clerk, None).await;
        if before_t.contains(&name) || before_u.contains(&name) {
            assert_eq!(s, StatusCode::OK, "{name}: {b}");
        } else {
            assert_ne!(s, StatusCode::OK, "{name} is readable by the clerk: {b}");
        }
    }
}
