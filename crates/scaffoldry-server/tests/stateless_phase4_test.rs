//! Stateless server phase 4: app definitions and process definitions are read through the cache mechanism, an
//! approved change is live on every instance at the next request, and a definition that no longer parses fails
//! for its own app only.

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

async fn send(app: &Router, method: &str, uri: &str, who: &str, body: Option<Value>) -> (StatusCode, Value) {
    send_with(app, method, uri, &issue_test_token_and_user(who), body).await
}

/// The same with a token already minted. A token is issued against the database named for the calling thread, so a
/// task that runs on another thread needs one made on the test's own.
async fn send_with(app: &Router, method: &str, uri: &str, token: &str, body: Option<Value>) -> (StatusCode, Value) {
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

fn field(name: &str) -> Value {
    json!({ "name": name, "label": name, "field_type": "Text", "required": false, "ferpa_sensitive": false })
}

fn gpa() -> Value {
    json!({ "name": "gpa", "label": "gpa", "field_type": "Number", "required": false, "ferpa_sensitive": false })
}

/// Applications with a gpa, a name and a status. `extra` adds one more field.
fn manifest(slug: &str, extra: Option<&str>) -> Value {
    let mut fields = vec![gpa(), field("name"), field("status")];
    if let Some(e) = extra {
        fields.push(field(e));
    }
    json!({
        "slug": slug, "title": "Applications", "description": "d", "organization_code": "PHYS", "department": "physics",
        "views": [],
        "tables": [{ "id": "apps", "name": "Applications", "slug": "apps", "fields": fields }]
    })
}

async fn live_app(app: &Router, slug: &str) {
    let (s, body) = send(app, "POST", &format!("/api/v1/workspaces/{WORKSPACE}/apps"), OWNER, Some(manifest(slug, None))).await;
    assert_eq!(s, StatusCode::CREATED, "{body}");
}

/// Proposes `manifest` for the app and approves it, through `app`.
async fn change_app(app: &Router, slug: &str, extra: &str) -> String {
    let (err, p) = tool(app, OWNER, "propose_app_change", json!({ "workspace_id": WORKSPACE, "manifest": manifest(slug, Some(extra)), "summary": "A field" })).await;
    assert!(!err, "{p}");
    let id = p["proposal_id"].as_str().unwrap().to_string();
    let (err, done) = tool(app, OWNER, "decide_proposal", json!({ "proposal_id": id, "decision": "approve", "rationale": "Reviewed", "digest": p["digest"] })).await;
    assert!(!err, "{done}");
    id
}

fn field_names(schema: &Value) -> Vec<String> {
    schema["tables"].as_array().into_iter().flatten().flat_map(|t| t["fields"].as_array().into_iter().flatten()).filter_map(|f| f["name"].as_str().map(String::from)).collect()
}

fn service_rule(id: &str, new_status: &str) -> Value {
    json!({
        "id": id, "name": format!("Rule {id}"), "enabled": true, "trigger": "RecordCreated",
        "predicates": [{ "field_name": "gpa", "operator": "GreaterThan", "expected_value": "3.85" }],
        "steps": [{ "id": "s1", "when": [], "kind": { "Service": { "action": { "UpdateRecordStatus": { "new_status": new_status } } } } }]
    })
}

async fn change_rule(app: &Router, slug: &str, rule: Value) {
    let (err, p) = tool(app, OWNER, "propose_app_change", json!({ "workspace_id": WORKSPACE, "app_slug": slug, "rules": [rule], "summary": "A rule" })).await;
    assert!(!err, "{p}");
    let (err, done) = tool(app, OWNER, "decide_proposal", json!({ "proposal_id": p["proposal_id"], "decision": "approve", "rationale": "Reviewed", "digest": p["digest"] })).await;
    assert!(!err, "{done}");
}

async fn new_record_status(app: &Router, slug: &str) -> Value {
    let (s, body) = send(app, "POST", &format!("/api/v1/apps/{slug}/records"), OWNER, Some(json!({ "data": { "_table_id": "apps", "name": "Ada", "gpa": 3.9 } }))).await;
    assert_eq!(s, StatusCode::CREATED, "{body}");
    body["data"]["status"].clone()
}

fn seeded_pair(db: &TestDb) -> (Router, std::sync::Arc<scaffoldry_server::state::ServerState>) {
    let state = db.state();
    state.seed_demo().expect("seed");
    (build_app_with_state(state.clone()).expect("router"), state)
}

// 1. A proposal approved through A makes the new field visible through B on the next request.
#[tokio::test]
async fn a_proposal_approved_through_one_instance_shows_its_field_through_the_other() {
    let db = TestDb::new();
    let (a, _) = seeded_pair(&db);
    let slug = format!("p4-field-{}", &Uuid::new_v4().to_string()[..8]);
    live_app(&a, &slug).await;
    let b = db.second_app();
    let (s, before) = send(&b, "GET", &format!("/api/v1/apps/{slug}/schema"), OWNER, None).await;
    assert_eq!(s, StatusCode::OK, "{before}");
    assert!(!field_names(&before).contains(&"nickname".to_string()));

    change_app(&a, &slug, "nickname").await;
    let (s, after) = send(&b, "GET", &format!("/api/v1/apps/{slug}/schema"), OWNER, None).await;
    assert_eq!(s, StatusCode::OK, "{after}");
    assert!(field_names(&after).contains(&"nickname".to_string()), "B shows the field A approved: {after}");
}

// 2. A process definition changed through A is the one B starts on the next record change.
#[tokio::test]
async fn a_process_definition_changed_through_one_instance_is_the_one_the_other_starts() {
    let db = TestDb::new();
    let (a, _) = seeded_pair(&db);
    let slug = format!("p4-rule-{}", &Uuid::new_v4().to_string()[..8]);
    live_app(&a, &slug).await;
    let b = db.second_app();

    let rule_id = format!("r-{}", &Uuid::new_v4().to_string()[..8]);
    change_rule(&a, &slug, service_rule(&rule_id, "Approved")).await;
    assert_eq!(new_record_status(&b, &slug).await, "Approved", "B runs the rule A approved");

    change_rule(&a, &slug, service_rule(&rule_id, "Waitlisted")).await;
    assert_eq!(new_record_status(&b, &slug).await, "Waitlisted", "and the version A approved after it");
}

// 3. Two instances approve the same proposal at once. One answer is 200 and one is 409, and the ledger has one entry.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn two_instances_approving_one_proposal_at_once_publish_it_once() {
    let db = TestDb::new();
    let (a, _) = seeded_pair(&db);
    let slug = format!("p4-race-{}", &Uuid::new_v4().to_string()[..8]);
    live_app(&a, &slug).await;
    let b = db.second_app();
    let (err, p) = tool(&a, OWNER, "propose_app_change", json!({ "workspace_id": WORKSPACE, "manifest": manifest(&slug, Some("nickname")), "summary": "A field" })).await;
    assert!(!err, "{p}");
    let id = p["proposal_id"].as_str().unwrap().to_string();
    let digest = p["digest"].clone();

    let token = issue_test_token_and_user(OWNER);
    let decide = |app: Router, id: String| {
        let (token, digest) = (token.clone(), digest.clone());
        tokio::spawn(async move { send_with(&app, "POST", &format!("/api/v1/proposals/{id}/decision"), &token, Some(json!({ "decision": "approve", "rationale": "Reviewed", "digest": digest }))).await })
    };
    let (x, y) = tokio::join!(decide(a.clone(), id.clone()), decide(b.clone(), id.clone()));
    let mut codes = vec![x.unwrap().0.as_u16(), y.unwrap().0.as_u16()];
    codes.sort();
    assert_eq!(codes, [200, 409], "one approval wins and the other is refused");

    let published: i64 = db.run(move |c| c.query_one("SELECT count(*) FROM governance_ledger WHERE decision_type = 'AppPublished' AND payload->>'proposal_id' = $1", &[&id]).unwrap().get(0));
    assert_eq!(published, 1, "one AppPublished entry for the proposal");
}

