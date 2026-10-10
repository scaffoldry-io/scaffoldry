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
    let _lock = DB_LOCK.lock().unwrap_or_else(|e| e.into_inner());
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

    // 3. Temporarily drop trigger to allow tampering in test
    let _ = client.execute("DROP TRIGGER IF EXISTS governance_ledger_no_change ON governance_ledger", &[]);
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

    // 5. Restore valid block 0 and restore trigger
    client
        .execute(
            "UPDATE governance_ledger SET rationale = $1 WHERE sequence = 0",
            &[&original_rationale],
        )
        .expect("Restore valid block 0");
    let _ = client.execute(
        "CREATE TRIGGER governance_ledger_no_change BEFORE UPDATE OR DELETE OR TRUNCATE ON governance_ledger FOR EACH STATEMENT EXECUTE FUNCTION governance_ledger_immutable()",
        &[],
    );
}

#[test]
fn test_postgres_persistence_lifecycle() {
    let _lock = DB_LOCK.lock().unwrap_or_else(|e| e.into_inner());
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
        created_at: chrono::Utc::now().to_rfc3339(),
        organization_id: Some(uuid::Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap()),
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
    let _lock = DB_LOCK.lock().unwrap_or_else(|e| e.into_inner());
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
                created_at: chrono::Utc::now().to_rfc3339(),
            organization_id: Some(uuid::Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap()),
        });
    }

    state.persist_workspaces_batch(&batch).expect("Failed batch insert at scale");

    let count = state.count_workspaces().expect("Failed to count workspaces");
    assert!(count >= scale_count, "Database must hold institutional scale of workspaces (found {count})");
}

#[test]
fn test_postgres_apps_and_datasets_persistence_lifecycle() {
    let _lock = DB_LOCK.lock().unwrap_or_else(|e| e.into_inner());
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
        workspace_id: None,
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
        admin_hold: false,
        emails: vec![serde_json::json!({"value": "curie@persisted.edu", "primary": true})],
        roles: vec![],
        enterprise_extension: None,
        title: None,
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

#[test]
fn test_schema_migrations_runs_each_file_once() {
    let _lock = DB_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut client = match connect_db() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("Skipping test: PostgreSQL not reachable ({e})");
            return;
        }
    };

    // Connect twice
    let repo1 = scaffoldry_server::repository::PostgresRepository::connect(Some(&get_db_url()));
    assert!(repo1.is_ok(), "First connect should succeed");
    let repo2 = scaffoldry_server::repository::PostgresRepository::connect(Some(&get_db_url()));
    assert!(repo2.is_ok(), "Second connect should succeed");

    // Verify schema_migrations has one row per file, not two
    let dup_rows = client
        .query(
            "SELECT filename, COUNT(*) FROM schema_migrations GROUP BY filename HAVING COUNT(*) > 1",
            &[],
        )
        .expect("Query duplicate migrations");
    assert!(dup_rows.is_empty(), "schema_migrations must not contain duplicate entries: found {dup_rows:?}");

    let total_rows = client
        .query("SELECT COUNT(*) FROM schema_migrations", &[])
        .expect("Query total migrations");
    let total_count: i64 = total_rows[0].get(0);
    assert!(total_count >= 8, "All 8 migrations must be recorded in schema_migrations (found {total_count})");
}

#[test]
fn test_ledger_append_only_trigger_blocks_delete() {
    let _lock = DB_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut client = match connect_db() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("Skipping test: PostgreSQL not reachable ({e})");
            return;
        }
    };

    let _repo = scaffoldry_server::repository::PostgresRepository::connect(Some(&get_db_url()))
        .expect("repo connect");

    // DELETE on governance_ledger MUST raise exception from trigger
    let delete_res = client.execute("DELETE FROM governance_ledger WHERE sequence = 0", &[]);
    assert!(delete_res.is_err(), "DELETE FROM governance_ledger must be rejected by append-only trigger");
    let err = delete_res.err().unwrap();
    let err_str = format!("{err:?}");
    assert!(
        err_str.contains("append-only"),
        "Error message must state ledger is append-only: {err_str}"
    );
}

