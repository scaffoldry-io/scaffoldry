//! Calculation graph phase 3: what a changed definition recalculates, and how a value converts.

use scaffoldry_engine::recalc::{changed_calculations, convert_value, conversion_supported, type_changes, Converted};
use scaffoldry_engine::{AppManifest, AppTable, FieldSpec, FieldType, LinkCardinality};
use serde_json::json;

fn num(name: &str) -> FieldSpec {
    FieldSpec::simple(name, name, FieldType::Number, false, false)
}

fn text(name: &str) -> FieldSpec {
    FieldSpec::simple(name, name, FieldType::Text, false, false)
}

fn formula(name: &str, expression: &str) -> FieldSpec {
    let mut f = FieldSpec::simple(name, name, FieldType::Formula, false, false);
    f.formula_expression = Some(expression.to_string());
    f
}

fn link(name: &str, target: &str) -> FieldSpec {
    let mut f = FieldSpec::simple(name, name, FieldType::Link, false, false);
    f.link_target_table = Some(target.to_string());
    f.link_cardinality = LinkCardinality::One;
    f
}

fn computed(name: &str, t: FieldType, via: &str, target: Option<&str>) -> FieldSpec {
    let mut f = FieldSpec::simple(name, name, t, false, false);
    f.computed_via = Some(via.to_string());
    f.target_display_field = target.map(String::from);
    f.rollup_function = Some("sum".to_string());
    f
}

fn table(id: &str, fields: Vec<FieldSpec>) -> AppTable {
    AppTable { id: id.into(), name: id.into(), slug: id.into(), description: None, icon: None, fields, primary_field: None, sample_records: vec![] }
}

fn manifest(tables: Vec<AppTable>) -> AppManifest {
    AppManifest {
        slug: "calc".into(),
        title: "Calc".into(),
        description: String::new(),
        organization_code: "BIO".into(),
        department: "biology".into(),
        workspace_id: None,
        herm_capability_id: None,
        custom_domain: None,
        custom_domain_verified: false,
        tables,
        relationships: vec![],
        views: vec![],
        ceds_mappings: Default::default(),
        pages: vec![],
    }
}

fn app(expression: &str, rollup_via: &str) -> AppManifest {
    manifest(vec![
        table("parents", vec![num("budget"), formula("remaining", expression), formula("status", "{remaining} + 1"), link("kids", "kids"), computed("total", FieldType::Rollup, rollup_via, Some("amount")), text("note")]),
        table("kids", vec![num("amount"), link("parent", "parents"), computed("parent_budget", FieldType::Lookup, "parent", Some("budget"))]),
    ])
}

// 1. A changed formula recalculates itself and everything that reads it, in run order. Nothing else.
#[test]
fn a_changed_formula_recalculates_it_and_what_reads_it() {
    let old = app("{budget} - 1", "kids");
    let new = app("{budget} - 2", "kids");
    let changes = changed_calculations(Some(&old), &new);
    assert_eq!(changes.len(), 1, "{changes:?}");
    assert_eq!(changes[0].table_id, "parents");
    assert_eq!(changes[0].fields, vec!["remaining", "status"], "remaining, then status which reads it; total is untouched");
}

// 2. An unchanged app recalculates nothing. A new app has no records, so nothing either.
#[test]
fn nothing_changed_means_nothing_to_recalculate() {
    let same = app("{budget} - 1", "kids");
    assert!(changed_calculations(Some(&same), &same).is_empty());
    assert!(changed_calculations(None, &same).is_empty());
}

// 3. A new computed field, or a changed rollup, is recalculated. A lookup never is: it is read, not stored.
#[test]
fn a_new_field_and_a_changed_rollup_are_recalculated_but_a_lookup_is_not() {
    let old = app("{budget} - 1", "kids");
    let mut new = app("{budget} - 1", "kids");
    new.tables[0].fields.push(formula("doubled", "{budget} * 2"));
    new.tables[0].fields.iter_mut().find(|f| f.name == "total").unwrap().rollup_function = Some("max".into());
    new.tables[1].fields.iter_mut().find(|f| f.name == "parent_budget").unwrap().target_display_field = Some("budget".into());
    let changes = changed_calculations(Some(&old), &new);
    let parents = changes.iter().find(|c| c.table_id == "parents").unwrap();
    assert!(parents.fields.contains(&"doubled".to_string()) && parents.fields.contains(&"total".to_string()), "{parents:?}");
    assert!(changes.iter().all(|c| c.table_id != "kids"), "a lookup is not stored: {changes:?}");
}

// 4. A change in one table reaches a rollup in another that reads it.
#[test]
fn a_change_reaches_a_rollup_in_another_table() {
    let mut old = app("{budget} - 1", "kids");
    old.tables[1].fields.push(formula("score", "{amount} + 1"));
    let mut new = old.clone();
    new.tables[1].fields.iter_mut().find(|f| f.name == "score").unwrap().formula_expression = Some("{amount} + 2".into());
    new.tables[0].fields.iter_mut().find(|f| f.name == "total").unwrap().target_display_field = Some("score".into());
    old.tables[0].fields.iter_mut().find(|f| f.name == "total").unwrap().target_display_field = Some("score".into());
    let changes = changed_calculations(Some(&old), &new);
    assert!(changes.iter().any(|c| c.table_id == "kids" && c.fields == vec!["score"]), "{changes:?}");
    assert!(changes.iter().any(|c| c.table_id == "parents" && c.fields.contains(&"total".to_string())), "{changes:?}");
}

// 5. A type change is found, and only where both ends are plain stored fields.
#[test]
fn type_changes_are_found() {
    let old = manifest(vec![table("t", vec![text("n"), num("m"), formula("f", "{m}")])]);
    let new = manifest(vec![table("t", vec![num("n"), num("m"), formula("f", "{m}")])]);
    let found = type_changes(&old, &new);
    assert_eq!(found.len(), 1);
    assert_eq!((found[0].field.as_str(), found[0].from, found[0].to), ("n", FieldType::Text, FieldType::Number));
}

// 6. Conversion: text to number blanks what is not a number. Blanks stay blank. A number stays.
#[test]
fn text_converts_to_number_and_what_is_not_a_number_becomes_blank() {
    let t = FieldType::Text;
    let n = FieldType::Number;
    assert_eq!(convert_value(&json!("12"), t, n), Converted::To(json!(12)));
    assert_eq!(convert_value(&json!(" 3.5 "), t, n), Converted::To(json!(3.5)));
    assert_eq!(convert_value(&json!("twelve"), t, n), Converted::Blank);
    assert_eq!(convert_value(&json!(""), t, n), Converted::Keep);
    assert_eq!(convert_value(&serde_json::Value::Null, t, n), Converted::Keep);
    assert_eq!(convert_value(&json!(7), t, n), Converted::Keep, "a value already of the type is left alone");
    assert_eq!(convert_value(&json!(5), n, t), Converted::To(json!("5")));
    assert_eq!(convert_value(&json!(5.25), n, t), Converted::To(json!("5.25")));
    assert_eq!(convert_value(&json!("a@b.edu"), t, FieldType::Email), Converted::Keep);
    assert_eq!(convert_value(&json!("2026-01-05"), t, FieldType::Date), Converted::Keep);
    assert_eq!(convert_value(&json!("soon"), t, FieldType::Date), Converted::Blank);
    assert!(conversion_supported(t, n) && conversion_supported(n, t));
    assert!(!conversion_supported(FieldType::Link, n) && !conversion_supported(t, FieldType::Formula));
}
