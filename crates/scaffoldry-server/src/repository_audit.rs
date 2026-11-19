//! Admin console phase 7: the ledger read with filters, a full chain check that says where it broke, and the token list.
//! The database filters, orders and cuts every page. A filter names a fixed platform column that has an index.

use crate::repository::{ledger_entry_of, ChainCheck, PostgresRepository, RepositoryError};
use crate::state::ApiToken;
use chrono::{DateTime, Utc};
use postgres::types::ToSql;
use scaffoldry_core::{DecisionType, LedgerEntry};
use serde_json::Value;

/// The columns every ledger read selects, in the order `ledger_entry_of` reads them.
const LEDGER_COLUMNS: &str = "sequence, entry_hash, previous_hash, timestamp_iso, principal, organization_code, app_slug, decision_type, \
    oscal_control_id, rationale, payload, COALESCE(payload_hash, '')";

/// A ledger entry with the payload it was stored with.
#[derive(Debug, Clone)]
pub struct LedgerRow {
    pub entry: LedgerEntry,
    pub payload: Value,
}

/// The filters of the Audit page.
#[derive(Debug, Default, Clone)]
pub struct LedgerFilter {
    pub principal: Option<String>,
    pub decision_type: Option<DecisionType>,
    pub app: Option<String>,
    /// Every way the control may be written in a stored row, so the control index is used.
    pub controls: Vec<String>,
    pub from: Option<DateTime<Utc>>,
    pub to: Option<DateTime<Utc>>,
}

/// The filters of the token list. `state` is `active`, `expired` or `revoked`.
#[derive(Debug, Default, Clone)]
pub struct TokenFilter {
    pub kind: Option<String>,
    pub owner_prefix: Option<String>,
    pub state: Option<String>,
}

/// The `WHERE` clause of a ledger filter and its parameters. `before` is the sequence a page starts under.
fn ledger_where(f: &LedgerFilter, before: Option<u64>) -> (String, Vec<Box<dyn ToSql + Sync + Send>>) {
    let mut clauses: Vec<String> = Vec::new();
    let mut params: Vec<Box<dyn ToSql + Sync + Send>> = Vec::new();
    let mut add = |clause: &str, value: Box<dyn ToSql + Sync + Send>| {
        params.push(value);
        clauses.push(clause.replace('?', &format!("${}", params.len())));
    };
    if let Some(p) = &f.principal {
        add("lower(principal) = lower(?)", Box::new(p.clone()));
    }
    if let Some(d) = f.decision_type {
        add("decision_type = ?", Box::new(d.as_str().to_string()));
    }
    if let Some(a) = &f.app {
        add("app_slug = ?", Box::new(a.clone()));
    }
    if !f.controls.is_empty() {
        add("oscal_control_id = ANY(?)", Box::new(f.controls.clone()));
    }
    if let Some(t) = f.from {
        add("timestamp_iso::timestamptz >= ?", Box::new(t));
    }
    if let Some(t) = f.to {
        add("timestamp_iso::timestamptz <= ?", Box::new(t));
    }
    if let Some(b) = before {
        add("sequence < ?", Box::new(b as i64));
    }
    let sql = if clauses.is_empty() { String::new() } else { format!(" WHERE {}", clauses.join(" AND ")) };
    (sql, params)
}

fn as_refs(params: &[Box<dyn ToSql + Sync + Send>]) -> Vec<&(dyn ToSql + Sync)> {
    params.iter().map(|p| &**p as &(dyn ToSql + Sync)).collect()
}

/// The `LIKE` pattern that matches names starting with `prefix`, with the pattern characters in it taken literally.
fn prefix_pattern(prefix: &str) -> String {
    let escaped = prefix.to_lowercase().replace('\\', "\\\\").replace('%', "\\%").replace('_', "\\_");
    format!("{escaped}%")
}

