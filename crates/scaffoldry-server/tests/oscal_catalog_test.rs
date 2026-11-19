//! OSCAL catalog phase 2: titles, validation, and coverage.

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::Router;
use common::TestDb;
use http_body_util::BodyExt;
use scaffoldry_core::ledger::DecisionType;
use scaffoldry_server::service::identity::issue_test_token_and_user;
use scaffoldry_server::state::{RecordDecisionInput, SharedState};
use serde_json::{json, Value};
use tower::ServiceExt;

const ADMIN: &str = "jordan.lee@state.edu";
const FACULTY: &str = "prof.curie@science.state.edu";

struct World {
    _db: TestDb,
    state: SharedState,
    app: Router,
    admin: String,
    faculty: String,
}

fn world() -> World {
    let db = TestDb::new();
    let state = db.state();
    let app = scaffoldry_server::build_app_with_state(state.clone()).expect("router");
    let admin = issue_test_token_and_user(ADMIN);
    let faculty = issue_test_token_and_user(FACULTY);
    World { _db: db, state, app, admin, faculty }
}

async fn send(app: &Router, method: &str, uri: &str, token: &str, body: Option<Value>) -> (StatusCode, Value) {
    let b = Request::builder().method(method).uri(format!("/api/v1{uri}")).header("authorization", format!("Bearer {token}"));
    let req = match body {
        Some(v) => b.header("content-type", "application/json").body(Body::from(v.to_string())).unwrap(),
        None => b.body(Body::empty()).unwrap(),
    };
    let resp = app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
}

impl World {
    async fn put(&self, control: &str, status: &str) -> (StatusCode, Value) {
        send(&self.app, "PUT", &format!("/admin/oscal/requirements/{control}"), &self.admin, Some(json!({ "statement": "Covered.", "status": status, "reason": "Recorded." }))).await
    }
}

// 1. A requirement for zz-99 is 400. AC-02 is accepted and stored as ac-2.
#[tokio::test]
async fn an_unknown_control_is_refused_and_a_known_one_is_normalized() {
    let w = world();
    let (s, b) = w.put("zz-99", "planned").await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{b}");
    assert!(b.to_string().contains("unknown control"), "{b}");
    let (s, b) = w.put("AC-02", "planned").await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_eq!(b["control_id"], "ac-2");
    assert_eq!(b["control_title"], "Account Management");
}

// 2. A requirement row and a ledger row each carry the right control_title for ac-2.
#[tokio::test]
async fn rows_and_ledger_entries_carry_the_control_title() {
    let w = world();
    w.put("ac-2", "implemented").await;
    let (_, rows) = send(&w.app, "GET", "/admin/oscal/requirements", &w.admin, None).await;
    let row = rows["rows"].as_array().unwrap().iter().find(|r| r["control_id"] == "ac-2").unwrap().clone();
    assert_eq!(row["control_title"], "Account Management");
    assert_eq!(row["catalog"], "sp800-53r5");
    assert_eq!(row["family_title"], "Access Control");
    assert_eq!(row["baselines"], json!(["low", "moderate", "high"]));
    w.state
        .append_ledger_entry(RecordDecisionInput {
            principal: ADMIN.to_string(),
            organization_code: "DIV-ADMIN-CONSOLE".to_string(),
            app_slug: None,
            decision_type: DecisionType::PolicyRevision,
            oscal_control_id: "AC-02".to_string(),
            rationale: "Evidence".to_string(),
            payload: &json!({}),
        })
        .unwrap();
    let (s, l) = send(&w.app, "GET", "/governance/ledger", &w.admin, None).await;
    assert_eq!(s, StatusCode::OK, "{l}");
    let entry = l["entries"].as_array().unwrap().iter().find(|e| e["oscal_control_id"] == "AC-02" && e["rationale"] == "Evidence").unwrap();
    assert_eq!(entry["control_title"], "Account Management");
}

fn counts_sum(c: &Value) -> i64 {
    ["implemented", "partial", "planned", "alternative", "not-applicable", "none"].iter().map(|k| c[*k].as_i64().unwrap()).sum()
}

// 3. Coverage for Moderate counts three requirements and leaves the rest none. The counts sum to the baseline size.
#[tokio::test]
async fn coverage_counts_each_status_and_the_rest_are_none() {
    let w = world();
    let size = scaffoldry_core::catalog::baseline("moderate").unwrap().len() as i64;
    let (s, before) = send(&w.app, "GET", "/admin/oscal/coverage?baseline=moderate", &w.admin, None).await;
    assert_eq!(s, StatusCode::OK, "{before}");
    assert_eq!(before["total"], size);
    assert_eq!(counts_sum(&before["counts"]), size);
    for (c, st) in [("ac-2", "implemented"), ("ac-2.1", "partial"), ("sc-7", "planned")] {
        assert_eq!(w.put(c, st).await.0, StatusCode::OK);
    }
    let (_, after) = send(&w.app, "GET", "/admin/oscal/coverage?baseline=moderate", &w.admin, None).await;
    let n = |v: &Value, k: &str| v["counts"][k].as_i64().unwrap();
    assert_eq!(n(&after, "implemented"), n(&before, "implemented") + 1, "{after}");
    assert_eq!(n(&after, "partial"), n(&before, "partial") + 1);
    assert_eq!(n(&after, "planned"), n(&before, "planned") + 1);
    assert_eq!(n(&after, "none"), n(&before, "none") - 3);
    assert_eq!(counts_sum(&after["counts"]), size);
    let fams = after["families"].as_array().unwrap();
    assert!(fams.iter().all(|f| f["family_title"].as_str().is_some()));
    assert_eq!(fams.iter().map(|f| f["total"].as_i64().unwrap()).sum::<i64>(), size);
    assert_eq!(fams.iter().map(|f| counts_sum(&f["counts"])).sum::<i64>(), size);
    assert!(after["with_evidence"].is_i64());
    // A baseline belongs to 800-53 only.
    let (s, _) = send(&w.app, "GET", "/admin/oscal/coverage?baseline=moderate&catalog=sp800-171r3", &w.admin, None).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    let (s, _) = send(&w.app, "GET", "/admin/oscal/coverage?baseline=nope", &w.admin, None).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
}

