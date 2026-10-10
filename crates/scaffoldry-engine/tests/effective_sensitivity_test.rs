//! One function decides whether a field is sensitive, and which categories it carries.
//! Admin console phase 4 (first part) made it the single place. Sensitive content phase 1
//! made the answer a set of categories, each of which the organization marks protected or not.

use scaffoldry_core::standards::eduperson::EduPersonIdentity;
use scaffoldry_engine::sensitivity::{validate_categories, validate_detectors, Category};
use scaffoldry_engine::{
    effective_ferpa_sensitive, AppManifest, AppView, DataLabel, FieldSpec, FieldType, LabelSet,
    ManifestEngine, ViewType,
};
use serde_json::json;
use std::path::{Path, PathBuf};

fn field(name: &str, flagged: bool) -> FieldSpec {
    FieldSpec::simple(name, name, FieldType::Text, false, flagged)
}

fn field_in(name: &str, categories: &[&str]) -> FieldSpec {
    let mut f = field(name, false);
    f.categories = categories.iter().map(|c| c.to_string()).collect();
    f
}

fn label(sensitive: bool) -> DataLabel {
    DataLabel { ferpa_sensitive: sensitive, note: "test".to_string(), set_by: "compliance@state.edu".to_string() }
}

fn category(id: &str, protected: bool) -> Category {
    Category { id: id.to_string(), name: id.to_string(), source: String::new(), protected, detectors: vec![] }
}

#[test]
fn with_no_label_or_category_the_manifest_flag_decides() {
    assert!(!effective_ferpa_sensitive(&field("a", false), None, &[]).is_protected());
    assert!(effective_ferpa_sensitive(&field("a", true), None, &[]).is_protected());
}

#[test]
fn a_label_can_raise_a_field_the_manifest_did_not_flag() {
    assert!(effective_ferpa_sensitive(&field("a", false), Some(&label(true)), &[]).is_protected());
}

#[test]
fn a_label_of_false_never_lowers_a_flagged_field() {
    assert!(effective_ferpa_sensitive(&field("a", true), Some(&label(false)), &[]).is_protected());
    assert!(!effective_ferpa_sensitive(&field("a", false), Some(&label(false)), &[]).is_protected());
}

// 7. The legacy flag is the category ferpa and is protected. A category marked unprotected is not.
#[test]
fn the_legacy_flag_is_the_category_ferpa_and_is_protected() {
    let s = effective_ferpa_sensitive(&field("a", true), None, &[]);
    assert!(s.is_protected());
    assert!(s.categories.contains("ferpa"));

    // Even when the organization has no ferpa category, or marks it unprotected.
    let s = effective_ferpa_sensitive(&field("a", true), None, &[category("ferpa", false)]);
    assert!(s.is_protected(), "the legacy flag protects on its own");
}

#[test]
fn a_field_is_protected_when_any_of_its_categories_is_protected() {
    let cats = [category("pci", true), category("pii", false)];
    let s = effective_ferpa_sensitive(&field_in("card", &["pci"]), None, &cats);
    assert!(s.is_protected());
    assert!(s.categories.contains("pci"));

    let both = effective_ferpa_sensitive(&field_in("x", &["pii", "pci"]), None, &cats);
    assert!(both.is_protected(), "one protected category is enough");
    assert_eq!(both.categories.len(), 2);
}

#[test]
fn a_field_whose_only_category_is_not_protected_is_not_protected() {
    let cats = [category("pii", false)];
    let s = effective_ferpa_sensitive(&field_in("email", &["pii"]), None, &cats);
    assert!(!s.is_protected());
    assert!(s.categories.contains("pii"), "it still carries the category");

    let unknown = effective_ferpa_sensitive(&field_in("x", &["not-defined"]), None, &cats);
    assert!(!unknown.is_protected(), "a category the organization has not defined protects nothing");
}

