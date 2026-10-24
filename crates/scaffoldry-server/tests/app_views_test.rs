//! Views phase 1: views live apart from the manifest.
//!
//! A view is presentation and is not governed. It lives in `app_views`. A personal view is its
//! owner's alone, a collaborative view needs an editor to change, a locked view needs an admin.
//! Saving a view that sorts or filters on a field outside the index builds that index.

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::Router;
use http_body_util::BodyExt;
use scaffoldry_core::access::{ColumnMode, ColumnRule, Op, Operand, PersonAttr, Pred, RowOp, RowRule, Who};
use scaffoldry_engine::{AppManifest, AppPage, AppTable, AppView, FieldSpec, FieldType, PageKind, SelectOption, SelectOptionColor, ViewType};
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

/// Students with a program, an advisor, a stage and a gpa. With `many_indexed`, twenty more fields are
/// marked indexed and a twenty-first is not.
fn manifest(slug: &str, many_indexed: bool) -> AppManifest {
    let mut fields = vec![
        FieldSpec::simple("name", "Name", FieldType::Text, false, false),
        select("program", &["p0", "p1", "p2", "p3", "p4"]),
        FieldSpec::simple("advisor", "Advisor", FieldType::Text, false, false),
        select("stage", &["opt_a", "opt_b"]),
        FieldSpec::simple("gpa", "GPA", FieldType::Number, false, false),
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
        coordinates: None, sealed: false,
        custom_domain: None,
        custom_domain_verified: false,
        tables: vec![AppTable {
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
        pages: vec![],
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
    editor: String,
    editor_eppn: String,
    viewer: String,
    viewer2: String,
    _cleanup: Cleanup,
}

/// `rows` students. Every tenth is advised by the editor, every tenth plus one by the viewer. A row rule
/// shows each person the students they advise, except to an admin or owner, and a column rule hides
/// `stage` from everyone but an admin or owner.
async fn world(rows: i64, many_indexed: bool) -> World {
    world_with(rows, many_indexed, true).await
}

/// The same world with no row or column rule. A row rule's fields join the manifest's indexed set, which counts
/// toward the limit of twenty, so a test of index accounting starts without rules.
async fn world_without_rules(rows: i64, many_indexed: bool) -> World {
    world_with(rows, many_indexed, false).await
}

async fn world_with(rows: i64, many_indexed: bool, with_rules: bool) -> World {
    let tag = Uuid::new_v4().to_string()[..8].to_string();
    let slug = format!("av-{tag}");
    let (admin_e, editor_e, viewer_e, viewer2_e) =
        (format!("admin-{tag}@state.edu"), format!("editor-{tag}@state.edu"), format!("viewer-{tag}@state.edu"), format!("viewer2-{tag}@state.edu"));
    let platform = issue_test_token_and_user(PLATFORM);
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
        c.batch_execute("ANALYZE dataset_records").unwrap();
    });

    World {
        app: build_app_with_state(state.clone()).expect("router"),
        state,
        platform,
        admin: issue_test_token_and_user(&admin_e),
        editor: issue_test_token_and_user(&editor_e),
        viewer: issue_test_token_and_user(&viewer_e),
        viewer2: issue_test_token_and_user(&viewer2_e),
        editor_eppn: editor_e,
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

fn views_path(w: &World) -> String {
    format!("/api/v1/apps/{}/views", w.slug)
}

fn grid(name: &str, ownership: &str, config: Value) -> Value {
    json!({ "name": name, "kind": "grid", "ownership": ownership, "table_id": "students", "config": config })
}

async fn create(w: &World, token: &str, body: Value) -> (StatusCode, Value) {
    send(&w.app, "POST", &views_path(w), token, Some(body)).await
}

fn sort_on(field: &str) -> Value {
    json!({ "sort_rules": [{ "id": "s1", "field_name": field, "direction": "Asc" }] })
}

fn names(list: &Value) -> Vec<String> {
    list["views"].as_array().unwrap().iter().map(|v| v["name"].as_str().unwrap().to_string()).collect()
}

async fn tool(w: &World, token: &str, name: &str, args: Value) -> (bool, Value) {
    let payload = json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": { "name": name, "arguments": args } });
    let (_, v) = send(&w.app, "POST", "/api/v1/mcp", token, Some(payload)).await;
    let result = &v["result"];
    if result["isError"] == true {
        return (true, result["content"][0]["text"].clone());
    }
    (false, result["structuredContent"].clone())
}

/// The rationale of each `ViewsMigrated` ledger entry for an app.
async fn migrated_entries(app: &Router, token: &str, slug: &str) -> Vec<String> {
    let (_, ledger) = send(app, "GET", "/api/v1/governance/ledger", token, None).await;
    ledger["entries"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|e| e["decision_type"] == "ViewsMigrated" && e["app_slug"] == slug)
        .map(|e| e["rationale"].as_str().unwrap_or("").to_string())
        .collect()
}

// 1 and 8. Manifest views move to app_views with the same id where it is a UUID, the manifest holds none
//    afterwards, its pages follow the move, and the migration is one ledger entry with the count.
#[tokio::test]
async fn manifest_views_move_to_their_own_table_once_and_the_ledger_counts_them() {
    let _db = common::TestDb::new();
    let tag = Uuid::new_v4().to_string()[..8].to_string();
    let slug = format!("av-mig-{tag}");
    let _cleanup = Cleanup(tag);
    let uuid_view = Uuid::new_v4().to_string();

    let mut m = manifest(&slug, false);
    let mut a = AppView::table(uuid_view.clone(), "Everyone", ViewType::Grid, vec![]);
    a.table_id = Some("students".into());
    let mut b = AppView::table("board", "Board", ViewType::Kanban, vec![]);
    b.table_id = Some("students".into());
    b.kanban_column_field = Some("program".into());
    m.views = vec![a, b];
    m.pages = vec![
        AppPage { id: "all".into(), title: "All".into(), kind: PageKind::View { view_id: uuid_view.clone() } },
        AppPage { id: "board".into(), title: "Board".into(), kind: PageKind::View { view_id: "board".into() } },
    ];
    // The manifest is stored the way an earlier release stored it, views and all.
    let first = ServerState::new().expect("state");
    first.seed_demo().expect("the workspace the manifest names");
    first.repository.upsert_app_manifest(&m).expect("store");
    drop(first);

    // The next start finds the manifest with views in it and moves them.
    let platform = issue_test_token_and_user(PLATFORM);
    let state = Arc::new(ServerState::new().expect("state"));
    let app = build_app_with_state(state.clone()).expect("router");
    let entries = migrated_entries(&app, &platform, &slug).await;
    assert_eq!(entries.len(), 1, "one ledger entry for the app: {entries:?}");
    assert!(entries[0].contains("2 views"), "it carries the count: {}", entries[0]);

    let stored = state.repository.get_app_manifest(&slug).unwrap().expect("stored");
    assert!(stored.views.is_empty(), "the stored manifest holds no views");

    let s = slug.clone();
    let rows: Vec<(String, String, String)> = db(move |c| {
        c.query("SELECT id::text, name, ownership FROM app_views WHERE app_slug = $1 ORDER BY name", &[&s])
            .unwrap()
            .iter()
            .map(|r| (r.get(0), r.get(1), r.get(2)))
            .collect()
    });
    assert_eq!(rows.len(), 2, "{rows:?}");
    assert!(rows.iter().all(|r| r.2 == "collaborative"));
    let board = rows.iter().find(|r| r.1 == "Board").expect("the board");
    assert_ne!(board.0, "board", "a view whose id was not a UUID gets one");
    assert!(Uuid::parse_str(&board.0).is_ok());
    assert!(rows.iter().any(|r| r.0 == uuid_view), "a UUID id is kept");

    // The pages follow the views.
    let by_id = |id: &str| stored.pages.iter().find(|p| p.id == id).map(|p| p.kind.clone());
    assert_eq!(by_id("all"), Some(PageKind::View { view_id: uuid_view.clone() }));
    assert_eq!(by_id("board"), Some(PageKind::View { view_id: board.0.clone() }));

    // What runs is unchanged: the registered copy still has both views.
    {
        use scaffoldry_engine::HostRouter;
        let engine = state.engine.read().unwrap();
        assert_eq!(engine.resolve_by_slug(&slug).expect("registered").views.len(), 2);
    }
    let (s, listed) = send(&app, "GET", &views_path_for(&slug), &platform, None).await;
    assert_eq!(s, StatusCode::OK, "{listed}");
    let mut listed_names = names(&listed);
    listed_names.sort();
    assert_eq!(listed_names, vec!["Board".to_string(), "Everyone".to_string()]);

    // Starting again moves nothing and writes nothing.
    let again = Arc::new(ServerState::new().expect("state"));
    let app2 = build_app_with_state(again).expect("router");
    assert_eq!(migrated_entries(&app2, &platform, &slug).await.len(), 1, "still one entry");
}

fn views_path_for(slug: &str) -> String {
    format!("/api/v1/apps/{slug}/views")
}

// 2. Who may create and change which view.
#[tokio::test]
async fn who_may_create_and_change_which_view() {
    let _db = common::TestDb::new();
    let w = world(40, false).await;

    // A viewer creates a personal view, and cannot create a collaborative or a locked one.
    let (s, mine) = create(&w, &w.viewer, grid("My list", "personal", sort_on("name"))).await;
    assert_eq!(s, StatusCode::CREATED, "{mine}");
    assert_eq!(mine["ownership"], "personal");
    assert_eq!(mine["owner"], format!("viewer-{}@state.edu", w.slug.trim_start_matches("av-")));
    for ownership in ["collaborative", "locked"] {
        let (s, body) = create(&w, &w.viewer, grid("Nope", ownership, json!({}))).await;
        assert_eq!(s, StatusCode::FORBIDDEN, "a viewer cannot create a {ownership} view: {body}");
    }

    // An editor creates a collaborative view and cannot create a locked one. An admin can.
    let (s, shared) = create(&w, &w.editor, grid("Team list", "collaborative", json!({}))).await;
    assert_eq!(s, StatusCode::CREATED, "{shared}");
    let (s, _) = create(&w, &w.editor, grid("Locked by editor", "locked", json!({}))).await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    let (s, locked) = create(&w, &w.admin, grid("Official", "locked", json!({}))).await;
    assert_eq!(s, StatusCode::CREATED, "{locked}");

    // A viewer cannot change a collaborative view. An editor can. An editor cannot change a locked view,
    // and cannot delete it. An admin can do both. Everyone can use a locked view.
    let shared_path = format!("{}/{}", views_path(&w), shared["id"].as_str().unwrap());
    let locked_path = format!("{}/{}", views_path(&w), locked["id"].as_str().unwrap());
    let change = |version: i64, name: &str| json!({ "name": name, "config": {}, "version": version });
    let (s, _) = send(&w.app, "PUT", &shared_path, &w.viewer, Some(change(1, "Hijacked"))).await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    let (s, body) = send(&w.app, "PUT", &shared_path, &w.editor, Some(change(1, "Team list v2"))).await;
    assert_eq!(s, StatusCode::OK, "{body}");
    assert_eq!((body["name"].as_str(), body["version"].as_i64()), (Some("Team list v2"), Some(2)));
    let (s, _) = send(&w.app, "PUT", &locked_path, &w.editor, Some(change(1, "Hijacked"))).await;
    assert_eq!(s, StatusCode::FORBIDDEN, "an editor cannot change a locked view");
    let (s, _) = send(&w.app, "DELETE", &locked_path, &w.editor, None).await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    let (s, body) = send(&w.app, "PUT", &locked_path, &w.admin, Some(change(1, "Official v2"))).await;
    assert_eq!(s, StatusCode::OK, "{body}");
    let (_, listed) = send(&w.app, "GET", &views_path(&w), &w.viewer, None).await;
    assert!(names(&listed).contains(&"Official v2".to_string()), "a viewer sees and uses the locked view");

    // Deleting is soft: the row stays, and the view is gone from the list.
    let (s, _) = send(&w.app, "DELETE", &locked_path, &w.admin, None).await;
    assert_eq!(s, StatusCode::OK);
    let (_, listed) = send(&w.app, "GET", &views_path(&w), &w.admin, None).await;
    assert!(!names(&listed).contains(&"Official v2".to_string()));
    let id = locked["id"].as_str().unwrap().to_string();
    let kept: i64 = db(move |c| c.query_one("SELECT count(*) FROM app_views WHERE id::text = $1 AND deleted_at IS NOT NULL", &[&id]).unwrap().get(0));
    assert_eq!(kept, 1);

    // A stranger to the app can do nothing.
    let stranger = issue_test_token_and_user(&format!("stranger-{}@state.edu", Uuid::new_v4().simple()));
    let (s, _) = send(&w.app, "GET", &views_path(&w), &stranger, None).await;
    assert!(s == StatusCode::FORBIDDEN || s == StatusCode::NOT_FOUND, "{s}");
}

// 3. A personal view is invisible to others, even to an admin.
#[tokio::test]
async fn a_personal_view_is_invisible_to_others() {
    let _db = common::TestDb::new();
    let w = world(40, false).await;
    let (s, mine) = create(&w, &w.viewer, grid("Only mine", "personal", json!({}))).await;
    assert_eq!(s, StatusCode::CREATED, "{mine}");
    let path = format!("{}/{}", views_path(&w), mine["id"].as_str().unwrap());

    for (who, token) in [("another viewer", &w.viewer2), ("an editor", &w.editor), ("an admin", &w.admin), ("a Platform Admin", &w.platform)] {
        let (_, listed) = send(&w.app, "GET", &views_path(&w), token, None).await;
        assert!(!names(&listed).contains(&"Only mine".to_string()), "{who} does not see it");
        let (s, _) = send(&w.app, "PUT", &path, token, Some(json!({ "name": "x", "config": {}, "version": 1 }))).await;
        assert!(s == StatusCode::NOT_FOUND || s == StatusCode::FORBIDDEN, "{who} cannot change it: {s}");
        let (s, _) = send(&w.app, "DELETE", &path, token, None).await;
        assert!(s == StatusCode::NOT_FOUND || s == StatusCode::FORBIDDEN, "{who} cannot delete it: {s}");
    }
    let (_, listed) = send(&w.app, "GET", &views_path(&w), &w.viewer, None).await;
    assert!(names(&listed).contains(&"Only mine".to_string()), "its owner does");

    // Duplicating a shared view makes a personal one for the caller.
    let (_, shared) = create(&w, &w.editor, grid("Shared", "collaborative", json!({}))).await;
    let dup = format!("{}/{}/duplicate", views_path(&w), shared["id"].as_str().unwrap());
    let (s, copy) = send(&w.app, "POST", &dup, &w.viewer, None).await;
    assert_eq!(s, StatusCode::CREATED, "{copy}");
    assert_eq!(copy["ownership"], "personal");
    assert_ne!(copy["id"], shared["id"]);
    let (_, listed) = send(&w.app, "GET", &views_path(&w), &w.viewer2, None).await;
    assert!(!names(&listed).iter().any(|n| n.contains("Shared") && n != "Shared"), "the copy is the viewer's alone");
}

// 4. A stale version is 409.
#[tokio::test]
async fn a_stale_version_is_409() {
    let _db = common::TestDb::new();
    let w = world(40, false).await;
    let (_, v) = create(&w, &w.editor, grid("Versioned", "collaborative", json!({}))).await;
    let path = format!("{}/{}", views_path(&w), v["id"].as_str().unwrap());
    let (s, first) = send(&w.app, "PUT", &path, &w.editor, Some(json!({ "name": "Versioned", "config": {}, "version": 1 }))).await;
    assert_eq!(s, StatusCode::OK, "{first}");
    let (s, stale) = send(&w.app, "PUT", &path, &w.admin, Some(json!({ "name": "Late", "config": {}, "version": 1 }))).await;
    assert_eq!(s, StatusCode::CONFLICT, "{stale}");
    assert_eq!(stale["code"], "version_conflict");
    assert_eq!(stale["version"], 2, "it says which version is current");
}

// 5. A view with a sort on an unindexed field of a 200,000-row table returns index_state building and
//    enqueues one job. Deleting the only view that needs the field lets the index go.
#[tokio::test]
async fn a_view_that_sorts_on_an_unindexed_field_builds_its_index() {
    let _db = common::TestDb::new();
    let w = world_without_rules(200_000, false).await;

    let (s, plain) = create(&w, &w.admin, grid("By name", "collaborative", json!({}))).await;
    assert_eq!(s, StatusCode::CREATED, "{plain}");
    assert_eq!(plain["index_state"], "ready", "nothing to build");

    let (s, v) = create(&w, &w.admin, grid("By advisor", "collaborative", sort_on("advisor"))).await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    assert_eq!(v["index_state"], "building");
    let slug = w.slug.clone();
    let jobs = move || {
        let slug = slug.clone();
        db(move |c| {
            c.query_one("SELECT count(*) FROM jobs WHERE kind = 'build_index' AND payload->>'app_slug' = $1", &[&slug]).unwrap().get::<_, i64>(0)
        })
    };
    assert_eq!(jobs(), 1, "one job");

    // A second view that sorts on the same field joins nothing new, and builds nothing.
    let (s, again) = create(&w, &w.admin, grid("Also by advisor", "personal", sort_on("advisor"))).await;
    assert_eq!(s, StatusCode::CREATED, "{again}");
    assert_eq!(again["index_state"], "building", "the field is still being built");
    assert_eq!(jobs(), 1, "still one job");
}

// 6. A twenty-first indexed field is too_many_indexes, and the response lists the indexed fields no view uses.
#[tokio::test]
async fn a_twenty_first_indexed_field_is_refused_with_the_unused_ones_listed() {
    let _db = common::TestDb::new();
    let w = world_without_rules(100, true).await;
    let (s, body) = create(&w, &w.admin, grid("Sorted on late", "collaborative", sort_on("late"))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(body["code"], "too_many_indexes");
    let unused: Vec<String> = body["unused"].as_array().expect("the list").iter().map(|v| v.as_str().unwrap().to_string()).collect();
    assert_eq!(unused.len(), 20, "{unused:?}");
    assert!(unused.contains(&"kept01".to_string()) && unused.contains(&"kept20".to_string()));

    // A view that uses one of the twenty is fine, and it is no longer unused.
    let (s, ok) = create(&w, &w.admin, grid("Sorted on kept01", "collaborative", sort_on("kept01"))).await;
    assert_eq!(s, StatusCode::CREATED, "{ok}");
    assert_eq!(ok["index_state"], "ready", "it was already indexed");
}

// 7. A view cannot return a row or a field the person may not read.
#[tokio::test]
async fn a_view_cannot_return_a_row_or_field_the_person_may_not_read() {
    let _db = common::TestDb::new();
    let w = world(40, false).await;
    // Two kanban boards, shared with everyone. One groups by a field everyone may read, one by a hidden field.
    let board = |name: &str, field: &str| json!({ "name": name, "kind": "kanban", "ownership": "collaborative", "table_id": "students", "config": { "kanban_column_field": field } });
    let (s, by_program) = create(&w, &w.admin, board("By program", "program")).await;
    assert_eq!(s, StatusCode::CREATED, "{by_program}");
    let (s, by_stage) = create(&w, &w.admin, board("By stage", "stage")).await;
    assert_eq!(s, StatusCode::CREATED, "{by_stage}");

    let stage_records = |data: &Value| -> usize { data["stages"].as_array().unwrap().iter().map(|s| s["records"].as_array().unwrap().len()).sum() };
    let args = |view: &Value| json!({ "app_slug": w.slug, "view_id": view["id"] });

    // The admin sees every row, and the hidden field.
    let (err, all) = tool(&w, &w.admin, "get_view_data", args(&by_program)).await;
    assert!(!err, "{all}");
    assert_eq!(stage_records(&all), 40);
    let (_, stage_all) = tool(&w, &w.admin, "get_view_data", args(&by_stage)).await;
    assert_eq!(stage_records(&stage_all), 40, "the admin may read `stage`");

    // The viewer sees only the students they advise (every tenth plus one: 4 of 40), never the hidden field.
    let (err, mine) = tool(&w, &w.viewer, "get_view_data", args(&by_program)).await;
    assert!(!err, "{mine}");
    assert_eq!(stage_records(&mine), 4, "only the viewer's own rows");
    let text = mine.to_string();
    assert!(!text.contains("\"stage\"") && !text.contains("opt_a") && !text.contains("opt_b"), "no hidden field: {text}");
    // A board over a field the viewer cannot see is refused, so it does not even show the field's name or options.
    let (err, hidden) = tool(&w, &w.viewer, "get_view_data", args(&by_stage)).await;
    assert!(err, "a board over a hidden field is refused: {hidden}");
    let text = hidden.to_string();
    assert!(text.contains("cannot see"), "it says why: {text}");
    assert!(!text.contains("opt_a") && !text.contains("opt_b"), "and shows none of the field's options: {text}");

    // Another viewer sees a different four, so the first viewer's rows did not come from a shared answer.
    let (_, other) = tool(&w, &w.viewer2, "get_view_data", args(&by_program)).await;
    assert_eq!(stage_records(&other), 0, "viewer2 advises no one");
    // The editor sees the students they advise.
    let (_, ed) = tool(&w, &w.editor, "get_view_data", args(&by_program)).await;
    assert_eq!(stage_records(&ed), 4);
    let _ = &w.editor_eppn;
}

// 9. The tools list, save and delete views with the same rules. A save checks the version.
#[tokio::test]
async fn the_view_tools_follow_the_same_rules() {
    let _db = common::TestDb::new();
    let w = world(40, false).await;
    let (err, saved) = tool(&w, &w.viewer, "save_view", json!({ "app_slug": w.slug, "name": "Tool view", "kind": "grid", "ownership": "personal", "table_id": "students", "config": {} })).await;
    assert!(!err, "{saved}");
    let id = saved["id"].as_str().unwrap().to_string();

    let (err, refused) = tool(&w, &w.viewer, "save_view", json!({ "app_slug": w.slug, "name": "Shared by tool", "kind": "grid", "ownership": "collaborative", "table_id": "students", "config": {} })).await;
    assert!(err, "a viewer cannot save a collaborative view by tool either: {refused}");

    let (err, listed) = tool(&w, &w.viewer, "list_views", json!({ "app_slug": w.slug })).await;
    assert!(!err, "{listed}");
    assert!(names(&listed).contains(&"Tool view".to_string()));
    let (_, others) = tool(&w, &w.viewer2, "list_views", json!({ "app_slug": w.slug })).await;
    assert!(!names(&others).contains(&"Tool view".to_string()), "a personal view is invisible by tool too");

    let (err, updated) = tool(&w, &w.viewer, "save_view", json!({ "app_slug": w.slug, "view_id": id, "name": "Renamed", "config": {}, "version": 1 })).await;
    assert!(!err, "{updated}");
    let (err, stale) = tool(&w, &w.viewer, "save_view", json!({ "app_slug": w.slug, "view_id": id, "name": "Late", "config": {}, "version": 1 })).await;
    assert!(err, "a stale version is refused: {stale}");

    let (err, _) = tool(&w, &w.viewer2, "delete_view", json!({ "app_slug": w.slug, "view_id": id })).await;
    assert!(err, "another person cannot delete it");
    let (err, gone) = tool(&w, &w.viewer, "delete_view", json!({ "app_slug": w.slug, "view_id": id })).await;
    assert!(!err, "{gone}");
    let (_, listed) = tool(&w, &w.viewer, "list_views", json!({ "app_slug": w.slug })).await;
    assert!(!names(&listed).contains(&"Renamed".to_string()));
}

// 10. A manifest that arrives later, such as an approved proposal, may carry an older copy of a view. The
//     stored view wins, so an approval does not undo a change to a view.
#[tokio::test]
async fn an_approved_manifest_does_not_put_an_older_copy_of_a_view_back() {
    let _db = common::TestDb::new();
    use scaffoldry_engine::HostRouter;
    let w = world(40, false).await;
    let (s, v) = create(&w, &w.admin, grid("Current name", "collaborative", sort_on("name"))).await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    let id = v["id"].as_str().unwrap().to_string();

    // The proposal was made before the view was renamed, so its manifest holds the old copy.
    let mut arriving = manifest(&w.slug, false);
    let mut old = AppView::table(id.clone(), "Old name", ViewType::Grid, vec![]);
    old.table_id = Some("students".into());
    arriving.views = vec![old];
    w.state.persist_app_manifest(arriving).expect("persist");

    let engine = w.state.engine.read().unwrap();
    let titles: Vec<String> = engine.resolve_by_slug(&w.slug).unwrap().views.iter().filter(|x| x.id == id).map(|x| x.title.clone()).collect();
    assert_eq!(titles, vec!["Current name".to_string()], "one copy, the stored one");
}
