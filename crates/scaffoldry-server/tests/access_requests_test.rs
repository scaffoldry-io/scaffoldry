//! Access model phase 13: asking for access.
//!
//! A refused person can ask. A rule approves the routine asks alone, a person decides the rest, and an AI tool can never
//! ask in a person's name without them.

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::Router;
use base64::Engine;
use common::TestDb;
use http_body_util::BodyExt;
use scaffoldry_server::build_app_with_state;
use scaffoldry_server::service::identity::issue_test_token_and_user;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tower::ServiceExt;
use uuid::Uuid;

const ADMIN: &str = "jordan.lee@state.edu";
const REQUESTER: &str = "dean.requester@state.edu";
const OFFICER: &str = "privacy.officer@state.edu";
const OUTSIDER: &str = "someone.else@state.edu";
const ROOT: &str = "00000000-0000-0000-0000-000000000001";
const CALLBACK: &str = "https://tool.example.edu/callback";
const ISSUER: &str = "https://api.test.edu";
const VERIFIER: &str = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";

async fn raw(app: &Router, method: &str, uri: &str, token: &str, body: Option<Value>) -> (StatusCode, Value) {
    let mut b = Request::builder().method(method).uri(uri).header("host", "api.test.edu");
    if !token.is_empty() {
        b = b.header("authorization", format!("Bearer {token}"));
    }
    let req = match body {
        Some(v) => b.header("content-type", "application/json").body(Body::from(v.to_string())).unwrap(),
        None => b.body(Body::empty()).unwrap(),
    };
    let resp = app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
}

async fn tool(app: &Router, token: &str, name: &str, args: Value) -> Result<Value, (String, String)> {
    let payload = json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": { "name": name, "arguments": args } });
    let (_, v) = raw(app, "POST", "/api/mcp", token, Some(payload)).await;
    if v["result"]["isError"] == true {
        return Err((v["result"]["structuredContent"]["code"].as_str().unwrap_or("").to_string(), v["result"]["content"][0]["text"].as_str().unwrap_or("").to_string()));
    }
    Ok(v["result"]["structuredContent"].clone())
}

struct World {
    db: TestDb,
    app: Router,
    unit_c: String,
    unit_d: String,
    unit_e: String,
    /// A departmental workspace in D, and the slug of its app.
    wd: String,
    app_d: String,
    /// A restricted workspace in D, and the slug of its app.
    wr: String,
    app_r: String,
    /// A departmental workspace in E, which the requester's unit does not contain.
    app_e: String,
    officer_position: String,
}

impl World {
    async fn rest(&self, method: &str, uri: &str, who: &str, body: Option<Value>) -> (StatusCode, Value) {
        raw(&self.app, method, uri, &issue_test_token_and_user(who), body).await
    }

    fn sql(&self, statement: String) {
        let url = self.db.url().to_string();
        std::thread::spawn(move || postgres::Client::connect(&url, postgres::NoTls).unwrap().batch_execute(&statement).unwrap()).join().unwrap();
    }

    /// The same database behind a router that has read nothing yet.
    fn fresh_app(&self) -> Router {
        build_app_with_state(self.db.state()).expect("router")
    }

    async fn records(&self, slug: &str, who: &str) -> (StatusCode, Value) {
        self.rest("GET", &format!("/api/v1/apps/{slug}/tables/t/records"), who, None).await
    }

    async fn file(&self, who: &str, slug: &str, role: &str, days: i64) -> (StatusCode, Value) {
        self.rest("POST", "/api/v1/access/requests", who, Some(json!({ "object_kind": "app", "object_id": slug, "role": role, "reason": "I need it for the audit", "days": days }))).await
    }

    fn grants_from_requests(&self) -> i64 {
        self.db.run(|c| c.query_one("SELECT COUNT(*) FROM access_grants WHERE source = 'request'", &[]).unwrap().get(0))
    }
}

async fn make_unit(app: &Router, name: &str, parent: &str) -> String {
    let code = format!("{name}{}", &Uuid::new_v4().simple().to_string()[..6]);
    let (s, unit) = raw(app, "POST", "/api/v1/orgs", &issue_test_token_and_user(ADMIN), Some(json!({ "name": code, "code": code, "org_type": "Department", "parent_id": parent }))).await;
    assert_eq!(s, StatusCode::CREATED, "{unit}");
    unit["id"].as_str().unwrap().to_string()
}

