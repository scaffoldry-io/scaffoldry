//! Data layer phase 3b: audience fields and conditions.
//!
//! A dataset's policy marks fields as audience fields. A person named in one, by address, by position (`key@unit-id`) or by a unit they
//! sit in, reads the row and nobody else does. The policy says what being named grants (read, or read and write) and on which fields, so
//! the row reaches the person cut to those fields. A policy may ask a named unit to accept first: until its steward does, the value grants
//! nothing, and the acceptance is a grant in the ledger. A category's requirement still applies to a named person. A requirement may
//! name conditions, each a date on the person's record, and a lapse withdraws access when the row is read, with no job. A synced
//! dataset's audience value comes from its source, and a normal write is refused.
//!
//! Every read goes through the real list, count and single-row routes. The million-row cost is `million_rows_test.rs`.

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::Router;
use http_body_util::BodyExt;
use scaffoldry_core::access::{Op, Operand, PersonAttr, Pred, RowOp, RowRule};
use scaffoldry_server::build_app_with_state;
use scaffoldry_server::service::identity::issue_test_token_and_user;
use scaffoldry_server::service::scope::{self, set_rules};
use scaffoldry_server::state::{AuthUser, CollaboratorRecord, DatasetRecord, OrganizationNode, PositionType, RoleRow, ServerState, WorkspaceRecord};
use serde_json::{json, Value};
use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::OnceLock;
use tower::ServiceExt;
use uuid::Uuid;

const WS: &str = "ws-audience";
const ROOT: &str = "00000000-0000-0000-0000-000000000001";
const ADMIN: &str = "jordan.lee@state.edu";
const OWNER: &str = "olga.owner@science.state.edu";
/// A steward of the dataset, at science. Walking up from physics finds this person.
const STEWARD: &str = "sam.steward@science.state.edu";
/// An editor the row rule narrows to the rows they advise, who sits in physics and holds `mentor` there.
const READER: &str = "nora.reader@physics.state.edu";
/// An editor who is a registrar at science, so meets the `phi` requirement.
const REGISTRAR: &str = "rex.registrar@physics.state.edu";
/// An editor who is named nowhere.
const STRANGER: &str = "sid.stranger@physics.state.edu";
const OTHER: &str = "someone.else@science.state.edu";

