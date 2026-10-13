//! Links phase 1: the link field type and what the manifest validator checks about it.

use scaffoldry_engine::{validate_manifest, AppManifest, AppTable, FieldSpec, FieldType, LinkCardinality, LinkOnDelete};
use serde_json::json;

fn plain(name: &str) -> FieldSpec {
    FieldSpec::simple(name, name, FieldType::Text, false, false)
}

fn link(name: &str, target_table: &str) -> FieldSpec {
    let mut f = FieldSpec::simple(name, name, FieldType::Link, false, false);
    f.link_target_table = Some(target_table.to_string());
    f
}

fn table(id: &str, fields: Vec<FieldSpec>) -> AppTable {
    AppTable {
        id: id.to_string(),
        name: id.to_string(),
        slug: id.to_string(),
        description: None,
        icon: None,
        fields,
        primary_field: None,
        sample_records: vec![],
    }
}

fn manifest(tables: Vec<AppTable>) -> AppManifest {
    AppManifest {
        slug: "linking".into(),
        title: "Linking".into(),
        description: String::new(),
        organization_code: "X".into(),
        department: "x".into(),
        workspace_id: None,
        herm_capability_id: None,
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

fn problems(m: &AppManifest) -> Vec<String> {
    validate_manifest(m).err().unwrap_or_default()
}

#[test]
fn a_link_to_a_table_that_exists_is_valid_and_a_link_to_none_is_not() {
    let ok = manifest(vec![table("apps", vec![link("program", "programs")]), table("programs", vec![plain("code")])]);
    assert!(validate_manifest(&ok).is_ok(), "{:?}", problems(&ok));

    let missing = manifest(vec![table("apps", vec![link("program", "nope")])]);
    assert!(problems(&missing).iter().any(|p| p.contains("'program'") && p.contains("'nope'")), "{:?}", problems(&missing));

    let mut unnamed = link("program", "programs");
    unnamed.link_target_table = None;
    let none = manifest(vec![table("apps", vec![unnamed]), table("programs", vec![plain("code")])]);
    assert!(problems(&none).iter().any(|p| p.contains("'program'") && p.contains("target table")), "{:?}", problems(&none));
}

#[test]
fn a_related_field_must_be_a_link_on_the_target_that_points_back() {
    let mut forward = link("program", "programs");
    forward.link_related_field = Some("applications".into());

    // The target has no such field.
    let none = manifest(vec![table("apps", vec![forward.clone()]), table("programs", vec![plain("code")])]);
    assert!(problems(&none).iter().any(|p| p.contains("'applications'")), "{:?}", problems(&none));

    // The field exists but is not a link.
    let text = manifest(vec![table("apps", vec![forward.clone()]), table("programs", vec![plain("applications")])]);
    assert!(problems(&text).iter().any(|p| p.contains("'applications'") && p.contains("not a Link field")), "{:?}", problems(&text));

    // It is a link, but it points at another table.
    let elsewhere = manifest(vec![table("apps", vec![forward.clone()]), table("programs", vec![link("applications", "other")]), table("other", vec![plain("x")])]);
    assert!(problems(&elsewhere).iter().any(|p| p.contains("'applications'") && p.contains("point")), "{:?}", problems(&elsewhere));

    // It points at the right table but names a different field back.
    let mut wrong_back = link("applications", "apps");
    wrong_back.link_related_field = Some("something_else".into());
    let m = manifest(vec![table("apps", vec![forward.clone()]), table("programs", vec![wrong_back])]);
    assert!(problems(&m).iter().any(|p| p.contains("'applications'") && p.contains("point")), "{:?}", problems(&m));

    // A pair that names each other is valid.
    let mut back = link("applications", "apps");
    back.link_related_field = Some("program".into());
    let good = manifest(vec![table("apps", vec![forward]), table("programs", vec![back])]);
    assert!(validate_manifest(&good).is_ok(), "{:?}", problems(&good));
}

#[test]
fn a_link_to_another_app_must_name_a_dataset_app_and_none_exist_yet() {
    let mut f = link("program", "programs");
    f.link_target_app = Some("registrar-feed".into());
    let m = manifest(vec![table("apps", vec![f])]);
    assert!(problems(&m).iter().any(|p| p.contains("'registrar-feed'") && p.contains("dataset app")), "{:?}", problems(&m));
}

#[test]
fn a_link_field_cannot_be_indexed_because_links_have_their_own_index() {
    let mut f = link("program", "programs");
    f.indexed = true;
    let m = manifest(vec![table("apps", vec![f]), table("programs", vec![plain("code")])]);
    assert!(problems(&m).iter().any(|p| p.contains("'program'") && p.contains("indexed")), "{:?}", problems(&m));
}

#[test]
fn cardinality_and_on_delete_default_to_many_and_unlink_and_round_trip() {
    let f: FieldSpec = serde_json::from_value(json!({
        "name": "program", "label": "Program", "field_type": "Link", "required": false, "ferpa_sensitive": false,
        "link_target_table": "programs"
    }))
    .unwrap();
    assert_eq!(f.link_cardinality, LinkCardinality::Many);
    assert_eq!(f.link_on_delete, LinkOnDelete::Unlink);

    let mut one = link("program", "programs");
    one.link_cardinality = LinkCardinality::One;
    one.link_on_delete = LinkOnDelete::Restrict;
    let back: FieldSpec = serde_json::from_value(serde_json::to_value(&one).unwrap()).unwrap();
    assert_eq!(back.link_cardinality, LinkCardinality::One);
    assert_eq!(back.link_on_delete, LinkOnDelete::Restrict);
    assert_eq!(serde_json::to_value(&one).unwrap()["link_cardinality"], "One");
}
