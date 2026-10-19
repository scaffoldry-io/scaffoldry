//! Stateless server phase 6: every write is proven across two instances.
//!
//! See `two_instance/mod.rs`. The cases that need a files directory are in `two_instance_files_test.rs`, a binary of its
//! own, so that setting the directory changes the environment of no other test.

mod common;
mod two_instance;

use common::TestDb;
use two_instance::{cases, gaps, Entry};

// ---------------------------------------------------------------------------------------------------------------------
// The tests
// ---------------------------------------------------------------------------------------------------------------------

#[tokio::test]
async fn every_write_route_and_tool_has_a_case() {
    let db = TestDb::new();
    let app = db.app();
    let entries = cases::entries();
    let (missing, stale) = gaps(&app, &entries);
    assert!(missing.is_empty(), "these write routes or tools have no case in two_instance/cases: {missing:#?}");
    assert!(stale.is_empty(), "these cases name a route or tool that is not there: {stale:#?}");
}

/// The check fails when a write route is added without a case. A route is added to the router in this test and the
/// check names it. Taking it away makes the check pass again.
#[tokio::test]
async fn the_check_fails_for_a_route_added_without_a_case() {
    let db = TestDb::new();
    let app = db.app();
    let entries = cases::entries();
    assert!(gaps(&app, &entries).0.is_empty(), "the router as built has a case for every write route");

    let with_new = app.clone().route("/api/v1/zz-added-in-the-test", axum::routing::post(|| async { "ok" }));
    let (missing, _) = gaps(&with_new, &entries);
    assert_eq!(missing, ["POST /api/v1/zz-added-in-the-test"], "the new route is named");

    // And a tool the registry holds that is not read-only is named the same way, by its entry going missing.
    let mut fewer = cases::entries();
    let (key, _) = fewer.iter().find(|(k, _)| k.starts_with("tool:")).map(|(k, v)| (k.clone(), v)).unwrap();
    fewer.remove(&key);
    assert_eq!(gaps(&app, &fewer).0, [key], "a tool with no case is named");

    // Removed again, the router passes.
    assert!(gaps(&app, &entries).0.is_empty());
}

/// Each case writes through A and reads through B.
#[tokio::test]
async fn every_write_through_one_instance_is_read_through_the_other() {
    let world = two_instance::world().await;
    let (ran, failures) = two_instance::run_cases(&world, |e| if let Entry::Case(c) = e { Some(c) } else { None }).await;
    println!("{ran} cases ran");
    assert!(failures.is_empty(), "{} of {ran} cases failed:\n{}", failures.len(), failures.join("\n"));
}
