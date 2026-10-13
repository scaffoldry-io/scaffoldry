//! Record history phase 4: the disclosure log.
//!
//! The log records candidate disclosures of sensitive fields: an export, or a bulk read by an
//! agent. It does not decide what is a disclosure. Each test builds an app of its own, with one
//! sensitive field, and cleans up after itself.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::Router;
use http_body_util::BodyExt;
use scaffoldry_engine::{AppManifest, AppTable, AppView, FieldSpec, FieldType, SelectOption, SelectOptionColor, ViewType};
use scaffoldry_server::build_app_with_state;
use scaffoldry_server::service::admin::ADMIN_ROUTES;
use scaffoldry_server::service::disclosure::{
    log_disclosure, Release, AGENT_RELEASES, AGENT_RELEASE_EXEMPT, AGENT_READ_THRESHOLD, EXPORT_ROW_LIMIT,
};
use scaffoldry_server::service::identity::{hash_token, issue_test_token_and_user};
use scaffoldry_server::state::{CollaboratorRecord, ServerState};
use serde_json::{json, Value};
use std::sync::Arc;
use tower::ServiceExt;
use uuid::Uuid;

const WORKSPACE: &str = "ws-bio-lab";
const ADMIN: &str = "jordan.lee@state.edu";
const COMPLIANCE: &str = "compliance.officer@state.edu";
const FACULTY: &str = "prof.curie@science.state.edu";
const STUDENTS: i64 = 40;

fn db_url() -> String {
    std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://scaffoldry:scaffoldry_dev_password@127.0.0.1:5433/scaffoldry".to_string())
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
                let _ = c.execute("DELETE FROM roles WHERE scoped_affiliation = 'compliance' AND person_id IN (SELECT id FROM persons WHERE LOWER(eppn) = LOWER($1))", &[&COMPLIANCE]);
                let _ = c.execute("DELETE FROM disclosure_records WHERE disclosure_id IN (SELECT id FROM disclosure_log WHERE app_slug LIKE $1)", &[&like]);
                let _ = c.execute("DELETE FROM disclosure_log WHERE app_slug LIKE $1", &[&like]);
                let _ = c.execute("DELETE FROM workspace_collaborators WHERE workspace_id = $1 AND eppn LIKE $2", &[&WORKSPACE, &like]);
                let _ = c.execute("DELETE FROM record_history WHERE app_slug LIKE $1", &[&like]);
                let _ = c.execute("DELETE FROM dataset_records WHERE app_slug LIKE $1", &[&like]);
            }
        })
        .join();
    }
}

fn manifest(slug: &str) -> AppManifest {
    let mut status = FieldSpec::simple("status", "Status", FieldType::Select, false, false);
    status.select_options = vec![
        SelectOption { id: "opt_a".into(), label: "A".into(), color: SelectOptionColor::Ok, order: 0 },
        SelectOption { id: "opt_b".into(), label: "B".into(), color: SelectOptionColor::Warn, order: 1 },
    ];
    let mut board = AppView::table("board", "Board", ViewType::Kanban, vec![]);
    board.table_id = Some("students".to_string());
    board.kanban_column_field = Some("status".to_string());
    AppManifest {
        slug: slug.to_string(),
        title: "Registrar".to_string(),
        description: String::new(),
        organization_code: "BIO".to_string(),
        department: "biology".to_string(),
        workspace_id: Some(WORKSPACE.to_string()),
        herm_capability_id: None,
        custom_domain: None,
        custom_domain_verified: false,
        tables: vec![AppTable {
            id: "students".to_string(),
            name: "students".to_string(),
            slug: "students".to_string(),
            description: None,
            icon: None,
            fields: vec![
                FieldSpec::simple("name", "Name", FieldType::Text, false, false),
                FieldSpec::simple("major", "Major", FieldType::Text, false, false),
                FieldSpec::simple("student_id", "Student ID", FieldType::Text, false, true),
                status,
            ],
            primary_field: Some("name".to_string()),
            sample_records: vec![],
        }],
        relationships: vec![],
        views: vec![board],
        ceds_mappings: Default::default(),
        pages: vec![],
        access: Default::default(),
    }
}