#[test]
fn test_workspace_service_persists_to_second_server_state() {
    let _lock = DB_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _client = match connect_db() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("Skipping test: PostgreSQL not reachable ({e})");
            return;
        }
    };

    let state1 = std::sync::Arc::new(ServerState::new().expect("State1 should initialize"));

    let caller = scaffoldry_server::state::AuthUser {
        eppn: "service.admin@state.edu".to_string(),
        name: "Service Admin".to_string(),
        role_title: "Central Admin".to_string(),
        affiliation: "central_admin".to_string(),
        department: "Central IT".to_string(),
    };

    let payload = scaffoldry_server::service::workspaces::CreateWorkspacePayload {
        name: "Service Persisted Genomics Lab".to_string(),
        code: format!("GENO_{}", &uuid::Uuid::new_v4().simple().to_string()[..6]),
        organization: Some("College of Sciences".to_string()),
        department: Some("Biology".to_string()),
        description: Some("Created via service and read back via second ServerState".to_string()),
        icon: Some("🧬".to_string()),
        visibility: Some("restricted".to_string()),
        allowed_affiliations: Some(vec!["faculty".to_string()]),
        data_classification: Some("Restricted".to_string()),
        organization_id: None,
    };

    let created = scaffoldry_server::service::workspaces::create_workspace(
        &caller,
        payload.clone(),
        &state1,
    ).expect("create_workspace via service");
    let test_ws_id = created.id.clone();

    // Build a second ServerState and read the workspace back
    let state2 = ServerState::new().expect("State2 should initialize cleanly");
    let read_back = state2.get_workspace(&test_ws_id);
    assert!(read_back.is_some(), "Second ServerState must read back the workspace created through service");
    let ws_record = read_back.unwrap();
    assert_eq!(ws_record.name, "Service Persisted Genomics Lab");
    assert_eq!(ws_record.code, payload.code);
    assert_eq!(ws_record.visibility, "restricted");
}

#[test]
fn test_empty_ledger_on_existing_database_fails_startup_with_tamper_detected() {
    let _lock = DB_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut client = match connect_db() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("Skipping test: PostgreSQL not reachable ({e})");
            return;
        }
    };

    // Ensure migrations exist
    let _repo = scaffoldry_server::repository::PostgresRepository::connect(Some(&get_db_url()))
        .expect("initial connect");

    // Drop trigger to empty the table
    client.execute("DROP TRIGGER IF EXISTS governance_ledger_no_change ON governance_ledger", &[])
        .expect("drop trigger");

    // Back up existing ledger rows
    let rows = client.query(
        "SELECT sequence, entry_hash, previous_hash, timestamp_iso, principal,                 organization_code, app_slug, decision_type, oscal_control_id, rationale, payload, payload_hash          FROM governance_ledger ORDER BY sequence ASC",
        &[],
    ).expect("query ledger backup");

    // Empty the ledger table
    client.execute("DELETE FROM governance_ledger", &[]).expect("empty ledger table");

    // Re-enable trigger
    client.execute(
        "CREATE TRIGGER governance_ledger_no_change BEFORE UPDATE OR DELETE OR TRUNCATE ON governance_ledger FOR EACH STATEMENT EXECUTE FUNCTION governance_ledger_immutable()",
        &[],
    ).expect("re-enable trigger");

    // Attempting to connect must now fail with TamperDetected!
    let conn_res = scaffoldry_server::repository::PostgresRepository::connect(Some(&get_db_url()));
    assert!(
        conn_res.is_err(),
        "Existing database with empty ledger MUST fail connect with TamperDetected"
    );

    match conn_res {
        Err(scaffoldry_server::repository::RepositoryError::TamperDetected(msg)) => {
            assert!(
                msg.contains("Empty ledger on existing database installation"),
                "Expected TamperDetected on empty existing ledger, got: {msg}"
            );
        }
        other => panic!("Expected RepositoryError::TamperDetected, got: {:?}", other),
    }

    // Restore ledger rows to leave database in valid state
    client.execute("DROP TRIGGER IF EXISTS governance_ledger_no_change ON governance_ledger", &[])
        .expect("drop trigger for restore");

    for r in rows {
        let seq: i64 = r.get(0);
        let entry_hash: String = r.get(1);
        let prev_hash: String = r.get(2);
        let ts: String = r.get(3);
        let princ: String = r.get(4);
        let org: String = r.get(5);
        let app: Option<String> = r.get(6);
        let dec: String = r.get(7);
        let oscal: String = r.get(8);
        let rat: String = r.get(9);
        let payload: serde_json::Value = r.get(10);
        let p_hash: String = r.get(11);

        client.execute(
            "INSERT INTO governance_ledger (sequence, entry_hash, previous_hash, timestamp_iso,                     principal, organization_code, app_slug, decision_type, oscal_control_id, rationale,                     payload, payload_hash)              VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)",
            &[&seq, &entry_hash, &prev_hash, &ts, &princ, &org, &app, &dec, &oscal, &rat, &payload, &p_hash],
        ).expect("restore ledger row");
    }

    client.execute(
        "CREATE TRIGGER governance_ledger_no_change BEFORE UPDATE OR DELETE OR TRUNCATE ON governance_ledger FOR EACH STATEMENT EXECUTE FUNCTION governance_ledger_immutable()",
        &[],
    ).expect("restore trigger");
}

