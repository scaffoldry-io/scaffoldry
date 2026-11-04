//! Workflows phase 5: My Work, the data and the API.
//!
//! `work_items` holds what each person is asked to do and what they wait on. A step that starts waiting opens an item
//! for each person who may act on it and a `waiting` item for the starter and the record's creator, in the transaction
//! that starts it. A decision closes them. Jobs repair drift: `reresolve_work` after a change of who holds what, and the
//! nightly `reconcile_work`. The decision route never trusts an item: it re-checks who may decide, live.

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::Router;
use common::TestDb;
use http_body_util::BodyExt;
use scaffoldry_core::approver::{ApproverSpec, Origin};
use scaffoldry_core::standards::eduperson::{EduPersonAffiliation, EduPersonIdentity};
use scaffoldry_core::{ActionType, AutomationRule, ProcessStep, StepKind, TriggerEvent};
use scaffoldry_server::build_app_with_state;
use scaffoldry_server::service::identity::issue_test_token_and_user;
use scaffoldry_server::state::{CollaboratorRecord, DatasetRecord, ServerState, WorkspaceRecord};
use serde_json::{json, Value};
use std::sync::Arc;
use tower::ServiceExt;
use uuid::Uuid;

const ROOT_UNIT: &str = "00000000-0000-0000-0000-000000000001";
/// A record value no title or summary may hold.
const SECRET: &str = "Rec-Value-7731-Confidential";

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

fn person(prefix: &str) -> String {
    format!("{prefix}-{}@state.edu", &Uuid::new_v4().to_string()[..8])
}

/// One department, its workspace and an app in it.
struct Dept {
    unit: String,
    ws: String,
    app: String,
}

struct World {
    db: TestDb,
    app: Router,
    state: Arc<ServerState>,
    admin: String,
    sam: String,
}

impl World {
    async fn user(&self, who: &str, member_of: &[&Dept]) -> String {
        let (s, _) = send(&self.app, "POST", "/api/v1/admin/users", &self.admin, Some(json!({
            "userName": who, "name": "Test Person", "email": who, "affiliation": "faculty", "department": "Physics", "title": "Lecturer", "reason": "Fixture"
        }))).await;
        assert_eq!(s, StatusCode::CREATED, "creating a test user failed");
        for d in member_of {
            self.state.collaborators.write().unwrap().entry(d.ws.clone()).or_default().push(CollaboratorRecord {
                id: format!("collab-{who}-{}", d.ws),
                workspace_id: d.ws.clone(),
                eppn: who.to_string(),
                name: who.to_string(),
                role: "editor".to_string(),
                scoped_affiliation: "faculty".to_string(),
                department: "physics".to_string(),
                added_at: String::new(),
                app_slug: String::new(),
            });
        }
        issue_test_token_and_user(who)
    }

    async fn dept(&self, name: &str) -> Dept {
        let tag = &Uuid::new_v4().simple().to_string()[..6];
        let (s, b) = send(&self.app, "POST", "/api/v1/orgs", &self.admin, Some(json!({ "name": format!("{name} {tag}"), "code": format!("D{tag}"), "org_type": "Department", "parent_id": ROOT_UNIT }))).await;
        assert_eq!(s, StatusCode::CREATED, "{b}");
        let unit = b["id"].as_str().unwrap().to_string();
        let ws = format!("ws-{tag}");
        let app = format!("{}-{tag}", name.to_lowercase());
        let record: WorkspaceRecord = serde_json::from_value(json!({ "id": ws, "name": name, "code": format!("W{tag}"), "organization": "State", "visibility": "restricted", "organization_id": unit })).unwrap();
        self.state.persist_workspace(record).unwrap();
        let manifest: scaffoldry_engine::AppManifest = serde_json::from_value(json!({
            "slug": app, "title": format!("{name} requests"), "description": "", "organization_code": "PHYS", "department": "physics", "workspace_id": ws, "views": [],
            "tables": [{ "id": "requests", "name": "Requests", "slug": "requests", "primary_field": "title", "fields": [
                { "name": "title", "label": "Title", "field_type": "Text", "required": false, "ferpa_sensitive": false },
                { "name": "status", "label": "Status", "field_type": "Text", "required": false, "ferpa_sensitive": false }
            ] }],
        }))
        .unwrap();
        self.state.engine.write().unwrap().register_manifest(manifest).unwrap();
        Dept { unit, ws, app }
    }