struct World {
    app: Router,
    state: Arc<ServerState>,
    slug: String,
    admin: String,
    faculty: String,
    compliance: String,
    owner: String,
    editor: String,
    stranger: String,
    agent: String,
    _cleanup: Cleanup,
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
    });
}

async fn world() -> World {
    // Identities first, so the server loads them at boot.
    let admin = issue_test_token_and_user(ADMIN);
    let faculty = issue_test_token_and_user(FACULTY);
    let compliance = issue_test_token_and_user(COMPLIANCE);

    let tag = Uuid::new_v4().to_string()[..8].to_string();
    let slug = format!("dl-{tag}");
    let (owner_eppn, editor_eppn, stranger_eppn) =
        (format!("owner-{tag}@state.edu"), format!("editor-{tag}@state.edu"), format!("stranger-{tag}@state.edu"));

    let state = Arc::new(ServerState::new().expect("state"));
    state.seed_demo().expect("seed");
    // A compliance officer is appointed, not claimed. The role sits on the root unit.
    let granted = db(|c| {
        c.execute("DELETE FROM roles WHERE scoped_affiliation = 'compliance' AND person_id IN (SELECT id FROM persons WHERE LOWER(eppn) = LOWER($1))", &[&COMPLIANCE]).unwrap();
        c.execute(
            "INSERT INTO roles (id, person_id, organization_id, role_title, scoped_affiliation, is_primary, source) \
             SELECT gen_random_uuid(), p.id, o.id, 'Compliance Officer', 'compliance', true, 'api' \
             FROM persons p, organizations o WHERE LOWER(p.eppn) = LOWER($1) AND o.parent_id IS NULL",
            &[&COMPLIANCE],
        )
        .unwrap()
    });
    assert_eq!(granted, 1, "the compliance officer exists");
    state.engine.write().unwrap().register_manifest(manifest(&slug)).unwrap();
    collaborator(&state, &owner_eppn, "admin");
    collaborator(&state, &editor_eppn, "editor");

    let s = slug.clone();
    db(move |c| {
        c.execute(
            "INSERT INTO dataset_records (id, app_slug, data, ceds_mapping, is_ferpa_sensitive, table_id, created_at) \
             SELECT $1::text || lpad(g::text, 6, '0'), $1::text, \
                    jsonb_build_object('_table_id', 'students', 'name', 'student-' || g, 'major', 'Biology', 'student_id', 'S' || lpad(g::text, 6, '0')) \
                    || CASE WHEN g <= 30 THEN jsonb_build_object('status', CASE WHEN g % 2 = 0 THEN 'opt_a' ELSE 'opt_b' END) ELSE '{}'::jsonb END, \
                    '{}'::jsonb, false, 'students', TIMESTAMPTZ '2026-01-01 00:00:00+00' + g * interval '1 millisecond' \
             FROM generate_series(1, $2::bigint) g",
            &[&s, &STUDENTS],
        )
        .unwrap();
    });

    // An agent token, minted for the Platform Admin.
    let raw = format!("sct_agent_{}", Uuid::new_v4().simple());
    let hash = hash_token(&raw);
    db(move |c| {
        let expires = chrono::Utc::now() + chrono::Duration::days(1);
        c.execute(
            "INSERT INTO api_tokens (token_hash, id, kind, eppn, label, created_at, expires_at) VALUES ($1, $2, 'agent', $3, 'Registrar bot', NOW(), $4)",
            &[&hash, &Uuid::new_v4(), &ADMIN, &expires],
        )
        .unwrap();
    });

    World {
        app: build_app_with_state(state.clone()).expect("router"),
        state,
        admin,
        faculty,
        compliance,
        owner: issue_test_token_and_user(&owner_eppn),
        editor: issue_test_token_and_user(&editor_eppn),
        stranger: issue_test_token_and_user(&stranger_eppn),
        agent: raw,
        slug,
        _cleanup: Cleanup(tag),
    }
}

