//! Row scale phase 2: which fields a table indexes, and what value each one stores.

use scaffoldry_engine::{
    index_rows, indexed_fields, validate_manifest, AppManifest, AppTable, AppView, CompoundFilter, FieldSpec, FieldType,
    FilterClause, FilterConjunction, FilterOperator, SortDirection, SortRule, TableRelationship, ViewType,
};
use serde_json::json;

fn field(name: &str, t: FieldType) -> FieldSpec {
    FieldSpec::simple(name, name, t, false, false)
}

fn manifest(fields: Vec<FieldSpec>) -> AppManifest {
    AppManifest {
        slug: "indexing".into(),
        title: "Indexing".into(),
        description: String::new(),
        organization_code: "X".into(),
        department: "x".into(),
        workspace_id: None,
        herm_capability_id: None,
        custom_domain: None,
        custom_domain_verified: false,
        tables: vec![AppTable {
            id: "t1".into(),
            name: "T".into(),
            slug: "t".into(),
            description: None,
            icon: None,
            fields,
            primary_field: None,
            sample_records: vec![],
        }],
        relationships: vec![],
        views: vec![],
        ceds_mappings: Default::default(),
        pages: vec![],
    }
}

fn view_on(table: &str, filter_field: Option<&str>, sort_field: Option<&str>) -> AppView {
    let mut v = AppView::table("v", "V", ViewType::Table, vec![]);
    v.table_id = Some(table.to_string());
    if let Some(f) = filter_field {
        v.filters = Some(CompoundFilter {
            conjunction: FilterConjunction::And,
            clauses: vec![FilterClause { id: "c".into(), field_name: f.into(), operator: FilterOperator::Equals, value: "x".into() }],
        });
    }
    if let Some(s) = sort_field {
        v.sort_rules = vec![SortRule { id: "s".into(), field_name: s.into(), direction: SortDirection::Asc }];
    }
    v
}

#[test]
fn a_field_is_indexed_when_marked_when_a_view_uses_it_or_when_a_relationship_joins_on_it() {
    let mut marked = field("marked", FieldType::Number);
    marked.indexed = true;
    let mut m = manifest(vec![marked, field("sorted", FieldType::Number), field("filtered", FieldType::Text), field("joined", FieldType::Text), field("plain", FieldType::Text)]);
    m.views = vec![view_on("t1", Some("filtered"), Some("sorted")), view_on("other", Some("plain"), None)];
    m.relationships = vec![TableRelationship {
        id: "r".into(),
        name: "r".into(),
        source_table_id: "t1".into(),
        target_table_id: "other".into(),
        source_field: "joined".into(),
        target_field: "x".into(),
        relationship_type: "one_to_many".into(),
        display_field: "x".into(),
    }];
    let set: Vec<String> = indexed_fields(&m, "t1").into_iter().collect();
    assert_eq!(set, vec!["filtered", "joined", "marked", "sorted"], "a view of another table does not count");
}

#[test]
fn a_table_with_21_indexed_fields_fails_validation_and_so_does_indexing_a_lookup() {
    let many: Vec<FieldSpec> = (0..21)
        .map(|i| {
            let mut f = field(&format!("f{i}"), FieldType::Number);
            f.indexed = true;
            f
        })
        .collect();
    let problems = validate_manifest(&manifest(many)).unwrap_err();
    assert!(problems.iter().any(|p| p.contains("indexes 21 fields")), "{problems:?}");

    let mut twenty: Vec<FieldSpec> = (0..20)
        .map(|i| {
            let mut f = field(&format!("f{i}"), FieldType::Number);
            f.indexed = true;
            f
        })
        .collect();
    assert!(validate_manifest(&manifest(twenty.clone())).is_ok(), "twenty is the limit");

    twenty.truncate(1);
    let mut lookup = field("looked", FieldType::Lookup);
    lookup.indexed = true;
    lookup.target_table_id = Some("t1".into());
    lookup.target_display_field = Some("f0".into());
    twenty.push(lookup);
    let problems = validate_manifest(&manifest(twenty)).unwrap_err();
    assert!(problems.iter().any(|p| p.contains("looked") && p.contains("Lookup")), "{problems:?}");
}

#[test]
fn each_field_type_stores_its_value_in_the_right_column() {
    let one = |t: FieldType, v: serde_json::Value| index_rows(&field("f", t), &v);
    // (num, txt, ts)
    assert_eq!(one(FieldType::Number, json!(12.5)), vec![(Some(12.5), None, None)]);
    assert_eq!(one(FieldType::Currency, json!("7")), vec![(Some(7.0), None, None)], "a number written as text");
    assert_eq!(one(FieldType::Boolean, json!(true)), vec![(Some(1.0), None, None)]);
    assert_eq!(one(FieldType::Checkbox, json!(false)), vec![(Some(0.0), None, None)]);
    assert_eq!(one(FieldType::Text, json!("Hello World")), vec![(None, Some("hello world".to_string()), None)], "lowercase");
    let long = "A".repeat(300);
    assert_eq!(one(FieldType::Text, json!(long))[0].1.as_ref().unwrap().len(), 128, "first 128 characters");
    assert_eq!(one(FieldType::Date, json!("2026-03-04")), vec![(None, None, Some("2026-03-04T00:00:00+00:00".to_string()))]);
    assert_eq!(one(FieldType::Date, json!("2026-03-04T10:00:00Z")), vec![(None, None, Some("2026-03-04T10:00:00+00:00".to_string()))]);
    // A formula stores a number as a number and anything else as text.
    assert_eq!(one(FieldType::Formula, json!(3)), vec![(Some(3.0), None, None)]);
    assert_eq!(one(FieldType::Formula, json!("Yes")), vec![(None, Some("yes".to_string()), None)]);
}

#[test]
fn a_blank_value_still_gets_one_row_and_a_multi_valued_field_gets_one_per_value() {
    let blank = (None, None, None);
    for v in [json!(null), json!(""), json!([])] {
        assert_eq!(index_rows(&field("f", FieldType::Number), &v), vec![blank.clone()], "{v}");
    }
    assert_eq!(index_rows(&field("f", FieldType::Text), &json!("   ")), vec![(None, Some("   ".to_string()), None)], "text is text");
    assert_eq!(index_rows(&field("f", FieldType::Number), &json!("not a number")), vec![blank.clone()], "a value that does not fit is blank");

    let tags = index_rows(&field("tags", FieldType::MultiSelect), &json!(["Red", "Blue"]));
    assert_eq!(tags, vec![(None, Some("red".to_string()), None), (None, Some("blue".to_string()), None)]);
}
