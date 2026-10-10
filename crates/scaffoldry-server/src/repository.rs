//! PostgreSQL 17 Repository Layer with Cryptographic Ledger Hash Chain Verification
//! Enforces:
//! - NIST OSCAL 1.1.2 AU-02 cryptographic immutable audit ledger verification on startup.
//! - Relational persistence for Workspaces, Collaborators, Records, Sessions, and Ledger.
//! - Institutional scale performance (1,000+ workspaces and collaborators).
//! - Dedicated background worker architecture to isolate synchronous postgres driver from Tokio runtimes.

use crate::state::{
    AuthSession, AuthUser, CollaboratorRecord, DatasetRecord, OrganizationNode, RoleRow, ScimGroup,
    ScimUser, WorkspaceRecord,
};
use chrono::Utc;
use postgres::{Client, NoTls};
use scaffoldry_core::{
    AutomationRule, DatasetRelationship, DecisionType, LedgerEntry, LedgerError, ProcessInstance,
    ProcessStatus, PublishedDataset, GENESIS_PREVIOUS_HASH,
};
use scaffoldry_engine::AppManifest;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::mpsc::{channel, sync_channel, Sender};

const SCHEMA_0001: &str = include_str!("../../scaffoldry-core/migrations/0001_initial_schema.sql");
const SCHEMA_0002: &str = include_str!("../../scaffoldry-core/migrations/0002_workspaces_and_ledger.sql");
const SCHEMA_0003: &str = include_str!("../../scaffoldry-core/migrations/0003_persist_apps_and_datasets.sql");
const SCHEMA_0004: &str = include_str!("../../scaffoldry-core/migrations/0004_organization_scope.sql");
const SCHEMA_0005: &str = include_str!("../../scaffoldry-core/migrations/0005_process_instances.sql");
const SCHEMA_0006: &str = include_str!("../../scaffoldry-core/migrations/0006_role_source.sql");
const SCHEMA_0008: &str = include_str!("../../scaffoldry-core/migrations/0008_schema_migrations.sql");
const SCHEMA_0009: &str = include_str!("../../scaffoldry-core/migrations/0009_ledger_append_only.sql");
const SCHEMA_0010: &str = include_str!("../../scaffoldry-core/migrations/0010_app_workspace.sql");
const SCHEMA_0012: &str = include_str!("../../scaffoldry-core/migrations/0012_api_tokens.sql");
const SCHEMA_0013: &str = include_str!("../../scaffoldry-core/migrations/0013_platform_settings.sql");
const SCHEMA_0026: &str = include_str!("../../scaffoldry-core/migrations/0026_jobs.sql");

#[derive(Debug, thiserror::Error)]
pub enum RepositoryError {
    #[error("Database error: {0}")]
    Db(#[from] postgres::Error),
    #[error("Ledger verification error: {0}")]
    Ledger(#[from] LedgerError),
    #[error("Cryptographic tamper detected in ledger: {0}")]
    TamperDetected(String),
    #[error("Serialization error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("Not found: {0}")]
    NotFound(String),
}

type WorkerJob = Box<dyn FnOnce(&mut Client) + Send>;

#[derive(Clone)]
pub struct PostgresRepository {
    worker_tx: Sender<WorkerJob>,
    worker_count: usize,
    db_url: std::sync::Arc<String>,
}

impl std::fmt::Debug for PostgresRepository {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PostgresRepository").finish()
    }
}

impl PostgresRepository {
    pub fn connect(url: Option<&str>) -> Result<Self, RepositoryError> {
        let db_url = url
            .map(str::to_string)
            .or_else(|| std::env::var("DATABASE_URL").ok())
            .unwrap_or_else(|| {
                "postgres://scaffoldry:scaffoldry_dev_password@127.0.0.1:5433/scaffoldry".to_string()
            });

        let num_workers: usize = std::env::var("SCAFFOLDRY_DB_WORKERS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(8);

        let (worker_tx, worker_rx) = channel::<WorkerJob>();
        let worker_rx = std::sync::Arc::new(std::sync::Mutex::new(worker_rx));
        let (init_tx, init_rx) = sync_channel::<Result<(), RepositoryError>>(1);

        let db_url_0 = db_url.clone();
        let init_tx_0 = init_tx;
        let rx_0 = worker_rx.clone();

        std::thread::Builder::new()
            .name("scaffoldry-db-worker-0".to_string())
            .spawn(move || {
                let mut client = match Client::connect(&db_url_0, NoTls) {
                    Ok(c) => c,
                    Err(e) => {
                        let _ = init_tx_0.send(Err(RepositoryError::Db(e)));
                        return;
                    }
                };

                // Run migrations with advisory lock to prevent concurrent DDL deadlocks
                let mig_res = (|| -> Result<bool, postgres::Error> {
                    client.execute("SELECT pg_advisory_lock(742199)", &[])?;
                    let res = (|| -> Result<bool, postgres::Error> {
                        client.batch_execute(SCHEMA_0008)?;

                        let rows = client.query("SELECT filename FROM schema_migrations", &[])?;
                        let applied: std::collections::HashSet<String> =
                            rows.into_iter().map(|r| r.get(0)).collect();

                        let migrations: [(&str, &str); 12] = [
                            ("0001_initial_schema.sql", SCHEMA_0001),
                            ("0002_workspaces_and_ledger.sql", SCHEMA_0002),
                            ("0003_persist_apps_and_datasets.sql", SCHEMA_0003),
                            ("0004_organization_scope.sql", SCHEMA_0004),
                            ("0005_process_instances.sql", SCHEMA_0005),
                            ("0006_role_source.sql", SCHEMA_0006),
                            ("0008_schema_migrations.sql", SCHEMA_0008),
                            ("0009_ledger_append_only.sql", SCHEMA_0009),
                            ("0010_app_workspace.sql", SCHEMA_0010),
                            ("0012_api_tokens.sql", SCHEMA_0012),
                            ("0013_platform_settings.sql", SCHEMA_0013),
                            ("0026_jobs.sql", SCHEMA_0026),
                        ];

                        let mut fresh_install = false;
                        for (filename, sql) in migrations {
                            if applied.contains(filename) {
                                continue;
                            }
                            let mut tx = client.transaction()?;
                            tx.batch_execute(sql)?;
                            tx.execute(
                                "INSERT INTO schema_migrations (filename) VALUES ($1) ON CONFLICT DO NOTHING",
                                &[&filename],
                            )?;
                            tx.commit()?;
                            if filename == "0002_workspaces_and_ledger.sql" {
                                fresh_install = true;
                            }
                        }

                        let _ = client.execute("DELETE FROM auth_sessions WHERE token LIKE 'sct_%'", &[]);
                        Ok(fresh_install)
                    })();
                    res
                })();

                // Verify (and on a fresh install seed) the ledger before the lock is released.
                // Otherwise a concurrent boot sees the migrated schema, treats it as an existing
                // install, and reports the not yet seeded ledger as tampering.
                let verified = match mig_res {
                    Ok(fresh_install) => Self::verify_client_ledger(&mut client, fresh_install),
                    Err(e) => Err(RepositoryError::Db(e)),
                };
                let _ = client.execute("SELECT pg_advisory_unlock(742199)", &[]);
                if let Err(e) = verified {
                    let _ = init_tx_0.send(Err(e));
                    return;
                }

                let _ = init_tx_0.send(Ok(()));

                loop {
                    let job = {
                        let rx = match rx_0.lock() {
                            Ok(g) => g,
                            Err(_) => break,
                        };
                        match rx.recv() {
                            Ok(j) => j,
                            Err(_) => break,
                        }
                    };
                    job(&mut client);
                }
            })
            .map_err(|e| RepositoryError::NotFound(format!("Failed to spawn db worker thread 0: {e}")))?;

        init_rx
            .recv()
            .map_err(|_| RepositoryError::NotFound("Worker init hung up".to_string()))??;

        for w_idx in 1..num_workers {
            let db_url_w = db_url.clone();
            let rx_w = worker_rx.clone();
            std::thread::Builder::new()
                .name(format!("scaffoldry-db-worker-{w_idx}"))
                .spawn(move || {
                    let mut client = match Client::connect(&db_url_w, NoTls) {
                        Ok(c) => c,
                        Err(e) => {
                            eprintln!("Warning: db worker {w_idx} connection failed: {e}");
                            return;
                        }
                    };
                    loop {
                        let job = {
                            let rx = match rx_w.lock() {
                                Ok(g) => g,
                                Err(_) => break,
                            };
                            match rx.recv() {
                                Ok(j) => j,
                                Err(_) => break,
                            }
                        };
                        job(&mut client);
                    }
                })
                .map_err(|e| RepositoryError::NotFound(format!("Failed to spawn db worker thread {w_idx}: {e}")))?;
        }

        let repo = Self {
            worker_tx,
            worker_count: num_workers,
            db_url: std::sync::Arc::new(db_url),
        };
        Ok(repo)
    }

    pub fn worker_count(&self) -> usize {
        self.worker_count
    }

    pub fn get_applied_migrations(&self) -> Result<Vec<String>, RepositoryError> {
        self.with_client(|client| {
            let rows = client.query("SELECT filename FROM schema_migrations ORDER BY filename ASC", &[])?;
            Ok(rows.into_iter().map(|r| r.get::<_, String>(0)).collect())
        })
    }

    /// A new connection that the caller owns. Job workers use one each, so a long job never
    /// holds up the request workers.
    pub fn dedicated_client(&self) -> Result<Client, RepositoryError> {
        Ok(Client::connect(&self.db_url, NoTls)?)
    }

    pub fn with_client<R: Send + 'static>(
        &self,
        f: impl FnOnce(&mut Client) -> Result<R, RepositoryError> + Send + 'static,
    ) -> Result<R, RepositoryError> {
        let (tx, rx) = sync_channel(1);
        self.worker_tx
            .send(Box::new(move |client| {
                let res = f(client);
                let _ = tx.send(res);
            }))
            .map_err(|_| RepositoryError::NotFound("Postgres worker channel closed".to_string()))?;
        rx.recv()
            .map_err(|_| RepositoryError::NotFound("Postgres worker response lost".to_string()))?
    }

