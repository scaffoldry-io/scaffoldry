//! Access rules phase 6: view as another user.
//!
//! Ann is an editor. Olga is an admin. A row rule hides `secret` rows from everyone but editors, so
//! Ann alone sees them. Olga viewing as Ann must see what both of them could: no secret rows.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::Router;
use http_body_util::BodyExt;
use scaffoldry_core::access::{Op, Operand, Pred, RowOp, RowRule};
use scaffoldry_engine::{AppManifest, AppTable, FieldSpec, FieldType, LinkCardinality};
use scaffoldry_server::build_app_with_state;
use scaffoldry_server::service::identity::issue_test_token_and_user;
use scaffoldry_server::service::scope::set_rules;
use scaffoldry_server::state::{CollaboratorRecord, ServerState};
use serde_json::{json, Value};
use std::sync::Arc;
use tower::ServiceExt;
use uuid::Uuid;

const WORKSPACE: &str = "ws-bio-lab";
const ADMIN: &str = "jordan.lee@state.edu";

fn db_url() -> String {
    std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://scaffoldry:scaffoldry_dev_password@127.0.0.1:5433/scaffoldry".to_string())
}

struct Cleanup(String);
impl Drop for Cleanup {
    fn drop(&mut self) {
        let (tag, url) = (self.0.clone(), db_url());
        let _ = std::thread::spawn(move || {
            if let Ok(mut c) = postgres::Client::connect(&url, postgres::NoTls) {
                let like = format!("%{tag}%");
                let _ = c.execute("DELETE FROM workspace_collaborators WHERE workspace_id = $1 AND eppn LIKE $2", &[&WORKSPACE, &like]);
                let _ = c.execute("DELETE FROM record_links WHERE app_slug LIKE $1", &[&like]);
                let _ = c.execute("DELETE FROM record_history WHERE app_slug LIKE $1", &[&like]);
                let _ = c.execute("DELETE FROM dataset_records WHERE app_slug LIKE $1", &[&like]);
            }
        })
        .join();
    }
}

fn text(name: &str) -> FieldSpec {
    FieldSpec::simple(name, name, FieldType::Text, false, false)
}

fn table(id: &str, fields: Vec<FieldSpec>, primary: &str) -> AppTable {
    AppTable {
        id: id.into(),
        name: id.into(),
        slug: id.into(),
        description: None,
        icon: None,
        fields,
        primary_field: Some(primary.into()),
        sample_records: vec![],
    }
}

fn manifest(slug: &str) -> AppManifest {
    let mut award = FieldSpec::simple("award", "award", FieldType::Link, false, false);
    award.link_target_table = Some("awards".into());
    award.link_cardinality = LinkCardinality::One;
    let mut award_amount = FieldSpec::simple("award_amount", "award_amount", FieldType::Lookup, false, false);
    award_amount.computed_via = Some("award".into());
    award_amount.target_display_field = Some("amount".into());
    AppManifest {
        slug: slug.into(),
        title: "Columns".into(),
        description: String::new(),
        organization_code: "BIO".into(),
        department: "biology".into(),
        workspace_id: Some(WORKSPACE.into()),
        herm_capability_id: None,
        custom_domain: None,
        custom_domain_verified: false,
        tables: vec![
            table("students", vec![text("name"), FieldSpec::simple("gpa", "gpa", FieldType::Number, false, false), award, award_amount], "name"),
            table("awards", vec![text("name"), FieldSpec::simple("amount", "amount", FieldType::Number, false, false)], "name"),
        ],
        relationships: vec![],
        views: vec![],
        ceds_mappings: Default::default(),
        pages: vec![],
        access: Default::default(),
    }
}


fn rules() -> Vec<RowRule> {
    vec![RowRule {
        id: "no-secrets".into(),
        table_id: "students".into(),
        ops: vec![RowOp::Read],
        visible_when: Pred::Cmp { field: "name".into(), op: Op::Ne, value: Operand::Literal(json!("secret")) },
        except_roles: vec!["editor".into()],
        message: None,
    }]
}

struct World {
    app: Router,
    slug: String,
    other_slug: String,
    ann_eppn: String,
    ann: String,
    olga: String,
    admin: String,
    _cleanup: Cleanup,
}

fn member(state: &ServerState, eppn: &str, role: &str) {
    state.collaborators.write().unwrap().entry(WORKSPACE.to_string()).or_default().push(CollaboratorRecord {
        id: Uuid::new_v4().to_string(),
        workspace_id: WORKSPACE.to_string(),
        eppn: eppn.to_string(),
        name: eppn.to_string(),
        role: role.to_string(),
        scoped_affiliation: "faculty".to_string(),
        department: "biology".to_string(),
        added_at: String::new(),
    });
}

