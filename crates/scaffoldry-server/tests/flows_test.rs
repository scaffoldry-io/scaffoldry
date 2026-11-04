//! Workflows phase 1: flows run inside approved grants and reach records by address.
//!
//! A flow is a stored rule with steps that find, create, update and link records, in its own app or another. Its grants
//! are derived from the steps and stored with the version. At run time every read and write is checked against them.

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use common::TestDb;
use http_body_util::BodyExt;
use scaffoldry_core::workflow::{FlowGrant, Target};
use scaffoldry_core::{ActionType, AutomationRule, ConditionOperator, FieldPredicate, ProcessStatus, ProcessStep, StepKind, TriggerEvent};
use scaffoldry_engine::flow::{derive_grants, grants_sentence, validate_flow};
use scaffoldry_engine::{AppManifest, HostRouter};
use scaffoldry_server::build_app_with_state;
use scaffoldry_server::service::flows;
use scaffoldry_server::service::identity::issue_test_token_and_user;
use scaffoldry_server::state::{DatasetRecord, ServerState};
use serde_json::{json, Value};
use std::sync::Arc;
use tower::ServiceExt;

/// Owns the applicants' workspace. Has no rights in the aid office's workspace.
const OWNER: &str = "einstein@physics.state.edu";
const SOURCE_WS: &str = "ws-physics-optics";
/// Owns the aid office's workspace.
const AID_OWNER: &str = "prof.curie@science.state.edu";
const AID_WS: &str = "ws-bio-lab";

async fn send(app: &axum::Router, method: &str, uri: &str, who: &str, body: Option<Value>) -> (StatusCode, Value) {
    let b = Request::builder().method(method).uri(uri).header("authorization", format!("Bearer {}", issue_test_token_and_user(who)));
    let req = match body {
        Some(v) => b.header("content-type", "application/json").body(Body::from(v.to_string())).unwrap(),
        None => b.body(Body::empty()).unwrap(),
    };
    let resp = app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
}

fn field(name: &str, kind: &str) -> Value {
    json!({ "name": name, "label": name, "field_type": kind, "required": false, "ferpa_sensitive": false })
}

struct World {
    db: TestDb,
    state: Arc<ServerState>,
    app: axum::Router,
    /// The applicants' app: one table, `applications`.
    source: String,
    /// The aid office's app: `students` and `awards`, and `items` for the cap test.
    aid: String,
}

async fn world() -> World {
    let db = TestDb::new();
    let state = db.state();
    state.seed_demo().expect("seed");
    let app = build_app_with_state(state.clone()).expect("router");
    let tag = &uuid::Uuid::new_v4().simple().to_string()[..8];
    let (source, aid) = (format!("apl-{tag}"), format!("aid-{tag}"));
    let applicants = json!({
        "slug": source, "title": "Admissions", "description": "", "organization_code": "PHYS", "department": "physics", "workspace_id": SOURCE_WS, "views": [],
        "tables": [{ "id": "applications", "name": "Applications", "slug": "applications", "primary_field": "name", "fields": [
            field("name", "Text"), field("email", "Text"), field("program", "Text"), field("status", "Text"), field("aid_file", "Text"), field("secret", "Text")
        ] }],
    });
    let mut indexed_email = field("email", "Text");
    indexed_email["indexed"] = json!(true);
    let mut indexed_tag = field("tag", "Text");
    indexed_tag["indexed"] = json!(true);
    let mut student_link = field("student", "Link");
    student_link["link_target_table"] = json!("students");
    let mut items: Vec<Value> = vec![indexed_tag];
    items.extend((1..=11).map(|n| field(&format!("f{n}"), "Text")));
    let financial_aid = json!({
        "slug": aid, "title": "Financial Aid", "description": "", "organization_code": "BIO", "department": "biology", "workspace_id": AID_WS, "views": [],
        "tables": [
            { "id": "students", "name": "Students", "slug": "students", "primary_field": "name", "fields": [field("name", "Text"), indexed_email] },
            { "id": "awards", "name": "Awards", "slug": "awards", "primary_field": "term", "fields": [field("amount", "Number"), field("term", "Text"), field("note", "Text"), student_link] },
            { "id": "items", "name": "Items", "slug": "items", "primary_field": "tag", "fields": items },
        ],
    });
    let (s, b) = send(&app, "POST", &format!("/api/v1/workspaces/{SOURCE_WS}/apps"), OWNER, Some(applicants)).await;
    assert_eq!(s, StatusCode::CREATED, "{b}");
    let (s, b) = send(&app, "POST", &format!("/api/v1/workspaces/{AID_WS}/apps"), AID_OWNER, Some(financial_aid)).await;
    assert_eq!(s, StatusCode::CREATED, "{b}");
    World { db, state, app, source, aid }
}

