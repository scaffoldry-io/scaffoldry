//! PostgreSQL 17 Repository Layer with Cryptographic Ledger Hash Chain Verification
//! Enforces:
//! - NIST OSCAL 1.1.2 AU-02 cryptographic immutable audit ledger verification on startup.
//! - Relational persistence for Workspaces, Collaborators, Records, Sessions, and Ledger.
//! - Institutional scale performance (1,000+ workspaces and collaborators).
//! - Dedicated background worker architecture to isolate synchronous postgres driver from Tokio runtimes.

pub use crate::repository_index::{
    build_aggregate_sql, build_indexed_sql, AggregateFunction, AggregateGroupRow, AggregatePlan,
    AggregateResultRaw, IndexPhase, IndexedClause, IndexedQuery, IndexedSort, IndexedValue,
};
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
const SCHEMA_0002: &str =
    include_str!("../../scaffoldry-core/migrations/0002_workspaces_and_ledger.sql");
const SCHEMA_0003: &str =
    include_str!("../../scaffoldry-core/migrations/0003_persist_apps_and_datasets.sql");
const SCHEMA_0004: &str =
    include_str!("../../scaffoldry-core/migrations/0004_organization_scope.sql");
const SCHEMA_0005: &str =
    include_str!("../../scaffoldry-core/migrations/0005_process_instances.sql");
const SCHEMA_0006: &str = include_str!("../../scaffoldry-core/migrations/0006_role_source.sql");
const SCHEMA_0007: &str = include_str!("../../scaffoldry-core/migrations/0007_record_version.sql");
const SCHEMA_0008: &str =
    include_str!("../../scaffoldry-core/migrations/0008_schema_migrations.sql");
const SCHEMA_0009: &str =
    include_str!("../../scaffoldry-core/migrations/0009_ledger_append_only.sql");
const SCHEMA_0010: &str = include_str!("../../scaffoldry-core/migrations/0010_app_workspace.sql");
const SCHEMA_0012: &str = include_str!("../../scaffoldry-core/migrations/0012_api_tokens.sql");
const SCHEMA_0013: &str =
    include_str!("../../scaffoldry-core/migrations/0013_platform_settings.sql");
const SCHEMA_0014: &str = include_str!("../../scaffoldry-core/migrations/0014_record_table_id.sql");
const SCHEMA_0015: &str = include_str!("../../scaffoldry-core/migrations/0015_record_index.sql");
const SCHEMA_0016: &str = include_str!("../../scaffoldry-core/migrations/0016_record_search.sql");
const SCHEMA_0021: &str = include_str!("../../scaffoldry-core/migrations/0021_positions.sql");
const SCHEMA_0022: &str =
    include_str!("../../scaffoldry-core/migrations/0022_record_created_by.sql");
const SCHEMA_0023: &str =
    include_str!("../../scaffoldry-core/migrations/0023_workspace_guards.sql");
const SCHEMA_0019: &str = include_str!("../../scaffoldry-core/migrations/0019_rule_versions.sql");
const SCHEMA_0024: &str = include_str!("../../scaffoldry-core/migrations/0024_proposal_rules.sql");
const SCHEMA_0011: &str = include_str!("../../scaffoldry-core/migrations/0011_app_proposals.sql");
const SCHEMA_0027: &str = include_str!("../../scaffoldry-core/migrations/0027_record_history.sql");
const SCHEMA_0026: &str = include_str!("../../scaffoldry-core/migrations/0026_jobs.sql");
const SCHEMA_0046: &str = include_str!("../../scaffoldry-core/migrations/0046_calc_status.sql");
const SCHEMA_0047: &str = include_str!("../../scaffoldry-core/migrations/0047_view_as_token.sql");
const SCHEMA_0048: &str = include_str!("../../scaffoldry-core/migrations/0048_disclosure_log.sql");
const SCHEMA_0049: &str = include_str!("../../scaffoldry-core/migrations/0049_app_views.sql");
const SCHEMA_0050: &str = include_str!("../../scaffoldry-core/migrations/0050_view_anchors.sql");
const SCHEMA_0051: &str = include_str!("../../scaffoldry-core/migrations/0051_files.sql");
const SCHEMA_0052: &str = include_str!("../../scaffoldry-core/migrations/0052_file_scans.sql");
const SCHEMA_0053: &str = include_str!("../../scaffoldry-core/migrations/0053_cache_versions.sql");
const SCHEMA_0054: &str = include_str!("../../scaffoldry-core/migrations/0054_people_indexes.sql");
const SCHEMA_0055: &str =
    include_str!("../../scaffoldry-core/migrations/0055_member_app_scope.sql");
const SCHEMA_0056: &str = include_str!("../../scaffoldry-core/migrations/0056_imports.sql");
const SCHEMA_0057: &str = include_str!("../../scaffoldry-core/migrations/0057_hierarchies.sql");
const SCHEMA_0045: &str = include_str!("../../scaffoldry-core/migrations/0045_index_status.sql");
const SCHEMA_0028: &str = include_str!("../../scaffoldry-core/migrations/0028_record_comments.sql");
const SCHEMA_0030: &str =
    include_str!("../../scaffoldry-core/migrations/0030_member_principals.sql");
const SCHEMA_0031: &str = include_str!("../../scaffoldry-core/migrations/0031_record_links.sql");
const SCHEMA_0032: &str = include_str!("../../scaffoldry-core/migrations/0032_trash_links.sql");
const SCHEMA_0042: &str = include_str!("../../scaffoldry-core/migrations/0042_lifecycle.sql");
const SCHEMA_0044: &str = include_str!("../../scaffoldry-core/migrations/0044_job_schedules.sql");

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
    #[error("Conflict: {0}")]
    Conflict(String),
    #[error("Constraint violation on '{field}': {message}")]
    ConstraintViolation {
        message: String,
        field: String,
        constraint: String,
    },
    /// A limit on queued or running jobs. The text is a plain sentence for the person.
    #[error("{0}")]
    Quota(String),
}

type WorkerJob = Box<dyn FnOnce(&mut Client) + Send>;

/// The next page of a table after a cursor. The row comparison walks the paging index.
pub const PAGE_SQL: &str = "SELECT id, app_slug, data, ceds_mapping, is_ferpa_sensitive, created_at::text AS created_at_text, created_by, version \
     FROM dataset_records \
     WHERE app_slug = $1 AND table_id = $2 AND deleted_at IS NULL AND (created_at, id) > ($3::text::timestamptz, $4::text) \
     ORDER BY created_at, id LIMIT $5";

/// The first page of a table.
pub const FIRST_PAGE_SQL: &str = "SELECT id, app_slug, data, ceds_mapping, is_ferpa_sensitive, created_at::text AS created_at_text, created_by, version \
     FROM dataset_records \
     WHERE app_slug = $1 AND table_id = $2 AND deleted_at IS NULL \
     ORDER BY created_at, id LIMIT $3";

/// Who wrote a record, and how. Every write takes one, so a change cannot go unattributed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActorKind {
    User,
    Agent,
    Automation,
    Import,
    Sync,
    System,
    Guest,
}

impl ActorKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::User => "user",
            Self::Agent => "agent",
            Self::Automation => "automation",
            Self::Import => "import",
            Self::Sync => "sync",
            Self::System => "system",
            Self::Guest => "guest",
        }
    }
}

/// Records read, records raised, and the last `(created_at, id)` of a flag catch-up batch.
pub type FlagBatch = (usize, u64, Option<(String, String)>);

static PAGE_STATEMENTS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// How many statements this process has run to read a page of records. A test reads it before and
/// after a request to show that a page costs a fixed number of statements.
pub fn page_statements() -> u64 {
    PAGE_STATEMENTS.load(std::sync::atomic::Ordering::SeqCst)
}

pub(crate) fn count_page_statement() {
    PAGE_STATEMENTS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
}

/// The attribution of one record write. The history row is built from it, in the same
/// transaction as the write.
#[derive(Debug, Clone)]
pub struct WriteCtx {
    pub actor: String,
    pub kind: ActorKind,
    pub token_label: Option<String>,
    /// Groups the records written by one action: a paste, an import batch, an automation run.
    pub batch_id: uuid::Uuid,
    /// Fields a person cannot write. A computed value follows from the others, so it is not recorded.
    pub computed_fields: Vec<String>,
    /// Set when this write undoes another batch.
    pub reverts_batch: Option<uuid::Uuid>,
    /// The indexed fields of the record's table. Every write keeps their index rows current.
    pub index_fields: Vec<scaffoldry_engine::FieldSpec>,
    /// The table's `Link` fields. Every write keeps their `record_links` rows current, and mirrors two-way links.
    pub links: Vec<crate::repository_links::LinkSpec>,
    /// Every `Rollup` and `Count` of the app. A write that touches the records they go through recomputes them.
    pub computed: Vec<crate::repository_links::ComputedSpec>,
    /// The formulas of the record's table that are stored, as `(field, expression)`, in field order.
    pub formulas: Vec<(String, String)>,
    /// What each table of the app needs when one of its records is recomputed because another changed:
    /// its stored formulas in run order, and its indexed fields.
    pub calc: std::sync::Arc<std::collections::HashMap<String, crate::repository_links::TableCalc>>,
    /// The action the history row names, when it is not the usual one. A restore says `restore`.
    pub action: Option<&'static str>,
}

impl WriteCtx {
    pub fn new(actor: impl Into<String>, kind: ActorKind) -> Self {
        Self {
            actor: actor.into(),
            kind,
            token_label: None,
            batch_id: uuid::Uuid::new_v4(),
            computed_fields: Vec::new(),
            reverts_batch: None,
            index_fields: Vec::new(),
            links: Vec::new(),
            computed: Vec::new(),
            formulas: Vec::new(),
            calc: Default::default(),
            action: None,
        }
    }

    /// A write by the platform itself, such as a fixture or a repair.
    pub fn system(actor: &str) -> Self {
        Self::new(actor, ActorKind::System)
    }
}

/// One row of `record_history`.
#[derive(Debug, Clone, serde::Serialize)]
pub struct HistoryEntry {
    pub id: i64,
    pub app_slug: String,
    pub table_id: String,
    pub record_id: String,
    pub version: i32,
    pub action: String,
    pub changes: Value,
    pub actor: String,
    pub actor_kind: String,
    pub token_label: Option<String>,
    pub batch_id: String,
    pub reverts_batch: Option<String>,
    pub proposal_id: Option<String>,
    pub at: String,
}

#[derive(Debug, Clone, Default)]
pub struct HistoryFilter {
    pub record_id: Option<String>,
    pub table_id: Option<String>,
    pub actor: Option<String>,
    pub kind: Option<String>,
    pub from: Option<String>,
    pub to: Option<String>,
}

/// The fields that differ, as `{ field: { old, new } }`. A field missing on one side has no `old`
/// or no `new`. Keys that start with `_` belong to the platform, and computed fields follow from
/// the rest, so neither is recorded.
pub fn diff_fields(
    old: Option<&Value>,
    new: Option<&Value>,
    computed: &[String],
) -> serde_json::Map<String, Value> {
    let empty = serde_json::Map::new();
    let (is_create, is_delete) = (old.is_none(), new.is_none());
    let old = old.and_then(|v| v.as_object()).unwrap_or(&empty);
    let new = new.and_then(|v| v.as_object()).unwrap_or(&empty);
    let mut keys: Vec<&String> = old.keys().chain(new.keys()).collect();
    keys.sort();
    keys.dedup();
    let mut out = serde_json::Map::new();
    for key in keys {
        if key.starts_with('_') || computed.iter().any(|c| c == key) {
            continue;
        }
        let (o, n) = (old.get(key), new.get(key));
        if o == n {
            continue;
        }
        let mut change = serde_json::Map::new();
        if !is_create {
            change.insert("old".to_string(), o.cloned().unwrap_or(Value::Null));
        }
        if !is_delete {
            change.insert("new".to_string(), n.cloned().unwrap_or(Value::Null));
        }
        out.insert(key.clone(), Value::Object(change));
    }
    out
}

