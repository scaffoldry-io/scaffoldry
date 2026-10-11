//! Record history phase 2: undo a batch, and restore a record's fields to an earlier version.
//!
//! The app is private to each run, so the tests share the database without touching anyone's rows.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::Router;
use http_body_util::BodyExt;
use scaffoldry_engine::{AppManifest, AppTable, AppView, FieldSpec, FieldType, ViewType};
use scaffoldry_server::build_app_with_state;
use scaffoldry_server::service::identity::issue_test_token_and_user;
use scaffoldry_server::state::ServerState;
use serde_json::{json, Value};
use std::sync::Arc;
use tokio::sync::Mutex;
use tower::ServiceExt;
use uuid::Uuid;

static LOCK: Mutex<()> = Mutex::const_new(());

const OWNER: &str = "prof.curie@science.state.edu";
const EDITOR2: &str = "grace.hopper@science.state.edu";
const EDITOR: &str = "student.smith@science.state.edu";
const WORKSPACE: &str = "ws-bio-lab";

fn db_url() -> String {
    std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://scaffoldry:scaffoldry_dev_password@127.0.0.1:5433/scaffoldry".to_string())
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
}

/// An app with one table, `t1`. `ssn` is FERPA sensitive. `total` is a formula, so it is computed.
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
            custom_domain: None,
            custom_domain_verified: false,
            views: vec![AppView::table("main", "Main", ViewType::Table, fields())],
            pages: vec![],
        access: Default::default(),
            ceds_mappings: Default::default(),
            tables: vec![AppTable {
                id: "t1".to_string(),
                name: "Things".to_string(),
                slug: "things".to_string(),
                description: None,
                icon: None,
                fields: fields(),
                primary_field: Some("title".to_string()),
                sample_records: vec![],
            }],
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
    });
    Fixture {
        app: build_app_with_state(state.clone()).expect("router"),
        slug,
        owner: issue_test_token_and_user(OWNER),
        editor: issue_test_token_and_user(EDITOR),
        editor2: issue_test_token_and_user(EDITOR2),
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


fn batch_header(id: &Uuid) -> String {
    id.to_string()
}

async fn patch(f: &Fixture, token: &str, id: &str, version: i64, data: Value, batch: Option<&Uuid>) -> (StatusCode, Value) {
    let hv = batch.map(batch_header);
    let headers: Vec<(&str, &str)> = hv.iter().map(|h| ("x-scaffoldry-batch", h.as_str())).collect();
    send(&f.app, "PATCH", &format!("/api/v1/apps/{}/records/{id}", f.slug), token, &headers, Some(json!({ "data": data, "version": version }))).await
}

async fn get(f: &Fixture, token: &str, id: &str) -> (StatusCode, Value) {
    send(&f.app, "GET", &format!("/api/v1/apps/{}/records/{id}", f.slug), token, &[], None).await
}

async fn undo(f: &Fixture, token: &str, batch: &str) -> (StatusCode, Value) {
    send(&f.app, "POST", &format!("/api/v1/apps/{}/history/{batch}/undo", f.slug), token, &[], Some(json!({}))).await
}

/// Ten records with `score` 1 to 10, then a paste that sets each score to 100 more, in one batch.
async fn pasted(f: &Fixture) -> (Vec<String>, Uuid) {
    let mut ids = Vec::new();
    for i in 1..=10 {
        ids.push(create(f, &f.editor, json!({ "_table_id": "t1", "title": format!("r{i}"), "score": i })).await["id"].as_str().unwrap().to_string());
    }
    let batch = Uuid::new_v4();
    for (i, id) in ids.iter().enumerate() {
        let (s, b) = patch(f, &f.editor, id, 1, json!({ "title": format!("r{}", i + 1), "score": 100 + i as i64 + 1 }), Some(&batch)).await;
        assert_eq!(s, StatusCode::OK, "{b}");
    }
    (ids, batch)
}

async fn score(f: &Fixture, id: &str) -> Value {
    get(f, &f.owner, id).await.1["data"]["score"].clone()
}

fn count_reverts(slug: &str, batch: &str) -> i64 {
    let (slug, batch) = (slug.to_string(), batch.to_string());
    db(move |c| c.query_one("SELECT count(*) FROM record_history WHERE app_slug = $1 AND reverts_batch = $2::text::uuid", &[&slug, &batch]).unwrap().get(0))
}

// 1. Paste ten cells, undo: all ten return and one new batch exists with `reverts_batch` set.
#[tokio::test]
async fn undoing_a_paste_of_ten_cells_brings_them_all_back_in_one_new_batch() {
    let _turn = LOCK.lock().await;
    let f = fixture().await;
    let (ids, batch) = pasted(&f).await;
    assert_eq!(score(&f, &ids[0]).await, 101);

    let (status, body) = undo(&f, &f.editor, &batch.to_string()).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    for (i, id) in ids.iter().enumerate() {
        assert_eq!(score(&f, id).await, (i + 1) as i64, "record {i} is back");
    }
    assert_eq!(count_reverts(&f.slug, &batch.to_string()), 10, "one reverting row for each record");
    let new_batch = body["batch_id"].as_str().expect("the new batch id").to_string();
    assert_ne!(new_batch, batch.to_string());
    let slug = f.slug.clone();
    let distinct: i64 = db(move |c| c.query_one("SELECT count(DISTINCT batch_id) FROM record_history WHERE app_slug = $1 AND reverts_batch IS NOT NULL", &[&slug]).unwrap().get(0));
    assert_eq!(distinct, 1, "all ten share one new batch");
}

// 2. Another user edits one of them: 409 listing that record, and nothing changes.
#[tokio::test]
async fn undo_stops_with_a_conflict_when_a_record_has_moved_on_and_writes_nothing() {
    let _turn = LOCK.lock().await;
    let f = fixture().await;
    let (ids, batch) = pasted(&f).await;
    let theirs = get(&f, &f.owner, &ids[2]).await.1;
    let (s, _) = patch(&f, &f.owner, &ids[2], theirs["version"].as_i64().unwrap(), json!({ "title": "r3", "score": 999 }), None).await;
    assert_eq!(s, StatusCode::OK);

    let (status, body) = undo(&f, &f.editor, &batch.to_string()).await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(body["code"], "undo_conflict", "{body}");
    let conflicts = body["conflicts"].as_array().expect("a list of conflicts");
    assert_eq!(conflicts.len(), 1, "{body}");
    assert_eq!(conflicts[0]["record_id"], ids[2].as_str());
    assert_eq!(conflicts[0]["fields"], json!(["score"]));
    assert_eq!(score(&f, &ids[0]).await, 101, "the other nine still hold the paste");
    assert_eq!(score(&f, &ids[2]).await, 999);
    assert_eq!(count_reverts(&f.slug, &batch.to_string()), 0, "nothing was written");
}

// 3. Undo of a create deletes the record, and history records it.
#[tokio::test]
async fn undoing_a_create_deletes_the_record_and_history_says_so() {
    let _turn = LOCK.lock().await;
    let f = fixture().await;
    let batch = Uuid::new_v4();
    let bh = batch.to_string();
    let (s, created) = send(&f.app, "POST", &format!("/api/v1/apps/{}/records", f.slug), &f.editor, &[("x-scaffoldry-batch", bh.as_str())], Some(json!({ "data": { "_table_id": "t1", "title": "Mistake", "score": 1 } }))).await;
    assert_eq!(s, StatusCode::CREATED, "{created}");
    let id = created["id"].as_str().unwrap().to_string();

    let (status, body) = undo(&f, &f.editor, &bh).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(get(&f, &f.owner, &id).await.0, StatusCode::NOT_FOUND, "the record is gone");
    let (slug, rid) = (f.slug.clone(), id.clone());
    let (action, reverts): (String, Option<uuid::Uuid>) = db(move |c| {
        let r = c.query_one("SELECT action, reverts_batch FROM record_history WHERE app_slug = $1 AND record_id = $2 ORDER BY id DESC LIMIT 1", &[&slug, &rid]).unwrap();
        (r.get(0), r.get(1))
    });
    assert_eq!(action, "delete");
    assert_eq!(reverts, Some(batch));
}

// 4. Undo then undo again restores the original change.
#[tokio::test]
async fn undoing_an_undo_puts_the_original_change_back() {
    let _turn = LOCK.lock().await;
    let f = fixture().await;
    let made = create(&f, &f.editor, json!({ "_table_id": "t1", "title": "A", "score": 1 })).await;
    let id = made["id"].as_str().unwrap().to_string();
    let batch = Uuid::new_v4();
    let (s, _) = patch(&f, &f.editor, &id, 1, json!({ "title": "B", "score": 2 }), Some(&batch)).await;
    assert_eq!(s, StatusCode::OK);

    let (s, first) = undo(&f, &f.editor, &batch.to_string()).await;
    assert_eq!(s, StatusCode::OK, "{first}");
    assert_eq!(get(&f, &f.owner, &id).await.1["data"]["title"], "A");

    let reverting = first["batch_id"].as_str().unwrap().to_string();
    let (s, second) = undo(&f, &f.editor, &reverting).await;
    assert_eq!(s, StatusCode::OK, "{second}");
    let now = get(&f, &f.owner, &id).await.1;
    assert_eq!((now["data"]["title"].clone(), now["data"]["score"].clone()), (json!("B"), json!(2)), "{now}");
}

// 5. A different editor cannot undo someone else's batch. An app admin can.
#[tokio::test]
async fn only_the_actor_or_an_owner_or_admin_can_undo_a_batch() {
    let _turn = LOCK.lock().await;
    let f = fixture().await;
    let (ids, batch) = pasted(&f).await;
    let (status, body) = undo(&f, &f.editor2, &batch.to_string()).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert_eq!(score(&f, &ids[0]).await, 101, "nothing changed");

    let (status, body) = undo(&f, &f.owner, &batch.to_string()).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(score(&f, &ids[0]).await, 1);
}

// 6. Restore of one field to version 2 writes only that field and bumps the version once.
#[tokio::test]
async fn restoring_one_field_to_an_earlier_version_writes_only_that_field() {
    let _turn = LOCK.lock().await;
    let f = fixture().await;
    let made = create(&f, &f.editor, json!({ "_table_id": "t1", "title": "A", "score": 1 })).await;
    let id = made["id"].as_str().unwrap().to_string();
    let (s, _) = patch(&f, &f.editor, &id, 1, json!({ "title": "B", "score": 2 }), None).await;
    assert_eq!(s, StatusCode::OK);
    let (s, _) = patch(&f, &f.editor, &id, 2, json!({ "title": "C", "score": 3 }), None).await;
    assert_eq!(s, StatusCode::OK);

    let (status, body) = send(&f.app, "POST", &format!("/api/v1/apps/{}/records/{id}/restore", f.slug), &f.editor, &[], Some(json!({ "to_version": 2, "fields": ["score"], "version": 3 }))).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["data"]["score"], 2, "the field went back to version 2");
    assert_eq!(body["data"]["title"], "C", "the others did not");
    assert_eq!(body["version"], 4, "one new version");

    let (slug, rid) = (f.slug.clone(), id.clone());
    let (action, changes): (String, Value) = db(move |c| {
        let r = c.query_one("SELECT action, changes FROM record_history WHERE app_slug = $1 AND record_id = $2 ORDER BY id DESC LIMIT 1", &[&slug, &rid]).unwrap();
        (r.get(0), r.get(1))
    });
    assert_eq!(action, "restore");
    assert_eq!(changes.as_object().unwrap().keys().collect::<Vec<_>>(), vec!["score"], "{changes}");

    // The same version check as any save: a stale version is refused.
    let (status, _) = send(&f.app, "POST", &format!("/api/v1/apps/{}/records/{id}/restore", f.slug), &f.editor, &[], Some(json!({ "to_version": 1, "fields": ["score"], "version": 3 }))).await;
    assert_eq!(status, StatusCode::CONFLICT);
}
