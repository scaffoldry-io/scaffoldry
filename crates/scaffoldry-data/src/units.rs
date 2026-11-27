//! The row's unit column (data layer phase 2c).
//!
//! A table names one of its Unit fields as its unit column. The name and the field's hierarchy are kept as JSON on the
//! dataset's row (`datasets.unit_spec`), written in the same transaction as the manifest that names them. Every write
//! reads that in its own transaction, so a write through any door, whether a request, an import or a sync, is checked the
//! same way.
//!
//! The value is a unit id. It is copied to `dataset_records.unit_id`, which has an index a list filtered by unit reads.
//! A blank value clears the column: the row is the dataset's own. A value that is not a unit of the field's hierarchy is
//! never treated as blank. It is refused by the database function `set_record_unit`, as a check violation that [`is_refused`]
//! names. The refusal carries the value, the field and the hierarchy, and no unit.
//!
//! Naming the nearest units is a separate step, because it depends on who is asking: [`nearest`] takes the units the
//! caller may see.
//!
//! A table may also require the column (data layer phase 3, `required`). A blank value is then refused too, as a check
//! violation that [`is_required_missing`] names, because a row that names no unit is read by the dataset's unit and nobody
//! chose that.
//!
//! A unit that is retired later keeps its rows, and the column keeps its id. Nothing here follows a retirement. A finding
//! for such rows is later work.

use postgres::error::SqlState;
use postgres::GenericClient;
use serde_json::Value;
use uuid::Uuid;

/// The constraint a refusal names.
pub const REFUSED: &str = "unit_not_on_list";

/// The constraint a refusal of a blank value names, for a table that requires its unit column.
pub const REQUIRED: &str = "unit_required";

/// The most record ids a list of bad values carries.
pub const MAX_LISTED: usize = 200;

/// The most units a refusal names.
pub const MAX_NEAREST: usize = 3;

/// A declared unit column: the field, and the hierarchy whose units it holds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnitSpec {
    pub field: String,
    pub hierarchy: String,
    /// A row that names no unit is refused. Off by default, so a table that declared its column before phase 3 is unchanged.
    pub required: bool,
}

impl UnitSpec {
    pub fn to_json(&self) -> Value {
        serde_json::json!({ "field": self.field, "hierarchy": self.hierarchy, "required": self.required })
    }

    pub fn from_json(v: &Value) -> Option<Self> {
        Some(Self {
            field: v.get("field")?.as_str()?.to_string(),
            hierarchy: v.get("hierarchy")?.as_str()?.to_string(),
            required: v.get("required").and_then(Value::as_bool).unwrap_or(false),
        })
    }
}

/// What a refusal says: the value as it was sent, the field, and the hierarchy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Refusal {
    pub value: String,
    pub field: String,
    pub hierarchy: String,
}

/// The text of a value, or `None` when it is blank. A blank value is no unit.
fn raw_of(v: Option<&Value>) -> Option<String> {
    let text = match v? {
        Value::Null => return None,
        Value::String(s) => s.trim().to_string(),
        other => other.to_string(),
    };
    (!text.is_empty()).then_some(text)
}

/// True when the error is a value that is not a unit on the list.
pub fn is_refused(e: &postgres::Error) -> bool {
    e.as_db_error().is_some_and(|d| d.code() == &SqlState::CHECK_VIOLATION && d.constraint() == Some(REFUSED))
}

/// What a refusal says, or `None` for any other error.
pub fn refusal(e: &postgres::Error) -> Option<Refusal> {
    let d = e.as_db_error().filter(|d| d.code() == &SqlState::CHECK_VIOLATION && d.constraint() == Some(REFUSED))?;
    Some(Refusal { value: d.message().to_string(), field: d.column().unwrap_or_default().to_string(), hierarchy: d.detail().unwrap_or_default().to_string() })
}

/// True when the error is a blank value in a table that requires its unit column.
pub fn is_required_missing(e: &postgres::Error) -> bool {
    e.as_db_error().is_some_and(|d| d.code() == &SqlState::CHECK_VIOLATION && d.constraint() == Some(REQUIRED))
}

/// The field and hierarchy of a blank value refused for a table that requires its unit column.
pub fn required_missing(e: &postgres::Error) -> Option<(String, String)> {
    let d = e.as_db_error().filter(|d| d.code() == &SqlState::CHECK_VIOLATION && d.constraint() == Some(REQUIRED))?;
    Some((d.column().unwrap_or_default().to_string(), d.detail().unwrap_or_default().to_string()))
}

/// What a writer is told when a table requires a unit and the row names none.
pub fn required_sentence(field: &str, hierarchy: &str) -> String {
    format!("This dataset requires a unit. Name one in '{field}', a unit of the '{hierarchy}' hierarchy.")
}

/// The unit column a table declares, as the dataset row holds it.
pub fn spec_for<C: GenericClient>(c: &mut C, app_slug: &str, table_id: &str) -> Result<Option<UnitSpec>, postgres::Error> {
    let row = c.query_opt("SELECT unit_spec FROM datasets WHERE app_slug = $1 AND table_id = $2", &[&app_slug, &table_id])?;
    Ok(row.and_then(|r| r.get::<_, Option<Value>>(0)).and_then(|v| UnitSpec::from_json(&v)))
}

