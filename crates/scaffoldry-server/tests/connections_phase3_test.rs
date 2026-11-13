//! Connections phase 3: the PostgreSQL connector.
//!
//! A connection runs one `SELECT` against another database, over TLS, in a read-only transaction, and syncs its rows
//! into a dataset. The password comes from the secrets store. The host must be on the outbound allowlist.

mod common;
mod connections_pg;

use axum::http::StatusCode;
use connections_pg::*;
use serde_json::json;

// 1. A configured query syncs rows into a dataset. The password comes from the secrets store, and appears nowhere.
#[tokio::test(flavor = "multi_thread")]
async fn a_configured_query_syncs_rows_and_the_password_stays_in_the_store() {
    let w = world().await;
    seed(&w, 50);
    let c = connect_ok(&w, body(&w, w.good_port, "verify-full", ALL, json!({}))).await;

    let first = run(&w, &c).await;
    assert_eq!(first["state"], "done", "{first}");
    assert_eq!((first["added"].as_i64(), first["changed"].as_i64()), (Some(50), Some(0)), "{first}");
    assert_eq!(live(&w, &c.app), 50);

    // The same rows again are `unchanged`, and a changed row is one change.
    assert_eq!(run(&w, &c).await["state"], "unchanged");
    w.src.run(|cl| cl.execute("UPDATE programs SET name = 'Renamed' WHERE code = 'P000001'", &[]).unwrap());
    let third = run(&w, &c).await;
    assert_eq!((third["state"].as_str(), third["changed"].as_i64(), third["unchanged"].as_i64()), (Some("done"), Some(1), Some(49)), "{third}");

    // The password is named, never shown: not in the connection, a run, or the ledger.
    let (_, listed) = send(&w.app, "GET", "/api/v1/admin/connections", ADMIN, None).await;
    let (_, runs) = send(&w.app, "GET", &format!("/api/v1/admin/connections/{}/runs", c.id), ADMIN, None).await;
    let ledger: String = w.db.run(|cl| cl.query_one("SELECT COALESCE(string_agg(rationale || payload::text, ' '), '') FROM governance_ledger", &[]).unwrap().get(0));
    for shown in [listed.to_string(), runs.to_string(), ledger] {
        assert!(!shown.contains(&w.password), "the password is not shown");
    }
    assert!(listed.to_string().contains(SECRET), "the secret is named");
}

// 2. A statement that would write is refused by the check and by the read-only transaction.
#[tokio::test(flavor = "multi_thread")]
async fn a_statement_that_would_write_is_refused_by_the_check_and_by_the_transaction() {
    let w = world().await;
    seed(&w, 3);
    // The check refuses it when the connection is made.
    for sql in [
        "DELETE FROM programs",
        "UPDATE programs SET name = 'x'",
        "INSERT INTO audit VALUES ('x')",
        "SELECT code, name, credits FROM programs; DROP TABLE programs",
        "SELECT code, name, credits FROM programs;",
        "  ",
    ] {
        let (s, b, _) = connect(&w, body(&w, w.good_port, "verify-full", sql, json!({}))).await;
        assert_eq!(s, StatusCode::BAD_REQUEST, "{sql:?}: {b}");
    }
    // A semicolon in a string, and a comment, are not statements. A WITH is allowed.
    for sql in ["SELECT code, name, credits FROM programs WHERE name <> ';' -- note; here", "WITH p AS (SELECT * FROM programs) SELECT code, name, credits FROM p"] {
        let (s, b, _) = connect(&w, body(&w, w.good_port, "verify-full", sql, json!({}))).await;
        assert_eq!(s, StatusCode::CREATED, "{sql:?}: {b}");
    }

    // A SELECT that calls a function that writes passes the check. The read-only transaction refuses it.
    let c = connect_ok(&w, body(&w, w.good_port, "verify-full", "SELECT touch() AS code, 'x' AS name, 1 AS credits", json!({}))).await;
    let r = run(&w, &c).await;
    assert_eq!(r["state"], "failed", "{r}");
    assert!(r["error"].as_str().unwrap().to_lowercase().contains("read-only"), "{r}");
    let touched: i64 = w.src.run(|cl| cl.query_one("SELECT COUNT(*) FROM audit", &[]).unwrap().get(0));
    assert_eq!(touched, 0, "nothing was written to the source");
    assert_eq!(live(&w, &c.app), 0);
}

