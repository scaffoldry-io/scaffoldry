//! Policy library phase 4: the library everyone reads, and what enforces each rule.

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

const ADMIN: &str = "jordan.lee@state.edu";
const OWNER: &str = "owner.library@state.edu";
const FACULTY: &str = "prof.curie@science.state.edu";
const DENIED: &str = "sarah.connor@state.edu";
const ROOT: &str = "00000000-0000-0000-0000-000000000001";
const P1: &str = "Student records are kept for seven years after the student leaves.";

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

struct World {
    db: TestDb,
    app: Router,
}

fn world() -> World {
    let db = TestDb::new();
    let state = db.state();
    state.seed_demo().expect("seed");
    let app = build_app_with_state(state).expect("router");
    for who in [OWNER, FACULTY, DENIED] {
        issue_test_token_and_user(who);
    }
    World { db, app }
}

impl World {
    async fn unit(&self, name: &str, parent: &str) -> String {
        let (s, u) = send(&self.app, "POST", "/api/v1/orgs", ADMIN, Some(json!({ "name": name, "code": name, "org_type": "Department", "parent_id": parent }))).await;
        assert_eq!(s, StatusCode::CREATED, "{u}");
        u["id"].as_str().unwrap().to_string()
    }

    /// A policy at `unit` with one text source and a snapshot of `text`. Returns the policy id and the snapshot id.
    async fn policy(&self, key: &str, unit: &str, text: &str) -> (String, String) {
        let body = json!({ "key": key, "title": format!("Policy {key}"), "summary": "A policy", "organization_id": unit, "owner": { "Person": { "eppn": OWNER } }, "reason": "Owned" });
        let (s, p) = send(&self.app, "POST", "/api/v1/admin/library/policies", ADMIN, Some(body)).await;
        assert_eq!(s, StatusCode::CREATED, "{p}");
        let policy = p["policy"]["id"].as_str().unwrap().to_string();
        let (_, src) = send(&self.app, "POST", &format!("/api/v1/admin/library/policies/{policy}/sources"), ADMIN, Some(json!({ "title": "Schedule", "kind": "text", "reason": "r" }))).await;
        let source = src["source"]["id"].as_str().unwrap().to_string();
        let (s, snap) = send(&self.app, "POST", &format!("/api/v1/admin/library/sources/{source}/snapshots"), ADMIN, Some(json!({ "text": text, "reason": "r" }))).await;
        assert_eq!(s, StatusCode::CREATED, "{snap}");
        (policy, snap["snapshot"]["id"].as_str().unwrap().to_string())
    }

    /// A rule drafted through the route and then set to `status` directly: the review flow is phase 3's.
    async fn rule(&self, policy: &str, snapshot: &str, key: &str, statement: &str, status: &str) {
        let body = json!({ "key": key, "statement": statement, "applies_to": "student records", "citations": [{ "snapshot_id": snapshot, "passage": P1 }], "reason": "r" });
        let (s, r) = send(&self.app, "POST", &format!("/api/v1/admin/library/policies/{policy}/rules"), ADMIN, Some(body)).await;
        assert_eq!(s, StatusCode::CREATED, "{r}");
        let id: Uuid = r["rule"]["id"].as_str().unwrap().parse().unwrap();
        let status = status.to_string();
        self.db.run(move |c| {
            c.execute("UPDATE library_rules SET status = $2, decided_by = 'reviewer@state.edu', decided_at = NOW() WHERE id = $1", &[&id, &status]).unwrap();
        });
    }

    async fn bind(&self, policy: &str, rule: &str, kind: &str, target: &str) -> (StatusCode, Value) {
        send(&self.app, "POST", &format!("/api/v1/admin/library/policies/{policy}/rules/{rule}/bindings"), ADMIN, Some(json!({ "target_kind": kind, "target_ref": target, "reason": "Enforced by this" }))).await
    }

    async fn library(&self, who: &str, query: &str) -> Value {
        let (s, v) = send(&self.app, "GET", &format!("/api/v1/library{query}"), who, None).await;
        assert_eq!(s, StatusCode::OK, "{v}");
        v
    }
}

fn rules_of(v: &Value) -> Vec<&Value> {
    v["rules"].as_array().unwrap().iter().collect()
}