#[test]
fn test_concurrent_worker_pool_executes_parallel_jobs() {
    let _lock = DB_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let repo = match scaffoldry_server::repository::PostgresRepository::connect(Some(&get_db_url())) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("Skipping test: PostgreSQL not reachable ({e})");
            return;
        }
    };

    let start = std::time::Instant::now();
    let repo1 = repo.clone();
    let repo2 = repo.clone();

    let t1 = std::thread::spawn(move || {
        repo1.with_client(|client| {
            client.execute("SELECT pg_sleep(0.5)", &[])?;
            Ok(())
        })
    });
    let t2 = std::thread::spawn(move || {
        repo2.with_client(|client| {
            client.execute("SELECT pg_sleep(0.5)", &[])?;
            Ok(())
        })
    });

    t1.join().unwrap().expect("worker job 1");
    t2.join().unwrap().expect("worker job 2");

    let elapsed = start.elapsed();
    assert!(
        elapsed < std::time::Duration::from_millis(900),
        "Two 0.5s queries on multi-connection worker pool must run in parallel and finish under 0.9s, took {elapsed:?}"
    );
}

#[test]
fn test_concurrent_ledger_appends_across_twenty_threads() {
    let _lock = DB_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let repo = match scaffoldry_server::repository::PostgresRepository::connect(Some(&get_db_url())) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("Skipping test: PostgreSQL not reachable ({e})");
            return;
        }
    };

    let initial_entries = repo.get_ledger().expect("get ledger");
    let initial_count = initial_entries.len();

    let mut handles = Vec::new();
    for i in 0..20 {
        let repo_clone = repo.clone();
        handles.push(std::thread::spawn(move || {
            repo_clone.append_ledger_decision(scaffoldry_server::repository::AppendDecisionParams {
                principal: format!("worker_{i}@state.edu"),
                organization_code: "DIV-PARALLEL".to_string(),
                app_slug: None,
                decision_type: scaffoldry_core::DecisionType::PolicyRevision,
                oscal_control_id: "AU-02".to_string(),
                rationale: format!("Concurrent ledger append from thread {i}"),
                payload: serde_json::json!({ "thread_index": i }),
            })
        }));
    }

    for h in handles {
        h.join().unwrap().expect("append decision");
    }

    // Verify the ledger hash chain
    repo.verify_and_initialize_ledger(false).expect("Ledger must verify cleanly");

    let entries = repo.get_ledger().expect("get ledger");
    assert_eq!(
        entries.len(),
        initial_count + 20,
        "Ledger must hold exactly 20 more entries"
    );

    // Verify sequences have no gaps and hash chain is continuous
    for w in entries.windows(2) {
        assert_eq!(
            w[0].sequence + 1,
            w[1].sequence,
            "Ledger sequence numbers must have no gap"
        );
        assert_eq!(
            w[0].entry_hash,
            w[1].previous_hash,
            "Ledger hash chain must be unbroken"
        );
    }
}
