//! Views phase 5: dashboards and linked widgets.
//!
//! A dashboard is a view of kind `dashboard`: a 12-column grid of widgets. Selecting a row in one widget filters another,
//! and the server does the filtering. A dashboard is presentation and needs no proposal, but a page widget names a page
//! the manifest already approved.

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
    slug: String,
    admin: String,
    viewer: String,
    viewer2: String,
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
        admin: issue_test_token_and_user(&admin_e),
        viewer: issue_test_token_and_user(&viewer_e),
        viewer2: issue_test_token_and_user(&viewer2_e),
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

async fn make(w: &World, token: &str, name: &str, kind: &str, config: Value) -> Value {
    let (s, v) = send(&w.app, "POST", &views_path(w), token, Some(json!({ "name": name, "kind": kind, "ownership": "collaborative", "table_id": if kind == "dashboard" { Value::Null } else { json!("students") }, "config": config }))).await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    v
}

fn id_of(v: &Value) -> String {
    v["id"].as_str().or_else(|| v["view"]["id"].as_str()).unwrap().to_string()
}

fn widget(id: &str, x: u8, y: u8, w: u8, h: u8, body: Value) -> Value {
    json!({ "id": id, "title": id, "x": x, "y": y, "w": w, "h": h, "body": body })
}

async fn dashboard(w: &World, token: &str, widgets: Vec<Value>) -> (StatusCode, Value) {
    send(&w.app, "POST", &views_path(w), token, Some(json!({ "name": "Dean", "kind": "dashboard", "ownership": "collaborative", "config": { "columns": 12, "widgets": widgets } }))).await
}

