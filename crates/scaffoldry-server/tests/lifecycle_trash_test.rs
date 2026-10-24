//! Lifecycle phase 1: delete moves a record to trash, and restore brings it back with its links.
//!
//! The app is private to each run, so the tests share the database without touching anyone's rows.

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::Router;
use http_body_util::BodyExt;
use scaffoldry_engine::{AppManifest, AppTable, AppView, FieldSpec, FieldType, LinkCardinality, ViewType};
use scaffoldry_server::repository::PostgresRepository;
use scaffoldry_server::build_app_with_state;
use scaffoldry_server::service::identity::issue_test_token_and_user;
use scaffoldry_server::state::ServerState;
use serde_json::{json, Value};
use std::sync::Arc;
use tower::ServiceExt;
use uuid::Uuid;


const OWNER: &str = "prof.curie@science.state.edu";
const EDITOR2: &str = "grace.hopper@science.state.edu";
const VIEWER: &str = "marcus.vance@state.edu";
const OUTSIDER: &str = "sarah.connor@state.edu";
const EDITOR: &str = "student.smith@science.state.edu";
const WORKSPACE: &str = "ws-bio-lab";

fn db_url() -> String {
    scaffoldry_server::testing::database_url()
}

/// Direct database work on its own thread. The synchronous client cannot start inside the async
/// test runtime.
fn db<R: Send + 'static>(f: impl FnOnce(&mut postgres::Client) -> R + Send + 'static) -> R {
    let url = db_url();
    std::thread::spawn(move || {
        let mut c = postgres::Client::connect(&url, postgres::NoTls).expect("database");
        f(&mut c)
    })
    .join()
    .expect("database thread")
}

struct Fixture {
    app: Router,
    slug: String,
    owner: String,
    editor: String,
    editor2: String,
    viewer: String,
    outsider: String,
}

fn link(name: &str, target: &str, cardinality: LinkCardinality, related: &str) -> FieldSpec {
    let mut f = FieldSpec::simple(name, name, FieldType::Link, false, false);
    f.link_target_table = Some(target.to_string());
    f.link_cardinality = cardinality;
    f.link_related_field = Some(related.to_string());
    f
}

