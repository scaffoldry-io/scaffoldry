//! Decision authority phase 2: the changes to the matrix and the rows they write. A confirmed change is applied in one
//! transaction, after the service has written its ledger entry.

use crate::repository::{PostgresRepository, RepositoryError};
use serde_json::{json, Value};
use uuid::Uuid;

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
}