    async fn hold(&self, unit: &str, who: &str) {
        let (s, b) = send(&self.app, "POST", &format!("/api/v1/orgs/{unit}/positions/chair/holders"), &self.admin, Some(json!({ "eppn": who, "replace": false, "reason": "Fixture" }))).await;
        assert_eq!(s, StatusCode::CREATED, "hold: {b}");
    }

    /// A rule whose one step goes to `approver`, started by Sam on a record Sam made. Returns the instance id.
    fn start(&self, d: &Dept, approver: ApproverSpec) -> String {
        let rule = AutomationRule {
            version: 0,
            home_workspace: None,
            grants: vec![],
            id: format!("rule-{}", &Uuid::new_v4().simple().to_string()[..8]),
            app_slug: d.app.clone(),
            name: "Equipment request".to_string(),
            description: String::new(),
            enabled: true,
            trigger: TriggerEvent::RecordCreated,
            predicates: vec![],
            actions: vec![],
            steps: vec![ProcessStep {
                id: "chair-review".into(),
                when: vec![],
                kind: StepKind::UserTask {
                    role: String::new(),
                    prompt: "Review the request".into(),
                    approve: vec![ActionType::UpdateRecordStatus { new_status: "Approved".into() }],
                    reject: vec![],
                    approver: Some(approver),
                    due: None,
                    remind_every: None,
                    on_due: vec![],
                    on_fail: vec![],
                    decision: Default::default(),
                    reject_rule: Default::default(),
                    kind: Default::default(),
                    show: vec![],
                    fill: vec![],
                    require: vec![],
                    decline_reason: false,
                },
            }],
        };
        let rule_id = rule.id.clone();
        self.state.automations.write().unwrap().insert(d.app.clone(), vec![rule]);
        let record_id = format!("rec-{}", &Uuid::new_v4().simple().to_string()[..8]);
        let data = json!({ "_table_id": "requests", "title": SECRET, "status": SECRET });
        self.state
            .put_record(
                &DatasetRecord { id: record_id.clone(), app_slug: d.app.clone(), data: data.clone(), ceds_mapping: Default::default(), is_ferpa_sensitive: false, created_at: String::new(), created_by: Some(self.sam.clone()), version: 1 },
                &scaffoldry_server::repository::WriteCtx::system("test fixture"),
            )
            .unwrap();
        let identity = EduPersonIdentity { eppn: self.sam.clone(), realm: "state.edu".into(), affiliations: vec![EduPersonAffiliation::Faculty] };
        let mut with_id = data;
        with_id["id"] = json!(record_id);
        let mut applied = vec![];
        scaffoldry_server::service::records::run_automations(&self.state, &d.app, TriggerEvent::RecordCreated, &with_id, &scaffoldry_server::service::records::Actor { identity: &identity, department: "Physics" }, 0, &mut applied);
        let id = format!("{rule_id}:{record_id}");
        assert!(self.state.repository.get_process_instance(&id).unwrap().is_some(), "the rule started an instance");
        id
    }

    async fn work(&self, token: &str, query: &str) -> Vec<Value> {
        let (s, b) = send(&self.app, "GET", &format!("/api/v1/me/work{query}"), token, None).await;
        assert_eq!(s, StatusCode::OK, "{b}");
        b["items"].as_array().cloned().unwrap_or_default()
    }

    async fn decide(&self, token: &str, d: &Dept, instance: &str) -> StatusCode {
        send(&self.app, "POST", &format!("/api/v1/apps/{}/processes/{instance}/decide", d.app), token, Some(json!({ "decision": "approve" }))).await.0
    }

    /// `(person, kind, via, state)` of an instance's items.
    fn items(&self, instance: &str) -> Vec<(String, String, String, String)> {
        let i = instance.to_string();
        self.db.run(move |c| {
            c.query("SELECT person, kind, via, state FROM work_items WHERE instance_id = $1 ORDER BY person, kind", &[&i]).unwrap().iter().map(|r| (r.get(0), r.get(1), r.get(2), r.get(3))).collect()
        })
    }
}

