//! Cedar Policy Authorization Engine Test Suite
//! Validates institutional RBAC/ABAC, FERPA safeguards, and InCommon scoped affiliations.

use scaffoldry_core::standards::eduperson::EduPersonIdentity;
use scaffoldry_policy::{PolicyDecision, ScaffoldryPolicyEngine};

#[test]
fn test_ceds_eduperson_policy_authorization() {
    let engine = ScaffoldryPolicyEngine::default_institutional_engine().expect("Failed to initialize Cedar policy engine");

    // Faculty identity with department metadata
    let faculty = EduPersonIdentity::parse("prof.curie@science.state.edu", vec!["faculty@science.state.edu"]).unwrap();
    let student = EduPersonIdentity::parse("student.smith@science.state.edu", vec!["student@science.state.edu"]).unwrap();
    let compliance = EduPersonIdentity::parse("compliance.officer@science.state.edu", vec!["staff@science.state.edu"]).unwrap();
    let physics_student = EduPersonIdentity::parse("einstein@physics.state.edu", vec!["student@physics.state.edu"]).unwrap();

    // 1. Biology Faculty can read and write Biology Lab records
    let bio_faculty_write = engine.authorize_record_action(
        &faculty,
        "write",
        "bio-lab-inventory",
        "biology",
        false, // not ferpa sensitive
    ).expect("Evaluation must succeed");
    assert_eq!(bio_faculty_write.decision, PolicyDecision::Allow, "Faculty must be allowed to write departmental records");

    // 2. Student can read non-FERPA records
    let student_read = engine.authorize_record_action(
        &student,
        "read",
        "bio-lab-inventory",
        "biology",
        false,
    ).expect("Evaluation must succeed");
    assert_eq!(student_read.decision, PolicyDecision::Allow, "Student must be allowed to read departmental records");

    // 3. Student is FORBIDDEN from exporting FERPA-sensitive records
    let student_ferpa_export = engine.authorize_record_action(
        &student,
        "export",
        "bio-lab-inventory",
        "biology",
        true, // FERPA SENSITIVE
    ).expect("Evaluation must succeed");
    assert_eq!(student_ferpa_export.decision, PolicyDecision::Deny, "FERPA policy must deny student export of sensitive student records");

    // 4. Compliance staff CAN export FERPA-sensitive records
    let compliance_export = engine.authorize_record_action(
        &compliance,
        "export",
        "bio-lab-inventory",
        "biology",
        true,
    ).expect("Evaluation must succeed");
    assert_eq!(compliance_export.decision, PolicyDecision::Allow, "Compliance staff must be permitted to export FERPA records");

    // 5. Cross-department access: Physics student denied access to Biology app
    let cross_dept_access = engine.authorize_record_action(
        &physics_student,
        "read",
        "bio-lab-inventory",
        "biology",
        false,
    ).expect("Evaluation must succeed");
    assert_eq!(cross_dept_access.decision, PolicyDecision::Deny, "Cross-departmental access without central authorization must be denied");
}

#[test]
fn test_admin_console_and_impersonation_authorization() {
    let engine = ScaffoldryPolicyEngine::default_institutional_engine()
        .expect("Failed to initialize Cedar policy engine");

    // 1. Central Admin is PERMITTED to access administrative console
    let admin_access = engine.authorize_institutional_action(
        "jordan.lee@state.edu",
        "central_admin",
        "Central Enterprise IT",
        "access_admin",
        "institutional-console",
    ).expect("Evaluation must succeed");
    assert_eq!(admin_access.decision, PolicyDecision::Allow, "Central admin must be permitted to access admin console");

    // 2. Faculty is FORBIDDEN from accessing administrative console
    let faculty_access = engine.authorize_institutional_action(
        "sarah.connor@state.edu",
        "faculty",
        "Computer Science",
        "access_admin",
        "institutional-console",
    ).expect("Evaluation must succeed");
    assert_eq!(faculty_access.decision, PolicyDecision::Deny, "Faculty must be denied access to admin console");

    // 3. Central Admin is PERMITTED to impersonate institutional users
    let admin_impersonate = engine.authorize_institutional_action(
        "jordan.lee@state.edu",
        "central_admin",
        "Central Enterprise IT",
        "impersonate",
        "sarah.connor@state.edu",
    ).expect("Evaluation must succeed");
    assert_eq!(admin_impersonate.decision, PolicyDecision::Allow, "Central admin must be permitted to impersonate users");

    // 4. Staff/Faculty is FORBIDDEN from impersonating users
    let faculty_impersonate = engine.authorize_institutional_action(
        "sarah.connor@state.edu",
        "faculty",
        "Computer Science",
        "impersonate",
        "marcus.vance@state.edu",
    ).expect("Evaluation must succeed");
    assert_eq!(faculty_impersonate.decision, PolicyDecision::Deny, "Faculty must be forbidden from impersonating other users");
}
