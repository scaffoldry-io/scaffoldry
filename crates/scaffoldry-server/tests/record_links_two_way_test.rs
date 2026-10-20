//! Links phase 2: two-way links, and what delete does to them.
//!
//! Each test uses a private app with a fresh slug and deletes its rows when it ends.

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::Router;
use http_body_util::BodyExt;
use scaffoldry_engine::{AppManifest, AppTable, FieldSpec, FieldType, LinkCardinality, LinkOnDelete};
use scaffoldry_server::build_app_with_state;
use scaffoldry_server::service::identity::issue_test_token_and_user;
use scaffoldry_server::state::ServerState;
use serde_json::{json, Value};
use std::sync::Arc;
use tower::ServiceExt;
use uuid::Uuid;

const OWNER: &str = "prof.curie@science.state.edu";
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
                let _ = c.execute("DELETE FROM record_links WHERE app_slug = $1", &[&slug]);
                let _ = c.execute("DELETE FROM dataset_records WHERE app_slug = $1", &[&slug]);
            }
        })
        .join();
    }
}

fn text(name: &str) -> FieldSpec {
    FieldSpec::simple(name, name, FieldType::Text, false, false)
}

fn link(name: &str, target: &str, cardinality: LinkCardinality, related: &str) -> FieldSpec {
    let mut f = FieldSpec::simple(name, name, FieldType::Link, false, false);
    f.link_target_table = Some(target.to_string());
    f.link_cardinality = cardinality;
    f.link_related_field = Some(related.to_string());
    f
}

fn table(id: &str, fields: Vec<FieldSpec>, primary: &str) -> AppTable {
    AppTable {
        id: id.to_string(),
        name: if id == "apps" { "Applications".to_string() } else { id.to_string() },
        slug: id.to_string(),
        description: None,
        icon: None,
        fields,
        primary_field: Some(primary.to_string()),
        sample_records: vec![],
        coordinates: None,
        row_place: Default::default(),
    }
}

/// `apps.program` (One) and `programs.applications` (Many) mirror each other. `students.courses` and
/// `courses.students` are many-to-many. `restrict` makes `apps.program` refuse the delete of a program.
fn manifest(slug: &str, restrict: bool) -> AppManifest {
    let mut program = link("program", "programs", LinkCardinality::One, "applications");
    if restrict {
        program.link_on_delete = LinkOnDelete::Restrict;
    }
    AppManifest {
        slug: slug.to_string(),
        title: "Links".to_string(),
        description: String::new(),
        organization_code: "BIO".to_string(),
        department: "biology".to_string(),
        workspace_id: Some(WORKSPACE.to_string()),
        herm_capability_id: None,
        coordinates: None,
        custom_domain: None,
        custom_domain_verified: false,
        tables: vec![
            table("apps", vec![text("applicant"), program], "applicant"),
            table("programs", vec![text("code"), link("applications", "apps", LinkCardinality::Many, "program")], "code"),
            table("students", vec![text("name"), link("courses", "courses", LinkCardinality::Many, "students")], "name"),
            table("courses", vec![text("title"), link("students", "students", LinkCardinality::Many, "courses")], "title"),
        ],
        relationships: vec![],
        views: vec![],
        ceds_mappings: Default::default(),
        pages: vec![],
        access: Default::default(),
    }
}

struct World {
    app: Router,
    token: String,
    slug: String,
    _cleanup: Cleanup,
}

