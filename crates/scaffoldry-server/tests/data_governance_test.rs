//! Admin console phase 4, the rest: data governance. A compliance officer raises a field's sensitivity at once with a label, which can only
//! add protection. Labels, the routes that set them, and the classification of published datasets.

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
                let _ = c.execute("DELETE FROM app_manifests WHERE slug LIKE $1", &[&like]);
            }
        })
        .join();
    }
}

fn manifest(slug: &str) -> AppManifest {
    let mut flagged = FieldSpec::simple("ssn", "Tax number", FieldType::Text, false, true);
    flagged.ferpa_sensitive = true;
    let students = vec![
        FieldSpec::simple("name", "Name", FieldType::Text, false, false),
        FieldSpec::simple("program", "Program", FieldType::Text, false, false),
        FieldSpec::simple("gpa", "GPA", FieldType::Number, false, false),
    ];
    let requests = vec![FieldSpec::simple("kind", "Kind", FieldType::Text, false, false), flagged];
    let table = |id: &str, fields: Vec<FieldSpec>, primary: &str| AppTable {
        retired_fields: Vec::new(), key: None, unit_field: None, unit_required: false, audience: Vec::new(),
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
        // The GPA is a FERPA protected element in the standard, and the manifest does not flag it.
        ceds_mappings: [("gpa".to_string(), "000127".to_string())].into_iter().collect(),
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
    slug: String,
    platform: String,
    compliance: String,
    editor: String,
    viewer: String,
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
        platform: issue_test_token_and_user("jordan.lee@state.edu"),
        compliance,
        editor: issue_test_token_and_user(&editor_e),
        viewer: issue_test_token_and_user(&viewer_e),
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

/// Three students. The first two have a program, and the third has none.
fn three_students(w: &World) -> Vec<String> {
    let slug = w.slug.clone();
    db(move |c| {
        let mut ids = Vec::new();
        for (i, data) in [json!({ "_table_id": "students", "name": "Ada", "program": "physics" }), json!({ "_table_id": "students", "name": "Bo", "program": "history" }), json!({ "_table_id": "students", "name": "Cy" })].into_iter().enumerate() {
            let id = format!("{slug}-s{i}");
            c.execute(
                "INSERT INTO dataset_records (id, app_slug, data, ceds_mapping, is_ferpa_sensitive, table_id, created_at) VALUES ($1, $2, $3, '{}'::jsonb, false, 'students', NOW())",
                &[&id, &slug, &data],
            )
            .unwrap();
            ids.push(id);
        }
        ids
    })
}

fn flags(ids: &[String]) -> Vec<bool> {
    let ids = ids.to_vec();
    db(move |c| ids.iter().map(|id| c.query_one("SELECT is_ferpa_sensitive FROM dataset_records WHERE id = $1", &[id]).unwrap().get(0)).collect())
}

fn label(w: &World, table: &str, field: &str, value: Value) -> Value {
    json!({ "app_slug": w.slug, "table_id": table, "field": field, "ferpa_sensitive": value })
}

async fn put_labels(w: &World, who: &str, changes: Vec<Value>) -> (StatusCode, Value) {
    send(&w.app, "PUT", "/api/v1/admin/labels", who, Some(json!({ "changes": changes, "reason": "The program is now protected." }))).await
}

async fn fields(w: &World, who: &str, query: &str) -> Vec<Value> {
    let (s, v) = send(&w.app, "GET", &format!("/api/v1/admin/fields?app={}&{query}", w.slug), who, None).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    v["fields"].as_array().unwrap().clone()
}

fn field_row<'a>(rows: &'a [Value], table: &str, field: &str) -> &'a Value {
    rows.iter().find(|r| r["table"] == table && r["field"] == field).unwrap_or_else(|| panic!("{table}.{field} in {rows:?}"))
}

// 1. Raise a label on a field of a table with three records, two of which hold that field. Those two are flagged, and the third is
// not. Remove the label. The two return to false.
#[tokio::test]
async fn a_label_flags_the_records_that_hold_the_field_and_removing_it_clears_them() {
    let _db = common::TestDb::new();
    let w = world().await;
    let ids = three_students(&w);
    assert_eq!(flags(&ids), vec![false, false, false]);

    let (s, v) = put_labels(&w, &w.compliance, vec![label(&w, "students", "program", json!(true))]).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(flags(&ids), vec![true, true, false], "only the two that hold the field");
    let rows = fields(&w, &w.platform, "").await;
    let row = field_row(&rows, "students", "program");
    assert_eq!((row["manifest_flag"].clone(), row["overlay_flag"].clone(), row["effective"].clone()), (json!(false), json!(true), json!(true)), "{row}");

    let (s, v) = put_labels(&w, &w.compliance, vec![label(&w, "students", "program", Value::Null)]).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(flags(&ids), vec![false, false, false], "the two return to false");
    let n: i64 = db(|c| c.query_one("SELECT count(*) FROM data_labels", &[]).unwrap().get(0));
    assert_eq!(n, 0);
    let ledger: i64 = db(|c| c.query_one("SELECT count(*) FROM governance_ledger WHERE decision_type = 'DataLabelChanged'", &[]).unwrap().get(0));
    assert_eq!(ledger, 2, "one entry for each change");
}

