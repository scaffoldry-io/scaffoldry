//! Data layer phase 10: row shares.
//!
//! A person shares rows of one table with another person. A share inside what the grantee could already read needs nothing and is not
//! stored. A share beyond it needs a justification and an expiry, carries the server's own assessment, and waits for a person to
//! approve it. A share of a row that carries a restricted category needs a steward who is authorized for the category. Nobody approves
//! their own request. An approved share opens the named rows and fields for the named person until its date, through the real list,
//! count and single-row routes, and opens nothing else. A read checks the date itself. A job writes the ledger entry and tells the
//! owners. The clock is never waited for: the job takes the time it should act at, and a test moves a date by writing it.

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::Router;
use http_body_util::BodyExt;
use scaffoldry_core::access::{Op, Operand, PersonAttr, Pred, RowOp, RowRule};
use scaffoldry_server::build_app_with_state;
use scaffoldry_server::service::identity::issue_test_token_and_user;
use scaffoldry_server::service::scope::set_rules;
use scaffoldry_server::state::{CollaboratorRecord, OrganizationNode, PositionType, RoleRow, ServerState, WorkspaceRecord};
use serde_json::{json, Value};
use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::OnceLock;
use tower::ServiceExt;
use uuid::Uuid;

const WS: &str = "ws-shares";
const ROOT: &str = "00000000-0000-0000-0000-000000000001";
const ADMIN: &str = "jordan.lee@state.edu";
const OWNER: &str = "olga.owner@science.state.edu";
/// A steward who is also a registrar, so authorized for `phi`.
const S1: &str = "sam.steward@science.state.edu";
const S2: &str = "sue.steward@science.state.edu";
/// A steward who is not a registrar, so not authorized for `phi`.
const S3: &str = "stan.steward@science.state.edu";
const GRANTEE: &str = "nora.editor@science.state.edu";