/// What a history row says about a record: where it is, which version, and what was done.
pub(crate) struct Touched<'a> {
    pub(crate) app_slug: &'a str,
    pub(crate) table_id: &'a str,
    pub(crate) record_id: &'a str,
    pub(crate) version: i32,
    pub(crate) action: &'a str,
}

/// Replaces a record's JSON without raising its version. For derived changes, such as the mirror side
/// of a link. The caller writes the history row in the same transaction.
pub(crate) fn set_record_data_in_tx(
    tx: &mut postgres::Transaction<'_>,
    app_slug: &str,
    id: &str,
    data: &Value,
) -> Result<(), postgres::Error> {
    tx.execute(
        "UPDATE dataset_records SET data = $3 WHERE app_slug = $1 AND id = $2",
        &[&app_slug, &id, data],
    )?;
    Ok(())
}

/// Writes the stored formulas of the table into the record's data. A value the caller sent for one is replaced.
pub(crate) fn apply_formulas(data: &mut Value, ctx: &WriteCtx) {
    apply_formula_list(data, &ctx.formulas);
}

/// The same for a list of `(field, expression)`, in the order given.
pub(crate) fn apply_formula_list(data: &mut Value, formulas: &[(String, String)]) {
    for (field, expression) in formulas {
        if scaffoldry_engine::is_formula_volatile(expression) && data.get(field).is_some() {
            continue;
        }
        let value = scaffoldry_engine::evaluate_formula(expression, data);
        if let Some(map) = data.as_object_mut() {
            if value.is_null() {
                map.remove(field);
            } else {
                map.insert(field.clone(), value);
            }
        }
    }
}

/// A new record, in the caller's transaction. The history row is written with it.
pub(crate) fn insert_record_in_tx(
    tx: &mut postgres::Transaction<'_>,
    rec: &DatasetRecord,
    ctx: &WriteCtx,
) -> Result<(), postgres::Error> {
    let mut rec = rec.clone();
    apply_formulas(&mut rec.data, ctx);
    let table = table_id_of(&rec.data);
    tx.execute(
        "INSERT INTO dataset_records (id, app_slug, data, ceds_mapping, is_ferpa_sensitive, created_by, table_id) \
         VALUES ($1, $2, $3, $4, $5, $6, $7)",
        &[&rec.id, &rec.app_slug, &rec.data, &rec.ceds_mapping, &rec.is_ferpa_sensitive, &rec.created_by, &table],
    )?;
    crate::repository_index::write_record_index(
        tx,
        &rec.app_slug,
        &table,
        &rec.id,
        &rec.data,
        &ctx.index_fields,
    )?;
    crate::repository_links::write_record_links(tx, &rec.app_slug, &rec.id, None, &rec.data, ctx)?;
    let changes = diff_fields(None, Some(&rec.data), &ctx.computed_fields);
    let action = ctx.action.unwrap_or("create");
    write_history(
        tx,
        Touched {
            app_slug: &rec.app_slug,
            table_id: &table,
            record_id: &rec.id,
            version: 1,
            action,
        },
        &changes,
        ctx,
    )?;
    Ok(())
}

/// One update, in the caller's transaction. With `expected_version`, a record at another version is
/// left alone and reported as a conflict. A write that changes nothing is skipped.
pub(crate) fn update_record_in_tx(
    tx: &mut postgres::Transaction<'_>,
    slug: &str,
    id: &str,
    data: &Value,
    expected_version: Option<i32>,
    ctx: &WriteCtx,
) -> Result<RecordWrite, postgres::Error> {
    let mut data = data.clone();
    apply_formulas(&mut data, ctx);
    let Some(row) = tx.query_opt("SELECT data, version FROM dataset_records WHERE app_slug = $1 AND id = $2 AND deleted_at IS NULL FOR UPDATE", &[&slug, &id])? else {
        return Ok(RecordWrite::Missing);
    };
    let (old, version): (Value, i32) = (row.get(0), row.get(1));
    if expected_version.is_some_and(|expected| expected != version) {
        return Ok(RecordWrite::Conflict(version));
    }
    let changes = diff_fields(Some(&old), Some(&data), &ctx.computed_fields);
    if changes.is_empty() {
        return Ok(RecordWrite::Saved(version));
    }
    let new_table = table_id_of(&data);
    let next: i32 = tx
        .query_one(
            "UPDATE dataset_records SET data = $1, table_id = $2, version = version + 1 \
             WHERE app_slug = $3 AND id = $4 RETURNING version",
            &[&data, &new_table, &slug, &id],
        )?
        .get(0);
    crate::repository_index::write_record_index(
        tx,
        slug,
        &new_table,
        id,
        &data,
        &ctx.index_fields,
    )?;
    crate::repository_links::write_record_links(tx, slug, id, Some(&old), &data, ctx)?;
    let action = ctx.action.unwrap_or("update");
    write_history(
        tx,
        Touched {
            app_slug: slug,
            table_id: &new_table,
            record_id: id,
            version: next,
            action,
        },
        &changes,
        ctx,
    )?;
    Ok(RecordWrite::Saved(next))
}

/// One delete, in the caller's transaction. The record goes to trash and stays there until a
/// retention job destroys it. Its index rows and links are removed, and the links are remembered so a
/// restore can bring them back. With `expected_version`, a record at another version is left alone and
/// reported as a conflict.
pub(crate) fn delete_record_in_tx(
    tx: &mut postgres::Transaction<'_>,
    slug: &str,
    id: &str,
    expected_version: Option<i32>,
    ctx: &WriteCtx,
) -> Result<RecordWrite, postgres::Error> {
    let batch = ctx.batch_id;
    let moved = match expected_version {
        Some(expected) => tx.query_opt(
            "UPDATE dataset_records SET deleted_at = NOW(), deleted_by = $4, delete_batch = $5 \
             WHERE app_slug = $1 AND id = $2 AND version = $3 AND deleted_at IS NULL RETURNING data, version, table_id",
            &[&slug, &id, &expected, &ctx.actor, &batch],
        )?,
        None => tx.query_opt(
            "UPDATE dataset_records SET deleted_at = NOW(), deleted_by = $3, delete_batch = $4 \
             WHERE app_slug = $1 AND id = $2 AND deleted_at IS NULL RETURNING data, version, table_id",
            &[&slug, &id, &ctx.actor, &batch],
        )?,
    };
    if let Some(row) = moved {
        let (old, version, table): (Value, i32, String) = (row.get(0), row.get(1), row.get(2));
        tx.execute("DELETE FROM record_index WHERE record_id = $1", &[&id])?;
        crate::repository_links::delete_record_links(tx, slug, id, ctx)?;
        let changes = diff_fields(Some(&old), None, &ctx.computed_fields);
        let action = ctx.action.unwrap_or("delete");
        write_history(
            tx,
            Touched {
                app_slug: slug,
                table_id: &table,
                record_id: id,
                version,
                action,
            },
            &changes,
            ctx,
        )?;
        return Ok(RecordWrite::Saved(version));
    }
    let current = tx.query_opt("SELECT version FROM dataset_records WHERE app_slug = $1 AND id = $2 AND deleted_at IS NULL", &[&slug, &id])?;
    Ok(match current {
        Some(row) => RecordWrite::Conflict(row.get(0)),
        None => RecordWrite::Missing,
    })
}

/// A record in trash, with who put it there and when.
#[derive(Debug, Clone)]
pub struct TrashRow {
    pub record: DatasetRecord,
    pub deleted_by: String,
    pub deleted_at: String,
    pub delete_batch: Option<uuid::Uuid>,
    /// Whole days left before a retention job may destroy it.
    pub days_left: i64,
}

/// Brings one record out of trash, in the caller's transaction: the flags clear and the index rows
/// are written again. Its links come back in a second pass (`restore_links_in_tx`), once every
/// record of the batch is live again.
pub(crate) fn restore_record_in_tx(
    tx: &mut postgres::Transaction<'_>,
    slug: &str,
    id: &str,
    ctx: &WriteCtx,
) -> Result<bool, postgres::Error> {
    let Some(row) = tx.query_opt(
        "UPDATE dataset_records SET deleted_at = NULL, deleted_by = NULL, delete_batch = NULL \
         WHERE app_slug = $1 AND id = $2 AND deleted_at IS NOT NULL RETURNING data, version, table_id",
        &[&slug, &id],
    )? else {
        return Ok(false);
    };
    let (data, version, table): (Value, i32, String) = (row.get(0), row.get(1), row.get(2));
    crate::repository_index::write_record_index(tx, slug, &table, id, &data, &ctx.index_fields)?;
    let action = ctx.action.unwrap_or("restore");
    write_history(
        tx,
        Touched {
            app_slug: slug,
            table_id: &table,
            record_id: id,
            version,
            action,
        },
        &serde_json::Map::new(),
        ctx,
    )?;
    Ok(true)
}

pub(crate) fn write_history(
    tx: &mut postgres::Transaction<'_>,
    at: Touched<'_>,
    changes: &serde_json::Map<String, Value>,
    ctx: &WriteCtx,
) -> Result<(), postgres::Error> {
    tx.execute(
        "INSERT INTO record_history (app_slug, table_id, record_id, version, action, changes, actor, actor_kind, token_label, batch_id, reverts_batch) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)",
        &[
            &at.app_slug,
            &at.table_id,
            &at.record_id,
            &at.version,
            &at.action,
            &Value::Object(changes.clone()),
            &ctx.actor,
            &ctx.kind.as_str(),
            &ctx.token_label,
            &ctx.batch_id,
            &ctx.reverts_batch,
        ],
    )?;
    Ok(())
}

/// Where a page after a cursor starts. `key` is the sort value of the last row, when sorted.
#[derive(Debug, Clone)]
pub struct RecordCursor {
    pub created_at: String,
    pub id: String,
    pub key: Option<Value>,
}

/// A page that filters or sorts. The caller has checked every field name against the manifest.
/// Even so, a name or a value enters the statement only as a bound parameter.
#[derive(Debug, Clone)]
pub struct RecordQuery {
    pub table_id: String,
    /// True joins the clauses with OR. False joins them with AND.
    pub any: bool,
    pub clauses: Vec<(String, scaffoldry_engine::FilterOperator, String)>,
    /// The sort field, and true for descending.
    pub sort: Option<(String, bool)>,
    pub after: Option<RecordCursor>,
    pub limit: usize,
    /// Only the records inside this scope.
    pub scope: Option<ScopePlan>,
    /// Full-text search string.
    pub search: Option<String>,
    /// A filter tree, read from each record's JSON. Used instead of `clauses` when set.
    pub expr: Option<crate::repository_index::Expr>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum SqlParam {
    Text(String),
    Texts(Vec<String>),
    Float(f64),
    Floats(Vec<f64>),
    Int(i64),
}

/// A row scope for a statement over one table: the predicate, and the indexed fields it may read.
#[derive(Debug, Clone)]
pub struct ScopePlan {
    pub pred: scaffoldry_core::access::Pred,
    pub indexed: Vec<scaffoldry_engine::FieldSpec>,
}

impl ScopePlan {
    /// The `AND` fragment for a statement that names the record id as `id_expr`, with placeholders
    /// from `first_param`, and the parameters it needs. A scope the engine cannot compile reads nothing.
    pub fn clause_at(
        &self,
        id_expr: &str,
        app_slug: &str,
        table_id: &str,
        first_param: usize,
    ) -> (String, Vec<SqlParam>) {
        use scaffoldry_engine::access::{to_sql, Param};
        match to_sql(
            &self.pred,
            &self.indexed,
            id_expr,
            app_slug,
            table_id,
            first_param,
        ) {
            Ok((sql, added)) => {
                let params = added
                    .into_iter()
                    .map(|p| match p {
                        Param::Text(t) => SqlParam::Text(t),
                        Param::Texts(t) => SqlParam::Texts(t),
                        Param::Num(n) => SqlParam::Float(n),
                        Param::Nums(n) => SqlParam::Floats(n),
                    })
                    .collect();
                (format!(" AND ({sql})"), params)
            }
            Err(_) => (" AND FALSE".to_string(), Vec::new()),
        }
    }

