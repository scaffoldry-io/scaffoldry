//! Where a test finds its database.
//!
//! The server reads `DATABASE_URL`. A test of the server cannot change the process environment, because other
//! tests run in the same process at the same moment. Each test names its own database for its own thread instead,
//! and `ServerState::new` and the token helper read it from here. A thread that names none gets `DATABASE_URL`.

use std::cell::RefCell;

const DEFAULT_URL: &str = "postgres://scaffoldry:scaffoldry_dev_password@127.0.0.1:5433/scaffoldry";

thread_local! {
    static THIS_TEST: RefCell<Option<String>> = const { RefCell::new(None) };
}

/// The database this thread uses: the one a test named, else `DATABASE_URL`, else the development database.
pub fn database_url() -> String {
    THIS_TEST
        .with(|u| u.borrow().clone())
        .or_else(|| std::env::var("DATABASE_URL").ok())
        .unwrap_or_else(|| DEFAULT_URL.to_string())
}

/// Names the database for the calling thread, or clears the name with `None`.
pub fn use_database(url: Option<String>) {
    THIS_TEST.with(|u| *u.borrow_mut() = url);
}
