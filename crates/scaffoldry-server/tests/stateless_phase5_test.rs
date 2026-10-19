//! Stateless server phase 5: process instances and the ledger are never held in memory. Two instances append to one
//! chain, two instances cannot both decide one step, ledger reads page in SQL, and the chain check reads in batches.

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
use std::time::Instant;
use tower::ServiceExt;
use uuid::Uuid;

const WORKSPACE: &str = "ws-physics-optics";
const OWNER: &str = "einstein@physics.state.edu";
const EDITOR: &str = "editor.p5@physics.state.edu";
const ADMIN: &str = "jordan.lee@state.edu";

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

async fn send(app: &Router, method: &str, uri: &str, who: &str, body: Option<Value>) -> (StatusCode, Value) {
    send_with(app, method, uri, &issue_test_token_and_user(who), body).await
}

fn entry(n: usize) -> scaffoldry_server::state::RecordDecisionInput<'static> {
    // A leaked payload keeps the input `'static` for the threads below.
    let payload: &'static Value = Box::leak(Box::new(json!({ "n": n })));
    scaffoldry_server::state::RecordDecisionInput {
        principal: "load@state.edu".to_string(),
        organization_code: "INST".to_string(),
        app_slug: None,
        decision_type: scaffoldry_core::ledger::DecisionType::PolicyRevision,
        oscal_control_id: "AU-02".to_string(),
        rationale: format!("Load entry {n}"),
        payload,
    }
}

fn append_many(state: SharedState, from: usize, count: usize) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        for n in from..from + count {
            state.append_ledger_entry(entry(n)).expect("append");
        }
    })
}

fn ledger_sequences(db: &TestDb) -> Vec<i64> {
    db.run(|c| c.query("SELECT sequence FROM governance_ledger ORDER BY sequence", &[]).unwrap().iter().map(|r| r.get(0)).collect())
}

// 1. Two instances append to the ledger at once, 200 entries each.
#[test]
fn two_instances_appending_at_once_leave_one_unbroken_chain() {
    let db = TestDb::new();
    let (a, b) = (db.state(), db.state());
    let before = ledger_sequences(&db).len();
    let (x, y) = (append_many(a.clone(), 0, 200), append_many(b.clone(), 1000, 200));
    x.join().unwrap();
    y.join().unwrap();

    let after = ledger_sequences(&db);
    assert_eq!(after.len(), before + 400, "the chain has 400 more entries");
    assert!(after.iter().enumerate().all(|(i, s)| *s == i as i64), "the sequence has no gap and no repeat");
    assert!(a.verify_ledger().unwrap(), "the chain check passes through A");
    assert!(b.verify_ledger().unwrap(), "and through B");
}

// 4. An instance started while another is appending verifies the chain and starts.
#[test]
fn an_instance_started_during_appends_verifies_and_starts() {
    let db = TestDb::new();
    let writer = append_many(db.state(), 0, 300);
    let mut started = 0;
    while !writer.is_finished() && started < 5 {
        let late = db.state();
        assert!(late.verify_ledger().unwrap(), "a server that starts mid-append sees an intact chain");
        started += 1;
    }
    writer.join().unwrap();
    assert!(started > 0, "at least one server started while the other was appending");
    assert!(db.state().verify_ledger().unwrap());
}

// 3. Ledger reads page in SQL and the chain check reads in batches.
#[tokio::test]
async fn a_large_ledger_pages_in_sql_and_the_chain_check_holds_one_batch() {
    const TOTAL: usize = 100_000;
    let db = TestDb::new();
    let state = db.state();
    let started = Instant::now();
    db.run(move |c| {
        let mut tx = c.transaction().unwrap();
        let head = tx.query_one("SELECT sequence, entry_hash FROM governance_ledger ORDER BY sequence DESC LIMIT 1", &[]).unwrap();
        let (mut seq, mut prev): (i64, String) = (head.get::<_, i64>(0) + 1, head.get(1));
        let insert = tx
            .prepare(
                "INSERT INTO governance_ledger (sequence, entry_hash, previous_hash, timestamp_iso, principal, organization_code, app_slug, \
                 decision_type, oscal_control_id, rationale, payload_hash, payload) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)",
            )
            .unwrap();
        for n in 0..TOTAL {
            let payload = json!({ "n": n });
            let e = scaffoldry_core::ledger::LedgerEntry::new(scaffoldry_core::NewLedgerEntryParams {
                sequence: seq as u64,
                timestamp_iso: "2026-01-01T00:00:00+00:00".to_string(),
                previous_hash: prev.clone(),
                principal: "bulk@state.edu".to_string(),
                organization_code: "INST".to_string(),
                app_slug: None,
                decision_type: scaffoldry_core::ledger::DecisionType::PolicyRevision,
                oscal_control_id: "AU-02".to_string(),
                rationale: format!("Bulk {n}"),
                payload: &payload,
            });
            tx.execute(
                &insert,
                &[&seq, &e.entry_hash, &e.previous_hash, &e.timestamp_iso, &e.principal, &e.organization_code, &e.app_slug, &"PolicyRevision", &e.oscal_control_id, &e.rationale, &e.payload_hash, &payload],
            )
            .unwrap();
            prev = e.entry_hash;
            seq += 1;
        }
        tx.commit().unwrap();
    });
    println!("built {TOTAL} entries in {:?}", started.elapsed());

    let app = build_app_with_state(state.clone()).expect("router");
    let token = issue_test_token_and_user(ADMIN);
    let total: i64 = db.run(|c| c.query_one("SELECT count(*) FROM governance_ledger", &[]).unwrap().get(0));
    // One warm-up, so a connection and a plan are not in the timing.
    let _ = send_with(&app, "GET", "/api/v1/governance/ledger?limit=10", &token, None).await;
    let mut times = Vec::new();
    for _ in 0..5 {
        let t = Instant::now();
        let _ = send_with(&app, "GET", "/api/v1/governance/ledger?limit=100", &token, None).await;
        times.push(t.elapsed());
    }
    println!("five reads: {times:?}");
    let t = Instant::now();
    let (s, page) = send_with(&app, "GET", "/api/v1/governance/ledger?limit=100", &token, None).await;
    let first_page = t.elapsed();
    println!("first page of {total}: {first_page:?}");
    assert_eq!(s, StatusCode::OK, "{page}");
    assert_eq!(page["total_entries"], total, "total_entries counts the table");
    let entries = page["entries"].as_array().unwrap();
    assert_eq!(entries.len(), 100, "one page, not the chain");
    assert_eq!(entries[0]["sequence"], total - 1, "newest first");
    assert_eq!(entries[99]["sequence"], total - 100);
    assert!(first_page.as_millis() < 50, "the first page returns in under 50 ms: {first_page:?}");

    // The cursor continues where the page stopped.
    let cursor = page["next_cursor"].as_i64().expect("a cursor");
    let (_, next) = send_with(&app, "GET", &format!("/api/v1/governance/ledger?limit=100&before={cursor}"), &token, None).await;
    assert_eq!(next["entries"][0]["sequence"], total - 101, "the next page starts after the last of the first");

    let t = Instant::now();
    let report = state.repository.verify_ledger_chain().expect("the chain is intact");
    println!("chain check of {total}: {:?}, largest batch {}", t.elapsed(), report.largest_batch);
    assert_eq!(report.entries as i64, total, "every entry was checked");
    assert!(report.largest_batch <= 1000, "never more than 1,000 entries at a time: {}", report.largest_batch);
}

