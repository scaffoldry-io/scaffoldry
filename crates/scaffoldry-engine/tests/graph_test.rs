//! Calculation graph phase 2: the dependency graph of an app's computed fields.

use scaffoldry_engine::graph::{build_graph, NodeId};
use scaffoldry_engine::{validate_manifest, AppManifest, AppTable, FieldSpec, FieldType, LinkCardinality};

fn field(name: &str, label: &str) -> FieldSpec {
    FieldSpec::simple(name, label, FieldType::Number, false, false)
}

fn formula(name: &str, label: &str, expression: &str) -> FieldSpec {
    let mut f = FieldSpec::simple(name, label, FieldType::Formula, false, false);
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
    f
}

fn table(id: &str, fields: Vec<FieldSpec>) -> AppTable {
    AppTable { id: id.into(), name: id.into(), slug: id.into(), description: None, icon: None, fields, primary_field: None, sample_records: vec![] , coordinates: None, sealed: false, row_place: Default::default() }
}

fn manifest(tables: Vec<AppTable>) -> AppManifest {
    AppManifest {
        slug: "graph-app".into(),
        title: "Graph".into(),
        description: String::new(),
        organization_code: "BIO".into(),
        department: "biology".into(),
        workspace_id: None,
        herm_capability_id: None,
        coordinates: None, sealed: false,
        custom_domain: None,
        custom_domain_verified: false,
        tables,
        relationships: vec![],
        views: vec![],
        ceds_mappings: Default::default(),
        pages: vec![],
        access: Default::default(),
    }
}

fn node(table: &str, field: &str) -> NodeId {
    NodeId { table: table.into(), field: field.into() }
}

// 1. A cycle is named in plain words. A chain is ordered from the leaf up.
#[test]
fn a_cycle_is_named_in_plain_words_and_a_chain_is_ordered_leaf_first() {
    let cyclic = manifest(vec![table("t", vec![formula("status", "Status", "IF({remaining} < 0, \"Over\", \"OK\")"), formula("remaining", "Remaining", "{status}")])]);
    let err = build_graph(&cyclic).unwrap_err();
    assert_eq!(err.to_string(), "Status depends on Remaining, which depends on Status.");
    let failures = validate_manifest(&cyclic).unwrap_err();
    assert!(failures.iter().any(|p| p.contains("Status depends on Remaining, which depends on Status.")), "{failures:?}");

    let three = manifest(vec![table("t", vec![formula("a", "A", "{b}"), formula("b", "B", "{c}"), formula("c", "C", "{a}")])]);
    assert_eq!(build_graph(&three).unwrap_err().to_string(), "A depends on B, which depends on C, which depends on A.");

    let chain = manifest(vec![table("t", vec![formula("a", "A", "{b}"), formula("b", "B", "{c}"), field("c", "C")])]);
    let graph = build_graph(&chain).unwrap();
    assert_eq!(graph.order("t"), vec!["c", "b", "a"]);
    assert!(validate_manifest(&chain).is_ok());
}

// 2. A rollup feeds the formulas that read it, across tables. A lookup is downstream of its link and its target.
#[test]
fn edges_follow_links_rollups_and_lookups_across_tables() {
    let m = manifest(vec![
        table(
            "parents",
            vec![
                field("name", "Name"),
                link("children", "children"),
                computed("total", FieldType::Rollup, "children", Some("amount")),
                formula("double", "Double", "{total} * 2"),
                formula("label", "Label", "{double} + 1"),
            ],
        ),
        table("children", vec![field("amount", "Amount"), link("parent", "parents"), computed("parent_name", FieldType::Lookup, "parent", Some("name"))]),
    ]);
    let graph = build_graph(&m).unwrap();
    let order = graph.order("parents");
    let pos = |f: &str| order.iter().position(|x| x == f).unwrap();
    assert!(pos("total") < pos("double") && pos("double") < pos("label"), "{order:?}");

    // A change to a child's amount reaches the parent's rollup, then the formulas that read it.
    let affected = graph.affected(&[node("children", "amount")]);
    assert_eq!(affected, vec![node("parents", "total"), node("parents", "double"), node("parents", "label")]);
    // A change to the parent's name reaches the child's lookup of it, and nothing else.
    assert_eq!(graph.affected(&[node("parents", "name")]), vec![node("children", "parent_name")]);
    // A change to the link reaches what goes through it.
    assert!(graph.affected(&[node("children", "parent")]).contains(&node("children", "parent_name")));
}

// 3. `affected` for a change to a leaf is exactly its descendants, in the order they must run.
#[test]
fn affected_is_exactly_the_descendants_in_run_order() {
    let m = manifest(vec![table(
        "t",
        vec![field("leaf", "Leaf"), field("other", "Other"), formula("b", "B", "{leaf} + 1"), formula("a", "A", "{b} + {other}"), formula("unrelated", "Unrelated", "{other} + 1")],
    )]);
    let graph = build_graph(&m).unwrap();
    assert_eq!(graph.affected(&[node("t", "leaf")]), vec![node("t", "b"), node("t", "a")]);
    assert_eq!(graph.affected(&[node("t", "other")]), vec![node("t", "a"), node("t", "unrelated")]);
    assert!(graph.affected(&[node("t", "a")]).is_empty(), "a field nothing reads has no descendants");
}

// 4. A lookup through a link that is later removed fails validation.
#[test]
fn a_lookup_through_a_removed_link_fails_validation() {
    let ok = manifest(vec![
        table("parents", vec![field("name", "Name")]),
        table("children", vec![link("parent", "parents"), computed("parent_name", FieldType::Lookup, "parent", Some("name"))]),
    ]);
    assert!(validate_manifest(&ok).is_ok());
    let removed = manifest(vec![
        table("parents", vec![field("name", "Name")]),
        table("children", vec![computed("parent_name", FieldType::Lookup, "parent", Some("name"))]),
    ]);
    let failures = validate_manifest(&removed).unwrap_err();
    assert!(failures.iter().any(|p| p.contains("goes through 'parent', which this table does not have")), "{failures:?}");
    // And building the graph over it does not panic.
    let _ = build_graph(&removed);
}

// 5. A field name used twice is one node. The validator reports it, and the graph does not panic.
#[test]
fn a_duplicate_field_name_does_not_break_the_graph() {
    let m = manifest(vec![table("t", vec![field("a", "A"), field("a", "A again"), formula("b", "B", "{a} + 1")])]);
    let graph = build_graph(&m).unwrap();
    assert_eq!(graph.order("t"), vec!["a", "b"]);
    let failures = validate_manifest(&m).unwrap_err();
    assert!(failures.iter().any(|p| p.contains("used twice")), "{failures:?}");
}
