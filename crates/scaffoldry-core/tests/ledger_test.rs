use scaffoldry_core::ledger::{
    verify_ledger_chain, DecisionType, LedgerEntry, LedgerError, LedgerHashInput,
    NewLedgerEntryParams, GENESIS_PREVIOUS_HASH,
};
use serde_json::json;

#[test]
fn test_cryptographic_ledger_chain_and_tamper_detection() {
    let payload_0 = json!({
        "app": "biology-lab-inventory",
        "action": "publish",
        "domain": "inventory.biology.state.edu"
    });

    let entry_0 = LedgerEntry::new(NewLedgerEntryParams {
        sequence: 0,
        timestamp_iso: "2026-10-04T12:00:00Z".to_string(),
        previous_hash: GENESIS_PREVIOUS_HASH.to_string(),
        principal: "prof.curie@science.state.edu".to_string(),
        organization_code: "DIV-SCIENCES".to_string(),
        app_slug: Some("biology-lab-inventory".to_string()),
        decision_type: DecisionType::AppPublished,
        oscal_control_id: "CM-03".to_string(),
        rationale: "Approved publication of biology research inventory system".to_string(),
        payload: &payload_0,
    });

    let payload_1 = json!({
        "rule_id": "auto-physics-honors-admit",
        "guard": "policy-ferpa-34cfr99"
    });

    let entry_1 = LedgerEntry::new(NewLedgerEntryParams {
        sequence: 1,
        timestamp_iso: "2026-10-04T12:05:00Z".to_string(),
        previous_hash: entry_0.entry_hash.clone(),
        principal: "dr.watson@science.state.edu".to_string(),
        organization_code: "DIV-COMPLIANCE".to_string(),
        app_slug: Some("physics-admissions-review".to_string()),
        decision_type: DecisionType::WorkflowRuleApproved,
        oscal_control_id: "AC-03".to_string(),
        rationale: "FERPA compliance approval for graduate fellowship trigger".to_string(),
        payload: &payload_1,
    });

    let payload_2 = json!({
        "dataset": "research_equipment",
        "granted_to": "student.smith@science.state.edu"
    });

    let entry_2 = LedgerEntry::new(NewLedgerEntryParams {
        sequence: 2,
        timestamp_iso: "2026-10-04T12:10:00Z".to_string(),
        previous_hash: entry_1.entry_hash.clone(),
        principal: "prof.curie@science.state.edu".to_string(),
        organization_code: "DIV-SCIENCES".to_string(),
        app_slug: None,
        decision_type: DecisionType::DatasetAccessShared,
        oscal_control_id: "AC-03".to_string(),
        rationale: "Graduate research access granted for lab equipment tracking".to_string(),
        payload: &payload_2,
    });

    let mut chain = vec![entry_0, entry_1, entry_2];

    // Valid chain verification
    assert!(verify_ledger_chain(&chain).is_ok());

    // Tamper detection: modify rationale in entry 1
    chain[1].rationale = "Unauthorized tampering with rationale".to_string();
    let tamper_result = verify_ledger_chain(&chain);
    assert!(tamper_result.is_err());
    match tamper_result.unwrap_err() {
        LedgerError::HashMismatch { sequence, .. } => assert_eq!(sequence, 1),
        err => panic!("Unexpected error: {:?}", err),
    }

    // Tamper detection: alter previous_hash pointer
    chain[1].rationale = "FERPA compliance approval for graduate fellowship trigger".to_string();
    chain[1].entry_hash = LedgerEntry::compute_entry_hash(&LedgerHashInput {
        sequence: chain[1].sequence,
        timestamp_iso: &chain[1].timestamp_iso,
        previous_hash: &chain[1].previous_hash,
        principal: &chain[1].principal,
        organization_code: &chain[1].organization_code,
        app_slug: chain[1].app_slug.as_deref(),
        decision_type: &chain[1].decision_type,
        oscal_control_id: &chain[1].oscal_control_id,
        rationale: &chain[1].rationale,
        payload_hash: &chain[1].payload_hash,
    });
    chain[2].previous_hash = "ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff".to_string();
    chain[2].entry_hash = LedgerEntry::compute_entry_hash(&LedgerHashInput {
        sequence: chain[2].sequence,
        timestamp_iso: &chain[2].timestamp_iso,
        previous_hash: &chain[2].previous_hash,
        principal: &chain[2].principal,
        organization_code: &chain[2].organization_code,
        app_slug: chain[2].app_slug.as_deref(),
        decision_type: &chain[2].decision_type,
        oscal_control_id: &chain[2].oscal_control_id,
        rationale: &chain[2].rationale,
        payload_hash: &chain[2].payload_hash,
    });
    let broken_chain_result = verify_ledger_chain(&chain);
    assert!(broken_chain_result.is_err());
    match broken_chain_result.unwrap_err() {
        LedgerError::BrokenChain { sequence, .. } => assert_eq!(sequence, 2),
        err => panic!("Unexpected error: {:?}", err),
    }
}
