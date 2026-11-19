//! Admin console phase 5: the running policy is a stored, versioned, testable thing, and the OSCAL mapping is data.

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::Router;
use common::TestDb;
use http_body_util::BodyExt;
use scaffoldry_core::ledger::DecisionType;
use scaffoldry_server::service::identity::{create_setup_token, issue_test_token_and_user};
use scaffoldry_server::state::{RecordDecisionInput, ServerState, SharedState};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tower::ServiceExt;

const ADMIN: &str = "jordan.lee@state.edu";
const ADMIN2: &str = "second.admin@state.edu";
const OFFICER: &str = "hold.officer@state.edu";
const FACULTY: &str = "prof.curie@science.state.edu";

struct World {
    db: TestDb,
    state: SharedState,
    app: Router,
    admin: String,
    admin2: String,
    officer: String,
    faculty: String,
}

fn appoint(db: &TestDb, eppn: &'static str, affiliation: &'static str) {
    db.run(move |c| {
        c.execute(
            "INSERT INTO roles (id, person_id, organization_id, role_title, scoped_affiliation, is_primary, source) \
             SELECT gen_random_uuid(), p.id, o.id, 'Appointed', $2, true, 'api' \
             FROM persons p, organizations o WHERE LOWER(p.eppn) = LOWER($1) AND o.parent_id IS NULL ON CONFLICT DO NOTHING",
            &[&eppn, &affiliation],
        )
        .unwrap();
    });
}

fn world() -> World {
    let db = TestDb::new();
    let state = db.state();
    let app = scaffoldry_server::build_app_with_state(state.clone()).expect("router");
    let admin = issue_test_token_and_user(ADMIN);
    let admin2 = issue_test_token_and_user(ADMIN2);
    let officer = issue_test_token_and_user(OFFICER);
    let faculty = issue_test_token_and_user(FACULTY);
    appoint(&db, ADMIN2, "platform_admin");
    appoint(&db, OFFICER, "compliance");
    World { db, state, app, admin, admin2, officer, faculty }
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
    async fn active(&self) -> Value {
        let (s, v) = send(&self.app, "GET", "/admin/policy/active", &self.admin, None).await;
        assert_eq!(s, StatusCode::OK, "{v}");
        v
    }

    async fn draft(&self, who: &str, source: &str) -> (StatusCode, Value) {
        send(&self.app, "POST", "/admin/policy/versions", who, Some(json!({ "source": source, "note": "A change" }))).await
    }

    async fn activate(&self, who: &str, id: &str) -> (StatusCode, Value) {
        send(&self.app, "POST", &format!("/admin/policy/versions/{id}/activate"), who, Some(json!({ "reason": "The change was reviewed." }))).await
    }
}

fn sha(text: &str) -> String {
    Sha256::digest(text.as_bytes()).iter().map(|b| format!("{b:02x}")).collect()
}

/// The default policy with one more rule, so a draft differs from it.
fn with_rule(base: &str, rule: &str) -> String {
    format!("{base}\n{rule}\n")
}

const STAFF_CANNOT_EXPORT: &str = "@id(\"no-staff-export\") @oscal(\"ac-3\")\nforbid (principal, action == Action::\"export\", resource) when { principal.scoped_affiliation == \"staff\" };";

// 1. A fresh database has one Active version and the default cases. All cases pass against it.
#[tokio::test]
async fn a_fresh_database_has_one_active_version_and_every_default_case_passes() {
    let w = world();
    let active = w.active().await;
    assert_eq!(active["version"]["version"], 1);
    assert_eq!(active["version"]["status"], "Active");
    assert_eq!(active["policies"].as_array().unwrap().len(), 25);
    assert_eq!(active["source"], w.state.policy().source());
    let (_, list) = send(&w.app, "GET", "/admin/policy/versions", &w.admin, None).await;
    assert_eq!(list["rows"].as_array().unwrap().len(), 1);
    let id = active["version"]["id"].as_str().unwrap();
    let (s, run) = send(&w.app, "POST", &format!("/admin/policy/versions/{id}/run-cases"), &w.admin, None).await;
    assert_eq!(s, StatusCode::OK, "{run}");
    let results = run["results"].as_array().unwrap();
    assert_eq!(results.len(), 16);
    assert!(results.iter().all(|r| r["pass"] == true), "{run}");
    assert_eq!(run["failed"], 0);
}

