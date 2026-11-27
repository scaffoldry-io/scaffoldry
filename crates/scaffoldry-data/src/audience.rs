//! Audience fields (data layer phase 3b).
//!
//! A table's manifest marks fields as audience fields. The value of one names people, positions or units. A copy of the policy sits on
//! the dataset (`datasets.audience_spec`), written in the same transaction as the manifest that names it, so every write reads it in
//! its own transaction. A write through any door, a request, an import or a sync, keeps `record_audience` current by [`write_with`].
//!
//! `record_audience` holds one row for each field and name, with the name in lower case:
//!
//! | Kind | The name | Who it reaches |
//! | --- | --- | --- |
//! | person | an address | that person |
//! | position | `key@unit-id` | whoever holds the position at that unit |
//! | unit | a unit id | whoever sits at that unit or under it |
//! | group | `group:id` | the members of the group |
//!
//! The read asks for the rows that name any of a reader's principals ([`set_sql`], [`probe_sql`]). It grants nothing a policy does not:
//! the caller cuts the row to the fields of the policy, and a category's requirement is asked on top. A unit a policy asks to accept
//! first is stored with `accepted = false`, and nothing reads such a row until [`accept`] sets it. A later write that leaves the name in
//! place keeps its acceptance.
//!
//! The crate decides nothing about who may accept. The server checks that, and writes the ledger entry in the same transaction.

use postgres::GenericClient;
use serde_json::Value;
use uuid::Uuid;

/// The most names one audience field of one row holds. A name after the last grants nothing.
pub const MAX_NAMES: usize = 100;

/// The rows a declaration writes at a time.
const PAGE: i64 = 2000;

/// One audience field of the policy, as the dataset holds it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AudienceField {
    pub field: String,
    pub write: bool,
    pub reach: Vec<String>,
    pub unit_acceptance: bool,
}

/// The audience fields a table declares.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct AudienceSpec {
    pub fields: Vec<AudienceField>,
}

impl AudienceSpec {
    /// The policy as the manifest writes it: a list of `{ field, grants, fields, unit_acceptance }`.
    pub fn from_json(v: &Value) -> Option<Self> {
        let list = v.as_array()?;
        let mut fields = Vec::with_capacity(list.len());
        for item in list {
            fields.push(AudienceField {
                field: item.get("field")?.as_str()?.to_string(),
                write: item.get("grants").and_then(Value::as_str) == Some("write"),
                reach: item.get("fields")?.as_array()?.iter().filter_map(|f| f.as_str().map(str::to_string)).collect(),
                unit_acceptance: item.get("unit_acceptance").and_then(Value::as_bool).unwrap_or(false),
            });
        }
        (!fields.is_empty()).then_some(Self { fields })
    }

    pub fn to_json(&self) -> Value {
        Value::Array(
            self.fields
                .iter()
                .map(|f| serde_json::json!({ "field": f.field, "grants": if f.write { "write" } else { "read" }, "fields": f.reach, "unit_acceptance": f.unit_acceptance }))
                .collect(),
        )
    }

    pub fn entry(&self, field: &str) -> Option<&AudienceField> {
        self.fields.iter().find(|f| f.field == field)
    }
}

/// What a name is, read from its shape.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Person,
    Position,
    Unit,
    Group,
}

impl Kind {
    pub fn as_str(self) -> &'static str {
        match self {
            Kind::Person => "person",
            Kind::Position => "position",
            Kind::Unit => "unit",
            Kind::Group => "group",
        }
    }
}

/// A name's kind and its stored form (lower case, trimmed). `None` for text that names nobody.
///
/// A bare unit id is a unit. `group:id` is a group. `key@unit-id` is a position. Any other text with an `@` between two parts is a person.
pub fn classify(name: &str) -> Option<(Kind, String)> {
    let name = name.trim().to_lowercase();
    if name.is_empty() || name.chars().count() > 255 {
        return None;
    }
    if Uuid::parse_str(&name).is_ok() {
        return Some((Kind::Unit, name));
    }
    if let Some(id) = name.strip_prefix("group:") {
        return (!id.is_empty() && !id.contains(char::is_whitespace)).then_some((Kind::Group, name));
    }
    let (left, right) = name.split_once('@')?;
    if left.is_empty() || right.is_empty() || name.contains(char::is_whitespace) {
        return None;
    }
    if Uuid::parse_str(right).is_ok() && left.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' || b == b'-') {
        return Some((Kind::Position, name));
    }
    Some((Kind::Person, name))
}