fn files_dir() {
    static DIR: OnceLock<PathBuf> = OnceLock::new();
    DIR.get_or_init(|| {
        let dir = std::env::temp_dir().join(format!("scaffoldry-audience-{}", Uuid::new_v4()));
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
    owner: String,
    steward: String,
    reader: String,
    registrar: String,
    stranger: String,
    science: Uuid,
    physics: Uuid,
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

fn appoint(state: &ServerState, eppn: &str, unit: Uuid, position: Option<&str>) {
    state
        .persist_role(&RoleRow {
            id: Uuid::new_v4(),
            person_id: Uuid::new_v4(),
            eppn: eppn.to_string(),
            organization_id: unit,
            role_title: "staff".to_string(),
            scoped_affiliation: "faculty".to_string(),
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
    let ws = WorkspaceRecord {
        id: WS.to_string(),
        name: "Audience".to_string(),
        code: "WS-AUDIENCE".to_string(),
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

/// `phi` needs a registrar at or above the row's unit. `ferpa_ok` needs the condition `ferpa_training` and no position.
fn categories() -> Value {
    json!([
        { "id": "phi", "name": "PHI", "protected": true, "detectors": [], "requires": [{ "position": "registrar", "reach": "unit_and_below" }] },
        { "id": "ferpa_ok", "name": "Needs FERPA training", "protected": true, "detectors": [], "conditions": ["ferpa_training"] },
        { "id": "open", "name": "Open", "protected": false, "detectors": [] }
    ])
}

async fn world() -> World {
    files_dir();
    let db = common::TestDb::new();
    let state = db.state();
    let root = Uuid::parse_str(ROOT).unwrap();
    let science = unit(&state, "science", root);
    let physics = unit(&state, "physics", science);
    workspace(&state, science);
    for (key, name) in [("registrar", "Registrar"), ("data_steward", "Data steward"), ("mentor", "Mentor")] {
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
    member_of(&state, STEWARD, "admin");
    appoint(&state, STEWARD, science, Some("data_steward"));
    member_of(&state, OWNER, "owner");
    for who in [READER, REGISTRAR, STRANGER] {
        member_of(&state, who, "editor");
    }
    appoint(&state, READER, physics, Some("mentor"));
    appoint(&state, REGISTRAR, science, Some("registrar"));
    appoint(&state, STRANGER, science, None);
    state.settings.write().unwrap().insert("sensitivity.categories".to_string(), categories());
    state.settings.write().unwrap().insert("sensitivity.conditions".to_string(), json!([{ "id": "ferpa_training", "name": "FERPA training", "owner": "registrar" }]));
    let app = build_app_with_state(state.clone()).expect("router");
    let slug = format!("students-{}", &Uuid::new_v4().to_string()[..8]);
    World {
        db,
        state: state.clone(),
        app,
        admin: issue_test_token_and_user(ADMIN),
        owner: issue_test_token_and_user(OWNER),
        steward: issue_test_token_and_user(STEWARD),
        reader: issue_test_token_and_user(READER),
        registrar: issue_test_token_and_user(REGISTRAR),
        stranger: issue_test_token_and_user(STRANGER),
        science,
        physics,
        slug,
    }
}

fn text(n: &str, indexed: bool) -> Value {
    json!({ "name": n, "label": n, "field_type": "Text", "required": false, "ferpa_sensitive": false, "indexed": indexed })
}

fn audience() -> Value {
    json!([
        { "field": "reviewer", "grants": "read", "fields": ["name", "score"] },
        { "field": "coach", "grants": "write", "fields": ["name"] },
        { "field": "partner", "grants": "read", "fields": ["name"], "unit_acceptance": true }
    ])
}

fn manifest(slug: &str) -> Value {
    json!({
        "slug": slug, "title": "Students", "description": "Students by unit", "organization_code": "PHYSICS", "department": "science", "workspace_id": WS, "views": [],
        "tables": [
            {
                "id": "students", "name": "Students", "slug": "students", "primary_field": "name",
                "fields": [
                    text("name", false),
                    { "name": "score", "label": "Score", "field_type": "Number", "required": false, "ferpa_sensitive": false },
                    text("advisor", true),
                    text("reviewer", false),
                    text("coach", false),
                    { "name": "partner", "label": "Partner", "field_type": "Unit", "hierarchy": "academic", "required": false, "ferpa_sensitive": false },
                    { "name": "unit", "label": "Unit", "field_type": "Unit", "hierarchy": "academic", "required": false, "ferpa_sensitive": false }
                ],
                "unit_field": "unit",
                "audience": audience()
            },
            { "id": "notes", "name": "Notes", "slug": "notes", "primary_field": "name", "fields": [text("name", false), text("advisor", true)] }
        ]
    })
}

fn rule(table: &str) -> RowRule {
    RowRule {
        id: format!("advisees-{table}"),
        table_id: table.to_string(),
        ops: vec![RowOp::Read, RowOp::Create, RowOp::Update, RowOp::Delete],
        visible_when: Pred::Cmp { field: "advisor".into(), op: Op::Eq, value: Operand::Person(PersonAttr::Address) },
        except_roles: vec!["owner".to_string(), "admin".to_string()],
        message: None,
    }
}

/// A live app whose dataset names `data_steward` as its steward position, with a row rule on each table. Returns the dataset id.
async fn live_app(w: &World) -> String {
    let (s, b) = rest(&w.app, "POST", &format!("/api/v1/workspaces/{WS}/apps"), &w.admin, Some(manifest(&w.slug))).await;
    assert_eq!(s, StatusCode::CREATED, "the manifest with an audience policy is accepted: {b}");
    let slug = w.slug.clone();
    w.db.run(move |c| c.execute("UPDATE index_status SET state = 'ready' WHERE app_slug = $1", &[&slug]).unwrap());
    set_rules(&w.slug, vec![rule("students"), rule("notes")], &w.state);
    let id = dataset_id(w, "students").await;
    let (s, b) = rest(&w.app, "PUT", &format!("/api/v1/admin/datasets/{id}/discoverability"), &w.admin, Some(json!({ "steward_position": "data_steward", "reason": "The stewards of the student data" }))).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    id
}

async fn dataset_id(w: &World, table: &str) -> String {
    let (s, body) = rest(&w.app, "GET", "/api/v1/data/datasets", &w.admin, None).await;
    assert_eq!(s, StatusCode::OK, "{body}");
    body["datasets"].as_array().unwrap().iter().find(|d| d["app_slug"] == json!(w.slug) && d["table_id"] == json!(table)).expect("a dataset")["id"].as_str().unwrap().to_string()
}

/// A student row made by the Platform Admin. `extra` is merged into the data.
async fn student(w: &World, name: &str, extra: Value) -> String {
    let mut data = json!({ "_table_id": "students", "name": name, "score": 7, "advisor": OTHER, "unit": w.physics.to_string() });
    for (k, v) in extra.as_object().unwrap() {
        data[k] = v.clone();
    }
    let (s, b) = rest(&w.app, "POST", &format!("/api/v1/apps/{}/records", w.slug), &w.admin, Some(json!({ "data": data }))).await;
    assert_eq!(s, StatusCode::CREATED, "{b}");
    b["id"].as_str().unwrap().to_string()
}

async fn note(w: &World, name: &str, advisor: &str) -> String {
    let data = json!({ "_table_id": "notes", "name": name, "advisor": advisor });
    let (s, b) = rest(&w.app, "POST", &format!("/api/v1/apps/{}/records", w.slug), &w.admin, Some(json!({ "data": data }))).await;
    assert_eq!(s, StatusCode::CREATED, "{b}");
    b["id"].as_str().unwrap().to_string()
}

async fn names(w: &World, token: &str, table: &str) -> BTreeSet<String> {
    let (s, body) = rest(&w.app, "GET", &format!("/api/v1/apps/{}/tables/{table}/records?limit=100", w.slug), token, None).await;
    assert_eq!(s, StatusCode::OK, "{body}");
    body["records"].as_array().unwrap().iter().map(|r| r["data"]["name"].as_str().unwrap_or("").to_string()).collect()
}

async fn count(w: &World, token: &str, table: &str) -> f64 {
    let (s, body) = rest(&w.app, "GET", &format!("/api/v1/apps/{}/tables/{table}/aggregate?function=count", w.slug), token, None).await;
    assert_eq!(s, StatusCode::OK, "{body}");
    body["value"].as_f64().unwrap_or(0.0)
}

async fn one(w: &World, token: &str, id: &str) -> (StatusCode, Value) {
    rest(&w.app, "GET", &format!("/api/v1/apps/{}/tables/students/records/{id}", w.slug), token, None).await
}

fn set(names: &[&str]) -> BTreeSet<String> {
    names.iter().map(|n| n.to_string()).collect()
}

/// The fields of a record's data that a person made, not the server's own.
fn shown(record: &Value) -> BTreeSet<String> {
    record["data"].as_object().unwrap().keys().filter(|k| !k.starts_with('_')).cloned().collect()
}

fn user(eppn: &str) -> AuthUser {
    AuthUser { eppn: eppn.to_string(), name: eppn.to_string(), role_title: String::new(), department: "science".to_string(), affiliation: "faculty".to_string(), view_as_by: None, via_client: None, shared_link: None }
}

fn ledger_entries(w: &World, decision: &str, source: &str, op: &str) -> i64 {
    let (decision, source, op) = (decision.to_string(), source.to_string(), op.to_string());
    w.db.run(move |c| {
        c.query_one("SELECT count(*) FROM governance_ledger WHERE decision_type = $1 AND payload->>'source' = $2 AND payload->>'op' = $3", &[&decision, &source, &op]).unwrap().get(0)
    })
}

// ---------------------------------------------------------------------------------------------------------------------
// 1. A person named in an audience field reads the row, through the list, the count and the single read. A person not named does not.
// ---------------------------------------------------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn a_person_named_in_an_audience_field_reads_the_row_and_nobody_else_does() {
    let w = world().await;
    live_app(&w).await;
    let by_address = student(&w, "Alpha", json!({ "reviewer": "Nora.Reader@Physics.State.EDU" })).await;
    let other = student(&w, "Bravo", json!({ "reviewer": OTHER })).await;
    let by_position = student(&w, "Charlie", json!({ "reviewer": format!("mentor@{}", w.physics) })).await;
    let wrong_unit = student(&w, "Delta", json!({ "reviewer": format!("mentor@{}", w.science) })).await;
    let unnamed = student(&w, "Echo", json!({})).await;
    let own = student(&w, "Foxtrot", json!({ "advisor": READER })).await;
    note(&w, "Note Hidden", OTHER).await;
    note(&w, "Note Mine", READER).await;

    // The named person reads the rows that name them, by address (in any case) and by the position they hold at that unit, and their own.
    assert_eq!(names(&w, &w.reader, "students").await, set(&["Alpha", "Charlie", "Foxtrot"]));
    assert_eq!(count(&w, &w.reader, "students").await, 3.0);
    for (id, name) in [(&by_address, "Alpha"), (&by_position, "Charlie"), (&own, "Foxtrot")] {
        let (s, row) = one(&w, &w.reader, id).await;
        assert_eq!(s, StatusCode::OK, "{name}: {row}");
        assert_eq!(row["data"]["name"], json!(name));
    }
    // A row that names someone else, a position at a unit they do not hold it at, or no one is not theirs.
    for id in [&other, &wrong_unit, &unnamed] {
        let (s, _) = one(&w, &w.reader, id).await;
        assert_eq!(s, StatusCode::NOT_FOUND, "a row that does not name the reader is not found");
    }
    // The audience grants nothing on a table with no policy.
    assert_eq!(names(&w, &w.reader, "notes").await, set(&["Note Mine"]));

    // A person who is named nowhere reads nothing, and the owner's own rights are what they were.
    assert_eq!(names(&w, &w.stranger, "students").await, BTreeSet::new());
    assert_eq!(count(&w, &w.stranger, "students").await, 0.0);
    let (s, _) = one(&w, &w.stranger, &by_address).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    assert_eq!(count(&w, &w.owner, "students").await, 6.0, "an owner reads every row, as before");

    // Taking the name away takes the access away in the same write.
    let (s, row) = one(&w, &w.admin, &by_address).await;
    assert_eq!(s, StatusCode::OK, "{row}");
    let (s, b) = rest(&w.app, "PATCH", &format!("/api/v1/apps/{}/records/{by_address}", w.slug), &w.admin, Some(json!({ "data": { "reviewer": OTHER }, "version": row["version"] }))).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_eq!(names(&w, &w.reader, "students").await, set(&["Charlie", "Foxtrot"]));
    assert_eq!(count(&w, &w.reader, "students").await, 2.0);
    let (s, _) = one(&w, &w.reader, &by_address).await;
    assert_eq!(s, StatusCode::NOT_FOUND);

    // A row in trash is read by nobody, and comes back with its names.
    let (s, _) = rest(&w.app, "DELETE", &format!("/api/v1/apps/{}/records/{by_position}", w.slug), &w.admin, None).await;
    assert!(s.is_success(), "{s}");
    assert_eq!(names(&w, &w.reader, "students").await, set(&["Foxtrot"]));
}

// ---------------------------------------------------------------------------------------------------------------------
// 2. Naming grants only the policy's fields, and only read unless the policy says write.
// ---------------------------------------------------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn naming_grants_only_the_fields_of_the_policy_and_only_read_unless_it_says_write() {
    let w = world().await;
    live_app(&w).await;
    let read_only = student(&w, "Alpha", json!({ "reviewer": READER })).await;
    let writable = student(&w, "Golf", json!({ "coach": READER })).await;

    // Read: the fields the policy lists, in the single read and in the list.
    let (s, row) = one(&w, &w.reader, &read_only).await;
    assert_eq!(s, StatusCode::OK, "{row}");
    assert_eq!(shown(&row), set(&["name", "score"]), "{row}");
    let (_, list) = rest(&w.app, "GET", &format!("/api/v1/apps/{}/tables/students/records?limit=100", w.slug), &w.reader, None).await;
    let found = list["records"].as_array().unwrap().iter().find(|r| r["id"] == json!(read_only)).expect("the named row is listed");
    assert_eq!(shown(found), set(&["name", "score"]), "the list cuts the row the same way: {found}");
    let (_, row) = one(&w, &w.reader, &writable).await;
    assert_eq!(shown(&row), set(&["name"]), "the coach policy lists one field: {row}");

    // A read-only policy writes nothing.
    let (s, row) = one(&w, &w.admin, &read_only).await;
    assert_eq!(s, StatusCode::OK);
    async fn patch(w: &World, id: &str, data: Value, version: &Value) -> (StatusCode, Value) {
        rest(&w.app, "PATCH", &format!("/api/v1/apps/{}/records/{id}", w.slug), &w.reader, Some(json!({ "data": data, "version": version }))).await
    }
    let (s, b) = patch(&w, &read_only, json!({ "name": "Changed" }), &row["version"]).await;
    assert!(!s.is_success(), "a read policy lets nobody write: {s} {b}");
    let (_, after) = one(&w, &w.admin, &read_only).await;
    assert_eq!(after["data"]["name"], json!("Alpha"));

    // A write policy writes its fields and no others.
    let (_, row) = one(&w, &w.admin, &writable).await;
    let (s, b) = patch(&w, &writable, json!({ "score": 99 }), &row["version"]).await;
    assert_eq!(s, StatusCode::FORBIDDEN, "a field the policy does not list is refused: {b}");
    let (s, b) = patch(&w, &writable, json!({ "name": "Golf Two" }), &row["version"]).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    let (_, after) = one(&w, &w.admin, &writable).await;
    assert_eq!((after["data"]["name"].clone(), after["data"]["score"].clone()), (json!("Golf Two"), json!(7)), "the name changed and the score did not");
    // Naming gives no right to delete, and none to change who is named.
    let (s, _) = rest(&w.app, "DELETE", &format!("/api/v1/apps/{}/records/{writable}", w.slug), &w.reader, None).await;
    assert!(!s.is_success(), "naming does not let a person delete the row");
    let (_, row) = one(&w, &w.admin, &writable).await;
    let (s, _) = patch(&w, &writable, json!({ "coach": OTHER }), &row["version"]).await;
    assert_eq!(s, StatusCode::FORBIDDEN, "the field that names the audience is not in the policy's list");
}

// ---------------------------------------------------------------------------------------------------------------------
// 3. An unaccepted unit value grants nothing. After acceptance the unit's people read it. The acceptance is in the ledger.
// ---------------------------------------------------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn an_unaccepted_unit_grants_nothing_and_the_acceptance_is_a_grant_in_the_ledger() {
    let w = world().await;
    let ds = live_app(&w).await;
    let hotel = student(&w, "Hotel", json!({ "partner": w.physics.to_string() })).await;
    let accept = format!("/api/v1/data/datasets/{ds}/records/{hotel}/audience/accept");

    // Until the receiving unit's steward accepts, the value is a proposal.
    assert_eq!(names(&w, &w.reader, "students").await, BTreeSet::new());
    assert_eq!(count(&w, &w.reader, "students").await, 0.0);
    let (s, _) = one(&w, &w.reader, &hotel).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    let (s, waiting) = rest(&w.app, "GET", &format!("/api/v1/data/datasets/{ds}/audience/pending"), &w.steward, None).await;
    assert_eq!(s, StatusCode::OK, "{waiting}");
    assert_eq!(waiting["pending"].as_array().map(Vec::len), Some(1), "the steward sees what waits: {waiting}");
    assert_eq!(ledger_entries(&w, "AccessGrantChanged", "audience", "create"), 0);

    // Someone who is not the unit's steward cannot accept. A unit that was not named cannot be accepted.
    let (s, b) = rest(&w.app, "POST", &accept, &w.reader, Some(json!({ "unit": w.physics.to_string(), "reason": "I would like to read it" }))).await;
    assert_eq!(s, StatusCode::FORBIDDEN, "{b}");
    let (s, b) = rest(&w.app, "POST", &accept, &w.steward, Some(json!({ "unit": w.science.to_string(), "reason": "Not the named unit" }))).await;
    assert_eq!(s, StatusCode::NOT_FOUND, "{b}");
    assert_eq!(count(&w, &w.reader, "students").await, 0.0, "a refusal accepts nothing");

    // The steward accepts. The unit's people read the row, cut to the policy's fields.
    let (s, b) = rest(&w.app, "POST", &accept, &w.steward, Some(json!({ "unit": w.physics.to_string(), "reason": "Physics advises these students" }))).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_eq!(names(&w, &w.reader, "students").await, set(&["Hotel"]));
    assert_eq!(count(&w, &w.reader, "students").await, 1.0);
    let (s, row) = one(&w, &w.reader, &hotel).await;
    assert_eq!(s, StatusCode::OK, "{row}");
    assert_eq!(shown(&row), set(&["name"]));
    assert_eq!(count(&w, &w.stranger, "students").await, 0.0, "a person outside the unit still reads nothing");
    assert_eq!(count(&w, &w.registrar, "students").await, 0.0, "and so does a person in a unit above it");
    // The acceptance is recorded as a grant, with who decided it. It is not a new decision type.
    assert_eq!(ledger_entries(&w, "AccessGrantChanged", "audience", "create"), 1);
    let entry: Value = w.db.run(|c| c.query_one("SELECT payload FROM governance_ledger WHERE decision_type = 'AccessGrantChanged' AND payload->>'source' = 'audience'", &[]).unwrap().get(0));
    assert_eq!(entry["holder"], json!(w.physics.to_string()), "{entry}");
    assert_eq!(entry["record"], json!(hotel), "{entry}");
    assert_eq!(entry["accepted_by"], json!(STEWARD), "{entry}");
    // Accepting twice is a conflict, and writes no second entry.
    let (s, _) = rest(&w.app, "POST", &accept, &w.steward, Some(json!({ "unit": w.physics.to_string(), "reason": "Again" }))).await;
    assert_eq!(s, StatusCode::CONFLICT);
    assert_eq!(ledger_entries(&w, "AccessGrantChanged", "audience", "create"), 1);

    // A later save that leaves the unit named keeps the acceptance. A save that names another unit starts again.
    let (_, row) = one(&w, &w.admin, &hotel).await;
    // A save by an admin names the whole row.
    let whole = |name: &str, partner: Uuid| json!({ "_table_id": "students", "name": name, "score": 7, "advisor": OTHER, "unit": w.physics.to_string(), "partner": partner.to_string() });
    let (s, b) = rest(&w.app, "PATCH", &format!("/api/v1/apps/{}/records/{hotel}", w.slug), &w.admin, Some(json!({ "data": whole("Hotel Two", w.physics), "version": row["version"] }))).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_eq!(names(&w, &w.reader, "students").await, set(&["Hotel Two"]));
    let (_, row) = one(&w, &w.admin, &hotel).await;
    let (s, b) = rest(&w.app, "PATCH", &format!("/api/v1/apps/{}/records/{hotel}", w.slug), &w.admin, Some(json!({ "data": whole("Hotel Two", w.science), "version": row["version"] }))).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_eq!(count(&w, &w.reader, "students").await, 0.0, "a unit that was not accepted grants nothing");
}

// ---------------------------------------------------------------------------------------------------------------------
// 4. A category's requirement still applies to a named person who does not meet it.
// ---------------------------------------------------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn a_category_requirement_still_applies_to_a_named_person() {
    let w = world().await;
    live_app(&w).await;
    let both = format!("{READER}, {REGISTRAR}");
    let phi = student(&w, "India", json!({ "reviewer": both, "_tags": ["phi"] })).await;
    let plain = student(&w, "Juno", json!({ "reviewer": both })).await;

    // The reader is named on both rows and is no registrar: the row with the requirement is not theirs. The registrar meets it.
    assert_eq!(names(&w, &w.reader, "students").await, set(&["Juno"]));
    assert_eq!(count(&w, &w.reader, "students").await, 1.0);
    let (s, _) = one(&w, &w.reader, &phi).await;
    assert_eq!(s, StatusCode::NOT_FOUND, "naming someone never bypasses the requirement");
    let (s, _) = one(&w, &w.reader, &plain).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(names(&w, &w.registrar, "students").await, set(&["India", "Juno"]));
    assert_eq!(count(&w, &w.registrar, "students").await, 2.0);
    let (s, _) = one(&w, &w.registrar, &phi).await;
    assert_eq!(s, StatusCode::OK);
    // And a registrar who is not named reads neither.
    let other_registrar = student(&w, "Kilo", json!({ "reviewer": OTHER, "_tags": ["phi"] })).await;
    let (s, _) = one(&w, &w.registrar, &other_registrar).await;
    assert_eq!(s, StatusCode::NOT_FOUND, "meeting the requirement is not being named");
}

// ---------------------------------------------------------------------------------------------------------------------
// 5. A lapsed condition withdraws access at read time, with no job.
// ---------------------------------------------------------------------------------------------------------------------

fn day(offset: i64) -> String {
    (chrono::Utc::now().date_naive() + chrono::Duration::days(offset)).format("%Y-%m-%d").to_string()
}

#[tokio::test(flavor = "multi_thread")]
async fn a_lapsed_condition_withdraws_access_when_the_row_is_read() {
    let w = world().await;
    live_app(&w).await;
    let row = student(&w, "Lima", json!({ "reviewer": READER, "_tags": ["ferpa_ok"] })).await;
    let condition = format!("/api/v1/admin/people/{READER}/conditions/ferpa_training");

    // No date on the person's record: the condition is not held.
    assert_eq!(names(&w, &w.reader, "students").await, BTreeSet::new());
    let (s, _) = one(&w, &w.reader, &row).await;
    assert_eq!(s, StatusCode::NOT_FOUND);

    // An admin sets it, with an owner and a source. Only an admin does.
    let body = |until: Value| json!({ "until": until, "owner": "registrar", "source": "manual", "reason": "The registrar confirmed the training" });
    let (s, _) = rest(&w.app, "PUT", &condition, &w.reader, Some(body(json!(day(30))))).await;
    assert_eq!(s, StatusCode::FORBIDDEN, "a person does not set their own condition");
    let (s, b) = rest(&w.app, "PUT", &condition, &w.admin, Some(body(json!(day(30))))).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_eq!(b["conditions"]["ferpa_training"], json!(day(30)), "{b}");
    assert_eq!(names(&w, &w.reader, "students").await, set(&["Lima"]));
    assert_eq!(count(&w, &w.reader, "students").await, 1.0);
    let (s, one_row) = one(&w, &w.reader, &row).await;
    assert_eq!(s, StatusCode::OK, "{one_row}");
    // Holding the condition is not being named.
    assert_eq!(count(&w, &w.stranger, "students").await, 0.0);

    // The date passes. Nothing runs: the next read compares the date and withdraws the access.
    let (s, b) = rest(&w.app, "PUT", &condition, &w.admin, Some(body(json!(day(-1))))).await;
    assert_eq!(s, StatusCode::OK, "a lapsed date is recorded as it is: {b}");
    assert_eq!(names(&w, &w.reader, "students").await, BTreeSet::new());
    assert_eq!(count(&w, &w.reader, "students").await, 0.0);
    let (s, _) = one(&w, &w.reader, &row).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    // The last day still counts. Clearing the date withdraws it too.
    rest(&w.app, "PUT", &condition, &w.admin, Some(body(json!(day(0))))).await;
    assert_eq!(count(&w, &w.reader, "students").await, 1.0, "the date is the last day the condition holds");
    let (s, b) = rest(&w.app, "PUT", &condition, &w.admin, Some(body(Value::Null))).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_eq!(count(&w, &w.reader, "students").await, 0.0);

    // Every change is in the ledger, with who made it, the owner and the source. A condition that is not on the closed list, a bad
    // date, and a person who is not in the directory are refused.
    assert_eq!(ledger_entries(&w, "UserAccessChanged", "condition", "set"), 3);
    assert_eq!(ledger_entries(&w, "UserAccessChanged", "condition", "clear"), 1);
    let (s, _) = rest(&w.app, "PUT", &format!("/api/v1/admin/people/{READER}/conditions/not_listed"), &w.admin, Some(body(json!(day(5))))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    let (s, _) = rest(&w.app, "PUT", &condition, &w.admin, Some(body(json!("next spring")))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    let (s, _) = rest(&w.app, "PUT", "/api/v1/admin/people/nobody@nowhere.example/conditions/ferpa_training", &w.admin, Some(body(json!(day(5))))).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    // The settings take only conditions on the list.
    let (s, _) = rest(&w.app, "PUT", "/api/v1/settings/sensitivity.categories", &w.admin, Some(json!([{ "id": "x", "name": "X", "protected": true, "detectors": [], "conditions": ["not_listed"] }]))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "a category names only conditions that settings list");
}

// ---------------------------------------------------------------------------------------------------------------------
// 6. A table with no audience fields runs the statement it ran before. The test asserts on the built clause.
// ---------------------------------------------------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn a_reader_of_a_table_with_no_audience_runs_the_same_statement() {
    let w = world().await;
    live_app(&w).await;
    student(&w, "Alpha", json!({ "reviewer": READER })).await;
    let reader = user(READER);
    let clause = |plan: &scaffoldry_server::repository::ScopePlan| plan.clause_at("r.id", &w.slug, "notes", 1);

    let before = scope::plan_of(&reader, &w.slug, "notes", &w.state).expect("the row rule narrows");
    for needs in [vec![], vec!["name".to_string()]] {
        let now = scope::plan_reading(&reader, &w.slug, "notes", needs, &w.state).expect("the row rule narrows");
        assert_eq!(clause(&now), clause(&before), "the statement of a table with no audience is the one it was");
        assert!(!clause(&now).0.contains("record_audience"));
        assert!(now.audience.is_none());
    }
    let (sql, _) = clause(&before);
    assert!(sql.starts_with(" AND ("), "{sql}");

    // The table with a policy: a reader the audience names nowhere runs the statement they ran before. It asks for no more than two small
    // index reads to find that out, and adds nothing to the statement.
    let stranger = user(STRANGER);
    let before = scope::plan_of(&stranger, &w.slug, "students", &w.state).expect("the row rule narrows");
    let now = scope::plan_reading(&stranger, &w.slug, "students", vec![], &w.state).expect("the row rule narrows");
    assert!(now.audience.is_none(), "a reader no row names has no audience");
    assert_eq!(now.clause_at("r.id", &w.slug, "students", 1), before.clause_at("r.id", &w.slug, "students", 1), "and runs the statement they ran before");
    assert_eq!(now.clause_for_page("r.id", &w.slug, "students", 1), before.clause_for_page("r.id", &w.slug, "students", 1), "page form too");

    // A reader the audience names names it in their statement, and only when a policy entry reaches every field the statement reads.
    let named = scope::plan_reading(&reader, &w.slug, "students", vec![], &w.state).expect("a plan");
    let (sql, params) = named.clause_at("r.id", &w.slug, "students", 1);
    assert!(sql.contains("record_audience"), "{sql}");
    assert!(sql.contains(" OR ") || sql.contains("UNION"), "the audience is one more way in: {sql}");
    assert!(params.len() >= 3);
    let beyond = scope::plan_reading(&reader, &w.slug, "students", vec!["advisor".to_string()], &w.state).expect("a plan");
    assert!(!beyond.clause_at("r.id", &w.slug, "students", 1).0.contains("record_audience"), "a filter on a field no policy reaches finds nothing through the audience");
    // The page form probes record by record, and the audience is one more probe.
    let (sql, _) = named.clause_for_page("r.id", &w.slug, "students", 1);
    assert!(sql.contains("record_audience"), "{sql}");

    // A reader who is not a person reading as themselves is not widened: a guest, a view-as session, an outside app.
    let mut viewed = user(READER);
    viewed.view_as_by = Some(Box::new(user(OWNER)));
    let plan = scope::plan_reading(&viewed, &w.slug, "students", vec![], &w.state);
    assert!(plan.is_none_or(|p| p.audience.is_none()), "view-as is narrower than the person by design");
    // A reader who already reads every row by their rules has nothing to widen, so the audience adds nothing, and asks for no principals.
    let owner_plan = scope::plan_reading(&user(OWNER), &w.slug, "students", vec![], &w.state);
    assert!(owner_plan.is_none_or(|p| p.audience.is_none()));
}

// ---------------------------------------------------------------------------------------------------------------------
// 7. A synced audience value arrives from its source. A normal write is refused with a conflict that points to a row share.
// ---------------------------------------------------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn a_synced_audience_value_cannot_be_overridden_by_a_normal_write() {
    let w = world().await;
    let slug = format!("synced-{}", &Uuid::new_v4().to_string()[..8]);
    let mut m = manifest(&slug);
    m["kind"] = json!("Dataset");
    m["tables"] = json!([{ "id": "students", "name": "Students", "slug": "students", "primary_field": "name", "fields": [text("name", false), text("advisor", true), text("reviewer", false)],
                           "audience": [{ "field": "reviewer", "grants": "read", "fields": ["name"] }] }]);
    w.state.persist_app_manifest(serde_json::from_value(m).expect("a dataset manifest")).expect("the manifest is stored");
    // A reader of the dataset reads the rows they advise, as in the app above.
    set_rules(&slug, vec![rule("students")], &w.state);

    // The sync writes the rows, and the audience follows them.
    let rec = |id: &str, name: &str, reviewer: &str| DatasetRecord {
        id: id.to_string(),
        app_slug: slug.clone(),
        data: json!({ "_table_id": "students", "name": name, "advisor": OTHER, "reviewer": reviewer }),
        ceds_mapping: json!({}),
        is_ferpa_sensitive: false,
        created_at: chrono::Utc::now().to_rfc3339(),
        created_by: Some("sync".to_string()),
        version: 1,
    };
    w.state.repository.upsert_record(&rec(&format!("{slug}-1"), "Named", READER), &scaffoldry_server::repository::WriteCtx::system("sync")).unwrap();
    w.state.repository.upsert_record(&rec(&format!("{slug}-2"), "Unnamed", OTHER), &scaffoldry_server::repository::WriteCtx::system("sync")).unwrap();
    let (s, list) = rest(&w.app, "GET", &format!("/api/v1/apps/{slug}/tables/students/records?limit=10"), &w.reader, None).await;
    assert_eq!(s, StatusCode::OK, "{list}");
    let seen: BTreeSet<String> = list["records"].as_array().unwrap().iter().map(|r| r["data"]["name"].as_str().unwrap().to_string()).collect();
    assert_eq!(seen, set(&["Named"]), "access follows the value the source sent");

    // A person cannot change who is named. The refusal is a conflict, and it says what to do instead.
    for (who, token) in [("admin", &w.admin), ("owner", &w.owner)] {
        let (s, b) = rest(&w.app, "PATCH", &format!("/api/v1/apps/{slug}/records/{slug}-2"), token, Some(json!({ "data": { "reviewer": READER }, "version": 1 }))).await;
        assert_eq!((s, b["code"].as_str()), (StatusCode::CONFLICT, Some("read_only_source")), "{who}: {b}");
        assert!(b["error"].as_str().unwrap_or_default().contains("row share"), "{who}: the refusal points to a row share: {b}");
    }
    let (s, list) = rest(&w.app, "GET", &format!("/api/v1/apps/{slug}/tables/students/records?limit=10"), &w.reader, None).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(list["records"].as_array().map(Vec::len), Some(1), "the refused write changed nothing");
}

// ---------------------------------------------------------------------------------------------------------------------
// 8. Changing who a row names is a change to the row, so the record history holds it, with who made it. Nothing new records it.
// ---------------------------------------------------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn a_change_to_an_audience_field_is_in_the_record_history_with_who_made_it() {
    let w = world().await;
    live_app(&w).await;
    let row = student(&w, "Mike", json!({ "reviewer": OTHER })).await;
    let (_, now) = one(&w, &w.admin, &row).await;
    let whole = json!({ "_table_id": "students", "name": "Mike", "score": 7, "advisor": OTHER, "unit": w.physics.to_string(), "reviewer": READER });
    let (s, b) = rest(&w.app, "PATCH", &format!("/api/v1/apps/{}/records/{row}", w.slug), &w.admin, Some(json!({ "data": whole, "version": now["version"] }))).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    let (s, history) = rest(&w.app, "GET", &format!("/api/v1/apps/{}/records/{row}/history", w.slug), &w.admin, None).await;
    assert_eq!(s, StatusCode::OK, "{history}");
    let entry = history["entries"].as_array().or_else(|| history["history"].as_array()).or_else(|| history.as_array()).expect("a list of entries");
    let update = entry.iter().find(|e| e["action"] == "update").expect("the update is in the history");
    assert_eq!(update["changes"]["reviewer"], json!({ "old": OTHER, "new": READER }), "{update}");
    assert_eq!(update["actor"], json!(ADMIN), "who made the change: {update}");
}

// ---------------------------------------------------------------------------------------------------------------------
// 9. A statement reads the rules and the audience as one small set when they reach few rows, and keeps the page's early stop and the count's
//    single scan when they reach many. The two give the same rows. Here both are run over the same few rows.
// ---------------------------------------------------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn the_set_form_and_the_scan_form_read_the_same_rows() {
    use scaffoldry_server::repository::{RecordQuery, FEW_ROWS};
    let w = world().await;
    live_app(&w).await;
    student(&w, "Alpha", json!({ "reviewer": READER })).await;
    student(&w, "Bravo", json!({ "reviewer": OTHER })).await;
    student(&w, "Charlie", json!({ "reviewer": format!("mentor@{}", w.physics) })).await;
    student(&w, "Foxtrot", json!({ "advisor": READER })).await;
    student(&w, "Golf", json!({ "advisor": READER, "reviewer": READER })).await;
    for i in 0..30 {
        student(&w, &format!("Filler {i}"), json!({})).await;
    }
    let reader = user(READER);
    let plan = scope::plan_reading(&reader, &w.slug, "students", vec![], &w.state).expect("a plan");
    assert!(plan.audience.as_ref().is_some_and(|a| a.few), "five rows of the reader's, out of thirty-five, are few (at most {FEW_ROWS})");

    let ask = |few: bool| {
        let mut plan = plan.clone();
        plan.audience.as_mut().unwrap().few = few;
        let page = RecordQuery { table_id: "students".into(), any: false, clauses: vec![], sort: None, after: None, limit: 101, scope: Some(plan.clone()), search: None, expr: None, unit: None };
        let rows: Vec<String> = w.state.repository.query_records(&w.slug, &page).unwrap().into_iter().map(|r| r.data["name"].as_str().unwrap().to_string()).collect();
        let count = w.state.repository.count_scoped_records(&w.slug, "students", Some(&plan)).unwrap();
        (rows, count)
    };
    let (few_rows, few_count) = ask(true);
    let (scan_rows, scan_count) = ask(false);
    assert_eq!(few_rows, scan_rows, "the page is the same rows in the same order");
    assert_eq!(few_rows, ["Alpha", "Charlie", "Foxtrot", "Golf"], "each row once, though Golf is advised and named");
    assert_eq!((few_count, scan_count), (4, 4), "the count is the same");
    let (set, _) = plan.clause_at("r.id", &w.slug, "students", 1);
    assert!(set.contains("UNION"), "few rows are one set: {set}");
    let mut wide = plan.clone();
    wide.audience.as_mut().unwrap().few = false;
    let (scan, _) = wide.clause_at("r.id", &w.slug, "students", 1);
    assert!(!scan.contains("UNION") && scan.contains(" OR "), "many rows are two memberships: {scan}");
    let (page, _) = wide.clause_for_page("r.id", &w.slug, "students", 1);
    assert!(page.contains("OFFSET 0"), "and the page keeps its probes: {page}");
}
