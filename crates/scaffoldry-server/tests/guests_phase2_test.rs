//! Guests phase 2: a form for guests.
//!
//! A guest proves they own an email address and may use one form. They ask for a link, which works once for 15 minutes, and
//! trade it for a session limited to that app and form. The start answers the same sentence whether or not anything was
//! done, so the form cannot be used to find out who has applied.

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::Router;
use http_body_util::BodyExt;
use scaffoldry_engine::{AppManifest, AppTable, FieldSpec, FieldType, HostRouter, SelectOption, SelectOptionColor};
use scaffoldry_server::build_app_with_state;
use scaffoldry_server::service::identity::issue_test_token_and_user;
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
                let _ = c.execute("DELETE FROM form_exposures WHERE app_slug LIKE $1", &[&like]);
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
fn manifest(slug: &str) -> AppManifest {
    let students = vec![
        FieldSpec::simple("name", "Name", FieldType::Text, false, false),
        indexed(select("program", &["p0", "p1", "p2", "p3", "p4"])),
        indexed(FieldSpec::simple("due", "Due", FieldType::Date, false, false)),
    ];
    let requests = vec![
        select("kind", &["grant", "loan"]),
        FieldSpec::simple("amount", "Amount", FieldType::Number, false, false),
        FieldSpec::simple("note", "Note", FieldType::Text, false, false),
        select("status", &["new", "decided"]),
        FieldSpec::simple("ssn", "Tax number", FieldType::Text, false, true),
        FieldSpec::simple("proof", "Proof", FieldType::Attachment { allow_multiple: false }, false, false),
        { let mut f = FieldSpec::simple("program_ref", "Program", FieldType::Link, false, false); f.link_target_app = Some("programs".to_string()); f.link_target_table = Some("programs".to_string()); f },
    ];
    let table = |id: &str, fields: Vec<FieldSpec>, primary: &str| AppTable {
        retired_fields: Vec::new(), key: None,
        id: id.to_string(),
        name: id.to_string(),
        slug: id.to_string(),
        description: None,
        icon: None,
        fields,
        primary_field: Some(primary.to_string()),
        sample_records: vec![],
        coordinates: None, sealed: false,
        row_place: Default::default(),
    };
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
        tables: vec![table("students", students, "name"), table("requests", requests, "kind")],
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
    platform: String,
    slug: String,
    admin: String,
    _cleanup: Cleanup,
}

