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

/// An error with its causes, since the database client's own text is only "db error".
fn why(e: impl std::error::Error + 'static) -> String {
    let mut text = e.to_string();
    let mut cause = e.source();
    while let Some(c) = cause {
        text.push_str(&format!(": {c}"));
        cause = c.source();
    }
    text
}

/// FNV-1a. It gives the same answer on every Rust version, which `DefaultHasher` does not promise.
fn fnv(bytes: &[u8], mut h: u64) -> u64 {
    for b in bytes {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

/// A template database is replaced this often. Migrations seed times relative to when they ran (the schedule that expires
/// trial copies is first due an hour later), so a template left on the server for longer would start every test with a
/// schedule already due. Half the hour keeps the seeded times at least half an hour ahead.
pub const TEMPLATE_BUCKET_SECS: u64 = 1_800;

fn now_secs() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).expect("the clock").as_secs()
}

/// The name of the migrated template database for the half hour that holds `at_secs`. It changes when a migration changes, or
/// `repository.rs`, which lists the migrations and applies them, so a template is never reused for a schema it was not built
/// for. It also changes every `TEMPLATE_BUCKET_SECS`, so a template is never older than that when it is cloned.
pub fn template_name_at(at_secs: u64) -> String {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut files: Vec<std::path::PathBuf> = std::fs::read_dir(root.join("../scaffoldry-core/migrations"))
        .expect("the migrations")
        .map(|e| e.expect("a migration").path())
        .collect();
    files.sort();
    files.push(root.join("src/repository.rs"));
    let mut h = 0xcbf2_9ce4_8422_2325_u64;
    for f in &files {
        h = fnv(f.file_name().expect("a file name").as_encoded_bytes(), h);
        h = fnv(&std::fs::read(f).expect("a migration file"), h);
    }
    format!("scaffoldry_tpl_{h:016x}_{:x}", at_secs / TEMPLATE_BUCKET_SECS)
}

fn template_name() -> String {
    template_name_at(now_secs())
}

/// Takes the lock that lets one test process build the template while the others wait for it.
const TEMPLATE_LOCK: i64 = 742_200;

/// Makes sure the migrated template exists. One process builds it, under an advisory lock. It is built under another name and
/// renamed when it is whole, so no one clones half of it. Templates for older migrations, and for any half hour before the last, are dropped.
fn ensure_template(name: &str) -> Result<(), String> {
    let (admin, name) = (admin_url(), name.to_string());
    std::thread::spawn(move || -> Result<(), String> {
        let mut c = postgres::Client::connect(&admin, postgres::NoTls).map_err(why)?;
        c.execute("SELECT pg_advisory_lock($1)", &[&TEMPLATE_LOCK]).map_err(why)?;
        let built = (|| -> Result<(), String> {
            let exists = c.query_opt("SELECT 1 FROM pg_database WHERE datname = $1", &[&name]).map_err(why)?.is_some();
            if exists {
                return Ok(());
            }
            let building = format!("{name}_building");
            // One statement at a time: several in one call run as a transaction, and a database cannot be dropped in one.
            c.batch_execute(&format!("DROP DATABASE IF EXISTS {building} WITH (FORCE)")).map_err(why)?;
            c.batch_execute(&format!("CREATE DATABASE {building}")).map_err(why)?;
            let repository = scaffoldry_server::repository::PostgresRepository::connect(Some(&url_of(&admin, &building))).map_err(why)?;
            drop(repository);
            // A database can be copied only when nothing is connected to it. New connections are refused first, so the workers
            // that were just dropped cannot come back.
            c.batch_execute(&format!("ALTER DATABASE {building} ALLOW_CONNECTIONS false")).map_err(why)?;
            c.batch_execute(&format!("SELECT pg_terminate_backend(pid) FROM pg_stat_activity WHERE datname = '{building}'")).map_err(why)?;
            c.batch_execute(&format!("ALTER DATABASE {building} RENAME TO {name}")).map_err(why)?;
            // The half hour before this one is kept: a process that began cloning from it a moment ago may still be at it.
            let previous = template_name_at(now_secs().saturating_sub(TEMPLATE_BUCKET_SECS));
            for old in c
                .query("SELECT datname FROM pg_database WHERE datname LIKE 'scaffoldry\\_tpl\\_%' AND datname <> $1 AND datname <> $2", &[&name, &previous])
                .map_err(why)?
            {
                let old: String = old.get(0);
                c.batch_execute(&format!("DROP DATABASE IF EXISTS {old} WITH (FORCE)")).map_err(why)?;
            }
            Ok(())
        })();
        let _ = c.execute("SELECT pg_advisory_unlock($1)", &[&TEMPLATE_LOCK]);
        built
    })
    .join()
    .map_err(|_| "the template thread panicked".to_string())?
}

/// Creates the database `name` as a copy of the migrated template.
fn clone_template(name: &str) -> Result<(), String> {
    let (admin, name, template) = (admin_url(), name.to_string(), template_name());
    ensure_template(&template)?;
    std::thread::spawn(move || -> Result<(), String> {
        let mut c = postgres::Client::connect(&admin, postgres::NoTls).map_err(why)?;
        c.batch_execute(&format!("CREATE DATABASE {name} TEMPLATE {template} STRATEGY = WAL_LOG")).map_err(why)
    })
    .join()
    .map_err(|_| "the clone thread panicked".to_string())?
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
        // A copy of the migrated template. If the template cannot be made or copied, the test still gets a database: an empty one
        // that connecting migrates, which is slower and the same in the end.
        let mut last_error = String::new();
        let cloned = (0..3).any(|_| match clone_template(&name) {
            Ok(()) => true,
            Err(e) => {
                last_error = e;
                false
            }
        });
        if !cloned {
            eprintln!("warning: no template database ({last_error}), migrating {name} from nothing");
            on_server(format!("CREATE DATABASE {name} STRATEGY = WAL_LOG"));
        }
        let url = url_of(&admin_url(), &name);
        // Connecting applies any migration the database lacks, none for a copy, so a test may touch it before it builds a server.
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