fn manifests(w: &World) -> Vec<AppManifest> {
    let eng = w.state.engine.read().unwrap();
    [&w.source, &w.aid].iter().map(|s| eng.resolve_by_slug(s).cloned().expect("app")).collect()
}

fn step(id: &str, action: ActionType) -> ProcessStep {
    ProcessStep { id: id.to_string(), when: vec![], kind: StepKind::Service { action } }
}

fn rule(w: &World, name: &str, steps: Vec<ProcessStep>) -> AutomationRule {
    AutomationRule {
        version: 0,
        id: format!("flow-{}", uuid::Uuid::new_v4().simple()),
        app_slug: w.source.clone(),
        name: name.to_string(),
        description: String::new(),
        enabled: true,
        trigger: TriggerEvent::RecordCreated,
        predicates: vec![FieldPredicate { field_name: "status".into(), operator: ConditionOperator::Equals, expected_value: "Admitted".into() }],
        actions: vec![],
        steps,
        home_workspace: Some(SOURCE_WS.to_string()),
        grants: vec![],
    }
}

/// Finds the student by the applicant's email, creates an award with two fields, links it to the student, and writes
/// the award's id back onto the application.
fn open_aid_file(w: &World) -> AutomationRule {
    rule(
        w,
        "Open aid file",
        vec![
            step("find_student", ActionType::FindRecord { id: "find_student".into(), app: w.aid.clone(), table: "students".into(), field: "email".into(), equals: "{trigger.email}".into(), limit: 1 }),
            step("create_award", ActionType::CreateRecord { id: "create_award".into(), app: w.aid.clone(), table: "awards".into(), fields: vec![("amount".into(), "1000".into()), ("term".into(), "{trigger.program}".into())] }),
            step("link_student", ActionType::SetLink { target: Target::Created { step: "create_award".into() }, link_field: "student".into(), to: Target::Found { step: "find_student".into() } }),
            step("note_file", ActionType::UpdateRecord { target: Target::Trigger, fields: vec![("aid_file".into(), "{create_award.id}".into())] }),
        ],
    )
}

/// The flow with its grants derived, as saving a version stores it.
fn with_grants(w: &World, mut flow: AutomationRule) -> AutomationRule {
    flow.grants = derive_grants(&flow, &manifests(w));
    flow
}

fn install(w: &World, flow: &AutomationRule) {
    w.state.automations.write().unwrap().entry(w.source.clone()).or_default().push(flow.clone());
}

fn insert(w: &World, slug: &str, data: Value) -> DatasetRecord {
    let id = uuid::Uuid::new_v4().to_string();
    let rec = DatasetRecord {
        id: id.clone(),
        app_slug: slug.to_string(),
        data,
        ceds_mapping: json!({}),
        is_ferpa_sensitive: false,
        created_at: chrono::Utc::now().to_rfc3339(),
        created_by: Some(OWNER.to_string()),
        version: 1,
    };
    let ctx = scaffoldry_server::service::history::ctx_for(OWNER, scaffoldry_server::repository::ActorKind::User, slug, &rec.data, &w.state);
    w.state.insert_record(&rec, &ctx).expect("insert");
    w.state.find_record(slug, &id).unwrap().expect("stored")
}

fn student(w: &World, email: &str) -> String {
    insert(w, &w.aid, json!({ "_table_id": "students", "name": "Ada Okoro", "email": email })).id
}

fn application(w: &World, email: &str) -> DatasetRecord {
    insert(w, &w.source, json!({ "_table_id": "applications", "name": "Ada Okoro", "email": email, "program": "Physics PhD", "status": "Admitted" }))
}

fn rows(w: &World, sql: &str, slug: &str) -> Vec<Value> {
    let (sql, slug) = (sql.to_string(), slug.to_string());
    w.db.run(move |c| c.query(sql.as_str(), &[&slug]).unwrap().iter().map(|r| r.get::<_, Value>(0)).collect())
}

fn awards(w: &World) -> Vec<Value> {
    rows(w, "SELECT jsonb_build_object('id', id) || data FROM dataset_records WHERE app_slug = $1 AND table_id = 'awards'", &w.aid)
}

fn effects_of(w: &World, instance: &str) -> i64 {
    let like = instance.to_string();
    w.db.run(move |c| c.query_one("SELECT COUNT(*) FROM flow_effects WHERE instance_id = $1", &[&like]).unwrap().get(0))
}