// 1. A rule bound to an active target shows Enforced. Remove the target and it shows Guidance with Missing, and nothing else changes.
#[tokio::test]
async fn a_bound_rule_is_enforced_until_its_target_goes_and_then_it_is_guidance() {
    let w = world();
    let (policy, snap) = w.policy("records", ROOT, P1).await;
    w.rule(&policy, &snap, "retention", "Student records are kept for seven years.", "Published").await;
    let row = Uuid::new_v4();
    w.db.run(move |c| {
        c.execute("INSERT INTO authority_rows (id, organization_id, tier, path) VALUES ($1, $2, 'Moderate', 'second_person')", &[&row, &Uuid::parse_str(ROOT).unwrap()]).unwrap();
    });
    let before = w.library(FACULTY, "").await;
    assert_eq!(rules_of(&before)[0]["enforcement"], "Guidance", "{before}");

    let (s, b) = w.bind(&policy, "retention", "authority_row", &row.to_string()).await;
    assert_eq!(s, StatusCode::CREATED, "{b}");
    let bound = w.library(FACULTY, "").await;
    let r = rules_of(&bound)[0];
    assert_eq!(r["enforcement"], "Enforced", "{bound}");
    assert_eq!(r["enforced_by"][0]["state"], "Enforced");
    assert!(r["enforced_by"][0]["name"].as_str().is_some_and(|n| !n.is_empty()), "a name for each binding: {r}");

    w.db.run(move |c| {
        c.execute("DELETE FROM authority_rows WHERE id = $1", &[&row]).unwrap();
    });
    let after = w.library(FACULTY, "").await;
    let r2 = rules_of(&after)[0];
    assert_eq!(r2["enforcement"], "Guidance", "{after}");
    assert_eq!(r2["enforced_by"][0]["state"], "Missing");
    assert!(r2["warning"].as_str().is_some_and(|w| w.contains("owner")), "the owner is warned: {r2}");
    assert_eq!((&r["statement"], &r["policy_title"]), (&r2["statement"], &r2["policy_title"]), "no other change");

    // Filtering by enforcement.
    assert_eq!(rules_of(&w.library(FACULTY, "?enforcement=Enforced").await).len(), 0);
    assert_eq!(rules_of(&w.library(FACULTY, "?enforcement=Guidance").await).len(), 1);
}

// 2. A binding to a target that does not exist is refused. A kind whose brief has not run says which brief adds it.
#[tokio::test]
async fn a_binding_needs_a_target_that_exists_and_names_the_brief_for_a_kind_not_built() {
    let w = world();
    let (policy, snap) = w.policy("records", ROOT, P1).await;
    w.rule(&policy, &snap, "retention", "Student records are kept for seven years.", "Published").await;
    for (kind, target) in [("cedar_policy", "no-such-policy"), ("guard_template", "no_such_template"), ("authority_row", &Uuid::new_v4().to_string()), ("authority_row", "not-an-id")] {
        let (s, b) = w.bind(&policy, "retention", kind, target).await;
        assert_eq!(s, StatusCode::BAD_REQUEST, "{kind} {target}: {b}");
    }
    let (s, b) = w.bind(&policy, "retention", "sensitivity_category", "ferpa").await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{b}");
    assert!(b.to_string().contains("sensitive-content.md"), "{b}");
    let (s, b) = w.bind(&policy, "retention", "retention_policy", "x").await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{b}");
    assert!(b.to_string().contains("lifecycle.md"), "{b}");
    let (s, _) = w.bind(&policy, "retention", "colour", "x").await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    let (s, _) = w.bind(&policy, "nothing-here", "guard_template", "nope").await;
    assert_eq!(s, StatusCode::NOT_FOUND, "a rule key the policy does not have");
    let n: i64 = w.db.run(|c| c.query_one("SELECT COUNT(*) FROM library_bindings", &[]).unwrap().get(0));
    assert_eq!(n, 0, "nothing was stored");

    // A guard template that exists binds. Only a Platform Admin or compliance binds. Removing takes a reason.
    let template = scaffoldry_core::guards::ALLOWED_TEMPLATES[0];
    let (s, b) = send(&w.app, "POST", &format!("/api/v1/admin/library/policies/{policy}/rules/retention/bindings"), FACULTY, Some(json!({ "target_kind": "guard_template", "target_ref": template, "reason": "r" }))).await;
    assert_eq!(s, StatusCode::FORBIDDEN, "{b}");
    let (s, b) = w.bind(&policy, "retention", "guard_template", template).await;
    assert_eq!(s, StatusCode::CREATED, "{b}");
    let id = b["binding"]["id"].as_str().unwrap().to_string();
    let (s, _) = send(&w.app, "DELETE", &format!("/api/v1/admin/library/bindings/{id}"), ADMIN, Some(json!({}))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "a reason is required");
    let (s, b) = send(&w.app, "DELETE", &format!("/api/v1/admin/library/bindings/{id}"), ADMIN, Some(json!({ "reason": "No longer enforced" }))).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    let n: i64 = w.db.run(|c| c.query_one("SELECT COUNT(*) FROM library_bindings", &[]).unwrap().get(0));
    assert_eq!(n, 0);
    let entries: i64 = w.db.run(|c| c.query_one("SELECT COUNT(*) FROM governance_ledger WHERE decision_type = 'PolicyLibraryChanged' AND payload->>'action' IN ('binding_added', 'binding_removed')", &[]).unwrap().get(0));
    assert_eq!(entries, 2, "one ledger entry for each");
}

