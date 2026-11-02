//! Lifecycle phase 3: retention policies and the one place a record's data is destroyed.
//!
//! Delete is trash. A record leaves the database only here, in `destroy`, and only when its app's retention policy no
//! longer keeps it. Each run for an app writes the ledger entry `RecordsPurged` first, then notes the ids it will destroy,
//! then destroys them in batches of 1,000, then writes a certificate: the counts and the SHA-256 of the sorted ids, one per
//! line, which proves which records went without keeping them. A run that stops resumes from its notes, and the notes go
//! in the same transaction as the certificate. The disclosure log is never touched: the fact of a disclosure outlives the
//! record. Legal holds are lifecycle phase 4, so nothing is held yet and `held_skipped` is 0.

use crate::jobs::{JobCtx, JobError};
use crate::repository::{AppendDecisionParams, PostgresRepository, RepositoryError};
use scaffoldry_core::ledger::DecisionType;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use uuid::Uuid;

/// The basis of a policy no records officer has set.
pub const PLACEHOLDER_BASIS: &str = "Placeholder. Set by the records officer.";
/// The workspace classifications. Each has a policy from boot.
pub const CLASSIFICATIONS: [&str; 4] = ["Public", "Internal", "Restricted", "FERPA Sensitive"];
/// Records destroyed per transaction.
pub const BATCH: i64 = 1_000;
const ACTOR: &str = "retention@scaffoldry.internal";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Policy {
    #[serde(default)]
    pub id: Option<Uuid>,
    pub name: String,
    pub scope_kind: String,
    #[serde(default)]
    pub classification: Option<String>,
    #[serde(default)]
    pub workspace_id: Option<String>,
    pub keep_trash_days: i32,
    #[serde(default)]
    pub keep_history_days: Option<i32>,
    pub keep_retired_days: i32,
    #[serde(default)]
    pub max_record_age_days: Option<i32>,
    #[serde(default = "trash")]
    pub on_expiry: String,
    pub basis: String,
    #[serde(default = "yes")]
    pub active: bool,
    #[serde(default)]
    pub created_by: Option<String>,
    #[serde(default)]
    pub created_at: Option<String>,
}

fn trash() -> String {
    "trash".to_string()
}
fn yes() -> bool {
    true
}

impl Policy {
    /// What a policy may not be. A policy that purges live records needs a real basis.
    pub fn problems(&self) -> Vec<String> {
        let mut out = Vec::new();
        if self.name.trim().is_empty() || self.name.chars().count() > 120 {
            out.push("'name' must be 1 to 120 characters".to_string());
        }
        match self.scope_kind.as_str() {
            "classification" if self.classification.as_deref().is_some_and(|c| CLASSIFICATIONS.contains(&c)) => {}
            "classification" => out.push(format!("'classification' must be one of {}", CLASSIFICATIONS.join(", "))),
            "workspace" if self.workspace_id.as_deref().is_some_and(|w| !w.trim().is_empty()) => {}
            "workspace" => out.push("'workspace_id' is required for a workspace policy".to_string()),
            _ => out.push("'scope_kind' must be classification or workspace".to_string()),
        }
        let days = [Some(self.keep_trash_days), self.keep_history_days, Some(self.keep_retired_days), self.max_record_age_days];
        if days.iter().flatten().any(|d| *d < 0) {
            out.push("a number of days cannot be negative".to_string());
        }
        if self.basis.trim().is_empty() {
            out.push("'basis' must name the schedule or rule the policy follows".to_string());
        }
        match self.on_expiry.as_str() {
            "trash" => {}
            "purge" if self.basis.trim().is_empty() || self.basis.trim() == PLACEHOLDER_BASIS => {
                out.push("'on_expiry' purge needs a 'basis' that a records officer set, not the placeholder".to_string())
            }
            "purge" => {}
            _ => out.push("'on_expiry' must be trash or purge".to_string()),
        }
        out
    }
}

/// What a run destroys, or would.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Counts {
    pub records: i64,
    pub history: i64,
    pub files: i64,
    pub comments: i64,
    /// Retired fields whose values are stripped.
    pub fields: i64,
    /// Live records past `max_record_age_days` moved to trash. Not destroyed.
    pub trashed: i64,
    pub held: i64,
}