    /// The same, appended to a statement's parameters.
    pub fn clause(
        &self,
        id_expr: &str,
        app_slug: &str,
        table_id: &str,
        params: &mut Vec<SqlParam>,
    ) -> String {
        let (sql, added) = self.clause_at(id_expr, app_slug, table_id, params.len() + 1);
        params.extend(added);
        sql
    }
}

/// Boxes parameters for the driver.
pub(crate) fn to_sql_params(params: Vec<SqlParam>) -> Vec<Box<dyn postgres::types::ToSql + Sync>> {
    params
        .into_iter()
        .map(|p| -> Box<dyn postgres::types::ToSql + Sync> {
            match p {
                SqlParam::Text(t) => Box::new(t),
                SqlParam::Texts(t) => Box::new(t),
                SqlParam::Float(f) => Box::new(f),
                SqlParam::Floats(f) => Box::new(f),
                SqlParam::Int(i) => Box::new(i),
            }
        })
        .collect()
}

/// The statement for a `RecordQuery`, and its parameters in order. Pure, so it can be tested
/// without a database.
pub fn build_record_query(app_slug: &str, q: &RecordQuery) -> (String, Vec<SqlParam>) {
    fn push(params: &mut Vec<SqlParam>, p: SqlParam) -> usize {
        params.push(p);
        params.len()
    }
    let mut params = vec![
        SqlParam::Text(app_slug.to_string()),
        SqlParam::Text(q.table_id.clone()),
    ];
    let joined = match &q.expr {
        Some(e) => json_expr(&mut params, e),
        None => {
            let mut preds: Vec<String> = Vec::new();
            for (field, op, value) in &q.clauses {
                if let Some(pred) = json_clause(&mut params, field, *op, value, None) {
                    preds.push(pred);
                }
            }
            if preds.is_empty() {
                "TRUE".to_string()
            } else {
                preds.join(if q.any { " OR " } else { " AND " })
            }
        }
    };

    let key = q.sort.as_ref().map(|(field, _)| {
        let f = push(&mut params, SqlParam::Text(field.clone()));
        format!("COALESCE(data -> ${f}::text, 'null'::jsonb)")
    });
    let desc = q.sort.as_ref().is_some_and(|(_, d)| *d);
    let dir = if desc { "DESC" } else { "ASC" };
    let order = match &key {
        Some(k) => format!("{k} {dir}, created_at {dir}, id {dir}"),
        None => "created_at, id".to_string(),
    };
    let after = match &q.after {
        None => String::new(),
        Some(c) => {
            let at = push(&mut params, SqlParam::Text(c.created_at.clone()));
            let id = push(&mut params, SqlParam::Text(c.id.clone()));
            let cmp = if desc { "<" } else { ">" };
            match &key {
                Some(k) => {
                    let v = push(
                        &mut params,
                        SqlParam::Text(c.key.clone().unwrap_or(Value::Null).to_string()),
                    );
                    format!(" AND ({k}, created_at, id) {cmp} (${v}::text::jsonb, ${at}::text::timestamptz, ${id}::text)")
                }
                None => {
                    format!(" AND (created_at, id) {cmp} (${at}::text::timestamptz, ${id}::text)")
                }
            }
        }
    };
    let search_clause = match &q.search {
        Some(s) if !s.trim().is_empty() => {
            let sp = push(&mut params, SqlParam::Text(s.clone()));
            format!(" AND search @@ plainto_tsquery('simple', ${sp})")
        }
        _ => String::new(),
    };
    let scope = match &q.scope {
        Some(plan) => {
            let (app, table) = (
                match &params[0] {
                    SqlParam::Text(t) => t.clone(),
                    _ => String::new(),
                },
                q.table_id.clone(),
            );
            plan.clause("dataset_records.id", &app, &table, &mut params)
        }
        None => String::new(),
    };
    let limit = push(
        &mut params,
        SqlParam::Int(i64::try_from(q.limit).unwrap_or(i64::MAX)),
    );
    let sql = format!(
        "SELECT id, app_slug, data, ceds_mapping, is_ferpa_sensitive, created_at::text AS created_at_text, created_by, version \
         FROM dataset_records WHERE app_slug = $1 AND table_id = $2 AND deleted_at IS NULL AND ({joined}){search_clause}{after}{scope} \
         ORDER BY {order} LIMIT ${limit}::bigint"
    );
    (sql, params)
}

fn push_sql(params: &mut Vec<SqlParam>, p: SqlParam) -> usize {
    params.push(p);
    params.len()
}

/// A date in a record's JSON, as an instant. A plain date is midnight UTC, as the index reads it.
fn json_instant(f: usize) -> String {
    format!(
        "(CASE WHEN data ->> ${f}::text ~ '^[0-9]{{4}}-[0-9]{{2}}-[0-9]{{2}}$' THEN ((data ->> ${f}::text) || 'T00:00:00+00:00')::timestamptz \
         WHEN data ->> ${f}::text ~ '^[0-9]{{4}}-[0-9]{{2}}-[0-9]{{2}}T[0-9]{{2}}:[0-9]{{2}}:[0-9]{{2}}(\\.[0-9]+)?(Z|[+-][0-9]{{2}}:[0-9]{{2}})$' THEN (data ->> ${f}::text)::timestamptz \
         ELSE NULL END)"
    )
}

/// The condition one clause puts on a record's JSON, or `None` for a clause that has no value and so is no clause.
/// The newer operators need the column of the field to know whether the cell is a number or a date.
fn json_clause(
    params: &mut Vec<SqlParam>,
    field: &str,
    op: scaffoldry_engine::FilterOperator,
    value: &str,
    column: Option<scaffoldry_engine::IndexColumn>,
) -> Option<String> {
    use scaffoldry_engine::{list_of, parse_instant, range_of, FilterOperator as Op, IndexColumn};
    // The older evaluator treats a clause with no value as no clause. So does this.
    if value.trim().is_empty() && !matches!(op, Op::IsEmpty | Op::IsNotEmpty) {
        return None;
    }
    Some(match op {
        Op::IsEmpty | Op::IsNotEmpty => {
            let f = push_sql(params, SqlParam::Text(field.to_string()));
            let empty = format!("((data -> ${f}::text) IS NULL OR (data -> ${f}::text) = 'null'::jsonb OR data ->> ${f}::text = '')");
            if matches!(op, Op::IsEmpty) {
                empty
            } else {
                format!("NOT {empty}")
            }
        }
        Op::Equals | Op::NotEquals | Op::Contains | Op::NotContains => {
            let f = push_sql(params, SqlParam::Text(field.to_string()));
            let v = push_sql(params, SqlParam::Text(value.to_string()));
            let cell = format!("lower(COALESCE(data ->> ${f}::text, ''))");
            let want = format!("lower(${v}::text)");
            match op {
                Op::Equals => format!("{cell} = {want}"),
                Op::NotEquals => format!("{cell} <> {want}"),
                Op::Contains => format!("strpos({cell}, {want}) > 0"),
                _ => format!("strpos({cell}, {want}) = 0"),
            }
        }
        Op::GreaterThan | Op::LessThan => match value.parse::<f64>() {
            Ok(n) if n.is_finite() => {
                let f = push_sql(params, SqlParam::Text(field.to_string()));
                let v = push_sql(params, SqlParam::Float(n));
                let sign = if matches!(op, Op::GreaterThan) {
                    ">"
                } else {
                    "<"
                };
                format!("CASE WHEN jsonb_typeof(data -> ${f}::text) = 'number' THEN (data ->> ${f}::text)::float8 {sign} ${v}::float8 ELSE FALSE END")
            }
            _ => "FALSE".to_string(),
        },
        Op::In => {
            let list: Vec<String> = list_of(value)
                .into_iter()
                .map(|v| v.to_lowercase())
                .collect();
            if list.is_empty() {
                return Some("FALSE".to_string());
            }
            let f = push_sql(params, SqlParam::Text(field.to_string()));
            let l = push_sql(params, SqlParam::Texts(list));
            format!(
                "(lower(COALESCE(data ->> ${f}::text, '')) = ANY(${l}::text[]) OR (jsonb_typeof(data -> ${f}::text) = 'array' AND EXISTS \
                 (SELECT 1 FROM jsonb_array_elements_text(data -> ${f}::text) e WHERE lower(e) = ANY(${l}::text[]))))"
            )
        }
        Op::Between => match (column, range_of(value)) {
            (Some(IndexColumn::Num), Some((low, high))) => {
                match (low.parse::<f64>(), high.parse::<f64>()) {
                    (Ok(a), Ok(b)) if a.is_finite() && b.is_finite() => {
                        let f = push_sql(params, SqlParam::Text(field.to_string()));
                        let (a, b) = (
                            push_sql(params, SqlParam::Float(a)),
                            push_sql(params, SqlParam::Float(b)),
                        );
                        format!("CASE WHEN jsonb_typeof(data -> ${f}::text) = 'number' THEN (data ->> ${f}::text)::float8 BETWEEN ${a}::float8 AND ${b}::float8 ELSE FALSE END")
                    }
                    _ => "FALSE".to_string(),
                }
            }
            (Some(IndexColumn::Ts), Some((low, high))) => {
                match (parse_instant(low), parse_instant(high)) {
                    (Some(a), Some(b)) => {
                        let f = push_sql(params, SqlParam::Text(field.to_string()));
                        let (a, b) = (
                            push_sql(params, SqlParam::Text(a)),
                            push_sql(params, SqlParam::Text(b)),
                        );
                        format!("COALESCE({} BETWEEN ${a}::text::timestamptz AND ${b}::text::timestamptz, FALSE)", json_instant(f))
                    }
                    _ => "FALSE".to_string(),
                }
            }
            _ => "FALSE".to_string(),
        },
        Op::Before | Op::After => match (column, parse_instant(value.trim())) {
            (Some(IndexColumn::Ts), Some(t)) => {
                let f = push_sql(params, SqlParam::Text(field.to_string()));
                let v = push_sql(params, SqlParam::Text(t));
                let sign = if matches!(op, Op::Before) { "<" } else { ">" };
                format!(
                    "COALESCE({} {sign} ${v}::text::timestamptz, FALSE)",
                    json_instant(f)
                )
            }
            _ => "FALSE".to_string(),
        },
    })
}

/// A filter tree as a condition on a record's JSON.
fn json_expr(params: &mut Vec<SqlParam>, e: &crate::repository_index::Expr) -> String {
    use crate::repository_index::Expr;
    match e {
        Expr::Leaf(c) => json_clause(params, &c.field, c.op, &c.value, Some(c.column))
            .unwrap_or_else(|| "TRUE".to_string()),
        Expr::And(v) if v.is_empty() => "TRUE".to_string(),
        Expr::Or(v) if v.is_empty() => "FALSE".to_string(),
        Expr::And(v) => format!(
            "({})",
            v.iter()
                .map(|x| json_expr(params, x))
                .collect::<Vec<_>>()
                .join(" AND ")
        ),
        Expr::Or(v) => format!(
            "({})",
            v.iter()
                .map(|x| json_expr(params, x))
                .collect::<Vec<_>>()
                .join(" OR ")
        ),
    }
}

/// The table a record belongs to, as the column holds it. The key also stays in the JSON.
fn table_id_of(data: &Value) -> String {
    data.get("_table_id")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string()
}

/// The result of a conditional write.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecordWrite {
    /// Written. The new version.
    Saved(i32),
    /// The stored version was not the one given. The current version.
    Conflict(i32),
    Missing,
}

