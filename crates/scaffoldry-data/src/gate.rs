//! The label requirement of a row, for one reader (data layer phase 3).
//!
//! A row's effective label is the largest of its dataset's floor, its own tags, and the categories of its fields (the engine's
//! `effective_label`). A category may carry a requirement: a position and a reach. A reader meets a requirement for a row when
//! they hold the position at a unit that covers the row's unit. The row's unit is its unit column, or the dataset's own unit when
//! the column is empty. A row with no unit at all is covered by nobody, unless the reach is "anywhere", which needs no unit.
//!
//! The server builds one [`Gate`] for a reader and a table. It holds only the categories the reader does not meet everywhere, so
//! a reader who meets every requirement has no gate, and a table whose categories carry no requirement costs nothing.
//!
//! A gate has two forms that must agree: [`Gate::admits`] reads one row's JSON in Rust, and [`Gate::sql`] reads the row's columns
//! (`unit_id` and `tags`) in a statement, so a list never returns a row the person may not read. A test runs both over the
//! same rows.

use serde_json::Value;
use uuid::Uuid;

/// The units a reader's holdings of the required position cover: `exact` covers a row whose unit is that unit, and `below`
/// covers a row whose unit is that unit or under it. A cover with neither covers nothing.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Cover {
    pub exact: Vec<Uuid>,
    pub below: Vec<Uuid>,
    /// Whether the cover reaches the dataset's own unit, which a row with no unit takes. The server fills it when it builds the
    /// gate, so a statement carries one fact for the whole table and not a test for each row.
    pub at_dataset_unit: bool,
}

impl Cover {
    /// True when `unit` is covered. `contains(ancestor, unit)` is true when `ancestor` is `unit` or above it in its hierarchy.
    pub fn covers(&self, unit: Option<Uuid>, contains: &dyn Fn(Uuid, Uuid) -> bool) -> bool {
        let Some(unit) = unit else { return false };
        self.exact.contains(&unit) || self.below.iter().any(|a| contains(*a, unit))
    }
}

/// What a reader must be covered for, on the rows of one table.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Gate {
    /// The data key that holds the row's unit, for a table that names a unit column.
    pub unit_field: Option<String>,
    /// The unit of the dataset, which a row with no unit of its own takes.
    pub dataset_unit: Option<Uuid>,
    /// The table requires its unit column, so a row with no unit does not exist, and is read by nobody if one does.
    pub unit_required: bool,
    /// Categories every row of the table carries (the floor, and the categories of the fields). Every cover must hold.
    pub always: Vec<Cover>,
    /// Categories a row carries when it is tagged. A row tagged with the category needs the cover.
    pub by_tag: Vec<(String, Cover)>,
}

/// The tags a row's JSON holds, read the way the database trigger reads them.
pub fn tags_of(data: &Value) -> Vec<String> {
    data.get("_tags")
        .and_then(Value::as_array)
        .map(|a| a.iter().map(|t| t.as_str().map_or_else(|| t.to_string(), str::to_string)).collect())
        .unwrap_or_default()
}

/// The unit a row's JSON names in `field`. `Ok(None)` is a blank value. A value that is not a unit id is an error: the write that
/// carries it is refused later, and until then it is covered by nobody.
fn unit_in(data: &Value, field: &str) -> Result<Option<Uuid>, ()> {
    match data.get(field) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) if s.trim().is_empty() => Ok(None),
        Some(Value::String(s)) => Uuid::parse_str(s.trim()).map(Some).map_err(|_| ()),
        Some(_) => Err(()),
    }
}

/// A category id as settings may hold it. Anything else is never put in a statement.
fn id_ok(id: &str) -> bool {
    !id.is_empty() && id.len() <= 64 && id.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' || b == b'-')
}

impl Gate {
    /// True when the gate asks nothing of the reader.
    pub fn is_open(&self) -> bool {
        self.always.is_empty() && self.by_tag.is_empty()
    }

    /// True when a row of this table can be read by nobody the gate stands for: a requirement every row carries that nothing covers.
    pub fn is_closed(&self) -> bool {
        self.always.iter().any(|c| *c == Cover::default())
    }

