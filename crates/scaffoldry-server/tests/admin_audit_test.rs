//! Admin console phase 7: Audit and security. The ledger explorer with a verify and an export, the token list with a revoke,
//! the agent controls (tools and pages an administrator turns off), and one read-only inventory tool for agents.

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use common::TestDb;
use http_body_util::BodyExt;
use scaffoldry_core::ledger::DecisionType;
use scaffoldry_server::build_app_with_state;
use scaffoldry_server::service::admin::ADMIN_ROUTES;
use scaffoldry_server::service::identity::issue_test_token_and_user;
use scaffoldry_server::service::tools::TOOLS;
use scaffoldry_server::state::{RecordDecisionInput, ServerState};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::sync::Arc;
use tower::ServiceExt;

const ADMIN: &str = "jordan.lee@state.edu";
const FACULTY: &str = "sarah.connor@state.edu";
const COMPLIANCE: &str = "audit.officer@state.edu";
const OWNER: &str = "einstein@physics.state.edu";
const WS: &str = "ws-physics-optics";

struct Raw {
    status: StatusCode,
    content_type: String,
    body: String,
}

impl Raw {
    fn json(&self) -> Value {
        serde_json::from_str(&self.body).unwrap_or(Value::Null)
    }
}

async fn raw_send(app: &axum::Router, method: &str, uri: &str, token: Option<&str>, body: Option<Value>) -> Raw {
    let mut b = Request::builder().method(method).uri(uri);
    if let Some(t) = token {
        b = b.header("authorization", format!("Bearer {t}"));
    }
    let req = match body {
        Some(v) => b.header("content-type", "application/json").body(Body::from(v.to_string())).unwrap(),
        None => b.body(Body::empty()).unwrap(),
    };
    let resp = app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let content_type = resp.headers().get("content-type").and_then(|v| v.to_str().ok()).unwrap_or("").to_string();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    Raw { status, content_type, body: String::from_utf8_lossy(&bytes).to_string() }
}

struct World {
    db: TestDb,
    state: Arc<ServerState>,
    app: axum::Router,
    admin: String,
    compliance: String,
}

async fn world() -> World {
    let db = TestDb::new();
    let state = db.state();
    state.seed_demo().expect("seed");
    let app = build_app_with_state(state.clone()).expect("router");
    let admin = issue_test_token_and_user(ADMIN);
    // Issuing the token makes the person. The role is put on them after.
    let compliance = issue_test_token_and_user(COMPLIANCE);
    db.run(|c| {
        c.execute(
            "INSERT INTO roles (id, person_id, organization_id, role_title, scoped_affiliation, is_primary, source) \
             SELECT gen_random_uuid(), p.id, o.id, 'Compliance Officer', 'compliance', true, 'api' \
             FROM persons p, organizations o WHERE LOWER(p.eppn) = LOWER($1) AND o.parent_id IS NULL ON CONFLICT DO NOTHING",
            &[&COMPLIANCE],
        )
        .unwrap();
    });
    World { db, state, app, admin, compliance }
}

impl World {
    async fn get(&self, uri: &str, token: &str) -> (StatusCode, Value) {
        let r = raw_send(&self.app, "GET", &format!("/api/v1{uri}"), Some(token), None).await;
        (r.status, r.json())
    }

    async fn send(&self, method: &str, uri: &str, token: &str, body: Option<Value>) -> (StatusCode, Value) {
        let r = raw_send(&self.app, method, &format!("/api/v1{uri}"), Some(token), body).await;
        (r.status, r.json())
    }

    async fn rpc(&self, token: &str, method: &str, params: Value) -> Value {
        let payload = json!({ "jsonrpc": "2.0", "id": 1, "method": method, "params": params });
        raw_send(&self.app, "POST", "/api/mcp", Some(token), Some(payload)).await.json()
    }

    /// A tool call. `Err` is the tool's own message.
    async fn tool(&self, token: &str, name: &str, args: Value) -> Result<Value, String> {
        let v = self.rpc(token, "tools/call", json!({ "name": name, "arguments": args })).await;
        if let Some(err) = v.get("error") {
            return Err(err["message"].to_string());
        }
        if v["result"]["isError"] == true {
            return Err(v["result"]["content"][0]["text"].as_str().unwrap_or("").to_string());
        }
        Ok(v["result"]["structuredContent"].clone())
    }