pub(crate) fn record_from_row(r: &postgres::Row) -> DatasetRecord {
    DatasetRecord {
        id: r.get(0),
        app_slug: r.get(1),
        data: r.get(2),
        ceds_mapping: r.get(3),
        is_ferpa_sensitive: r.get(4),
        created_at: r.get(5),
        created_by: r.get(6),
        version: r.get(7),
    }
}

/// What `move_organization` did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OrgMove {
    Moved,
    /// The new parent is the unit or below it. Nothing changed.
    Cycle,
    /// The unit, or its new parent, is not in the table. Nothing changed.
    Missing,
    /// The unit and its new parent belong to different hierarchies. Nothing changed.
    BadHierarchy,
}

#[derive(Clone)]
pub struct PostgresRepository {
    worker_tx: Sender<WorkerJob>,
    worker_count: usize,
    db_url: std::sync::Arc<String>,
    /// How many times work went to the database through `with_client`. A test reads it to count the statements a
    /// request makes.
    statements: std::sync::Arc<std::sync::atomic::AtomicU64>,
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
            .unwrap_or_else(crate::testing::database_url);

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

                        let migrations: [(&str, &str); 42] = [
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
                            ("0014_record_table_id.sql", SCHEMA_0014),
                            ("0015_record_index.sql", SCHEMA_0015),
                            ("0016_record_search.sql", SCHEMA_0016),
                            ("0021_positions.sql", SCHEMA_0021),
                            ("0022_record_created_by.sql", SCHEMA_0022),
                            ("0007_record_version.sql", SCHEMA_0007),
                            ("0023_workspace_guards.sql", SCHEMA_0023),
                            ("0026_jobs.sql", SCHEMA_0026),
                            ("0028_record_comments.sql", SCHEMA_0028),
                            ("0030_member_principals.sql", SCHEMA_0030),
                            ("0031_record_links.sql", SCHEMA_0031),
                            ("0032_trash_links.sql", SCHEMA_0032),
                            ("0042_lifecycle.sql", SCHEMA_0042),
                            ("0044_job_schedules.sql", SCHEMA_0044),
                            ("0045_index_status.sql", SCHEMA_0045),
                            ("0046_calc_status.sql", SCHEMA_0046),
                            ("0047_view_as_token.sql", SCHEMA_0047),
                            ("0048_disclosure_log.sql", SCHEMA_0048),
                            ("0049_app_views.sql", SCHEMA_0049),
                            ("0050_view_anchors.sql", SCHEMA_0050),
                            ("0051_files.sql", SCHEMA_0051),
                            ("0052_file_scans.sql", SCHEMA_0052),
                            ("0053_cache_versions.sql", SCHEMA_0053),
                            ("0054_people_indexes.sql", SCHEMA_0054),
                            ("0055_member_app_scope.sql", SCHEMA_0055),
                            ("0056_imports.sql", SCHEMA_0056),
                            ("0057_hierarchies.sql", SCHEMA_0057),
                            ("0027_record_history.sql", SCHEMA_0027),
                            ("0011_app_proposals.sql", SCHEMA_0011),
                            ("0019_rule_versions.sql", SCHEMA_0019),
                            ("0024_proposal_rules.sql", SCHEMA_0024),
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
                .map_err(|e| {
                    RepositoryError::NotFound(format!(
                        "Failed to spawn db worker thread {w_idx}: {e}"
                    ))
                })?;
        }