/// `march` students due in March 2026, and `april` due on 5 April. Every fifth is in each of five programs.
async fn world(march: i64, april: i64) -> World {
    let tag = Uuid::new_v4().to_string()[..8].to_string();
    let slug = format!("av-{tag}");
    let (admin_e, editor_e, viewer_e) = (format!("admin-{tag}@state.edu"), format!("editor-{tag}@state.edu"), format!("viewer-{tag}@state.edu"));
    let state = Arc::new(ServerState::new().expect("state"));
    state.seed_demo().expect("seed");
    state.engine.write().unwrap().register_manifest(manifest(&slug)).unwrap();
    collaborator(&state, &admin_e, "admin");
    collaborator(&state, &editor_e, "editor");
    collaborator(&state, &viewer_e, "viewer");

    let s = slug.clone();
    db(move |c| {
        c.execute(
            "INSERT INTO dataset_records (id, app_slug, data, ceds_mapping, is_ferpa_sensitive, table_id, created_at) \
             SELECT $1::text || lpad(g::text, 8, '0'), $1::text, \
                    jsonb_build_object('_table_id', 'students', 'name', 'student-' || g, 'program', 'p' || (g % 5), \
                                       'due', to_char(CASE WHEN g <= $2::bigint THEN TIMESTAMPTZ '2026-03-01 00:00:00+00' + (g % 28) * interval '1 day' ELSE TIMESTAMPTZ '2026-04-05 00:00:00+00' END, 'YYYY-MM-DD\"T\"HH24:MI:SS\"Z\"')), \
                    '{}'::jsonb, false, 'students', TIMESTAMPTZ '2026-01-01 00:00:00+00' + g * interval '1 millisecond' \
             FROM generate_series(1, $2::bigint + $3::bigint) g",
            &[&s, &march, &april],
        )
        .unwrap();
        for (field, col, expr) in [("program", "txt", "lower(data->>'program')"), ("due", "ts", "(data->>'due')::timestamptz")] {
            c.execute(
                &format!("INSERT INTO record_index (app_slug, table_id, field, record_id, {col}) SELECT app_slug, 'students', '{field}', id, {expr} FROM dataset_records WHERE app_slug = $1"),
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
        platform: issue_test_token_and_user("jordan.lee@state.edu"),
        admin: issue_test_token_and_user(&admin_e),
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



const BASE: &str = "https://desk.state.edu";

fn views_path(w: &World) -> String {
    format!("/api/v1/apps/{}/views", w.slug)
}

/// Mail on, with a base address, so a link can be sent. Nothing is sent over a network: a mail is a row in the outbox.
fn mail_on(w: &World) {
    let mut s = w.state.settings.write().unwrap();
    s.insert("mail.enabled".to_string(), json!(true));
    s.insert("mail.smtp".to_string(), json!({ "host": "mail.test", "port": 587, "security": "starttls", "from": "desk@state.edu", "username": "desk", "secret": "smtp_password" }));
    s.insert("platform.base_url".to_string(), json!(BASE));
}

async fn setting(w: &World, key: &str, value: Value) {
    let (s, v) = send(&w.app, "PUT", &format!("/api/v1/settings/{key}"), &w.platform, Some(value)).await;
    assert_eq!(s, StatusCode::OK, "{key}: {v}");
}

/// A form view that is open to guests: made as a members form, then opened through a proposal that a second person approves.
async fn guest_form_with(w: &World, extra: Value) -> String {
    let mut config = json!({ "form_fields": [{ "name": "kind" }, { "name": "amount" }], "success_message": "Thanks.", "read_only_fields": ["status"] });
    for (k, v) in extra.as_object().cloned().unwrap_or_default() {
        config[k] = v;
    }
    let (s, v) = send(&w.app, "POST", &views_path(w), &w.admin, Some(json!({ "name": "Ask for help", "kind": "form", "ownership": "collaborative", "table_id": "requests", "config": config }))).await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    let id = v["id"].as_str().or_else(|| v["view"]["id"].as_str()).unwrap().to_string();
    let (s, v) = send(&w.app, "POST", &format!("{}/{id}/guest-exposure", views_path(w)), &w.admin, Some(json!({}))).await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    let xid = v["id"].as_str().unwrap().to_string();
    let (s, v) = send(&w.app, "POST", &format!("{}/{id}/guest-exposure/{xid}/decide", views_path(w)), &w.platform, Some(json!({ "decision": "approve", "rationale": "Applicants are outside the directory." }))).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    id
}

async fn guest_form(w: &World) -> String {
    guest_form_with(w, json!({})).await
}

async fn start_with(w: &World, form: &str, body: Value, address: Option<&str>) -> (StatusCode, Value) {
    let mut b = Request::builder().method("POST").uri(format!("/api/v1/guest/{}/{form}/start", w.slug)).header("content-type", "application/json");
    if let Some(a) = address {
        b = b.header("x-forwarded-for", a);
    }
    let resp = w.app.clone().oneshot(b.body(Body::from(body.to_string())).unwrap()).await.unwrap();
    let status = resp.status();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
}

async fn start(w: &World, form: &str, email: &str) -> (StatusCode, Value) {
    start_with(w, form, json!({ "email": email }), Some("198.51.100.1")).await
}

async fn verify(w: &World, token: &str) -> (StatusCode, Value) {
    let req = Request::builder().method("POST").uri("/api/v1/guest/verify").header("content-type", "application/json").body(Body::from(json!({ "token": token }).to_string())).unwrap();
    let resp = w.app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
}

fn outbox(w: &World) -> Vec<(String, String, String)> {
    let _ = &w.slug;
    db(|c| c.query("SELECT to_address, subject, body FROM email_outbox ORDER BY created_at", &[]).unwrap().iter().map(|r| (r.get(0), r.get(1), r.get(2))).collect())
}


/// The token in the newest mail to an address.
fn link_to(w: &World, email: &str) -> String {
    let mail = outbox(w).into_iter().rev().find(|(to, _, _)| to == email).unwrap_or_else(|| panic!("a mail to {email}"));
    let at = mail.2.find("/g/").expect("a link in the mail");
    mail.2[at + 3..].chars().take_while(|c| c.is_ascii_hexdigit()).collect()
}

async fn ready() -> (World, String) {
    let w = world(1, 0).await;
    mail_on(&w);
    setting(&w, "guests.enabled", json!(true)).await;
    let form = guest_form(&w).await;
    (w, form)
}


fn gpath(w: &World, form: &str, rest: &str) -> String {
    format!("/api/v1/guest/{}/{form}{rest}", w.slug)
}

/// A guest session for an address: asks for a link, reads it from the outbox, trades it.
async fn session(w: &World, form: &str, email: &str) -> String {
    let (s, _) = start(w, form, email).await;
    assert_eq!(s, StatusCode::OK);
    let (s, v) = verify(w, &link_to(w, email)).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    v["token"].as_str().unwrap().to_string()
}

async fn submit(w: &World, form: &str, token: &str, data: Value) -> (StatusCode, Value) {
    send(&w.app, "POST", &gpath(w, form, "/records"), token, Some(json!({ "data": data }))).await
}

fn id_of(v: &Value) -> String {
    v["record"]["id"].as_str().unwrap_or_else(|| panic!("a record id in {v}")).to_string()
}

fn stored_submitter(w: &World, id: &str) -> Option<String> {
    let _ = &w.slug;
    let id = id.to_string();
    db(move |c| c.query_opt("SELECT data->>'_submitter' FROM dataset_records WHERE id = $1", &[&id]).unwrap().and_then(|r| r.get(0)))
}

// 1. A guest creates one record. Their session cannot read another record, even by guessing its id, and gets not found.
#[tokio::test]
async fn a_guest_reads_only_what_they_made_and_a_guessed_id_is_not_found() {
    let _db = common::TestDb::new();
    let (w, form) = ready().await;
    let ada = session(&w, &form, "ada@example.org").await;
    let bob = session(&w, &form, "bob@example.org").await;

    let (s, made) = submit(&w, &form, &ada, json!({ "kind": "grant", "amount": 40 })).await;
    assert_eq!(s, StatusCode::CREATED, "{made}");
    assert_eq!(made["success_message"], "Thanks.");
    let mine = id_of(&made);
    let (_, theirs) = submit(&w, &form, &bob, json!({ "kind": "loan", "amount": 90 })).await;
    let theirs = id_of(&theirs);
    // A record a member made is not a guest's either.
    let (s, member) = send(&w.app, "POST", &format!("/api/v1/apps/{}/records", w.slug), &w.admin, Some(json!({ "data": { "_table_id": "requests", "kind": "grant", "amount": 1 } }))).await;
    assert_eq!(s, StatusCode::CREATED, "{member}");
    let member_id = member["id"].as_str().or_else(|| member["record"]["id"].as_str()).unwrap().to_string();

    let (s, own) = send(&w.app, "GET", &gpath(&w, &form, &format!("/records/{mine}")), &ada, None).await;
    assert_eq!(s, StatusCode::OK, "{own}");
    assert_eq!(own["record"]["data"]["kind"], "grant");
    assert!(own["record"]["data"].get("_submitter").is_none(), "the submitter is not shown back: {own}");
    for other in [&theirs, &member_id, &Uuid::new_v4().to_string()] {
        let (s, v) = send(&w.app, "GET", &gpath(&w, &form, &format!("/records/{other}")), &ada, None).await;
        assert_eq!(s, StatusCode::NOT_FOUND, "{other}: {v}");
        let (s, v) = send(&w.app, "PUT", &gpath(&w, &form, &format!("/records/{other}")), &ada, Some(json!({ "version": 1, "data": { "kind": "loan" } }))).await;
        assert_eq!(s, StatusCode::NOT_FOUND, "{other}: {v}");
    }
    let guest_id = db(|c| c.query_one("SELECT id FROM guests WHERE email = 'ada@example.org'", &[]).unwrap().get::<_, Uuid>(0));
    assert_eq!(stored_submitter(&w, &mine), Some(format!("guest:{guest_id}@guest.invalid")));
}

// 2. A request with a field outside the form is 400 naming it.
#[tokio::test]
async fn a_field_outside_the_form_is_refused_by_name() {
    let _db = common::TestDb::new();
    let (w, form) = ready().await;
    let ada = session(&w, &form, "ada@example.org").await;
    for (field, value) in [("note", json!("x")), ("ssn", json!("123")), ("_submitter", json!("guest:someone@guest.invalid")), ("status", json!("decided")), ("_table_id", json!("students"))] {
        let (s, v) = submit(&w, &form, &ada, json!({ "kind": "grant", field: value })).await;
        assert_eq!(s, StatusCode::BAD_REQUEST, "{field}: {v}");
        assert!(v.to_string().contains(field), "{field} is named: {v}");
    }
    let n: i64 = db(|c| c.query_one("SELECT count(*) FROM dataset_records WHERE data->>'_submitter' IS NOT NULL", &[]).unwrap().get(0));
    assert_eq!(n, 0, "nothing was saved");
}

// 3. After `locks_when` holds, the guest's update is refused with 409 and a message.
#[tokio::test]
async fn once_the_lock_holds_the_guest_cannot_change_the_record() {
    let _db = common::TestDb::new();
    let w = world(1, 0).await;
    mail_on(&w);
    setting(&w, "guests.enabled", json!(true)).await;
    let form = guest_form_with(&w, json!({ "locks_when": { "field": "status", "in": ["decided"] } })).await;
    let ada = session(&w, &form, "ada@example.org").await;
    let (_, made) = submit(&w, &form, &ada, json!({ "kind": "grant", "amount": 40 })).await;
    let id = id_of(&made);
    let version = made["record"]["version"].as_i64().unwrap_or(1);

    let (s, v) = send(&w.app, "PUT", &gpath(&w, &form, &format!("/records/{id}")), &ada, Some(json!({ "version": version, "data": { "amount": 45 } }))).await;
    assert_eq!(s, StatusCode::OK, "an open record can be changed: {v}");
    let version = v["record"]["version"].as_i64().unwrap();

    let (s, v) = send(&w.app, "PATCH", &format!("/api/v1/apps/{}/records/{id}", w.slug), &w.admin, Some(json!({ "version": version, "data": { "_table_id": "requests", "kind": "grant", "amount": 45, "status": "decided" } }))).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let version = v["version"].as_i64().or_else(|| v["record"]["version"].as_i64()).unwrap();

    let (s, v) = send(&w.app, "PUT", &gpath(&w, &form, &format!("/records/{id}")), &ada, Some(json!({ "version": version, "data": { "amount": 50 } }))).await;
    assert_eq!(s, StatusCode::CONFLICT, "{v}");
    assert!(v.to_string().contains("can no longer be changed"), "{v}");
    let (s, v) = send(&w.app, "GET", &gpath(&w, &form, &format!("/records/{id}")), &ada, None).await;
    assert_eq!((s, v["record"]["data"]["status"].as_str(), v["record"]["data"]["amount"].as_i64()), (StatusCode::OK, Some("decided"), Some(45)), "the status is shown, read only: {v}");
}

// A save by a member keeps who the record belongs to, and a member cannot name a guest as the submitter of a record.
#[tokio::test]
async fn a_members_save_keeps_the_submitter_and_cannot_set_one() {
    let _db = common::TestDb::new();
    let (w, form) = ready().await;
    let ada = session(&w, &form, "ada@example.org").await;
    let (_, made) = submit(&w, &form, &ada, json!({ "kind": "grant", "amount": 5 })).await;
    let id = id_of(&made);
    let before = stored_submitter(&w, &id);
    assert!(before.is_some());
    let (s, v) = send(&w.app, "PATCH", &format!("/api/v1/apps/{}/records/{id}", w.slug), &w.admin, Some(json!({ "version": made["record"]["version"], "data": { "_table_id": "requests", "kind": "loan", "amount": 6, "_submitter": "guest:x@guest.invalid" } }))).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(stored_submitter(&w, &id), before, "a save by a member keeps the submitter, and ignores one they send");
    let (s, v) = send(&w.app, "GET", &gpath(&w, &form, &format!("/records/{id}")), &ada, None).await;
    assert_eq!((s, v["record"]["data"]["kind"].as_str()), (StatusCode::OK, Some("loan")), "the guest still reads it: {v}");
    let (s, v) = send(&w.app, "POST", &format!("/api/v1/apps/{}/records", w.slug), &w.admin, Some(json!({ "data": { "_table_id": "requests", "kind": "grant", "_submitter": "guest:x@guest.invalid", "_form": "f" } }))).await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    let made = v["id"].as_str().or_else(|| v["record"]["id"].as_str()).unwrap().to_string();
    assert_eq!(stored_submitter(&w, &made), None, "a member cannot make a record a guest's");
}

// 4. A guest session on a list, search, aggregate, export, history, comment, or link route is refused.
#[tokio::test]
async fn a_guest_session_is_refused_on_every_route_that_reaches_other_records() {
    let _db = common::TestDb::new();
    let (w, form) = ready().await;
    let ada = session(&w, &form, "ada@example.org").await;
    let (_, made) = submit(&w, &form, &ada, json!({ "kind": "grant" })).await;
    let id = id_of(&made);
    let app = format!("/api/v1/apps/{}", w.slug);
    for (method, uri) in [
        ("GET", format!("{app}/records")),
        ("GET", format!("{app}/tables/requests/records")),
        ("GET", format!("{app}/search?q=grant")),
        ("GET", format!("{app}/tables/requests/aggregate")),
        ("GET", format!("{app}/export")),
        ("POST", format!("{app}/exports")),
        ("GET", format!("{app}/records/{id}/history")),
        ("GET", format!("{app}/records/{id}/comments")),
        ("POST", format!("{app}/records/{id}/comments")),
        ("GET", format!("{app}/link-options")),
        ("GET", format!("{app}/views/{form}/run")),
        ("GET", format!("{app}/records/{id}")),
    ] {
        let (s, v) = send(&w.app, method, &uri, &ada, if method == "POST" { Some(json!({})) } else { None }).await;
        assert_eq!((s, v["code"].as_str()), (StatusCode::FORBIDDEN, Some("token_scope")), "{method} {uri}: {v}");
    }
    // The guest routes have no list, and a session for one form is no session for another.
    let (s, _) = send(&w.app, "GET", &gpath(&w, &form, "/records"), &ada, None).await;
    assert!(matches!(s, StatusCode::NOT_FOUND | StatusCode::METHOD_NOT_ALLOWED), "{s}");
    let other = Uuid::new_v4().to_string();
    let (s, v) = submit(&w, &other, &ada, json!({ "kind": "grant" })).await;
    assert_eq!((s, v["code"].as_str()), (StatusCode::FORBIDDEN, Some("token_scope")), "{v}");
    let (s, v) = send(&w.app, "POST", &format!("/api/v1/guest/other-app/{form}/records"), &ada, Some(json!({ "data": {} }))).await;
    assert_eq!((s, v["code"].as_str()), (StatusCode::FORBIDDEN, Some("token_scope")), "{v}");
    // And no session at all is no session.
    let (s, _) = send(&w.app, "POST", &gpath(&w, &form, "/records"), "scf_nothing", Some(json!({ "data": {} }))).await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);
}

async fn upload(w: &World, form: &str, token: &str, size: usize) -> (StatusCode, Value) {
    let req = Request::builder()
        .method("POST")
        .uri(gpath(w, form, "/files"))
        .header("authorization", format!("Bearer {token}"))
        .header("content-length", size.to_string())
        .header("x-file-name", "proof.pdf")
        .body(Body::from(vec![b'a'; size]))
        .unwrap();
    let resp = w.app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
}

// 5. A file from a guest is scanned or refused. A file over the guest limit is refused.
#[tokio::test]
async fn a_guests_file_needs_a_scanner_and_fits_the_guest_limit() {
    let _db = common::TestDb::new();
    let dir = std::env::temp_dir().join(format!("guest-files-{}", Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    std::env::set_var("SCAFFOLDRY_FILES_DIR", &dir);
    let (w, form) = ready().await;
    let ada = session(&w, &form, "ada@example.org").await;
    // `files.require_scan` is off, and a guest's file still needs a scan. No scanner answers, so it is refused.
    let (s, v) = upload(&w, &form, &ada, 1000).await;
    assert_eq!((s, v["code"].as_str()), (StatusCode::SERVICE_UNAVAILABLE, Some("scanner_unavailable")), "{v}");
    // Over the guest limit, whatever the scanner says. The limit is `guests.max_file_mb`, 10 MB unless it is set.
    let (s, v) = upload(&w, &form, &ada, 11 * 1024 * 1024).await;
    assert_eq!((s, v["code"].as_str()), (StatusCode::PAYLOAD_TOO_LARGE, Some("file_too_large")), "{v}");
    assert!(v.to_string().contains("10 MB"), "the limit is stated: {v}");
    setting(&w, "guests.max_file_mb", json!(1)).await;
    let (s, v) = upload(&w, &form, &ada, 2 * 1024 * 1024).await;
    assert_eq!((s, v["code"].as_str()), (StatusCode::PAYLOAD_TOO_LARGE, Some("file_too_large")), "{v}");
    assert!(v.to_string().contains("1 MB"), "{v}");
    std::fs::remove_dir_all(&dir).ok();
}

// 6. The fourth submission in a day is refused with the limit stated.
#[tokio::test]
async fn the_fourth_submission_in_a_day_and_the_total_limit_are_refused_with_the_number() {
    let _db = common::TestDb::new();
    let (w, form) = ready().await;
    let ada = session(&w, &form, "ada@example.org").await;
    for i in 0..3 {
        let (s, v) = submit(&w, &form, &ada, json!({ "kind": "grant", "amount": i })).await;
        assert_eq!(s, StatusCode::CREATED, "{i}: {v}");
    }
    let (s, v) = submit(&w, &form, &ada, json!({ "kind": "grant" })).await;
    assert_eq!((s, v["code"].as_str()), (StatusCode::TOO_MANY_REQUESTS, Some("rate_limited")), "{v}");
    assert!(v.to_string().contains("3 submissions a day"), "{v}");
    // Another guest is not held back by it.
    let bob = session(&w, &form, "bob@example.org").await;
    assert_eq!(submit(&w, &form, &bob, json!({ "kind": "loan" })).await.0, StatusCode::CREATED);

    // A form with a total of two takes two and no more, from anyone.
    let tiny = guest_form_with(&w, json!({ "max_total": 2 })).await;
    let cy = session(&w, &tiny, "cy@example.org").await;
    let di = session(&w, &tiny, "di@example.org").await;
    assert_eq!(submit(&w, &tiny, &cy, json!({ "kind": "grant" })).await.0, StatusCode::CREATED);
    assert_eq!(submit(&w, &tiny, &di, json!({ "kind": "grant" })).await.0, StatusCode::CREATED);
    let (s, v) = submit(&w, &tiny, &cy, json!({ "kind": "grant" })).await;
    assert_eq!(s, StatusCode::CONFLICT, "{v}");
    assert!(v.to_string().contains("2 submissions"), "{v}");
}

async fn plain_form(w: &World, extra: Value) -> (StatusCode, Value) {
    let mut config = json!({ "form_fields": [{ "name": "kind" }, { "name": "amount" }] });
    for (k, v) in extra.as_object().cloned().unwrap_or_default() {
        config[k] = v;
    }
    send(&w.app, "POST", &views_path(w), &w.admin, Some(json!({ "name": "Ask", "kind": "form", "ownership": "collaborative", "table_id": "requests", "config": config }))).await
}

// 7. Changing a form to `Guests` raises `guest_exposure`. The proposer cannot approve alone.
#[tokio::test]
async fn opening_a_form_to_guests_is_a_proposal_a_second_person_decides() {
    let _db = common::TestDb::new();
    let w = world(1, 0).await;
    mail_on(&w);
    setting(&w, "guests.enabled", json!(true)).await;
    // The audience is not saved by a view save. Saying `Guests` there is refused, and says where to go.
    let (s, v) = plain_form(&w, json!({ "audience": "Guests" })).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{v}");
    assert!(v.to_string().contains("guest-exposure"), "{v}");

    let (s, v) = plain_form(&w, json!({})).await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    let form = v["id"].as_str().or_else(|| v["view"]["id"].as_str()).unwrap().to_string();
    let (s, p) = send(&w.app, "POST", &format!("{}/{form}/guest-exposure", views_path(&w)), &w.admin, Some(json!({}))).await;
    assert_eq!(s, StatusCode::CREATED, "{p}");
    assert_eq!(p["status"], "Pending");
    assert_eq!(p["change"], "This form lets people outside the institution create records in requests. They can see only their own.");
    let check = p["checks"].as_array().unwrap().iter().find(|c| c["name"] == "guest_exposure").unwrap_or_else(|| panic!("a guest_exposure check in {p}"));
    assert_eq!(check["outcome"], "warn", "the table has a sensitive field the form does not list: {check}");
    assert_eq!(p["needs_second_person"], true);
    let xid = p["id"].as_str().unwrap().to_string();

    // Not open yet: a guest asking for a link gets the neutral answer and nothing is sent.
    start(&w, &form, "ada@example.org").await;
    assert!(outbox(&w).is_empty(), "a pending proposal opens nothing");

    let decide = format!("{}/{form}/guest-exposure/{xid}/decide", views_path(&w));
    let (s, v) = send(&w.app, "POST", &decide, &w.admin, Some(json!({ "decision": "approve", "rationale": "ok" }))).await;
    assert_eq!(s, StatusCode::FORBIDDEN, "the proposer cannot approve: {v}");
    let (s, v) = send(&w.app, "POST", &decide, &w.platform, Some(json!({ "decision": "approve", "rationale": "Applicants are outside the directory." }))).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    start(&w, &form, "ada@example.org").await;
    assert_eq!(outbox(&w).len(), 1, "open now");
    let ledger: i64 = db(|c| c.query_one("SELECT count(*) FROM governance_ledger WHERE decision_type = 'GuestExposureChanged'", &[]).unwrap().get(0));
    assert!(ledger >= 2, "the proposal and the decision are in the ledger: {ledger}");

    // A decision is made once. Setting the audience back is a plain save and closes the form.
    let (s, _) = send(&w.app, "POST", &decide, &w.platform, Some(json!({ "decision": "approve", "rationale": "again" }))).await;
    assert_eq!(s, StatusCode::CONFLICT);
}

// 8. A form that lists a link field to a dataset fails validation. A sensitive field the person did not mark as their own fails the check.
#[tokio::test]
async fn a_guest_form_cannot_list_a_link_and_a_sensitive_field_needs_to_be_the_persons_own() {
    let _db = common::TestDb::new();
    let w = world(1, 0).await;
    let (s, v) = plain_form(&w, json!({ "form_fields": [{ "name": "kind" }, { "name": "program_ref" }] })).await;
    assert_eq!(s, StatusCode::CREATED, "a members form may list a link: {v}");
    let linked = v["id"].as_str().or_else(|| v["view"]["id"].as_str()).unwrap().to_string();
    let (s, v) = send(&w.app, "POST", &format!("{}/{linked}/guest-exposure", views_path(&w)), &w.admin, Some(json!({}))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{v}");
    assert!(v.to_string().contains("program_ref"), "{v}");

    let (_, v) = plain_form(&w, json!({ "form_fields": [{ "name": "kind" }, { "name": "ssn" }] })).await;
    let sensitive = v["id"].as_str().or_else(|| v["view"]["id"].as_str()).unwrap().to_string();
    let (s, p) = send(&w.app, "POST", &format!("{}/{sensitive}/guest-exposure", views_path(&w)), &w.admin, Some(json!({}))).await;
    assert_eq!(s, StatusCode::CREATED, "{p}");
    let check = p["checks"].as_array().unwrap().iter().find(|c| c["name"] == "guest_exposure").unwrap();
    assert_eq!(check["outcome"], "fail", "{check}");
    let xid = p["id"].as_str().unwrap();
    let (s, v) = send(&w.app, "POST", &format!("{}/{sensitive}/guest-exposure/{xid}/decide", views_path(&w)), &w.platform, Some(json!({ "decision": "approve", "rationale": "ok" }))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "a failed check cannot be approved: {v}");

    // Marked as something the person is giving about themself, the same field passes, and nothing sensitive is left unlisted.
    let (_, v) = plain_form(&w, json!({ "form_fields": [{ "name": "kind" }, { "name": "ssn", "own_data": true }] })).await;
    let own = v["id"].as_str().or_else(|| v["view"]["id"].as_str()).unwrap().to_string();
    let (s, p) = send(&w.app, "POST", &format!("{}/{own}/guest-exposure", views_path(&w)), &w.admin, Some(json!({}))).await;
    assert_eq!(s, StatusCode::CREATED, "{p}");
    let check = p["checks"].as_array().unwrap().iter().find(|c| c["name"] == "guest_exposure").unwrap();
    assert_eq!(check["outcome"], "pass", "{check}");
}

// 9. The row rule holds even when the table has no access rules of its own. This world has none, and the guest still sees one row.
// A live guest form keeps its field list: changing it is a new proposal.
#[tokio::test]
async fn the_row_rule_needs_no_rules_and_a_live_form_keeps_its_fields() {
    let _db = common::TestDb::new();
    let (w, form) = ready().await;
    assert!(w.state.engine.read().unwrap().resolve_by_slug(&w.slug).unwrap().access.rows.is_empty(), "no rules of any kind");
    let ada = session(&w, &form, "ada@example.org").await;
    let bob = session(&w, &form, "bob@example.org").await;
    let (_, a) = submit(&w, &form, &ada, json!({ "kind": "grant" })).await;
    let (_, b) = submit(&w, &form, &bob, json!({ "kind": "loan" })).await;
    let (s, _) = send(&w.app, "GET", &gpath(&w, &form, &format!("/records/{}", id_of(&b))), &ada, None).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    let (s, _) = send(&w.app, "GET", &gpath(&w, &form, &format!("/records/{}", id_of(&a))), &ada, None).await;
    assert_eq!(s, StatusCode::OK);

    let (s, v) = send(&w.app, "PUT", &format!("{}/{form}", views_path(&w)), &w.admin, Some(json!({ "version": 1, "config": { "audience": "Guests", "form_fields": [{ "name": "kind" }, { "name": "note" }] } }))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{v}");
    assert!(v.to_string().contains("guest-exposure"), "{v}");
}

// A person's address cannot be a guest's in a grant or a delegation either.
#[tokio::test]
async fn a_guest_address_is_refused_by_a_delegation() {
    let _db = common::TestDb::new();
    let (w, _form) = ready().await;
    let (s, v) = send(&w.app, "POST", "/api/v1/me/delegations", &w.admin, Some(json!({ "delegate": "guest:00000000-0000-0000-0000-000000000001@guest.invalid", "reason": "away", "starts_at": "2030-01-01T00:00:00Z", "ends_at": "2030-01-08T00:00:00Z" }))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{v}");
    assert!(v.to_string().to_lowercase().contains("guest"), "{v}");
}