/// An app with the table `t1`, and `apps` and `programs`, which link to each other. `ssn` is FERPA sensitive. `total` is a formula, so it is computed.
async fn fixture() -> Fixture {
    let state = Arc::new(ServerState::new().expect("state"));
    state.seed_demo().expect("seed");
    let slug = format!("hist-{}", &Uuid::new_v4().to_string()[..8]);
    let fields = || {
        vec![
            FieldSpec::simple("title", "Title", FieldType::Text, false, false),
            FieldSpec::simple("score", "Score", FieldType::Number, false, false),
            FieldSpec::simple("ssn", "SSN", FieldType::Text, false, true),
            FieldSpec::simple("total", "Total", FieldType::Formula, false, false),
        ]
    };
    state
        .engine
        .write()
        .unwrap()
        .register_manifest(AppManifest {
            slug: slug.clone(),
            title: "History".to_string(),
            description: String::new(),
            organization_code: "BIO".to_string(),
            department: "biology".to_string(),
            workspace_id: Some(WORKSPACE.to_string()),
            herm_capability_id: None,
            coordinates: None, sealed: false,
            custom_domain: None,
            custom_domain_verified: false,
            views: vec![AppView::table("main", "Main", ViewType::Table, fields())],
            pages: vec![],
        access: Default::default(),
            ceds_mappings: Default::default(),
            tables: vec![
                AppTable {
                    id: "t1".to_string(),
                    name: "Things".to_string(),
                    slug: "things".to_string(),
                    description: None,
                    icon: None,
                    fields: fields(),
                    primary_field: Some("title".to_string()),
                    sample_records: vec![],
                    coordinates: None, sealed: false,
                    row_place: Default::default(),
                },
                AppTable {
                    id: "apps".to_string(),
                    name: "Applications".to_string(),
                    slug: "apps".to_string(),
                    description: None,
                    icon: None,
                    fields: vec![FieldSpec::simple("applicant", "Applicant", FieldType::Text, false, false), link("program", "programs", LinkCardinality::One, "applications")],
                    primary_field: Some("applicant".to_string()),
                    sample_records: vec![],
                    coordinates: None, sealed: false,
                    row_place: Default::default(),
                },
                AppTable {
                    id: "programs".to_string(),
                    name: "Programs".to_string(),
                    slug: "programs".to_string(),
                    description: None,
                    icon: None,
                    fields: vec![FieldSpec::simple("code", "Code", FieldType::Text, false, false), link("applications", "apps", LinkCardinality::Many, "program")],
                    primary_field: Some("code".to_string()),
                    sample_records: vec![],
                    coordinates: None, sealed: false,
                    row_place: Default::default(),
                },
            ],
            relationships: vec![],
        })
        .unwrap();
    state.collaborators.write().unwrap().entry(WORKSPACE.to_string()).or_default().push(scaffoldry_server::state::CollaboratorRecord {
        id: format!("collab-{slug}"),
        workspace_id: WORKSPACE.to_string(),
        eppn: EDITOR2.to_string(),
        name: "Grace Hopper".to_string(),
        role: "editor".to_string(),
        scoped_affiliation: "staff".to_string(),
        department: "biology".to_string(),
        added_at: chrono::Utc::now().to_rfc3339(),
        app_slug: String::new(),
    });
    Fixture {
        app: build_app_with_state(state.clone()).expect("router"),
        slug,
        owner: issue_test_token_and_user(OWNER),
        editor: issue_test_token_and_user(EDITOR),
        editor2: issue_test_token_and_user(EDITOR2),
        viewer: issue_test_token_and_user(VIEWER),
        outsider: issue_test_token_and_user(OUTSIDER),
    }
}

async fn send(app: &Router, method: &str, uri: &str, token: &str, headers: &[(&str, &str)], body: Option<Value>) -> (StatusCode, Value) {
    let mut b = Request::builder().method(method).uri(uri).header("authorization", format!("Bearer {token}"));
    for (k, v) in headers {
        b = b.header(*k, *v);
    }
    let req = match body {
        Some(v) => b.header("content-type", "application/json").body(Body::from(v.to_string())).unwrap(),
        None => b.body(Body::empty()).unwrap(),
    };
    let resp = app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
}

async fn create(f: &Fixture, token: &str, data: Value) -> Value {
    let (s, body) = send(&f.app, "POST", &format!("/api/v1/apps/{}/records", f.slug), token, &[], Some(json!({ "data": data }))).await;
    assert_eq!(s, StatusCode::CREATED, "{body}");
    body
}



async fn make(f: &Fixture, token: &str, table: &str, data: Value) -> String {
    let mut d = data;
    d["_table_id"] = json!(table);
    create(f, token, d).await["id"].as_str().unwrap().to_string()
}

async fn del(f: &Fixture, token: &str, id: &str, batch: Option<&Uuid>) -> (StatusCode, Value) {
    let hv = batch.map(|b| b.to_string());
    let headers: Vec<(&str, &str)> = hv.iter().map(|h| ("x-scaffoldry-batch", h.as_str())).collect();
    send(&f.app, "DELETE", &format!("/api/v1/apps/{}/records/{id}", f.slug), token, &headers, None).await
}

async fn get(f: &Fixture, token: &str, id: &str) -> (StatusCode, Value) {
    send(&f.app, "GET", &format!("/api/v1/apps/{}/records/{id}", f.slug), token, &[], None).await
}

async fn trash(f: &Fixture, token: &str) -> (StatusCode, Value) {
    send(&f.app, "GET", &format!("/api/v1/apps/{}/trash", f.slug), token, &[], None).await
}

