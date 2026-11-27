//! Data layer phase 6: a steward reads the evaluation and decides.
//!
//! A label change is proposed by a person or by a review finding. It is stored and applies nothing. The evaluation is built on
//! request: what changes, why, who gains and loses access, what depends on it, and who would be locked out of unfinished work.
//! A steward approves, changes or declines. A removal of protection needs two approvers, and neither is the proposer. Raising
//! protection needs one steward. Nothing applies by itself.

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

const WS: &str = "ws-decision";
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
        let dir = std::env::temp_dir().join(format!("scaffoldry-decision-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        std::env::set_var("SCAFFOLDRY_FILES_DIR", &dir);
        dir
    });
}

struct World {
    db: common::TestDb,
    app: Router,
    admin: String,
    compliance: String,
    chem: String,
    outsider: String,
    steward: String,
    steward2: String,
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

fn editor_of(state: &ServerState, eppn: &str) {
    let record = CollaboratorRecord {
        id: Uuid::new_v4().to_string(),
        workspace_id: WS.to_string(),
        eppn: eppn.to_string(),
        name: eppn.to_string(),
        role: "editor".to_string(),
        scoped_affiliation: "faculty".to_string(),
        department: "science".to_string(),
        added_at: String::new(),
        app_slug: String::new(),
    };
    state.repository.upsert_collaborator(&record).expect("a member");
    state.collaborators.write().unwrap().entry(WS.to_string()).or_default().push(record);
}

fn workspace(state: &ServerState, unit: Uuid) {
    let ws = WorkspaceRecord {
        id: WS.to_string(),
        name: "Decision".to_string(),
        code: "WS-DECISION".to_string(),
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
    state.workspaces.write().unwrap().insert(WS.to_string(), ws.clone());
    state.repository.upsert_workspace(&ws).unwrap();
    state.collaborators.write().unwrap().insert(WS.to_string(), Vec::new());
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
        editor_of(&state, who);
    }
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
        app,
        admin: issue_test_token_and_user(ADMIN),
        compliance: issue_test_token_and_user(COMPLIANCE),
        chem: issue_test_token_and_user(CHEM_REG),
        outsider: issue_test_token_and_user(OUTSIDER),
        steward: issue_test_token_and_user(STEWARD),
        steward2: issue_test_token_and_user(STEWARD2),
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

async fn evaluation(w: &World, who: &str, id: &str) -> Value {
    let (s, b) = rest(&w.app, "GET", &format!("/api/v1/data/label-changes/{id}"), who, None).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    b
}

async fn decide(w: &World, who: &str, id: &str, body: Value) -> (StatusCode, Value) {
    rest(&w.app, "POST", &format!("/api/v1/data/label-changes/{id}/decide"), who, Some(body)).await
}

fn approve() -> Value {
    json!({ "decision": "approve", "reason": "I have read the evaluation" })
}

/// Everything that decides who reads what, as a fingerprint: rows with their tags, the floor, and the ledger length.
fn fingerprint(w: &World) -> String {
    w.db.run(|c| {
        let rows: String = c.query_one("SELECT coalesce(md5(string_agg(t::text, ',' ORDER BY t::text)), '') FROM (SELECT id, data, tags, version, is_ferpa_sensitive FROM dataset_records) t", &[]).unwrap().get(0);
        let floors: String = c.query_one("SELECT coalesce(md5(string_agg(label_floor::text, ',' ORDER BY id)), '') FROM datasets", &[]).unwrap().get(0);
        let ledger: i64 = c.query_one("SELECT count(*) FROM governance_ledger WHERE decision_type = 'DataLabelChanged'", &[]).unwrap().get(0);
        let labels: String = c.query_one("SELECT coalesce(md5(string_agg(t::text, ',' ORDER BY t::text)), '') FROM data_labels t", &[]).unwrap().get(0);
        format!("{rows}|{floors}|{ledger}|{labels}")
    })
}

fn tags_of_row(w: &World, id: &str) -> Vec<String> {
    let id = id.to_string();
    w.db.run(move |c| c.query_one("SELECT tags FROM dataset_records WHERE id = $1", &[&id]).unwrap().get(0))
}

/// An open task on a row: the person holds a grant on it, and a process instance of the app waits on it.
fn open_work(w: &World, person: &str, record: &str) {
    let (slug, person, record) = (w.slug.clone(), person.to_string(), record.to_string());
    w.db.run(move |c| {
        let instance = format!("review:{record}");
        c.execute(
            "INSERT INTO task_grants (instance_id, step_id, person, app_slug, table_id, record_id, read_fields, write_fields) VALUES ($1, 'check', $2, $3, 'students', $4, '{name}', '{}')",
            &[&instance, &person, &slug, &record],
        )
        .unwrap();
        let json = json!({ "id": instance, "rule_id": "review", "app_slug": slug, "record_id": record, "status": "Waiting", "waiting_step_id": "check", "log": [] });
        c.execute("INSERT INTO process_instances (id, app_slug, rule_id, status, instance_json) VALUES ($1, $2, 'review', 'Waiting', $3)", &[&instance, &slug, &json]).unwrap();
    });
}

fn close_work(w: &World) {
    w.db.run(|c| {
        c.execute("UPDATE task_grants SET state = 'closed', closed_at = NOW()", &[]).unwrap();
        c.execute("UPDATE process_instances SET status = 'Completed', instance_json = jsonb_set(instance_json, '{status}', '\"Completed\"')", &[]).unwrap();
    });
}

// 1. A proposal changes nothing until it is decided.
#[tokio::test(flavor = "multi_thread")]
async fn a_proposal_changes_nothing_until_it_is_decided() {
    let w = world().await;
    let dataset = live_app(&w).await;
    let chem = row(&w, "chem", w.chemistry, None).await;
    let tagged = row(&w, "tagged", w.physics, Some(json!(["phi"]))).await;
    assert_eq!(listed(&w, &w.outsider).await, set(&["chem"]));
    let before = fingerprint(&w);

    // A person proposes a tag and a floor, and a removal. Each is stored as waiting, and none is applied.
    let (s, tag) = propose_tags(&w, &dataset, &w.steward, &["phi"], &[], &[&chem]).await;
    assert_eq!(s, StatusCode::CREATED, "{tag}");
    assert_eq!(tag["status"], "Pending");
    let (s, floor) = propose_floor(&w, &dataset, &w.admin, &["phi"]).await;
    assert_eq!(s, StatusCode::CREATED, "{floor}");
    let (s, removal) = propose_tags(&w, &dataset, &w.steward, &[], &["phi"], &[&tagged]).await;
    assert_eq!(s, StatusCode::CREATED, "{removal}");
    assert_eq!(removal["removes_protection"], true);
    assert_eq!(fingerprint(&w), before, "a proposal changed a row, a floor, a label or the ledger");
    assert_eq!(listed(&w, &w.outsider).await, set(&["chem"]));
    assert!(tags_of_row(&w, &chem).is_empty());

    // Reading the evaluation changes nothing either.
    evaluation(&w, &w.steward, tag["id"].as_str().unwrap()).await;
    assert_eq!(fingerprint(&w), before, "the evaluation changed something");

    // A proposal that is declined changes nothing. It needs a reason, and it is closed.
    let id = floor["id"].as_str().unwrap();
    let (s, b) = decide(&w, &w.steward, id, json!({ "decision": "decline" })).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "a decision needs a reason: {b}");
    let (s, b) = decide(&w, &w.steward, id, json!({ "decision": "decline", "reason": "The floor is too wide" })).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_eq!(b["status"], "Declined");
    assert_eq!(fingerprint(&w), before, "a declined change changed something");
    let (s, _) = decide(&w, &w.steward, id, approve()).await;
    assert_eq!(s, StatusCode::CONFLICT, "a declined change cannot be approved");

    // A person who is not a steward may not propose, and a bad proposal is refused.
    let (s, _) = propose_tags(&w, &dataset, &w.outsider, &["phi"], &[], &[&chem]).await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    let (s, _) = propose_tags(&w, &dataset, &w.steward, &["nope"], &[], &[&chem]).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "an unknown category");
    let (s, _) = propose_tags(&w, &dataset, &w.steward, &["phi"], &[], &[]).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "no rows named");
    let (s, _) = propose_tags(&w, &dataset, &w.steward, &["phi"], &[], &["no-such-row"]).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "a row that is not in the dataset");
    assert_eq!(fingerprint(&w), before);
}

