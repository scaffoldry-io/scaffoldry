//! Admin console phase 5: stored policy versions, their test cases, and the OSCAL documents an administrator edits.
//!
//! Switching a version on, the ledger entry that records it and the cache counter commit together. A server that reads the
//! counter after the commit loads the new policy, and a failed write leaves the old one running.

use crate::repository::{append_ledger_in_tx, AppendDecisionParams, PostgresRepository, RepositoryError};
use scaffoldry_core::LedgerEntry;
use scaffoldry_policy::cases::PolicyCase;
use serde::Serialize;
use serde_json::Value;
use uuid::Uuid;

const STAMP: &str = "'YYYY-MM-DD\"T\"HH24:MI:SS\"Z\"'";

#[derive(Debug, Clone, Serialize)]
pub struct PolicyVersionRow {
    pub id: Uuid,
    pub version: i32,
    pub source: String,
    pub status: String,
    pub note: String,
    pub created_by: String,
    pub created_at: String,
    pub activated_by: Option<String>,
    pub activated_at: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct StoredCase {
    pub id: Uuid,
    pub name: String,
    pub principal: Value,
    pub action: String,
    pub resource: Value,
    pub expected: String,
    pub created_by: String,
    pub created_at: String,
}

impl StoredCase {
    pub fn as_case(&self) -> PolicyCase {
        PolicyCase { name: self.name.clone(), principal: self.principal.clone(), action: self.action.clone(), resource: self.resource.clone(), expected: self.expected.clone() }
    }
}

fn select_version() -> String {
    format!(
        "SELECT id, version, source, status, note, created_by, to_char(created_at AT TIME ZONE 'UTC', {STAMP}), activated_by, \
         to_char(activated_at AT TIME ZONE 'UTC', {STAMP}) FROM policy_versions"
    )
}

fn version_of(r: &postgres::Row) -> PolicyVersionRow {
    PolicyVersionRow { id: r.get(0), version: r.get(1), source: r.get(2), status: r.get(3), note: r.get(4), created_by: r.get(5), created_at: r.get(6), activated_by: r.get(7), activated_at: r.get(8) }
}

fn case_of(r: &postgres::Row) -> StoredCase {
    StoredCase { id: r.get(0), name: r.get(1), principal: r.get(2), action: r.get(3), resource: r.get(4), expected: r.get(5), created_by: r.get(6), created_at: r.get(7) }
}

const CASE_COLUMNS: &str = "id, name, principal, action, resource, expected, created_by, to_char(created_at AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS\"Z\"')";

/// What `activate_policy` did.
pub struct Activation {
    pub version: i32,
    pub previous: Option<i32>,
    pub entry: LedgerEntry,
}

impl PostgresRepository {
    /// With no version stored, stores version 1 as `Active` from the compiled default, with the default cases. Does nothing
    /// otherwise. Two servers starting together leave one version 1.
    pub fn seed_policy(&self, source: &str, cases: Vec<PolicyCase>) -> Result<(), RepositoryError> {
        let source = source.to_string();
        self.with_client(move |c| {
            let mut tx = c.transaction()?;
            let made = tx.execute(
                "INSERT INTO policy_versions (id, version, source, status, note, created_by, activated_by, activated_at) \
                 SELECT $1, 1, $2, 'Active', 'The policy that ships with Scaffoldry', 'system', 'system', NOW() \
                 WHERE NOT EXISTS (SELECT 1 FROM policy_versions) ON CONFLICT DO NOTHING",
                &[&Uuid::new_v4(), &source],
            )?;
            if made == 1 {
                for case in &cases {
                    tx.execute(
                        "INSERT INTO policy_cases (id, name, principal, action, resource, expected, created_by) VALUES ($1, $2, $3, $4, $5, $6, 'system')",
                        &[&Uuid::new_v4(), &case.name, &case.principal, &case.action, &case.resource, &case.expected],
                    )?;
                }
                crate::cache::bump(&mut tx, "policy")?;
            }
            tx.commit()?;
            Ok(())
        })
    }

    pub fn active_policy(&self) -> Result<Option<PolicyVersionRow>, RepositoryError> {
        self.with_client(|c| Ok(c.query_opt(&format!("{} WHERE status = 'Active'", select_version()), &[])?.as_ref().map(version_of)))
    }

    pub fn get_policy_version(&self, id: Uuid) -> Result<Option<PolicyVersionRow>, RepositoryError> {
        self.with_client(move |c| Ok(c.query_opt(&format!("{} WHERE id = $1", select_version()), &[&id])?.as_ref().map(version_of)))
    }

    /// Newest first.
    pub fn list_policy_versions(&self, limit: i64, offset: i64) -> Result<Vec<PolicyVersionRow>, RepositoryError> {
        self.with_client(move |c| Ok(c.query(&format!("{} ORDER BY version DESC LIMIT $1 OFFSET $2", select_version()), &[&limit, &offset])?.iter().map(version_of).collect()))
    }

    /// Stores a new `Draft` with the next version number.
    pub fn insert_policy_draft(&self, source: &str, note: &str, by: &str) -> Result<PolicyVersionRow, RepositoryError> {
        let (source, note, by) = (source.to_string(), note.to_string(), by.to_string());
        self.with_client(move |c| {
            let mut tx = c.transaction()?;
            tx.execute("LOCK TABLE policy_versions IN EXCLUSIVE MODE", &[])?;
            let id = Uuid::new_v4();
            tx.execute(
                "INSERT INTO policy_versions (id, version, source, status, note, created_by) \
                 SELECT $1, COALESCE(MAX(version), 0) + 1, $2, 'Draft', $3, $4 FROM policy_versions",
                &[&id, &source, &note, &by],
            )?;
            let row = tx.query_one(&format!("{} WHERE id = $1", select_version()), &[&id])?;
            tx.commit()?;
            Ok(version_of(&row))
        })
    }

