//! Data layer phase 9: system users for automations.
//!
//! An automation (a flow) runs as its own identity, `flow:{rule_id}`, and reads and writes only what that system user was granted. A grant
//! names a dataset, an action and fields. A person approves it, and the approver must hold the access being granted: nobody grants more than
//! they have. The approval is a ledger entry with a reason. Each grant has a review date. When it lapses the automation stops, with a reason
//! in the run, and its owner is told once. The date is read at run time, so no job is needed. The owner is a person, a group or a position.
//! Changing a name, notes or schedule needs no approval. Changing what the flow reads or writes does.

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use common::TestDb;
use http_body_util::BodyExt;
use scaffoldry_core::{ActionType, AutomationRule, ConditionOperator, FieldPredicate, ProcessStatus, ProcessStep, StepKind, TriggerEvent};
use scaffoldry_core::workflow::Target;
use scaffoldry_engine::flow::derive_grants;
use scaffoldry_engine::{AppManifest, HostRouter};
use scaffoldry_server::build_app_with_state;
use scaffoldry_server::service::flows;
use scaffoldry_server::service::identity::issue_test_token_and_user;
use scaffoldry_server::state::{AuthUser, DatasetRecord, OrganizationNode, PositionType, RoleRow, ScimGroup, ServerState, ViaClient};
use serde_json::{json, Value};
use std::sync::Arc;
use tower::ServiceExt;
use uuid::Uuid;

/// Owns the applicants' workspace. Has no rights in the aid office's workspace.
const OWNER: &str = "einstein@physics.state.edu";
const SOURCE_WS: &str = "ws-physics-optics";
/// Owns the aid office's workspace. Has no rights in the applicants' workspace.
const AID_OWNER: &str = "prof.curie@science.state.edu";
const AID_WS: &str = "ws-bio-lab";
const ADMIN: &str = "jordan.lee@state.edu";
const ROOT: &str = "00000000-0000-0000-0000-000000000001";

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
    source: String,
    aid: String,
}