/// The names a field's value holds. A text is split on commas, semicolons and spaces. A list holds one name for each text in it.
pub fn names_in(value: Option<&Value>) -> Vec<(Kind, String)> {
    let mut parts: Vec<&str> = Vec::new();
    match value {
        Some(Value::String(s)) => parts.extend(s.split([',', ';', ' ', '\n', '\t'])),
        Some(Value::Array(items)) => {
            for item in items {
                if let Some(s) = item.as_str() {
                    parts.extend(s.split([',', ';', ' ', '\n', '\t']));
                }
            }
        }
        _ => {}
    }
    let mut out: Vec<(Kind, String)> = Vec::new();
    for part in parts {
        if let Some(named) = classify(part) {
            if !out.iter().any(|(_, p)| *p == named.1) {
                out.push(named);
            }
        }
        if out.len() >= MAX_NAMES {
            break;
        }
    }
    out
}

/// A row of `record_audience` as it is written.
struct Row {
    field: String,
    kind: Kind,
    principal: String,
    accepted: bool,
}

/// The rows a record's data asks for under a policy. A unit starts unaccepted when its field asks the unit to accept.
fn rows_of(spec: &AudienceSpec, data: &Value) -> Vec<Row> {
    let mut out = Vec::new();
    for f in &spec.fields {
        for (kind, principal) in names_in(data.get(&f.field)) {
            out.push(Row { accepted: !(f.unit_acceptance && kind == Kind::Unit), field: f.field.clone(), kind, principal });
        }
    }
    out
}

fn insert_rows<C: GenericClient>(tx: &mut C, app_slug: &str, table_id: &str, records: &[String], rows: &[(usize, Row)]) -> Result<(), postgres::Error> {
    if rows.is_empty() {
        return Ok(());
    }
    let ids: Vec<&str> = rows.iter().map(|(i, _)| records[*i].as_str()).collect();
    let fields: Vec<&str> = rows.iter().map(|(_, r)| r.field.as_str()).collect();
    let kinds: Vec<&str> = rows.iter().map(|(_, r)| r.kind.as_str()).collect();
    let principals: Vec<&str> = rows.iter().map(|(_, r)| r.principal.as_str()).collect();
    let accepted: Vec<bool> = rows.iter().map(|(_, r)| r.accepted).collect();
    tx.execute(
        "INSERT INTO record_audience (app_slug, table_id, record_id, field, kind, principal, accepted) \
         SELECT $1, $2, u.r, u.f, u.k, u.p, u.a FROM unnest($3::text[], $4::text[], $5::text[], $6::text[], $7::bool[]) AS u(r, f, k, p, a) \
         ON CONFLICT (record_id, field, principal) DO NOTHING",
        &[&app_slug, &table_id, &ids, &fields, &kinds, &principals, &accepted],
    )?;
    Ok(())
}

/// The policy a table declares, as the dataset row holds it.
pub fn spec_for<C: GenericClient>(c: &mut C, app_slug: &str, table_id: &str) -> Result<Option<AudienceSpec>, postgres::Error> {
    let row = c.query_opt("SELECT audience_spec FROM datasets WHERE app_slug = $1 AND table_id = $2", &[&app_slug, &table_id])?;
    Ok(row.and_then(|r| r.get::<_, Option<Value>>(0)).and_then(|v| AudienceSpec::from_json(&v)))
}

/// Keeps a record's rows in step with its data, in the caller's transaction. A name that is gone loses its row. A name that is new gets
/// one. A name that stays keeps its row, and so its acceptance. Nothing is read or written for a table with no policy.
pub fn write_with<C: GenericClient>(tx: &mut C, spec: Option<&AudienceSpec>, app_slug: &str, table_id: &str, record_id: &str, data: &Value) -> Result<(), postgres::Error> {
    let Some(spec) = spec else { return Ok(()) };
    let wanted = rows_of(spec, data);
    let fields: Vec<&str> = wanted.iter().map(|r| r.field.as_str()).collect();
    let principals: Vec<&str> = wanted.iter().map(|r| r.principal.as_str()).collect();
    tx.execute(
        "DELETE FROM record_audience WHERE record_id = $1 AND (field, principal) NOT IN (SELECT u.f, u.p FROM unnest($2::text[], $3::text[]) AS u(f, p))",
        &[&record_id, &fields, &principals],
    )?;
    let ids = [record_id.to_string()];
    let rows: Vec<(usize, Row)> = wanted.into_iter().map(|r| (0, r)).collect();
    insert_rows(tx, app_slug, table_id, &ids, &rows)
}