// 3. The library shows published rules only.
#[tokio::test]
async fn the_library_lists_only_published_rules() {
    let w = world();
    let (policy, snap) = w.policy("records", ROOT, P1).await;
    for (key, status) in [("pub", "Published"), ("draft", "Draft"), ("inreview", "InReview"), ("nope", "Rejected"), ("old", "Superseded"), ("gone", "Retired")] {
        w.rule(&policy, &snap, key, "Student records are kept for seven years.", status).await;
    }
    let all = w.library(FACULTY, "").await;
    let rules = rules_of(&all);
    assert_eq!(rules.len(), 1, "{all}");
    assert_eq!(rules[0]["rule_key"], "pub");
    assert_eq!(rules[0]["applies_to"], "student records");
    assert_eq!(rules[0]["policy_title"], "Policy records");
    assert!(rules[0].get("last_reviewed_at").is_some());
    // An unsigned caller gets nothing.
    let anon = w.app.clone().oneshot(Request::builder().uri("/api/v1/library").body(Body::empty()).unwrap()).await.unwrap();
    assert_eq!(anon.status(), StatusCode::UNAUTHORIZED);
    // Search and paging.
    assert_eq!(rules_of(&w.library(FACULTY, "?q=seven").await).len(), 1);
    assert_eq!(rules_of(&w.library(FACULTY, "?q=parking").await).len(), 0);
    assert_eq!(rules_of(&w.library(FACULTY, "?policy=records").await).len(), 1);
    assert_eq!(rules_of(&w.library(FACULTY, "?policy=other").await).len(), 0);
}

// 4. A person sees the policies of their own unit, the units above it, and the institution. Not another branch.
#[tokio::test]
async fn a_person_sees_their_units_policies_the_ones_above_and_the_institution_only() {
    let w = world();
    let college = w.unit("Science College", ROOT).await;
    let physics = w.unit("Physics Dept", &college).await;
    let athletics = w.unit("Athletics Dept", ROOT).await;
    for (key, unit) in [("inst", ROOT), ("college", college.as_str()), ("physics", physics.as_str()), ("athletics", athletics.as_str())] {
        let (policy, snap) = w.policy(key, unit, P1).await;
        w.rule(&policy, &snap, "retention", &format!("Rule of {key}."), "Published").await;
    }
    let (s, b) = send(&w.app, "POST", &format!("/api/v1/orgs/{physics}/appointments"), ADMIN, Some(json!({ "eppn": FACULTY, "scoped_affiliation": "unit_admin", "reason": "Physics" }))).await;
    assert_eq!(s, StatusCode::CREATED, "{b}");
    let seen = w.library(FACULTY, "").await;
    let mut keys: Vec<String> = rules_of(&seen).iter().map(|r| r["policy_key"].as_str().unwrap().to_string()).collect();
    keys.sort();
    assert_eq!(keys, vec!["college", "inst", "physics"], "{seen}");
    // The policy page and the rule page follow the same rule.
    let (s, _) = send(&w.app, "GET", "/api/v1/library/policies/athletics", FACULTY, None).await;
    assert_eq!(s, StatusCode::NOT_FOUND, "not visible, so not found");
    let (s, _) = send(&w.app, "GET", "/api/v1/library/rules/athletics/retention", FACULTY, None).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    let (s, p) = send(&w.app, "GET", "/api/v1/library/policies/physics", FACULTY, None).await;
    assert_eq!(s, StatusCode::OK, "{p}");
    // A person with no unit sees the institution's.
    let nobody = w.library(DENIED, "").await;
    assert_eq!(rules_of(&nobody).len(), 1, "{nobody}");
    assert_eq!(rules_of(&nobody)[0]["policy_key"], "inst");
}