        let repo = Self {
            worker_tx,
            worker_count: num_workers,
            db_url: std::sync::Arc::new(db_url),
            statements: std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0)),
        };
        Ok(repo)
    }

    /// The number of calls to `with_client` so far.
    pub fn statements(&self) -> u64 {
        self.statements.load(std::sync::atomic::Ordering::Relaxed)
    }

    /// Every cache counter, in one statement.
    pub fn cache_versions(&self) -> Result<Vec<(String, i64)>, RepositoryError> {
        self.with_client(|c| {
            Ok(c.query("SELECT name, version FROM cache_versions", &[])?
                .iter()
                .map(|r| (r.get(0), r.get(1)))
                .collect())
        })
    }

    pub fn worker_count(&self) -> usize {
        self.worker_count
    }

    pub fn get_applied_migrations(&self) -> Result<Vec<String>, RepositoryError> {
        self.with_client(|client| {
            let rows = client.query(
                "SELECT filename FROM schema_migrations ORDER BY filename ASC",
                &[],
            )?;
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
        self.statements
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
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

    fn verify_client_ledger(
        client: &mut Client,
        fresh_install: bool,
    ) -> Result<(), RepositoryError> {
        let r = client.query(
            "SELECT sequence, entry_hash, previous_hash, timestamp_iso, principal,                     organization_code, app_slug, decision_type, oscal_control_id, rationale, payload,                     COALESCE(payload_hash, '')              FROM governance_ledger ORDER BY sequence ASC",
            &[],
        )?;
        let rows = r
            .into_iter()
            .map(|row| {
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
                (
                    seq,
                    entry_hash,
                    prev_hash,
                    timestamp_iso,
                    principal,
                    organization_code,
                    app_slug,
                    decision_type_str,
                    oscal_control_id,
                    rationale,
                    payload,
                    raw_payload_hash,
                )
            })
            .collect::<Vec<_>>();

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

        for (
            seq,
            entry_hash,
            recorded_prev_hash,
            timestamp_iso,
            principal,
            organization_code,
            app_slug,
            decision_type_str,
            oscal_control_id,
            rationale,
            payload,
            raw_payload_hash,
        ) in rows
        {
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
        self.with_client(move |client| Self::verify_client_ledger(client, fresh_install))
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
            rationale: "Institutional statutory compliance attestation for 34 CFR Part 99"
                .to_string(),
            payload: &serde_json::json!({"framework": "FERPA", "standard": "34 CFR Part 99"}),
        });

        let entries = vec![
            (genesis, serde_json::json!({"status": "genesis"})),
            (
                entry_1,
                serde_json::json!({"status": "Published", "domain": "inventory.biology.state.edu"}),
            ),
            (
                entry_2,
                serde_json::json!({"domain": "inventory.biology.state.edu", "verified": true}),
            ),
            (
                entry_3,
                serde_json::json!({"rule": "auto-physics-honors-admit", "policy": "policy-ferpa-34cfr99"}),
            ),
            (
                entry_4,
                serde_json::json!({"framework": "FERPA", "standard": "34 CFR Part 99"}),
            ),
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
        self.with_client(Self::seed_client_genesis_ledger)
    }
    // -------------------------------------------------------------------------
    // WORKSPACES
    // -------------------------------------------------------------------------

    pub fn list_workspaces(&self) -> Result<Vec<WorkspaceRecord>, RepositoryError> {
        self.with_client(|client| {
            let rows = client.query(
                "SELECT id, name, code, organization, department, description, icon, lead, \
                        visibility, allowed_affiliations, data_classification, \
                        created_at::text, organization_id \
                 FROM workspaces ORDER BY created_at ASC",
                &[],
            )?;

            let mut workspaces = Vec::new();
            for r in rows {
                let allowed_affiliations: Value = r.get(9);
                let affs: Vec<String> =
                    serde_json::from_value(allowed_affiliations).unwrap_or_default();
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
                    created_at: r.get(11),
                    organization_id: r.get(12),
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
                        visibility, allowed_affiliations, data_classification, \
                        created_at::text, organization_id \
                 FROM workspaces WHERE id = $1",
                &[&id_str],
            )?;

            Ok(row.map(|r| {
                let allowed_affiliations: Value = r.get(9);
                let affs: Vec<String> =
                    serde_json::from_value(allowed_affiliations).unwrap_or_default();
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
                    created_at: r.get(11),
                    organization_id: r.get(12),
                }
            }))
        })
    }

    pub fn upsert_workspace(&self, ws: &WorkspaceRecord) -> Result<(), RepositoryError> {
        let ws = ws.clone();
        self.with_client(move |client| {
            let mut tx = client.transaction()?;
            let affs_json = serde_json::to_value(&ws.allowed_affiliations)?;
            tx.execute(
                "INSERT INTO workspaces (id, name, code, organization, department, description, \
                        icon, lead, visibility, allowed_affiliations, data_classification, organization_id) \
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
                    &ws.organization_id,
                ],
            )?;
            crate::cache::bump(&mut tx, "workspaces")?;
            tx.commit()?;
            Ok(())
        })
    }

    pub fn persist_workspaces_batch(
        &self,
        batch: &[WorkspaceRecord],
    ) -> Result<(), RepositoryError> {
        let batch = batch.to_vec();
        self.with_client(move |client| {
            let mut tx = client.transaction()?;
            for ws in batch {
                let affs_json = serde_json::to_value(&ws.allowed_affiliations)?;
                tx.execute(
                    "INSERT INTO workspaces (id, name, code, organization, department, description, \
                            icon, lead, visibility, allowed_affiliations, data_classification, organization_id) \
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
                        &ws.organization_id,
                    ],
                )?;
            }
            crate::cache::bump(&mut tx, "workspaces")?;
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

    pub fn get_collaborators(
        &self,
        workspace_id: &str,
    ) -> Result<Vec<CollaboratorRecord>, RepositoryError> {
        let ws_id = workspace_id.to_string();
        self.with_client(move |client| {
            let rows = client.query(
                "SELECT id, workspace_id, eppn, name, role, scoped_affiliation, department, added_at::text, app_slug \
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
                app_slug: r.get(8),
            }).collect())
        })
    }

    pub fn upsert_collaborator(&self, c: &CollaboratorRecord) -> Result<(), RepositoryError> {
        let c = c.clone();
        let (kind, id) = crate::service::members::split_principal(&c.eppn);
        let (kind, id) = (kind.as_str().to_string(), id.to_string());
        self.with_client(move |client| {
            let mut tx = client.transaction()?;
            tx.execute(
                "INSERT INTO workspace_collaborators (id, workspace_id, eppn, name, role, scoped_affiliation, department, principal_type, principal_id, app_slug) \
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10) \
                 ON CONFLICT (workspace_id, eppn, app_slug) DO UPDATE SET \
                    role = EXCLUDED.role, \
                    name = EXCLUDED.name, \
                    scoped_affiliation = EXCLUDED.scoped_affiliation, \
                    department = EXCLUDED.department, \
                    principal_type = EXCLUDED.principal_type, \
                    principal_id = EXCLUDED.principal_id",
                &[&c.id, &c.workspace_id, &c.eppn, &c.name, &c.role, &c.scoped_affiliation, &c.department, &kind, &id, &c.app_slug],
            )?;
            crate::cache::bump(&mut tx, "workspaces")?;
            tx.commit()?;
            Ok(())
        })
    }

    /// Removes one membership: the workspace-wide one when `app` is empty, else the one limited to that app.
    pub fn remove_collaborator(
        &self,
        workspace_id: &str,
        eppn: &str,
        app: &str,
    ) -> Result<(), RepositoryError> {
        let ws_id = workspace_id.to_string();
        let eppn_str = eppn.to_string();
        let app = app.to_string();
        self.with_client(move |client| {
            let mut tx = client.transaction()?;
            tx.execute(
                "DELETE FROM workspace_collaborators WHERE workspace_id = $1 AND eppn = $2 AND app_slug = $3",
                &[&ws_id, &eppn_str, &app],
            )?;
            crate::cache::bump(&mut tx, "workspaces")?;
            tx.commit()?;
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
                "SELECT id, app_slug, data, ceds_mapping, is_ferpa_sensitive, created_at::text AS created_at_text, created_by, version \
                 FROM dataset_records WHERE app_slug = $1 AND deleted_at IS NULL ORDER BY created_at ASC, id ASC",
                &[&slug],
            )?;
            Ok(rows.iter().map(record_from_row).collect())
        })
    }

    /// One page of a table, in `(created_at, id)` order. `cursor` is the last row of the previous
    /// page. There is no `OFFSET`, so the cost does not grow with how far a reader has paged.
    pub fn list_records_page(
        &self,
        app_slug: &str,
        table_id: &str,
        limit: usize,
        cursor: Option<(String, String)>,
    ) -> Result<Vec<DatasetRecord>, RepositoryError> {
        let (slug, table) = (app_slug.to_string(), table_id.to_string());
        let limit = i64::try_from(limit).unwrap_or(i64::MAX);
        self.with_client(move |client| {
            count_page_statement();
            let rows = match cursor {
                Some((created_at, id)) => {
                    client.query(PAGE_SQL, &[&slug, &table, &created_at, &id, &limit])?
                }
                None => client.query(FIRST_PAGE_SQL, &[&slug, &table, &limit])?,
            };
            Ok(rows.iter().map(record_from_row).collect())
        })
    }

    /// One page that filters or sorts. Use `list_records_page` when there is neither.
    pub fn aggregate_records(
        &self,
        plan: &AggregatePlan,
    ) -> Result<AggregateResultRaw, RepositoryError> {
        let (sql, params) = build_aggregate_sql(plan);
        let is_group_by = plan.group_by.is_some();
        self.with_client(move |client| {
            let boxed = to_sql_params(params);
            let refs: Vec<&(dyn postgres::types::ToSql + Sync)> =
                boxed.iter().map(|b| b.as_ref()).collect();
            if is_group_by {
                let rows = client.query(&sql, &refs)?;
                let mut groups = Vec::new();
                for r in rows {
                    let key: String = r.get(0);
                    let key2: Option<String> = r.get(1);
                    let val: Option<f64> = r.get(2);
                    groups.push(AggregateGroupRow {
                        key,
                        key2,
                        value: val.unwrap_or(0.0),
                    });
                }
                let has_more = groups.len() > 200;
                groups.truncate(200);
                Ok(AggregateResultRaw {
                    value: None,
                    groups: Some(groups),
                    has_more,
                })
            } else {
                let row = client.query_one(&sql, &refs)?;
                let val: Option<f64> = row.get(0);
                Ok(AggregateResultRaw {
                    value: val,
                    groups: None,
                    has_more: false,
                })
            }
        })
    }

    pub fn query_records(
        &self,
        app_slug: &str,
        query: &RecordQuery,
    ) -> Result<Vec<DatasetRecord>, RepositoryError> {
        let (sql, params) = build_record_query(app_slug, query);
        self.with_client(move |client| {
            count_page_statement();
            let boxed = to_sql_params(params);
            let refs: Vec<&(dyn postgres::types::ToSql + Sync)> =
                boxed.iter().map(|b| b.as_ref()).collect();
            let rows = client.query(&sql, &refs)?;
            Ok(rows.iter().map(record_from_row).collect())
        })
    }

    /// True when the table holds more than `limit` rows. It reads at most `limit + 1` index entries.
    pub fn table_has_more_than(
        &self,
        app_slug: &str,
        table_id: &str,
        limit: i64,
    ) -> Result<bool, RepositoryError> {
        let (slug, table) = (app_slug.to_string(), table_id.to_string());
        self.with_client(move |client| {
            let n: i64 = client
                .query_one(
                    "SELECT count(*) FROM (SELECT 1 FROM dataset_records WHERE app_slug = $1 AND table_id = $2 AND deleted_at IS NULL LIMIT $3) t",
                    &[&slug, &table, &(limit + 1)],
                )?
                .get(0);
            Ok(n > limit)
        })
    }

    pub fn count_scoped_records(
        &self,
        app_slug: &str,
        table_id: &str,
        scope: Option<&ScopePlan>,
    ) -> Result<u64, RepositoryError> {
        let (slug, table) = (app_slug.to_string(), table_id.to_string());
        let mut params = vec![SqlParam::Text(slug), SqlParam::Text(table)];
        let scope_clause = match scope {
            Some(s) => s.clause("id", app_slug, table_id, &mut params),
            None => String::new(),
        };
        let sql = format!(
            "SELECT count(*) FROM dataset_records WHERE app_slug = $1 AND table_id = $2 AND deleted_at IS NULL{scope_clause}"
        );
        self.with_client(move |client| {
            let boxed = to_sql_params(params);
            let refs: Vec<&(dyn postgres::types::ToSql + Sync)> =
                boxed.iter().map(|b| b.as_ref()).collect();
            let row = client.query_one(&sql, &refs)?;
            let count: i64 = row.get(0);
            Ok(count as u64)
        })
    }

    pub fn get_record(
        &self,
        app_slug: &str,
        id: &str,
    ) -> Result<Option<DatasetRecord>, RepositoryError> {
        let (slug, id) = (app_slug.to_string(), id.to_string());
        self.with_client(move |client| {
            let row = client.query_opt(
                "SELECT id, app_slug, data, ceds_mapping, is_ferpa_sensitive, created_at::text AS created_at_text, created_by, version \
                 FROM dataset_records WHERE app_slug = $1 AND id = $2 AND deleted_at IS NULL",
                &[&slug, &id],
            )?;
            Ok(row.as_ref().map(record_from_row))
        })
    }

    /// A new record. The version starts at 1. The history row commits with it.
    pub fn advisory_hash(app_slug: &str, table_id: &str, field: &str, value: &str) -> i64 {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        (app_slug, table_id, field, value).hash(&mut hasher);
        hasher.finish() as i64
    }

    pub fn check_unique_constraints(
        tx: &mut postgres::Transaction<'_>,
        app_slug: &str,
        table: &str,
        record_id: &str,
        data: &Value,
        fields: &[scaffoldry_engine::FieldSpec],
    ) -> Result<(), RepositoryError> {
        for f in fields {
            if f.constraints.as_ref().is_some_and(|c| c.unique) {
                if let Some(val) = data.get(&f.name) {
                    if !val.is_null() {
                        let val_str = match val {
                            Value::String(s) => s.to_lowercase(),
                            Value::Number(n) => n.to_string(),
                            Value::Bool(b) => b.to_string(),
                            _ => val.to_string(),
                        };
                        let lock_key = Self::advisory_hash(app_slug, table, &f.name, &val_str);
                        tx.query_one("SELECT pg_advisory_xact_lock($1)", &[&lock_key])?;

                        let clash: Option<String> = match val {
                            Value::Number(n) => {
                                let num = n.as_f64().unwrap_or(0.0);
                                tx.query_opt(
                                "SELECT record_id FROM record_index WHERE app_slug = $1 AND table_id = $2 AND field = $3 AND num = $4 AND record_id != $5 LIMIT 1",
                                &[&app_slug, &table, &f.name, &num, &record_id],
                            )?.map(|r| r.get(0))
                            }
                            Value::String(s) => {
                                let s_lower = s.to_lowercase();
                                tx.query_opt(
                                "SELECT record_id FROM record_index WHERE app_slug = $1 AND table_id = $2 AND field = $3 AND txt = $4 AND record_id != $5 LIMIT 1",
                                &[&app_slug, &table, &f.name, &s_lower, &record_id],
                            )?.map(|r| r.get(0))
                            }
                            _ => None,
                        };
                        if clash.is_some() {
                            return Err(RepositoryError::ConstraintViolation {
                                message: format!("field '{}' violates unique constraint", f.name),
                                field: f.name.clone(),
                                constraint: "unique".to_string(),
                            });
                        }
                    }
                }
            }
        }
        Ok(())
    }

    pub fn insert_record(
        &self,
        rec: &DatasetRecord,
        ctx: &WriteCtx,
    ) -> Result<(), RepositoryError> {
        let (rec, ctx) = (rec.clone(), ctx.clone());
        self.with_client(move |client| {
            let mut tx = client.transaction()?;
            let table = table_id_of(&rec.data);
            Self::check_unique_constraints(
                &mut tx,
                &rec.app_slug,
                &table,
                &rec.id,
                &rec.data,
                &ctx.index_fields,
            )?;
            insert_record_in_tx(&mut tx, &rec, &ctx)?;
            tx.commit()?;
            Ok(())
        })
    }

    /// One conditional update. A write that changes nothing is skipped. The history row commits
    /// with the write, or neither does.
    pub fn update_record_checked(
        &self,
        app_slug: &str,
        id: &str,
        data: &Value,
        expected_version: i32,
        ctx: &WriteCtx,
    ) -> Result<RecordWrite, RepositoryError> {
        let (slug, id, data, ctx) = (
            app_slug.to_string(),
            id.to_string(),
            data.clone(),
            ctx.clone(),
        );
        self.with_client(move |client| {
            let mut tx = client.transaction()?;
            let table = table_id_of(&data);
            Self::check_unique_constraints(&mut tx, &slug, &table, &id, &data, &ctx.index_fields)?;
            let outcome =
                update_record_in_tx(&mut tx, &slug, &id, &data, Some(expected_version), &ctx)?;
            tx.commit()?;
            Ok(outcome)
        })
    }

    /// A write by the system, such as an automation. It always applies and still bumps the version,
    /// unless it changes nothing.
    pub fn overwrite_record_data(
        &self,
        app_slug: &str,
        id: &str,
        data: &Value,
        ctx: &WriteCtx,
    ) -> Result<Option<i32>, RepositoryError> {
        let (slug, id, data, ctx) = (
            app_slug.to_string(),
            id.to_string(),
            data.clone(),
            ctx.clone(),
        );
        self.with_client(move |client| {
            let mut tx = client.transaction()?;
            let outcome = update_record_in_tx(&mut tx, &slug, &id, &data, None, &ctx)?;
            tx.commit()?;
            Ok(match outcome {
                RecordWrite::Saved(version) => Some(version),
                RecordWrite::Missing | RecordWrite::Conflict(_) => None,
            })
        })
    }

    pub fn delete_record(
        &self,
        app_slug: &str,
        id: &str,
        ctx: &WriteCtx,
    ) -> Result<bool, RepositoryError> {
        let (slug, id, ctx) = (app_slug.to_string(), id.to_string(), ctx.clone());
        self.with_client(move |client| {
            let mut tx = client.transaction()?;
            let outcome = delete_record_in_tx(&mut tx, &slug, &id, None, &ctx)?;
            tx.commit()?;
            Ok(matches!(outcome, RecordWrite::Saved(_)))
        })
    }

    /// Deletes a record only if it is still at `expected_version`.
    pub fn delete_record_checked(
        &self,
        app_slug: &str,
        id: &str,
        expected_version: i32,
        ctx: &WriteCtx,
    ) -> Result<RecordWrite, RepositoryError> {
        let (slug, id, ctx) = (app_slug.to_string(), id.to_string(), ctx.clone());
        self.with_client(move |client| {
            let mut tx = client.transaction()?;
            let outcome = delete_record_in_tx(&mut tx, &slug, &id, Some(expected_version), &ctx)?;
            tx.commit()?;
            Ok(outcome)
        })
    }

    /// One batch of a flag catch-up: raises the stored flag of each record after `after` that holds one
    /// of `fields`. Returns the records read, the records raised, and the last `(created_at, id)`.
    /// A flag is only raised here. This is a derived value, so it writes no history row.
    pub fn raise_flags_batch(
        &self,
        app_slug: &str,
        table_id: &str,
        fields: &[String],
        after: Option<&(String, String)>,
        limit: i64,
    ) -> Result<FlagBatch, RepositoryError> {
        let (slug, table, names, from) = (
            app_slug.to_string(),
            table_id.to_string(),
            fields.to_vec(),
            after.cloned(),
        );
        self.with_client(move |client| {
            let mut tx = client.transaction()?;
            let rows = match &from {
                None => tx.query(
                    "SELECT id, created_at::text FROM dataset_records WHERE app_slug = $1 AND table_id = $2 \
                     ORDER BY created_at, id LIMIT $3",
                    &[&slug, &table, &limit],
                )?,
                Some((at, id)) => tx.query(
                    "SELECT id, created_at::text FROM dataset_records WHERE app_slug = $1 AND table_id = $2 \
                     AND (created_at, id) > ($3::text::timestamptz, $4::text) ORDER BY created_at, id LIMIT $5",
                    &[&slug, &table, at, id, &limit],
                )?,
            };
            let ids: Vec<String> = rows.iter().map(|r| r.get(0)).collect();
            let changed = tx.execute(
                "UPDATE dataset_records SET is_ferpa_sensitive = true \
                 WHERE id = ANY($1) AND NOT is_ferpa_sensitive AND EXISTS ( \
                     SELECT 1 FROM unnest($2::text[]) AS f(name) \
                     WHERE data->f.name IS NOT NULL AND data->f.name NOT IN ('null'::jsonb, '\"\"'::jsonb, '[]'::jsonb))",
                &[&ids, &names],
            )?;
            tx.commit()?;
            Ok((rows.len(), changed, rows.last().map(|r| (r.get::<_, String>(1), r.get::<_, String>(0)))))
        })
    }

    /// Inserts the record, or replaces the stored one and bumps its version. For fixtures.
    pub fn upsert_record(
        &self,
        rec: &DatasetRecord,
        ctx: &WriteCtx,
    ) -> Result<(), RepositoryError> {
        let (mut rec, ctx) = (rec.clone(), ctx.clone());
        apply_formulas(&mut rec.data, &ctx);
        self.with_client(move |client| {
            let mut tx = client.transaction()?;
            let before = tx.query_opt("SELECT data FROM dataset_records WHERE id = $1 FOR UPDATE", &[&rec.id])?;
            let old: Option<Value> = before.map(|r| r.get(0));
            let table = table_id_of(&rec.data);
            let version: i32 = tx
                .query_one(
                    "INSERT INTO dataset_records (id, app_slug, data, ceds_mapping, is_ferpa_sensitive, created_by, table_id) \
                     VALUES ($1, $2, $3, $4, $5, $6, $7) \
                     ON CONFLICT (id) DO UPDATE SET \
                        data = EXCLUDED.data, \
                        ceds_mapping = EXCLUDED.ceds_mapping, \
                        is_ferpa_sensitive = EXCLUDED.is_ferpa_sensitive, \
                        table_id = EXCLUDED.table_id, \
                        version = dataset_records.version + 1 \
                     RETURNING version",
                    &[&rec.id, &rec.app_slug, &rec.data, &rec.ceds_mapping, &rec.is_ferpa_sensitive, &rec.created_by, &table],
                )?
                .get(0);
            crate::repository_index::write_record_index(&mut tx, &rec.app_slug, &table, &rec.id, &rec.data, &ctx.index_fields)?;
            crate::repository_links::write_record_links(&mut tx, &rec.app_slug, &rec.id, old.as_ref(), &rec.data, &ctx)?;
            let changes = diff_fields(old.as_ref(), Some(&rec.data), &ctx.computed_fields);
            if old.is_none() || !changes.is_empty() {
                let action = if old.is_none() { "create" } else { "update" };
                write_history(&mut tx, Touched { app_slug: &rec.app_slug, table_id: &table, record_id: &rec.id, version, action }, &changes, &ctx)?;
            }
            tx.commit()?;
            Ok(())
        })
    }

    /// History rows, newest first. `before` is the id of the last row of the previous page.
    pub fn list_history(
        &self,
        app_slug: &str,
        filter: &HistoryFilter,
        before: Option<i64>,
        limit: usize,
    ) -> Result<Vec<HistoryEntry>, RepositoryError> {
        let (slug, f) = (app_slug.to_string(), filter.clone());
        let limit = i64::try_from(limit).unwrap_or(i64::MAX);
        self.with_client(move |client| {
            let rows = client.query(
                "SELECT id, app_slug, table_id, record_id, version, action, changes, actor, actor_kind, token_label, \
                        batch_id::text, reverts_batch::text, proposal_id::text, \
                        to_char(at AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS.MS\"Z\"') \
                 FROM record_history WHERE app_slug = $1 \
                   AND ($2::text IS NULL OR record_id = $2) \
                   AND ($3::text IS NULL OR table_id = $3) \
                   AND ($4::text IS NULL OR actor = $4) \
                   AND ($5::text IS NULL OR actor_kind = $5) \
                   AND ($6::text IS NULL OR at >= $6::text::timestamptz) \
                   AND ($7::text IS NULL OR at <= $7::text::timestamptz) \
                   AND ($8::bigint IS NULL OR id < $8) \
                 ORDER BY id DESC LIMIT $9",
                &[&slug, &f.record_id, &f.table_id, &f.actor, &f.kind, &f.from, &f.to, &before, &limit],
            )?;
            Ok(rows
                .iter()
                .map(|r| HistoryEntry {
                    id: r.get(0),
                    app_slug: r.get(1),
                    table_id: r.get(2),
                    record_id: r.get(3),
                    version: r.get(4),
                    action: r.get(5),
                    changes: r.get(6),
                    actor: r.get(7),
                    actor_kind: r.get(8),
                    token_label: r.get(9),
                    batch_id: r.get(10),
                    reverts_batch: r.get(11),
                    proposal_id: r.get(12),
                    at: r.get(13),
                })
                .collect())
        })
    }

    pub fn count_records_by_app_slugs(
        &self,
        app_slugs: &[String],
    ) -> Result<HashMap<String, usize>, RepositoryError> {
        if app_slugs.is_empty() {
            return Ok(HashMap::new());
        }
        let slugs = app_slugs.to_vec();
        self.with_client(move |client| {
            let rows = client.query(
                "SELECT app_slug, count(*)::bigint FROM dataset_records WHERE app_slug = ANY($1) AND deleted_at IS NULL GROUP BY app_slug",
                &[&slugs],
            )?;
            let mut map = HashMap::new();
            for r in rows {
                let slug: String = r.get(0);
                let count: i64 = r.get(1);
                map.insert(slug, count as usize);
            }
            Ok(map)
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
        view_as_by: None,
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

    pub fn append_ledger_entry(
        &self,
        entry: &LedgerEntry,
        payload: &Value,
    ) -> Result<(), RepositoryError> {
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

    pub fn app_manifest_version(&self, slug: &str) -> Result<Option<i32>, RepositoryError> {
        let slug = slug.to_string();
        self.with_client(move |client| {
            let row = client.query_opt(
                "SELECT version FROM app_manifests WHERE slug = $1",
                &[&slug],
            )?;
            Ok(row.map(|r| r.get(0)))
        })
    }

    /// Saves a manifest only when the stored version is the one the caller read.
    pub fn update_app_manifest_checked(
        &self,
        m: &AppManifest,
        expected_version: i32,
    ) -> Result<RecordWrite, RepositoryError> {
        let m = m.clone();
        self.with_client(move |client| {
            let mut tx = client.transaction()?;
            let manifest_json = serde_json::to_value(&m)?;
            let updated = tx.query_opt(
                "UPDATE app_manifests SET title = $2, description = $3, organization_code = $4, department = $5, \
                        herm_capability_id = $6, custom_domain = $7, custom_domain_verified = $8, workspace_id = $9, \
                        manifest = $10, version = version + 1, updated_at = NOW() \
                 WHERE slug = $1 AND version = $11 RETURNING version",
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
                    &expected_version,
                ],
            )?;
            if let Some(row) = updated {
                crate::cache::bump(&mut tx, "manifests")?;
                tx.commit()?;
                return Ok(RecordWrite::Saved(row.get(0)));
            }
            let current = tx.query_opt("SELECT version FROM app_manifests WHERE slug = $1", &[&m.slug])?;
            Ok(match current {
                Some(row) => RecordWrite::Conflict(row.get(0)),
                None => RecordWrite::Missing,
            })
        })
    }

    pub fn upsert_app_manifest(&self, m: &AppManifest) -> Result<(), RepositoryError> {
        let m = m.clone();
        self.with_client(move |client| {
            let mut tx = client.transaction()?;
            let manifest_json = serde_json::to_value(&m)?;
            tx.execute(
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
                    version = app_manifests.version + 1, \
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
            crate::cache::bump(&mut tx, "manifests")?;
            tx.commit()?;
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
            let mut tx = client.transaction()?;
            let payload_json = serde_json::to_value(&ds)?;
            tx.execute(
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
            crate::cache::bump(&mut tx, "datasets")?;
            tx.commit()?;
            Ok(())
        })
    }

    // -------------------------------------------------------------------------
    // DATASET RELATIONSHIPS
    // -------------------------------------------------------------------------

    pub fn list_dataset_relationships(&self) -> Result<Vec<DatasetRelationship>, RepositoryError> {
        self.with_client(|client| {
            let rows = client.query("SELECT payload FROM dataset_relationships", &[])?;
            let mut rels = Vec::new();
            for r in rows {
                let val: Value = r.get(0);
                let rel: DatasetRelationship = serde_json::from_value(val)?;
                rels.push(rel);
            }
            Ok(rels)
        })
    }

    pub fn upsert_dataset_relationship(
        &self,
        rel: &DatasetRelationship,
    ) -> Result<(), RepositoryError> {
        let rel = rel.clone();
        self.with_client(move |client| {
            let mut tx = client.transaction()?;
            let payload_json = serde_json::to_value(&rel)?;
            let rel_type = format!("{:?}", rel.relationship_type);
            tx.execute(
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
            crate::cache::bump(&mut tx, "relationships")?;
            tx.commit()?;
            Ok(())
        })
    }

    // -------------------------------------------------------------------------
    // WORKFLOW AUTOMATIONS
    // -------------------------------------------------------------------------

    pub fn list_workflow_automations(
        &self,
        app_slug: Option<&str>,
    ) -> Result<Vec<AutomationRule>, RepositoryError> {
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

    pub fn get_process_instance(
        &self,
        id: &str,
    ) -> Result<Option<ProcessInstance>, RepositoryError> {
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
                &[&user.id, &user.user_name, &user.active, &payload_json],
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

    pub fn list_hierarchies(&self) -> Result<Vec<crate::state::HierarchyRecord>, RepositoryError> {
        self.with_client(|client| {
            let rows = client.query(
                "SELECT key, name, kind, source, status, created_by, created_at::text, retired_at::text \
                 FROM hierarchies ORDER BY key ASC",
                &[],
            )?;
            let mut list = Vec::new();
            for r in rows {
                list.push(crate::state::HierarchyRecord {
                    key: r.get(0),
                    name: r.get(1),
                    kind: r.get(2),
                    source: r.get(3),
                    status: r.get(4),
                    created_by: r.get(5),
                    created_at: r.get(6),
                    retired_at: r.get(7),
                });
            }
            Ok(list)
        })
    }

    pub fn get_hierarchy(
        &self,
        key: &str,
    ) -> Result<Option<crate::state::HierarchyRecord>, RepositoryError> {
        let key_str = key.to_string();
        self.with_client(move |client| {
            let row = client.query_opt(
                "SELECT key, name, kind, source, status, created_by, created_at::text, retired_at::text \
                 FROM hierarchies WHERE key = $1",
                &[&key_str],
            )?;
            Ok(row.map(|r| crate::state::HierarchyRecord {
                key: r.get(0),
                name: r.get(1),
                kind: r.get(2),
                source: r.get(3),
                status: r.get(4),
                created_by: r.get(5),
                created_at: r.get(6),
                retired_at: r.get(7),
            }))
        })
    }

    pub fn list_org_closure(
        &self,
    ) -> Result<Vec<scaffoldry_core::tree::OrgClosureRow>, RepositoryError> {
        self.with_client(|client| {
            let rows = client.query(
                "SELECT hierarchy_key, ancestor, descendant, depth \
                 FROM org_closure ORDER BY depth ASC",
                &[],
            )?;
            let mut list = Vec::new();
            for r in rows {
                list.push(scaffoldry_core::tree::OrgClosureRow {
                    hierarchy_key: r.get(0),
                    ancestor: r.get(1),
                    descendant: r.get(2),
                    depth: r.get(3),
                });
            }
            Ok(list)
        })
    }

    pub fn list_organizations(&self) -> Result<Vec<OrganizationNode>, RepositoryError> {
        self.with_client(|client| {
            let rows = client.query(
                "SELECT id, parent_id, name, code, org_type, hierarchy_key FROM organizations ORDER BY name ASC",
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
                    hierarchy_key: r.get(5),
                });
            }
            Ok(orgs)
        })
    }

    pub fn upsert_organization(&self, org: &OrganizationNode) -> Result<(), RepositoryError> {
        const TREE_LOCK: i64 = 0x5343_4f52_4754_5245; // "SCORGTRE"
        let org = org.clone();
        let hierarchy_key = if org.hierarchy_key.is_empty() {
            "academic".to_string()
        } else {
            org.hierarchy_key
        };
        self.with_client(move |client| {
            let mut tx = client.transaction()?;
            tx.query_one("SELECT pg_advisory_xact_lock($1)", &[&TREE_LOCK])?;
            tx.execute(
                "INSERT INTO organizations (id, parent_id, name, code, org_type, hierarchy_key) \
                 VALUES ($1, $2, $3, $4, $5, $6) \
                 ON CONFLICT (code) DO UPDATE SET \
                    parent_id = EXCLUDED.parent_id, \
                    name = EXCLUDED.name, \
                    org_type = EXCLUDED.org_type, \
                    hierarchy_key = EXCLUDED.hierarchy_key",
                &[
                    &org.id,
                    &org.parent_id,
                    &org.name,
                    &org.code,
                    &org.org_type,
                    &hierarchy_key,
                ],
            )?;

            // Self-row at depth 0
            tx.execute(
                "INSERT INTO org_closure (hierarchy_key, ancestor, descendant, depth) \
                 VALUES ($1, $2, $2, 0) \
                 ON CONFLICT (ancestor, descendant) DO NOTHING",
                &[&hierarchy_key, &org.id],
            )?;

            // If parent_id is set, copy parent's ancestors
            if let Some(parent_id) = org.parent_id {
                tx.execute(
                    "INSERT INTO org_closure (hierarchy_key, ancestor, descendant, depth) \
                     SELECT $1, ancestor, $2, (depth + 1)::smallint \
                     FROM org_closure \
                     WHERE descendant = $3 \
                     ON CONFLICT (ancestor, descendant) DO NOTHING",
                    &[&hierarchy_key, &org.id, &parent_id],
                )?;
            }

            crate::cache::bump(&mut tx, "organizations")?;
            tx.commit()?;
            Ok(())
        })
    }

    /// Changes a unit's parent or name. The unit's row is locked and the tree is checked for a cycle inside the same
    /// transaction, so two moves made at once cannot each pass a check made before the other committed.
    ///
    /// `ponytail:` one advisory lock serializes every change to the shape of the tree. Moving a unit is rare, and a lock on
    /// the unit and its ancestors would have to be taken in an order that two moves in opposite directions share.
    pub fn move_organization(&self, org: &OrganizationNode) -> Result<OrgMove, RepositoryError> {
        const TREE_LOCK: i64 = 0x5343_4f52_4754_5245; // "SCORGTRE"
        let org = org.clone();
        self.with_client(move |client| {
            let mut tx = client.transaction()?;
            tx.query_one("SELECT pg_advisory_xact_lock($1)", &[&TREE_LOCK])?;

            let unit_row = match tx.query_opt(
                "SELECT hierarchy_key, parent_id FROM organizations WHERE id = $1 FOR UPDATE",
                &[&org.id],
            )? {
                Some(r) => r,
                None => return Ok(OrgMove::Missing),
            };
            let unit_hierarchy: String = unit_row.get(0);
            let old_parent: Option<uuid::Uuid> = unit_row.get(1);

            if org.parent_id == old_parent {
                // Name or org_type change only
                tx.execute(
                    "UPDATE organizations SET name = $2, org_type = $3 WHERE id = $1",
                    &[&org.id, &org.name, &org.org_type],
                )?;
                crate::cache::bump(&mut tx, "organizations")?;
                tx.commit()?;
                return Ok(OrgMove::Moved);
            }

            if let Some(new_parent_id) = org.parent_id {
                if new_parent_id == org.id {
                    return Ok(OrgMove::Cycle);
                }
                let parent_row = match tx.query_opt(
                    "SELECT hierarchy_key FROM organizations WHERE id = $1",
                    &[&new_parent_id],
                )? {
                    Some(r) => r,
                    None => return Ok(OrgMove::Missing),
                };
                let parent_hierarchy: String = parent_row.get(0);
                if parent_hierarchy != unit_hierarchy {
                    return Ok(OrgMove::BadHierarchy);
                }

                // Check cycle using org_closure: is new_parent a descendant of org.id?
                let is_descendant = tx
                    .query_opt(
                        "SELECT 1 FROM org_closure WHERE ancestor = $1 AND descendant = $2",
                        &[&org.id, &new_parent_id],
                    )?
                    .is_some();
                if is_descendant {
                    return Ok(OrgMove::Cycle);
                }
            }

            tx.execute(
                "UPDATE organizations SET parent_id = $2, name = $3, org_type = $4 WHERE id = $1",
                &[&org.id, &org.parent_id, &org.name, &org.org_type],
            )?;

            // 1. Delete closure rows connecting ancestors strictly above org.id to any node in org.id's subtree
            tx.execute(
                "DELETE FROM org_closure \
                 WHERE descendant IN (SELECT descendant FROM org_closure WHERE ancestor = $1) \
                   AND ancestor NOT IN (SELECT descendant FROM org_closure WHERE ancestor = $1)",
                &[&org.id],
            )?;

            // 2. If moving under a new parent, connect ancestors of new_parent to descendants of org.id
            if let Some(new_parent_id) = org.parent_id {
                tx.execute(
                    "INSERT INTO org_closure (hierarchy_key, ancestor, descendant, depth) \
                     SELECT \
                         p_anc.hierarchy_key, \
                         p_anc.ancestor, \
                         sub_desc.descendant, \
                         (p_anc.depth + 1 + sub_desc.depth)::smallint \
                     FROM org_closure p_anc \
                     CROSS JOIN org_closure sub_desc \
                     WHERE p_anc.descendant = $1 AND sub_desc.ancestor = $2 \
                     ON CONFLICT (ancestor, descendant) DO NOTHING",
                    &[&new_parent_id, &org.id],
                )?;
            }

            crate::cache::bump(&mut tx, "organizations")?;
            tx.commit()?;
            Ok(OrgMove::Moved)
        })
    }

    pub fn list_roles(&self) -> Result<Vec<RoleRow>, RepositoryError> {
        self.with_client(|client| {
            let rows = client.query(
                "SELECT r.id, r.person_id, p.eppn, r.organization_id, r.role_title, \
                        r.scoped_affiliation, r.is_primary, COALESCE(r.source, 'api'), r.position_key \
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
                    position_key: r.get(8),
                });
            }
            Ok(roles)
        })
    }

    pub fn upsert_role(&self, role: &RoleRow) -> Result<(), RepositoryError> {
        let role = role.clone();
        self.with_client(move |client| upsert_role_in(client, &role))
    }

    pub fn list_position_types(&self) -> Result<Vec<crate::state::PositionType>, RepositoryError> {
        self.with_client(|client| {
            let rows = client.query(
                "SELECT key, name, description, org_types, max_holders, retired_at::text, created_by, created_at::text \
                 FROM position_types ORDER BY key",
                &[],
            )?;
            Ok(rows
                .iter()
                .map(|r| crate::state::PositionType {
                    key: r.get(0),
                    name: r.get(1),
                    description: r.get(2),
                    org_types: serde_json::from_value(r.get::<_, serde_json::Value>(3)).unwrap_or_default(),
                    max_holders: r.get(4),
                    retired_at: r.get(5),
                    created_by: r.get(6),
                    created_at: r.get(7),
                })
                .collect())
        })
    }

    pub fn upsert_position_type(
        &self,
        pt: &crate::state::PositionType,
    ) -> Result<(), RepositoryError> {
        let pt = pt.clone();
        self.with_client(move |client| {
            let mut tx = client.transaction()?;
            let org_types = serde_json::to_value(&pt.org_types).unwrap_or(serde_json::Value::Null);
            let retired = pt.retired_at.is_some();
            tx.execute(
                "INSERT INTO position_types (key, name, description, org_types, max_holders, retired_at, created_by) \
                 VALUES ($1, $2, $3, $4, $5, CASE WHEN $6::bool THEN NOW() END, $7) \
                 ON CONFLICT (key) DO UPDATE SET \
                    name = EXCLUDED.name, \
                    description = EXCLUDED.description, \
                    org_types = EXCLUDED.org_types, \
                    max_holders = EXCLUDED.max_holders, \
                    retired_at = CASE WHEN $6::bool THEN COALESCE(position_types.retired_at, NOW()) END",
                &[&pt.key, &pt.name, &pt.description, &org_types, &pt.max_holders, &retired, &pt.created_by],
            )?;
            crate::cache::bump(&mut tx, "position_types")?;
            tx.commit()?;
            Ok(())
        })
    }

    pub fn delete_role(&self, id: uuid::Uuid) -> Result<(), RepositoryError> {
        self.with_client(move |client| {
            client.execute("DELETE FROM roles WHERE id = $1", &[&id])?;
            Ok(())
        })
    }

    /// Removes the roles SCIM holds for a person, before a sync writes the ones it now reports. A `platform_admin` or
    /// `unit_admin` row that SCIM wrote before SCIM stopped granting them is kept when `keep_admin_rows` is set. A
    /// Platform Admin re-appoints or removes it. A person who is deactivated or deleted loses it with the rest.
    pub fn delete_scim_roles_for_user(
        &self,
        eppn: &str,
        keep_admin_rows: bool,
    ) -> Result<(), RepositoryError> {
        let eppn_str = eppn.to_string();
        self.with_client(move |client| {
            client.execute(
                "DELETE FROM roles r USING persons p \
                 WHERE r.person_id = p.id AND p.eppn = $1 AND r.source = 'scim' \
                   AND ($2::bool = false OR r.scoped_affiliation NOT IN ('platform_admin', 'unit_admin'))",
                &[&eppn_str, &keep_admin_rows],
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
                &[&group.id, &group.display_name, &payload_json],
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
            let entry = append_ledger_in_tx(&mut tx, params)?;
            tx.commit()?;
            Ok(entry)
        })
    }
}

/// The settings a locked update wrote, and the ledger entry written with them.
pub type SettingsUpdate = (Vec<(String, serde_json::Value)>, LedgerEntry);

fn upsert_setting(
    tx: &mut postgres::Transaction<'_>,
    key: &str,
    value: &serde_json::Value,
    by: &str,
) -> Result<(), RepositoryError> {
    tx.execute(
        "INSERT INTO platform_settings (key, value, updated_by, updated_at) VALUES ($1, $2, $3, NOW()) \
         ON CONFLICT (key) DO UPDATE SET value = EXCLUDED.value, updated_by = EXCLUDED.updated_by, updated_at = NOW()",
        &[&key, value, &by],
    )?;
    Ok(())
}

/// Appends one ledger entry inside the caller's transaction, so a caller can keep it or lose it
/// together with its own writes.
pub(crate) fn append_ledger_in_tx(
    tx: &mut postgres::Transaction<'_>,
    params: AppendDecisionParams,
) -> Result<LedgerEntry, RepositoryError> {
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
    Ok(entry)
}

fn parse_decision_type(s: &str) -> Result<DecisionType, RepositoryError> {
    DecisionType::parse(s)
        .ok_or_else(|| RepositoryError::TamperDetected(format!("Unknown decision type: {s}")))
}

impl PostgresRepository {
    pub fn insert_api_token(&self, token: &crate::state::ApiToken) -> Result<(), RepositoryError> {
        let token = token.clone();
        self.with_client(move |client| {
            client.execute(
                "INSERT INTO api_tokens (token_hash, id, kind, eppn, label, original_admin, created_at, expires_at, last_used_at, revoked_at, app_slug)                  VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)                  ON CONFLICT (token_hash) DO UPDATE SET                     label = EXCLUDED.label,                     last_used_at = EXCLUDED.last_used_at,                     revoked_at = EXCLUDED.revoked_at",
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
                    &token.app_slug,
                ],
            )?;
            Ok(())
        })
    }

    pub fn get_api_token(
        &self,
        token_hash: &str,
    ) -> Result<Option<crate::state::ApiToken>, RepositoryError> {
        let hash = token_hash.to_string();
        self.with_client(move |client| {
            let row_opt = client.query_opt(
                "SELECT token_hash, id, kind, eppn, label, original_admin,                         created_at, expires_at, last_used_at, revoked_at, app_slug                  FROM api_tokens WHERE token_hash = $1",
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
                    app_slug: r.get(10),
                })),
                None => Ok(None),
            }
        })
    }

    pub fn update_token_last_used(
        &self,
        token_hash: &str,
        now: chrono::DateTime<chrono::Utc>,
    ) -> Result<(), RepositoryError> {
        let hash = token_hash.to_string();
        self.with_client(move |client| {
            client.execute(
                "UPDATE api_tokens SET last_used_at = $2 WHERE token_hash = $1",
                &[&hash, &now],
            )?;
            Ok(())
        })
    }

    pub fn revoke_api_token(
        &self,
        id: uuid::Uuid,
        caller_eppn: &str,
        is_platform_admin: bool,
    ) -> Result<bool, RepositoryError> {
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

    pub fn list_api_tokens_for_eppn(
        &self,
        eppn: &str,
    ) -> Result<Vec<crate::state::ApiToken>, RepositoryError> {
        let eppn = eppn.to_string();
        self.with_client(move |client| {
            let rows = client.query(
                "SELECT token_hash, id, kind, eppn, label, original_admin,                         created_at::text, expires_at::text, last_used_at::text, revoked_at::text, app_slug                  FROM api_tokens WHERE eppn = $1 AND kind = 'agent' AND revoked_at IS NULL ORDER BY created_at DESC",
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
                    app_slug: r.get(10),
                });
            }
            Ok(list)
        })
    }

    pub fn list_all_api_tokens_for_eppn(
        &self,
        eppn: &str,
    ) -> Result<Vec<crate::state::ApiToken>, RepositoryError> {
        let eppn = eppn.to_string();
        self.with_client(move |client| {
            let rows = client.query(
                "SELECT token_hash, id, kind, eppn, label, original_admin,                         created_at::text, expires_at::text, last_used_at::text, revoked_at::text, app_slug                  FROM api_tokens WHERE eppn = $1  ORDER BY created_at DESC",
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
                    app_slug: r.get(10),
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

    pub fn get_scim_user_by_username(
        &self,
        user_name: &str,
    ) -> Result<Option<crate::state::ScimUser>, RepositoryError> {
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

    pub fn get_platform_settings(
        &self,
    ) -> Result<std::collections::HashMap<String, serde_json::Value>, RepositoryError> {
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

    /// Writes a setting, raises the `settings` counter, and appends the ledger entry, in one transaction. A server that
    /// reads the counter after the commit reloads the setting.
    pub fn put_platform_setting(
        &self,
        key: &str,
        value: &serde_json::Value,
        updated_by: &str,
        ledger: AppendDecisionParams,
    ) -> Result<LedgerEntry, RepositoryError> {
        let (k, val, by) = (key.to_string(), value.clone(), updated_by.to_string());
        self.with_client(move |client| {
            let mut tx = client.transaction()?;
            upsert_setting(&mut tx, &k, &val, &by)?;
            crate::cache::bump(&mut tx, "settings")?;
            let entry = append_ledger_in_tx(&mut tx, ledger)?;
            tx.commit()?;
            Ok(entry)
        })
    }

    /// Changes two settings that depend on each other. Both rows are locked first, so two requests that change them
    /// take turns on any number of server instances. `decide` sees the current values and returns what to write and the
    /// ledger entry, or a refusal, which writes nothing.
    pub fn update_settings_locked<E: Send + 'static>(
        &self,
        updated_by: &str,
        keys: [&'static str; 2],
        decide: impl FnOnce(
                [serde_json::Value; 2],
            )
                -> Result<(Vec<(String, serde_json::Value)>, AppendDecisionParams), E>
            + Send
            + 'static,
    ) -> Result<Result<SettingsUpdate, E>, RepositoryError> {
        let by = updated_by.to_string();
        self.with_client(move |client| {
            let mut tx = client.transaction()?;
            for k in keys {
                tx.execute(
                    "INSERT INTO platform_settings (key, value, updated_by, updated_at) VALUES ($1, '[]'::jsonb, $2, NOW()) ON CONFLICT (key) DO NOTHING",
                    &[&k, &by],
                )?;
            }
            let rows = tx.query(
                "SELECT key, value FROM platform_settings WHERE key = ANY($1) ORDER BY key FOR UPDATE",
                &[&keys.to_vec()],
            )?;
            let value_of = |k: &str| -> serde_json::Value {
                rows.iter().find(|r| r.get::<_, String>(0) == k).map(|r| r.get(1)).unwrap_or(serde_json::json!([]))
            };
            let current = [value_of(keys[0]), value_of(keys[1])];
            match decide(current) {
                Err(refusal) => {
                    tx.rollback()?;
                    Ok(Err(refusal))
                }
                Ok((updates, ledger)) => {
                    for (k, v) in &updates {
                        upsert_setting(&mut tx, k, v, &by)?;
                    }
                    crate::cache::bump(&mut tx, "settings")?;
                    let entry = append_ledger_in_tx(&mut tx, ledger)?;
                    tx.commit()?;
                    Ok(Ok((updates, entry)))
                }
            }
        })
    }

    pub fn count_legacy_workspace_guards(&self) -> Result<i64, RepositoryError> {
        self.with_client(|client| {
            let row = client.query_one("SELECT COUNT(*) FROM workspace_guards_legacy", &[])?;
            let count: i64 = row.get(0);
            Ok(count)
        })
    }

    pub fn list_all_workspace_guards(
        &self,
    ) -> Result<Vec<scaffoldry_core::WorkspaceGuardRecord>, RepositoryError> {
        self.with_client(|client| {
            let rows = client.query(
                "SELECT workspace_id, version, rules, compiled, reason, created_by, created_at::text                  FROM workspace_guards ORDER BY workspace_id ASC, version ASC",
                &[],
            )?;
            let mut list = Vec::new();
            for r in rows {
                list.push(scaffoldry_core::WorkspaceGuardRecord {
                    workspace_id: r.get(0),
                    version: r.get(1),
                    rules: r.get(2),
                    compiled: r.get(3),
                    reason: r.get(4),
                    created_by: r.get(5),
                    created_at: r.get(6),
                });
            }
            Ok(list)
        })
    }

    pub fn get_workspace_guards(
        &self,
        workspace_id: &str,
    ) -> Result<Vec<scaffoldry_core::WorkspaceGuardRecord>, RepositoryError> {
        let ws_id = workspace_id.to_string();
        self.with_client(move |client| {
            let rows = client.query(
                "SELECT workspace_id, version, rules, compiled, reason, created_by, created_at::text                  FROM workspace_guards WHERE workspace_id = $1 ORDER BY version ASC",
                &[&ws_id],
            )?;
            let mut list = Vec::new();
            for r in rows {
                list.push(scaffoldry_core::WorkspaceGuardRecord {
                    workspace_id: r.get(0),
                    version: r.get(1),
                    rules: r.get(2),
                    compiled: r.get(3),
                    reason: r.get(4),
                    created_by: r.get(5),
                    created_at: r.get(6),
                });
            }
            Ok(list)
        })
    }

    pub fn insert_workspace_guard(
        &self,
        guard: &scaffoldry_core::WorkspaceGuardRecord,
    ) -> Result<(), RepositoryError> {
        let g = guard.clone();
        self.with_client(move |client| {
            let mut tx = client.transaction()?;
            tx.execute(
                "INSERT INTO workspace_guards (workspace_id, version, rules, compiled, reason, created_by, created_at)                  VALUES ($1, $2, $3, $4, $5, $6, NOW())",
                &[&g.workspace_id, &g.version, &g.rules, &g.compiled, &g.reason, &g.created_by],
            )?;
            crate::cache::bump(&mut tx, "guards")?;
            tx.commit()?;
            Ok(())
        })
    }
}

/// A token row read with its times as text: `token_hash, id, kind, eppn, label, original_admin, created_at, expires_at,
/// last_used_at, revoked_at, app_slug`.
pub(crate) fn api_token_of_text_row(
    r: &postgres::Row,
) -> Result<crate::state::ApiToken, RepositoryError> {
    let parse_time = |s: &str| -> chrono::DateTime<chrono::Utc> {
        chrono::DateTime::parse_from_rfc3339(s)
            .or_else(|_| chrono::DateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S%.f%#z"))
            .map(|d| d.with_timezone(&chrono::Utc))
            .unwrap_or_else(|_| chrono::Utc::now())
    };
    let created: String = r.get(6);
    let expires: String = r.get(7);
    let last_used: Option<String> = r.get(8);
    let revoked: Option<String> = r.get(9);
    Ok(crate::state::ApiToken {
        token_hash: r.get(0),
        id: r.get(1),
        kind: r.get(2),
        eppn: r.get(3),
        label: r.get(4),
        original_admin: r.get(5),
        created_at: parse_time(&created),
        expires_at: parse_time(&expires),
        last_used_at: last_used.map(|s| parse_time(&s)),
        revoked_at: revoked.map(|s| parse_time(&s)),
        app_slug: r.get(10),
    })
}

/// Writes one appointment or holding, and the person it belongs to when the platform has not met them. It runs on
/// a connection or inside a transaction.
pub(crate) fn upsert_role_in(
    client: &mut impl postgres::GenericClient,
    role: &RoleRow,
) -> Result<(), RepositoryError> {
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
        // Another server may have added the person a moment ago, under another id.
        client
            .query_one("SELECT id FROM persons WHERE eppn = $1", &[&role.eppn])?
            .get::<_, uuid::Uuid>(0)
    };

    client.execute(
        "INSERT INTO roles (id, person_id, organization_id, role_title, scoped_affiliation, is_primary, source, position_key) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8) \
         ON CONFLICT (id) DO UPDATE SET \
            organization_id = EXCLUDED.organization_id, \
            role_title = EXCLUDED.role_title, \
            scoped_affiliation = EXCLUDED.scoped_affiliation, \
            is_primary = EXCLUDED.is_primary, \
            source = EXCLUDED.source, \
            position_key = EXCLUDED.position_key",
        &[
            &role.id,
            &person_id,
            &role.organization_id,
            &role.role_title,
            &role.scoped_affiliation,
            &role.is_primary,
            &role.source,
            &role.position_key,
        ],
    )?;
    Ok(())
}
