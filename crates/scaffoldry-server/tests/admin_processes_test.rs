//! Admin console phase 6: the Business processes panel. Definitions with their versions, a switch to turn one off or on,
//! a simulate button, and the instances that wait: who can decide, reassign, cancel.

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use common::TestDb;
use http_body_util::BodyExt;
use scaffoldry_server::build_app_with_state;
use scaffoldry_server::service::admin::ADMIN_ROUTES;
use scaffoldry_server::service::identity::issue_test_token_and_user;
use scaffoldry_server::state::ServerState;
use serde_json::{json, Value};
use std::sync::Arc;
use tower::ServiceExt;

const ADMIN: &str = "jordan.lee@state.edu";
/// Owns the workspace and decides the steps that wait on the role `owner`.
const OWNER: &str = "einstein@physics.state.edu";
const WS: &str = "ws-physics-optics";

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
    state: Arc<ServerState>,
    app: axum::Router,
    slug: String,
    /// Makes the records, so the process never lets this person decide.
    editor: String,
}

fn field(name: &str) -> Value {
    json!({ "name": name, "label": name, "field_type": "Text", "required": false, "ferpa_sensitive": false })
}

fn unique(prefix: &str) -> String {
    format!("{prefix}-{}", &uuid::Uuid::new_v4().simple().to_string()[..8])
}

async fn world() -> World {
    let db = TestDb::new();
    let state = db.state();
    state.seed_demo().expect("seed");
    let app = build_app_with_state(state.clone()).expect("router");
    let slug = unique("proc");
    let manifest = json!({
        "slug": slug, "title": "Requests", "description": "", "organization_code": "PHYS", "department": "physics", "workspace_id": WS, "views": [],
        "tables": [{ "id": "requests", "name": "Requests", "slug": "requests", "primary_field": "name", "fields": [field("name"), field("status")] }],
    });
    let (s, b) = send(&app, "POST", &format!("/api/v1/workspaces/{WS}/apps"), OWNER, Some(manifest)).await;
    assert_eq!(s, StatusCode::CREATED, "{b}");
    let editor = format!("{}@state.edu", unique("editor"));
    let w = World { db, state, app, slug, editor: editor.clone() };
    w.person(&editor, Some("editor")).await;
    w
}

impl World {
    /// A person the directory knows. With a role, also a collaborator on the workspace, so they can read the app.
    async fn person(&self, who: &str, role: Option<&str>) -> Value {
        let (s, b) = send(&self.app, "POST", "/api/v1/admin/users", ADMIN, Some(json!({
            "userName": who, "name": "Test Person", "email": who, "affiliation": "faculty", "department": "Physics", "title": "Lecturer", "reason": "Fixture"
        }))).await;
        assert_eq!(s, StatusCode::CREATED, "{b}");
        if let Some(role) = role {
            let (s, b) = send(&self.app, "POST", &format!("/api/v1/workspaces/{WS}/collaborators"), ADMIN, Some(json!({ "eppn": who, "role": role }))).await;
            assert!(s.is_success(), "{b}");
        }
        b_user(&b)
    }

    /// A rule that waits on the role `role` after a record is created.
    async fn rule(&self, role: &str) -> String {
        let id = unique("rule");
        let rule = json!({
            "id": id, "name": format!("Review {id}"), "enabled": true, "trigger": "RecordCreated",
            "steps": [{ "id": "decide", "when": [], "kind": { "UserTask": {
                "role": role, "prompt": "Decide",
                "approve": [{ "UpdateRecordStatus": { "new_status": "Admitted" } }],
                "reject": [{ "UpdateRecordStatus": { "new_status": "Denied" } }],
                "approver": { "Role": { "name": role } }
            }}}]
        });
        let (s, b) = send(&self.app, "POST", &format!("/api/v1/apps/{}/automations", self.slug), OWNER, Some(rule)).await;
        assert_eq!(s, StatusCode::CREATED, "{b}");
        id
    }

    /// A record the editor makes. Returns its id.
    async fn record(&self) -> String {
        let (s, b) = send(&self.app, "POST", &format!("/api/v1/apps/{}/records", self.slug), &self.editor, Some(json!({ "data": { "_table_id": "requests", "name": "Ada" } }))).await;
        assert_eq!(s, StatusCode::CREATED, "{b}");
        b["id"].as_str().unwrap().to_string()
    }