impl Counts {
    fn is_empty(&self) -> bool {
        self.records + self.history + self.files + self.comments + self.fields + self.trashed == 0
    }
}

/// The plan for one app: the counts, the ids to destroy, and what else changes.
struct Plan {
    counts: Counts,
    destroy: Vec<String>,
    age_to_trash: Vec<String>,
    /// `(table_id, field)` of each retired field to strip.
    strip: Vec<(String, String)>,
}

const COLUMNS: &str = "id, name, scope_kind, classification, workspace_id, keep_trash_days, keep_history_days, keep_retired_days, \
                       max_record_age_days, on_expiry, basis, active, created_by, to_char(created_at AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS\"Z\"')";

fn policy_of(r: &postgres::Row) -> Policy {
    Policy {
        id: Some(r.get(0)),
        name: r.get(1),
        scope_kind: r.get(2),
        classification: r.get(3),
        workspace_id: r.get(4),
        keep_trash_days: r.get(5),
        keep_history_days: r.get(6),
        keep_retired_days: r.get(7),
        max_record_age_days: r.get(8),
        on_expiry: r.get(9),
        basis: r.get(10),
        active: r.get(11),
        created_by: Some(r.get(12)),
        created_at: Some(r.get(13)),
    }
}

impl PostgresRepository {
    /// With no policies at all, one placeholder per classification: trash kept 365 days, history kept, retired values
    /// kept 365 days, no age limit.
    pub fn seed_retention_policies(&self) -> Result<(), RepositoryError> {
        self.with_client(|c| {
            let n: i64 = c.query_one("SELECT count(*) FROM retention_policies", &[])?.get(0);
            if n == 0 {
                for class in CLASSIFICATIONS {
                    c.execute(
                        "INSERT INTO retention_policies (id, name, scope_kind, classification, keep_trash_days, keep_retired_days, basis, created_by) \
                         VALUES ($1, $2, 'classification', $3, 365, 365, $4, 'system')",
                        &[&Uuid::new_v4(), &format!("{class} records"), &class, &PLACEHOLDER_BASIS],
                    )?;
                }
            }
            Ok(())
        })
    }

    pub fn list_retention_policies(&self) -> Result<Vec<Policy>, RepositoryError> {
        self.with_client(|c| Ok(c.query(&format!("SELECT {COLUMNS} FROM retention_policies ORDER BY scope_kind, classification, workspace_id, created_at"), &[])?.iter().map(policy_of).collect()))
    }

    pub fn get_retention_policy(&self, id: Uuid) -> Result<Option<Policy>, RepositoryError> {
        self.with_client(move |c| Ok(c.query_opt(&format!("SELECT {COLUMNS} FROM retention_policies WHERE id = $1"), &[&id])?.as_ref().map(policy_of)))
    }

    /// Inserts a policy, or replaces the one with its id. `Err(Conflict)` when another active policy has the scope.
    pub fn save_retention_policy(&self, p: &Policy, by: &str) -> Result<Uuid, RepositoryError> {
        let (p, by) = (p.clone(), by.to_string());
        self.with_client(move |c| {
            let id = p.id.unwrap_or_else(Uuid::new_v4);
            let classification = if p.scope_kind == "classification" { p.classification.clone() } else { None };
            let workspace = if p.scope_kind == "workspace" { p.workspace_id.clone() } else { None };
            let r = c.execute(
                "INSERT INTO retention_policies (id, name, scope_kind, classification, workspace_id, keep_trash_days, keep_history_days, keep_retired_days, \
                     max_record_age_days, on_expiry, basis, active, created_by) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13) \
                 ON CONFLICT (id) DO UPDATE SET name = EXCLUDED.name, scope_kind = EXCLUDED.scope_kind, classification = EXCLUDED.classification, \
                     workspace_id = EXCLUDED.workspace_id, keep_trash_days = EXCLUDED.keep_trash_days, keep_history_days = EXCLUDED.keep_history_days, \
                     keep_retired_days = EXCLUDED.keep_retired_days, max_record_age_days = EXCLUDED.max_record_age_days, on_expiry = EXCLUDED.on_expiry, \
                     basis = EXCLUDED.basis, active = EXCLUDED.active",
                &[&id, &p.name.trim(), &p.scope_kind, &classification, &workspace, &p.keep_trash_days, &p.keep_history_days, &p.keep_retired_days, &p.max_record_age_days, &p.on_expiry, &p.basis.trim(), &p.active, &by],
            );
            match r {
                Ok(_) => Ok(id),
                Err(e) if e.code() == Some(&postgres::error::SqlState::UNIQUE_VIOLATION) => {
                    Err(RepositoryError::Conflict("Another active policy already covers this classification or workspace".to_string()))
                }
                Err(e) => Err(e.into()),
            }
        })
    }