async fn send(app: &Router, method: &str, uri: &str, token: &str, body: Option<Value>) -> (StatusCode, Value) {
    let (status, _, bytes) = send_raw(app, method, uri, token, body).await;
    (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
}

async fn send_raw(app: &Router, method: &str, uri: &str, token: &str, body: Option<Value>) -> (StatusCode, axum::http::HeaderMap, Vec<u8>) {
    let b = Request::builder().method(method).uri(uri).header("authorization", format!("Bearer {token}"));
    let req = match body {
        Some(v) => b.header("content-type", "application/json").body(Body::from(v.to_string())).unwrap(),
        None => b.body(Body::empty()).unwrap(),
    };
    let resp = app.clone().oneshot(req).await.unwrap();
    let (status, headers) = (resp.status(), resp.headers().clone());
    (status, headers, resp.into_body().collect().await.unwrap().to_bytes().to_vec())
}

fn id(w: &World, n: i64) -> String {
    format!("{}{:06}", w.slug, n)
}

fn ids(w: &World, n: i64) -> Vec<String> {
    (1..=n).map(|i| id(w, i)).collect()
}

/// The entries of this app, oldest first: (kind, actor, record_count, fields, recipient, note, truncated).
type Entry = (String, String, i64, Vec<String>, Option<String>, String, bool);

fn entries(w: &World) -> Vec<Entry> {
    let slug = w.slug.clone();
    db(move |c| {
        c.query(
            "SELECT kind, actor, record_count, fields, recipient, note, records_truncated FROM disclosure_log WHERE app_slug = $1 ORDER BY at, id",
            &[&slug],
        )
        .unwrap()
        .iter()
        .map(|r| (r.get(0), r.get(1), r.get(2), r.get(3), r.get(4), r.get(5), r.get(6)))
        .collect()
    })
}

fn record_rows(w: &World) -> i64 {
    let slug = w.slug.clone();
    db(move |c| {
        c.query_one(
            "SELECT count(*) FROM disclosure_records WHERE disclosure_id IN (SELECT id FROM disclosure_log WHERE app_slug = $1)",
            &[&slug],
        )
        .unwrap()
        .get(0)
    })
}

fn export(w: &World, n: i64, fields: &[&str]) -> Option<Uuid> {
    log_disclosure(
        &w.state,
        Release {
            kind: "export",
            actor: "dr.rivera@state.edu",
            app_slug: &w.slug,
            table_id: Some("students"),
            record_ids: ids(w, n),
            fields: fields.iter().map(|f| f.to_string()).collect(),
            recipient: None,
            note: "CSV export",
        },
    )
    .expect("log_disclosure")
}

// 1. An export that includes a sensitive field writes one entry with the right count and fields and
//    one record row per record. An export of only non-sensitive fields writes nothing.
#[tokio::test]
async fn an_export_with_a_sensitive_field_is_logged_and_one_without_is_not() {
    let w = world().await;
    assert!(export(&w, 12, &["name", "major"]).is_none(), "no sensitive field, nothing to log");
    assert!(entries(&w).is_empty());

    assert!(export(&w, 12, &["name", "major", "student_id"]).is_some());
    let log = entries(&w);
    assert_eq!(log.len(), 1);
    let (kind, actor, count, fields, recipient, note, truncated) = &log[0];
    assert_eq!((kind.as_str(), actor.as_str(), *count), ("export", "dr.rivera@state.edu", 12));
    assert_eq!(fields, &vec!["student_id".to_string()], "only the sensitive fields are kept");
    assert_eq!((recipient, note.as_str(), *truncated), (&None, "CSV export", false));
    assert_eq!(record_rows(&w), 12);

    // An unknown kind is refused, so a typo cannot write a kind nothing reads.
    let bad = log_disclosure(
        &w.state,
        Release { kind: "sharing", actor: "x@state.edu", app_slug: &w.slug, table_id: Some("students"), record_ids: ids(&w, 1), fields: vec!["student_id".into()], recipient: None, note: "" },
    );
    assert!(bad.is_err(), "kind must be one of the known kinds");
}

// 2. An agent token reading 26 records with a sensitive field writes an agent_read entry whose
//    recipient is the token label. 25 writes none. A user session writes none.
#[tokio::test]
async fn an_agent_reading_more_than_25_sensitive_records_is_logged_a_person_is_not() {
    let w = world().await;
    assert_eq!(AGENT_READ_THRESHOLD, 25);
    let list = |n: i64| format!("/api/v1/apps/{}/tables/students/records?limit={n}", w.slug);

    let (s, body) = send(&w.app, "GET", &list(25), &w.agent, None).await;
    assert_eq!(s, StatusCode::OK, "{body}");
    assert_eq!(body["records"].as_array().unwrap().len(), 25);
    assert!(entries(&w).is_empty(), "25 is not more than 25");

    let (s, body) = send(&w.app, "GET", &list(26), &w.admin, None).await;
    assert_eq!(s, StatusCode::OK, "{body}");
    assert!(entries(&w).is_empty(), "a person reading the same writes none");

    let (s, body) = send(&w.app, "GET", &list(26), &w.agent, None).await;
    assert_eq!(s, StatusCode::OK, "{body}");
    assert_eq!(body["records"].as_array().unwrap().len(), 26);
    let log = entries(&w);
    assert_eq!(log.len(), 1, "{log:?}");
    let (kind, actor, count, fields, recipient, note, _) = &log[0];
    assert_eq!((kind.as_str(), actor.as_str(), *count), ("agent_read", ADMIN, 26));
    assert_eq!(fields, &vec!["student_id".to_string()]);
    assert_eq!((recipient.as_deref(), note.as_str()), (Some("Registrar bot"), "read by an agent"));
    assert_eq!(record_rows(&w), 26);
}

// 2b. The other ways an agent reads: the whole-app list, the older sorted path, and the MCP tool.
#[tokio::test]
async fn every_way_an_agent_lists_records_is_logged() {
    let w = world().await;
    let base = format!("/api/v1/apps/{}", w.slug);

    let (s, body) = send(&w.app, "GET", &format!("{base}/records"), &w.agent, None).await;
    assert_eq!(s, StatusCode::OK, "{body}");
    assert_eq!(entries(&w).len(), 1, "the whole-app list");
    assert_eq!(entries(&w)[0].2, STUDENTS);

    // The older path reads the app, then pages. The entry counts what the page returned.
    let (s, body) = send(&w.app, "GET", &format!("{base}/tables/students/records?sort_field=name&page_size=30"), &w.agent, None).await;
    assert_eq!(s, StatusCode::OK, "{body}");
    let log = entries(&w);
    assert_eq!(log.len(), 2, "{log:?}");
    assert_eq!(log[1].2, 30, "30 came back, not the whole app");

    let call = json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/call",
        "params": { "name": "list_records", "arguments": { "app_slug": w.slug, "table_id": "students", "limit": 30 } } });
    let (s, body) = send(&w.app, "POST", "/api/v1/mcp", &w.agent, Some(call.clone())).await;
    assert_eq!(s, StatusCode::OK, "{body}");
    assert_ne!(body["result"]["isError"], json!(true), "{body}");
    let log = entries(&w);
    assert_eq!(log.len(), 3, "the MCP tool: {log:?}");
    assert_eq!((log[2].0.as_str(), log[2].2), ("agent_read", 30));
}

