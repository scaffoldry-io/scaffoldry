//! Rich Field Types and Computed Field Engine Integration Test

use scaffoldry_engine::{compute_field_value, evaluate_formula, FieldSpec, FieldType};
use serde_json::json;

#[test]
fn test_formula_evaluation() {
    let original_record = json!({
        "first_name": "Marie",
        "last_name": "Curie",
        "budget": 500000.0,
        "overhead_rate": 0.20,
        "spent": 120000.0
    });

    // Four original assertions
    assert_eq!(evaluate_formula("{budget}", &original_record), json!(500000.0));
    assert_eq!(evaluate_formula("{budget} * 0.15", &original_record), json!(75000.0));
    assert_eq!(evaluate_formula("{first_name} + \" \" + {last_name}", &original_record), json!("Marie Curie"));
    assert_eq!(evaluate_formula("{budget} - {spent}", &original_record), json!(380000.0));

    // Shared formula vectors against Ada Lovelace record
    let record = json!({
        "budget": 500000.0,
        "spent": 120000.0,
        "rate": 0.2,
        "first_name": "Ada",
        "last_name": "Lovelace",
        "title": ""
    });

    assert_eq!(evaluate_formula("{budget}", &record), json!(500000.0));
    assert_eq!(evaluate_formula("{budget} * 0.20", &record), json!(100000.0));
    assert_eq!(evaluate_formula("{budget} / 10", &record), json!(50000.0));
    assert_eq!(evaluate_formula("{budget} - {spent}", &record), json!(380000.0));
    assert_eq!(evaluate_formula("{first_name} + \" \" + {last_name}", &record), json!("Ada Lovelace"));
    assert_eq!(evaluate_formula("{budget} - {spent} * {rate}", &record), json!(476000.0));
    assert_eq!(evaluate_formula("({budget} - {spent}) * {rate}", &record), json!(76000.0));
    assert_eq!(evaluate_formula("IF({spent} > 100000, {budget} - {spent}, 0)", &record), json!(380000.0));
    assert_eq!(evaluate_formula("IF(ISBLANK({title}), \"untitled\", {title})", &record), json!("untitled"));
    assert_eq!(evaluate_formula("ROUND({budget} * {rate}, 0)", &record), json!(100000.0));
    assert_eq!(evaluate_formula("AND({spent} > 0, {budget} > {spent})", &record), json!(true));
    assert_eq!(evaluate_formula("{missing}", &record), serde_json::Value::Null);
    assert_eq!(evaluate_formula("{budget} / 0", &record), serde_json::Value::Null);
}

#[test]
fn test_relational_lookup() {
    let mut field = FieldSpec::simple("investigator_email", "Investigator Email", FieldType::Lookup, false, false);
    field.target_display_field = Some("email".to_string());

    let record = json!({ "id": "APP-001" });
    let linked = vec![
        json!({ "id": "INV-01", "name": "Dr. Marie Curie", "email": "curie@univ.edu" })
    ];

    let computed = compute_field_value(&field, &record, &linked);
    assert_eq!(computed, json!("curie@univ.edu"));
}

#[test]
fn test_relational_count() {
    let field = FieldSpec::simple("allocations_count", "Allocations Count", FieldType::Count, false, false);
    let record = json!({ "id": "APP-001" });
    let linked = vec![
        json!({ "id": "ALC-101", "amount": 100000.0 }),
        json!({ "id": "ALC-102", "amount": 150000.0 }),
        json!({ "id": "ALC-103", "amount": 250000.0 }),
    ];

    let computed = compute_field_value(&field, &record, &linked);
    assert_eq!(computed, json!(3));
}

#[test]
fn test_relational_rollup() {
    let mut field = FieldSpec::simple("total_disbursed", "Total Disbursed", FieldType::Rollup, false, false);
    field.target_display_field = Some("amount".to_string());
    field.rollup_function = Some("sum".to_string());

    let record = json!({ "id": "APP-001" });
    let linked = vec![
        json!({ "id": "ALC-101", "amount": 100000.0 }),
        json!({ "id": "ALC-102", "amount": 150000.0 }),
        json!({ "id": "ALC-103", "amount": 250000.0 }),
    ];

    // SUM
    let computed_sum = compute_field_value(&field, &record, &linked);
    assert_eq!(computed_sum, json!(500000.0));

    // AVG
    field.rollup_function = Some("avg".to_string());
    let computed_avg = compute_field_value(&field, &record, &linked);
    assert_eq!(computed_avg, json!(500000.0 / 3.0));

    // MAX
    field.rollup_function = Some("max".to_string());
    let computed_max = compute_field_value(&field, &record, &linked);
    assert_eq!(computed_max, json!(250000.0));
}

#[test]
fn test_rich_field_type_serialization() {
    let mut field = FieldSpec::simple("grant_rating", "Grant Rating", FieldType::Rating, false, false);
    field.currency_symbol = Some("$".to_string());
    field.precision = Some(2);
    field.select_options = vec!["Level 1".to_string(), "Level 2".to_string()];

    let serialized = serde_json::to_string(&field).expect("Must serialize FieldSpec");
    let deserialized: FieldSpec = serde_json::from_str(&serialized).expect("Must deserialize FieldSpec");

    assert_eq!(deserialized.field_type, FieldType::Rating);
    assert_eq!(deserialized.select_options.len(), 2);
    assert_eq!(deserialized.currency_symbol.as_deref(), Some("$"));
}
