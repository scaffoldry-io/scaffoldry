//! Storage for app proposals and page sources. Approval is one transaction on one connection:
//! the ledger entry, the manifest write, and the proposal's status all commit together or not at all.

use crate::repository::{append_ledger_in_tx, AppendDecisionParams, PostgresRepository, RepositoryError};
use scaffoldry_core::{DecisionType, LedgerEntry};
use scaffoldry_engine::AppManifest;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

/// The checks that need a second person to approve when they fail.
pub const FLAGGED_CHECKS: &[&str] = &["sensitivity", "data_loss", "page_sensitivity"];

#[derive(Debug, Clone, serde::Serialize)]
pub struct ProposalRow {
    pub id: uuid::Uuid,
    pub workspace_id: String,
    pub app_slug: String,
    pub base_version: Option<i32>,
    pub manifest: Value,
    /// `{ "checks": [...], "changes": [...] }`
    pub checks: Value,
    pub summary: String,
    pub status: String,
    pub proposed_by: String,
    pub decided_by: Option<String>,
    pub created_at: String,
    pub decided_at: Option<String>,
}

#[derive(Debug)]
pub enum Approval {
    Approved { version: i32, entry: Box<LedgerEntry> },
    /// The proposal was already decided. Carries its status.
    NotPending(String),
    /// The live app is no longer at the proposal's base version. The row is now `Superseded`.
    Superseded { live: Option<i32> },
    /// A flagged check failed and the decider is the proposer.
    SecondPersonRequired,
}

const COLUMNS: &str = "id, workspace_id, app_slug, base_version, manifest, checks, summary, status, proposed_by, decided_by, \
     to_char(created_at AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS\"Z\"'), \
     to_char(decided_at AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS\"Z\"')";

fn row_of(r: &postgres::Row) -> ProposalRow {
    ProposalRow {
        id: r.get(0),
        workspace_id: r.get(1),
        app_slug: r.get(2),
        base_version: r.get(3),
        manifest: r.get(4),
        checks: r.get(5),
        summary: r.get(6),
        status: r.get(7),
        proposed_by: r.get(8),
        decided_by: r.get(9),
        created_at: r.get(10),
        decided_at: r.get(11),
    }
}

/// True when a flagged check failed.
pub fn has_flagged_failure(checks: &Value) -> bool {
    checks["checks"].as_array().is_some_and(|list| {
        list.iter().any(|c| c["outcome"] == "fail" && c["name"].as_str().is_some_and(|n| FLAGGED_CHECKS.contains(&n)))
    })
}

pub fn manifest_sha256(manifest: &Value) -> String {
    Sha256::digest(serde_json::to_vec(manifest).unwrap_or_default()).iter().map(|b| format!("{b:02x}")).collect()
}

impl PostgresRepository {
    /// Stores a page source under its hash. An existing row is left as it is.
    pub fn store_page_source(&self, sha256: &str, source: &str, created_by: &str) -> Result<(), RepositoryError> {
        let (sha, source, by) = (sha256.to_string(), source.to_string(), created_by.to_string());
        self.with_client(move |client| {
            client.execute(
                "INSERT INTO app_page_sources (sha256, source, created_by) VALUES ($1, $2, $3) ON CONFLICT (sha256) DO NOTHING",
                &[&sha, &source, &by],
            )?;
            Ok(())
        })
    }

    pub fn page_sources_present(&self, hashes: &[String]) -> Result<Vec<String>, RepositoryError> {
        let hashes = hashes.to_vec();
        self.with_client(move |client| {
            let rows = client.query("SELECT sha256 FROM app_page_sources WHERE sha256 = ANY($1)", &[&hashes])?;
            Ok(rows.iter().map(|r| r.get::<_, String>(0).trim().to_string()).collect())
        })
    }

    pub fn insert_proposal(&self, p: &ProposalRow, sources: &[(String, String)]) -> Result<(), RepositoryError> {
        let (p, sources) = (p.clone(), sources.to_vec());
        self.with_client(move |client| {
            let mut tx = client.transaction()?;
            for (sha, source) in &sources {
                tx.execute(
                    "INSERT INTO app_page_sources (sha256, source, created_by) VALUES ($1, $2, $3) ON CONFLICT (sha256) DO NOTHING",
                    &[sha, source, &p.proposed_by],
                )?;
            }
            tx.execute(
                "INSERT INTO app_proposals (id, workspace_id, app_slug, base_version, manifest, checks, summary, proposed_by) \
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
                &[&p.id, &p.workspace_id, &p.app_slug, &p.base_version, &p.manifest, &p.checks, &p.summary, &p.proposed_by],
            )?;
            tx.commit()?;
            Ok(())
        })
    }

    pub fn get_proposal(&self, id: uuid::Uuid) -> Result<Option<ProposalRow>, RepositoryError> {
        self.with_client(move |client| {
            let row = client.query_opt(&format!("SELECT {COLUMNS} FROM app_proposals WHERE id = $1"), &[&id])?;
            Ok(row.as_ref().map(row_of))
        })
    }