// 2. A label of `false` on a field that the manifest flags leaves the effective flag true.
#[tokio::test]
async fn a_label_of_false_never_lowers_a_manifest_flag() {
    let _db = common::TestDb::new();
    let w = world().await;
    let (s, v) = put_labels(&w, &w.compliance, vec![label(&w, "requests", "ssn", json!(false))]).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let rows = fields(&w, &w.compliance, "").await;
    let row = field_row(&rows, "requests", "ssn");
    assert_eq!((row["manifest_flag"].clone(), row["overlay_flag"].clone(), row["effective"].clone()), (json!(true), json!(false), json!(true)), "{row}");
}

// 3. A viewer's list response no longer holds a field after a label raises it. A new record with that field is flagged. Search on
// that table is refused for a viewer. These are written against what the earlier briefs built: the viewer filter and the viewer search
// refusal exist, and the record flag is set on write.
#[tokio::test]
async fn a_raised_field_is_hidden_from_a_viewer_flagged_on_write_and_closes_search() {
    let _db = common::TestDb::new();
    let w = world().await;
    three_students(&w);
    let list = format!("/api/v1/apps/{}/tables/students/records", w.slug);
    let (s, before) = send(&w.app, "GET", &list, &w.viewer, None).await;
    assert_eq!(s, StatusCode::OK, "{before}");
    assert!(before.to_string().contains("physics"), "a viewer reads the program at first: {before}");
    let (s, v) = send(&w.app, "GET", &format!("{list}?search=Ada"), &w.viewer, None).await;
    assert_eq!(s, StatusCode::OK, "search is open while nothing in the table is sensitive: {v}");

    put_labels(&w, &w.compliance, vec![label(&w, "students", "program", json!(true))]).await;
    let (s, after) = send(&w.app, "GET", &list, &w.viewer, None).await;
    assert_eq!(s, StatusCode::OK, "{after}");
    assert!(!after.to_string().contains("physics"), "the field is gone from a viewer's list: {after}");
    let (s, v) = send(&w.app, "GET", &format!("{list}?search=Ada"), &w.viewer, None).await;
    assert_eq!((s, v["code"].as_str()), (StatusCode::FORBIDDEN, Some("field_hidden")), "{v}");
    let (s, v) = send(&w.app, "GET", &format!("{list}?search=Ada"), &w.editor, None).await;
    assert_eq!(s, StatusCode::OK, "an editor still searches: {v}");

    let (s, made) = send(&w.app, "POST", &format!("/api/v1/apps/{}/records", w.slug), &w.editor, Some(json!({ "data": { "_table_id": "students", "name": "Di", "program": "art" } }))).await;
    assert_eq!(s, StatusCode::CREATED, "{made}");
    let id = made["id"].as_str().or_else(|| made["record"]["id"].as_str()).unwrap().to_string();
    assert_eq!(flags(&[id]), vec![true], "a new record with the field is flagged");
}

// 4. A batch with one unknown field is 400 and changes nothing.
#[tokio::test]
async fn a_batch_with_an_unknown_field_is_refused_whole() {
    let _db = common::TestDb::new();
    let w = world().await;
    let ids = three_students(&w);
    let (s, v) = put_labels(&w, &w.compliance, vec![label(&w, "students", "program", json!(true)), label(&w, "students", "no_such_field", json!(true))]).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{v}");
    assert!(v.to_string().contains("no_such_field"), "{v}");
    assert_eq!(flags(&ids), vec![false, false, false], "nothing changed");
    let n: i64 = db(|c| c.query_one("SELECT count(*) FROM data_labels", &[]).unwrap().get(0));
    assert_eq!(n, 0);
    // A reason is needed, and a batch holds up to 200 changes.
    let (s, _) = send(&w.app, "PUT", "/api/v1/admin/labels", &w.compliance, Some(json!({ "changes": [label(&w, "students", "program", json!(true))] }))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "a reason is required");
    let many: Vec<Value> = (0..201).map(|_| label(&w, "students", "program", json!(true))).collect();
    assert_eq!(put_labels(&w, &w.compliance, many).await.0, StatusCode::BAD_REQUEST, "201 changes");
}