// 2c. The board view puts every record it shows into stages. An agent fetching it reads those records, so
//     the read is logged for what came back (30 of the 40 are in a stage), and a person's fetch is not.
#[tokio::test]
async fn an_agent_fetching_a_board_is_logged_for_the_records_it_receives() {
    let w = world().await;
    let call = json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/call",
        "params": { "name": "get_view_data", "arguments": { "app_slug": w.slug, "view_id": "board" } } });

    let (s, body) = send(&w.app, "POST", "/api/v1/mcp", &w.admin, Some(call.clone())).await;
    assert_eq!(s, StatusCode::OK, "{body}");
    assert_ne!(body["result"]["isError"], json!(true), "{body}");
    assert!(entries(&w).is_empty(), "a person fetching the board writes none");

    let (s, body) = send(&w.app, "POST", "/api/v1/mcp", &w.agent, Some(call)).await;
    assert_eq!(s, StatusCode::OK, "{body}");
    assert_ne!(body["result"]["isError"], json!(true), "{body}");
    let log = entries(&w);
    assert_eq!(log.len(), 1, "{log:?}");
    let (kind, actor, count, fields, recipient, _, _) = &log[0];
    assert_eq!((kind.as_str(), actor.as_str(), *count), ("agent_read", ADMIN, 30), "only records in a stage were returned");
    assert_eq!(fields, &vec!["student_id".to_string()]);
    assert_eq!(recipient.as_deref(), Some("Registrar bot"));
    assert_eq!(record_rows(&w), 30);
}