async fn make_app(app: &Router, ws_visibility: &str, unit: &str) -> (String, String) {
    let admin = issue_test_token_and_user(ADMIN);
    let code = format!("W{}", &Uuid::new_v4().simple().to_string()[..8]);
    let (s, ws) = raw(app, "POST", "/api/v1/workspaces", &admin, Some(json!({ "name": code, "code": code, "visibility": ws_visibility, "department": "physics", "organization_id": unit }))).await;
    assert_eq!(s, StatusCode::CREATED, "{ws}");
    let ws = ws["id"].as_str().unwrap().to_string();
    let slug = format!("req-{}", &Uuid::new_v4().simple().to_string()[..8]);
    let f = |n: &str| json!({ "name": n, "label": n, "field_type": "Text", "required": false, "ferpa_sensitive": false });
    let manifest = json!({ "slug": slug, "title": "Audit log", "description": "d", "organization_code": "PHYS", "department": "physics", "workspace_id": ws, "views": [], "pages": [], "tables": [{ "id": "t", "name": "T", "slug": "t", "fields": [f("title")] }] });
    let p = tool(app, &admin, "propose_app_change", json!({ "workspace_id": ws, "manifest": manifest, "summary": "An app" })).await.unwrap();
    tool(app, &admin, "decide_proposal", json!({ "proposal_id": p["proposal_id"], "decision": "approve", "rationale": "Set up", "digest": p["digest"] })).await.unwrap();
    (ws, slug)
}

/// Root, a college C, a department D below it, and a separate unit E. The requester holds a position in C.
async fn world() -> World {
    let db = TestDb::new();
    let app = build_app_with_state(db.state()).expect("router");
    for who in [ADMIN, REQUESTER, OFFICER, OUTSIDER] {
        issue_test_token_and_user(who);
    }
    let admin = issue_test_token_and_user(ADMIN);
    let unit_c = make_unit(&app, "C", ROOT).await;
    let unit_d = make_unit(&app, "D", &unit_c).await;
    let unit_e = make_unit(&app, "E", ROOT).await;
    let position = format!("member-{}", &Uuid::new_v4().simple().to_string()[..6]);
    let (s, p) = raw(&app, "POST", "/api/v1/admin/positions", &admin, Some(json!({ "key": position, "name": "Member", "description": "Test", "org_types": ["Department"], "max_holders": 20, "reason": "Define" }))).await;
    assert_eq!(s, StatusCode::CREATED, "{p}");
    for (unit, who) in [(&unit_c, REQUESTER), (&unit_d, OFFICER)] {
        let (s, b) = raw(&app, "POST", &format!("/api/v1/orgs/{unit}/positions/{position}/holders"), &admin, Some(json!({ "eppn": who, "replace": false, "reason": "Assign" }))).await;
        assert_eq!(s, StatusCode::CREATED, "{b}");
    }
    let (wd, app_d) = make_app(&app, "departmental", &unit_d).await;
    let (wr, app_r) = make_app(&app, "restricted", &unit_d).await;
    let (_, app_e) = make_app(&app, "departmental", &unit_e).await;
    World { db, app, unit_c, unit_d, unit_e, wd, app_d, wr, app_r, app_e, officer_position: position }
}

fn agent_sign_in_body(client_id: &str, access: &str) -> Value {
    let challenge = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(Sha256::digest(VERIFIER.as_bytes()));
    json!({ "client_id": client_id, "redirect_uri": CALLBACK, "response_type": "code", "state": "s", "code_challenge": challenge, "code_challenge_method": "S256", "resource": ISSUER, "decision": "allow", "access": access })
}