/// Copies the unit of a record's data to its column, or clears the column when the value is blank. A value that is not a
/// unit of the field's hierarchy is refused, and [`is_refused`] names the error. Nothing is read or written for a table that
/// names no unit column.
pub fn write<C: GenericClient>(tx: &mut C, app_slug: &str, table_id: &str, record_id: &str, data: &Value) -> Result<(), postgres::Error> {
    let spec = spec_for(tx, app_slug, table_id)?;
    write_with(tx, spec.as_ref(), record_id, data)
}

/// [`write`] with the unit column already read, so a caller that reads both specs pays one round trip (`crate::specs`).
pub fn write_with<C: GenericClient>(tx: &mut C, spec: Option<&UnitSpec>, record_id: &str, data: &Value) -> Result<(), postgres::Error> {
    let Some(spec) = spec else { return Ok(()) };
    let raw = raw_of(data.get(&spec.field));
    let unit: Option<Uuid> = raw.as_deref().and_then(|r| Uuid::parse_str(r).ok());
    tx.execute("SELECT set_record_unit_checked($1, $2, $3, $4, $5, $6)", &[&record_id, &unit, &raw, &spec.hierarchy, &spec.field, &spec.required])?;
    Ok(())
}

/// Names a unit column, or takes it away with `None`, and copies the unit of every live row to match. A row whose value is
/// not a unit of the hierarchy blocks it: nothing is kept that matters, and the ids of those rows come back (at most
/// [`MAX_LISTED`]) for the caller to roll back and report. The column of a table that no longer names one is cleared.
pub fn declare<C: GenericClient>(tx: &mut C, app_slug: &str, table_id: &str, spec: Option<&UnitSpec>) -> Result<Result<u64, Vec<String>>, postgres::Error> {
    let stored = spec.map(UnitSpec::to_json);
    tx.execute("UPDATE datasets SET unit_spec = $3 WHERE app_slug = $1 AND table_id = $2", &[&app_slug, &table_id, &stored])?;
    tx.execute("UPDATE dataset_records SET unit_id = NULL WHERE app_slug = $1 AND table_id = $2 AND unit_id IS NOT NULL", &[&app_slug, &table_id])?;
    let Some(spec) = spec else { return Ok(Ok(0)) };

    // The value of a live row is its text, trimmed. A value that is blank is no unit. Any other must be a unit on the list.
    // A table that requires the column also counts a blank value as bad.
    const VALUE: &str = "NULLIF(btrim(r.data ->> $3), '')";
    let bad: Vec<String> = tx
        .query(
            &format!(
                "SELECT r.id FROM dataset_records r WHERE r.app_slug = $1 AND r.table_id = $2 AND r.deleted_at IS NULL \
                 AND (($5 AND {VALUE} IS NULL) OR ({VALUE} IS NOT NULL \
                      AND NOT EXISTS (SELECT 1 FROM organizations o WHERE o.hierarchy_key = $4 AND o.id::text = lower({VALUE})))) ORDER BY r.id LIMIT {}",
                MAX_LISTED + 1
            ),
            &[&app_slug, &table_id, &spec.field, &spec.hierarchy, &spec.required],
        )?
        .iter()
        .map(|r| r.get(0))
        .collect();
    if !bad.is_empty() {
        return Ok(Err(bad.into_iter().take(MAX_LISTED).collect()));
    }
    let n = tx.execute(
        &format!(
            "UPDATE dataset_records r SET unit_id = o.id FROM organizations o \
             WHERE r.app_slug = $1 AND r.table_id = $2 AND r.deleted_at IS NULL AND o.hierarchy_key = $4 AND o.id::text = lower({VALUE})"
        ),
        &[&app_slug, &table_id, &spec.field, &spec.hierarchy],
    )?;
    Ok(Ok(n))
}

/// The units a caller may see: every unit, or the units at and below some roots.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Visible {
    All,
    Under(Vec<Uuid>),
}

/// A unit a refusal may name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Near {
    pub id: Uuid,
    pub name: String,
    pub code: String,
}

/// True when the value is a unit of the hierarchy.
pub fn on_list<C: GenericClient>(c: &mut C, hierarchy: &str, value: &str) -> Result<bool, postgres::Error> {
    let Ok(id) = Uuid::parse_str(value.trim()) else { return Ok(false) };
    Ok(c.query_opt("SELECT 1 FROM organizations WHERE id = $1 AND hierarchy_key = $2", &[&id, &hierarchy])?.is_some())
}

/// The units of a hierarchy the caller may see, which the value is compared with.
fn candidates<C: GenericClient>(c: &mut C, hierarchy: &str, visible: &Visible) -> Result<Vec<Near>, postgres::Error> {
    let rows = match visible {
        Visible::All => c.query("SELECT id, name, code FROM organizations WHERE hierarchy_key = $1", &[&hierarchy])?,
        Visible::Under(roots) if roots.is_empty() => Vec::new(),
        Visible::Under(roots) => c.query(
            "SELECT DISTINCT o.id, o.name, o.code FROM org_closure z JOIN organizations o ON o.id = z.descendant WHERE z.hierarchy_key = $1 AND z.ancestor = ANY($2)",
            &[&hierarchy, roots],
        )?,
    };
    Ok(rows.iter().map(|r| Near { id: r.get(0), name: r.get(1), code: r.get(2) }).collect())
}