    /// Pending first, then newest first.
    pub fn list_proposals(&self, workspace_id: &str, limit: i64) -> Result<Vec<ProposalRow>, RepositoryError> {
        let ws = workspace_id.to_string();
        self.with_client(move |client| {
            let rows = client.query(
                &format!(
                    "SELECT {COLUMNS} FROM app_proposals WHERE workspace_id = $1 \
                     ORDER BY (status = 'Pending') DESC, created_at DESC, id LIMIT $2"
                ),
                &[&ws, &limit],
            )?;
            Ok(rows.iter().map(row_of).collect())
        })
    }

    /// Rejects a pending proposal. Returns the status found when it was not pending.
    pub fn reject_proposal(&self, id: uuid::Uuid, decider: &str) -> Result<Result<(), String>, RepositoryError> {
        let decider = decider.to_string();
        self.with_client(move |client| {
            let done = client.execute(
                "UPDATE app_proposals SET status = 'Rejected', decided_by = $2, decided_at = NOW() WHERE id = $1 AND status = 'Pending'",
                &[&id, &decider],
            )?;
            if done == 1 {
                return Ok(Ok(()));
            }
            let status = client
                .query_opt("SELECT status FROM app_proposals WHERE id = $1", &[&id])?
                .map(|r| r.get::<_, String>(0))
                .ok_or_else(|| RepositoryError::NotFound(format!("proposal {id}")))?;
            Ok(Err(status))
        })
    }

    /// Approves a proposal in one transaction on one connection. If any step fails, none is kept.
    pub fn approve_proposal(&self, id: uuid::Uuid, decider: &str, rationale: &str) -> Result<Approval, RepositoryError> {
        let (decider, rationale) = (decider.to_string(), rationale.to_string());
        self.with_client(move |client| {
            let mut tx = client.transaction()?;
            let row = tx
                .query_opt(&format!("SELECT {COLUMNS} FROM app_proposals WHERE id = $1 FOR UPDATE"), &[&id])?
                .ok_or_else(|| RepositoryError::NotFound(format!("proposal {id}")))?;
            let p = row_of(&row);

            // 1. Only a pending proposal can be decided.
            if p.status != "Pending" {
                return Ok(Approval::NotPending(p.status));
            }

            // 2. The live app must still be at the version the proposal was made against.
            let live: Option<i32> = tx
                .query_opt("SELECT version FROM app_manifests WHERE slug = $1 FOR UPDATE", &[&p.app_slug])?
                .map(|r| r.get(0));
            if live != p.base_version {
                tx.execute("UPDATE app_proposals SET status = 'Superseded' WHERE id = $1", &[&id])?;
                tx.commit()?;
                return Ok(Approval::Superseded { live });
            }

            // 3. A flagged change needs a second person.
            if has_flagged_failure(&p.checks) && p.proposed_by.eq_ignore_ascii_case(&decider) {
                return Ok(Approval::SecondPersonRequired);
            }

            // 4. The ledger entry.
            let manifest: AppManifest = serde_json::from_value(p.manifest.clone())?;
            let entry = append_ledger_in_tx(
                &mut tx,
                AppendDecisionParams {
                    principal: decider.clone(),
                    organization_code: manifest.organization_code.clone(),
                    app_slug: Some(p.app_slug.clone()),
                    decision_type: DecisionType::AppPublished,
                    oscal_control_id: "CM-03".to_string(),
                    rationale,
                    payload: json!({
                        "proposal_id": p.id.to_string(),
                        "manifest_sha256": manifest_sha256(&p.manifest),
                        "base_version": p.base_version,
                    }),
                },
            )?;

            // 5. The manifest, one version up.
            let version: i32 = match p.base_version {
                Some(base) => tx
                    .query_one(
                        "UPDATE app_manifests SET title = $2, description = $3, organization_code = $4, department = $5, \
                                herm_capability_id = $6, custom_domain = $7, custom_domain_verified = $8, workspace_id = $9, \
                                manifest = $10, version = version + 1, updated_at = NOW() \
                         WHERE slug = $1 AND version = $11 RETURNING version",
                        &[
                            &manifest.slug,
                            &manifest.title,
                            &manifest.description,
                            &manifest.organization_code,
                            &manifest.department,
                            &manifest.herm_capability_id,
                            &manifest.custom_domain,
                            &manifest.custom_domain_verified,
                            &manifest.workspace_id,
                            &p.manifest,
                            &base,
                        ],
                    )?
                    .get(0),
                None => tx
                    .query_one(
                        "INSERT INTO app_manifests (slug, title, description, organization_code, department, \
                                herm_capability_id, custom_domain, custom_domain_verified, workspace_id, manifest, updated_at) \
                         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, NOW()) RETURNING version",
                        &[
                            &manifest.slug,
                            &manifest.title,
                            &manifest.description,
                            &manifest.organization_code,
                            &manifest.department,
                            &manifest.herm_capability_id,
                            &manifest.custom_domain,
                            &manifest.custom_domain_verified,
                            &manifest.workspace_id,
                            &p.manifest,
                        ],
                    )?
                    .get(0),
            };

            // 6. The decision.
            tx.execute(
                "UPDATE app_proposals SET status = 'Approved', decided_by = $2, decided_at = NOW() WHERE id = $1",
                &[&id, &decider],
            )?;
            tx.commit()?;
            Ok(Approval::Approved { version, entry: Box::new(entry) })
        })
    }
}
