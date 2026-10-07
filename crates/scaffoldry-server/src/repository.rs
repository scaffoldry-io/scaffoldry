//! PostgreSQL 17 Repository Layer with Cryptographic Ledger Hash Chain Verification
//! Enforces:
//! - NIST OSCAL 1.1.2 AU-02 cryptographic immutable audit ledger verification on startup.
//! - Relational persistence for Workspaces, Collaborators, Records, Sessions, and Ledger.
//! - Institutional scale performance (1,000+ workspaces and collaborators).
//! - Dedicated background worker architecture to isolate synchronous postgres driver from Tokio runtimes.

use crate::state::{
    AuthSession, AuthUser, CollaboratorRecord, DatasetRecord, WorkspaceRecord,
};
use chrono::Utc;
use postgres::{Client, NoTls};
use scaffoldry_core::{DecisionType, LedgerEntry, LedgerError, GENESIS_PREVIOUS_HASH};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::mpsc::{channel, sync_channel, Sender};

const SCHEMA_0001: &str = include_str!("../../scaffoldry-core/migrations/0001_initial_schema.sql");
const SCHEMA_0002: &str = include_str!("../../scaffoldry-core/migrations/0002_workspaces_and_ledger.sql");

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
}

impl PostgresRepository {
    pub fn connect(url: Option<&str>) -> Result<Self, RepositoryError> {
        let db_url = url
            .map(str::to_string)
            .or_else(|| std::env::var("DATABASE_URL").ok())
            .unwrap_or_else(|| {
                "postgres://scaffoldry:scaffoldry_dev_password@127.0.0.1:5433/scaffoldry".to_string()
            });

        let (worker_tx, worker_rx) = channel::<WorkerJob>();
        let (init_tx, init_rx) = sync_channel::<Result<(), RepositoryError>>(1);

        std::thread::Builder::new()
            .name("scaffoldry-db-worker".to_string())
            .spawn(move || {
                let mut client = match Client::connect(&db_url, NoTls) {
                    Ok(c) => c,
                    Err(e) => {
                        let _ = init_tx.send(Err(RepositoryError::Db(e)));
                        return;
                    }
                };

                // Run migrations with advisory lock to prevent concurrent DDL deadlocks
                let mig_res = (|| -> Result<(), postgres::Error> {
                    client.execute("SELECT pg_advisory_lock(742199)", &[])?;
                    let res = (|| {
                        client.batch_execute(SCHEMA_0001)?;
                        client.batch_execute(SCHEMA_0002)?;
                        let _ = client.execute("DELETE FROM auth_sessions WHERE token LIKE 'sct_%'", &[]);
                        Ok(())
                    })();
                    let _ = client.execute("SELECT pg_advisory_unlock(742199)", &[]);
                    res
                })();

                if let Err(e) = mig_res {
                    let _ = init_tx.send(Err(RepositoryError::Db(e)));
                    return;
                }

                let _ = init_tx.send(Ok(()));

                while let Ok(job) = worker_rx.recv() {
                    job(&mut client);
                }
            })
            .map_err(|e| RepositoryError::NotFound(format!("Failed to spawn db worker thread: {e}")))?;

        init_rx
            .recv()
            .map_err(|_| RepositoryError::NotFound("Worker init hung up".to_string()))??;

        let repo = Self { worker_tx };

        // Verify ledger hash chain on startup - refuse to start on broken chain!
        repo.verify_and_initialize_ledger()?;

        Ok(repo)
    }

    fn with_client<R: Send + 'static>(
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

