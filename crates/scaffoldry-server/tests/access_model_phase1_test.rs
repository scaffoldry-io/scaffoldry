//! Access model phase 1: close what fails open.

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use common::TestDb;
use http_body_util::BodyExt;
use scaffoldry_core::access::{Op, Operand, PersonAttr, Pred, RowOp, RowRule};
use scaffoldry_engine::{AppManifest, AppTable, FieldSpec, FieldType};
use scaffoldry_server::build_app_with_state;
use scaffoldry_server::service::identity::issue_test_token_and_user;
use scaffoldry_server::state::{CollaboratorRecord, OrganizationNode, RoleRow, ServerState, WorkspaceRecord};
use serde_json::{json, Value};
use tower::ServiceExt;
use uuid::Uuid;

const WORKSPACE: &str = "ws-physics-optics";
const OWNER: &str = "einstein@physics.state.edu";

async fn send(app: &axum::Router, method: &str, uri: &str, token: &str, body: Option<Value>) -> (StatusCode, Value) {
    let mut req = Request::builder().method(method).uri(uri).header("authorization", format!("Bearer {token}"));
    let body = match body {
        Some(v) => {
            req = req.header("content-type", "application/json");
            Body::from(v.to_string())
        }
        None => Body::empty(),
    };
    let response = app.clone().oneshot(req.body(body).unwrap()).await.unwrap();
    let status = response.status();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    (status, if bytes.is_empty() { Value::Null } else { serde_json::from_slice(&bytes).unwrap_or(Value::Null) })
}

async fn tool(app: &axum::Router, token: &str, name: &str, args: Value) -> (bool, Value) {
    let payload = json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": { "name": name, "arguments": args } });
    let (_, v) = send(app, "POST", "/api/mcp", token, Some(payload)).await;
    if let Some(err) = v.get("error") {
        return (true, err["message"].clone());
    }
    let result = &v["result"];
    if result["isError"] == true {
        return (true, result["content"][0]["text"].clone());
    }
    (false, result["structuredContent"].clone())
}

/// Rows are visible to the person named in `advisor`. The rule covers everyone, owners too.
fn advisor_rule() -> RowRule {
    RowRule {
        id: "r_advisor".into(),
        table_id: "students".into(),
        ops: vec![RowOp::Read],
        visible_when: Pred::Cmp { field: "advisor".into(), op: Op::Eq, value: Operand::Person(PersonAttr::Address) },
        except_roles: vec![],
        message: None,
    }
}

fn manifest(slug: &str) -> Value {
    json!({
        "slug": slug,
        "title": "Advising App",
        "description": "Student advising records",
        "organization_code": "PHYS",
        "department": "Physics",
        "workspace_id": WORKSPACE,
        "tables": [{
            "id": "students", "name": "Students", "slug": "students",
            "fields": [
                { "name": "name", "label": "Name", "field_type": "Text", "required": false, "ferpa_sensitive": false },
                { "name": "advisor", "label": "Advisor", "field_type": "Text", "required": false, "ferpa_sensitive": false, "indexed": true }
            ],
            "sample_records": []
        }],
        "views": [], "pages": [],
        "access": { "rows": [advisor_rule()], "columns": [] }
    })
}

/// An app with a row rule, approved through `app`, and three records. The owner advises two of them.
async fn app_with_a_rule(app: &axum::Router, owner: &str) -> String {
    let slug = format!("app-{}", &uuid::Uuid::new_v4().to_string()[..8]);
    let (err, proposed) = tool(app, owner, "propose_app_change", json!({ "workspace_id": WORKSPACE, "manifest": manifest(&slug), "summary": "Advising app" })).await;
    assert!(!err, "{proposed}");
    let (err, done) = tool(app, owner, "decide_proposal", json!({ "proposal_id": proposed["proposal_id"], "decision": "approve", "rationale": "Set up" })).await;
    assert!(!err, "{done}");
    for (name, advisor) in [("Alice", OWNER), ("Bob", OWNER), ("Charlie", "other@state.edu")] {
        let data = json!({ "_table_id": "students", "name": name, "advisor": advisor });
        let (s, body) = send(app, "POST", &format!("/api/v1/apps/{slug}/records"), owner, Some(json!({ "data": data }))).await;
        assert_eq!(s, StatusCode::CREATED, "{body}");
    }
    slug
}

async fn names(app: &axum::Router, slug: &str, token: &str) -> Vec<String> {
    let (s, body) = send(app, "GET", &format!("/api/v1/apps/{slug}/tables/students/records"), token, None).await;
    assert_eq!(s, StatusCode::OK, "{body}");
    let mut out: Vec<String> = body["records"].as_array().unwrap().iter().map(|r| r["data"]["name"].as_str().unwrap().to_string()).collect();
    out.sort();
    out
}

