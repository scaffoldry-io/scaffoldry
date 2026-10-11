//! Record history phase 3: comments and mentions on a record.
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
const COMMENTER: &str = "grace.hopper@science.state.edu";
const VIEWER: &str = "marcus.vance@state.edu";
const OUTSIDER: &str = "sarah.connor@state.edu";
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
    commenter: String,
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
        eppn: COMMENTER.to_string(),
        name: "Grace Hopper".to_string(),
        role: "commenter".to_string(),
        scoped_affiliation: "staff".to_string(),
        department: "biology".to_string(),
        added_at: chrono::Utc::now().to_rfc3339(),
    });
    Fixture {
        app: build_app_with_state(state.clone()).expect("router"),
        slug,
        owner: issue_test_token_and_user(OWNER),
        editor: issue_test_token_and_user(EDITOR),
        commenter: issue_test_token_and_user(COMMENTER),
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


async fn record(f: &Fixture) -> String {
    create(f, &f.editor, json!({ "_table_id": "t1", "title": "A", "score": 1 })).await["id"].as_str().unwrap().to_string()
}

async fn post(f: &Fixture, token: &str, id: &str, body: Value) -> (StatusCode, Value) {
    send(&f.app, "POST", &format!("/api/v1/apps/{}/records/{id}/comments", f.slug), token, &[], Some(body)).await
}

async fn list(f: &Fixture, token: &str, id: &str) -> (StatusCode, Value) {
    send(&f.app, "GET", &format!("/api/v1/apps/{}/records/{id}/comments", f.slug), token, &[], None).await
}

async fn edit(f: &Fixture, token: &str, comment: &str, body: Value) -> (StatusCode, Value) {
    send(&f.app, "PATCH", &format!("/api/v1/apps/{}/comments/{comment}", f.slug), token, &[], Some(body)).await
}

async fn remove(f: &Fixture, token: &str, comment: &str) -> (StatusCode, Value) {
    send(&f.app, "DELETE", &format!("/api/v1/apps/{}/comments/{comment}", f.slug), token, &[], None).await
}

// 1. A viewer cannot comment. A commenter can, and cannot edit the record.
#[tokio::test]
async fn a_viewer_cannot_comment_and_a_commenter_can_but_cannot_edit_the_record() {
    let _turn = LOCK.lock().await;
    let f = fixture().await;
    let id = record(&f).await;

    let (status, body) = post(&f, &f.viewer, &id, json!({ "body": "Looks good" })).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");

    let (status, body) = post(&f, &f.commenter, &id, json!({ "body": "Looks good" })).await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    assert_eq!(body["author"], COMMENTER);
    assert_eq!(body["body"], "Looks good");

    let (status, _) = send(&f.app, "PATCH", &format!("/api/v1/apps/{}/records/{id}", f.slug), &f.commenter, &[], Some(json!({ "data": { "title": "Changed" }, "version": 1 }))).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "a commenter cannot edit the record");

    // And an editor can comment too.
    let (status, _) = post(&f, &f.editor, &id, json!({ "body": "Thanks" })).await;
    assert_eq!(status, StatusCode::CREATED);
}

// 2. A mention of someone without read access is dropped silently. One of someone with access is stored.
#[tokio::test]
async fn a_mention_of_someone_without_access_is_dropped_without_a_word() {
    let _turn = LOCK.lock().await;
    let f = fixture().await;
    let id = record(&f).await;
    let text = format!("Please look, @[{EDITOR}] and @[{OUTSIDER}], and @[nobody@nowhere.example]");
    let (status, body) = post(&f, &f.owner, &id, json!({ "body": text })).await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    assert_eq!(body["mentions"], json!([EDITOR]), "only the person with access is stored: {body}");
    // The text is the author's own words. Nothing the server adds says who was dropped.
    let mut added = body.clone();
    added.as_object_mut().unwrap().remove("body");
    assert!(!added.to_string().contains(OUTSIDER) && !added.to_string().contains("nowhere"), "the author is not told who was dropped: {added}");
    assert!(added.get("dropped").is_none());
    assert_eq!(body["body"], text, "the text is kept as written");
}

// 3. A body of 4,001 characters is 400.
#[tokio::test]
async fn a_body_over_4000_characters_is_refused() {
    let _turn = LOCK.lock().await;
    let f = fixture().await;
    let id = record(&f).await;
    let (status, body) = post(&f, &f.owner, &id, json!({ "body": "x".repeat(4001) })).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert!(body["fields"].get("body").is_some(), "{body}");
    let (status, _) = post(&f, &f.owner, &id, json!({ "body": "x".repeat(4000) })).await;
    assert_eq!(status, StatusCode::CREATED, "4,000 is the limit");
    let (status, _) = post(&f, &f.owner, &id, json!({ "body": "   " })).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "an empty comment is refused");
}

fn age_comment(slug: &str, id: &str, minutes: i32) {
    let (slug, id) = (slug.to_string(), id.to_string());
    db(move |c| {
        c.execute("UPDATE record_comments SET created_at = NOW() - make_interval(mins => $3) WHERE app_slug = $1 AND id = $2::text::uuid", &[&slug, &id, &minutes]).unwrap();
    });
}