async fn world() -> World {
    let db = TestDb::new();
    let admin = issue_test_token_and_user("jordan.lee@state.edu");
    let state = db.state();
    let app = build_app_with_state(state.clone()).expect("router");
    let mut w = World { db, app, state, admin, sam: person("sam") };
    let (s, b) = send(&w.app, "POST", "/api/v1/admin/positions", &w.admin, Some(json!({
        "key": "chair", "name": "Chair", "description": "", "org_types": ["Department"], "max_holders": 1, "reason": "Fixture"
    })))
    .await;
    assert_eq!(s, StatusCode::CREATED, "{b}");
    w.sam = person("sam");
    w
}

fn chair() -> ApproverSpec {
    ApproverSpec::Position { key: "chair".into(), from: Origin::Workspace, walk_up: false }
}

// 1. A person who holds a position in three workspaces sees an item from each in one list.
#[tokio::test(flavor = "multi_thread")]
async fn three_workspaces_in_one_list() {
    let w = world().await;
    let (a, b, c) = (w.dept("Physics").await, w.dept("Chemistry").await, w.dept("Biology").await);
    let rivera = person("rivera");
    let token = w.user(&rivera, &[&a, &b, &c]).await;
    w.user(&w.sam.clone(), &[&a, &b, &c]).await;
    for d in [&a, &b, &c] {
        w.hold(&d.unit, &rivera).await;
        w.start(d, chair());
    }
    let items = w.work(&token, "?kind=decision").await;
    let mut spaces: Vec<String> = items.iter().map(|i| i["workspace_id"].as_str().unwrap_or_default().to_string()).collect();
    spaces.sort();
    let mut want = vec![a.ws.clone(), b.ws.clone(), c.ws.clone()];
    want.sort();
    assert_eq!(spaces, want, "{items:?}");
    assert!(items.iter().all(|i| i["via"] == "holder" && i["kind"] == "decision"));

    let (s, sum) = send(&w.app, "GET", "/api/v1/me/work/summary", &token, None).await;
    assert_eq!(s, StatusCode::OK, "{sum}");
    assert_eq!(sum["by_kind"]["decision"], 3, "{sum}");

    let sams = w.work(&issue_test_token_and_user(&w.sam), "?kind=waiting").await;
    assert_eq!(sams.len(), 3, "the starter waits on each: {sams:?}");
}

// 2. A delegate sees the delegator's items marked `delegate` and `acting_for`. When the delegation ends, the items go on
// the next re-resolve and the decide route refuses at once.
#[tokio::test(flavor = "multi_thread")]
async fn a_delegate_sees_the_items_until_the_delegation_ends() {
    let w = world().await;
    let d = w.dept("Physics").await;
    let (rivera, ada) = (person("rivera"), person("ada"));
    let rivera_t = w.user(&rivera, &[&d]).await;
    let ada_t = w.user(&ada, &[&d]).await;
    w.user(&w.sam.clone(), &[&d]).await;
    w.hold(&d.unit, &rivera).await;
    let now = chrono::Utc::now();
    let (s, del) = send(&w.app, "POST", "/api/v1/me/delegations", &rivera_t, Some(json!({
        "delegate": ada, "position_key": "chair", "starts_at": (now - chrono::Duration::hours(1)).to_rfc3339(),
        "ends_at": (now + chrono::Duration::days(7)).to_rfc3339(), "reason": "Away"
    })))
    .await;
    assert_eq!(s, StatusCode::CREATED, "{del}");
    let inst = w.start(&d, chair());

    let mine = w.work(&ada_t, "?kind=decision").await;
    assert_eq!(mine.len(), 1, "{mine:?}");
    assert_eq!(mine[0]["via"], "delegate");
    assert_eq!(mine[0]["acting_for"], rivera.as_str());
    assert_eq!(w.work(&ada_t, &format!("?acting_for={rivera}")).await.len(), 1);

    let (s, _) = send(&w.app, "DELETE", &format!("/api/v1/me/delegations/{}", del["id"].as_str().unwrap()), &rivera_t, None).await;
    assert!(s.is_success());
    assert_eq!(w.decide(&ada_t, &d, &inst).await, StatusCode::FORBIDDEN, "the decide route checks live, before any re-resolve");
    scaffoldry_server::service::work::reresolve(&w.state).unwrap();
    assert!(w.work(&ada_t, "?kind=decision").await.is_empty(), "the item went with the re-resolve");
}