// 2. A draft with a syntax error is 400 and reports a line.
#[tokio::test]
async fn a_draft_with_a_syntax_error_is_refused_with_its_line() {
    let w = world();
    let (s, b) = w.draft(&w.admin, "permit (principal, action, resource);\n\nforbid (principal action, resource);\n").await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{b}");
    let text = b["fields"].to_string();
    assert!(text.contains("line 3"), "{b}");
    // The same check without storing anything.
    let (s, v) = send(&w.app, "POST", "/admin/policy/validate", &w.admin, Some(json!({ "source": "permit (principal, action, resource);\n\nforbid (principal action, resource);\n" }))).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["ok"], false);
    assert_eq!(v["problems"][0]["line"], 3);
    let (_, v) = send(&w.app, "POST", "/admin/policy/validate", &w.admin, Some(json!({ "source": w.state.policy().source() }))).await;
    assert_eq!(v["ok"], true);
    // Nothing was stored.
    let (_, list) = send(&w.app, "GET", "/admin/policy/versions", &w.admin, None).await;
    assert_eq!(list["rows"].as_array().unwrap().len(), 1);
}

// 2b. A draft that parses and does not type-check is 400 as well.
#[tokio::test]
async fn a_draft_that_does_not_type_check_is_refused() {
    let w = world();
    let bad = with_rule(w.state.policy().source(), "forbid (principal, action, resource) when { principal.no_such_attribute == 1 };");
    let (s, b) = w.draft(&w.admin, &bad).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{b}");
    assert!(b["fields"].to_string().contains("line"), "{b}");
}

// 3. A draft that breaks a case is refused with 409 and names the case. After the case is replaced by one that expects the new behavior, a
// new draft activates and the next request reflects it with no restart.
#[tokio::test]
async fn a_draft_that_breaks_a_case_cannot_be_activated_until_the_case_says_otherwise() {
    let w = world();
    let source = with_rule(w.state.policy().source(), STAFF_CANNOT_EXPORT);
    let (s, d) = w.draft(&w.admin, &source).await;
    assert_eq!(s, StatusCode::CREATED, "{d}");
    let id = d["id"].as_str().unwrap().to_string();
    assert_eq!(d["status"], "Draft");
    let (_, run) = send(&w.app, "POST", &format!("/admin/policy/versions/{id}/run-cases"), &w.admin, None).await;
    assert_eq!(run["failed"], 1, "{run}");
    let (s, b) = w.activate(&w.admin2, &id).await;
    assert_eq!(s, StatusCode::CONFLICT, "{b}");
    assert!(b["error"].as_str().unwrap().contains("Staff can export protected student records"), "{b}");
    assert_eq!(w.active().await["version"]["version"], 1, "the running policy did not change");

    // The institution decides staff may no longer export. The old case goes and a case for the new rule is added.
    let (_, cases) = send(&w.app, "GET", "/admin/policy/cases", &w.admin, None).await;
    let old = cases["rows"].as_array().unwrap().iter().find(|c| c["name"] == "Staff can export protected student records").unwrap().clone();
    let (s, b) = send(&w.app, "DELETE", &format!("/admin/policy/cases/{}", old["id"].as_str().unwrap()), &w.admin, Some(json!({ "reason": "Staff may no longer export." }))).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    let (s, b) = send(&w.app, "POST", "/admin/policy/cases", &w.admin, Some(json!({
        "name": "Staff cannot export protected student records", "principal": old["principal"], "action": "export", "resource": old["resource"], "expected": "Deny",
        "reason": "Staff may no longer export."
    })))
    .await;
    assert_eq!(s, StatusCode::CREATED, "{b}");
    let (s, b) = w.activate(&w.admin2, &id).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    let active = w.active().await;
    assert_eq!(active["version"]["version"], 2);
    assert_eq!(active["source"], source);
    assert_eq!(w.state.policy().source(), source, "the next request reflects it with no restart");
    let (_, list) = send(&w.app, "GET", "/admin/policy/versions", &w.admin, None).await;
    let statuses: Vec<&str> = list["rows"].as_array().unwrap().iter().map(|r| r["status"].as_str().unwrap()).collect();
    assert_eq!(statuses, ["Active", "Retired"]);
    // Another server on the same database reads the same policy.
    assert_eq!(w.db.state().policy().source(), source);

    // Rollback is a clone, then an activation.
    let v1 = list["rows"][1]["id"].as_str().unwrap();
    let (s, c) = send(&w.app, "POST", &format!("/admin/policy/versions/{v1}/clone"), &w.admin, Some(json!({}))).await;
    assert_eq!(s, StatusCode::CREATED, "{c}");
    assert_eq!(c["status"], "Draft");
    assert_eq!(c["version"], 3);
}

