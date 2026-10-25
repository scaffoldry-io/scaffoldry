//! Access model phase 9: full impersonation is for synthetic people only.
//!
//! A real person can be seen as (read-only) but never acted as. A synthetic person can be acted as,
//! and the history says who held the keyboard.

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::Router;
use http_body_util::BodyExt;
use scaffoldry_engine::{AppManifest, AppTable, FieldSpec, FieldType};
use scaffoldry_server::build_app_with_state;
use scaffoldry_server::service::identity::{issue_test_token_and_user, resolve_user};
use scaffoldry_server::state::{CollaboratorRecord, ServerState};
use serde_json::{json, Value};
use std::sync::Arc;
use tower::ServiceExt;
use uuid::Uuid;

const WORKSPACE: &str = "ws-bio-lab";
const ADMIN: &str = "jordan.lee@state.edu";

fn table() -> AppTable {
    AppTable {
        id: "notes".into(),
        name: "notes".into(),
        slug: "notes".into(),
        description: None,
        icon: None,
        fields: vec![FieldSpec::simple("name", "name", FieldType::Text, false, false)],
        primary_field: Some("name".into()),
        sample_records: vec![],
        coordinates: None,
        sealed: false,
        row_place: Default::default(),
    }
}

fn manifest(slug: &str) -> AppManifest {
    AppManifest {
        slug: slug.into(),
        title: "Impersonation".into(),
        description: String::new(),
        organization_code: "BIO".into(),
        department: "biology".into(),
        workspace_id: Some(WORKSPACE.into()),
        herm_capability_id: None,
        coordinates: None,
        sealed: false,
        custom_domain: None,
        custom_domain_verified: false,
        tables: vec![table()],
        relationships: vec![],
        views: vec![],
        ceds_mappings: Default::default(),
        pages: vec![],
        access: Default::default(),
    }
}

