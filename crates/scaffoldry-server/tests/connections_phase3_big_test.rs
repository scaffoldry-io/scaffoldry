//! Connections phase 3: a source of 200,000 rows syncs in batches, and the process's memory stays flat.

mod common;
mod connections_pg;

use connections_pg::*;
use serde_json::json;

/// The most memory this process has held, in kilobytes, from the kernel.
fn peak_kb() -> u64 {
    std::fs::read_to_string("/proc/self/status").ok().and_then(|s| s.lines().find(|l| l.starts_with("VmHWM:")).and_then(|l| l.split_whitespace().nth(1)?.parse().ok())).unwrap_or(0)
}

#[tokio::test(flavor = "multi_thread")]
async fn a_200000_row_source_syncs_in_batches_with_bounded_memory() {
    let w = world().await;
    seed(&w, 200_000);
    let c = connect_ok(&w, body(&w, w.good_port, "verify-full", ALL, json!({}))).await;
    let before = peak_kb();
    let started = std::time::Instant::now();
    let r = run(&w, &c).await;
    let (after, took) = (peak_kb(), started.elapsed());
    eprintln!("CONNECTIONS PHASE 3: 200,000 rows synced in {took:?}. Peak memory {} MB before the run and {} MB after, a rise of {} MB.", before / 1024, after / 1024, (after.saturating_sub(before)) / 1024);
    assert_eq!(r["state"], "done", "{r}");
    assert_eq!(r["added"], 200_000, "{r}");
    assert_eq!(live(&w, &c.app), 200_000);
    // The rows are never all held at once: the batches are 5,000 read and 1,000 written, and only the keys are kept.
    assert!((after.saturating_sub(before)) / 1024 < 300, "the run raised the peak by {} MB", (after.saturating_sub(before)) / 1024);
}