    pub fn list_purge_certificates(&self, limit: i64) -> Result<Vec<Value>, RepositoryError> {
        self.with_client(move |c| Ok(c.query(&format!("{CERT} ORDER BY run_at DESC LIMIT $1"), &[&limit])?.iter().map(cert_of).collect()))
    }

    pub fn get_purge_certificate(&self, id: Uuid) -> Result<Option<Value>, RepositoryError> {
        self.with_client(move |c| Ok(c.query_opt(&format!("{CERT} WHERE id = $1"), &[&id])?.as_ref().map(cert_of)))
    }
}

const CERT: &str = "SELECT id, to_char(run_at AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS\"Z\"'), policy_id, app_slug, counts, ids_sha256, held_skipped FROM purge_certificates";

fn cert_of(r: &postgres::Row) -> Value {
    json!({
        "id": r.get::<_, Uuid>(0), "run_at": r.get::<_, String>(1), "policy_id": r.get::<_, Option<Uuid>>(2), "app_slug": r.get::<_, String>(3),
        "counts": r.get::<_, Value>(4), "ids_sha256": r.get::<_, String>(5).trim(), "held_skipped": r.get::<_, i32>(6),
        "hash_of": "SHA-256 of the destroyed record ids, sorted, one per line",
    })
}

/// Each app with the policy that covers it: its workspace's, or else its workspace classification's.
fn covered_apps(c: &mut postgres::Client, overriding: Option<&Policy>) -> Result<Vec<(String, Policy)>, RepositoryError> {
    let policies: Vec<Policy> = c.query(&format!("SELECT {COLUMNS} FROM retention_policies WHERE active"), &[])?.iter().map(policy_of).collect();
    let apps = c.query(
        "SELECT m.slug, m.manifest->>'workspace_id', w.data_classification FROM app_manifests m LEFT JOIN workspaces w ON w.id = m.manifest->>'workspace_id' ORDER BY m.slug",
        &[],
    )?;
    let mut out = Vec::new();
    for r in apps {
        let (slug, ws, class): (String, Option<String>, Option<String>) = (r.get(0), r.get(1), r.get(2));
        let covers = |p: &&Policy| (p.scope_kind == "workspace" && p.workspace_id == ws) || (p.scope_kind == "classification" && p.classification == class);
        // A proposed policy in a preview stands in for the one with its scope.
        let proposed = overriding.filter(|p| covers(p));
        let by_ws = policies.iter().find(|p| p.scope_kind == "workspace" && p.workspace_id == ws);
        let chosen = match (proposed, by_ws) {
            (Some(p), _) if p.scope_kind == "workspace" => Some(p.clone()),
            (_, Some(w)) => Some(w.clone()),
            (Some(p), None) => Some(p.clone()),
            (None, None) => policies.iter().find(|p| p.scope_kind == "classification" && p.classification == class).cloned(),
        };
        if let Some(p) = chosen {
            out.push((slug, p));
        }
    }
    Ok(out)
}

fn days_ago(d: i32) -> chrono::DateTime<chrono::Utc> {
    chrono::Utc::now() - chrono::Duration::days(i64::from(d))
}