    /// Verifies the cryptographic SHA-256 block chain from sequence 0 to N.
    /// Fails closed if any block hash mismatch or chain discontinuity is found.
    pub fn verify_and_initialize_ledger(&self) -> Result<(), RepositoryError> {
        let rows = self.with_client(|client| {
            let r = client.query(
                "SELECT sequence, entry_hash, previous_hash, timestamp_iso, principal, \
                        organization_code, app_slug, decision_type, oscal_control_id, rationale, payload, \
                        COALESCE(payload_hash, '') \
                 FROM governance_ledger ORDER BY sequence ASC",
                &[],
            )?;
            let mapped = r.into_iter().map(|row| {
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
            Ok(mapped)
        })?;

        if rows.is_empty() {
            // Seed genesis ledger entries if table is newly created
            self.seed_genesis_ledger()?;
            return Ok(());
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

            let decision_type = parse_decision_type(&decision_type_str);
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

    fn seed_genesis_ledger(&self) -> Result<(), RepositoryError> {
        let now = Utc::now().to_rfc3339();

        let entry_0 = LedgerEntry::new(scaffoldry_core::NewLedgerEntryParams {
            sequence: 0,
            timestamp_iso: now.clone(),
            previous_hash: GENESIS_PREVIOUS_HASH.to_string(),
            principal: "prof.curie@science.state.edu".to_string(),
            organization_code: "DIV-SCIENCES".to_string(),
            app_slug: Some("biology-lab-inventory".to_string()),
            decision_type: DecisionType::AppPublished,
            oscal_control_id: "CM-03".to_string(),
            rationale: "Initial publication of Biology Research Chemical Inventory".to_string(),
            payload: &serde_json::json!({"status": "Published", "domain": "inventory.biology.state.edu"}),
        });

        let entry_1 = LedgerEntry::new(scaffoldry_core::NewLedgerEntryParams {
            sequence: 1,
            timestamp_iso: now.clone(),
            previous_hash: entry_0.entry_hash.clone(),
            principal: "prof.curie@science.state.edu".to_string(),
            organization_code: "DIV-SCIENCES".to_string(),
            app_slug: Some("biology-lab-inventory".to_string()),
            decision_type: DecisionType::VanityDnsBound,
            oscal_control_id: "SC-07".to_string(),
            rationale: "Vanity DNS alias bound with sovereign gateway validation".to_string(),
            payload: &serde_json::json!({"domain": "inventory.biology.state.edu", "verified": true}),
        });

        let entry_2 = LedgerEntry::new(scaffoldry_core::NewLedgerEntryParams {
            sequence: 2,
            timestamp_iso: now.clone(),
            previous_hash: entry_1.entry_hash.clone(),
            principal: "dr.watson@science.state.edu".to_string(),
            organization_code: "DIV-COMPLIANCE".to_string(),
            app_slug: Some("physics-admissions-review".to_string()),
            decision_type: DecisionType::WorkflowRuleApproved,
            oscal_control_id: "AC-03".to_string(),
            rationale: "FERPA compliance and GPA threshold auto-admit rule approved".to_string(),
            payload: &serde_json::json!({"rule": "auto-physics-honors-admit", "policy": "policy-ferpa-34cfr99"}),
        });

        let entry_3 = LedgerEntry::new(scaffoldry_core::NewLedgerEntryParams {
            sequence: 3,
            timestamp_iso: now,
            previous_hash: entry_2.entry_hash.clone(),
            principal: "dr.watson@science.state.edu".to_string(),
            organization_code: "DIV-COMPLIANCE".to_string(),
            app_slug: Some("compliance-ferpa-requests".to_string()),
            decision_type: DecisionType::StatutoryAttestation,
            oscal_control_id: "AU-02".to_string(),
            rationale: "Institutional statutory compliance attestation for 34 CFR Part 99".to_string(),
            payload: &serde_json::json!({"framework": "FERPA", "standard": "34 CFR Part 99"}),
        });

        let entries = vec![
            (entry_0, serde_json::json!({"status": "Published", "domain": "inventory.biology.state.edu"})),
            (entry_1, serde_json::json!({"domain": "inventory.biology.state.edu", "verified": true})),
            (entry_2, serde_json::json!({"rule": "auto-physics-honors-admit", "policy": "policy-ferpa-34cfr99"})),
            (entry_3, serde_json::json!({"framework": "FERPA", "standard": "34 CFR Part 99"})),
        ];

        self.with_client(move |client| {
            for (e, payload) in entries {
                let seq = e.sequence as i64;
                let dec_str = format!("{:?}", e.decision_type);
                client.execute(
                    "INSERT INTO governance_ledger (sequence, entry_hash, previous_hash, timestamp_iso, \
                            principal, organization_code, app_slug, decision_type, oscal_control_id, \
                            rationale, payload_hash, payload) \
                     VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12) \
                     ON CONFLICT (sequence) DO UPDATE SET \
                        payload_hash = EXCLUDED.payload_hash, \
                        entry_hash = EXCLUDED.entry_hash",
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
                        created_at::text \
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
                        created_at::text \
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
                        icon, lead, visibility, allowed_affiliations, data_classification, cedar_policy_guard) \
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12) \
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
                    cedar_policy_guard = EXCLUDED.cedar_policy_guard",
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
                            icon, lead, visibility, allowed_affiliations, data_classification, cedar_policy_guard) \
                     VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12) \
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
                        cedar_policy_guard = EXCLUDED.cedar_policy_guard",
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
                    decision_type: parse_decision_type(&dec_str),
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
            let row = client.query_opt(
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
                    &params.payload,
                ],
            )?;

            Ok(entry)
        })
    }
}

fn parse_decision_type(s: &str) -> DecisionType {
    match s {
        "AppPublished" => DecisionType::AppPublished,
        "VanityDnsBound" => DecisionType::VanityDnsBound,
        "PolicyRevision" => DecisionType::PolicyRevision,
        "WorkflowRuleApproved" => DecisionType::WorkflowRuleApproved,
        "AccessRoleGranted" => DecisionType::AccessRoleGranted,
        "DatasetAccessShared" => DecisionType::DatasetAccessShared,
        "StatutoryAttestation" => DecisionType::StatutoryAttestation,
        "ImpersonationSessionStarted" => DecisionType::ImpersonationSessionStarted,
        "ImpersonationSessionEnded" => DecisionType::ImpersonationSessionEnded,
        "WorkspaceCreated" => DecisionType::WorkspaceCreated,
        "WorkspaceUpdated" => DecisionType::WorkspaceUpdated,
        "WorkspaceMemberAdded" => DecisionType::WorkspaceMemberAdded,
        "WorkspaceMemberRemoved" => DecisionType::WorkspaceMemberRemoved,
        "WorkspaceMemberRoleUpdated" => DecisionType::WorkspaceMemberRoleUpdated,
        _ => DecisionType::PolicyRevision,
    }
}
