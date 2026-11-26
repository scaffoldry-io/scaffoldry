//! Data layer phase 2c: the row's unit column.
//!
//! A table names one of its Unit fields as the unit column. A write, an import row or an API call whose value is not a
//! unit on the governed list is refused, and the error names the nearest units the caller may see. An import row that
//! leaves the column blank takes the dataset's unit, and the dry run says so. The value is copied to an indexed column on
//! the row, and a list filtered by unit reads that column in SQL.

mod common;

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use axum::Router;
use http_body_util::BodyExt;
use scaffoldry_server::build_app_with_state;
use scaffoldry_server::jobs::{get_job, JobConfig, JobRunner, JOB_KINDS};
use scaffoldry_server::repository::{build_record_query, RecordQuery};
use scaffoldry_server::service::identity::issue_test_token_and_user;
use scaffoldry_server::state::{CollaboratorRecord, OrganizationNode, RoleRow, ServerState, WorkspaceRecord};
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};
use tower::ServiceExt;
use uuid::Uuid;

const WS: &str = "ws-units";
const ROOT: &str = "00000000-0000-0000-0000-000000000001";
const ADMIN: &str = "jordan.lee@state.edu";
const EDITOR: &str = "nora.editor@science.state.edu";
const LEAD: &str = "pat.lead@physics.state.edu";
const SECOND: &str = "second.admin@state.edu";

