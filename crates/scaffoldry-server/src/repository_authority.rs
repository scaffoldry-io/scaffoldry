//! Decision authority phase 2: the changes to the matrix and the rows they write. A confirmed change is applied in one
//! transaction, after the service has written its ledger entry.

use crate::repository::{PostgresRepository, RepositoryError};
use scaffoldry_core::authority::FLAGGED;
use serde_json::{json, Value};
use uuid::Uuid;

/// The proposals of one workspace, proposer and week that failed the same flagged checks. The load report reads counts of
/// these and never a proposal's content.
#[derive(Debug, Clone)]
pub struct LoadGroup {
    pub workspace_id: String,
    pub proposed_by: String,
    /// 0 is the last seven days.
    pub weeks_ago: usize,
    pub raised: Vec<String>,
    pub count: i64,
}

/// How long decided proposals took, in seconds. `None` when none were decided in the window.
#[derive(Debug, Clone, Copy)]
pub struct DecisionTimes {
    pub decided: i64,
    pub median: Option<i64>,
    pub p95: Option<i64>,
}

const TS: &str = "'YYYY-MM-DD\"T\"HH24:MI:SS\"Z\"'";

fn ts(col: &str) -> String {
    format!("to_char({col} AT TIME ZONE 'UTC', {TS})")
}

fn select() -> String {
    format!("SELECT id, organization_id, change, reason, status, proposed_by, {}, decided_by, {} FROM authority_changes", ts("proposed_at"), ts("decided_at"))
}

fn change_of(r: &postgres::Row) -> Value {
    json!({
        "id": r.get::<_, Uuid>(0), "organization_id": r.get::<_, Uuid>(1), "change": r.get::<_, Value>(2), "reason": r.get::<_, String>(3),
        "status": r.get::<_, String>(4), "proposed_by": r.get::<_, String>(5), "proposed_at": r.get::<_, String>(6),
        "decided_by": r.get::<_, Option<String>>(7), "decided_at": r.get::<_, Option<String>>(8),
    })
}

impl PostgresRepository {
    pub fn authority_insert_change(&self, organization_id: Uuid, change: &Value, reason: &str, by: &str) -> Result<Value, RepositoryError> {
        let (change, reason, by) = (change.clone(), reason.to_string(), by.to_string());
        self.with_client(move |c| {
            let id = Uuid::new_v4();
            c.execute("INSERT INTO authority_changes (id, organization_id, change, reason, proposed_by) VALUES ($1, $2, $3, $4, $5)", &[&id, &organization_id, &change, &reason, &by])?;
            Ok(change_of(&c.query_one(&format!("{} WHERE id = $1", select()), &[&id])?))
        })
    }

    pub fn authority_change(&self, id: Uuid) -> Result<Option<Value>, RepositoryError> {
        self.with_client(move |c| Ok(c.query_opt(&format!("{} WHERE id = $1", select()), &[&id])?.map(|r| change_of(&r))))
    }

    /// Changes, newest first. `units` limits them to those units; `None` is every unit.
    pub fn authority_changes(&self, units: Option<Vec<Uuid>>) -> Result<Vec<Value>, RepositoryError> {
        self.with_client(move |c| {
            Ok(c.query(&format!("{} WHERE ($1::uuid[] IS NULL OR organization_id = ANY($1)) ORDER BY proposed_at DESC, id LIMIT 200", select()), &[&units])?.iter().map(change_of).collect())
        })
    }