    /// The unit a row is read by: its own, or the dataset's. `None` when it has none, or names one that is not an id.
    pub fn unit_of(&self, data: &Value) -> Option<Uuid> {
        let own = match &self.unit_field {
            Some(f) => match unit_in(data, f) {
                Ok(u) => u,
                Err(()) => return None,
            },
            None => None,
        };
        if self.unit_required {
            return own;
        }
        own.or(self.dataset_unit)
    }

    /// Whether the reader may read this row, from its JSON.
    pub fn admits(&self, data: &Value, contains: &dyn Fn(Uuid, Uuid) -> bool) -> bool {
        if self.is_open() {
            return true;
        }
        let unit = self.unit_of(data);
        if !self.always.iter().all(|c| c.covers(unit, contains)) {
            return false;
        }
        if self.by_tag.is_empty() {
            return true;
        }
        let tags = tags_of(data);
        self.by_tag.iter().all(|(category, cover)| !tags.contains(category) || cover.covers(unit, contains))
    }

    /// The units the cover reads in a row's own column, as a condition on `lg.unit_id`: the exact units, and every unit under the
    /// units it reaches below. `None` for a cover of nobody.
    fn on_column(cover: &Cover) -> Option<String> {
        let list = |ids: &[Uuid]| format!("ARRAY[{}]::uuid[]", ids.iter().map(|i| format!("'{i}'")).collect::<Vec<_>>().join(", "));
        let mut parts = Vec::new();
        if !cover.exact.is_empty() {
            parts.push(format!("lg.unit_id = ANY({})", list(&cover.exact)));
        }
        if !cover.below.is_empty() {
            parts.push(format!("lg.unit_id IN (SELECT lz.descendant FROM org_closure lz WHERE lz.ancestor = ANY({}))", list(&cover.below)));
        }
        match parts.len() {
            0 => None,
            1 => Some(parts.remove(0)),
            _ => Some(format!("({})", parts.join(" OR "))),
        }
    }

    /// The ids of the rows the cover reads, as a subquery over `dataset_records`. A row with a unit is read when the cover reaches it. A
    /// row with none is read when the cover reaches the dataset's unit, and that part is one condition for the whole table, so it
    /// costs nothing when it is false.
    fn covered_ids(&self, cover: &Cover, app: &str, table: &str) -> String {
        let mine = format!("lg.app_slug = {app} AND lg.table_id = {table} AND lg.deleted_at IS NULL");
        let mut selects = Vec::new();
        if let Some(column) = Self::on_column(cover) {
            selects.push(format!("SELECT lg.id FROM dataset_records lg WHERE {mine} AND {column}"));
        }
        // A row that names no unit is the dataset's. A table that requires its unit has none, so nothing is read for them.
        if cover.at_dataset_unit && !self.unit_required && self.dataset_unit.is_some() {
            selects.push(format!("SELECT lg.id FROM dataset_records lg WHERE {mine} AND lg.unit_id IS NULL"));
        }
        if selects.is_empty() {
            return format!("SELECT lg.id FROM dataset_records lg WHERE {mine} AND FALSE");
        }
        selects.join(" UNION ALL ")
    }

