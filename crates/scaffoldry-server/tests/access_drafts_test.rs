//! Access model phase 14: AI tools draft, people decide.
//!
//! `draft_access_change` stores a pending change that applies nothing. `preview_access_change` is the same without the
//! store. Suggestions come from queries, not from a model. A widening draft is confirmed by someone other than the person
//! it was drafted for, and a draft that is a week old is refused.

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
const SECOND: &str = "second.admin@state.edu";
const HOLDER_A: &str = "holder.a@state.edu";
const HOLDER_B: &str = "holder.b@state.edu";
const OTHER: &str = "someone.else@state.edu";
const TARGET: &str = "target.person@state.edu";
const FACULTY: &str = "sarah.connor@state.edu";
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

/// A tool call. `Err` holds the code and the sentence.
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
    unit: String,
    position: String,
    w1: String,
    w2: String,
    app1: String,
}

impl World {
    async fn rest(&self, method: &str, uri: &str, who: &str, body: Option<Value>) -> (StatusCode, Value) {
        raw(&self.app, method, uri, &issue_test_token_and_user(who), body).await
    }

    fn sql(&self, statement: String) {
        let url = self.db.url().to_string();
        std::thread::spawn(move || postgres::Client::connect(&url, postgres::NoTls).unwrap().batch_execute(&statement).unwrap()).join().unwrap();
    }

    fn count(&self, statement: &'static str) -> i64 {
        self.db.run(move |c| c.query_one(statement, &[]).unwrap().get(0))
    }

    /// A person's token as an AI tool session at `access`, with the task it names.
    async fn agent(&self, user: &str, access: &str, task: &str) -> String {
        let admin = issue_test_token_and_user(ADMIN);
        let (s, made) = raw(&self.app, "POST", "/api/v1/clients", &admin, Some(json!({ "name": "Desk assistant", "operator": "Academic Computing", "kind": "agent", "redirect_uris": [CALLBACK] }))).await;
        assert_eq!(s, StatusCode::CREATED, "{made}");
        let id = made["client_id"].as_str().unwrap().to_string();
        let (s, d) = raw(&self.app, "POST", &format!("/api/v1/clients/{id}/decide"), &admin, Some(json!({ "decision": "approve", "rationale": "An allowed tool" }))).await;
        assert_eq!(s, StatusCode::OK, "{d}");
        let challenge = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(Sha256::digest(VERIFIER.as_bytes()));
        let body = json!({ "client_id": id, "redirect_uri": CALLBACK, "response_type": "code", "state": "s", "code_challenge": challenge, "code_challenge_method": "S256", "resource": ISSUER, "decision": "allow", "access": access });
        let (s, allowed) = raw(&self.app, "POST", "/oauth/authorize", &issue_test_token_and_user(user), Some(body)).await;
        assert_eq!(s, StatusCode::OK, "{allowed}");
        let code = allowed["redirect_to"].as_str().unwrap().split_once('?').unwrap().1.split('&').find_map(|p| p.strip_prefix("code=")).unwrap().to_string();
        let form = format!("grant_type=authorization_code&code={code}&redirect_uri={CALLBACK}&client_id={id}&code_verifier={VERIFIER}");
        let req = Request::builder().method("POST").uri("/oauth/token").header("host", "api.test.edu").header("content-type", "application/x-www-form-urlencoded").body(Body::from(form)).unwrap();
        let resp = self.app.clone().oneshot(req).await.unwrap();
        let reply: Value = serde_json::from_slice(&resp.into_body().collect().await.unwrap().to_bytes()).unwrap();
        let token = reply["access_token"].as_str().unwrap_or_else(|| panic!("{reply}")).to_string();
        tool(&self.app, &token, "begin_task", json!({ "task": task })).await.unwrap();
        token
    }

    fn grant_body(&self, role: &str) -> Value {
        json!({ "holder_kind": "person", "holder_id": TARGET, "role": role, "anchor_kind": "container", "anchor_id": format!("app:{}", self.app1), "source": "task", "reason": "The audit needs it" })
    }

    async fn reads(&self, who: &str) -> StatusCode {
        self.rest("GET", &format!("/api/v1/apps/{}/tables/t/records", self.app1), who, None).await.0
    }
}