fn editor(state: &ServerState, eppn: &str) {
    state.collaborators.write().unwrap().entry(WORKSPACE.to_string()).or_default().push(CollaboratorRecord {
        id: Uuid::new_v4().to_string(),
        workspace_id: WORKSPACE.to_string(),
        eppn: eppn.to_string(),
        name: eppn.to_string(),
        role: "editor".to_string(),
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
    tag: String,
    admin: String,
}

impl Drop for World {
    fn drop(&mut self) {
        let (slug, url) = (self.slug.clone(), scaffoldry_server::testing::database_url());
        let _ = std::thread::spawn(move || {
            if let Ok(mut c) = postgres::Client::connect(&url, postgres::NoTls) {
                let _ = c.execute("DELETE FROM record_history WHERE app_slug = $1", &[&slug]);
                let _ = c.execute("DELETE FROM dataset_records WHERE app_slug = $1", &[&slug]);
            }
        })
        .join();
    }
}

fn world() -> World {
    let state = Arc::new(ServerState::new().expect("state"));
    state.seed_demo().expect("seed");
    let tag = Uuid::new_v4().to_string()[..8].to_string();
    let slug = format!("imp-{tag}");
    state.engine.write().unwrap().register_manifest(manifest(&slug)).unwrap();
    World { app: build_app_with_state(state.clone()).expect("router"), state, slug, tag, admin: issue_test_token_and_user(ADMIN) }
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

async fn impersonate(w: &World, target: &str) -> (StatusCode, Value) {
    send(&w.app, "POST", "/api/v1/auth/impersonate", &w.admin, Some(json!({ "target_eppn": target }))).await
}

fn history_of(slug: &str) -> Vec<(String, Option<String>)> {
    let (slug, url) = (slug.to_string(), scaffoldry_server::testing::database_url());
    std::thread::spawn(move || {
        let mut c = postgres::Client::connect(&url, postgres::NoTls).unwrap();
        c.query("SELECT actor, acted_by FROM record_history WHERE app_slug = $1 ORDER BY id", &[&slug])
            .unwrap()
            .iter()
            .map(|r| (r.get::<_, String>(0), r.get::<_, Option<String>>(1)))
            .collect()
    })
    .join()
    .unwrap()
}

// 1. A real person is refused. A synthetic person works, and a write records both addresses.
#[tokio::test]
async fn a_real_person_cannot_be_impersonated() {
    let _db = common::TestDb::new();
    let w = world();
    let real = format!("real-{}@state.edu", w.tag);
    issue_test_token_and_user(&real);
    let (s, body) = impersonate(&w, &real).await;
    assert_eq!(s, StatusCode::FORBIDDEN, "{body}");
    assert_eq!(body["code"], "impersonation_not_synthetic", "{body}");
    assert!(body["remedy"].as_str().unwrap_or("").contains("view-as"), "the remedy names view-as: {body}");
}

#[tokio::test]
async fn a_write_under_a_synthetic_person_records_both_addresses() {
    let _db = common::TestDb::new();
    let w = world();
    let synth = format!("synth_{}@synthetic.invalid", w.tag);
    issue_test_token_and_user(&synth);
    editor(&w.state, &synth);

    let (s, body) = impersonate(&w, &synth).await;
    assert_eq!(s, StatusCode::OK, "{body}");
    let token = body["token"].as_str().unwrap().to_string();

    let data = json!({ "_table_id": "notes", "name": "written under impersonation" });
    let (s, body) = send(&w.app, "POST", &format!("/api/v1/apps/{}/records", w.slug), &token, Some(json!({ "data": data }))).await;
    assert_eq!(s, StatusCode::CREATED, "{body}");

    assert_eq!(history_of(&w.slug), vec![(synth.clone(), Some(ADMIN.to_string()))], "the target acted, the administrator held the keyboard");

    // A write made by the person themselves names no one else.
    let own = issue_test_token_and_user(&synth);
    let data = json!({ "_table_id": "notes", "name": "written by the person" });
    let (s, body) = send(&w.app, "POST", &format!("/api/v1/apps/{}/records", w.slug), &own, Some(json!({ "data": data }))).await;
    assert_eq!(s, StatusCode::CREATED, "{body}");
    assert_eq!(history_of(&w.slug).last().cloned(), Some((synth, None)));
}

// 2. View-as of a real person still works and is still read-only.
#[tokio::test]
async fn view_as_of_a_real_person_is_unchanged() {
    let _db = common::TestDb::new();
    let w = world();
    let real = format!("real-{}@state.edu", w.tag);
    issue_test_token_and_user(&real);
    editor(&w.state, &real);
    let (s, body) = send(&w.app, "POST", &format!("/api/v1/apps/{}/view-as", w.slug), &w.admin, Some(json!({ "eppn": real }))).await;
    assert_eq!(s, StatusCode::OK, "{body}");
    let token = body["token"].as_str().unwrap().to_string();
    let data = json!({ "_table_id": "notes", "name": "x" });
    let (s, body) = send(&w.app, "POST", &format!("/api/v1/apps/{}/records", w.slug), &token, Some(json!({ "data": data }))).await;
    assert_eq!((s, body["code"].as_str()), (StatusCode::FORBIDDEN, Some("view_as_read_only")), "{body}");
}

// 3. No address is turned into a persona anywhere in the server's source.
#[test]
fn no_persona_is_guessed_from_an_address() {
    let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let old = [
        "default_persona_attributes",
        "Central Enterprise Administrator",
        "Professor & Lab Director",
        "Department Chair & Professor",
        "Senior Research Administrator",
        "Professor of Physics",
        "Graduate Research Assistant",
        "Department Administrator",
        "Principal Investigator\".to_string(),\n            \"faculty",
        "Editor User",
        "Owner User",
        "Admin User",
    ];
    let mut hits = vec![];
    let mut stack = vec![src];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).unwrap().flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().is_some_and(|e| e == "rs") {
                let text = std::fs::read_to_string(&path).unwrap();
                for needle in old {
                    if text.contains(needle) {
                        hits.push(format!("{}: {needle}", path.display()));
                    }
                }
            }
        }
    }
    assert!(hits.is_empty(), "guessed personas remain: {hits:#?}");
}

// 4. A person with no directory record is a member with no department, whatever the address says.
#[test]
fn an_address_with_curie_in_it_and_no_record_is_a_member() {
    let _db = common::TestDb::new();
    let w = world();
    let eppn = format!("curie-{}@nowhere.example", w.tag);
    let user = resolve_user(&eppn, &w.state).expect("a person with no record still resolves");
    assert_eq!(user.affiliation, "member");
    assert_eq!(user.department, "");
    assert_eq!(user.name, eppn);
}
