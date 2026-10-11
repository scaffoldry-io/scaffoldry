//! Links phase 1: a record points at another record by id, and `record_links` indexes the pointers.
//!
//! Each test uses a private app with a fresh slug and deletes its rows when it ends.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::Router;
use http_body_util::BodyExt;
use scaffoldry_engine::{AppManifest, AppTable, FieldSpec, FieldType, LinkCardinality};
use scaffoldry_server::build_app_with_state;
use scaffoldry_server::repository::{PostgresRepository, SqlParam};
use scaffoldry_server::repository_links::{LINKS_FROM_SQL, LINKS_TO_SQL};
use scaffoldry_server::service::identity::issue_test_token_and_user;
use scaffoldry_server::state::ServerState;
use serde_json::{json, Value};
use std::sync::Arc;
use tower::ServiceExt;
use uuid::Uuid;

const OWNER: &str = "prof.curie@science.state.edu";
const WORKSPACE: &str = "ws-bio-lab";
const LINKS: i64 = 200_000;

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
        let slug = self.0.clone();
        let url = db_url();
        let _ = std::thread::spawn(move || {
            if let Ok(mut c) = postgres::Client::connect(&url, postgres::NoTls) {
                let _ = c.execute("DELETE FROM record_links WHERE app_slug = $1", &[&slug]);
                let _ = c.execute("DELETE FROM dataset_records WHERE app_slug = $1", &[&slug]);
            }
        })
        .join();
    }
}

fn text(name: &str) -> FieldSpec {
    FieldSpec::simple(name, name, FieldType::Text, false, false)
}

fn link(name: &str, target: &str, cardinality: LinkCardinality) -> FieldSpec {
    let mut f = FieldSpec::simple(name, name, FieldType::Link, false, false);
    f.link_target_table = Some(target.to_string());
    f.link_cardinality = cardinality;
    f
}

fn table(id: &str, fields: Vec<FieldSpec>, primary: &str) -> AppTable {
    AppTable {
        id: id.to_string(),
        name: id.to_string(),
        slug: id.to_string(),
        description: None,
        icon: None,
        fields,
        primary_field: Some(primary.to_string()),
        sample_records: vec![],
    }
}

/// `apps.program` holds one program. `apps.also` holds many. `programs.code` is the display name.
fn manifest(slug: &str) -> AppManifest {
    AppManifest {
        slug: slug.to_string(),
        title: "Links".to_string(),
        description: String::new(),
        organization_code: "BIO".to_string(),
        department: "biology".to_string(),
        workspace_id: Some(WORKSPACE.to_string()),
        herm_capability_id: None,
        custom_domain: None,
        custom_domain_verified: false,
        tables: vec![
            table("apps", vec![text("applicant"), link("program", "programs", LinkCardinality::One), link("also", "programs", LinkCardinality::Many)], "applicant"),
            table("programs", vec![text("code")], "code"),
        ],
        relationships: vec![],
        views: vec![],
        ceds_mappings: Default::default(),
        pages: vec![],
        access: Default::default(),
    }
}

fn link_fields() -> Vec<FieldSpec> {
    vec![link("program", "programs", LinkCardinality::One), link("also", "programs", LinkCardinality::Many)]
}

struct World {
    app: Router,
    token: String,
    slug: String,
    _cleanup: Cleanup,
}

