//! Guests phase 4: operating guests.
//!
//! A guest proves they own an email address and may use one form. They ask for a link, which works once for 15 minutes, and
//! trade it for a session limited to that app and form. The start answers the same sentence whether or not anything was
//! done, so the form cannot be used to find out who has applied.

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::Router;
use http_body_util::BodyExt;
use scaffoldry_engine::{AppManifest, AppTable, FieldSpec, FieldType, SelectOption, SelectOptionColor};
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
        retired_fields: Vec::new(), key: None, unit_field: None,
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



const NEUTRAL: &str = "If that address can use this form, a link is on its way.";
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




fn admin_path(rest: &str) -> String {
    format!("/api/v1/admin/guests{rest}")
}

fn ledger(kind: &str) -> Vec<Value> {
    let kind = kind.to_string();
    db(move |c| c.query("SELECT payload, rationale FROM governance_ledger WHERE decision_type = $1 ORDER BY sequence", &[&kind]).unwrap().iter().map(|r| json!({ "payload": r.get::<_, Value>(0), "rationale": r.get::<_, String>(1) })).collect())
}

fn outbox_to(w: &World, email: &str) -> i64 {
    let _ = &w.slug;
    let email = email.to_string();
    db(move |c| c.query_one("SELECT count(*) FROM email_outbox WHERE to_address = $1", &[&email]).unwrap().get(0))
}

// 1. Closing a form refuses new starts and submissions at once and keeps existing records.
#[tokio::test]
async fn closing_a_form_refuses_starts_and_submissions_at_once_and_keeps_the_records() {
    let _db = common::TestDb::new();
    let (w, form) = ready().await;
    let ada = session(&w, &form, "ada@example.org").await;
    let (_, made) = submit(&w, &form, &ada, json!({ "kind": "grant", "amount": 40 })).await;
    let id = id_of(&made);

    let (s, v) = send(&w.app, "POST", &admin_path(&format!("/forms/{form}/close")), &w.platform, Some(json!({}))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "a reason is needed: {v}");
    let (s, v) = send(&w.app, "POST", &admin_path(&format!("/forms/{form}/close")), &w.platform, Some(json!({ "reason": "Too many applications this week." }))).await;
    assert_eq!(s, StatusCode::OK, "{v}");

    let before = outbox(&w).len();
    let (s, v) = start(&w, &form, "bob@example.org").await;
    assert_eq!((s, v["message"].as_str()), (StatusCode::OK, Some(NEUTRAL)), "a start says what it always says");
    assert_eq!(outbox(&w).len(), before, "and sends nothing");
    // The next request on a session that was open says so in words.
    let (s, v) = submit(&w, &form, &ada, json!({ "kind": "grant" })).await;
    assert_eq!(s, StatusCode::FORBIDDEN, "{v}");
    assert!(v.to_string().contains("This form is not accepting responses."), "{v}");
    let (s, _) = send(&w.app, "GET", &gpath(&w, &form, ""), &ada, None).await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    // The records are still there, and a member reads them.
    let (s, v) = send(&w.app, "GET", &format!("/api/v1/apps/{}/records/{id}", w.slug), &w.admin, None).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let entries = ledger("GuestChanged");
    assert_eq!(entries.len(), 1);
    assert!(entries[0]["rationale"].as_str().unwrap().contains("Too many applications"), "{entries:?}");

    // Opening it again lets starts through.
    let (s, v) = send(&w.app, "POST", &admin_path(&format!("/forms/{form}/open")), &w.platform, Some(json!({ "reason": "The week is over." }))).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    start(&w, &form, "cy@example.org").await;
    assert_eq!(outbox(&w).len(), before + 1, "a start sends again");
}