    fn verify_client_ledger(client: &mut Client, fresh_install: bool) -> Result<(), RepositoryError> {
        let r = client.query(
            "SELECT sequence, entry_hash, previous_hash, timestamp_iso, principal,                     organization_code, app_slug, decision_type, oscal_control_id, rationale, payload,                     COALESCE(payload_hash, '')              FROM governance_ledger ORDER BY sequence ASC",
            &[],
        )?;
        let rows = r.into_iter().map(|row| {
            let seq: i64 = row.get(0);
            let entry_hash: String = row.get(1);
            let prev_hash: String = row.get(2);
            let timestamp_iso: String = row.get(3);
            let principal: String = row.get(4);
            let organization_code: String = row.get(5);
            let app_slug: Option<String> = row.get(6);
            let decision_type_str: String = row.get(7);
            let oscal_control_id: String = row.get(8);
            let rationale: String = row.get(9);
            let payload: Value = row.get(10);
            let raw_payload_hash: String = row.get(11);
            (seq, entry_hash, prev_hash, timestamp_iso, principal, organization_code, app_slug, decision_type_str, oscal_control_id, rationale, payload, raw_payload_hash)
        }).collect::<Vec<_>>();

        if rows.is_empty() {
            if fresh_install {
                Self::seed_client_genesis_ledger(client)?;
                return Ok(());
            } else {
                return Err(RepositoryError::TamperDetected(
                    "Empty ledger on existing database installation".to_string(),
                ));
            }
        }

        let mut prev_hash = GENESIS_PREVIOUS_HASH.to_string();

        for (seq, entry_hash, recorded_prev_hash, timestamp_iso, principal, organization_code, app_slug, decision_type_str, oscal_control_id, rationale, payload, raw_payload_hash) in rows {
            let sequence = seq as u64;

            // Verify chain continuity
            if sequence == 0 {
                if recorded_prev_hash != GENESIS_PREVIOUS_HASH {
                    return Err(RepositoryError::TamperDetected(format!(
                        "Genesis block (sequence 0) has invalid previous hash: {}",
                        recorded_prev_hash
                    )));
                }
            } else if recorded_prev_hash != prev_hash {
                return Err(RepositoryError::TamperDetected(format!(
                    "Ledger hash chain broken at sequence {}: expected previous hash {}, got {}",
                    sequence, prev_hash, recorded_prev_hash
                )));
            }

            let decision_type = parse_decision_type(&decision_type_str)?;
            let payload_hash = if raw_payload_hash.is_empty() {
                LedgerEntry::compute_payload_hash(&payload)
            } else {
                raw_payload_hash
            };

            let entry = LedgerEntry {
                sequence,
                timestamp_iso,
                previous_hash: recorded_prev_hash,
                principal,
                organization_code,
                app_slug,
                decision_type,
                oscal_control_id,
                rationale,
                payload_hash,
                entry_hash: entry_hash.clone(),
            };

            // Verify SHA-256 block hash integrity
            if let Err(e) = entry.verify() {
                return Err(RepositoryError::TamperDetected(format!(
                    "Cryptographic tamper detected at sequence {}: {}",
                    sequence, e
                )));
            }

            prev_hash = entry_hash;
        }

        Ok(())
    }

    /// Verifies the cryptographic SHA-256 block chain from sequence 0 to N.
    /// Fails closed if any block hash mismatch or chain discontinuity is found.
    pub fn verify_and_initialize_ledger(&self, fresh_install: bool) -> Result<(), RepositoryError> {
        self.with_client(move |client| {
            Self::verify_client_ledger(client, fresh_install)
        })
    }