// 3. A host that is not on the allowlist is refused. A private address is refused without a `cidr` entry.
#[tokio::test(flavor = "multi_thread")]
async fn a_host_off_the_allowlist_and_a_private_address_are_refused() {
    let w = world().await;
    seed(&w, 3);
    let c = connect_ok(&w, body(&w, w.good_port, "verify-full", ALL, json!({}))).await;

    w.allow_nothing();
    let r = run(&w, &c).await;
    assert_eq!(r["state"], "failed", "{r}");
    assert!(r["error"].as_str().unwrap().contains("not on the outbound allowlist"), "{r}");

    // The name is listed, and it resolves to loopback, which only a network entry can allow.
    w.allow_host_only(&[w.good_port]);
    let r = run(&w, &c).await;
    assert_eq!(r["state"], "failed", "{r}");
    let said = r["error"].as_str().unwrap();
    assert!(said.contains("loopback") && said.contains("no cidr entry allows"), "{r}");
    assert_eq!(live(&w, &c.app), 0);

    // With the network named on purpose, it goes through.
    w.allow(&[w.good_port]);
    assert_eq!(run(&w, &c).await["state"], "done");
}

// 4. `verify-full` fails against a certificate for another name. `require` encrypts and checks the chain, not the name.
#[tokio::test(flavor = "multi_thread")]
async fn verify_full_fails_against_a_certificate_for_another_name() {
    let w = world().await;
    seed(&w, 3);
    let wrong = connect_ok(&w, body(&w, w.other_port, "verify-full", ALL, json!({}))).await;
    let r = run(&w, &wrong).await;
    assert_eq!(r["state"], "failed", "{r}");
    let said = r["error"].as_str().unwrap().to_lowercase();
    assert!(said.contains("certificate"), "the error says the certificate: {r}");
    assert_eq!(live(&w, &wrong.app), 0);

    // The same server, with `require`: the chain is trusted and the name is not checked.
    let lax = connect_ok(&w, body(&w, w.other_port, "require", ALL, json!({}))).await;
    assert_eq!(run(&w, &lax).await["state"], "done");
    // And the right name passes `verify-full`.
    let right = connect_ok(&w, body(&w, w.good_port, "verify-full", ALL, json!({}))).await;
    assert_eq!(run(&w, &right).await["state"], "done");
}

// 5. A query that runs past the timeout fails the run with a clear error and changes nothing.
#[tokio::test(flavor = "multi_thread")]
async fn a_query_past_the_timeout_fails_the_run_and_changes_nothing() {
    let w = world().await;
    let c = connect_ok(&w, body(&w, w.good_port, "verify-full", "SELECT pg_sleep(5)::text AS code, 'x' AS name, 1 AS credits", json!({ "timeout_seconds": 1 }))).await;
    let started = std::time::Instant::now();
    let r = run(&w, &c).await;
    assert_eq!(r["state"], "failed", "{r}");
    assert!(r["error"].as_str().unwrap().contains("time limit of 1 second"), "{r}");
    assert!(started.elapsed() < std::time::Duration::from_secs(4), "it stopped at the limit, not at the end of the sleep");
    assert_eq!(live(&w, &c.app), 0);
}

// A connection of this kind needs its parts, and a secret that exists.
#[tokio::test(flavor = "multi_thread")]
async fn a_connection_needs_its_parts_and_an_existing_secret() {
    let w = world().await;
    let mut no_secret = body(&w, w.good_port, "verify-full", ALL, json!({}));
    no_secret["secret"] = json!("no_such_secret");
    assert_eq!(connect(&w, no_secret).await.0, StatusCode::BAD_REQUEST);
    let mut none = body(&w, w.good_port, "verify-full", ALL, json!({}));
    none.as_object_mut().unwrap().remove("secret");
    assert_eq!(connect(&w, none).await.0, StatusCode::BAD_REQUEST);
    assert_eq!(connect(&w, body(&w, w.good_port, "disable", ALL, json!({}))).await.0, StatusCode::BAD_REQUEST, "TLS cannot be turned off");
    assert_eq!(connect(&w, body(&w, w.good_port, "verify-full", ALL, json!({ "host": "" }))).await.0, StatusCode::BAD_REQUEST);
}