/// Names a policy, or takes it away with `None`, and rebuilds the rows of every live record to match. A table that names no policy
/// holds no rows. A name a policy asks a unit to accept starts unaccepted. Returns how many rows it wrote.
pub fn declare<C: GenericClient>(tx: &mut C, app_slug: &str, table_id: &str, spec: Option<&AudienceSpec>) -> Result<u64, postgres::Error> {
    let stored = spec.map(AudienceSpec::to_json);
    tx.execute("UPDATE datasets SET audience_spec = $3 WHERE app_slug = $1 AND table_id = $2", &[&app_slug, &table_id, &stored])?;
    tx.execute("DELETE FROM record_audience WHERE app_slug = $1 AND table_id = $2", &[&app_slug, &table_id])?;
    let Some(spec) = spec else { return Ok(0) };
    let (mut after, mut written) = (String::new(), 0u64);
    loop {
        let page = tx.query(
            "SELECT id, data FROM dataset_records WHERE app_slug = $1 AND table_id = $2 AND deleted_at IS NULL AND id > $3 ORDER BY id LIMIT $4",
            &[&app_slug, &table_id, &after, &PAGE],
        )?;
        if page.is_empty() {
            return Ok(written);
        }
        let ids: Vec<String> = page.iter().map(|r| r.get(0)).collect();
        let mut rows: Vec<(usize, Row)> = Vec::new();
        for (i, r) in page.iter().enumerate() {
            let data: Value = r.get(1);
            rows.extend(rows_of(spec, &data).into_iter().map(|row| (i, row)));
        }
        written += rows.len() as u64;
        insert_rows(tx, app_slug, table_id, &ids, &rows)?;
        after = ids.last().cloned().unwrap_or_default();
        if (page.len() as i64) < PAGE {
            return Ok(written);
        }
    }
}

/// Removes the rows of records that are gone for good.
pub fn delete_for_records<C: GenericClient>(tx: &mut C, record_ids: &[String]) -> Result<(), postgres::Error> {
    tx.execute("DELETE FROM record_audience WHERE record_id = ANY($1)", &[&record_ids])?;
    Ok(())
}

// ---------------------------------------------------------------------------------------------------------------------
// The read path
// ---------------------------------------------------------------------------------------------------------------------

/// The ids of the records that name any of the principals in any of the fields, as a set of `id` for a statement that reads every match
/// (a count, or a list that is cut after it is sorted). `app`, `table`, `principals` (text[]) and `fields` (text[]) are the placeholders
/// the caller bound. It reads the partial index of accepted rows by principal, so it costs the size of what is named and not of the table.
pub fn set_sql(app: &str, table: &str, principals: &str, fields: &str) -> String {
    format!(
        "SELECT ra.record_id AS id FROM record_audience ra WHERE ra.app_slug = {app} AND ra.table_id = {table} AND ra.principal = ANY({principals}) \
         AND ra.field = ANY({fields}) AND ra.accepted"
    )
}

/// The same as one test for the record named by `id_expr`, for a statement that reads records in order and stops after a page. `OFFSET 0`
/// keeps the planner from turning it back into a hashed set, which could not stop early.
pub fn probe_sql(id_expr: &str, app: &str, table: &str, principals: &str, fields: &str) -> String {
    format!(
        "EXISTS (SELECT 1 FROM record_audience ra WHERE ra.record_id = {id_expr} AND ra.app_slug = {app} AND ra.table_id = {table} \
         AND ra.principal = ANY({principals}) AND ra.field = ANY({fields}) AND ra.accepted OFFSET 0)"
    )
}

/// The fields of these records in which the principals are named and the name is accepted: `(record id, field, table id)`.
pub fn named_in<C: GenericClient>(c: &mut C, app_slug: &str, ids: &[String], principals: &[String]) -> Result<Vec<(String, String, String)>, postgres::Error> {
    let rows = c.query(
        "SELECT record_id, field, table_id FROM record_audience WHERE record_id = ANY($1) AND app_slug = $2 AND principal = ANY($3) AND accepted ORDER BY record_id, field",
        &[&ids, &app_slug, &principals],
    )?;
    Ok(rows.iter().map(|r| (r.get(0), r.get(1), r.get(2))).collect())
}

// ---------------------------------------------------------------------------------------------------------------------
// Acceptance
// ---------------------------------------------------------------------------------------------------------------------

/// A unit value that waits for its unit to accept.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Waiting {
    pub record_id: String,
    pub field: String,
    pub unit: Uuid,
}

/// The unit values of a table that wait, oldest record id first.
pub fn waiting<C: GenericClient>(c: &mut C, app_slug: &str, table_id: &str, limit: i64) -> Result<Vec<Waiting>, postgres::Error> {
    let rows = c.query(
        "SELECT a.record_id, a.field, a.principal FROM record_audience a JOIN dataset_records r ON r.id = a.record_id \
         WHERE a.app_slug = $1 AND a.table_id = $2 AND NOT a.accepted AND a.kind = 'unit' AND r.deleted_at IS NULL ORDER BY a.record_id, a.field LIMIT $3",
        &[&app_slug, &table_id, &limit],
    )?;
    Ok(rows.iter().filter_map(|r| Some(Waiting { record_id: r.get(0), field: r.get(1), unit: Uuid::parse_str(&r.get::<_, String>(2)).ok()? })).collect())
}

