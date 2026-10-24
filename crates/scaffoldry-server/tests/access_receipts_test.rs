//! Access model phase 8: a receipt for every denial, every privileged action and every read that returns a protected
//! field. A receipt holds ids, never values. Sealed batches make an edit show.

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::Router;
use common::TestDb;
use http_body_util::BodyExt;
use scaffoldry_server::build_app_with_state;
use scaffoldry_server::service::identity::issue_test_token_and_user;
use scaffoldry_server::state::SharedState;
use serde_json::{json, Value};
use tower::ServiceExt;
use uuid::Uuid;

const WORKSPACE: &str = "ws-physics-optics";
const OWNER: &str = "einstein@physics.state.edu";
const ADMIN: &str = "jordan.lee@state.edu";

struct World {
    db: TestDb,
    state: SharedState,
    app: Router,
}

fn world() -> World {
    let db = TestDb::new();
    let state = db.state();
    state.seed_demo().expect("seed");
    let app = build_app_with_state(state.clone()).expect("router");
    World { db, state, app }
}

async fn send(app: &Router, method: &str, uri: &str, who: &str, body: Option<Value>) -> (StatusCode, Value) {
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

fn field(name: &str, protected: bool) -> Value {
    json!({ "name": name, "label": name, "field_type": "Text", "required": false, "ferpa_sensitive": protected })
}

fn manifest(slug: &str, sensitivity: &str, fields: Vec<Value>) -> Value {
    json!({
        "slug": slug, "title": "Receipts", "description": "d", "organization_code": "PHYS", "department": "physics", "views": [],
        "coordinates": { "sensitivity": sensitivity },
        "tables": [{ "id": "items", "name": "Items", "slug": "items", "fields": fields }]
    })
}

async fn make_app(w: &World, slug: &str, sensitivity: &str, fields: Vec<Value>) {
    // Proposed and approved by two people, since a protected field in an Internal workspace needs a second one.
    let call = |who: &'static str, name: &'static str, args: Value| {
        let app = w.app.clone();
        async move {
            let payload = json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": { "name": name, "arguments": args } });
            let (_, v) = send(&app, "POST", "/api/mcp", who, Some(payload)).await;
            v["result"].clone()
        }
    };
    let proposed = call(OWNER, "propose_app_change", json!({ "workspace_id": WORKSPACE, "manifest": manifest(slug, sensitivity, fields), "summary": "Receipts" })).await;
    let proposal = &proposed["structuredContent"];
    let id = proposal["proposal_id"].as_str().unwrap_or_else(|| panic!("a proposal: {proposed}")).to_string();
    for who in [OWNER, ADMIN] {
        let decided = call(who, "decide_proposal", json!({ "proposal_id": id, "decision": "approve", "rationale": "Reviewed", "digest": proposal["digest"] })).await;
        if decided["isError"] != true {
            return;
        }
    }
    panic!("nobody could approve the app");
}

async fn receipts(w: &World, query: &str) -> Vec<Value> {
    let (s, body) = send(&w.app, "GET", &format!("/api/v1/admin/access/receipts?{query}"), ADMIN, None).await;
    assert_eq!(s, StatusCode::OK, "{body}");
    body["receipts"].as_array().unwrap().clone()
}

fn slug(prefix: &str) -> String {
    format!("{prefix}-{}", &Uuid::new_v4().to_string()[..8])
}

// 1. A denial writes one receipt with the policy id. An allowed read of an unprotected field writes none.
#[tokio::test]
async fn a_denial_writes_one_receipt_with_the_policy_and_an_unprotected_read_writes_none() {
    let w = world();
    let (open, closed) = (slug("rcpt-int"), slug("rcpt-res"));
    make_app(&w, &open, "Internal", vec![field("name", false)]).await;
    make_app(&w, &closed, "Restricted", vec![field("name", false)]).await;
    let (s, body) = send(&w.app, "PUT", &format!("/api/v1/workspaces/{WORKSPACE}/guards"), ADMIN, Some(json!({ "rules": [{ "template": "deny_above_sensitivity", "sensitivity": "Internal" }], "reason": "Nothing above Internal" }))).await;
    assert_eq!(s, StatusCode::OK, "{body}");
    let before = receipts(&w, &format!("actor={OWNER}")).await.len();

    let (s, _) = send(&w.app, "GET", &format!("/api/v1/apps/{open}/schema"), OWNER, None).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(receipts(&w, &format!("actor={OWNER}")).await.len(), before, "a read of an unprotected field writes none");

    let (s, _) = send(&w.app, "GET", &format!("/api/v1/apps/{closed}/schema"), OWNER, None).await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    let after = receipts(&w, &format!("actor={OWNER}&outcome=deny")).await;
    assert_eq!(after.len(), 1, "{after:?}");
    let r = &after[0];
    assert_eq!(r["actor"], OWNER);
    assert_eq!(r["outcome"], "deny");
    assert_eq!(r["object_kind"], "app");
    assert_eq!(r["object_id"], closed.as_str());
    assert!(!r["policy_ids"].as_array().unwrap().is_empty(), "the policy that forbade it is named: {r}");
    assert!(r["counters"]["policy"].is_number() || r["counters"]["guards"].is_number(), "the counters in force are kept: {r}");
    assert!(!r.to_string().contains("Receipts"), "a receipt holds ids and no values");
}

// 2. A list call that returns 200 rows with a protected field writes one receipt.
#[tokio::test]
async fn a_list_of_many_rows_with_a_protected_field_writes_one_receipt() {
    let w = world();
    let s = slug("rcpt-list");
    make_app(&w, &s, "Internal", vec![field("name", false), field("ssn", true)]).await;
    for i in 0..200 {
        let (code, body) = send(&w.app, "POST", &format!("/api/v1/apps/{s}/records"), OWNER, Some(json!({ "data": { "_table_id": "items", "name": format!("n{i}"), "ssn": format!("s{i}") } }))).await;
        assert_eq!(code, StatusCode::CREATED, "{body}");
    }
    // Saving a record answers with the record, so each save left a receipt of its own. Only what the list adds is counted.
    let newest = receipts(&w, &format!("actor={OWNER}&object={s}&limit=1")).await[0]["id"].as_i64().unwrap();
    let (code, page) = send(&w.app, "GET", &format!("/api/v1/apps/{s}/tables/items/records?limit=200"), OWNER, None).await;
    assert_eq!(code, StatusCode::OK);
    assert_eq!(page["records"].as_array().unwrap().len(), 200);
    let added: Vec<Value> = receipts(&w, &format!("actor={OWNER}&object={s}&limit=200")).await.into_iter().filter(|r| r["id"].as_i64().unwrap() > newest).collect();
    assert_eq!(added.len(), 1, "one receipt for the call, not one per row: {added:?}");
    assert_eq!(added[0]["action"], "read_protected");
    assert_eq!(added[0]["outcome"], "allow");
}

// 3. Editing a receipt in a sealed batch makes verification fail and name the batch.
#[tokio::test]
async fn editing_a_sealed_receipt_fails_verification_and_names_the_batch() {
    let w = world();
    let s = slug("rcpt-seal");
    make_app(&w, &s, "Restricted", vec![field("name", false)]).await;
    send(&w.app, "PUT", &format!("/api/v1/workspaces/{WORKSPACE}/guards"), ADMIN, Some(json!({ "rules": [{ "template": "deny_above_sensitivity", "sensitivity": "Internal" }], "reason": "Nothing above Internal" }))).await;
    for _ in 0..3 {
        send(&w.app, "GET", &format!("/api/v1/apps/{s}/schema"), OWNER, None).await;
    }
    let batch = scaffoldry_server::service::receipts::seal(&w.state).expect("a seal").expect("a batch, since receipts were waiting");
    let (code, ok) = send(&w.app, "POST", "/api/v1/admin/access/receipts/verify", ADMIN, None).await;
    assert_eq!(code, StatusCode::OK, "{ok}");
    assert_eq!(ok["ok"], true, "{ok}");

    // The batch is in the ledger.
    let (_, ledger) = send(&w.app, "GET", "/api/v1/governance/ledger", ADMIN, None).await;
    assert!(ledger.to_string().contains("AccessReceiptsSealed"), "{ledger}");

    let id = receipts(&w, &format!("actor={OWNER}&outcome=deny")).await[0]["id"].as_i64().unwrap();
    w.db.run(move |c| c.execute("UPDATE access_receipts SET actor = 'someone.else@state.edu' WHERE id = $1", &[&id]).unwrap());
    let (_, bad) = send(&w.app, "POST", "/api/v1/admin/access/receipts/verify", ADMIN, None).await;
    assert_eq!(bad["ok"], false, "{bad}");
    assert_eq!(bad["failed_batch"], batch, "{bad}");
}

// 4. Replaying a receipt with its counters reproduces the outcome.
#[tokio::test]
async fn replaying_a_receipt_reproduces_its_outcome_while_the_counters_hold() {
    let w = world();
    let s = slug("rcpt-replay");
    make_app(&w, &s, "Restricted", vec![field("name", false)]).await;
    send(&w.app, "PUT", &format!("/api/v1/workspaces/{WORKSPACE}/guards"), ADMIN, Some(json!({ "rules": [{ "template": "deny_above_sensitivity", "sensitivity": "Internal" }], "reason": "Nothing above Internal" }))).await;
    send(&w.app, "GET", &format!("/api/v1/apps/{s}/schema"), OWNER, None).await;
    let id = receipts(&w, &format!("actor={OWNER}&outcome=deny")).await[0]["id"].as_i64().unwrap();

    let (code, replay) = send(&w.app, "POST", &format!("/api/v1/admin/access/receipts/{id}/replay"), ADMIN, None).await;
    assert_eq!(code, StatusCode::OK, "{replay}");
    assert_eq!(replay["outcome"], "deny", "{replay}");
    assert_eq!(replay["matches"], true, "{replay}");
    assert_eq!(replay["counters_match"], true, "{replay}");

    // The guard is lifted. The old denial no longer reproduces, and the replay says the state has moved.
    let (c, body) = send(&w.app, "PUT", &format!("/api/v1/workspaces/{WORKSPACE}/guards"), ADMIN, Some(json!({ "rules": [], "reason": "Lifted" }))).await;
    assert_eq!(c, StatusCode::OK, "{body}");
    let (_, later) = send(&w.app, "POST", &format!("/api/v1/admin/access/receipts/{id}/replay"), ADMIN, None).await;
    assert_eq!(later["counters_match"], false, "{later}");
    assert_eq!(later["outcome"], "allow", "{later}");
    assert_eq!(later["matches"], false, "{later}");
}

// A privileged action leaves a receipt, and so does everything done under view-as.
#[tokio::test]
async fn a_privileged_action_and_a_view_as_request_leave_receipts() {
    let w = world();
    let (s, body) = send(&w.app, "POST", "/api/v1/admin/access/grants", ADMIN, Some(json!({ "holder_kind": "person", "holder_id": "x@state.edu", "role": "viewer", "anchor_kind": "container", "anchor_id": format!("workspace:{WORKSPACE}"), "source": "request", "reason": "Test grant" }))).await;
    assert_eq!(s, StatusCode::ACCEPTED, "{body}");
    let mine = receipts(&w, &format!("actor={ADMIN}&outcome=allow")).await;
    assert!(mine.iter().any(|r| r["action"].as_str().unwrap().contains("admin") && r["action"].as_str().unwrap().contains("grants")), "{mine:?}");
    // Receipts are for a Platform Admin or compliance.
    let (s, _) = send(&w.app, "GET", "/api/v1/admin/access/receipts", OWNER, None).await;
    assert_eq!(s, StatusCode::FORBIDDEN);
}

// The disclosure log stays, and each disclosure also writes its receipt.
#[tokio::test]
async fn a_disclosure_also_writes_its_receipt() {
    use scaffoldry_server::service::disclosure::{log_disclosure, Release};
    let w = world();
    let s = slug("rcpt-disc");
    make_app(&w, &s, "Internal", vec![field("name", false), field("ssn", true)]).await;
    let before = receipts(&w, "actor=agent.reader@state.edu&limit=200").await.len();
    let logged = log_disclosure(&w.state, Release { kind: "agent_read", actor: "agent.reader@state.edu", app_slug: &s, table_id: Some("items"), record_ids: vec!["r1".into(), "r2".into()], fields: vec!["ssn".into()], recipient: None, note: "a bulk read" }).expect("a release");
    assert!(logged.is_some(), "the log still gets its entry");
    let after = receipts(&w, "actor=agent.reader@state.edu&limit=200").await;
    assert_eq!(after.len(), before + 1, "{after:?}");
    assert_eq!(after[0]["action"], "disclosure:agent_read");
    assert_eq!(after[0]["object_id"], format!("{s}/items").as_str());
    assert!(!after[0].to_string().contains("r1"), "a receipt names no record");
}