// 1. A flow triggered by an application creates an award in another app, sets two fields, and links the award. The
// award's history shows the flow as actor and the triggering person as `on_behalf_of`.
#[tokio::test(flavor = "multi_thread")]
async fn a_flow_creates_and_links_a_record_in_another_app() {
    let w = world().await;
    let student_id = student(&w, "ada@state.edu");
    let flow = with_grants(&w, open_aid_file(&w));
    install(&w, &flow);

    let (s, b) = send(&w.app, "POST", &format!("/api/v1/apps/{}/records", w.source), OWNER, Some(json!({ "data": { "_table_id": "applications", "name": "Ada Okoro", "email": "ada@state.edu", "program": "Physics PhD", "status": "Admitted" } }))).await;
    assert!(s.is_success(), "{b}");
    let app_id = b["id"].as_str().or_else(|| b["record"]["id"].as_str()).unwrap().to_string();

    let found = awards(&w);
    assert_eq!(found.len(), 1, "one award: {found:?}");
    let award = &found[0];
    assert_eq!(award["amount"].as_f64(), Some(1000.0), "{award}");
    assert_eq!(award["term"], json!("Physics PhD"), "{award}");
    assert_eq!(award["student"], json!([student_id]), "linked to the student: {award}");
    let back = w.state.repository.links_to(&student_id, "student").unwrap();
    assert_eq!(back.len(), 1, "the link is in the link index too");
    let award_id = award["id"].as_str().unwrap().to_string();
    let application = w.state.find_record(&w.source, &app_id).unwrap().unwrap();
    assert_eq!(application.data["aid_file"], json!(award_id), "the award is noted on the application");

    let history = rows(&w, "SELECT jsonb_build_object('actor', actor, 'kind', actor_kind, 'on_behalf_of', on_behalf_of) FROM record_history WHERE app_slug = $1 AND table_id = 'awards'", &w.aid);
    assert!(!history.is_empty());
    for h in &history {
        assert_eq!(h["actor"], json!(format!("flow:{}", flow.id)), "{h}");
        assert_eq!(h["kind"], json!("automation"), "{h}");
        assert_eq!(h["on_behalf_of"], json!(OWNER), "{h}");
    }
    let (s, b) = send(&w.app, "GET", &format!("/api/v1/apps/{}/records/{award_id}/history", w.aid), AID_OWNER, None).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    let entry = &b["entries"][0];
    assert_eq!(entry["on_behalf_of"], json!(OWNER), "{entry}");
    assert!(entry["actor_name"].as_str().unwrap().starts_with("Flow \"Open aid file\", started by "), "{entry}");
    assert!(!entry["actor_name"].as_str().unwrap().ends_with("by "), "names the person: {entry}");
}

// 2. `derive_grants` returns exactly the fields the flow reads and writes, and its sentence is the expected text.
// Saving a version stores them.
#[tokio::test(flavor = "multi_thread")]
async fn grants_are_derived_from_the_steps_and_said_in_a_sentence() {
    let w = world().await;
    let flow = open_aid_file(&w);
    let grants = derive_grants(&flow, &manifests(&w));
    let s = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>();
    assert_eq!(
        grants,
        vec![
            FlowGrant { app_slug: w.source.clone(), table_id: "applications".into(), read: s(&["status", "email", "program"]), write: s(&["aid_file"]), create: false, link: vec![] },
            FlowGrant { app_slug: w.aid.clone(), table_id: "students".into(), read: s(&["email"]), write: vec![], create: false, link: vec![] },
            FlowGrant { app_slug: w.aid.clone(), table_id: "awards".into(), read: vec![], write: s(&["amount", "term"]), create: true, link: s(&["student"]) },
        ]
    );
    assert_eq!(
        grants_sentence(&grants, &w.source, &manifests(&w)),
        "This flow may read Applications: status, email, program; Students: email. It may create records in Financial Aid / Awards. \
         It may set Applications: aid_file; Awards: amount, term, student. It may not change anything else."
    );
    assert!(validate_flow(&flow, &manifests(&w)).is_empty(), "{:?}", validate_flow(&flow, &manifests(&w)));

    let saved = scaffoldry_server::service::processes::save_rule(OWNER, &w.source, flow, None, "Open aid file", &w.state).expect("saved");
    assert_eq!(saved.grants, grants, "the version stores its grants");
}

