//! Data layer phase 2e: request access from a catalog entry.
//!
//! A request on a catalog entry is an access model phase 13 request. It carries the fields the dataset's form lists, goes to
//! the people who hold the dataset's steward position, reminds them through the inbox while it waits, and shows the
//! requester where it is. The outcome is a ledger entry, and a refusal carries a reason the person can read.

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::Router;
use http_body_util::BodyExt;
use scaffoldry_server::build_app_with_state;
use scaffoldry_server::service::identity::issue_test_token_and_user;
use scaffoldry_server::state::{OrganizationNode, PositionType, RoleRow, SharedState};
use serde_json::{json, Value};
use tower::ServiceExt;
use uuid::Uuid;

const ROOT: &str = "00000000-0000-0000-0000-000000000001";
const ADMIN: &str = "jordan.lee@state.edu";
const MEMBER: &str = "pat.lead@physics.state.edu";
const OTHER: &str = "sam.other@physics.state.edu";
const OUTSIDER: &str = "olive.outsider@elsewhere.edu";
const CHAIR: &str = "chair.person@science.state.edu";
const DEPUTY: &str = "second.chair@science.state.edu";

struct World {
    db: common::TestDb,
    state: SharedState,
    app: Router,
    enroll: String,
    payroll: String,
}

async fn rest(app: &Router, method: &str, uri: &str, who: &str, body: Option<Value>) -> (StatusCode, Value) {
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

fn unit(state: &SharedState, name: &str, code: &str) -> Uuid {
    let id = Uuid::new_v4();
    let node = OrganizationNode { id, parent_id: Some(Uuid::parse_str(ROOT).unwrap()), name: name.to_string(), code: code.to_string(), org_type: "Department".to_string(), hierarchy_key: "academic".to_string(), synthetic: false };
    state.organizations.write().unwrap().insert(id, node.clone());
    state.persist_organization(&node).unwrap();
    id
}

fn appoint(state: &SharedState, eppn: &str, unit: Uuid, position: Option<&str>) {
    state
        .persist_role(&RoleRow {
            id: Uuid::new_v4(),
            person_id: Uuid::new_v4(),
            eppn: eppn.to_string(),
            organization_id: unit,
            role_title: position.unwrap_or("faculty").to_string(),
            scoped_affiliation: "faculty".to_string(),
            is_primary: false,
            source: "test".to_string(),
            position_key: position.map(str::to_string),
        })
        .unwrap();
}

fn position(state: &SharedState, key: &str, name: &str) {
    state
        .persist_position_type(&PositionType {
            key: key.to_string(),
            name: name.to_string(),
            description: String::new(),
            org_types: vec!["Department".to_string()],
            max_holders: 2,
            retired_at: None,
            created_by: "test".to_string(),
            created_at: chrono::Utc::now().to_rfc3339(),
        })
        .unwrap();
}

fn settings(db: &common::TestDb) {
    db.run(|c| {
        for (k, v) in [
            ("mail.enabled", json!(true)),
            ("mail.smtp", json!({ "host": "mail.test", "port": 465, "security": "tls", "from": "desk@state.edu" })),
            ("platform.base_url", json!("https://desk.state.edu")),
        ] {
            c.execute("INSERT INTO platform_settings (key, value, updated_by) VALUES ($1, $2, 'test') ON CONFLICT (key) DO UPDATE SET value = EXCLUDED.value", &[&k, &v]).unwrap();
        }
    });
}

async fn world() -> World {
    let db = common::TestDb::new();
    settings(&db);
    let state = db.state();
    state.seed_demo().expect("seed");
    let chem = unit(&state, "Chemistry Department", "CHEM");
    let hidden = unit(&state, "Hidden Office", "HIDOFF");
    let physics = unit(&state, "Physics Department", "PHYS");
    let _ = hidden;
    position(&state, "chair", "Department chair");
    position(&state, "dean", "Dean");
    appoint(&state, MEMBER, physics, None);
    appoint(&state, OTHER, physics, None);
    appoint(&state, CHAIR, chem, Some("chair"));
    appoint(&state, DEPUTY, chem, Some("chair"));
    let app = build_app_with_state(state.clone()).expect("router");
    let tag = &Uuid::new_v4().to_string()[..8];
    let (enroll, payroll) = (format!("enroll-{tag}"), format!("payroll-{tag}"));
    let table = |name: &str| {
        json!({ "id": "t", "name": name, "slug": "t", "primary_field": "title", "fields": [{ "name": "title", "label": "Title", "field_type": "Text", "required": true, "ferpa_sensitive": false }] })
    };
    for (slug, title, code, name) in [(&enroll, "Enrollment", "CHEM", "Course Enrollment"), (&payroll, "Payroll", "HIDOFF", "Pay Bands")] {
        let m = json!({ "slug": slug, "title": title, "description": format!("{title} app"), "organization_code": code, "department": "registrar", "workspace_id": "ws-bio-lab", "views": [], "tables": [table(name)] });
        let (s, b) = rest(&app, "POST", "/api/v1/workspaces/ws-bio-lab/apps", ADMIN, Some(m)).await;
        assert_eq!(s, StatusCode::CREATED, "{b}");
        let (s, b) = rest(&app, "POST", &format!("/api/v1/apps/{slug}/tables/t/records"), ADMIN, Some(json!({ "title": "A row" }))).await;
        assert_eq!(s, StatusCode::CREATED, "{b}");
    }
    World { db, state, app, enroll, payroll }
}

async fn dataset_id(w: &World, app: &str) -> String {
    let (s, body) = rest(&w.app, "GET", "/api/v1/data/datasets", ADMIN, None).await;
    assert_eq!(s, StatusCode::OK, "{body}");
    body["datasets"].as_array().unwrap().iter().find(|d| d["app_slug"] == json!(app)).unwrap_or_else(|| panic!("no dataset of {app}"))["id"].as_str().unwrap().to_string()
}

async fn policy(w: &World, id: &str, body: Value) {
    let (s, b) = rest(&w.app, "PUT", &format!("/api/v1/admin/datasets/{id}/discoverability"), ADMIN, Some(body)).await;
    assert_eq!(s, StatusCode::OK, "{b}");
}

fn inbox(w: &World, who: &'static str, kind: &'static str) -> Vec<(String, String)> {
    w.db.run(move |c| c.query("SELECT title, body FROM notifications WHERE lower(recipient) = lower($1) AND kind = $2 ORDER BY created_at", &[&who, &kind]).unwrap().iter().map(|r| (r.get(0), r.get(1))).collect())
}

fn mails_to(w: &World, who: &'static str) -> usize {
    w.db.run(move |c| c.query_one("SELECT count(*) FROM email_outbox WHERE lower(to_address) = lower($1)", &[&who]).unwrap().get::<_, i64>(0) as usize)
}

fn ledger(w: &World, request: &str) -> Vec<(String, String)> {
    let request = request.to_string();
    w.db.run(move |c| {
        c.query("SELECT payload->>'outcome', rationale FROM governance_ledger WHERE decision_type = 'AccessRequestDecided' AND payload->>'request_id' = $1 ORDER BY sequence", &[&request])
            .unwrap()
            .iter()
            .map(|r| (r.get(0), r.get(1)))
            .collect()
    })
}

const FORM: [&str; 4] = ["reason", "project", "duration", "agreement_reference"];

fn ask(dataset: &str) -> Value {
    json!({ "object_kind": "dataset", "object_id": dataset, "role": "viewer", "reason": "Retention study", "project": "Persistence 2027", "days": 14, "agreement_reference": "DUA-2026-114" })
}

async fn can_read(w: &World, who: &str, app: &str) -> bool {
    rest(&w.app, "GET", &format!("/api/v1/apps/{app}/tables/t/records"), who, None).await.0 == StatusCode::OK
}

// 1. A request on a catalog entry carries the form's fields, goes to the steward position's holders, tells them through the
// inbox and email, reminds them when it waits, and shows the requester where it is.
#[tokio::test]
async fn a_catalog_request_carries_the_form_goes_to_the_steward_and_is_reminded() {
    let w = world().await;
    let ds = dataset_id(&w, &w.enroll).await;
    policy(&w, &ds, json!({ "steward_position": "chair", "request_fields": FORM, "reason": "A request form" })).await;
    assert!(!can_read(&w, MEMBER, &w.enroll).await, "the requester has no access to start with");

    let (s, b) = rest(&w.app, "POST", "/api/v1/access/requests", MEMBER, Some(ask(&ds))).await;
    assert_eq!(s, StatusCode::CREATED, "{b}");
    let id = b["id"].as_str().unwrap().to_string();
    assert_eq!(b["status"], "Pending", "{b}");
    assert_eq!(b["route"], "approver");
    assert_eq!(b["dataset_id"], json!(ds));
    assert_eq!(b["object_kind"], "app", "the request names the dataset's app");
    assert_eq!(b["object_id"], json!(w.enroll));
    assert_eq!(b["answers"], json!({ "reason": "Retention study", "project": "Persistence 2027", "duration": 14, "agreement_reference": "DUA-2026-114" }));
    assert_eq!(b["days"], 14);
    // It shows the requester where it is and who has it.
    assert_eq!(b["waiting_on"], json!([CHAIR, DEPUTY]));
    assert_eq!(b["routing"]["to"], "steward");
    assert_eq!(b["routing"]["position"], "chair");
    assert!(b["routing"]["sentence"].as_str().unwrap().contains("Department chair"), "{b}");
    assert!(b["object"].as_str().unwrap().contains("Course Enrollment"), "{b}");
    let (_, mine) = rest(&w.app, "GET", "/api/v1/me/access/requests", MEMBER, None).await;
    assert_eq!(mine["requests"][0]["routing"]["to"], "steward");

    // Only the steward's holders may decide it. Not a Platform Admin, not the requester, not a stranger.
    for (who, why) in [(ADMIN, "a Platform Admin is not the steward"), (MEMBER, "the requester"), (OTHER, "a colleague")] {
        let (s, _) = rest(&w.app, "POST", &format!("/api/v1/access/requests/{id}/decide"), who, Some(json!({ "decision": "approve", "note": "ok" }))).await;
        assert_eq!(s, StatusCode::FORBIDDEN, "{why}");
    }
    let (_, todo) = rest(&w.app, "GET", "/api/v1/me/access/requests/to-decide", CHAIR, None).await;
    assert_eq!(todo["requests"].as_array().unwrap().len(), 1, "{todo}");
    let (_, todo) = rest(&w.app, "GET", "/api/v1/me/access/requests/to-decide", ADMIN, None).await;
    assert!(todo["requests"].as_array().unwrap().is_empty(), "{todo}");

    // The steward's holders are told once, in the inbox and by email. The requester is not.
    for who in [CHAIR, DEPUTY] {
        let told = inbox(&w, who, "access_request_waiting");
        assert_eq!(told.len(), 1, "{who}: {told:?}");
        assert!(told[0].0.starts_with("Access requested") && told[0].0.contains("Course Enrollment"), "{told:?}");
        assert_eq!(mails_to(&w, who), 1, "{who} is mailed");
    }
    assert!(inbox(&w, MEMBER, "access_request_waiting").is_empty());

    // The reminder: nothing before three days, one after, and not again until three days more.
    let now = chrono::Utc::now();
    let st = w.state.clone();
    let remind = |at: chrono::DateTime<chrono::Utc>| {
        let st = st.clone();
        tokio::task::spawn_blocking(move || scaffoldry_server::service::access_requests::remind_due(&st, at).unwrap())
    };
    assert_eq!(remind(now + chrono::Duration::days(2)).await.unwrap(), 0);
    assert_eq!(remind(now + chrono::Duration::days(4)).await.unwrap(), 1);
    assert_eq!(remind(now + chrono::Duration::days(5)).await.unwrap(), 0, "once for each wait");
    assert_eq!(remind(now + chrono::Duration::days(8)).await.unwrap(), 1, "again after three more days");
    let told = inbox(&w, CHAIR, "access_request_waiting");
    assert_eq!(told.len(), 3, "{told:?}");
    assert!(told[1].0.starts_with("Still waiting"), "{told:?}");
    assert!(inbox(&w, MEMBER, "access_request_waiting").is_empty());

    // The steward approves. The outcome is a ledger entry and a dated grant that works.
    let (s, b) = rest(&w.app, "POST", &format!("/api/v1/access/requests/{id}/decide"), CHAIR, Some(json!({ "decision": "approve", "note": "The agreement is on file" }))).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_eq!(b["status"], "Approved");
    assert_eq!(b["decision_note"], "The agreement is on file");
    assert!(b["ends_at"].is_string(), "{b}");
    assert_eq!(ledger(&w, &id), vec![("Approved".to_string(), "The agreement is on file".to_string())]);
    assert!(can_read(&w, MEMBER, &w.enroll).await, "the grant lets the requester read");
    assert!(!can_read(&w, OTHER, &w.enroll).await);
    // Once decided, it is no longer reminded.
    assert_eq!(remind(now + chrono::Duration::days(30)).await.unwrap(), 0);
}

// 2. A refusal carries the steward's reason, which the requester reads on the request and in the ledger.
#[tokio::test]
async fn a_refusal_carries_a_reason_the_requester_can_read() {
    let w = world().await;
    let ds = dataset_id(&w, &w.enroll).await;
    policy(&w, &ds, json!({ "steward_position": "chair", "reason": "Name the steward" })).await;
    let (s, b) = rest(&w.app, "POST", "/api/v1/access/requests", MEMBER, Some(json!({ "object_kind": "dataset", "object_id": ds, "reason": "Curious" }))).await;
    assert_eq!(s, StatusCode::CREATED, "{b}");
    let id = b["id"].as_str().unwrap().to_string();
    // A decision needs a reason.
    let (s, b) = rest(&w.app, "POST", &format!("/api/v1/access/requests/{id}/decide"), CHAIR, Some(json!({ "decision": "reject" }))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{b}");
    assert!(b.to_string().contains("note"), "{b}");
    let (s, b) = rest(&w.app, "POST", &format!("/api/v1/access/requests/{id}/decide"), CHAIR, Some(json!({ "decision": "reject", "note": "Ask for one course, not all of them" }))).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_eq!(b["status"], "Rejected");
    let (_, mine) = rest(&w.app, "GET", "/api/v1/me/access/requests", MEMBER, None).await;
    assert_eq!(mine["requests"][0]["status"], "Rejected");
    assert_eq!(mine["requests"][0]["decision_note"], "Ask for one course, not all of them");
    assert_eq!(ledger(&w, &id), vec![("Rejected".to_string(), "Ask for one course, not all of them".to_string())]);
    assert!(!can_read(&w, MEMBER, &w.enroll).await);
}

// 3. With no steward position, or nobody holding it, the existing routing applies and the status says so.
#[tokio::test]
async fn with_no_steward_holder_the_existing_routing_applies_and_the_status_says_so() {
    let w = world().await;
    let ds = dataset_id(&w, &w.enroll).await;
    let (s, none) = rest(&w.app, "POST", "/api/v1/access/requests", MEMBER, Some(json!({ "object_kind": "dataset", "object_id": ds, "reason": "No steward set" }))).await;
    assert_eq!(s, StatusCode::CREATED, "{none}");
    assert_eq!(none["routing"]["to"], "approver");
    assert_eq!(none["routing"]["fallback"], true);
    assert!(none["routing"]["sentence"].as_str().unwrap().contains("no steward position"), "{none}");
    assert!(none["authority"].is_object(), "the access rules' answer is shown: {none}");

    // A position nobody holds.
    policy(&w, &ds, json!({ "steward_position": "dean", "reason": "Name the steward" })).await;
    let (s, empty) = rest(&w.app, "POST", "/api/v1/access/requests", OTHER, Some(json!({ "object_kind": "dataset", "object_id": ds, "reason": "Nobody is dean" }))).await;
    assert_eq!(s, StatusCode::CREATED, "{empty}");
    assert_eq!(empty["routing"]["fallback"], true);
    assert!(empty["routing"]["sentence"].as_str().unwrap().contains("Nobody holds the steward position 'Dean'"), "{empty}");
    // It is still a pending request that the access rules route, and a workspace admin may decide it.
    assert_eq!(empty["status"], "Pending");
    let id = empty["id"].as_str().unwrap();
    let (s, b) = rest(&w.app, "POST", &format!("/api/v1/access/requests/{id}/decide"), CHAIR, Some(json!({ "decision": "approve", "note": "ok" }))).await;
    assert_eq!(s, StatusCode::FORBIDDEN, "a chair is not a steward of this dataset by that position: {b}");
}

// 4. The form is a dataset setting from a fixed list. A missing required field is a 400 that names it.
#[tokio::test]
async fn the_form_is_a_setting_from_a_fixed_list_and_a_missing_field_is_named() {
    let w = world().await;
    let ds = dataset_id(&w, &w.enroll).await;
    // The default form asks for a reason only.
    let (_, entry) = rest(&w.app, "GET", &format!("/api/v1/datasets/{ds}"), MEMBER, None).await;
    assert_eq!(entry["request_fields"], json!(["reason"]));
    // A name outside the list is refused. A subset is kept in the form's order, once each.
    let (s, b) = rest(&w.app, "PUT", &format!("/api/v1/admin/datasets/{ds}/discoverability"), ADMIN, Some(json!({ "request_fields": ["reason", "budget"], "reason": "Form" }))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{b}");
    assert!(b.to_string().contains("agreement_reference"), "the message lists the choices: {b}");
    let (s, b) = rest(&w.app, "PUT", &format!("/api/v1/admin/datasets/{ds}/discoverability"), ADMIN, Some(json!({ "request_fields": ["agreement_reference", "reason", "reason", "project"], "reason": "Form" }))).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_eq!(b["request_fields"], json!(["reason", "project", "agreement_reference"]));
    let (s, _) = rest(&w.app, "PUT", &format!("/api/v1/admin/datasets/{ds}/discoverability"), MEMBER, Some(json!({ "request_fields": ["reason"], "reason": "Form" }))).await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    let (_, entry) = rest(&w.app, "GET", &format!("/api/v1/datasets/{ds}"), MEMBER, None).await;
    assert_eq!(entry["request_fields"], json!(["reason", "project", "agreement_reference"]));

    // A request that leaves them out is refused, and the message names each one.
    let (s, b) = rest(&w.app, "POST", "/api/v1/access/requests", MEMBER, Some(json!({ "object_kind": "dataset", "object_id": ds, "role": "viewer" }))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{b}");
    let text = b.to_string();
    for name in ["reason", "project", "agreement_reference"] {
        assert!(text.contains(&format!("{name} is required")), "{name}: {b}");
    }
    assert!(!text.contains("duration"), "a field not on the form is not asked for: {b}");
    // A field that is not on the form is refused by name, not dropped.
    let (s, b) = rest(&w.app, "POST", "/api/v1/access/requests", MEMBER, Some(json!({ "object_kind": "dataset", "object_id": ds, "reason": "r", "project": "p", "agreement_reference": "a", "days": 0 }))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{b}");
    assert!(b.to_string().contains("duration"), "{b}");
    // With duration on the form, it is required.
    let (s, _) = rest(&w.app, "PUT", &format!("/api/v1/admin/datasets/{ds}/discoverability"), ADMIN, Some(json!({ "request_fields": ["duration"], "reason": "Form" }))).await;
    assert_eq!(s, StatusCode::OK);
    let (s, b) = rest(&w.app, "POST", "/api/v1/access/requests", MEMBER, Some(json!({ "object_kind": "dataset", "object_id": ds, "project": "p" }))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{b}");
    assert!(b.to_string().contains("duration is required") && b.to_string().contains("project is not on this dataset's request form"), "{b}");
    // And one that is complete files, with the reason the server gave it.
    let (s, b) = rest(&w.app, "POST", "/api/v1/access/requests", MEMBER, Some(json!({ "object_kind": "dataset", "object_id": ds, "duration": 7 }))).await;
    assert_eq!(s, StatusCode::CREATED, "{b}");
    assert_eq!(b["days"], 7);
    assert!(b["reason"].as_str().unwrap().contains("Course Enrollment"), "{b}");
}

// 5. A dataset the caller may not discover is the same 404 as one that does not exist. A published dataset with no app is
// refused with words.
#[tokio::test]
async fn a_dataset_the_caller_cannot_discover_is_not_found_and_a_published_one_is_refused_in_words() {
    let w = world().await;
    let ds = dataset_id(&w, &w.payroll).await;
    policy(&w, &ds, json!({ "discoverability": "hidden", "reason": "Hide it" })).await;
    let (s, hidden) = rest(&w.app, "POST", "/api/v1/access/requests", MEMBER, Some(json!({ "object_kind": "dataset", "object_id": ds, "reason": "Let me in" }))).await;
    let (s2, missing) = rest(&w.app, "POST", "/api/v1/access/requests", MEMBER, Some(json!({ "object_kind": "dataset", "object_id": "ds_nothing", "reason": "Let me in" }))).await;
    assert_eq!((s, s2), (StatusCode::NOT_FOUND, StatusCode::NOT_FOUND), "{hidden} {missing}");
    assert_eq!(hidden, missing, "the answers are the same");
    assert_eq!(w.db.run(|c| c.query_one("SELECT count(*) FROM access_requests WHERE dataset_id IS NOT NULL", &[]).unwrap().get::<_, i64>(0)), 0);
    // The explain call is the same.
    let (s, _) = rest(&w.app, "POST", "/api/mcp", MEMBER, Some(json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": { "name": "request_access", "arguments": { "object_kind": "dataset", "object_id": ds, "role": "viewer", "explain": true } } }))).await;
    assert_eq!(s, StatusCode::OK);
    // A published dataset has no app and holds no rows.
    let (s, b) = rest(&w.app, "POST", "/api/v1/access/requests", MEMBER, Some(json!({ "object_kind": "dataset", "object_id": "courses", "reason": "Let me in" }))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{b}");
    assert!(b.to_string().contains("published on its own") && b.to_string().contains("Ask its steward"), "{b}");
}

// 6. The MCP tool `request_access` files the same request, and an AI tool's request waits for the person.
#[tokio::test]
async fn request_access_over_mcp_takes_a_dataset() {
    let w = world().await;
    let ds = dataset_id(&w, &w.enroll).await;
    policy(&w, &ds, json!({ "steward_position": "chair", "request_fields": ["reason", "project"], "reason": "Form" })).await;
    let call = |args: Value| json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": { "name": "request_access", "arguments": args } });
    let (s, v) = rest(&w.app, "POST", "/api/mcp", MEMBER, Some(call(json!({ "object_kind": "dataset", "object_id": ds, "role": "viewer", "reason": "Study", "project": "P1" })))).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_ne!(v["result"]["isError"], true, "{v}");
    let r = &v["result"]["structuredContent"];
    assert_eq!(r["status"], "Pending", "{v}");
    assert_eq!(r["dataset_id"], json!(ds));
    assert_eq!(r["routing"]["to"], "steward");
    assert_eq!(r["answers"]["project"], "P1");
    // A missing field is an error the tool can read.
    let (_, v) = rest(&w.app, "POST", "/api/mcp", OTHER, Some(call(json!({ "object_kind": "dataset", "object_id": ds, "role": "viewer", "reason": "Study" })))).await;
    assert_eq!(v["result"]["isError"], true, "{v}");
    assert!(v.to_string().contains("project is required"), "{v}");
    let _ = (OUTSIDER, w.state.clone());
}
