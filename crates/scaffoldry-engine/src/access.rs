//! Evaluates an access predicate over a record, and compiles it to SQL over `record_index`.
//!
//! Both read a field the way the index stores it (`index_rows`), and both normalise a literal the
//! way the index normalises a value (`index_value`), so they cannot disagree about case, length,
//! number format, or dates. A test runs thousands of random cases against PostgreSQL to prove it.

use crate::index::{index_column, index_rows, index_value, IndexColumn, IndexCols};
use crate::{FieldSpec, FieldType};
use scaffoldry_core::access::{check, AccessError, Op, Operand, Pred};
use serde_json::Value;

/// A bound parameter of a compiled predicate. A date travels as text and is cast in the statement.
#[derive(Debug, Clone, PartialEq)]
pub enum Param {
    Text(String),
    Texts(Vec<String>),
    Num(f64),
    Nums(Vec<f64>),
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum CompileError {
    #[error("{0}")]
    Access(#[from] AccessError),
    #[error("field '{0}' is not indexed, so a rule cannot use it")]
    NotIndexed(String),
    #[error("field '{0}' has a type a rule cannot compare")]
    UnsupportedType(String),
    #[error("field '{0}' is text, which a rule can match but not order")]
    TextOrdering(String),
    #[error("the predicate still names the person. Substitute first")]
    NotSubstituted,
}

/// An instant as microseconds since 1970, from the form `parse_instant` returns.
fn instant_micros(s: &str) -> Option<i64> {
    let num = |t: &str| t.parse::<i64>().ok();
    let (y, mo, d) = (num(s.get(0..4)?)?, num(s.get(5..7)?)?, num(s.get(8..10)?)?);
    let (h, mi, sec) = (num(s.get(11..13)?)?, num(s.get(14..16)?)?, num(s.get(17..19)?)?);
    let mut rest = s.get(19..)?;
    let mut micros = 0i64;
    if let Some(frac) = rest.strip_prefix('.') {
        let digits: String = frac.chars().take_while(char::is_ascii_digit).collect();
        rest = &frac[digits.len()..];
        micros = format!("{:0<6}", &digits[..digits.len().min(6)]).parse().ok()?;
    }
    let offset = match rest {
        "Z" | "+00:00" => 0,
        _ if rest.len() == 6 => {
            let sign = if rest.starts_with('-') { -1 } else { 1 };
            sign * (num(&rest[1..3])? * 3600 + num(&rest[4..6])? * 60)
        }
        _ => return None,
    };
    // Days from civil date (Howard Hinnant's algorithm).
    let y = if mo <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * (if mo > 2 { mo - 3 } else { mo + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    Some(((days * 86_400 + h * 3600 + mi * 60 + sec - offset) * 1_000_000) + micros)
}

fn is_supported(t: FieldType) -> bool {
    !matches!(t, FieldType::Formula | FieldType::Link | FieldType::Lookup)
}

/// The values of a field in a record, as the index holds them. A blank value is absent.
fn stored(field: &FieldSpec, record: &Value) -> Vec<IndexCols> {
    let value = record.get(&field.name).unwrap_or(&Value::Null);
    index_rows(field, value).into_iter().filter(|c| c.0.is_some() || c.1.is_some() || c.2.is_some()).collect()
}

/// The values of an operand, normalised like stored values. A value that does not fit is dropped.
fn operand_cols(field: &FieldSpec, operand: &Operand) -> Vec<IndexCols> {
    let Operand::Literal(v) = operand else { return Vec::new() };
    let items: Vec<&Value> = match v {
        Value::Array(a) => a.iter().collect(),
        other => vec![other],
    };
    items.into_iter().map(|i| index_value(field, i)).filter(|c| c.0.is_some() || c.1.is_some() || c.2.is_some()).collect()
}

fn same(a: &IndexCols, b: &IndexCols) -> bool {
    match (a, b) {
        ((Some(x), _, _), (Some(y), _, _)) => x == y,
        ((_, Some(x), _), (_, Some(y), _)) => x == y,
        ((_, _, Some(x)), (_, _, Some(y))) => instant_micros(x).is_some() && instant_micros(x) == instant_micros(y),
        _ => false,
    }
}

fn order(a: &IndexCols, b: &IndexCols) -> Option<std::cmp::Ordering> {
    match (a, b) {
        ((Some(x), _, _), (Some(y), _, _)) => x.partial_cmp(y),
        ((_, _, Some(x)), (_, _, Some(y))) => Some(instant_micros(x)?.cmp(&instant_micros(y)?)),
        _ => None,
    }
}

/// True when the record satisfies the predicate. A field the table does not have is blank. A
/// predicate that still names the person is false.
pub fn eval(pred: &Pred, record: &Value, fields: &[FieldSpec]) -> bool {
    match pred {
        Pred::True => true,
        Pred::And(ps) => ps.iter().all(|p| eval(p, record, fields)),
        Pred::Or(ps) => ps.iter().any(|p| eval(p, record, fields)),
        Pred::Not(p) => !eval(p, record, fields),
        Pred::Cmp { field, op, value } => {
            let Some(spec) = fields.iter().find(|f| &f.name == field).filter(|f| is_supported(f.field_type)) else {
                return matches!(op, Op::Ne | Op::IsEmpty);
            };
            let have = stored(spec, record);
            let want = operand_cols(spec, value);
            let equal = have.iter().any(|h| want.iter().any(|w| same(h, w)));
            match op {
                Op::Eq | Op::In => equal,
                Op::Ne => !equal,
                Op::IsEmpty => have.is_empty(),
                Op::IsNotEmpty => !have.is_empty(),
                Op::Gt | Op::Lt => {
                    let wanted = if *op == Op::Gt { std::cmp::Ordering::Greater } else { std::cmp::Ordering::Less };
                    want.first().is_some_and(|w| have.iter().any(|h| order(h, w) == Some(wanted)))
                }
            }
        }
    }
}

struct Compiler<'a> {
    indexed: &'a [FieldSpec],
    params: Vec<Param>,
    first: usize,
}

/// Placeholders 0 and 1 of a compiled predicate hold the app and the table. Every subquery names
/// them, so the planner can start from `record_index`'s leading columns.
const APP: usize = 0;
const TABLE: usize = 1;

impl Compiler<'_> {
    fn bind(&mut self, p: Param, cast: &str) -> String {
        self.params.push(p);
        format!("${}{cast}", self.first + self.params.len() - 1)
    }

    fn exists(&mut self, field: &FieldSpec, test: &str) -> String {
        let name = self.bind(Param::Text(field.name.clone()), "");
        let (app, table) = (self.first + APP, self.first + TABLE);
        format!("EXISTS (SELECT 1 FROM record_index i WHERE i.app_slug = ${app} AND i.table_id = ${table} AND i.field = {name} AND i.record_id = r.id{test})")
    }

    fn cmp(&mut self, field: &str, op: Op, value: &Operand) -> Result<String, CompileError> {
        if matches!(value, Operand::Person(_)) || field.starts_with("person.") {
            return Err(CompileError::NotSubstituted);
        }
        let spec = self.indexed.iter().find(|f| f.name == field).ok_or_else(|| CompileError::NotIndexed(field.to_string()))?;
        if !is_supported(spec.field_type) {
            return Err(CompileError::UnsupportedType(field.to_string()));
        }
        let column = index_column(spec.field_type);
        let col = match column {
            IndexColumn::Num => "i.num",
            IndexColumn::Txt => "i.txt",
            IndexColumn::Ts => "i.ts",
        };
        if matches!(op, Op::IsEmpty | Op::IsNotEmpty) {
            let exists = self.exists(spec, &format!(" AND {col} IS NOT NULL"));
            return Ok(if op == Op::IsEmpty { format!("NOT {exists}") } else { exists });
        }
        let want = operand_cols(spec, value);
        let bound = |cols: &[IndexCols]| -> Option<(Param, &'static str)> {
            match column {
                IndexColumn::Num => Some((Param::Nums(cols.iter().filter_map(|c| c.0).collect()), "::float8[]")),
                IndexColumn::Txt => Some((Param::Texts(cols.iter().filter_map(|c| c.1.clone()).collect()), "::text[]")),
                IndexColumn::Ts => Some((Param::Texts(cols.iter().filter_map(|c| c.2.clone()).collect()), "::text[]::timestamptz[]")),
            }
        };
        match op {
            Op::Eq | Op::In | Op::Ne => {
                let sql = match bound(&want) {
                    Some((p, cast)) if !want.is_empty() => {
                        let arr = self.bind(p, cast);
                        self.exists(spec, &format!(" AND {col} = ANY({arr})"))
                    }
                    _ => "FALSE".to_string(),
                };
                Ok(if op == Op::Ne { format!("NOT ({sql})") } else { sql })
            }
            Op::Gt | Op::Lt => {
                if column == IndexColumn::Txt {
                    return Err(CompileError::TextOrdering(field.to_string()));
                }
                let Some(first) = want.first() else { return Ok("FALSE".to_string()) };
                let (param, cast) = match column {
                    IndexColumn::Num => (Param::Num(first.0.unwrap_or_default()), "::float8"),
                    _ => (Param::Text(first.2.clone().unwrap_or_default()), "::text::timestamptz"),
                };
                let sym = if op == Op::Gt { ">" } else { "<" };
                let v = self.bind(param, cast);
                Ok(self.exists(spec, &format!(" AND {col} {sym} {v}")))
            }
            Op::IsEmpty | Op::IsNotEmpty => unreachable!("handled above"),
        }
    }

    fn pred(&mut self, p: &Pred) -> Result<String, CompileError> {
        Ok(match p {
            Pred::True => "TRUE".to_string(),
            Pred::Cmp { field, op, value } => self.cmp(field, *op, value)?,
            Pred::And(ps) if ps.is_empty() => "TRUE".to_string(),
            Pred::Or(ps) if ps.is_empty() => "FALSE".to_string(),
            Pred::And(ps) => format!("({})", ps.iter().map(|p| self.pred(p)).collect::<Result<Vec<_>, _>>()?.join(" AND ")),
            Pred::Or(ps) => format!("({})", ps.iter().map(|p| self.pred(p)).collect::<Result<Vec<_>, _>>()?.join(" OR ")),
            Pred::Not(p) => format!("NOT ({})", self.pred(p)?),
        })
    }
}

/// A `WHERE` fragment over `dataset_records r`, and its parameters. Placeholders start at
/// `first_param`; the first two parameters returned are the app and the table, which the caller
/// binds in that order. Each comparison is an `EXISTS` over `record_index` for the same record, and
/// every value is a bound parameter. A field that is not in `indexed` is an error: the rule would
/// otherwise scan the table.
pub fn to_sql(pred: &Pred, indexed: &[FieldSpec], app_slug: &str, table_id: &str, first_param: usize) -> Result<(String, Vec<Param>), CompileError> {
    check(pred)?;
    let mut c = Compiler { indexed, params: vec![Param::Text(app_slug.to_string()), Param::Text(table_id.to_string())], first: first_param };
    let sql = c.pred(pred)?;
    Ok((sql, c.params))
}
