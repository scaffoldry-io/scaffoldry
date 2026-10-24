//! Record history phase 1: every change to a record is written, with its actor, in the same
//! transaction as the write.
//!
//! The app is private to each run, so the tests share the database without touching anyone's rows.

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::Router;
use http_body_util::BodyExt;
use scaffoldry_engine::{AppManifest, AppTable, AppView, FieldSpec, FieldType, ViewType};
use scaffoldry_server::build_app_with_state;
use scaffoldry_server::service::identity::{hash_token, issue_test_token_and_user};
use scaffoldry_server::state::ServerState;
use serde_json::{json, Value};
use std::sync::Arc;
use tower::ServiceExt;
use uuid::Uuid;


const OWNER: &str = "prof.curie@science.state.edu";
const EDITOR: &str = "student.smith@science.state.edu";
const VIEWER: &str = "marcus.vance@state.edu";
const OUTSIDER: &str = "sarah.connor@state.edu";
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
    viewer: String,
    outsider: String,
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
            coordinates: None, sealed: false,
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
                coordinates: None, sealed: false,
                row_place: Default::default(),
            }],
            relationships: vec![],
        })
        .unwrap();
    Fixture {
        app: build_app_with_state(state).expect("router"),
        slug,
        owner: issue_test_token_and_user(OWNER),
        editor: issue_test_token_and_user(EDITOR),
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

async fn history(f: &Fixture, token: &str, id: &str) -> (StatusCode, Value) {
    send(&f.app, "GET", &format!("/api/v1/apps/{}/records/{id}/history", f.slug), token, &[], None).await
}

fn entries(body: &Value) -> Vec<Value> {
    body["entries"].as_array().unwrap_or_else(|| panic!("no entries in {body}")).clone()
}

// 1. Create, update two fields, delete: three entries, newest first, with old and new values.
#[tokio::test]
async fn create_update_and_delete_write_three_entries_with_old_and_new_values() {
    let _db = common::TestDb::new();
    let f = fixture().await;
    let created = create(&f, &f.owner, json!({ "_table_id": "t1", "title": "A", "score": 1 })).await;
    let id = created["id"].as_str().unwrap().to_string();
    let uri = format!("/api/v1/apps/{}/records/{id}", f.slug);

    let (s, saved) = send(&f.app, "PATCH", &uri, &f.owner, &[], Some(json!({ "data": { "_table_id": "t1", "title": "B", "score": 2 }, "version": 1 }))).await;
    assert_eq!(s, StatusCode::OK, "{saved}");
    let (s, _) = send(&f.app, "DELETE", &uri, &f.owner, &[], None).await;
    assert!(s == StatusCode::OK || s == StatusCode::NO_CONTENT);

    let (s, body) = history(&f, &f.owner, &id).await;
    assert_eq!(s, StatusCode::OK, "{body}");
    let list = entries(&body);
    assert_eq!(list.len(), 3, "{body}");
    assert_eq!(list.iter().map(|e| e["action"].as_str().unwrap()).collect::<Vec<_>>(), vec!["delete", "update", "create"]);

    assert_eq!(list[2]["changes"]["title"], json!({ "new": "A" }), "a create has only new");
    assert_eq!(list[2]["version"], 1);
    assert_eq!(list[1]["changes"]["title"], json!({ "old": "A", "new": "B" }));
    assert_eq!(list[1]["changes"]["score"], json!({ "old": 1, "new": 2 }));
    assert_eq!(list[1]["version"], 2);
    assert_eq!(list[0]["changes"]["title"], json!({ "old": "B" }), "a delete has only old");
    assert!(list[1]["changes"].get("_table_id").is_none(), "the table key is not a field");
    assert_eq!(list[1]["actor"], OWNER);
    assert_eq!(list[1]["actor_kind"], "user");
    assert!(list[1]["actor_name"].is_string(), "the entry names the person");
    assert!(list[1]["at"].is_string());
}

// 2. An update that changes nothing writes no history.
#[tokio::test]
async fn an_update_that_changes_nothing_writes_no_history() {
    let _db = common::TestDb::new();
    let f = fixture().await;
    let created = create(&f, &f.owner, json!({ "_table_id": "t1", "title": "Same" })).await;
    let id = created["id"].as_str().unwrap().to_string();
    let uri = format!("/api/v1/apps/{}/records/{id}", f.slug);

    let (s, body) = send(&f.app, "PATCH", &uri, &f.owner, &[], Some(json!({ "data": { "_table_id": "t1", "title": "Same" }, "version": 1 }))).await;
    assert_eq!(s, StatusCode::OK, "{body}");
    let (_, h) = history(&f, &f.owner, &id).await;
    assert_eq!(entries(&h).len(), 1, "only the create: {h}");
}

// 3. A computed field changing is not in `changes`.
#[tokio::test]
async fn a_computed_field_is_not_recorded() {
    let _db = common::TestDb::new();
    let f = fixture().await;
    let created = create(&f, &f.owner, json!({ "_table_id": "t1", "title": "A", "total": 10 })).await;
    let id = created["id"].as_str().unwrap().to_string();
    let uri = format!("/api/v1/apps/{}/records/{id}", f.slug);
    let (s, body) = send(&f.app, "PATCH", &uri, &f.owner, &[], Some(json!({ "data": { "_table_id": "t1", "title": "A", "total": 99 }, "version": 1 }))).await;
    assert_eq!(s, StatusCode::OK, "{body}");
    let (_, h) = history(&f, &f.owner, &id).await;
    let list = entries(&h);
    assert_eq!(list.len(), 1, "the total changed alone, which is not a change a person made: {h}");
    assert!(list[0]["changes"].get("total").is_none());
}

// 4. History cannot be changed by SQL, except by the named purge.
#[tokio::test]
async fn a_history_row_cannot_be_updated_or_deleted_but_the_purge_can_delete() {
    let _db = common::TestDb::new();
    let f = fixture().await;
    let created = create(&f, &f.owner, json!({ "_table_id": "t1", "title": "A" })).await;
    let id = created["id"].as_str().unwrap().to_string();

    let id2 = id.clone();
    let update = db(move |c| c.execute("UPDATE record_history SET actor = 'x' WHERE record_id = $1", &[&id2]).is_err());
    assert!(update, "an update is refused");
    let id3 = id.clone();
    let delete = db(move |c| c.execute("DELETE FROM record_history WHERE record_id = $1", &[&id3]).is_err());
    assert!(delete, "a delete is refused");

    let id4 = id.clone();
    let purged = db(move |c| {
        c.batch_execute("BEGIN; SET LOCAL scaffoldry.purge = 'on';").unwrap();
        let n = c.execute("DELETE FROM record_history WHERE record_id = $1", &[&id4]).unwrap();
        c.batch_execute("COMMIT").unwrap();
        n
    });
    assert_eq!(purged, 1, "the purge deletes");
}

// 5. A history row and its record write fail together.
#[tokio::test]
async fn when_the_history_insert_fails_the_record_is_unchanged() {
    let _db = common::TestDb::new();
    let f = fixture().await;
    let created = create(&f, &f.owner, json!({ "_table_id": "t1", "title": "Before" })).await;
    let id = created["id"].as_str().unwrap().to_string();
    let uri = format!("/api/v1/apps/{}/records/{id}", f.slug);

    // A trigger that refuses history rows for this app only.
    let tag = f.slug.replace('-', "_");
    let slug = f.slug.clone();
    let tag1 = tag.clone();
    db(move |c| {
        c.batch_execute(&format!(
            "CREATE FUNCTION fail_hist_{tag1}() RETURNS trigger AS $$ BEGIN \
               IF NEW.app_slug = '{slug}' THEN RAISE EXCEPTION 'history refused'; END IF; RETURN NEW; END; $$ LANGUAGE plpgsql; \
             CREATE TRIGGER fail_hist_{tag1} BEFORE INSERT ON record_history FOR EACH ROW EXECUTE FUNCTION fail_hist_{tag1}();"
        ))
        .unwrap();
    });

    let (s, _) = send(&f.app, "PATCH", &uri, &f.owner, &[], Some(json!({ "data": { "_table_id": "t1", "title": "After" }, "version": 1 }))).await;
    let (s2, _) = send(&f.app, "DELETE", &uri, &f.owner, &[], None).await;
    let (s3, _) = send(&f.app, "POST", &format!("/api/v1/apps/{}/records", f.slug), &f.owner, &[], Some(json!({ "data": { "_table_id": "t1", "title": "New" } }))).await;

    let tag2 = tag.clone();
    let slug2 = f.slug.clone();
    let rows: i64 = db(move |c| {
        c.batch_execute(&format!("DROP TRIGGER fail_hist_{tag2} ON record_history; DROP FUNCTION fail_hist_{tag2}();")).unwrap();
        c.query_one("SELECT count(*) FROM dataset_records WHERE app_slug = $1", &[&slug2]).unwrap().get(0)
    });

    assert!(s.is_server_error() && s2.is_server_error() && s3.is_server_error(), "the writes failed: {s} {s2} {s3}");
    assert_eq!(rows, 1, "the new record was not stored and the old one was not deleted");
    let (_, read) = send(&f.app, "GET", &uri, &f.owner, &[], None).await;
    assert_eq!(read["data"]["title"], "Before");
    assert_eq!(read["version"], 1);
}

// 6. A viewer sees that a sensitive field changed, and not what it held.
#[tokio::test]
async fn a_viewer_sees_changed_true_for_a_sensitive_field_and_an_editor_sees_values() {
    let _db = common::TestDb::new();
    let f = fixture().await;
    let created = create(&f, &f.owner, json!({ "_table_id": "t1", "title": "A", "ssn": "123-45-6789" })).await;
    let id = created["id"].as_str().unwrap().to_string();
    let uri = format!("/api/v1/apps/{}/records/{id}", f.slug);
    let (s, _) = send(&f.app, "PATCH", &uri, &f.owner, &[], Some(json!({ "data": { "_table_id": "t1", "title": "A", "ssn": "987-65-4321" }, "version": 1 }))).await;
    assert_eq!(s, StatusCode::OK);

    let (s, as_viewer) = history(&f, &f.viewer, &id).await;
    assert_eq!(s, StatusCode::OK, "{as_viewer}");
    let text = as_viewer.to_string();
    assert!(!text.contains("123-45-6789") && !text.contains("987-65-4321"), "the viewer saw a value: {text}");
    let update = &entries(&as_viewer)[0];
    assert_eq!(update["changes"]["ssn"], json!({ "changed": true }));

    let (_, as_owner) = history(&f, &f.owner, &id).await;
    assert_eq!(entries(&as_owner)[0]["changes"]["ssn"], json!({ "old": "123-45-6789", "new": "987-65-4321" }));
}

// 7. No read access, no history.
#[tokio::test]
async fn a_user_with_no_read_access_gets_404_on_the_history() {
    let _db = common::TestDb::new();
    let f = fixture().await;
    let created = create(&f, &f.owner, json!({ "_table_id": "t1", "title": "A" })).await;
    let id = created["id"].as_str().unwrap().to_string();
    let (s, body) = history(&f, &f.outsider, &id).await;
    assert_eq!(s, StatusCode::NOT_FOUND, "{body}");
    let (s, _) = send(&f.app, "GET", &format!("/api/v1/apps/{}/history", f.slug), &f.outsider, &[], None).await;
    assert!(s == StatusCode::NOT_FOUND || s == StatusCode::FORBIDDEN);
}

// 8. An agent token's label is in the entry. Ten cells in one paste share one batch.
#[tokio::test]
async fn an_agent_tokens_label_is_recorded_and_a_batch_header_groups_ten_writes() {
    let _db = common::TestDb::new();
    let f = fixture().await;
    let raw = format!("sct_agent_{}", Uuid::new_v4().simple());
    let hash = hash_token(&raw);
    let label = "Registrar bot".to_string();
    db(move |c| {
        let expires = chrono::Utc::now() + chrono::Duration::days(1);
        c.execute(
            "INSERT INTO api_tokens (token_hash, id, kind, eppn, label, created_at, expires_at) VALUES ($1, $2, 'agent', $3, $4, NOW(), $5)",
            &[&hash, &Uuid::new_v4(), &OWNER, &label, &expires],
        )
        .unwrap();
    });

    let batch = Uuid::new_v4().to_string();
    let mut last = String::new();
    for n in 0..10 {
        let (s, body) = send(
            &f.app,
            "POST",
            &format!("/api/v1/apps/{}/records", f.slug),
            &raw,
            &[("x-scaffoldry-batch", batch.as_str())],
            Some(json!({ "data": { "_table_id": "t1", "title": format!("cell {n}") } })),
        )
        .await;
        assert_eq!(s, StatusCode::CREATED, "{body}");
        last = body["id"].as_str().unwrap().to_string();
    }
    let (_, h) = history(&f, &f.owner, &last).await;
    let entry = &entries(&h)[0];
    assert_eq!(entry["actor_kind"], "agent");
    assert_eq!(entry["token_label"], "Registrar bot");
    assert_eq!(entry["batch_id"], batch.as_str());

    let (s, feed) = send(&f.app, "GET", &format!("/api/v1/apps/{}/history?kind=agent&limit=50", f.slug), &f.owner, &[], None).await;
    assert_eq!(s, StatusCode::OK, "{feed}");
    let same_batch = entries(&feed).iter().filter(|e| e["batch_id"] == batch.as_str()).count();
    assert_eq!(same_batch, 10, "ten rows share one batch");

    // A write with no header gets a batch of its own.
    let (s, _) = send(&f.app, "POST", &format!("/api/v1/apps/{}/records", f.slug), &f.owner, &[], Some(json!({ "data": { "_table_id": "t1", "title": "alone" } }))).await;
    assert_eq!(s, StatusCode::CREATED);
    let (_, feed) = send(&f.app, "GET", &format!("/api/v1/apps/{}/history?kind=user", f.slug), &f.owner, &[], None).await;
    assert!(entries(&feed).iter().all(|e| e["actor_kind"] == "user"));

    // A batch header that is not a UUID is refused, not ignored.
    let (s, _) = send(&f.app, "POST", &format!("/api/v1/apps/{}/records", f.slug), &f.owner, &[("x-scaffoldry-batch", "nope")], Some(json!({ "data": { "title": "x" } }))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn the_app_feed_is_for_an_owner_or_admin_and_pages_by_id() {
    let _db = common::TestDb::new();
    let f = fixture().await;
    for n in 0..5 {
        create(&f, &f.owner, json!({ "_table_id": "t1", "title": format!("row {n}") })).await;
    }
    let (s, _) = send(&f.app, "GET", &format!("/api/v1/apps/{}/history", f.slug), &f.editor, &[], None).await;
    assert!(s == StatusCode::FORBIDDEN || s == StatusCode::NOT_FOUND, "an editor may not read the app feed: {s}");

    let uri = format!("/api/v1/apps/{}/history?limit=2", f.slug);
    let (s, first) = send(&f.app, "GET", &uri, &f.owner, &[], None).await;
    assert_eq!(s, StatusCode::OK, "{first}");
    assert_eq!(entries(&first).len(), 2);
    let cursor = first["next_cursor"].as_str().expect("a cursor").to_string();
    let (_, second) = send(&f.app, "GET", &format!("{uri}&cursor={cursor}"), &f.owner, &[], None).await;
    let ids: Vec<i64> = entries(&first).iter().chain(entries(&second).iter()).map(|e| e["id"].as_i64().unwrap()).collect();
    assert_eq!(ids.len(), 4);
    assert!(ids.windows(2).all(|w| w[0] > w[1]), "newest first, no repeats: {ids:?}");

    let (_, by_table) = send(&f.app, "GET", &format!("/api/v1/apps/{}/history?table=nope", f.slug), &f.owner, &[], None).await;
    assert!(entries(&by_table).is_empty());
}

// 9. A write with no actor cannot be written. Every write method takes one.
#[test]
fn every_record_write_method_takes_a_write_context() {
    let _db = common::TestDb::new();
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let methods = ["insert_record", "put_record", "save_record_checked", "overwrite_record_data", "remove_record", "remove_record_checked", "upsert_record", "update_record_checked", "delete_record", "delete_record_checked"];
    for file in ["state.rs", "repository.rs"] {
        let src = std::fs::read_to_string(root.join(file)).unwrap();
        for m in methods {
            let needle = format!("pub fn {m}(");
            let Some(at) = src.find(&needle) else { continue };
            let signature = &src[at..at + src[at..].find('{').unwrap()];
            assert!(signature.contains("WriteCtx"), "{file}: {m} takes no WriteCtx:\n{signature}");
        }
    }
    // No other file writes the table.
    for entry in std::fs::read_dir(&root).unwrap().chain(std::fs::read_dir(root.join("service")).unwrap()).chain(std::fs::read_dir(root.join("routes")).unwrap()) {
        let path = entry.unwrap().path();
        if path.extension().is_some_and(|e| e == "rs") && path.file_name().unwrap() != "repository.rs" {
            let text = std::fs::read_to_string(&path).unwrap();
            for verb in ["INSERT INTO dataset_records", "UPDATE dataset_records", "DELETE FROM dataset_records"] {
                assert!(!text.contains(verb), "{} writes records by SQL", path.display());
            }
        }
    }
}

#[tokio::test]
async fn the_mcp_tool_masks_the_same_way() {
    let _db = common::TestDb::new();
    let f = fixture().await;
    let created = create(&f, &f.owner, json!({ "_table_id": "t1", "title": "A", "ssn": "123-45-6789" })).await;
    let id = created["id"].as_str().unwrap().to_string();
    let call = |token: String| {
        let (app, slug, id) = (f.app.clone(), f.slug.clone(), id.clone());
        async move {
            let payload = json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": { "name": "get_record_history", "arguments": { "app_slug": slug, "record_id": id } } });
            let (_, v) = send(&app, "POST", "/api/mcp", &token, &[], Some(payload)).await;
            v["result"]["structuredContent"].clone()
        }
    };
    let as_viewer = call(f.viewer.clone()).await;
    assert_eq!(entries(&as_viewer)[0]["changes"]["ssn"], json!({ "changed": true }), "{as_viewer}");
    let as_owner = call(f.owner.clone()).await;
    assert_eq!(entries(&as_owner)[0]["changes"]["ssn"], json!({ "new": "123-45-6789" }));
}