// 3. An export of 120,000 records sets records_truncated and keeps the count.
#[tokio::test]
async fn an_entry_above_100000_records_keeps_the_count_and_drops_the_record_rows() {
    let w = world().await;
    let big: Vec<String> = (1..=120_000).map(|i| format!("{}-r{i}", w.slug)).collect();
    log_disclosure(
        &w.state,
        Release { kind: "export", actor: "dr.rivera@state.edu", app_slug: &w.slug, table_id: Some("students"), record_ids: big, fields: vec!["student_id".into()], recipient: None, note: "big" },
    )
    .expect("logged");
    let log = entries(&w);
    assert_eq!(log.len(), 1);
    assert_eq!((log[0].2, log[0].6), (120_000, true));
    assert_eq!(log[0].3, vec!["student_id".to_string()], "the fields are kept");
    assert_eq!(record_rows(&w), 0, "above the cap no record is listed");

    // At the cap, every record is listed.
    export(&w, STUDENTS, &["student_id"]);
    assert!(!entries(&w)[1].6, "at the cap the records are listed");
    assert_eq!(record_rows(&w), STUDENTS);
}

// 4. GET .../records/{id}/disclosures returns only entries that include that record, and 404s for a
//    caller without access.
#[tokio::test]
async fn a_records_own_disclosures_are_listed_for_those_who_may_see_them() {
    let w = world().await;
    export(&w, 10, &["student_id"]); // records 1 to 10
    let tail: Vec<String> = (8..=20).map(|i| id(&w, i)).collect();
    log_disclosure(
        &w.state,
        Release { kind: "export", actor: "dr.park@state.edu", app_slug: &w.slug, table_id: Some("students"), record_ids: tail, fields: vec!["student_id".into()], recipient: Some("Registrar"), note: "second" },
    )
    .unwrap();

    let path = |n: i64| format!("/api/v1/apps/{}/records/{}/disclosures", w.slug, id(&w, n));
    let (s, body) = send(&w.app, "GET", &path(9), &w.owner, None).await;
    assert_eq!(s, StatusCode::OK, "{body}");
    let list = body["disclosures"].as_array().unwrap();
    assert_eq!(list.len(), 2, "record 9 is in both: {body}");
    assert_eq!(list[0]["actor"], "dr.park@state.edu", "newest first");

    let (_, body) = send(&w.app, "GET", &path(3), &w.owner, None).await;
    assert_eq!(body["disclosures"].as_array().unwrap().len(), 1, "record 3 is only in the first");
    let (_, body) = send(&w.app, "GET", &path(30), &w.owner, None).await;
    assert!(body["disclosures"].as_array().unwrap().is_empty(), "record 30 was never released");

    for who in [&w.admin, &w.compliance] {
        let (s, _) = send(&w.app, "GET", &path(9), who, None).await;
        assert_eq!(s, StatusCode::OK, "a Platform Admin and compliance may read it");
    }
    let (s, _) = send(&w.app, "GET", &path(9), &w.stranger, None).await;
    assert_eq!(s, StatusCode::NOT_FOUND, "a stranger cannot learn that the record was disclosed");
    let (s, _) = send(&w.app, "GET", &path(9), &w.editor, None).await;
    assert_eq!(s, StatusCode::NOT_FOUND, "an editor can work in the app but is not its owner or admin");
}

