//! Lifecycle phase 4: legal holds. The database function `is_held(app, record)` decides, and every destruction path in
//! `purge.rs` asks it. This module stores holds, pins the records a hold names when it is placed, and counts them first.

use crate::jobs::{JobCtx, JobError};
use crate::repository::{PostgresRepository, RepositoryError};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use uuid::Uuid;

/// What a hold covers.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Scope {
    pub scope_kind: String,
    #[serde(default)]
    pub workspace_id: Option<String>,
    #[serde(default)]
    pub app_slug: Option<String>,
    #[serde(default)]
    pub table_id: Option<String>,
    /// A query hold: field = value pairs a record's data must contain.
    #[serde(default)]
    pub filter: Option<Value>,
    #[serde(default)]
    pub include_future: bool,
}

impl Scope {
    pub fn problems(&self) -> Vec<String> {
        let mut out = Vec::new();
        let has = |v: &Option<String>| v.as_deref().is_some_and(|s| !s.trim().is_empty());
        match self.scope_kind.as_str() {
            "workspace" if has(&self.workspace_id) => {}
            "workspace" => out.push("A workspace hold names its 'workspace_id'".to_string()),
            "app" if has(&self.app_slug) => {}
            "app" => out.push("An app hold names its 'app_slug'".to_string()),
            "table" if has(&self.app_slug) && has(&self.table_id) => {}
            "table" => out.push("A table hold names its 'app_slug' and 'table_id'".to_string()),
            "query" if has(&self.app_slug) && self.filter.as_ref().is_some_and(|f| f.as_object().is_some_and(|o| !o.is_empty())) => {}
            "query" => out.push("A query hold names its 'app_slug' and a 'filter' of field = value pairs".to_string()),
            _ => out.push("'scope_kind' must be workspace, app, table or query".to_string()),
        }
        out
    }

    /// Whether placing the hold pins the records it matches now. A workspace or app hold, and a table hold that includes
    /// later records, need no list: `is_held` reads their scope.
    pub fn pins(&self) -> bool {
        self.scope_kind == "query" || (self.scope_kind == "table" && !self.include_future)
    }

    /// The scope in words.
    pub fn sentence(&self) -> String {
        let app = self.app_slug.clone().unwrap_or_default();
        let later = if self.include_future { ", including records written later" } else { ", as they are now" };
        match self.scope_kind.as_str() {
            "workspace" => format!("Every record of every app in the workspace '{}'", self.workspace_id.clone().unwrap_or_default()),
            "app" => format!("Every record of the app '{app}'"),
            "table" => format!("Every record of the table '{}' in the app '{app}'{later}", self.table_id.clone().unwrap_or_default()),
            _ => {
                let pairs: Vec<String> = self
                    .filter
                    .as_ref()
                    .and_then(Value::as_object)
                    .map(|o| o.iter().map(|(k, v)| format!("{k} is {}", v.as_str().map(str::to_string).unwrap_or_else(|| v.to_string()))).collect())
                    .unwrap_or_default();
                let table = self.table_id.as_ref().map(|t| format!(" of the table '{t}'")).unwrap_or_default();
                format!("Records{table} in the app '{app}' where {}{later}", pairs.join(" and "))
            }
        }
    }

    /// The records the scope matches, as SQL over `dataset_records r`, in trash or not. `$1` is the scope as JSON.
    const MATCH: &'static str = "( \
        (($1::jsonb)->>'scope_kind' = 'workspace' AND r.app_slug IN (SELECT slug FROM app_manifests WHERE manifest->>'workspace_id' = ($1::jsonb)->>'workspace_id')) \
        OR (($1::jsonb)->>'scope_kind' = 'app' AND r.app_slug = ($1::jsonb)->>'app_slug') \
        OR (($1::jsonb)->>'scope_kind' = 'table' AND r.app_slug = ($1::jsonb)->>'app_slug' AND r.table_id = ($1::jsonb)->>'table_id') \
        OR (($1::jsonb)->>'scope_kind' = 'query' AND r.app_slug = ($1::jsonb)->>'app_slug' \
            AND (($1::jsonb)->>'table_id' IS NULL OR r.table_id = ($1::jsonb)->>'table_id') AND r.data @> (($1::jsonb)->'filter')))";
}

