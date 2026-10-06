//! RFC 4180 CSV Serialization and Record Mutation Integration Test

use scaffoldry_engine::{
    batch_delete_records, duplicate_record, export_records_to_csv, parse_csv_to_records,
};
use serde_json::json;

#[test]
fn test_csv_export_and_escape() {
    let records = [
        json!({
            "id": "REC-1",
            "name": "Quantum Materials, Phase 1",
            "notes": "Line 1\nLine 2 with \"quotes\"",
            "budget": 500000.0,
            "approved": true
        }),
        json!({
            "id": "REC-2",
            "name": "Biomedical Sensor, Advanced",
            "notes": "Standard single line",
            "budget": 250000.0,
            "approved": false
        }),
    ];

    let headers = ["id", "name", "notes", "budget", "approved"];
    let csv = export_records_to_csv(&headers, &records);

    assert!(csv.starts_with("id,name,notes,budget,approved\n"));
    assert!(csv.contains("\"Quantum Materials, Phase 1\""));
    assert!(csv.contains("\"Line 1\nLine 2 with \"\"quotes\"\"\""));
    assert!(csv.contains("500000"));
}

#[test]
fn test_csv_parse_roundtrip() {
    let raw_csv = "id,name,budget,status\nREC-1,\"Quantum, Inc.\",750000,Approved\nREC-2,Neural Net,300000,Pending\n";
    let records = parse_csv_to_records(raw_csv).expect("Parse CSV");

    assert_eq!(records.len(), 2);
    assert_eq!(records[0]["id"], "REC-1");
    assert_eq!(records[0]["name"], "Quantum, Inc.");
    assert_eq!(records[0]["budget"], 750000.0);
    assert_eq!(records[0]["status"], "Approved");

    assert_eq!(records[1]["id"], "REC-2");
    assert_eq!(records[1]["name"], "Neural Net");
    assert_eq!(records[1]["budget"], 300000.0);
    assert_eq!(records[1]["status"], "Pending");
}

#[test]
fn test_record_duplication_and_batch_delete() {
    let record = json!({
        "id": "REC-ORIGINAL",
        "title": "Clean Energy Storage Proposal",
        "amount": 1200000.0
    });

    let copy = duplicate_record(&record, "title", "REC-NEW-99");
    assert_eq!(copy["id"], "REC-NEW-99");
    assert_eq!(copy["title"], "Clean Energy Storage Proposal (Copy)");
    assert_eq!(copy["amount"], 1200000.0);

    let mut records = vec![
        json!({ "id": "R1", "val": 10 }),
        json!({ "id": "R2", "val": 20 }),
        json!({ "id": "R3", "val": 30 }),
    ];

    batch_delete_records(&mut records, &["R1", "R3"]);
    assert_eq!(records.len(), 1);
    assert_eq!(records[0]["id"], "R2");
}