// 4. Activation writes one PolicyRevision entry that holds the SHA-256 of the source.
#[tokio::test]
async fn activation_writes_one_ledger_entry_with_the_hash_of_the_source() {
    let w = world();
    let source = with_rule(w.state.policy().source(), "@id(\"no-affiliate-read\") @oscal(\"ac-3\")\nforbid (principal, action == Action::\"read\", resource) when { principal.scoped_affiliation == \"affiliate\" };");
    let (_, d) = w.draft(&w.admin, &source).await;
    let before: i64 = w.db.run(|c| c.query_one("SELECT COUNT(*) FROM governance_ledger WHERE decision_type = 'PolicyRevision'", &[]).unwrap().get(0));
    let (s, b) = w.activate(&w.admin2, d["id"].as_str().unwrap()).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    let (count, payload): (i64, Value) = w.db.run(|c| {
        let n = c.query_one("SELECT COUNT(*) FROM governance_ledger WHERE decision_type = 'PolicyRevision'", &[]).unwrap().get(0);
        let p = c.query_one("SELECT payload FROM governance_ledger WHERE decision_type = 'PolicyRevision' ORDER BY sequence DESC LIMIT 1", &[]).unwrap().get(0);
        (n, p)
    });
    assert_eq!(count, before + 1);
    assert_eq!(payload["version"], 2);
    assert_eq!(payload["previous_version"], 1);
    assert_eq!(payload["source_sha256"], sha(&source));
}

// 5. The creator cannot activate their own draft, even as the only Platform Admin. The setup identity can.
#[tokio::test]
async fn the_author_cannot_activate_their_own_draft_and_the_setup_identity_can() {
    let w = world();
    let source = with_rule(w.state.policy().source(), "@id(\"no-affiliate-read\") @oscal(\"ac-3\")\nforbid (principal, action == Action::\"read\", resource) when { principal.scoped_affiliation == \"affiliate\" };");
    let (_, d) = w.draft(&w.admin, &source).await;
    let id = d["id"].as_str().unwrap().to_string();
    let (s, b) = w.activate(&w.admin, &id).await;
    assert_eq!(s, StatusCode::FORBIDDEN, "{b}");
    assert_eq!(w.active().await["version"]["version"], 1);
    // With one Platform Admin the author is still refused. The setup identity exists only while no Platform Admin is appointed.
    w.db.run(|c| c.batch_execute("DELETE FROM roles WHERE scoped_affiliation = 'platform_admin'").unwrap());
    let token = create_setup_token(&w.state).expect("a setup token");
    let (s, b) = w.activate(&token, &id).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_eq!(w.state.policy().source(), source);
}