/// What the policy would do to one app now. Reads only.
fn plan(c: &mut postgres::Client, slug: &str, p: &Policy) -> Result<Plan, RepositoryError> {
    let mut destroy: Vec<String> = c
        .query("SELECT id FROM dataset_records WHERE app_slug = $1 AND deleted_at IS NOT NULL AND deleted_at < NOW() - make_interval(days => $2)", &[&slug, &p.keep_trash_days])?
        .iter()
        .map(|r| r.get(0))
        .collect();
    let mut age_to_trash = Vec::new();
    if let Some(max) = p.max_record_age_days {
        let aged: Vec<String> = c
            .query("SELECT id FROM dataset_records WHERE app_slug = $1 AND deleted_at IS NULL AND created_at < NOW() - make_interval(days => $2)", &[&slug, &max])?
            .iter()
            .map(|r| r.get(0))
            .collect();
        if p.on_expiry == "purge" {
            destroy.extend(aged);
        } else {
            age_to_trash = aged;
        }
    }
    destroy.sort();
    destroy.dedup();
    let one = |c: &mut postgres::Client, sql: &str, ids: &Vec<String>| -> Result<i64, RepositoryError> { Ok(c.query_one(sql, &[&slug, ids])?.get(0)) };
    let mut history = one(c, "SELECT count(*) FROM record_history WHERE app_slug = $1 AND record_id = ANY($2)", &destroy)?;
    if let Some(keep) = p.keep_history_days {
        history += c.query_one(
            "SELECT count(*) FROM record_history WHERE app_slug = $1 AND NOT (record_id = ANY($2)) AND at < NOW() - make_interval(days => $3)",
            &[&slug, &destroy, &keep],
        )?.get::<_, i64>(0);
    }
    let files = one(c, "SELECT count(*) FROM file_refs WHERE app_slug = $1 AND record_id = ANY($2)", &destroy)?;
    let comments = one(c, "SELECT count(*) FROM record_comments WHERE app_slug = $1 AND record_id = ANY($2)", &destroy)?;
    let manifest: Option<Value> = c.query_opt("SELECT manifest FROM app_manifests WHERE slug = $1", &[&slug])?.map(|r| r.get(0));
    let cutoff = days_ago(p.keep_retired_days);
    let mut strip = Vec::new();
    for t in manifest.as_ref().and_then(|m| m["tables"].as_array()).into_iter().flatten() {
        for f in t["retired_fields"].as_array().into_iter().flatten() {
            let at = f["retired_at"].as_str().and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok());
            if at.is_some_and(|at| at < cutoff) {
                strip.push((t["id"].as_str().unwrap_or("").to_string(), f["name"].as_str().unwrap_or("").to_string()));
            }
        }
    }
    let counts = Counts { records: destroy.len() as i64, history, files, comments, fields: strip.len() as i64, trashed: age_to_trash.len() as i64, held: 0 };
    Ok(Plan { counts, destroy, age_to_trash, strip })
}

/// Destroys records and everything that is theirs: index rows, link rows, remembered links, comments, history and file
/// references. The disclosure log keeps its entries. The only place these rows are deleted (lifecycle phase 3 test 9).
pub fn destroy(tx: &mut postgres::Transaction<'_>, app_slug: &str, ids: &[String]) -> Result<u64, postgres::Error> {
    let ids = ids.to_vec();
    tx.execute("DELETE FROM record_index WHERE record_id = ANY($1)", &[&ids])?;
    tx.execute("DELETE FROM record_links WHERE from_record = ANY($1) OR to_record = ANY($1)", &[&ids])?;
    tx.execute("DELETE FROM trash_links WHERE record_id = ANY($1) OR other_record = ANY($1)", &[&ids])?;
    tx.execute("DELETE FROM record_comments WHERE app_slug = $1 AND record_id = ANY($2)", &[&app_slug, &ids])?;
    // History is append-only. The trigger lets a delete through only with this setting, for this transaction.
    tx.execute("SELECT set_config('scaffoldry.purge', 'on', true)", &[])?;
    tx.execute("DELETE FROM record_history WHERE app_slug = $1 AND record_id = ANY($2)", &[&app_slug, &ids])?;
    tx.execute("DELETE FROM file_refs WHERE app_slug = $1 AND record_id = ANY($2)", &[&app_slug, &ids])?;
    tx.execute("DELETE FROM dataset_records WHERE app_slug = $1 AND id = ANY($2)", &[&app_slug, &ids])
}