/// Up to [`MAX_NEAREST`] units of the hierarchy the caller may see, nearest to the value first. See [`rank`] for the rule.
pub fn nearest<C: GenericClient>(c: &mut C, hierarchy: &str, value: &str, visible: &Visible) -> Result<Vec<Near>, postgres::Error> {
    Ok(rank(value, candidates(c, hierarchy, visible)?))
}

/// The nearest units, by one rule. The distance of a unit is the smallest edit distance (Levenshtein, over characters,
/// ignoring case and the spaces around the value) between the value and the unit's name, the unit's code, and, when the
/// value is as long as an id, the unit's id. Units are ordered by distance, then name, then id, and the first
/// [`MAX_NEAREST`] are kept. There is no cut-off: a value that is nothing like any unit still gets the three nearest the
/// caller may see.
pub fn rank(value: &str, mut units: Vec<Near>) -> Vec<Near> {
    let want: Vec<char> = value.trim().to_lowercase().chars().take(128).collect();
    let by_id = want.len() == 36;
    let mut scored: Vec<(usize, Near)> = units
        .drain(..)
        .map(|u| {
            let mut d = distance(&want, &u.name.to_lowercase()).min(distance(&want, &u.code.to_lowercase()));
            if by_id {
                d = d.min(distance(&want, &u.id.to_string()));
            }
            (d, u)
        })
        .collect();
    scored.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.name.cmp(&b.1.name)).then_with(|| a.1.id.cmp(&b.1.id)));
    scored.into_iter().take(MAX_NEAREST).map(|(_, u)| u).collect()
}

/// Levenshtein distance between `a` and `b`, over characters.
fn distance(a: &[char], b: &str) -> usize {
    let b: Vec<char> = b.chars().take(128).collect();
    let mut row: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.iter().enumerate() {
        let mut diagonal = row[0];
        row[0] = i + 1;
        for (j, cb) in b.iter().enumerate() {
            let up = row[j + 1];
            row[j + 1] = (diagonal + usize::from(ca != cb)).min(up + 1).min(row[j] + 1);
            diagonal = up;
        }
    }
    row[b.len()]
}

/// What a writer is told when a value is not a unit on the list, with the nearest units the writer may see. It is the
/// same words on every door.
pub fn sentence(value: &str, hierarchy: &str, near: &[Near]) -> String {
    let shown: String = value.chars().take(100).collect();
    if near.is_empty() {
        return format!("'{shown}' is not a unit of the '{hierarchy}' hierarchy. No unit you can see is close to it. Ask the administrator of your unit for the unit to use.");
    }
    let list: Vec<String> = near.iter().map(|u| format!("{} (code {}, id {})", u.name, u.code, u.id)).collect();
    format!("'{shown}' is not a unit of the '{hierarchy}' hierarchy. The nearest units you can see are {}. Send the id of the unit.", list.join("; "))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unit(name: &str, code: &str) -> Near {
        Near { id: Uuid::new_v4(), name: name.to_string(), code: code.to_string() }
    }

    #[test]
    fn the_nearest_by_name_or_code_come_first_and_at_most_three_are_kept() {
        let all = vec![unit("Physics", "PHYS"), unit("Chemistry", "CHEM"), unit("Physiology", "PHYSIO"), unit("Physical Plant", "PLANT"), unit("Biology", "BIO")];
        let near = rank("  phys ", all.clone());
        assert_eq!(near.len(), 3);
        assert_eq!((near[0].name.as_str(), near[1].name.as_str()), ("Physics", "Physiology"));
        assert_eq!(rank("chemistri", all)[0].name, "Chemistry");
    }

    #[test]
    fn ties_break_by_name_and_an_id_sized_value_is_compared_with_ids() {
        let a = unit("Alpha", "AAA");
        let b = unit("Beta", "BBB");
        let near = rank("zzz", vec![b.clone(), a.clone()]);
        assert_eq!((near[0].name.as_str(), near[1].name.as_str()), ("Alpha", "Beta"));
        let mut typo = b.id.to_string();
        typo.replace_range(0..1, if typo.starts_with('0') { "1" } else { "0" });
        assert_eq!(rank(&typo, vec![a, b.clone()])[0].id, b.id);
    }

    #[test]
    fn the_sentence_names_the_units_or_says_there_are_none() {
        let u = unit("Physics", "PHYS");
        let s = sentence("physic", "academic", std::slice::from_ref(&u));
        assert!(s.contains("'physic' is not a unit of the 'academic' hierarchy") && s.contains(&u.id.to_string()), "{s}");
        let none = sentence("physic", "academic", &[]);
        assert!(none.contains("No unit you can see") && !none.contains("Physics"), "{none}");
    }
}