// 2. `Erase` removes the address and pending mail, keeps the records, and writes a ledger entry.
#[tokio::test]
async fn erasing_a_guest_removes_the_address_and_pending_mail_and_keeps_the_records() {
    let _db = common::TestDb::new();
    let (w, form) = ready().await;
    let ada = session(&w, &form, "ada@example.org").await;
    let (_, made) = submit(&w, &form, &ada, json!({ "kind": "grant", "amount": 40 })).await;
    let id = id_of(&made);
    // A mail that has not gone yet.
    db(|c| c.execute("INSERT INTO email_outbox (id, to_address, subject, body) VALUES (gen_random_uuid(), 'ada@example.org', 'An update on your request', 'x')", &[]).unwrap());
    assert!(outbox_to(&w, "ada@example.org") >= 1);
    let guest: Uuid = db(|c| c.query_one("SELECT id FROM guests WHERE email = 'ada@example.org'", &[]).unwrap().get(0));

    let (s, v) = send(&w.app, "POST", &admin_path(&format!("/{guest}/erase")), &w.platform, Some(json!({ "reason": "Asked to be forgotten." }))).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let (stored, erased): (String, bool) = db(move |c| {
        let r = c.query_one("SELECT email, erased_at IS NOT NULL FROM guests WHERE id = $1", &[&guest]).unwrap();
        (r.get(0), r.get(1))
    });
    assert!(stored.starts_with("erased:") && !stored.contains("ada"), "{stored}");
    assert!(erased);
    assert_eq!(outbox_to(&w, "ada@example.org"), 0, "no mail is left for the address");
    // The record stays, with its submitter.
    let submitter: Option<String> = db(move |c| c.query_one("SELECT data->>'_submitter' FROM dataset_records WHERE id = $1", &[&id]).unwrap().get(0));
    assert_eq!(submitter, Some(format!("guest:{guest}@guest.invalid")));
    // The session stops, and the address can apply again as a new person.
    let (s, _) = send(&w.app, "GET", &gpath(&w, &form, ""), &ada, None).await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);
    start(&w, &form, "ada@example.org").await;
    let guests: i64 = db(|c| c.query_one("SELECT count(*) FROM guests WHERE email = 'ada@example.org'", &[]).unwrap().get(0));
    assert_eq!(guests, 1, "a new guest");
    // The ledger names the guest by id and holds no address.
    let entries = ledger("GuestChanged");
    assert_eq!(entries.len(), 1);
    assert!(!entries[0].to_string().contains("ada@example.org") && entries[0].to_string().contains(&guest.to_string()), "{entries:?}");
    // The panel shows the erased guest as a former applicant.
    let (s, panel) = send(&w.app, "GET", &admin_path(""), &w.platform, None).await;
    assert_eq!(s, StatusCode::OK, "{panel}");
    let row = panel["guests"].as_array().unwrap().iter().find(|g| g["id"] == json!(guest.to_string())).unwrap();
    assert_eq!((row["address"].as_str(), row["state"].as_str(), row["submissions"].as_i64()), (Some("Former applicant"), Some("Erased"), Some(1)), "{row}");
    let (s, _) = send(&w.app, "POST", &admin_path(&format!("/{guest}/erase")), &w.platform, Some(json!({ "reason": "Again." }))).await;
    assert_eq!(s, StatusCode::CONFLICT, "erased once");
}

// 3. A guest with no records and 91 days of silence is removed. One with records is not.
#[tokio::test]
async fn a_silent_guest_with_no_records_is_removed_and_one_with_records_is_not() {
    let _db = common::TestDb::new();
    let (w, form) = ready().await;
    let ada = session(&w, &form, "ada@example.org").await;
    submit(&w, &form, &ada, json!({ "kind": "grant" })).await;
    session(&w, &form, "bob@example.org").await;
    session(&w, &form, "cy@example.org").await;
    db(|c| {
        c.execute("UPDATE guests SET created_at = NOW() - INTERVAL '120 days', last_seen_at = NOW() - INTERVAL '91 days' WHERE email IN ('ada@example.org', 'bob@example.org')", &[]).unwrap();
        c.execute("UPDATE guests SET last_seen_at = NOW() - INTERVAL '10 days' WHERE email = 'cy@example.org'", &[]).unwrap();
    });
    let removed = scaffoldry_server::service::guest_admin::purge(&w.state.repository).unwrap();
    assert_eq!(removed, 1);
    let left: Vec<String> = db(|c| c.query("SELECT email FROM guests ORDER BY email", &[]).unwrap().iter().map(|r| r.get(0)).collect());
    assert_eq!(left, vec!["ada@example.org".to_string(), "cy@example.org".to_string()], "bob is gone; ada has a record and cy was seen lately");
}