// The field list: filters, the suggestion from the standard, and where a field is used.
#[tokio::test]
async fn the_field_list_filters_suggests_from_the_standard_and_counts_where_a_field_is_used() {
    let _db = common::TestDb::new();
    let w = world().await;
    // A view that lists and sorts the program, and a formula field that names it.
    let (s, v) = send(&w.app, "POST", &format!("/api/v1/apps/{}/views", w.slug), &w.platform, Some(json!({ "name": "By program", "kind": "grid", "ownership": "collaborative", "table_id": "students", "config": { "column_order": ["name", "program"] } }))).await;
    assert_eq!(s, StatusCode::CREATED, "{v}");

    let all = fields(&w, &w.platform, "").await;
    assert_eq!(all.len(), 5, "three student fields and two request fields");
    let gpa = field_row(&all, "students", "gpa");
    assert_eq!((gpa["suggested"].clone(), gpa["ceds_code"].as_str()), (json!(true), Some("000127")), "the standard marks a GPA as protected: {gpa}");
    assert_eq!(field_row(&all, "students", "program")["suggested"], false, "no name based guess");
    assert!(field_row(&all, "students", "program")["used_in"].as_i64().unwrap() >= 1, "the view uses it: {all:?}");

    let sensitive = fields(&w, &w.platform, "sensitive=true").await;
    assert_eq!(sensitive.len(), 1, "{sensitive:?}");
    assert_eq!(sensitive[0]["field"], "ssn");
    assert_eq!(fields(&w, &w.platform, "sensitive=false").await.len(), 4);
    let suggested = fields(&w, &w.platform, "suggested=true").await;
    assert_eq!(suggested.len(), 1);
    assert_eq!(fields(&w, &w.platform, "search=tax").await.len(), 1, "a search matches the label");
    // Marking the suggestion takes it off the suggested list.
    put_labels(&w, &w.compliance, vec![label(&w, "students", "gpa", json!(true))]).await;
    assert!(fields(&w, &w.platform, "suggested=true").await.is_empty());
}

// Published datasets and their classification.
#[tokio::test]
async fn a_dataset_is_classified_from_a_closed_list_with_a_reason_and_a_ledger_entry() {
    let _db = common::TestDb::new();
    let w = world().await;
    let (s, list) = send(&w.app, "GET", "/api/v1/admin/datasets", &w.compliance, None).await;
    assert_eq!(s, StatusCode::OK, "{list}");
    let first = list["datasets"].as_array().unwrap().iter().find(|d| d["sensitivity_level"] == "Directory").unwrap().clone();
    let id = first["id"].as_str().unwrap().to_string();
    assert!(first["record_count"].is_number() && first["department"].is_string(), "{first}");
    let uri = format!("/api/v1/admin/datasets/{id}/classification");
    let (s, v) = send(&w.app, "PUT", &uri, &w.compliance, Some(json!({ "sensitivity_level": "Secret", "reason": "x" }))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "closed list: {v}");
    let (s, v) = send(&w.app, "PUT", &uri, &w.compliance, Some(json!({ "sensitivity_level": "Internal" }))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "a reason is required: {v}");
    let (s, v) = send(&w.app, "PUT", &uri, &w.compliance, Some(json!({ "sensitivity_level": "Restricted / FERPA", "reason": "It holds grades." }))).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let (col, payload): (String, Value) = db({
        let id = id.clone();
        move |c| {
            let r = c.query_one("SELECT sensitivity_level, payload FROM published_datasets WHERE id = $1", &[&id]).unwrap();
            (r.get(0), r.get(1))
        }
    });
    assert_eq!(col, "Restricted / FERPA");
    assert_eq!(payload["sensitivity_level"], "Restricted / FERPA", "the column and the stored payload change together");
    let ledger: i64 = db(|c| c.query_one("SELECT count(*) FROM governance_ledger WHERE decision_type = 'DataLabelChanged'", &[]).unwrap().get(0));
    assert_eq!(ledger, 1);
    let (s, _) = send(&w.app, "PUT", "/api/v1/admin/datasets/no-such/classification", &w.compliance, Some(json!({ "sensitivity_level": "Internal", "reason": "x" }))).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
}

// Who may: a Platform Admin and a compliance officer, and no one else.
#[tokio::test]
async fn only_a_platform_admin_or_a_compliance_officer_reaches_these_routes() {
    let _db = common::TestDb::new();
    let w = world().await;
    for who in [&w.editor, &w.viewer] {
        for (method, uri, body) in [
            ("GET", "/api/v1/admin/fields".to_string(), None),
            ("PUT", "/api/v1/admin/labels".to_string(), Some(json!({ "changes": [], "reason": "x" }))),
            ("GET", "/api/v1/admin/datasets".to_string(), None),
            ("PUT", "/api/v1/admin/datasets/x/classification".to_string(), Some(json!({ "sensitivity_level": "Internal", "reason": "x" }))),
        ] {
            let (s, v) = send(&w.app, method, &uri, who, body).await;
            assert_eq!(s, StatusCode::FORBIDDEN, "{method} {uri}: {v}");
        }
    }
    assert_eq!(send(&w.app, "GET", "/api/v1/admin/fields", &w.platform, None).await.0, StatusCode::OK);
    assert_eq!(send(&w.app, "GET", "/api/v1/admin/fields", &w.compliance, None).await.0, StatusCode::OK);
}