    fn seed_client_genesis_ledger(client: &mut Client) -> Result<(), RepositoryError> {
        let now = Utc::now().to_rfc3339();

        let genesis = LedgerEntry::new(scaffoldry_core::NewLedgerEntryParams {
            sequence: 0,
            timestamp_iso: now.clone(),
            previous_hash: GENESIS_PREVIOUS_HASH.to_string(),
            principal: "system".to_string(),
            organization_code: "DIV-SECURITY-CENTRAL".to_string(),
            app_slug: None,
            decision_type: DecisionType::PolicyRevision,
            oscal_control_id: "PL-02".to_string(),
            rationale: "Ledger initialized".to_string(),
            payload: &serde_json::json!({ "status": "genesis" }),
        });

        let entry_1 = LedgerEntry::new(scaffoldry_core::NewLedgerEntryParams {
            sequence: 1,
            timestamp_iso: now.clone(),
            previous_hash: genesis.entry_hash.clone(),
            principal: "prof.curie@science.state.edu".to_string(),
            organization_code: "DIV-SCIENCES".to_string(),
            app_slug: Some("biology-lab-inventory".to_string()),
            decision_type: DecisionType::AppPublished,
            oscal_control_id: "CM-03".to_string(),
            rationale: "Initial publication of Biology Research Chemical Inventory".to_string(),
            payload: &serde_json::json!({"status": "Published", "domain": "inventory.biology.state.edu"}),
        });

        let entry_2 = LedgerEntry::new(scaffoldry_core::NewLedgerEntryParams {
            sequence: 2,
            timestamp_iso: now.clone(),
            previous_hash: entry_1.entry_hash.clone(),
            principal: "prof.curie@science.state.edu".to_string(),
            organization_code: "DIV-SCIENCES".to_string(),
            app_slug: Some("biology-lab-inventory".to_string()),
            decision_type: DecisionType::VanityDnsBound,
            oscal_control_id: "SC-07".to_string(),
            rationale: "Vanity DNS alias bound with sovereign gateway validation".to_string(),
            payload: &serde_json::json!({"domain": "inventory.biology.state.edu", "verified": true}),
        });

        let entry_3 = LedgerEntry::new(scaffoldry_core::NewLedgerEntryParams {
            sequence: 3,
            timestamp_iso: now.clone(),
            previous_hash: entry_2.entry_hash.clone(),
            principal: "dr.watson@science.state.edu".to_string(),
            organization_code: "DIV-COMPLIANCE".to_string(),
            app_slug: Some("physics-admissions-review".to_string()),
            decision_type: DecisionType::WorkflowRuleApproved,
            oscal_control_id: "AC-03".to_string(),
            rationale: "FERPA compliance and GPA threshold auto-admit rule approved".to_string(),
            payload: &serde_json::json!({"rule": "auto-physics-honors-admit", "policy": "policy-ferpa-34cfr99"}),
        });

        let entry_4 = LedgerEntry::new(scaffoldry_core::NewLedgerEntryParams {
            sequence: 4,
            timestamp_iso: now,
            previous_hash: entry_3.entry_hash.clone(),
            principal: "dr.watson@science.state.edu".to_string(),
            organization_code: "DIV-COMPLIANCE".to_string(),
            app_slug: Some("compliance-ferpa-requests".to_string()),
            decision_type: DecisionType::StatutoryAttestation,
            oscal_control_id: "AU-02".to_string(),
            rationale: "Institutional statutory compliance attestation for 34 CFR Part 99".to_string(),
            payload: &serde_json::json!({"framework": "FERPA", "standard": "34 CFR Part 99"}),
        });

        let entries = vec![
            (genesis, serde_json::json!({"status": "genesis"})),
            (entry_1, serde_json::json!({"status": "Published", "domain": "inventory.biology.state.edu"})),
            (entry_2, serde_json::json!({"domain": "inventory.biology.state.edu", "verified": true})),
            (entry_3, serde_json::json!({"rule": "auto-physics-honors-admit", "policy": "policy-ferpa-34cfr99"})),
            (entry_4, serde_json::json!({"framework": "FERPA", "standard": "34 CFR Part 99"})),
        ];

        for (e, payload) in entries {
            let seq = e.sequence as i64;
            let dec_str = format!("{:?}", e.decision_type);
            client.execute(
                "INSERT INTO governance_ledger (sequence, entry_hash, previous_hash, timestamp_iso,                         principal, organization_code, app_slug, decision_type, oscal_control_id,                         rationale, payload_hash, payload)                  VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)",
                &[
                    &seq,
                    &e.entry_hash,
                    &e.previous_hash,
                    &e.timestamp_iso,
                    &e.principal,
                    &e.organization_code,
                    &e.app_slug,
                    &dec_str,
                    &e.oscal_control_id,
                    &e.rationale,
                    &e.payload_hash,
                    &payload,
                ],
            )?;
        }
        Ok(())
    }

    pub fn seed_genesis_ledger(&self) -> Result<(), RepositoryError> {
        self.with_client(|client| {
            Self::seed_client_genesis_ledger(client)
        })
    }
    // -------------------------------------------------------------------------
    // WORKSPACES
    // -------------------------------------------------------------------------

    pub fn list_workspaces(&self) -> Result<Vec<WorkspaceRecord>, RepositoryError> {
        self.with_client(|client| {
            let rows = client.query(
                "SELECT id, name, code, organization, department, description, icon, lead, \
                        visibility, allowed_affiliations, data_classification, cedar_policy_guard, \
                        created_at::text, organization_id \
                 FROM workspaces ORDER BY created_at ASC",
                &[],
            )?;

            let mut workspaces = Vec::new();
            for r in rows {
                let allowed_affiliations: Value = r.get(9);
                let affs: Vec<String> = serde_json::from_value(allowed_affiliations).unwrap_or_default();
                workspaces.push(WorkspaceRecord {
                    id: r.get(0),
                    name: r.get(1),
                    code: r.get(2),
                    organization: r.get(3),
                    department: r.get(4),
                    description: r.get(5),
                    icon: r.get(6),
                    lead: r.get(7),
                    visibility: r.get(8),
                    allowed_affiliations: affs,
                    data_classification: r.get(10),
                    cedar_policy_guard: r.get(11),
                    created_at: r.get(12),
                    organization_id: r.get(13),
                });
            }
            Ok(workspaces)
        })
    }

    pub fn get_workspace(&self, id: &str) -> Result<Option<WorkspaceRecord>, RepositoryError> {
        let id_str = id.to_string();
        self.with_client(move |client| {
            let row = client.query_opt(
                "SELECT id, name, code, organization, department, description, icon, lead, \
                        visibility, allowed_affiliations, data_classification, cedar_policy_guard, \
                        created_at::text, organization_id \
                 FROM workspaces WHERE id = $1",
                &[&id_str],
            )?;

            Ok(row.map(|r| {
                let allowed_affiliations: Value = r.get(9);
                let affs: Vec<String> = serde_json::from_value(allowed_affiliations).unwrap_or_default();
                WorkspaceRecord {
                    id: r.get(0),
                    name: r.get(1),
                    code: r.get(2),
                    organization: r.get(3),
                    department: r.get(4),
                    description: r.get(5),
                    icon: r.get(6),
                    lead: r.get(7),
                    visibility: r.get(8),
                    allowed_affiliations: affs,
                    data_classification: r.get(10),
                    cedar_policy_guard: r.get(11),
                    created_at: r.get(12),
                    organization_id: r.get(13),
                }
            }))
        })
    }

    pub fn upsert_workspace(&self, ws: &WorkspaceRecord) -> Result<(), RepositoryError> {
        let ws = ws.clone();
        self.with_client(move |client| {
            let affs_json = serde_json::to_value(&ws.allowed_affiliations)?;
            client.execute(
                "INSERT INTO workspaces (id, name, code, organization, department, description, \
                        icon, lead, visibility, allowed_affiliations, data_classification, cedar_policy_guard, organization_id) \
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13) \
                 ON CONFLICT (id) DO UPDATE SET \
                    name = EXCLUDED.name, \
                    code = EXCLUDED.code, \
                    organization = EXCLUDED.organization, \
                    department = EXCLUDED.department, \
                    description = EXCLUDED.description, \
                    icon = EXCLUDED.icon, \
                    lead = EXCLUDED.lead, \
                    visibility = EXCLUDED.visibility, \
                    allowed_affiliations = EXCLUDED.allowed_affiliations, \
                    data_classification = EXCLUDED.data_classification, \
                    cedar_policy_guard = EXCLUDED.cedar_policy_guard, \
                    organization_id = EXCLUDED.organization_id",
                &[
                    &ws.id,
                    &ws.name,
                    &ws.code,
                    &ws.organization,
                    &ws.department,
                    &ws.description,
                    &ws.icon,
                    &ws.lead,
                    &ws.visibility,
                    &affs_json,
                    &ws.data_classification,
                    &ws.cedar_policy_guard,
                    &ws.organization_id,
                ],
            )?;
            Ok(())
        })
    }

    pub fn persist_workspaces_batch(&self, batch: &[WorkspaceRecord]) -> Result<(), RepositoryError> {
        let batch = batch.to_vec();
        self.with_client(move |client| {
            let mut tx = client.transaction()?;
            for ws in batch {
                let affs_json = serde_json::to_value(&ws.allowed_affiliations)?;
                tx.execute(
                    "INSERT INTO workspaces (id, name, code, organization, department, description, \
                            icon, lead, visibility, allowed_affiliations, data_classification, cedar_policy_guard, organization_id) \
                     VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13) \
                     ON CONFLICT (id) DO UPDATE SET \
                        name = EXCLUDED.name, \
                        code = EXCLUDED.code, \
                        organization = EXCLUDED.organization, \
                        department = EXCLUDED.department, \
                        description = EXCLUDED.description, \
                        icon = EXCLUDED.icon, \
                        lead = EXCLUDED.lead, \
                        visibility = EXCLUDED.visibility, \
                        allowed_affiliations = EXCLUDED.allowed_affiliations, \
                        data_classification = EXCLUDED.data_classification, \
                        cedar_policy_guard = EXCLUDED.cedar_policy_guard, \
                        organization_id = EXCLUDED.organization_id",
                    &[
                        &ws.id,
                        &ws.name,
                        &ws.code,
                        &ws.organization,
                        &ws.department,
                        &ws.description,
                        &ws.icon,
                        &ws.lead,
                        &ws.visibility,
                        &affs_json,
                        &ws.data_classification,
                        &ws.cedar_policy_guard,
                        &ws.organization_id,
                    ],
                )?;
            }
            tx.commit()?;
            Ok(())
        })
    }