// 3. A person removed from a position loses their items after `reresolve_work`, and an attempt to decide before it runs
// is refused.
#[tokio::test(flavor = "multi_thread")]
async fn removed_from_a_position_the_items_go() {
    let w = world().await;
    let d = w.dept("Physics").await;
    let rivera = person("rivera");
    let token = w.user(&rivera, &[&d]).await;
    w.user(&w.sam.clone(), &[&d]).await;
    w.hold(&d.unit, &rivera).await;
    let inst = w.start(&d, chair());
    assert_eq!(w.work(&token, "?kind=decision").await.len(), 1);

    let (s, b) = send(&w.app, "DELETE", &format!("/api/v1/orgs/{}/positions/chair/holders/{rivera}", d.unit), &w.admin, Some(json!({ "reason": "Left" }))).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_eq!(w.decide(&token, &d, &inst).await, StatusCode::FORBIDDEN, "refused before the re-resolve");
    scaffoldry_server::service::work::reresolve(&w.state).unwrap();
    assert!(w.work(&token, "?kind=decision").await.is_empty());
    assert!(w.items(&inst).iter().any(|i| i.0 == rivera && i.3 == "closed"), "the item is closed, not deleted");
}

// 4. Deliberately corrupt an item (delete one, add a wrong one): `reconcile_work` repairs both and reports two repairs.
#[tokio::test(flavor = "multi_thread")]
async fn reconcile_repairs_a_missing_and_a_wrong_item() {
    let w = world().await;
    let d = w.dept("Physics").await;
    let rivera = person("rivera");
    w.user(&rivera, &[&d]).await;
    w.user(&w.sam.clone(), &[&d]).await;
    w.hold(&d.unit, &rivera).await;
    let inst = w.start(&d, chair());
    let before = w.items(&inst);
    let (i, stranger) = (inst.clone(), person("stranger"));
    let (r, s) = (rivera.clone(), stranger.clone());
    w.db.run(move |c| {
        c.execute("DELETE FROM work_items WHERE instance_id = $1 AND person = $2", &[&i, &r]).unwrap();
        c.execute(
            "INSERT INTO work_items (id, person, kind, instance_id, step_id, title, summary) VALUES ($1, $2, 'decision', $3, 'chair-review', 'x', 'x')",
            &[&Uuid::new_v4(), &s, &i],
        )
        .unwrap();
    });
    assert_eq!(scaffoldry_server::service::work::reconcile(&w.state).unwrap(), 2, "one item added back, one closed");
    let open: Vec<_> = w.items(&inst).into_iter().filter(|i| i.3 == "open").collect();
    let open_before: Vec<_> = before.into_iter().filter(|i| i.3 == "open").collect();
    assert_eq!(open, open_before, "the open items are as they were");
    assert!(w.items(&inst).iter().any(|i| i.0 == stranger && i.3 == "closed"));
}

// 5. An item for an app the person can no longer read and holds no grant for does not appear.
#[tokio::test(flavor = "multi_thread")]
async fn an_item_for_an_app_the_person_cannot_read_is_hidden() {
    let w = world().await;
    let d = w.dept("Physics").await;
    let outsider = person("outsider");
    let token = w.user(&outsider, &[]).await;
    w.user(&w.sam.clone(), &[&d]).await;
    let inst = w.start(&d, ApproverSpec::Person { eppn: outsider.clone() });
    assert!(w.items(&inst).iter().any(|i| i.0 == outsider && i.3 == "open"), "the item exists");
    assert!(w.work(&token, "").await.is_empty(), "and is not shown: the person cannot read the app and holds no grant");
}

