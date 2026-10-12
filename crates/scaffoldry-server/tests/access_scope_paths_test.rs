//! Access rules phase 3: every path that returns record data applies the person's row scope.
//!
//! The app has one rule: a person sees the students they advise. Ann advises 12 of 4,000 rows.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::Router;
use http_body_util::BodyExt;
use scaffoldry_core::access::{Op, Operand, Pred, PersonAttr, RowOp, RowRule};
use scaffoldry_engine::{AppManifest, AppTable, FieldSpec, FieldType};
use scaffoldry_server::build_app_with_state;
use scaffoldry_server::service::identity::issue_test_token_and_user;
use scaffoldry_server::service::scope::{set_rules, RECORD_INTERNALS, RECORD_READERS};
use scaffoldry_server::state::{CollaboratorRecord, ServerState};
use serde_json::{json, Value};
use std::sync::Arc;
use tower::ServiceExt;
use uuid::Uuid;

const WORKSPACE: &str = "ws-bio-lab";
const ADMIN: &str = "jordan.lee@state.edu";
const ROWS: i64 = 4000;

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
                let _ = c.execute("DELETE FROM workspace_collaborators WHERE workspace_id = $1 AND eppn LIKE $2", &[&WORKSPACE, &like]);
                let _ = c.execute("DELETE FROM record_comments WHERE app_slug LIKE $1", &[&like]);
                let _ = c.execute("DELETE FROM record_history WHERE app_slug LIKE $1", &[&like]);
                let _ = c.execute("DELETE FROM dataset_records WHERE app_slug LIKE $1", &[&like]);
            }
        })
        .join();
    }
}

fn manifest(slug: &str) -> AppManifest {
    let mut advisor = FieldSpec::simple("advisor", "advisor", FieldType::Text, false, false);
    advisor.indexed = true;
    AppManifest {
        slug: slug.to_string(),
        title: "Advising".to_string(),
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
            fields: vec![FieldSpec::simple("name", "name", FieldType::Text, false, false), advisor],
            primary_field: Some("name".to_string()),
            sample_records: vec![],
        }],
        relationships: vec![],
        views: vec![],
        ceds_mappings: Default::default(),
        pages: vec![],
        access: Default::default(),
    }
}

fn rule() -> RowRule {
    RowRule {
        id: "advisees".into(),
        table_id: "students".into(),
        ops: vec![RowOp::Read, RowOp::Create, RowOp::Update, RowOp::Delete],
        visible_when: Pred::Cmp { field: "advisor".into(), op: Op::Eq, value: Operand::Person(PersonAttr::Address) },
        except_roles: vec![],
        message: None,
    }
}

struct World {
    app: Router,
    slug: String,
    tag: String,
    ann: String,
    ann_token: String,
    /// A workspace admin the rule also restricts. Only an owner or admin reads the app-wide history.
    olga_token: String,
    admin: String,
    _cleanup: Cleanup,
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
    });
}

/// `ROWS` students. Ann advises every 333rd. Everyone else is advised by one of 300 others.
fn fill(slug: &str, ann: &str, rows: i64) {
    let (slug, ann) = (slug.to_string(), ann.to_string());
    db(move |c| {
        c.execute(
            "INSERT INTO dataset_records (id, app_slug, data, ceds_mapping, is_ferpa_sensitive, table_id, created_at) \
             SELECT $1::text || lpad(g::text, 8, '0'), $1::text, \
                    jsonb_build_object('_table_id', 'students', 'name', 'student-' || g, \
                                       'advisor', CASE WHEN g % 333 = 0 THEN $2::text ELSE 'adv' || (g % 300) || '@state.edu' END), \
                    '{}'::jsonb, false, 'students', TIMESTAMPTZ '2026-01-01 00:00:00+00' + g * interval '1 millisecond' \
             FROM generate_series(1, $3::bigint) g",
            &[&slug, &ann, &rows],
        )
        .unwrap();
        c.execute(
            "INSERT INTO record_index (app_slug, table_id, field, record_id, txt, ord) \
             SELECT $1::text, 'students', 'advisor', $1::text || lpad(g::text, 8, '0'), \
                    CASE WHEN g % 333 = 0 THEN lower($2::text) ELSE 'adv' || (g % 300) || '@state.edu' END, 0 \
             FROM generate_series(1, $3::bigint) g",
            &[&slug, &ann, &rows],
        )
        .unwrap();
        c.batch_execute("ANALYZE dataset_records; ANALYZE record_index").unwrap();
    });
}

