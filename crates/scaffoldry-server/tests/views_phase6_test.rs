//! Views phase 6: shared links.
//!
//! A share is a link to one view. It is off until a Platform Admin enables a scope. A shared view is read-only and never
//! carries a sensitive or a hidden field. A public share is for a workspace classified Public, and the viewer has no person
//! attributes. Each request counts against a rate limit.

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::Router;
use http_body_util::BodyExt;
use scaffoldry_core::access::{ColumnMode, ColumnRule, Op, Operand, PersonAttr, Pred, RowOp, RowRule, Who};
use scaffoldry_engine::{AppManifest, AppPage, AppTable, FieldSpec, FieldType, PageKind, SelectOption, SelectOptionColor};
use scaffoldry_server::build_app_with_state;
use scaffoldry_server::service::identity::issue_test_token_and_user;
use scaffoldry_server::service::scope::{set_column_rules, set_rules};
use scaffoldry_server::state::{CollaboratorRecord, ServerState};
use serde_json::{json, Value};
use std::sync::Arc;
use tower::ServiceExt;
use uuid::Uuid;

const WORKSPACE: &str = "ws-bio-lab";
const PLATFORM: &str = "jordan.lee@state.edu";

fn db_url() -> String {
    scaffoldry_server::testing::database_url()
}

fn db<R: Send + 'static>(f: impl FnOnce(&mut postgres::Client) -> R + Send + 'static) -> R {
    let url = db_url();
    std::thread::spawn(move || {
        let mut c = postgres::Client::connect(&url, postgres::NoTls).expect("database");
        f(&mut c)
    })
    .join()
    .expect("database thread")
}

struct Cleanup(String);
impl Drop for Cleanup {
    fn drop(&mut self) {
        let (tag, url) = (self.0.clone(), db_url());
        let _ = std::thread::spawn(move || {
            if let Ok(mut c) = postgres::Client::connect(&url, postgres::NoTls) {
                let like = format!("%{tag}%");
                let _ = c.execute("DELETE FROM jobs WHERE payload->>'app_slug' LIKE $1", &[&like]);
                let _ = c.execute("DELETE FROM index_status WHERE app_slug LIKE $1", &[&like]);
                let _ = c.execute("DELETE FROM record_index WHERE app_slug LIKE $1", &[&like]);
                let _ = c.execute("DELETE FROM app_views WHERE app_slug LIKE $1", &[&like]);
                let _ = c.execute("DELETE FROM view_shares WHERE app_slug LIKE $1", &[&like]);
                let _ = c.execute("DELETE FROM workspace_collaborators WHERE workspace_id = $1 AND eppn LIKE $2", &[&WORKSPACE, &like]);
                let _ = c.execute("DELETE FROM record_history WHERE app_slug LIKE $1", &[&like]);
                let _ = c.execute("DELETE FROM dataset_records WHERE app_slug LIKE $1", &[&like]);
                let _ = c.execute("DELETE FROM app_manifests WHERE slug LIKE $1", &[&like]);
            }
        })
        .join();
    }
}

fn select(name: &str, options: &[&str]) -> FieldSpec {
    let mut f = FieldSpec::simple(name, name, FieldType::Select, false, false);
    f.select_options = options
        .iter()
        .enumerate()
        .map(|(i, id)| SelectOption { id: id.to_string(), label: id.to_string(), color: SelectOptionColor::Neutral, order: i as i32 })
        .collect();
    f
}

/// A field the table indexes. The test fills its index rows by hand, so no build job has to run.
fn indexed(mut f: FieldSpec) -> FieldSpec {
    f.indexed = true;
    f
}

