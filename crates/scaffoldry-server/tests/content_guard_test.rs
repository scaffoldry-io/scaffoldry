//! Sensitive content phase 3: new writes, imports and plain-text files are checked. Each detector flags, warns or blocks, as the
//! organization chooses. A matched value is never stored, and never in a response.

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::Router;
use http_body_util::BodyExt;
use scaffoldry_engine::{AppManifest, AppTable, FieldSpec, FieldType};
use scaffoldry_server::build_app_with_state;
use scaffoldry_server::service::identity::issue_test_token_and_user;
use scaffoldry_server::state::{CollaboratorRecord, ServerState};
use serde_json::{json, Value};
use std::sync::Arc;
use tower::ServiceExt;
use uuid::Uuid;

const WORKSPACE: &str = "ws-bio-lab";
const COMPLIANCE: &str = "hold.officer@state.edu";

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
                let _ = c.execute("DELETE FROM data_labels WHERE app_slug LIKE $1", &[&like]);
                let _ = c.execute("DELETE FROM content_findings WHERE app_slug LIKE $1", &[&like]);
                let _ = c.execute("DELETE FROM content_expected WHERE app_slug LIKE $1", &[&like]);
                let _ = c.execute("DELETE FROM authorized_stores WHERE app_slug LIKE $1", &[&like]);
                let _ = c.execute("DELETE FROM app_manifests WHERE slug LIKE $1", &[&like]);
            }
        })
        .join();
    }
}

fn manifest(slug: &str) -> AppManifest {
    let notes = vec![
        FieldSpec::simple("title", "Title", FieldType::Text, false, false),
        FieldSpec::simple("body", "Body", FieldType::LongText, false, false),
        FieldSpec::simple("amount", "Amount", FieldType::Number, false, false),
    ];
    let cards = vec![FieldSpec::simple("holder", "Holder", FieldType::Text, false, false), FieldSpec::simple("card", "Card", FieldType::Text, false, false)];
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
        tables: vec![table("notes", notes, "title"), table("cards", cards, "holder")],
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
    slug: String,
    platform: String,
    compliance: String,
    editor: String,
    editor_eppn: String,
    _cleanup: Cleanup,
}

