//! Views phase 7: `show_when` on the fields of a form view.
//!
//! The evaluator here is authoritative: the server runs it at submit. The browser has a second one, kept as small as this and
//! tested against the vectors below. The vectors are generated here, from this evaluator, into
//! `apps/web/src/test/vectors/show-when.json`, which the TypeScript test reads. Run this test with `SCAFFOLDRY_WRITE_VECTORS=1`
//! to write the file again. Without it, the test fails when the file no longer matches.

use scaffoldry_engine::show_when::{visible_fields, Condition, FormField, ShowOp};
use serde_json::{json, Value};

fn cond(field: &str, op: ShowOp, value: Value) -> Condition {
    Condition { field: field.to_string(), op, value }
}

fn field(name: &str, show_when: Vec<Condition>) -> FormField {
    FormField { name: name.to_string(), show_when }
}

/// A form of a few fields, and answers, with the fields each answer set shows.
struct Case {
    name: &'static str,
    fields: Vec<FormField>,
    answers: Value,
}

fn cases() -> Vec<Case> {
    let aid = || vec![field("kind", vec![]), field("amount", vec![cond("kind", ShowOp::Eq, json!("grant"))]), field("note", vec![cond("kind", ShowOp::Ne, json!("grant"))])];
    vec![
        Case { name: "eq shows when the answer matches", fields: aid(), answers: json!({ "kind": "grant" }) },
        Case { name: "eq and ne hide and show the other way when it does not", fields: aid(), answers: json!({ "kind": "loan" }) },
        Case { name: "an unanswered field is empty, so ne shows and eq hides", fields: aid(), answers: json!({}) },
        Case { name: "null and a blank string are empty", fields: aid(), answers: json!({ "kind": null }) },
        Case { name: "a blank string is empty too", fields: aid(), answers: json!({ "kind": "   " }) },
        Case {
            name: "in matches one of a list",
            fields: vec![field("level", vec![]), field("thesis", vec![cond("level", ShowOp::In, json!(["ms", "phd"]))])],
            answers: json!({ "level": "phd" }),
        },
        Case {
            name: "in does not match outside the list",
            fields: vec![field("level", vec![]), field("thesis", vec![cond("level", ShowOp::In, json!(["ms", "phd"]))])],
            answers: json!({ "level": "ba" }),
        },
        Case {
            name: "in takes a comma list too",
            fields: vec![field("level", vec![]), field("thesis", vec![cond("level", ShowOp::In, json!("ms, phd"))])],
            answers: json!({ "level": "ms" }),
        },
        Case {
            name: "is_empty and is_not_empty",
            fields: vec![field("email", vec![]), field("why_not", vec![cond("email", ShowOp::IsEmpty, Value::Null)]), field("confirm", vec![cond("email", ShowOp::IsNotEmpty, Value::Null)])],
            answers: json!({ "email": "a@b.edu" }),
        },
        Case {
            name: "is_empty when nothing is typed",
            fields: vec![field("email", vec![]), field("why_not", vec![cond("email", ShowOp::IsEmpty, Value::Null)]), field("confirm", vec![cond("email", ShowOp::IsNotEmpty, Value::Null)])],
            answers: json!({ "email": "" }),
        },
        Case {
            name: "an empty list is empty",
            fields: vec![field("tags", vec![]), field("why", vec![cond("tags", ShowOp::IsEmpty, Value::Null)])],
            answers: json!({ "tags": [] }),
        },
        Case {
            name: "a list answer matches eq and in when any item does",
            fields: vec![field("tags", vec![]), field("a", vec![cond("tags", ShowOp::Eq, json!("x"))]), field("b", vec![cond("tags", ShowOp::In, json!(["y", "z"]))])],
            answers: json!({ "tags": ["x", "z"] }),
        },
        Case {
            name: "every condition of a field must hold",
            fields: vec![field("a", vec![]), field("b", vec![]), field("c", vec![cond("a", ShowOp::Eq, json!("1")), cond("b", ShowOp::Eq, json!("2"))])],
            answers: json!({ "a": "1", "b": "3" }),
        },
        Case {
            name: "numbers and booleans compare as text",
            fields: vec![field("n", vec![]), field("flag", vec![]), field("x", vec![cond("n", ShowOp::Eq, json!("3"))]), field("y", vec![cond("flag", ShowOp::Eq, json!(true))])],
            answers: json!({ "n": 3, "flag": true }),
        },
        Case {
            name: "a hidden field answers as empty to the fields after it",
            fields: vec![
                field("kind", vec![]),
                field("amount", vec![cond("kind", ShowOp::Eq, json!("grant"))]),
                field("big", vec![cond("amount", ShowOp::IsNotEmpty, Value::Null)]),
                field("small", vec![cond("amount", ShowOp::IsEmpty, Value::Null)]),
            ],
            answers: json!({ "kind": "loan", "amount": "5000" }),
        },
        Case {
            name: "a condition on a field that is not on the form sees it empty",
            fields: vec![field("a", vec![cond("elsewhere", ShowOp::IsEmpty, Value::Null)])],
            answers: json!({ "elsewhere": "set" }),
        },
        Case {
            name: "a condition on a field that comes later sees it empty",
            fields: vec![field("a", vec![cond("b", ShowOp::IsNotEmpty, Value::Null)]), field("b", vec![])],
            answers: json!({ "b": "set" }),
        },
        Case { name: "a field with no condition always shows", fields: vec![field("a", vec![])], answers: json!({}) },
    ]
}