    async fn admin(&self, method: &str, uri: &str, body: Option<Value>) -> (StatusCode, Value) {
        send(&self.app, method, &format!("/api/v1{uri}"), ADMIN, body).await
    }

    async fn instances(&self, query: &str) -> Vec<Value> {
        let (s, b) = self.admin("GET", &format!("/admin/process-instances?app={}{query}", self.slug), None).await;
        assert_eq!(s, StatusCode::OK, "{b}");
        b["rows"].as_array().unwrap().clone()
    }

    fn ledger(&self, kind: &str) -> Vec<Value> {
        let kind = kind.to_string();
        self.db.run(move |c| c.query("SELECT payload FROM governance_ledger WHERE decision_type = $1 ORDER BY sequence", &[&kind]).unwrap().iter().map(|r| r.get(0)).collect())
    }
}

fn b_user(b: &Value) -> Value {
    b["user"].clone()
}

fn enc(id: &str) -> String {
    id.replace(':', "%3A")
}

// 1. Disable a rule. A new matching record starts no instance. A waiting instance can still be decided.
#[tokio::test(flavor = "multi_thread")]
async fn a_disabled_rule_starts_nothing_new_and_a_waiting_instance_can_still_be_decided() {
    let w = world().await;
    let rule = w.rule("owner").await;
    let first = w.record().await;
    let waiting = format!("{rule}:{first}");
    assert_eq!(w.instances("").await.len(), 1);

    let (s, b) = w.admin("PATCH", &format!("/admin/processes/{rule}"), Some(json!({ "enabled": false, "reason": "Paused while the office reorganizes" }))).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_eq!(b["enabled"], false);

    w.record().await;
    assert_eq!(w.instances("").await.len(), 1, "a new record starts no instance while the rule is off");

    let (s, b) = send(&w.app, "POST", &format!("/api/v1/apps/{}/processes/{}/decide", w.slug, enc(&waiting)), OWNER, Some(json!({ "decision": "approve" }))).await;
    assert_eq!(s, StatusCode::OK, "the waiting instance is still decided: {b}");
    assert_eq!(w.instances("").await[0]["status"], "Completed");
}

// 2. Disable writes a new version with no proposal id, and one ledger entry.
#[tokio::test(flavor = "multi_thread")]
async fn disable_writes_one_new_version_with_no_proposal_and_one_ledger_entry() {
    let w = world().await;
    let rule = w.rule("owner").await;
    let before = w.ledger("ProcessDefinitionChanged").len();

    let (s, b) = w.admin("PATCH", &format!("/admin/processes/{rule}"), Some(json!({ "enabled": false }))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "a reason is required: {b}");
    let (s, _) = send(&w.app, "PATCH", &format!("/api/v1/admin/processes/{rule}"), "prof.curie@science.state.edu", Some(json!({ "enabled": false, "reason": "No" }))).await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    assert_eq!(w.ledger("ProcessDefinitionChanged").len(), before, "a refused call writes nothing");

    let (s, b) = w.admin("PATCH", &format!("/admin/processes/{rule}"), Some(json!({ "enabled": false, "reason": "Pause for the audit" }))).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_eq!(b["version"], 2);

    let id = rule.clone();
    let versions: Vec<(i32, Option<uuid::Uuid>, String)> = w.db.run(move |c| {
        c.query("SELECT version, proposal_id, changed_by FROM workflow_rule_versions WHERE rule_id = $1 ORDER BY version", &[&id]).unwrap().iter().map(|r| (r.get(0), r.get(1), r.get(2))).collect()
    });
    assert_eq!(versions.len(), 2);
    assert_eq!(versions[1].1, None, "no proposal");
    assert_eq!(versions[1].2, ADMIN);
    let entries = w.ledger("ProcessDefinitionChanged");
    assert_eq!(entries.len(), before + 1, "exactly one ledger entry");
    assert_eq!(entries.last().unwrap()["version"], 2);

    let (s, detail) = w.admin("GET", &format!("/admin/processes/{rule}"), None).await;
    assert_eq!(s, StatusCode::OK, "{detail}");
    assert_eq!(detail["rule"]["enabled"], false);
    assert_eq!(detail["versions"].as_array().unwrap().len(), 2);
    assert_eq!(detail["versions"][1]["sha256"].as_str().unwrap().len(), 64);
    assert!(detail["versions"][1]["summary"].as_str().unwrap().contains("Pause for the audit"));
    assert!(!detail["sentences"].as_array().unwrap().is_empty());
    let (s, v1) = w.admin("GET", &format!("/admin/processes/{rule}/versions/1"), None).await;
    assert_eq!(s, StatusCode::OK, "{v1}");
    assert_eq!(v1["rule"]["enabled"], true, "version 1 is as it was");
}