async fn make_app(app: &Router, unit: &str) -> (String, String) {
    let admin = issue_test_token_and_user(ADMIN);
    let code = format!("W{}", &Uuid::new_v4().simple().to_string()[..8]);
    let (s, ws) = raw(app, "POST", "/api/v1/workspaces", &admin, Some(json!({ "name": code, "code": code, "visibility": "restricted", "department": "physics", "organization_id": unit }))).await;
    assert_eq!(s, StatusCode::CREATED, "{ws}");
    let ws = ws["id"].as_str().unwrap().to_string();
    let slug = format!("dr-{}", &Uuid::new_v4().simple().to_string()[..8]);
    let f = |n: &str, extra: Value| {
        let mut v = json!({ "name": n, "label": n, "field_type": "Text", "required": false, "ferpa_sensitive": false });
        if let Value::Object(m) = extra {
            for (k, x) in m {
                v[k] = x;
            }
        }
        v
    };
    let manifest = json!({ "slug": slug, "title": "Audit log", "description": "d", "organization_code": "PHYS", "department": "physics", "workspace_id": ws, "views": [], "pages": [], "tables": [{ "id": "t", "name": "T", "slug": "t", "fields": [
        f("title", json!({})),
        f("ssn", json!({ "ferpa_sensitive": true })),
        f("dob", json!({ "ferpa_sensitive": true, "mask": { "kind": "year" } })),
    ] }] });
    let p = tool(app, &admin, "propose_app_change", json!({ "workspace_id": ws, "manifest": manifest, "summary": "An app" })).await.unwrap();
    tool(app, &issue_test_token_and_user(SECOND), "decide_proposal", json!({ "proposal_id": p["proposal_id"], "decision": "approve", "rationale": "Set up", "digest": p["digest"] })).await.unwrap();
    (ws, slug)
}

async fn world() -> World {
    let db = TestDb::new();
    let app = build_app_with_state(db.state()).expect("router");
    for who in [ADMIN, SECOND, HOLDER_A, HOLDER_B, OTHER, TARGET, FACULTY] {
        issue_test_token_and_user(who);
    }
    let admin = issue_test_token_and_user(ADMIN);
    let code = format!("D{}", &Uuid::new_v4().simple().to_string()[..6]);
    let (s, unit) = raw(&app, "POST", "/api/v1/orgs", &admin, Some(json!({ "name": code, "code": code, "org_type": "Department", "parent_id": ROOT }))).await;
    assert_eq!(s, StatusCode::CREATED, "{unit}");
    let unit = unit["id"].as_str().unwrap().to_string();
    let position = format!("member-{}", &Uuid::new_v4().simple().to_string()[..6]);
    let (s, p) = raw(&app, "POST", "/api/v1/admin/positions", &admin, Some(json!({ "key": position, "name": "Member", "description": "Test", "org_types": ["Department"], "max_holders": 20, "reason": "Define" }))).await;
    assert_eq!(s, StatusCode::CREATED, "{p}");
    for who in [HOLDER_A, HOLDER_B] {
        let (s, b) = raw(&app, "POST", &format!("/api/v1/orgs/{unit}/positions/{position}/holders"), &admin, Some(json!({ "eppn": who, "replace": false, "reason": "Assign" }))).await;
        assert_eq!(s, StatusCode::CREATED, "{b}");
    }
    let (s, b) = raw(&app, "POST", &format!("/api/v1/orgs/{ROOT}/appointments"), &admin, Some(json!({ "eppn": SECOND, "scoped_affiliation": "platform_admin", "reason": "A second administrator" }))).await;
    assert_eq!(s, StatusCode::CREATED, "{b}");
    let (w1, app1) = make_app(&app, &unit).await;
    let (w2, _) = make_app(&app, &unit).await;
    World { db, app, unit, position, w1, w2, app1 }
}