    async fn tool_names(&self, token: &str) -> Vec<String> {
        let v = self.rpc(token, "tools/list", json!({})).await;
        v["result"]["tools"].as_array().unwrap().iter().map(|t| t["name"].as_str().unwrap().to_string()).collect()
    }

    fn append(&self, decision: DecisionType, control: &str, rationale: &str, payload: Value) {
        self.state
            .append_ledger_entry(RecordDecisionInput {
                principal: ADMIN.to_string(),
                organization_code: "DIV-ADMIN-CONSOLE".to_string(),
                app_slug: None,
                decision_type: decision,
                oscal_control_id: control.to_string(),
                rationale: rationale.to_string(),
                payload: &payload,
            })
            .expect("a ledger entry");
    }

    fn count(&self, kind: &str) -> i64 {
        let kind = kind.to_string();
        self.db.run(move |c| c.query_one("SELECT count(*) FROM governance_ledger WHERE decision_type = $1", &[&kind]).unwrap().get(0))
    }
}

// 1. 120 ledger rows, filter by decision_type, limit 50: pages are newest first with no repeats. Each row has a payload.
#[tokio::test(flavor = "multi_thread")]
async fn the_ledger_pages_newest_first_with_no_repeats_and_every_row_has_its_payload() {
    let w = world().await;
    for i in 0..120 {
        w.append(DecisionType::JobCancelled, "AU-02", &format!("Cancelled job {i}"), json!({ "job_id": format!("job-{i}") }));
    }
    w.append(DecisionType::TokenRevoked, "IA-05", "Another kind", json!({}));
    let expected = w.count("JobCancelled");
    assert!(expected >= 120);

    let mut seen: Vec<i64> = Vec::new();
    let mut cursor: Option<String> = None;
    let mut pages = 0;
    loop {
        let mut uri = "/admin/ledger?decision_type=JobCancelled&limit=50".to_string();
        if let Some(c) = &cursor {
            uri.push_str(&format!("&cursor={}", c.replace('+', "%2B").replace('=', "%3D")));
        }
        let (s, b) = w.get(&uri, &w.admin).await;
        assert_eq!(s, StatusCode::OK, "{b}");
        let rows = b["rows"].as_array().unwrap();
        assert!(rows.len() <= 50);
        for r in rows {
            assert_eq!(r["decision_type"], "JobCancelled");
            assert!(r["payload"].is_object(), "each row carries its stored payload: {r}");
            assert_eq!(r["control_title"], scaffoldry_core::catalog::title("AU-02").map(Value::from).unwrap_or(Value::Null));
            seen.push(r["sequence"].as_i64().unwrap());
        }
        pages += 1;
        cursor = b["next_cursor"].as_str().map(str::to_string);
        if cursor.is_none() {
            break;
        }
    }
    assert!(pages >= 3, "{pages} pages");
    assert_eq!(seen.len() as i64, expected, "every matching row, once");
    assert!(seen.windows(2).all(|p| p[0] > p[1]), "newest first, no repeats");

    // Other filters: control (any spelling), principal, and an unknown type.
    let (_, by_control) = w.get("/admin/ledger?control=ia-5&limit=200", &w.admin).await;
    assert!(by_control["rows"].as_array().unwrap().iter().any(|r| r["rationale"] == "Another kind"), "{by_control}");
    let (_, by_principal) = w.get(&format!("/admin/ledger?principal={ADMIN}&decision_type=TokenRevoked"), &w.admin).await;
    assert!(!by_principal["rows"].as_array().unwrap().is_empty());
    let (s, _) = w.get("/admin/ledger?decision_type=Nonsense", &w.admin).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
}