async fn world(rows: i64) -> World {
    let state = Arc::new(ServerState::new().expect("state"));
    state.seed_demo().expect("seed");
    let tag = Uuid::new_v4().to_string()[..8].to_string();
    let slug = format!("asp-{tag}");
    let ann = format!("ann-{tag}@state.edu");
    state.engine.write().unwrap().register_manifest(manifest(&slug)).unwrap();
    editor(&state, &ann);
    let olga = format!("olga-{tag}@state.edu");
    editor(&state, &olga);
    state.collaborators.write().unwrap().get_mut(WORKSPACE).unwrap().iter_mut().filter(|c| c.eppn == olga).for_each(|c| c.role = "admin".to_string());
    set_rules(&slug, vec![rule()], &state);
    fill(&slug, &ann, rows);
    World {
        app: build_app_with_state(state).expect("router"),
        ann_token: issue_test_token_and_user(&ann),
        olga_token: issue_test_token_and_user(&olga),
        admin: issue_test_token_and_user(ADMIN),
        ann,
        slug,
        _cleanup: Cleanup(tag.clone()),
        tag,
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

fn id(w: &World, g: i64) -> String {
    format!("{}{:08}", w.slug, g)
}

/// A student someone else advises, and one Ann advises.
fn hidden(w: &World) -> String {
    id(w, 1)
}
fn mine(w: &World) -> String {
    id(w, 333)
}

async fn list_all(w: &World, token: &str, limit: usize) -> Vec<Value> {
    let mut out = Vec::new();
    let mut cursor: Option<String> = None;
    loop {
        let mut uri = format!("/api/v1/apps/{}/tables/students/records?limit={limit}", w.slug);
        if let Some(c) = &cursor {
            uri.push_str(&format!("&cursor={c}"));
        }
        let (s, body) = send(&w.app, "GET", &uri, token, None).await;
        assert_eq!(s, StatusCode::OK, "{body}");
        out.extend(body["records"].as_array().unwrap().iter().cloned());
        match body["next_cursor"].as_str() {
            Some(c) => cursor = Some(c.to_string()),
            None => return out,
        }
    }
}

// 1. A restricted person lists their 12 of 4,000 rows, in pages, and the legacy full list holds the same 12.
#[tokio::test]
async fn a_restricted_person_lists_only_their_rows() {
    let w = world(ROWS).await;
    let rows = list_all(&w, &w.ann_token, 5).await;
    assert_eq!(rows.len(), 12, "12 of {ROWS}");
    assert!(rows.iter().all(|r| r["data"]["advisor"].as_str().unwrap().eq_ignore_ascii_case(&w.ann)));
    let everyone = list_all(&w, &w.admin, 500).await;
    assert_eq!(everyone.len() as i64, ROWS, "a Platform Admin skips the rule");

    let (s, legacy) = send(&w.app, "GET", &format!("/api/v1/apps/{}/records", w.slug), &w.ann_token, None).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(legacy["total"], 12, "{}", legacy["total"]);
    assert_eq!(legacy["records"].as_array().unwrap().len(), 12);
}

// 2. A row outside the scope is not found, and the answer matches a row that does not exist.
#[tokio::test]
async fn a_row_outside_the_scope_is_not_found_like_a_row_that_does_not_exist() {
    let w = world(ROWS).await;
    let (s, _) = send(&w.app, "GET", &format!("/api/v1/apps/{}/records/{}", w.slug, mine(&w)), &w.ann_token, None).await;
    assert_eq!(s, StatusCode::OK);
    for route in ["records", "tables/students/records"] {
        let (hs, hbody) = send(&w.app, "GET", &format!("/api/v1/apps/{}/{route}/{}", w.slug, hidden(&w)), &w.ann_token, None).await;
        let missing = id(&w, 9_999_999);
        let (ms, mbody) = send(&w.app, "GET", &format!("/api/v1/apps/{}/{route}/{missing}", w.slug), &w.ann_token, None).await;
        assert_eq!((hs, ms), (StatusCode::NOT_FOUND, StatusCode::NOT_FOUND), "{route}");
        let same = hbody.to_string().replace(&hidden(&w), "ID") == mbody.to_string().replace(&missing, "ID");
        assert!(same, "{route}: {hbody} vs {mbody}");
    }
}

// 3. Writes are checked against the scope.
#[tokio::test]
async fn writes_are_checked_against_the_scope() {
    let w = world(ROWS).await;
    let base = format!("/api/v1/apps/{}/records", w.slug);
    let new = |advisor: &str| json!({ "data": { "_table_id": "students", "name": "new", "advisor": advisor } });
    let (s, body) = send(&w.app, "POST", &base, &w.ann_token, Some(new("someone.else@state.edu"))).await;
    assert!(s == StatusCode::FORBIDDEN || s == StatusCode::NOT_FOUND, "a record outside her scope: {s} {body}");
    let (s, own) = send(&w.app, "POST", &base, &w.ann_token, Some(new(&w.ann))).await;
    assert_eq!(s, StatusCode::CREATED, "{own}");

    // Her own row cannot be handed to someone else.
    let (s, body) = send(&w.app, "PATCH", &format!("{base}/{}", mine(&w)), &w.ann_token, Some(json!({ "data": { "advisor": "someone.else@state.edu" }, "version": 1 }))).await;
    assert!(s == StatusCode::FORBIDDEN || s == StatusCode::NOT_FOUND, "{s} {body}");
    // A row she cannot see cannot be changed or deleted, and she is told it does not exist.
    let (s, _) = send(&w.app, "PATCH", &format!("{base}/{}", hidden(&w)), &w.ann_token, Some(json!({ "data": { "name": "x" }, "version": 1 }))).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    let (s, _) = send(&w.app, "DELETE", &format!("{base}/{}?version=1", hidden(&w)), &w.ann_token, None).await;
    assert_eq!(s, StatusCode::NOT_FOUND);

    // An admin can do all of it.
    let (s, body) = send(&w.app, "PATCH", &format!("{base}/{}", mine(&w)), &w.admin, Some(json!({ "data": { "advisor": "someone.else@state.edu" }, "version": 1 }))).await;
    assert_eq!(s, StatusCode::OK, "{body}");
}

// 4. History, comments, and the link picker for a hidden row are not found, or leave it out.
#[tokio::test]
async fn history_comments_and_the_link_picker_leave_out_a_hidden_row() {
    let w = world(ROWS).await;
    let base = format!("/api/v1/apps/{}", w.slug);
    // Give both rows a comment and a history entry through the admin.
    for rec in [hidden(&w), mine(&w)] {
        let (s, body) = send(&w.app, "POST", &format!("{base}/records/{rec}/comments"), &w.admin, Some(json!({ "body": "note" }))).await;
        assert_eq!(s, StatusCode::CREATED, "{body}");
        let (s, body) = send(&w.app, "PATCH", &format!("{base}/records/{rec}"), &w.admin, Some(json!({ "data": { "name": format!("renamed-{rec}"), "advisor": if rec == mine(&w) { w.ann.clone() } else { "someone.else@state.edu".to_string() } }, "version": 1 }))).await;
        assert_eq!(s, StatusCode::OK, "{body}");
    }
    for path in [format!("records/{}/history", hidden(&w)), format!("records/{}/comments", hidden(&w))] {
        let (s, _) = send(&w.app, "GET", &format!("{base}/{path}"), &w.ann_token, None).await;
        assert_eq!(s, StatusCode::NOT_FOUND, "{path}");
    }
    for path in [format!("records/{}/history", mine(&w)), format!("records/{}/comments", mine(&w))] {
        let (s, _b) = send(&w.app, "GET", &format!("{base}/{path}"), &w.ann_token, None).await;
        assert_eq!(s, StatusCode::OK, "{path} {_b}");
    }
    // The app-wide feed is for an owner or admin. The rule restricts them too, and they are not advisers,
    // so it holds no row of Ann's. An admin who is not restricted sees both.
    let (s, feed) = send(&w.app, "GET", &format!("{base}/history?limit=200"), &w.admin, None).await;
    assert_eq!(s, StatusCode::OK, "{feed}");
    let text = feed.to_string();
    assert!(text.contains(&hidden(&w)) && text.contains(&mine(&w)), "a Platform Admin sees both");
    let (s, feed) = send(&w.app, "GET", &format!("{base}/history?limit=200"), &w.olga_token, None).await;
    assert_eq!(s, StatusCode::OK, "{feed}");
    let text = feed.to_string();
    assert!(!text.contains(&hidden(&w)) && !text.contains(&mine(&w)), "a restricted admin who advises nobody sees no row: {text}");

    let (s, options) = send(&w.app, "GET", &format!("{base}/tables/students/link-options?limit=50"), &w.ann_token, None).await;
    assert_eq!(s, StatusCode::OK, "{options}");
    assert_eq!(options["options"].as_array().unwrap().len(), 12, "the picker lists her 12");
}

// 5. The trash lists only what the person could see.
#[tokio::test]
async fn the_trash_lists_only_what_the_person_could_see() {
    let w = world(ROWS).await;
    let base = format!("/api/v1/apps/{}", w.slug);
    // Ann is an editor, so she sees only her own deletions. Delete one of her rows as herself.
    let (s, _) = send(&w.app, "DELETE", &format!("{base}/records/{}?version=1", mine(&w)), &w.ann_token, None).await;
    assert!(s.is_success(), "{s}");
    // An admin deletes a row she could never see.
    let (s, _) = send(&w.app, "DELETE", &format!("{base}/records/{}?version=1", hidden(&w)), &w.admin, None).await;
    assert!(s.is_success(), "{s}");
    let (s, trash) = send(&w.app, "GET", &format!("{base}/trash"), &w.ann_token, None).await;
    assert_eq!(s, StatusCode::OK, "{trash}");
    let text = trash.to_string();
    assert!(text.contains(&mine(&w)) && !text.contains(&hidden(&w)), "{text}");
    // Restoring a row outside her scope is not found.
    let (s, _) = send(&w.app, "POST", &format!("{base}/trash/{}/restore", hidden(&w)), &w.ann_token, None).await;
    assert!(s == StatusCode::NOT_FOUND || s == StatusCode::FORBIDDEN, "{s}");
}

// 6. The list of readers is complete: no service function that loads records is missing from it.
#[test]
fn every_service_function_that_loads_records_is_listed() {
    let loaders = [".find_record(", ".list_app_records(", ".list_records_page(", ".query_records(", ".indexed_page(", ".list_history(", ".list_comments(", ".link_option_rows(", ".trash_page("];
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut unlisted = Vec::new();
    for dir in ["service", "routes"] {
        for entry in std::fs::read_dir(root.join(dir)).unwrap() {
            let path = entry.unwrap().path();
            if path.extension().and_then(|e| e.to_str()) != Some("rs") {
                continue;
            }
            let file = path.file_name().unwrap().to_string_lossy().to_string();
            let text = std::fs::read_to_string(&path).unwrap();
            let mut name = String::new();
            for line in text.lines() {
                let trimmed = line.trim_start();
                let at_top = !line.starts_with(' ');
                if at_top && (trimmed.starts_with("pub fn ") || trimmed.starts_with("fn ") || trimmed.starts_with("pub(crate) fn ") || trimmed.starts_with("async fn ") || trimmed.starts_with("pub async fn ")) {
                    name = trimmed.split("fn ").nth(1).unwrap().split(['(', '<']).next().unwrap().to_string();
                }
                if !line.trim_start().starts_with("//") && loaders.iter().any(|l| line.contains(l)) {
                    let listed = RECORD_READERS.contains(&name.as_str()) || RECORD_INTERNALS.contains(&name.as_str());
                    if !listed {
                        unlisted.push(format!("{dir}/{file}::{name}"));
                    }
                }
            }
        }
    }
    unlisted.sort();
    unlisted.dedup();
    assert!(unlisted.is_empty(), "these load records and are in neither RECORD_READERS nor RECORD_INTERNALS: {unlisted:?}");
}

// 7. At 200,000 rows a scoped list is fast and does not scan.
#[tokio::test]
async fn a_scoped_list_at_200000_rows_is_fast_and_does_not_scan() {
    let w = world(200_000).await;
    // Warm the connection, then time the call.
    let (s, _) = send(&w.app, "GET", &format!("/api/v1/apps/{}/tables/students/records?limit=50", w.slug), &w.ann_token, None).await;
    assert_eq!(s, StatusCode::OK);
    let started = std::time::Instant::now();
    let (s, body) = send(&w.app, "GET", &format!("/api/v1/apps/{}/tables/students/records?limit=50", w.slug), &w.ann_token, None).await;
    let took = started.elapsed();
    assert_eq!(s, StatusCode::OK);
    assert_eq!(body["records"].as_array().unwrap().len(), 50, "ann advises 200000/333 = 600 rows");
    assert!(took < std::time::Duration::from_millis(200), "took {took:?}");

    // The statement the page runs.
    use scaffoldry_server::repository::{build_record_query, RecordQuery, ScopePlan, SqlParam};
    let plan = ScopePlan {
        pred: Pred::Cmp { field: "advisor".into(), op: Op::Eq, value: Operand::Literal(json!(w.ann.to_lowercase())) },
        indexed: manifest(&w.slug).tables[0].fields.clone(),
    };
    let (sql, params) = build_record_query(&w.slug, &RecordQuery { table_id: "students".into(), any: false, clauses: vec![], sort: None, after: None, limit: 51, scope: Some(plan), search: None, expr: None });
    let explained = format!("EXPLAIN {sql}");
    let plan_text = db(move |c| {
        let boxed: Vec<Box<dyn postgres::types::ToSql + Sync>> = params
            .into_iter()
            .map(|p| -> Box<dyn postgres::types::ToSql + Sync> {
                match p {
                    SqlParam::Text(t) => Box::new(t),
                    SqlParam::Texts(t) => Box::new(t),
                    SqlParam::Float(f) => Box::new(f),
                    SqlParam::Floats(f) => Box::new(f),
                    SqlParam::Int(i) => Box::new(i),
                }
            })
            .collect();
        let refs: Vec<&(dyn postgres::types::ToSql + Sync)> = boxed.iter().map(|b| b.as_ref()).collect();
        c.query(&explained, &refs).unwrap().iter().map(|r| r.get::<_, String>(0)).collect::<Vec<_>>().join("\n")
    });
    assert!(!plan_text.contains("Seq Scan on dataset_records") && !plan_text.contains("Seq Scan on record_index"), "{plan_text}");
    let _ = (&w.tag, Op::Eq);
}

// 8. A process instance on a record the person cannot read is not listed, and cannot be decided.
#[tokio::test]
async fn the_process_queue_leaves_out_a_record_the_person_cannot_read() {
    use scaffoldry_core::workflow::{ProcessInstance, ProcessStatus};
    let state = Arc::new(ServerState::new().expect("state"));
    state.seed_demo().expect("seed");
    let tag = Uuid::new_v4().to_string()[..8].to_string();
    let slug = format!("asp-{tag}");
    let ann = format!("ann-{tag}@state.edu");
    state.engine.write().unwrap().register_manifest(manifest(&slug)).unwrap();
    editor(&state, &ann);
    set_rules(&slug, vec![rule()], &state);
    fill(&slug, &ann, 700);
    let _cleanup = Cleanup(tag.clone());
    let instance = |record: String| ProcessInstance {
        id: format!("rule:{record}"),
        rule_id: "rule".into(),
        app_slug: slug.clone(),
        record_id: record,
        status: ProcessStatus::Waiting,
        waiting_step_id: Some("step".into()),
        role: None,
        prompt: None,
        log: vec![],
        started_by: None,
        started_at: "2026-01-01T00:00:00Z".into(),
        assigned_to: Some(ann.clone()),
        no_approver: None,
        rule_version: 0,
    };
    let (hidden_id, mine_id) = (format!("{slug}{:08}", 1), format!("{slug}{:08}", 333));
    for r in [&hidden_id, &mine_id] {
        state.process_instances.write().unwrap().insert(format!("rule:{r}"), instance(r.clone()));
    }
    let app = build_app_with_state(state).expect("router");
    let token = issue_test_token_and_user(&ann);
    let (s, listed) = send(&app, "GET", &format!("/api/v1/apps/{slug}/processes"), &token, None).await;
    assert_eq!(s, StatusCode::OK, "{listed}");
    let text = listed.to_string();
    assert!(text.contains(&mine_id) && !text.contains(&hidden_id), "{text}");
    let (s, _) = send(&app, "POST", &format!("/api/v1/apps/{slug}/processes/rule:{hidden_id}/decide"), &token, Some(json!({ "decision": "approve" }))).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
}