async fn agent_client(w: &World) -> String {
    let admin = issue_test_token_and_user(ADMIN);
    let (s, made) = raw(&w.app, "POST", "/api/v1/clients", &admin, Some(json!({ "name": "Desk assistant", "operator": "Academic Computing", "kind": "agent", "redirect_uris": [CALLBACK] }))).await;
    assert_eq!(s, StatusCode::CREATED, "{made}");
    let id = made["client_id"].as_str().unwrap().to_string();
    let (s, d) = raw(&w.app, "POST", &format!("/api/v1/clients/{id}/decide"), &admin, Some(json!({ "decision": "approve", "rationale": "An allowed tool" }))).await;
    assert_eq!(s, StatusCode::OK, "{d}");
    id
}

/// A session of `client_id` for `user` at `access`. Returns its access token.
async fn session(w: &World, user: &str, client_id: &str, access: &str) -> String {
    let (s, allowed) = raw(&w.app, "POST", "/oauth/authorize", &issue_test_token_and_user(user), Some(agent_sign_in_body(client_id, access))).await;
    assert_eq!(s, StatusCode::OK, "{allowed}");
    let code = allowed["redirect_to"].as_str().unwrap().split_once('?').unwrap().1.split('&').find_map(|p| p.strip_prefix("code=")).unwrap().to_string();
    let form = format!("grant_type=authorization_code&code={code}&redirect_uri={CALLBACK}&client_id={client_id}&code_verifier={VERIFIER}");
    let req = Request::builder().method("POST").uri("/oauth/token").header("host", "api.test.edu").header("content-type", "application/x-www-form-urlencoded").body(Body::from(form)).unwrap();
    let resp = w.app.clone().oneshot(req).await.unwrap();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let reply: Value = serde_json::from_slice(&bytes).unwrap();
    reply["access_token"].as_str().unwrap_or_else(|| panic!("{reply}")).to_string()
}

// 1. A viewer request inside the requester's own unit, for an Internal app, for 14 days, is approved at once by the
//    default rule, and the grant expires after 14 days.
#[tokio::test]
async fn a_viewer_request_in_the_own_unit_for_an_internal_app_is_approved_at_once_and_expires() {
    let w = world().await;
    assert_eq!(w.records(&w.app_d, REQUESTER).await.0, StatusCode::FORBIDDEN, "a dean of the college has no access to the department's app yet");
    let (s, filed) = w.file(REQUESTER, &w.app_d, "viewer", 14).await;
    assert_eq!(s, StatusCode::CREATED, "{filed}");
    assert_eq!((filed["status"].as_str(), filed["route"].as_str()), (Some("Approved"), Some("auto")), "{filed}");
    assert!(filed["rule_id"].is_string() && filed["grant_id"].is_number(), "the grant records the rule: {filed}");
    assert_eq!(w.records(&w.app_d, REQUESTER).await.0, StatusCode::OK, "the access is real");

    let days: f64 = w.db.run(|c| c.query_one("SELECT EXTRACT(EPOCH FROM (valid_until - NOW())) / 86400.0 FROM access_grants WHERE source = 'request'", &[]).unwrap().get(0));
    assert!((13.9..=14.1).contains(&days), "expires after 14 days, not {days}");
    let reason: String = w.db.run(|c| c.query_one("SELECT reason FROM access_grants WHERE source = 'request'", &[]).unwrap().get(0));
    assert!(reason.contains(filed["id"].as_str().unwrap()), "the grant names the request: {reason}");

    // It expires by itself: the same database, a router that has read nothing yet.
    w.sql("UPDATE access_grants SET valid_until = NOW() - INTERVAL '1 hour' WHERE source = 'request'".to_string());
    let later = w.fresh_app();
    let (s, _) = raw(&later, "GET", &format!("/api/v1/apps/{}/tables/t/records", w.app_d), &issue_test_token_and_user(REQUESTER), None).await;
    assert_eq!(s, StatusCode::FORBIDDEN, "the grant has ended");
    let (_, mine) = raw(&later, "GET", "/api/v1/me/access/requests", &issue_test_token_and_user(REQUESTER), None).await;
    assert_eq!(mine["requests"][0]["status"], "Expired", "{mine}");
}

