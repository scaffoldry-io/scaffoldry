//! Links phase 3: lookup, rollup, and count go through a link field, by name.

use scaffoldry_engine::{
    indexed_fields, manifest_warnings, validate_manifest, AppManifest, AppTable, FieldSpec, FieldType, LinkCardinality, TableRelationship,
};

fn field(name: &str, t: FieldType) -> FieldSpec {
    FieldSpec::simple(name, name, t, false, false)
}

fn link(name: &str, target: &str, related: &str) -> FieldSpec {
    let mut f = field(name, FieldType::Link);
    f.link_target_table = Some(target.to_string());
    f.link_related_field = Some(related.to_string());
    f.link_cardinality = LinkCardinality::Many;
    f
}

fn computed(name: &str, t: FieldType, via: Option<&str>, target: Option<&str>, function: Option<&str>) -> FieldSpec {
    let mut f = field(name, t);
    f.computed_via = via.map(String::from);
    f.target_display_field = target.map(String::from);
    f.rollup_function = function.map(String::from);
    f
}

fn table(id: &str, fields: Vec<FieldSpec>) -> AppTable {
    AppTable {
        id: id.into(),
        name: id.into(),
        slug: id.into(),
        description: None,
        icon: None,
        fields,
        primary_field: None,
        sample_records: vec![],
    }
}

/// `parents.children` (Many) mirrors `children.parent` (Many here, to keep the fixture small).
fn manifest(parent_extra: Vec<FieldSpec>, child_extra: Vec<FieldSpec>) -> AppManifest {
    let mut parent_fields = vec![field("name", FieldType::Text), link("children", "children", "parent")];
    parent_fields.extend(parent_extra);
    let mut child_fields = vec![field("title", FieldType::Text), field("amount", FieldType::Number), link("parent", "parents", "children")];
    child_fields.extend(child_extra);
    AppManifest {
        slug: "computing".into(),
        title: "Computing".into(),
        description: String::new(),
        organization_code: "X".into(),
        department: "x".into(),
        workspace_id: None,
        herm_capability_id: None,
        custom_domain: None,
        custom_domain_verified: false,
        tables: vec![table("parents", parent_fields), table("children", child_fields)],
        relationships: vec![],
        views: vec![],
        ceds_mappings: Default::default(),
        pages: vec![],
        access: Default::default(),
    }
}

fn problems(m: &AppManifest) -> Vec<String> {
    validate_manifest(m).err().unwrap_or_default()
}

#[test]
fn a_rollup_count_and_lookup_through_a_link_field_are_valid() {
    let m = manifest(
        vec![
            computed("total", FieldType::Rollup, Some("children"), Some("amount"), Some("sum")),
            computed("n", FieldType::Count, Some("children"), None, None),
        ],
        vec![computed("parent_name", FieldType::Lookup, Some("parent"), Some("name"), None)],
    );
    assert!(validate_manifest(&m).is_ok(), "{:?}", problems(&m));
}

#[test]
fn the_link_field_is_required_and_must_be_a_link_on_the_same_table() {
    let none = manifest(vec![computed("total", FieldType::Rollup, None, Some("amount"), Some("sum"))], vec![]);
    assert!(problems(&none).iter().any(|p| p.contains("'total'") && p.contains("computed_via")), "{:?}", problems(&none));

    let missing = manifest(vec![computed("total", FieldType::Rollup, Some("nope"), Some("amount"), Some("sum"))], vec![]);
    assert!(problems(&missing).iter().any(|p| p.contains("'total'") && p.contains("'nope'")), "{:?}", problems(&missing));

    let not_a_link = manifest(vec![computed("total", FieldType::Rollup, Some("name"), Some("amount"), Some("sum"))], vec![]);
    assert!(problems(&not_a_link).iter().any(|p| p.contains("'total'") && p.contains("not a Link field")), "{:?}", problems(&not_a_link));
}

#[test]
fn the_target_field_must_exist_and_a_numeric_rollup_needs_a_numeric_target() {
    let ghost = manifest(vec![computed("total", FieldType::Rollup, Some("children"), Some("ghost"), Some("sum"))], vec![]);
    assert!(problems(&ghost).iter().any(|p| p.contains("'ghost'")), "{:?}", problems(&ghost));

    let text = manifest(vec![computed("total", FieldType::Rollup, Some("children"), Some("title"), Some("sum"))], vec![]);
    assert!(problems(&text).iter().any(|p| p.contains("'total'") && p.contains("number")), "{:?}", problems(&text));

    // count_distinct takes any type. An unknown function is refused.
    let distinct = manifest(vec![computed("kinds", FieldType::Rollup, Some("children"), Some("title"), Some("count_distinct"))], vec![]);
    assert!(validate_manifest(&distinct).is_ok(), "{:?}", problems(&distinct));
    let odd = manifest(vec![computed("total", FieldType::Rollup, Some("children"), Some("amount"), Some("median"))], vec![]);
    assert!(problems(&odd).iter().any(|p| p.contains("'median'")), "{:?}", problems(&odd));

    // A lookup needs a target field too.
    let bare = manifest(vec![], vec![computed("parent_name", FieldType::Lookup, Some("parent"), None, None)]);
    assert!(problems(&bare).iter().any(|p| p.contains("'parent_name'") && p.contains("target field")), "{:?}", problems(&bare));
}

#[test]
fn a_rollups_target_field_joins_the_indexed_set() {
    let m = manifest(vec![computed("total", FieldType::Rollup, Some("children"), Some("amount"), Some("sum"))], vec![]);
    let set: Vec<String> = indexed_fields(&m, "children").into_iter().collect();
    assert!(set.contains(&"amount".to_string()), "{set:?}");
    assert!(!indexed_fields(&m, "parents").contains("amount"));
}

#[test]
fn a_relationship_is_deprecated_and_the_validator_warns() {
    let mut m = manifest(vec![], vec![]);
    assert!(manifest_warnings(&m).is_empty());
    m.relationships.push(TableRelationship {
        id: "r".into(),
        name: "r".into(),
        source_table_id: "children".into(),
        target_table_id: "parents".into(),
        source_field: "title".into(),
        target_field: "name".into(),
        relationship_type: "one_to_many".into(),
        display_field: "name".into(),
    });
    let warnings = manifest_warnings(&m);
    assert!(warnings.iter().any(|w| w.contains("'r'") && w.contains("Link")), "{warnings:?}");
    assert!(validate_manifest(&m).is_ok(), "a relationship still validates: {:?}", problems(&m));
}