/// Destroys history older than `days` of the records that stay.
fn destroy_old_history(tx: &mut postgres::Transaction<'_>, app_slug: &str, days: i32) -> Result<u64, postgres::Error> {
    tx.execute("SELECT set_config('scaffoldry.purge', 'on', true)", &[])?;
    tx.execute("DELETE FROM record_history WHERE app_slug = $1 AND at < NOW() - make_interval(days => $2)", &[&app_slug, &days])
}

/// Strips retired values from every record of a table and drops the fields from the definition, which can no longer
/// restore them. The definition's version moves, so every server reloads it.
fn strip_retired(tx: &mut postgres::Transaction<'_>, app_slug: &str, strip: &[(String, String)]) -> Result<(), postgres::Error> {
    if strip.is_empty() {
        return Ok(());
    }
    for (table, field) in strip {
        tx.execute("UPDATE dataset_records SET data = data - $3 WHERE app_slug = $1 AND table_id = $2 AND data ? $3", &[&app_slug, table, field])?;
    }
    let Some(row) = tx.query_opt("SELECT manifest FROM app_manifests WHERE slug = $1 FOR UPDATE", &[&app_slug])? else { return Ok(()) };
    let mut manifest: Value = row.get(0);
    for t in manifest["tables"].as_array_mut().into_iter().flatten() {
        let id = t["id"].as_str().unwrap_or("").to_string();
        if let Some(list) = t["retired_fields"].as_array_mut() {
            list.retain(|f| !strip.iter().any(|(tid, name)| *tid == id && f["name"].as_str() == Some(name.as_str())));
        }
    }
    tx.execute("UPDATE app_manifests SET manifest = $2, version = version + 1, updated_at = NOW() WHERE slug = $1", &[&app_slug, &manifest])?;
    tx.execute("UPDATE cache_versions SET version = version + 1 WHERE name = 'manifests'", &[])?;
    Ok(())
}

fn sha_of_sorted(mut ids: Vec<String>) -> String {
    ids.sort();
    Sha256::digest(ids.join("\n").as_bytes()).iter().map(|b| format!("{b:02x}")).collect()
}

fn ledger(repo: &PostgresRepository, slug: &str, policy: &Policy, counts: &Counts) -> Result<(), String> {
    repo.append_ledger_decision(AppendDecisionParams {
        principal: ACTOR.to_string(),
        organization_code: String::new(),
        app_slug: Some(slug.to_string()),
        decision_type: DecisionType::RecordsPurged,
        oscal_control_id: "SI-12".to_string(),
        rationale: format!("Retention policy '{}' ({}) no longer keeps these", policy.name, policy.basis),
        payload: json!({ "policy_id": policy.id, "counts": counts }),
    })
    .map(|_| ())
    .map_err(|e| format!("The ledger could not record the purge, so nothing was destroyed: {e}"))
}