// The policy page and the rule page.
#[tokio::test]
async fn the_policy_page_lists_its_sources_and_the_rule_page_its_citations_and_history() {
    let w = world();
    let (policy, snap) = w.policy("records", ROOT, &format!("{} {P1} {}", "Lead in. ".repeat(40), "Trail out. ".repeat(40))).await;
    w.rule(&policy, &snap, "retention", "Student records are kept for seven years.", "Superseded").await;
    w.rule(&policy, &snap, "retention", "Student records are kept seven years.", "Published").await;
    let (s, p) = send(&w.app, "GET", "/api/v1/library/policies/records", FACULTY, None).await;
    assert_eq!(s, StatusCode::OK, "{p}");
    assert_eq!(p["policy"]["title"], "Policy records");
    assert_eq!(p["rules"].as_array().unwrap().len(), 1, "published rules only: {p}");
    let src = &p["sources"][0];
    assert_eq!(src["title"], "Schedule");
    assert!(src["last_checked_at"].is_string(), "Last checked {{date}}: {src}");

    let (s, r) = send(&w.app, "GET", "/api/v1/library/rules/records/retention", FACULTY, None).await;
    assert_eq!(s, StatusCode::OK, "{r}");
    assert_eq!(r["rule"]["statement"], "Student records are kept seven years.");
    let c = &r["citations"][0];
    assert_eq!(c["passage"], P1);
    assert!(c["before"].as_str().unwrap().contains("Lead in."), "{c}");
    assert!(c["after"].as_str().unwrap().contains("Trail out."), "{c}");
    let history = r["history"].as_array().unwrap();
    assert_eq!(history.len(), 2, "both published versions, newest first: {r}");
    assert_eq!((history[0]["version"].as_i64(), history[1]["version"].as_i64()), (Some(2), Some(1)));
    assert_eq!(history[0]["decided_by"], "reviewer@state.edu");
    // A rule that is not published has no page.
    w.rule(&policy, &snap, "draft", "A draft.", "Draft").await;
    let (s, _) = send(&w.app, "GET", "/api/v1/library/rules/records/draft", FACULTY, None).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
}

