//! MCP apps phase 3: the manifest has a format. The same fixtures are checked against the JSON
//! Schema by `governance/scripts/validate-manifest.py` and against `validate_manifest` here.

use scaffoldry_engine::{validate_manifest, AppManifest, AppPage, AppTable, FieldSpec, FieldType, PageGrant, PageKind};
use std::path::PathBuf;

fn fixtures(kind: &str) -> Vec<(String, AppManifest)> {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/manifests").join(kind);
    let mut out: Vec<(String, AppManifest)> = std::fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("{}: {e}", dir.display()))
        .map(|entry| {
            let path = entry.unwrap().path();
            let text = std::fs::read_to_string(&path).unwrap();
            let manifest: AppManifest = serde_json::from_str(&text).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
            (path.file_name().unwrap().to_string_lossy().to_string(), manifest)
        })
        .collect();
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

#[test]
fn valid_fixtures_pass() {
    let valid = fixtures("valid");
    assert!(valid.len() >= 4, "the fixtures are present");
    for (name, m) in valid {
        if let Err(problems) = validate_manifest(&m) {
            panic!("{name} should pass: {problems:?}");
        }
    }
}

#[test]
fn invalid_fixtures_fail() {
    let invalid = fixtures("invalid");
    assert!(invalid.len() >= 6, "the fixtures are present");
    for (name, m) in invalid {
        assert!(validate_manifest(&m).is_err(), "{name} should be refused");
    }
}

fn base() -> AppManifest {
    serde_json::from_str(include_str!("fixtures/manifests/valid/one-table.json")).unwrap()
}

fn field(name: &str, t: FieldType) -> FieldSpec {
    FieldSpec::simple(name, name, t, false, false)
}

// 3. A formula naming a missing field returns that field's name.
#[test]
fn a_formula_naming_a_missing_field_names_it() {
    let mut m = base();
    let mut f = field("total", FieldType::Formula);
    f.formula_expression = Some("{gpa} + {missing_field}".to_string());
    m.tables[0].fields.push(f);
    let problems = validate_manifest(&m).unwrap_err();
    assert!(problems.iter().any(|p| p.contains("missing_field")), "{problems:?}");
}

#[test]
fn a_formula_that_does_not_parse_is_refused() {
    let mut m = base();
    let mut f = field("broken", FieldType::Formula);
    f.formula_expression = Some("{gpa".to_string());
    m.tables[0].fields.push(f);
    let problems = validate_manifest(&m).unwrap_err();
    assert!(problems.iter().any(|p| p.contains("broken") && p.contains("parse")), "{problems:?}");
}

// 4. A custom page whose grant writes a formula field is refused. So is an unknown field.
#[test]
fn a_grant_may_not_write_a_computed_field_or_name_a_field_that_is_missing() {
    let mut m = base();
    let grant = |read: &[&str], write: &[&str]| PageGrant {
        table_id: "students".to_string(),
        read: read.iter().map(|s| s.to_string()).collect(),
        write: write.iter().map(|s| s.to_string()).collect(),
        create: false,
        delete: false,
    };
    let page = |g: PageGrant| AppPage {
        id: "p".to_string(),
        title: "P".to_string(),
        kind: PageKind::Custom { source_sha256: "b".repeat(64), grants: vec![g] },
    };

    m.pages = vec![page(grant(&["standing"], &["standing"]))];
    let problems = validate_manifest(&m).unwrap_err();
    assert!(problems.iter().any(|p| p.contains("computed field 'standing'")), "{problems:?}");

    m.pages = vec![page(grant(&["nope"], &[]))];
    let problems = validate_manifest(&m).unwrap_err();
    assert!(problems.iter().any(|p| p.contains("'nope'")), "{problems:?}");

    m.pages = vec![page(grant(&["name"], &["gpa"]))];
    let problems = validate_manifest(&m).unwrap_err();
    assert!(problems.iter().any(|p| p.contains("without reading it")), "write must be a subset of read: {problems:?}");

    m.pages = vec![page(PageGrant { table_id: "ghost".to_string(), read: vec![], write: vec![], create: false, delete: false })];
    assert!(validate_manifest(&m).unwrap_err().iter().any(|p| p.contains("'ghost'")));

    m.pages = vec![page(grant(&["name", "gpa"], &["gpa"]))];
    assert!(validate_manifest(&m).is_ok());
}

#[test]
fn it_returns_every_problem_not_the_first() {
    let mut m = base();
    m.slug = "Bad Slug".to_string();
    m.tables[0].fields.push(field("Upper", FieldType::Text));
    m.tables[0].fields.push(field("name", FieldType::Text));
    m.pages = vec![AppPage { id: "x".into(), title: "x".into(), kind: PageKind::View { view_id: "ghost".into() } }];
    let problems = validate_manifest(&m).unwrap_err();
    assert!(problems.len() >= 4, "{problems:?}");
}

#[test]
fn views_pages_and_links_must_point_at_things_that_exist() {
    let mut m = base();
    m.views[0].table_id = Some("ghost".to_string());
    assert!(validate_manifest(&m).unwrap_err().iter().any(|p| p.contains("'ghost'")));

    let mut m = base();
    m.views[0].fields.push(field("not_in_table", FieldType::Text));
    assert!(validate_manifest(&m).unwrap_err().iter().any(|p| p.contains("not_in_table")));

    // A rollup goes through a link field of its table, and names a target field that exists.
    let mut m = base();
    let mut courses_link = field("courses", FieldType::Link);
    courses_link.link_target_table = Some("courses".to_string());
    m.tables[0].fields.push(courses_link);
    m.tables.push(AppTable {
        id: "courses".into(),
        name: "Courses".into(),
        slug: "courses".into(),
        description: None,
        icon: None,
        fields: vec![field("credits", FieldType::Number)],
        primary_field: None,
        sample_records: vec![],
    });
    let mut roll = field("total_credits", FieldType::Rollup);
    roll.target_display_field = Some("credits".to_string());
    m.tables[0].fields.push(roll);
    assert!(validate_manifest(&m).unwrap_err().iter().any(|p| p.contains("computed_via")));
    m.tables[0].fields.last_mut().unwrap().computed_via = Some("courses".to_string());
    assert!(validate_manifest(&m).is_ok());
    m.tables[0].fields.last_mut().unwrap().target_display_field = Some("missing".to_string());
    assert!(validate_manifest(&m).unwrap_err().iter().any(|p| p.contains("'missing'")));
}

#[test]
fn a_manifest_over_256_kb_is_refused() {
    let mut m = base();
    m.description = "x".repeat(300 * 1024);
    assert!(validate_manifest(&m).unwrap_err().iter().any(|p| p.contains("bytes")));
}