// 2. An editor request is not approved by the default rule and is routed to the people `authority::required` names.
#[tokio::test]
async fn an_editor_request_goes_to_the_people_the_authority_matrix_names() {
    let w = world().await;
    let (s, filed) = w.file(REQUESTER, &w.app_d, "editor", 14).await;
    assert_eq!(s, StatusCode::CREATED, "{filed}");
    assert_eq!((filed["status"].as_str(), filed["route"].as_str()), (Some("Pending"), Some("approver")), "{filed}");
    assert!(filed["authority"]["reasons_detail"].as_array().unwrap().iter().any(|r| r["check"] == "request_write"), "the signal is named: {filed}");
    assert_eq!(w.grants_from_requests(), 0);
    // With no matrix row, the workspace's owner or admin decides.
    assert!(filed["waiting_on"].as_array().unwrap().iter().any(|p| p == ADMIN), "{filed}");

    // A unit's matrix sends Moderate changes to an officer of the department: they, and not the owner, decide.
    let spec = json!({ "Position": { "key": w.officer_position, "from": "Workspace", "walk_up": false } });
    let unit = Uuid::parse_str(&w.unit_d).unwrap();
    w.db.run(move |c| {
        c.execute("INSERT INTO authority_rows (id, organization_id, tier, path, approver) VALUES ($1, $2, 'Moderate', 'approver', $3)", &[&Uuid::new_v4(), &unit, &spec]).unwrap();
    });
    let (_, second) = w.file(REQUESTER, &w.app_d, "editor", 7).await;
    assert_eq!(second["authority"]["path"], "approver", "{second}");
    let id = second["id"].as_str().unwrap();
    let decide = |note: &str| json!({ "decision": "approve", "note": note });
    assert_eq!(w.rest("POST", &format!("/api/v1/access/requests/{id}/decide"), ADMIN, Some(decide("Fine"))).await.0, StatusCode::FORBIDDEN, "the owner is not the named approver");
    let (s, done) = w.rest("POST", &format!("/api/v1/access/requests/{id}/decide"), OFFICER, Some(decide("Reviewed"))).await;
    assert_eq!(s, StatusCode::OK, "{done}");
    assert_eq!(done["status"], "Approved");
    assert!(done["grant_id"].is_number());
    let role: String = w.db.run(|c| c.query_one("SELECT role FROM access_grants WHERE source = 'request'", &[]).unwrap().get(0));
    assert_eq!(role, "editor");
    let (_, listed) = w.rest("GET", "/api/v1/me/access/requests/to-decide", OFFICER, None).await;
    assert!(!listed["requests"].as_array().unwrap().iter().any(|r| r["id"] == id), "decided requests leave the list");
}

// 2a. A viewer request in a restricted workspace is never approved by a rule, and a rule may not name `restricted`.
#[tokio::test]
async fn a_request_for_a_restricted_workspace_always_goes_to_its_owner() {
    let w = world().await;
    let (s, filed) = w.file(REQUESTER, &w.app_r, "viewer", 14).await;
    assert_eq!(s, StatusCode::CREATED, "{filed}");
    assert_eq!((filed["status"].as_str(), filed["route"].as_str()), (Some("Pending"), Some("approver")), "{filed}");
    assert!(filed["waiting_on"].as_array().unwrap().iter().any(|p| p == ADMIN), "{filed}");
    assert_eq!(w.grants_from_requests(), 0);

    let rule = json!({ "organization_id": ROOT, "reason": "Open it up", "rule": { "max_role": "viewer", "max_sensitivity": "Internal", "visibilities": ["departmental", "restricted"], "own_unit": true, "max_days": 30 } });
    let (s, refused) = w.rest("POST", "/api/v1/admin/access/request-rules", ADMIN, Some(rule)).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{refused}");
    assert!(refused.to_string().contains("restricted"), "{refused}");
}