// 3. Reassign: a held user, the creator, a completed instance. After a reassign, `approvers` is the assignee alone.
#[tokio::test(flavor = "multi_thread")]
async fn reassign_refuses_a_held_user_the_creator_and_a_finished_instance() {
    let w = world().await;
    let rule = w.rule("owner").await;
    let record = w.record().await;
    let id = format!("{rule}:{record}");
    let url = |tail: &str| format!("/admin/process-instances/{}{tail}", enc(&id));

    let held = format!("{}@state.edu", unique("held"));
    let user = w.person(&held, Some("viewer")).await;
    let (s, b) = w.admin("POST", &format!("/admin/users/{}/hold", user["id"].as_str().unwrap()), Some(json!({ "hold": true, "reason": "Fixture" }))).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    let (s, b) = w.admin("POST", &url("/reassign"), Some(json!({ "eppn": held, "reason": "Cover" }))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "a held user: {b}");

    let (s, b) = w.admin("POST", &url("/reassign"), Some(json!({ "eppn": w.editor, "reason": "Cover" }))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "the creator: {b}");
    assert_eq!(b["code"], "separation_of_duties", "{b}");

    let stranger = format!("{}@state.edu", unique("stranger"));
    w.person(&stranger, None).await;
    let (s, b) = w.admin("POST", &url("/reassign"), Some(json!({ "eppn": stranger, "reason": "Cover" }))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "no read access to the app: {b}");

    let (s, b) = w.admin("POST", &url("/reassign"), Some(json!({ "eppn": "nobody@state.edu", "reason": "Cover" }))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "unknown person: {b}");

    let cover = format!("{}@state.edu", unique("cover"));
    w.person(&cover, Some("viewer")).await;
    let (s, b) = w.admin("POST", &url("/reassign"), Some(json!({ "eppn": cover }))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "a reason is required: {b}");
    assert!(w.ledger("ProcessInstanceReassigned").is_empty(), "refused calls write nothing");
    let (s, b) = w.admin("POST", &url("/reassign"), Some(json!({ "eppn": cover, "reason": "The owner is away" }))).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_eq!(w.ledger("ProcessInstanceReassigned").len(), 1);

    let (s, b) = w.admin("GET", &url("/approvers"), None).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    let who: Vec<&str> = b["approvers"].as_array().unwrap().iter().map(|a| a["eppn"].as_str().unwrap()).collect();
    assert_eq!(who, vec![cover.as_str()], "only the assignee");
    let (s, detail) = w.admin("GET", &url(""), None).await;
    assert_eq!(s, StatusCode::OK, "{detail}");
    assert!(detail["log"].as_array().unwrap().iter().any(|l| l.as_str().unwrap().contains(&cover)), "a log line names the assignee: {detail}");

    // Back to role-based.
    let (s, b) = w.admin("POST", &url("/reassign"), Some(json!({ "eppn": null, "reason": "Owner is back" }))).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    let (_, b) = w.admin("GET", &url("/approvers"), None).await;
    assert!(b["approvers"].as_array().unwrap().iter().any(|a| a["eppn"] == OWNER), "{b}");

    // A completed instance.
    let (s, _) = send(&w.app, "POST", &format!("/api/v1/apps/{}/processes/{}/decide", w.slug, enc(&id)), OWNER, Some(json!({ "decision": "approve" }))).await;
    assert_eq!(s, StatusCode::OK);
    let (s, b) = w.admin("POST", &url("/reassign"), Some(json!({ "eppn": cover, "reason": "Too late" }))).await;
    assert_eq!(s, StatusCode::CONFLICT, "{b}");
}