const COLUMNS: &str = "id, name, case_ref, reason, scope_kind, workspace_id, app_slug, table_id, filter, include_future, created_by, \
    to_char(created_at AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS\"Z\"'), released_by, to_char(released_at AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS\"Z\"')";

#[derive(Debug, Clone, Serialize)]
pub struct Hold {
    pub id: Uuid,
    pub name: String,
    pub case_ref: String,
    pub reason: String,
    #[serde(flatten)]
    pub scope: Scope,
    pub created_by: String,
    pub created_at: String,
    pub released_by: Option<String>,
    pub released_at: Option<String>,
}

fn hold_of(r: &postgres::Row) -> Hold {
    Hold {
        id: r.get(0),
        name: r.get(1),
        case_ref: r.get(2),
        reason: r.get(3),
        scope: Scope { scope_kind: r.get(4), workspace_id: r.get(5), app_slug: r.get(6), table_id: r.get(7), filter: r.get(8), include_future: r.get(9) },
        created_by: r.get(10),
        created_at: r.get(11),
        released_by: r.get(12),
        released_at: r.get(13),
    }
}

/// How many records a scope covers now, and in how many apps.
fn count(c: &mut impl postgres::GenericClient, scope: &Scope) -> Result<(i64, i64), RepositoryError> {
    let s = serde_json::to_value(scope)?;
    let row = c.query_one(&format!("SELECT count(*), count(DISTINCT r.app_slug) FROM dataset_records r WHERE {}", Scope::MATCH), &[&s])?;
    Ok((row.get(0), row.get(1)))
}

pub fn sentence_of(records: i64, apps: i64) -> String {
    let r = if records == 1 { "record" } else { "records" };
    let a = if apps == 1 { "app" } else { "apps" };
    format!("This hold covers {records} {r} across {apps} {a}.")
}

impl PostgresRepository {
    pub fn list_holds(&self) -> Result<Vec<(Hold, i64)>, RepositoryError> {
        self.with_client(|c| {
            let holds: Vec<Hold> = c.query(&format!("SELECT {COLUMNS} FROM legal_holds ORDER BY released_at IS NOT NULL, created_at DESC"), &[])?.iter().map(hold_of).collect();
            let mut out = Vec::new();
            for h in holds {
                let n = if h.scope.pins() {
                    c.query_one("SELECT count(*) FROM hold_records WHERE hold_id = $1", &[&h.id])?.get(0)
                } else {
                    count(c, &h.scope)?.0
                };
                out.push((h, n));
            }
            Ok(out)
        })
    }

    pub fn get_hold(&self, id: Uuid) -> Result<Option<Hold>, RepositoryError> {
        self.with_client(move |c| Ok(c.query_opt(&format!("SELECT {COLUMNS} FROM legal_holds WHERE id = $1"), &[&id])?.as_ref().map(hold_of)))
    }

    /// Stores a hold. The caller has written the ledger entry first.
    pub fn insert_hold(&self, name: &str, case_ref: &str, reason: &str, scope: &Scope, by: &str) -> Result<Uuid, RepositoryError> {
        let (name, case_ref, reason, scope, by) = (name.to_string(), case_ref.to_string(), reason.to_string(), scope.clone(), by.to_string());
        self.with_client(move |c| {
            let id = Uuid::new_v4();
            c.execute(
                "INSERT INTO legal_holds (id, name, case_ref, reason, scope_kind, workspace_id, app_slug, table_id, filter, include_future, created_by) \
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)",
                &[&id, &name, &case_ref, &reason, &scope.scope_kind, &scope.workspace_id, &scope.app_slug, &scope.table_id, &scope.filter, &scope.include_future, &by],
            )?;
            Ok(id)
        })
    }