// 1. `draft_access_change` stores a pending change and changes no access.
#[tokio::test]
async fn a_draft_stores_a_pending_change_and_changes_no_access() {
    let w = world().await;
    assert_eq!(w.reads(TARGET).await, StatusCode::FORBIDDEN);
    let token = w.agent(ADMIN, "propose", "Tidy the audit app's access").await;
    let before = w.count("SELECT COUNT(*) FROM access_grants");
    let draft = tool(&w.app, &token, "draft_access_change", json!({ "kind": "grant", "change": w.grant_body("viewer"), "reason": "The audit needs it" })).await.unwrap();
    assert_eq!(draft["status"], "Pending", "{draft}");
    assert!(draft["change_id"].is_string(), "{draft}");
    assert_eq!(w.count("SELECT COUNT(*) FROM access_grants"), before, "no grant was made");
    assert_eq!(w.reads(TARGET).await, StatusCode::FORBIDDEN, "nobody gained access");
    assert_eq!(w.count("SELECT COUNT(*) FROM access_changes WHERE status = 'Pending'"), 1);
}

// 2. Its sentences and impact equal what the console shows for the same change.
#[tokio::test]
async fn its_sentences_and_impact_equal_what_the_console_shows() {
    let w = world().await;
    let token = w.agent(ADMIN, "propose", "Check the impact").await;
    let draft = tool(&w.app, &token, "draft_access_change", json!({ "kind": "grant", "change": w.grant_body("viewer"), "reason": "The audit needs it" })).await.unwrap();
    let id = draft["change_id"].as_str().unwrap();
    let (s, console) = w.rest("POST", &format!("/api/v1/admin/access/changes/{id}/impact"), ADMIN, None).await;
    assert_eq!(s, StatusCode::OK, "{console}");
    assert_eq!(draft["impact"]["gain"], console["gain"], "the same people gain");
    assert_eq!(draft["impact"]["lose"], console["lose"], "the same people lose");
    assert_eq!(draft["impact"]["per_role"], console["per_role"]);
    assert!(draft["sentences"].as_array().is_some_and(|s| !s.is_empty() && s[0].as_str().unwrap().contains(TARGET)), "{draft}");
    assert!(draft["checks"].is_array(), "{draft}");
    // A preview of the same change says the same and stores nothing.
    let stored = w.count("SELECT COUNT(*) FROM access_changes");
    let preview = tool(&w.app, &token, "draft_access_change", json!({ "kind": "grant", "change": w.grant_body("viewer"), "reason": "The audit needs it", "preview": true })).await.unwrap();
    assert_eq!(preview["sentences"], draft["sentences"]);
    assert_eq!(preview["impact"]["gain"], draft["impact"]["gain"]);
    assert!(preview["change_id"].is_null(), "{preview}");
    assert_eq!(preview["status"], "Preview");
    assert_eq!(w.count("SELECT COUNT(*) FROM access_changes"), stored, "a preview stores nothing");
}

// 3. A caller without the right to propose a change cannot draft it.
#[tokio::test]
async fn a_caller_without_the_right_cannot_draft() {
    let w = world().await;
    let token = w.agent(FACULTY, "propose", "Try to give access").await;
    let (code, _) = tool(&w.app, &token, "draft_access_change", json!({ "kind": "grant", "change": w.grant_body("viewer"), "reason": "No right" })).await.unwrap_err();
    assert_eq!(code, "grant_outside_scope");
    assert_eq!(w.count("SELECT COUNT(*) FROM access_changes"), 0);
    // A read session may not draft at all.
    let read = w.agent(ADMIN, "read", "Only reading").await;
    let (code, _) = tool(&w.app, &read, "draft_access_change", json!({ "kind": "grant", "change": w.grant_body("viewer"), "reason": "r" })).await.unwrap_err();
    assert_eq!(code, "client_scope");
    // A kind this brief does not draft here is refused, with where to go instead.
    let propose = w.agent(ADMIN, "propose", "Other kinds").await;
    let (code, why) = tool(&w.app, &propose, "draft_access_change", json!({ "kind": "mask", "change": {}, "reason": "r" })).await.unwrap_err();
    assert_eq!(code, "bad_request");
    assert!(why.contains("propose_app_change"), "{why}");
}