impl PostgresRepository {
    /// One page of the ledger, newest first, with each entry's stored payload. `before` is the sequence the page starts under.
    pub fn ledger_rows(&self, filter: &LedgerFilter, before: Option<u64>, limit: i64) -> Result<Vec<LedgerRow>, RepositoryError> {
        let (clause, params) = ledger_where(filter, before);
        self.with_client(move |client| {
            let sql = format!("SELECT {LEDGER_COLUMNS} FROM governance_ledger{clause} ORDER BY sequence DESC LIMIT {}", limit.max(0));
            let rows = client.query(&sql, &as_refs(&params))?;
            rows.iter().map(|r| Ok(LedgerRow { entry: ledger_entry_of(r)?, payload: r.get(10) })).collect()
        })
    }

    /// How many entries match, counted no higher than `cap + 1`, so an export can refuse a huge one without counting it all.
    pub fn ledger_count_up_to(&self, filter: &LedgerFilter, cap: i64) -> Result<i64, RepositoryError> {
        let (clause, params) = ledger_where(filter, None);
        self.with_client(move |client| {
            let sql = format!("SELECT count(*) FROM (SELECT 1 FROM governance_ledger{clause} LIMIT {}) m", cap.max(0) + 1);
            Ok(client.query_one(&sql, &as_refs(&params))?.get(0))
        })
    }

    /// Walks the whole chain and reports where it first breaks. Unlike the startup check, a break is an answer, not an error.
    pub fn ledger_chain_check(&self) -> Result<ChainCheck, RepositoryError> {
        self.with_client(PostgresRepository::check_chain)
    }

    /// One page of tokens, newest first, by offset.
    pub fn list_tokens_admin(&self, filter: &TokenFilter, offset: i64, limit: i64) -> Result<Vec<ApiToken>, RepositoryError> {
        let mut clauses: Vec<String> = Vec::new();
        let mut params: Vec<Box<dyn ToSql + Sync + Send>> = Vec::new();
        if let Some(k) = &filter.kind {
            params.push(Box::new(k.clone()));
            clauses.push(format!("kind = ${}", params.len()));
        }
        if let Some(o) = &filter.owner_prefix {
            params.push(Box::new(prefix_pattern(o)));
            clauses.push(format!("lower(eppn) LIKE ${} ESCAPE '\\'", params.len()));
        }
        match filter.state.as_deref() {
            Some("active") => clauses.push("revoked_at IS NULL AND expires_at > NOW()".to_string()),
            Some("expired") => clauses.push("revoked_at IS NULL AND expires_at <= NOW()".to_string()),
            Some("revoked") => clauses.push("revoked_at IS NOT NULL".to_string()),
            _ => {}
        }
        let clause = if clauses.is_empty() { String::new() } else { format!(" WHERE {}", clauses.join(" AND ")) };
        self.with_client(move |client| {
            let sql = format!(
                "SELECT token_hash, id, kind, eppn, label, original_admin, created_at::text, expires_at::text, last_used_at::text, \
                        revoked_at::text, app_slug, scope FROM api_tokens{clause} ORDER BY created_at DESC, id LIMIT {} OFFSET {}",
                limit.max(0),
                offset.max(0)
            );
            client.query(&sql, &as_refs(&params))?.iter().map(crate::repository::api_token_of_text_row).collect()
        })
    }

    /// One token by its id, for a revoke to find out what it is revoking. The row carries the hash, so it never leaves the server.
    pub fn api_token_by_id(&self, id: uuid::Uuid) -> Result<Option<ApiToken>, RepositoryError> {
        self.with_client(move |client| {
            let row = client.query_opt(
                "SELECT token_hash, id, kind, eppn, label, original_admin, created_at::text, expires_at::text, last_used_at::text, \
                        revoked_at::text, app_slug, scope FROM api_tokens WHERE id = $1",
                &[&id],
            )?;
            row.map(|r| crate::repository::api_token_of_text_row(&r)).transpose()
        })
    }
}