fn manifest() -> AppManifest {
    AppManifest {
        slug: "sensitivity-app".to_string(),
        title: "Sensitivity".to_string(),
        description: String::new(),
        organization_code: "X".to_string(),
        department: "x".to_string(),
        workspace_id: None,
        herm_capability_id: None,
        custom_domain: None,
        custom_domain_verified: false,
        views: vec![AppView::table(
            "main",
            "Main",
            ViewType::Table,
            vec![field("title", false), field("notes", false), field_in("card", &["pci"])],
        )],
        ceds_mappings: Default::default(),
        tables: vec![],
        relationships: vec![],
    }
}

fn who() -> EduPersonIdentity {
    EduPersonIdentity::parse("prof.curie@science.state.edu", vec!["faculty@science.state.edu"]).unwrap()
}

#[test]
fn a_submitted_record_is_flagged_when_a_label_raises_a_field_it_holds() {
    let mut engine = ManifestEngine::new().unwrap();
    engine.register_manifest(manifest()).unwrap();
    let payload = json!({ "title": "t", "notes": "n" });

    let none = engine.submit_record(&who(), "sensitivity-app", &payload).unwrap();
    assert!(!none.is_ferpa_sensitive, "no label, no flag");

    let mut labels = LabelSet::new();
    labels.insert(("sensitivity-app".into(), "".into(), "notes".into()), label(true));
    let raised = engine.submit_record_labelled(&who(), "sensitivity-app", &payload, &labels, &[]).unwrap();
    assert!(raised.is_ferpa_sensitive, "the label raises the record");

    let without_field = json!({ "title": "t" });
    let r = engine.submit_record_labelled(&who(), "sensitivity-app", &without_field, &labels, &[]).unwrap();
    assert!(!r.is_ferpa_sensitive);
}

#[test]
fn a_submitted_record_is_flagged_by_a_protected_category_only() {
    let mut engine = ManifestEngine::new().unwrap();
    engine.register_manifest(manifest()).unwrap();
    let payload = json!({ "title": "t", "card": "4111 1111 1111 1111" });
    let labels = LabelSet::new();

    let protected = engine
        .submit_record_labelled(&who(), "sensitivity-app", &payload, &labels, &[category("pci", true)])
        .unwrap();
    assert!(protected.is_ferpa_sensitive);

    let open = engine
        .submit_record_labelled(&who(), "sensitivity-app", &payload, &labels, &[category("pci", false)])
        .unwrap();
    assert!(!open.is_ferpa_sensitive, "pci marked unprotected protects nothing");
}

// 8. Settings are validated, and an error names its path.
#[test]
fn a_category_naming_an_unknown_detector_is_rejected_with_its_path() {
    let detectors = validate_detectors(&json!([{ "id": "ssn", "kind": "us_ssn", "action": "flag", "enabled": true }])).unwrap();
    let ok = json!([{ "id": "pii", "name": "PII", "source": "NIST SP 800-122", "protected": true, "detectors": ["ssn"] }]);
    assert!(validate_categories(&ok, &detectors).is_ok());

    let bad = json!([
        { "id": "pii", "name": "PII", "protected": true, "detectors": ["ssn"] },
        { "id": "pci", "name": "Cards", "protected": true, "detectors": ["ssn", "card_typo"] }
    ]);
    let err = validate_categories(&bad, &detectors).unwrap_err();
    assert_eq!(err.path, "[1].detectors[1]");
    assert!(err.message.contains("card_typo"), "{}", err.message);
}