    pub fn count_workspaces(&self) -> Result<usize, RepositoryError> {
        self.with_client(|client| {
            let row = client.query_one("SELECT COUNT(*) FROM workspaces", &[])?;
            let count: i64 = row.get(0);
            Ok(count as usize)
        })
    }

    // -------------------------------------------------------------------------
    // COLLABORATORS
    // -------------------------------------------------------------------------

    pub fn get_collaborators(&self, workspace_id: &str) -> Result<Vec<CollaboratorRecord>, RepositoryError> {
        let ws_id = workspace_id.to_string();
        self.with_client(move |client| {
            let rows = client.query(
                "SELECT id, workspace_id, eppn, name, role, scoped_affiliation, department, added_at::text \
                 FROM workspace_collaborators WHERE workspace_id = $1 ORDER BY added_at ASC",
                &[&ws_id],
            )?;

            Ok(rows.iter().map(|r| CollaboratorRecord {
                id: r.get(0),
                workspace_id: r.get(1),
                eppn: r.get(2),
                name: r.get(3),
                role: r.get(4),
                scoped_affiliation: r.get(5),
                department: r.get(6),
                added_at: r.get(7),
            }).collect())
        })
    }

    pub fn upsert_collaborator(&self, c: &CollaboratorRecord) -> Result<(), RepositoryError> {
        let c = c.clone();
        self.with_client(move |client| {
            client.execute(
                "INSERT INTO workspace_collaborators (id, workspace_id, eppn, name, role, scoped_affiliation, department) \
                 VALUES ($1, $2, $3, $4, $5, $6, $7) \
                 ON CONFLICT (workspace_id, eppn) DO UPDATE SET \
                    role = EXCLUDED.role, \
                    name = EXCLUDED.name, \
                    scoped_affiliation = EXCLUDED.scoped_affiliation, \
                    department = EXCLUDED.department",
                &[&c.id, &c.workspace_id, &c.eppn, &c.name, &c.role, &c.scoped_affiliation, &c.department],
            )?;
            Ok(())
        })
    }

    pub fn remove_collaborator(&self, workspace_id: &str, eppn: &str) -> Result<(), RepositoryError> {
        let ws_id = workspace_id.to_string();
        let eppn_str = eppn.to_string();
        self.with_client(move |client| {
            client.execute(
                "DELETE FROM workspace_collaborators WHERE workspace_id = $1 AND eppn = $2",
                &[&ws_id, &eppn_str],
            )?;
            Ok(())
        })
    }

    // -------------------------------------------------------------------------
    // RECORDS
    // -------------------------------------------------------------------------

    pub fn list_records(&self, app_slug: &str) -> Result<Vec<DatasetRecord>, RepositoryError> {
        let slug = app_slug.to_string();
        self.with_client(move |client| {
            let rows = client.query(
                "SELECT id, app_slug, data, ceds_mapping, is_ferpa_sensitive, created_at::text \
                 FROM dataset_records WHERE app_slug = $1 ORDER BY created_at ASC",
                &[&slug],
            )?;

            Ok(rows.iter().map(|r| DatasetRecord {
                id: r.get(0),
                app_slug: r.get(1),
                data: r.get(2),
                ceds_mapping: r.get(3),
                is_ferpa_sensitive: r.get(4),
                created_at: r.get(5),
            }).collect())
        })
    }

    pub fn upsert_record(&self, rec: &DatasetRecord) -> Result<(), RepositoryError> {
        let rec = rec.clone();
        self.with_client(move |client| {
            client.execute(
                "INSERT INTO dataset_records (id, app_slug, data, ceds_mapping, is_ferpa_sensitive) \
                 VALUES ($1, $2, $3, $4, $5) \
                 ON CONFLICT (id) DO UPDATE SET \
                    data = EXCLUDED.data, \
                    ceds_mapping = EXCLUDED.ceds_mapping, \
                    is_ferpa_sensitive = EXCLUDED.is_ferpa_sensitive",
                &[&rec.id, &rec.app_slug, &rec.data, &rec.ceds_mapping, &rec.is_ferpa_sensitive],
            )?;
            Ok(())
        })
    }

    // -------------------------------------------------------------------------
    // SESSIONS
    // -------------------------------------------------------------------------

    pub fn list_sessions(&self) -> Result<HashMap<String, AuthSession>, RepositoryError> {
        self.with_client(|client| {
            let rows = client.query(
                "SELECT token, user_eppn, user_name, user_role_title, user_affiliation, user_department, \
                        original_admin, created_at::text \
                 FROM auth_sessions",
                &[],
            )?;

            let mut sessions = HashMap::new();
            for r in rows {
                let orig_admin: Option<Value> = r.get(6);
                let user = AuthUser {
                    eppn: r.get(1),
                    name: r.get(2),
                    role_title: r.get(3),
                    affiliation: r.get(4),
                    department: r.get(5),
                };
                let original_admin = orig_admin.and_then(|v| serde_json::from_value(v).ok());
                let session = AuthSession {
                    token: r.get(0),
                    user,
                    original_admin,
                    created_at: r.get(7),
                };
                sessions.insert(session.token.clone(), session);
            }
            Ok(sessions)
        })
    }

    pub fn upsert_session(&self, s: &AuthSession) -> Result<(), RepositoryError> {
        let s = s.clone();
        self.with_client(move |client| {
            let orig_admin_json = serde_json::to_value(&s.original_admin)?;
            client.execute(
                "INSERT INTO auth_sessions (token, user_eppn, user_name, user_role_title, user_affiliation, \
                        user_department, original_admin) \
                 VALUES ($1, $2, $3, $4, $5, $6, $7) \
                 ON CONFLICT (token) DO UPDATE SET \
                    user_eppn = EXCLUDED.user_eppn, \
                    user_name = EXCLUDED.user_name, \
                    user_role_title = EXCLUDED.user_role_title, \
                    user_affiliation = EXCLUDED.user_affiliation, \
                    user_department = EXCLUDED.user_department, \
                    original_admin = EXCLUDED.original_admin",
                &[
                    &s.token,
                    &s.user.eppn,
                    &s.user.name,
                    &s.user.role_title,
                    &s.user.affiliation,
                    &s.user.department,
                    &orig_admin_json,
                ],
            )?;
            Ok(())
        })
    }

    pub fn delete_session(&self, token: &str) -> Result<(), RepositoryError> {
        let tok = token.to_string();
        self.with_client(move |client| {
            client.execute("DELETE FROM auth_sessions WHERE token = $1", &[&tok])?;
            Ok(())
        })
    }

    // -------------------------------------------------------------------------
    // LEDGER
    // -------------------------------------------------------------------------