// 4. Cancel leaves the record alone, sets Failed, writes one ledger entry.
#[tokio::test(flavor = "multi_thread")]
async fn cancel_fails_the_instance_leaves_the_record_and_writes_one_ledger_entry() {
    let w = world().await;
    let rule = w.rule("owner").await;
    let record = w.record().await;
    let id = format!("{rule}:{record}");
    let read = || w.state.find_record(&w.slug, &record).unwrap().unwrap().data;
    let before = read();

    let (s, b) = w.admin("POST", &format!("/admin/process-instances/{}/cancel", enc(&id)), Some(json!({}))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "a reason is required: {b}");
    let (s, b) = w.admin("POST", &format!("/admin/process-instances/{}/cancel", enc(&id)), Some(json!({ "reason": "Raised in error" }))).await;
    assert_eq!(s, StatusCode::OK, "{b}");

    let rows = w.instances("").await;
    assert_eq!(rows[0]["status"], "Failed");
    let (_, detail) = w.admin("GET", &format!("/admin/process-instances/{}", enc(&id)), None).await;
    assert!(detail["log"].as_array().unwrap().iter().any(|l| l.as_str().unwrap() == format!("cancelled by {ADMIN}: Raised in error")), "{detail}");
    assert_eq!(read(), before, "the record is unchanged");
    assert_eq!(w.ledger("ProcessInstanceCancelled").len(), 1);

    let (s, b) = w.admin("POST", &format!("/admin/process-instances/{}/cancel", enc(&id)), Some(json!({ "reason": "Again" }))).await;
    assert_eq!(s, StatusCode::CONFLICT, "only a waiting instance: {b}");
    assert_eq!(w.ledger("ProcessInstanceCancelled").len(), 1);
}

// 5. `stale` is true at 15 days and false at 13, with the default setting.
#[tokio::test(flavor = "multi_thread")]
async fn stale_is_true_at_fifteen_days_and_false_at_thirteen() {
    let w = world().await;
    let rule = w.rule("owner").await;
    let (old, young) = (w.record().await, w.record().await);
    for (record, days) in [(&old, 15), (&young, 13)] {
        let (id, at) = (format!("{rule}:{record}"), (chrono::Utc::now() - chrono::Duration::days(days)).to_rfc3339());
        w.db.run(move |c| c.execute("UPDATE process_instances SET instance_json = jsonb_set(instance_json, '{started_at}', to_jsonb($2::text)) WHERE id = $1", &[&id, &at]).unwrap());
    }
    let rows = w.instances("").await;
    let of = |record: &String| rows.iter().find(|r| r["record_id"] == record.as_str()).unwrap().clone();
    assert_eq!(of(&old)["stale"], true);
    assert_eq!(of(&old)["age_days"], 15);
    assert_eq!(of(&young)["stale"], false);
    let stale = w.instances("&stale=true").await;
    assert_eq!(stale.len(), 1);
    assert_eq!(stale[0]["record_id"], old.as_str());
}

// 6. `unassigned=true` returns exactly the instances that have a no_approver problem.
#[tokio::test(flavor = "multi_thread")]
async fn unassigned_returns_exactly_the_instances_nobody_can_decide() {
    let w = world().await;
    let covered = w.rule("owner").await;
    let orphan = w.rule("registrar").await;
    let first = w.record().await;
    let rows = w.instances("&unassigned=true").await;
    let ids: Vec<&str> = rows.iter().map(|r| r["rule_id"].as_str().unwrap()).collect();
    assert_eq!(ids, vec![orphan.as_str()], "only the rule nobody can decide: {rows:?}");
    assert!(rows[0]["no_approver"].is_string());
    assert!(rows[0]["no_approver_reason"].as_str().unwrap().len() > 10, "said in words");
    let all = w.instances("").await;
    assert_eq!(all.len(), 2, "{first}");
    assert!(all.iter().any(|r| r["rule_id"] == covered.as_str() && r["no_approver"].is_null()));
    assert_eq!(w.instances("&assigned=true").await.len(), 0);
}

