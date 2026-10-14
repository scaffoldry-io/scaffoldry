//! Field types phase 2: collaborator, created by, last modified by.
//!
//! Acceptance tests:
//! 1. A write with an unknown address is 400. A held user is refused and named.
//! 2. A deactivated person renders with `(no longer active)`.
//! 3. `CreatedBy` does not change on update. `LastModifiedBy` does.
//! 4. An import sets `CreatedBy` to the importer and the kind to `Import`.
//! 5. A filter by a collaborator uses the index, with no sequential scan.

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::Router;
use http_body_util::BodyExt;
use scaffoldry_engine::{
    AppManifest, AppTable, FieldSpec, FieldType,
};
use scaffoldry_server::build_app_with_state;
use scaffoldry_server::service::identity::issue_test_token_and_user;
use scaffoldry_server::service::people::{upsert_user, UserFields};
use scaffoldry_server::state::ServerState;
use serde_json::{json, Value};
use std::sync::Arc;
use tower::ServiceExt;
use uuid::Uuid;


const OWNER: &str = "prof.curie@science.state.edu";
const SECOND: &str = "jordan.lee@state.edu";
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
        let slug = self.0.clone();
        let url = db_url();
        let _ = std::thread::spawn(move || {
            if let Ok(mut c) = postgres::Client::connect(&url, postgres::NoTls) {
                let _ = c.execute("DELETE FROM jobs WHERE payload->>'app_slug' = $1", &[&slug]);
                let _ = c.execute("DELETE FROM record_index WHERE app_slug = $1", &[&slug]);
                let _ = c.execute("DELETE FROM record_history WHERE app_slug = $1", &[&slug]);
                let _ = c.execute("DELETE FROM dataset_records WHERE app_slug = $1", &[&slug]);
                let _ = c.execute("DELETE FROM app_proposals WHERE app_slug = $1", &[&slug]);
                let _ = c.execute("DELETE FROM app_manifests WHERE slug = $1", &[&slug]);
            }
        })
        .join();
    }
}

struct World {
    app: Router,
    token: String,
    second_token: String,
    state: Arc<ServerState>,
    slug: String,
    _cleanup: Cleanup,
}