// 6. A title or summary built with a record value in the fixture contains none. Every producer goes through the template
// builder.
#[tokio::test(flavor = "multi_thread")]
async fn no_title_or_summary_holds_a_record_value() {
    let w = world().await;
    let d = w.dept("Physics").await;
    let rivera = person("rivera");
    let token = w.user(&rivera, &[&d]).await;
    w.user(&w.sam.clone(), &[&d]).await;
    w.hold(&d.unit, &rivera).await;
    w.start(&d, chair());
    let rows: Vec<(String, String)> = w.db.run(|c| c.query("SELECT title, summary FROM work_items", &[]).unwrap().iter().map(|r| (r.get(0), r.get(1))).collect());
    assert!(!rows.is_empty());
    for (title, summary) in &rows {
        assert!(!title.contains(SECRET) && !summary.contains(SECRET), "{title} / {summary}");
    }
    let shown = w.work(&token, "").await;
    assert!(!Value::Array(shown).to_string().contains(SECRET));

    // The only place an item's text is made is the template builder: no other source writes into work_items.
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut writers = vec![];
    for dir in [root.clone(), root.join("service"), root.join("routes")] {
        for entry in std::fs::read_dir(&dir).unwrap() {
            let p = entry.unwrap().path();
            if p.extension().and_then(|e| e.to_str()) == Some("rs") && std::fs::read_to_string(&p).unwrap().contains("INSERT INTO work_items") {
                writers.push(p.file_name().unwrap().to_string_lossy().to_string());
            }
        }
    }
    assert_eq!(writers, vec!["repository_work.rs".to_string()], "only the work repository writes items");
}

// 7. 10,000 open items for one person page through with the index and no sequential scan.
#[tokio::test(flavor = "multi_thread")]
async fn ten_thousand_items_page_through_with_the_index() {
    let w = world().await;
    let who = person("busy");
    let p = who.clone();
    w.db.run(move |c| {
        c.execute(
            "INSERT INTO work_items (id, person, kind, title, summary, due_at, created_at) \
             SELECT gen_random_uuid(), $1, 'decision', 'Decide', 'A step waits', NOW() + make_interval(mins => g), NOW() - make_interval(secs => g) FROM generate_series(1, 10000) g",
            &[&p],
        )
        .unwrap();
        // Other people's items, so the person's are a small part of the table.
        c.execute(
            "INSERT INTO work_items (id, person, kind, title, summary, due_at) SELECT gen_random_uuid(), 'p' || g || '@state.edu', 'decision', 'Decide', 'A step waits', NOW() FROM generate_series(1, 20000) g",
            &[],
        )
        .unwrap();
        c.batch_execute("ANALYZE work_items").unwrap();
    });
    let mut seen = std::collections::HashSet::new();
    let mut cursor: Option<String> = None;
    loop {
        let (page, next) = w.state.repository.work_page(&who, &Default::default(), cursor.as_deref(), 200).unwrap();
        for item in &page {
            assert!(seen.insert(item.id), "an item came back twice");
        }
        match next {
            Some(n) => cursor = Some(n),
            None => break,
        }
    }
    assert_eq!(seen.len(), 10_000);

    let plans: Vec<String> = w.db.run(move |c| {
        let sql = format!("EXPLAIN {}", scaffoldry_server::repository_work::PAGE_SQL);
        let first = c.query(&sql, &[&who, &Option::<chrono::DateTime<chrono::Utc>>::None, &Option::<chrono::DateTime<chrono::Utc>>::None, &Option::<Uuid>::None, &201i64]).unwrap();
        first.iter().map(|r| r.get::<_, String>(0)).collect()
    });
    let plan = plans.join("\n");
    assert!(!plan.contains("Seq Scan"), "the page reads the index: {plan}");
}