// 2. The evaluation lists who gains and who loses access with correct counts, and shows the lock-out list first when someone has open work.
#[tokio::test(flavor = "multi_thread")]
async fn the_evaluation_counts_who_gains_and_loses_and_puts_the_lock_out_list_first() {
    let w = world().await;
    let dataset = live_app(&w).await;
    let chem = row(&w, "chem", w.chemistry, None).await;
    let phys = row(&w, "phys", w.physics, None).await;
    open_work(&w, OUTSIDER, &chem);

    // Tagging the chemistry row phi: the chemistry and science registrars still read it. Five of the seven lose it.
    let (s, made) = propose_tags(&w, &dataset, &w.admin, &["phi"], &[], &[&chem]).await;
    assert_eq!(s, StatusCode::CREATED, "{made}");
    let ev = evaluation(&w, &w.steward, made["id"].as_str().unwrap()).await["evaluation"].clone();

    assert_eq!(ev["access"]["people_counted"], 7, "{ev}");
    assert_eq!(ev["access"]["gain"]["people"], 0);
    assert_eq!(ev["access"]["lose"]["people"], 5, "{ev}");
    let by: Vec<(Option<String>, Option<String>, i64)> = ev["access"]["lose"]["by_position"]
        .as_array()
        .unwrap()
        .iter()
        .map(|b| (b["position"].as_str().map(str::to_string), b["unit"].as_str().map(str::to_string), b["people"].as_i64().unwrap()))
        .collect();
    assert!(by.contains(&(Some("Registrar".to_string()), Some("physics".to_string()), 1)), "{by:?}");
    assert!(by.contains(&(None, None, 4)), "four people hold no position the requirement lists: {by:?}");
    assert!(!ev["access"].to_string().contains(CHEM_REG) && !ev["access"].to_string().contains(OUTSIDER), "access names no person");

    // What changes: the rows, the count, the category and a few example ids. No value.
    assert_eq!(ev["what_changes"]["rows"], 1);
    assert_eq!(ev["what_changes"]["add"], json!(["phi"]));
    assert_eq!(ev["what_changes"]["examples"], json!([chem]));
    assert!(!ev.to_string().contains("\"chem\""), "the evaluation shows no value of a row");
    // Why: the reason, in a sentence, and no finding.
    assert!(ev["why"]["sentence"].as_str().unwrap().contains("health information"), "{ev}");
    assert!(ev["why"]["finding"].is_null());
    // What depends on it: the app, the waiting process and the open task, each named.
    assert_eq!(ev["depends_on"]["apps"][0]["slug"], json!(w.slug));
    assert_eq!(ev["depends_on"]["processes"][0]["id"], json!(format!("review:{chem}")));
    assert_eq!(ev["depends_on"]["tasks"][0]["person"], OUTSIDER);
    // The lock-out list is first, and it names the person with the open work.
    assert_eq!(ev["order"][0], "locked_out", "{ev}");
    assert_eq!(ev["locked_out"]["count"], 1);
    assert_eq!(ev["locked_out"]["people"][0]["person"], OUTSIDER);
    // The options: apply now, apply for new work only, apply on a date (phase 6b).
    let options: Vec<(String, bool)> = ev["options"].as_array().unwrap().iter().map(|o| (o["id"].as_str().unwrap().to_string(), o["available"].as_bool().unwrap())).collect();
    assert_eq!(options, vec![("now".to_string(), true), ("new_work_only".to_string(), true), ("date".to_string(), true)]);
    assert!(ev["undo"]["text"].as_str().unwrap().len() > 10);

    // The physics row has no open work. Nobody is locked out, and the list is not first.
    let (s, other) = propose_tags(&w, &dataset, &w.admin, &["phi"], &[], &[&phys]).await;
    assert_eq!(s, StatusCode::CREATED, "{other}");
    let ev2 = evaluation(&w, &w.steward, other["id"].as_str().unwrap()).await["evaluation"].clone();
    assert_eq!(ev2["locked_out"]["count"], 0);
    assert_ne!(ev2["order"][0], "locked_out", "{ev2}");
    assert_eq!(ev2["access"]["lose"]["people"], 5);
    assert_eq!(ev2["access"]["gain"]["people"], 0);

    // A floor of phi on the dataset: every row is held. The chemistry registrar loses the physics row and the other way round, but
    // each registrar still reads one row, so the people who lose a row are all seven bar the science registrar.
    let (s, floor) = propose_floor(&w, &dataset, &w.admin, &["phi"]).await;
    assert_eq!(s, StatusCode::CREATED, "{floor}");
    let ev3 = evaluation(&w, &w.steward, floor["id"].as_str().unwrap()).await["evaluation"].clone();
    assert_eq!(ev3["what_changes"]["rows"], 2);
    assert_eq!(ev3["access"]["lose"]["people"], 6, "{ev3}");
    assert_eq!(ev3["access"]["gain"]["people"], 0);

    // Once phi is applied, the reverse change gains: the five who lost it gain it back.
    let (s, b) = decide(&w, &w.steward, made["id"].as_str().unwrap(), json!({ "decision": "approve", "reason": "Yes", "open_work": "now" })).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    let (s, back) = propose_tags(&w, &dataset, &w.admin, &[], &["phi"], &[&chem]).await;
    assert_eq!(s, StatusCode::CREATED, "{back}");
    let ev4 = evaluation(&w, &w.steward, back["id"].as_str().unwrap()).await["evaluation"].clone();
    assert_eq!(ev4["access"]["gain"]["people"], 5, "{ev4}");
    assert_eq!(ev4["access"]["lose"]["people"], 0);
}