// 4. Each suggestion kind finds its seeded case and not its neighbor.
#[tokio::test]
async fn a_membership_a_grant_already_covers_is_suggested_and_a_lone_one_is_not() {
    let w = world().await;
    for who in [HOLDER_A, OTHER] {
        let (s, b) = w.rest("POST", &format!("/api/v1/workspaces/{}/collaborators", w.w1), ADMIN, Some(json!({ "eppn": who, "role": "viewer" }))).await;
        assert_eq!(s, StatusCode::CREATED, "{b}");
    }
    w.sql(format!("INSERT INTO access_grants (holder_kind, holder_id, role, anchor_kind, anchor_id, source, reason, created_by) VALUES ('person', '{HOLDER_A}', 'editor', 'container', 'workspace:{}', 'task', 'Covers the membership', '{ADMIN}')", w.w1));
    let (s, got) = w.rest("GET", "/api/v1/admin/access/suggestions", ADMIN, None).await;
    assert_eq!(s, StatusCode::OK, "{got}");
    let covered: Vec<&Value> = got["suggestions"].as_array().unwrap().iter().filter(|x| x["kind"] == "membership_covered").collect();
    assert!(covered.iter().any(|x| x["person"] == HOLDER_A && x["workspace_id"] == w.w1.as_str()), "{got}");
    assert!(!covered.iter().any(|x| x["person"] == OTHER), "a membership nothing covers is not suggested: {got}");
    assert!(covered[0]["sentence"].as_str().unwrap().contains(HOLDER_A), "{covered:?}");
}

#[tokio::test]
async fn memberships_of_one_position_at_one_unit_are_suggested_as_one_holder_unit_grant() {
    let w = world().await;
    for (who, ws) in [(HOLDER_A, &w.w1), (HOLDER_A, &w.w2), (HOLDER_B, &w.w1), (HOLDER_B, &w.w2), (OTHER, &w.w1)] {
        let (s, b) = w.rest("POST", &format!("/api/v1/workspaces/{ws}/collaborators"), ADMIN, Some(json!({ "eppn": who, "role": "editor" }))).await;
        assert_eq!(s, StatusCode::CREATED, "{b}");
    }
    let (_, got) = w.rest("GET", "/api/v1/admin/access/suggestions", ADMIN, None).await;
    let group = got["suggestions"].as_array().unwrap().iter().find(|x| x["kind"] == "holder_unit_grant").unwrap_or_else(|| panic!("{got}"));
    assert_eq!((group["position"].as_str(), group["unit"].as_str(), group["role"].as_str()), (Some(w.position.as_str()), Some(w.unit.as_str()), Some("editor")), "{group}");
    assert_eq!(group["memberships"].as_array().unwrap().len(), 4, "every holder in every workspace of the unit: {group}");
    assert!(!group.to_string().contains(OTHER), "a person who does not hold the position is not in the group: {group}");
    assert_eq!(group["grant"]["anchor_kind"], "holder_unit", "the grant that would replace them: {group}");
}

#[tokio::test]
async fn a_grant_that_would_reach_more_than_the_memberships_do_is_not_suggested() {
    let w = world().await;
    // Both holders are editors of one workspace only. A grant for the position would also open the other to them.
    for (who, ws) in [(HOLDER_A, &w.w1), (HOLDER_B, &w.w1)] {
        let (s, b) = w.rest("POST", &format!("/api/v1/workspaces/{ws}/collaborators"), ADMIN, Some(json!({ "eppn": who, "role": "editor" }))).await;
        assert_eq!(s, StatusCode::CREATED, "{b}");
    }
    let (_, got) = w.rest("GET", "/api/v1/admin/access/suggestions", ADMIN, None).await;
    assert!(!got["suggestions"].as_array().unwrap().iter().any(|x| x["kind"] == "holder_unit_grant"), "{got}");
}

