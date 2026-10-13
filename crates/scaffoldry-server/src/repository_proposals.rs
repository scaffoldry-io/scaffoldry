//! Storage for app proposals and page sources. Approval is one transaction on one connection:
//! the ledger entry, the manifest write, and the proposal's status all commit together or not at all.

use crate::repository::{append_ledger_in_tx, AppendDecisionParams, PostgresRepository, RepositoryError};
use scaffoldry_core::{AutomationRule, DecisionType, LedgerEntry};
use scaffoldry_engine::AppManifest;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

/// The checks that need a second person to approve when they fail.
pub const FLAGGED_CHECKS: &[&str] = &["sensitivity", "data_loss", "page_sensitivity", "rule_effects", "access_loosened"];

#[derive(Debug, Clone, serde::Serialize)]
pub struct ProposalRow {
    pub id: uuid::Uuid,
    pub workspace_id: String,
    pub app_slug: String,
    pub base_version: Option<i32>,
    /// `None` for a proposal that changes rules only.
    pub manifest: Option<Value>,
    /// `{ "checks": [...], "changes": [...] }`
    pub checks: Value,
    pub summary: String,
    pub status: String,
    pub proposed_by: String,
    pub decided_by: Option<String>,
    pub created_at: String,
    pub decided_at: Option<String>,
    /// `[{ "rule": <AutomationRule>, "base_version": <number> }]`. `base_version` is the rule's
    /// live version when it was proposed, and 0 for a new rule.
    pub rules: Option<Value>,
}

/// What an approval kept.
#[derive(Debug)]
pub struct Approved {
    pub entries: Vec<LedgerEntry>,
    pub rules: Vec<AutomationRule>,
}

#[derive(Debug)]
pub enum Approval {
    Approved(Box<Approved>),
    /// The proposal was already decided. Carries its status.
    NotPending(String),
    /// The live app is no longer at the proposal's base version. The row is now `Superseded`.
    Superseded { live: Option<i32> },
    /// A flagged check failed and the decider is the proposer.
    SecondPersonRequired,
}

const COLUMNS: &str = "id, workspace_id, app_slug, base_version, manifest, checks, summary, status, proposed_by, decided_by, \
     to_char(created_at AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS\"Z\"'), \
     to_char(decided_at AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS\"Z\"'), rules";

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
        rules: r.get(12),
    }
}

/// True when a flagged check failed.
pub fn has_flagged_failure(checks: &Value) -> bool {
    checks["checks"].as_array().is_some_and(|list| {
        list.iter().any(|c| c["outcome"] == "fail" && c["name"].as_str().is_some_and(|n| FLAGGED_CHECKS.contains(&n)))
    })
}