// 5. The exported catalog is OSCAL: every control cites a source that is in the back matter.
#[tokio::test]
async fn the_exported_catalog_validates_and_every_reference_resolves() {
    let w = world();
    let (policy, snap) = w.policy("records", ROOT, P1).await;
    w.rule(&policy, &snap, "retention", "Student records are kept for seven years.", "Published").await;
    w.rule(&policy, &snap, "hold", "A hold stops every destruction.", "Published").await;
    w.rule(&policy, &snap, "draft", "Not published.", "Draft").await;
    let template = scaffoldry_core::guards::ALLOWED_TEMPLATES[0];
    let (s, _) = w.bind(&policy, "retention", "guard_template", template).await;
    assert_eq!(s, StatusCode::CREATED);
    let (s, _) = send(&w.app, "GET", "/api/v1/admin/library/catalog", FACULTY, None).await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    let (s, doc) = send(&w.app, "GET", "/api/v1/admin/library/catalog", ADMIN, None).await;
    assert_eq!(s, StatusCode::OK, "{doc}");

    let cat = &doc["catalog"];
    assert_eq!(cat["metadata"]["oscal-version"], "1.1.2");
    let controls: Vec<&Value> = cat["groups"].as_array().unwrap().iter().flat_map(|g| g["controls"].as_array().unwrap().iter()).collect();
    assert_eq!(controls.len(), 2, "published rules only: {doc}");
    let resources: Vec<&str> = cat["back-matter"]["resources"].as_array().unwrap().iter().map(|r| r["uuid"].as_str().unwrap()).collect();
    for c in &controls {
        assert!(c["parts"].as_array().unwrap().iter().any(|p| p["name"] == "statement" && p["prose"].is_string()));
        let refs: Vec<&str> = c["links"].as_array().unwrap().iter().filter(|l| l["rel"] == "reference").map(|l| l["href"].as_str().unwrap()).collect();
        assert!(!refs.is_empty(), "every control has a reference link: {c}");
        for href in refs {
            assert!(resources.contains(&href.trim_start_matches('#')), "{href} resolves to a back-matter resource");
        }
    }
    let bound = controls.iter().find(|c| c["id"] == "records-retention").unwrap();
    assert!(bound["props"].as_array().unwrap().iter().any(|p| p["name"] == "enforced-by" && p["ns"] == "https://scaffoldry.io/ns/oscal"), "{bound}");
    let resource = &cat["back-matter"]["resources"][0];
    assert_eq!(resource["rlinks"][0]["hashes"][0]["algorithm"], "SHA-256");

    // The file the build checks, and the official validator on it.
    let path = std::env::var("OSCAL_CATALOG_OUT").ok().map(std::path::PathBuf::from).unwrap_or_else(|| std::env::temp_dir().join(format!("catalog-{}.json", Uuid::new_v4())));
    std::fs::write(&path, serde_json::to_vec_pretty(&doc).unwrap()).unwrap();
    let script = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../governance/scripts/validate-oscal.py");
    match std::process::Command::new("python3").arg(&script).arg(&path).output() {
        Ok(out) if String::from_utf8_lossy(&out.stderr).contains("No module named") => eprintln!("jsonschema is not installed here: the build's gate validates the file"),
        Ok(out) => assert!(out.status.success(), "validate-oscal.py: {}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr)),
        Err(e) => eprintln!("python3 is not here ({e}): the build's gate validates the file"),
    }
}

// 6. A denial whose policy has a binding carries the rule's address.
#[tokio::test]
async fn a_denial_whose_policy_has_a_binding_carries_the_rules_address() {
    let w = world();
    let (s, denial) = send(&w.app, "GET", "/api/v1/workspaces/ws-bio-lab", DENIED, None).await;
    assert_eq!(s, StatusCode::FORBIDDEN, "{denial}");
    let policy_id = denial["policy"]["id"].as_str().expect("the denial names its policy").to_string();
    assert!(denial["policy"].get("rule").is_none(), "no binding yet: {denial}");

    let (policy, snap) = w.policy("access", ROOT, P1).await;
    w.rule(&policy, &snap, "restricted-labs", "Restricted labs are for their members.", "Published").await;
    let (s, b) = w.bind(&policy, "restricted-labs", "cedar_policy", &policy_id).await;
    assert_eq!(s, StatusCode::CREATED, "{b}");
    let (_, again) = send(&w.app, "GET", "/api/v1/workspaces/ws-bio-lab", DENIED, None).await;
    assert_eq!(again["policy"]["rule"], "/library/rules/access/restricted-labs", "{again}");
    // The rule shows what enforces it.
    let (_, r) = send(&w.app, "GET", "/api/v1/library/rules/access/restricted-labs", FACULTY, None).await;
    assert_eq!(r["rule"]["enforcement"], "Enforced", "{r}");
}

// The agent's tool shows the same enforcement state.
#[tokio::test]
async fn find_library_rules_marks_a_bound_rule_enforced() {
    let w = world();
    let (policy, snap) = w.policy("records", ROOT, P1).await;
    w.rule(&policy, &snap, "retention", "Student records are kept for seven years.", "Published").await;
    let template = scaffoldry_core::guards::ALLOWED_TEMPLATES[0];
    w.bind(&policy, "retention", "guard_template", template).await;
    let payload = json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": { "name": "find_library_rules", "arguments": { "query": "seven years" } } });
    let (_, v) = send(&w.app, "POST", "/api/v1/mcp", FACULTY, Some(payload)).await;
    assert_eq!(v["result"]["structuredContent"]["rules"][0]["enforcement"], "Enforced", "{v}");
}
