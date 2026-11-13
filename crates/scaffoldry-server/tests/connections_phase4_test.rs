//! Connections phase 4: the HTTP connector.
//!
//! A connection reads a JSON or JSON Lines web service, page by page. The address is https only and on the outbound
//! allowlist, with no redirects, 5 MB for each page, at most 100 pages, and an optional bearer token from the secrets
//! store. Every page is read and checked before a record is written.

mod common;
mod connections_pg;

use axum::http::StatusCode;
use connections_pg::*;
use serde_json::{json, Value};
use std::sync::Arc;

const TOKEN: &str = "tok-4f9c2e71-b6d0-4d39-8a0e-7c1d5a93e2b8";

/// A service of three pages of 40, each naming the next by its full address.
fn three_pages(port: std::sync::Arc<std::sync::atomic::AtomicU16>) -> Handler {
    Arc::new(move |got: &Got| {
        let n: usize = got.path.split("page=").nth(1).and_then(|p| p.parse().ok()).unwrap_or(1);
        let next = (n < 3).then(|| format!("https://{DB_HOST}:{}/api/programs?page={}", port.load(std::sync::atomic::Ordering::SeqCst), n + 1));
        Reply::json(&page((n - 1) * 40 + 1, 40, next))
    })
}

async fn server(w: &World, make: impl FnOnce(Arc<std::sync::atomic::AtomicU16>) -> Handler) -> (u16, Arc<std::sync::Mutex<Vec<Got>>>) {
    let port = Arc::new(std::sync::atomic::AtomicU16::new(0));
    let (p, got) = https_server(DB_HOST, make(port.clone())).await;
    port.store(p, std::sync::atomic::Ordering::SeqCst);
    w.allow(&[p]);
    (p, got)
}

// 1. A paged JSON source syncs all pages. The `next` address off the allowlist stops the run.
#[tokio::test(flavor = "multi_thread")]
async fn a_paged_source_syncs_every_page_and_a_next_address_off_the_list_stops_the_run() {
    let w = world().await;
    let (port, got) = server(&w, three_pages).await;
    let c = connect_ok(&w, http_body(port, json!({}), None)).await;
    let r = run(&w, &c).await;
    assert_eq!(r["state"], "done", "{r}");
    assert_eq!(r["added"], 120, "{r}");
    assert_eq!(live(&w, &c.app), 120);
    assert_eq!(got.lock().unwrap().len(), 3, "one request for each page, and the rows are kept for the second pass");
    assert_eq!(run(&w, &c).await["state"], "unchanged");

    // The second page's address is on a host that is not on the list. Nothing is written.
    let (port2, _) = server(&w, |_| Arc::new(|_: &Got| Reply::json(&page(1, 5, Some("https://elsewhere.example.net/api/programs?page=2".to_string()))))).await;
    let off = connect_ok(&w, http_body(port2, json!({}), None)).await;
    let r = run(&w, &off).await;
    assert_eq!(r["state"], "failed", "{r}");
    assert!(r["error"].as_str().unwrap().contains("not on the outbound allowlist"), "{r}");
    assert_eq!(live(&w, &off.app), 0, "the first page was read and checked, and nothing was written");
}

// 2. A redirect is a failure, and is not followed.
#[tokio::test(flavor = "multi_thread")]
async fn a_redirect_is_a_failure() {
    let w = world().await;
    let (port, got) = server(&w, |p| Arc::new(move |g: &Got| {
        if g.path.contains("moved") {
            return Reply::json(&page(1, 3, None));
        }
        Reply { status: 302, headers: vec![("Location".into(), format!("https://{DB_HOST}:{}/api/moved", p.load(std::sync::atomic::Ordering::SeqCst)))], body: Vec::new() }
    })).await;
    let c = connect_ok(&w, http_body(port, json!({}), None)).await;
    let r = run(&w, &c).await;
    assert_eq!(r["state"], "failed", "{r}");
    assert!(r["error"].as_str().unwrap().to_lowercase().contains("redirect"), "{r}");
    assert_eq!(got.lock().unwrap().len(), 1, "the redirect was not followed");
    assert_eq!(live(&w, &c.app), 0);
}

// 3. A response over the limit is a failure.
#[tokio::test(flavor = "multi_thread")]
async fn a_response_over_the_limit_is_a_failure() {
    let w = world().await;
    let (port, _) = server(&w, |_| Arc::new(|_: &Got| Reply { status: 200, headers: vec![("Content-Type".into(), "application/json".into())], body: format!("{{\"data\":[{{\"code\":\"P1\",\"name\":\"{}\",\"credits\":3}}]}}", "x".repeat(6 * 1024 * 1024)).into_bytes() })).await;
    let c = connect_ok(&w, http_body(port, json!({}), None)).await;
    let r = run(&w, &c).await;
    assert_eq!(r["state"], "failed", "{r}");
    let said = r["error"].as_str().unwrap().to_lowercase();
    assert!(said.contains("larger") || said.contains("too large") || said.contains("limit"), "{r}");
    assert_eq!(live(&w, &c.app), 0);
}