async fn restore(f: &Fixture, token: &str, what: &str) -> (StatusCode, Value) {
    send(&f.app, "POST", &format!("/api/v1/apps/{}/trash/{what}/restore", f.slug), token, &[], Some(json!({}))).await
}

async fn listed(f: &Fixture, table: &str) -> Vec<String> {
    let payload = json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/call",
        "params": { "name": "list_records", "arguments": { "app_slug": f.slug, "table_id": table, "limit": 100 } } });
    let (_, v) = send(&f.app, "POST", "/api/mcp", &f.owner, &[], Some(payload)).await;
    v["result"]["structuredContent"]["records"].as_array().unwrap_or(&vec![]).iter().map(|r| r["id"].as_str().unwrap().to_string()).collect()
}

async fn filtered(f: &Fixture, table: &str, title: &str) -> Vec<String> {
    let filter = json!({ "conjunction": "And", "clauses": [{ "id": "c", "field_name": "title", "operator": "Equals", "value": title }] });
    let payload = json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/call",
        "params": { "name": "list_records", "arguments": { "app_slug": f.slug, "table_id": table, "filter": filter } } });
    let (_, v) = send(&f.app, "POST", "/api/mcp", &f.owner, &[], Some(payload)).await;
    v["result"]["structuredContent"]["records"].as_array().unwrap_or(&vec![]).iter().map(|r| r["id"].as_str().unwrap().to_string()).collect()
}

fn held(record: &Value, field: &str) -> Vec<String> {
    match record["data"].get(field) {
        Some(Value::String(s)) => vec![s.clone()],
        Some(Value::Array(a)) => a.iter().filter_map(|v| v.as_str().map(String::from)).collect(),
        _ => vec![],
    }
}

fn ago(slug: &str, id: &str, days: i32) {
    let (slug, id) = (slug.to_string(), id.to_string());
    db(move |c| {
        c.execute("UPDATE dataset_records SET deleted_at = NOW() - make_interval(days => $3) WHERE app_slug = $1 AND id = $2", &[&slug, &id, &days]).unwrap();
    });
}

// 1. Delete moves a record to trash: gone from lists, counts, search, and link options. In trash, with time left.
#[tokio::test]
async fn delete_moves_a_record_to_trash_and_it_vanishes_from_every_read() {
    let _db = common::TestDb::new();
    let f = fixture().await;
    let keep = make(&f, &f.editor, "t1", json!({ "title": "Keep", "score": 1 })).await;
    let gone = make(&f, &f.editor, "t1", json!({ "title": "Gone", "score": 2 })).await;
    let program = make(&f, &f.editor, "programs", json!({ "code": "CS" })).await;
    let repo = PostgresRepository::connect(None).unwrap();
    assert_eq!(repo.count_records_by_app_slugs(std::slice::from_ref(&f.slug)).unwrap()[&f.slug], 3);

    let (status, body) = del(&f, &f.editor, &gone, None).await;
    assert!(status.is_success(), "{status} {body}");
    assert_eq!(get(&f, &f.owner, &gone).await.0, StatusCode::NOT_FOUND, "a read by id");
    assert_eq!(listed(&f, "t1").await, vec![keep.clone()], "a list");
    assert!(filtered(&f, "t1", "Gone").await.is_empty(), "a filtered search");
    assert_eq!(repo.count_records_by_app_slugs(std::slice::from_ref(&f.slug)).unwrap()[&f.slug], 2, "a count");

    // Trash a program: it is no longer a link option.
    let (status, _) = del(&f, &f.editor, &program, None).await;
    assert!(status.is_success());
    let (status, body) = send(&f.app, "POST", &format!("/api/v1/apps/{}/records", f.slug), &f.editor, &[], Some(json!({ "data": { "_table_id": "apps", "applicant": "X", "program": program } }))).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");

    // In trash, with 30 days to go. Three days later, 27.
    let (status, page) = trash(&f, &f.owner).await;
    assert_eq!(status, StatusCode::OK, "{page}");
    let entry = page["entries"].as_array().unwrap().iter().find(|e| e["record_id"] == gone.as_str()).expect("in trash");
    assert_eq!(entry["title"], "Gone", "the primary field names it: {entry}");
    assert_eq!(entry["deleted_by"], EDITOR);
    assert_eq!(entry["days_left"], 30);
    ago(&f.slug, &gone, 3);
    let (_, page) = trash(&f, &f.owner).await;
    let entry = page["entries"].as_array().unwrap().iter().find(|e| e["record_id"] == gone.as_str()).unwrap().clone();
    assert_eq!(entry["days_left"], 27, "{entry}");
}

