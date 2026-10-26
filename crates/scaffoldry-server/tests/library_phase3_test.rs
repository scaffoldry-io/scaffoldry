//! Policy library phase 3: review, answers, and publishing.

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use common::TestDb;
use http_body_util::BodyExt;
use scaffoldry_server::build_app_with_state;
use scaffoldry_server::service::identity::issue_test_token_and_user;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tower::ServiceExt;

const ADMIN: &str = "jordan.lee@state.edu";
const R1: &str = "reviewer.one@state.edu";
const R2: &str = "reviewer.two@state.edu";
const FACULTY: &str = "prof.curie@science.state.edu";
const ROOT: &str = "00000000-0000-0000-0000-000000000001";
const P1: &str = "Student records are kept for seven years after the student leaves.";

async fn send(app: &axum::Router, method: &str, uri: &str, who: &str, body: Option<Value>) -> (StatusCode, Value) {
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

struct World {
    db: TestDb,
    app: axum::Router,
    policy: String,
    snapshot: String,
}

fn filler(n: usize) -> String {
    "Lorem ipsum dolor sit amet. ".repeat(n)
}

/// A policy whose owner is the `records-officer` position of a new department. `holders` are the people in it.
async fn world(holders: &[&str], text: &str, origin_supplied: bool) -> World {
    let _ = origin_supplied;
    let db = TestDb::new();
    let app = build_app_with_state(db.state()).expect("router");
    for who in [R1, R2, FACULTY] {
        issue_test_token_and_user(who);
    }
    let code = format!("L{}", &uuid::Uuid::new_v4().simple().to_string()[..8]);
    let (s, unit) = send(&app, "POST", "/api/v1/orgs", ADMIN, Some(json!({ "name": code, "code": code, "org_type": "Department", "parent_id": ROOT }))).await;
    assert_eq!(s, StatusCode::CREATED, "{unit}");
    let unit = unit["id"].as_str().unwrap().to_string();
    let key = format!("records-officer-{}", code.to_lowercase());
    let (s, p) = send(&app, "POST", "/api/v1/admin/positions", ADMIN, Some(json!({ "key": key, "name": "Records officer", "description": "Test", "org_types": ["Department"], "max_holders": 5, "reason": "Define" }))).await;
    assert_eq!(s, StatusCode::CREATED, "{p}");
    for h in holders {
        let (s, b) = send(&app, "POST", &format!("/api/v1/orgs/{unit}/positions/{key}/holders"), ADMIN, Some(json!({ "eppn": h, "replace": false, "reason": "Assign" }))).await;
        assert_eq!(s, StatusCode::CREATED, "{b}");
    }
    let body = json!({ "key": "records", "title": "Records retention", "organization_id": unit, "owner": { "Position": { "key": key, "from": "Workspace", "walk_up": false } }, "reason": "Owned by the records officers" });
    let (s, p) = send(&app, "POST", "/api/v1/admin/library/policies", ADMIN, Some(body)).await;
    assert_eq!(s, StatusCode::CREATED, "{p}");
    let policy = p["policy"]["id"].as_str().unwrap().to_string();
    let (_, src) = send(&app, "POST", &format!("/api/v1/admin/library/policies/{policy}/sources"), ADMIN, Some(json!({ "title": "Schedule", "kind": "text", "reason": "r" }))).await;
    let source = src["source"]["id"].as_str().unwrap().to_string();
    let (s, snap) = send(&app, "POST", &format!("/api/v1/admin/library/sources/{source}/snapshots"), ADMIN, Some(json!({ "text": text, "reason": "r" }))).await;
    assert_eq!(s, StatusCode::CREATED, "{snap}");
    World { db, app, policy, snapshot: snap["snapshot"]["id"].as_str().unwrap().to_string() }
}

async fn default_world(holders: &[&str]) -> World {
    world(holders, &format!("{} {P1} {}", filler(30), filler(30)), false).await
}

/// A rule drafted by `who` through the route and submitted for review. Returns its id.
async fn drafted(w: &World, who: &str, key: &str, submit: bool) -> String {
    let body = json!({ "key": key, "statement": "Student records are kept for seven years.", "applies_to": "student records", "citations": [{ "snapshot_id": w.snapshot, "passage": P1 }], "reason": "Read from the schedule" });
    let (s, r) = send(&w.app, "POST", &format!("/api/v1/admin/library/policies/{}/rules", w.policy), who, Some(body)).await;
    assert_eq!(s, StatusCode::CREATED, "{r}");
    let id = r["rule"]["id"].as_str().unwrap().to_string();
    if submit {
        let (s, r) = send(&w.app, "POST", &format!("/api/v1/library/rules/{id}/submit"), who, Some(json!({}))).await;
        assert_eq!(s, StatusCode::OK, "{r}");
    }
    id
}

async fn decide(w: &World, who: &str, id: &str, body: Value) -> (StatusCode, Value) {
    send(&w.app, "POST", &format!("/api/v1/library/rules/{id}/decide"), who, Some(body)).await
}

fn rows(w: &World, key: &'static str) -> Vec<(i32, String)> {
    w.db.run(move |c| c.query("SELECT version, status FROM library_rules WHERE key = $1 ORDER BY version", &[&key]).unwrap().iter().map(|r| (r.get(0), r.get(1))).collect())
}

// 1. The submitter cannot decide their own rule, even as a Platform Admin.
#[tokio::test]
async fn the_submitter_cannot_decide_their_own_rule() {
    let w = default_world(&[R1, R2]).await;
    let id = drafted(&w, R1, "retention", true).await;
    let (s, b) = decide(&w, R1, &id, json!({ "decision": "publish", "note": "Mine" })).await;
    assert_eq!(s, StatusCode::FORBIDDEN, "a holder who submitted it: {b}");
    let (s, b) = decide(&w, ADMIN, &id, json!({ "decision": "publish", "note": "Admin" })).await;
    assert_eq!(s, StatusCode::FORBIDDEN, "a Platform Admin is not a reviewer unless the owner spec names them: {b}");
    assert_eq!(rows(&w, "retention"), vec![(1, "InReview".to_string())]);
}

#[tokio::test]
async fn an_admin_who_owns_and_submitted_a_rule_still_cannot_decide_it() {
    let w = default_world(&[R1]).await;
    // The owner becomes a person: the Platform Admin.
    let (s, b) = send(&w.app, "PATCH", &format!("/api/v1/admin/library/policies/{}", w.policy), ADMIN, Some(json!({ "owner": { "Person": { "eppn": ADMIN } }, "reason": "Admin owns it" }))).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    let id = drafted(&w, ADMIN, "retention", true).await;
    let (s, b) = decide(&w, ADMIN, &id, json!({ "decision": "publish", "note": "Mine" })).await;
    assert_eq!(s, StatusCode::FORBIDDEN, "{b}");
}

// 2. A reviewer who rewords cannot publish that version. A second reviewer can.
#[tokio::test]
async fn a_reworded_rule_needs_a_different_reviewer() {
    let w = default_world(&[R1, R2]).await;
    let id = drafted(&w, R1, "retention", true).await;
    let (s, b) = decide(&w, R2, &id, json!({ "decision": "publish", "note": "Clearer", "statement": "Student records are kept seven years after the student leaves." })).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    let new = b["rule"].clone();
    assert_eq!(new["version"], 2, "{b}");
    assert_eq!(new["status"], "InReview");
    assert_eq!(new["submitted_by"], R2);
    assert_eq!(new["citations"].as_array().unwrap().len(), 1, "citations carry over");
    let new_id = new["id"].as_str().unwrap().to_string();
    let (s, b) = decide(&w, R2, &new_id, json!({ "decision": "publish", "note": "Again" })).await;
    assert_eq!(s, StatusCode::FORBIDDEN, "the rewording reviewer cannot publish it: {b}");
    let (s, b) = decide(&w, R1, &new_id, json!({ "decision": "publish", "note": "Agreed" })).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_eq!(b["rule"]["status"], "Published");
    assert_eq!(rows(&w, "retention"), vec![(1, "Superseded".to_string()), (2, "Published".to_string())]);
}

// 3. Publishing version 2 supersedes version 1, and one row is Published.
#[tokio::test]
async fn publishing_a_new_version_supersedes_the_old_one() {
    let w = default_world(&[R1, R2]).await;
    let v1 = drafted(&w, R1, "retention", true).await;
    let (s, b) = decide(&w, R2, &v1, json!({ "decision": "publish", "note": "Fine" })).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_eq!(rows(&w, "retention"), vec![(1, "Published".to_string())]);
    let v2 = drafted(&w, R1, "retention", true).await;
    let (s, b) = decide(&w, R2, &v2, json!({ "decision": "publish", "note": "Newer" })).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_eq!(rows(&w, "retention"), vec![(1, "Superseded".to_string()), (2, "Published".to_string())]);
}

// 4. A vacant owner position: the rule is no_approver, listed as Unassigned, and stays InReview.
#[tokio::test]
async fn a_policy_with_no_one_to_decide_is_unassigned_and_stays_in_review() {
    let w = default_world(&[]).await;
    let id = drafted(&w, ADMIN, "retention", true).await;
    let mine = send(&w.app, "GET", "/api/v1/me/library/reviews", ADMIN, None).await.1;
    let un = mine["unassigned"].as_array().unwrap();
    assert!(un.iter().any(|r| r["rule_id"] == id.as_str() && r["no_approver"] == true), "{mine}");
    assert!(mine["reviews"].as_array().unwrap().is_empty(), "{mine}");
    for who in [ADMIN, R1, FACULTY] {
        let (s, _) = decide(&w, who, &id, json!({ "decision": "publish", "note": "x" })).await;
        assert_eq!(s, StatusCode::FORBIDDEN, "{who}");
    }
    assert_eq!(rows(&w, "retention"), vec![(1, "InReview".to_string())]);
}

// 6. The ledger holds the statement's hash.
#[tokio::test]
async fn the_decision_entry_holds_the_sha256_of_the_statement() {
    let w = default_world(&[R1, R2]).await;
    let a = drafted(&w, R1, "one", true).await;
    let (s, _) = decide(&w, R2, &a, json!({ "decision": "publish", "note": "Fine" })).await;
    assert_eq!(s, StatusCode::OK);
    let body = json!({ "key": "two", "statement": "Student records are kept for seven year.", "citations": [{ "snapshot_id": w.snapshot, "passage": P1 }], "reason": "r" });
    let (_, r) = send(&w.app, "POST", &format!("/api/v1/admin/library/policies/{}/rules", w.policy), R1, Some(body)).await;
    let b = r["rule"]["id"].as_str().unwrap().to_string();
    send(&w.app, "POST", &format!("/api/v1/library/rules/{b}/submit"), R1, Some(json!({}))).await;
    let (s, _) = decide(&w, R2, &b, json!({ "decision": "publish", "note": "Fine" })).await;
    assert_eq!(s, StatusCode::OK);
    let payloads: Vec<Value> = w.db.run(|c| c.query("SELECT payload FROM governance_ledger WHERE decision_type = 'PolicyRuleDecided' ORDER BY sequence", &[]).unwrap().iter().map(|r| r.get::<_, Value>(0)).collect());
    assert_eq!(payloads.len(), 2, "{payloads:?}");
    let hash = |t: &str| Sha256::digest(t.as_bytes()).iter().map(|b| format!("{b:02x}")).collect::<String>();
    assert_eq!(payloads[0]["statement_sha256"], hash("Student records are kept for seven years.").as_str(), "{:?}", payloads[0]);
    assert_eq!(payloads[1]["statement_sha256"], hash("Student records are kept for seven year.").as_str());
    assert_ne!(payloads[0]["statement_sha256"], payloads[1]["statement_sha256"], "one character changes the hash");
    assert_eq!(payloads[0]["version"], 1);
    assert!(payloads[0]["rule_id"].is_string());
}

// The review view shows each passage in context, the previous version, and who submitted it and how.
#[tokio::test]
async fn the_review_view_shows_each_passage_with_300_characters_around_it() {
    let w = default_world(&[R1, R2]).await;
    let id = drafted(&w, R1, "retention", true).await;
    let (s, v) = send(&w.app, "GET", &format!("/api/v1/library/rules/{id}/review"), R2, None).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let c = &v["citations"][0];
    assert_eq!(c["passage"], P1);
    assert_eq!(c["before"].as_str().unwrap().chars().count(), 300, "{c}");
    assert_eq!(c["after"].as_str().unwrap().chars().count(), 300);
    assert_eq!(c["text_origin"], "supplied", "a pasted snapshot is marked supplied");
    assert_eq!(v["submitted_by"], R1);
    assert_eq!(v["submitted_via"], "person");
    assert!(v["previous"].is_null());
    assert!(v["questions"].is_array());
    let (s, _) = send(&w.app, "GET", &format!("/api/v1/library/rules/{id}/review"), FACULTY, None).await;
    assert_eq!(s, StatusCode::FORBIDDEN, "only a reviewer sees it");
}

#[tokio::test]
async fn my_reviews_lists_rules_the_caller_may_decide_and_questions_they_may_answer() {
    let w = default_world(&[R1, R2]).await;
    let id = drafted(&w, R1, "retention", true).await;
    let (_, q) = send(&w.app, "POST", "/api/v1/mcp", R1, Some(json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": { "name": "propose_library_rules", "arguments": { "policy_id": w.policy, "questions": [{ "question": "Does a hold pause it?" }] } } }))).await;
    assert!(q["result"]["isError"].is_null(), "{q}");
    let r2 = send(&w.app, "GET", "/api/v1/me/library/reviews", R2, None).await.1;
    assert!(r2["reviews"].as_array().unwrap().iter().any(|r| r["rule_id"] == id.as_str()), "{r2}");
    let r1 = send(&w.app, "GET", "/api/v1/me/library/reviews", R1, None).await.1;
    assert!(r1["reviews"].as_array().unwrap().is_empty(), "the submitter has nothing to decide: {r1}");
    assert_eq!(r1["questions"].as_array().unwrap().len(), 1, "a writer may answer: {r1}");
    let none = send(&w.app, "GET", "/api/v1/me/library/reviews", FACULTY, None).await.1;
    assert!(none["reviews"].as_array().unwrap().is_empty() && none["questions"].as_array().unwrap().is_empty(), "{none}");
}

#[tokio::test]
async fn a_writer_answers_a_question_and_the_review_shows_it() {
    let w = default_world(&[R1, R2]).await;
    let id = drafted(&w, R1, "retention", true).await;
    send(&w.app, "POST", "/api/v1/mcp", R1, Some(json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": { "name": "propose_library_rules", "arguments": { "policy_id": w.policy, "questions": [{ "question": "Does a hold pause it?" }] } } }))).await;
    let qid: String = w.db.run(|c| c.query_one("SELECT id::text FROM library_questions", &[]).unwrap().get(0));
    let (s, _) = send(&w.app, "POST", &format!("/api/v1/library/questions/{qid}/answer"), FACULTY, Some(json!({ "answer": "Yes" }))).await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    let (s, b) = send(&w.app, "POST", &format!("/api/v1/library/questions/{qid}/answer"), R2, Some(json!({ "answer": "Yes, a legal hold pauses it." }))).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    let v = send(&w.app, "GET", &format!("/api/v1/library/rules/{id}/review"), R2, None).await.1;
    let q = &v["questions"][0];
    assert_eq!(q["answer"], "Yes, a legal hold pauses it.", "{v}");
    assert_eq!(q["answered_by"], R2);
}

#[tokio::test]
async fn reject_and_publish_need_a_note_and_retire_needs_a_reason() {
    let w = default_world(&[R1, R2]).await;
    let id = drafted(&w, R1, "retention", true).await;
    let (s, b) = decide(&w, R2, &id, json!({ "decision": "reject" })).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{b}");
    assert!(b["fields"].get("note").is_some(), "{b}");
    let (s, _) = decide(&w, R2, &id, json!({ "decision": "maybe", "note": "n" })).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    let (s, b) = decide(&w, R2, &id, json!({ "decision": "reject", "note": "Not what the text says" })).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_eq!(rows(&w, "retention"), vec![(1, "Rejected".to_string())]);
    let (s, _) = decide(&w, R2, &id, json!({ "decision": "publish", "note": "Again" })).await;
    assert_eq!(s, StatusCode::CONFLICT, "a decided rule is not decided twice");

    let id2 = drafted(&w, R1, "destroy", true).await;
    decide(&w, R2, &id2, json!({ "decision": "publish", "note": "Fine" })).await;
    let (s, _) = send(&w.app, "POST", &format!("/api/v1/library/rules/{id2}/retire"), R2, Some(json!({}))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "a reason is required");
    let (s, b) = send(&w.app, "POST", &format!("/api/v1/library/rules/{id2}/retire"), R2, Some(json!({ "reason": "No longer the law" }))).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_eq!(rows(&w, "destroy"), vec![(1, "Retired".to_string())]);
}

#[tokio::test]
async fn submit_is_for_the_submitter_or_a_writer_and_every_step_leaves_a_ledger_entry() {
    let w = default_world(&[R1, R2]).await;
    let id = drafted(&w, R1, "retention", false).await;
    let (s, _) = send(&w.app, "POST", &format!("/api/v1/library/rules/{id}/submit"), FACULTY, Some(json!({}))).await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    let (s, _) = send(&w.app, "POST", &format!("/api/v1/library/rules/{id}/submit"), R2, Some(json!({}))).await;
    assert_eq!(s, StatusCode::OK, "a policy writer may submit another's draft");
    let (s, _) = send(&w.app, "POST", &format!("/api/v1/library/rules/{id}/submit"), R1, Some(json!({}))).await;
    assert_eq!(s, StatusCode::CONFLICT, "only a Draft is submitted");
    let anon = w.app.clone().oneshot(Request::builder().method("GET").uri("/api/v1/me/library/reviews").body(Body::empty()).unwrap()).await.unwrap();
    assert_eq!(anon.status(), StatusCode::UNAUTHORIZED);
    let (s, _) = decide(&w, R1, &id, json!({ "decision": "publish", "note": "ok" })).await;
    assert_eq!(s, StatusCode::FORBIDDEN, "submitting for the author does not make the other person the submitter, the drafter stays it");
    let (s, _) = decide(&w, R2, &id, json!({ "decision": "publish", "note": "ok" })).await;
    assert_eq!(s, StatusCode::OK);
    let n: i64 = w.db.run(|c| c.query_one("SELECT COUNT(*) FROM governance_ledger WHERE decision_type = 'PolicyRuleDecided'", &[]).unwrap().get(0));
    assert_eq!(n, 1);
}
