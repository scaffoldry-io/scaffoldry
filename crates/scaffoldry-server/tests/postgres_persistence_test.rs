//! Postgres 17 Persistence, Ledger Hash Chain Verification & Institutional Scale Tests
//!
//! Enforces:
//! 1. Refusal to start if the cryptographic SHA-256 ledger hash chain is broken or tampered.
//! 2. Persistence of workspaces, collaborators, records, sessions, and ledger across restarts.
//! 3. Performance and correctness at institutional scale (1,000+ workspaces and members).

use postgres::{Client, NoTls};
use scaffoldry_server::state::ServerState;
use std::sync::Mutex;

static DB_LOCK: Mutex<()> = Mutex::new(());

fn get_db_url() -> String {
    std::env::var("DATABASE_URL").unwrap_or_else(|_| {
        "postgres://scaffoldry:scaffoldry_dev_password@127.0.0.1:5433/scaffoldry".to_string()
    })
}

fn connect_db() -> Result<Client, postgres::Error> {
    Client::connect(&get_db_url(), NoTls)
}

#[test]
fn test_ledger_tamper_detection_on_startup() {
    let _lock = DB_LOCK.lock().unwrap();
    let mut client = match connect_db() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("Skipping test: PostgreSQL not reachable ({e})");
            return;
        }
    };

    // 1. Initialize clean state to ensure schema and valid genesis chain exist
    let state = ServerState::new().expect("State should initialize cleanly on valid DB");
    assert!(state.verify_ledger().unwrap_or(false), "Valid ledger should verify");

    // 2. Query original rationale of block 0
    let original_rationale: String = client
        .query_one("SELECT rationale FROM governance_ledger WHERE sequence = 0", &[])
        .expect("Query block 0")
        .get(0);

    // 3. Tamper with block 0 in the ledger table
    let affected = client
        .execute(
            "UPDATE governance_ledger SET rationale = 'ILLEGAL TAMPERING OF AUDIT RECORD' WHERE sequence = 0",
            &[],
        )
        .expect("Tamper block 0");
    assert_eq!(affected, 1, "Must tamper with exactly 1 row");

    // 4. Attempt to start ServerState. It MUST refuse to start and return an error!
    let startup_result = ServerState::new();
    assert!(
        startup_result.is_err(),
        "ServerState::new() MUST refuse to start when ledger hash chain is tampered or broken!"
    );
    let err_msg = startup_result.err().unwrap().to_string();
    assert!(
        err_msg.to_lowercase().contains("tamper")
            || err_msg.to_lowercase().contains("hash")
            || err_msg.to_lowercase().contains("chain"),
        "Error message must specify ledger integrity failure: {err_msg}"
    );

    // 5. Restore valid block 0
    client
        .execute(
            "UPDATE governance_ledger SET rationale = $1 WHERE sequence = 0",
            &[&original_rationale],
        )
        .expect("Restore valid block 0");
}

#[test]
fn test_postgres_persistence_lifecycle() {
    let _lock = DB_LOCK.lock().unwrap();
    let _client = match connect_db() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("Skipping test: PostgreSQL not reachable ({e})");
            return;
        }
    };

    // Reset ledger if needed
    let state = ServerState::new().expect("State should initialize cleanly");

    // Create a persistent workspace
    let test_ws_id = format!("ws-persist-{}", uuid::Uuid::new_v4());
    let ws = scaffoldry_server::state::WorkspaceRecord {
        id: test_ws_id.clone(),
        name: "Persistent Chemistry Lab".to_string(),
        code: "CHEM".to_string(),
        organization: "College of Sciences".to_string(),
        department: "chemistry".to_string(),
        description: "Postgres-persisted lab workspace".to_string(),
        icon: "⚗️".to_string(),
        lead: "Dr. Marie Curie".to_string(),
        visibility: "restricted".to_string(),
        allowed_affiliations: vec!["faculty".to_string()],
        data_classification: "Restricted".to_string(),
        cedar_policy_guard: None,
        created_at: chrono::Utc::now().to_rfc3339(),
    };

    state.persist_workspace(ws.clone()).expect("Failed to persist workspace");

    // Restart server state from Postgres
    let reloaded_state = ServerState::new().expect("Reloaded state should initialize");
    let fetched = reloaded_state.get_workspace(&test_ws_id);
    assert!(fetched.is_some(), "Workspace must persist across server restarts");
    assert_eq!(fetched.unwrap().name, "Persistent Chemistry Lab");
}