// 4. An author cannot edit after 15 minutes. An admin can delete.
#[tokio::test]
async fn an_author_has_fifteen_minutes_and_an_admin_can_remove_any_comment() {
    let _turn = LOCK.lock().await;
    let f = fixture().await;
    let id = record(&f).await;
    let (_, made) = post(&f, &f.commenter, &id, json!({ "body": "First" })).await;
    let cid = made["id"].as_str().unwrap().to_string();

    let (status, body) = edit(&f, &f.commenter, &cid, json!({ "body": "First, edited" })).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["body"], "First, edited");
    assert!(body["edited_at"].is_string());

    let (status, _) = edit(&f, &f.editor, &cid, json!({ "body": "Not mine" })).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "nobody else edits it");

    age_comment(&f.slug, &cid, 16);
    let (status, body) = edit(&f, &f.commenter, &cid, json!({ "body": "Too late" })).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    let (status, _) = remove(&f, &f.commenter, &cid).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "the author cannot remove it after 15 minutes either");

    let (status, body) = remove(&f, &f.owner, &cid).await;
    assert!(status.is_success(), "an owner or admin can: {status} {body}");
}

// 5. A deleted comment's body is gone from the response and from the database row.
#[tokio::test]
async fn a_removed_comment_loses_its_text_everywhere() {
    let _turn = LOCK.lock().await;
    let f = fixture().await;
    let id = record(&f).await;
    let (_, made) = post(&f, &f.commenter, &id, json!({ "body": format!("Secret words @[{EDITOR}]") })).await;
    let cid = made["id"].as_str().unwrap().to_string();
    let (status, _) = remove(&f, &f.commenter, &cid).await;
    assert!(status.is_success());

    let (status, page) = list(&f, &f.owner, &id).await;
    assert_eq!(status, StatusCode::OK, "{page}");
    let c = &page["comments"][0];
    assert_eq!(c["deleted"], true, "{c}");
    assert!(c["body"].is_null(), "{c}");
    assert_eq!(c["author"], COMMENTER, "the author and the time stay");
    assert!(c["created_at"].is_string());
    assert!(!page.to_string().contains("Secret"), "{page}");

    let (slug, cid2) = (f.slug.clone(), cid.clone());
    let (stored, mentions): (String, Vec<String>) = db(move |c| {
        let r = c.query_one("SELECT body, mentions FROM record_comments WHERE app_slug = $1 AND id = $2::text::uuid", &[&slug, &cid2]).unwrap();
        (r.get(0), r.get(1))
    });
    assert_eq!(stored, "", "the row holds no text");
    assert!(mentions.is_empty(), "and no mentions");

    // The history says a comment was added, by whom, and holds no text.
    let (slug, rid) = (f.slug.clone(), id.clone());
    let rows: Vec<(String, String, Value)> = db(move |c| {
        c.query("SELECT action, actor, changes FROM record_history WHERE app_slug = $1 AND record_id = $2 AND action = 'comment' ORDER BY id", &[&slug, &rid])
            .unwrap()
            .iter()
            .map(|r| (r.get(0), r.get(1), r.get(2)))
            .collect()
    });
    assert!(rows.len() >= 2, "added and removed: {rows:?}");
    assert!(rows.iter().all(|r| r.1 == COMMENTER));
    assert!(!format!("{rows:?}").contains("Secret"), "history holds no text: {rows:?}");
}

// 6. Comments are not returned to someone who cannot read the record.
#[tokio::test]
async fn comments_are_hidden_from_someone_who_cannot_read_the_record() {
    let _turn = LOCK.lock().await;
    let f = fixture().await;
    let id = record(&f).await;
    post(&f, &f.owner, &id, json!({ "body": "Private" })).await;
    let (status, body) = list(&f, &f.outsider, &id).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    assert!(!body.to_string().contains("Private"));
    let (status, _) = post(&f, &f.outsider, &id, json!({ "body": "Hi" })).await;
    assert!(status == StatusCode::NOT_FOUND || status == StatusCode::FORBIDDEN);
    // A viewer can read them.
    let (status, page) = list(&f, &f.viewer, &id).await;
    assert_eq!(status, StatusCode::OK, "{page}");
    assert_eq!(page["comments"].as_array().unwrap().len(), 1);
}

// 7a. The mention picker offers only people with read access.
#[tokio::test]
async fn the_mention_list_names_only_people_with_read_access() {
    let _turn = LOCK.lock().await;
    let f = fixture().await;
    let (status, body) = send(&f.app, "GET", &format!("/api/v1/apps/{}/mentionable", f.slug), &f.owner, &[], None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let names: Vec<String> = body["people"].as_array().unwrap().iter().map(|p| p["eppn"].as_str().unwrap().to_string()).collect();
    assert!(names.contains(&EDITOR.to_string()) && names.contains(&VIEWER.to_string()) && names.contains(&COMMENTER.to_string()), "{names:?}");
    assert!(!names.contains(&OUTSIDER.to_string()), "{names:?}");

    let (status, _) = send(&f.app, "GET", &format!("/api/v1/apps/{}/mentionable", f.slug), &f.outsider, &[], None).await;
    assert!(status == StatusCode::NOT_FOUND || status == StatusCode::FORBIDDEN, "someone outside gets no list");
}