// 2. verify on a clean ledger returns valid. With one row's rationale changed, it returns the first bad sequence.
#[tokio::test(flavor = "multi_thread")]
async fn verify_reports_a_clean_chain_and_the_first_bad_sequence() {
    let w = world().await;
    for i in 0..5 {
        w.append(DecisionType::JobCancelled, "AU-02", &format!("Entry {i}"), json!({ "n": i }));
    }
    let (s, clean) = w.send("POST", "/admin/ledger/verify", &w.admin, None).await;
    assert_eq!(s, StatusCode::OK, "{clean}");
    assert_eq!(clean["valid"], true, "{clean}");
    assert!(clean["entries"].as_i64().unwrap() >= 5);
    assert_eq!(clean["head_hash"].as_str().unwrap().len(), 64);
    assert!(clean["first_bad_sequence"].is_null());

    let target: i64 = w.db.run(|c| {
        c.batch_execute("DROP TRIGGER governance_ledger_no_change ON governance_ledger").unwrap();
        let seq: i64 = c.query_one("SELECT sequence FROM governance_ledger WHERE rationale = 'Entry 2'", &[]).unwrap().get(0);
        c.execute("UPDATE governance_ledger SET rationale = 'Rewritten' WHERE sequence = $1", &[&seq]).unwrap();
        seq
    });
    let (s, bad) = w.send("POST", "/admin/ledger/verify", &w.admin, None).await;
    assert_eq!(s, StatusCode::OK, "{bad}");
    assert_eq!(bad["valid"], false, "{bad}");
    assert_eq!(bad["first_bad_sequence"], target, "{bad}");
}

// 3. Export with 60,000 matching rows is 400. A JSON export starts with the head hash. A CSV export has a header row.
#[tokio::test(flavor = "multi_thread")]
async fn export_starts_with_the_head_hash_has_a_csv_header_and_refuses_too_many_rows() {
    let w = world().await;
    w.append(DecisionType::TokenRevoked, "IA-05", "=cmd|' /C calc'!A0", json!({ "token_id": "t1" }));
    let (_, head) = w.send("POST", "/admin/ledger/verify", &w.admin, None).await;

    let json_export = raw_send(&w.app, "GET", "/api/v1/admin/ledger/export?format=json&decision_type=TokenRevoked", Some(&w.admin), None).await;
    assert_eq!(json_export.status, StatusCode::OK, "{}", json_export.body);
    assert!(json_export.body.starts_with("{\"head_hash\":\""), "the head hash comes first: {}", &json_export.body[..60.min(json_export.body.len())]);
    let doc = json_export.json();
    assert_eq!(doc["head_hash"], head["head_hash"]);
    assert_eq!(doc["verify"]["valid"], true);
    assert!(!doc["entries"].as_array().unwrap().is_empty());
    assert!(doc["entries"][0]["payload"].is_object());

    let csv = raw_send(&w.app, "GET", "/api/v1/admin/ledger/export?format=csv&decision_type=TokenRevoked", Some(&w.admin), None).await;
    assert_eq!(csv.status, StatusCode::OK);
    assert!(csv.content_type.starts_with("text/csv"), "{}", csv.content_type);
    let first = csv.body.lines().next().unwrap();
    assert!(first.starts_with("sequence,") && first.contains("decision_type") && first.contains("control_title") && first.contains("payload"), "{first}");
    assert!(csv.body.contains("'=cmd"), "a cell that starts like a formula is defused");

    let bad = raw_send(&w.app, "GET", "/api/v1/admin/ledger/export?format=xml", Some(&w.admin), None).await;
    assert_eq!(bad.status, StatusCode::BAD_REQUEST);

    w.db.run(|c| {
        c.execute(
            "INSERT INTO governance_ledger (sequence, entry_hash, previous_hash, timestamp_iso, principal, organization_code, app_slug, decision_type, oscal_control_id, rationale, payload_hash, payload) \
             SELECT 100000 + g, md5(g::text) || md5((g + 1)::text), repeat('0', 64), '2026-01-01T00:00:00+00:00', 'bulk@state.edu', 'X', NULL, 'AppArchived', 'CM-03', 'bulk', repeat('0', 64), '{}' \
             FROM generate_series(1, 60000) g",
            &[],
        )
        .unwrap();
    });
    let big = raw_send(&w.app, "GET", "/api/v1/admin/ledger/export?format=json&decision_type=AppArchived", Some(&w.admin), None).await;
    assert_eq!(big.status, StatusCode::BAD_REQUEST, "{}", &big.body[..200.min(big.body.len())]);
    assert!(big.body.to_lowercase().contains("narrow"), "{}", big.body);
    let big_csv = raw_send(&w.app, "GET", "/api/v1/admin/ledger/export?format=csv&principal=bulk@state.edu", Some(&w.admin), None).await;
    assert_eq!(big_csv.status, StatusCode::BAD_REQUEST);
}