    /// A condition for a row of `dataset_records` named by `id_expr`, true when the reader may read it, for a statement that names the
    /// record id as `id_expr`. `app` and `table` are the placeholders the caller bound to the app slug and the table id.
    ///
    /// A requirement every row carries is a subquery of the covered ids, so a list of a narrow reach reads the unit index and stops
    /// early, and a count reads the same set once. A requirement a tag adds excludes the tagged rows the reader does not cover, which
    /// are few, from a partial index. Measured on 1,000,000 rows in the data-layer notes. Every value is a uuid or a category id of the
    /// allowed shape, so none is built from text a person sent.
    pub fn sql(&self, id_expr: &str, app: &str, table: &str) -> String {
        if self.is_open() {
            return "TRUE".to_string();
        }
        // A table with no unit column places every row at the dataset's unit, so a requirement every row carries is one fact for
        // the whole table, not a condition on a row.
        let mut conditions: Vec<String> = self
            .always
            .iter()
            .map(|c| match &self.unit_field {
                None => (c.at_dataset_unit && self.dataset_unit.is_some()).to_string().to_uppercase(),
                Some(_) => format!("{id_expr} IN ({})", self.covered_ids(c, app, table)),
            })
            .collect();
        for (category, cover) in &self.by_tag {
            if !id_ok(category) {
                conditions.push("FALSE".to_string());
                continue;
            }
            // A row that is tagged and not covered is out. A row whose unit is blank is covered by the dataset's unit or by nobody.
            // A table with no unit column never reads the column.
            let on_column = if self.unit_field.is_some() { Self::on_column(cover).unwrap_or_else(|| "FALSE".to_string()) } else { "FALSE".to_string() };
            let on_dataset = (cover.at_dataset_unit && self.dataset_unit.is_some() && !self.unit_required).to_string().to_uppercase();
            conditions.push(format!(
                "NOT EXISTS (SELECT 1 FROM dataset_records lg WHERE lg.id = {id_expr} AND lg.tags <> '{{}}' AND lg.tags @> ARRAY['{category}']::text[] \
                 AND (CASE WHEN lg.unit_id IS NULL THEN {on_dataset} ELSE {on_column} END) IS NOT TRUE)"
            ));
        }
        conditions.join(" AND ")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn id(n: u128) -> Uuid {
        Uuid::from_u128(n)
    }

    /// 1 is the parent of 2 and 3. 4 is a root of its own.
    fn contains(a: Uuid, u: Uuid) -> bool {
        a == u || (a == id(1) && (u == id(2) || u == id(3)))
    }

    fn gate(always: Vec<Cover>, by_tag: Vec<(String, Cover)>) -> Gate {
        Gate { unit_field: Some("unit".to_string()), dataset_unit: Some(id(3)), unit_required: false, always, by_tag }
    }

    /// The cover, with the fact the server fills in: whether it reaches the dataset's unit (3).
    fn at_3(mut c: Cover) -> Cover {
        c.at_dataset_unit = c.exact.contains(&id(3)) || c.below.iter().any(|a| contains(*a, id(3)));
        c
    }

    fn exact(u: u128) -> Cover {
        at_3(Cover { exact: vec![id(u)], ..Default::default() })
    }

    fn below(u: u128) -> Cover {
        at_3(Cover { below: vec![id(u)], ..Default::default() })
    }

    fn row(unit: Option<u128>, tags: &[&str]) -> Value {
        let mut v = json!({ "_tags": tags });
        if let Some(u) = unit {
            v["unit"] = json!(id(u).to_string());
        }
        v
    }

    #[test]
    fn a_gate_with_nothing_in_it_admits_every_row_and_a_requirement_every_row_carries_is_checked_against_the_unit() {
        assert!(Gate::default().admits(&json!({}), &contains));
        let g = gate(vec![exact(2)], vec![]);
        assert!(g.admits(&row(Some(2), &[]), &contains));
        assert!(!g.admits(&row(Some(3), &[]), &contains), "a sibling unit is not covered");
        assert!(!g.admits(&row(Some(1), &[]), &contains), "a cover of a unit does not reach up");
        let g = gate(vec![below(1)], vec![]);
        assert!(g.admits(&row(Some(2), &[]), &contains) && g.admits(&row(Some(1), &[]), &contains));
        assert!(!g.admits(&row(Some(4), &[]), &contains));
    }

    #[test]
    fn a_row_with_no_unit_takes_the_datasets_unit_and_a_row_with_neither_is_covered_by_nobody() {
        let g = gate(vec![exact(3)], vec![]);
        assert!(g.admits(&row(None, &[]), &contains), "the dataset's unit is 3");
        assert!(!gate(vec![exact(2)], vec![]).admits(&row(None, &[]), &contains));
        let mut none = gate(vec![below(1)], vec![]);
        none.dataset_unit = None;
        assert!(!none.admits(&row(None, &[]), &contains), "no unit at all");
        // A value that is not a unit id is not read as blank. It takes no unit.
        assert!(!g.admits(&json!({ "unit": "not-a-unit" }), &contains));
        assert!(!g.admits(&json!({ "unit": 7 }), &contains));
        assert!(g.admits(&json!({ "unit": "   " }), &contains), "blank is blank");
    }

    #[test]
    fn a_tag_adds_a_requirement_only_to_the_rows_that_carry_it() {
        let g = gate(vec![], vec![("phi".to_string(), exact(2))]);
        assert!(g.admits(&row(Some(3), &[]), &contains));
        assert!(g.admits(&row(Some(3), &["other"]), &contains));
        assert!(!g.admits(&row(Some(3), &["phi"]), &contains));
        assert!(g.admits(&row(Some(2), &["phi", "other"]), &contains));
        assert!(!gate(vec![], vec![("phi".to_string(), Cover::default())]).admits(&row(Some(2), &["phi"]), &contains));
    }

    #[test]
    fn a_cover_with_nothing_in_it_closes_the_table() {
        assert!(gate(vec![Cover::default()], vec![]).is_closed());
        assert!(!gate(vec![exact(2)], vec![]).is_closed());
    }

    #[test]
    fn the_statement_names_only_ids_and_category_ids_of_the_allowed_shape() {
        let g = gate(vec![below(1)], vec![("phi".to_string(), exact(2)), ("Bad'; DROP".to_string(), exact(2))]);
        let sql = g.sql("r.id", "$3", "$4");
        assert!(sql.contains("r.id IN (SELECT lg.id FROM dataset_records lg WHERE lg.app_slug = $3 AND lg.table_id = $4"), "{sql}");
        assert!(sql.contains("lg.tags @> ARRAY['phi']::text[]") && !sql.contains("DROP"), "{sql}");
        assert!(sql.contains("org_closure") && sql.contains(&id(1).to_string()), "{sql}");
        assert_eq!(Gate::default().sql("r.id", "$3", "$4"), "TRUE");
    }

    #[test]
    fn a_cover_of_nobody_selects_no_row_and_the_datasets_unit_is_one_condition_for_the_table() {
        assert!(gate(vec![Cover::default()], vec![]).sql("id", "$1", "$2").ends_with("AND FALSE)"));
        let g = gate(vec![below(1)], vec![]);
        // The dataset's unit is 3, under 1: a row with no unit is read, by one condition that does not depend on the row.
        assert!(g.sql("id", "$1", "$2").contains("lg.unit_id IS NULL"), "{}", g.sql("id", "$1", "$2"));
        assert!(!gate(vec![exact(2)], vec![]).sql("id", "$1", "$2").contains("lg.unit_id IS NULL"), "a cover that misses the dataset's unit reads no blank row");
        let mut none = g.clone();
        none.dataset_unit = None;
        assert!(!none.sql("id", "$1", "$2").contains("lg.unit_id IS NULL"), "no dataset unit: a row with no unit is read by nobody");
        // A tag the cover does not reach is excluded, and a null verdict is excluded too.
        let tagged = gate(vec![], vec![("phi".to_string(), Cover::default())]);
        assert!(tagged.sql("id", "$1", "$2").contains("IS NOT TRUE"));
    }

    #[test]
    fn a_table_that_requires_its_unit_reads_no_blank_row_and_one_with_no_column_asks_one_question_of_the_whole_table() {
        let mut required = gate(vec![below(1)], vec![]);
        required.unit_required = true;
        assert!(!required.admits(&row(None, &[]), &contains), "a blank unit is read by nobody here");
        assert!(required.admits(&row(Some(2), &[]), &contains));
        assert!(!required.sql("id", "$1", "$2").contains("lg.unit_id IS NULL"));
        let mut columnless = gate(vec![below(1)], vec![]);
        columnless.unit_field = None;
        // The dataset's unit is 3, under 1, so every row is read. A row's own unit key means nothing without a column.
        assert!(columnless.admits(&row(Some(4), &[]), &contains));
        assert!(!columnless.sql("id", "$1", "$2").contains("dataset_records lg WHERE lg.app_slug"), "no subquery: {}", columnless.sql("id", "$1", "$2"));
        columnless.always = vec![exact(2)];
        assert!(!columnless.admits(&row(Some(2), &[]), &contains));
        assert_eq!(columnless.sql("id", "$1", "$2"), "FALSE");
        columnless.always = vec![below(1)];
        assert_eq!(columnless.sql("id", "$1", "$2"), "TRUE");
    }

    #[test]
    fn tags_are_read_the_way_the_trigger_reads_them() {
        assert_eq!(tags_of(&json!({ "_tags": ["a", "b"] })), ["a", "b"]);
        assert_eq!(tags_of(&json!({ "_tags": [1, true] })), ["1", "true"]);
        assert!(tags_of(&json!({ "_tags": "a" })).is_empty() && tags_of(&json!({})).is_empty());
    }
}