#[tokio::test]
async fn a_grant_past_its_end_that_was_never_revoked_is_suggested_and_a_current_one_is_not() {
    let w = world().await;
    let insert = |holder: &str, until: &str| format!("INSERT INTO access_grants (holder_kind, holder_id, role, anchor_kind, anchor_id, valid_from, valid_until, source, reason, created_by) VALUES ('person', '{holder}', 'viewer', 'container', 'workspace:{}', NOW() - INTERVAL '40 days', NOW() {until}, 'task', 'Seeded', '{ADMIN}')", w.w1);
    w.sql(insert(HOLDER_A, "- INTERVAL '10 days'"));
    w.sql(insert(HOLDER_B, "+ INTERVAL '10 days'"));
    let (_, got) = w.rest("GET", "/api/v1/admin/access/suggestions", ADMIN, None).await;
    let expired: Vec<&Value> = got["suggestions"].as_array().unwrap().iter().filter(|x| x["kind"] == "grant_expired").collect();
    assert!(expired.iter().any(|x| x["person"] == HOLDER_A), "{got}");
    assert!(!expired.iter().any(|x| x["person"] == HOLDER_B), "{got}");
}

#[tokio::test]
async fn a_grant_that_reached_nothing_in_180_days_is_suggested_only_when_receipts_exist() {
    let w = world().await;
    let insert = |holder: &str, age: &str| format!("INSERT INTO access_grants (holder_kind, holder_id, role, anchor_kind, anchor_id, source, reason, created_by, created_at, valid_from) VALUES ('person', '{holder}', 'viewer', 'container', 'workspace:{}', 'task', 'Seeded', '{ADMIN}', NOW() - INTERVAL '{age}', NOW() - INTERVAL '{age}')", w.w1);
    w.sql(insert(HOLDER_A, "300 days"));
    w.sql(insert(HOLDER_B, "300 days"));
    w.sql(insert(OTHER, "10 days"));
    // With no receipts at all, the claim cannot be made. Setting the world up wrote some, so clear them.
    w.sql("DELETE FROM access_receipts".to_string());
    let (_, none) = w.rest("GET", "/api/v1/admin/access/suggestions", ADMIN, None).await;
    assert!(!none["suggestions"].as_array().unwrap().iter().any(|x| x["kind"] == "grant_unused"), "{none}");
    // A receipt names the grant HOLDER_B used last week.
    w.sql(format!("INSERT INTO access_receipts (actor, action, object_kind, object_id, outcome, grant_ids, counters) SELECT '{HOLDER_B}', 'read_app', 'app', 'x', 'allow', ARRAY[id], '{{}}'::jsonb FROM access_grants WHERE holder_id = '{HOLDER_B}'"));
    let (_, got) = w.rest("GET", "/api/v1/admin/access/suggestions", ADMIN, None).await;
    let unused: Vec<&Value> = got["suggestions"].as_array().unwrap().iter().filter(|x| x["kind"] == "grant_unused").collect();
    assert!(unused.iter().any(|x| x["person"] == HOLDER_A), "an old grant nothing used: {got}");
    assert!(!unused.iter().any(|x| x["person"] == HOLDER_B), "one a receipt names is used: {got}");
    assert!(!unused.iter().any(|x| x["person"] == OTHER), "a new grant has had no time to be used: {got}");
}

#[tokio::test]
async fn a_protected_field_with_no_mask_of_its_own_is_suggested_and_one_with_a_mask_is_not() {
    let w = world().await;
    let (_, got) = w.rest("GET", "/api/v1/admin/access/suggestions", ADMIN, None).await;
    let fields: Vec<&Value> = got["suggestions"].as_array().unwrap().iter().filter(|x| x["kind"] == "field_unmasked").collect();
    assert!(fields.iter().any(|x| x["app_slug"] == w.app1.as_str() && x["field"] == "ssn"), "{got}");
    assert!(!fields.iter().any(|x| x["field"] == "dob"), "a field that names a mask is not: {got}");
    assert!(!fields.iter().any(|x| x["field"] == "title"), "a field that is not protected is not: {got}");
}

