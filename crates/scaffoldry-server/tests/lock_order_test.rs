//! A request that holds a read lock must not ask for the same lock again while a writer waits. The standard library's
//! lock lets a waiting writer block every new reader, so a nested read plus a refresh that writes is a deadlock.
//! `member_role` reads workspaces, collaborators, the unit tree and organizations, so no caller may hold those across it.

mod common;

use common::TestDb;
use scaffoldry_server::state::AuthUser;
use std::sync::mpsc;
use std::time::Duration;

fn caller() -> AuthUser {
    AuthUser {
        eppn: "einstein@physics.state.edu".to_string(),
        name: "Albert Einstein".to_string(),
        role_title: "Professor".to_string(),
        affiliation: "faculty".to_string(),
        department: "physics".to_string(),
        view_as_by: None,
    }
}

/// The cost of a role lookup, printed and not asserted: a list of the demo workspaces, one caller, nothing contending.
#[test]
fn a_workspace_list_costs_a_few_database_reads_per_workspace() {
    let db = TestDb::new();
    let state = db.state();
    state.seed_demo().expect("seed");
    let me = caller();
    let count = scaffoldry_server::service::workspaces::list_workspaces(&me, &state).expect("a list").len();
    let started = std::time::Instant::now();
    for _ in 0..50 {
        scaffoldry_server::service::workspaces::list_workspaces(&me, &state).expect("a list");
    }
    eprintln!("list_workspaces: {:.1} ms for {} visible workspaces", started.elapsed().as_secs_f64() * 1000.0 / 50.0, count);
}

/// Runs readers that list workspaces while a writer takes each lock they use, and fails if they do not all finish.
#[test]
fn listing_workspaces_does_not_deadlock_against_a_writer() {
    let db = TestDb::new();
    let state = db.state();
    state.seed_demo().expect("seed");

    let (done_tx, done_rx) = mpsc::channel::<&'static str>();
    let mut handles = Vec::new();
    for _ in 0..6 {
        let (state, tx) = (state.clone(), done_tx.clone());
        handles.push(std::thread::spawn(move || {
            let me = caller();
            for _ in 0..150 {
                let _ = scaffoldry_server::service::workspaces::list_workspaces(&me, &state);
            }
            let _ = tx.send("reader");
        }));
    }
    {
        let (state, tx) = (state.clone(), done_tx.clone());
        handles.push(std::thread::spawn(move || {
            for _ in 0..600 {
                drop(state.workspaces.write().unwrap_or_else(|p| p.into_inner()));
                drop(state.collaborators.write().unwrap_or_else(|p| p.into_inner()));
                drop(state.tree.write().unwrap_or_else(|p| p.into_inner()));
                drop(state.organizations.write().unwrap_or_else(|p| p.into_inner()));
            }
            let _ = tx.send("writer");
        }));
    }
    drop(done_tx);
    for _ in 0..7 {
        done_rx.recv_timeout(Duration::from_secs(60)).expect("a reader or the writer is stuck: the locks deadlock");
    }
    for h in handles {
        h.join().unwrap();
    }
}

/// The roles in a list come from one read of the person's roles, groups and grants, not one read per workspace.
#[test]
fn the_database_reads_for_a_workspace_list_do_not_grow_with_the_number_of_workspaces() {
    use scaffoldry_server::state::WorkspaceRecord;
    let db = TestDb::new();
    let state = db.state();
    state.seed_demo().expect("seed");
    let me = caller();
    let measure = |state: &scaffoldry_server::state::SharedState| {
        let before = state.repository.statements();
        scaffoldry_server::service::workspaces::list_workspaces(&me, state).expect("a list");
        state.repository.statements() - before
    };
    let few = measure(&state);

    let now = chrono::Utc::now().to_rfc3339();
    for i in 0..40 {
        let id = format!("ws-bulk-{i}");
        let ws = WorkspaceRecord {
            id: id.clone(),
            name: id.clone(),
            code: id.to_uppercase(),
            organization: "INST".to_string(),
            department: "physics".to_string(),
            description: String::new(),
            icon: "x".to_string(),
            lead: "Lead".to_string(),
            visibility: "restricted".to_string(),
            coordinates: Default::default(),
            data_classification: "restricted".to_string(),
            created_at: now.clone(),
            organization_id: None,
        };
        state.workspaces.write().unwrap().insert(id.clone(), ws);
        state.collaborators.write().unwrap().insert(id, Vec::new());
    }
    let many = measure(&state);
    eprintln!("statements for a list: {few} with the demo workspaces, {many} with 40 more");
    assert!(many <= few + 2, "the reads grow with the workspaces: {few} then {many}");
}