/// The proposed rules as `(rule, base_version)` pairs.
pub fn rule_changes(rules: &Option<Value>) -> Vec<(AutomationRule, u32)> {
    rules
        .as_ref()
        .and_then(|v| v.as_array())
        .map(|list| {
            list.iter()
                .filter_map(|entry| {
                    let rule: AutomationRule = serde_json::from_value(entry["rule"].clone()).ok()?;
                    Some((rule, entry["base_version"].as_u64().unwrap_or(0) as u32))
                })
                .collect()
        })
        .unwrap_or_default()
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
                "INSERT INTO app_proposals (id, workspace_id, app_slug, base_version, manifest, checks, summary, proposed_by, rules) \
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
                &[&p.id, &p.workspace_id, &p.app_slug, &p.base_version, &p.manifest, &p.checks, &p.summary, &p.proposed_by, &p.rules],
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

            // 2. The live app and each live rule must still be at the version the proposal was made against.
            let manifest: Option<AppManifest> = match &p.manifest {
                Some(m) => Some(serde_json::from_value(m.clone())?),
                None => None,
            };
            let mut stale = false;
            let mut live: Option<i32> = None;
            if manifest.is_some() {
                live = tx
                    .query_opt("SELECT version FROM app_manifests WHERE slug = $1 FOR UPDATE", &[&p.app_slug])?
                    .map(|r| r.get(0));
                stale = live != p.base_version;
            }
            let changes = rule_changes(&p.rules);
            for (rule, base) in &changes {
                if rule_live_version_in_tx(&mut tx, &rule.id)? != *base {
                    stale = true;
                }
            }
            if stale {
                tx.execute("UPDATE app_proposals SET status = 'Superseded' WHERE id = $1", &[&id])?;
                tx.commit()?;
                return Ok(Approval::Superseded { live });
            }

            // 3. A flagged change needs a second person.
            if has_flagged_failure(&p.checks) && p.proposed_by.eq_ignore_ascii_case(&decider) {
                return Ok(Approval::SecondPersonRequired);
            }

            let mut entries = Vec::new();

            // 4 and 5. The manifest: its ledger entry, then the write at one version up.
            if let (Some(manifest), Some(raw)) = (&manifest, &p.manifest) {
                entries.push(append_ledger_in_tx(
                    &mut tx,
                    AppendDecisionParams {
                        principal: decider.clone(),
                        organization_code: manifest.organization_code.clone(),
                        app_slug: Some(p.app_slug.clone()),
                        decision_type: DecisionType::AppPublished,
                        oscal_control_id: "CM-03".to_string(),
                        rationale: rationale.clone(),
                        payload: json!({
                            "proposal_id": p.id.to_string(),
                            "manifest_sha256": manifest_sha256(raw),
                            "base_version": p.base_version,
                        }),
                    },
                )?);
                sync_indexes_in_tx(&mut tx, manifest, p.base_version, &decider)?;
                sync_calcs_in_tx(&mut tx, manifest, p.base_version, &decider)?;
                write_manifest_in_tx(&mut tx, manifest, raw, p.base_version)?;
            }

            // Each rule: its own ledger entry, a version row, and the live row.
            let mut saved_rules = Vec::new();
            for (rule, _) in changes {
                let (saved, entry) = save_rule_in_tx(&mut tx, rule, &p.summary, Some(p.id), &decider, "institutional")?;
                entries.push(entry);
                saved_rules.push(saved);
            }

            // 6. The decision.
            tx.execute(
                "UPDATE app_proposals SET status = 'Approved', decided_by = $2, decided_at = NOW() WHERE id = $1",
                &[&id, &decider],
            )?;
            tx.commit()?;
            Ok(Approval::Approved(Box::new(Approved { entries, rules: saved_rules })))
        })
    }
}

/// The approved manifest indexes different fields than the live one. Drop the rows of the fields
/// that left, and queue the build of the fields that joined, in the approval's transaction.
fn sync_indexes_in_tx(tx: &mut postgres::Transaction<'_>, new: &AppManifest, base_version: Option<i32>, requested_by: &str) -> Result<(), RepositoryError> {
    let old: Option<AppManifest> = match base_version {
        Some(_) => tx
            .query_opt("SELECT manifest FROM app_manifests WHERE slug = $1", &[&new.slug])?
            .and_then(|r| serde_json::from_value(r.get::<_, Value>(0)).ok()),
        None => None,
    };
    for table in &new.tables {
        let now = scaffoldry_engine::indexed_fields(new, &table.id);
        let before = old.as_ref().map(|m| scaffoldry_engine::indexed_fields(m, &table.id)).unwrap_or_default();
        let left: Vec<String> = before.difference(&now).cloned().collect();
        let joined: Vec<scaffoldry_engine::FieldSpec> = table.fields.iter().filter(|f| now.contains(&f.name) && !before.contains(&f.name)).cloned().collect();
        if !left.is_empty() || !joined.is_empty() {
            crate::repository_index::sync_index(tx, &new.slug, &table.id, &left, &joined, requested_by)?;
        }
    }
    Ok(())
}

/// The approved manifest changes what a computed field holds, or a field's type. Queue the jobs that
/// write the stored values again, in the approval's transaction.
fn sync_calcs_in_tx(tx: &mut postgres::Transaction<'_>, new: &AppManifest, base_version: Option<i32>, requested_by: &str) -> Result<(), RepositoryError> {
    let old: Option<AppManifest> = match base_version {
        Some(_) => tx
            .query_opt("SELECT manifest FROM app_manifests WHERE slug = $1", &[&new.slug])?
            .and_then(|r| serde_json::from_value(r.get::<_, Value>(0)).ok()),
        None => None,
    };
    crate::repository_calc::sync_calcs_in_tx(tx, old.as_ref(), new, requested_by)
}

fn write_manifest_in_tx(
    tx: &mut postgres::Transaction<'_>,
    manifest: &AppManifest,
    raw: &Value,
    base_version: Option<i32>,
) -> Result<i32, RepositoryError> {
    let version = match base_version {
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
                    raw,
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
                    raw,
                ],
            )?
            .get(0),
    };
    Ok(version)
}

fn rule_live_version_in_tx(tx: &mut postgres::Transaction<'_>, rule_id: &str) -> Result<u32, RepositoryError> {
    let v: i32 = tx
        .query_one("SELECT COALESCE(MAX(version), 0) FROM workflow_rule_versions WHERE rule_id = $1", &[&rule_id])?
        .get(0);
    Ok(v as u32)
}

