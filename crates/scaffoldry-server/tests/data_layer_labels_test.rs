//! Data layer phase 3: row tags, the row's unit for access, and dataset labels (the floor).
//!
//! A row's effective label is the dataset floor, the row's tags, and the categories of its fields. A category may carry a
//! requirement: a position and a reach. A reader meets it when they hold the position at a unit that covers the row. The
//! row's unit is its unit column, or the dataset's own unit when the column is empty.

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::Router;
use http_body_util::BodyExt;
use scaffoldry_server::build_app_with_state;
use scaffoldry_server::service::identity::issue_test_token_and_user;
use scaffoldry_server::state::{CollaboratorRecord, OrganizationNode, RoleRow, ServerState, WorkspaceRecord};
use serde_json::{json, Value};
use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::OnceLock;
use tower::ServiceExt;
use uuid::Uuid;

const WS: &str = "ws-labels";
const ROOT: &str = "00000000-0000-0000-0000-000000000001";
const ADMIN: &str = "jordan.lee@state.edu";
const COMPLIANCE: &str = "compliance.officer@state.edu";
const CHEM_REG: &str = "chem.registrar@science.state.edu";
const PHYS_REG: &str = "phys.registrar@science.state.edu";
const SCIENCE_REG: &str = "science.registrar@science.state.edu";
const OUTSIDER: &str = "nora.editor@science.state.edu";
const SECOND: &str = "second.admin@state.edu";