fn register_apps(state: &scaffoldry_server::state::SharedState, count: usize, prefix: &str) {
    for i in 0..count {
        let manifest: scaffoldry_engine::AppManifest = serde_json::from_value(serde_json::json!({
            "slug": format!("{prefix}-{i}"), "title": format!("App {i}"), "description": "d", "organization_code": "PHYS",
            "department": "physics", "workspace_id": "ws-physics-optics", "views": [], "pages": [],
            "tables": [{ "id": "t", "name": "T", "slug": "t", "fields": [{ "name": "a", "label": "A", "field_type": "Text", "required": false, "ferpa_sensitive": false }] }]
        }))
        .unwrap();
        state.engine.write().unwrap().register_manifest(manifest).unwrap();
    }
}

/// The apps a person can reach come from one read of their roles, groups and grants, not one read per app.
#[test]
fn the_database_reads_for_the_apps_page_do_not_grow_with_the_number_of_apps() {
    let db = TestDb::new();
    let state = db.state();
    state.seed_demo().expect("seed");
    let me = caller();
    let measure = |state: &scaffoldry_server::state::SharedState| {
        let before = state.repository.statements();
        let apps = scaffoldry_server::service::my_apps::my_apps(&me, state);
        (state.repository.statements() - before, apps.len())
    };
    register_apps(&state, 3, "few");
    let (few, few_apps) = measure(&state);
    register_apps(&state, 30, "many");
    let (many, many_apps) = measure(&state);
    eprintln!("statements for my_apps: {few} for {few_apps} apps, {many} for {many_apps} apps");
    assert!(many_apps > few_apps, "the person reaches the added apps: {few_apps} then {many_apps}");
    assert!(many <= few + 2, "the reads grow with the apps: {few} then {many}");
}

/// A person's page in the admin console lists their workspace roles with one read, not one per workspace.
#[tokio::test]
async fn the_database_reads_for_a_persons_admin_page_do_not_grow_with_the_number_of_workspaces() {
    use axum::body::Body;
    use axum::http::Request;
    use http_body_util::BodyExt;
    use scaffoldry_server::service::identity::issue_test_token_and_user;
    use tower::ServiceExt;

    let db = TestDb::new();
    let state = db.state();
    state.seed_demo().expect("seed");
    let app = scaffoldry_server::build_app_with_state(state.clone()).expect("router");
    let send = |method: &'static str, uri: String, body: Option<serde_json::Value>| {
        let app = app.clone();
        async move {
            let b = Request::builder().method(method).uri(uri).header("authorization", format!("Bearer {}", issue_test_token_and_user("jordan.lee@state.edu")));
            let req = match body {
                Some(v) => b.header("content-type", "application/json").body(Body::from(v.to_string())).unwrap(),
                None => b.body(Body::empty()).unwrap(),
            };
            let resp = app.oneshot(req).await.unwrap();
            let status = resp.status();
            let bytes = resp.into_body().collect().await.unwrap().to_bytes();
            (status, serde_json::from_slice::<serde_json::Value>(&bytes).unwrap_or_default())
        }
    };
    let (s, user) = send("POST", "/api/v1/admin/users".to_string(), Some(serde_json::json!({
        "userName": "batch.person@state.edu", "name": "Batch Person", "email": "batch.person@state.edu",
        "affiliation": "faculty", "department": "physics", "title": "Lecturer", "reason": "Fixture"
    }))).await;
    assert_eq!(s, 201, "{user}");
    let id = user["user"]["id"].as_str().unwrap().to_string();

    let measure = || async {
        let before = state.repository.statements();
        let (s, body) = send("GET", format!("/api/v1/admin/users/{id}"), None).await;
        assert_eq!(s, 200, "{body}");
        state.repository.statements() - before
    };
    let few = measure().await;
    let now = chrono::Utc::now().to_rfc3339();
    for i in 0..40 {
        let wid = format!("ws-page-{i}");
        let ws = scaffoldry_server::state::WorkspaceRecord {
            id: wid.clone(), name: wid.clone(), code: wid.to_uppercase(), organization: "INST".to_string(), department: "physics".to_string(),
            description: String::new(), icon: "x".to_string(), lead: "Lead".to_string(), visibility: "restricted".to_string(),
            coordinates: Default::default(), data_classification: "restricted".to_string(), created_at: now.clone(), organization_id: None,
        };
        state.workspaces.write().unwrap().insert(wid.clone(), ws);
        state.collaborators.write().unwrap().insert(wid, Vec::new());
    }
    let many = measure().await;
    eprintln!("statements for a person's page: {few} then {many} with 40 more workspaces");
    assert!(many <= few + 2, "the reads grow with the workspaces: {few} then {many}");
}