// 3. A step edited after approval to write a field outside the stored grants fails at run time with
// `flow_grant_exceeded`, and the instance shows the reason.
#[tokio::test(flavor = "multi_thread")]
async fn a_write_outside_the_grants_fails_the_run() {
    let w = world().await;
    student(&w, "ada@state.edu");
    let mut flow = with_grants(&w, open_aid_file(&w));
    flow.steps.push(step("note", ActionType::UpdateRecord { target: Target::Created { step: "create_award".into() }, fields: vec![("note".into(), "\"edited in\"".into())] }));
    let trigger = application(&w, "ada@state.edu");
    let inst = flows::run_flow(&w.state, &flow, &trigger, OWNER, "run-3");
    assert_eq!(inst.status, ProcessStatus::Failed, "{inst:?}");
    let reason = inst.log.join(" ");
    assert!(reason.contains("flow_grant_exceeded") && reason.contains("note"), "{reason}");
    let stored = w.state.repository.get_process_instance(&inst.id).unwrap().expect("the instance is stored");
    assert_eq!(stored.status, ProcessStatus::Failed);
    assert!(stored.log.join(" ").contains("flow_grant_exceeded"), "{:?}", stored.log);
    assert!(awards(&w).iter().all(|a| a.get("note").is_none()), "the note was never written");
}

// 4. `FindRecord` with a limit of 101 fails validation. It matches by an indexed field only, and reads only the records
// that match.
#[tokio::test(flavor = "multi_thread")]
async fn find_record_is_bounded_and_uses_the_index() {
    let w = world().await;
    let too_many = rule(&w, "Too many", vec![step("f", ActionType::FindRecord { id: "f".into(), app: w.aid.clone(), table: "students".into(), field: "email".into(), equals: "{trigger.email}".into(), limit: 101 })]);
    let problems = validate_flow(&too_many, &manifests(&w));
    assert!(problems.iter().any(|p| p.contains("101") && p.contains("100")), "{problems:?}");
    let unindexed = rule(&w, "Unindexed", vec![step("f", ActionType::FindRecord { id: "f".into(), app: w.aid.clone(), table: "students".into(), field: "name".into(), equals: "{trigger.name}".into(), limit: 1 })]);
    let problems = validate_flow(&unindexed, &manifests(&w));
    assert!(problems.iter().any(|p| p.contains("name") && p.contains("not indexed")), "{problems:?}");

    let ada = student(&w, "Ada@State.edu");
    student(&w, "grace@state.edu");
    student(&w, "alan@state.edu");
    let found = flows::find_records(&w.state, &w.aid, "students", "email", &json!("ada@state.edu"), 100).unwrap();
    assert_eq!(found.iter().map(|r| r.id.clone()).collect::<Vec<_>>(), vec![ada], "a case-insensitive match on the index");
    assert!(flows::FIND_SQL.contains("record_index") && !flows::FIND_SQL.contains("data"), "the match reads the index only: {}", flows::FIND_SQL);
}

// 5. There is no delete variant, and the flow runner has no way to delete.
#[test]
fn a_flow_cannot_delete() {
    let core = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../scaffoldry-core/src/workflow.rs")).unwrap();
    let start = core.find("pub enum ActionType").expect("ActionType");
    let body = &core[start..start + core[start..].find("\n}").unwrap()];
    assert!(!body.to_lowercase().contains("delete") && !body.to_lowercase().contains("remove"), "no delete effect: {body}");
    let runner = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/src/service/flows.rs")).unwrap();
    for word in ["delete_record", "remove_record", "DELETE FROM"] {
        assert!(!runner.contains(word), "the runner holds {word}");
    }
}

// 6. A reference to a later step fails validation, naming it.
#[tokio::test(flavor = "multi_thread")]
async fn a_reference_to_a_later_step_fails_validation() {
    let w = world().await;
    let flow = rule(
        &w,
        "Backwards",
        vec![
            step("step_1", ActionType::UpdateRecord { target: Target::Trigger, fields: vec![("aid_file".into(), "{step_9.term}".into())] }),
            step("step_9", ActionType::CreateRecord { id: "step_9".into(), app: w.aid.clone(), table: "awards".into(), fields: vec![("term".into(), "\"Fall\"".into())] }),
        ],
    );
    let problems = validate_flow(&flow, &manifests(&w));
    assert!(problems.iter().any(|p| p.contains("step_9") && p.contains("later")), "{problems:?}");
    let unknown = rule(&w, "Unknown", vec![step("s", ActionType::UpdateRecord { target: Target::Trigger, fields: vec![("aid_file".into(), "{nowhere.term}".into())] })]);
    assert!(validate_flow(&unknown, &manifests(&w)).iter().any(|p| p.contains("nowhere")), "an unknown reference fails too");
}

