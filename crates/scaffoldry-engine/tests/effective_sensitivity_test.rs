//! Admin console phase 4, first part: one function decides whether a field is sensitive.
//! The label set is empty today. The label table, routes and screens come in the later part.

use scaffoldry_core::standards::eduperson::EduPersonIdentity;
use scaffoldry_engine::{
    effective_ferpa_sensitive, AppManifest, AppView, DataLabel, FieldSpec, FieldType, LabelSet,
    ManifestEngine, ViewType,
};
use serde_json::json;
use std::path::{Path, PathBuf};

fn field(name: &str, flagged: bool) -> FieldSpec {
    FieldSpec::simple(name, name, FieldType::Text, false, flagged)
}

fn label(sensitive: bool) -> DataLabel {
    DataLabel { ferpa_sensitive: sensitive, note: "test".to_string(), set_by: "compliance@state.edu".to_string() }
}

#[test]
fn with_no_label_the_manifest_flag_decides() {
    assert!(!effective_ferpa_sensitive(&field("a", false), None));
    assert!(effective_ferpa_sensitive(&field("a", true), None));
}

#[test]
fn a_label_can_raise_a_field_the_manifest_did_not_flag() {
    assert!(effective_ferpa_sensitive(&field("a", false), Some(&label(true))));
}

#[test]
fn a_label_of_false_never_lowers_a_flagged_field() {
    assert!(effective_ferpa_sensitive(&field("a", true), Some(&label(false))));
    assert!(!effective_ferpa_sensitive(&field("a", false), Some(&label(false))));
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
            vec![field("title", false), field("notes", false)],
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
    let raised = engine.submit_record_labelled(&who(), "sensitivity-app", &payload, &labels).unwrap();
    assert!(raised.is_ferpa_sensitive, "the label raises the record");

    // A label on a field the record does not hold changes nothing.
    let without_field = json!({ "title": "t" });
    let r = engine.submit_record_labelled(&who(), "sensitivity-app", &without_field, &labels).unwrap();
    assert!(!r.is_ferpa_sensitive);
}

/// Every read of a field spec's flag goes through `effective_ferpa_sensitive`. A direct
/// `.ferpa_sensitive` read anywhere else in the source is a bypass of a label.
#[test]
fn no_source_file_reads_a_field_flag_outside_the_one_function() {
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
    let crates = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    let mut files = Vec::new();
    for c in std::fs::read_dir(&crates).unwrap() {
        let src = c.unwrap().path().join("src");
        if src.is_dir() {
            sources(&src, &mut files);
        }
    }
    assert!(files.len() > 10, "the scan found the sources");

    let mut hits = Vec::new();
    for f in &files {
        let text = std::fs::read_to_string(f).unwrap();
        // The one function is exempt.
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