// 5. An app whose stored definition does not parse returns storage_unavailable for that app. Another app still answers.
#[tokio::test]
async fn an_app_that_no_longer_parses_fails_alone() {
    let db = TestDb::new();
    let (a, _) = seeded_pair(&db);
    let (good, broken) = (format!("p4-good-{}", &Uuid::new_v4().to_string()[..8]), format!("p4-broken-{}", &Uuid::new_v4().to_string()[..8]));
    live_app(&a, &good).await;
    live_app(&a, &broken).await;
    let b = db.second_app();
    let (s, body) = send(&b, "GET", &format!("/api/v1/apps/{broken}/schema"), OWNER, None).await;
    assert_eq!(s, StatusCode::OK, "{body}");

    // The stored definition is damaged and its version moves, as a write would move it.
    let slug = broken.clone();
    db.run(move |c| {
        c.execute("UPDATE app_manifests SET manifest = '{\"slug\": \"broken\"}'::jsonb, version = version + 1 WHERE slug = $1", &[&slug]).unwrap();
        c.execute("UPDATE cache_versions SET version = version + 1 WHERE name = 'manifests'", &[]).unwrap();
    });

    let (s, body) = send(&b, "GET", &format!("/api/v1/apps/{broken}/schema"), OWNER, None).await;
    assert_eq!(s, StatusCode::SERVICE_UNAVAILABLE, "the damaged app fails: {body}");
    assert_eq!(body["code"], "storage_unavailable", "{body}");
    assert!(body.to_string().contains(&broken), "and names the app: {body}");
    let (s, body) = send(&b, "GET", &format!("/api/v1/apps/{good}/schema"), OWNER, None).await;
    assert_eq!(s, StatusCode::OK, "another app on the same instance still answers: {body}");

    // A server that starts afterwards does the same, and starts.
    let c = db.app();
    let (s, _) = send(&c, "GET", &format!("/api/v1/apps/{broken}/schema"), OWNER, None).await;
    assert_eq!(s, StatusCode::SERVICE_UNAVAILABLE, "a new server starts and fails only that app");
    let (s, _) = send(&c, "GET", &format!("/api/v1/apps/{good}/schema"), OWNER, None).await;
    assert_eq!(s, StatusCode::OK);
}

// 3 (policy). Policy activation bumps `policy` in the activating transaction, and the engine is swapped by `refresh`.
//
// `admin-console.md` phase 5 has not run. The policy engine is a plain field of `ServerState` built once at start,
// there is no `policy_versions` table, and nothing activates a policy. So there is nothing to swap yet. This test is
// written for that phase and says what it must show: a policy activated through A is the one B decides with on its next
// request, with no direct swap in the activating route.
#[test]
#[ignore = "needs admin-console.md phase 5: there is no policy activation yet"]
fn a_policy_activated_through_one_instance_is_the_one_the_other_decides_with() {
    todo!("admin-console.md phase 5: activate a policy through A, then show B denies what the new policy denies");
}