    pub fn get_ledger(&self) -> Result<Vec<LedgerEntry>, RepositoryError> {
        self.with_client(|client| {
            let rows = client.query(
                "SELECT sequence, entry_hash, previous_hash, timestamp_iso, principal, \
                        organization_code, app_slug, decision_type, oscal_control_id, rationale, \
                        payload, COALESCE(payload_hash, '') \
                 FROM governance_ledger ORDER BY sequence ASC",
                &[],
            )?;

            let mut entries = Vec::new();
            for r in rows {
                let seq: i64 = r.get(0);
                let dec_str: String = r.get(7);
                let payload: Value = r.get(10);
                let raw_payload_hash: String = r.get(11);
                let payload_hash = if raw_payload_hash.is_empty() {
                    LedgerEntry::compute_payload_hash(&payload)
                } else {
                    raw_payload_hash
                };
                entries.push(LedgerEntry {
                    sequence: seq as u64,
                    entry_hash: r.get(1),
                    previous_hash: r.get(2),
                    timestamp_iso: r.get(3),
                    principal: r.get(4),
                    organization_code: r.get(5),
                    app_slug: r.get(6),
                    decision_type: parse_decision_type(&dec_str)?,
                    oscal_control_id: r.get(8),
                    rationale: r.get(9),
                    payload_hash,
                });
            }
            Ok(entries)
        })
    }

    pub fn append_ledger_entry(&self, entry: &LedgerEntry, payload: &Value) -> Result<(), RepositoryError> {
        let entry = entry.clone();
        let payload = payload.clone();
        self.with_client(move |client| {
            let seq = entry.sequence as i64;
            let dec_str = format!("{:?}", entry.decision_type);
            client.execute(
                "INSERT INTO governance_ledger (sequence, entry_hash, previous_hash, timestamp_iso, \
                        principal, organization_code, app_slug, decision_type, oscal_control_id, \
                        rationale, payload_hash, payload) \
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)",
                &[
                    &seq,
                    &entry.entry_hash,
                    &entry.previous_hash,
                    &entry.timestamp_iso,
                    &entry.principal,
                    &entry.organization_code,
                    &entry.app_slug,
                    &dec_str,
                    &entry.oscal_control_id,
                    &entry.rationale,
                    &entry.payload_hash,
                    &payload,
                ],
            )?;
            Ok(())
        })
    }

    // -------------------------------------------------------------------------
    // APP MANIFESTS
    // -------------------------------------------------------------------------

    pub fn list_app_manifests(&self) -> Result<Vec<AppManifest>, RepositoryError> {
        self.with_client(|client| {
            let rows = client.query(
                "SELECT manifest FROM app_manifests ORDER BY updated_at ASC",
                &[],
            )?;
            let mut manifests = Vec::new();
            for r in rows {
                let val: Value = r.get(0);
                let m: AppManifest = serde_json::from_value(val)?;
                manifests.push(m);
            }
            Ok(manifests)
        })
    }

    pub fn get_app_manifest(&self, slug: &str) -> Result<Option<AppManifest>, RepositoryError> {
        let slug = slug.to_string();
        self.with_client(move |client| {
            let row = client.query_opt(
                "SELECT manifest FROM app_manifests WHERE slug = $1",
                &[&slug],
            )?;
            match row {
                Some(r) => {
                    let val: Value = r.get(0);
                    let m: AppManifest = serde_json::from_value(val)?;
                    Ok(Some(m))
                }
                None => Ok(None),
            }
        })
    }

    pub fn upsert_app_manifest(&self, m: &AppManifest) -> Result<(), RepositoryError> {
        let m = m.clone();
        self.with_client(move |client| {
            let manifest_json = serde_json::to_value(&m)?;
            client.execute(
                "INSERT INTO app_manifests (slug, title, description, organization_code, department, \
                        herm_capability_id, custom_domain, custom_domain_verified, workspace_id, manifest, updated_at) \
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, NOW()) \
                 ON CONFLICT (slug) DO UPDATE SET \
                    title = EXCLUDED.title, \
                    description = EXCLUDED.description, \
                    organization_code = EXCLUDED.organization_code, \
                    department = EXCLUDED.department, \
                    herm_capability_id = EXCLUDED.herm_capability_id, \
                    custom_domain = EXCLUDED.custom_domain, \
                    custom_domain_verified = EXCLUDED.custom_domain_verified, \
                    workspace_id = EXCLUDED.workspace_id, \
                    manifest = EXCLUDED.manifest, \
                    updated_at = NOW()",
                &[
                    &m.slug,
                    &m.title,
                    &m.description,
                    &m.organization_code,
                    &m.department,
                    &m.herm_capability_id,
                    &m.custom_domain,
                    &m.custom_domain_verified,
                    &m.workspace_id,
                    &manifest_json,
                ],
            )?;
            Ok(())
        })
    }

    // -------------------------------------------------------------------------
    // PUBLISHED DATASETS
    // -------------------------------------------------------------------------

    pub fn list_published_datasets(&self) -> Result<Vec<PublishedDataset>, RepositoryError> {
        self.with_client(|client| {
            let rows = client.query(
                "SELECT payload FROM published_datasets ORDER BY updated_at ASC",
                &[],
            )?;
            let mut datasets = Vec::new();
            for r in rows {
                let val: Value = r.get(0);
                let ds: PublishedDataset = serde_json::from_value(val)?;
                datasets.push(ds);
            }
            Ok(datasets)
        })
    }

    pub fn upsert_published_dataset(&self, ds: &PublishedDataset) -> Result<(), RepositoryError> {
        let ds = ds.clone();
        self.with_client(move |client| {
            let payload_json = serde_json::to_value(&ds)?;
            client.execute(
                "INSERT INTO published_datasets (id, name, description, department, organization, \
                        sensitivity_level, herm_capability_id, payload, updated_at) \
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, NOW()) \
                 ON CONFLICT (id) DO UPDATE SET \
                    name = EXCLUDED.name, \
                    description = EXCLUDED.description, \
                    department = EXCLUDED.department, \
                    organization = EXCLUDED.organization, \
                    sensitivity_level = EXCLUDED.sensitivity_level, \
                    herm_capability_id = EXCLUDED.herm_capability_id, \
                    payload = EXCLUDED.payload, \
                    updated_at = NOW()",
                &[
                    &ds.id,
                    &ds.name,
                    &ds.description,
                    &ds.department,
                    &ds.organization,
                    &ds.sensitivity_level,
                    &ds.herm_capability_id,
                    &payload_json,
                ],
            )?;
            Ok(())
        })
    }

    // -------------------------------------------------------------------------
    // DATASET RELATIONSHIPS
    // -------------------------------------------------------------------------

    pub fn list_dataset_relationships(&self) -> Result<Vec<DatasetRelationship>, RepositoryError> {
        self.with_client(|client| {
            let rows = client.query(
                "SELECT payload FROM dataset_relationships",
                &[],
            )?;
            let mut rels = Vec::new();
            for r in rows {
                let val: Value = r.get(0);
                let rel: DatasetRelationship = serde_json::from_value(val)?;
                rels.push(rel);
            }
            Ok(rels)
        })
    }

    pub fn upsert_dataset_relationship(&self, rel: &DatasetRelationship) -> Result<(), RepositoryError> {
        let rel = rel.clone();
        self.with_client(move |client| {
            let payload_json = serde_json::to_value(&rel)?;
            let rel_type = format!("{:?}", rel.relationship_type);
            client.execute(
                "INSERT INTO dataset_relationships (id, name, source_dataset_id, target_dataset_id, \
                        source_field, target_field, relationship_type, display_field, payload) \
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9) \
                 ON CONFLICT (id) DO UPDATE SET \
                    name = EXCLUDED.name, \
                    source_dataset_id = EXCLUDED.source_dataset_id, \
                    target_dataset_id = EXCLUDED.target_dataset_id, \
                    source_field = EXCLUDED.source_field, \
                    target_field = EXCLUDED.target_field, \
                    relationship_type = EXCLUDED.relationship_type, \
                    display_field = EXCLUDED.display_field, \
                    payload = EXCLUDED.payload",
                &[
                    &rel.id,
                    &rel.name,
                    &rel.source_dataset_id,
                    &rel.target_dataset_id,
                    &rel.source_field,
                    &rel.target_field,
                    &rel_type,
                    &rel.display_field,
                    &payload_json,
                ],
            )?;
            Ok(())
        })
    }

    // -------------------------------------------------------------------------
    // WORKFLOW AUTOMATIONS
    // -------------------------------------------------------------------------