// 2 and 3. Restore brings it back with its links, except to a record that was also deleted.
#[tokio::test]
async fn restore_brings_a_record_back_with_its_links_except_to_one_that_was_also_deleted() {
    let _db = common::TestDb::new();
    let f = fixture().await;
    let program = make(&f, &f.editor, "programs", json!({ "code": "CS" })).await;
    let a1 = make(&f, &f.editor, "apps", json!({ "applicant": "Ada", "program": program })).await;
    let a2 = make(&f, &f.editor, "apps", json!({ "applicant": "Bo", "program": program })).await;

    // A link to a trashed record is removed from the other record's value.
    del(&f, &f.editor, &program, None).await;
    assert!(held(&get(&f, &f.owner, &a1).await.1, "program").is_empty(), "the application forgets the trashed program");
    // The other application is trashed too, while the program is in trash.
    del(&f, &f.editor, &a2, None).await;

    let (status, body) = restore(&f, &f.editor, &program).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let back = get(&f, &f.owner, &program).await.1;
    assert_eq!(held(&back, "applications"), vec![a1.clone()], "only the live application: {back}");
    assert_eq!(held(&get(&f, &f.owner, &a1).await.1, "program"), vec![program.clone()], "and it is restored with the program");
    assert_eq!(get(&f, &f.owner, &a2).await.0, StatusCode::NOT_FOUND, "the other one is still in trash");
}

// 4. Trash shows each person what they may see: editors their own deletions, owners all, viewers none.
#[tokio::test]
async fn each_person_sees_only_their_part_of_the_trash() {
    let _db = common::TestDb::new();
    let f = fixture().await;
    let r1 = make(&f, &f.editor, "t1", json!({ "title": "Mine", "score": 1, "ssn": "123-45-6789" })).await;
    let r2 = make(&f, &f.editor2, "t1", json!({ "title": "Theirs", "score": 2 })).await;
    del(&f, &f.editor, &r1, None).await;
    del(&f, &f.editor2, &r2, None).await;
    let ids = |page: &Value| -> Vec<String> { page["entries"].as_array().unwrap().iter().map(|e| e["record_id"].as_str().unwrap().to_string()).collect() };

    let (_, mine) = trash(&f, &f.editor).await;
    assert_eq!(ids(&mine), vec![r1.clone()], "an editor sees their own deletions");
    let (_, theirs) = trash(&f, &f.editor2).await;
    assert_eq!(ids(&theirs), vec![r2.clone()]);
    let (_, all) = trash(&f, &f.owner).await;
    let mut both = ids(&all);
    both.sort();
    let mut want = vec![r1.clone(), r2.clone()];
    want.sort();
    assert_eq!(both, want, "an owner sees all");
    let (status, none) = trash(&f, &f.viewer).await;
    assert_eq!(status, StatusCode::OK, "{none}");
    assert!(ids(&none).is_empty(), "a viewer sees none");
    let (status, _) = trash(&f, &f.outsider).await;
    assert!(status == StatusCode::NOT_FOUND || status == StatusCode::FORBIDDEN);
    // An editor cannot restore what someone else deleted. An owner can.
    let (status, _) = restore(&f, &f.editor, &r2).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, _) = restore(&f, &f.owner, &r2).await;
    assert_eq!(status, StatusCode::OK);
}