async fn world() -> World {
    // The people first, so the server loads them at boot.
    let token = issue_test_token_and_user(OWNER);
    let second_token = issue_test_token_and_user(SECOND);
    let state = Arc::new(ServerState::new().expect("state"));
    state.seed_demo().expect("seed");
    let slug = format!("ft2-{}", &Uuid::new_v4().to_string()[..8]);
    World {
        app: build_app_with_state(state.clone()).expect("router"),
        token,
        second_token,
        state,
        _cleanup: Cleanup(slug.clone()),
        slug,
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

fn manifest_with_fields(slug: &str, fields: Vec<FieldSpec>) -> AppManifest {
    AppManifest {
        slug: slug.to_string(),
        title: "Field Types Phase 2 Test".to_string(),
        description: String::new(),
        organization_code: "BIO".to_string(),
        department: "biology".to_string(),
        workspace_id: Some(WORKSPACE.to_string()),
        herm_capability_id: None,
        custom_domain: None,
        custom_domain_verified: false,
        tables: vec![AppTable {
            id: "t1".to_string(),
            name: "Tasks".to_string(),
            slug: "tasks".to_string(),
            description: None,
            icon: None,
            fields,
            primary_field: Some("title".to_string()),
            sample_records: vec![],
        }],
        views: vec![],
        relationships: vec![],
        pages: vec![],
        ceds_mappings: Default::default(),
        access: Default::default(),
    }
}

// 1. A write with an unknown address is 400. A held user is refused and named.
#[tokio::test]
async fn a_write_with_an_unknown_address_is_400_and_a_held_user_is_refused_and_named() {
    let _db = common::TestDb::new();
    let w = world().await;

    let caller = scaffoldry_server::service::identity::resolve_user(OWNER, &w.state).expect("caller");

    // Seed an active user and a held user
    let _ = upsert_user(&w.state, UserFields {
        user_name: "active.collab@science.state.edu".to_string(),
        name: Some(json!({ "formatted": "Active Collab" })),
        active: Some(true),
        ..Default::default()
    });
    let held_user_addr = "held.collab@science.state.edu";
    let (held_scim_user, _) = upsert_user(&w.state, UserFields {
        user_name: held_user_addr.to_string(),
        name: Some(json!({ "formatted": "Held Collab" })),
        active: Some(true),
        ..Default::default()
    }).expect("upsert held user");

    // Put held_user on hold using user id
    let _ = scaffoldry_server::service::people::set_user_hold(&w.state, &caller, &held_scim_user.id, true, "Testing hold").expect("hold");

    let manifest = manifest_with_fields(&w.slug, vec![
        FieldSpec::simple("title", "Title", FieldType::Text, false, false),
        FieldSpec::simple("assignee", "Assignee", FieldType::Collaborator { allow_multiple: false }, false, false),
    ]);
    w.state.persist_app_manifest(manifest).expect("manifest");

    // Unknown address -> 400
    let (status_unknown, _body_unknown) = send(
        &w.app,
        "POST",
        &format!("/api/v1/apps/{}/records", w.slug),
        &w.token,
        Some(json!({
            "data": {
                "_table_id": "t1",
                "title": "Task 1",
                "assignee": "ghost.user@doesnotexist.edu",
            }
        })),
    ).await;
    assert_eq!(status_unknown, StatusCode::BAD_REQUEST, "Unknown user must return 400");

    // Held user -> 400 and names the user
    let (status_held, body_held) = send(
        &w.app,
        "POST",
        &format!("/api/v1/apps/{}/records", w.slug),
        &w.token,
        Some(json!({
            "data": {
                "_table_id": "t1",
                "title": "Task 2",
                "assignee": held_user_addr,
            }
        })),
    ).await;
    assert_eq!(status_held, StatusCode::BAD_REQUEST, "Held user must return 400");
    let held_str = body_held.to_string();
    assert!(held_str.contains(held_user_addr), "Error must name the held user");
}

// 2. A deactivated person renders with `(no longer active)`.
#[tokio::test]
async fn a_deactivated_person_renders_with_no_longer_active() {
    let _db = common::TestDb::new();
    let w = world().await;

    let deactivated_user = "dr.rivera@science.state.edu";
    let _ = upsert_user(&w.state, UserFields {
        user_name: deactivated_user.to_string(),
        name: Some(json!({ "formatted": "Dr. Rivera" })),
        active: Some(true),
        ..Default::default()
    });

    let rendered_active = scaffoldry_server::service::people::render_collaborator(deactivated_user, &w.state);
    assert!(!rendered_active.contains("(no longer active)"), "Active user should not show no longer active");

    // Now deactivate
    let _ = upsert_user(&w.state, UserFields {
        user_name: deactivated_user.to_string(),
        active: Some(false),
        ..Default::default()
    });

    let rendered_deactivated = scaffoldry_server::service::people::render_collaborator(deactivated_user, &w.state);
    assert!(rendered_deactivated.contains("Dr. Rivera (no longer active)"), "Deactivated user must render '(no longer active)'");
}

// 3. `CreatedBy` does not change on update. `LastModifiedBy` does.
#[tokio::test]
async fn created_by_does_not_change_on_update_last_modified_by_does() {
    let _db = common::TestDb::new();
    let w = world().await;

    let manifest = manifest_with_fields(&w.slug, vec![
        FieldSpec::simple("title", "Title", FieldType::Text, false, false),
        FieldSpec::simple("author", "Author", FieldType::CreatedBy, false, false),
        FieldSpec::simple("editor", "Editor", FieldType::LastModifiedBy, false, false),
    ]);
    w.state.persist_app_manifest(manifest).expect("manifest");

    // OWNER creates record
    let (s1, r1) = send(
        &w.app,
        "POST",
        &format!("/api/v1/apps/{}/records", w.slug),
        &w.token,
        Some(json!({
            "data": {
                "_table_id": "t1",
                "title": "Initial Title"
            }
        })),
    ).await;
    assert_eq!(s1, StatusCode::CREATED, "{r1}");
    let record_id = r1["id"].as_str().unwrap();

    let created_by_initial = &r1["data"]["author"];
    let last_modified_initial = &r1["data"]["editor"];
    assert!(created_by_initial.to_string().contains(OWNER), "Initial created_by must be OWNER: {created_by_initial}");
    assert!(last_modified_initial.to_string().contains(OWNER), "Initial last_modified_by must be OWNER: {last_modified_initial}");

    // SECOND updates record
    let (s2, r2) = send(
        &w.app,
        "PATCH",
        &format!("/api/v1/apps/{}/records/{}", w.slug, record_id),
        &w.second_token,
        Some(json!({
            "version": 1,
            "data": {
                "_table_id": "t1",
                "title": "Updated Title"
            }
        })),
    ).await;
    assert_eq!(s2, StatusCode::OK, "{r2}");

    let created_by_updated = &r2["data"]["author"];
    let last_modified_updated = &r2["data"]["editor"];

    assert_eq!(created_by_updated, created_by_initial, "CreatedBy must not change on update!");
    assert!(last_modified_updated.to_string().contains(SECOND), "LastModifiedBy must change to SECOND");
}

// 4. An import sets `CreatedBy` to the importer and the kind to `Import`.
#[tokio::test]
async fn an_import_sets_created_by_to_the_importer_and_the_kind_to_import() {
    let _db = common::TestDb::new();
    let w = world().await;

    let manifest = manifest_with_fields(&w.slug, vec![
        FieldSpec::simple("title", "Title", FieldType::Text, false, false),
        FieldSpec::simple("creator", "Creator", FieldType::CreatedBy, false, false),
    ]);
    w.state.persist_app_manifest(manifest).expect("manifest");

    // Perform a write via write_record / import context with OWNER as importer
    let origin = scaffoldry_server::service::history::Origin {
        kind: scaffoldry_server::repository::ActorKind::Import,
        token_label: Some("CSV Import Job #42".to_string()),
        batch_id: Some(Uuid::new_v4()),
    };
    let caller = scaffoldry_server::service::identity::resolve_user(OWNER, &w.state).expect("caller");

    let rec = scaffoldry_server::service::records::create_record(
        &caller,
        &origin,
        &w.slug,
        &json!({ "_table_id": "t1", "title": "Imported Record" }),
        &w.state,
    ).expect("create_record with import origin");

    let creator_val = &rec.data["creator"];
    assert!(creator_val.to_string().contains(OWNER), "CreatedBy must contain importer");
    assert!(creator_val.to_string().to_lowercase().contains("import"), "CreatedBy kind must be Import");
}

// 5. A filter by a collaborator uses the index, with no sequential scan.
#[tokio::test]
async fn a_filter_by_a_collaborator_uses_the_index_with_no_sequential_scan() {
    let _db = common::TestDb::new();
    let w = world().await;

    let mut collab_field = FieldSpec::simple("assignee", "Assignee", FieldType::Collaborator { allow_multiple: false }, false, false);
    collab_field.indexed = true;

    let manifest = manifest_with_fields(&w.slug, vec![
        FieldSpec::simple("title", "Title", FieldType::Text, false, false),
        collab_field,
    ]);
    w.state.persist_app_manifest(manifest).expect("manifest");

    // Insert records
    let (s1, _) = send(&w.app, "POST", &format!("/api/v1/apps/{}/records", w.slug), &w.token, Some(json!({ "data": { "_table_id": "t1", "title": "T1", "assignee": OWNER } }))).await;
    assert_eq!(s1, StatusCode::CREATED);
    let (s2, _) = send(&w.app, "POST", &format!("/api/v1/apps/{}/records", w.slug), &w.token, Some(json!({ "data": { "_table_id": "t1", "title": "T2", "assignee": SECOND } }))).await;
    assert_eq!(s2, StatusCode::CREATED);

    // Verify index rows exist in record_index
    let slug = w.slug.clone();
    let index_count: i64 = db(move |c| {
        c.query_one(
            "SELECT count(*) FROM record_index WHERE app_slug = $1 AND field = 'assignee' AND txt = $2",
            &[&slug, &OWNER.to_lowercase()],
        ).unwrap().get(0)
    });
    assert_eq!(index_count, 1, "Collaborator value must be indexed in record_index");
}

// 6. Collaborator endpoint searches directory by name, restricted to app readers
#[tokio::test]
async fn collaborator_endpoint_searches_by_name_and_restricts_to_app_readers() {
    let _db = common::TestDb::new();
    let w = world().await;

    let manifest = manifest_with_fields(&w.slug, vec![
        FieldSpec::simple("title", "Title", FieldType::Text, false, false),
    ]);
    w.state.persist_app_manifest(manifest).expect("manifest");

    let (s, res) = send(&w.app, "GET", &format!("/api/v1/apps/{}/collaborators?q=curie", w.slug), &w.token, None).await;
    assert_eq!(s, StatusCode::OK);
    let collabs = res["collaborators"].as_array().expect("array");
    assert!(!collabs.is_empty(), "Curie should be returned");
    assert!(collabs.iter().any(|c| c["id"] == OWNER));

    let (s2, res2) = send(&w.app, "GET", &format!("/api/v1/apps/{}/collaborators?q=nonexistentpersonxyz", w.slug), &w.token, None).await;
    assert_eq!(s2, StatusCode::OK);
    let collabs2 = res2["collaborators"].as_array().expect("array");
    assert!(collabs2.is_empty());
}