// 4. A faculty caller is 403 on every route here.
#[tokio::test]
async fn a_faculty_caller_is_refused_on_every_guest_admin_route() {
    let _db = common::TestDb::new();
    let (w, form) = ready().await;
    let faculty = issue_test_token_and_user("faculty.curie@state.edu");
    let zero = "00000000-0000-0000-0000-0000000000aa";
    for (method, uri) in [
        ("GET", admin_path("")),
        ("POST", admin_path(&format!("/forms/{form}/close"))),
        ("POST", admin_path(&format!("/forms/{form}/open"))),
        ("POST", admin_path(&format!("/{zero}/disable"))),
        ("POST", admin_path(&format!("/{zero}/enable"))),
        ("POST", admin_path(&format!("/{zero}/erase"))),
    ] {
        let (s, v) = send(&w.app, method, &uri, &faculty, if method == "POST" { Some(json!({ "reason": "x" })) } else { None }).await;
        assert_eq!(s, StatusCode::FORBIDDEN, "{method} {uri}: {v}");
    }
}

// 5. The summary lists an address after three bounces, and a network that hit a limit.
#[tokio::test]
async fn the_summary_lists_an_address_after_three_bounces_and_a_network_over_its_limit() {
    let _db = common::TestDb::new();
    let (w, form) = ready().await;
    session(&w, &form, "ada@example.org").await;
    session(&w, &form, "bob@example.org").await;
    db(|c| {
        for _ in 0..3 {
            c.execute("INSERT INTO email_outbox (id, to_address, subject, body, state, last_error) VALUES (gen_random_uuid(), 'ada@example.org', 'x', 'x', 'failed', 'mailbox unavailable')", &[]).unwrap();
        }
        c.execute("INSERT INTO email_outbox (id, to_address, subject, body, state) VALUES (gen_random_uuid(), 'bob@example.org', 'x', 'x', 'failed')", &[]).unwrap();
        c.execute("INSERT INTO rate_limits (key, window_start, count) VALUES ('guest-net:203.0.113.9', date_trunc('hour', NOW()), 25)", &[]).unwrap();
        c.execute("INSERT INTO rate_limits (key, window_start, count) VALUES ('guest-net:198.51.100.7', date_trunc('hour', NOW()), 4)", &[]).unwrap();
    });
    let (s, panel) = send(&w.app, "GET", &admin_path(""), &w.platform, None).await;
    assert_eq!(s, StatusCode::OK, "{panel}");
    let bounces = panel["abuse"]["bounces"].as_array().unwrap();
    assert_eq!(bounces.len(), 1, "{panel}");
    assert_eq!((bounces[0]["address"].as_str(), bounces[0]["bounces"].as_i64()), (Some("ada@example.org"), Some(3)));
    let nets = panel["abuse"]["networks"].as_array().unwrap();
    assert_eq!(nets.len(), 1, "{panel}");
    assert_eq!((nets[0]["network"].as_str(), nets[0]["hits"].as_i64()), (Some("203.0.113.9"), Some(5)));
    // The forms that are open, with what they have taken.
    let forms = panel["forms"].as_array().unwrap();
    let row = forms.iter().find(|f| f["form_id"] == json!(form)).unwrap_or_else(|| panic!("the form in {panel}"));
    assert_eq!((row["app"].as_str(), row["closed"].clone(), row["link_requests_today"].as_i64()), (Some(w.slug.as_str()), json!(false), Some(2)), "{row}");
}

