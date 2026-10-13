//! The pivot index, `record_index`: writing it, rebuilding it, and reading a page through it.
//!
//! Every value from a caller is a bound parameter. A field name is checked against the manifest
//! before it gets here, and then it is a bound parameter too. It is never pasted into SQL text.

use crate::repository::{PostgresRepository, RepositoryError, SqlParam};
use crate::state::DatasetRecord;
use scaffoldry_engine::{index_rows, FieldSpec, FilterOperator, IndexColumn};
use serde_json::Value;

/// How many records a rebuild reads at a time.
const REBUILD_BATCH: i64 = 5_000;

fn column_name(c: IndexColumn) -> &'static str {
    match c {
        IndexColumn::Num => "num",
        IndexColumn::Txt => "txt",
        IndexColumn::Ts => "ts",
    }
}

// ---------------------------------------------------------------------------------------------
// Writing
// ---------------------------------------------------------------------------------------------

/// One row to insert.
struct Row {
    record_id: String,
    field: String,
    ord: i16,
    cols: scaffoldry_engine::IndexCols,
}

fn rows_of(record_id: &str, data: &Value, fields: &[FieldSpec]) -> Vec<Row> {
    let mut out = Vec::new();
    for f in fields {
        let value = data.get(&f.name).unwrap_or(&Value::Null);
        for (ord, cols) in index_rows(f, value).into_iter().enumerate() {
            out.push(Row { record_id: record_id.to_string(), field: f.name.clone(), ord: i16::try_from(ord).unwrap_or(i16::MAX), cols });
        }
    }
    out
}

fn insert_rows(tx: &mut postgres::Transaction<'_>, app_slug: &str, table_id: &str, rows: &[Row]) -> Result<(), postgres::Error> {
    if rows.is_empty() {
        return Ok(());
    }
    let fields: Vec<&str> = rows.iter().map(|r| r.field.as_str()).collect();
    let ids: Vec<&str> = rows.iter().map(|r| r.record_id.as_str()).collect();
    let nums: Vec<Option<f64>> = rows.iter().map(|r| r.cols.0).collect();
    let txts: Vec<Option<&str>> = rows.iter().map(|r| r.cols.1.as_deref()).collect();
    let tss: Vec<Option<&str>> = rows.iter().map(|r| r.cols.2.as_deref()).collect();
    let ords: Vec<i16> = rows.iter().map(|r| r.ord).collect();
    tx.execute(
        "INSERT INTO record_index (app_slug, table_id, field, record_id, num, txt, ts, ord) \
         SELECT $1, $2, u.f, u.r, u.n, u.t, u.s::text::timestamptz, u.o \
         FROM unnest($3::text[], $4::text[], $5::float8[], $6::text[], $7::text[], $8::int2[]) AS u(f, r, n, t, s, o) \
         ON CONFLICT (record_id, field, ord) DO NOTHING",
        &[&app_slug, &table_id, &fields, &ids, &nums, &txts, &tss, &ords],
    )?;
    Ok(())
}

/// Replaces a record's index rows with the current set. Called by every record write, inside its
/// transaction. A table that indexes nothing only clears.
pub(crate) fn write_record_index(
    tx: &mut postgres::Transaction<'_>,
    app_slug: &str,
    table_id: &str,
    record_id: &str,
    data: &Value,
    fields: &[FieldSpec],
) -> Result<(), postgres::Error> {
    tx.execute("DELETE FROM record_index WHERE record_id = $1", &[&record_id])?;
    insert_rows(tx, app_slug, table_id, &rows_of(record_id, data, fields))
}

/// One batch of a copy.
pub(crate) struct Batch {
    pub read: usize,
    pub written: u64,
    /// The `(created_at, id)` of the last record read.
    pub last: Option<(String, String)>,
}

/// Reads up to `limit` records after `after`, in `(created_at, id)` order, and writes the index rows
/// of `fields` for each. The records stay locked against writers until the transaction ends, so no
/// write slips between the read and the insert. A row already present is left as it is.
pub(crate) fn copy_batch(
    tx: &mut postgres::Transaction<'_>,
    app_slug: &str,
    table_id: &str,
    fields: &[FieldSpec],
    after: Option<&(String, String)>,
    limit: i64,
) -> Result<Batch, postgres::Error> {
    let batch = match after {
        None => tx.query(
            "SELECT id, data, created_at::text FROM dataset_records WHERE app_slug = $1 AND table_id = $2 AND deleted_at IS NULL \
             ORDER BY created_at, id LIMIT $3 FOR SHARE",
            &[&app_slug, &table_id, &limit],
        )?,
        Some((at, id)) => tx.query(
            "SELECT id, data, created_at::text FROM dataset_records WHERE app_slug = $1 AND table_id = $2 AND deleted_at IS NULL \
             AND (created_at, id) > ($3::text::timestamptz, $4::text) ORDER BY created_at, id LIMIT $5 FOR SHARE",
            &[&app_slug, &table_id, at, id, &limit],
        )?,
    };
    let mut rows = Vec::new();
    for r in &batch {
        let (id, data): (String, Value) = (r.get(0), r.get(1));
        rows.extend(rows_of(&id, &data, fields));
    }
    insert_rows(tx, app_slug, table_id, &rows)?;
    Ok(Batch {
        read: batch.len(),
        written: rows.len() as u64,
        last: batch.last().map(|r| (r.get::<_, String>(2), r.get::<_, String>(0))),
    })
}

/// Writes the index rows of `fields` for every record of a table, in batches.
fn copy_fields(tx: &mut postgres::Transaction<'_>, app_slug: &str, table_id: &str, fields: &[FieldSpec]) -> Result<u64, RepositoryError> {
    if fields.is_empty() {
        return Ok(0);
    }
    let mut written = 0u64;
    let mut after: Option<(String, String)> = None;
    loop {
        let batch = copy_batch(tx, app_slug, table_id, fields, after.as_ref(), REBUILD_BATCH)?;
        written += batch.written;
        after = batch.last;
        if (batch.read as i64) < REBUILD_BATCH {
            break;
        }
    }
    Ok(written)
}