// 3. A request filed by an AI tool waits for the person, and an approver cannot decide it before then.
#[tokio::test]
async fn a_request_filed_by_an_ai_tool_waits_for_the_person() {
    let w = world().await;
    let client = agent_client(&w).await;
    let token = session(&w, REQUESTER, &client, "read").await;
    tool(&w.app, &token, "begin_task", json!({ "task": "Check the audit log" })).await.unwrap();
    let filed = tool(&w.app, &token, "request_access", json!({ "object_kind": "app", "object_id": w.app_d, "role": "viewer", "reason": "I need the audit log", "days": 14 })).await.unwrap();
    assert_eq!(filed["status"], "Waiting for person", "{filed}");
    assert_eq!(filed["task"], "Check the audit log");
    assert!(filed["filed_by_client"].is_string(), "{filed}");
    let id = filed["id"].as_str().unwrap().to_string();
    assert_eq!(w.grants_from_requests(), 0);

    // An approver cannot decide it, and the tool cannot confirm it.
    let decide = Some(json!({ "decision": "approve", "note": "ok" }));
    assert_eq!(w.rest("POST", &format!("/api/v1/access/requests/{id}/decide"), ADMIN, decide.clone()).await.0, StatusCode::CONFLICT);
    let (s, body) = raw(&w.app, "POST", &format!("/api/v1/access/requests/{id}/confirm"), &token, None).await;
    assert!(s.is_client_error(), "the tool cannot confirm for the person: {s} {body}");
    assert_eq!(w.rest("POST", &format!("/api/v1/access/requests/{id}/confirm"), OUTSIDER, None).await.0, StatusCode::NOT_FOUND, "nor can another person");

    // The person confirms in the desk. The default rule then applies as it would to the person's own request.
    let (s, done) = w.rest("POST", &format!("/api/v1/access/requests/{id}/confirm"), REQUESTER, None).await;
    assert_eq!(s, StatusCode::OK, "{done}");
    assert_eq!((done["status"].as_str(), done["route"].as_str()), (Some("Approved"), Some("auto")), "{done}");
    assert_eq!(w.grants_from_requests(), 1);
}

// 4. A `self` request widens only the session that asked, and a new session does not have it.
#[tokio::test]
async fn a_self_request_widens_only_the_session_that_asked() {
    let w = world().await;
    // The person is an editor of the app already. Their AI tool was only given Read.
    let (s, b) = w.rest("POST", &format!("/api/v1/workspaces/{}/collaborators", w.wd), ADMIN, Some(json!({ "eppn": REQUESTER, "role": "editor" }))).await;
    assert_eq!(s, StatusCode::CREATED, "{b}");
    let client = agent_client(&w).await;
    let read = session(&w, REQUESTER, &client, "read").await;
    let write = json!({ "app_slug": w.app_d, "data": { "_table_id": "t", "title": "x" } });
    assert_eq!(tool(&w.app, &read, "create_record", write.clone()).await.unwrap_err().0, "client_scope");

    let filed = tool(&w.app, &read, "request_access", json!({ "object_kind": "app", "object_id": w.app_d, "role": "editor", "reason": "Enter the audit rows", "days": 1 })).await.unwrap();
    assert_eq!((filed["route"].as_str(), filed["status"].as_str()), (Some("self"), Some("Waiting for person")), "{filed}");
    let id = filed["id"].as_str().unwrap();
    assert_eq!(tool(&w.app, &read, "create_record", write.clone()).await.unwrap_err().0, "client_scope", "nothing changes before the person confirms");

    let (s, done) = w.rest("POST", &format!("/api/v1/access/requests/{id}/confirm"), REQUESTER, None).await;
    assert_eq!(s, StatusCode::OK, "{done}");
    assert_eq!((done["status"].as_str(), done["route"].as_str()), (Some("Approved"), Some("self")), "{done}");
    assert!(done["grant_id"].is_null(), "no grant is stored: {done}");
    assert_eq!(w.grants_from_requests(), 0);
    assert!(tool(&w.app, &read, "create_record", write.clone()).await.is_ok(), "the session that asked can now write");

    let another = session(&w, REQUESTER, &client, "read").await;
    assert_eq!(tool(&w.app, &another, "create_record", write).await.unwrap_err().0, "client_scope", "a new session does not have it");
}