async fn world() -> World {
    let db = TestDb::new();
    let state = db.state();
    state.seed_demo().expect("seed");
    let app = build_app_with_state(state.clone()).expect("router");
    let tag = &Uuid::new_v4().simple().to_string()[..8];
    let (source, aid) = (format!("apl-{tag}"), format!("aid-{tag}"));
    let applicants = json!({
        "slug": source, "title": "Admissions", "description": "", "organization_code": "PHYS", "department": "physics", "workspace_id": SOURCE_WS, "views": [],
        "tables": [{ "id": "applications", "name": "Applications", "slug": "applications", "primary_field": "name", "fields": [
            field("name", "Text"), field("email", "Text"), field("program", "Text"), field("status", "Text"), field("aid_file", "Text")
        ] }],
    });
    let mut indexed_email = field("email", "Text");
    indexed_email["indexed"] = json!(true);
    let mut ssn = field("ssn", "Text");
    ssn["ferpa_sensitive"] = json!(true);
    let mut student_link = field("student", "Link");
    student_link["link_target_table"] = json!("students");
    let financial_aid = json!({
        "slug": aid, "title": "Financial Aid", "description": "", "organization_code": "BIO", "department": "biology", "workspace_id": AID_WS, "views": [],
        "tables": [
            { "id": "students", "name": "Students", "slug": "students", "primary_field": "name", "fields": [field("name", "Text"), indexed_email, ssn] },
            { "id": "awards", "name": "Awards", "slug": "awards", "primary_field": "term", "fields": [field("amount", "Number"), field("term", "Text"), field("note", "Text"), student_link] },
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
        id: format!("flow-{}", Uuid::new_v4().simple()),
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

/// Finds the student by the applicant's email, creates an award with two fields, links it to the student, and writes the award's id back.
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

/// The flow with its grants derived, as saving a version stores it, and installed so the server finds it.
fn install(w: &World, mut flow: AutomationRule) -> AutomationRule {
    flow.grants = derive_grants(&flow, &manifests(w));
    let mut live = w.state.automations.write().unwrap();
    let list = live.entry(w.source.clone()).or_default();
    list.retain(|r| r.id != flow.id);
    list.push(flow.clone());
    flow
}

fn insert(w: &World, slug: &str, data: Value) -> DatasetRecord {
    let id = Uuid::new_v4().to_string();
    let rec = DatasetRecord { id: id.clone(), app_slug: slug.to_string(), data, ceds_mapping: json!({}), is_ferpa_sensitive: false, created_at: chrono::Utc::now().to_rfc3339(), created_by: Some(OWNER.to_string()), version: 1 };
    let ctx = scaffoldry_server::service::history::ctx_for(OWNER, scaffoldry_server::repository::ActorKind::User, slug, &rec.data, &w.state);
    w.state.insert_record(&rec, &ctx).expect("insert");
    w.state.find_record(slug, &id).unwrap().expect("stored")
}

fn student(w: &World, email: &str, extra: Value) -> String {
    let mut data = json!({ "_table_id": "students", "name": "Ada Okoro", "email": email });
    if let (Some(d), Some(e)) = (data.as_object_mut(), extra.as_object()) {
        d.extend(e.clone());
    }
    insert(w, &w.aid, data).id
}

fn application(w: &World, email: &str) -> DatasetRecord {
    insert(w, &w.source, json!({ "_table_id": "applications", "name": "Ada Okoro", "email": email, "program": "Physics PhD", "status": "Admitted" }))
}

fn awards(w: &World) -> Vec<Value> {
    let aid = w.aid.clone();
    w.db.run(move |c| c.query("SELECT jsonb_build_object('id', id) || data FROM dataset_records WHERE app_slug = $1 AND table_id = 'awards'", &[&aid]).unwrap().iter().map(|r| r.get::<_, Value>(0)).collect())
}

async fn dataset(w: &World, slug: &str, table: &str) -> String {
    let (s, body) = send(&w.app, "GET", "/api/v1/data/datasets", ADMIN, None).await;
    assert_eq!(s, StatusCode::OK, "{body}");
    body["datasets"].as_array().unwrap().iter().find(|d| d["app_slug"] == json!(slug) && d["table_id"] == json!(table)).expect("a dataset")["id"].as_str().unwrap().to_string()
}

fn in_days(n: i64) -> String {
    (chrono::Utc::now() + chrono::Duration::days(n)).to_rfc3339()
}

async fn register(w: &World, flow: &AutomationRule, owner: Value) -> (StatusCode, Value) {
    send(&w.app, "POST", "/api/v1/data/system-users", OWNER, Some(json!({ "flow_id": flow.id, "owner": owner, "notes": "Opens an aid file for each admitted applicant" }))).await
}

fn person(who: &str) -> Value {
    json!({ "kind": "user", "id": who })
}

/// Who holds the access to each app's tables in these tests: the owner of its workspace.
fn approver_of(w: &World, app: &str) -> &'static str {
    if app == w.source { OWNER } else { AID_OWNER }
}

async fn grant(w: &World, who: &str, flow: &str, app: &str, table: &str, action: &str, fields: &[&str]) -> (StatusCode, Value) {
    let id = dataset(w, app, table).await;
    send(&w.app, "POST", &format!("/api/v1/data/system-users/{flow}/grants"), who, Some(json!({ "dataset": id, "action": action, "fields": fields, "reason": "The aid office needs this to open a file", "review_by": in_days(90) }))).await
}

/// Approves every grant the flow's steps need, each by the person who holds that access.
async fn grant_all(w: &World, flow: &AutomationRule) {
    for g in &flow.grants {
        let who = approver_of(w, &g.app_slug);
        let mut writes: Vec<&str> = g.write.iter().chain(g.link.iter()).map(String::as_str).collect();
        writes.dedup();
        for (action, fields) in [("read", g.read.iter().map(String::as_str).collect::<Vec<_>>()), ("write", writes), ("create", vec![])] {
            if (action == "create" && !g.create) || (action != "create" && fields.is_empty()) {
                continue;
            }
            let (s, b) = grant(w, who, &flow.id, &g.app_slug, &g.table_id, action, &fields).await;
            assert_eq!(s, StatusCode::CREATED, "{action} {}: {b}", g.table_id);
        }
    }
}

fn notices(w: &World, kind: &str) -> Vec<String> {
    let kind = kind.to_string();
    w.db.run(move |c| c.query("SELECT lower(recipient) FROM notifications WHERE kind = $1 ORDER BY recipient", &[&kind]).unwrap().iter().map(|r| r.get(0)).collect())
}

fn ledger(w: &World, op: &str) -> Vec<(String, Value)> {
    let op = op.to_string();
    w.db.run(move |c| {
        c.query("SELECT rationale, payload FROM governance_ledger WHERE decision_type = 'AccessGrantChanged' AND payload->>'source' = 'system_grant' AND payload->>'op' = $1 ORDER BY sequence", &[&op])
            .unwrap()
            .iter()
            .map(|r| (r.get(0), r.get(1)))
            .collect()
    })
}

/// Moves every grant of a flow past its review date by writing it. A test never waits for a date.
fn lapse(w: &World, flow: &str) {
    let flow = flow.to_string();
    w.db.run(move |c| c.execute("UPDATE system_grants SET review_by = NOW() - interval '1 minute' WHERE flow_id = $1", &[&flow]).unwrap());
}

fn run(w: &World, flow: &AutomationRule, run_id: &str) -> scaffoldry_core::ProcessInstance {
    let trigger = application(w, "ada@state.edu");
    flows::run_flow(&w.state, flow, &trigger, ADMIN, run_id)
}

fn ordinary(eppn: &str) -> AuthUser {
    AuthUser { eppn: eppn.to_string(), name: eppn.to_string(), role_title: "Member".to_string(), affiliation: "faculty".to_string(), department: "x".to_string(), view_as_by: None, via_client: None, shared_link: None }
}

// 1. An automation with a system user has nothing until a grant is approved. It never borrows the person who started it: the run is
// started by a Platform Admin, who holds everything, and still reads and writes nothing.
#[tokio::test(flavor = "multi_thread")]
async fn a_system_user_has_nothing_until_it_is_granted() {
    let w = world().await;
    student(&w, "ada@state.edu", json!({}));
    let flow = install(&w, open_aid_file(&w));
    let (s, b) = register(&w, &flow, person(OWNER)).await;
    assert_eq!(s, StatusCode::CREATED, "{b}");
    assert_eq!(b["flow_id"], json!(flow.id));
    assert_eq!(b["grants"], json!([]), "nothing by default: {b}");

    let inst = run(&w, &flow, "run-1");
    assert_eq!(inst.status, ProcessStatus::Failed, "{inst:?}");
    assert!(inst.log[0].starts_with("system_user_not_granted:"), "a clear reason: {:?}", inst.log);
    assert!(awards(&w).is_empty(), "nothing was written");
    assert!(notices(&w, "system_grant_lapsed").is_empty(), "nothing lapsed, so nobody is told");
}

// 2. A grant reaches only the dataset, action and fields it names. Changing what the flow reads or writes is governed: a new version that
// writes one more field is refused until that field is granted.
#[tokio::test(flavor = "multi_thread")]
async fn a_grant_opens_only_what_it_names_and_a_wider_flow_needs_a_new_one() {
    let w = world().await;
    student(&w, "ada@state.edu", json!({}));
    let flow = install(&w, open_aid_file(&w));
    assert_eq!(register(&w, &flow, person(OWNER)).await.0, StatusCode::CREATED);
    grant_all(&w, &flow).await;

    let inst = run(&w, &flow, "run-2a");
    assert_eq!(inst.status, ProcessStatus::Completed, "{inst:?}");
    assert_eq!(awards(&w).len(), 1);

    // A new version writes `note` as well. The grants of the old version do not reach it.
    let mut wider = flow.clone();
    wider.steps.push(step("note", ActionType::UpdateRecord { target: Target::Created { step: "create_award".into() }, fields: vec![("note".into(), "\"edited in\"".into())] }));
    let wider = install(&w, wider);
    let inst = run(&w, &wider, "run-2b");
    assert_eq!(inst.status, ProcessStatus::Failed, "{inst:?}");
    assert!(inst.log[0].starts_with("system_user_not_granted:") && inst.log[0].contains("note"), "names the field: {:?}", inst.log);
    assert_eq!(awards(&w).len(), 1, "the refused run wrote nothing");

    // A grant of `note` alone, by the person who holds it, opens it and nothing else.
    let (s, b) = grant(&w, AID_OWNER, &flow.id, &w.aid, "awards", "write", &["note"]).await;
    assert_eq!(s, StatusCode::CREATED, "{b}");
    let inst = run(&w, &wider, "run-2c");
    assert_eq!(inst.status, ProcessStatus::Completed, "{inst:?}");
}

// 3. The approver must hold the access. Somebody who lacks it is told so in a plain sentence, and nothing is written. An approval is one
// ledger entry with the reason.
#[tokio::test(flavor = "multi_thread")]
async fn an_approver_must_hold_the_access_and_the_approval_is_in_the_ledger() {
    let w = world().await;
    let flow = install(&w, open_aid_file(&w));
    assert_eq!(register(&w, &flow, person(OWNER)).await.0, StatusCode::CREATED);

    // The owner of the applicants' workspace has no rights in the aid office's apps.
    let (s, b) = grant(&w, OWNER, &flow.id, &w.aid, "awards", "write", &["amount"]).await;
    assert_eq!(s, StatusCode::FORBIDDEN, "{b}");
    let said = b["error"].as_str().unwrap_or_default().to_string();
    assert!(said.contains("Nobody can grant more access than they hold"), "a plain sentence: {b}");
    assert!(ledger(&w, "create").is_empty(), "a refusal writes nothing");
    let (_, shown) = send(&w.app, "GET", &format!("/api/v1/data/system-users/{}", flow.id), OWNER, None).await;
    assert_eq!(shown["grants"], json!([]), "{shown}");

    // The person who holds it approves, and the ledger says who, what and why.
    let (s, b) = grant(&w, AID_OWNER, &flow.id, &w.aid, "awards", "write", &["amount"]).await;
    assert_eq!(s, StatusCode::CREATED, "{b}");
    let entries = ledger(&w, "create");
    assert_eq!(entries.len(), 1, "{entries:?}");
    assert_eq!(entries[0].0, "The aid office needs this to open a file");
    assert_eq!(entries[0].1["approved_by"], json!(AID_OWNER));
    assert_eq!(entries[0].1["holder"], json!(format!("flow:{}", flow.id)));
    assert_eq!(entries[0].1["fields"], json!(["amount"]));

    // A grant needs a reason, and a review date that is ahead and not more than a year away.
    let id = dataset(&w, &w.aid, "awards").await;
    for bad in [
        json!({ "dataset": id, "action": "write", "fields": ["term"], "review_by": in_days(30) }),
        json!({ "dataset": id, "action": "write", "fields": ["term"], "reason": "Needed", "review_by": in_days(-1) }),
        json!({ "dataset": id, "action": "write", "fields": ["term"], "reason": "Needed", "review_by": in_days(900) }),
        json!({ "dataset": id, "action": "write", "fields": ["nope"], "reason": "Needed", "review_by": in_days(30) }),
        json!({ "dataset": id, "action": "delete", "fields": ["term"], "reason": "Needed", "review_by": in_days(30) }),
    ] {
        let (s, b) = send(&w.app, "POST", &format!("/api/v1/data/system-users/{}/grants", flow.id), AID_OWNER, Some(bad.clone())).await;
        assert_eq!(s, StatusCode::BAD_REQUEST, "{bad}: {b}");
    }
    assert_eq!(ledger(&w, "create").len(), 1);
}

// 3b. A guest, a view-as session and a call through an outside app's ceiling cannot approve or register.
#[tokio::test(flavor = "multi_thread")]
async fn a_guest_a_view_as_and_an_outside_app_cannot_approve() {
    let w = world().await;
    let flow = install(&w, open_aid_file(&w));
    assert_eq!(register(&w, &flow, person(OWNER)).await.0, StatusCode::CREATED);
    let dataset_id = dataset(&w, &w.aid, "awards").await;
    let body = json!({ "dataset": dataset_id, "action": "write", "fields": ["amount"], "reason": "Needed", "review_by": in_days(30) });

    let mut guest = ordinary(AID_OWNER);
    guest.affiliation = "guest".to_string();
    let mut viewing = ordinary(AID_OWNER);
    viewing.view_as_by = Some(Box::new(ordinary(ADMIN)));
    let mut outside = ordinary(AID_OWNER);
    outside.via_client = Some(Box::new(ViaClient { client_id: "c".into(), name: "Outside".into(), ceiling: Default::default() }));
    for who in [guest, viewing, outside] {
        let (flow_id, body, state, who2) = (flow.id.clone(), body.clone(), w.state.clone(), who.clone());
        let got = tokio::task::spawn_blocking(move || scaffoldry_server::service::system_users::approve(&who2, &flow_id, &body, &state)).await.unwrap();
        assert!(matches!(got, Err(scaffoldry_server::service::ServiceError::Forbidden { .. })), "{:?} {got:?}", who.affiliation);
    }
    assert!(ledger(&w, "create").is_empty());
}

// 4. A sensitive field is never granted, and a row whose category the system user does not meet is not read by the flow.
#[tokio::test(flavor = "multi_thread")]
async fn a_sensitive_field_is_refused_and_a_category_gate_still_closes_a_row() {
    let w = world().await;
    let (s, b) = {
        let flow = install(&w, open_aid_file(&w));
        assert_eq!(register(&w, &flow, person(OWNER)).await.0, StatusCode::CREATED);
        grant(&w, AID_OWNER, &flow.id, &w.aid, "students", "read", &["ssn"]).await
    };
    assert_eq!(s, StatusCode::BAD_REQUEST, "{b}");
    assert!(b.to_string().contains("sensitive"), "{b}");

    // A category that asks a registrar. The flow has no position, so it reads no tagged row.
    w.state.settings.write().unwrap().insert("sensitivity.categories".to_string(), json!([{ "id": "phi", "name": "PHI", "protected": true, "detectors": [], "requires": [{ "position": "registrar", "reach": "unit_and_below" }] }]));
    let tagged = student(&w, "ada@state.edu", json!({ "_tags": ["phi"] }));
    // Without a system user the flow reads inside its own grants, as it did before, and links the row.
    let plain = install(&w, { let mut r = open_aid_file(&w); r.name = "Plain".into(); r });
    assert_eq!(run(&w, &plain, "run-4a").status, ProcessStatus::Completed);
    assert_eq!(awards(&w)[0]["student"], json!([tagged]), "the control flow links the tagged row");

    // As a system user, with every grant its steps need, it still does not read the row: it holds no position.
    let flow = install(&w, open_aid_file(&w));
    assert_eq!(register(&w, &flow, person(OWNER)).await.0, StatusCode::CREATED);
    grant_all(&w, &flow).await;
    assert_eq!(run(&w, &flow, "run-4b").status, ProcessStatus::Completed);
    let made = awards(&w);
    assert_eq!(made.len(), 2, "{made:?}");
    let unlinked = made.iter().filter(|a| a["student"] != json!([tagged])).count();
    assert_eq!(unlinked, 1, "the system user's award links no tagged row: {made:?}");
}

// 5. A lapsed grant stops the automation. The run is refused with a reason, with no job running. The owner is told once.
#[tokio::test(flavor = "multi_thread")]
async fn a_lapsed_grant_refuses_the_run_and_tells_the_owner_once() {
    let w = world().await;
    student(&w, "ada@state.edu", json!({}));
    let flow = install(&w, open_aid_file(&w));
    assert_eq!(register(&w, &flow, person(OWNER)).await.0, StatusCode::CREATED);
    grant_all(&w, &flow).await;
    assert_eq!(run(&w, &flow, "run-5a").status, ProcessStatus::Completed);
    assert!(notices(&w, "system_grant_lapsed").is_empty());

    lapse(&w, &flow.id);
    let inst = run(&w, &flow, "run-5b");
    assert_eq!(inst.status, ProcessStatus::Failed, "{inst:?}");
    assert!(inst.log[0].starts_with("system_user_lapsed:"), "a clear reason: {:?}", inst.log);
    assert_eq!(awards(&w).len(), 1, "the lapsed run wrote nothing");
    let told = notices(&w, "system_grant_lapsed");
    assert!(!told.is_empty() && told.iter().all(|t| t == OWNER), "the owner is told: {told:?}");
    let first = told.len();

    let inst = run(&w, &flow, "run-5c");
    assert!(inst.log[0].starts_with("system_user_lapsed:"), "{:?}", inst.log);
    assert_eq!(notices(&w, "system_grant_lapsed").len(), first, "and not told twice");

    // The shown system user says which grants lapsed. A new grant, with a new date, opens the same access again.
    let (_, shown) = send(&w.app, "GET", &format!("/api/v1/data/system-users/{}", flow.id), OWNER, None).await;
    assert!(shown["grants"].as_array().unwrap().iter().all(|g| g["status"] == "Lapsed"), "{shown}");
    grant_all(&w, &flow).await;
    assert_eq!(run(&w, &flow, "run-5d").status, ProcessStatus::Completed);
}

// 6. A position owns it, and the alert follows whoever holds the seat now. A group owns it, and its members are told.
#[tokio::test(flavor = "multi_thread")]
async fn a_position_or_group_owner_is_told_by_who_holds_the_seat_now() {
    let w = world().await;
    let root = Uuid::parse_str(ROOT).unwrap();
    let unit = Uuid::new_v4();
    let node = OrganizationNode { id: unit, parent_id: Some(root), name: "aid".into(), code: "AID".into(), org_type: "Unit".into(), hierarchy_key: "academic".into(), synthetic: false };
    w.state.organizations.write().unwrap().insert(unit, node.clone());
    w.state.persist_organization(&node).unwrap();
    w.state
        .persist_position_type(&PositionType { key: "automation_steward".into(), name: "Automation steward".into(), description: String::new(), org_types: vec![], max_holders: 5, retired_at: None, created_by: "test".into(), created_at: chrono::Utc::now().to_rfc3339() })
        .unwrap();
    let appoint = |eppn: &str| {
        let id = Uuid::new_v4();
        w.state
            .persist_role(&RoleRow { id, person_id: Uuid::new_v4(), eppn: eppn.into(), organization_id: unit, role_title: "steward".into(), scoped_affiliation: "staff".into(), is_primary: false, source: "test".into(), position_key: Some("automation_steward".into()) })
            .unwrap();
        id
    };
    let first_seat = appoint("first.holder@state.edu");

    student(&w, "ada@state.edu", json!({}));
    let flow = install(&w, open_aid_file(&w));
    let (s, b) = register(&w, &flow, json!({ "kind": "position", "id": format!("automation_steward@{unit}") })).await;
    assert_eq!(s, StatusCode::CREATED, "{b}");
    assert_eq!(b["owner"]["people"], json!(["first.holder@state.edu"]), "{b}");
    grant_all(&w, &flow).await;

    // The seat changes hands before the grant lapses.
    w.state.delete_role(first_seat).unwrap();
    appoint("second.holder@state.edu");
    lapse(&w, &flow.id);
    assert_eq!(run(&w, &flow, "run-6a").status, ProcessStatus::Failed);
    let told = notices(&w, "system_grant_lapsed");
    assert!(!told.is_empty() && told.iter().all(|t| t == "second.holder@state.edu"), "the current holder is told, and the old one is not: {told:?}");

    // A group.
    let group = format!("grp-{}", &Uuid::new_v4().simple().to_string()[..8]);
    w.state.repository.upsert_scim_group(&ScimGroup { id: group.clone(), display_name: group.clone(), members: vec![json!({ "value": "g1@state.edu" }), json!({ "value": "g2@state.edu" })] }).unwrap();
    let other = install(&w, { let mut r = open_aid_file(&w); r.name = "Second".into(); r });
    let (s, b) = register(&w, &other, json!({ "kind": "group", "id": group })).await;
    assert_eq!(s, StatusCode::CREATED, "{b}");
    grant_all(&w, &other).await;
    lapse(&w, &other.id);
    assert_eq!(run(&w, &other, "run-6b").status, ProcessStatus::Failed);
    let told = notices(&w, "system_grant_lapsed");
    assert!(told.contains(&"g1@state.edu".to_string()) && told.contains(&"g2@state.edu".to_string()), "{told:?}");

    // An owner who is nobody the server knows is refused.
    let third = install(&w, { let mut r = open_aid_file(&w); r.name = "Third".into(); r });
    for owner in [json!({ "kind": "group", "id": "no-such-group" }), json!({ "kind": "position", "id": format!("automation_steward@{}", Uuid::new_v4()) }), json!({ "kind": "user", "id": "nobody@nowhere.example" }), json!({ "kind": "unit", "id": "x" })] {
        let (s, b) = register(&w, &third, owner.clone()).await;
        assert_eq!(s, StatusCode::BAD_REQUEST, "{owner}: {b}");
    }
}

// 7. A name, notes or schedule change needs no approval and writes no access entry. The owner makes it, and a stranger does not.
#[tokio::test(flavor = "multi_thread")]
async fn a_change_of_name_notes_or_schedule_needs_no_approval() {
    let w = world().await;
    student(&w, "ada@state.edu", json!({}));
    let flow = install(&w, open_aid_file(&w));
    assert_eq!(register(&w, &flow, person(OWNER)).await.0, StatusCode::CREATED);
    grant_all(&w, &flow).await;
    let before = ledger(&w, "create").len();

    let (s, b) = send(&w.app, "PATCH", &format!("/api/v1/data/system-users/{}", flow.id), OWNER, Some(json!({ "name": "Aid file opener", "notes": "Moved to the spring cycle" }))).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_eq!(b["name"], json!("Aid file opener"));
    assert_eq!(b["notes"], json!("Moved to the spring cycle"));

    // A stranger who owns nothing here changes nothing.
    let (s, b) = send(&w.app, "PATCH", &format!("/api/v1/data/system-users/{}", flow.id), "prof.lovelace@state.edu", Some(json!({ "name": "Mine now" }))).await;
    assert_eq!(s, StatusCode::FORBIDDEN, "{b}");

    // The owner hands it to another person, with no approval and no new grant.
    let (s, b) = send(&w.app, "PATCH", &format!("/api/v1/data/system-users/{}", flow.id), OWNER, Some(json!({ "owner": person(AID_OWNER) }))).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_eq!(b["owner"]["id"], json!(AID_OWNER));

    // A schedule is the flow's own trigger. A new version with another trigger and the same steps reads and writes the same, so it still runs.
    let mut rescheduled = flow.clone();
    rescheduled.trigger = TriggerEvent::RecordUpdated;
    rescheduled.name = "Renamed".to_string();
    let rescheduled = install(&w, rescheduled);
    assert_eq!(run(&w, &rescheduled, "run-7").status, ProcessStatus::Completed);
    assert_eq!(ledger(&w, "create").len(), before, "nothing was approved");
    assert!(ledger(&w, "end").is_empty());
}

// 8. A revoked grant stops the run at once and writes a ledger entry with its reason. A grant is revoked by the person who manages the flow.
#[tokio::test(flavor = "multi_thread")]
async fn a_revoked_grant_stops_the_run_and_is_in_the_ledger() {
    let w = world().await;
    student(&w, "ada@state.edu", json!({}));
    let flow = install(&w, open_aid_file(&w));
    assert_eq!(register(&w, &flow, person(OWNER)).await.0, StatusCode::CREATED);
    grant_all(&w, &flow).await;
    assert_eq!(run(&w, &flow, "run-8a").status, ProcessStatus::Completed);
    let (_, shown) = send(&w.app, "GET", &format!("/api/v1/data/system-users/{}", flow.id), OWNER, None).await;
    let first = shown["grants"][0]["id"].as_str().unwrap().to_string();
    let uri = format!("/api/v1/data/system-users/{}/grants/{first}/revoke", flow.id);
    let (s, b) = send(&w.app, "POST", &uri, "prof.lovelace@state.edu", Some(json!({ "reason": "No" }))).await;
    assert_eq!(s, StatusCode::FORBIDDEN, "{b}");
    let (s, b) = send(&w.app, "POST", &uri, OWNER, Some(json!({ "reason": "The cycle is over" }))).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_eq!(ledger(&w, "end").len(), 1);
    let inst = run(&w, &flow, "run-8b");
    assert_eq!(inst.status, ProcessStatus::Failed, "{inst:?}");
    assert!(inst.log[0].starts_with("system_user_not_granted:"), "{:?}", inst.log);
    let (s, _) = send(&w.app, "POST", &uri, OWNER, Some(json!({ "reason": "Twice" }))).await;
    assert_eq!(s, StatusCode::CONFLICT);
}