/// One pass over every app. `max_batches` stops after that many batches of destruction, as a run that dies does; the
/// next run resumes. Returns the counts of the apps it finished.
pub fn run(repo: &PostgresRepository, max_batches: Option<usize>) -> Result<Value, String> {
    let apps = repo.with_client(|c| covered_apps(c, None)).map_err(|e| e.to_string())?;
    let mut budget = max_batches;
    let mut finished = Vec::new();
    for (slug, policy) in apps {
        let open: Option<(Uuid, Value)> = repo
            .with_client({
                let slug = slug.clone();
                move |c| Ok(c.query_opt("SELECT id, counts FROM purge_runs WHERE app_slug = $1", &[&slug])?.map(|r| (r.get(0), r.get(1))))
            })
            .map_err(|e| e.to_string())?;
        let (run_id, counts) = match open {
            Some(found) => found,
            None => {
                let p = repo.with_client({
                    let (slug, policy) = (slug.clone(), policy.clone());
                    move |c| plan(c, &slug, &policy)
                });
                let p = p.map_err(|e| e.to_string())?;
                if p.counts.is_empty() {
                    continue;
                }
                // The ledger first. Without its entry, nothing below happens.
                ledger(repo, &slug, &policy, &p.counts)?;
                let run_id = Uuid::new_v4();
                let counts = serde_json::to_value(&p.counts).unwrap_or(Value::Null);
                let (s, pol, c2) = (slug.clone(), policy.clone(), counts.clone());
                repo.with_client(move |c| {
                    let mut tx = c.transaction()?;
                    tx.execute("INSERT INTO purge_runs (id, app_slug, policy_id, counts) VALUES ($1, $2, $3, $4)", &[&run_id, &s, &pol.id, &c2])?;
                    tx.execute("INSERT INTO purge_pending (run_id, record_id) SELECT $1, unnest($2::text[])", &[&run_id, &p.destroy])?;
                    let ctx = crate::repository::WriteCtx::new(ACTOR, crate::repository::ActorKind::System);
                    for id in &p.age_to_trash {
                        crate::repository::delete_record_in_tx(&mut tx, &s, id, None, &ctx)?;
                    }
                    if let Some(days) = pol.keep_history_days {
                        destroy_old_history(&mut tx, &s, days)?;
                    }
                    strip_retired(&mut tx, &s, &p.strip)?;
                    tx.commit()?;
                    Ok(())
                })
                .map_err(|e| e.to_string())?;
                (run_id, counts)
            }
        };
        // Destroy in batches. Each batch is its own transaction and marks its ids done in it.
        loop {
            if budget == Some(0) {
                return Ok(json!({ "finished": finished, "stopped": true }));
            }
            let s = slug.clone();
            let more = repo
                .with_client(move |c| {
                    let ids: Vec<String> = c.query("SELECT record_id FROM purge_pending WHERE run_id = $1 AND NOT done ORDER BY record_id LIMIT $2", &[&run_id, &BATCH])?.iter().map(|r| r.get(0)).collect();
                    if ids.is_empty() {
                        return Ok(false);
                    }
                    let mut tx = c.transaction()?;
                    destroy(&mut tx, &s, &ids)?;
                    tx.execute("UPDATE purge_pending SET done = TRUE WHERE run_id = $1 AND record_id = ANY($2)", &[&run_id, &ids])?;
                    tx.commit()?;
                    Ok(true)
                })
                .map_err(|e| e.to_string())?;
            if !more {
                break;
            }
            if let Some(b) = budget.as_mut() {
                *b -= 1;
            }
        }
        // The certificate, and the notes go, in one transaction.
        let (s, pol, c2) = (slug.clone(), policy.id, counts.clone());
        repo.with_client(move |c| {
            let mut tx = c.transaction()?;
            let ids: Vec<String> = tx.query("SELECT record_id FROM purge_pending WHERE run_id = $1", &[&run_id])?.iter().map(|r| r.get(0)).collect();
            tx.execute(
                "INSERT INTO purge_certificates (id, policy_id, app_slug, counts, ids_sha256, held_skipped) VALUES ($1, $2, $3, $4, $5, 0)",
                &[&Uuid::new_v4(), &pol, &s, &c2, &sha_of_sorted(ids)],
            )?;
            tx.execute("DELETE FROM purge_runs WHERE id = $1", &[&run_id])?;
            tx.commit()?;
            Ok(())
        })
        .map_err(|e| e.to_string())?;
        finished.push(json!({ "app_slug": slug, "counts": counts }));
    }
    Ok(json!({ "finished": finished, "stopped": false }))
}

/// The nightly job.
pub fn purge_expired(ctx: &JobCtx, _payload: Value) -> Result<Value, JobError> {
    run(ctx.repository(), None).map_err(JobError::Retry)
}