// 5. The requester cannot approve their own request, even as an owner.
#[tokio::test]
async fn the_requester_cannot_approve_their_own_request_even_as_an_owner() {
    let w = world().await;
    let (s, b) = w.rest("POST", &format!("/api/v1/workspaces/{}/collaborators", w.wd), ADMIN, Some(json!({ "eppn": REQUESTER, "role": "owner" }))).await;
    assert_eq!(s, StatusCode::CREATED, "{b}");
    let (s, filed) = w.rest("POST", "/api/v1/access/requests", REQUESTER, Some(json!({ "object_kind": "app", "object_id": w.app_d, "role": "viewer", "clearance": "Restricted", "reason": "Cleared data", "days": 7 }))).await;
    assert_eq!(s, StatusCode::CREATED, "{filed}");
    assert_eq!(filed["status"], "Pending", "a clearance above Internal is not the default rule's: {filed}");
    assert!(filed["authority"]["reasons_detail"].as_array().unwrap().iter().any(|r| r["check"] == "request_sensitive"), "{filed}");
    let id = filed["id"].as_str().unwrap();
    let (s, refused) = w.rest("POST", &format!("/api/v1/access/requests/{id}/decide"), REQUESTER, Some(json!({ "decision": "approve", "note": "Me" }))).await;
    assert_eq!(s, StatusCode::FORBIDDEN, "{refused}");
    assert!(refused.to_string().contains("own request"), "{refused}");
    let (_, listed) = w.rest("GET", "/api/v1/me/access/requests/to-decide", REQUESTER, None).await;
    assert!(!listed["requests"].as_array().unwrap().iter().any(|r| r["id"] == id));
    // The platform admin, who is a different person, can decide it. A request outside the unit is flagged as such.
    let (_, outside) = w.file(REQUESTER, &w.app_e, "viewer", 7).await;
    assert!(outside["authority"]["reasons_detail"].as_array().unwrap().iter().any(|r| r["check"] == "request_outside_unit"), "{outside}");
    assert_eq!((outside["status"].as_str(), outside["route"].as_str()), (Some("Pending"), Some("approver")));
}

// 6. A denial carries a filled `request` object that, sent back unchanged, files the request.
#[tokio::test]
async fn a_denial_carries_a_request_that_files_itself() {
    let w = world().await;
    let (s, denied) = w.records(&w.app_d, REQUESTER).await;
    assert_eq!(s, StatusCode::FORBIDDEN, "{denied}");
    let request = denied["request"].clone();
    assert_eq!((request["object_kind"].as_str(), request["object_id"].as_str(), request["role"].as_str()), (Some("app"), Some(w.app_d.as_str()), Some("viewer")), "{denied}");
    assert!(request["reason"].as_str().is_some_and(|r| !r.is_empty()) && request["days"].is_number(), "{request}");
    let (s, filed) = w.rest("POST", "/api/v1/access/requests", REQUESTER, Some(request)).await;
    assert_eq!(s, StatusCode::CREATED, "{filed}");
    assert_eq!(filed["status"], "Approved", "the default rule applies: {filed}");
    // A write the person may not make asks for a role that would allow it.
    let (s, denied) = w.rest("POST", &format!("/api/v1/apps/{}/tables/t/records", w.app_e), OUTSIDER, Some(json!({ "data": { "title": "x" } }))).await;
    assert_eq!(s, StatusCode::FORBIDDEN, "{denied}");
    assert_eq!(denied["request"]["role"], "editor", "{denied}");
}