    pub fn list_workflow_automations(&self, app_slug: Option<&str>) -> Result<Vec<AutomationRule>, RepositoryError> {
        let slug_opt = app_slug.map(str::to_string);
        self.with_client(move |client| {
            let rows = match slug_opt {
                Some(slug) => client.query(
                    "SELECT rule_json FROM workflow_automations WHERE app_slug = $1 ORDER BY updated_at ASC",
                    &[&slug],
                )?,
                None => client.query(
                    "SELECT rule_json FROM workflow_automations ORDER BY updated_at ASC",
                    &[],
                )?,
            };
            let mut rules = Vec::new();
            for r in rows {
                let val: Value = r.get(0);
                let rule: AutomationRule = serde_json::from_value(val)?;
                rules.push(rule);
            }
            Ok(rules)
        })
    }

    pub fn upsert_workflow_automation(&self, rule: &AutomationRule) -> Result<(), RepositoryError> {
        let rule = rule.clone();
        self.with_client(move |client| {
            let rule_json = serde_json::to_value(&rule)?;
            client.execute(
                "INSERT INTO workflow_automations (id, app_slug, name, rule_json, enabled, updated_at) \
                 VALUES ($1, $2, $3, $4, $5, NOW()) \
                 ON CONFLICT (id) DO UPDATE SET \
                    app_slug = EXCLUDED.app_slug, \
                    name = EXCLUDED.name, \
                    rule_json = EXCLUDED.rule_json, \
                    enabled = EXCLUDED.enabled, \
                    updated_at = NOW()",
                &[
                    &rule.id,
                    &rule.app_slug,
                    &rule.name,
                    &rule_json,
                    &rule.enabled,
                ],
            )?;
            Ok(())
        })
    }

    // -------------------------------------------------------------------------
    // PROCESS INSTANCES
    // -------------------------------------------------------------------------

    pub fn list_process_instances(
        &self,
        app_slug: Option<&str>,
        status: Option<&str>,
    ) -> Result<Vec<ProcessInstance>, RepositoryError> {
        let slug_opt = app_slug.map(str::to_string);
        let status_opt = status.map(str::to_string);
        self.with_client(move |client| {
            let rows = match (&slug_opt, &status_opt) {
                (Some(slug), Some(st)) => client.query(
                    "SELECT instance_json FROM process_instances WHERE app_slug = $1 AND status = $2 ORDER BY updated_at ASC",
                    &[&slug, &st],
                )?,
                (Some(slug), None) => client.query(
                    "SELECT instance_json FROM process_instances WHERE app_slug = $1 ORDER BY updated_at ASC",
                    &[&slug],
                )?,
                (None, Some(st)) => client.query(
                    "SELECT instance_json FROM process_instances WHERE status = $1 ORDER BY updated_at ASC",
                    &[&st],
                )?,
                (None, None) => client.query(
                    "SELECT instance_json FROM process_instances ORDER BY updated_at ASC",
                    &[],
                )?,
            };
            let mut instances = Vec::new();
            for r in rows {
                let val: Value = r.get(0);
                let inst: ProcessInstance = serde_json::from_value(val)?;
                instances.push(inst);
            }
            Ok(instances)
        })
    }

    pub fn upsert_process_instance(&self, inst: &ProcessInstance) -> Result<(), RepositoryError> {
        let inst = inst.clone();
        self.with_client(move |client| {
            let inst_json = serde_json::to_value(&inst)?;
            let status_str = match inst.status {
                ProcessStatus::Waiting => "Waiting",
                ProcessStatus::Completed => "Completed",
                ProcessStatus::Rejected => "Rejected",
                ProcessStatus::Failed => "Failed",
            };
            client.execute(
                "INSERT INTO process_instances (id, app_slug, rule_id, status, instance_json, updated_at)                  VALUES ($1, $2, $3, $4, $5, NOW())                  ON CONFLICT (id) DO UPDATE SET                     app_slug = EXCLUDED.app_slug,                     rule_id = EXCLUDED.rule_id,                     status = EXCLUDED.status,                     instance_json = EXCLUDED.instance_json,                     updated_at = NOW()",
                &[
                    &inst.id,
                    &inst.app_slug,
                    &inst.rule_id,
                    &status_str,
                    &inst_json,
                ],
            )?;
            Ok(())
        })
    }

    pub fn get_process_instance(&self, id: &str) -> Result<Option<ProcessInstance>, RepositoryError> {
        let id = id.to_string();
        self.with_client(move |client| {
            let row = client.query_opt(
                "SELECT instance_json FROM process_instances WHERE id = $1",
                &[&id],
            )?;
            match row {
                Some(r) => {
                    let val: Value = r.get(0);
                    let inst: ProcessInstance = serde_json::from_value(val)?;
                    Ok(Some(inst))
                }
                None => Ok(None),
            }
        })
    }

    // -------------------------------------------------------------------------
    // SCIM USERS AND GROUPS
    // -------------------------------------------------------------------------

    pub fn list_scim_users(&self) -> Result<Vec<ScimUser>, RepositoryError> {
        self.with_client(|client| {
            let rows = client.query(
                "SELECT payload FROM scim_users ORDER BY updated_at ASC",
                &[],
            )?;
            let mut users = Vec::new();
            for r in rows {
                let val: Value = r.get(0);
                let u: ScimUser = serde_json::from_value(val)?;
                users.push(u);
            }
            Ok(users)
        })
    }

    pub fn upsert_scim_user(&self, user: &ScimUser) -> Result<(), RepositoryError> {
        let user = user.clone();
        self.with_client(move |client| {
            let payload_json = serde_json::to_value(&user)?;
            client.execute(
                "INSERT INTO scim_users (id, user_name, active, payload, updated_at) \
                 VALUES ($1, $2, $3, $4, NOW()) \
                 ON CONFLICT (id) DO UPDATE SET \
                    user_name = EXCLUDED.user_name, \
                    active = EXCLUDED.active, \
                    payload = EXCLUDED.payload, \
                    updated_at = NOW()",
                &[
                    &user.id,
                    &user.user_name,
                    &user.active,
                    &payload_json,
                ],
            )?;
            Ok(())
        })
    }

    pub fn delete_scim_user(&self, id: &str) -> Result<(), RepositoryError> {
        let id_str = id.to_string();
        self.with_client(move |client| {
            client.execute("DELETE FROM scim_users WHERE id = $1", &[&id_str])?;
            Ok(())
        })
    }

    pub fn list_organizations(&self) -> Result<Vec<OrganizationNode>, RepositoryError> {
        self.with_client(|client| {
            let rows = client.query(
                "SELECT id, parent_id, name, code, org_type FROM organizations ORDER BY name ASC",
                &[],
            )?;
            let mut orgs = Vec::new();
            for r in rows {
                orgs.push(OrganizationNode {
                    id: r.get(0),
                    parent_id: r.get(1),
                    name: r.get(2),
                    code: r.get(3),
                    org_type: r.get(4),
                });
            }
            Ok(orgs)
        })
    }

    pub fn upsert_organization(&self, org: &OrganizationNode) -> Result<(), RepositoryError> {
        let org = org.clone();
        self.with_client(move |client| {
            client.execute(
                "INSERT INTO organizations (id, parent_id, name, code, org_type) \
                 VALUES ($1, $2, $3, $4, $5) \
                 ON CONFLICT (code) DO UPDATE SET \
                    parent_id = EXCLUDED.parent_id, \
                    name = EXCLUDED.name, \
                    org_type = EXCLUDED.org_type",
                &[&org.id, &org.parent_id, &org.name, &org.code, &org.org_type],
            )?;
            Ok(())
        })
    }