/// A table's indexed set changed. The fields that left lose their rows and their status. The fields
/// that joined get a `building` status and one `build_index` job. Nothing is copied here, so an
/// approval stays fast on a table of any size. The approval calls this in its transaction.
pub(crate) fn sync_index(
    tx: &mut postgres::Transaction<'_>,
    app_slug: &str,
    table_id: &str,
    left: &[String],
    joined: &[FieldSpec],
    requested_by: &str,
) -> Result<(), RepositoryError> {
    if !left.is_empty() {
        tx.execute("DELETE FROM record_index WHERE app_slug = $1 AND table_id = $2 AND field = ANY($3)", &[&app_slug, &table_id, &left])?;
        tx.execute("DELETE FROM index_status WHERE app_slug = $1 AND table_id = $2 AND field = ANY($3)", &[&app_slug, &table_id, &left])?;
    }
    if joined.is_empty() {
        return Ok(());
    }
    let mut names: Vec<String> = joined.iter().map(|f| f.name.clone()).collect();
    names.sort();
    // A joining field has no rows yet. Clear any stale ones so a repeat is safe.
    tx.execute("DELETE FROM record_index WHERE app_slug = $1 AND table_id = $2 AND field = ANY($3)", &[&app_slug, &table_id, &names])?;
    for name in &names {
        tx.execute(
            "INSERT INTO index_status (app_slug, table_id, field, state, job_id) VALUES ($1, $2, $3, 'building', NULL) \
             ON CONFLICT (app_slug, table_id, field) DO UPDATE SET state = 'building', job_id = NULL, updated_at = NOW()",
            &[&app_slug, &table_id, name],
        )?;
    }
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(format!("{app_slug}|{table_id}|{}", names.join(",")).as_bytes());
    let key = format!("build_index:{}", digest.iter().map(|b| format!("{b:02x}")).collect::<String>());
    let payload = serde_json::json!({ "app_slug": app_slug, "table_id": table_id, "fields": names });
    let job = crate::jobs::enqueue_in_tx(tx, "build_index", &payload, requested_by, Some(&key))?;
    tx.execute(
        "UPDATE index_status SET job_id = $4 WHERE app_slug = $1 AND table_id = $2 AND field = ANY($3)",
        &[&app_slug, &table_id, &names, &job],
    )?;
    Ok(())
}

impl PostgresRepository {
    /// Rebuilds a table's index from its records, from nothing, for the whole indexed set. Returns the
    /// number of rows written.
    pub fn rebuild_index(&self, app_slug: &str, table_id: &str, fields: &[FieldSpec]) -> Result<u64, RepositoryError> {
        let (slug, table, fields) = (app_slug.to_string(), table_id.to_string(), fields.to_vec());
        self.with_client(move |client| {
            let mut tx = client.transaction()?;
            tx.execute("DELETE FROM record_index WHERE app_slug = $1 AND table_id = $2", &[&slug, &table])?;
            let n = copy_fields(&mut tx, &slug, &table, &fields)?;
            tx.commit()?;
            Ok(n)
        })
    }

    /// The percent done of the build that holds back one of `fields`, or `None` when none is building.
    pub fn index_building_percent(&self, app_slug: &str, table_id: &str, fields: &[String]) -> Result<Option<i64>, RepositoryError> {
        let (slug, table, fields) = (app_slug.to_string(), table_id.to_string(), fields.to_vec());
        self.with_client(move |client| {
            let row = client.query_opt(
                "SELECT COALESCE(j.progress_done, 0), COALESCE(j.progress_total, 0) \
                 FROM index_status s LEFT JOIN jobs j ON j.id = s.job_id \
                 WHERE s.app_slug = $1 AND s.table_id = $2 AND s.field = ANY($3) AND s.state = 'building' LIMIT 1",
                &[&slug, &table, &fields],
            )?;
            Ok(row.map(|r| {
                let (done, total): (i64, i64) = (r.get(0), r.get(1));
                if total > 0 { (done * 100 / total).clamp(0, 99) } else { 0 }
            }))
        })
    }

    /// The plan for a statement, as text. For tests that prove a statement does not scan a table.
    pub fn explain(&self, sql: &str, params: &[SqlParam]) -> Result<String, RepositoryError> {
        let (sql, params) = (format!("EXPLAIN {sql}"), params.to_vec());
        self.with_client(move |client| {
            let boxed = crate::repository::to_sql_params(params);
            let refs: Vec<&(dyn postgres::types::ToSql + Sync)> = boxed.iter().map(|b| b.as_ref()).collect();
            let rows = client.query(&sql, &refs)?;
            Ok(rows.iter().map(|r| r.get::<_, String>(0)).collect::<Vec<_>>().join("\n"))
        })
    }

    /// One page through the index. A descending sort reads the values first and the blanks after,
    /// because that is the order a backward index scan gives without a sort step.
    pub fn indexed_page(&self, q: &IndexedQuery) -> Result<Vec<DatasetRecord>, RepositoryError> {
        let desc = q.sort.as_ref().is_some_and(|s| s.descending);
        // The first page of an ascending walk has no cursor, so one statement reads the values and then the blanks.
        if !desc && q.after.is_none() {
            return self.run_indexed(q, IndexPhase::All);
        }
        let in_blanks = matches!(q.after, Some((IndexedValue::Blank, _)));
        let mut rows = if in_blanks { Vec::new() } else { self.run_indexed(q, IndexPhase::NonBlank)? };
        if in_blanks || rows.len() < q.limit {
            let mut rest = q.clone();
            rest.limit = q.limit - rows.len();
            if !in_blanks {
                rest.after = None;
            }
            rows.extend(self.run_indexed(&rest, IndexPhase::Blank)?);
        }
        Ok(rows)
    }

    fn run_indexed(&self, q: &IndexedQuery, phase: IndexPhase) -> Result<Vec<DatasetRecord>, RepositoryError> {
        let (sql, params) = build_indexed_sql(q, phase);
        self.with_client(move |client| {
            crate::repository::count_page_statement();
            let boxed = crate::repository::to_sql_params(params);
            let refs: Vec<&(dyn postgres::types::ToSql + Sync)> = boxed.iter().map(|b| b.as_ref()).collect();
            let rows = client.query(&sql, &refs)?;
            Ok(rows.iter().map(crate::repository::record_from_row).collect())
        })
    }
}

// ---------------------------------------------------------------------------------------------
// Reading
// ---------------------------------------------------------------------------------------------