async fn world() -> World {
    let tag = Uuid::new_v4().to_string()[..8].to_string();
    let slug = format!("dg-{tag}");
    let (editor_e, viewer_e) = (format!("editor-{tag}@state.edu"), format!("viewer-{tag}@state.edu"));
    let state = Arc::new(ServerState::new().expect("state"));
    state.seed_demo().expect("seed");
    state.engine.write().unwrap().register_manifest(manifest(&slug)).unwrap();
    collaborator(&state, &editor_e, "editor");
    collaborator(&state, &viewer_e, "viewer");
    // Issuing the token makes the person. The role is put on them after.
    let compliance = issue_test_token_and_user(COMPLIANCE);
    db(|c| {
        c.execute(
            "INSERT INTO roles (id, person_id, organization_id, role_title, scoped_affiliation, is_primary, source) \
             SELECT gen_random_uuid(), p.id, o.id, 'Compliance Officer', 'compliance', true, 'api' \
             FROM persons p, organizations o WHERE LOWER(p.eppn) = LOWER($1) AND o.parent_id IS NULL ON CONFLICT DO NOTHING",
            &[&COMPLIANCE],
        )
        .unwrap();
    });
    World {
        app: build_app_with_state(state.clone()).expect("router"),
        state: state.clone(),
        platform: issue_test_token_and_user("jordan.lee@state.edu"),
        compliance,
        editor: issue_test_token_and_user(&editor_e),
        editor_eppn: editor_e.clone(),
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


const SSN: &str = "123-45-6789";
const SSN_B: &str = "234-56-7890";
const CARD: &str = "4111 1111 1111 1111";

/// Whether text holds the planted card number, as written or with its spaces removed. A bare "4111" is not enough: a
/// random id or a SHA-256 holds those four digits about once in every few thousand values.
fn holds_card(text: &str) -> bool {
    text.contains(CARD) || text.contains(&CARD.replace(' ', ""))
}

async fn findings(w: &World, query: &str) -> Vec<Value> {
    let (s, v) = send(&w.app, "GET", &format!("/api/v1/admin/content-findings?app_slug={}&{query}", w.slug), &w.compliance, None).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    v["findings"].as_array().unwrap().clone()
}

/// Everything the database holds that a value could have leaked into, as one string.
fn everything_stored() -> String {
    db(|c| {
        let mut out = String::new();
        for sql in [
            "SELECT coalesce(string_agg(row_to_json(f)::text, ' '), '') FROM content_findings f",
            "SELECT coalesce(string_agg(row_to_json(e)::text, ' '), '') FROM content_expected e",
            "SELECT coalesce(string_agg(l.rationale || ' ' || l.payload::text, ' '), '') FROM governance_ledger l",
            "SELECT coalesce(string_agg(j.payload::text || ' ' || coalesce(j.result::text, '') || ' ' || coalesce(j.log::text, '') || ' ' || coalesce(j.checkpoint::text, ''), ' '), '') FROM jobs j",
        ] {
            out.push_str(&c.query_one(sql, &[]).unwrap().get::<_, String>(0));
            out.push(' ');
        }
        out
    })
}


/// Sets one detector's action, as a Platform Admin does in the settings.
async fn set_action(w: &World, detector: &str, action: &str) {
    let (_, v) = send(&w.app, "GET", "/api/v1/admin/sensitivity-settings", &w.compliance, None).await;
    let mut list = v["detectors"].clone();
    let at = list.as_array().unwrap().iter().position(|d| d["id"] == detector).unwrap();
    list[at]["action"] = json!(action);
    let (s, v) = send(&w.app, "PUT", "/api/v1/settings/sensitivity.detectors", &w.platform, Some(list)).await;
    assert_eq!(s, StatusCode::OK, "{v}");
}

async fn write(w: &World, table: &str, data: Value) -> (StatusCode, Value) {
    let mut data = data;
    data["_table_id"] = json!(table);
    send(&w.app, "POST", &format!("/api/v1/apps/{}/records", w.slug), &w.editor, Some(json!({ "data": data }))).await
}

fn stored(w: &World, table: &str) -> i64 {
    let (slug, table) = (w.slug.clone(), table.to_string());
    db(move |c| c.query_one("SELECT count(*) FROM dataset_records WHERE app_slug = $1 AND table_id = $2", &[&slug, &table]).unwrap().get(0))
}

async fn pci_store(w: &World) {
    let body = json!({ "category": "pci", "scope": "field", "app_slug": w.slug, "table_id": "cards", "field": "card", "reason": "Payments." });
    assert_eq!(send(&w.app, "PUT", "/api/v1/admin/authorized-stores", &w.compliance, Some(body)).await.0, StatusCode::OK);
}

// 1. A write with a card number in a text field under `flag` succeeds and leaves a finding. Under `block` it is 422 and nothing is stored.
#[tokio::test]
async fn a_card_in_a_write_is_flagged_or_blocked_as_the_organization_chooses() {
    let _db = common::TestDb::new();
    let w = world().await;
    let (s, v) = write(&w, "notes", json!({ "title": "paid", "body": format!("paid with {CARD}") })).await;
    assert_eq!(s, StatusCode::CREATED, "flag is the default and the write succeeds: {v}");
    assert!(v.get("sensitive_content").is_none(), "a flag says nothing to the writer: {v}");
    let rows = findings(&w, "").await;
    assert_eq!(rows.len(), 1, "{rows:?}");
    assert_eq!((rows[0]["source"].as_str(), rows[0]["field"].as_str(), rows[0]["detector_id"].as_str(), rows[0]["match_count"].as_i64()), (Some("write"), Some("body"), Some("payment_card"), Some(1)));
    assert!(!holds_card(&everything_stored()), "no card number was stored in a finding");

    set_action(&w, "payment_card", "block").await;
    let before = stored(&w, "notes");
    let (s, v) = write(&w, "notes", json!({ "title": "again", "body": format!("card {CARD}") })).await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "{v}");
    assert_eq!(v["code"], "sensitive_content_blocked", "{v}");
    let text = v.to_string();
    assert!(text.contains("body") && text.contains("payment_card"), "it names the field and the detector: {text}");
    assert!(!holds_card(&text), "and never the value: {text}");
    assert_eq!(stored(&w, "notes"), before, "nothing was stored");
    // The message says where the category may live, once a place is designated.
    pci_store(&w).await;
    let (_, v) = write(&w, "notes", json!({ "title": "again", "body": format!("card {CARD}") })).await;
    assert!(v.to_string().contains("cards"), "it names the authorized stores for the category: {v}");
}

// 2. The same write into a field designated as an authorized store for `pci` succeeds with no finding, under every action.
#[tokio::test]
async fn a_write_into_an_authorized_store_is_never_a_finding_whatever_the_action() {
    let _db = common::TestDb::new();
    let w = world().await;
    pci_store(&w).await;
    for action in ["flag", "warn", "block"] {
        set_action(&w, "payment_card", action).await;
        let (s, v) = write(&w, "cards", json!({ "holder": "ada", "card": CARD })).await;
        assert_eq!(s, StatusCode::CREATED, "{action}: {v}");
        assert!(v.get("sensitive_content").is_none(), "{action}: {v}");
    }
    assert!(findings(&w, "").await.is_empty(), "no finding for a card in the store");
    // The same number anywhere else is still a finding.
    set_action(&w, "payment_card", "flag").await;
    write(&w, "notes", json!({ "title": "t", "body": CARD })).await;
    assert_eq!(findings(&w, "").await.len(), 1);
}

// 3. A write that changes only a clean field runs no detector over the unchanged sensitive field.
#[tokio::test]
async fn an_update_that_changes_a_clean_field_does_not_look_at_the_unchanged_one() {
    let _db = common::TestDb::new();
    let w = world().await;
    let (_, made) = write(&w, "notes", json!({ "title": "a", "body": format!("ssn {SSN}") })).await;
    let id = made["id"].as_str().unwrap().to_string();
    let before = findings(&w, "").await;
    assert_eq!(before.len(), 1);
    set_action(&w, "us_ssn", "block").await;
    // The body still holds the number, and the detector now blocks. Only the title changes, so nothing is looked at.
    let (s, v) = send(&w.app, "PATCH", &format!("/api/v1/apps/{}/records/{id}", w.slug), &w.editor, Some(json!({ "version": made["version"], "data": { "_table_id": "notes", "title": "b", "body": format!("ssn {SSN}") } }))).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    // Changing the body to a new number is looked at, and blocked.
    let (s, v) = send(&w.app, "PATCH", &format!("/api/v1/apps/{}/records/{id}", w.slug), &w.editor, Some(json!({ "version": v["version"], "data": { "_table_id": "notes", "title": "b", "body": format!("ssn {SSN_B}") } }))).await;
    assert_eq!((s, v["code"].as_str()), (StatusCode::UNPROCESSABLE_ENTITY, Some("sensitive_content_blocked")), "{v}");
}

// 4. The `warn` response contains the field and the detector and not the value.
#[tokio::test]
async fn a_warning_names_the_field_and_the_detector_and_never_the_value() {
    let _db = common::TestDb::new();
    let w = world().await;
    set_action(&w, "us_ssn", "warn").await;
    let (s, v) = write(&w, "notes", json!({ "title": "a", "body": format!("ssn {SSN}") })).await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    let warned = v["sensitive_content"].as_array().unwrap_or_else(|| panic!("a sensitive_content note in {v}"));
    assert_eq!(warned.len(), 1);
    assert_eq!((warned[0]["field"].as_str(), warned[0]["detector"].as_str(), warned[0]["category"].as_str()), (Some("body"), Some("us_ssn"), Some("pii")));
    assert!(!v["sensitive_content"].to_string().contains(SSN), "the note holds no value: {v}");
    assert!(findings(&w, "").await.is_empty(), "a warning leaves no finding");
}

// 7. A write of a 10 MB string scans only the first 64 KB and says so in the finding note.
#[tokio::test]
async fn a_ten_megabyte_string_is_read_for_its_first_64_kb_and_the_finding_says_so() {
    let _db = common::TestDb::new();
    let w = world().await;
    let editor = w.editor_eppn.clone();
    let user = scaffoldry_server::service::identity::resolve_user(&editor, &w.state).expect("the editor resolves");
    // An SSN near the start, and 10 MB of filler. A second SSN after 64 KB is never looked at.
    let mut text = format!("start {SSN} ");
    text.push_str(&"x".repeat(10 * 1024 * 1024));
    text.push_str(&format!(" end {SSN_B}"));
    let saved = scaffoldry_server::service::records::create_record(&user, &scaffoldry_server::service::history::Origin::user(), &w.slug, &json!({ "_table_id": "notes", "title": text, "body": "plain" }), &w.state);
    assert!(saved.is_ok(), "the write was refused: {:?}", saved.as_ref().err().map(|e| format!("{e:?}").chars().take(300).collect::<String>()));
    let rows = findings(&w, "").await;
    assert_eq!(rows.len(), 1, "{rows:?}");
    assert_eq!(rows[0]["field"], "title");
    assert_eq!(rows[0]["match_count"], 1, "the number after 64 KB was not found: {rows:?}");
    let note = rows[0]["note"].as_str().unwrap_or("");
    assert!(note.contains("first 64 KB"), "the finding says only the start was read: {rows:?}");
    assert!(!everything_stored().contains(SSN_B));
}