// 8. The proposal `review` items exclude the proposer and close on a decision.
#[tokio::test(flavor = "multi_thread")]
async fn proposal_review_items_exclude_the_proposer_and_close_on_a_decision() {
    const OWNER: &str = "einstein@physics.state.edu";
    const SECOND: &str = "bohr@physics.state.edu";
    const WORKSPACE: &str = "ws-physics-optics";
    let db = TestDb::new();
    let state = db.state();
    state.seed_demo().expect("seed");
    let app = build_app_with_state(state.clone()).expect("router");
    let admin = issue_test_token_and_user("jordan.lee@state.edu");
    let (owner, second) = (issue_test_token_and_user(OWNER), issue_test_token_and_user(SECOND));
    let (s, b) = send(&app, "POST", &format!("/api/v1/workspaces/{WORKSPACE}/collaborators"), &admin, Some(json!({ "eppn": SECOND, "role": "owner" }))).await;
    assert!(s.is_success(), "{b}");
    let slug = format!("revw-{}", &Uuid::new_v4().simple().to_string()[..8]);
    let manifest = json!({
        "slug": slug, "title": "Reviews", "description": "", "organization_code": "PHYS", "department": "physics", "workspace_id": WORKSPACE, "views": [],
        "tables": [{ "id": "t", "name": "T", "slug": "t", "primary_field": "title", "fields": [
            { "name": "title", "label": "Title", "field_type": "Text", "required": false, "ferpa_sensitive": false },
            { "name": "ssn", "label": "SSN", "field_type": "Text", "required": false, "ferpa_sensitive": true }
        ] }],
    });
    let (s, b) = send(&app, "POST", &format!("/api/v1/workspaces/{WORKSPACE}/apps"), &owner, Some(manifest)).await;
    assert!(s.is_success(), "{b}");
    let pid = b["proposal_id"].as_str().or_else(|| b["proposal"]["id"].as_str()).unwrap_or_else(|| panic!("a sensitive field makes the app a proposal: {b}")).to_string();
    let p = pid.clone();
    let items: Vec<(String, String, String)> = db.run(move |c| {
        c.query("SELECT person, kind, state FROM work_items WHERE proposal_id = $1::text::uuid ORDER BY person", &[&p]).unwrap().iter().map(|r| (r.get(0), r.get(1), r.get(2))).collect()
    });
    assert!(items.iter().any(|i| i.0 == SECOND && i.1 == "review" && i.2 == "open"), "the other owner reviews: {items:?}");
    assert!(!items.iter().any(|i| i.0 == OWNER && i.1 == "review"), "the proposer does not review their own change: {items:?}");
    assert!(items.iter().any(|i| i.0 == OWNER && i.1 == "waiting"), "the proposer waits: {items:?}");

    let call = |token: String, name: &'static str, args: Value| {
        let app = app.clone();
        async move { send(&app, "POST", "/api/mcp", &token, Some(json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": { "name": name, "arguments": args } }))).await.1 }
    };
    let digest = call(second.clone(), "get_proposal", json!({ "proposal_id": pid })).await["result"]["structuredContent"]["digest"].clone();
    let v = call(second.clone(), "decide_proposal", json!({ "proposal_id": pid, "decision": "approve", "rationale": "Reviewed", "digest": digest })).await;
    assert!(v["result"]["isError"] != true && v.get("error").is_none(), "{v}");
    let p = pid.clone();
    let open: i64 = db.run(move |c| c.query_one("SELECT count(*) FROM work_items WHERE proposal_id = $1::text::uuid AND state = 'open'", &[&p]).unwrap().get(0));
    assert_eq!(open, 0, "the decision closed every item of the proposal");
}

// 9. `list_my_work` and the route return the same ids for the same person.
#[tokio::test(flavor = "multi_thread")]
async fn the_tool_and_the_route_list_the_same_items() {
    let w = world().await;
    let (a, b) = (w.dept("Physics").await, w.dept("Chemistry").await);
    let rivera = person("rivera");
    let token = w.user(&rivera, &[&a, &b]).await;
    w.user(&w.sam.clone(), &[&a, &b]).await;
    for d in [&a, &b] {
        w.hold(&d.unit, &rivera).await;
        w.start(d, chair());
    }
    let mut from_route: Vec<String> = w.work(&token, "").await.iter().map(|i| i["id"].as_str().unwrap().to_string()).collect();
    let (_, v) = send(&w.app, "POST", "/api/mcp", &token, Some(json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": { "name": "list_my_work", "arguments": {} } }))).await;
    let mut from_tool: Vec<String> = v["result"]["structuredContent"]["items"].as_array().unwrap_or_else(|| panic!("{v}")).iter().map(|i| i["id"].as_str().unwrap().to_string()).collect();
    from_route.sort();
    from_tool.sort();
    assert_eq!(from_route.len(), 2);
    assert_eq!(from_route, from_tool);
}