/// Counts, per app, what the next run would destroy under a proposed policy. Destroys nothing.
pub fn retention_preview(ctx: &JobCtx, payload: Value) -> Result<Value, JobError> {
    let policy: Policy = serde_json::from_value(payload.get("policy").cloned().unwrap_or(Value::Null)).map_err(|e| JobError::Permanent(format!("The policy is not a policy: {e}")))?;
    let repo = ctx.repository();
    let apps = repo.with_client(move |c| covered_apps(c, Some(&policy))).map_err(|e| JobError::Retry(e.to_string()))?;
    let mut out = Vec::new();
    for (slug, p) in apps {
        let s = slug.clone();
        let plan = repo.with_client(move |c| plan(c, &s, &p)).map_err(|e| JobError::Retry(e.to_string()))?;
        let mut row = serde_json::to_value(&plan.counts).unwrap_or(Value::Null);
        row["app_slug"] = json!(slug);
        out.push(row);
    }
    Ok(json!({ "apps": out }))
}

/// How long a reference keeps its blob after it was made, even once no record holds it (attachments phase 4).
pub const ORPHAN_GRACE_HOURS: i32 = 24;

/// Deletes the blobs no live reference holds and no reference made in the last 24 hours holds, with the references that
/// let go of them. The ledger entry `BlobsPurged` comes first and holds the count only: a file's name can say whose it
/// was. Legal holds are lifecycle phase 4, so nothing is held yet.
pub fn purge_orphan_blobs(ctx: &JobCtx, _payload: Value) -> Result<Value, JobError> {
    let repo = ctx.repository();
    let orphans: Vec<String> = repo
        .with_client(|c| {
            Ok(c.query(
                "SELECT f.sha256 FROM files f WHERE NOT EXISTS (SELECT 1 FROM file_refs r WHERE r.sha256 = f.sha256 \
                   AND (r.deleted_at IS NULL OR r.uploaded_at > NOW() - make_interval(hours => $1))) \
                   AND NOT EXISTS (SELECT 1 FROM app_snapshots s WHERE s.file_sha256 = f.sha256)",
                &[&ORPHAN_GRACE_HOURS],
            )?
            .iter()
            .map(|r| r.get::<_, String>(0))
            .collect())
        })
        .map_err(|e| JobError::Retry(e.to_string()))?;
    if orphans.is_empty() {
        return Ok(json!({ "deleted": 0 }));
    }
    repo.append_ledger_decision(AppendDecisionParams {
        principal: ACTOR.to_string(),
        organization_code: String::new(),
        app_slug: None,
        decision_type: DecisionType::BlobsPurged,
        oscal_control_id: "MP-6".to_string(),
        rationale: "Stored files that no record refers to any more".to_string(),
        payload: json!({ "count": orphans.len() }),
    })
    .map_err(|e| JobError::Retry(format!("The ledger could not record the purge, so nothing was deleted: {e}")))?;
    let gone: Vec<String> = repo
        .with_client(move |c| {
            let mut tx = c.transaction()?;
            // Checked again in the transaction: a reference made since the list was read keeps its blob.
            tx.execute(
                "DELETE FROM file_refs r WHERE r.sha256 = ANY($1) AND r.deleted_at IS NOT NULL AND NOT EXISTS (SELECT 1 FROM file_refs l WHERE l.sha256 = r.sha256 \
                   AND (l.deleted_at IS NULL OR l.uploaded_at > NOW() - make_interval(hours => $2)))",
                &[&orphans, &ORPHAN_GRACE_HOURS],
            )?;
            let gone = tx
                .query(
                    "DELETE FROM files f WHERE f.sha256 = ANY($1) AND NOT EXISTS (SELECT 1 FROM file_refs r WHERE r.sha256 = f.sha256) \
                     AND NOT EXISTS (SELECT 1 FROM app_snapshots s WHERE s.file_sha256 = f.sha256) RETURNING sha256",
                    &[&orphans],
                )?
                .iter()
                .map(|r| r.get::<_, String>(0).trim().to_string())
                .collect();
            tx.commit()?;
            Ok(gone)
        })
        .map_err(|e| JobError::Retry(e.to_string()))?;
    if let Some(dir) = crate::files::dir() {
        for sha in &gone {
            let _ = std::fs::remove_file(crate::files::blob_path(&dir, sha));
        }
    }
    Ok(json!({ "deleted": gone.len() }))
}