    pub fn list_roles(&self) -> Result<Vec<RoleRow>, RepositoryError> {
        self.with_client(|client| {
            let rows = client.query(
                "SELECT r.id, r.person_id, p.eppn, r.organization_id, r.role_title, \
                        r.scoped_affiliation, r.is_primary, COALESCE(r.source, 'api') \
                 FROM roles r \
                 JOIN persons p ON r.person_id = p.id",
                &[],
            )?;
            let mut roles = Vec::new();
            for r in rows {
                roles.push(RoleRow {
                    id: r.get(0),
                    person_id: r.get(1),
                    eppn: r.get(2),
                    organization_id: r.get(3),
                    role_title: r.get(4),
                    scoped_affiliation: r.get(5),
                    is_primary: r.get(6),
                    source: r.get(7),
                });
            }
            Ok(roles)
        })
    }

    pub fn upsert_role(&self, role: &RoleRow) -> Result<(), RepositoryError> {
        let role = role.clone();
        self.with_client(move |client| {
            let person_rows = client.query("SELECT id FROM persons WHERE eppn = $1", &[&role.eppn])?;
            let person_id = if let Some(row) = person_rows.first() {
                row.get::<_, uuid::Uuid>(0)
            } else {
                let new_pid = role.person_id;
                let email = role.eppn.clone();
                client.execute(
                    "INSERT INTO persons (id, first_name, last_name, email, eppn) \
                     VALUES ($1, 'User', 'User', $2, $3) \
                     ON CONFLICT (eppn) DO NOTHING",
                    &[&new_pid, &email, &role.eppn],
                )?;
                new_pid
            };

            client.execute(
                "INSERT INTO roles (id, person_id, organization_id, role_title, scoped_affiliation, is_primary, source) \
                 VALUES ($1, $2, $3, $4, $5, $6, $7) \
                 ON CONFLICT (id) DO UPDATE SET \
                    organization_id = EXCLUDED.organization_id, \
                    role_title = EXCLUDED.role_title, \
                    scoped_affiliation = EXCLUDED.scoped_affiliation, \
                    is_primary = EXCLUDED.is_primary, \
                    source = EXCLUDED.source",
                &[
                    &role.id,
                    &person_id,
                    &role.organization_id,
                    &role.role_title,
                    &role.scoped_affiliation,
                    &role.is_primary,
                    &role.source,
                ],
            )?;
            Ok(())
        })
    }

    pub fn delete_role(&self, id: uuid::Uuid) -> Result<(), RepositoryError> {
        self.with_client(move |client| {
            client.execute("DELETE FROM roles WHERE id = $1", &[&id])?;
            Ok(())
        })
    }

    pub fn delete_scim_roles_for_user(&self, eppn: &str) -> Result<(), RepositoryError> {
        let eppn_str = eppn.to_string();
        self.with_client(move |client| {
            client.execute(
                "DELETE FROM roles r USING persons p \
                 WHERE r.person_id = p.id AND p.eppn = $1 AND r.source = 'scim'",
                &[&eppn_str],
            )?;
            Ok(())
        })
    }

    pub fn list_scim_groups(&self) -> Result<Vec<ScimGroup>, RepositoryError> {
        self.with_client(|client| {
            let rows = client.query(
                "SELECT payload FROM scim_groups ORDER BY updated_at ASC",
                &[],
            )?;
            let mut groups = Vec::new();
            for r in rows {
                let val: Value = r.get(0);
                let g: ScimGroup = serde_json::from_value(val)?;
                groups.push(g);
            }
            Ok(groups)
        })
    }

    pub fn upsert_scim_group(&self, group: &ScimGroup) -> Result<(), RepositoryError> {
        let group = group.clone();
        self.with_client(move |client| {
            let payload_json = serde_json::to_value(&group)?;
            client.execute(
                "INSERT INTO scim_groups (id, display_name, payload, updated_at) \
                 VALUES ($1, $2, $3, NOW()) \
                 ON CONFLICT (id) DO UPDATE SET \
                    display_name = EXCLUDED.display_name, \
                    payload = EXCLUDED.payload, \
                    updated_at = NOW()",
                &[
                    &group.id,
                    &group.display_name,
                    &payload_json,
                ],
            )?;
            Ok(())
        })
    }

    pub fn delete_scim_group(&self, id: &str) -> Result<(), RepositoryError> {
        let id_str = id.to_string();
        self.with_client(move |client| {
            client.execute("DELETE FROM scim_groups WHERE id = $1", &[&id_str])?;
            Ok(())
        })
    }
}

#[derive(Debug, Clone)]
pub struct AppendDecisionParams {
    pub principal: String,
    pub organization_code: String,
    pub app_slug: Option<String>,
    pub decision_type: DecisionType,
    pub oscal_control_id: String,
    pub rationale: String,
    pub payload: Value,
}

impl PostgresRepository {
    pub fn append_ledger_decision(
        &self,
        params: AppendDecisionParams,
    ) -> Result<LedgerEntry, RepositoryError> {
        self.with_client(move |client| {
            let mut tx = client.transaction()?;
            tx.execute("LOCK TABLE governance_ledger IN EXCLUSIVE MODE", &[])?;

            let row = tx.query_opt(
                "SELECT sequence, entry_hash FROM governance_ledger ORDER BY sequence DESC LIMIT 1",
                &[],
            )?;
            let (sequence, previous_hash) = match row {
                Some(r) => {
                    let last_seq: i64 = r.get(0);
                    let last_hash: String = r.get(1);
                    ((last_seq + 1) as u64, last_hash)
                }
                None => (0, GENESIS_PREVIOUS_HASH.to_string()),
            };

            let now = Utc::now().to_rfc3339();
            let entry = LedgerEntry::new(scaffoldry_core::NewLedgerEntryParams {
                sequence,
                timestamp_iso: now,
                previous_hash,
                principal: params.principal,
                organization_code: params.organization_code,
                app_slug: params.app_slug,
                decision_type: params.decision_type,
                oscal_control_id: params.oscal_control_id,
                rationale: params.rationale,
                payload: &params.payload,
            });

            let seq = entry.sequence as i64;
            let dec_str = format!("{:?}", entry.decision_type);
            tx.execute(
                "INSERT INTO governance_ledger (sequence, entry_hash, previous_hash, timestamp_iso, \
                        principal, organization_code, app_slug, decision_type, oscal_control_id, \
                        rationale, payload_hash, payload) \
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)",
                &[
                    &seq,
                    &entry.entry_hash,
                    &entry.previous_hash,
                    &entry.timestamp_iso,
                    &entry.principal,
                    &entry.organization_code,
                    &entry.app_slug,
                    &dec_str,
                    &entry.oscal_control_id,
                    &entry.rationale,
                    &entry.payload_hash,
                    &params.payload,
                ],
            )?;

            tx.commit()?;
            Ok(entry)
        })
    }
}

fn parse_decision_type(s: &str) -> Result<DecisionType, RepositoryError> {
    DecisionType::parse(s).ok_or_else(|| RepositoryError::TamperDetected(format!("Unknown decision type: {s}")))
}

impl PostgresRepository {
    pub fn insert_api_token(&self, token: &crate::state::ApiToken) -> Result<(), RepositoryError> {
        let token = token.clone();
        self.with_client(move |client| {
            client.execute(
                "INSERT INTO api_tokens (token_hash, id, kind, eppn, label, original_admin, created_at, expires_at, last_used_at, revoked_at)                  VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)                  ON CONFLICT (token_hash) DO UPDATE SET                     label = EXCLUDED.label,                     last_used_at = EXCLUDED.last_used_at,                     revoked_at = EXCLUDED.revoked_at",
                &[
                    &token.token_hash,
                    &token.id,
                    &token.kind,
                    &token.eppn,
                    &token.label,
                    &token.original_admin,
                    &token.created_at,
                    &token.expires_at,
                    &token.last_used_at,
                    &token.revoked_at,
                ],
            )?;
            Ok(())
        })
    }