fn files_dir() {
    static DIR: OnceLock<PathBuf> = OnceLock::new();
    DIR.get_or_init(|| {
        let dir = std::env::temp_dir().join(format!("scaffoldry-shares-{}", Uuid::new_v4()));
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
    s1: String,
    s2: String,
    s3: String,
    grantee: String,
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
    let ws = WorkspaceRecord {
        id: WS.to_string(),
        name: "Shares".to_string(),
        code: "WS-SHARES".to_string(),
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

/// `phi` is protected and needs a registrar at or above the row's unit.
fn categories() -> Value {
    json!([
        { "id": "phi", "name": "PHI", "protected": true, "detectors": [], "requires": [{ "position": "registrar", "reach": "unit_and_below" }] },
        { "id": "open", "name": "Open", "protected": false, "detectors": [] }
    ])
}

/// science > physics. The workspace sits at science and the app's own unit is physics. The workspace owner and admins read every row the
/// category gate lets them read. The grantee is an editor, whom the row rule narrows to the rows they advise.
async fn world() -> World {
    files_dir();
    let db = common::TestDb::new();
    let state = db.state();
    let root = Uuid::parse_str(ROOT).unwrap();
    let science = unit(&state, "science", root);
    let physics = unit(&state, "physics", science);
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
    for who in [S1, S2, S3] {
        member_of(&state, who, "admin");
        appoint(&state, who, science, "faculty", Some("data_steward"));
    }
    appoint(&state, S1, science, "faculty", Some("registrar"));
    appoint(&state, S2, science, "faculty", Some("registrar"));
    member_of(&state, GRANTEE, "editor");
    member_of(&state, OWNER, "owner");
    state.settings.write().unwrap().insert("sensitivity.categories".to_string(), categories());
    let app = build_app_with_state(state.clone()).expect("router");
    let slug = format!("students-{}", &Uuid::new_v4().to_string()[..8]);
    World {
        db,
        state: state.clone(),
        app,
        admin: issue_test_token_and_user(ADMIN),
        owner: issue_test_token_and_user(OWNER),
        s1: issue_test_token_and_user(S1),
        s2: issue_test_token_and_user(S2),
        s3: issue_test_token_and_user(S3),
        grantee: issue_test_token_and_user(GRANTEE),
        physics,
        slug,
    }
}

fn manifest(slug: &str) -> Value {
    let text = |n: &str, indexed: bool| json!({ "name": n, "label": n, "field_type": "Text", "required": false, "ferpa_sensitive": false, "indexed": indexed });
    json!({
        "slug": slug, "title": "Students", "description": "Students by unit", "organization_code": "PHYSICS", "department": "science", "workspace_id": WS, "views": [],
        "tables": [
            {
                "id": "students", "name": "Students", "slug": "students", "primary_field": "name",
                "fields": [
                    text("name", false),
                    { "name": "score", "label": "Score", "field_type": "Number", "required": false, "ferpa_sensitive": false },
                    text("advisor", true),
                    { "name": "unit", "label": "Unit", "field_type": "Unit", "hierarchy": "academic", "required": false, "ferpa_sensitive": false }
                ],
                "unit_field": "unit"
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
    assert_eq!(s, StatusCode::CREATED, "{b}");
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

/// A student row made by the Platform Admin.
async fn student(w: &World, name: &str, advisor: &str, score: i64, tags: Option<Value>) -> String {
    let mut data = json!({ "_table_id": "students", "name": name, "score": score, "advisor": advisor, "unit": w.physics.to_string() });
    if let Some(t) = tags {
        data["_tags"] = t;
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

struct Rows {
    dataset: String,
    /// Advised by someone else. The share's target.
    r1: String,
    /// Advised by someone else. Never shared.
    r2: String,
    /// Advised by the grantee.
    r3: String,
    /// Advised by someone else and tagged `phi`.
    p1: String,
    /// Tagged `phi`. Never shared.
    p2: String,
    n1: String,
}

const OTHER: &str = "someone.else@science.state.edu";

async fn rows(w: &World) -> Rows {
    let dataset = live_app(w).await;
    let r1 = student(w, "Zed Secretname", OTHER, 11, None).await;
    let r2 = student(w, "Quin Hiddenvalue", OTHER, 22, None).await;
    let r3 = student(w, "Mine Visible", GRANTEE, 33, None).await;
    let p1 = student(w, "Phil Phivalue", OTHER, 44, Some(json!(["phi"]))).await;
    let p2 = student(w, "Pia Phisecond", OTHER, 55, Some(json!(["phi"]))).await;
    let n1 = note(w, "Note Hidden", OTHER).await;
    note(w, "Note Mine", GRANTEE).await;
    Rows { dataset, r1, r2, r3, p1, p2, n1 }
}

fn at(days: i64) -> String {
    (chrono::Utc::now() + chrono::Duration::days(days)).to_rfc3339()
}

fn body(records: &[&str], access: &str, fields: &[&str], days: i64) -> Value {
    json!({ "records": records, "grantee": GRANTEE, "access": access, "fields": fields, "justification": "The grantee advises this student for the term", "expires_at": at(days) })
}

async fn request(w: &World, who: &str, dataset: &str, b: Value) -> (StatusCode, Value) {
    rest(&w.app, "POST", &format!("/api/v1/data/datasets/{dataset}/row-shares"), who, Some(b)).await
}

async fn decide(w: &World, who: &str, id: &str, decision: &str) -> (StatusCode, Value) {
    rest(&w.app, "POST", &format!("/api/v1/data/row-shares/{id}/decide"), who, Some(json!({ "decision": decision, "reason": "The reason fits and the dates are short" }))).await
}

async fn list_shares(w: &World, who: &str, dataset: &str, query: &str) -> (StatusCode, Value) {
    rest(&w.app, "GET", &format!("/api/v1/data/datasets/{dataset}/row-shares{query}"), who, None).await
}

/// A request that is made and approved by a second steward. Returns the share id.
async fn approved(w: &World, r: &Rows, b: Value) -> String {
    let (s, made) = request(w, &w.owner, &r.dataset, b).await;
    assert_eq!(s, StatusCode::CREATED, "{made}");
    let id = made["id"].as_str().unwrap().to_string();
    // A beyond share is approved by any steward. A restricted one by an authorized steward, and S2 is one.
    let (s, done) = decide(w, &w.s2, &id, "approve").await;
    assert_eq!(s, StatusCode::OK, "{done}");
    assert_eq!(done["status"], "Approved", "{done}");
    id
}

async fn names(w: &World, token: &str, table: &str) -> BTreeSet<String> {
    let (s, body) = rest(&w.app, "GET", &format!("/api/v1/apps/{}/tables/{table}/records?limit=100", w.slug), token, None).await;
    assert_eq!(s, StatusCode::OK, "{body}");
    body["records"].as_array().unwrap().iter().map(|r| r["data"]["name"].as_str().unwrap_or("").to_string()).collect()
}

async fn list_body(w: &World, token: &str) -> Value {
    let (s, body) = rest(&w.app, "GET", &format!("/api/v1/apps/{}/tables/students/records?limit=100", w.slug), token, None).await;
    assert_eq!(s, StatusCode::OK, "{body}");
    body
}

async fn count(w: &World, token: &str, table: &str, query: &str) -> f64 {
    let (s, body) = rest(&w.app, "GET", &format!("/api/v1/apps/{}/tables/{table}/aggregate?function=count{query}", w.slug), token, None).await;
    assert_eq!(s, StatusCode::OK, "{body}");
    body["value"].as_f64().unwrap_or(0.0)
}

async fn one(w: &World, token: &str, table: &str, id: &str) -> (StatusCode, Value) {
    rest(&w.app, "GET", &format!("/api/v1/apps/{}/tables/{table}/records/{id}", w.slug), token, None).await
}

fn set(names: &[&str]) -> BTreeSet<String> {
    names.iter().map(|n| n.to_string()).collect()
}

fn encode(s: &str) -> String {
    s.bytes().map(|b| if b.is_ascii_alphanumeric() { (b as char).to_string() } else { format!("%{b:02X}") }).collect()
}

fn equals(field: &str, value: &str) -> String {
    encode(&json!({ "conjunction": "And", "clauses": [{ "id": "c", "field_name": field, "operator": "Equals", "value": value }] }).to_string())
}

fn shares_in_db(w: &World) -> i64 {
    w.db.run(|c| c.query_one("SELECT count(*) FROM row_shares", &[]).unwrap().get(0))
}

fn ledger_entries(w: &World, op: &str) -> i64 {
    let op = op.to_string();
    w.db.run(move |c| {
        c.query_one("SELECT count(*) FROM governance_ledger WHERE decision_type = 'AccessGrantChanged' AND payload->>'source' = 'row_share' AND payload->>'op' = $1", &[&op]).unwrap().get(0)
    })
}

fn notices(w: &World, kind: &str) -> Vec<String> {
    let kind = kind.to_string();
    w.db.run(move |c| c.query("SELECT lower(recipient) FROM notifications WHERE kind = $1 ORDER BY recipient", &[&kind]).unwrap().iter().map(|r| r.get(0)).collect())
}

/// Moves a share's date into the past by writing it. A test never waits for a date.
fn expire(w: &World, id: &str) {
    let id = Uuid::parse_str(id).unwrap();
    w.db.run(move |c| c.execute("UPDATE row_shares SET expires_at = NOW() - interval '1 minute' WHERE id = $1", &[&id]).unwrap());
}

async fn end_due(w: &World) -> usize {
    let state = w.db.state();
    tokio::task::spawn_blocking(move || scaffoldry_server::service::row_shares::end_due(&state, chrono::Utc::now())).await.unwrap().unwrap()
}

// 1. A share beyond the default without a justification or an expiry is refused.
#[tokio::test]
async fn a_share_beyond_the_default_needs_a_justification_and_an_expiry() {
    let w = world().await;
    let r = rows(&w).await;
    let mut no_reason = body(&[&r.r1], "read", &["name"], 30);
    no_reason.as_object_mut().unwrap().remove("justification");
    let (s, b) = request(&w, &w.owner, &r.dataset, no_reason).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{b}");
    let mut blank = body(&[&r.r1], "read", &["name"], 30);
    blank["justification"] = json!("   ");
    assert_eq!(request(&w, &w.owner, &r.dataset, blank).await.0, StatusCode::BAD_REQUEST);
    let mut no_date = body(&[&r.r1], "read", &["name"], 30);
    no_date.as_object_mut().unwrap().remove("expires_at");
    let (s, b) = request(&w, &w.owner, &r.dataset, no_date).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{b}");
    let (s, b) = request(&w, &w.owner, &r.dataset, body(&[&r.r1], "read", &["name"], -1)).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "an expiry in the past: {b}");
    let (s, b) = request(&w, &w.owner, &r.dataset, body(&[&r.r1], "read", &["name"], 400)).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "an expiry more than a year ahead: {b}");
    assert_eq!(shares_in_db(&w), 0, "nothing was stored");
    // Nothing the grantee reads changed.
    assert_eq!(names(&w, &w.grantee, "students").await, set(&["Mine Visible"]));
    let (s, b) = request(&w, &w.owner, &r.dataset, body(&[&r.r1], "read", &["name"], 30)).await;
    assert_eq!(s, StatusCode::CREATED, "{b}");
    assert_eq!((b["status"].as_str(), b["kind"].as_str()), (Some("Requested"), Some("beyond")), "{b}");
    assert_eq!(shares_in_db(&w), 1);
}

// A share inside what the grantee already reads needs nothing, and nothing is stored.
#[tokio::test]
async fn a_share_inside_what_the_grantee_already_reads_needs_nothing_and_is_not_stored() {
    let w = world().await;
    let r = rows(&w).await;
    let plain = json!({ "records": [r.r3], "grantee": GRANTEE, "access": "read", "fields": ["name"] });
    let (s, b) = request(&w, &w.owner, &r.dataset, plain).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_eq!((b["stored"].clone(), b["kind"].as_str()), (json!(false), Some("default")), "{b}");
    assert_eq!(shares_in_db(&w), 0);
    assert_eq!(names(&w, &w.grantee, "students").await, set(&["Mine Visible"]));
    // A row the grantee does not read is not inside the default, so it asks for more.
    let (s, _) = request(&w, &w.owner, &r.dataset, json!({ "records": [r.r1], "grantee": GRANTEE, "access": "read", "fields": ["name"] })).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    // The requester must read what they share. The grantee cannot pass on a row they do not read.
    let mut hand_on = body(&[&r.r2], "read", &["name"], 30);
    hand_on["grantee"] = json!(OWNER);
    let (s, b) = request(&w, &w.grantee, &r.dataset, hand_on).await;
    assert_eq!(s, StatusCode::NOT_FOUND, "{b}");
}

// 2. Before approval the grantee reads nothing new. After approval they read exactly the named rows and fields.
#[tokio::test]
async fn before_approval_the_grantee_reads_nothing_new_and_after_approval_exactly_the_named_rows_and_fields() {
    let w = world().await;
    let r = rows(&w).await;
    let before = names(&w, &w.grantee, "students").await;
    assert_eq!(before, set(&["Mine Visible"]));
    let (s, made) = request(&w, &w.owner, &r.dataset, body(&[&r.r1], "read", &["name"], 30)).await;
    assert_eq!(s, StatusCode::CREATED, "{made}");
    let id = made["id"].as_str().unwrap().to_string();
    // Before approval: nothing new, on the list, the count and the single row.
    assert_eq!(names(&w, &w.grantee, "students").await, before);
    assert_eq!(count(&w, &w.grantee, "students", "").await, 1.0);
    assert_eq!(one(&w, &w.grantee, "students", &r.r1).await.0, StatusCode::NOT_FOUND);
    // A share the grantee has no right to decide cannot approve itself.
    assert_ne!(decide(&w, &w.grantee, &id, "approve").await.0, StatusCode::OK);
    assert_eq!(names(&w, &w.grantee, "students").await, before);
    let (s, done) = decide(&w, &w.s3, &id, "approve").await;
    assert_eq!(s, StatusCode::OK, "{done}");
    // After approval: the named row, and only the named field.
    let list = list_body(&w, &w.grantee).await;
    let got: BTreeSet<String> = list["records"].as_array().unwrap().iter().map(|r| r["data"]["name"].as_str().unwrap().to_string()).collect();
    assert_eq!(got, set(&["Mine Visible", "Zed Secretname"]), "{list}");
    let shared = list["records"].as_array().unwrap().iter().find(|x| x["id"] == json!(r.r1)).unwrap();
    let keys: BTreeSet<&str> = shared["data"].as_object().unwrap().keys().map(String::as_str).filter(|k| !k.starts_with('_')).collect();
    assert_eq!(keys, BTreeSet::from(["name"]), "only the shared field: {shared}");
    // The row the grantee already read keeps every field it had.
    let mine = list["records"].as_array().unwrap().iter().find(|x| x["id"] == json!(r.r3)).unwrap();
    assert!(mine["data"]["score"] == json!(33) && mine["data"]["advisor"] == json!(GRANTEE), "{mine}");
    assert_eq!(count(&w, &w.grantee, "students", "").await, 2.0);
    let (s, row) = one(&w, &w.grantee, "students", &r.r1).await;
    assert_eq!(s, StatusCode::OK, "{row}");
    assert_eq!(row["data"]["name"], "Zed Secretname");
    assert!(row["data"].get("score").is_none() && row["data"].get("advisor").is_none(), "{row}");
    // Not other rows.
    for other in [&r.r2, &r.p1, &r.p2] {
        assert_eq!(one(&w, &w.grantee, "students", other).await.0, StatusCode::NOT_FOUND, "{other}");
    }
    // Not other tables, and not by the other table's route for the same id.
    assert_eq!(names(&w, &w.grantee, "notes").await, set(&["Note Mine"]));
    assert_eq!(count(&w, &w.grantee, "notes", "").await, 1.0);
    assert_eq!(one(&w, &w.grantee, "notes", &r.n1).await.0, StatusCode::NOT_FOUND);
    // Not by asking about a field the share does not cover. A filter on `advisor` would show the value by what it matches.
    assert_eq!(count(&w, &w.grantee, "students", &format!("&filter={}", equals("advisor", OTHER))).await, 0.0, "a filter on an unshared field finds no shared row");
    // The same on the list: a search matches on any field, so it does not take the shared row in. The older offset path (a sort or a formula
    // filter) reads no share at all.
    for query in ["sort_field=advisor", "search=Zed"] {
        let (s, b) = rest(&w.app, "GET", &format!("/api/v1/apps/{}/tables/students/records?limit=100&{query}", w.slug), &w.grantee, None).await;
        assert_eq!(s, StatusCode::OK, "{b}");
        let got: BTreeSet<String> = b["records"].as_array().unwrap().iter().map(|r| r["data"]["name"].as_str().unwrap_or("").to_string()).collect();
        assert!(!got.contains("Zed Secretname"), "{query}: {b}");
    }
    // Nobody else's reads moved.
    assert_eq!(names(&w, &w.s3, "students").await.len(), 3, "an admin reads the rows the gate lets them read");
}

// A share must name rows of its own dataset.
#[tokio::test]
async fn a_share_names_only_live_rows_of_its_own_table() {
    let w = world().await;
    let r = rows(&w).await;
    let (s, b) = request(&w, &w.owner, &r.dataset, body(&[&r.n1], "read", &["name"], 30)).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "a row of another table: {b}");
    let (s, b) = request(&w, &w.owner, &r.dataset, body(&["no-such-row"], "read", &["name"], 30)).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{b}");
    let (s, b) = request(&w, &w.owner, &r.dataset, body(&[&r.r1], "read", &["no_such_field"], 30)).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{b}");
    let (s, b) = request(&w, &w.owner, &r.dataset, body(&[&r.r1], "read", &[], 30)).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{b}");
    let (s, b) = request(&w, &w.owner, &r.dataset, json!({ "records": [r.r1], "grantee": "nobody.here@science.state.edu", "access": "read", "fields": ["name"], "justification": "A reason", "expires_at": at(30) })).await;
    assert!(s == StatusCode::BAD_REQUEST || s == StatusCode::CONFLICT, "a grantee the server does not know opens no app: {s} {b}");
    assert_eq!(shares_in_db(&w), 0);
}

// 3. A restricted-category share needs an authorized steward. A steward cannot approve their own request. A second person must.
#[tokio::test]
async fn a_restricted_share_needs_an_authorized_steward_and_nobody_approves_their_own_request() {
    let w = world().await;
    let r = rows(&w).await;
    // S1 is a steward and a registrar, so S1 reads the phi row and may ask. The grantee has no registrar position.
    let (s, made) = request(&w, &w.s1, &r.dataset, body(&[&r.p1], "read", &["name"], 30)).await;
    assert_eq!(s, StatusCode::CREATED, "{made}");
    assert_eq!(made["kind"], "restricted", "{made}");
    assert!(made["assessment"]["exposure"]["categories"].as_array().unwrap().contains(&json!("phi")), "{made}");
    let id = made["id"].as_str().unwrap().to_string();
    // S3 asked to approve a phi row is a steward and no registrar, so is not authorized for the category.
    let (s, b) = decide(&w, &w.s3, &id, "approve").await;
    assert_eq!(s, StatusCode::FORBIDDEN, "{b}");
    // S1 asked, so S1 may not approve their own request, though S1 is authorized.
    let (s, b) = decide(&w, &w.s1, &id, "approve").await;
    assert_eq!(s, StatusCode::FORBIDDEN, "{b}");
    // The grantee cannot approve their own access.
    assert_eq!(decide(&w, &w.grantee, &id, "approve").await.0, StatusCode::FORBIDDEN);
    assert_eq!(names(&w, &w.grantee, "students").await, set(&["Mine Visible"]), "nothing opened");
    assert_eq!(ledger_entries(&w, "create"), 0);
    // S2 is a second steward and authorized for the category.
    let (s, b) = decide(&w, &w.s2, &id, "approve").await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_eq!(names(&w, &w.grantee, "students").await, set(&["Mine Visible", "Phil Phivalue"]));
    assert_eq!(one(&w, &w.grantee, "students", &r.p2).await.0, StatusCode::NOT_FOUND, "the other phi row stays closed");
    assert_eq!(ledger_entries(&w, "create"), 1);
}

// A beyond share is approved by a steward who did not ask for it.
#[tokio::test]
async fn a_steward_cannot_approve_a_share_they_asked_for_and_a_second_person_can() {
    let w = world().await;
    let r = rows(&w).await;
    let (s, made) = request(&w, &w.s3, &r.dataset, body(&[&r.r1], "read", &["name"], 30)).await;
    assert_eq!(s, StatusCode::CREATED, "{made}");
    assert_eq!(made["kind"], "beyond");
    let id = made["id"].as_str().unwrap().to_string();
    assert_eq!(decide(&w, &w.s3, &id, "approve").await.0, StatusCode::FORBIDDEN, "the steward who asked");
    // An owner who is no steward does not decide it either.
    assert_eq!(decide(&w, &w.owner, &id, "approve").await.0, StatusCode::FORBIDDEN);
    assert_eq!(names(&w, &w.grantee, "students").await, set(&["Mine Visible"]));
    assert_eq!(decide(&w, &w.s1, &id, "approve").await.0, StatusCode::OK);
    assert_eq!(names(&w, &w.grantee, "students").await, set(&["Mine Visible", "Zed Secretname"]));
    // A decided request is decided once.
    assert_eq!(decide(&w, &w.s2, &id, "approve").await.0, StatusCode::CONFLICT);
}

// A decline opens nothing and writes the existing decision type.
#[tokio::test]
async fn a_declined_share_opens_nothing() {
    let w = world().await;
    let r = rows(&w).await;
    let (_, made) = request(&w, &w.owner, &r.dataset, body(&[&r.r1], "read", &["name"], 30)).await;
    let id = made["id"].as_str().unwrap().to_string();
    let (s, b) = decide(&w, &w.s1, &id, "decline").await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_eq!(b["status"], "Declined");
    assert_eq!(names(&w, &w.grantee, "students").await, set(&["Mine Visible"]));
    let declined: i64 = w.db.run(|c| c.query_one("SELECT count(*) FROM governance_ledger WHERE decision_type = 'AccessRequestDecided' AND payload->>'source' = 'row_share'", &[]).unwrap().get(0));
    assert_eq!(declined, 1);
    assert_eq!(decide(&w, &w.s2, &id, "approve").await.0, StatusCode::CONFLICT, "a declined request stays declined");
}

// 4. After the expiry, the read closes with no job running.
#[tokio::test]
async fn after_the_expiry_the_read_closes_with_no_job_running() {
    let w = world().await;
    let r = rows(&w).await;
    let id = approved(&w, &r, body(&[&r.r1], "read", &["name"], 30)).await;
    assert_eq!(names(&w, &w.grantee, "students").await, set(&["Mine Visible", "Zed Secretname"]));
    expire(&w, &id);
    assert_eq!(names(&w, &w.grantee, "students").await, set(&["Mine Visible"]), "the list");
    assert_eq!(count(&w, &w.grantee, "students", "").await, 1.0, "the count");
    assert_eq!(one(&w, &w.grantee, "students", &r.r1).await.0, StatusCode::NOT_FOUND, "the single row");
    // No job ran. The share is still marked approved, and the read closed on the date alone.
    let status: String = w.db.run({
        let id = Uuid::parse_str(&id).unwrap();
        move |c| c.query_one("SELECT status FROM row_shares WHERE id = $1", &[&id]).unwrap().get(0)
    });
    assert_eq!(status, "Approved");
    // An approved share that has run out is not live for an owner either.
    let (s, live) = list_shares(&w, &w.owner, &r.dataset, "").await;
    assert_eq!(s, StatusCode::OK, "{live}");
    assert!(live["shares"].as_array().unwrap().is_empty(), "{live}");
}

// 5. The ending writes a ledger entry and notifies the owner.
#[tokio::test]
async fn the_ending_writes_a_ledger_entry_and_tells_the_owners() {
    let w = world().await;
    let r = rows(&w).await;
    let live = approved(&w, &r, body(&[&r.r1], "read", &["name"], 30)).await;
    let gone = approved(&w, &r, body(&[&r.r2], "read", &["name"], 30)).await;
    assert_eq!(ledger_entries(&w, "end"), 0);
    // A share that has not reached its date is left alone.
    assert_eq!(end_due(&w).await, 0);
    expire(&w, &gone);
    assert_eq!(end_due(&w).await, 1);
    assert_eq!(ledger_entries(&w, "end"), 1);
    let entry: Value = w.db.run(|c| c.query_one("SELECT payload FROM governance_ledger WHERE decision_type = 'AccessGrantChanged' AND payload->>'op' = 'end' AND payload->>'source' = 'row_share'", &[]).unwrap().get(0));
    assert_eq!(entry["share"], json!(gone), "{entry}");
    assert_eq!(entry["reason"], "expired", "{entry}");
    // The owner who made the share and the owners of the app are told. The grantee's rows are not named in the notice.
    let told = notices(&w, "row_share_ended");
    assert!(told.contains(&OWNER.to_string()), "{told:?}");
    let body: String = w.db.run(|c| c.query_one("SELECT string_agg(title || ' ' || body, ' ') FROM notifications WHERE kind = 'row_share_ended'", &[]).unwrap().get(0));
    for value in ["Quin Hiddenvalue", "Zed Secretname", &r.r2] {
        assert!(!body.contains(value), "{body}");
    }
    // It ends once.
    let before = notices(&w, "row_share_ended").len();
    assert_eq!(end_due(&w).await, 0);
    assert_eq!(ledger_entries(&w, "end"), 1);
    assert_eq!(notices(&w, "row_share_ended").len(), before);
    // The share that has time left still reads. The one that ended does not.
    assert_eq!(names(&w, &w.grantee, "students").await, set(&["Mine Visible", "Zed Secretname"]));
    // The approval wrote its own entry for each.
    assert_eq!(ledger_entries(&w, "create"), 2);
    let _ = live;
}

// 6. An owner lists the live shares on their rows. A non-owner cannot.
#[tokio::test]
async fn an_owner_lists_the_live_shares_on_their_rows_and_a_non_owner_cannot() {
    let w = world().await;
    let r = rows(&w).await;
    let id = approved(&w, &r, body(&[&r.r1], "read", &["name"], 30)).await;
    let (_, waiting) = request(&w, &w.s1, &r.dataset, body(&[&r.r2], "read", &["name"], 30)).await;
    for who in [&w.owner, &w.s1, &w.admin] {
        let (s, live) = list_shares(&w, who, &r.dataset, "").await;
        assert_eq!(s, StatusCode::OK, "{live}");
        let list = live["shares"].as_array().unwrap();
        assert_eq!(list.len(), 1, "only the live one: {live}");
        assert_eq!(list[0]["id"], json!(id));
        assert_eq!((list[0]["grantee"].as_str(), list[0]["requested_by"].as_str(), list[0]["access"].as_str()), (Some(GRANTEE), Some(OWNER), Some("read")), "{live}");
        assert_eq!(list[0]["records"], json!([r.r1]));
        assert_eq!(list[0]["fields"], json!(["name"]));
        assert_eq!(list[0]["decided_by"], json!(S2));
        assert!(list[0]["expires_at"].is_string() && list[0]["justification"].is_string(), "{live}");
    }
    // The shares on one row.
    let (_, on_r1) = list_shares(&w, &w.owner, &r.dataset, &format!("?record={}", r.r1)).await;
    assert_eq!(on_r1["shares"].as_array().unwrap().len(), 1);
    let (_, on_r3) = list_shares(&w, &w.owner, &r.dataset, &format!("?record={}", r.r3)).await;
    assert!(on_r3["shares"].as_array().unwrap().is_empty());
    // The queue of requests that wait is a list too.
    let (_, queue) = list_shares(&w, &w.s2, &r.dataset, "?status=Requested").await;
    assert_eq!(queue["shares"][0]["id"], waiting["id"], "{queue}");
    // A person who owns nothing here does not list them, and neither does the grantee.
    let (s, b) = list_shares(&w, &w.grantee, &r.dataset, "").await;
    assert_eq!(s, StatusCode::FORBIDDEN, "{b}");
    assert!(!b.to_string().contains("Zed Secretname"));
    // A share is read by the people it concerns.
    for who in [&w.grantee, &w.owner, &w.s2] {
        assert_eq!(rest(&w.app, "GET", &format!("/api/v1/data/row-shares/{id}"), who, None).await.0, StatusCode::OK);
    }
}

// 7. The assessment is attached and holds no record values.
#[tokio::test]
async fn the_assessment_is_attached_and_holds_no_record_value() {
    let w = world().await;
    let r = rows(&w).await;
    w.state.settings.write().unwrap().insert("access.request_max_days".to_string(), json!(30));
    let (s, made) = request(&w, &w.s1, &r.dataset, body(&[&r.r1, &r.p1], "write", &["name", "score"], 60)).await;
    assert_eq!(s, StatusCode::CREATED, "{made}");
    let a = &made["assessment"];
    assert_eq!(a["source"], "server_precheck", "{a}");
    assert_eq!(a["exposure"]["rows"], 2, "{a}");
    assert_eq!(a["exposure"]["rows_restricted"], 1, "{a}");
    assert_eq!(a["exposure"]["access"], "write", "{a}");
    assert_eq!(a["exposure"]["fields"], json!(["name", "score"]), "{a}");
    assert!(a["exposure"]["categories"].as_array().unwrap().contains(&json!("phi")), "{a}");
    assert_eq!(a["duration"]["max_days"], 30, "{a}");
    assert!(a["duration"]["days"].as_i64().unwrap() >= 59, "{a}");
    assert_eq!(a["duration"]["too_long"], true, "{a}");
    let unusual: Vec<&str> = a["unusual"].as_array().unwrap().iter().map(|u| u.as_str().unwrap()).collect();
    for flag in ["write_access", "restricted_category", "longer_than_setting"] {
        assert!(unusual.contains(&flag), "{flag} in {unusual:?}");
    }
    assert_eq!(a["reason"]["fits"], "not_assessed", "the server calls no model: {a}");
    assert!(a["narrative"].is_null(), "a slot for an agent or a person: {a}");
    // The assessment never approves or refuses: the share still waits, and the long date did not refuse it.
    assert_eq!(made["status"], "Requested");
    // No record value is in it, in the response or in what is stored.
    let stored: Value = w.db.run({
        let id = Uuid::parse_str(made["id"].as_str().unwrap()).unwrap();
        move |c| c.query_one("SELECT assessment FROM row_shares WHERE id = $1", &[&id]).unwrap().get(0)
    });
    assert_eq!(&stored, a);
    let text = stored.to_string();
    for value in ["Zed Secretname", "Phil Phivalue", OTHER, &r.r1, &r.p1] {
        assert!(!text.contains(value), "{value} in {text}");
    }
    // The steward reads it from the single read too.
    let (s, read) = rest(&w.app, "GET", &format!("/api/v1/data/row-shares/{}", made["id"].as_str().unwrap()), &w.s2, None).await;
    assert_eq!(s, StatusCode::OK, "{read}");
    assert_eq!(&read["assessment"], a);
}

// 8. A share for read does not allow a write. A write share allows a write only to the named fields.
#[tokio::test]
async fn a_read_share_allows_no_write_and_a_write_share_allows_only_the_named_fields() {
    let w = world().await;
    let r = rows(&w).await;
    let patch = |version: i64, data: Value| json!({ "version": version, "data": data });
    let url = |id: &str| format!("/api/v1/apps/{}/tables/students/records/{id}", w.slug);
    let read_share = approved(&w, &r, body(&[&r.r1], "read", &["name", "score"], 30)).await;
    let (_, row) = one(&w, &w.grantee, "students", &r.r1).await;
    let version = row["version"].as_i64().unwrap();
    // A read share reads the two fields and writes nothing.
    assert_eq!(row["data"]["score"], json!(11));
    let (s, b) = rest(&w.app, "PATCH", &url(&r.r1), &w.grantee, Some(patch(version, json!({ "score": 99 })))).await;
    assert!(!s.is_success(), "{s} {b}");
    let score = |id: &str| -> Value {
        let id = id.to_string();
        w.db.run(move |c| c.query_one("SELECT data->'score' FROM dataset_records WHERE id = $1", &[&id]).unwrap().get(0))
    };
    let name = |id: &str| -> Value {
        let id = id.to_string();
        w.db.run(move |c| c.query_one("SELECT data->'name' FROM dataset_records WHERE id = $1", &[&id]).unwrap().get(0))
    };
    assert_eq!(score(&r.r1), json!(11));
    // The row the share does not name cannot be written either.
    let (s, _) = rest(&w.app, "PATCH", &url(&r.r2), &w.grantee, Some(patch(1, json!({ "score": 99 })))).await;
    assert!(!s.is_success());
    assert_eq!(score(&r.r2), json!(22));
    // End the read share, and give a write share for `score` only.
    expire(&w, &read_share);
    assert_eq!(one(&w, &w.grantee, "students", &r.r1).await.0, StatusCode::NOT_FOUND);
    approved(&w, &r, body(&[&r.r1], "write", &["score"], 30)).await;
    let (s, row) = one(&w, &w.grantee, "students", &r.r1).await;
    assert_eq!(s, StatusCode::OK, "{row}");
    assert_eq!(row["data"]["score"], json!(11));
    assert!(row["data"].get("name").is_none(), "a write share reads the fields it names: {row}");
    let version = row["version"].as_i64().unwrap();
    // A field the share does not name is refused, whole.
    let (s, b) = rest(&w.app, "PATCH", &url(&r.r1), &w.grantee, Some(patch(version, json!({ "name": "Changed", "score": 77 })))).await;
    assert_eq!(s, StatusCode::FORBIDDEN, "{b}");
    assert_eq!((name(&r.r1), score(&r.r1)), (json!("Zed Secretname"), json!(11)));
    // The named field is written, and the rest of the row stays as it was.
    let (s, b) = rest(&w.app, "PATCH", &url(&r.r1), &w.grantee, Some(patch(version, json!({ "score": 77 })))).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_eq!((name(&r.r1), score(&r.r1)), (json!("Zed Secretname"), json!(77)));
    let advisor: Value = w.db.run({
        let id = r.r1.clone();
        move |c| c.query_one("SELECT data->'advisor' FROM dataset_records WHERE id = $1", &[&id]).unwrap().get(0)
    });
    assert_eq!(advisor, json!(OTHER), "a share cannot move the row to its grantee");
    // The share cannot write a row it does not name, and a delete is never allowed.
    let (s, _) = rest(&w.app, "PATCH", &url(&r.r2), &w.grantee, Some(patch(1, json!({ "score": 99 })))).await;
    assert!(!s.is_success());
    let (s, _) = rest(&w.app, "DELETE", &url(&r.r1), &w.grantee, None).await;
    assert!(!s.is_success());
    assert_eq!(score(&r.r2), json!(22));
    // Once it has run out the write is closed too.
    let write_share: String = w.db.run(|c| c.query_one("SELECT id::text FROM row_shares WHERE access = 'write'", &[]).unwrap().get(0));
    expire(&w, &write_share);
    let (_, current) = rest(&w.app, "GET", &url(&r.r1), &w.admin, None).await;
    let (s, _) = rest(&w.app, "PATCH", &url(&r.r1), &w.grantee, Some(patch(current["version"].as_i64().unwrap(), json!({ "score": 5 })))).await;
    assert!(!s.is_success());
    assert_eq!(score(&r.r1), json!(77));
}

// A share can be ended early, and a share does not reach other apps' reads of the same rows.
#[tokio::test]
async fn a_share_ends_early_by_its_requester_or_a_steward_and_the_read_closes_at_once() {
    let w = world().await;
    let r = rows(&w).await;
    let id = approved(&w, &r, body(&[&r.r1], "read", &["name"], 30)).await;
    assert_eq!(names(&w, &w.grantee, "students").await.len(), 2);
    // A person with no part in it cannot end it.
    assert_eq!(rest(&w.app, "POST", &format!("/api/v1/data/row-shares/{id}/withdraw"), &w.grantee, Some(json!({ "reason": "No longer needed" }))).await.0, StatusCode::FORBIDDEN);
    let (s, b) = rest(&w.app, "POST", &format!("/api/v1/data/row-shares/{id}/withdraw"), &w.owner, Some(json!({ "reason": "The term ended early" }))).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_eq!(b["status"], "Withdrawn");
    assert_eq!(names(&w, &w.grantee, "students").await, set(&["Mine Visible"]));
    assert_eq!(ledger_entries(&w, "end"), 1);
    // Ending twice is a conflict, and writes nothing more.
    assert_eq!(rest(&w.app, "POST", &format!("/api/v1/data/row-shares/{id}/withdraw"), &w.owner, Some(json!({ "reason": "Again" }))).await.0, StatusCode::CONFLICT);
    assert_eq!(ledger_entries(&w, "end"), 1);
}

// The shares of a dataset that is not the grantee's own never open another dataset's rows, and a guest or a non-reader is no grantee.
#[tokio::test]
async fn a_grantee_who_cannot_open_the_app_is_refused() {
    let w = world().await;
    let r = rows(&w).await;
    let stranger = "pat.stranger@elsewhere.edu";
    issue_test_token_and_user(stranger);
    let (s, b) = request(&w, &w.owner, &r.dataset, json!({ "records": [r.r1], "grantee": stranger, "access": "read", "fields": ["name"], "justification": "A reason", "expires_at": at(30) })).await;
    assert_eq!(s, StatusCode::CONFLICT, "{b}");
    assert_eq!(shares_in_db(&w), 0);
    // A share with oneself is no share.
    let (s, _) = request(&w, &w.owner, &r.dataset, json!({ "records": [r.r1], "grantee": OWNER, "access": "read", "fields": ["name"], "justification": "A reason", "expires_at": at(30) })).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
}

// A request nobody decided before its date lapses, and cannot be approved late.
#[tokio::test]
async fn a_request_nobody_decided_before_its_date_lapses_and_cannot_be_approved() {
    let w = world().await;
    let r = rows(&w).await;
    let (_, made) = request(&w, &w.owner, &r.dataset, body(&[&r.r1], "read", &["name"], 30)).await;
    let id = made["id"].as_str().unwrap().to_string();
    expire(&w, &id);
    assert_eq!(decide(&w, &w.s2, &id, "approve").await.0, StatusCode::CONFLICT);
    assert_eq!(names(&w, &w.grantee, "students").await, set(&["Mine Visible"]));
    end_due(&w).await;
    let status: String = w.db.run({
        let id = Uuid::parse_str(&id).unwrap();
        move |c| c.query_one("SELECT status FROM row_shares WHERE id = $1", &[&id]).unwrap().get(0)
    });
    assert_eq!(status, "Lapsed");
}

// The read adds one probe only for a reader who holds a live share.
#[tokio::test]
async fn a_reader_with_no_share_gets_the_statement_they_always_got() {
    let w = world().await;
    let r = rows(&w).await;
    approved(&w, &r, body(&[&r.r1], "read", &["name"], 30)).await;
    // The owner holds no share, and reads what they read before.
    assert_eq!(names(&w, &w.owner, "students").await.len(), 3, "the owner reads the rows the category gate lets them read");
    // Row rules and the gate are unchanged for the grantee's other reads: a count of the other table, an app-wide list.
    let (s, b) = rest(&w.app, "GET", &format!("/api/v1/apps/{}/records", w.slug), &w.grantee, None).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    let text = b.to_string();
    assert!(!text.contains("Zed Secretname"), "the app-wide list is not opened: {text}");
}

// ---------------------------------------------------------------------------------------------------------------------
// The cost of "or the row is shared with me" on a big table. It seeds 1,000,000 rows, so it is marked `#[ignore]`:
// `cargo test --release -p scaffoldry-server --test data_layer_row_shares_test -- --ignored --nocapture list_cost`
// ---------------------------------------------------------------------------------------------------------------------

mod cost {
    use super::*;
    use scaffoldry_engine::{AppManifest, AppTable, FieldSpec, FieldType};
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    const BIO: &str = "ws-bio-lab";
    const ROWS: i64 = 1_000_000;
    const RUNS: usize = 20;

    fn manifest(slug: &str) -> AppManifest {
        let mut advisor = FieldSpec::simple("advisor", "advisor", FieldType::Text, false, false);
        advisor.indexed = true;
        AppManifest {
            slug: slug.to_string(),
            title: "Advising".to_string(),
            description: String::new(),
            organization_code: "BIO".to_string(),
            department: "biology".to_string(),
            workspace_id: Some(BIO.to_string()),
            herm_capability_id: None,
            coordinates: None, sealed: false, integrations: Default::default(), kind: Default::default(),
            custom_domain: None,
            custom_domain_verified: false,
            tables: vec![AppTable {
                retired_fields: Vec::new(), key: None, unit_field: None, unit_required: false, audience: Vec::new(),
                id: "students".to_string(),
                name: "students".to_string(),
                slug: "students".to_string(),
                description: None,
                icon: None,
                fields: vec![FieldSpec::simple("name", "name", FieldType::Text, false, false), advisor],
                primary_field: Some("name".to_string()),
                sample_records: vec![],
                coordinates: None, sealed: false,
                row_place: Default::default(),
            }],
            relationships: vec![],
            views: vec![],
            ceds_mappings: Default::default(),
            pages: vec![],
            access: Default::default(),
        }
    }

    fn db_of<R: Send + 'static>(db: &common::TestDb, f: impl FnOnce(&mut postgres::Client) -> R + Send + 'static) -> R {
        db.run(f)
    }

    fn p95(runs: &mut [Duration]) -> (Duration, Duration) {
        runs.sort();
        (runs[runs.len() / 2], runs[(runs.len() * 95 / 100).min(runs.len() - 1)])
    }

    async fn time(app: &Router, uri: &str, token: &str) -> (Duration, Duration) {
        let (s, v) = rest(app, "GET", uri, token, None).await;
        assert_eq!(s, StatusCode::OK, "{v}");
        let mut runs = Vec::new();
        for _ in 0..RUNS {
            let started = Instant::now();
            let (s, v) = rest(app, "GET", uri, token, None).await;
            runs.push(started.elapsed());
            assert_eq!(s, StatusCode::OK, "{v}");
        }
        p95(&mut runs)
    }

    #[tokio::test]
    #[ignore]
    async fn list_cost_with_shares_on_a_million_rows() {
        files_dir();
        let db = common::TestDb::new();
        let state: Arc<ServerState> = db.state();
        state.seed_demo().expect("seed");
        let slug = format!("cost-{}", &Uuid::new_v4().to_string()[..8]);
        let ann = format!("ann-{}@state.edu", &slug[5..]);
        state.engine.write().unwrap().register_manifest(manifest(&slug)).unwrap();
        state.collaborators.write().unwrap().entry(BIO.to_string()).or_default().push(CollaboratorRecord {
            id: Uuid::new_v4().to_string(),
            workspace_id: BIO.to_string(),
            eppn: ann.clone(),
            name: ann.clone(),
            role: "editor".to_string(),
            scoped_affiliation: "faculty".to_string(),
            department: "biology".to_string(),
            added_at: String::new(),
            app_slug: String::new(),
        });
        let mut rule = rule("students");
        rule.except_roles = vec![];
        set_rules(&slug, vec![rule], &state);
        let (s, rows) = (slug.clone(), ann.clone());
        db.run(move |c| {
            c.execute(
                "INSERT INTO dataset_records (id, app_slug, data, ceds_mapping, is_ferpa_sensitive, table_id, created_at) \
                 SELECT $1::text || lpad(g::text, 8, '0'), $1::text, \
                        jsonb_build_object('_table_id', 'students', 'name', 'student-' || g, 'advisor', CASE WHEN g % 333 = 0 THEN $2::text ELSE 'adv' || (g % 300) || '@state.edu' END), \
                        '{}'::jsonb, false, 'students', TIMESTAMPTZ '2026-01-01 00:00:00+00' + g * interval '1 millisecond' FROM generate_series(1, $3::bigint) g",
                &[&s, &rows, &ROWS],
            )
            .unwrap();
            c.execute(
                "INSERT INTO record_index (app_slug, table_id, field, record_id, txt, ord) \
                 SELECT $1::text, 'students', 'advisor', $1::text || lpad(g::text, 8, '0'), CASE WHEN g % 333 = 0 THEN lower($2::text) ELSE 'adv' || (g % 300) || '@state.edu' END, 0 FROM generate_series(1, $3::bigint) g",
                &[&s, &rows, &ROWS],
            )
            .unwrap();
            c.batch_execute("ANALYZE dataset_records; ANALYZE record_index").unwrap();
        });
        let app = build_app_with_state(state.clone()).expect("router");
        let token = issue_test_token_and_user(&ann);
        let list = format!("/api/v1/apps/{slug}/tables/students/records?limit=50");
        let count = format!("/api/v1/apps/{slug}/tables/students/aggregate?function=count");
        let before = (time(&app, &list, &token).await, time(&app, &count, &token).await);
        // A share of 100 rows the reader does not advise, and 99 other readers' shares of 100 rows each, so the lookup holds 10,000 rows.
        let (s, who) = (slug.clone(), ann.clone());
        db.run(move |c| {
            c.execute("INSERT INTO datasets (id, app_slug, table_id) VALUES ('ds-cost', $1, 'students') ON CONFLICT DO NOTHING", &[&s]).unwrap();
            c.execute(
                "INSERT INTO row_shares (id, dataset_id, app_slug, table_id, records, grantee, access, fields, kind, justification, requested_by, expires_at, assessment, status) \
                 SELECT ('00000000-0000-0000-0000-' || lpad(n::text, 12, '0'))::uuid, (SELECT id FROM datasets WHERE app_slug = $1::text AND table_id = 'students'), $1::text, 'students', '{}', CASE WHEN n = 1 THEN $2::text ELSE 'other' || n || '@state.edu' END, 'read', '{name}', 'beyond', 'why', 'x@state.edu', NOW() + interval '30 days', '{}', 'Approved' \
                 FROM generate_series(1, 100) n",
                &[&s, &who],
            )
            .unwrap();
            c.execute(
                "INSERT INTO row_share_rows (share_id, grantee, app_slug, table_id, record_id) \
                 SELECT h.id, h.grantee, $1::text, 'students', $1::text || lpad((((h.n * 100 + g) * 9973) % 1000000 + 1)::text, 8, '0') FROM (SELECT id, grantee, right(id::text, 12)::int n FROM row_shares) h, generate_series(1, 100) g",
                &[&s],
            )
            .unwrap();
            c.batch_execute("ANALYZE row_shares; ANALYZE row_share_rows").unwrap();
        });
        let after = (time(&app, &list, &token).await, time(&app, &count, &token).await);
        // The statement the page runs, with and without the share, and what the planner does with it.
        {
            use scaffoldry_core::access::{Op, Operand, Pred};
            use scaffoldry_server::repository::{build_record_query, RecordQuery, ScopePlan, Shared, SqlParam};
            for (label, shared) in [("without a share", None), ("with a share", Some(Shared { grantee: ann.to_lowercase(), needs: vec![] }))] {
                let plan = ScopePlan { pred: Pred::Cmp { field: "advisor".into(), op: Op::Eq, value: Operand::Literal(json!(ann.to_lowercase())) }, indexed: manifest(&slug).tables[0].fields.clone(), labels: vec![], shared, audience: None };
                let (sql, params) = build_record_query(&slug, &RecordQuery { table_id: "students".into(), any: false, clauses: vec![], sort: None, after: None, limit: 51, scope: Some(plan), search: None, expr: None, unit: None });
                let explained = format!("EXPLAIN (ANALYZE, BUFFERS) {sql}");
                let text = db_of(&db, move |c| {
                    let boxed: Vec<Box<dyn postgres::types::ToSql + Sync>> = params
                        .into_iter()
                        .map(|p| -> Box<dyn postgres::types::ToSql + Sync> {
                            match p {
                                SqlParam::Text(t) => Box::new(t),
                                SqlParam::Texts(t) => Box::new(t),
                                SqlParam::Float(f) => Box::new(f),
                                SqlParam::Floats(f) => Box::new(f),
                                SqlParam::Int(i) => Box::new(i),
                            }
                        })
                        .collect();
                    let refs: Vec<&(dyn postgres::types::ToSql + Sync)> = boxed.iter().map(|b| b.as_ref()).collect();
                    c.query(&explained, &refs).unwrap().iter().map(|r| r.get::<_, String>(0)).collect::<Vec<_>>().join("\n")
                });
                println!("--- the page {label}:\n{text}");
            }
        }
        let shared = format!("{slug}{:08}", (101 * 9973) % 1_000_000 + 1);
        let (s, v) = rest(&app, "GET", &format!("/api/v1/apps/{slug}/tables/students/records/{shared}"), &token, None).await;
        assert_eq!(s, StatusCode::OK, "the shared row is read: {v}");
        let (_, list_body) = rest(&app, "GET", &list, &token, None).await;
        let (_, count_body) = rest(&app, "GET", &count, &token, None).await;
        println!("| Read | No share, median / p95 | With shares, median / p95 |");
        println!("| --- | --- | --- |");
        let ms = |d: Duration| d.as_secs_f64() * 1000.0;
        println!("| First page of 50 | {:.1} / {:.1} ms | {:.1} / {:.1} ms |", ms(before.0 .0), ms(before.0 .1), ms(after.0 .0), ms(after.0 .1));
        println!("| Count | {:.1} / {:.1} ms | {:.1} / {:.1} ms |", ms(before.1 .0), ms(before.1 .1), ms(after.1 .0), ms(after.1 .1));
        println!("list rows with the share: {}, count with the share: {}", list_body["records"].as_array().map_or(0, Vec::len), count_body["value"]);
        // The limits do not move: the existing scoped list is held to 200 ms.
        assert!(after.0 .1 < Duration::from_millis(200), "a first page took {:?} at p95", after.0 .1);
        assert!(after.1 .1 < Duration::from_millis(1000), "a count took {:?} at p95", after.1 .1);
        let (s, slug2) = (slug.clone(), slug);
        db.run(move |c| {
            for t in ["row_share_rows", "row_shares"] {
                c.execute(&format!("DELETE FROM {t}"), &[]).unwrap();
            }
            let _ = (&s, &slug2);
            c.execute("DELETE FROM record_index WHERE app_slug = $1", &[&s]).unwrap();
            c.execute("DELETE FROM dataset_records WHERE app_slug = $1", &[&s]).unwrap();
        });
    }
}
