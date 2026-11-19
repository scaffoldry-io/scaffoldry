use scaffoldry_core::catalog;

#[test]
fn test_known_controls_have_a_title_and_statement() {
    for id in ["ac-2", "ac-2.1", "sc-7"] {
        let c = catalog::control(id).unwrap_or_else(|| panic!("{id} missing"));
        assert!(!c.title.is_empty(), "{id} title");
        assert!(!c.statement.is_empty(), "{id} statement");
    }
    assert_eq!(catalog::title("ac-2"), Some("Account Management"));
    assert_eq!(catalog::control("ac-2.1").unwrap().parent_id.as_deref(), Some("ac-2"));
    assert_eq!(catalog::control("AC-02").unwrap().id, "ac-2");
    assert!(catalog::control("zz-99").is_none());
}

#[test]
fn test_both_catalogs_are_loaded() {
    let all = catalog::search("", None, None);
    assert!(all.iter().any(|c| c.catalog == "sp800-53r5"));
    assert!(all.iter().any(|c| c.catalog == "sp800-171r3"));
    let only = catalog::search("", None, Some("sp800-171r3"));
    assert!(!only.is_empty() && only.iter().all(|c| c.catalog == "sp800-171r3"));
}

#[test]
fn test_no_statement_has_a_template_marker() {
    for c in catalog::search("", None, None) {
        assert!(!c.statement.contains("{{"), "{}", c.id);
    }
}

#[test]
fn test_baselines_nest_and_exist_in_the_catalog() {
    let low = catalog::baseline("low").expect("low");
    let moderate = catalog::baseline("moderate").expect("moderate");
    let high = catalog::baseline("high").expect("high");
    assert!(low.iter().all(|id| moderate.contains(id)));
    assert!(moderate.iter().all(|id| high.contains(id)));
    for id in high {
        let c = catalog::control(id).unwrap_or_else(|| panic!("{id} not in catalog"));
        assert_eq!(c.catalog, "sp800-53r5");
    }
    assert!(catalog::baseline("MODERATE").is_some());
    assert!(catalog::baseline("nope").is_none());
}

#[test]
fn test_no_withdrawn_control_is_present() {
    // Withdrawn in Rev 5. Their status property is "withdrawn" in the NIST source.
    for id in ["ac-13", "ac-15", "sc-9"] {
        assert!(catalog::control(id).is_none(), "{id} is withdrawn");
    }
}

#[test]
fn test_normalize_table() {
    let table = [
        ("AC-02", "ac-2"),
        ("ac-03", "ac-3"),
        ("SC-07", "sc-7"),
        ("AC-2(1)", "ac-2.1"),
        ("ac-2.1", "ac-2.1"),
        ("AC-02(01)", "ac-2.1"),
        (" ac-2 ", "ac-2"),
        ("SP_800_171_03.01.01", "sp_800_171_03.01.01"),
        ("nonsense", "nonsense"),
    ];
    for (raw, want) in table {
        assert_eq!(catalog::normalize(raw), want, "{raw}");
    }
}

#[test]
fn test_search_finds_account_management_early() {
    let hits = catalog::search("account", None, None);
    let first_five: Vec<&str> = hits.iter().take(5).map(|c| c.id.as_str()).collect();
    assert!(first_five.contains(&"ac-2"), "{first_five:?}");
}

#[test]
fn test_search_filters_by_family() {
    let hits = catalog::search("", Some("ac"), Some("sp800-53r5"));
    assert!(!hits.is_empty() && hits.iter().all(|c| c.family_id == "ac"));
    let by_title = catalog::search("", Some("Access Control"), Some("sp800-53r5"));
    assert_eq!(hits.len(), by_title.len());
}

#[test]
fn test_each_catalog_has_its_canonical_source_url_from_provenance() {
    let u53 = scaffoldry_core::catalog::source_url("sp800-53r5").expect("800-53 url");
    assert!(u53.starts_with("https://raw.githubusercontent.com/usnistgov/oscal-content/f0de6d8d"), "{u53}");
    assert!(u53.ends_with("NIST_SP-800-53_rev5_catalog.json"));
    let u171 = scaffoldry_core::catalog::source_url("sp800-171r3").expect("800-171 url");
    assert!(u171.ends_with("NIST_SP800-171_rev3_catalog.json"), "{u171}");
    assert!(scaffoldry_core::catalog::source_url("nope").is_none());
}