// 4. Revoking a token makes it 401 on the next request and adds one ledger entry. The response never contains a hash.
#[tokio::test(flavor = "multi_thread")]
async fn revoking_a_token_ends_it_writes_one_entry_and_shows_no_hash() {
    let w = world().await;
    let (s, minted) = w.send("POST", "/auth/tokens", &w.admin, Some(json!({ "label": "Audit bot", "kind": "agent", "days": 5 }))).await;
    assert_eq!(s, StatusCode::CREATED, "{minted}");
    let (id, secret) = (minted["id"].as_str().unwrap().to_string(), minted["token"].as_str().unwrap().to_string());
    assert_eq!(w.get("/auth/me", &secret).await.0, StatusCode::OK);

    let (s, list) = w.get("/admin/tokens?kind=agent&owner=jordan", &w.admin).await;
    assert_eq!(s, StatusCode::OK, "{list}");
    let row = list["rows"].as_array().unwrap().iter().find(|r| r["id"] == id.as_str()).expect("the token is listed").clone();
    assert_eq!(row["kind"], "agent");
    assert_eq!(row["owner"], ADMIN);
    assert_eq!(row["label"], "Audit bot");
    assert_eq!(row["state"], "active");
    assert!(!list.to_string().contains("token_hash") && !list.to_string().contains(&secret), "no hash or token in the list");

    let before = w.count("TokenRevoked");
    let (s, none) = w.send("DELETE", &format!("/admin/tokens/{id}"), &w.admin, Some(json!({}))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "a reason is required: {none}");
    assert_eq!(w.count("TokenRevoked"), before);
    let (s, done) = w.send("DELETE", &format!("/admin/tokens/{id}"), &w.admin, Some(json!({ "reason": "The contractor left" }))).await;
    assert_eq!(s, StatusCode::OK, "{done}");
    let text = done.to_string();
    assert!(!text.contains("token_hash") && !text.contains(&secret), "{text}");
    assert_eq!(w.count("TokenRevoked"), before + 1, "one ledger entry");
    assert_eq!(w.get("/auth/me", &secret).await.0, StatusCode::UNAUTHORIZED, "the next request is refused");

    let (_, revoked) = w.get("/admin/tokens?state=revoked&kind=agent", &w.admin).await;
    assert!(revoked["rows"].as_array().unwrap().iter().any(|r| r["id"] == id.as_str() && r["state"] == "revoked"));
    let (s, _) = w.send("DELETE", &format!("/admin/tokens/{id}"), &w.admin, Some(json!({ "reason": "Again" }))).await;
    assert_eq!(s, StatusCode::CONFLICT, "revoking twice is a conflict");
    assert_eq!(w.count("TokenRevoked"), before + 1);
    let (s, _) = w.send("DELETE", &format!("/admin/tokens/{}", uuid::Uuid::new_v4()), &w.admin, Some(json!({ "reason": "Nope" }))).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
}

fn sha(text: &str) -> String {
    Sha256::digest(text.as_bytes()).iter().map(|b| format!("{b:02x}")).collect()
}