// 3. Lowering protection cannot be completed by the proposer or a single approver. A second person completes it.
#[tokio::test(flavor = "multi_thread")]
async fn lowering_protection_is_completed_by_a_second_person() {
    let w = world().await;
    let dataset = live_app(&w).await;
    let tagged = row(&w, "tagged", w.chemistry, Some(json!(["phi"]))).await;
    assert_eq!(listed(&w, &w.outsider).await, set(&[]));

    let (s, made) = propose_tags(&w, &dataset, &w.steward, &[], &["phi"], &[&tagged]).await;
    assert_eq!(s, StatusCode::CREATED, "{made}");
    let id = made["id"].as_str().unwrap().to_string();
    assert_eq!(made["approvals_needed"], 2);

    // The proposer may not decide it.
    let (s, b) = decide(&w, &w.steward, &id, approve()).await;
    assert_eq!(s, StatusCode::FORBIDDEN, "{b}");
    // One approver does not complete it. The row is still protected.
    let (s, b) = decide(&w, &w.steward2, &id, approve()).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_eq!(b["status"], "Pending", "{b}");
    assert_eq!(b["approvals"], 1);
    assert_eq!(tags_of_row(&w, &tagged), vec!["phi".to_string()]);
    assert_eq!(listed(&w, &w.outsider).await, set(&[]));
    // The same approver cannot approve twice.
    let (s, b) = decide(&w, &w.steward2, &id, approve()).await;
    assert_eq!(s, StatusCode::CONFLICT, "{b}");
    assert_eq!(tags_of_row(&w, &tagged), vec!["phi".to_string()]);
    // A person who is not a steward cannot be the second.
    let (s, _) = decide(&w, &w.outsider, &id, approve()).await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    // A second person completes it.
    let (s, b) = decide(&w, &w.admin, &id, approve()).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_eq!(b["status"], "Applied", "{b}");
    assert!(tags_of_row(&w, &tagged).is_empty());
    assert_eq!(listed(&w, &w.outsider).await, set(&["tagged"]));
    // History shows the tag change, as for any write: the row was made with its tag, and the change took it off.
    let hist: Vec<Value> = w.db.run({
        let tagged = tagged.clone();
        move |c| c.query("SELECT changes FROM record_history WHERE record_id = $1 AND changes ? '_tags' ORDER BY id", &[&tagged]).unwrap().iter().map(|r| r.get(0)).collect()
    });
    assert_eq!(hist.len(), 2, "{hist:?}");
    assert_eq!(hist[1]["_tags"], json!({ "old": ["phi"], "new": [] }));

    // Lowering a floor is the same. The route that set a floor no longer lowers one.
    let (s, b) = propose_floor(&w, &dataset, &w.admin, &["phi"]).await;
    assert_eq!(s, StatusCode::CREATED, "{b}");
    let (s, b) = decide(&w, &w.steward, b["id"].as_str().unwrap(), approve()).await;
    assert_eq!(s, StatusCode::OK, "raising the floor needs one steward: {b}");
    assert_eq!(b["status"], "Applied");
    assert_eq!(listed(&w, &w.outsider).await, set(&[]));
    let (s, b) = propose_floor(&w, &dataset, &w.steward2, &[]).await;
    assert_eq!(s, StatusCode::CREATED, "{b}");
    assert_eq!(b["approvals_needed"], 2);
    let lower = b["id"].as_str().unwrap().to_string();
    let (s, b) = rest(&w.app, "PUT", &format!("/api/v1/admin/datasets/{dataset}/label-floor"), &w.admin, Some(json!({ "floor": [], "reason": "Not through here" }))).await;
    assert_eq!(s, StatusCode::CONFLICT, "the floor route does not lower a floor: {b}");
    assert_eq!(listed(&w, &w.outsider).await, set(&[]));
    for who in [&w.steward, &w.admin] {
        let (s, b) = decide(&w, who, &lower, approve()).await;
        assert_eq!(s, StatusCode::OK, "{b}");
    }
    assert_eq!(listed(&w, &w.outsider).await, set(&["tagged"]));
}