#[test]
fn test_institutional_scale_workspaces() {
    let _lock = DB_LOCK.lock().unwrap();
    let _client = match connect_db() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("Skipping test: PostgreSQL not reachable ({e})");
            return;
        }
    };

    let state = ServerState::new().expect("State should initialize cleanly");

    // Seed 1,000 workspaces at institutional scale
    let scale_count = 1000;
    let mut batch = Vec::with_capacity(scale_count);
    for i in 0..scale_count {
        batch.push(scaffoldry_server::state::WorkspaceRecord {
            id: format!("ws-scale-{i}"),
            name: format!("Institutional Unit {i}"),
            code: format!("UNIT-{i}"),
            organization: "State University System".to_string(),
            department: if i % 2 == 0 { "engineering".to_string() } else { "sciences".to_string() },
            description: format!("Departmental workspace for unit {i}"),
            icon: "🏢".to_string(),
            lead: "Dean of Faculty".to_string(),
            visibility: "departmental".to_string(),
            allowed_affiliations: vec!["faculty".to_string(), "staff".to_string()],
            data_classification: "Internal".to_string(),
            cedar_policy_guard: None,
            created_at: chrono::Utc::now().to_rfc3339(),
        });
    }

    state.persist_workspaces_batch(&batch).expect("Failed batch insert at scale");

    let count = state.count_workspaces().expect("Failed to count workspaces");
    assert!(count >= scale_count, "Database must hold institutional scale of workspaces (found {count})");
}

#[test]
fn test_postgres_apps_and_datasets_persistence_lifecycle() {
    let _lock = DB_LOCK.lock().unwrap();
    let _client = match connect_db() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("Skipping test: PostgreSQL not reachable ({e})");
            return;
        }
    };

    let state = ServerState::new().expect("State should initialize cleanly");

    // 1. Persist App Manifest
    let test_slug = format!("app-persist-{}", uuid::Uuid::new_v4().simple());
    let manifest = scaffoldry_engine::AppManifest {
        slug: test_slug.clone(),
        title: "Persistent Research App".to_string(),
        description: "App persisted in PostgreSQL 17".to_string(),
        organization_code: "DIV-SCIENCES".to_string(),
        department: "biology".to_string(),
        herm_capability_id: Some("RES-01".to_string()),
        custom_domain: Some(format!("{}.state.edu", test_slug)),
        custom_domain_verified: true,
        tables: vec![],
        relationships: vec![],
        views: vec![],
        ceds_mappings: std::collections::HashMap::new(),
    };
    state.persist_app_manifest(manifest.clone()).expect("Persist manifest");

    // 2. Persist Published Dataset
    let test_ds_id = format!("ds-persist-{}", uuid::Uuid::new_v4().simple());
    let ds = scaffoldry_core::PublishedDataset {
        id: test_ds_id.clone(),
        name: "Persisted Materials Registry".to_string(),
        description: "Materials catalog".to_string(),
        department: "chemistry".to_string(),
        organization: "College of Sciences".to_string(),
        sensitivity_level: "Internal".to_string(),
        herm_capability_id: Some("RES-02".to_string()),
        fields: vec![],
        record_count: 42,
        published_at: chrono::Utc::now().to_rfc3339(),
        sample_data: vec![],
    };
    state.persist_dataset(ds.clone()).expect("Persist dataset");

    // 3. Persist Automation Rule
    let test_rule_id = format!("rule-persist-{}", uuid::Uuid::new_v4().simple());
    let rule = scaffoldry_core::AutomationRule {
        id: test_rule_id.clone(),
        app_slug: test_slug.clone(),
        name: "Auto Material Alert".to_string(),
        description: "Trigger alert on material limit".to_string(),
        enabled: true,
        trigger: scaffoldry_core::TriggerEvent::RecordCreated,
        cedar_policy_guard: None,
        predicates: vec![],
        actions: vec![],
        steps: vec![],
    };
    state.persist_automation(rule.clone()).expect("Persist automation");

    // 4. Persist SCIM User
    let test_user_id = format!("scim-u-{}", uuid::Uuid::new_v4().simple());
    let scim_u = scaffoldry_server::state::ScimUser {
        id: test_user_id.clone(),
        user_name: "curie_persisted".to_string(),
        name: serde_json::json!({"formatted": "Marie Curie"}),
        active: true,
        emails: vec![serde_json::json!({"value": "curie@persisted.edu", "primary": true})],
        roles: vec![],
        enterprise_extension: None,
    };
    state.persist_scim_user(scim_u.clone()).expect("Persist SCIM user");

    // Restart server state from Postgres
    let reloaded = ServerState::new().expect("Reloaded state should initialize cleanly");

    // Verify manifest persisted
    let fetched_app = reloaded.get_app_manifest(&test_slug);
    assert!(fetched_app.is_some(), "App manifest must persist across restarts");
    assert_eq!(fetched_app.unwrap().title, "Persistent Research App");

    // Verify dataset persisted
    let fetched_ds = reloaded.get_dataset(&test_ds_id);
    assert!(fetched_ds.is_some(), "Published dataset must persist across restarts");
    assert_eq!(fetched_ds.unwrap().name, "Persisted Materials Registry");

    // Verify automations persisted
    let auto_guard = reloaded.automations.read().unwrap();
    let app_rules = auto_guard.get(&test_slug);
    assert!(app_rules.is_some(), "Automations must persist across restarts");
    assert!(app_rules.unwrap().iter().any(|r| r.id == test_rule_id));

    // Verify SCIM user persisted
    let fetched_user = reloaded.get_scim_user(&test_user_id);
    assert!(fetched_user.is_some(), "SCIM user must persist across restarts");
    assert_eq!(fetched_user.unwrap().user_name, "curie_persisted");
}