async fn world(restrict: bool) -> World {
    let state = Arc::new(ServerState::new().expect("state"));
    state.seed_demo().expect("seed");
    let slug = format!("lnk-{}", &Uuid::new_v4().to_string()[..8]);
    state.engine.write().unwrap().register_manifest(manifest(&slug, restrict)).unwrap();
    World {
        app: build_app_with_state(state.clone()).expect("router"),
        token: issue_test_token_and_user(OWNER),
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

async fn try_create(w: &World, table: &str, mut data: Value) -> (StatusCode, Value) {
    data["_table_id"] = json!(table);
    send(&w.app, "POST", &format!("/api/v1/apps/{}/records", w.slug), &w.token, Some(json!({ "data": data }))).await
}

async fn create(w: &World, table: &str, data: Value) -> Value {
    let (s, body) = try_create(w, table, data).await;
    assert_eq!(s, StatusCode::CREATED, "{body}");
    body
}

async fn try_patch(w: &World, id: &str, version: i64, data: Value) -> (StatusCode, Value) {
    send(&w.app, "PATCH", &format!("/api/v1/apps/{}/records/{id}", w.slug), &w.token, Some(json!({ "data": data, "version": version }))).await
}

async fn patch(w: &World, id: &str, version: i64, data: Value) -> Value {
    let (s, body) = try_patch(w, id, version, data).await;
    assert_eq!(s, StatusCode::OK, "{body}");
    body
}

fn id_of(v: &Value) -> String {
    v["id"].as_str().unwrap().to_string()
}

/// The link rows of an app, ordered: `(from, field, to, ord)`.
type LinkRow = (String, String, String, i16);

fn link_rows(slug: &str) -> Vec<LinkRow> {
    let slug = slug.to_string();
    db(move |c| {
        c.query("SELECT from_record, field, to_record, ord FROM record_links WHERE app_slug = $1 ORDER BY from_record, field, ord, to_record", &[&slug])
            .unwrap()
            .iter()
            .map(|r| (r.get(0), r.get(1), r.get(2), r.get(3)))
            .collect()
    })
}


async fn get(w: &World, id: &str) -> Value {
    let (s, body) = send(&w.app, "GET", &format!("/api/v1/apps/{}/records/{id}", w.slug), &w.token, None).await;
    assert_eq!(s, StatusCode::OK, "{body}");
    body
}

async fn delete(w: &World, id: &str) -> (StatusCode, Value) {
    send(&w.app, "DELETE", &format!("/api/v1/apps/{}/records/{id}", w.slug), &w.token, None).await
}

/// The ids a record holds in a link field.
fn held(record: &Value, field: &str) -> Vec<String> {
    match record["data"].get(field) {
        Some(Value::String(s)) => vec![s.clone()],
        Some(Value::Array(a)) => a.iter().filter_map(|v| v.as_str().map(String::from)).collect(),
        _ => vec![],
    }
}

fn sorted(mut v: Vec<String>) -> Vec<String> {
    v.sort();
    v
}

fn rows_of(slug: &str, from: &str, field: &str) -> Vec<String> {
    sorted(link_rows(slug).into_iter().filter(|r| r.0 == from && r.1 == field).map(|r| r.2).collect())
}

// 1. Linking an application to a program shows the application in the program's mirror field.
#[tokio::test]
async fn linking_an_application_shows_it_in_the_programs_mirror_field() {
    let _db = common::TestDb::new();
    let w = world(false).await;
    let p = id_of(&create(&w, "programs", json!({ "code": "CS" })).await);
    let a = id_of(&create(&w, "apps", json!({ "applicant": "Ada", "program": p })).await);
    let program = get(&w, &p).await;
    assert_eq!(held(&program, "applications"), vec![a.clone()], "{program}");
    assert_eq!(rows_of(&w.slug, &p, "applications"), vec![a.clone()], "the mirror has its index row too");
    assert_eq!(rows_of(&w.slug, &a, "program"), vec![p.clone()]);

    // Moving the application to another program moves the mirror.
    let p2 = id_of(&create(&w, "programs", json!({ "code": "BIO" })).await);
    patch(&w, &a, 1, json!({ "applicant": "Ada", "program": p2 })).await;
    assert!(held(&get(&w, &p).await, "applications").is_empty());
    assert_eq!(held(&get(&w, &p2).await, "applications"), vec![a]);
}

// 2. Unlinking from the program side removes the link from the application.
#[tokio::test]
async fn unlinking_from_the_program_side_removes_it_from_the_application() {
    let _db = common::TestDb::new();
    let w = world(false).await;
    let p = id_of(&create(&w, "programs", json!({ "code": "CS" })).await);
    let a1 = id_of(&create(&w, "apps", json!({ "applicant": "Ada", "program": p })).await);
    let a2 = id_of(&create(&w, "apps", json!({ "applicant": "Bo", "program": p })).await);
    let program = get(&w, &p).await;
    assert_eq!(sorted(held(&program, "applications")), sorted(vec![a1.clone(), a2.clone()]));

    patch(&w, &p, program["version"].as_i64().unwrap(), json!({ "code": "CS", "applications": [a2.clone()] })).await;
    assert!(held(&get(&w, &a1).await, "program").is_empty(), "the application no longer points at the program");
    assert_eq!(held(&get(&w, &a2).await, "program"), vec![p.clone()]);
    assert!(rows_of(&w.slug, &a1, "program").is_empty());
    assert_eq!(rows_of(&w.slug, &a2, "program"), vec![p.clone()]);

    // Linking from the program side works the same way.
    let program = get(&w, &p).await;
    patch(&w, &p, program["version"].as_i64().unwrap(), json!({ "code": "CS", "applications": [a1.clone(), a2.clone()] })).await;
    assert_eq!(held(&get(&w, &a1).await, "program"), vec![p]);
}

// 3. The mirror write does not change the other record's version, and it is recorded as the system's.
#[tokio::test]
async fn a_mirror_write_keeps_the_other_records_version_and_is_recorded_as_the_systems() {
    let _db = common::TestDb::new();
    let w = world(false).await;
    let p = id_of(&create(&w, "programs", json!({ "code": "CS" })).await);
    let before = get(&w, &p).await["version"].as_i64().unwrap();
    let a = create(&w, "apps", json!({ "applicant": "Ada", "program": p })).await;
    let after = get(&w, &p).await["version"].as_i64().unwrap();
    assert_eq!(before, after, "linking an application did not make the program's save conflict");

    // A person's save of the program, made at the version they read, still succeeds.
    let saved = patch(&w, &p, before, json!({ "code": "CS-2", "applications": [id_of(&a)] })).await;
    assert_eq!(saved["version"].as_i64().unwrap(), before + 1);

    // History: the program has an update by the system, in the same batch as the application's create.
    let slug = w.slug.clone();
    let (pid, aid) = (p.clone(), id_of(&a));
    let (kind, program_batch, app_batch): (String, uuid::Uuid, uuid::Uuid) = db(move |c| {
        let m = c.query_one("SELECT actor_kind, batch_id FROM record_history WHERE app_slug = $1 AND record_id = $2 AND action = 'update' ORDER BY id LIMIT 1", &[&slug, &pid]).unwrap();
        let a = c.query_one("SELECT batch_id FROM record_history WHERE app_slug = $1 AND record_id = $2 AND action = 'create'", &[&slug, &aid]).unwrap();
        (m.get(0), m.get(1), a.get(0))
    });
    assert_eq!(kind, "system");
    assert_eq!(program_batch, app_batch, "the mirror write carries the batch of the user's change");
}


// 4. Deleting a program removes its links and restoring it brings them back, except to a record that is gone.
#[tokio::test]
async fn deleting_a_program_removes_its_links_and_restoring_it_brings_them_back() {
    let _db = common::TestDb::new();
    let w = world(false).await;
    let p = id_of(&create(&w, "programs", json!({ "code": "CS" })).await);
    let a1 = id_of(&create(&w, "apps", json!({ "applicant": "Ada", "program": p })).await);
    let a2 = id_of(&create(&w, "apps", json!({ "applicant": "Bo", "program": p })).await);

    // Delete: every link to and from the program is gone, and the applications no longer point at it.
    let (status, body) = delete(&w, &p).await;
    assert!(status.is_success(), "{status} {body}");
    assert!(held(&get(&w, &a1).await, "program").is_empty() && held(&get(&w, &a2).await, "program").is_empty());
    assert!(link_rows(&w.slug).iter().all(|r| r.0 != p && r.2 != p), "{:?}", link_rows(&w.slug));
    let pid = p.clone();
    let saved: i64 = db(move |c| c.query_one("SELECT count(*) FROM trash_links WHERE record_id = $1", &[&pid]).unwrap().get(0));
    assert_eq!(saved, 4, "two links out and two links in were remembered");

    // Restore: the record is back from trash, and so are the links to records that are still there.
    let restore = |what: String| {
        let (app, token, slug) = (w.app.clone(), w.token.clone(), w.slug.clone());
        async move { send(&app, "POST", &format!("/api/v1/apps/{slug}/trash/{what}/restore"), &token, Some(json!({}))).await }
    };
    let (status, body) = restore(p.clone()).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(held(&get(&w, &a1).await, "program"), vec![p.clone()]);
    assert_eq!(held(&get(&w, &a2).await, "program"), vec![p.clone()]);
    assert_eq!(rows_of(&w.slug, &p, "applications"), sorted(vec![a1.clone(), a2.clone()]));
    let pid = p.clone();
    let left: i64 = db(move |c| c.query_one("SELECT count(*) FROM trash_links WHERE record_id = $1", &[&pid]).unwrap().get(0));
    assert_eq!(left, 0, "nothing is left to restore");

    // Again, but one application is deleted while the program is in the trash.
    let (status, _) = delete(&w, &p).await;
    assert!(status.is_success());
    let (status, _) = delete(&w, &a2).await;
    assert!(status.is_success());
    let (status, body) = restore(p.clone()).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(held(&get(&w, &a1).await, "program"), vec![p.clone()]);
    let program = get(&w, &p).await;
    assert_eq!(held(&program, "applications"), vec![a1.clone()], "the program no longer names the deleted application: {program}");
    assert_eq!(rows_of(&w.slug, &p, "applications"), vec![a1]);
}

// 5. A Restrict field refuses the delete and reports the count.
#[tokio::test]
async fn a_restrict_field_refuses_the_delete_and_reports_the_count() {
    let _db = common::TestDb::new();
    let w = world(true).await;
    let p = id_of(&create(&w, "programs", json!({ "code": "CS" })).await);
    let mut apps = Vec::new();
    for name in ["A", "B", "C"] {
        apps.push(id_of(&create(&w, "apps", json!({ "applicant": name, "program": p })).await));
    }
    let (status, body) = delete(&w, &p).await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(body["code"], "constraint_violated", "{body}");
    assert_eq!(body["error"], "3 applications still link to this program.");
    assert!(held(&get(&w, &p).await, "applications").len() == 3, "the program is untouched");

    // Once the links are gone, the delete goes through.
    for a in &apps {
        let v = get(&w, a).await["version"].as_i64().unwrap();
        patch(&w, a, v, json!({ "applicant": "x" })).await;
    }
    let (status, body) = delete(&w, &p).await;
    assert!(status.is_success(), "{status} {body}");
}

// 6. A many-to-many pair keeps both sides in step across 50 random changes.
#[tokio::test]
async fn a_many_to_many_pair_stays_in_step_across_fifty_random_changes() {
    let _db = common::TestDb::new();
    let w = world(false).await;
    let mut students = Vec::new();
    let mut courses = Vec::new();
    for i in 0..5 {
        students.push(id_of(&create(&w, "students", json!({ "name": format!("s{i}") })).await));
        courses.push(id_of(&create(&w, "courses", json!({ "title": format!("c{i}") })).await));
    }
    let mut versions: std::collections::HashMap<String, i64> = students.iter().chain(courses.iter()).map(|id| (id.clone(), 1)).collect();
    let mut seed: u64 = 0x2545_F491_4F6C_DD1D;
    let mut next = move || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed
    };
    for step in 0..50 {
        let from_students = next() % 2 == 0;
        let (side, other, field, name_field) = if from_students { (&students, &courses, "courses", "name") } else { (&courses, &students, "students", "title") };
        let record = side[(next() % 5) as usize].clone();
        let picks: Vec<String> = other.iter().filter(|_| next() % 2 == 0).cloned().collect();
        // Read the current version, since the other side's writes do not change it.
        let current = get(&w, &record).await;
        let mut data = json!({ name_field: current["data"][name_field].clone() });
        data[field] = json!(picks);
        let saved = patch(&w, &record, current["version"].as_i64().unwrap(), data).await;
        versions.insert(record.clone(), saved["version"].as_i64().unwrap());

        // Both sides agree.
        for s in &students {
            for c in &courses {
                let sc = held(&get(&w, s).await, "courses").contains(c);
                let cs = held(&get(&w, c).await, "students").contains(s);
                assert_eq!(sc, cs, "step {step}: student {s} and course {c} disagree");
            }
        }
    }
    // And the index agrees with the JSON.
    for id in students.iter().chain(courses.iter()) {
        let r = get(&w, id).await;
        let field = if students.contains(id) { "courses" } else { "students" };
        assert_eq!(rows_of(&w.slug, id, field), sorted(held(&r, field)), "{id}");
    }
}
