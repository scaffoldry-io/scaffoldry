use scaffoldry_core::standards::{
    ceds::{CedsDomain, CedsElement},
    eduperson::{EduPersonAffiliation, EduPersonIdentity},
    herm::{HermDomain, HermCapability},
};

#[test]
fn test_ceds_element_crosswalk_mapping() {
    let element = CedsElement::from_code("000127").expect("CEDS GradePointAverageCumulative must exist");
    assert_eq!(element.domain, CedsDomain::PostsecondaryStudent);
    assert_eq!(element.name, "GradePointAverageCumulative");
    assert!(element.is_ferpa_sensitive());
}

#[test]
fn test_eduperson_scoped_affiliation_parsing() {
    let identity = EduPersonIdentity::parse("johann@virginia.edu", vec!["faculty@virginia.edu", "employee@virginia.edu"]).expect("Valid eduPerson assertion");
    assert_eq!(identity.eppn, "johann@virginia.edu");
    assert_eq!(identity.realm, "virginia.edu");
    assert!(identity.has_affiliation(EduPersonAffiliation::Faculty));
    assert!(!identity.has_affiliation(EduPersonAffiliation::Student));
}

#[test]
fn test_herm_capability_crosswalk() {
    let cap = HermCapability::resolve("2.2.3").expect("HERM capability must exist");
    assert_eq!(cap.domain, HermDomain::Research);
    assert_eq!(cap.name, "Research Grant Administration");
}