async fn world() -> World {
    let state = Arc::new(ServerState::new().expect("state"));
    state.seed_demo().expect("seed");
    let tag = Uuid::new_v4().to_string()[..8].to_string();
    let (slug, other_slug) = (format!("va-{tag}"), format!("vb-{tag}"));
    let (ann, olga) = (format!("ann-{tag}@state.edu"), format!("olga-{tag}@state.edu"));
    state.engine.write().unwrap().register_manifest(manifest(&slug)).unwrap();
    state.engine.write().unwrap().register_manifest(manifest(&other_slug)).unwrap();
    member(&state, &ann, "editor");
    member(&state, &olga, "admin");
    set_rules(&slug, rules(), &state);
    World {
        app: build_app_with_state(state).expect("router"),
        ann_eppn: ann.clone(),
        ann: issue_test_token_and_user(&ann),
        olga: issue_test_token_and_user(&olga),
        admin: issue_test_token_and_user(ADMIN),
        _cleanup: Cleanup(tag),
        slug,
        other_slug,
    }
}

async fn send(app: &Router, method: &str, uri: &str, token: &str, body: Option<Value>) -> (StatusCode, Value) {
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

async fn create(w: &World, slug: &str, name: &str) -> String {
    let data = json!({ "_table_id": "students", "name": name, "gpa": 3.5 });
    let (s, body) = send(&w.app, "POST", &format!("/api/v1/apps/{slug}/records"), &w.admin, Some(json!({ "data": data }))).await;
    assert_eq!(s, StatusCode::CREATED, "{body}");
    body["id"].as_str().unwrap().to_string()
}

async fn names(w: &World, slug: &str, token: &str) -> Vec<String> {
    let (s, body) = send(&w.app, "GET", &format!("/api/v1/apps/{slug}/tables/students/records"), token, None).await;
    assert_eq!(s, StatusCode::OK, "{body}");
    let mut out: Vec<String> = body["records"].as_array().unwrap().iter().map(|r| r["data"]["name"].as_str().unwrap().to_string()).collect();
    out.sort();
    out
}

/// Olga views `slug` as Ann. Returns the view-as token.
async fn view_as(w: &World, caller: &str, slug: &str, target_eppn: &str) -> (StatusCode, Value) {
    send(&w.app, "POST", &format!("/api/v1/apps/{slug}/view-as"), caller, Some(json!({ "eppn": target_eppn }))).await
}

fn target(w: &World) -> String {
    w.ann_eppn.clone()
}

// 1. A view-as read returns what a direct read as the target returns, intersected with the caller's
//    own. Ann sees the secret row directly. Olga cannot, so viewing as Ann does not show it either.
#[tokio::test]
async fn a_view_as_read_is_the_target_intersected_with_the_caller() {
    let w = world().await;
    create(&w, &w.slug, "plain").await;
    create(&w, &w.slug, "secret").await;
    assert_eq!(names(&w, &w.slug, &w.ann).await, vec!["plain", "secret"], "Ann reads both");
    assert_eq!(names(&w, &w.slug, &w.olga).await, vec!["plain"], "Olga reads one");

    let (s, body) = view_as(&w, &w.olga, &w.slug, &target(&w)).await;
    assert_eq!(s, StatusCode::OK, "{body}");
    let token = body["token"].as_str().unwrap().to_string();
    assert_eq!(names(&w, &w.slug, &token).await, vec!["plain"], "never more than the caller could see");
}

// 2. A write is view_as_read_only, on a route and on a tool.
#[tokio::test]
async fn a_write_in_view_as_is_refused() {
    let w = world().await;
    let id = create(&w, &w.slug, "plain").await;
    let (_, body) = view_as(&w, &w.olga, &w.slug, &target(&w)).await;
    let token = body["token"].as_str().unwrap().to_string();
    let base = format!("/api/v1/apps/{}", w.slug);

    let data = json!({ "_table_id": "students", "name": "new" });
    let (s, err) = send(&w.app, "POST", &format!("{base}/records"), &token, Some(json!({ "data": data }))).await;
    assert_eq!((s, err["code"].as_str()), (StatusCode::FORBIDDEN, Some("view_as_read_only")), "{err}");
    let (s, err) = send(&w.app, "DELETE", &format!("{base}/records/{id}"), &token, None).await;
    assert_eq!((s, err["code"].as_str()), (StatusCode::FORBIDDEN, Some("view_as_read_only")), "{err}");

    let payload = json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": { "name": "create_record", "arguments": { "app_slug": w.slug, "table_id": "students", "data": { "name": "x" } } } });
    let (_, rpc) = send(&w.app, "POST", "/api/mcp", &token, Some(payload)).await;
    assert_eq!(rpc["result"]["structuredContent"]["code"], "view_as_read_only", "{rpc}");
    assert_eq!(names(&w, &w.slug, &w.admin).await.len(), 1, "nothing was written");
}

// 3. The token fails on another app.
#[tokio::test]
async fn the_token_does_not_reach_another_app() {
    let w = world().await;
    create(&w, &w.other_slug, "elsewhere").await;
    let (_, body) = view_as(&w, &w.olga, &w.slug, &target(&w)).await;
    let token = body["token"].as_str().unwrap().to_string();
    let (s, _) = send(&w.app, "GET", &format!("/api/v1/apps/{}/tables/students/records", w.other_slug), &token, None).await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    let (s, _) = send(&w.app, "GET", &format!("/api/v1/apps/{}/tables/students/records", w.slug), &token, None).await;
    assert_eq!(s, StatusCode::OK);
}

// 4. It expires at 30 minutes. Exit revokes it.
#[tokio::test]
async fn the_token_lasts_thirty_minutes_and_exit_revokes_it() {
    let w = world().await;
    let (_, body) = view_as(&w, &w.olga, &w.slug, &target(&w)).await;
    let token = body["token"].as_str().unwrap().to_string();
    let expires = chrono::DateTime::parse_from_rfc3339(body["expires_at"].as_str().unwrap()).unwrap();
    let minutes = (expires.with_timezone(&chrono::Utc) - chrono::Utc::now()).num_minutes();
    assert!((29..=30).contains(&minutes), "expires in {minutes} minutes");

    let list = format!("/api/v1/apps/{}/tables/students/records", w.slug);
    // Expiry: move the stored expiry into the past.
    {
        let slug = w.slug.clone();
        let n = std::thread::spawn(move || {
            let mut c = postgres::Client::connect(&db_url(), postgres::NoTls).unwrap();
            c.execute("UPDATE api_tokens SET expires_at = NOW() - interval '1 second' WHERE kind = 'viewas' AND app_slug = $1", &[&slug]).unwrap()
        })
        .join()
        .unwrap();
        assert_eq!(n, 1, "one token stored");
    }
    let (s, _) = send(&w.app, "GET", &list, &token, None).await;
    assert_eq!(s, StatusCode::UNAUTHORIZED, "expired");

    // Exit: a fresh one, then stop.
    let (_, body) = view_as(&w, &w.olga, &w.slug, &target(&w)).await;
    let token = body["token"].as_str().unwrap().to_string();
    let (s, _) = send(&w.app, "GET", &list, &token, None).await;
    assert_eq!(s, StatusCode::OK);
    let (s, out) = send(&w.app, "POST", "/api/v1/auth/stop-view-as", &token, None).await;
    assert_eq!(s, StatusCode::OK, "{out}");
    let (s, _) = send(&w.app, "GET", &list, &token, None).await;
    assert_eq!(s, StatusCode::UNAUTHORIZED, "revoked");
}

// 5. An editor cannot start view-as. An app admin can.
#[tokio::test]
async fn only_an_owner_or_admin_may_start_view_as() {
    let w = world().await;
    let (s, _) = view_as(&w, &w.ann, &w.slug, &target(&w)).await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    let (s, _) = view_as(&w, &w.olga, &w.slug, &target(&w)).await;
    assert_eq!(s, StatusCode::OK);
    let (s, _) = view_as(&w, &w.admin, &w.slug, &target(&w)).await;
    assert_eq!(s, StatusCode::OK, "a Platform Admin may too");
}

// 6. Each start is a ledger entry.
#[tokio::test]
async fn each_start_is_a_ledger_entry() {
    let w = world().await;
    let count = |body: &Value| body["entries"].as_array().unwrap().iter().filter(|e| e["decision_type"] == "ViewAsStarted").count();
    let (_, before) = send(&w.app, "GET", "/api/v1/governance/ledger", &w.admin, None).await;
    view_as(&w, &w.olga, &w.slug, &target(&w)).await;
    view_as(&w, &w.olga, &w.slug, &target(&w)).await;
    let (_, after) = send(&w.app, "GET", "/api/v1/governance/ledger", &w.admin, None).await;
    assert_eq!(count(&after), count(&before) + 2);
    let last = after["entries"].as_array().unwrap().last().unwrap();
    assert_eq!(last["decision_type"], "ViewAsStarted");
    assert_eq!(last["app_slug"], w.slug.as_str());
}