// 5. Setting mcp.disabled_tools to ["create_record"] removes it from tools/list and makes a call a tool error. A name not
// in TOOLS is 400. A faculty caller cannot change the setting.
#[tokio::test(flavor = "multi_thread")]
async fn disabling_a_tool_removes_it_from_the_list_and_refuses_the_call() {
    let w = world().await;
    assert!(w.tool_names(&w.admin).await.contains(&"create_record".to_string()));
    let faculty = issue_test_token_and_user(FACULTY);

    let (s, _) = w.send("PUT", "/settings/mcp.disabled_tools?reason=Test", &faculty, Some(json!(["create_record"]))).await;
    assert_eq!(s, StatusCode::FORBIDDEN, "a faculty caller cannot change it");
    let (s, b) = w.send("PUT", "/settings/mcp.disabled_tools?reason=Test", &w.admin, Some(json!(["no_such_tool"]))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{b}");
    assert!(b.to_string().contains("no_such_tool"), "the error names the tool: {b}");
    let (s, _) = w.send("PUT", "/settings/mcp.disabled_tools", &w.admin, Some(json!("create_record"))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);

    // The tool works before.
    let args = json!({ "app_slug": "physics-admissions-review", "data": {} });
    let before = w.tool(&w.admin, "create_record", args.clone()).await;
    assert!(!before.as_ref().is_err_and(|e| e.contains("disabled")), "{before:?}");

    let (s, b) = w.send("PUT", "/settings/mcp.disabled_tools?reason=Pause%20writes", &w.admin, Some(json!(["create_record"]))).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert!(!w.tool_names(&w.admin).await.contains(&"create_record".to_string()), "tools/list omits it");
    let refused = w.tool(&w.admin, "create_record", args).await.unwrap_err();
    assert!(refused.contains("Tool disabled by administrator"), "{refused}");
    // The refusal comes before the scope check: a caller with no right gets the same words, not a scope answer.
    let sarah_args = json!({ "app_slug": "physics-admissions-review", "data": {} });
    let refused = w.tool(&faculty, "create_record", sarah_args).await.unwrap_err();
    assert!(refused.contains("Tool disabled by administrator"), "{refused}");

    // The list of tools for the console says which are off.
    let (s, tools) = w.get("/admin/tools", &w.admin).await;
    assert_eq!(s, StatusCode::OK, "{tools}");
    let row = tools["tools"].as_array().unwrap().iter().find(|t| t["name"] == "create_record").unwrap().clone();
    assert_eq!(row["disabled"], true);
    assert_eq!(row["read_only"], false);
    assert_eq!(tools["tools"].as_array().unwrap().len(), TOOLS.len());
    assert_eq!(w.get("/admin/tools", &faculty).await.0, StatusCode::FORBIDDEN);
}

fn field(name: &str) -> Value {
    json!({ "name": name, "label": name, "field_type": "Text", "required": false, "ferpa_sensitive": false })
}

// 5b. pages.disabled: a disabled page is refused by get_page, page_call, and the page route. The list is bounded.
#[tokio::test(flavor = "multi_thread")]
async fn a_disabled_page_does_not_run_and_the_setting_is_checked() {
    let w = world().await;
    let owner = issue_test_token_and_user(OWNER);
    let slug = format!("pg-{}", &uuid::Uuid::new_v4().simple().to_string()[..8]);
    let source = "export default function Page() { return <p>HELLO</p>; }";
    let manifest = json!({
        "slug": slug, "title": "Pages", "description": "d", "organization_code": "PHYS", "department": "physics", "workspace_id": WS,
        "views": [],
        "tables": [{ "id": "grants", "name": "Grants", "slug": "grants", "fields": [field("title")] }],
        "pages": [{ "id": "board", "title": "Board", "kind": { "Custom": { "source_sha256": sha(source), "grants": [
            { "table_id": "grants", "read": ["title"], "write": [], "create": false, "delete": false }
        ]}}}]
    });
    let proposed = w.tool(&owner, "propose_app_change", json!({ "workspace_id": WS, "manifest": manifest, "summary": "Pages", "page_sources": [source] })).await.expect("proposed");
    w.tool(&owner, "decide_proposal", json!({ "proposal_id": proposed["proposal_id"], "decision": "approve", "rationale": "Reviewed", "digest": proposed["digest"] })).await.expect("approved");
    assert!(w.tool(&owner, "get_page", json!({ "app_slug": slug, "page_id": "board" })).await.is_ok());

    let key = format!("{slug}/board");
    let (s, _) = w.send("PUT", "/settings/pages.disabled?reason=Hold", &w.admin, Some(json!(["no-slash"]))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "an entry is app/page");
    let too_many: Vec<String> = (0..201).map(|i| format!("app{i}/page")).collect();
    let (s, _) = w.send("PUT", "/settings/pages.disabled?reason=Hold", &w.admin, Some(json!(too_many))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "at most 200");

    let (s, b) = w.send("PUT", "/settings/pages.disabled?reason=Hold", &w.admin, Some(json!([key]))).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    let err = w.tool(&owner, "get_page", json!({ "app_slug": slug, "page_id": "board" })).await.unwrap_err();
    assert!(err.contains("This page was disabled by an administrator."), "{err}");
    let err = w.tool(&owner, "page_call", json!({ "app_slug": slug, "page_id": "board", "op": "list", "table_id": "grants" })).await.unwrap_err();
    assert!(err.contains("This page was disabled by an administrator."), "{err}");
    let (s, b) = w.get(&format!("/apps/{slug}/pages/board"), &owner).await;
    assert_eq!(s, StatusCode::FORBIDDEN, "{b}");
    assert!(b.to_string().contains("This page was disabled by an administrator."), "{b}");

    let (s, pages) = w.get("/admin/pages", &w.admin).await;
    assert_eq!(s, StatusCode::OK, "{pages}");
    let row = pages["pages"].as_array().unwrap().iter().find(|p| p["app_slug"] == slug.as_str()).expect("the page is listed").clone();
    assert_eq!(row["page_id"], "board");
    assert_eq!(row["disabled"], true);
    assert!(row["grants"].as_str().unwrap().contains("grants"), "a grants sentence: {row}");

    let (s, _) = w.send("PUT", "/settings/pages.disabled?reason=Back", &w.admin, Some(json!([]))).await;
    assert_eq!(s, StatusCode::OK);
    assert!(w.tool(&owner, "get_page", json!({ "app_slug": slug, "page_id": "board" })).await.is_ok(), "on again");
}

// 6. admin_inventory is absent from tools/list for a faculty caller, is read_only, and returns the same rows as GET /admin/users.
#[tokio::test(flavor = "multi_thread")]
async fn admin_inventory_is_for_a_platform_admin_is_read_only_and_matches_the_route() {
    let w = world().await;
    let faculty = issue_test_token_and_user(FACULTY);
    assert!(!w.tool_names(&faculty).await.contains(&"admin_inventory".to_string()), "a faculty caller does not see it");
    assert!(w.tool_names(&w.admin).await.contains(&"admin_inventory".to_string()));
    let listed = w.rpc(&w.admin, "tools/list", json!({})).await;
    let entry = listed["result"]["tools"].as_array().unwrap().iter().find(|t| t["name"] == "admin_inventory").unwrap().clone();
    assert_eq!(entry["annotations"]["readOnlyHint"], true);
    assert!(TOOLS.iter().find(|t| t.name == "admin_inventory").unwrap().read_only);

    let refused = w.tool(&faculty, "admin_inventory", json!({ "kind": "users" })).await.unwrap_err();
    assert!(refused.to_lowercase().contains("platform admin"), "{refused}");

    let (_, route) = w.get("/admin/users?affiliation=faculty&limit=3", &w.admin).await;
    let tool = w.tool(&w.admin, "admin_inventory", json!({ "kind": "users", "filter": { "affiliation": "faculty" }, "limit": 3 })).await.expect("the tool");
    assert_eq!(tool["rows"], route["users"], "the same rows as the route");
    assert_eq!(tool["next_cursor"], route["next_cursor"]);

    // Every kind answers, with rows and a cursor.
    for kind in ["users", "workspaces", "apps", "fields", "processes", "process_instances", "tokens", "ledger"] {
        let t = w.tool(&w.admin, "admin_inventory", json!({ "kind": kind, "limit": 2 })).await.unwrap_or_else(|e| panic!("{kind}: {e}"));
        assert!(t["rows"].is_array(), "{kind}: {t}");
        assert!(t["rows"].as_array().unwrap().len() <= 2, "{kind}");
    }
    let t = w.tool(&w.admin, "admin_inventory", json!({ "kind": "ledger", "filter": { "decision_type": "PolicyRevision" }, "limit": 2 })).await.unwrap();
    assert!(t["rows"].as_array().unwrap().iter().all(|r| r["decision_type"] == "PolicyRevision" && r["payload"].is_object()));
    let t = w.tool(&w.admin, "admin_inventory", json!({ "kind": "tokens" })).await.unwrap();
    assert!(!t.to_string().contains("token_hash"));
    assert!(w.tool(&w.admin, "admin_inventory", json!({ "kind": "secrets" })).await.is_err(), "an unknown kind is refused");
    assert!(w.tool(&w.admin, "admin_inventory", json!({ "kind": "users", "limit": 500 })).await.is_err(), "limit is at most 200");
}

// 7. No TOOLS entry whose name starts with admin_ is a write tool.
#[test]
fn no_admin_tool_writes() {
    let names: Vec<&str> = TOOLS.iter().filter(|t| t.name.starts_with("admin_")).map(|t| t.name).collect();
    assert!(names.contains(&"admin_inventory"));
    for t in TOOLS.iter().filter(|t| t.name.starts_with("admin_")) {
        assert!(t.read_only, "{} must be read only: there is no admin write tool", t.name);
        assert!(matches!(t.scope, scaffoldry_server::service::tools::Scope::PlatformAdmin), "{} is for a Platform Admin", t.name);
    }
}

// 9. Every route above is in ADMIN_ROUTES, with the compliance exceptions tested.
#[tokio::test(flavor = "multi_thread")]
async fn every_audit_route_is_listed_and_the_compliance_exceptions_hold() {
    let routes: [(&str, &str); 7] = [
        ("GET", "/admin/ledger"),
        ("POST", "/admin/ledger/verify"),
        ("GET", "/admin/ledger/export"),
        ("GET", "/admin/tokens"),
        ("DELETE", "/admin/tokens/{id}"),
        ("GET", "/admin/tools"),
        ("GET", "/admin/pages"),
    ];
    for route in routes {
        assert!(ADMIN_ROUTES.contains(&route), "{route:?} is in ADMIN_ROUTES");
    }
    let w = world().await;
    let faculty = issue_test_token_and_user(FACULTY);
    let id = uuid::Uuid::new_v4().to_string();
    for (method, path) in routes {
        let uri = format!("/api/v1{}", path.replace("{id}", &id));
        assert_eq!(raw_send(&w.app, method, &uri, None, None).await.status, StatusCode::UNAUTHORIZED, "{method} {path} with no token");
        assert_eq!(raw_send(&w.app, method, &uri, Some(&faculty), None).await.status, StatusCode::FORBIDDEN, "{method} {path} as faculty");
    }
    // A compliance officer reads the ledger, verifies it and exports it. The tokens and the agent controls are not theirs.
    for (method, path) in [("GET", "/admin/ledger"), ("POST", "/admin/ledger/verify"), ("GET", "/admin/ledger/export?format=csv")] {
        let r = raw_send(&w.app, method, &format!("/api/v1{path}"), Some(&w.compliance), None).await;
        assert_eq!(r.status, StatusCode::OK, "{method} {path} as compliance: {}", r.body);
    }
    for (method, path) in [("GET", "/admin/tokens"), ("GET", "/admin/tools"), ("GET", "/admin/pages")] {
        let r = raw_send(&w.app, method, &format!("/api/v1{path}"), Some(&w.compliance), None).await;
        assert_eq!(r.status, StatusCode::FORBIDDEN, "{method} {path} as compliance");
    }
    let r = raw_send(&w.app, "DELETE", &format!("/api/v1/admin/tokens/{id}"), Some(&w.compliance), Some(json!({ "reason": "x" }))).await;
    assert_eq!(r.status, StatusCode::FORBIDDEN);
    let (s, _) = w.send("PUT", "/settings/mcp.disabled_tools", &w.compliance, Some(json!([]))).await;
    assert_eq!(s, StatusCode::FORBIDDEN, "a compliance officer cannot turn a tool off");
}

// The Overview's recent changes carry the control title, as the ledger rows do.
#[tokio::test(flavor = "multi_thread")]
async fn the_overview_lists_recent_changes_with_the_control_title() {
    let w = world().await;
    w.append(DecisionType::PolicyRevision, "AC-02", "Checked accounts", json!({}));
    let (s, o) = w.get("/admin/overview", &w.admin).await;
    assert_eq!(s, StatusCode::OK, "{o}");
    let recent = o["ledger"]["recent"].as_array().expect("recent changes");
    assert!(!recent.is_empty() && recent.len() <= 5);
    assert_eq!(recent[0]["rationale"], "Checked accounts");
    assert_eq!(recent[0]["oscal_control_id"], "AC-02");
    assert_eq!(recent[0]["control_title"], "Account Management");
}
