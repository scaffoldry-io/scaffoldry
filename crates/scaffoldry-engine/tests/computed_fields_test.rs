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



#[test]
fn test_logic_function_group() {
    let rec = json!({
        "status": "active",
        "empty": null
    });

    // 1. IF true
    assert_eq!(evaluate_formula(r#"IF(1, "yes", "no")"#, &rec), json!("yes"));
    // 2. IF false
    assert_eq!(evaluate_formula(r#"IF(0, "yes", "no")"#, &rec), json!("no"));
    // 3. IF blank condition
    assert_eq!(evaluate_formula(r#"IF(BLANK(), "yes", "no")"#, &rec), json!("no"));
    // 4. SWITCH match
    assert_eq!(evaluate_formula(r#"SWITCH("apple", "banana", 1, "apple", 2, 3)"#, &rec), json!(2.0));
    // 5. SWITCH default
    assert_eq!(evaluate_formula(r#"SWITCH("orange", "banana", 1, "apple", 2, 3)"#, &rec), json!(3.0));
    // 6. SWITCH blank matching blank
    assert_eq!(evaluate_formula(r#"SWITCH(BLANK(), "a", 1, BLANK(), 2, 0)"#, &rec), json!(2.0));
    // 7. AND / NOT
    assert_eq!(evaluate_formula(r#"AND(1, "true", NOT(0))"#, &rec), json!(true));
    // 8. OR with truthy
    assert_eq!(evaluate_formula(r#"OR(0, BLANK(), {status})"#, &rec), json!(true));
    // 9. ISBLANK on BLANK()
    assert_eq!(evaluate_formula(r#"ISBLANK(BLANK())"#, &rec), json!(true));
    // 10. ISBLANK on non-blank vs blank string
    assert_eq!(evaluate_formula(r#"ISBLANK("")"#, &rec), json!(true));
    assert_eq!(evaluate_formula(r#"ISBLANK("hello")"#, &rec), json!(false));
}

#[test]
fn test_math_function_group() {
    let rec = json!({});

    // 1. ROUND digits
    assert_eq!(evaluate_formula(r#"ROUND(3.456, 2)"#, &rec), json!(3.46));
    // 2. ROUND default 0
    assert_eq!(evaluate_formula(r#"ROUND(3.5)"#, &rec), json!(4.0));
    // 3. CEILING
    assert_eq!(evaluate_formula(r#"CEILING(4.2)"#, &rec), json!(5.0));
    // 4. CEILING blank
    assert_eq!(evaluate_formula(r#"CEILING(BLANK())"#, &rec), serde_json::Value::Null);
    // 5. FLOOR
    assert_eq!(evaluate_formula(r#"FLOOR(4.8)"#, &rec), json!(4.0));
    // 6. ABS
    assert_eq!(evaluate_formula(r#"ABS(-42)"#, &rec), json!(42.0));
    // 7. ABS blank
    assert_eq!(evaluate_formula(r#"ABS(BLANK())"#, &rec), serde_json::Value::Null);
    // 8. MOD
    assert_eq!(evaluate_formula(r#"MOD(10, 3)"#, &rec), json!(1.0));
    // 9. MOD div by zero
    assert_eq!(evaluate_formula(r#"MOD(10, 0)"#, &rec), serde_json::Value::Null);
    // 10. MIN and MAX with blanks
    assert_eq!(evaluate_formula(r#"MIN(5, 2, 8, BLANK())"#, &rec), json!(2.0));
    assert_eq!(evaluate_formula(r#"MAX(5, 2, 8, BLANK())"#, &rec), json!(8.0));
}

#[test]
fn test_text_function_group() {
    let rec = json!({ "first": "Marie", "last": "Curie" });

    // 1. CONCAT
    assert_eq!(evaluate_formula(r#"CONCAT("Hello", " ", "World")"#, &rec), json!("Hello World"));
    // 2. CONCAT with blank
    assert_eq!(evaluate_formula(r#"CONCAT("Hello", BLANK(), "World")"#, &rec), json!("HelloWorld"));
    // 3. LEN
    assert_eq!(evaluate_formula(r#"LEN("Antigravity")"#, &rec), json!(11));
    // 4. LEN blank
    assert_eq!(evaluate_formula(r#"LEN(BLANK())"#, &rec), json!(0));
    // 5. LEFT
    assert_eq!(evaluate_formula(r#"LEFT("University", 4)"#, &rec), json!("Univ"));
    // 6. RIGHT
    assert_eq!(evaluate_formula(r#"RIGHT("University", 4)"#, &rec), json!("sity"));
    // 7. MID
    assert_eq!(evaluate_formula(r#"MID("University", 5, 3)"#, &rec), json!("ers"));
    // 8. UPPER & LOWER
    assert_eq!(evaluate_formula(r#"UPPER("hello")"#, &rec), json!("HELLO"));
    assert_eq!(evaluate_formula(r#"LOWER("WORLD")"#, &rec), json!("world"));
    // 9. TRIM
    assert_eq!(evaluate_formula(r#"TRIM("  padded  ")"#, &rec), json!("padded"));
    // 10. SUBSTITUTE & FIND
    assert_eq!(evaluate_formula(r#"SUBSTITUTE("banana", "a", "o")"#, &rec), json!("bonono"));
    assert_eq!(evaluate_formula(r#"FIND("cat", "concatenate")"#, &rec), json!(4));
}

#[test]
fn test_date_function_group() {
    let rec = json!({ "date": "2026-10-11" });

    // 1. YEAR
    assert_eq!(evaluate_formula(r#"YEAR("2026-10-11")"#, &rec), json!(2026));
    // 2. MONTH
    assert_eq!(evaluate_formula(r#"MONTH("2026-10-11")"#, &rec), json!(10));
    // 3. DAY
    assert_eq!(evaluate_formula(r#"DAY("2026-10-11")"#, &rec), json!(11));
    // 4. YEAR blank
    assert_eq!(evaluate_formula(r#"YEAR(BLANK())"#, &rec), serde_json::Value::Null);
    // 5. DATEADD days
    assert_eq!(evaluate_formula(r#"DATEADD("2026-10-11", 5, "days")"#, &rec), json!("2026-10-16"));
    // 6. DATEADD years
    assert_eq!(evaluate_formula(r#"DATEADD("2026-10-11", -1, "years")"#, &rec), json!("2025-10-11"));
    // 7. DATEDIFF
    assert_eq!(evaluate_formula(r#"DATEDIFF("2026-10-16", "2026-10-11", "days")"#, &rec), json!(5));
    // 8. TODAY
    let today = evaluate_formula(r#"TODAY()"#, &rec);
    assert!(today.is_string() && today.as_str().unwrap().len() == 10);
    // 9. NOW
    let now = evaluate_formula(r#"NOW()"#, &rec);
    assert!(now.is_string() && now.as_str().unwrap().contains('T'));
    // 10. FORMAT
    assert_eq!(evaluate_formula(r#"FORMAT("2026-10-11", "YYYY/MM/DD")"#, &rec), json!("2026/10/11"));
}

#[test]
fn test_list_function_group() {
    let rec = json!({
        "tags": ["rust", "wasm", "cedar"],
        "scores": [10.0, 20.0, 30.0],
        "empty": []
    });

    // 1. ARRAYJOIN default
    assert_eq!(evaluate_formula(r#"ARRAYJOIN({tags})"#, &rec), json!("rust, wasm, cedar"));
    // 2. ARRAYJOIN custom separator
    assert_eq!(evaluate_formula(r#"ARRAYJOIN({tags}, "-")"#, &rec), json!("rust-wasm-cedar"));
    // 3. ARRAYJOIN blank
    assert_eq!(evaluate_formula(r#"ARRAYJOIN(BLANK())"#, &rec), json!(""));
    // 4. ARRAYUNIQUE
    assert_eq!(evaluate_formula(r#"ARRAYUNIQUE({tags})"#, &rec), json!(["rust", "wasm", "cedar"]));
    // 5. COUNT
    assert_eq!(evaluate_formula(r#"COUNT({tags})"#, &rec), json!(3));
    // 6. COUNT blank
    assert_eq!(evaluate_formula(r#"COUNT(BLANK())"#, &rec), json!(0));
    // 7. SUM
    assert_eq!(evaluate_formula(r#"SUM({scores})"#, &rec), json!(60.0));
    // 8. SUM blank
    assert_eq!(evaluate_formula(r#"SUM(BLANK())"#, &rec), json!(0.0));
    // 9. AVERAGE
    assert_eq!(evaluate_formula(r#"AVERAGE({scores})"#, &rec), json!(20.0));
    // 10. AVERAGE empty
    assert_eq!(evaluate_formula(r#"AVERAGE({empty})"#, &rec), serde_json::Value::Null);
}

#[test]
fn test_precedence_and_error_handling() {
    let rec = json!({
        "a": 5.0,
        "b": 3.0,
        "c": 2.0,
        "str": "not_a_num"
    });

    // Precedence: {a} + {b} * {c} -> 5 + (3 * 2) = 11
    assert_eq!(evaluate_formula(r#"{a} + {b} * {c}"#, &rec), json!(11.0));
    // Precedence: -{a} - -{b} -> -5 - (-3) = -2
    assert_eq!(evaluate_formula(r#"-{a} - -{b}"#, &rec), json!(-2.0));
    // Division by zero gives blank, not error string
    assert_eq!(evaluate_formula(r#"{a} / 0"#, &rec), serde_json::Value::Null);
    // Non-numeric operand gives blank, not error string
    assert_eq!(evaluate_formula(r#"{a} - {str}"#, &rec), serde_json::Value::Null);
    assert_eq!(evaluate_formula(r#"{str} * 2"#, &rec), serde_json::Value::Null);
}