// 6. A database whose Active source does not parse makes the server refuse to start.
#[test]
fn a_stored_policy_that_does_not_parse_stops_the_server() {
    let db = TestDb::new();
    drop(db.state());
    db.run(|c| c.execute("UPDATE policy_versions SET source = 'permit (principal' WHERE status = 'Active'", &[]).unwrap());
    let err = ServerState::connect(db.url()).err().expect("the server must not start");
    assert!(err.to_string().contains("policy"), "{err}");
}

// 8. The export has no requirement whose only source is a ledger entry. A control with three ledger entries shows evidence count 3.
// 9. Control ids in the export are normalized.
#[tokio::test]
async fn evidence_is_counted_and_a_ledger_entry_is_not_a_requirement() {
    let w = world();
    let entry = |control: &str| {
        w.state
            .append_ledger_entry(RecordDecisionInput {
                principal: ADMIN.to_string(),
                organization_code: "DIV-ADMIN-CONSOLE".to_string(),
                app_slug: None,
                decision_type: DecisionType::PolicyRevision,
                oscal_control_id: control.to_string(),
                rationale: "Evidence".to_string(),
                payload: &json!({}),
            })
            .unwrap()
    };
    let (s, b) = send(&w.app, "PUT", "/admin/oscal/requirements/SA-09", &w.admin, Some(json!({ "statement": "Outside services are reviewed.", "status": "planned", "reason": "Adding the requirement." }))).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    for spelled in ["SA-09", "SA-9", "sa-9"] {
        entry(spelled);
    }
    entry("PE-03");
    entry("PE-3");
    let (s, rows) = send(&w.app, "GET", "/admin/oscal/requirements", &w.admin, None).await;
    assert_eq!(s, StatusCode::OK, "{rows}");
    let rows = rows["rows"].as_array().unwrap();
    let sa = rows.iter().find(|r| r["control_id"] == "sa-9").expect("the stored requirement");
    assert_eq!(sa["evidence_count"], 3, "{sa}");
    assert_eq!(sa["status"], "planned");
    assert!(sa["latest_evidence"].as_str().is_some());
    assert!(!rows.iter().any(|r| r["control_id"] == "pe-3"), "a ledger entry alone is not a requirement");
    let ac3 = rows.iter().find(|r| r["control_id"] == "ac-3").expect("a control a policy names");
    assert!(!ac3["policy_ids"].as_array().unwrap().is_empty());
    assert!(ac3["control_title"].as_str().unwrap().len() > 3, "a control id carries its title: {ac3}");

    let (s, doc) = send(&w.app, "GET", "/admin/oscal/component-definition", &w.admin, None).await;
    assert_eq!(s, StatusCode::OK);
    let reqs = doc["component-definition"]["components"][0]["control-implementations"][0]["implemented-requirements"].as_array().unwrap();
    let ids: Vec<&str> = reqs.iter().map(|r| r["control-id"].as_str().unwrap()).collect();
    assert!(!ids.contains(&"pe-3"), "{ids:?}");
    for id in &ids {
        assert_eq!(*id, scaffoldry_core::catalog::normalize(id), "{id} is not normalized");
    }
    let sa_req = reqs.iter().find(|r| r["control-id"] == "sa-9").unwrap();
    let prop = |name: &str| sa_req["props"].as_array().unwrap().iter().find(|p| p["name"] == name).map(|p| p["value"].clone());
    assert_eq!(prop("ledger-evidence-count"), Some(json!("3")));
    assert_eq!(prop("implementation-status"), Some(json!("planned")));
    let ac3_req = reqs.iter().find(|r| r["control-id"] == "ac-3").unwrap();
    assert!(ac3_req["props"].as_array().unwrap().iter().any(|p| p["name"] == "cedar-policy-id" && p["ns"] == "https://scaffoldry.io/ns/oscal"));
}