// 5. A faculty caller is 403 on /admin/disclosures. A compliance caller is 200.
#[tokio::test]
async fn the_admin_log_is_for_a_platform_admin_or_compliance() {
    let w = world().await;
    assert!(ADMIN_ROUTES.contains(&("GET", "/admin/disclosures")));
    assert!(ADMIN_ROUTES.contains(&("GET", "/admin/disclosures/export")));
    export(&w, 5, &["student_id"]);
    export(&w, 3, &["student_id"]);

    let all = format!("/api/v1/admin/disclosures?app={}", w.slug);
    let (s, _) = send(&w.app, "GET", &all, &w.faculty, None).await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    for who in [&w.compliance, &w.admin] {
        let (s, body) = send(&w.app, "GET", &all, who, None).await;
        assert_eq!(s, StatusCode::OK, "{body}");
        assert_eq!(body["entries"].as_array().unwrap().len(), 2);
    }

    // Filters: by record, actor, kind, and a page at a time.
    let (_, body) = send(&w.app, "GET", &format!("{all}&record={}", id(&w, 4)), &w.compliance, None).await;
    assert_eq!(body["entries"].as_array().unwrap().len(), 1, "only the 5-record entry holds record 4");
    let (_, body) = send(&w.app, "GET", &format!("{all}&kind=agent_read"), &w.compliance, None).await;
    assert!(body["entries"].as_array().unwrap().is_empty());
    let (_, body) = send(&w.app, "GET", &format!("{all}&actor=dr.rivera@state.edu&limit=1"), &w.compliance, None).await;
    assert_eq!(body["entries"].as_array().unwrap().len(), 1);
    let cursor = body["next_cursor"].as_str().expect("a second page").to_string();
    let (_, body) = send(&w.app, "GET", &format!("{all}&actor=dr.rivera@state.edu&limit=1&cursor={cursor}"), &w.compliance, None).await;
    assert_eq!(body["entries"].as_array().unwrap().len(), 1);
    assert!(body["next_cursor"].is_null());
}

// 5b. The export of the log is capped, in two formats, and is itself a ledger entry.
#[tokio::test]
async fn exporting_the_log_is_capped_and_is_a_ledger_entry() {
    let w = world().await;
    assert_eq!(EXPORT_ROW_LIMIT, 50_000);
    export(&w, 5, &["student_id"]);

    let ledger_count = |body: &Value| body["entries"].as_array().unwrap().iter().filter(|e| e["decision_type"] == "DisclosureLogExported").count();
    let (_, before) = send(&w.app, "GET", "/api/v1/governance/ledger", &w.admin, None).await;

    let (s, headers, bytes) = send_raw(&w.app, "GET", &format!("/api/v1/admin/disclosures/export?app={}&format=csv", w.slug), &w.compliance, None).await;
    assert_eq!(s, StatusCode::OK);
    assert!(headers["content-type"].to_str().unwrap().starts_with("text/csv"));
    let csv = String::from_utf8(bytes).unwrap();
    assert!(csv.starts_with("id,at,actor,kind,app_slug,table_id,record_count,fields,recipient,note,records_truncated\n"), "{csv}");
    assert_eq!(csv.lines().count(), 2, "a header and one entry");

    let (_, after) = send(&w.app, "GET", "/api/v1/governance/ledger", &w.admin, None).await;
    assert_eq!(ledger_count(&after), ledger_count(&before) + 1);
    let last = after["entries"].as_array().unwrap().last().unwrap();
    assert_eq!((last["decision_type"].as_str(), last["principal"].as_str()), (Some("DisclosureLogExported"), Some(COMPLIANCE)));

    let (s, _) = send(&w.app, "GET", &format!("/api/v1/admin/disclosures/export?app={}", w.slug), &w.faculty, None).await;
    assert_eq!(s, StatusCode::FORBIDDEN);

    // 50,001 entries: the export stops at 50,000 and says it stopped.
    let slug = w.slug.clone();
    db(move |c| {
        c.execute(
            "INSERT INTO disclosure_log (id, actor, kind, app_slug, table_id, record_count, fields, note) \
             SELECT gen_random_uuid(), 'bulk@state.edu', 'export', $1, 'students', 1, ARRAY['student_id'], 'bulk' FROM generate_series(1, 50001)",
            &[&slug],
        )
        .unwrap();
    });
    let (s, body) = send(&w.app, "GET", &format!("/api/v1/admin/disclosures/export?app={}&format=json&actor=bulk@state.edu", w.slug), &w.compliance, None).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(body["entries"].as_array().unwrap().len(), EXPORT_ROW_LIMIT);
    assert_eq!(body["truncated"], true);
}

