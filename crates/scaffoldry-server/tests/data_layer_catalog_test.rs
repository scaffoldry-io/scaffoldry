//! Data layer phase 2d: the catalog.
//!
//! One entry for each dataset, found by words, function, unit, category or field name. Who may discover an entry is a
//! policy set for each dataset. A hidden dataset is in no list, no count, no facet and no error for a caller who may not
//! discover it. A sample is made from the schema and holds no stored value. A small group's count is masked.

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::Router;
use http_body_util::BodyExt;
use scaffoldry_server::build_app_with_state;
use scaffoldry_server::service::identity::issue_test_token_and_user;
use scaffoldry_server::state::{OrganizationNode, PositionType, RoleRow, SharedState};
use serde_json::{json, Value};
use tower::ServiceExt;
use uuid::Uuid;

const ROOT: &str = "00000000-0000-0000-0000-000000000001";
const ADMIN: &str = "jordan.lee@state.edu";
const EDITOR: &str = "nora.editor@science.state.edu";
const MEMBER: &str = "pat.lead@physics.state.edu";
const CHAIR: &str = "chair.person@science.state.edu";
const PLANTED: &str = "PLANTED-9f3a-value";
const IN_MANIFEST: &str = "MANIFEST-SAMPLE-77";

struct World {
    db: common::TestDb,
    app: Router,
    admin: String,
    editor: String,
    member: String,
    chair: String,
    chemistry: Uuid,
    enroll: String,
    payroll: String,
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

async fn tool(w: &World, token: &str, name: &str, args: Value) -> Value {
    let payload = json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": { "name": name, "arguments": args } });
    let (_, v) = rest(&w.app, "POST", "/api/mcp", token, Some(payload)).await;
    assert!(v["result"]["isError"] != true, "{name}: {v}");
    v["result"]["structuredContent"].clone()
}

fn unit(state: &SharedState, name: &str, code: &str, parent: Uuid) -> Uuid {
    let id = Uuid::new_v4();
    let node = OrganizationNode { id, parent_id: Some(parent), name: name.to_string(), code: code.to_string(), org_type: "Department".to_string(), hierarchy_key: "academic".to_string(), synthetic: false };
    state.organizations.write().unwrap().insert(id, node.clone());
    state.persist_organization(&node).unwrap();
    id
}

fn appoint(state: &SharedState, eppn: &str, unit: Uuid, position: Option<&str>) {
    state
        .persist_role(&RoleRow {
            id: Uuid::new_v4(),
            person_id: Uuid::new_v4(),
            eppn: eppn.to_string(),
            organization_id: unit,
            role_title: position.unwrap_or("faculty").to_string(),
            scoped_affiliation: "faculty".to_string(),
            is_primary: false,
            source: "test".to_string(),
            position_key: position.map(str::to_string),
        })
        .unwrap();
}

fn manifest(slug: &str, title: &str, code: &str, department: &str, table: Value) -> Value {
    json!({
        "slug": slug, "title": title, "description": format!("{title} app"), "organization_code": code, "department": department,
        "workspace_id": "ws-bio-lab", "views": [], "tables": [table]
    })
}

async fn world() -> World {
    let db = common::TestDb::new();
    let state = db.state();
    state.seed_demo().expect("seed");
    let root = Uuid::parse_str(ROOT).unwrap();
    let chemistry = unit(&state, "Chemistry Department", "CHEM", root);
    let secret = unit(&state, "Hidden Office", "HIDOFF", root);
    let _ = secret;
    let physics = unit(&state, "Physics Department", "PHYS", root);
    appoint(&state, MEMBER, physics, None);
    state
        .persist_position_type(&PositionType {
            key: "chair".to_string(),
            name: "Department chair".to_string(),
            description: String::new(),
            org_types: vec!["Department".to_string()],
            max_holders: 1,
            retired_at: None,
            created_by: "test".to_string(),
            created_at: chrono::Utc::now().to_rfc3339(),
        })
        .unwrap();
    appoint(&state, CHAIR, chemistry, Some("chair"));
    let app = build_app_with_state(state.clone()).expect("router");
    let admin = issue_test_token_and_user(ADMIN);
    let tag = &Uuid::new_v4().to_string()[..8];
    let enroll = format!("enroll-{tag}");
    let payroll = format!("payroll-{tag}");
    let enrollments = json!({
        "id": "enrollments", "name": "Course Enrollment", "slug": "enrollments", "description": "Who enrolled in which course",
        "primary_field": "student_name",
        "fields": [
            { "name": "student_name", "label": "Student name", "field_type": "Text", "required": true, "ferpa_sensitive": false },
            { "name": "final_grade", "label": "Final grade", "field_type": "Text", "required": false, "ferpa_sensitive": true },
            { "name": "credits", "label": "Credit hours", "field_type": "Number", "required": false, "ferpa_sensitive": false },
            { "name": "enrolled_on", "label": "Enrolled on", "field_type": "Date", "required": false, "ferpa_sensitive": false }
        ]
    });
    let waitlist = json!({
        "id": "waitlist", "name": "Waiting List", "slug": "waitlist", "primary_field": "student_name",
        "fields": [{ "name": "student_name", "label": "Student name", "field_type": "Text", "required": true, "ferpa_sensitive": false }],
        "sample_records": [{ "student_name": IN_MANIFEST }]
    });
    let salaries = json!({
        "id": "salaries", "name": "Pay Bands", "slug": "salaries", "primary_field": "title",
        "fields": [{ "name": "title", "label": "Job title", "field_type": "Text", "required": true, "ferpa_sensitive": false },
                   { "name": "band_code", "label": "Band code", "field_type": "Text", "required": false, "ferpa_sensitive": false }]
    });
    let mut m = manifest(&enroll, "Enrollment", "CHEM", "registrar", enrollments);
    m["tables"].as_array_mut().unwrap().push(waitlist);
    for (m, name) in [(m, "enroll"), (manifest(&payroll, "Payroll", "HIDOFF", "finance", salaries), "payroll")] {
        let (s, b) = rest(&app, "POST", "/api/v1/workspaces/ws-bio-lab/apps", &admin, Some(m)).await;
        assert_eq!(s, StatusCode::CREATED, "{name}: {b}");
    }
    let w = World { db, app, admin, editor: issue_test_token_and_user(EDITOR), member: issue_test_token_and_user(MEMBER), chair: issue_test_token_and_user(CHAIR), chemistry, enroll, payroll };
    // 12 enrollment rows, one holding the planted value. 3 waiting-list rows.
    for i in 0..12 {
        let name = if i == 0 { PLANTED.to_string() } else { format!("Student {i}") };
        let (s, b) = rest(&w.app, "POST", &format!("/api/v1/apps/{}/tables/enrollments/records", w.enroll), &w.admin, Some(json!({ "student_name": name, "final_grade": "B+", "credits": 3 }))).await;
        assert_eq!(s, StatusCode::CREATED, "{b}");
    }
    for i in 0..3 {
        let (s, b) = rest(&w.app, "POST", &format!("/api/v1/apps/{}/tables/waitlist/records", w.enroll), &w.admin, Some(json!({ "student_name": format!("Waiting {i}") }))).await;
        assert_eq!(s, StatusCode::CREATED, "{b}");
    }
    let (s, b) = rest(&w.app, "POST", &format!("/api/v1/apps/{}/tables/salaries/records", w.payroll), &w.admin, Some(json!({ "title": "Dean", "band_code": "SECRET-BAND-1" }))).await;
    assert_eq!(s, StatusCode::CREATED, "{b}");
    w
}

/// The catalog id of an app table, as the administrator sees it.
async fn dataset_id(w: &World, app: &str, table: &str) -> String {
    let (s, body) = rest(&w.app, "GET", "/api/v1/data/datasets", &w.admin, None).await;
    assert_eq!(s, StatusCode::OK, "{body}");
    body["datasets"].as_array().unwrap().iter().find(|d| d["app_slug"] == json!(app) && d["table_id"] == json!(table)).unwrap_or_else(|| panic!("no dataset {app}/{table}"))["id"].as_str().unwrap().to_string()
}

async fn search(w: &World, token: &str, query: &str) -> Value {
    let uri = if query.is_empty() { "/api/v1/datasets".to_string() } else { format!("/api/v1/datasets?query={}", query.replace(' ', "%20")) };
    let (s, body) = rest(&w.app, "GET", &uri, token, None).await;
    assert_eq!(s, StatusCode::OK, "{body}");
    body
}

fn names(body: &Value) -> Vec<String> {
    body["datasets"].as_array().unwrap().iter().map(|d| d["name"].as_str().unwrap().to_string()).collect()
}

async fn set_policy(w: &World, id: &str, body: Value) -> (StatusCode, Value) {
    rest(&w.app, "PUT", &format!("/api/v1/admin/datasets/{id}/discoverability"), &w.admin, Some(body)).await
}

#[tokio::test]
async fn a_search_finds_a_dataset_by_words_function_unit_category_and_field_name() {
    let w = world().await;
    // Words in the description.
    assert!(names(&search(&w, &w.editor, "enrolled course").await).contains(&"Course Enrollment".to_string()));
    // The function that owns it.
    assert!(names(&search(&w, &w.editor, "registrar").await).contains(&"Course Enrollment".to_string()));
    // The unit, by name and by code.
    assert!(names(&search(&w, &w.editor, "chemistry").await).contains(&"Course Enrollment".to_string()));
    assert!(names(&search(&w, &w.editor, "chem").await).contains(&"Course Enrollment".to_string()));
    // A category a field carries.
    let by_category = names(&search(&w, &w.editor, "ferpa").await);
    assert!(by_category.contains(&"Course Enrollment".to_string()), "{by_category:?}");
    assert!(!by_category.contains(&"Waiting List".to_string()), "{by_category:?}");
    // A field, by name and by label.
    assert!(names(&search(&w, &w.editor, "final_grade").await).contains(&"Course Enrollment".to_string()));
    assert!(names(&search(&w, &w.editor, "credit hours").await).contains(&"Course Enrollment".to_string()));
    // Every word must match. A word nothing holds finds nothing, and a wildcard character is only itself.
    assert!(names(&search(&w, &w.editor, "enrolled zebra").await).is_empty());
    assert!(names(&search(&w, &w.editor, "%25").await).is_empty());
    // The published datasets are entries of the same catalog, found the same way.
    let published = search(&w, &w.editor, "registrar").await;
    let courses = published["datasets"].as_array().unwrap().iter().find(|d| d["id"] == json!("courses")).expect("the published course catalog");
    assert!(courses["dataset_id"].as_str().unwrap().starts_with("ds_"), "{courses}");
    // Search never reads a row: a value stored in a row finds nothing.
    assert!(names(&search(&w, &w.admin, PLANTED).await).is_empty());
}

#[tokio::test]
async fn a_hidden_dataset_is_in_no_list_count_facet_or_error_for_a_caller_who_may_not_discover_it() {
    let w = world().await;
    let id = dataset_id(&w, &w.payroll, "salaries").await;
    let before = search(&w, &w.editor, "").await;
    assert!(names(&before).contains(&"Pay Bands".to_string()), "the default is everyone");
    let (s, b) = set_policy(&w, &id, json!({ "discoverability": "hidden", "reason": "Salary bands are not public." })).await;
    assert_eq!(s, StatusCode::OK, "{b}");

    let after = search(&w, &w.editor, "").await;
    assert!(!names(&after).contains(&"Pay Bands".to_string()));
    assert_eq!(after["total"].as_i64().unwrap(), before["total"].as_i64().unwrap() - 1, "the count does not include it");
    let everything = after.to_string();
    for needle in ["Pay Bands", "salaries", "band_code", "Hidden Office", "HIDOFF", "SECRET-BAND-1", &w.payroll, &id] {
        assert!(!everything.contains(needle), "{needle} is in the editor's catalog: {everything}");
    }
    // Searching for it by any of its metadata finds nothing, and counts nothing.
    for q in ["pay bands", "band_code", "hidden office", "finance", "job title"] {
        let hit = search(&w, &w.editor, q).await;
        assert_eq!(hit["total"], json!(0), "{q}: {hit}");
        assert!(hit["facets"]["units"].as_array().unwrap().is_empty() && hit["facets"]["functions"].as_array().unwrap().is_empty(), "{q}: {hit}");
    }
    // Asking for it by id is the same answer as asking for an id that does not exist.
    let (s1, hidden) = rest(&w.app, "GET", &format!("/api/v1/datasets/{id}"), &w.editor, None).await;
    let (s2, missing) = rest(&w.app, "GET", "/api/v1/datasets/ds_nothing", &w.editor, None).await;
    assert_eq!((s1, s2), (StatusCode::NOT_FOUND, StatusCode::NOT_FOUND));
    assert_eq!(hidden, missing);
    // The same over MCP.
    let listed = tool(&w, &w.editor, "list_datasets", json!({ "query": "pay bands" })).await;
    assert!(!listed.to_string().contains("Pay Bands"), "{listed}");
    // The administrator who sets the policy still finds it, so it can be changed back.
    assert!(names(&search(&w, &w.admin, "pay bands").await).contains(&"Pay Bands".to_string()));
}

#[tokio::test]
async fn discoverability_by_unit_and_by_position() {
    let w = world().await;
    let id = dataset_id(&w, &w.enroll, "enrollments").await;
    let (s, b) = set_policy(&w, &id, json!({ "discoverability": { "units": [w.chemistry] }, "reason": "Only chemistry." })).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    // The chair holds a post in chemistry. The editor and the physics member do not.
    assert!(names(&search(&w, &w.chair, "enrollment").await).contains(&"Course Enrollment".to_string()));
    assert!(!names(&search(&w, &w.editor, "enrollment").await).contains(&"Course Enrollment".to_string()));
    assert!(!names(&search(&w, &w.member, "enrollment").await).contains(&"Course Enrollment".to_string()));

    let (s, b) = set_policy(&w, &id, json!({ "discoverability": { "positions": ["chair"] }, "reason": "Chairs." })).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert!(names(&search(&w, &w.chair, "enrollment").await).contains(&"Course Enrollment".to_string()));
    assert!(!names(&search(&w, &w.member, "enrollment").await).contains(&"Course Enrollment".to_string()));

    // A unit that is not one, a position that is not one, and a mode that is not one are refused.
    for bad in [json!({ "units": [Uuid::new_v4()] }), json!({ "positions": ["no-such-post"] }), json!("sometimes"), json!({ "units": [] })] {
        let (s, b) = set_policy(&w, &id, json!({ "discoverability": bad, "reason": "x" })).await;
        assert_eq!(s, StatusCode::BAD_REQUEST, "{bad}: {b}");
    }
    // Back to everyone.
    let (s, _) = set_policy(&w, &id, json!({ "discoverability": "everyone", "reason": "Open again." })).await;
    assert_eq!(s, StatusCode::OK);
    assert!(names(&search(&w, &w.member, "enrollment").await).contains(&"Course Enrollment".to_string()));
}

#[tokio::test]
async fn a_restricted_field_is_shown_as_a_restricted_field() {
    let w = world().await;
    let id = dataset_id(&w, &w.enroll, "enrollments").await;
    let (s, b) = set_policy(&w, &id, json!({ "restricted_fields": ["final_grade"], "reason": "Grades are not described." })).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    let body = search(&w, &w.editor, "enrollment").await;
    let entry = body["datasets"].as_array().unwrap().iter().find(|d| d["dataset_id"] == json!(id)).unwrap().clone();
    let fields = entry["fields"].as_array().unwrap();
    assert_eq!(fields.len(), 4);
    let restricted: Vec<&Value> = fields.iter().filter(|f| f["restricted"] == json!(true)).collect();
    assert_eq!(restricted.len(), 1);
    assert_eq!(restricted[0]["label"], json!("a restricted field"));
    let text = entry.to_string();
    assert!(!text.contains("final_grade") && !text.contains("Final grade"), "{text}");
    // The category it carries is not listed either, and a search for its name finds nothing.
    assert!(!entry["categories"].to_string().to_lowercase().contains("ferpa"), "{entry}");
    assert!(names(&search(&w, &w.editor, "final_grade").await).is_empty());
    assert!(names(&search(&w, &w.editor, "ferpa").await).iter().all(|n| n != "Course Enrollment"));
    // The field is not a field the sample shows.
    assert!(!entry["sample"].to_string().contains("B+"));
    // A field the table does not have is refused.
    let (s, _) = set_policy(&w, &id, json!({ "restricted_fields": ["no_such_field"], "reason": "x" })).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn a_sample_is_made_from_the_schema_and_holds_no_stored_value() {
    let w = world().await;
    let all = [
        search(&w, &w.admin, "").await.to_string(),
        search(&w, &w.editor, "enrollment").await.to_string(),
        search(&w, &w.editor, "waiting").await.to_string(),
        tool(&w, &w.editor, "list_datasets", json!({})).await.to_string(),
        tool(&w, &w.admin, "list_datasets", json!({ "query": "enrollment" })).await.to_string(),
        rest(&w.app, "GET", &format!("/api/v1/datasets/{}", dataset_id(&w, &w.enroll, "enrollments").await), &w.admin, None).await.1.to_string(),
    ];
    for text in &all {
        for planted in [PLANTED, IN_MANIFEST, "Student 5", "Waiting 1", "B+"] {
            assert!(!text.contains(planted), "{planted} is in the catalog: {text}");
        }
    }
    let body = search(&w, &w.editor, "enrollment").await;
    let entry = body["datasets"].as_array().unwrap().iter().find(|d| d["name"] == json!("Course Enrollment")).unwrap();
    let rows = entry["sample"]["rows"].as_array().expect("sample rows");
    assert!(!rows.is_empty() && rows.len() <= 3);
    assert_eq!(entry["sample"]["synthetic"], json!(true));
    // A row has a value for each field, of the field's type.
    let row = &rows[0];
    assert!(row["student_name"].is_string() && row["credits"].is_number() && row["enrolled_on"].is_string(), "{row}");
    // The same schema gives the same sample.
    let again = search(&w, &w.editor, "enrollment").await;
    assert_eq!(again["datasets"], body["datasets"]);
}

#[tokio::test]
async fn a_small_groups_count_is_masked_under_the_threshold() {
    let w = world().await;
    let body = search(&w, &w.editor, "").await;
    let find = |body: &Value, name: &str| body["datasets"].as_array().unwrap().iter().find(|d| d["name"] == json!(name)).cloned().unwrap();
    let small = find(&body, "Waiting List");
    assert_eq!(small["record_count"], Value::Null, "{small}");
    assert_eq!(small["size"]["masked"], json!(true));
    assert_eq!(small["size"]["label"], json!("fewer than 11 rows"));
    assert!(!small.to_string().contains("\"3\"") && !small["size"].to_string().contains('3'), "{small}");
    let big = find(&body, "Course Enrollment");
    assert_eq!(big["record_count"], json!(12));
    assert_eq!(big["size"]["masked"], json!(false));
    // A published dataset's declared count follows the same rule.
    assert_eq!(find(&body, "University Course Catalog")["record_count"], json!(840));
    // Freshness is a day, not an instant.
    assert!(big["freshness"].as_str().is_some_and(|d| d.len() == 10), "{big}");
    // The threshold is a setting. At 2 the group of 3 is shown.
    let (s, b) = rest(&w.app, "PUT", "/api/v1/settings/catalog.small_group_min?reason=test", &w.admin, Some(json!(2))).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_eq!(find(&search(&w, &w.editor, "").await, "Waiting List")["record_count"], json!(3));
    for bad in [json!(0), json!(1), json!("11"), json!(100001)] {
        let (s, _) = rest(&w.app, "PUT", "/api/v1/settings/catalog.small_group_min", &w.admin, Some(bad.clone())).await;
        assert_eq!(s, StatusCode::BAD_REQUEST, "{bad}");
    }
}

#[tokio::test]
async fn list_datasets_with_a_query_returns_the_same_entries_over_mcp() {
    let w = world().await;
    for (token, q) in [(&w.editor, "enrollment"), (&w.admin, "registrar"), (&w.editor, "ferpa")] {
        let http = search(&w, token, q).await;
        let mcp = tool(&w, token, "list_datasets", json!({ "query": q })).await;
        let mut items = mcp.get("items").cloned().unwrap_or(mcp);
        // Typed text is marked under `untrusted_text` over MCP (mcp-guidance phase 6). The route does not mark it.
        scaffoldry_server::service::untrusted::restore(&mut items);
        assert_eq!(items, http["datasets"], "{q}");
        assert!(!items.as_array().unwrap().is_empty(), "{q}");
    }
    // The department filter still works, and the query narrows it.
    let both = tool(&w, &w.editor, "list_datasets", json!({ "department": "Office of the Registrar", "query": "course" })).await;
    let items = both.get("items").cloned().unwrap_or(both);
    assert!(items.as_array().unwrap().iter().all(|d| d["department"] == json!("Office of the Registrar")));
    assert!(!items.as_array().unwrap().is_empty());
}

#[tokio::test]
async fn setting_the_policy_needs_an_administrator_a_reason_and_leaves_a_ledger_entry() {
    let w = world().await;
    let id = dataset_id(&w, &w.enroll, "enrollments").await;
    let uri = format!("/api/v1/admin/datasets/{id}/discoverability");
    let (s, _) = rest(&w.app, "PUT", &uri, &w.editor, Some(json!({ "discoverability": "hidden", "reason": "x" }))).await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    let (s, _) = rest(&w.app, "PUT", &uri, &w.admin, Some(json!({ "discoverability": "hidden" }))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "a reason is required");
    let (s, _) = rest(&w.app, "PUT", "/api/v1/admin/datasets/ds_nothing/discoverability", &w.admin, Some(json!({ "discoverability": "hidden", "reason": "x" }))).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    let (s, b) = rest(&w.app, "PUT", &uri, &w.admin, Some(json!({ "discoverability": "hidden", "steward_position": "chair", "reason": "Testing." }))).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_eq!(b["discoverability"], json!("hidden"));
    assert_eq!(b["steward_position"], json!("chair"));
    let (rows, ids): (i64, i64) = w.db.run({
        let id = id.clone();
        move |c| {
            let n = c.query_one("SELECT count(*) FROM governance_ledger WHERE decision_type = 'DataLabelChanged' AND payload->>'dataset' = $1", &[&id]).unwrap().get(0);
            let m = c.query_one("SELECT count(*) FROM governance_ledger WHERE decision_type = 'DataLabelChanged' AND payload->>'setting' = 'discoverability'", &[]).unwrap().get(0);
            (n, m)
        }
    });
    assert_eq!((rows, ids), (1, 1));
    // The steward position shows on the entry for anyone who may discover it. An unset steward is null.
    let (s, _) = set_policy(&w, &id, json!({ "discoverability": "everyone", "reason": "Open." })).await;
    assert_eq!(s, StatusCode::OK);
    let body = search(&w, &w.editor, "enrollment").await;
    let entry = body["datasets"].as_array().unwrap().iter().find(|d| d["dataset_id"] == json!(id)).unwrap().clone();
    assert_eq!(entry["steward"]["position_key"], json!("chair"), "{entry}");
    let other = body["datasets"].as_array().unwrap().iter().find(|d| d["name"] == json!("Waiting List")).unwrap().clone();
    assert_eq!(other["steward"], Value::Null);
    // A position that does not exist is refused.
    let (s, _) = set_policy(&w, &id, json!({ "steward_position": "no-such-post", "reason": "x" })).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    // Only the administrator's own view carries the policy.
    assert!(entry.get("discoverability").is_none(), "{entry}");
    let admin_view = search(&w, &w.admin, "enrollment").await;
    assert!(admin_view["datasets"].as_array().unwrap().iter().any(|d| d["discoverability"].is_string()));
}