// 4. The gaps route pages through exactly the none controls.
#[tokio::test]
async fn gaps_page_through_exactly_the_none_controls() {
    let w = world();
    w.put("ac-2", "implemented").await;
    let (_, cov) = send(&w.app, "GET", "/admin/oscal/coverage?baseline=low", &w.admin, None).await;
    let none = cov["counts"]["none"].as_i64().unwrap();
    let mut seen: Vec<String> = Vec::new();
    let mut cursor = String::new();
    let mut pages = 0;
    loop {
        let (s, g) = send(&w.app, "GET", &format!("/admin/oscal/coverage/gaps?baseline=low&limit=7&cursor={cursor}"), &w.admin, None).await;
        assert_eq!(s, StatusCode::OK, "{g}");
        for r in g["rows"].as_array().unwrap() {
            assert!(r["control_title"].as_str().is_some_and(|t| !t.is_empty()), "{r}");
            seen.push(r["control_id"].as_str().unwrap().to_string());
        }
        pages += 1;
        match g["next_cursor"].as_str() {
            Some(c) => cursor = c.to_string(),
            None => break,
        }
    }
    assert!(pages > 1);
    assert_eq!(seen.len() as i64, none);
    assert!(!seen.contains(&"ac-2".to_string()));
    let mut unique = seen.clone();
    unique.sort();
    unique.dedup();
    assert_eq!(unique.len(), seen.len(), "no control twice");
}

// 5. The export names each catalog's canonical URL and holds only controls that are in a loaded catalog.
#[tokio::test]
async fn the_export_uses_provenance_urls_and_only_known_controls() {
    let w = world();
    w.put("ac-2", "implemented").await;
    w.put("SP_800_171_03.01.01", "planned").await;
    let (s, doc) = send(&w.app, "GET", "/admin/oscal/component-definition", &w.admin, None).await;
    assert_eq!(s, StatusCode::OK, "{doc}");
    let impls = doc["component-definition"]["components"][0]["control-implementations"].as_array().unwrap();
    let sources: Vec<&str> = impls.iter().map(|i| i["source"].as_str().unwrap()).collect();
    assert!(sources.contains(&scaffoldry_core::catalog::source_url("sp800-53r5").unwrap()), "{sources:?}");
    assert!(sources.contains(&scaffoldry_core::catalog::source_url("sp800-171r3").unwrap()), "{sources:?}");
    for i in impls {
        for r in i["implemented-requirements"].as_array().unwrap() {
            assert!(scaffoldry_core::catalog::control(r["control-id"].as_str().unwrap()).is_some(), "{r}");
        }
    }
}

// 6. GET /governance/oscal is gone.
#[tokio::test]
async fn the_hand_written_catalog_route_is_gone() {
    let w = world();
    let (s, _) = send(&w.app, "GET", "/governance/oscal", &w.admin, None).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
}

// 7. Search with baseline=moderate returns only Moderate controls, and says whether each has a requirement.
#[tokio::test]
async fn search_filters_by_baseline_and_reports_requirements() {
    let w = world();
    w.put("ac-2", "partial").await;
    let moderate: std::collections::HashSet<&String> = scaffoldry_core::catalog::baseline("moderate").unwrap().iter().collect();
    let (s, b) = send(&w.app, "GET", "/admin/oscal/catalog?q=account&baseline=moderate&limit=200", &w.admin, None).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    let rows = b["rows"].as_array().unwrap();
    assert!(!rows.is_empty());
    for r in rows {
        assert!(moderate.contains(&r["control_id"].as_str().unwrap().to_string()), "{r}");
        assert!(r["control_title"].as_str().is_some());
    }
    let ac2 = rows.iter().find(|r| r["control_id"] == "ac-2").expect("ac-2 matches account");
    assert_eq!(ac2["has_requirement"], true);
    assert_eq!(ac2["status"], "partial");
    // Paged, at most 200.
    let (_, page) = send(&w.app, "GET", "/admin/oscal/catalog?limit=5000", &w.admin, None).await;
    assert_eq!(page["rows"].as_array().unwrap().len(), 200);
    assert!(page["next_cursor"].as_str().is_some());
    // Only Platform Admin or compliance.
    assert_eq!(send(&w.app, "GET", "/admin/oscal/catalog", &w.faculty, None).await.0, StatusCode::FORBIDDEN);
    assert_eq!(send(&w.app, "GET", "/admin/oscal/coverage", &w.faculty, None).await.0, StatusCode::FORBIDDEN);
    assert_eq!(send(&w.app, "GET", "/admin/oscal/coverage/gaps", &w.faculty, None).await.0, StatusCode::FORBIDDEN);
}