fn vectors() -> Value {
    let list: Vec<Value> = cases()
        .into_iter()
        .map(|c| {
            let shown = visible_fields(&c.fields, c.answers.as_object().expect("answers"));
            json!({
                "name": c.name,
                "fields": c.fields,
                "answers": c.answers,
                "visible": c.fields.iter().zip(shown).map(|(f, v)| json!({ "name": f.name, "visible": v })).collect::<Vec<_>>(),
            })
        })
        .collect();
    json!({ "note": "Generated by crates/scaffoldry-engine/tests/show_when_test.rs. Do not edit.", "vectors": list })
}

fn path() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../apps/web/src/test/vectors/show-when.json")
}

#[test]
fn the_vectors_file_is_what_this_evaluator_gives() {
    let text = serde_json::to_string_pretty(&vectors()).unwrap() + "\n";
    if std::env::var("SCAFFOLDRY_WRITE_VECTORS").is_ok() {
        std::fs::create_dir_all(path().parent().unwrap()).unwrap();
        std::fs::write(path(), &text).unwrap();
    }
    let on_disk = std::fs::read_to_string(path()).expect("the vectors file. Run this test with SCAFFOLDRY_WRITE_VECTORS=1 to write it");
    assert_eq!(on_disk, text, "the vectors file is stale. Run this test with SCAFFOLDRY_WRITE_VECTORS=1");
}

#[test]
fn the_cases_say_what_they_say() {
    let v = vectors();
    let shown = |name: &str| -> Vec<(String, bool)> {
        v["vectors"].as_array().unwrap().iter().find(|c| c["name"] == name).unwrap()["visible"].as_array().unwrap().iter().map(|f| (f["name"].as_str().unwrap().to_string(), f["visible"].as_bool().unwrap())).collect()
    };
    let t = |n: &str, b: bool| (n.to_string(), b);
    assert_eq!(shown("eq shows when the answer matches"), vec![t("kind", true), t("amount", true), t("note", false)]);
    assert_eq!(shown("eq and ne hide and show the other way when it does not"), vec![t("kind", true), t("amount", false), t("note", true)]);
    assert_eq!(shown("a hidden field answers as empty to the fields after it"), vec![t("kind", true), t("amount", false), t("big", false), t("small", true)]);
    assert_eq!(shown("a condition on a field that comes later sees it empty"), vec![t("a", false), t("b", true)]);
    assert_eq!(shown("every condition of a field must hold"), vec![t("a", true), t("b", true), t("c", false)]);
}