/// The fields of one record that name the unit and wait. Empty when the record does not name it, or it was accepted.
pub fn waiting_fields<C: GenericClient>(tx: &mut C, record_id: &str, unit: Uuid) -> Result<Vec<String>, postgres::Error> {
    let principal = unit.to_string();
    let rows = tx.query("SELECT field FROM record_audience WHERE record_id = $1 AND principal = $2 AND kind = 'unit' AND NOT accepted ORDER BY field FOR UPDATE", &[&record_id, &principal])?;
    Ok(rows.iter().map(|r| r.get(0)).collect())
}

/// Whether a live record names the unit in any field, accepted or not.
pub fn names_unit<C: GenericClient>(c: &mut C, app_slug: &str, table_id: &str, record_id: &str, unit: Uuid) -> Result<bool, postgres::Error> {
    let principal = unit.to_string();
    Ok(c.query_opt(
        "SELECT 1 FROM record_audience a JOIN dataset_records r ON r.id = a.record_id \
         WHERE a.record_id = $1 AND a.app_slug = $2 AND a.table_id = $3 AND a.principal = $4 AND a.kind = 'unit' AND r.deleted_at IS NULL LIMIT 1",
        &[&record_id, &app_slug, &table_id, &principal],
    )?
    .is_some())
}

/// Accepts the unit's name in every field of the record that waits. Returns how many fields it accepted, which is zero when none waited.
pub fn accept<C: GenericClient>(tx: &mut C, record_id: &str, unit: Uuid, by: &str) -> Result<u64, postgres::Error> {
    let principal = unit.to_string();
    tx.execute(
        "UPDATE record_audience SET accepted = TRUE, accepted_by = lower($3), accepted_at = NOW() WHERE record_id = $1 AND principal = $2 AND kind = 'unit' AND NOT accepted",
        &[&record_id, &principal, &by],
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_name_is_read_by_its_shape() {
        let unit = Uuid::from_u128(7).to_string();
        assert_eq!(classify("Ann.Lee@State.edu"), Some((Kind::Person, "ann.lee@state.edu".to_string())));
        assert_eq!(classify(&unit.to_uppercase()), Some((Kind::Unit, unit.clone())));
        assert_eq!(classify(&format!("Chair@{unit}")), Some((Kind::Position, format!("chair@{unit}"))));
        assert_eq!(classify("group:advisors"), Some((Kind::Group, "group:advisors".to_string())));
        for none in ["", "  ", "nobody", "@x", "x@", "group:", "two words@x.edu"] {
            assert_eq!(classify(none), None, "{none:?} names nobody");
        }
    }

    #[test]
    fn a_value_holds_one_name_or_many_without_repeats() {
        let names = |v: Value| names_in(Some(&v)).into_iter().map(|(_, p)| p).collect::<Vec<_>>();
        assert_eq!(names(json!("a@x.edu, B@x.edu; a@x.edu")), ["a@x.edu", "b@x.edu"]);
        assert_eq!(names(json!(["a@x.edu", "c@x.edu b@x.edu"])), ["a@x.edu", "c@x.edu", "b@x.edu"]);
        assert!(names(json!(7)).is_empty() && names_in(None).is_empty());
        let many: Vec<String> = (0..MAX_NAMES + 20).map(|i| format!("p{i}@x.edu")).collect();
        assert_eq!(names_in(Some(&json!(many))).len(), MAX_NAMES, "a name past the cap grants nothing");
    }

    #[test]
    fn a_unit_starts_unaccepted_only_where_the_policy_asks() {
        let unit = Uuid::from_u128(9).to_string();
        let spec = AudienceSpec::from_json(&json!([
            { "field": "a", "grants": "read", "fields": ["n"], "unit_acceptance": true },
            { "field": "b", "grants": "write", "fields": ["n"] }
        ]))
        .unwrap();
        let rows = rows_of(&spec, &json!({ "a": [unit.clone(), "p@x.edu"], "b": unit }));
        let state = |field: &str, p: &str| rows.iter().find(|r| r.field == field && r.principal == p).map(|r| r.accepted);
        assert_eq!(state("a", &unit), Some(false), "a unit that must accept waits");
        assert_eq!(state("a", "p@x.edu"), Some(true), "a person never waits");
        assert_eq!(state("b", &unit), Some(true), "a field that asks nothing grants at once");
    }

    #[test]
    fn the_policy_round_trips_through_its_json() {
        let spec = AudienceSpec::from_json(&json!([{ "field": "a", "grants": "write", "fields": ["x", "y"], "unit_acceptance": true }])).unwrap();
        assert_eq!(AudienceSpec::from_json(&spec.to_json()), Some(spec.clone()));
        assert!(spec.entry("a").unwrap().write);
        assert_eq!(AudienceSpec::from_json(&json!([])), None, "an empty policy is no policy");
    }
}
