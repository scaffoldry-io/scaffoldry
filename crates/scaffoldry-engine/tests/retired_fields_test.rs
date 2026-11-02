//! Lifecycle phase 2: a retired field is kept out of the formula graph. A formula that reads one does not validate.

use scaffoldry_engine::{validate_manifest, AppManifest};
use serde_json::json;

fn manifest(formula: &str) -> AppManifest {
    serde_json::from_value(json!({
        "slug": "retired-demo", "title": "Retired", "description": "", "organization_code": "PHYS", "department": "physics", "views": [],
        "tables": [{
            "id": "cases", "name": "Cases", "slug": "cases",
            "fields": [
                { "name": "title", "label": "Title", "field_type": "Text", "required": false, "ferpa_sensitive": false },
                { "name": "shout", "label": "Shout", "field_type": "Formula", "required": false, "ferpa_sensitive": false, "formula_expression": formula }
            ],
            "retired_fields": [
                { "name": "note", "label": "Note", "field_type": "Text", "required": false, "ferpa_sensitive": false, "retired_at": "2026-11-01T00:00:00Z" }
            ]
        }]
    }))
    .expect("a manifest")
}

#[test]
fn a_formula_over_a_retired_field_fails_validation_and_says_so() {
    let problems = validate_manifest(&manifest("UPPER({note})")).expect_err("a retired field is not in the formula graph");
    assert!(problems.iter().any(|p| p.contains("retired field 'note'")), "{problems:?}");
    assert!(validate_manifest(&manifest("UPPER({title})")).is_ok());
}

#[test]
fn a_field_cannot_be_live_and_retired_at_once() {
    let mut m = manifest("UPPER({title})");
    let note = m.tables[0].retired_fields[0].clone();
    m.tables[0].fields.push(note);
    let problems = validate_manifest(&m).expect_err("one name, two states");
    assert!(problems.iter().any(|p| p.contains("both live and retired")), "{problems:?}");
}

#[test]
fn retired_fields_round_trip_and_an_app_without_them_serializes_as_before() {
    let m = manifest("UPPER({title})");
    let v = serde_json::to_value(&m).unwrap();
    assert_eq!(v["tables"][0]["retired_fields"][0]["retired_at"], "2026-11-01T00:00:00Z");
    let mut plain = m.clone();
    plain.tables[0].retired_fields.clear();
    assert!(serde_json::to_value(&plain).unwrap()["tables"][0].get("retired_fields").is_none());
}