#[test]
fn settings_reject_duplicates_bad_shapes_and_oversize_lists() {
    // A duplicate detector id.
    let dup = json!([{ "id": "a", "kind": "email" }, { "id": "a", "kind": "us_ssn" }]);
    assert_eq!(validate_detectors(&dup).unwrap_err().path, "[1].id");

    // A shape over 64 characters, a shape with no shape, and a bad checksum.
    let long = json!([{ "id": "s", "kind": "shape", "shape": "#".repeat(65) }]);
    assert_eq!(validate_detectors(&long).unwrap_err().path, "[0].shape");
    let missing = json!([{ "id": "s", "kind": "shape" }]);
    assert_eq!(validate_detectors(&missing).unwrap_err().path, "[0].shape");
    let checksum = json!([{ "id": "s", "kind": "shape", "shape": "####", "checksum": "crc" }]);
    assert_eq!(validate_detectors(&checksum).unwrap_err().path, "[0].checksum");

    // An unknown kind and an unknown action.
    assert_eq!(validate_detectors(&json!([{ "id": "x", "kind": "dna" }])).unwrap_err().path, "[0]");
    assert_eq!(validate_detectors(&json!([{ "id": "x", "kind": "email", "action": "delete" }])).unwrap_err().path, "[0]");

    // Not a list.
    assert_eq!(validate_detectors(&json!({})).unwrap_err().path, "");

    // 101 detectors and 51 categories.
    let many: Vec<_> = (0..101).map(|i| json!({ "id": format!("d{i}"), "kind": "email" })).collect();
    assert!(validate_detectors(&json!(many)).is_err());
    let cats: Vec<_> = (0..51).map(|i| json!({ "id": format!("c{i}"), "name": "c", "protected": false, "detectors": [] })).collect();
    assert!(validate_categories(&json!(cats), &[]).is_err());

    // A duplicate category id, and a bad id.
    let two = json!([{ "id": "a", "name": "A", "protected": true, "detectors": [] }, { "id": "a", "name": "B", "protected": true, "detectors": [] }]);
    assert_eq!(validate_categories(&two, &[]).unwrap_err().path, "[1].id");
    let bad_id = json!([{ "id": "Has Space", "name": "A", "protected": true, "detectors": [] }]);
    assert_eq!(validate_categories(&bad_id, &[]).unwrap_err().path, "[0].id");
}

fn sources(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let p = entry.unwrap().path();
        if p.is_dir() {
            sources(&p, out);
        } else if p.extension().is_some_and(|e| e == "rs") {
            out.push(p);
        }
    }
}

fn all_sources() -> Vec<PathBuf> {
    let crates = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    let mut files = Vec::new();
    for c in std::fs::read_dir(&crates).unwrap() {
        let src = c.unwrap().path().join("src");
        if src.is_dir() {
            sources(&src, &mut files);
        }
    }
    assert!(files.len() > 10, "the scan found the sources");
    files
}

/// Every read of a field spec's flag goes through `effective_ferpa_sensitive`. A direct
/// `.ferpa_sensitive` read anywhere else in the source is a bypass of a label or a category.
#[test]
fn no_source_file_reads_a_field_flag_outside_the_one_function() {
    let mut hits = Vec::new();
    for f in &all_sources() {
        let text = std::fs::read_to_string(f).unwrap();
        let exempt = text.find("pub fn effective_ferpa_sensitive").map(|start| {
            let end = text[start..].find("\n}\n").map(|e| start + e + 3).unwrap_or(text.len());
            (start, end)
        });
        let mut from = 0;
        while let Some(at) = text[from..].find(".ferpa_sensitive") {
            let pos = from + at;
            let after = text[pos + ".ferpa_sensitive".len()..].chars().next();
            let is_word_end = !after.is_some_and(|c| c.is_alphanumeric() || c == '_');
            let in_exempt = exempt.is_some_and(|(s, e)| pos >= s && pos < e);
            if is_word_end && !in_exempt {
                hits.push(format!("{}:{}", f.display(), text[..pos].matches('\n').count() + 1));
            }
            from = pos + 1;
        }
    }
    assert!(hits.is_empty(), "direct reads of a field's flag:\n{}", hits.join("\n"));
}

// 10. The name `ferpa` as a category appears in code once: the legacy mapping.
#[test]
fn the_category_name_ferpa_appears_in_source_only_as_the_legacy_mapping() {
    let mut hits = Vec::new();
    for f in &all_sources() {
        let text = std::fs::read_to_string(f).unwrap();
        for (n, line) in text.lines().enumerate() {
            if line.contains("\"ferpa\"") {
                hits.push(format!("{}:{}: {}", f.display(), n + 1, line.trim()));
            }
        }
    }
    println!("grep '\"ferpa\"' over crates/*/src:\n{}", hits.join("\n"));
    assert_eq!(hits.len(), 1, "only the legacy mapping may name the category:\n{}", hits.join("\n"));
    assert!(hits[0].contains("LEGACY_FLAG_CATEGORY"), "{}", hits[0]);
}