/// Students with a program, an advisor, a stage and a gpa. With `many_indexed`, twenty more fields are
/// marked indexed and a twenty-first is not.
fn manifest(slug: &str, many_indexed: bool) -> AppManifest {
    let mut fields = vec![
        FieldSpec::simple("name", "Name", FieldType::Text, false, false),
        indexed(select("program", &["p0", "p1", "p2", "p3", "p4"])),
        indexed(FieldSpec::simple("advisor", "Advisor", FieldType::Text, false, false)),
        select("stage", &["opt_a", "opt_b"]),
        {
            let mut gpa = FieldSpec::simple("gpa", "GPA", FieldType::Number, false, false);
            gpa.ferpa_sensitive = true;
            gpa
        },
    ];
    if many_indexed {
        for i in 1..=20 {
            let mut f = FieldSpec::simple(format!("kept{i:02}"), format!("kept{i:02}"), FieldType::Text, false, false);
            f.indexed = true;
            fields.push(f);
        }
        fields.push(FieldSpec::simple("late", "Late", FieldType::Text, false, false));
    }
    AppManifest {
        slug: slug.to_string(),
        title: "Registrar".to_string(),
        description: String::new(),
        organization_code: "BIO".to_string(),
        department: "biology".to_string(),
        workspace_id: Some(WORKSPACE.to_string()),
        herm_capability_id: None,
        coordinates: None, sealed: false, integrations: Default::default(), kind: Default::default(),
        custom_domain: None,
        custom_domain_verified: false,
        tables: vec![AppTable {
            retired_fields: Vec::new(),
            id: "students".to_string(),
            name: "students".to_string(),
            slug: "students".to_string(),
            description: None,
            icon: None,
            fields,
            primary_field: Some("name".to_string()),
            sample_records: vec![],
            coordinates: None, sealed: false,
            row_place: Default::default(),
        }],
        relationships: vec![],
        views: vec![],
        ceds_mappings: Default::default(),
        pages: vec![AppPage { id: "board".to_string(), title: "Board".to_string(), kind: PageKind::Custom { source_sha256: "0".repeat(64), grants: vec![], format: Default::default(), dependencies: vec![] } }],
        access: Default::default(),
    }
}

fn collaborator(state: &ServerState, eppn: &str, role: &str) {
    state.collaborators.write().unwrap().entry(WORKSPACE.to_string()).or_default().push(CollaboratorRecord {
        id: Uuid::new_v4().to_string(),
        workspace_id: WORKSPACE.to_string(),
        eppn: eppn.to_string(),
        name: eppn.to_string(),
        role: role.to_string(),
        scoped_affiliation: "faculty".to_string(),
        department: "biology".to_string(),
        added_at: String::new(),
        app_slug: String::new(),
    });
}

struct World {
    app: Router,
    state: Arc<ServerState>,
    slug: String,
    platform: String,
    admin: String,
    viewer: String,
    _cleanup: Cleanup,
}