// 4. Raising needs one steward. A non-steward is refused.
#[tokio::test(flavor = "multi_thread")]
async fn raising_needs_one_steward_and_a_non_steward_is_refused() {
    let w = world().await;
    let dataset = live_app(&w).await;
    let chem = row(&w, "chem", w.chemistry, None).await;
    let (s, made) = propose_tags(&w, &dataset, &w.admin, &["phi"], &[], &[&chem]).await;
    assert_eq!(s, StatusCode::CREATED, "{made}");
    assert_eq!(made["approvals_needed"], 1);
    let id = made["id"].as_str().unwrap().to_string();

    // A person who is neither a steward, a Platform Admin nor a compliance officer is refused, for the proposal and for a decision.
    let (s, _) = decide(&w, &w.outsider, &id, approve()).await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    let (s, _) = decide(&w, &w.chem, &id, approve()).await;
    assert_eq!(s, StatusCode::FORBIDDEN, "a registrar is not the steward");
    let (s, _) = rest(&w.app, "GET", &format!("/api/v1/data/label-changes/{id}"), &w.outsider, None).await;
    assert_eq!(s, StatusCode::FORBIDDEN, "the evaluation is for the people who decide");
    assert!(tags_of_row(&w, &chem).is_empty());
    assert_eq!(listed(&w, &w.outsider).await, set(&["chem"]));

    // One steward decides it. The tag is set, the flag Cedar reads follows it, and the outsider no longer reads the row.
    let (s, b) = decide(&w, &w.steward, &id, approve()).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_eq!(b["status"], "Applied");
    assert_eq!(tags_of_row(&w, &chem), vec!["phi".to_string()]);
    assert_eq!(listed(&w, &w.outsider).await, set(&[]));
    let flagged: bool = w.db.run({
        let chem = chem.clone();
        move |c| c.query_one("SELECT is_ferpa_sensitive FROM dataset_records WHERE id = $1", &[&chem]).unwrap().get(0)
    });
    assert!(flagged, "a protected tag flags the row");
    let (s, _) = decide(&w, &w.steward, &id, approve()).await;
    assert_eq!(s, StatusCode::CONFLICT, "a change is applied once");

    // A compliance officer may decide too.
    let phys = row(&w, "phys", w.physics, None).await;
    let (s, made) = propose_tags(&w, &dataset, &w.steward, &["phi"], &[], &[&phys]).await;
    assert_eq!(s, StatusCode::CREATED, "{made}");
    let (s, b) = decide(&w, &w.compliance, made["id"].as_str().unwrap(), approve()).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_eq!(tags_of_row(&w, &phys), vec!["phi".to_string()]);
}