// 4. The bearer token is used and never logged.
#[tokio::test(flavor = "multi_thread")]
async fn the_bearer_token_is_used_and_never_logged() {
    let w = world().await;
    w.put_secret("api_token", TOKEN);
    let (port, got) = server(&w, |_| Arc::new(|g: &Got| {
        if g.header("authorization").as_deref() != Some(&format!("Bearer {TOKEN}")) {
            return Reply { status: 401, headers: vec![], body: format!("Bad token. You sent {}", g.header("authorization").unwrap_or_default()).into_bytes() };
        }
        Reply::json(&page(1, 10, None))
    })).await;
    let c = connect_ok(&w, http_body(port, json!({}), Some("api_token"))).await;
    let r = run(&w, &c).await;
    assert_eq!(r["state"], "done", "{r}");
    assert!(got.lock().unwrap().iter().all(|g| g.header("authorization").as_deref() == Some(&format!("Bearer {TOKEN}"))), "every request carried the token");

    let (_, listed) = send(&w.app, "GET", "/api/v1/admin/connections", ADMIN, None).await;
    let (_, runs) = send(&w.app, "GET", &format!("/api/v1/admin/connections/{}/runs", c.id), ADMIN, None).await;
    let ledger: String = w.db.run(|cl| cl.query_one("SELECT COALESCE(string_agg(rationale || payload::text, ' '), '') FROM governance_ledger", &[]).unwrap().get(0));
    for shown in [listed.to_string(), runs.to_string(), ledger] {
        assert!(!shown.contains(TOKEN), "the token is not shown");
    }

    // A service that answers 401 and repeats the token it was sent does not put it in the run's error.
    w.put_secret("api_token", "a-different-token-value");
    w.put_secret("wrong_token", TOKEN);
    let wrong = connect_ok(&w, http_body(port, json!({}), Some("wrong_token"))).await;
    let ok = run(&w, &wrong).await;
    assert_eq!(ok["state"], "done");
    let bad = connect_ok(&w, http_body(port, json!({ "url": format!("https://{DB_HOST}:{port}/api/programs?x=1") }), Some("api_token"))).await;
    let r = run(&w, &bad).await;
    assert_eq!(r["state"], "failed", "{r}");
    assert!(r["error"].as_str().unwrap().contains("401"), "{r}");
    assert!(!r.to_string().contains("a-different-token-value"), "the token is scrubbed from the error: {r}");
}

// 5. The address is https, the format and paths are checked, and at most 100 pages are read.
#[tokio::test(flavor = "multi_thread")]
async fn the_address_is_https_and_a_run_reads_at_most_100_pages() {
    let w = world().await;
    let (port, got) = server(&w, |p| Arc::new(move |g: &Got| {
        let n: usize = g.path.split("page=").nth(1).and_then(|x| x.parse().ok()).unwrap_or(1);
        Reply::json(&page(n, 1, Some(format!("https://{DB_HOST}:{}/api/programs?page={}", p.load(std::sync::atomic::Ordering::SeqCst), n + 1))))
    })).await;
    let plain = http_body(port, json!({ "url": format!("http://{DB_HOST}:{port}/api/programs") }), None);
    assert_eq!(connect(&w, plain).await.0, StatusCode::BAD_REQUEST, "http is refused");
    assert_eq!(connect(&w, http_body(port, json!({ "format": "jsonl" }), None)).await.0, StatusCode::BAD_REQUEST, "a next path needs json");
    assert_eq!(connect(&w, http_body(port, json!({ "format": "xml" }), None)).await.0, StatusCode::BAD_REQUEST);
    assert_eq!(connect(&w, http_body(port, json!({ "url": "" }), None)).await.0, StatusCode::BAD_REQUEST);

    let c = connect_ok(&w, http_body(port, json!({}), None)).await;
    let r = run(&w, &c).await;
    assert_eq!(r["state"], "failed", "{r}");
    assert!(r["error"].as_str().unwrap().contains("100 pages"), "{r}");
    assert_eq!(got.lock().unwrap().len(), 100, "it stopped at the hundredth page");
    assert_eq!(live(&w, &c.app), 0);
}

// A JSON Lines source has no pages, and a service that answers with something else is a clear failure.
#[tokio::test(flavor = "multi_thread")]
async fn a_json_lines_source_syncs_and_a_wrong_shape_is_a_clear_failure() {
    let w = world().await;
    let lines: String = (1..=5).map(|i| format!("{{\"code\":\"P{i:06}\",\"name\":\"Program {i}\",\"credits\":3}}\n")).collect();
    let (port, _) = server(&w, |_| Arc::new(move |_: &Got| Reply { status: 200, headers: vec![("Content-Type".into(), "application/x-ndjson".into())], body: lines.clone().into_bytes() })).await;
    let c = connect_ok(&w, http_body(port, json!({ "format": "jsonl", "next": null, "path": null }), None)).await;
    let r = run(&w, &c).await;
    assert_eq!((r["state"].as_str(), r["added"].as_i64()), (Some("done"), Some(5)), "{r}");

    let (port2, _) = server(&w, |_| Arc::new(|_: &Got| Reply::json(&json!({ "rows": [] })))).await;
    let wrong = connect_ok(&w, http_body(port2, json!({}), None)).await;
    let r = run(&w, &wrong).await;
    assert_eq!(r["state"], "failed", "{r}");
    assert!(r["error"].as_str().unwrap().contains("data"), "the error names the path it looked for: {r}");
    let _: Value = r;
}