// 7. The admin simulate route and the app's simulate route return the same result.
#[tokio::test(flavor = "multi_thread")]
async fn admin_simulate_and_app_simulate_agree() {
    let w = world().await;
    let rule = w.rule("owner").await;
    let record = json!({ "id": "sample-1", "name": "Ada", "status": "New" });
    let (s, admin) = w.admin("POST", &format!("/admin/processes/{rule}/simulate"), Some(json!({ "record": record, "principal": OWNER }))).await;
    assert_eq!(s, StatusCode::OK, "{admin}");
    let (s, app) = send(&w.app, "POST", &format!("/api/v1/apps/{}/automations/simulate", w.slug), OWNER, Some(json!({ "event": "RecordCreated", "record": record }))).await;
    assert_eq!(s, StatusCode::OK, "{app}");
    let mut from_app = app.as_array().unwrap().iter().find(|r| r["rule_id"] == rule.as_str()).unwrap().clone();
    let mut from_admin = admin["result"].clone();
    for r in [&mut from_app, &mut from_admin] {
        r.as_object_mut().unwrap().remove("execution_timestamp");
    }
    assert_eq!(from_admin, from_app);
    assert_eq!(from_admin["trigger_matched"], true);
    assert!(from_admin["waiting_instance"].is_object(), "the step would wait: {from_admin}");

    let (s, b) = w.admin("POST", &format!("/admin/processes/{rule}/simulate"), Some(json!({ "record": record }))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "a principal is required: {b}");
}

// The inventory: a row per definition with its counts, filters, and paging.
#[tokio::test(flavor = "multi_thread")]
async fn the_inventory_lists_a_row_for_each_definition_with_its_waiting_count() {
    let w = world().await;
    let rule = w.rule("owner").await;
    let other = w.rule("registrar").await;
    w.record().await;
    let (s, b) = w.admin("GET", &format!("/admin/processes?app={}", w.slug), None).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    let rows = b["rows"].as_array().unwrap();
    assert_eq!(rows.len(), 2);
    let r = rows.iter().find(|r| r["rule_id"] == rule.as_str()).unwrap();
    assert_eq!(r["waiting_count"], 1);
    assert_eq!(r["version"], 1);
    assert_eq!(r["enabled"], true);
    assert_eq!(r["step_count"], 1);
    assert_eq!(r["user_task_roles"], json!(["owner"]));
    assert_eq!(r["workspace_id"], WS);
    assert_eq!(r["last_changed_by"], OWNER);
    assert!(r["trigger"].as_str().unwrap().contains("created"));
    w.admin("PATCH", &format!("/admin/processes/{other}"), Some(json!({ "enabled": false, "reason": "Off" }))).await;
    let (_, off) = w.admin("GET", &format!("/admin/processes?app={}&enabled=false", w.slug), None).await;
    assert_eq!(off["rows"].as_array().unwrap().len(), 1);
    let (_, page) = w.admin("GET", &format!("/admin/processes?app={}&limit=1", w.slug), None).await;
    assert_eq!(page["rows"].as_array().unwrap().len(), 1);
    assert!(page["next_cursor"].is_string());
    let (_, found) = w.admin("GET", &format!("/admin/processes?search={}", &rule[..9]), None).await;
    assert_eq!(found["rows"].as_array().unwrap().len(), 1);
}

// 8. Every route above is in ADMIN_ROUTES.
#[test]
fn every_process_route_is_listed_in_admin_routes() {
    for route in [
        ("GET", "/admin/processes"),
        ("GET", "/admin/processes/{rule_id}"),
        ("GET", "/admin/processes/{rule_id}/versions/{v}"),
        ("PATCH", "/admin/processes/{rule_id}"),
        ("POST", "/admin/processes/{rule_id}/simulate"),
        ("GET", "/admin/process-instances"),
        ("GET", "/admin/process-instances/{id}"),
        ("GET", "/admin/process-instances/{id}/approvers"),
        ("POST", "/admin/process-instances/{id}/reassign"),
        ("POST", "/admin/process-instances/{id}/cancel"),
    ] {
        assert!(ADMIN_ROUTES.contains(&route), "{route:?} is in ADMIN_ROUTES");
    }
}