// 5. Replacing the suggested memberships with the suggested holder_unit grant leaves every person's role unchanged.
#[tokio::test]
async fn replacing_the_suggested_memberships_with_the_suggested_grant_changes_no_one_s_role() {
    let w = world().await;
    for (who, ws) in [(HOLDER_A, &w.w1), (HOLDER_A, &w.w2), (HOLDER_B, &w.w1), (HOLDER_B, &w.w2)] {
        let (s, b) = w.rest("POST", &format!("/api/v1/workspaces/{ws}/collaborators"), ADMIN, Some(json!({ "eppn": who, "role": "editor" }))).await;
        assert_eq!(s, StatusCode::CREATED, "{b}");
    }
    let objects = [format!("workspace:{}", w.w1), format!("workspace:{}", w.w2), format!("app:{}", w.app1)];
    let people = [HOLDER_A, HOLDER_B, OTHER, TARGET];
    let before = snapshot_roles(&w, &objects, &people).await;
    let (_, got) = w.rest("GET", "/api/v1/admin/access/suggestions", ADMIN, None).await;
    let group = got["suggestions"].as_array().unwrap().iter().find(|x| x["kind"] == "holder_unit_grant").unwrap_or_else(|| panic!("{got}")).clone();
    // Draft the grant, confirm it as a different administrator, then remove the memberships.
    let token = w.agent(ADMIN, "propose", "Tidy memberships").await;
    let draft = tool(&w.app, &token, "draft_access_change", json!({ "kind": "grant", "change": group["grant"], "reason": "One grant for the position" })).await.unwrap();
    let id = draft["change_id"].as_str().unwrap();
    let (s, done) = w.rest("POST", &format!("/api/v1/admin/access/changes/{id}/decide"), SECOND, Some(json!({ "decision": "confirm", "reason": "Reviewed" }))).await;
    assert_eq!(s, StatusCode::OK, "{done}");
    for m in group["memberships"].as_array().unwrap() {
        let (s, b) = w.rest("DELETE", &format!("/api/v1/workspaces/{}/collaborators/{}", m["workspace_id"].as_str().unwrap(), m["person"].as_str().unwrap()), ADMIN, None).await;
        assert!(s.is_success(), "{s} {b}");
    }
    let after = snapshot_roles(&w, &objects, &people).await;
    assert_eq!(before, after, "no one's role changed on any object");
}

async fn snapshot_roles(w: &World, objects: &[String], people: &[&str]) -> Vec<(String, String, Value)> {
    let mut out = Vec::new();
    for o in objects {
        for p in people {
            let (_, e) = w.rest("GET", &format!("/api/v1/access/explain?person={p}&object={o}"), ADMIN, None).await;
            out.push((o.clone(), p.to_string(), e["role"].clone()));
        }
    }
    out
}

// 6. A widening draft cannot be confirmed by the person who asked their AI tool to draft it.
#[tokio::test]
async fn a_widening_draft_is_not_confirmed_by_the_person_it_was_drafted_for_and_a_narrowing_one_may_be() {
    let w = world().await;
    let token = w.agent(ADMIN, "propose", "Give the auditor access").await;
    let draft = tool(&w.app, &token, "draft_access_change", json!({ "kind": "grant", "change": w.grant_body("viewer"), "reason": "The audit needs it" })).await.unwrap();
    let id = draft["change_id"].as_str().unwrap();
    let (s, refused) = w.rest("POST", &format!("/api/v1/admin/access/changes/{id}/decide"), ADMIN, Some(json!({ "decision": "confirm", "reason": "Mine" }))).await;
    assert_eq!(s, StatusCode::FORBIDDEN, "{refused}");
    assert_eq!(w.reads(TARGET).await, StatusCode::FORBIDDEN);
    let (s, done) = w.rest("POST", &format!("/api/v1/admin/access/changes/{id}/decide"), SECOND, Some(json!({ "decision": "confirm", "reason": "Reviewed" }))).await;
    assert_eq!(s, StatusCode::OK, "{done}");
    assert_eq!(w.reads(TARGET).await, StatusCode::OK, "a different person confirmed it");

    // Taking access away narrows. The person it was drafted for may confirm that.
    let grant_id = done["grant_id"].as_i64().unwrap();
    let revoke = tool(&w.app, &token, "draft_access_change", json!({ "kind": "revoke", "change": { "grant_id": grant_id }, "reason": "The audit is done" })).await.unwrap();
    assert_eq!(revoke["widens"], false, "{revoke}");
    assert_eq!(w.reads(TARGET).await, StatusCode::OK, "a draft applies nothing");
    let rid = revoke["change_id"].as_str().unwrap();
    let (s, done) = w.rest("POST", &format!("/api/v1/admin/access/changes/{rid}/decide"), ADMIN, Some(json!({ "decision": "confirm", "reason": "Done" }))).await;
    assert_eq!(s, StatusCode::OK, "{done}");
    assert_eq!(w.reads(TARGET).await, StatusCode::FORBIDDEN, "the grant ended when a person confirmed the revocation");
}