/// `rows` students. Every tenth is advised by the editor, every tenth plus one by the viewer. A row rule
/// shows each person the students they advise, except to an admin or owner, and a column rule hides
/// `stage` from everyone but an admin or owner.
async fn world(rows: i64, many_indexed: bool) -> World {
    let with_rules = true;
    let tag = Uuid::new_v4().to_string()[..8].to_string();
    let slug = format!("av-{tag}");
    let (admin_e, editor_e, viewer_e, viewer2_e) =
        (format!("admin-{tag}@state.edu"), format!("editor-{tag}@state.edu"), format!("viewer-{tag}@state.edu"), format!("viewer2-{tag}@state.edu"));
    let state = Arc::new(ServerState::new().expect("state"));
    state.seed_demo().expect("seed");
    state.engine.write().unwrap().register_manifest(manifest(&slug, many_indexed)).unwrap();
    collaborator(&state, &admin_e, "admin");
    collaborator(&state, &editor_e, "editor");
    collaborator(&state, &viewer_e, "viewer");
    collaborator(&state, &viewer2_e, "viewer");

    let staff = || Who { roles: vec!["admin".into(), "owner".into()], ..Default::default() };
    if with_rules {
        set_rules(
            &slug,
            vec![RowRule {
                id: "advisees".into(),
                table_id: "students".into(),
                ops: vec![RowOp::Read],
                visible_when: Pred::Cmp { field: "advisor".into(), op: Op::Eq, value: Operand::Person(PersonAttr::Address) },
                except_roles: vec!["admin".into(), "owner".into()],
                message: None,
            }],
            &state,
        );
        set_column_rules(
            &slug,
            vec![ColumnRule { id: "stage".into(), table_id: "students".into(), fields: vec!["stage".into()], mode: ColumnMode::Hidden, unless: staff(), message: None }],
            &state,
        );
    }

    let (s, ed, vw) = (slug.clone(), editor_e.clone(), viewer_e.clone());
    db(move |c| {
        c.execute(
            "INSERT INTO dataset_records (id, app_slug, data, ceds_mapping, is_ferpa_sensitive, table_id, created_at) \
             SELECT $1::text || lpad(g::text, 8, '0'), $1::text, \
                    jsonb_build_object('_table_id', 'students', 'name', 'student-' || g, 'program', 'p' || (g % 5), \
                                       'advisor', CASE WHEN g % 10 = 0 THEN $2::text WHEN g % 10 = 1 THEN $3::text ELSE 'adv' || g || '@state.edu' END, \
                                       'stage', CASE WHEN g % 2 = 0 THEN 'opt_a' ELSE 'opt_b' END, 'gpa', 3.0 + (g % 10) / 10.0), \
                    '{}'::jsonb, false, 'students', TIMESTAMPTZ '2026-01-01 00:00:00+00' + g * interval '1 millisecond' \
             FROM generate_series(1, $4::bigint) g",
            &[&s, &ed, &vw, &rows],
        )
        .unwrap();
        for field in ["program", "advisor"] {
            c.execute(
                &format!("INSERT INTO record_index (app_slug, table_id, field, record_id, txt) SELECT app_slug, 'students', '{field}', id, lower(data->>'{field}') FROM dataset_records WHERE app_slug = $1"),
                &[&s],
            )
            .unwrap();
        }
        c.batch_execute("ANALYZE dataset_records").unwrap();
        c.batch_execute("ANALYZE record_index").unwrap();
    });

    World {
        app: build_app_with_state(state.clone()).expect("router"),
        state,
        platform: issue_test_token_and_user(PLATFORM),
        admin: issue_test_token_and_user(&admin_e),
        viewer: issue_test_token_and_user(&viewer_e),
        slug,
        _cleanup: Cleanup(tag),
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


async fn send_plain(app: &Router, uri: &str, bearer: Option<&str>, address: Option<&str>) -> (StatusCode, Value, String) {
    let mut b = Request::builder().method("GET").uri(uri);
    if let Some(t) = bearer {
        b = b.header("authorization", format!("Bearer {t}"));
    }
    if let Some(a) = address {
        b = b.header("x-forwarded-for", a);
    }
    let resp = app.clone().oneshot(b.body(Body::empty()).unwrap()).await.unwrap();
    let status = resp.status();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let text = String::from_utf8_lossy(&bytes).to_string();
    (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null), text)
}

fn views_path(w: &World) -> String {
    format!("/api/v1/apps/{}/views", w.slug)
}

async fn setting(w: &World, scopes: Value) {
    let (s, v) = send(&w.app, "PUT", "/api/v1/settings/sharing.allowed_scopes", &w.platform, Some(scopes)).await;
    assert_eq!(s, StatusCode::OK, "{v}");
}

fn classify(w: &World, class: &str) {
    w.state.workspaces.write().unwrap().get_mut(WORKSPACE).expect("the workspace").data_classification = class.to_string();
}

async fn a_view(w: &World) -> String {
    let (s, v) = send(&w.app, "POST", &views_path(w), &w.admin, Some(json!({ "name": "All students", "kind": "grid", "ownership": "collaborative", "table_id": "students", "config": {} }))).await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    v["id"].as_str().or_else(|| v["view"]["id"].as_str()).unwrap().to_string()
}

async fn share(w: &World, token: &str, view: &str, body: Value) -> (StatusCode, Value) {
    send(&w.app, "POST", &format!("{}/{view}/shares", views_path(w)), token, Some(body)).await
}

fn token_of(v: &Value) -> String {
    v["token"].as_str().unwrap_or_else(|| panic!("a token in {v}")).to_string()
}

fn fields_in(v: &Value) -> std::collections::BTreeSet<String> {
    v["records"].as_array().unwrap().iter().flat_map(|r| r["data"].as_object().unwrap().keys().cloned()).collect()
}

// 1. With no scope enabled, creating a share is share_not_allowed. With institution enabled it works.
#[tokio::test]
async fn a_share_needs_an_enabled_scope() {
    let _db = common::TestDb::new();
    let w = world(10, false).await;
    let view = a_view(&w).await;
    let (s, v) = share(&w, &w.admin, &view, json!({ "scope": "institution" })).await;
    assert_eq!((s, v["code"].as_str()), (StatusCode::FORBIDDEN, Some("share_not_allowed")), "{v}");

    setting(&w, json!(["institution"])).await;
    let (s, v) = share(&w, &w.admin, &view, json!({ "scope": "institution" })).await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    assert_eq!(v["token"].as_str().unwrap().len(), 32, "128 random bits, in hex: {v}");
    assert_eq!(v["link"], json!(format!("/s/{}", v["token"].as_str().unwrap())));
    assert_eq!(v["share"]["scope"], "institution");
    assert!(v["share"].get("token").is_none() && v["share"].get("token_hash").is_none(), "the list never holds the token: {v}");
    // The default is 30 days. Past 90, or none, is a bad request. A scope that is not one is refused.
    let days = |v: &Value| chrono::DateTime::parse_from_rfc3339(v["share"]["expires_at"].as_str().unwrap()).unwrap().signed_duration_since(chrono::Utc::now()).num_days();
    assert!((29..=30).contains(&days(&v)), "{v}");
    for body in [json!({ "scope": "institution", "expires_in_days": 91 }), json!({ "scope": "institution", "expires_in_days": 0 }), json!({ "scope": "everyone" })] {
        assert_eq!(share(&w, &w.admin, &view, body.clone()).await.0, StatusCode::BAD_REQUEST, "{body}");
    }
    // A setting that is not a list of the two scopes is refused.
    let (s, _) = send(&w.app, "PUT", "/api/v1/settings/sharing.allowed_scopes", &w.platform, Some(json!(["world"]))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    // Only an owner or an admin of the app may share.
    let (s, v) = share(&w, &w.viewer, &view, json!({ "scope": "institution" })).await;
    assert_eq!(s, StatusCode::FORBIDDEN, "{v}");
}

// 2. public on a workspace that is not Public is share_not_allowed.
#[tokio::test]
async fn a_public_share_needs_a_public_workspace() {
    let _db = common::TestDb::new();
    let w = world(10, false).await;
    let view = a_view(&w).await;
    setting(&w, json!(["institution", "public"])).await;
    classify(&w, "Internal");
    let (s, v) = share(&w, &w.admin, &view, json!({ "scope": "public" })).await;
    assert_eq!((s, v["code"].as_str()), (StatusCode::FORBIDDEN, Some("share_not_allowed")), "{v}");
    assert!(v["error"].as_str().unwrap().contains("Public"), "it says why: {v}");
    classify(&w, "Public");
    let (s, v) = share(&w, &w.admin, &view, json!({ "scope": "public" })).await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    // Public is not on unless the Platform Admin enabled it.
    setting(&w, json!(["institution"])).await;
    let (s, v) = share(&w, &w.admin, &view, json!({ "scope": "public" })).await;
    assert_eq!((s, v["code"].as_str()), (StatusCode::FORBIDDEN, Some("share_not_allowed")), "{v}");
}

// 3. A shared view's response has no sensitive or hidden field, for any share and any person.
#[tokio::test]
async fn a_shared_view_never_carries_a_sensitive_or_hidden_field() {
    let _db = common::TestDb::new();
    let w = world(40, false).await;
    let view = a_view(&w).await;
    setting(&w, json!(["institution", "public"])).await;
    classify(&w, "Public");
    // Without sharing, the admin reads every field, the sensitive `gpa` and the column-ruled `stage` among them.
    let (_, plain) = send(&w.app, "GET", &format!("{}/{view}/data?limit=5", views_path(&w)), &w.admin, None).await;
    let plain_fields = fields_in(&plain);
    assert!(plain_fields.contains("gpa") && plain_fields.contains("stage"), "{plain_fields:?}");

    let (_, inst) = share(&w, &w.admin, &view, json!({ "scope": "institution" })).await;
    let (_, public) = share(&w, &w.admin, &view, json!({ "scope": "public" })).await;
    // A sensitive field is left out for everyone. A field the person's own rules hide is left out for them: the column rule
    // hides `stage` from everyone but staff, so the admin reads it as themself and no one else does.
    for (who, token, bearer, stage) in [("admin", token_of(&inst), Some(w.admin.as_str()), true), ("viewer", token_of(&inst), Some(w.viewer.as_str()), false), ("anyone", token_of(&public), None, false)] {
        let (s, body, text) = send_plain(&w.app, &format!("/api/v1/shared/{token}/data?limit=100"), bearer, None).await;
        assert_eq!(s, StatusCode::OK, "{who}: {body}");
        let fields = fields_in(&body);
        assert!(!fields.contains("gpa"), "{who}: a sensitive field never travels: {fields:?}");
        assert_eq!(fields.contains("stage"), stage && !body["records"].as_array().unwrap().is_empty(), "{who}: {fields:?}");
        assert!(stage || (!text.contains("opt_a") && !text.contains("opt_b")), "{who}: no hidden option leaks: {text}");
    }
    // The admin, as an institution viewer, sees every row (a staff role skips the row rule) but never the two fields.
    let (_, as_admin, _) = send_plain(&w.app, &format!("/api/v1/shared/{}/data?limit=100", token_of(&inst)), Some(&w.admin), None).await;
    assert_eq!(as_admin["records"].as_array().unwrap().len(), 40);
}

// 4. A public share of a table with a person-dependent row rule returns no rows, and the creator was warned.
#[tokio::test]
async fn a_public_share_of_a_person_dependent_table_shows_no_rows_and_warns_the_creator() {
    let _db = common::TestDb::new();
    let w = world(40, false).await;
    let view = a_view(&w).await;
    setting(&w, json!(["institution", "public"])).await;
    classify(&w, "Public");
    let (s, public) = share(&w, &w.admin, &view, json!({ "scope": "public" })).await;
    assert_eq!(s, StatusCode::CREATED, "{public}");
    let warnings = public["warnings"].as_array().unwrap();
    assert!(warnings.iter().any(|m| m.as_str().unwrap().contains("no rows")), "the creator is told: {public}");
    let (s, body, _) = send_plain(&w.app, &format!("/api/v1/shared/{}/data", token_of(&public)), None, None).await;
    assert_eq!(s, StatusCode::OK, "{body}");
    assert_eq!(body["records"].as_array().unwrap().len(), 0, "{body}");
    // The same view as an institution share is read as the person: the viewer's four students.
    let (_, inst) = share(&w, &w.admin, &view, json!({ "scope": "institution" })).await;
    assert!(inst["warnings"].as_array().is_none_or(Vec::is_empty), "an institution share has no such warning: {inst}");
    let (_, mine, _) = send_plain(&w.app, &format!("/api/v1/shared/{}/data?limit=100", token_of(&inst)), Some(&w.viewer), None).await;
    assert_eq!(mine["records"].as_array().unwrap().len(), 4, "{mine}");
    // An institution link needs someone signed in.
    let (s, _, _) = send_plain(&w.app, &format!("/api/v1/shared/{}/data", token_of(&inst)), None, None).await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);
}

// 5. A revoked or expired token is 404. The token is stored only as a hash.
#[tokio::test]
async fn a_revoked_or_expired_token_is_not_found_and_only_its_hash_is_stored() {
    let _db = common::TestDb::new();
    let w = world(10, false).await;
    let view = a_view(&w).await;
    setting(&w, json!(["institution"])).await;
    let (_, a) = share(&w, &w.admin, &view, json!({ "scope": "institution" })).await;
    let (_, b) = share(&w, &w.admin, &view, json!({ "scope": "institution" })).await;
    let (ta, tb) = (token_of(&a), token_of(&b));
    for t in [&ta, &tb] {
        assert_eq!(send_plain(&w.app, &format!("/api/v1/shared/{t}/data"), Some(&w.admin), None).await.0, StatusCode::OK);
    }

    let tokens = (ta.clone(), tb.clone());
    let stored = db(move |c| {
        let any: i64 = c.query_one("SELECT count(*) FROM view_shares WHERE token_hash IN ($1, $2)", &[&tokens.0, &tokens.1]).unwrap().get(0);
        let hashed: i64 = c.query_one("SELECT count(*) FROM view_shares WHERE token_hash IN (encode(sha256($1::bytea), 'hex'), encode(sha256($2::bytea), 'hex'))", &[&tokens.0.as_bytes(), &tokens.1.as_bytes()]).unwrap().get(0);
        let leaked: i64 = c.query_one("SELECT count(*) FROM view_shares v WHERE v::text LIKE '%' || $1 || '%'", &[&tokens.0]).unwrap().get(0);
        (any, hashed, leaked)
    });
    assert_eq!(stored, (0, 2, 0), "the hash is stored, never the token");

    // Revoke one through the route. Expire the other in the table.
    let (s, list) = send(&w.app, "GET", &format!("{}/{view}/shares", views_path(&w)), &w.admin, None).await;
    assert_eq!(s, StatusCode::OK, "{list}");
    let id_a = a["share"]["id"].as_str().unwrap().to_string();
    let (s, v) = send(&w.app, "DELETE", &format!("{}/{view}/shares/{id_a}", views_path(&w)), &w.admin, None).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let id_b: uuid::Uuid = b["share"]["id"].as_str().unwrap().parse().unwrap();
    db(move |c| c.execute("UPDATE view_shares SET expires_at = NOW() - INTERVAL '1 minute' WHERE id = $1", &[&id_b]).unwrap());
    for t in [&ta, &tb] {
        assert_eq!(send_plain(&w.app, &format!("/api/v1/shared/{t}/data"), Some(&w.admin), None).await.0, StatusCode::NOT_FOUND);
    }
    // A made-up token is 404 too, so a guess learns nothing.
    assert_eq!(send_plain(&w.app, "/api/v1/shared/00000000000000000000000000000000/data", Some(&w.admin), None).await.0, StatusCode::NOT_FOUND);
}

// 6. The 61st request in a minute is 429. A public link also counts 20 a minute for each address.
#[tokio::test]
async fn the_sixty_first_request_in_a_minute_is_rate_limited() {
    let _db = common::TestDb::new();
    let w = world(10, false).await;
    let view = a_view(&w).await;
    setting(&w, json!(["institution", "public"])).await;
    classify(&w, "Public");
    let (_, inst) = share(&w, &w.admin, &view, json!({ "scope": "institution" })).await;
    let (_, public) = share(&w, &w.admin, &view, json!({ "scope": "public" })).await;
    // The limits count in whole minutes. Start just after one begins, so the run does not straddle two.
    let into = chrono::Utc::now().timestamp() % 60;
    if into >= 50 {
        std::thread::sleep(std::time::Duration::from_secs((61 - into) as u64));
    }
    let uri = format!("/api/v1/shared/{}/data", token_of(&inst));
    for n in 1..=60 {
        let (s, v, _) = send_plain(&w.app, &uri, Some(&w.admin), None).await;
        assert_eq!(s, StatusCode::OK, "request {n}: {v}");
    }
    let (s, v, _) = send_plain(&w.app, &uri, Some(&w.admin), None).await;
    assert_eq!((s, v["code"].as_str()), (StatusCode::TOO_MANY_REQUESTS, Some("rate_limited")), "{v}");

    // A public link: 20 a minute for one address, and another address has its own.
    let uri = format!("/api/v1/shared/{}/data", token_of(&public));
    for n in 1..=20 {
        let (s, v, _) = send_plain(&w.app, &uri, None, Some("203.0.113.7")).await;
        assert_eq!(s, StatusCode::OK, "request {n}: {v}");
    }
    assert_eq!(send_plain(&w.app, &uri, None, Some("203.0.113.7")).await.0, StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(send_plain(&w.app, &uri, None, Some("203.0.113.8")).await.0, StatusCode::OK);
}

// 7. A guard that forbids a unit blocks an institution share for that unit.
#[tokio::test]
async fn a_guard_that_forbids_a_unit_blocks_an_institution_share() {
    let _db = common::TestDb::new();
    let w = world(10, false).await;
    let view = a_view(&w).await;
    setting(&w, json!(["institution"])).await;
    let (_, inst) = share(&w, &w.admin, &view, json!({ "scope": "institution" })).await;
    let uri = format!("/api/v1/shared/{}/data", token_of(&inst));
    assert_eq!(send_plain(&w.app, &uri, Some(&w.viewer), None).await.0, StatusCode::OK);

    // Only people in some other unit may open anything of this workspace.
    let unit = *w.state.organizations.read().unwrap().keys().next().expect("a unit");
    let (s, v) = send(
        &w.app,
        "PUT",
        &format!("/api/v1/workspaces/{WORKSPACE}/guards"),
        &w.platform,
        Some(json!({ "rules": [{ "template": "deny_access_outside_units", "unit_ids": [unit.to_string()] }], "reason": "Another college only" })),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let (s, v, _) = send_plain(&w.app, &uri, Some(&w.viewer), None).await;
    assert_eq!(s, StatusCode::FORBIDDEN, "the guard still decides: {v}");
}

// 8. At most 20 active shares for a view. Another view has its own twenty.
#[tokio::test]
async fn a_view_holds_twenty_active_shares() {
    let _db = common::TestDb::new();
    let w = world(5, false).await;
    let (one, two) = (a_view(&w).await, a_view(&w).await);
    setting(&w, json!(["institution"])).await;
    for n in 1..=20 {
        let (s, v) = share(&w, &w.admin, &one, json!({ "scope": "institution" })).await;
        assert_eq!(s, StatusCode::CREATED, "share {n}: {v}");
    }
    let (s, v) = share(&w, &w.admin, &one, json!({ "scope": "institution" })).await;
    assert_eq!((s, v["code"].as_str()), (StatusCode::TOO_MANY_REQUESTS, Some("quota_exceeded")), "{v}");
    assert_eq!(share(&w, &w.admin, &two, json!({ "scope": "institution" })).await.0, StatusCode::CREATED);
}

// 9. A shared view is read-only. It downloads only if the share was made to, and never for a public link on a table with a
// sensitive field.
#[tokio::test]
async fn a_download_needs_the_share_to_allow_it_and_never_carries_a_sensitive_field() {
    let _db = common::TestDb::new();
    let w = world(12, false).await;
    let view = a_view(&w).await;
    setting(&w, json!(["institution", "public"])).await;
    classify(&w, "Public");
    let (_, plain) = share(&w, &w.admin, &view, json!({ "scope": "institution" })).await;
    let (s, v, _) = send_plain(&w.app, &format!("/api/v1/shared/{}/download", token_of(&plain)), Some(&w.admin), None).await;
    assert_eq!((s, v["code"].as_str()), (StatusCode::FORBIDDEN, Some("share_not_allowed")), "{v}");

    let (s, allowed) = share(&w, &w.admin, &view, json!({ "scope": "institution", "allow_download": true })).await;
    assert_eq!(s, StatusCode::CREATED, "{allowed}");
    let (s, _, csv) = send_plain(&w.app, &format!("/api/v1/shared/{}/download", token_of(&allowed)), Some(&w.admin), None).await;
    assert_eq!(s, StatusCode::OK);
    let header = csv.lines().next().unwrap();
    assert!(header.contains("name") && !header.contains("gpa"), "a sensitive field is not in a download: {header}");
    assert_eq!(csv.lines().count(), 13, "a header and every row: {csv}");

    // A table with a sensitive field is not offered for download to anyone with the link.
    let (s, v) = share(&w, &w.admin, &view, json!({ "scope": "public", "allow_download": true })).await;
    assert_eq!((s, v["code"].as_str()), (StatusCode::FORBIDDEN, Some("share_not_allowed")), "{v}");
    // A shared view takes no writes: the data routes have only reads.
    let (s, _) = send(&w.app, "POST", &format!("/api/v1/shared/{}/data", token_of(&allowed)), &w.admin, Some(json!({}))).await;
    assert!(matches!(s, StatusCode::METHOD_NOT_ALLOWED | StatusCode::NOT_FOUND), "{s}");
}

// 10. The admin console lists every share with its use, and revokes all of an app's at once.
#[tokio::test]
async fn the_console_lists_shares_with_their_use_and_revokes_an_apps_at_once() {
    let _db = common::TestDb::new();
    let w = world(5, false).await;
    let view = a_view(&w).await;
    setting(&w, json!(["institution"])).await;
    let (_, a) = share(&w, &w.admin, &view, json!({ "scope": "institution" })).await;
    let (_, b) = share(&w, &w.admin, &view, json!({ "scope": "institution" })).await;
    for _ in 0..3 {
        send_plain(&w.app, &format!("/api/v1/shared/{}/data", token_of(&a)), Some(&w.admin), None).await;
    }
    let (s, list) = send(&w.app, "GET", "/api/v1/admin/shares", &w.platform, None).await;
    assert_eq!(s, StatusCode::OK, "{list}");
    let mine: Vec<&Value> = list["shares"].as_array().unwrap().iter().filter(|r| r["app"] == w.slug.as_str()).collect();
    assert_eq!(mine.len(), 2, "{list}");
    let used = mine.iter().find(|r| r["id"] == a["share"]["id"]).unwrap();
    assert_eq!((used["use_count"].as_i64(), used["state"].as_str(), used["created_by"].as_str().is_some()), (Some(3), Some("Active"), true), "{used}");
    assert!(used["view_name"].as_str().is_some() && used["app_title"].as_str().is_some() && used.get("token").is_none(), "{used}");

    let (s, _) = send(&w.app, "GET", "/api/v1/admin/shares", &w.admin, None).await;
    assert_eq!(s, StatusCode::FORBIDDEN, "only a Platform Admin");
    let (s, v) = send(&w.app, "POST", "/api/v1/admin/shares/revoke", &w.platform, Some(json!({ "app": w.slug }))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "a reason is required: {v}");
    let (s, v) = send(&w.app, "POST", "/api/v1/admin/shares/revoke", &w.platform, Some(json!({ "app": w.slug, "reason": "The app moved to a new owner" }))).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["revoked"], 2);
    for t in [token_of(&a), token_of(&b)] {
        assert_eq!(send_plain(&w.app, &format!("/api/v1/shared/{t}/data"), Some(&w.admin), None).await.0, StatusCode::NOT_FOUND);
    }
    let (_, after) = send(&w.app, "GET", "/api/v1/admin/shares", &w.platform, None).await;
    assert!(after["shares"].as_array().unwrap().iter().filter(|r| r["app"] == w.slug.as_str()).all(|r| r["state"] == "Revoked"), "{after}");
}
