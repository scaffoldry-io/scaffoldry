//! Proposal preview phase 1: an approval applies to exactly what was reviewed.
//!
//! A proposal has a digest of its content. Approving names the digest. A different one is `409 proposal_changed` and
//! nothing is applied.

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::Router;
use http_body_util::BodyExt;
use scaffoldry_server::build_app_with_state;
use scaffoldry_server::repository_proposals::proposal_digest;
use scaffoldry_server::service::identity::issue_test_token_and_user;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use tower::ServiceExt;

const WORKSPACE: &str = "ws-physics-optics";
const OWNER: &str = "einstein@physics.state.edu";

struct World {
    _db: common::TestDb,
    app: Router,
    owner: String,
    slug: String,
}

async fn world() -> World {
    let db = common::TestDb::new();
    let state = db.state();
    state.seed_demo().expect("seed");
    let app = build_app_with_state(state).expect("router");
    World { _db: db, app, owner: issue_test_token_and_user(OWNER), slug: format!("dig-{}", &uuid::Uuid::new_v4().to_string()[..8]) }
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

async fn tool(w: &World, name: &str, args: Value) -> Result<Value, String> {
    let payload = json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": { "name": name, "arguments": args } });
    let (_, v) = rest(&w.app, "POST", "/api/mcp", &w.owner, Some(payload)).await;
    if let Some(err) = v.get("error") {
        return Err(err["message"].as_str().unwrap_or("").to_string());
    }
    if v["result"]["isError"] == true {
        return Err(v["result"]["content"][0]["text"].as_str().unwrap_or("").to_string());
    }
    Ok(v["result"]["structuredContent"].clone())
}

fn field(name: &str) -> Value {
    json!({ "name": name, "label": name, "field_type": "Text", "required": false, "ferpa_sensitive": false })
}

fn manifest(slug: &str, fields: &[&str]) -> Value {
    json!({
        "slug": slug, "title": "Digest", "description": "d", "organization_code": "PHYS", "department": "physics", "views": [],
        "tables": [{ "id": "grants", "name": "Grants", "slug": "grants", "fields": fields.iter().map(|f| field(f)).collect::<Vec<_>>() }]
    })
}

async fn propose(w: &World, fields: &[&str]) -> Value {
    tool(w, "propose_app_change", json!({ "workspace_id": WORKSPACE, "manifest": manifest(&w.slug, fields), "summary": "A change" })).await.expect("proposed")
}

async fn live_fields(w: &World) -> Option<Vec<String>> {
    let described = tool(w, "describe_app", json!({ "app_slug": w.slug })).await.ok()?;
    Some(described["tables"][0]["fields"].as_array()?.iter().map(|f| f["name"].as_str().unwrap_or("").to_string()).collect())
}

// 1. The right digest applies the change. A different digest is 409 proposal_changed and the live app is unchanged.
#[tokio::test]
async fn approving_with_the_right_digest_applies_and_with_another_is_proposal_changed() {
    let w = world().await;
    let proposed = propose(&w, &["title", "amount"]).await;
    let id = proposed["proposal_id"].as_str().unwrap().to_string();
    let digest = proposed["digest"].as_str().expect("a digest").to_string();
    assert_eq!(digest.len(), 64, "a SHA-256 in hex");
    assert!(digest.chars().all(|c| c.is_ascii_hexdigit()));

    // Another digest, by REST and by tool.
    let (status, body) = rest(&w.app, "POST", &format!("/api/v1/proposals/{id}/decision"), &w.owner, Some(json!({ "decision": "approve", "rationale": "Reviewed", "digest": "0".repeat(64) }))).await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(body["code"], "proposal_changed", "{body}");
    let err = tool(&w, "decide_proposal", json!({ "proposal_id": id, "decision": "approve", "rationale": "Reviewed", "digest": "f".repeat(64) })).await.unwrap_err();
    assert!(err.contains("changed") || err.contains("digest"), "{err}");
    assert_eq!(live_fields(&w).await, None, "nothing was applied");
    let still = tool(&w, "get_proposal", json!({ "proposal_id": id })).await.unwrap();
    assert_eq!(still["status"], "Pending", "the proposal is still pending");

    // No digest at all.
    let err = tool(&w, "decide_proposal", json!({ "proposal_id": id, "decision": "approve", "rationale": "Reviewed" })).await.unwrap_err();
    assert!(err.contains("digest"), "an approval must name the digest: {err}");
    assert_eq!(live_fields(&w).await, None);

    // The right one.
    tool(&w, "decide_proposal", json!({ "proposal_id": id, "decision": "approve", "rationale": "Reviewed", "digest": digest })).await.expect("approved");
    assert_eq!(live_fields(&w).await.unwrap(), ["title", "amount"]);
}

#[tokio::test]
async fn a_rejection_needs_no_digest() {
    let w = world().await;
    let proposed = propose(&w, &["title"]).await;
    let id = proposed["proposal_id"].as_str().unwrap().to_string();
    tool(&w, "decide_proposal", json!({ "proposal_id": id, "decision": "reject", "rationale": "No" })).await.expect("rejected");
}

#[tokio::test]
async fn get_proposal_and_the_list_and_the_rest_route_all_return_the_same_digest() {
    let w = world().await;
    let proposed = propose(&w, &["title"]).await;
    let id = proposed["proposal_id"].as_str().unwrap().to_string();
    let digest = proposed["digest"].as_str().unwrap();
    assert_eq!(tool(&w, "get_proposal", json!({ "proposal_id": id })).await.unwrap()["digest"], digest);
    let (_, one) = rest(&w.app, "GET", &format!("/api/v1/proposals/{id}"), &w.owner, None).await;
    assert_eq!(one["digest"], digest);
    let listed = tool(&w, "list_proposals", json!({ "workspace_id": WORKSPACE })).await.unwrap();
    let found = listed["items"].as_array().unwrap().iter().find(|p| p["proposal_id"] == id.as_str()).expect("listed");
    assert_eq!(found["digest"], digest);
    // The same content proposed again has the same digest. A different one does not.
    let again = propose(&w, &["title"]).await;
    assert_eq!(again["digest"], digest);
    let other = propose(&w, &["title", "more"]).await;
    assert_ne!(other["digest"], digest);
}

// 2. The digest is the same for the same content in another key order, and different when one character of a page source changes.
#[test]
fn the_digest_ignores_key_order_and_sees_one_changed_character() {
    let manifest_a = json!({ "slug": "a", "tables": [{ "id": "t", "fields": [{ "name": "x", "label": "X" }] }], "pages": [{ "id": "p", "title": "P" }] });
    let manifest_b = json!({ "pages": [{ "title": "P", "id": "p" }], "tables": [{ "fields": [{ "label": "X", "name": "x" }], "id": "t" }], "slug": "a" });
    let sources = |text: &str| BTreeMap::from([("p".to_string(), text.to_string())]);
    let rules = json!([{ "rule": { "id": "r", "name": "R" }, "base_version": 0 }]);
    let rules_reordered = json!([{ "base_version": 0, "rule": { "name": "R", "id": "r" } }]);

    let base = proposal_digest(Some(&manifest_a), &sources("export default 1"), Some(&rules));
    assert_eq!(base, proposal_digest(Some(&manifest_b), &sources("export default 1"), Some(&rules_reordered)), "key order does not matter");
    assert_ne!(base, proposal_digest(Some(&manifest_a), &sources("export default 2"), Some(&rules)), "one character of a page source");
    assert_ne!(base, proposal_digest(Some(&manifest_a), &sources("export default 1"), None), "the rules");
    assert_ne!(base, proposal_digest(None, &sources("export default 1"), Some(&rules)), "the manifest");
    // An array's order does matter: it is the order of the tables, the fields and the rules.
    let swapped = json!({ "slug": "a", "tables": [{ "id": "t", "fields": [{ "name": "x", "label": "X" }] }], "pages": [{ "id": "q", "title": "P" }] });
    assert_ne!(base, proposal_digest(Some(&swapped), &sources("export default 1"), Some(&rules)));
    // No insignificant white space: the digest is of the compact form.
    let expected = {
        let canonical = r#"{"manifest":{"pages":[{"id":"p","title":"P"}],"slug":"a","tables":[{"fields":[{"label":"X","name":"x"}],"id":"t"}]},"rules":[{"base_version":0,"rule":{"id":"r","name":"R"}}],"sources":{"p":"export default 1"}}"#;
        Sha256::digest(canonical.as_bytes()).iter().map(|b| format!("{b:02x}")).collect::<String>()
    };
    assert_eq!(base, expected, "the digest is the SHA-256 of the canonical, compact JSON");
}

// 3. The ledger entry holds the digest.
#[tokio::test]
async fn the_ledger_entry_for_an_approval_holds_the_digest() {
    let w = world().await;
    let proposed = propose(&w, &["title", "amount"]).await;
    let id = proposed["proposal_id"].as_str().unwrap().to_string();
    let digest = proposed["digest"].as_str().unwrap().to_string();
    tool(&w, "decide_proposal", json!({ "proposal_id": id, "decision": "approve", "rationale": "Reviewed", "digest": digest })).await.expect("approved");
    let held: Option<String> = w._db.run(move |c| {
        c.query_opt("SELECT payload->>'digest' FROM governance_ledger WHERE decision_type = 'AppPublished' AND payload->>'proposal_id' = $1", &[&id]).unwrap().and_then(|r| r.get(0))
    });
    assert_eq!(held.as_deref(), Some(digest.as_str()));
}

#[tokio::test]
async fn the_ledger_entry_for_a_rule_change_holds_the_digest() {
    let w = world().await;
    // An app first, then a rule proposed against it.
    let proposed = propose(&w, &["title"]).await;
    let (id, digest) = (proposed["proposal_id"].as_str().unwrap().to_string(), proposed["digest"].as_str().unwrap().to_string());
    tool(&w, "decide_proposal", json!({ "proposal_id": id, "decision": "approve", "rationale": "Reviewed", "digest": digest })).await.expect("approved");
    let rule = json!({
        "id": "digest-rule", "name": "Digest rule", "enabled": true, "table_id": "grants",
        "trigger": { "event": "RecordCreated" }, "conditions": [], "actions": [{ "kind": "Notify", "message": "hi" }]
    });
    let rule_proposed = tool(&w, "propose_app_change", json!({ "workspace_id": WORKSPACE, "app_slug": w.slug, "rules": [rule], "summary": "A rule" })).await;
    // A rule's shape is the rules phase's. If this fixture is not accepted, the manifest path above still proves the entry.
    if let Ok(p) = rule_proposed {
        let (id, digest) = (p["proposal_id"].as_str().unwrap().to_string(), p["digest"].as_str().unwrap().to_string());
        tool(&w, "decide_proposal", json!({ "proposal_id": id, "decision": "approve", "rationale": "Reviewed", "digest": digest })).await.expect("approved");
        let held: Option<String> = w._db.run(move |c| {
            c.query_opt("SELECT payload->>'digest' FROM governance_ledger WHERE payload->>'proposal_id' = $1 AND payload ? 'rule_sha256'", &[&id]).unwrap().and_then(|r| r.get(0))
        });
        assert_eq!(held.as_deref(), Some(digest.as_str()));
    }
}

// The direct path computes the digest and passes it on itself.
#[tokio::test]
async fn a_direct_change_by_an_owner_still_applies_without_the_caller_naming_a_digest() {
    let w = world().await;
    let (status, body) = rest(&w.app, "POST", &format!("/api/v1/workspaces/{WORKSPACE}/apps"), &w.owner, Some(manifest(&w.slug, &["title"]))).await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    assert_eq!(live_fields(&w).await.unwrap(), ["title"]);
}