// 5. Approving with a non-empty lock-out list and no open-work choice is refused.
#[tokio::test(flavor = "multi_thread")]
async fn approving_with_people_locked_out_needs_a_choice_about_open_work() {
    let w = world().await;
    let dataset = live_app(&w).await;
    let chem = row(&w, "chem", w.chemistry, None).await;
    open_work(&w, OUTSIDER, &chem);
    let (s, made) = propose_tags(&w, &dataset, &w.steward, &["phi"], &[], &[&chem]).await;
    assert_eq!(s, StatusCode::CREATED, "{made}");
    let id = made["id"].as_str().unwrap().to_string();
    let before = fingerprint(&w);

    let (s, b) = decide(&w, &w.steward, &id, approve()).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "no default decides for the steward: {b}");
    let (s, b) = decide(&w, &w.steward, &id, json!({ "decision": "approve", "reason": "Yes", "open_work": "whenever" })).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{b}");
    let (s, b) = decide(&w, &w.steward, &id, json!({ "decision": "approve", "reason": "Yes", "open_work": "date" })).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "a date needs an effective_at: {b}");
    assert_eq!(fingerprint(&w), before, "a refused decision changed nothing");

    // Apply for new work only: the change is approved and held. Nothing applies while the work is open.
    let (s, b) = decide(&w, &w.steward, &id, json!({ "decision": "approve", "reason": "Let the work finish", "open_work": "new_work_only" })).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_eq!(b["status"], "Approved", "{b}");
    assert_eq!(fingerprint(&w), before, "a held change changed nothing");
    assert!(tags_of_row(&w, &chem).is_empty());
    let (s, b) = rest(&w.app, "POST", &format!("/api/v1/data/label-changes/{id}/apply"), &w.steward, None).await;
    assert_eq!(s, StatusCode::CONFLICT, "the work is still open: {b}");
    assert_eq!(fingerprint(&w), before);
    // When the work has ended it applies. Anyone else cannot apply it.
    close_work(&w);
    let (s, _) = rest(&w.app, "POST", &format!("/api/v1/data/label-changes/{id}/apply"), &w.outsider, None).await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    let (s, b) = rest(&w.app, "POST", &format!("/api/v1/data/label-changes/{id}/apply"), &w.steward, None).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_eq!(b["status"], "Applied");
    assert_eq!(tags_of_row(&w, &chem), vec!["phi".to_string()]);

    // Apply now, with the work open, is a choice the steward may make.
    let phys = row(&w, "phys", w.physics, None).await;
    open_work(&w, OUTSIDER, &phys);
    let (s, made) = propose_tags(&w, &dataset, &w.steward, &["phi"], &[], &[&phys]).await;
    assert_eq!(s, StatusCode::CREATED, "{made}");
    let (s, b) = decide(&w, &w.steward, made["id"].as_str().unwrap(), json!({ "decision": "approve", "reason": "Now", "open_work": "now" })).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_eq!(b["status"], "Applied");
    assert_eq!(b["open_work"], "now");
}