    /// Releases a hold and removes its pinned rows, in one transaction. `false` when it was released already.
    pub fn release_hold(&self, id: Uuid, by: &str) -> Result<bool, RepositoryError> {
        let by = by.to_string();
        self.with_client(move |c| {
            let mut tx = c.transaction()?;
            let n = tx.execute("UPDATE legal_holds SET released_by = $2, released_at = NOW() WHERE id = $1 AND released_at IS NULL", &[&id, &by])?;
            tx.execute("DELETE FROM hold_records WHERE hold_id = $1", &[&id])?;
            tx.commit()?;
            Ok(n > 0)
        })
    }

    /// Whether a legal hold keeps this record. Every destruction path asks the same database function.
    pub fn is_held(&self, app_slug: &str, record_id: &str) -> Result<bool, RepositoryError> {
        let (a, r) = (app_slug.to_string(), record_id.to_string());
        self.with_client(move |c| Ok(c.query_one("SELECT is_held($1, $2)", &[&a, &r])?.get(0)))
    }

    /// The names of the active holds that keep a record.
    pub fn holds_of_record(&self, app_slug: &str, record_id: &str) -> Result<Vec<String>, RepositoryError> {
        let (a, r) = (app_slug.to_string(), record_id.to_string());
        self.with_client(move |c| {
            let holds: Vec<Hold> = c.query(&format!("SELECT {COLUMNS} FROM legal_holds WHERE released_at IS NULL"), &[])?.iter().map(hold_of).collect();
            let mut names = Vec::new();
            for h in holds {
                let s = serde_json::to_value(&h.scope)?;
                let pinned: bool = c.query_one("SELECT EXISTS (SELECT 1 FROM hold_records WHERE hold_id = $1 AND record_id = $2)", &[&h.id, &r])?.get(0);
                let scoped = !h.scope.pins() || h.scope.include_future;
                let live: bool = scoped
                    && c.query_one(&format!("SELECT EXISTS (SELECT 1 FROM dataset_records r WHERE r.id = $2 AND r.app_slug = $3 AND {})", Scope::MATCH), &[&s, &r, &a])?.get(0);
                if pinned || live {
                    names.push(h.name);
                }
            }
            Ok(names)
        })
    }
}

/// Pins the records a hold matches now, in batches.
pub fn pin_hold(ctx: &JobCtx, payload: Value) -> Result<Value, JobError> {
    let id = payload.get("hold_id").and_then(Value::as_str).and_then(|s| Uuid::parse_str(s).ok()).ok_or_else(|| JobError::Permanent("The job names no hold".to_string()))?;
    let hold = ctx.repository().get_hold(id).map_err(|e| JobError::Retry(e.to_string()))?.ok_or_else(|| JobError::Permanent("The hold is gone".to_string()))?;
    if hold.released_at.is_some() {
        return Ok(json!({ "pinned": 0 }));
    }
    let scope = serde_json::to_value(&hold.scope).map_err(|e| JobError::Permanent(e.to_string()))?;
    let mut pinned = 0u64;
    loop {
        let s = scope.clone();
        let n = ctx
            .repository()
            .with_client(move |c| {
                Ok(c.execute(
                    &format!(
                        "INSERT INTO hold_records (hold_id, record_id) SELECT $2, r.id FROM dataset_records r WHERE {} \
                         AND NOT EXISTS (SELECT 1 FROM hold_records x WHERE x.hold_id = $2 AND x.record_id = r.id) LIMIT 1000",
                        Scope::MATCH
                    ),
                    &[&s, &id],
                )?)
            })
            .map_err(|e| JobError::Retry(e.to_string()))?;
        pinned += n;
        if n == 0 {
            break;
        }
    }
    Ok(json!({ "pinned": pinned }))
}

/// Counts what a proposed hold covers, by app, before it is placed.
pub fn hold_preview(ctx: &JobCtx, payload: Value) -> Result<Value, JobError> {
    let scope: Scope = serde_json::from_value(payload.get("scope").cloned().unwrap_or(Value::Null)).map_err(|e| JobError::Permanent(format!("The scope is not a scope: {e}")))?;
    let (records, apps) = ctx.repository().with_client(move |c| count(c, &scope)).map_err(|e| JobError::Retry(e.to_string()))?;
    Ok(json!({ "records": records, "apps": apps, "sentence": sentence_of(records, apps) }))
}