// A guest can be disabled, which stops the session at once, and enabled again.
#[tokio::test]
async fn disabling_a_guest_stops_their_session_and_enabling_restores_it() {
    let _db = common::TestDb::new();
    let (w, form) = ready().await;
    let ada = session(&w, &form, "ada@example.org").await;
    let guest: Uuid = db(|c| c.query_one("SELECT id FROM guests WHERE email = 'ada@example.org'", &[]).unwrap().get(0));
    let (s, v) = send(&w.app, "POST", &admin_path(&format!("/{guest}/disable")), &w.platform, Some(json!({ "reason": "Repeated bounces." }))).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(send(&w.app, "GET", &gpath(&w, &form, ""), &ada, None).await.0, StatusCode::UNAUTHORIZED);
    let (s, v) = send(&w.app, "POST", &admin_path(&format!("/{guest}/enable")), &w.platform, Some(json!({ "reason": "Fixed." }))).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(send(&w.app, "GET", &gpath(&w, &form, ""), &ada, None).await.0, StatusCode::OK);
}

// The queries the guest routes and the console run look records up by `_submitter` and `_form`. On a table of many other
// records they use the partial indexes that only guest records are in, and do not read the table.
#[tokio::test]
async fn the_guest_lookups_use_their_partial_indexes_on_a_big_table() {
    let _db = common::TestDb::new();
    let w = world(1, 0).await;
    let slug = w.slug.clone();
    let plans: Vec<String> = db(move |c| {
        c.execute(
            "INSERT INTO dataset_records (id, app_slug, data, ceds_mapping, is_ferpa_sensitive, table_id, created_at) \
             SELECT 'big-' || g, $1, jsonb_build_object('_table_id', 'requests', 'kind', 'grant'), '{}'::jsonb, false, 'requests', NOW() \
             FROM generate_series(1, 30000) g",
            &[&slug],
        )
        .unwrap();
        c.execute(
            "INSERT INTO dataset_records (id, app_slug, data, ceds_mapping, is_ferpa_sensitive, table_id, created_at) \
             SELECT 'guest-' || g, $1, jsonb_build_object('_table_id', 'requests', '_form', 'f' || (g % 5), '_submitter', 'guest:' || g || '@guest.invalid'), '{}'::jsonb, false, 'requests', NOW() \
             FROM generate_series(1, 300) g",
            &[&slug],
        )
        .unwrap();
        c.batch_execute("ANALYZE dataset_records").unwrap();
        let explain = |c: &mut postgres::Client, sql: &str| -> String {
            c.query(&format!("EXPLAIN {}", sql.replace("$1", &format!("'{slug}'"))), &[]).unwrap().iter().map(|r| r.get::<_, String>(0)).collect::<Vec<_>>().join("\n")
        };
        vec![
            explain(c, "SELECT COUNT(*) FROM dataset_records WHERE app_slug = $1 AND data ? '_submitter' AND data->>'_form' = 'f1' AND data->>'_submitter' = 'guest:7@guest.invalid' AND created_at > NOW() - INTERVAL '1 day'"),
            explain(c, "SELECT COUNT(*) FROM dataset_records WHERE app_slug = $1 AND data ? '_form' AND data->>'_form' = 'f1'"),
            explain(c, "SELECT 1 FROM dataset_records r WHERE r.data ? '_submitter' AND r.data->>'_submitter' = 'guest:7@guest.invalid'"),
        ]
    });
    for plan in &plans {
        assert!(plan.contains("idx_dataset_records_guest_"), "uses a guest index: {plan}");
        assert!(!plan.contains("Seq Scan on dataset_records"), "does not read the table: {plan}");
    }
}