// 5. A multi-delete restores as one batch.
#[tokio::test]
async fn a_multi_delete_restores_as_one_batch() {
    let _db = common::TestDb::new();
    let f = fixture().await;
    let mut ids = Vec::new();
    for i in 0..3 {
        ids.push(make(&f, &f.editor, "t1", json!({ "title": format!("r{i}"), "score": i })).await);
    }
    let lone = make(&f, &f.editor, "t1", json!({ "title": "lone", "score": 9 })).await;
    let batch = Uuid::new_v4();
    for id in &ids {
        let (s, _) = del(&f, &f.editor, id, Some(&batch)).await;
        assert!(s.is_success());
    }
    del(&f, &f.editor, &lone, None).await;
    assert!(listed(&f, "t1").await.is_empty());

    let (status, body) = restore(&f, &f.editor, &batch.to_string()).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["restored"], 3);
    let mut back = listed(&f, "t1").await;
    back.sort();
    let mut want = ids.clone();
    want.sort();
    assert_eq!(back, want, "the batch is back and the lone record is not");
}

// 6. History has delete and restore entries.
#[tokio::test]
async fn history_records_the_delete_and_the_restore() {
    let _db = common::TestDb::new();
    let f = fixture().await;
    let id = make(&f, &f.editor, "t1", json!({ "title": "A", "score": 1 })).await;
    del(&f, &f.editor, &id, None).await;
    restore(&f, &f.editor, &id).await;
    let (slug, rid) = (f.slug.clone(), id.clone());
    let actions: Vec<String> = db(move |c| c.query("SELECT action FROM record_history WHERE app_slug = $1 AND record_id = $2 ORDER BY id", &[&slug, &rid]).unwrap().iter().map(|r| r.get(0)).collect());
    assert_eq!(actions, vec!["create", "delete", "restore"]);
}

// 7. The partial index serves the list query, with no sequential scan.
#[tokio::test]
async fn the_list_query_uses_the_partial_index() {
    let _db = common::TestDb::new();
    let f = fixture().await;
    let slug = f.slug.clone();
    db(move |c| {
        c.execute(
            "INSERT INTO dataset_records (id, app_slug, data, ceds_mapping, is_ferpa_sensitive, table_id, created_at) \
             SELECT 'rec-' || $1::text || '-' || g, $1::text, jsonb_build_object('_table_id', 't1', 'title', 'x'), '{}', false, 't1', \
                    TIMESTAMPTZ '2026-01-01 00:00:00+00' + g * interval '1 millisecond' \
             FROM generate_series(1, 200000) AS g",
            &[&slug],
        )
        .unwrap();
        c.batch_execute("ANALYZE dataset_records").unwrap();
    });
    let repo = PostgresRepository::connect(None).unwrap();
    use scaffoldry_server::repository::{SqlParam, FIRST_PAGE_SQL, PAGE_SQL};
    let first = repo.explain(FIRST_PAGE_SQL, &[SqlParam::Text(f.slug.clone()), SqlParam::Text("t1".into()), SqlParam::Int(201)]).unwrap();
    assert!(first.contains("idx_dataset_records_live") && !first.contains("Seq Scan"), "{first}");
    let next = repo
        .explain(PAGE_SQL, &[SqlParam::Text(f.slug.clone()), SqlParam::Text("t1".into()), SqlParam::Text("2026-01-01 00:00:50+00".into()), SqlParam::Text("rec-x".into()), SqlParam::Int(201)])
        .unwrap();
    assert!(next.contains("idx_dataset_records_live") && !next.contains("Seq Scan"), "{next}");
    let slug = f.slug.clone();
    db(move |c| {
        c.execute("DELETE FROM dataset_records WHERE app_slug = $1", &[&slug]).unwrap();
    });
}