// 7. A unit rule that removes a condition from the institution's rule is refused.
#[tokio::test]
async fn a_unit_rule_that_removes_a_condition_is_refused_and_one_that_adds_one_is_kept() {
    let w = world().await;
    let (s, shown) = w.rest("GET", "/api/v1/admin/access/request-rules", ADMIN, None).await;
    assert_eq!(s, StatusCode::OK, "{shown}");
    let defaults = shown["rules"].as_array().unwrap();
    assert_eq!(defaults.len(), 1, "{shown}");
    assert!(defaults[0]["sentence"].as_str().unwrap().starts_with("A viewer request for an app at Internal or below"), "{shown}");

    let body = |own_unit: bool, days: i64| json!({ "organization_id": w.unit_c, "reason": "Our college", "rule": { "max_role": "viewer", "max_sensitivity": "Internal", "visibilities": ["departmental"], "own_unit": own_unit, "max_days": days } });
    let (s, refused) = w.rest("POST", "/api/v1/admin/access/request-rules", ADMIN, Some(body(false, 30))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{refused}");
    assert!(refused.to_string().contains("own_unit"), "names the condition: {refused}");
    let (s, refused) = w.rest("POST", "/api/v1/admin/access/request-rules", ADMIN, Some(body(true, 90))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{refused}");
    assert!(refused.to_string().contains("max_days"), "{refused}");

    // A stricter rule waits for a second person, then holds. The college now approves for 7 days at most.
    let (s, proposed) = w.rest("POST", "/api/v1/admin/access/request-rules", ADMIN, Some(body(true, 7))).await;
    assert_eq!(s, StatusCode::ACCEPTED, "{proposed}");
    let change = proposed["change_id"].as_str().unwrap();
    let decide = Some(json!({ "decision": "confirm", "reason": "Second person" }));
    assert_eq!(w.rest("POST", &format!("/api/v1/admin/access/changes/{change}/decide"), ADMIN, decide.clone()).await.0, StatusCode::FORBIDDEN, "the proposer cannot confirm");
    let second = "second.admin@state.edu";
    issue_test_token_and_user(second);
    let (s, b) = w.rest("POST", &format!("/api/v1/orgs/{ROOT}/appointments"), ADMIN, Some(json!({ "eppn": second, "scoped_affiliation": "platform_admin", "reason": "A second administrator" }))).await;
    assert_eq!(s, StatusCode::CREATED, "{b}");
    let (s, done) = w.rest("POST", &format!("/api/v1/admin/access/changes/{change}/decide"), second, decide).await;
    assert_eq!(s, StatusCode::OK, "{done}");
    let (_, now) = w.rest("GET", "/api/v1/admin/access/request-rules", ADMIN, None).await;
    assert_eq!(now["rules"].as_array().unwrap().len(), 2, "{now}");
    let (s, filed) = w.file(REQUESTER, &w.app_d, "viewer", 14).await;
    assert_eq!(s, StatusCode::CREATED, "{filed}");
    assert_eq!(filed["status"], "Pending", "14 days is over the college's 7: {filed}");
    let _ = &w.unit_e;
}

#[tokio::test]
async fn a_request_names_a_real_object_a_reason_and_a_number_of_days_within_the_limit() {
    let w = world().await;
    for (body, field) in [
        (json!({ "object_kind": "app", "object_id": "no-such-app", "role": "viewer", "reason": "r", "days": 7 }), "object_id"),
        (json!({ "object_kind": "app", "object_id": w.app_d, "role": "emperor", "reason": "r", "days": 7 }), "role"),
        (json!({ "object_kind": "app", "object_id": w.app_d, "role": "viewer", "reason": "  ", "days": 7 }), "reason"),
        (json!({ "object_kind": "app", "object_id": w.app_d, "role": "viewer", "reason": "r", "days": 0 }), "days"),
        (json!({ "object_kind": "app", "object_id": w.app_d, "role": "viewer", "reason": "r", "days": 91 }), "days"),
    ] {
        let (s, refused) = w.rest("POST", "/api/v1/access/requests", REQUESTER, Some(body)).await;
        assert!(s == StatusCode::BAD_REQUEST || s == StatusCode::NOT_FOUND, "{field}: {s} {refused}");
        assert!(refused.to_string().contains(field), "{field}: {refused}");
    }
    assert_eq!(w.grants_from_requests(), 0);
    // A requester can withdraw a pending request, and nobody else can.
    let (_, filed) = w.file(REQUESTER, &w.app_d, "editor", 7).await;
    let id = filed["id"].as_str().unwrap();
    assert_eq!(w.rest("POST", &format!("/api/v1/access/requests/{id}/withdraw"), OUTSIDER, None).await.0, StatusCode::NOT_FOUND);
    let (s, done) = w.rest("POST", &format!("/api/v1/access/requests/{id}/withdraw"), REQUESTER, None).await;
    assert_eq!((s, done["status"].as_str()), (StatusCode::OK, Some("Withdrawn")), "{done}");
    let (s, _) = w.rest("POST", &format!("/api/v1/access/requests/{id}/decide"), ADMIN, Some(json!({ "decision": "approve", "note": "late" }))).await;
    assert_eq!(s, StatusCode::CONFLICT, "a withdrawn request cannot be approved");
    assert_eq!(w.rest("POST", "/api/v1/access/requests", "", None).await.0, StatusCode::UNAUTHORIZED);
}
