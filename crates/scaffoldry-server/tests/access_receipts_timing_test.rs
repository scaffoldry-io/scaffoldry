//! Access model phase 8, test 5: the added time of one receipt write. It has a file of its own so it runs alone: the
//! tests of `access_receipts_test` would compete with it for the machine and the number would measure them.

mod common;

use common::TestDb;

// 5. The added time of one receipt write. The median of 1,000 writes is recorded, and if it is above 1 ms the work stops.
#[tokio::test]
async fn a_receipt_write_adds_under_a_millisecond_at_the_median() {
    let db = TestDb::new();
    let state = db.state();
    let mut times = Vec::new();
    for i in 0..1000 {
        let receipt = scaffoldry_server::service::receipts::NewReceipt {
            actor: "timing@state.edu".to_string(),
            acting_as: None,
            action: "read_protected".to_string(),
            object_kind: "app".to_string(),
            object_id: format!("timing-{}", i % 10),
            outcome: "allow",
            grant_ids: vec![1, 2],
            policy_ids: vec!["policy0".to_string()],
            narrowed_by: vec![],
        };
        let t = std::time::Instant::now();
        state.repository.insert_access_receipt(&receipt).expect("a receipt");
        times.push(t.elapsed());
    }
    times.sort();
    let median = times[500];
    eprintln!("receipt write: median {:?}, p95 {:?}, max {:?}", median, times[950], times[999]);
    assert!(median <= std::time::Duration::from_millis(1), "the median of 1,000 receipt writes is {median:?}, above 1 ms: stop and report");
}