// 1a. A restarted server applies the rules the approved manifest holds. It does not show every row.
#[tokio::test]
async fn a_restarted_server_still_hides_the_rows_a_rule_hides() {
    let db = TestDb::new();
    let state = db.state();
    state.seed_demo().expect("seed");
    let a = build_app_with_state(state).expect("router");
    let owner = issue_test_token_and_user(OWNER);
    let slug = app_with_a_rule(&a, &owner).await;
    assert_eq!(names(&a, &slug, &owner).await, ["Alice", "Bob"], "the rule hides Charlie before the restart");

    let restarted = db.app();
    assert_eq!(names(&restarted, &slug, &owner).await, ["Alice", "Bob"], "and after it");
}

// 1b. A second instance that started before the approval hides the rows on its first request.
#[tokio::test]
async fn a_second_instance_hides_the_rows_on_its_first_request() {
    let db = TestDb::new();
    let state = db.state();
    state.seed_demo().expect("seed");
    let a = build_app_with_state(state).expect("router");
    let b = db.second_app();
    let owner = issue_test_token_and_user(OWNER);
    let slug = app_with_a_rule(&a, &owner).await;

    assert_eq!(names(&b, &slug, &owner).await, ["Alice", "Bob"], "the second instance applies the rule it never saw approved");
}

// ---------------------------------------------------------------------------------------------
// Guards: every check goes through `decide`, a forbid always wins, sensitivity reaches Cedar.
// ---------------------------------------------------------------------------------------------

const ADMIN: &str = "jordan.lee@state.edu";
const STUDENT: &str = "student.p1@state.edu";
const STAFF: &str = "marcus.vance@state.edu";
const UNIT_ADMIN: &str = "einstein@state.edu";

fn unit(state: &ServerState, name: &str, parent: Option<Uuid>) -> Uuid {
    let id = Uuid::new_v4();
    let node = OrganizationNode { id, parent_id: parent, name: name.to_string(), code: name.to_uppercase(), org_type: "Unit".to_string() };
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

/// A departmental workspace in `unit`, with these members as (address, role).
fn workspace(state: &ServerState, id: &str, unit: Option<Uuid>, members: &[(&str, &str)]) {
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
        allowed_affiliations: vec!["faculty".into(), "staff".into(), "student".into()],
        data_classification: "restricted".to_string(),
        created_at: now.clone(),
        organization_id: unit,
    };
    state.workspaces.write().unwrap().insert(id.to_string(), ws.clone());
    state.repository.upsert_workspace(&ws).unwrap();
    let collabs: Vec<CollaboratorRecord> = members
        .iter()
        .map(|(eppn, role)| CollaboratorRecord {
            id: Uuid::new_v4().to_string(),
            workspace_id: id.to_string(),
            eppn: eppn.to_string(),
            name: eppn.to_string(),
            role: role.to_string(),
            scoped_affiliation: "member".to_string(),
            department: "biology".to_string(),
            added_at: now.clone(),
            app_slug: String::new(),
        })
        .collect();
    for c in &collabs {
        state.repository.upsert_collaborator(c).unwrap();
    }
    state.collaborators.write().unwrap().insert(id.to_string(), collabs);
}

/// An app in the workspace with one protected field.
fn sensitive_app(state: &ServerState, slug: &str, ws: &str) {
    let mut ssn = FieldSpec::simple("ssn", "SSN", FieldType::Text, false, true);
    ssn.categories = vec![];
    let manifest = AppManifest {
        slug: slug.to_string(),
        title: "Records".to_string(),
        description: String::new(),
        organization_code: "BIO".to_string(),
        department: "biology".to_string(),
        workspace_id: Some(ws.to_string()),
        herm_capability_id: None,
        custom_domain: None,
        custom_domain_verified: false,
        tables: vec![AppTable {
            id: "people".to_string(),
            name: "people".to_string(),
            slug: "people".to_string(),
            description: None,
            icon: None,
            fields: vec![FieldSpec::simple("name", "Name", FieldType::Text, false, false), ssn],
            primary_field: Some("name".to_string()),
            sample_records: vec![],
        }],
        relationships: vec![],
        views: vec![],
        ceds_mappings: Default::default(),
        pages: vec![],
        access: Default::default(),
    };
    state.engine.write().unwrap().register_manifest(manifest).unwrap();
}

async fn put_guard(app: &axum::Router, ws: &str, rule: Value) {
    let (s, body) = send(app, "PUT", &format!("/api/v1/workspaces/{ws}/guards"), &issue_test_token_and_user(ADMIN), Some(json!({ "rules": [rule], "reason": "Test guard" }))).await;
    assert_eq!(s, StatusCode::OK, "{body}");
}