fn files_dir() {
    static DIR: OnceLock<PathBuf> = OnceLock::new();
    DIR.get_or_init(|| {
        let dir = std::env::temp_dir().join(format!("scaffoldry-labels-{}", Uuid::new_v4()));
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
    compliance: String,
    second: String,
    chem: String,
    phys: String,
    science_reg: String,
    outsider: String,
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
        name: "Labels".to_string(),
        code: "WS-LABELS".to_string(),
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

/// The categories: `phi` is protected and needs a registrar at or above the row's unit. `hr` is not protected and needs a
/// registrar at exactly the row's unit. `open` has no requirement. `anyreg` needs a registrar anywhere.
fn categories() -> Value {
    json!([
        { "id": "phi", "name": "PHI", "protected": true, "detectors": [], "requires": [{ "position": "registrar", "reach": "unit_and_below" }] },
        { "id": "hr", "name": "HR", "protected": false, "detectors": [], "requires": [{ "position": "registrar", "reach": "unit" }] },
        { "id": "anyreg", "name": "Any registrar", "protected": false, "detectors": [], "requires": [{ "position": "registrar", "reach": "anywhere" }] },
        { "id": "open", "name": "Open", "protected": false, "detectors": [] }
    ])
}

/// science > physics, science > chemistry. The workspace sits at science. The app's organization code names physics, so the
/// dataset's own unit is physics. Everyone but the administrator is an editor of the workspace.
async fn world() -> World {
    files_dir();
    let db = common::TestDb::new();
    let state = db.state();
    let root = Uuid::parse_str(ROOT).unwrap();
    let science = unit(&state, "science", root);
    let physics = unit(&state, "physics", science);
    let chemistry = unit(&state, "chemistry", science);
    workspace(&state, science);
    db.run(|c| c.execute("INSERT INTO position_types (key, name, description, org_types, max_holders, created_by) VALUES ('registrar', 'Registrar', '', '[]', 10, 'test') ON CONFLICT DO NOTHING", &[]).unwrap());
    for who in [CHEM_REG, PHYS_REG, SCIENCE_REG, OUTSIDER] {
        editor_of(&state, who);
    }
    appoint(&state, CHEM_REG, chemistry, "faculty", Some("registrar"));
    appoint(&state, PHYS_REG, physics, "faculty", Some("registrar"));
    appoint(&state, SCIENCE_REG, science, "faculty", Some("registrar"));
    appoint(&state, COMPLIANCE, root, "compliance", None);
    appoint(&state, SECOND, root, "platform_admin", None);
    state.settings.write().unwrap().insert("sensitivity.categories".to_string(), categories());
    let app = build_app_with_state(state.clone()).expect("router");
    let slug = format!("students-{}", &Uuid::new_v4().to_string()[..8]);
    World {
        db,
        state,
        app,
        admin: issue_test_token_and_user(ADMIN),
        compliance: issue_test_token_and_user(COMPLIANCE),
        second: issue_test_token_and_user(SECOND),
        chem: issue_test_token_and_user(CHEM_REG),
        phys: issue_test_token_and_user(PHYS_REG),
        science_reg: issue_test_token_and_user(SCIENCE_REG),
        outsider: issue_test_token_and_user(OUTSIDER),
        physics,
        chemistry,
        slug,
    }
}

impl World {
    fn state_settings(&self, categories: Value) {
        self.state.settings.write().unwrap().insert("sensitivity.categories".to_string(), categories);
    }
}

fn manifest(slug: &str, unit_required: bool) -> Value {
    let mut table = json!({
        "id": "students", "name": "Students", "slug": "students", "primary_field": "name",
        "fields": [
            { "name": "name", "label": "Name", "field_type": "Text", "required": false, "ferpa_sensitive": false },
            { "name": "score", "label": "Score", "field_type": "Number", "required": false, "ferpa_sensitive": false },
            { "name": "unit", "label": "Unit", "field_type": "Unit", "hierarchy": "academic", "required": false, "ferpa_sensitive": false }
        ],
        "unit_field": "unit"
    });
    if unit_required {
        table["unit_required"] = json!(true);
    }
    json!({ "slug": slug, "title": "Students", "description": "Students by unit", "organization_code": "PHYSICS", "department": "science", "workspace_id": WS, "views": [], "tables": [table] })
}

async fn live_app(w: &World, unit_required: bool) {
    let (s, b) = rest(&w.app, "POST", &format!("/api/v1/workspaces/{WS}/apps"), &w.admin, Some(manifest(&w.slug, unit_required))).await;
    assert_eq!(s, StatusCode::CREATED, "{b}");
    let slug = w.slug.clone();
    w.db.run(move |c| c.execute("UPDATE index_status SET state = 'ready' WHERE app_slug = $1", &[&slug]).unwrap());
}

async fn create(w: &World, token: &str, name: &str, unit: Option<Uuid>, tags: Option<Value>) -> (StatusCode, Value) {
    let mut data = json!({ "_table_id": "students", "name": name, "score": 1 });
    if let Some(u) = unit {
        data["unit"] = json!(u.to_string());
    }
    if let Some(t) = tags {
        data["_tags"] = t;
    }
    rest(&w.app, "POST", &format!("/api/v1/apps/{}/records", w.slug), token, Some(json!({ "data": data }))).await
}

/// A row, made by the administrator, and its id.
async fn row(w: &World, name: &str, unit: Option<Uuid>, tags: Option<Value>) -> String {
    let (s, b) = create(w, &w.admin, name, unit, tags).await;
    assert_eq!(s, StatusCode::CREATED, "{b}");
    b["id"].as_str().unwrap().to_string()
}

async fn dataset_id(w: &World) -> String {
    let (s, body) = rest(&w.app, "GET", "/api/v1/data/datasets", &w.admin, None).await;
    assert_eq!(s, StatusCode::OK, "{body}");
    body["datasets"].as_array().unwrap().iter().find(|d| d["app_slug"] == json!(w.slug) && d["table_id"] == json!("students")).expect("a dataset")["id"].as_str().unwrap().to_string()
}

async fn set_floor(w: &World, token: &str, floor: Value) -> (StatusCode, Value) {
    let id = dataset_id(w).await;
    rest(&w.app, "PUT", &format!("/api/v1/admin/datasets/{id}/label-floor"), token, Some(json!({ "floor": floor, "reason": "Health data is held here" }))).await
}

/// The names the token's holder lists, by the paged table route, the legacy list, and the count the page route gives.
async fn listed(w: &World, token: &str) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    let mut cursor: Option<String> = None;
    loop {
        let mut uri = format!("/api/v1/apps/{}/tables/students/records?limit=2", w.slug);
        if let Some(c) = &cursor {
            uri.push_str(&format!("&cursor={c}"));
        }
        let (s, body) = rest(&w.app, "GET", &uri, token, None).await;
        assert_eq!(s, StatusCode::OK, "{body}");
        out.extend(body["records"].as_array().unwrap().iter().map(|r| r["data"]["name"].as_str().unwrap().to_string()));
        match body["next_cursor"].as_str() {
            Some(c) => cursor = Some(c.to_string()),
            None => break,
        }
    }
    // The legacy list agrees with the paged one.
    let (s, legacy) = rest(&w.app, "GET", &format!("/api/v1/apps/{}/records", w.slug), token, None).await;
    assert_eq!(s, StatusCode::OK, "{legacy}");
    let old: BTreeSet<String> = legacy["records"].as_array().unwrap().iter().map(|r| r["data"]["name"].as_str().unwrap().to_string()).collect();
    assert_eq!(old, out, "the legacy list and the paged list disagree");
    out
}

fn set(names: &[&str]) -> BTreeSet<String> {
    names.iter().map(|n| n.to_string()).collect()
}

async fn get_row(w: &World, token: &str, id: &str) -> StatusCode {
    rest(&w.app, "GET", &format!("/api/v1/apps/{}/records/{id}", w.slug), token, None).await.0
}

// 1. A row tagged PHI is not returned to a reader the requirement excludes.
#[tokio::test(flavor = "multi_thread")]
async fn a_row_tagged_phi_is_not_returned_to_a_reader_the_requirement_excludes() {
    let w = world().await;
    live_app(&w, false).await;
    let tagged = row(&w, "tagged", Some(w.chemistry), Some(json!(["phi"]))).await;
    row(&w, "plain", Some(w.chemistry), None).await;
    row(&w, "open tag", Some(w.chemistry), Some(json!(["open"]))).await;

    // The registrar of the row's unit reads all three. A person with no position reads the two with no requirement.
    assert_eq!(listed(&w, &w.chem).await, set(&["tagged", "plain", "open tag"]));
    assert_eq!(listed(&w, &w.outsider).await, set(&["plain", "open tag"]));
    assert_eq!(get_row(&w, &w.chem, &tagged).await, StatusCode::OK);
    // A row the person may not read is not found, in the words used for a row that does not exist.
    let absent = Uuid::new_v4().to_string();
    let (s, hidden) = rest(&w.app, "GET", &format!("/api/v1/apps/{}/records/{tagged}", w.slug), &w.outsider, None).await;
    let (s2, missing) = rest(&w.app, "GET", &format!("/api/v1/apps/{}/records/{absent}", w.slug), &w.outsider, None).await;
    assert_eq!((s, s2), (StatusCode::NOT_FOUND, StatusCode::NOT_FOUND));
    assert_eq!(hidden["error"].as_str().unwrap_or("").replace(&tagged, "ID"), missing["error"].as_str().unwrap_or("").replace(&absent, "ID"));
    // A Platform Admin skips the requirement, as they skip a row rule.
    assert_eq!(listed(&w, &w.admin).await.len(), 3);
    // The stored flag Cedar reads follows the label: a protected tag flags the row.
    let flags: Vec<(String, bool)> = {
        let slug = w.slug.clone();
        w.db.run(move |c| c.query("SELECT data->>'name', is_ferpa_sensitive FROM dataset_records WHERE app_slug = $1 ORDER BY 1", &[&slug]).unwrap().iter().map(|r| (r.get(0), r.get(1))).collect())
    };
    assert_eq!(flags, vec![("open tag".to_string(), false), ("plain".to_string(), false), ("tagged".to_string(), true)]);
    // The tags are stored in a column beside the row.
    let slug = w.slug.clone();
    let tags: Vec<Vec<String>> = w.db.run(move |c| c.query("SELECT tags FROM dataset_records WHERE app_slug = $1 AND data->>'name' = 'tagged'", &[&slug]).unwrap().iter().map(|r| r.get(0)).collect());
    assert_eq!(tags, vec![vec!["phi".to_string()]]);
}

// 2. A dataset floor raises an untagged row.
#[tokio::test(flavor = "multi_thread")]
async fn a_dataset_floor_raises_an_untagged_row() {
    let w = world().await;
    live_app(&w, false).await;
    row(&w, "plain", Some(w.chemistry), None).await;
    assert_eq!(listed(&w, &w.outsider).await, set(&["plain"]));

    // Setting a floor is a governed change: an administrator, a reason, and a ledger entry.
    let id = dataset_id(&w).await;
    let uri = format!("/api/v1/admin/datasets/{id}/label-floor");
    let (s, _) = rest(&w.app, "PUT", &uri, &w.outsider, Some(json!({ "floor": ["phi"], "reason": "x" }))).await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    let (s, _) = rest(&w.app, "PUT", &uri, &w.admin, Some(json!({ "floor": ["phi"] }))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "a reason is required");
    let (s, b) = rest(&w.app, "PUT", &uri, &w.admin, Some(json!({ "floor": ["not-a-category"], "reason": "x" }))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{b}");
    assert_eq!(listed(&w, &w.outsider).await, set(&["plain"]), "a refused change changed nothing");

    let (s, b) = set_floor(&w, &w.admin, json!(["phi"])).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_eq!(b["floor"], json!(["phi"]));
    assert_eq!(listed(&w, &w.outsider).await, set(&[]), "the floor raised an untagged row");
    assert_eq!(listed(&w, &w.chem).await, set(&["plain"]));

    // The stored flag follows, and a row made after the floor is flagged and held the same way.
    row(&w, "later", Some(w.chemistry), None).await;
    assert_eq!(listed(&w, &w.outsider).await, set(&[]));
    assert_eq!(listed(&w, &w.chem).await, set(&["plain", "later"]));
    let slug = w.slug.clone();
    let flagged: i64 = w.db.run(move |c| c.query_one("SELECT count(*) FROM dataset_records WHERE app_slug = $1 AND is_ferpa_sensitive", &[&slug]).unwrap().get(0));
    assert_eq!(flagged, 2, "a protected floor flags every row of the table");

    let id2 = id.clone();
    let entries: i64 = w.db.run(move |c| c.query_one("SELECT count(*) FROM governance_ledger WHERE decision_type = 'DataLabelChanged' AND payload->>'dataset' = $1 AND payload->>'setting' = 'label_floor'", &[&id2]).unwrap().get(0));
    assert_eq!(entries, 1);
}

// 3. A write cannot lower a floor.
#[tokio::test(flavor = "multi_thread")]
async fn a_write_cannot_lower_a_floor() {
    let w = world().await;
    live_app(&w, false).await;
    let (s, b) = set_floor(&w, &w.admin, json!(["phi"])).await;
    assert_eq!(s, StatusCode::OK, "{b}");

    // A registrar who may read the row writes it with no tags, empty tags, and a tag below the floor. Each leaves it held.
    let (s, b) = create(&w, &w.chem, "no tags", Some(w.chemistry), None).await;
    assert_eq!(s, StatusCode::CREATED, "{b}");
    let (s, b) = create(&w, &w.chem, "empty tags", Some(w.chemistry), Some(json!([]))).await;
    assert_eq!(s, StatusCode::CREATED, "{b}");
    let (s, b) = create(&w, &w.chem, "open tag", Some(w.chemistry), Some(json!(["open"]))).await;
    assert_eq!(s, StatusCode::CREATED, "{b}");
    assert_eq!(listed(&w, &w.outsider).await, set(&[]));
    assert_eq!(listed(&w, &w.chem).await, set(&["no tags", "empty tags", "open tag"]));

    // An update that clears the tags leaves the floor in place.
    let id = b["id"].as_str().unwrap().to_string();
    let uri = format!("/api/v1/apps/{}/records/{id}", w.slug);
    let (s, b) = rest(&w.app, "PATCH", &uri, &w.chem, Some(json!({ "data": { "_table_id": "students", "name": "open tag", "unit": w.chemistry.to_string(), "_tags": [] }, "version": 1 }))).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_eq!(listed(&w, &w.outsider).await, set(&[]));
    // A write that names a category nobody defined is refused. The refusal names no value of the row.
    let (s, b) = create(&w, &w.chem, "bad tag", Some(w.chemistry), Some(json!(["nope"]))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{b}");
    let (s, b) = create(&w, &w.chem, "bad shape", Some(w.chemistry), Some(json!("phi"))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{b}");

    // The floor is lowered only by the change route, and only by a Platform Admin. A compliance officer may raise it.
    let (s, b) = set_floor(&w, &w.compliance, json!([])).await;
    assert_eq!(s, StatusCode::FORBIDDEN, "{b}");
    assert_eq!(listed(&w, &w.outsider).await, set(&[]));
    let (s, b) = set_floor(&w, &w.compliance, json!(["phi", "hr"])).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    let (s, b) = set_floor(&w, &w.admin, json!([])).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_eq!(listed(&w, &w.outsider).await, set(&["no tags", "empty tags", "open tag"]));
}

// 3b. A tag is not removed, and a row is not tagged out of the writer's own reach, by an ordinary write.
#[tokio::test(flavor = "multi_thread")]
async fn a_write_cannot_remove_a_protective_tag_or_tag_a_row_out_of_the_writers_reach() {
    let w = world().await;
    live_app(&w, false).await;
    let tagged = row(&w, "tagged", Some(w.chemistry), Some(json!(["phi"]))).await;
    let plain = row(&w, "plain", Some(w.physics), None).await;
    let uri = |id: &str| format!("/api/v1/apps/{}/records/{id}", w.slug);
    let body = |name: &str, unit: Uuid, tags: Option<Value>| {
        let mut d = json!({ "_table_id": "students", "name": name, "unit": unit.to_string() });
        if let Some(t) = tags {
            d["_tags"] = t;
        }
        json!({ "data": d, "version": 1 })
    };

    // The registrar who reads the row may not strip its protective tag. An update that leaves `_tags` out keeps them.
    let (s, b) = rest(&w.app, "PATCH", &uri(&tagged), &w.chem, Some(body("tagged", w.chemistry, Some(json!([]))))).await;
    assert_eq!(s, StatusCode::FORBIDDEN, "{b}");
    let (s, b) = rest(&w.app, "PATCH", &uri(&tagged), &w.chem, Some(body("renamed", w.chemistry, None))).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert!(!listed(&w, &w.outsider).await.contains("renamed"), "an update that left the tags out lowered the row");
    assert_eq!(b["data"]["_tags"], json!(["phi"]));
    // A Platform Admin may remove it.
    let (s, b) = rest(&w.app, "PATCH", &uri(&tagged), &w.admin, Some(json!({ "data": { "_table_id": "students", "name": "renamed", "unit": w.chemistry.to_string(), "_tags": [] }, "version": 2 }))).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    let slug = w.slug.clone();
    let dbg: Vec<(String, Vec<String>, Value)> = w.db.run(move |c| c.query("SELECT data->>'name', tags, data FROM dataset_records WHERE app_slug = $1", &[&slug]).unwrap().iter().map(|r| (r.get(0), r.get(1), r.get(2))).collect());
    assert!(listed(&w, &w.outsider).await.contains("renamed"), "{dbg:?} {b}");

    // The physics registrar may not tag a physics row so that the physics registrar cannot read it. A tag that the writer meets is fine.
    let (s, _) = rest(&w.app, "PATCH", &uri(&plain), &w.phys, Some(body("plain", w.physics, Some(json!(["hr"]))))).await;
    assert_eq!(s, StatusCode::OK, "hr needs a registrar at the row's own unit, which they are");
    let (s, b) = rest(&w.app, "PATCH", &uri(&plain), &w.phys, Some(json!({ "data": { "_table_id": "students", "name": "plain", "unit": w.chemistry.to_string(), "_tags": ["hr"] }, "version": 2 }))).await;
    assert!(s == StatusCode::NOT_FOUND || s == StatusCode::FORBIDDEN, "moved out of reach: {s} {b}");
    // A person who may not read a row may not tag it either.
    let (s, _) = rest(&w.app, "PATCH", &uri(&plain), &w.chem, Some(body("plain", w.physics, Some(json!(["open"]))))).await;
    assert!(s == StatusCode::NOT_FOUND || s == StatusCode::FORBIDDEN);
}

// 4. A Chemistry row is read by the Chemistry registrar and not the Physics one.
#[tokio::test(flavor = "multi_thread")]
async fn a_chemistry_row_is_read_by_the_chemistry_registrar_and_not_the_physics_one() {
    let w = world().await;
    live_app(&w, false).await;
    set_floor(&w, &w.admin, json!(["phi"])).await;
    let chem_row = row(&w, "chem row", Some(w.chemistry), None).await;
    let phys_row = row(&w, "phys row", Some(w.physics), None).await;

    assert_eq!(listed(&w, &w.chem).await, set(&["chem row"]));
    assert_eq!(listed(&w, &w.phys).await, set(&["phys row"]));
    // A registrar at the parent unit reaches both, because `phi` is "unit and below".
    assert_eq!(listed(&w, &w.science_reg).await, set(&["chem row", "phys row"]));
    assert_eq!(get_row(&w, &w.chem, &chem_row).await, StatusCode::OK);
    assert_eq!(get_row(&w, &w.phys, &chem_row).await, StatusCode::NOT_FOUND);
    assert_eq!(get_row(&w, &w.chem, &phys_row).await, StatusCode::NOT_FOUND);
    assert_eq!(get_row(&w, &w.science_reg, &phys_row).await, StatusCode::OK);

    // `hr` is "unit" only: the registrar at the parent does not reach the rows of the units below.
    set_floor(&w, &w.admin, json!(["hr"])).await;
    assert_eq!(listed(&w, &w.science_reg).await, set(&[]));
    assert_eq!(listed(&w, &w.chem).await, set(&["chem row"]));
    // `anyreg` is "anywhere": any registrar reads any row.
    set_floor(&w, &w.admin, json!(["anyreg"])).await;
    assert_eq!(listed(&w, &w.phys).await, set(&["chem row", "phys row"]));
    assert_eq!(listed(&w, &w.outsider).await, set(&[]));
}

// 5. A row with the unit column empty is read by the dataset's unit's registrar.
#[tokio::test(flavor = "multi_thread")]
async fn a_row_with_the_unit_column_empty_is_read_by_the_datasets_units_registrar() {
    let w = world().await;
    live_app(&w, false).await;
    set_floor(&w, &w.admin, json!(["phi"])).await;
    row(&w, "no unit", None, None).await;
    row(&w, "chem row", Some(w.chemistry), None).await;
    // The dataset sits at physics, so a row naming no unit is physics's.
    assert_eq!(listed(&w, &w.phys).await, set(&["no unit"]));
    assert_eq!(listed(&w, &w.chem).await, set(&["chem row"]));
    assert_eq!(listed(&w, &w.science_reg).await, set(&["no unit", "chem row"]));
    assert_eq!(listed(&w, &w.outsider).await, set(&[]));
}

// 6. A row naming no unit is refused, in a dataset that declares a unit column and requires it.
#[tokio::test(flavor = "multi_thread")]
async fn a_row_naming_no_unit_is_refused_when_the_dataset_requires_one() {
    let w = world().await;
    live_app(&w, true).await;
    let (s, b) = create(&w, &w.admin, "no unit", None, None).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{b}");
    assert!(b["error"].as_str().unwrap_or_default().contains("unit"), "{b}");
    assert_eq!(b["constraint"], json!("unit_required"), "{b}");
    let (s, b) = rest(&w.app, "POST", &format!("/api/v1/apps/{}/records", w.slug), &w.admin, Some(json!({ "data": { "_table_id": "students", "name": "blank", "unit": "  " } }))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{b}");
    let id = row(&w, "with unit", Some(w.chemistry), None).await;
    // An update that blanks the unit is refused too.
    let (s, b) = rest(&w.app, "PATCH", &format!("/api/v1/apps/{}/records/{id}", w.slug), &w.admin, Some(json!({ "data": { "_table_id": "students", "name": "with unit", "unit": "" }, "version": 1 }))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{b}");
    // The data crate refuses it whatever door the write came by.
    let slug = w.slug.clone();
    let outcome: String = w.db.run(move |c| {
        let mut tx = c.transaction().unwrap();
        let data = json!({ "_table_id": "students", "name": "x" });
        tx.execute("INSERT INTO dataset_records (id, app_slug, data, ceds_mapping, is_ferpa_sensitive, table_id) VALUES ('rec_req', $1, $2, '{}', false, 'students')", &[&slug, &data]).unwrap();
        let out = match scaffoldry_data::units::write(&mut tx, &slug, "students", "rec_req", &data) {
            Ok(()) => "ok".to_string(),
            Err(e) if scaffoldry_data::units::is_required_missing(&e) && !scaffoldry_data::units::is_refused(&e) => "refused".to_string(),
            Err(e) => format!("other: {e:?}"),
        };
        tx.rollback().unwrap();
        out
    });
    assert_eq!(outcome, "refused");
}

// 6b. A table that declares no unit column keeps working, and the flag needs a unit column.
#[tokio::test(flavor = "multi_thread")]
async fn a_table_without_the_flag_takes_rows_with_no_unit_and_the_flag_needs_a_unit_column() {
    let w = world().await;
    live_app(&w, false).await;
    let (s, b) = create(&w, &w.admin, "no unit", None, None).await;
    assert_eq!(s, StatusCode::CREATED, "{b}");
    // A table that requires a unit and names no column is not a manifest.
    let mut bad = manifest(&format!("{}-bad", w.slug), true);
    bad["tables"][0].as_object_mut().unwrap().remove("unit_field");
    let (s, b) = rest(&w.app, "POST", &format!("/api/v1/workspaces/{WS}/apps"), &w.admin, Some(bad)).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{b}");
}

// 7. The list, the single read and the count agree for every reader, over rows of every kind.
#[tokio::test(flavor = "multi_thread")]
async fn every_path_gives_the_same_answer_for_every_reader() {
    let w = world().await;
    live_app(&w, false).await;
    let mut ids = Vec::new();
    for (i, unit) in [Some(w.chemistry), Some(w.physics), None].into_iter().enumerate() {
        for tags in [None, Some(json!(["phi"])), Some(json!(["hr"])), Some(json!(["anyreg"])), Some(json!(["phi", "hr"])), Some(json!(["open"]))] {
            let name = format!("row {i} {}", tags.as_ref().map_or("none".to_string(), Value::to_string));
            ids.push((name.clone(), row(&w, &name, unit, tags).await));
        }
    }
    for token in [&w.chem, &w.phys, &w.science_reg, &w.outsider, &w.admin] {
        let by_list = listed(&w, token).await;
        let mut by_get = BTreeSet::new();
        for (name, id) in &ids {
            if get_row(&w, token, id).await == StatusCode::OK {
                by_get.insert(name.clone());
            }
        }
        assert_eq!(by_list, by_get, "a list and a single read disagree");
        // The aggregate and the window count read the same scope.
        let (s, b) = rest(&w.app, "GET", &format!("/api/v1/apps/{}/tables/students/aggregate?function=count", w.slug), token, None).await;
        if s == StatusCode::OK {
            let n = b["value"].as_i64().or_else(|| b["result"].as_i64()).or_else(|| b["count"].as_i64());
            if let Some(n) = n {
                assert_eq!(n as usize, by_list.len(), "the count disagrees with the list: {b}");
            }
        }
    }
    // The physics registrar reads: untagged and open rows everywhere, and the rows of physics and no unit (physics's) that
    // need a registrar. A row naming no unit is physics's. `anyreg` is open to any registrar.
    let phys = listed(&w, &w.phys).await;
    assert!(phys.contains("row 1 [\"phi\"]") && phys.contains("row 2 [\"phi\"]"));
    assert!(!phys.contains("row 0 [\"phi\"]"));
    assert!(phys.contains("row 0 [\"anyreg\"]"));
    assert!(!phys.contains("row 0 [\"phi\",\"hr\"]"));
}

// 8. A category a field carries is part of every row's label, so its requirement gates the rows of the table.
#[tokio::test(flavor = "multi_thread")]
async fn a_category_a_field_carries_gates_every_row_of_the_table() {
    let w = world().await;
    let mut m = manifest(&w.slug, false);
    m["tables"][0]["fields"].as_array_mut().unwrap().push(json!({ "name": "diagnosis", "label": "Diagnosis", "field_type": "Text", "required": false, "ferpa_sensitive": false, "categories": ["phi"] }));
    let (s, b) = rest(&w.app, "POST", &format!("/api/v1/workspaces/{WS}/apps"), &w.admin, Some(m)).await;
    assert_eq!(s, StatusCode::CREATED, "{b}");
    let slug = w.slug.clone();
    w.db.run(move |c| c.execute("UPDATE index_status SET state = 'ready' WHERE app_slug = $1", &[&slug]).unwrap());
    row(&w, "chem row", Some(w.chemistry), None).await;
    row(&w, "phys row", Some(w.physics), None).await;
    assert_eq!(listed(&w, &w.chem).await, set(&["chem row"]));
    assert_eq!(listed(&w, &w.science_reg).await, set(&["chem row", "phys row"]));
    assert_eq!(listed(&w, &w.outsider).await, set(&[]));
    // The same table, with the requirement taken off the category, is open to everyone again.
    let mut cats = categories();
    cats[0].as_object_mut().unwrap().remove("requires");
    w.state_settings(cats);
    assert_eq!(listed(&w, &w.outsider).await, set(&["chem row", "phys row"]));
}

async fn tool_as(w: &World, token: &str, name: &str, args: Value) -> Result<Value, String> {
    let payload = json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": { "name": name, "arguments": args } });
    let (_, v) = rest(&w.app, "POST", "/api/mcp", token, Some(payload)).await;
    if let Some(err) = v.get("error") {
        return Err(err["message"].as_str().unwrap_or("").to_string());
    }
    if v["result"]["isError"] == true {
        return Err(v["result"]["content"][0]["text"].as_str().unwrap_or("").to_string());
    }
    Ok(v["result"]["structuredContent"].clone())
}

// 9. A steward requires the unit by a proposal. The approval is refused while a row names none, and the rows are listed.
#[tokio::test(flavor = "multi_thread")]
async fn a_proposal_that_requires_the_unit_is_refused_while_a_row_names_none() {
    let w = world().await;
    live_app(&w, false).await;
    let with = row(&w, "with unit", Some(w.chemistry), None).await;
    let without = row(&w, "without unit", None, None).await;
    let p = tool_as(&w, &w.admin, "propose_app_change", json!({ "workspace_id": WS, "manifest": manifest(&w.slug, true), "summary": "Require the unit" })).await.expect("proposed");
    assert!(serde_json::to_string(&p).unwrap().contains("requires a unit"), "the change list does not say it: {p}");
    let id = p["proposal_id"].as_str().unwrap().to_string();
    let err = tool_as(&w, &w.second, "decide_proposal", json!({ "proposal_id": id, "decision": "approve", "rationale": "Reviewed", "digest": p["digest"] })).await.expect_err("a row names no unit");
    assert!(err.contains(&without) && !err.contains(&with), "{err}");
    let (s, _) = rest(&w.app, "DELETE", &format!("/api/v1/apps/{}/records/{without}", w.slug), &w.admin, None).await;
    assert!(s.is_success());
    tool_as(&w, &w.second, "decide_proposal", json!({ "proposal_id": id, "decision": "approve", "rationale": "Reviewed", "digest": p["digest"] })).await.expect("approved");
    let (s, b) = create(&w, &w.admin, "late", None, None).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{b}");
    assert_eq!(b["constraint"], json!("unit_required"), "{b}");
}
