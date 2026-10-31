//! A database of its own for each test.
//!
//! `TestDb::new()` makes `scaffoldry_test_<uuid>` on the server `DATABASE_URL` names, and drops it when the value is
//! dropped. It also names the database for the calling thread, so `ServerState::new` and
//! `issue_test_token_and_user` use it. No test changes the process environment.
//!
//! `app()` is a router on a new `ServerState`. `second_app()` is another, on the same database. The pair is how a
//! test shows that a write through one instance is read through the other.
#![allow(dead_code)]

use axum::Router;
use scaffoldry_server::build_app_with_state;
use scaffoldry_server::state::{ServerState, SharedState};
use std::sync::Arc;
use uuid::Uuid;

const DEFAULT_URL: &str = "postgres://scaffoldry:scaffoldry_dev_password@127.0.0.1:5433/scaffoldry";

fn admin_url() -> String {
    std::env::var("DATABASE_URL").unwrap_or_else(|_| DEFAULT_URL.to_string())
}

/// The URL of another database on the same server.
fn url_of(admin: &str, name: &str) -> String {
    match admin.rsplit_once('/') {
        Some((head, tail)) => {
            let query = tail.split_once('?').map(|(_, q)| format!("?{q}")).unwrap_or_default();
            format!("{head}/{name}{query}")
        }
        None => format!("{admin}/{name}"),
    }
}

/// Runs a statement on the server's own database, on a thread of its own: the synchronous client cannot start
/// inside the async test runtime.
fn on_server(sql: String) {
    let url = admin_url();
    std::thread::spawn(move || {
        let mut c = postgres::Client::connect(&url, postgres::NoTls).expect("the test server");
        c.batch_execute(&sql).expect("create or drop a test database");
    })
    .join()
    .expect("the database thread");
}

/// Adds the provisioning credential the SCIM tests use. A test that empties `api_tokens` adds it again with this.
pub fn add_test_scim_credential(c: &mut postgres::Client) {
    let hash = scaffoldry_server::service::identity::hash_token("test-scim-token");
    c.execute(
        "INSERT INTO api_tokens (token_hash, id, kind, eppn, label, created_at, expires_at) \
         VALUES ($1, $2, 'scim', 'scim-tests@scaffoldry.invalid', 'Test provisioning credential', NOW(), NOW() + INTERVAL '365 days') ON CONFLICT DO NOTHING",
        &[&hash, &Uuid::new_v4()],
    )
    .expect("add the test provisioning credential");
}

pub struct TestDb {
    name: String,
    url: String,
}

impl TestDb {
    /// A database with the provisioning credential the SCIM tests use. The credential exists in a test database only: a
    /// server ships with none, and an operator mints one.
    pub fn new() -> TestDb {
        let db = TestDb::bare();
        db.run(add_test_scim_credential);
        db
    }

    /// A migrated, empty database, as a new installation has it.
    pub fn bare() -> TestDb {
        let name = format!("scaffoldry_test_{}", Uuid::new_v4().simple());
        on_server(format!("CREATE DATABASE {name} STRATEGY = WAL_LOG"));
        let url = url_of(&admin_url(), &name);
        // A new database is empty. Connecting migrates it, so a test may touch it before it builds a server.
        scaffoldry_server::repository::PostgresRepository::connect(Some(&url)).expect("migrate the test database");
        scaffoldry_server::testing::use_database(Some(url.clone()));
        TestDb { name, url }
    }

    pub fn url(&self) -> &str {
        &self.url
    }

    /// A new server on this database.
    pub fn state(&self) -> SharedState {
        Arc::new(ServerState::connect(&self.url).expect("a server on the test database"))
    }

    /// A router on a new server.
    pub fn app(&self) -> Router {
        build_app_with_state(self.state()).expect("router")
    }

    /// Another router, on another server, on the same database.
    pub fn second_app(&self) -> Router {
        self.app()
    }

    /// Makes this database unreachable: it refuses new connections and drops the ones open. Its data stays.
    pub fn cut_off(&self) {
        let name = self.name.clone();
        on_server(format!(
            "ALTER DATABASE {name} ALLOW_CONNECTIONS false; \
             SELECT pg_terminate_backend(pid) FROM pg_stat_activity WHERE datname = '{name}' AND pid <> pg_backend_pid();"
        ));
    }

    /// Lets connections in again after `cut_off`.
    pub fn restore(&self) {
        on_server(format!("ALTER DATABASE {} ALLOW_CONNECTIONS true", self.name));
    }

    /// Direct database work on its own thread.
    pub fn run<R: Send + 'static>(&self, f: impl FnOnce(&mut postgres::Client) -> R + Send + 'static) -> R {
        let url = self.url.clone();
        std::thread::spawn(move || {
            let mut c = postgres::Client::connect(&url, postgres::NoTls).expect("the test database");
            f(&mut c)
        })
        .join()
        .expect("the database thread")
    }
}

impl Drop for TestDb {
    fn drop(&mut self) {
        scaffoldry_server::testing::use_database(None);
        // For looking at a database after a long seed: KEEP_TEST_DB=1 leaves it, and prints its name.
        if std::env::var_os("KEEP_TEST_DB").is_some() {
            eprintln!("KEEP_TEST_DB: left the database {}", self.name);
            return;
        }
        // FORCE ends the connections the servers of this test still hold.
        let name = self.name.clone();
        let _ = std::thread::spawn(move || {
            let url = admin_url();
            if let Ok(mut c) = postgres::Client::connect(&url, postgres::NoTls) {
                let _ = c.batch_execute(&format!("DROP DATABASE IF EXISTS {name} WITH (FORCE)"));
            }
        })
        .join();
    }
}