    /// Decides a pending change. On `confirm` it writes the signal or the row in the same transaction. False when the
    /// change was not pending.
    pub fn authority_decide(&self, id: Uuid, confirm: bool, by: &str) -> Result<bool, RepositoryError> {
        let by = by.to_string();
        self.with_client(move |c| {
            let mut tx = c.transaction()?;
            let Some(row) = tx.query_opt("SELECT organization_id, change FROM authority_changes WHERE id = $1 AND status = 'Pending' FOR UPDATE", &[&id])? else { return Ok(false) };
            let (org, change): (Uuid, Value) = (row.get(0), row.get(1));
            if confirm {
                match change["kind"].as_str() {
                    Some("signal") => {
                        let (check, tier) = (change["check_name"].as_str().unwrap_or_default().to_string(), change["tier"].as_str().unwrap_or_default().to_string());
                        tx.execute("INSERT INTO authority_signals (organization_id, check_name, tier) VALUES ($1, $2, $3) ON CONFLICT (organization_id, check_name) DO UPDATE SET tier = EXCLUDED.tier", &[&org, &check, &tier])?;
                    }
                    _ => {
                        let r = &change["row"];
                        let text = |k: &str| r[k].as_str().map(str::to_string);
                        let approver = (!r["approver"].is_null()).then(|| r["approver"].clone());
                        tx.execute(
                            "INSERT INTO authority_rows (id, organization_id, tier, path, approver, control_id, library_policy_key, library_rule_key) VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
                             ON CONFLICT (organization_id, tier) DO UPDATE SET path = EXCLUDED.path, approver = EXCLUDED.approver, control_id = EXCLUDED.control_id, library_policy_key = EXCLUDED.library_policy_key, library_rule_key = EXCLUDED.library_rule_key",
                            &[&Uuid::new_v4(), &org, &text("tier"), &text("path"), &approver, &text("control_id"), &text("library_policy_key"), &text("library_rule_key")],
                        )?;
                    }
                }
            }
            tx.execute("UPDATE authority_changes SET status = $2, decided_by = $3, decided_at = NOW() WHERE id = $1", &[&id, &(if confirm { "Confirmed" } else { "Rejected" }), &by])?;
            tx.commit()?;
            Ok(true)
        })
    }

    /// A matrix row by id: its tier and unit.
    pub fn authority_row(&self, id: Uuid) -> Result<Option<(String, Uuid)>, RepositoryError> {
        self.with_client(move |c| Ok(c.query_opt("SELECT tier, organization_id FROM authority_rows WHERE id = $1", &[&id])?.map(|r| (r.get(0), r.get(1)))))
    }

    /// Proposals made in the last `days` days, counted by workspace, proposer, week and the flagged checks that failed.
    /// One statement. The checks are read in SQL, so no proposal body leaves the database.
    pub fn authority_load_groups(&self, days: i32) -> Result<Vec<LoadGroup>, RepositoryError> {
        let flagged: Vec<String> = FLAGGED.iter().map(|(n, _)| (*n).to_string()).collect();
        self.with_client(move |c| {
            let rows = c.query(
                "SELECT workspace_id, proposed_by,
                        LEAST(FLOOR(EXTRACT(EPOCH FROM (NOW() - created_at)) / 604800)::int, 12) AS weeks_ago,
                        ARRAY(SELECT x->>'name' FROM jsonb_array_elements(COALESCE(checks->'checks', '[]'::jsonb)) x
                              WHERE x->>'outcome' = 'fail' AND x->>'name' = ANY($2) ORDER BY 1) AS raised,
                        COUNT(*)
                 FROM app_proposals
                 WHERE created_at >= NOW() - make_interval(days => $1)
                 GROUP BY 1, 2, 3, 4",
                &[&days, &flagged],
            )?;
            Ok(rows.iter().map(|r| LoadGroup { workspace_id: r.get(0), proposed_by: r.get(1), weeks_ago: r.get::<_, i32>(2).max(0) as usize, raised: r.get(3), count: r.get(4) }).collect())
        })
    }

    /// The count, median and 95th percentile of seconds from a proposal to its decision, for proposals made in the last
    /// `days` days and approved or rejected. One statement. `percentile_disc` is the nearest-rank method.
    pub fn authority_decision_times(&self, days: i32) -> Result<DecisionTimes, RepositoryError> {
        self.with_client(move |c| {
            let r = c.query_one(
                "SELECT COUNT(*), percentile_disc(0.5) WITHIN GROUP (ORDER BY secs), percentile_disc(0.95) WITHIN GROUP (ORDER BY secs)
                 FROM (SELECT FLOOR(EXTRACT(EPOCH FROM (decided_at - created_at)))::bigint AS secs
                       FROM app_proposals
                       WHERE decided_at IS NOT NULL AND status IN ('Approved', 'Rejected') AND created_at >= NOW() - make_interval(days => $1)) t",
                &[&days],
            )?;
            Ok(DecisionTimes { decided: r.get(0), median: r.get(1), p95: r.get(2) })
        })
    }
}