// 6. The ledger names the proposer and the decider or deciders.
#[tokio::test(flavor = "multi_thread")]
async fn the_ledger_names_the_proposer_and_the_deciders() {
    let w = world().await;
    let dataset = live_app(&w).await;
    let tagged = row(&w, "tagged", w.chemistry, Some(json!(["phi"]))).await;
    let (s, made) = propose_tags(&w, &dataset, &w.steward, &[], &["phi"], &[&tagged]).await;
    assert_eq!(s, StatusCode::CREATED, "{made}");
    let id = made["id"].as_str().unwrap().to_string();
    let before: i64 = w.db.run(|c| c.query_one("SELECT count(*) FROM governance_ledger WHERE decision_type = 'DataLabelChanged'", &[]).unwrap().get(0));
    decide(&w, &w.steward2, &id, approve()).await;
    let mid: i64 = w.db.run(|c| c.query_one("SELECT count(*) FROM governance_ledger WHERE decision_type = 'DataLabelChanged'", &[]).unwrap().get(0));
    assert_eq!(mid, before, "an approval that completes nothing writes no label entry");
    let (s, b) = decide(&w, &w.admin, &id, approve()).await;
    assert_eq!(s, StatusCode::OK, "{b}");

    let id2 = id.clone();
    let entries: Vec<(String, Value)> = w.db.run(move |c| {
        c.query("SELECT principal, payload FROM governance_ledger WHERE decision_type = 'DataLabelChanged' AND payload->>'change' = $1", &[&id2]).unwrap().iter().map(|r| (r.get(0), r.get(1))).collect()
    });
    assert_eq!(entries.len(), 1, "one entry for the change");
    let (principal, p) = &entries[0];
    assert_eq!(principal, ADMIN);
    assert_eq!(p["proposed_by"], STEWARD);
    assert_eq!(p["decided_by"], json!([STEWARD2, ADMIN]));
    assert_eq!(p["dataset"], json!(dataset));
    assert_eq!(p["setting"], "row_tags");
    assert!(p["finding"].is_null());
    assert_eq!(p["remove"], json!(["phi"]));
    let wrote: i64 = w.db.run(|c| c.query_one("SELECT count(*) FROM governance_ledger WHERE decision_type = 'DataLabelChanged'", &[]).unwrap().get(0));
    assert_eq!(wrote, before + 1);
    // The ledger holds the ids of rows only as a count and a few examples, and never a value.
    assert!(!p.to_string().contains("\"tagged\""));
}

