use scaffoldry_core::access::{
    never, rule_sentence, Op, Operand, PersonAttr, Pred, RowOp, RowRule,
};
use serde_json::json;

#[test]
fn test_sentence_for_advisor_eq_address() {
    let rule = RowRule {
        id: "r1".into(),
        table_id: "students".into(),
        ops: vec![RowOp::Read],
        visible_when: Pred::Cmp {
            field: "advisor".into(),
            op: Op::Eq,
            value: Operand::Person(PersonAttr::Address),
        },
        except_roles: vec!["owner".into(), "admin".into()],
        message: None,
    };
    assert_eq!(
        rule_sentence(&rule),
        "People who are not owners or admins can see only rows where Advisor is themselves."
    );
}

#[test]
fn test_sentence_for_plain_tree_is_nonempty() {
    let rule = RowRule {
        id: "r2".into(),
        table_id: "students".into(),
        ops: vec![RowOp::Read],
        visible_when: Pred::Or(vec![
            Pred::Cmp {
                field: "gpa".into(),
                op: Op::Gt,
                value: Operand::Literal(json!(3.5)),
            },
            Pred::Cmp {
                field: "major".into(),
                op: Op::Eq,
                value: Operand::Literal(json!("CS")),
            },
        ]),
        except_roles: vec![],
        message: None,
    };
    let s = rule_sentence(&rule);
    assert!(!s.is_empty());
    assert!(s.contains("Gpa is greater than 3.5"));
    assert!(s.contains("Major is \"CS\"") || s.contains("Major is CS"));
}

#[test]
fn test_never_true_contradiction() {
    let x = Operand::Literal(json!("cs"));
    let contradiction = Pred::And(vec![
        Pred::Cmp {
            field: "dept".into(),
            op: Op::Eq,
            value: x.clone(),
        },
        Pred::Cmp {
            field: "dept".into(),
            op: Op::Ne,
            value: x,
        },
    ]);
    assert!(contradiction.is_never_true());
    assert!(never().is_never_true());

    // Normal comparison depends on record values, so not never_true
    let normal = Pred::Cmp {
        field: "dept".into(),
        op: Op::Eq,
        value: Operand::Literal(json!("cs")),
    };
    assert!(!normal.is_never_true());
}

#[test]
fn test_predicate_nodes_depth_fields() {
    let pred = Pred::And(vec![
        Pred::Cmp {
            field: "advisor".into(),
            op: Op::Eq,
            value: Operand::Person(PersonAttr::Address),
        },
        Pred::Cmp {
            field: "status".into(),
            op: Op::Eq,
            value: Operand::Literal(json!("active")),
        },
    ]);
    assert_eq!(pred.node_count(), 3);
    assert_eq!(pred.depth(), 2);
    let fields = pred.fields();
    assert_eq!(fields.len(), 2);
    assert!(fields.contains("advisor"));
    assert!(fields.contains("status"));
}
