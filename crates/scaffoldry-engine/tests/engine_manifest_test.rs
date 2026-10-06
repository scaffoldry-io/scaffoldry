//! Dynamic Engine Manifest & DNS Host Header Routing Integration Test

use scaffoldry_core::standards::eduperson::EduPersonIdentity;
use scaffoldry_engine::{
    AppManifest, AppView, FieldSpec, FieldType, HostRouter, ManifestEngine, ViewType,
};
use serde_json::json;

fn sample_manifest() -> AppManifest {
    AppManifest {
        slug: "bio-lab-inventory".to_string(),
        title: "Biology Lab Equipment Inventory".to_string(),
        description: "Departmental lab instrument tracking".to_string(),
        organization_code: "DEPT-BIO".to_string(),
        department: "biology".to_string(),
        herm_capability_id: Some("2.2.3".to_string()),
        custom_domain: Some("bio-inventory.science.state.edu".to_string()),
        custom_domain_verified: true,
        views: vec![
            AppView::table(
                "inventory-table",
                "All Lab Equipment",
                ViewType::Table,
                vec![
                    FieldSpec::simple("item_name", "Equipment Name", FieldType::Text, true, false),
                    FieldSpec::simple("serial_number", "Serial Number", FieldType::Text, true, false),
                    FieldSpec::simple("operator_eval", "Student Operator Evaluation", FieldType::Text, false, true),
                ],
            )
        ],
        ceds_mappings: [
            ("item_name".to_string(), "000185".to_string()), // CEDS FacilityIdentifier / Equipment
        ].into_iter().collect(),
        tables: vec![],
        relationships: vec![],
    }
}

#[test]
fn test_manifest_registration_and_dns_host_routing() {
    let mut engine = ManifestEngine::new().expect("Failed to initialize Manifest Engine");
    let manifest = sample_manifest();

    // 1. Register Manifest
    engine.register_manifest(manifest.clone()).expect("Failed to register manifest");

    // 2. Resolve via Host Header (DNS Aliasing)
    let resolved_by_domain = engine.resolve_by_host("bio-inventory.science.state.edu");
    assert!(resolved_by_domain.is_some(), "Must resolve app by custom domain host header");
    assert_eq!(resolved_by_domain.unwrap().slug, "bio-lab-inventory");

    // 3. Resolve via Slug
    let resolved_by_slug = engine.resolve_by_slug("bio-lab-inventory");
    assert!(resolved_by_slug.is_some(), "Must resolve app by slug");

    // 4. Unregistered Domain returns None
    let unmapped = engine.resolve_by_host("unregistered.domain.edu");
    assert!(unmapped.is_none());

    // 5. Validate Record Payload Submission
    let valid_payload = json!({
        "item_name": "Fluorescence Microscope",
        "serial_number": "FM-8821",
        "operator_eval": "Satisfactory performance on bioassay"
    });

    let faculty = EduPersonIdentity::parse("prof.curie@science.state.edu", vec!["faculty@science.state.edu"]).unwrap();
    let physics_student = EduPersonIdentity::parse("einstein@physics.state.edu", vec!["student@physics.state.edu"]).unwrap();

    // Faculty authorized write
    let submission_result = engine.submit_record(&faculty, "bio-lab-inventory", &valid_payload);
    assert!(submission_result.is_ok(), "Faculty must be authorized to submit record");

    let record = submission_result.unwrap();
    assert_eq!(record.data["item_name"], "Fluorescence Microscope");
    assert_eq!(record.ceds_mapping["item_name"], "000185");
    assert!(record.is_ferpa_sensitive, "Must detect FERPA sensitivity from field spec");

    // Cross-department unauthorized write
    let unauthorized_submission = engine.submit_record(&physics_student, "bio-lab-inventory", &valid_payload);
    assert!(unauthorized_submission.is_err(), "Cross-department student write must be denied");
}