// 7. Confirming a review finding makes a proposal.
#[tokio::test(flavor = "multi_thread")]
async fn confirming_a_review_finding_makes_a_proposal() {
    let w = world().await;
    let dataset = live_app(&w).await;
    let chem = row(&w, "chem", w.chemistry, None).await;
    let finding = Uuid::new_v4();
    {
        let (slug, chem) = (w.slug.clone(), chem.clone());
        w.db.run(move |c| {
            c.execute(
                "INSERT INTO content_findings (id, app_slug, table_id, field, detector_id, category, source, match_count, record_count, sample_record_ids) \
                 VALUES ($1, $2, 'students', 'name', 'review:phi', 'phi', 'review', 3, 1, $3)",
                &[&finding, &slug, &vec![chem]],
            )
            .unwrap();
        });
    }
    let before = fingerprint(&w);
    let (s, b) = rest(&w.app, "POST", &format!("/api/v1/admin/content-findings/{finding}/decide"), &w.compliance, Some(json!({ "decision": "confirm", "reason": "This is health information" }))).await;
    assert_eq!(s, StatusCode::OK, "confirming no longer returns 409: {b}");
    assert_eq!(b["status"], "confirmed");
    let proposal = b["change_id"].as_str().expect("the proposal it made").to_string();
    // It changed no tag, no floor and no label. The finding is decided once.
    let after = fingerprint(&w);
    let (rows_before, rows_after) = (before.split('|').next().unwrap().to_string(), after.split('|').next().unwrap().to_string());
    assert_eq!(rows_before, rows_after, "confirming a finding tagged a row");
    assert!(tags_of_row(&w, &chem).is_empty());
    let (s, _) = rest(&w.app, "POST", &format!("/api/v1/admin/content-findings/{finding}/decide"), &w.compliance, Some(json!({ "decision": "confirm", "reason": "Again" }))).await;
    assert_eq!(s, StatusCode::CONFLICT, "a finding is decided once");

    // The proposal is from the finding, and says which finding.
    let got = evaluation(&w, &w.steward, &proposal).await;
    assert_eq!(got["change"]["source"], "review");
    assert_eq!(got["change"]["proposed_by"], COMPLIANCE);
    assert_eq!(got["evaluation"]["why"]["finding"]["id"], json!(finding.to_string()));
    assert_eq!(got["evaluation"]["why"]["finding"]["category"], "phi");
    assert_eq!(got["evaluation"]["what_changes"]["add"], json!(["phi"]));
    assert_eq!(got["evaluation"]["what_changes"]["examples"], json!([chem]));
    assert_eq!(got["change"]["dataset_id"], json!(dataset));

    // A steward applies it, and the ledger names the finding.
    let (s, b) = decide(&w, &w.steward, &proposal, approve()).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_eq!(tags_of_row(&w, &chem), vec!["phi".to_string()]);
    let p2 = proposal.clone();
    let payload: Value = w.db.run(move |c| c.query_one("SELECT payload FROM governance_ledger WHERE decision_type = 'DataLabelChanged' AND payload->>'change' = $1", &[&p2]).unwrap().get(0));
    assert_eq!(payload["finding"], json!(finding.to_string()));
    assert_eq!(payload["proposed_by"], COMPLIANCE);
    assert_eq!(payload["decided_by"], json!([STEWARD]));

    // A dismissed finding still dismisses, and makes no proposal.
    let other = Uuid::new_v4();
    {
        let plain = row(&w, "plain", w.physics, None).await;
        let slug = w.slug.clone();
        w.db.run(move |c| {
            c.execute(
                "INSERT INTO content_findings (id, app_slug, table_id, field, detector_id, category, source, match_count, record_count, sample_record_ids) \
                 VALUES ($1, $2, 'students', 'score', 'review:phi', 'phi', 'review', 1, 1, $3)",
                &[&other, &slug, &vec![plain]],
            )
            .unwrap();
        });
    }
    let (s, b) = rest(&w.app, "POST", &format!("/api/v1/admin/content-findings/{other}/decide"), &w.compliance, Some(json!({ "decision": "dismiss", "reason": "Not health" }))).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    let n: i64 = w.db.run(|c| c.query_one("SELECT count(*) FROM label_changes", &[]).unwrap().get(0));
    assert_eq!(n, 1);
}