fn files_dir() {
    static DIR: OnceLock<PathBuf> = OnceLock::new();
    DIR.get_or_init(|| {
        let dir = std::env::temp_dir().join(format!("scaffoldry-units-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        std::env::set_var("SCAFFOLDRY_FILES_DIR", &dir);
        dir
    });
}

struct World {
    db: common::TestDb,
    state: Arc<ServerState>,
    app: Router,
    admin: String,
    second: String,
    editor: String,
    lead: String,
    science: Uuid,
    physics: Uuid,
    lab: Uuid,
    chemistry: Uuid,
    slug: String,
}

async fn rest(app: &Router, method: &str, uri: &str, token: &str, body: Option<Value>) -> (StatusCode, Value) {
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

async fn tool_as(w: &World, token: &str, name: &str, args: Value) -> Result<Value, String> {
    let payload = json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": { "name": name, "arguments": args } });
    let (_, v) = rest(&w.app, "POST", "/api/mcp", token, Some(payload)).await;
    if let Some(err) = v.get("error") {
        return Err(err["message"].as_str().unwrap_or("").to_string());
    }
    if v["result"]["isError"] == true {
        return Err(v["result"]["content"][0]["text"].as_str().unwrap_or("").to_string());
    }
    Ok(v["result"]["structuredContent"].clone())
}

fn unit(state: &ServerState, name: &str, parent: Uuid) -> Uuid {
    let id = Uuid::new_v4();
    let node = OrganizationNode { id, parent_id: Some(parent), name: name.to_string(), code: name.to_uppercase(), org_type: "Unit".to_string(), hierarchy_key: "academic".to_string(), synthetic: false };
    state.organizations.write().unwrap().insert(id, node.clone());
    state.persist_organization(&node).unwrap();
    id
}

fn appoint(state: &ServerState, eppn: &str, unit: Uuid, scoped_affiliation: &str) {
    state
        .persist_role(&RoleRow {
            id: Uuid::new_v4(),
            person_id: Uuid::new_v4(),
            eppn: eppn.to_string(),
            organization_id: unit,
            role_title: scoped_affiliation.to_string(),
            scoped_affiliation: scoped_affiliation.to_string(),
            is_primary: false,
            source: "test".to_string(),
            position_key: None,
        })
        .unwrap();
}

fn editor_of(state: &ServerState, eppn: &str) {
    let record = CollaboratorRecord {
        id: Uuid::new_v4().to_string(),
        workspace_id: WS.to_string(),
        eppn: eppn.to_string(),
        name: eppn.to_string(),
        role: "editor".to_string(),
        scoped_affiliation: "faculty".to_string(),
        department: "science".to_string(),
        added_at: String::new(),
        app_slug: String::new(),
    };
    state.repository.upsert_collaborator(&record).expect("a member");
    state.collaborators.write().unwrap().entry(WS.to_string()).or_default().push(record);
}

fn workspace(state: &ServerState, unit: Uuid) {
    let ws = WorkspaceRecord {
        id: WS.to_string(),
        name: "Units".to_string(),
        code: "WS-UNITS".to_string(),
        organization: "INST".to_string(),
        department: "science".to_string(),
        description: "Test".to_string(),
        icon: "dna".to_string(),
        lead: "Lead".to_string(),
        visibility: "departmental".to_string(),
        coordinates: Default::default(),
        data_classification: "restricted".to_string(),
        created_at: chrono::Utc::now().to_rfc3339(),
        organization_id: Some(unit),
        synthetic: false,
    };
    state.workspaces.write().unwrap().insert(WS.to_string(), ws.clone());
    state.repository.upsert_workspace(&ws).unwrap();
    state.collaborators.write().unwrap().insert(WS.to_string(), Vec::new());
}

/// science > physics > lab, and science > chemistry. The workspace sits at science. The administrator sees every unit, the
/// lead of physics sees physics and the lab, and the editor sees none.
async fn world() -> World {
    files_dir();
    let db = common::TestDb::new();
    let state = db.state();
    let root = Uuid::parse_str(ROOT).unwrap();
    let science = unit(&state, "science", root);
    let physics = unit(&state, "physics", science);
    let lab = unit(&state, "lab", physics);
    let chemistry = unit(&state, "chemistry", science);
    workspace(&state, science);
    editor_of(&state, EDITOR);
    editor_of(&state, LEAD);
    appoint(&state, LEAD, physics, "unit_admin");
    appoint(&state, SECOND, root, "platform_admin");
    let app = build_app_with_state(state.clone()).expect("router");
    let slug = format!("staff-{}", &Uuid::new_v4().to_string()[..8]);
    World {
        db,
        state,
        app,
        admin: issue_test_token_and_user(ADMIN),
        second: issue_test_token_and_user(SECOND),
        editor: issue_test_token_and_user(EDITOR),
        lead: issue_test_token_and_user(LEAD),
        science,
        physics,
        lab,
        chemistry,
        slug,
    }
}

fn manifest(slug: &str, unit_field: Option<&str>) -> Value {
    let mut table = json!({
        "id": "staff", "name": "Staff", "slug": "staff", "primary_field": "name",
        "fields": [
            { "name": "name", "label": "Name", "field_type": "Text", "required": false, "ferpa_sensitive": false },
            { "name": "score", "label": "Score", "field_type": "Number", "required": false, "ferpa_sensitive": false, "indexed": true },
            { "name": "unit", "label": "Unit", "field_type": "Unit", "hierarchy": "academic", "required": false, "ferpa_sensitive": false }
        ]
    });
    if let Some(f) = unit_field {
        table["unit_field"] = json!(f);
    }
    json!({ "slug": slug, "title": "Staff", "description": "Staff by unit", "organization_code": "INST", "department": "science", "workspace_id": WS, "views": [], "tables": [table] })
}

async fn live_app(w: &World, unit_field: Option<&str>) {
    let (s, b) = rest(&w.app, "POST", &format!("/api/v1/workspaces/{WS}/apps"), &w.admin, Some(manifest(&w.slug, unit_field))).await;
    assert_eq!(s, StatusCode::CREATED, "{b}");
    // The index of a new app is built by a job. The rows below write their own entries.
    let slug = w.slug.clone();
    w.db.run(move |c| c.execute("UPDATE index_status SET state = 'ready' WHERE app_slug = $1", &[&slug]).unwrap());
}

async fn create_as(w: &World, token: &str, name: &str, unit: &str) -> (StatusCode, Value) {
    rest(&w.app, "POST", &format!("/api/v1/apps/{}/records", w.slug), token, Some(json!({ "data": { "_table_id": "staff", "name": name, "unit": unit } }))).await
}

fn stored(w: &World) -> Vec<(String, Option<Uuid>)> {
    let slug = w.slug.clone();
    w.db.run(move |c| c.query("SELECT data->>'name', unit_id FROM dataset_records WHERE app_slug = $1 AND deleted_at IS NULL ORDER BY data->>'name'", &[&slug]).unwrap().iter().map(|r| (r.get(0), r.get(1))).collect())
}

/// The first units a refusal names, in the order it names them.
fn named(message: &str, all: &[(&str, Uuid)]) -> Vec<String> {
    let mut at: Vec<(usize, String)> = all.iter().filter_map(|(n, id)| message.find(&id.to_string()).map(|i| (i, n.to_string()))).collect();
    at.sort();
    at.into_iter().map(|(_, n)| n).collect()
}

fn all_units(w: &World) -> Vec<(&'static str, Uuid)> {
    vec![("science", w.science), ("physics", w.physics), ("lab", w.lab), ("chemistry", w.chemistry)]
}

// 1. A value that is not a unit on the governed list is refused. The refusal names the nearest units the caller may see.
#[tokio::test(flavor = "multi_thread")]
async fn a_value_that_is_not_a_unit_is_refused_and_the_nearest_units_are_named() {
    let w = world().await;
    live_app(&w, Some("unit")).await;
    let units = all_units(&w);

    // An administrator sees every unit. "physic" is nearest to physics, and at most three units are named.
    let (s, b) = create_as(&w, &w.admin, "Ada", "physic").await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{b}");
    let message = b["error"].as_str().unwrap_or_default().to_string();
    assert!(message.contains("'physic' is not a unit of the 'academic' hierarchy"), "{message}");
    let order = named(&message, &units);
    assert_eq!(order.first().map(String::as_str), Some("physics"), "{message}");
    assert!(order.len() <= 3, "more than three units named: {message}");
    assert!(stored(&w).is_empty(), "a refused row was stored");

    // A well-formed id that no unit has is refused in the same way, and the nearest by id is named.
    let near = w.lab.to_string().replace(&w.lab.to_string()[..4], "0000");
    let (s, b) = create_as(&w, &w.admin, "Bo", &near).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{b}");
    assert!(b["error"].as_str().unwrap_or_default().contains(&w.lab.to_string()), "{b}");

    // The same words reach an agent.
    let err = tool_as(&w, &w.admin, "create_record", json!({ "app_slug": w.slug, "data": { "_table_id": "staff", "name": "Cy", "unit": "physic" } })).await.expect_err("refused");
    assert!(err.contains(&w.physics.to_string()), "{err}");

    // An editor with no unit in reach is told nothing about any unit.
    let (s, b) = create_as(&w, &w.editor, "Di", "physic").await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{b}");
    let message = b["error"].as_str().unwrap_or_default().to_lowercase();
    assert!(message.contains("'physic' is not a unit"), "{message}");
    for (name, id) in &units {
        assert!(!message.contains(&id.to_string()) && !message.contains(&format!("{name} ")), "the refusal names the unit {name} to a caller who cannot see it: {message}");
    }
    assert!(!message.contains("institution"), "{message}");

    // The lead of physics sees physics and the lab. Chemistry is nearest to "chemistri" and is not named to them.
    let (s, b) = create_as(&w, &w.lead, "Ed", "chemistri").await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{b}");
    let message = b["error"].as_str().unwrap_or_default().to_string();
    let order = named(&message, &units);
    assert!(!order.is_empty() && order.iter().all(|n| n == "physics" || n == "lab"), "{message}");
    assert!(!message.contains(&w.chemistry.to_string()) && !message.contains(&w.science.to_string()), "{message}");
    assert!(stored(&w).is_empty());
}

// 2. A unit on the list is stored on the row, in the column, and the column follows the value.
#[tokio::test(flavor = "multi_thread")]
async fn the_value_is_copied_to_the_unit_column_and_follows_the_row() {
    let w = world().await;
    live_app(&w, Some("unit")).await;
    let (s, b) = create_as(&w, &w.admin, "Ada", &w.physics.to_string()).await;
    assert_eq!(s, StatusCode::CREATED, "{b}");
    let id = b["id"].as_str().unwrap().to_string();
    // A blank value is no unit. The row is the dataset's own.
    let (s, b2) = create_as(&w, &w.admin, "Bo", "").await;
    assert_eq!(s, StatusCode::CREATED, "{b2}");
    assert_eq!(stored(&w), vec![("Ada".to_string(), Some(w.physics)), ("Bo".to_string(), None)]);

    let patch = |unit: &str, version: i64| json!({ "data": { "_table_id": "staff", "name": "Ada", "unit": unit }, "version": version });
    let uri = format!("/api/v1/apps/{}/records/{id}", w.slug);
    let (s, b) = rest(&w.app, "PATCH", &uri, &w.admin, Some(patch(&w.lab.to_string(), 1))).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_eq!(stored(&w)[0].1, Some(w.lab), "an update did not move the column");
    let (s, b) = rest(&w.app, "PATCH", &uri, &w.admin, Some(patch("nowhere", 2))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{b}");
    assert_eq!(stored(&w)[0].1, Some(w.lab), "a refused update changed the column");
    let (s, b) = rest(&w.app, "PATCH", &uri, &w.admin, Some(patch("", 2))).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_eq!(stored(&w)[0].1, None, "a blank value left the old unit on the row");
}

// 3. The data crate refuses, in the write's own transaction, a unit that is not on the list or is in another hierarchy.
#[tokio::test(flavor = "multi_thread")]
async fn the_data_crate_refuses_a_unit_that_is_not_on_the_list_whatever_door_the_write_came_by() {
    let w = world().await;
    live_app(&w, Some("unit")).await;
    let (slug, physics) = (w.slug.clone(), w.physics);
    let outcome: Vec<String> = w.db.run(move |c| {
        let mut out = Vec::new();
        for (value, id) in [(physics.to_string(), "rec_unit_ok"), ("physic".to_string(), "rec_unit_bad"), (Uuid::new_v4().to_string(), "rec_unit_unknown")] {
            let mut tx = c.transaction().unwrap();
            let data = json!({ "_table_id": "staff", "name": id, "unit": value });
            tx.execute("INSERT INTO dataset_records (id, app_slug, data, ceds_mapping, is_ferpa_sensitive, table_id) VALUES ($1, $2, $3, '{}', false, 'staff')", &[&id, &slug, &data]).unwrap();
            match scaffoldry_data::units::write(&mut tx, &slug, "staff", id, &data) {
                Ok(()) => out.push("ok".to_string()),
                Err(e) => out.push(if scaffoldry_data::units::is_refused(&e) { "refused".to_string() } else { format!("other: {e}") }),
            }
            tx.rollback().unwrap();
        }
        // Another hierarchy's unit is not on this field's list.
        let mut tx = c.transaction().unwrap();
        tx.execute("INSERT INTO hierarchies (key, name, kind, source, created_by) VALUES ('other', 'Other', 'units', 'api', 'test')", &[]).unwrap();
        let foreign = Uuid::new_v4();
        tx.execute("INSERT INTO organizations (id, name, code, org_type, hierarchy_key) VALUES ($1, 'Foreign', 'FOREIGN', 'Unit', 'other')", &[&foreign]).unwrap();
        let data = json!({ "_table_id": "staff", "name": "foreign", "unit": foreign.to_string() });
        tx.execute("INSERT INTO dataset_records (id, app_slug, data, ceds_mapping, is_ferpa_sensitive, table_id) VALUES ('rec_unit_foreign', $1, $2, '{}', false, 'staff')", &[&slug, &data]).unwrap();
        out.push(match scaffoldry_data::units::write(&mut tx, &slug, "staff", "rec_unit_foreign", &data) {
            Ok(()) => "ok".to_string(),
            Err(e) if scaffoldry_data::units::is_refused(&e) => "refused".to_string(),
            Err(e) => format!("other: {e}"),
        });
        tx.rollback().unwrap();
        out
    });
    assert_eq!(outcome, ["ok", "refused", "refused", "refused"]);
}

// 4. An import row with a blank unit takes the dataset's unit, and the dry run says so. A bad row names the nearest units.
fn fast() -> JobConfig {
    JobConfig { workers: 2, poll_ms: 20, backoff_secs: [0, 0, 0], ..JobConfig::default() }
}

fn wait_for_job(w: &World, id: Uuid) -> scaffoldry_server::jobs::Job {
    let deadline = Instant::now() + Duration::from_secs(120);
    loop {
        let job = get_job(&w.state.repository, id).expect("query").expect("job exists");
        if job.state == "done" || job.state == "failed" {
            return job;
        }
        assert!(Instant::now() < deadline, "job stuck in {}", job.state);
        std::thread::sleep(Duration::from_millis(50));
    }
}

async fn upload(w: &World, token: &str, csv: &str) -> String {
    let req = Request::post(format!("/api/v1/apps/{}/imports", w.slug))
        .header(header::AUTHORIZATION, format!("Bearer {token}"))
        .header(header::CONTENT_LENGTH, csv.len().to_string())
        .header("x-file-name", "staff.csv")
        .body(Body::from(csv.to_string()))
        .unwrap();
    let res = w.app.clone().oneshot(req).await.unwrap();
    let status = res.status();
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    assert_eq!(status, StatusCode::CREATED, "{}", String::from_utf8_lossy(&bytes));
    serde_json::from_slice::<Value>(&bytes).unwrap()["import_id"].as_str().unwrap().to_string()
}

fn mapping() -> Value {
    json!({ "target_table": "staff", "column_mappings": { "name": "name", "unit": "unit" } })
}

async fn dry_run(w: &World, token: &str, csv: &str) -> Value {
    let import_id = upload(w, token, csv).await;
    let (s, body) = rest(&w.app, "POST", &format!("/api/v1/apps/{}/imports/{import_id}/dry-run", w.slug), token, Some(mapping())).await;
    assert!(s.is_success(), "{s} {body}");
    let runner = JobRunner::start(&w.state.repository, JOB_KINDS, fast()).unwrap();
    let job = wait_for_job(w, body["job_id"].as_str().unwrap().parse().unwrap());
    runner.stop();
    assert_eq!(job.state, "done", "{:?}", job.error);
    job.result.expect("a dry run result")
}

async fn import(w: &World, token: &str, csv: &str, accept_errors: bool) -> scaffoldry_server::jobs::Job {
    let import_id = upload(w, token, csv).await;
    let uri = format!("/api/v1/apps/{}/imports/{import_id}/run?accept_errors={accept_errors}", w.slug);
    let (s, body) = rest(&w.app, "POST", &uri, token, Some(mapping())).await;
    assert!(s.is_success(), "{s} {body}");
    let runner = JobRunner::start(&w.state.repository, JOB_KINDS, fast()).unwrap();
    let job = wait_for_job(w, body["job_id"].as_str().unwrap().parse().unwrap());
    runner.stop();
    job
}

#[tokio::test(flavor = "multi_thread")]
async fn an_import_that_leaves_the_unit_blank_takes_the_datasets_unit_and_the_preview_says_so() {
    let w = world().await;
    live_app(&w, Some("unit")).await;
    let csv = format!("name,unit\nAda,{}\nBo,\nCy,physic\nDi,\n", w.physics);

    let preview = dry_run(&w, &w.admin, &csv).await;
    assert_eq!((preview["valid_count"].as_i64(), preview["invalid_count"].as_i64()), (Some(3), Some(1)), "{preview}");
    let said = &preview["unit_default"];
    assert_eq!(said["field"], "unit", "{preview}");
    assert_eq!(said["unit_id"], w.science.to_string(), "{preview}");
    assert_eq!(said["blank_rows"], 2, "{preview}");
    let line = said["message"].as_str().unwrap_or_default();
    assert!(line.contains("2 rows") && line.contains("science"), "{line}");
    let bad = preview["errors"][0]["message"].as_str().unwrap_or_default();
    assert!(bad.contains("Row 3") && bad.contains(&w.physics.to_string()), "{bad}");

    // Without accept_errors the import stops on the bad row, in the same words.
    let job = import(&w, &w.admin, &csv, false).await;
    assert_eq!(job.state, "failed");
    assert!(job.error.unwrap_or_default().contains(&w.physics.to_string()));
    assert!(stored(&w).is_empty(), "a failed import kept rows");

    // With it, the bad row is rejected and the blanks take the dataset's unit, in the data and in the column.
    let job = import(&w, &w.admin, &csv, true).await;
    assert_eq!(job.state, "done", "{:?}", job.error);
    let result = job.result.unwrap();
    assert_eq!((result["created_count"].as_i64(), result["rejected_count"].as_i64()), (Some(3), Some(1)), "{result}");
    assert_eq!(stored(&w), vec![("Ada".to_string(), Some(w.physics)), ("Bo".to_string(), Some(w.science)), ("Di".to_string(), Some(w.science))]);
    let slug = w.slug.clone();
    let shown: String = w.db.run(move |c| c.query_one("SELECT data->>'unit' FROM dataset_records WHERE app_slug = $1 AND data->>'name' = 'Bo'", &[&slug]).unwrap().get(0));
    assert_eq!(shown, w.science.to_string(), "the row does not show the unit it was given");

    // An importer who sees no unit is told no unit by name.
    let job = import(&w, &w.editor, "name,unit\nEd,chemistri\n", false).await;
    assert_eq!(job.state, "failed");
    let error = job.error.unwrap_or_default();
    assert!(error.contains("'chemistri' is not a unit"), "{error}");
    for (_, id) in all_units(&w) {
        assert!(!error.contains(&id.to_string()), "the refusal names a unit to a caller who cannot see it: {error}");
    }
}

// 5. A list filtered by unit reads the column in SQL, on a table too large to filter on a field that is not indexed.
async fn filter_page(w: &World, token: &str, filter: Value, limit: i64, cursor: Option<&str>) -> Result<Value, String> {
    let mut args = json!({ "app_slug": w.slug, "table_id": "staff", "filter": filter, "limit": limit });
    if let Some(c) = cursor {
        args["cursor"] = json!(c);
    }
    tool_as(w, token, "list_records", args).await
}

fn clause(field: &str, op: &str, value: &str) -> Value {
    json!({ "conjunction": "And", "clauses": [{ "id": "c", "field_name": field, "operator": op, "value": value }], "groups": [] })
}

#[tokio::test(flavor = "multi_thread")]
async fn a_list_filtered_by_unit_uses_the_column_in_sql() {
    let w = world().await;
    live_app(&w, Some("unit")).await;
    // 12,000 rows, 3,000 in each of four units, written below the API so they have no index entries.
    let (slug, ids) = (w.slug.clone(), [w.science, w.physics, w.lab, w.chemistry]);
    w.db.run(move |c| {
        c.execute(
            "INSERT INTO dataset_records (id, app_slug, data, ceds_mapping, is_ferpa_sensitive, table_id, unit_id, created_at) \
             SELECT 'rec_seed_' || g, $1, jsonb_build_object('_table_id', 'staff', 'name', 'n' || g, 'unit', ($2::uuid[])[1 + g % 4]::text), '{}', false, 'staff', ($2::uuid[])[1 + g % 4], NOW() + g * interval '1 millisecond' \
             FROM generate_series(1, 12000) g",
            &[&slug, &ids.to_vec()],
        )
        .unwrap();
        c.batch_execute("ANALYZE dataset_records").unwrap();
    });

    // A filter on the unit works, and a filter on a field that is not indexed does not: the table is too large.
    let refused = filter_page(&w, &w.admin, clause("name", "Equals", "n5"), 10, None).await.expect_err("too large");
    assert!(refused.contains("too large"), "{refused}");
    let mut seen = 0;
    let mut cursor: Option<String> = None;
    loop {
        let page = filter_page(&w, &w.admin, clause("unit", "Equals", &w.physics.to_string()), 200, cursor.as_deref()).await.expect("a page");
        let rows = page["records"].as_array().unwrap();
        assert!(rows.iter().all(|r| r["data"]["unit"] == w.physics.to_string()), "a row of another unit came back");
        seen += rows.len();
        match page["next_cursor"].as_str() {
            Some(c) => cursor = Some(c.to_string()),
            None => break,
        }
    }
    assert_eq!(seen, 3000);
    // A list of several units is not read from the column: it keeps the rule of any other filter on a field that is not indexed.
    let two = filter_page(&w, &w.admin, clause("unit", "In", &format!("{},{}", w.physics, w.lab)), 200, None).await.expect_err("too large");
    assert!(two.contains("too large"), "{two}");

    // The statement reads the column through its index. It does not read the table in order and filter.
    let q = RecordQuery {
        table_id: "staff".to_string(),
        any: false,
        clauses: Vec::new(),
        sort: None,
        after: None,
        limit: 51,
        scope: None,
        search: None,
        expr: None,
        unit: Some(w.physics.to_string()),
    };
    let (sql, params) = build_record_query(&w.slug, &q);
    assert!(sql.contains("unit_id = $3::text::uuid"), "{sql}");
    let plan = w.state.repository.explain(&sql, &params).unwrap();
    assert!(plan.contains("idx_dataset_records_unit"), "the unit index is not used:\n{plan}");
    assert!(!plan.contains("Seq Scan on dataset_records"), "{plan}");

    // A unit filter with a sort on an indexed field reads the same column, with the other rows kept out.
    for (name, score, unit) in [("Low", 1, w.lab), ("High", 9, w.lab), ("Other", 5, w.chemistry)] {
        let (s, b) = rest(&w.app, "POST", &format!("/api/v1/apps/{}/records", w.slug), &w.admin, Some(json!({ "data": { "_table_id": "staff", "name": name, "score": score, "unit": unit.to_string() } }))).await;
        assert_eq!(s, StatusCode::CREATED, "{b}");
    }
    let both = json!({ "conjunction": "And", "clauses": [
        { "id": "a", "field_name": "unit", "operator": "Equals", "value": w.lab.to_string() },
        { "id": "b", "field_name": "score", "operator": "GreaterThan", "value": "0" } ], "groups": [] });
    let page = filter_page(&w, &w.admin, both, 10, None).await.expect("a page");
    let names: Vec<&str> = page["records"].as_array().unwrap().iter().map(|r| r["data"]["name"].as_str().unwrap()).collect();
    assert_eq!(names, ["Low", "High"], "{page}");
}

// 6. The steward names the unit column by a proposal. Rows already stored are copied to the column. A value that is not
// a unit blocks the approval and the rows are listed.
#[tokio::test(flavor = "multi_thread")]
async fn a_proposal_that_names_the_unit_column_copies_existing_rows_and_is_refused_over_bad_values() {
    let w = world().await;
    live_app(&w, None).await;
    let (s, good) = create_as(&w, &w.admin, "Ada", &w.physics.to_string()).await;
    assert_eq!(s, StatusCode::CREATED, "{good}");
    let (s, bad) = create_as(&w, &w.admin, "Bo", &w.lab.to_string()).await;
    assert_eq!(s, StatusCode::CREATED, "{bad}");
    // A value written before the table named a unit column, which is not a unit.
    let (slug, bad_id) = (w.slug.clone(), bad["id"].as_str().unwrap().to_string());
    let id = bad_id.clone();
    w.db.run(move |c| c.execute("UPDATE dataset_records SET data = jsonb_set(data, '{unit}', '\"Physics Dept\"') WHERE app_slug = $1 AND id = $2", &[&slug, &id]).unwrap());
    assert!(stored(&w).iter().all(|(_, u)| u.is_none()));

    let p = tool_as(&w, &w.admin, "propose_app_change", json!({ "workspace_id": WS, "manifest": manifest(&w.slug, Some("unit")), "summary": "Name the unit column" })).await.expect("proposed");
    let change = serde_json::to_string(&p).unwrap();
    assert!(change.to_lowercase().contains("unit column"), "the change list does not say it: {change}");
    let id = p["proposal_id"].as_str().unwrap().to_string();
    let err = tool_as(&w, &w.second, "decide_proposal", json!({ "proposal_id": id, "decision": "approve", "rationale": "Reviewed", "digest": p["digest"] })).await.expect_err("a row has no unit");
    assert!(err.contains(&bad_id) && !err.contains(good["id"].as_str().unwrap()), "{err}");
    assert!(stored(&w).iter().all(|(_, u)| u.is_none()), "a refused approval left units on rows");
    let status = w.state.repository.get_proposal(id.parse().unwrap()).unwrap().expect("the proposal").status;
    assert_eq!(status, "Pending");

    let (s, _) = rest(&w.app, "DELETE", &format!("/api/v1/apps/{}/records/{bad_id}", w.slug), &w.admin, None).await;
    assert!(s.is_success());
    tool_as(&w, &w.second, "decide_proposal", json!({ "proposal_id": id, "decision": "approve", "rationale": "Reviewed", "digest": p["digest"] })).await.expect("approved");
    assert_eq!(stored(&w), vec![("Ada".to_string(), Some(w.physics))]);
}

// 7. The unit column must be a Unit field of the table.
#[tokio::test(flavor = "multi_thread")]
async fn a_unit_column_that_is_not_a_unit_field_is_not_a_manifest() {
    let w = world().await;
    live_app(&w, None).await;
    for field in ["nope", "name"] {
        let err = tool_as(&w, &w.admin, "propose_app_change", json!({ "workspace_id": WS, "manifest": manifest(&w.slug, Some(field)), "summary": "Bad unit column" })).await.expect_err("refused");
        assert!(err.contains("unit column"), "{err}");
    }
}