async fn lists(app: &axum::Router, who: &str, ws: &str) -> bool {
    let (s, body) = send(app, "GET", "/api/v1/workspaces", &issue_test_token_and_user(who), None).await;
    assert_eq!(s, StatusCode::OK, "{body}");
    body.as_array().map(|a| a.iter().any(|w| w["workspace"]["id"] == ws || w["id"] == ws)).unwrap_or(false)
}

// 2. A guard hides a workspace from the list and refuses a direct get, for a caller outside its units.
#[tokio::test]
async fn a_unit_guard_hides_the_workspace_from_the_list_and_refuses_a_direct_get() {
    let db = TestDb::new();
    let state = db.state();
    let college = unit(&state, "science", None);
    workspace(&state, "ws-p1-units", None, &[(STUDENT, "viewer"), (STAFF, "viewer")]);
    appoint(&state, STAFF, college, "staff");
    let app = build_app_with_state(state).expect("router");

    // Without the guard both members see the workspace.
    assert!(lists(&app, STUDENT, "ws-p1-units").await, "a member sees it before the guard");
    put_guard(&app, "ws-p1-units", json!({ "template": "deny_access_outside_units", "unit_ids": [college.to_string()] })).await;

    assert!(lists(&app, STAFF, "ws-p1-units").await, "a person inside the unit still sees it");
    assert!(!lists(&app, STUDENT, "ws-p1-units").await, "a person outside the unit does not see it in the list");
    let (s, body) = send(&app, "GET", "/api/v1/workspaces/ws-p1-units", &issue_test_token_and_user(STUDENT), None).await;
    assert_eq!(s, StatusCode::FORBIDDEN, "and cannot get it directly: {body}");
    let (s, body) = send(&app, "GET", "/api/v1/workspaces/ws-p1-units", &issue_test_token_and_user(STAFF), None).await;
    assert_eq!(s, StatusCode::OK, "{body}");
}

// 3. A guard's forbid is final. A unit administrator is let in by a Cedar permit, and a guard that forbids denies them.
#[tokio::test]
async fn a_guard_that_forbids_read_app_denies_a_unit_administrator() {
    let db = TestDb::new();
    let state = db.state();
    let science = unit(&state, "science", None);
    let arts = unit(&state, "arts", None);
    workspace(&state, "ws-p1-admin", Some(science), &[(STAFF, "viewer")]);
    sensitive_app(&state, "p1-admin-app", "ws-p1-admin");
    appoint(&state, UNIT_ADMIN, science, "unit_admin");
    let app = build_app_with_state(state).expect("router");
    let path = "/api/v1/apps/p1-admin-app/schema";

    let (s, body) = send(&app, "GET", path, &issue_test_token_and_user(UNIT_ADMIN), None).await;
    assert_eq!(s, StatusCode::OK, "a unit administrator of the workspace's unit reads its apps: {body}");

    put_guard(&app, "ws-p1-admin", json!({ "template": "deny_access_outside_units", "unit_ids": [arts.to_string()] })).await;
    let (s, body) = send(&app, "GET", path, &issue_test_token_and_user(UNIT_ADMIN), None).await;
    assert_eq!(s, StatusCode::FORBIDDEN, "the guard forbids them, and a forbid wins: {body}");
}

// 4. Sensitivity reaches Cedar: the guard denies a student a protected app and allows staff.
#[tokio::test]
async fn the_sensitive_data_guard_denies_a_student_and_allows_staff() {
    let db = TestDb::new();
    let state = db.state();
    workspace(&state, "ws-p1-sensitive", None, &[(STUDENT, "viewer"), (STAFF, "viewer")]);
    sensitive_app(&state, "p1-sensitive-app", "ws-p1-sensitive");
    let app = build_app_with_state(state).expect("router");
    let path = "/api/v1/apps/p1-sensitive-app/schema";

    let (s, body) = send(&app, "GET", path, &issue_test_token_and_user(STUDENT), None).await;
    assert_eq!(s, StatusCode::OK, "before the guard a member reads the app: {body}");
    put_guard(&app, "ws-p1-sensitive", json!({ "template": "deny_sensitive_unless_affiliation", "affiliations": ["staff"] })).await;

    let (s, body) = send(&app, "GET", path, &issue_test_token_and_user(STUDENT), None).await;
    assert_eq!(s, StatusCode::FORBIDDEN, "a student is refused an app with a protected field: {body}");
    let (s, body) = send(&app, "GET", path, &issue_test_token_and_user(STAFF), None).await;
    assert_eq!(s, StatusCode::OK, "staff are allowed: {body}");
}