// 8. Removing a protected tag directly, outside a proposal, is still refused as before.
#[tokio::test(flavor = "multi_thread")]
async fn removing_a_protected_tag_directly_is_still_refused() {
    let w = world().await;
    let dataset = live_app(&w).await;
    let tagged = row(&w, "tagged", w.chemistry, Some(json!(["phi"]))).await;
    // The registrar who reads the row may not strip its tag by a write.
    let uri = format!("/api/v1/apps/{}/records/{tagged}", w.slug);
    let (s, b) = rest(&w.app, "PATCH", &uri, &w.chem, Some(json!({ "data": { "_table_id": "students", "name": "tagged", "unit": w.chemistry.to_string(), "_tags": [] }, "version": 1 }))).await;
    assert_eq!(s, StatusCode::FORBIDDEN, "{b}");
    assert_eq!(tags_of_row(&w, &tagged), vec!["phi".to_string()]);
    // A steward who is not an administrator may not either: the proposal is the way.
    let (s, b) = rest(&w.app, "PATCH", &uri, &w.steward, Some(json!({ "data": { "_table_id": "students", "name": "tagged", "unit": w.chemistry.to_string(), "_tags": [] }, "version": 1 }))).await;
    assert!(s == StatusCode::FORBIDDEN || s == StatusCode::NOT_FOUND, "{s} {b}");
    assert_eq!(tags_of_row(&w, &tagged), vec!["phi".to_string()]);
    // The floor route still raises, for a compliance officer, and no longer lowers for anyone.
    let (s, b) = rest(&w.app, "PUT", &format!("/api/v1/admin/datasets/{dataset}/label-floor"), &w.compliance, Some(json!({ "floor": ["phi"], "reason": "Health data" }))).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    for token in [&w.admin, &w.compliance] {
        let (s, b) = rest(&w.app, "PUT", &format!("/api/v1/admin/datasets/{dataset}/label-floor"), token, Some(json!({ "floor": [], "reason": "Lower it" }))).await;
        assert_eq!(s, StatusCode::CONFLICT, "{b}");
    }
    assert_eq!(listed(&w, &w.outsider).await, set(&[]));
}

// A steward may change the proposal before approving it. A change takes back the approvals already given.
#[tokio::test(flavor = "multi_thread")]
async fn a_steward_may_change_a_proposal_and_the_approvals_start_again() {
    let w = world().await;
    let dataset = live_app(&w).await;
    let a = row(&w, "a", w.chemistry, Some(json!(["phi"]))).await;
    let b_row = row(&w, "b", w.physics, Some(json!(["phi"]))).await;
    let (s, made) = propose_tags(&w, &dataset, &w.steward, &[], &["phi"], &[&a, &b_row]).await;
    assert_eq!(s, StatusCode::CREATED, "{made}");
    let id = made["id"].as_str().unwrap().to_string();
    let (s, b) = decide(&w, &w.steward2, &id, approve()).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_eq!(b["approvals"], 1);
    // The second steward narrows it to one row.
    let (s, b) = decide(&w, &w.steward2, &id, json!({ "decision": "change", "reason": "Only the chemistry row", "records": [a] })).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_eq!(b["status"], "Pending");
    assert_eq!(b["approvals"], 0, "{b}");
    let got = evaluation(&w, &w.steward, &id).await;
    assert_eq!(got["evaluation"]["what_changes"]["rows"], 1);
    for who in [&w.steward2, &w.admin] {
        let (s, b) = decide(&w, who, &id, approve()).await;
        assert_eq!(s, StatusCode::OK, "{b}");
    }
    assert!(tags_of_row(&w, &a).is_empty());
    assert_eq!(tags_of_row(&w, &b_row), vec!["phi".to_string()], "the row left out is still tagged");
}