// 7. The same run twice, as a retry: the second writes nothing because the dedupe keys exist.
#[tokio::test(flavor = "multi_thread")]
async fn a_retry_never_writes_twice() {
    let w = world().await;
    student(&w, "ada@state.edu");
    let flow = with_grants(&w, open_aid_file(&w));
    let trigger = application(&w, "ada@state.edu");
    let first = flows::run_flow(&w.state, &flow, &trigger, OWNER, "run-7");
    assert_eq!(first.status, ProcessStatus::Completed, "{first:?}");
    let history = rows(&w, "SELECT to_jsonb(COUNT(*)) FROM record_history WHERE app_slug = $1", &w.aid);
    let effects = effects_of(&w, &first.id);
    assert_eq!(effects, 3, "create, link, and the note on the application");

    let second = flows::run_flow(&w.state, &flow, &trigger, OWNER, "run-7");
    assert_eq!(second.status, ProcessStatus::Completed, "{second:?}");
    assert_eq!(awards(&w).len(), 1, "one award");
    assert_eq!(rows(&w, "SELECT to_jsonb(COUNT(*)) FROM record_history WHERE app_slug = $1", &w.aid), history, "nothing written again");
    assert_eq!(effects_of(&w, &first.id), effects);
}

// 8. A person with no rights on the target app triggers the flow by editing a record they can edit, and causes no write
// outside the grants.
#[tokio::test(flavor = "multi_thread")]
async fn a_person_without_rights_on_the_target_triggers_only_the_granted_writes() {
    let w = world().await;
    student(&w, "ada@state.edu");
    let (s, _) = send(&w.app, "GET", &format!("/api/v1/apps/{}/records", w.aid), OWNER, None).await;
    assert!(s == StatusCode::FORBIDDEN || s == StatusCode::NOT_FOUND, "the owner of the applicants' app cannot read the aid app: {s}");
    let flow = with_grants(&w, open_aid_file(&w));
    install(&w, &flow);

    let (s, b) = send(&w.app, "POST", &format!("/api/v1/apps/{}/records", w.source), OWNER, Some(json!({ "data": { "_table_id": "applications", "name": "Ada", "email": "ada@state.edu", "program": "Physics PhD", "status": "Admitted", "secret": "Not-For-Aid-4411" } }))).await;
    assert!(s.is_success(), "{b}");
    let found = awards(&w);
    assert_eq!(found.len(), 1, "{found:?}");
    let allowed = ["id", "_table_id", "amount", "term", "student"];
    for key in found[0].as_object().unwrap().keys() {
        assert!(allowed.contains(&key.as_str()), "the award holds only granted fields, not {key}: {}", found[0]);
    }
    let everything = rows(&w, "SELECT to_jsonb(data::text) FROM dataset_records WHERE app_slug = $1", &w.aid);
    assert!(everything.iter().all(|d| !d.as_str().unwrap().contains("Not-For-Aid-4411")), "nothing outside the grants reached the aid app");
}

// 9. A run that would make 1,001 writes fails with the count. The first writes stay, recorded, and the instance is
// `Failed`.
#[tokio::test(flavor = "multi_thread")]
async fn a_run_over_the_write_cap_fails_with_the_count() {
    let w = world().await;
    for _ in 0..100 {
        insert(&w, &w.aid, json!({ "_table_id": "items", "tag": "batch" }));
    }
    let mut steps = vec![step("find", ActionType::FindRecord { id: "find".into(), app: w.aid.clone(), table: "items".into(), field: "tag".into(), equals: "\"batch\"".into(), limit: 100 })];
    for n in 1..=11 {
        steps.push(step(&format!("set_{n}"), ActionType::UpdateRecord { target: Target::Found { step: "find".into() }, fields: vec![(format!("f{n}"), "\"done\"".into())] }));
    }
    let flow = with_grants(&w, rule(&w, "Mark the batch", steps));
    let trigger = application(&w, "ada@state.edu");
    let inst = flows::run_flow(&w.state, &flow, &trigger, OWNER, "run-9");
    assert_eq!(inst.status, ProcessStatus::Failed, "{inst:?}");
    let reason = inst.log.join(" ");
    assert!(reason.contains("1001") && reason.contains("1000"), "the count and the limit: {reason}");
    assert_eq!(effects_of(&w, &inst.id), 1000, "the first writes are recorded");
    let done = rows(&w, "SELECT to_jsonb(COUNT(*)) FROM dataset_records WHERE app_slug = $1 AND data->>'f10' = 'done'", &w.aid);
    assert_eq!(done, vec![json!(100)], "the first writes stay");
    let stored = w.state.repository.get_process_instance(&inst.id).unwrap().unwrap();
    assert_eq!(stored.status, ProcessStatus::Failed);
}
