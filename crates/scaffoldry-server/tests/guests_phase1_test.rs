//! Guests phase 1: a guest and a link.
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
        FieldSpec::simple("amount", "Amount", FieldType::Number, true, false),
        FieldSpec::simple("note", "Note", FieldType::Text, false, false),
    ];
    let table = |id: &str, fields: Vec<FieldSpec>, primary: &str| AppTable {
        retired_fields: Vec::new(),
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

/// A form view that is open to guests.
async fn guest_form(w: &World) -> String {
    let config = json!({ "audience": "Guests", "form_fields": [{ "name": "kind" }, { "name": "amount" }], "success_message": "Thanks." });
    let (s, v) = send(&w.app, "POST", &views_path(w), &w.admin, Some(json!({ "name": "Ask for help", "kind": "form", "ownership": "collaborative", "table_id": "requests", "config": config }))).await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    v["id"].as_str().or_else(|| v["view"]["id"].as_str()).unwrap().to_string()
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

fn guests(w: &World) -> i64 {
    let _ = &w.slug;
    db(|c| c.query_one("SELECT count(*) FROM guests", &[]).unwrap().get(0))
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

// 1. A start for an address creates a guest and one mail. The neutral response is identical for an address that exists, one
// that does not, and a disabled one.
#[tokio::test]
async fn a_start_makes_a_guest_and_one_mail_and_says_the_same_to_everyone() {
    let _db = common::TestDb::new();
    let (w, form) = ready().await;
    let (s, new) = start(&w, &form, "Ada@Example.org").await;
    assert_eq!((s, new["message"].as_str()), (StatusCode::OK, Some(NEUTRAL)), "{new}");
    assert_eq!(guests(&w), 1);
    let mail = outbox(&w);
    assert_eq!(mail.len(), 1, "one mail: {mail:?}");
    assert_eq!(mail[0].0, "ada@example.org", "the address is kept in lower case");
    assert!(mail[0].2.contains(&format!("{BASE}/g/")), "a link on the desk's address: {}", mail[0].2);
    assert!(mail[0].2.contains("15 minutes"), "it says how long the link lasts");

    // An address that exists says the same, and so does a disabled one, and so does a form that is not there.
    let (_, again) = start(&w, &form, "ada@example.org").await;
    db(|c| c.execute("UPDATE guests SET disabled_at = NOW() WHERE email = 'ada@example.org'", &[]).unwrap());
    let (_, disabled) = start(&w, &form, "ada@example.org").await;
    let (_, nobody) = start(&w, &form, "grace@example.org").await;
    let (_, no_form) = start(&w, &Uuid::new_v4().to_string(), "grace@example.org").await;
    let (_, junk) = start(&w, &form, "not an address").await;
    for other in [&again, &disabled, &nobody, &no_form, &junk] {
        assert_eq!(other, &new, "the same answer for everyone");
    }
    // Only the first, the second (the guest existed and was not disabled) and the new address were mailed.
    let to: Vec<String> = outbox(&w).into_iter().map(|m| m.0).collect();
    assert_eq!(to, vec!["ada@example.org", "ada@example.org", "grace@example.org"], "a disabled guest, a missing form and junk get no mail");
}

// 2. A link works once. A second use fails. An expired one fails.
#[tokio::test]
async fn a_link_works_once_and_not_after_it_expires() {
    let _db = common::TestDb::new();
    let (w, form) = ready().await;
    start(&w, &form, "ada@example.org").await;
    let token = link_to(&w, "ada@example.org");
    assert_eq!(token.len(), 32, "128 random bits, in hex");

    let (s, session) = verify(&w, &token).await;
    assert_eq!(s, StatusCode::OK, "{session}");
    assert!(session["token"].as_str().is_some_and(|t| t.len() > 20), "a session token: {session}");
    assert_eq!(session["app"], w.slug.as_str());
    assert_eq!(session["form"], form.as_str());
    let hours = chrono::DateTime::parse_from_rfc3339(session["expires_at"].as_str().unwrap()).unwrap().signed_duration_since(chrono::Utc::now()).num_minutes();
    assert!((470..=480).contains(&hours), "eight hours: {hours} minutes");
    let verified: bool = db(|c| c.query_one("SELECT email_verified_at IS NOT NULL FROM guests WHERE email = 'ada@example.org'", &[]).unwrap().get(0));
    assert!(verified, "the address is now verified");

    let (s, v) = verify(&w, &token).await;
    assert_eq!(s, StatusCode::NOT_FOUND, "a second use fails: {v}");

    // An expired link fails, and so does one nobody made.
    start(&w, &form, "grace@example.org").await;
    let second = link_to(&w, "grace@example.org");
    db(|c| c.execute("UPDATE guest_links SET expires_at = NOW() - INTERVAL '1 minute'", &[]).unwrap());
    assert_eq!(verify(&w, &second).await.0, StatusCode::NOT_FOUND);
    assert_eq!(verify(&w, "00000000000000000000000000000000").await.0, StatusCode::NOT_FOUND);
    // The link is good for fifteen minutes.
    start(&w, &form, "hopper@example.org").await;
    let minutes: i64 = db(|c| c.query_one("SELECT (EXTRACT(EPOCH FROM expires_at - NOW()) / 60)::bigint FROM guest_links ORDER BY expires_at DESC LIMIT 1", &[]).unwrap().get(0));
    assert!((14..=15).contains(&minutes), "{minutes}");
}

// 3. The session's scope is one app and form, and any other route refuses it with token_scope.
#[tokio::test]
async fn a_guest_session_reaches_one_form_and_nothing_else() {
    let _db = common::TestDb::new();
    let (w, form) = ready().await;
    start(&w, &form, "ada@example.org").await;
    let (_, session) = verify(&w, &link_to(&w, "ada@example.org")).await;
    let token = session["token"].as_str().unwrap().to_string();

    let row: (String, Option<String>, Value, String) = db({
        let token = token.clone();
        move |c| {
            let r = c.query_one("SELECT kind, app_slug, scope, label FROM api_tokens WHERE token_hash = encode(sha256($1::bytea), 'hex')", &[&token.as_bytes()]).unwrap();
            (r.get(0), r.get(1), r.get(2), r.get(3))
        }
    });
    assert_eq!(row.0, "guest");
    assert_eq!(row.1.as_deref(), Some(w.slug.as_str()));
    assert_eq!(row.2["apps"], json!([w.slug]), "limited to this app");
    assert!(row.3.contains(&form), "and this form: {}", row.3);

    for (method, uri) in [
        ("GET", "/api/v1/apps".to_string()),
        ("GET", format!("/api/v1/apps/{}/records", w.slug)),
        ("GET", format!("/api/v1/apps/{}/tables/requests/records", w.slug)),
        ("POST", format!("/api/v1/apps/{}/records", w.slug)),
        ("GET", format!("/api/v1/apps/{}/views", w.slug)),
        ("GET", "/api/v1/workspaces".to_string()),
        ("GET", "/api/v1/settings".to_string()),
        ("POST", "/api/v1/mcp".to_string()),
    ] {
        let (s, v) = send(&w.app, method, &uri, &token, if method == "POST" { Some(json!({})) } else { None }).await;
        assert_eq!((s, v["code"].as_str()), (StatusCode::FORBIDDEN, Some("token_scope")), "{method} {uri}: {v}");
    }
}

// 4. The fourth request in an hour for an address is 429 and sends nothing. A network address and a form have limits too.
#[tokio::test]
async fn the_fourth_request_for_an_address_in_an_hour_is_refused_and_sends_nothing() {
    let _db = common::TestDb::new();
    let (w, form) = ready().await;
    for n in 1..=3 {
        assert_eq!(start(&w, &form, "ada@example.org").await.0, StatusCode::OK, "request {n}");
    }
    let (s, v) = start(&w, &form, "ada@example.org").await;
    assert_eq!((s, v["code"].as_str()), (StatusCode::TOO_MANY_REQUESTS, Some("rate_limited")), "{v}");
    assert_eq!(outbox(&w).len(), 3, "the fourth sent nothing");
    // The address limit does not depend on the case of the address.
    assert_eq!(start(&w, &form, "ADA@example.org").await.0, StatusCode::TOO_MANY_REQUESTS);

    // Twenty requests an hour from one network address, whoever they ask for.
    for n in 1..=20 {
        let (s, _) = start_with(&w, &form, json!({ "email": format!("person{n}@example.org") }), Some("203.0.113.9")).await;
        assert_eq!(s, StatusCode::OK, "network request {n}");
    }
    let (s, _) = start_with(&w, &form, json!({ "email": "one-more@example.org" }), Some("203.0.113.9")).await;
    assert_eq!(s, StatusCode::TOO_MANY_REQUESTS);
    let (s, _) = start_with(&w, &form, json!({ "email": "one-more@example.org" }), Some("203.0.113.10")).await;
    assert_eq!(s, StatusCode::OK, "another network address has its own");

    // Two hundred requests an hour for one form.
    let key = format!("guest-form:{form}");
    db(move |c| c.execute("INSERT INTO rate_limits (key, window_start, count) VALUES ($1, date_trunc('hour', NOW()), 200) ON CONFLICT (key, window_start) DO UPDATE SET count = 200", &[&key]).unwrap());
    let (s, _) = start_with(&w, &form, json!({ "email": "late@example.org" }), Some("203.0.113.11")).await;
    assert_eq!(s, StatusCode::TOO_MANY_REQUESTS);
}

// 5. A filled honeypot gets the neutral text and no mail.
#[tokio::test]
async fn a_filled_honeypot_gets_the_same_answer_and_no_mail() {
    let _db = common::TestDb::new();
    let (w, form) = ready().await;
    let (s, v) = start_with(&w, &form, json!({ "email": "bot@example.org", "website": "http://spam.example" }), Some("198.51.100.2")).await;
    assert_eq!((s, v["message"].as_str()), (StatusCode::OK, Some(NEUTRAL)), "{v}");
    assert_eq!((guests(&w), outbox(&w).len()), (0, 0), "no guest and no mail");
    // An empty honeypot is what a person sends.
    let (s, _) = start_with(&w, &form, json!({ "email": "ada@example.org", "website": "" }), Some("198.51.100.3")).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(outbox(&w).len(), 1);
}

// 6. With guests.enabled false nothing is created or sent. A closed form is the same.
#[tokio::test]
async fn nothing_happens_while_guests_are_off_or_the_form_is_closed() {
    let _db = common::TestDb::new();
    let w = world(1, 0).await;
    mail_on(&w);
    let form = guest_form(&w).await;
    // The default is off.
    let (s, v) = start(&w, &form, "ada@example.org").await;
    assert_eq!((s, v["message"].as_str()), (StatusCode::OK, Some(NEUTRAL)));
    assert_eq!((guests(&w), outbox(&w).len()), (0, 0));

    setting(&w, "guests.enabled", json!(true)).await;
    setting(&w, "forms.disabled", json!([form])).await;
    let (s, v) = start(&w, &form, "ada@example.org").await;
    assert_eq!((s, v["message"].as_str()), (StatusCode::OK, Some(NEUTRAL)));
    assert_eq!((guests(&w), outbox(&w).len()), (0, 0), "a closed form sends nothing");

    // A form that is not open to guests is the same, and so is a form whose audience is members.
    setting(&w, "forms.disabled", json!([])).await;
    let (s, v) = send(&w.app, "POST", &views_path(&w), &w.admin, Some(json!({ "name": "Members", "kind": "form", "ownership": "collaborative", "table_id": "requests", "config": { "form_fields": [{ "name": "kind" }] } }))).await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    let members = v["id"].as_str().or_else(|| v["view"]["id"].as_str()).unwrap().to_string();
    start(&w, &members, "ada@example.org").await;
    assert_eq!((guests(&w), outbox(&w).len()), (0, 0));

    // Turned on, and open, it works.
    start(&w, &form, "ada@example.org").await;
    assert_eq!(outbox(&w).len(), 1);

    // The settings are checked.
    let (s, _) = send(&w.app, "PUT", "/api/v1/settings/guests.enabled", &w.platform, Some(json!("yes"))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    let (s, _) = send(&w.app, "PUT", "/api/v1/settings/forms.disabled", &w.platform, Some(json!([1]))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
}

// 7. The token is stored only as a hash.
#[tokio::test]
async fn a_link_and_a_session_are_stored_only_as_hashes() {
    let _db = common::TestDb::new();
    let (w, form) = ready().await;
    start(&w, &form, "ada@example.org").await;
    let link = link_to(&w, "ada@example.org");
    let (_, session) = verify(&w, &link).await;
    let session = session["token"].as_str().unwrap().to_string();
    let (l, s) = (link.clone(), session.clone());
    let found = db(move |c| {
        let hashed: i64 = c.query_one("SELECT count(*) FROM guest_links WHERE token_hash = encode(sha256($1::bytea), 'hex')", &[&l.as_bytes()]).unwrap().get(0);
        let raw: i64 = c.query_one("SELECT count(*) FROM guest_links WHERE token_hash = $1", &[&l]).unwrap().get(0);
        let count = |c: &mut postgres::Client, sql: &str, v: &String| -> i64 { c.query_one(sql, &[v]).unwrap().get(0) };
        let leaked = count(c, "SELECT count(*) FROM guest_links g WHERE g::text LIKE '%' || $1 || '%'", &l)
            + count(c, "SELECT count(*) FROM api_tokens t WHERE t::text LIKE '%' || $1 || '%'", &s)
            + count(c, "SELECT count(*) FROM guests g WHERE g::text LIKE '%' || $1 || '%'", &l);
        (hashed, raw, leaked)
    });
    assert_eq!(found, (1, 0, 0), "the hash is stored, never the token");
}

// 8. A guest address cannot be resolved as a directory person, and cannot be added to a workspace.
#[tokio::test]
async fn a_guest_is_never_a_directory_person() {
    let _db = common::TestDb::new();
    let (w, form) = ready().await;
    start(&w, &form, "ada@example.org").await;
    let id: Uuid = db(|c| c.query_one("SELECT id FROM guests WHERE email = 'ada@example.org'", &[]).unwrap().get(0));
    let address = format!("guest:{id}@guest.invalid");
    assert!(scaffoldry_server::service::identity::resolve_user(&address, &w.state).is_none(), "no directory person");

    let (s, v) = send(&w.app, "POST", &format!("/api/v1/workspaces/{WORKSPACE}/collaborators"), &w.admin, Some(json!({ "eppn": address, "role": "viewer" }))).await;
    assert!(s.is_client_error(), "a guest cannot be a member: {s} {v}");
    let members = w.state.collaborators.read().unwrap().get(WORKSPACE).cloned().unwrap_or_default();
    assert!(members.iter().all(|m| !m.eppn.contains("guest.invalid")), "{members:?}");
    // The session's own address is the same shape.
    verify(&w, &link_to(&w, "ada@example.org")).await;
    let who: String = db(|c| c.query_one("SELECT eppn FROM api_tokens WHERE kind = 'guest' ORDER BY created_at DESC LIMIT 1", &[]).unwrap().get(0));
    assert_eq!(who, address);
}