// 6. Every call site that hands record data to a caller who may be an agent goes through the log, or is
//    named as exempt with its reason. A new reader that is in neither list fails here.
#[test]
fn every_function_that_releases_records_logs_the_release_or_says_why_not() {
    assert!(AGENT_RELEASE_EXEMPT.iter().all(|(_, reason)| reason.len() > 20), "an exemption carries a reason");
    let loaders = ["records::list_page(", "records::list_records(", "records::get_record(", "trash::list_trash(", "link_options::link_options("];
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let (mut unlisted, mut seen, mut missing_call) = (Vec::new(), Vec::new(), Vec::new());
    for dir in ["routes", "service"] {
        for entry in std::fs::read_dir(root.join(dir)).unwrap() {
            let path = entry.unwrap().path();
            let file = path.file_name().unwrap().to_string_lossy().to_string();
            if path.extension().and_then(|e| e.to_str()) != Some("rs") || (dir == "service" && ["records.rs", "disclosure.rs"].contains(&file.as_str())) {
                continue;
            }
            // Split the file into top-level functions.
            let text = std::fs::read_to_string(&path).unwrap();
            let mut functions: Vec<(String, String)> = Vec::new();
            for line in text.lines() {
                let t = line.trim_start();
                let top = !line.starts_with(' ');
                if top && (t.starts_with("fn ") || t.starts_with("pub fn ") || t.starts_with("async fn ") || t.starts_with("pub async fn ") || t.starts_with("pub(crate) fn ")) {
                    functions.push((t.split("fn ").nth(1).unwrap().split(['(', '<']).next().unwrap().to_string(), String::new()));
                }
                if let Some(last) = functions.last_mut() {
                    if !t.starts_with("//") {
                        last.1.push_str(line);
                        last.1.push('\n');
                    }
                }
            }
            for (name, body) in functions {
                if !loaders.iter().any(|l| body.contains(l)) {
                    continue;
                }
                let site = format!("{dir}/{file}::{name}");
                seen.push(site.clone());
                if AGENT_RELEASES.contains(&site.as_str()) {
                    if !body.contains("record_agent_read(") {
                        missing_call.push(site);
                    }
                } else if !AGENT_RELEASE_EXEMPT.iter().any(|(s, _)| *s == site) {
                    unlisted.push(site);
                }
            }
        }
    }
    assert!(unlisted.is_empty(), "these release records and are in neither AGENT_RELEASES nor AGENT_RELEASE_EXEMPT: {unlisted:?}");
    assert!(missing_call.is_empty(), "these are listed in AGENT_RELEASES but never call record_agent_read: {missing_call:?}");
    let stale: Vec<_> = AGENT_RELEASES.iter().chain(AGENT_RELEASE_EXEMPT.iter().map(|(s, _)| s)).filter(|s| !seen.contains(&s.to_string())).collect();
    assert!(stale.is_empty(), "these are listed but release nothing any more: {stale:?}");
}