// 5. A guard whose stored source no longer compiles fails the request. It does not widen access.
#[tokio::test]
async fn a_guard_that_no_longer_compiles_fails_the_request() {
    let db = TestDb::new();
    let state = db.state();
    workspace(&state, "ws-p1-broken", None, &[(STUDENT, "viewer")]);
    let app = build_app_with_state(state).expect("router");
    put_guard(&app, "ws-p1-broken", json!({ "template": "deny_export_unless_affiliation", "affiliations": ["staff"] })).await;

    // The stored rule is damaged. A server that starts afterwards reads it as it is.
    db.run(|c| c.batch_execute("UPDATE workspace_guards SET rules = '[{\"template\": \"no_such_template\"}]'::jsonb, compiled = ''").unwrap());
    let restarted = db.app();
    let (s, body) = send(&restarted, "GET", "/api/v1/workspaces/ws-p1-broken", &issue_test_token_and_user(STUDENT), None).await;
    assert_eq!(s, StatusCode::INTERNAL_SERVER_ERROR, "the request fails, it is not allowed and not a plain denial: {body}");
    assert_eq!(body["code"], "internal", "{body}");
}

// ---------------------------------------------------------------------------------------------
// The wrappers that skipped guards are gone, and nothing calls them.
// ---------------------------------------------------------------------------------------------

/// Methods that decided without loading a workspace's guards. The engine no longer has them.
const REMOVED_WRAPPERS: &[&str] = &["authorize_workspace_action", "authorize_institutional_action", "authorize_departmental_action", "authorize_record_action"];

/// The lines where `names` are called. The source is read as tokens: a comment or a string is skipped, and a call
/// split over several lines still counts, because only the next token after the name matters.
fn calls_to(source: &str, names: &[&str]) -> Vec<usize> {
    let bytes: Vec<char> = source.chars().collect();
    let mut found = Vec::new();
    let (mut i, mut line) = (0, 1);
    while i < bytes.len() {
        let c = bytes[i];
        if c == '\n' {
            line += 1;
            i += 1;
        } else if c == '/' && bytes.get(i + 1) == Some(&'/') {
            while i < bytes.len() && bytes[i] != '\n' {
                i += 1;
            }
        } else if c == '/' && bytes.get(i + 1) == Some(&'*') {
            i += 2;
            while i + 1 < bytes.len() && !(bytes[i] == '*' && bytes[i + 1] == '/') {
                if bytes[i] == '\n' {
                    line += 1;
                }
                i += 1;
            }
            i += 2;
        } else if c == '"' {
            i += 1;
            while i < bytes.len() && bytes[i] != '"' {
                if bytes[i] == '\\' {
                    i += 1;
                }
                if bytes.get(i) == Some(&'\n') {
                    line += 1;
                }
                i += 1;
            }
            i += 1;
        } else if c.is_alphabetic() || c == '_' {
            let start = i;
            while i < bytes.len() && (bytes[i].is_alphanumeric() || bytes[i] == '_') {
                i += 1;
            }
            let word: String = bytes[start..i].iter().collect();
            if names.contains(&word.as_str()) {
                let mut j = i;
                while j < bytes.len() && bytes[j].is_whitespace() {
                    j += 1;
                }
                if bytes.get(j) == Some(&'(') || (bytes.get(j) == Some(&':') && bytes.get(j + 1) == Some(&':')) {
                    found.push(line);
                }
            }
        } else {
            i += 1;
        }
    }
    found
}

fn rust_files(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
    for entry in std::fs::read_dir(dir).unwrap().flatten() {
        let p = entry.path();
        if p.is_dir() {
            rust_files(&p, out);
        } else if p.extension().is_some_and(|e| e == "rs") {
            out.push(p);
        }
    }
}

// 8a. The scan sees a removed wrapper called across three lines, and ignores one named in a comment or a string.
#[test]
fn the_source_scan_sees_a_call_split_over_three_lines() {
    let split = "fn f() {\n    engine\n        .authorize_workspace_action\n        (&input);\n}\n";
    assert_eq!(calls_to(split, REMOVED_WRAPPERS), vec![3], "the call is found on the line of its name");
    let quiet = "// authorize_record_action(x)\n/* authorize_workspace_action(y) */\nlet s = \"authorize_institutional_action(z)\";\n";
    assert!(calls_to(quiet, REMOVED_WRAPPERS).is_empty(), "a comment or a string is not a call");
}

// 8b. No source file of any crate calls a removed wrapper.
#[test]
fn no_source_calls_a_wrapper_that_skips_guards() {
    let crates = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    let mut files = Vec::new();
    for krate in ["scaffoldry-server", "scaffoldry-core", "scaffoldry-engine", "scaffoldry-policy"] {
        rust_files(&crates.join(krate).join("src"), &mut files);
    }
    assert!(files.len() > 20, "the scan found the source: {} files", files.len());
    let mut offenders = Vec::new();
    for f in files {
        let text = std::fs::read_to_string(&f).unwrap();
        for line in calls_to(&text, REMOVED_WRAPPERS) {
            offenders.push(format!("{}:{line}", f.display()));
        }
    }
    assert!(offenders.is_empty(), "these call a wrapper that skips workspace guards: {offenders:?}");
}