/// A student list, a summary of students by program, and a dashboard that shows both, the chart linked to the list.
async fn linked(w: &World) -> (String, Value) {
    let list = id_of(&make(w, &w.admin, "All students", "grid", json!({})).await);
    let by_program = id_of(&make(w, &w.admin, "By program", "summary", json!({ "group_by": ["program"], "aggregations": [{ "fn": "count" }] })).await);
    let mut chart = widget("chart", 6, 0, 6, 4, json!({ "type": "chart", "view_id": by_program }));
    chart["select_by"] = json!({ "source_widget": "list", "source_field": "program", "target_field": "program" });
    let (s, d) = dashboard(
        w,
        &w.admin,
        vec![
            widget("list", 0, 0, 6, 4, json!({ "type": "view", "view_id": list })),
            chart,
            widget("total", 0, 4, 3, 2, json!({ "type": "stat", "table_id": "students", "fn": "count" })),
            widget("note", 3, 4, 3, 2, json!({ "type": "text", "markdown": "Read **this** first" })),
            widget("board", 6, 4, 6, 4, json!({ "type": "page", "page_id": "board" })),
        ],
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{d}");
    (id_of(&d), d)
}

async fn widget_data(w: &World, token: &str, dash: &str, widget: &str, query: &str) -> (StatusCode, Value) {
    send(&w.app, "GET", &format!("{}/{dash}/widgets/{widget}{query}", views_path(w)), token, None).await
}

// 1. Selecting a row in a list widget filters a linked chart through the server, and the request holds the filter.
#[tokio::test]
async fn selecting_a_row_filters_a_linked_chart_through_the_server() {
    let _db = common::TestDb::new();
    let w = world(40, false).await;
    let (dash, _) = linked(&w).await;

    let (s, all) = widget_data(&w, &w.admin, &dash, "chart", "").await;
    assert_eq!(s, StatusCode::OK, "{all}");
    assert_eq!(all["data"]["summary"]["groups"].as_array().unwrap().len(), 5, "no selection, every program: {all}");
    assert!(all["selected"].is_null(), "{all}");

    let (s, one) = widget_data(&w, &w.admin, &dash, "chart", "?select=p1").await;
    assert_eq!(s, StatusCode::OK, "{one}");
    let groups = one["data"]["summary"]["groups"].as_array().unwrap();
    assert_eq!(groups.len(), 1, "one program: {one}");
    assert_eq!(groups[0]["key"], json!(["p1"]));
    assert_eq!(groups[0]["values"], json!([8.0]));
    assert_eq!(one["selected"], json!({ "field": "program", "value": "p1" }), "the response says what was filtered");

    // The list is the source. It has no select_by, so a selection means nothing to it.
    let (s, refused) = widget_data(&w, &w.admin, &dash, "list", "?select=p1").await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{refused}");
    let (s, rows) = widget_data(&w, &w.admin, &dash, "list", "?limit=10").await;
    assert_eq!(s, StatusCode::OK, "{rows}");
    assert_eq!(rows["data"]["records"].as_array().unwrap().len(), 10);
    // A stat counts every row. A text and a page widget carry no data of their own.
    let (_, total) = widget_data(&w, &w.admin, &dash, "total", "").await;
    assert_eq!(total["data"]["stat"]["value"], json!(40.0), "{total}");
    let (s, text) = widget_data(&w, &w.admin, &dash, "note", "").await;
    assert_eq!((s, text["kind"].as_str()), (StatusCode::OK, Some("text")));
    // An unknown widget is not found.
    assert_eq!(widget_data(&w, &w.admin, &dash, "nope", "").await.0, StatusCode::NOT_FOUND);
}

// 2. Overlapping or out-of-bounds widgets are 400.
#[tokio::test]
async fn overlapping_or_out_of_bounds_widgets_are_refused() {
    let _db = common::TestDb::new();
    let w = world(5, false).await;
    let text = |id: &str, x: u8, y: u8, wd: u8, h: u8| widget(id, x, y, wd, h, json!({ "type": "text", "markdown": "hi" }));
    let (s, ok) = dashboard(&w, &w.admin, vec![text("a", 0, 0, 6, 2), text("b", 6, 0, 6, 2), text("c", 0, 2, 12, 1)]).await;
    assert_eq!(s, StatusCode::CREATED, "edge to edge fits: {ok}");

    for (what, widgets) in [
        ("overlap", vec![text("a", 0, 0, 6, 2), text("b", 5, 1, 6, 2)]),
        ("past the right edge", vec![text("a", 8, 0, 6, 2)]),
        ("zero width", vec![text("a", 0, 0, 0, 2)]),
        ("zero height", vec![text("a", 0, 0, 2, 0)]),
        ("repeated id", vec![text("a", 0, 0, 2, 2), text("a", 4, 0, 2, 2)]),
        ("no id", vec![text("", 0, 0, 2, 2)]),
    ] {
        let (s, v) = dashboard(&w, &w.admin, widgets).await;
        assert_eq!(s, StatusCode::BAD_REQUEST, "{what}: {v}");
    }
    let (_, overlap) = dashboard(&w, &w.admin, vec![text("a", 0, 0, 6, 2), text("b", 5, 1, 6, 2)]).await;
    assert!(overlap.to_string().contains("overlap"), "it says why: {overlap}");

    // An update is checked the same way.
    let id = id_of(&ok);
    let (s, v) = send(&w.app, "PUT", &format!("{}/{id}", views_path(&w)), &w.admin, Some(json!({ "config": { "columns": 12, "widgets": [text("a", 0, 0, 7, 2), text("b", 6, 0, 6, 2)] }, "version": 1 }))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{v}");
    // A widget may only name what exists: a view of this app, a source widget that is not itself.
    let ghost = widget("a", 0, 0, 6, 2, json!({ "type": "view", "view_id": Uuid::new_v4().to_string() }));
    assert_eq!(dashboard(&w, &w.admin, vec![ghost]).await.0, StatusCode::BAD_REQUEST);
    let mut linked_to_itself = text("a", 0, 0, 6, 2);
    linked_to_itself["select_by"] = json!({ "source_widget": "a", "source_field": "program", "target_field": "program" });
    assert_eq!(dashboard(&w, &w.admin, vec![linked_to_itself]).await.0, StatusCode::BAD_REQUEST);
    let mut linked_to_nothing = text("a", 0, 0, 6, 2);
    linked_to_nothing["select_by"] = json!({ "source_widget": "zzz", "source_field": "program", "target_field": "program" });
    assert_eq!(dashboard(&w, &w.admin, vec![linked_to_nothing]).await.0, StatusCode::BAD_REQUEST);
}

// 3. A page widget naming an unapproved page is 400.
#[tokio::test]
async fn a_page_widget_must_name_an_approved_custom_page() {
    let _db = common::TestDb::new();
    let w = world(5, false).await;
    let page = |id: &str| widget("p", 0, 0, 6, 4, json!({ "type": "page", "page_id": id }));
    let (s, ok) = dashboard(&w, &w.admin, vec![page("board")]).await;
    assert_eq!(s, StatusCode::CREATED, "{ok}");
    let (s, v) = dashboard(&w, &w.admin, vec![page("not-approved")]).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{v}");
    assert!(v.to_string().contains("not-approved"), "it names the page: {v}");
}

// 4. A restricted person's dashboard shows only their scope in every widget.
#[tokio::test]
async fn a_restricted_person_sees_only_their_scope_in_every_widget() {
    let _db = common::TestDb::new();
    let w = world(40, false).await;
    let (dash, _) = linked(&w).await;

    let rows = |v: &Value| v["data"]["records"].as_array().unwrap().len();
    let (s, mine) = widget_data(&w, &w.viewer, &dash, "list", "?limit=100").await;
    assert_eq!(s, StatusCode::OK, "{mine}");
    assert_eq!(rows(&mine), 4, "only the students the viewer advises");
    let (_, theirs) = widget_data(&w, &w.admin, &dash, "list", "?limit=100").await;
    assert_eq!(rows(&theirs), 40);

    let (_, chart) = widget_data(&w, &w.viewer, &dash, "chart", "").await;
    let counted: f64 = chart["data"]["summary"]["groups"].as_array().unwrap().iter().map(|g| g["values"][0].as_f64().unwrap()).sum();
    assert_eq!(counted, 4.0, "the chart totals the viewer's own rows: {chart}");
    let (_, total) = widget_data(&w, &w.viewer, &dash, "total", "").await;
    assert_eq!(total["data"]["stat"]["value"], json!(4.0), "the stat counts the viewer's own rows: {total}");
    // A selection cannot reach past the scope. The viewer's four students are all in p1. Program p0 has eight students,
    // and the viewer advises none of them.
    let (_, p1) = widget_data(&w, &w.viewer, &dash, "chart", "?select=p1").await;
    assert_eq!(p1["data"]["summary"]["groups"][0]["values"], json!([4.0]), "{p1}");
    let (_, p0) = widget_data(&w, &w.viewer, &dash, "chart", "?select=p0").await;
    assert_eq!(p0["data"]["summary"]["groups"].as_array().unwrap().len(), 0, "{p0}");
    let other = widget_data(&w, &w.viewer2, &dash, "list", "?limit=100").await.1;
    assert_eq!(rows(&other), 0, "someone who advises no one sees no rows");
}