async fn world() -> World {
    let state = Arc::new(ServerState::new().expect("state"));
    state.seed_demo().expect("seed");
    let slug = format!("lnk-{}", &Uuid::new_v4().to_string()[..8]);
    state.engine.write().unwrap().register_manifest(manifest(&slug)).unwrap();
    World {
        app: build_app_with_state(state.clone()).expect("router"),
        token: issue_test_token_and_user(OWNER),
        _cleanup: Cleanup(slug.clone()),
        slug,
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

async fn try_create(w: &World, table: &str, mut data: Value) -> (StatusCode, Value) {
    data["_table_id"] = json!(table);
    send(&w.app, "POST", &format!("/api/v1/apps/{}/records", w.slug), &w.token, Some(json!({ "data": data }))).await
}

async fn create(w: &World, table: &str, data: Value) -> Value {
    let (s, body) = try_create(w, table, data).await;
    assert_eq!(s, StatusCode::CREATED, "{body}");
    body
}

async fn try_patch(w: &World, id: &str, version: i64, data: Value) -> (StatusCode, Value) {
    send(&w.app, "PATCH", &format!("/api/v1/apps/{}/records/{id}", w.slug), &w.token, Some(json!({ "data": data, "version": version }))).await
}

async fn patch(w: &World, id: &str, version: i64, data: Value) -> Value {
    let (s, body) = try_patch(w, id, version, data).await;
    assert_eq!(s, StatusCode::OK, "{body}");
    body
}

fn id_of(v: &Value) -> String {
    v["id"].as_str().unwrap().to_string()
}

/// The link rows of an app, ordered: `(from, field, to, ord)`.
type LinkRow = (String, String, String, i16);

fn link_rows(slug: &str) -> Vec<LinkRow> {
    let slug = slug.to_string();
    db(move |c| {
        c.query("SELECT from_record, field, to_record, ord FROM record_links WHERE app_slug = $1 ORDER BY from_record, field, ord, to_record", &[&slug])
            .unwrap()
            .iter()
            .map(|r| (r.get(0), r.get(1), r.get(2), r.get(3)))
            .collect()
    })
}

// 1. A write keeps one row per linked id, and a change replaces the old rows.
#[tokio::test]
async fn a_link_writes_a_row_and_changing_it_replaces_the_row() {
    let w = world().await;
    let p1 = id_of(&create(&w, "programs", json!({ "code": "CS" })).await);
    let p2 = id_of(&create(&w, "programs", json!({ "code": "BIO" })).await);
    let a = create(&w, "apps", json!({ "applicant": "Ada", "program": p1 })).await;
    let aid = id_of(&a);
    assert_eq!(link_rows(&w.slug), vec![(aid.clone(), "program".into(), p1.clone(), 0)]);

    let a = patch(&w, &aid, 1, json!({ "applicant": "Ada", "program": p2 })).await;
    assert_eq!(link_rows(&w.slug), vec![(aid.clone(), "program".into(), p2.clone(), 0)], "the old row is gone and the new one is present");

    // A many-field keeps its order.
    let a = patch(&w, &aid, a["version"].as_i64().unwrap(), json!({ "applicant": "Ada", "program": p2, "also": [p2.clone(), p1.clone()] })).await;
    let rows = link_rows(&w.slug);
    assert!(rows.contains(&(aid.clone(), "also".into(), p2.clone(), 0)) && rows.contains(&(aid.clone(), "also".into(), p1.clone(), 1)), "{rows:?}");
    patch(&w, &aid, a["version"].as_i64().unwrap(), json!({ "applicant": "Ada", "program": p2, "also": [p1.clone()] })).await;
    let rows = link_rows(&w.slug);
    assert_eq!(rows.iter().filter(|r| r.1 == "also").count(), 1, "{rows:?}");
}

// 2. One accepts one id. A missing id and an id from another table are 400 and name the field.
#[tokio::test]
async fn a_link_with_too_many_ids_a_missing_id_or_an_id_from_another_table_is_refused_naming_the_field() {
    let w = world().await;
    let p1 = id_of(&create(&w, "programs", json!({ "code": "CS" })).await);
    let p2 = id_of(&create(&w, "programs", json!({ "code": "BIO" })).await);
    let other = id_of(&create(&w, "apps", json!({ "applicant": "Not a program" })).await);
    let before = link_rows(&w.slug);

    for (label, value) in [
        ("two ids in a One field", json!([p1.clone(), p2.clone()])),
        ("a missing id", json!("rec-does-not-exist")),
        ("an id from another table", json!(other.clone())),
        ("a value that is not an id", json!(42)),
    ] {
        let (status, body) = try_create(&w, "apps", json!({ "applicant": "X", "program": value })).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{label}: {body}");
        assert_eq!(body["code"], "bad_request", "{label}: {body}");
        assert!(body["fields"].get("program").is_some(), "{label} names the field: {body}");
    }
    assert_eq!(link_rows(&w.slug), before, "a refused write leaves no rows");
    // A Many field takes the list that a One field refused.
    let (status, body) = try_create(&w, "apps", json!({ "applicant": "Y", "also": [p1, p2] })).await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
}

// 3. A link to a record the writer cannot see is refused the same way as a missing one.
#[tokio::test]
async fn a_link_to_a_record_in_an_app_the_writer_cannot_see_is_refused_like_a_missing_one() {
    let w = world().await;
    // A program record that belongs to another app, which the writer has no part in.
    let foreign_slug = format!("lnk-foreign-{}", &Uuid::new_v4().to_string()[..8]);
    let foreign_id = format!("rec-{foreign_slug}");
    {
        let (slug, id) = (foreign_slug.clone(), foreign_id.clone());
        db(move |c| {
            c.execute(
                "INSERT INTO dataset_records (id, app_slug, data, ceds_mapping, is_ferpa_sensitive, table_id) VALUES ($1, $2, '{\"_table_id\":\"programs\",\"code\":\"SECRET\"}', '{}', false, 'programs')",
                &[&id, &slug],
            )
            .unwrap();
        });
    }
    let _foreign = Cleanup(foreign_slug.clone());

    let (s_foreign, b_foreign) = try_create(&w, "apps", json!({ "applicant": "X", "program": foreign_id })).await;
    let (s_missing, b_missing) = try_create(&w, "apps", json!({ "applicant": "X", "program": "rec-nobody-has-this" })).await;
    assert_eq!(s_foreign, StatusCode::BAD_REQUEST, "{b_foreign}");
    assert_eq!(s_foreign, s_missing);
    let shape = |b: &Value, id: &str| b.to_string().replace(id, "<id>");
    assert_eq!(shape(&b_foreign, &foreign_id), shape(&b_missing, "rec-nobody-has-this"), "the answer does not tell the two apart");
    assert!(!b_foreign.to_string().contains("SECRET"));
}

// 4. The JSON and the table always agree, and a rebuild from the JSON reproduces the table.
#[tokio::test]
async fn the_json_and_record_links_agree_after_mixed_writes_and_a_rebuild_reproduces_them() {
    let w = world().await;
    let mut programs = Vec::new();
    for code in ["P1", "P2", "P3", "P4"] {
        programs.push(id_of(&create(&w, "programs", json!({ "code": code })).await));
    }
    let a = create(&w, "apps", json!({ "applicant": "A", "program": programs[0], "also": [programs[1], programs[2]] })).await;
    let b = create(&w, "apps", json!({ "applicant": "B", "also": [programs[3]] })).await;
    let c = create(&w, "apps", json!({ "applicant": "C", "program": programs[2] })).await;
    let (aid, bid, cid) = (id_of(&a), id_of(&b), id_of(&c));
    let a2 = patch(&w, &aid, 1, json!({ "applicant": "A", "program": programs[3], "also": [programs[2], programs[1], programs[0]] })).await;
    patch(&w, &bid, 1, json!({ "applicant": "B", "also": [] })).await;
    patch(&w, &aid, a2["version"].as_i64().unwrap(), json!({ "applicant": "A2", "also": [programs[0]] })).await;
    let (status, _) = send(&w.app, "DELETE", &format!("/api/v1/apps/{}/records/{cid}?version=1", w.slug), &w.token, None).await;
    assert!(status.is_success() || status == StatusCode::NO_CONTENT, "{status}");

    // What the JSON says.
    let expected = || {
        let slug = w.slug.clone();
        db(move |c| {
            let mut out: Vec<LinkRow> = Vec::new();
            for row in c.query("SELECT id, data FROM dataset_records WHERE app_slug = $1 AND table_id = 'apps' AND deleted_at IS NULL", &[&slug]).unwrap() {
                let (id, data): (String, Value) = (row.get(0), row.get(1));
                for field in ["program", "also"] {
                    let ids: Vec<String> = match data.get(field) {
                        Some(Value::String(s)) => vec![s.clone()],
                        Some(Value::Array(a)) => a.iter().filter_map(|v| v.as_str().map(String::from)).collect(),
                        _ => vec![],
                    };
                    for (ord, to) in ids.into_iter().enumerate() {
                        out.push((id.clone(), field.to_string(), to, ord as i16));
                    }
                }
            }
            out.sort_by(|x, y| (&x.0, &x.1, x.3, &x.2).cmp(&(&y.0, &y.1, y.3, &y.2)));
            out
        })
    };
    let table_now = link_rows(&w.slug);
    assert_eq!(table_now, expected(), "the table follows the JSON");

    let repo = PostgresRepository::connect(None).unwrap();
    let written = repo.rebuild_links(&w.slug, "apps", &link_fields()).unwrap();
    assert_eq!(written as usize, table_now.len());
    assert_eq!(link_rows(&w.slug), table_now, "the rebuild reproduces the table exactly");
}

// 6. At 200,000 links, a record's links and its reverse links each use an index.
#[tokio::test]
async fn at_200000_links_finding_links_and_reverse_links_uses_an_index() {
    let w = world().await;
    {
        let slug = w.slug.clone();
        db(move |c| {
            c.execute(
                "INSERT INTO record_links (app_slug, field, from_record, to_record, ord) \
                 SELECT $1, 'program', 'from-' || g, 'to-' || (g % 1000), 0 FROM generate_series(1, $2::bigint) AS g",
                &[&slug, &LINKS],
            )
            .unwrap();
            c.batch_execute("ANALYZE record_links").unwrap();
        });
    }
    let repo = PostgresRepository::connect(None).unwrap();
    let forward = vec![SqlParam::Text("from-777".into()), SqlParam::Text("program".into())];
    let reverse = vec![SqlParam::Text("to-777".into()), SqlParam::Text("program".into())];
    for (name, sql, params) in [("forward", LINKS_FROM_SQL, &forward), ("reverse", LINKS_TO_SQL, &reverse)] {
        let plan = repo.explain(sql, params).unwrap();
        assert!(!plan.contains("Seq Scan"), "{name}: {plan}");
        assert!(plan.contains("record_links"), "{name}: {plan}");
    }
    assert_eq!(repo.links_from("from-777", "program").unwrap(), vec!["to-777".to_string()]);
    let back = repo.links_to("to-777", "program").unwrap();
    assert_eq!(back.len(), 200, "{} records link to to-777", back.len());
}

// 7. History records the id list, and shows names instead of ids.
#[tokio::test]
async fn history_shows_the_display_names_of_linked_records_not_their_ids() {
    let w = world().await;
    let cs = id_of(&create(&w, "programs", json!({ "code": "CS-101" })).await);
    let bio = id_of(&create(&w, "programs", json!({ "code": "BIO-200" })).await);
    let a = create(&w, "apps", json!({ "applicant": "Ada", "program": cs })).await;
    let aid = id_of(&a);
    patch(&w, &aid, 1, json!({ "applicant": "Ada", "program": bio })).await;

    let (s, h) = send(&w.app, "GET", &format!("/api/v1/apps/{}/records/{aid}/history", w.slug), &w.token, None).await;
    assert_eq!(s, StatusCode::OK, "{h}");
    let text = h.to_string();
    assert!(text.contains("CS-101") && text.contains("BIO-200"), "names are shown: {text}");
    assert!(!text.contains(&cs) && !text.contains(&bio), "ids are not: {text}");

    // The stored row still holds ids, so the change can be traced.
    let slug = w.slug.clone();
    let stored: String = db(move |c| c.query_one("SELECT changes::text FROM record_history WHERE app_slug = $1 AND action = 'update' LIMIT 1", &[&slug]).unwrap().get(0));
    assert!(stored.contains(&bio), "{stored}");
}