    /// Makes a `Draft` the `Active` version and retires the one that was. The ledger entry, both rows and the counter commit
    /// together. `ledger` gets the version being switched on and the version number it replaces.
    pub fn activate_policy(&self, id: Uuid, by: &str, ledger: impl FnOnce(&PolicyVersionRow, Option<i32>) -> AppendDecisionParams + Send + 'static) -> Result<Activation, RepositoryError> {
        let by = by.to_string();
        self.with_client(move |c| {
            let mut tx = c.transaction()?;
            tx.execute("LOCK TABLE policy_versions IN EXCLUSIVE MODE", &[])?;
            let Some(target) = tx.query_opt(&format!("{} WHERE id = $1", select_version()), &[&id])?.as_ref().map(version_of) else {
                return Err(RepositoryError::NotFound("No such policy version".to_string()));
            };
            if target.status != "Draft" {
                return Err(RepositoryError::Conflict(format!("Version {} is {}, not a draft. Clone it to make a draft.", target.version, target.status)));
            }
            let previous = tx.query_opt("SELECT id, version FROM policy_versions WHERE status = 'Active'", &[])?.map(|r| (r.get::<_, Uuid>(0), r.get::<_, i32>(1)));
            let entry = append_ledger_in_tx(&mut tx, ledger(&target, previous.map(|p| p.1)))?;
            if let Some((old, _)) = previous {
                tx.execute("UPDATE policy_versions SET status = 'Retired' WHERE id = $1", &[&old])?;
            }
            tx.execute("UPDATE policy_versions SET status = 'Active', activated_by = $2, activated_at = NOW() WHERE id = $1", &[&id, &by])?;
            crate::cache::bump(&mut tx, "policy")?;
            tx.commit()?;
            Ok(Activation { version: target.version, previous: previous.map(|p| p.1), entry })
        })
    }

    pub fn list_policy_cases(&self) -> Result<Vec<StoredCase>, RepositoryError> {
        self.with_client(|c| Ok(c.query(&format!("SELECT {CASE_COLUMNS} FROM policy_cases ORDER BY created_at, name, id"), &[])?.iter().map(case_of).collect()))
    }

    pub fn get_policy_case(&self, id: Uuid) -> Result<Option<StoredCase>, RepositoryError> {
        self.with_client(move |c| Ok(c.query_opt(&format!("SELECT {CASE_COLUMNS} FROM policy_cases WHERE id = $1"), &[&id])?.as_ref().map(case_of)))
    }

    pub fn insert_policy_case(&self, id: Uuid, case: &PolicyCase, by: &str) -> Result<(), RepositoryError> {
        let (case, by) = (case.clone(), by.to_string());
        self.with_client(move |c| {
            c.execute(
                "INSERT INTO policy_cases (id, name, principal, action, resource, expected, created_by) VALUES ($1, $2, $3, $4, $5, $6, $7)",
                &[&id, &case.name, &case.principal, &case.action, &case.resource, &case.expected, &by],
            )?;
            Ok(())
        })
    }

    /// `false` when there was no such case.
    pub fn delete_policy_case(&self, id: Uuid) -> Result<bool, RepositoryError> {
        self.with_client(move |c| Ok(c.execute("DELETE FROM policy_cases WHERE id = $1", &[&id])? == 1))
    }

    pub fn get_governance_document(&self, kind: &str) -> Result<Option<Value>, RepositoryError> {
        let kind = kind.to_string();
        self.with_client(move |c| Ok(c.query_opt("SELECT document FROM governance_documents WHERE kind = $1", &[&kind])?.map(|r| r.get(0))))
    }

    /// Reads a document, changes it, and stores it, while holding a lock on its kind, so two editors never overwrite each
    /// other. `edit` gets the stored document, or `None` when there is none yet.
    pub fn edit_governance_document(&self, kind: &str, by: &str, edit: impl FnOnce(Option<Value>) -> Result<Value, RepositoryError> + Send + 'static) -> Result<Value, RepositoryError> {
        let (kind, by) = (kind.to_string(), by.to_string());
        self.with_client(move |c| {
            let mut tx = c.transaction()?;
            let lock = format!("governance_documents:{kind}");
            tx.execute("SELECT pg_advisory_xact_lock(hashtext($1))", &[&lock])?;
            let held: Option<Value> = tx.query_opt("SELECT document FROM governance_documents WHERE kind = $1", &[&kind])?.map(|r| r.get(0));
            let document = edit(held)?;
            tx.execute(
                "INSERT INTO governance_documents (kind, document, updated_by) VALUES ($1, $2, $3) \
                 ON CONFLICT (kind) DO UPDATE SET document = $2, updated_by = $3, updated_at = NOW()",
                &[&kind, &document, &by],
            )?;
            tx.commit()?;
            Ok(document)
        })
    }

    /// For each control id as the ledger spells it: how many entries name it and the time of the newest.
    pub fn ledger_control_evidence(&self) -> Result<Vec<(String, i64, String)>, RepositoryError> {
        self.with_client(|c| Ok(c.query("SELECT oscal_control_id, COUNT(*), MAX(timestamp_iso) FROM governance_ledger GROUP BY oscal_control_id", &[])?.iter().map(|r| (r.get(0), r.get(1), r.get(2))).collect()))
    }
}
