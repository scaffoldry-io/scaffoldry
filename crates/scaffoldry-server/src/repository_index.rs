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
         FROM unnest($3::text[], $4::text[], $5::float8[], $6::text[], $7::text[], $8::int2[]) AS u(f, r, n, t, s, o)",
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

/// Reads a table's records in batches and writes the index rows of `fields` for each.
fn copy_fields(tx: &mut postgres::Transaction<'_>, app_slug: &str, table_id: &str, fields: &[FieldSpec]) -> Result<u64, RepositoryError> {
    if fields.is_empty() {
        return Ok(0);
    }
    let mut written = 0u64;
    let mut after: Option<(String, String)> = None;
    loop {
        let batch = match &after {
            None => tx.query(
                "SELECT id, data, created_at::text FROM dataset_records WHERE app_slug = $1 AND table_id = $2 \
                 ORDER BY created_at, id LIMIT $3",
                &[&app_slug, &table_id, &REBUILD_BATCH],
            )?,
            Some((at, id)) => tx.query(
                "SELECT id, data, created_at::text FROM dataset_records WHERE app_slug = $1 AND table_id = $2 \
                 AND (created_at, id) > ($3::text::timestamptz, $4::text) ORDER BY created_at, id LIMIT $5",
                &[&app_slug, &table_id, &at, &id, &REBUILD_BATCH],
            )?,
        };
        if batch.is_empty() {
            break;
        }
        let mut rows = Vec::new();
        for r in &batch {
            let (id, data): (String, Value) = (r.get(0), r.get(1));
            rows.extend(rows_of(&id, &data, fields));
        }
        written += rows.len() as u64;
        insert_rows(tx, app_slug, table_id, &rows)?;
        let last = batch.last().unwrap();
        after = Some((last.get::<_, String>(2), last.get::<_, String>(0)));
        if (batch.len() as i64) < REBUILD_BATCH {
            break;
        }
    }
    Ok(written)
}

/// A table's indexed set changed: drop the rows of the fields that left, and write the rows of the
/// fields that joined. The approval calls this in its transaction.
pub(crate) fn sync_index(
    tx: &mut postgres::Transaction<'_>,
    app_slug: &str,
    table_id: &str,
    left: &[String],
    joined: &[FieldSpec],
) -> Result<u64, RepositoryError> {
    if !left.is_empty() {
        tx.execute("DELETE FROM record_index WHERE app_slug = $1 AND table_id = $2 AND field = ANY($3)", &[&app_slug, &table_id, &left])?;
    }
    // A joining field has no rows yet. Clear any stale ones so a repeat is safe.
    let names: Vec<String> = joined.iter().map(|f| f.name.clone()).collect();
    if !names.is_empty() {
        tx.execute("DELETE FROM record_index WHERE app_slug = $1 AND table_id = $2 AND field = ANY($3)", &[&app_slug, &table_id, &names])?;
    }
    copy_fields(tx, app_slug, table_id, joined)
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
        if !desc {
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

#[derive(Debug, Clone)]
pub struct IndexedClause {
    pub field: String,
    pub column: IndexColumn,
    /// True for a multi-valued field, which may match on any of its rows.
    pub multi: bool,
    pub op: FilterOperator,
    /// The text the caller gave. It is read as the type of `column`; text that does not parse matches nothing.
    pub value: String,
}

#[derive(Debug, Clone)]
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
    let predicate = |params: &mut Vec<SqlParam>, alias: &str, c: &IndexedClause| -> String {
        let cname = column_name(c.column);
        let blank = format!("({alias}.num IS NULL AND {alias}.txt IS NULL AND {alias}.ts IS NULL)");
        match c.op {
            FilterOperator::IsEmpty => blank,
            FilterOperator::IsNotEmpty => format!("NOT {blank}"),
            op => {
                let typed = match c.column {
                    IndexColumn::Num => c.value.trim().parse::<f64>().ok().filter(|n| n.is_finite()).map(IndexedValue::Num),
                    IndexColumn::Txt => Some(IndexedValue::Txt(c.value.chars().take(128).collect())),
                    IndexColumn::Ts => scaffoldry_engine::parse_instant(&c.value).map(IndexedValue::Ts),
                };
                let Some(v) = typed.and_then(|t| value_param(params, &t)) else { return "FALSE".to_string() };
                match op {
                    FilterOperator::Equals => format!("{alias}.{cname} = {v}"),
                    FilterOperator::NotEquals => format!("{alias}.{cname} IS DISTINCT FROM {v}"),
                    FilterOperator::GreaterThan => format!("{alias}.{cname} > {v}"),
                    FilterOperator::LessThan => format!("{alias}.{cname} < {v}"),
                    _ => "FALSE".to_string(),
                }
            }
        }
    };
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

    // The cursor and the order.
    let order;
    match (descending, phase) {
        (false, _) | (_, IndexPhase::All) => {
            if let Some((v, id)) = &q.after {
                let idp = push(&mut params, SqlParam::Text(id.clone()));
                match value_param(&mut params, v) {
                    Some(vp) => wheres.push(format!("(i.{col} > {vp} OR (i.{col} = {vp} AND i.record_id > ${idp}) OR i.{col} IS NULL)")),
                    None => wheres.push(format!("(i.{col} IS NULL AND i.record_id > ${idp})")),
                }
            }
            order = format!("i.{col} ASC NULLS LAST, i.record_id ASC");
        }
        (true, IndexPhase::NonBlank) => {
            wheres.push(format!("i.{col} IS NOT NULL"));
            if let Some((v, id)) = &q.after {
                let idp = push(&mut params, SqlParam::Text(id.clone()));
                if let Some(vp) = value_param(&mut params, v) {
                    wheres.push(format!("(i.{col} < {vp} OR (i.{col} = {vp} AND i.record_id < ${idp}))"));
                }
            }
            order = format!("i.{col} DESC, i.record_id DESC");
        }
        (true, IndexPhase::Blank) => {
            wheres.push(format!("i.{col} IS NULL"));
            if let Some((IndexedValue::Blank, id)) = &q.after {
                let idp = push(&mut params, SqlParam::Text(id.clone()));
                wheres.push(format!("i.record_id > ${idp}"));
            }
            order = "i.record_id ASC".to_string();
        }
    }
    let limit = push(&mut params, SqlParam::Int(i64::try_from(q.limit).unwrap_or(i64::MAX)));
    let sql = format!(
        "SELECT r.id, r.app_slug, r.data, r.ceds_mapping, r.is_ferpa_sensitive, r.created_at::text AS created_at_text, r.created_by, r.version \
         FROM record_index i JOIN dataset_records r ON r.id = i.record_id \
         WHERE {} ORDER BY {order} LIMIT ${limit}::bigint",
        wheres.join(" AND ")
    );
    (sql, params)
}