// 10. A compliance caller can edit OSCAL requirements and cannot create or activate a policy version.
#[tokio::test]
async fn a_compliance_officer_edits_requirements_and_nothing_else_in_policy() {
    let w = world();
    let (s, b) = send(&w.app, "PUT", "/admin/oscal/requirements/ac-3", &w.officer, Some(json!({ "statement": "Access is enforced by Cedar.", "status": "implemented", "reason": "Reviewed." }))).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_eq!(send(&w.app, "GET", "/admin/oscal/requirements", &w.officer, None).await.0, StatusCode::OK);
    assert_eq!(send(&w.app, "GET", "/admin/oscal/component-definition", &w.officer, None).await.0, StatusCode::OK);
    assert_eq!(w.draft(&w.officer, w.state.policy().source()).await.0, StatusCode::FORBIDDEN);
    let id = w.active().await["version"]["id"].as_str().unwrap().to_string();
    assert_eq!(w.activate(&w.officer, &id).await.0, StatusCode::FORBIDDEN);
    assert_eq!(send(&w.app, "GET", "/admin/policy/active", &w.officer, None).await.0, StatusCode::FORBIDDEN);
    // A bad status and a missing reason are refused.
    let (s, _) = send(&w.app, "PUT", "/admin/oscal/requirements/ac-3", &w.officer, Some(json!({ "statement": "x", "status": "done", "reason": "r" }))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    let (s, _) = send(&w.app, "PUT", "/admin/oscal/requirements/ac-3", &w.officer, Some(json!({ "statement": "x", "status": "planned" }))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    // A faculty member reaches none of it.
    assert_eq!(send(&w.app, "GET", "/admin/oscal/requirements", &w.faculty, None).await.0, StatusCode::FORBIDDEN);
}

// The simulator asks the real engine of the active version, or of a draft.
#[tokio::test]
async fn the_simulator_uses_the_real_engine() {
    let w = world();
    let (_, cases) = send(&w.app, "GET", "/admin/policy/cases", &w.admin, None).await;
    let c = cases["rows"].as_array().unwrap().iter().find(|c| c["name"] == "Staff can export protected student records").unwrap().clone();
    let ask = |version: Option<&str>| {
        let mut body = json!({ "principal": c["principal"], "action": "export", "resource": c["resource"] });
        if let Some(v) = version {
            body["version_id"] = json!(v);
        }
        body
    };
    let (s, b) = send(&w.app, "POST", "/admin/policy/simulate", &w.admin, Some(ask(None))).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_eq!(b["decision"], "Allow");
    let (_, d) = w.draft(&w.admin, &with_rule(w.state.policy().source(), STAFF_CANNOT_EXPORT)).await;
    let (s, b) = send(&w.app, "POST", "/admin/policy/simulate", &w.admin, Some(ask(d["id"].as_str()))).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_eq!(b["decision"], "Deny");
    let (s, _) = send(&w.app, "POST", "/admin/policy/simulate", &w.admin, Some(json!({ "principal": { "type": "User" }, "action": "export", "resource": {} }))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
}

// 11. The export validates against the OSCAL schema. CI sets OSCAL_OUT and validates the file with `validate-oscal.py`.
#[tokio::test]
async fn the_export_is_written_for_validation() {
    let w = world();
    let (s, doc) = send(&w.app, "GET", "/admin/oscal/component-definition", &w.admin, None).await;
    assert_eq!(s, StatusCode::OK);
    // The old route and the new one answer with the same document, apart from the ids made fresh each time.
    let (s, old) = send(&w.app, "GET", "/governance/oscal/export", &w.admin, None).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(old["component-definition"]["components"][0]["control-implementations"], doc["component-definition"]["components"][0]["control-implementations"]);
    if let Some(path) = std::env::var_os("OSCAL_OUT") {
        std::fs::write(path, serde_json::to_vec_pretty(&doc).unwrap()).expect("write OSCAL_OUT");
    }
}