    pub fn get_api_token(&self, token_hash: &str) -> Result<Option<crate::state::ApiToken>, RepositoryError> {
        let hash = token_hash.to_string();
        self.with_client(move |client| {
            let row_opt = client.query_opt(
                "SELECT token_hash, id, kind, eppn, label, original_admin,                         created_at, expires_at, last_used_at, revoked_at                  FROM api_tokens WHERE token_hash = $1",
                &[&hash],
            )?;
            match row_opt {
                Some(r) => Ok(Some(crate::state::ApiToken {
                    token_hash: r.get(0),
                    id: r.get(1),
                    kind: r.get(2),
                    eppn: r.get(3),
                    label: r.get(4),
                    original_admin: r.get(5),
                    created_at: r.get(6),
                    expires_at: r.get(7),
                    last_used_at: r.get(8),
                    revoked_at: r.get(9),
                })),
                None => Ok(None),
            }
        })
    }

    pub fn update_token_last_used(&self, token_hash: &str, now: chrono::DateTime<chrono::Utc>) -> Result<(), RepositoryError> {
        let hash = token_hash.to_string();
        self.with_client(move |client| {
            client.execute(
                "UPDATE api_tokens SET last_used_at = $2 WHERE token_hash = $1",
                &[&hash, &now],
            )?;
            Ok(())
        })
    }

    pub fn revoke_api_token(&self, id: uuid::Uuid, caller_eppn: &str, is_platform_admin: bool) -> Result<bool, RepositoryError> {
        let caller_eppn = caller_eppn.to_string();
        self.with_client(move |client| {
            let rows_affected = if is_platform_admin {
                client.execute(
                    "UPDATE api_tokens SET revoked_at = NOW() WHERE id = $1 AND revoked_at IS NULL",
                    &[&id],
                )?
            } else {
                client.execute(
                    "UPDATE api_tokens SET revoked_at = NOW() WHERE id = $1 AND eppn = $2 AND revoked_at IS NULL",
                    &[&id, &caller_eppn],
                )?
            };
            Ok(rows_affected > 0)
        })
    }

    pub fn revoke_api_token_by_hash(&self, token_hash: &str) -> Result<bool, RepositoryError> {
        let hash = token_hash.to_string();
        self.with_client(move |client| {
            let rows_affected = client.execute(
                "UPDATE api_tokens SET revoked_at = NOW() WHERE token_hash = $1 AND revoked_at IS NULL",
                &[&hash],
            )?;
            Ok(rows_affected > 0)
        })
    }

    pub fn list_api_tokens_for_eppn(&self, eppn: &str) -> Result<Vec<crate::state::ApiToken>, RepositoryError> {
        let eppn = eppn.to_string();
        self.with_client(move |client| {
            let rows = client.query(
                "SELECT token_hash, id, kind, eppn, label, original_admin,                         created_at::text, expires_at::text, last_used_at::text, revoked_at::text                  FROM api_tokens WHERE eppn = $1 AND kind = 'agent' AND revoked_at IS NULL ORDER BY created_at DESC",
                &[&eppn],
            )?;
            let parse_time = |s: &str| -> chrono::DateTime<chrono::Utc> {
                chrono::DateTime::parse_from_rfc3339(s)
                    .or_else(|_| chrono::DateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S%.f%#z"))
                    .map(|d| d.with_timezone(&chrono::Utc))
                    .unwrap_or_else(|_| chrono::Utc::now())
            };
            let mut list = Vec::new();
            for r in rows {
                let created_str: String = r.get(6);
                let expires_str: String = r.get(7);
                let last_used_str: Option<String> = r.get(8);
                let revoked_str: Option<String> = r.get(9);
                list.push(crate::state::ApiToken {
                    token_hash: r.get(0),
                    id: r.get(1),
                    kind: r.get(2),
                    eppn: r.get(3),
                    label: r.get(4),
                    original_admin: r.get(5),
                    created_at: parse_time(&created_str),
                    expires_at: parse_time(&expires_str),
                    last_used_at: last_used_str.map(|s| parse_time(&s)),
                    revoked_at: revoked_str.map(|s| parse_time(&s)),
                });
            }
            Ok(list)
        })
    }

    pub fn list_all_api_tokens_for_eppn(&self, eppn: &str) -> Result<Vec<crate::state::ApiToken>, RepositoryError> {
        let eppn = eppn.to_string();
        self.with_client(move |client| {
            let rows = client.query(
                "SELECT token_hash, id, kind, eppn, label, original_admin,                         created_at::text, expires_at::text, last_used_at::text, revoked_at::text                  FROM api_tokens WHERE eppn = $1  ORDER BY created_at DESC",
                &[&eppn],
            )?;
            let parse_time = |s: &str| -> chrono::DateTime<chrono::Utc> {
                chrono::DateTime::parse_from_rfc3339(s)
                    .or_else(|_| chrono::DateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S%.f%#z"))
                    .map(|d| d.with_timezone(&chrono::Utc))
                    .unwrap_or_else(|_| chrono::Utc::now())
            };
            let mut list = Vec::new();
            for r in rows {
                let created_str: String = r.get(6);
                let expires_str: String = r.get(7);
                let last_used_str: Option<String> = r.get(8);
                let revoked_str: Option<String> = r.get(9);
                list.push(crate::state::ApiToken {
                    token_hash: r.get(0),
                    id: r.get(1),
                    kind: r.get(2),
                    eppn: r.get(3),
                    label: r.get(4),
                    original_admin: r.get(5),
                    created_at: parse_time(&created_str),
                    expires_at: parse_time(&expires_str),
                    last_used_at: last_used_str.map(|s| parse_time(&s)),
                    revoked_at: revoked_str.map(|s| parse_time(&s)),
                });
            }
            Ok(list)
        })
    }

    pub fn count_api_tokens(&self) -> Result<i64, RepositoryError> {
        self.with_client(|client| {
            let row = client.query_one("SELECT COUNT(*) FROM api_tokens", &[])?;
            let cnt: i64 = row.get(0);
            Ok(cnt)
        })
    }

    pub fn revoke_setup_tokens(&self) -> Result<(), RepositoryError> {
        self.with_client(|client| {
            client.execute(
                "UPDATE api_tokens SET revoked_at = NOW() WHERE kind = 'setup' AND revoked_at IS NULL",
                &[],
            )?;
            Ok(())
        })
    }

    pub fn get_scim_user_by_username(&self, user_name: &str) -> Result<Option<crate::state::ScimUser>, RepositoryError> {
        let name = user_name.to_string();
        self.with_client(move |client| {
            let row_opt = client.query_opt(
                "SELECT payload, active FROM scim_users WHERE LOWER(user_name) = LOWER($1) ORDER BY updated_at DESC LIMIT 1",
                &[&name],
            )?;
            match row_opt {
                Some(r) => {
                    let val: serde_json::Value = r.get(0);
                    let mut user: crate::state::ScimUser = serde_json::from_value(val)?;
                    user.active = r.get(1);
                    Ok(Some(user))
                }
                None => Ok(None),
            }
        })
    }

    pub fn get_platform_settings(&self) -> Result<std::collections::HashMap<String, serde_json::Value>, RepositoryError> {
        self.with_client(|client| {
            let rows = client.query("SELECT key, value FROM platform_settings", &[])?;
            let mut map = std::collections::HashMap::new();
            for r in rows {
                let k: String = r.get(0);
                let v: serde_json::Value = r.get(1);
                map.insert(k, v);
            }
            Ok(map)
        })
    }

    pub fn put_platform_setting(&self, key: &str, value: &serde_json::Value, updated_by: &str) -> Result<(), RepositoryError> {
        let k = key.to_string();
        let val = value.clone();
        let by = updated_by.to_string();
        self.with_client(move |client| {
            client.execute(
                "INSERT INTO platform_settings (key, value, updated_by, updated_at)                  VALUES ($1, $2, $3, NOW())                  ON CONFLICT (key) DO UPDATE SET value = EXCLUDED.value, updated_by = EXCLUDED.updated_by, updated_at = NOW()",
                &[&k, &val, &by],
            )?;
            Ok(())
        })
    }
}
