//! Data layer phase 3: a row's effective label, and a category's requirement.
//!
//! The label of a row is the largest of the dataset floor, the row's tags and the categories of its fields. Categories are a set, so
//! the largest is the union. Nothing lowers: a floor, a tag or a field only adds.

use scaffoldry_engine::sensitivity::{validate_categories, Category, Reach, Requirement, Sensitivity};
use scaffoldry_engine::effective_label;
use serde_json::json;
use std::collections::BTreeSet;

fn category(id: &str, protected: bool) -> Category {
    Category { id: id.to_string(), name: id.to_string(), source: String::new(), protected, detectors: vec![], default_mask: Default::default(), requires: vec![], conditions: vec![] }
}

fn field(ids: &[&str], protected: bool) -> Sensitivity {
    Sensitivity { protected, categories: ids.iter().map(|s| s.to_string()).collect() }
}

fn names(s: &Sensitivity) -> Vec<&str> {
    s.categories.iter().map(String::as_str).collect()
}

fn v(items: &[&str]) -> Vec<String> {
    items.iter().map(|s| s.to_string()).collect()
}

#[test]
fn the_label_is_the_union_of_the_floor_the_tags_and_the_fields() {
    let cats = [category("phi", true), category("hr", false), category("open", false)];
    let label = effective_label(&v(&["hr"]), &v(&["phi"]), &[field(&["open"], false)], &cats);
    assert_eq!(names(&label), ["hr", "open", "phi"]);
    assert!(label.protected, "phi is protected in settings");
}

#[test]
fn a_row_with_nothing_has_an_empty_label_that_is_not_protected() {
    let label = effective_label(&[], &[], &[], &[category("phi", true)]);
    assert!(label.categories.is_empty() && !label.protected);
}

#[test]
fn a_floor_raises_a_row_with_no_tag_and_no_field() {
    let cats = [category("phi", true)];
    assert!(effective_label(&v(&["phi"]), &[], &[], &cats).protected);
    assert!(effective_label(&[], &v(&["phi"]), &[], &cats).protected);
}

#[test]
fn a_tag_or_a_floor_below_a_fields_label_cannot_lower_it() {
    let cats = [category("phi", true), category("open", false)];
    let label = effective_label(&v(&["open"]), &v(&["open"]), &[field(&["phi"], true)], &cats);
    assert!(label.protected && label.categories.contains("phi"));
}

#[test]
fn a_category_settings_do_not_define_protects_nothing_but_is_carried() {
    let label = effective_label(&[], &v(&["gone"]), &[], &[category("phi", true)]);
    assert!(!label.protected);
    assert_eq!(label.categories, BTreeSet::from(["gone".to_string()]));
}

#[test]
fn adding_to_any_input_never_removes_a_category_or_a_protection() {
    let cats = [category("phi", true), category("hr", false)];
    let base = effective_label(&v(&["phi"]), &v(&["hr"]), &[], &cats);
    for extra in [("floor", "hr"), ("tag", "phi"), ("tag", "zzz")] {
        let (mut floor, mut tags) = (v(&["phi"]), v(&["hr"]));
        match extra.0 {
            "floor" => floor.push(extra.1.to_string()),
            _ => tags.push(extra.1.to_string()),
        }
        let more = effective_label(&floor, &tags, &[], &cats);
        assert!(base.categories.is_subset(&more.categories) && (!base.protected || more.protected));
    }
}

#[test]
fn a_requirement_is_a_position_and_a_reach_and_a_list_with_none_means_none() {
    let ok = json!([
        { "id": "phi", "name": "PHI", "protected": true, "detectors": [], "requires": [{ "position": "registrar", "reach": "unit_and_below" }, { "position": "dean", "reach": "anywhere" }] },
        { "id": "open", "name": "Open", "protected": false, "detectors": [] }
    ]);
    let cats = validate_categories(&ok, &[]).unwrap();
    assert_eq!(cats[0].requires, vec![Requirement { position: "registrar".into(), reach: Reach::UnitAndBelow }, Requirement { position: "dean".into(), reach: Reach::Anywhere }]);
    assert!(cats[1].requires.is_empty());
    // It round-trips, and a category with no requirement writes none.
    let back = serde_json::to_value(&cats).unwrap();
    assert!(back[1].get("requires").is_none());
    assert_eq!(validate_categories(&back, &[]).unwrap(), cats);
}

#[test]
fn a_bad_requirement_is_refused_with_its_path() {
    let with = |requires: serde_json::Value| json!([{ "id": "phi", "name": "PHI", "protected": true, "detectors": [], "requires": requires }]);
    let e = validate_categories(&with(json!([{ "position": "registrar", "reach": "everywhere" }])), &[]).unwrap_err();
    assert_eq!(e.path, "[0]", "{e:?}");
    let e = validate_categories(&with(json!([{ "position": "Has Space", "reach": "unit" }])), &[]).unwrap_err();
    assert_eq!(e.path, "[0].requires[0].position");
    let e = validate_categories(&with(json!([{ "position": "a", "reach": "unit" }, { "position": "a", "reach": "unit" }])), &[]).unwrap_err();
    assert_eq!(e.path, "[0].requires[1]");
    let nine: Vec<_> = (0..9).map(|i| json!({ "position": format!("p{i}"), "reach": "unit" })).collect();
    assert_eq!(validate_categories(&with(json!(nine)), &[]).unwrap_err().path, "[0].requires");
    let e = validate_categories(&with(json!([{ "position": "a", "reach": "unit", "extra": 1 }])), &[]).unwrap_err();
    assert_eq!(e.path, "[0]");
}