/// Saves one version of a rule inside the caller's transaction: the ledger entry, the version row,
/// and the live row. The ledger entry names the rule, the new version, the SHA-256 of the rule, and
/// the proposal. Returns the rule as saved, with its version.
pub(crate) fn save_rule_in_tx(
    tx: &mut postgres::Transaction<'_>,
    mut rule: AutomationRule,
    summary: &str,
    proposal_id: Option<uuid::Uuid>,
    changed_by: &str,
    organization_code: &str,
) -> Result<(AutomationRule, LedgerEntry), RepositoryError> {
    rule.version = rule_live_version_in_tx(tx, &rule.id)? + 1;
    let rule_json = serde_json::to_value(&rule)?;
    let entry = append_ledger_in_tx(
        tx,
        AppendDecisionParams {
            principal: changed_by.to_string(),
            organization_code: organization_code.to_string(),
            app_slug: Some(rule.app_slug.clone()),
            decision_type: DecisionType::ProcessDefinitionChanged,
            oscal_control_id: "CM-03".to_string(),
            rationale: summary.to_string(),
            payload: json!({
                "rule_id": rule.id,
                "version": rule.version,
                "rule_sha256": manifest_sha256(&rule_json),
                "proposal_id": proposal_id.map(|p| p.to_string()),
            }),
        },
    )?;
    let version = rule.version as i32;
    tx.execute(
        "INSERT INTO workflow_rule_versions (rule_id, version, app_slug, rule_json, summary, proposal_id, changed_by) \
         VALUES ($1, $2, $3, $4, $5, $6, $7)",
        &[&rule.id, &version, &rule.app_slug, &rule_json, &summary, &proposal_id, &changed_by],
    )?;
    tx.execute(
        "INSERT INTO workflow_automations (id, app_slug, name, rule_json, enabled, updated_at) \
         VALUES ($1, $2, $3, $4, $5, NOW()) \
         ON CONFLICT (id) DO UPDATE SET app_slug = EXCLUDED.app_slug, name = EXCLUDED.name, \
            rule_json = EXCLUDED.rule_json, enabled = EXCLUDED.enabled, updated_at = NOW()",
        &[&rule.id, &rule.app_slug, &rule.name, &rule_json, &rule.enabled],
    )?;
    Ok((rule, entry))
}

impl PostgresRepository {
    /// The live version of a rule: 0 when no version was ever saved.
    pub fn rule_live_version(&self, rule_id: &str) -> Result<u32, RepositoryError> {
        let id = rule_id.to_string();
        self.with_client(move |client| {
            let v: i32 = client
                .query_one("SELECT COALESCE(MAX(version), 0) FROM workflow_rule_versions WHERE rule_id = $1", &[&id])?
                .get(0);
            Ok(v as u32)
        })
    }

    /// A saved version of a rule, as it was.
    pub fn get_rule_version(&self, rule_id: &str, version: u32) -> Result<Option<AutomationRule>, RepositoryError> {
        let (id, version) = (rule_id.to_string(), version as i32);
        self.with_client(move |client| {
            let row = client.query_opt(
                "SELECT rule_json FROM workflow_rule_versions WHERE rule_id = $1 AND version = $2",
                &[&id, &version],
            )?;
            match row {
                Some(r) => Ok(Some(serde_json::from_value(r.get(0))?)),
                None => Ok(None),
            }
        })
    }

    /// Saves one version of a rule in its own transaction. Called from approval, and from the
    /// administrator's enable and disable.
    pub fn save_rule(
        &self,
        rule: &AutomationRule,
        summary: &str,
        proposal_id: Option<uuid::Uuid>,
        changed_by: &str,
        organization_code: &str,
    ) -> Result<(AutomationRule, LedgerEntry), RepositoryError> {
        let (rule, summary, by, org) = (rule.clone(), summary.to_string(), changed_by.to_string(), organization_code.to_string());
        self.with_client(move |client| {
            let mut tx = client.transaction()?;
            let saved = save_rule_in_tx(&mut tx, rule, &summary, proposal_id, &by, &org)?;
            tx.commit()?;
            Ok(saved)
        })
    }

    pub fn count_pending_proposals(&self) -> Result<i64, RepositoryError> {
        self.with_client(|client| Ok(client.query_one("SELECT count(*) FROM app_proposals WHERE status = 'Pending'", &[])?.get(0)))
    }
}