/// A value in the typed column of the field it is compared to.
#[derive(Debug, Clone, PartialEq)]
pub enum IndexedValue {
    Num(f64),
    Txt(String),
    /// A form PostgreSQL reads as a `timestamptz`.
    Ts(String),
    Blank,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct IndexedClause {
    pub field: String,
    pub column: IndexColumn,
    /// True for a multi-valued field, which may match on any of its rows.
    pub multi: bool,
    pub op: FilterOperator,
    /// The text the caller gave. It is read as the type of `column`; text that does not parse matches nothing.
    pub value: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct IndexedSort {
    pub field: String,
    pub column: IndexColumn,
    pub descending: bool,
}

/// A page read through `record_index`. Clauses join with AND. With no sort, the first clause that
/// can use the index drives the statement and orders the result.
#[derive(Debug, Clone)]
pub struct IndexedQuery {
    pub app_slug: String,
    pub table_id: String,
    pub clauses: Vec<IndexedClause>,
    pub sort: Option<IndexedSort>,
    /// The last row of the previous page: its value and its record id.
    pub after: Option<(IndexedValue, String)>,
    pub limit: usize,
    /// Only the records inside this scope.
    pub scope: Option<crate::repository::ScopePlan>,
    /// Full-text search string.
    pub search: Option<String>,
    /// A filter tree that does not fit `clauses`: groups, OR, or the newer operators. Joined with AND to the clauses.
    /// A query that has one also has a sort, which is what its pages walk.
    pub expr: Option<Expr>,
    /// Rows to skip after `after`. Only a window reads it (`build_window_sql`).
    pub offset: usize,
}

/// A filter tree whose leaves are typed clauses.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub enum Expr {
    Leaf(IndexedClause),
    And(Vec<Expr>),
    Or(Vec<Expr>),
}

impl Expr {
    /// Every clause in the tree.
    pub fn leaves(&self) -> Vec<&IndexedClause> {
        match self {
            Expr::Leaf(c) => vec![c],
            Expr::And(v) | Expr::Or(v) => v.iter().flat_map(Expr::leaves).collect(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IndexPhase {
    /// Values and blanks in one statement.
    All,
    /// A descending sort: the rows that have a value.
    NonBlank,
    /// A descending sort: the rows that have none.
    Blank,
}

fn sargable(op: FilterOperator) -> bool {
    matches!(op, FilterOperator::Equals | FilterOperator::GreaterThan | FilterOperator::LessThan | FilterOperator::IsEmpty)
}

/// The field a page walks, its column, and whether it walks backward.
pub fn walk_of(q: &IndexedQuery) -> Option<(String, IndexColumn, bool)> {
    match (&q.sort, indexed_drive(q)) {
        (Some(s), _) => Some((s.field.clone(), s.column, s.descending)),
        (None, Some(i)) => Some((q.clauses[i].field.clone(), q.clauses[i].column, false)),
        (None, None) => None,
    }
}

fn indexed_drive(q: &IndexedQuery) -> Option<usize> {
    if q.sort.is_some() {
        return None;
    }
    q.clauses.iter().position(|c| sargable(c.op)).or(if q.clauses.is_empty() { None } else { Some(0) })
}

/// The SQL for one phase of an indexed page, and its parameters in order.
pub fn build_indexed_sql(q: &IndexedQuery, phase: IndexPhase) -> (String, Vec<SqlParam>) {
    let mut params = vec![SqlParam::Text(q.app_slug.clone()), SqlParam::Text(q.table_id.clone())];
    let push = |params: &mut Vec<SqlParam>, p: SqlParam| -> usize {
        params.push(p);
        params.len()
    };

    // The field the statement walks: the sort, or else the first clause that can use the index.
    let drive = indexed_drive(q);
    let (field, column, descending, single) = match (&q.sort, drive) {
        (Some(s), _) => (s.field.clone(), s.column, s.descending, true),
        (None, Some(i)) => (q.clauses[i].field.clone(), q.clauses[i].column, false, false),
        (None, None) => (String::new(), IndexColumn::Num, false, false),
    };
    let field_name = field.clone();
    let f = push(&mut params, SqlParam::Text(field));
    let col = column_name(column);
    let mut wheres = vec![format!("i.app_slug = $1 AND i.table_id = $2 AND i.field = ${f}")];
    if single {
        wheres.push("i.ord = 0".to_string());
    }

    let value_param = |params: &mut Vec<SqlParam>, v: &IndexedValue| -> Option<String> {
        match v {
            IndexedValue::Num(n) => {
                let n = push(params, SqlParam::Float(*n));
                Some(format!("${n}::float8"))
            }
            IndexedValue::Txt(t) => {
                let n = push(params, SqlParam::Text(t.clone()));
                Some(format!("${n}::text"))
            }
            IndexedValue::Ts(t) => {
                let n = push(params, SqlParam::Text(t.clone()));
                Some(format!("${n}::text::timestamptz"))
            }
            IndexedValue::Blank => None,
        }
    };
    let predicate = |params: &mut Vec<SqlParam>, alias: &str, c: &IndexedClause| -> String { agg_predicate(params, alias, c) };
    for (i, c) in q.clauses.iter().enumerate() {
        // A clause on the field being walked reads the walked row itself.
        let on_walked = single && !c.multi && c.field == field_name;
        if Some(i) == drive || on_walked {
            wheres.push(predicate(&mut params, "i", c));
        } else {
            let cf = push(&mut params, SqlParam::Text(c.field.clone()));
            let p = predicate(&mut params, "f", c);
            wheres.push(format!("EXISTS (SELECT 1 FROM record_index f WHERE f.app_slug = $1 AND f.table_id = $2 AND f.record_id = i.record_id AND f.field = ${cf} AND {p})"));
        }
    }

    if let Some(e) = &q.expr {
        let walked = if single { Some(field_name.as_str()) } else { None };
        wheres.push(expr_sql(&mut params, e, "i.record_id", walked));
    }

    if let Some(s) = &q.search {
        if !s.trim().is_empty() {
            let sp = push(&mut params, SqlParam::Text(s.clone()));
            wheres.push(format!("r.search @@ plainto_tsquery('simple', ${sp})"));
        }
    }

    // The cursor and the order.
    // A cursor is a row comparison on the two columns the index ends with, so it is a boundary in the index and the scan
    // starts there. The older form, `value > v OR (value = v AND id > x) OR value IS NULL`, was a filter: the scan read
    // from the front of the field and threw away every row before the cursor, so a page near the end of a million rows
    // took 300 ms. The blanks sort last, so they are a phase of their own after the values.
    let order = match (descending, phase) {
        (false, IndexPhase::All) => {
            // The first page has no cursor to find, so one statement covers the values and then the blanks.
            format!("i.{col} ASC NULLS LAST, i.record_id ASC")
        }
        (false, IndexPhase::NonBlank) => {
            wheres.push(format!("i.{col} IS NOT NULL"));
            if let Some((v, id)) = &q.after {
                let idp = push(&mut params, SqlParam::Text(id.clone()));
                if let Some(vp) = value_param(&mut params, v) {
                    wheres.push(format!("(i.{col}, i.record_id) > ({vp}, ${idp})"));
                }
            }
            format!("i.{col} ASC, i.record_id ASC")
        }
        (false, IndexPhase::Blank) => {
            wheres.push(format!("i.{col} IS NULL"));
            if let Some((IndexedValue::Blank, id)) = &q.after {
                let idp = push(&mut params, SqlParam::Text(id.clone()));
                wheres.push(format!("i.record_id > ${idp}"));
            }
            // Ordered the way the index is, so it is read in place. `ORDER BY record_id` alone made the planner walk the
            // primary key and filter a million rows to find none.
            format!("i.{col} ASC NULLS LAST, i.record_id ASC")
        }
        (true, IndexPhase::All | IndexPhase::NonBlank) => {
            wheres.push(format!("i.{col} IS NOT NULL"));
            if let Some((v, id)) = &q.after {
                let idp = push(&mut params, SqlParam::Text(id.clone()));
                if let Some(vp) = value_param(&mut params, v) {
                    wheres.push(format!("(i.{col}, i.record_id) < ({vp}, ${idp})"));
                }
            }
            format!("i.{col} DESC, i.record_id DESC")
        }
        (true, IndexPhase::Blank) => {
            wheres.push(format!("i.{col} IS NULL"));
            if let Some((IndexedValue::Blank, id)) = &q.after {
                let idp = push(&mut params, SqlParam::Text(id.clone()));
                wheres.push(format!("i.record_id > ${idp}"));
            }
            // Ordered the way the index is, so it is read in place. `ORDER BY record_id` alone made the planner walk the
            // primary key and filter a million rows to find none.
            format!("i.{col} ASC NULLS LAST, i.record_id ASC")
        }
    };
    let scope = match &q.scope {
        Some(plan) => plan.clause("r.id", &q.app_slug, &q.table_id, &mut params),
        None => String::new(),
    };
    let limit = push(&mut params, SqlParam::Int(i64::try_from(q.limit).unwrap_or(i64::MAX)));
    let sql = format!(
        "SELECT r.id, r.app_slug, r.data, r.ceds_mapping, r.is_ferpa_sensitive, r.created_at::text AS created_at_text, r.created_by, r.version \
         FROM record_index i JOIN dataset_records r ON r.id = i.record_id AND r.deleted_at IS NULL \
         WHERE {}{scope} ORDER BY {order} LIMIT ${limit}::bigint",
        wheres.join(" AND ")
    );
    (sql, params)
}


// ---------------------------------------------------------------------------------------------
// Windows, counts and anchors (views phase 3)
// ---------------------------------------------------------------------------------------------


/// What an anchor build walks: the view's order and filter, already resolved (relative dates are dates by now).
/// It rides in the job's payload, so the job needs nothing from the request that asked for it.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct AnchorSpec {
    pub app_slug: String,
    pub table_id: String,
    pub view_id: uuid::Uuid,
    pub sort: IndexedSort,
    pub clauses: Vec<IndexedClause>,
    pub expr: Option<Expr>,
}

impl AnchorSpec {
    /// A name for this order and filter. Anchors built for a different one are not used.
    pub fn hash(&self) -> String {
        use sha2::{Digest, Sha256};
        let text = serde_json::to_string(&(&self.app_slug, &self.table_id, &self.sort, &self.clauses, &self.expr)).unwrap_or_default();
        Sha256::digest(text.as_bytes()).iter().map(|b| format!("{b:02x}")).collect()
    }

    pub fn query(&self) -> IndexedQuery {
        IndexedQuery {
            app_slug: self.app_slug.clone(),
            table_id: self.table_id.clone(),
            clauses: self.clauses.clone(),
            sort: Some(self.sort.clone()),
            after: None,
            limit: 0,
            scope: None,
            search: None,
            expr: self.expr.clone(),
            offset: 0,
        }
    }
}

/// The most rows a window may skip.
pub const MAX_WINDOW_OFFSET: usize = 1_000_000;
/// A jump to a row this near the front skips the rows rather than starting at an anchor.
pub const ANCHOR_STEP: usize = 1_000;
/// A count that reaches this many says `at least`.
pub const COUNT_CAP: i64 = 250_000;

/// The conditions every window and count over a walked field share, and their parameters. A row of the walked field
/// at `ord = 0` stands for each record, so one row is one record.
fn window_wheres(q: &IndexedQuery, params: &mut Vec<SqlParam>) -> String {
    let s = q.sort.as_ref().expect("a window walks a sort");
    let f = push_param(params, SqlParam::Text(s.field.clone()));
    let mut wheres = vec![format!("i.app_slug = $1 AND i.table_id = $2 AND i.field = ${f} AND i.ord = 0")];
    for c in &q.clauses {
        if !c.multi && c.field == s.field {
            wheres.push(agg_predicate(params, "i", c));
        } else {
            let cf = push_param(params, SqlParam::Text(c.field.clone()));
            let p = agg_predicate(params, "fc", c);
            wheres.push(format!("EXISTS (SELECT 1 FROM record_index fc WHERE fc.app_slug = $1 AND fc.table_id = $2 AND fc.record_id = i.record_id AND fc.field = ${cf} AND {p})"));
        }
    }
    if let Some(e) = &q.expr {
        wheres.push(expr_sql(params, e, "i.record_id", Some(s.field.as_str())));
    }
    if let Some(plan) = &q.scope {
        // The scope clause starts with its own AND.
        let clause = plan.clause("i.record_id", &q.app_slug, &q.table_id, params);
        wheres.push(clause.trim_start().trim_start_matches("AND").trim().to_string());
    }
    wheres.join(" AND ")
}

/// The order of a window. An ascending sort reads the key up with blanks last. A descending sort is exactly that
/// reversed, so one index serves both and a backward scan needs no sort step. `reverse` flips it again.
fn window_order(q: &IndexedQuery, reverse: bool) -> (String, bool) {
    let s = q.sort.as_ref().expect("a window walks a sort");
    let col = column_name(s.column);
    let backward = s.descending != reverse;
    if backward {
        (format!("i.{col} DESC NULLS FIRST, i.record_id DESC"), true)
    } else {
        (format!("i.{col} ASC NULLS LAST, i.record_id ASC"), false)
    }
}

/// One window of a view: the 200 rows from `offset`, in the order of the walked field. The skip runs on the index
/// alone and the records are fetched for the rows that survive, so the cost of a deep offset is the index scan and
/// not a join. With `reverse` the order is flipped and the caller flips the rows back.
pub fn build_window_sql(q: &IndexedQuery, reverse: bool) -> (String, Vec<SqlParam>) {
    let mut params = vec![SqlParam::Text(q.app_slug.clone()), SqlParam::Text(q.table_id.clone())];
    let wheres = window_wheres(q, &mut params);
    let s = q.sort.as_ref().expect("a window walks a sort");
    let col = column_name(s.column);
    let (order, backward) = window_order(q, reverse);
    let tail = |params: &mut Vec<SqlParam>| {
        let limit = push_param(params, SqlParam::Int(i64::try_from(q.limit).unwrap_or(i64::MAX)));
        let offset = push_param(params, SqlParam::Int(i64::try_from(q.offset).unwrap_or(i64::MAX)));
        (limit, offset)
    };
    let fetch = |inner: String, w_order: &str| {
        format!(
            "SELECT r.id, r.app_slug, r.data, r.ceds_mapping, r.is_ferpa_sensitive, r.created_at::text AS created_at_text, r.created_by, r.version \
             FROM ({inner}) w JOIN dataset_records r ON r.id = w.record_id AND r.deleted_at IS NULL ORDER BY {w_order}"
        )
    };
    // A cursor is a row comparison on the two columns the index ends with, so the scan starts at it. The blanks sort last
    // (first, when the walk runs backward), so a forward walk past a value also reads every blank, and that is a second
    // branch of its own with its own bound.
    if let (Some((v, id)), false, false) = (&q.after, backward, matches!(q.after, Some((IndexedValue::Blank, _)))) {
        let idp = push_param(&mut params, SqlParam::Text(id.clone()));
        let vp = agg_value_param(&mut params, v).expect("a value that is not blank has a parameter");
        let (limit, offset) = tail(&mut params);
        let cap = push_param(&mut params, SqlParam::Int(i64::try_from(q.limit + q.offset).unwrap_or(i64::MAX)));
        let inner = format!(
            "SELECT record_id, k FROM (\
               (SELECT i.record_id, i.{col} AS k, 0 AS part FROM record_index i WHERE {wheres} AND i.{col} IS NOT NULL AND (i.{col}, i.record_id) > ({vp}, ${idp}) \
                ORDER BY i.{col} ASC, i.record_id ASC LIMIT ${cap}::bigint) \
               UNION ALL \
               (SELECT i.record_id, i.{col} AS k, 1 AS part FROM record_index i WHERE {wheres} AND i.{col} IS NULL ORDER BY i.{col} ASC NULLS LAST, i.record_id ASC LIMIT ${cap}::bigint)\
             ) u ORDER BY part, k, record_id LIMIT ${limit}::bigint OFFSET ${offset}::bigint"
        );
        return (fetch(inner, "w.k ASC NULLS LAST, w.record_id ASC"), params);
    }
    let mut wheres = wheres;
    if let Some((v, id)) = &q.after {
        let idp = push_param(&mut params, SqlParam::Text(id.clone()));
        let vp = agg_value_param(&mut params, v);
        let after = match (vp, backward) {
            // Forward past a blank: only blanks are left.
            (None, false) => format!("(i.{col} IS NULL AND i.record_id > ${idp})"),
            (Some(vp), true) => format!("(i.{col}, i.record_id) < ({vp}, ${idp})"),
            (None, true) => format!("((i.{col} IS NULL AND i.record_id < ${idp}) OR i.{col} IS NOT NULL)"),
            (Some(_), false) => unreachable!("handled above"),
        };
        wheres = format!("{wheres} AND {after}");
    }
    let (limit, offset) = tail(&mut params);
    let w_order = order.replace("i.", "w.").replace(&format!("w.{col}"), "w.k");
    let inner = format!("SELECT i.record_id, i.{col} AS k FROM record_index i WHERE {wheres} ORDER BY {order} LIMIT ${limit}::bigint OFFSET ${offset}::bigint");
    (fetch(inner, &w_order), params)
}

/// The statement that counts the rows of a view, to a cap when `cap` is given.
pub fn build_count_sql(q: &IndexedQuery, cap: Option<i64>) -> (String, Vec<SqlParam>) {
    let mut params = vec![SqlParam::Text(q.app_slug.clone()), SqlParam::Text(q.table_id.clone())];
    let wheres = window_wheres(q, &mut params);
    let sql = match cap {
        None => format!("SELECT count(*) FROM record_index i WHERE {wheres}"),
        Some(c) => {
            let n = push_param(&mut params, SqlParam::Int(c));
            format!("SELECT count(*) FROM (SELECT 1 FROM record_index i WHERE {wheres} LIMIT ${n}::bigint) c")
        }
    };
    (sql, params)
}

/// The statement that samples the walk: for each row at a multiple of `ANCHOR_STEP`, its position and the key of the
/// row before it, as a page cursor takes it. The anchors are for the whole table, never a person's scope.
pub fn build_anchor_sql(q: &IndexedQuery) -> (String, Vec<SqlParam>) {
    let mut unscoped = q.clone();
    unscoped.scope = None;
    let mut params = vec![SqlParam::Text(q.app_slug.clone()), SqlParam::Text(q.table_id.clone())];
    let wheres = window_wheres(&unscoped, &mut params);
    let s = q.sort.as_ref().expect("anchors walk a sort");
    let (order, _) = window_order(q, false);
    let key = match s.column {
        IndexColumn::Num => "lag(i.num) OVER w".to_string(),
        IndexColumn::Txt => "lag(i.txt) OVER w".to_string(),
        IndexColumn::Ts => "to_char(lag(i.ts) OVER w AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS.US\"Z\"')".to_string(),
    };
    let step = push_param(&mut params, SqlParam::Int(ANCHOR_STEP as i64));
    let sql = format!(
        "SELECT pos, pk, pid, total FROM (SELECT row_number() OVER w - 1 AS pos, {key} AS pk, lag(i.record_id) OVER w AS pid, count(*) OVER () AS total \
         FROM record_index i WHERE {wheres} WINDOW w AS (ORDER BY {order})) a WHERE pos % ${step}::bigint = 0 ORDER BY pos"
    );
    (sql, params)
}

/// A page cursor's key for the value, as the anchors store it: the number, the text, or null.
fn key_json(row: &postgres::Row, column: IndexColumn) -> Value {
    match column {
        IndexColumn::Num => row.get::<_, Option<f64>>(1).and_then(serde_json::Number::from_f64).map(Value::Number).unwrap_or(Value::Null),
        _ => row.get::<_, Option<String>>(1).map(Value::String).unwrap_or(Value::Null),
    }
}

/// Is this error the database cancelling a statement that ran past its timeout?
fn timed_out(e: &postgres::Error) -> bool {
    e.code() == Some(&postgres::error::SqlState::QUERY_CANCELED)
}

pub enum Counted {
    /// Every row, counted.
    Exact(i64),
    /// At least this many. The exact count ran out of time.
    AtLeast(i64),
}

impl PostgresRepository {
    /// One window of a view. See `build_window_sql`.
    pub fn indexed_window(&self, q: &IndexedQuery, reverse: bool) -> Result<Vec<DatasetRecord>, RepositoryError> {
        let (sql, params) = build_window_sql(q, reverse);
        self.with_client(move |client| {
            crate::repository::count_page_statement();
            let boxed = crate::repository::to_sql_params(params);
            let refs: Vec<&(dyn postgres::types::ToSql + Sync)> = boxed.iter().map(|b| b.as_ref()).collect();
            let rows = client.query(&sql, &refs)?;
            Ok(rows.iter().map(crate::repository::record_from_row).collect())
        })
    }

    /// The row count of a view. It runs under `exact_ms`. If that runs out, a count capped at `COUNT_CAP` runs under two
    /// seconds, and the answer says at least. If that runs out too, the answer is at least nothing.
    pub fn count_window(&self, q: &IndexedQuery, exact_ms: u64) -> Result<Counted, RepositoryError> {
        let (exact_sql, exact_params) = build_count_sql(q, None);
        let (cap_sql, cap_params) = build_count_sql(q, Some(COUNT_CAP + 1));
        self.with_client(move |client| {
            let mut run = |sql: &str, params: Vec<SqlParam>, ms: u64| -> Result<Option<i64>, postgres::Error> {
                let mut tx = client.transaction()?;
                tx.batch_execute(&format!("SET LOCAL statement_timeout = {}", ms.max(1)))?;
                let boxed = crate::repository::to_sql_params(params);
                let refs: Vec<&(dyn postgres::types::ToSql + Sync)> = boxed.iter().map(|b| b.as_ref()).collect();
                match tx.query_one(sql, &refs) {
                    Ok(row) => {
                        tx.commit()?;
                        Ok(Some(row.get::<_, i64>(0)))
                    }
                    Err(e) if timed_out(&e) => Ok(None),
                    Err(e) => Err(e),
                }
            };
            if let Some(n) = run(&exact_sql, exact_params, exact_ms)? {
                return Ok(Counted::Exact(n));
            }
            Ok(match run(&cap_sql, cap_params, 2_000)? {
                Some(n) => Counted::AtLeast(n.min(COUNT_CAP)),
                None => Counted::AtLeast(0),
            })
        })
    }

    /// The anchors for a view, if they were built for `spec_hash`.
    pub fn nearest_anchor(&self, view_id: uuid::Uuid, spec_hash: &str, position: usize) -> Result<Option<(usize, Value)>, RepositoryError> {
        let (hash, pos) = (spec_hash.to_string(), position as i32);
        self.with_client(move |client| {
            let row = client.query_opt(
                "SELECT a.position, a.cursor FROM view_anchors a JOIN view_anchor_builds b ON b.view_id = a.view_id AND b.spec_hash = $2 \
                 WHERE a.view_id = $1 AND a.position <= $3 ORDER BY a.position DESC LIMIT 1",
                &[&view_id, &hash, &pos],
            )?;
            Ok(row.map(|r| (r.get::<_, i32>(0) as usize, r.get::<_, Option<Value>>(1).unwrap_or(Value::Null))))
        })
    }

    /// Samples a view's walk and replaces its anchors. Returns the number of rows the walk holds.
    pub fn build_anchors(&self, view_id: uuid::Uuid, spec_hash: &str, q: &IndexedQuery) -> Result<i64, RepositoryError> {
        let (sql, params) = build_anchor_sql(q);
        let column = q.sort.as_ref().expect("anchors walk a sort").column;
        let hash = spec_hash.to_string();
        self.with_client(move |client| {
            let boxed = crate::repository::to_sql_params(params);
            let refs: Vec<&(dyn postgres::types::ToSql + Sync)> = boxed.iter().map(|b| b.as_ref()).collect();
            let rows = client.query(&sql, &refs)?;
            let total: i64 = rows.first().map(|r| r.get::<_, i64>(3)).unwrap_or(0);
            let mut tx = client.transaction()?;
            tx.execute("DELETE FROM view_anchors WHERE view_id = $1", &[&view_id])?;
            for r in &rows {
                let pos: i64 = r.get(0);
                let id: Option<String> = r.get(2);
                let cursor = id.map(|id| serde_json::json!({ "key": key_json(r, column), "id": id }));
                tx.execute("INSERT INTO view_anchors (view_id, position, cursor) VALUES ($1, $2, $3)", &[&view_id, &(pos as i32), &cursor])?;
            }
            tx.execute(
                "INSERT INTO view_anchor_builds (view_id, spec_hash, total, built_at) VALUES ($1, $2, $3, NOW()) \
                 ON CONFLICT (view_id) DO UPDATE SET spec_hash = $2, total = $3, built_at = NOW()",
                &[&view_id, &hash, &total],
            )?;
            tx.commit()?;
            Ok(total)
        })
    }

    /// When the anchors of a view were built, if they were built for `spec_hash`.
    pub fn anchors_built(&self, view_id: uuid::Uuid, spec_hash: &str) -> Result<Option<f64>, RepositoryError> {
        let hash = spec_hash.to_string();
        self.with_client(move |client| {
            let row = client.query_opt(
                "SELECT EXTRACT(EPOCH FROM (NOW() - built_at))::float8 FROM view_anchor_builds WHERE view_id = $1 AND spec_hash = $2",
                &[&view_id, &hash],
            )?;
            Ok(row.map(|r| r.get::<_, f64>(0)))
        })
    }
}

// ---------------------------------------------------------------------------------------------
// Aggregates over record_index
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum AggregateFunction {
    #[default]
    Count,
    Sum,
    Avg,
    Min,
    Max,
    /// Rows with a value in the field. Any field type.
    Filled,
    /// Rows with none. Any field type.
    Empty,
    /// Distinct values. Any field type.
    Unique,
}

/// A date field grouped into days, weeks, months, quarters or years, in the institution's zone.
#[derive(Debug, Clone)]
pub struct Bucket {
    /// `day`, `week` (opens Monday), `month`, `quarter` or `year`. Anything else reads as `day`.
    pub unit: String,
    /// An IANA zone name. The caller has checked it is one.
    pub zone: String,
}

impl Bucket {
    fn sql_unit(&self) -> &'static str {
        match self.unit.as_str() {
            "week" => "week",
            "month" => "month",
            "quarter" => "quarter",
            "year" => "year",
            _ => "day",
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct AggregatePlan {
    pub app_slug: String,
    pub table_id: String,
    pub function: AggregateFunction,
    pub field: Option<String>,
    /// The field whose rows the statement walks when `field` is not given.
    pub driver: Option<String>,
    pub group_by: Option<String>,
    /// A second field to group by, inside the first.
    pub group_by2: Option<String>,
    /// Groups the first field, a date, by day, week, month, quarter or year.
    pub bucket: Option<Bucket>,
    /// Order the groups by key, which gives the same groups for every measure. Otherwise the biggest value comes first.
    pub order_by_key: bool,
    pub clauses: Vec<IndexedClause>,
    /// A filter tree, joined with AND to the clauses.
    pub expr: Option<Expr>,
    pub scope: Option<crate::repository::ScopePlan>,
}

#[derive(Debug, Clone)]
pub struct AggregateGroupRow {
    pub key: String,
    /// The key of the second group, when there is one.
    pub key2: Option<String>,
    pub value: f64,
}

#[derive(Debug, Clone)]
pub struct AggregateResultRaw {
    pub value: Option<f64>,
    pub groups: Option<Vec<AggregateGroupRow>>,
    pub has_more: bool,
}

fn push_param(params: &mut Vec<SqlParam>, p: SqlParam) -> usize {
    params.push(p);
    params.len()
}

fn agg_value_param(params: &mut Vec<SqlParam>, v: &IndexedValue) -> Option<String> {
    match v {
        IndexedValue::Num(n) => {
            let n = push_param(params, SqlParam::Float(*n));
            Some(format!("${n}::float8"))
        }
        IndexedValue::Txt(t) => {
            let n = push_param(params, SqlParam::Text(t.clone()));
            Some(format!("${n}::text"))
        }
        IndexedValue::Ts(t) => {
            let n = push_param(params, SqlParam::Text(t.clone()));
            Some(format!("${n}::text::timestamptz"))
        }
        IndexedValue::Blank => None,
    }
}

fn agg_predicate(params: &mut Vec<SqlParam>, alias: &str, c: &IndexedClause) -> String {
    use scaffoldry_engine::{list_of, parse_instant, range_of};
    let cname = column_name(c.column);
    let blank = format!("({alias}.num IS NULL AND {alias}.txt IS NULL AND {alias}.ts IS NULL)");
    let ts = |params: &mut Vec<SqlParam>, v: &str| -> Option<String> { parse_instant(v.trim()).map(|t| { let n = push_param(params, SqlParam::Text(t)); format!("${n}::text::timestamptz") }) };
    let num = |params: &mut Vec<SqlParam>, v: &str| -> Option<String> { v.trim().parse::<f64>().ok().filter(|n| n.is_finite()).map(|n| { let i = push_param(params, SqlParam::Float(n)); format!("${i}::float8") }) };
    match c.op {
        FilterOperator::IsEmpty => blank,
        FilterOperator::IsNotEmpty => format!("NOT {blank}"),
        FilterOperator::In => match c.column {
            IndexColumn::Txt => {
                let list: Vec<String> = list_of(&c.value).into_iter().map(|v| v.to_lowercase().chars().take(128).collect()).collect();
                if list.is_empty() {
                    return "FALSE".to_string();
                }
                let n = push_param(params, SqlParam::Texts(list));
                format!("{alias}.txt = ANY(${n}::text[])")
            }
            IndexColumn::Num => {
                let list: Vec<f64> = list_of(&c.value).iter().filter_map(|v| v.parse::<f64>().ok()).filter(|n| n.is_finite()).collect();
                if list.is_empty() {
                    return "FALSE".to_string();
                }
                let n = push_param(params, SqlParam::Floats(list));
                format!("{alias}.num = ANY(${n}::float8[])")
            }
            IndexColumn::Ts => "FALSE".to_string(),
        },
        FilterOperator::Between => {
            let Some((low, high)) = range_of(&c.value) else { return "FALSE".to_string() };
            let (a, b) = match c.column {
                IndexColumn::Num => (num(params, low), num(params, high)),
                IndexColumn::Ts => (ts(params, low), ts(params, high)),
                IndexColumn::Txt => return "FALSE".to_string(),
            };
            match (a, b) {
                (Some(a), Some(b)) => format!("{alias}.{cname} BETWEEN {a} AND {b}"),
                _ => "FALSE".to_string(),
            }
        }
        FilterOperator::Before | FilterOperator::After => match (c.column, ts(params, &c.value)) {
            (IndexColumn::Ts, Some(v)) => format!("{alias}.ts {} {v}", if c.op == FilterOperator::Before { "<" } else { ">" }),
            _ => "FALSE".to_string(),
        },
        op => {
            let typed = match c.column {
                IndexColumn::Num => c.value.trim().parse::<f64>().ok().filter(|n| n.is_finite()).map(IndexedValue::Num),
                IndexColumn::Txt => Some(IndexedValue::Txt(c.value.to_lowercase().chars().take(128).collect())),
                IndexColumn::Ts => scaffoldry_engine::parse_instant(&c.value).map(IndexedValue::Ts),
            };
            let Some(v) = typed.and_then(|t| agg_value_param(params, &t)) else { return "FALSE".to_string() };
            match op {
                FilterOperator::Equals => format!("{alias}.{cname} = {v}"),
                FilterOperator::NotEquals => format!("{alias}.{cname} IS DISTINCT FROM {v}"),
                FilterOperator::GreaterThan => format!("{alias}.{cname} > {v}"),
                FilterOperator::LessThan => format!("{alias}.{cname} < {v}"),
                _ => "FALSE".to_string(),
            }
        }
    }
}

/// The condition a filter tree puts on the record in `record_col`. A leaf on the field the statement walks reads
/// the walked row. Any other leaf asks the index for that field's row of the same record.
pub fn expr_sql(params: &mut Vec<SqlParam>, e: &Expr, record_col: &str, walked: Option<&str>) -> String {
    match e {
        Expr::Leaf(c) => {
            if !c.multi && walked == Some(c.field.as_str()) {
                return agg_predicate(params, "i", c);
            }
            let cf = push_param(params, SqlParam::Text(c.field.clone()));
            let pred = agg_predicate(params, "fc", c);
            format!("EXISTS (SELECT 1 FROM record_index fc WHERE fc.app_slug = $1 AND fc.table_id = $2 AND fc.record_id = {record_col} AND fc.field = ${cf} AND {pred})")
        }
        Expr::And(v) if v.is_empty() => "TRUE".to_string(),
        Expr::Or(v) if v.is_empty() => "FALSE".to_string(),
        Expr::And(v) => format!("({})", v.iter().map(|x| expr_sql(params, x, record_col, walked)).collect::<Vec<_>>().join(" AND ")),
        Expr::Or(v) => format!("({})", v.iter().map(|x| expr_sql(params, x, record_col, walked)).collect::<Vec<_>>().join(" OR ")),
    }
}

const BLANK: &str = "(i.num IS NULL AND i.txt IS NULL AND i.ts IS NULL)";

pub fn build_aggregate_sql(p: &AggregatePlan) -> (String, Vec<SqlParam>) {
    let mut params = vec![SqlParam::Text(p.app_slug.clone()), SqlParam::Text(p.table_id.clone())];

    if let Some(ref g) = p.group_by {
        let gp = push_param(&mut params, SqlParam::Text(g.clone()));
        let mut wheres = vec![format!("g.app_slug = $1 AND g.table_id = $2 AND g.field = ${gp} AND g.ord = 0")];
        // The key of a group as text, whatever column the field is in. A date is its instant in UTC unless it is bucketed.
        let key = |alias: &str| format!("COALESCE({alias}.txt, {alias}.num::text, to_char({alias}.ts AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS.US\"Z\"'), '')");
        let group_expr = match &p.bucket {
            Some(b) => {
                let tz = push_param(&mut params, SqlParam::Text(b.zone.clone()));
                format!("COALESCE(to_char(date_trunc('{}', g.ts AT TIME ZONE ${tz}::text), 'YYYY-MM-DD'), '')", b.sql_unit())
            }
            None => key("g"),
        };
        let mut join = String::new();
        let group2_expr = match &p.group_by2 {
            Some(g2) => {
                let g2p = push_param(&mut params, SqlParam::Text(g2.clone()));
                join.push_str(&format!(" JOIN record_index g2 ON g2.app_slug = $1 AND g2.table_id = $2 AND g2.record_id = g.record_id AND g2.field = ${g2p} AND g2.ord = 0"));
                key("g2")
            }
            None => "NULL::text".to_string(),
        };
        let agg_expr = match p.function {
            AggregateFunction::Count => "COUNT(*)".to_string(),
            fn_type => {
                let f_name = p.field.as_ref().expect("field required for non-count aggregate");
                // The measured field is one of the groups, or else it is joined.
                let alias = if f_name == g {
                    "g"
                } else if p.group_by2.as_ref() == Some(f_name) {
                    "g2"
                } else {
                    let fp = push_param(&mut params, SqlParam::Text(f_name.clone()));
                    join.push_str(&format!(" JOIN record_index f ON f.app_slug = $1 AND f.table_id = $2 AND f.record_id = g.record_id AND f.field = ${fp} AND f.ord = 0"));
                    "f"
                };
                match fn_type {
                    AggregateFunction::Sum => format!("SUM({alias}.num)"),
                    AggregateFunction::Avg => format!("AVG({alias}.num)"),
                    AggregateFunction::Min => format!("MIN({alias}.num)"),
                    AggregateFunction::Max => format!("MAX({alias}.num)"),
                    AggregateFunction::Unique => format!("COUNT(DISTINCT {})", key(alias)),
                    _ => format!("COUNT({alias}.num)"),
                }
            }
        };
        for c in &p.clauses {
            if c.field == *g && !c.multi {
                wheres.push(agg_predicate(&mut params, "g", c));
            } else {
                let cf = push_param(&mut params, SqlParam::Text(c.field.clone()));
                let pred = agg_predicate(&mut params, "fc", c);
                wheres.push(format!("EXISTS (SELECT 1 FROM record_index fc WHERE fc.app_slug = $1 AND fc.table_id = $2 AND fc.record_id = g.record_id AND fc.field = ${cf} AND {pred})"));
            }
        }
        if let Some(e) = &p.expr {
            wheres.push(expr_sql(&mut params, e, "g.record_id", Some(g.as_str())));
        }
        let scope_clause = match &p.scope {
            Some(plan) => plan.clause("g.record_id", &p.app_slug, &p.table_id, &mut params),
            None => String::new(),
        };
        let order = if p.order_by_key { "ORDER BY 1, 2" } else { "ORDER BY val DESC NULLS LAST" };
        let sql = format!(
            "SELECT {group_expr} AS grp, {group2_expr} AS grp2, {agg_expr}::float8 AS val FROM record_index g{join} WHERE {}{scope_clause} GROUP BY 1, 2 {order} LIMIT 201",
            wheres.join(" AND ")
        );
        (sql, params)
    } else {
        let driving_field = match (&p.field, &p.driver) {
            (Some(f), _) | (None, Some(f)) => f.clone(),
            (None, None) => p.clauses.first().map(|c| c.field.clone()).unwrap_or_default(),
        };
        let fp = push_param(&mut params, SqlParam::Text(driving_field.clone()));
        let mut wheres = vec![format!("i.app_slug = $1 AND i.table_id = $2 AND i.field = ${fp} AND i.ord = 0")];
        let agg_expr = match p.function {
            AggregateFunction::Count => {
                if p.field.is_some() {
                    "COUNT(i.num)".to_string()
                } else {
                    "COUNT(*)".to_string()
                }
            }
            AggregateFunction::Sum => "SUM(i.num)".to_string(),
            AggregateFunction::Avg => "AVG(i.num)".to_string(),
            AggregateFunction::Min => "MIN(i.num)".to_string(),
            AggregateFunction::Max => "MAX(i.num)".to_string(),
            AggregateFunction::Filled => format!("COUNT(*) FILTER (WHERE NOT {BLANK})"),
            AggregateFunction::Empty => format!("COUNT(*) FILTER (WHERE {BLANK})"),
            AggregateFunction::Unique => format!("COUNT(DISTINCT COALESCE(i.txt, i.num::text, i.ts::text)) FILTER (WHERE NOT {BLANK})"),
        };
        for c in &p.clauses {
            if c.field == driving_field && !c.multi {
                wheres.push(agg_predicate(&mut params, "i", c));
            } else {
                let cf = push_param(&mut params, SqlParam::Text(c.field.clone()));
                let pred = agg_predicate(&mut params, "fc", c);
                wheres.push(format!("EXISTS (SELECT 1 FROM record_index fc WHERE fc.app_slug = $1 AND fc.table_id = $2 AND fc.record_id = i.record_id AND fc.field = ${cf} AND {pred})"));
            }
        }
        if let Some(e) = &p.expr {
            wheres.push(expr_sql(&mut params, e, "i.record_id", Some(driving_field.as_str())));
        }
        let scope_clause = match &p.scope {
            Some(plan) => plan.clause("i.record_id", &p.app_slug, &p.table_id, &mut params),
            None => String::new(),
        };
        let default_val = match p.function {
            AggregateFunction::Count | AggregateFunction::Sum | AggregateFunction::Filled | AggregateFunction::Empty | AggregateFunction::Unique => "COALESCE(val, 0.0)",
            _ => "val",
        };
        let sql = format!(
            "SELECT {default_val} FROM (SELECT {agg_expr}::float8 AS val FROM record_index i WHERE {}{scope_clause}) sub",
            wheres.join(" AND ")
        );
        (sql, params)
    }
}