// 2. Two instances decide the same waiting step at once. One is 200 and one is 409. The ledger has one decision.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn two_instances_deciding_one_step_at_once_decide_it_once() {
    let db = TestDb::new();
    let a = {
        let state = db.state();
        state.seed_demo().expect("seed");
        build_app_with_state(state).expect("router")
    };
    let slug = format!("p5-{}", &Uuid::new_v4().to_string()[..8]);
    let manifest = json!({
        "slug": slug, "title": "Process", "description": "d", "organization_code": "PHYS", "department": "physics", "views": [],
        "tables": [{ "id": "apps", "name": "Applications", "slug": "apps", "fields": [
            { "name": "name", "label": "Name", "field_type": "Text", "required": false, "ferpa_sensitive": false }
        ]}]
    });
    let (s, body) = send(&a, "POST", &format!("/api/v1/workspaces/{WORKSPACE}/apps"), OWNER, Some(manifest)).await;
    assert_eq!(s, StatusCode::CREATED, "{body}");
    let (s, body) = send(&a, "POST", &format!("/api/v1/workspaces/{WORKSPACE}/collaborators"), OWNER, Some(json!({ "eppn": EDITOR, "role": "editor" }))).await;
    assert_eq!(s, StatusCode::CREATED, "{body}");
    let rule_id = format!("r-{}", &Uuid::new_v4().to_string()[..8]);
    let rule = json!({
        "id": rule_id, "name": "Review", "enabled": true, "trigger": "RecordCreated",
        "steps": [{ "id": "decide", "when": [], "kind": { "UserTask": {
            "role": "owner", "prompt": "Decide",
            "approve": [{ "UpdateRecordStatus": { "new_status": "Admitted" } }],
            "reject": [{ "UpdateRecordStatus": { "new_status": "Denied" } }],
            "approver": { "Role": { "name": "owner" } }
        }}}]
    });
    let (s, body) = send(&a, "POST", &format!("/api/v1/apps/{slug}/automations"), OWNER, Some(rule)).await;
    assert_eq!(s, StatusCode::CREATED, "{body}");
    let b = db.second_app();
    let (s, created) = send(&b, "POST", &format!("/api/v1/apps/{slug}/records"), EDITOR, Some(json!({ "data": { "_table_id": "apps", "name": "A" } }))).await;
    assert_eq!(s, StatusCode::CREATED, "{created}");
    let instance = format!("{rule_id}:{}", created["id"].as_str().unwrap());

    let token = issue_test_token_and_user(OWNER);
    let uri = format!("/api/v1/apps/{slug}/processes/{}/decide", instance.replace(':', "%3A"));
    let decide = |app: Router| {
        let (token, uri) = (token.clone(), uri.clone());
        tokio::spawn(async move { send_with(&app, "POST", &uri, &token, Some(json!({ "decision": "approve" }))).await })
    };
    let (x, y) = tokio::join!(decide(a.clone()), decide(b.clone()));
    let (x, y) = (x.unwrap(), y.unwrap());
    let mut codes = vec![x.0.as_u16(), y.0.as_u16()];
    codes.sort();
    assert_eq!(codes, [200, 409], "one decision wins and the other is refused: {x:?} {y:?}");
    let loser = if x.0 == StatusCode::CONFLICT { &x.1 } else { &y.1 };
    assert_eq!(loser["code"], "version_conflict", "{loser}");

    let id = instance.clone();
    let decisions: i64 = db.run(move |c| {
        c.query_one("SELECT count(*) FROM governance_ledger WHERE decision_type = 'WorkflowRuleApproved' AND payload->>'instance_id' = $1", &[&id]).unwrap().get(0)
    });
    assert_eq!(decisions, 1, "the ledger has one decision for the instance");
}