// 7. A stored draft records the person, the tool and the task. A draft a week old is expired and cannot be confirmed.
#[tokio::test]
async fn a_draft_records_who_for_which_tool_and_task_and_expires_after_seven_days() {
    let w = world().await;
    let token = w.agent(ADMIN, "propose", "Give the auditor access").await;
    let draft = tool(&w.app, &token, "draft_access_change", json!({ "kind": "grant", "change": w.grant_body("viewer"), "reason": "The audit needs it" })).await.unwrap();
    let id = draft["change_id"].as_str().unwrap().to_string();
    let (_, listed) = w.rest("GET", "/api/v1/admin/access/grants?state=pending", ADMIN, None).await;
    let change = listed["changes"].as_array().unwrap().iter().find(|c| c["change_id"] == id.as_str()).unwrap_or_else(|| panic!("{listed}")).clone();
    assert_eq!(change["proposed_via"], "agent", "{change}");
    assert_eq!(change["drafted_for"], ADMIN);
    assert_eq!(change["task"], "Give the auditor access");
    assert!(change["via_client"].is_string(), "the tool: {change}");
    let days = chrono::DateTime::parse_from_rfc3339(change["expires_at"].as_str().unwrap()).unwrap().signed_duration_since(chrono::Utc::now()).num_hours();
    assert!((166..=168).contains(&days), "seven days, not {days} hours");

    w.sql(format!("UPDATE access_changes SET expires_at = NOW() - INTERVAL '1 minute' WHERE id = '{id}'"));
    let (s, refused) = w.rest("POST", &format!("/api/v1/admin/access/changes/{id}/decide"), SECOND, Some(json!({ "decision": "confirm", "reason": "Late" }))).await;
    assert_eq!(s, StatusCode::CONFLICT, "{refused}");
    assert!(refused.to_string().to_lowercase().contains("expired"), "{refused}");
    assert_eq!(w.reads(TARGET).await, StatusCode::FORBIDDEN, "no access changed");
    assert_eq!(w.count("SELECT COUNT(*) FROM access_changes WHERE status = 'Confirmed'"), 0);
    // A person's own proposal does not expire.
    let (s, own) = w.rest("POST", "/api/v1/admin/access/grants", ADMIN, Some(w.grant_body("viewer"))).await;
    assert_eq!(s, StatusCode::ACCEPTED, "{own}");
    let (_, listed) = w.rest("GET", "/api/v1/admin/access/grants?state=pending", ADMIN, None).await;
    let mine = listed["changes"].as_array().unwrap().iter().find(|c| c["change_id"] == own["change_id"]).unwrap();
    assert!(mine["expires_at"].is_null() && mine["proposed_via"] == "person", "{mine}");
}

// The tidy_access prompt tells the tool to read the suggestions, draft at most five, preview each and never confirm.
#[tokio::test]
async fn the_tidy_access_prompt_reads_the_suggestions_previews_each_and_never_confirms() {
    let w = world().await;
    let (_, listed) = w.rest("POST", "/api/mcp", ADMIN, Some(json!({ "jsonrpc": "2.0", "id": 1, "method": "prompts/list" }))).await;
    assert!(listed["result"]["prompts"].as_array().unwrap().iter().any(|p| p["name"] == "tidy_access"), "{listed}");
    let (_, got) = w.rest("POST", "/api/mcp", ADMIN, Some(json!({ "jsonrpc": "2.0", "id": 2, "method": "prompts/get", "params": { "name": "tidy_access", "arguments": {} } }))).await;
    let text = got["result"]["messages"][0]["content"]["text"].as_str().unwrap();
    for needle in ["access_suggestions", "draft_access_change", "preview", "at most five", "never confirm"] {
        assert!(text.to_lowercase().contains(needle), "{needle} in {text}");
    }
}
