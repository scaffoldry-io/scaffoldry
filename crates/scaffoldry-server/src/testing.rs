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

/// One row of `tests/fixtures/personas.json`: the directory record a test gives a person it names by address.
#[derive(serde::Deserialize)]
struct Persona {
    #[serde(rename = "match")]
    how: String,
    address: String,
    name: String,
    role_title: String,
    affiliation: String,
    department: String,
}

/// The directory record a test fixture gives `eppn`: name, title, affiliation and department. The first row that
/// matches wins. A person with no row is a member with no department. Production code never calls this.
pub fn fixture_persona(eppn: &str) -> (String, String, String, String) {
    #[derive(serde::Deserialize)]
    struct File {
        personas: Vec<Persona>,
    }
    let file: File = serde_json::from_str(include_str!("../tests/fixtures/personas.json")).expect("personas.json");
    file.personas
        .into_iter()
        .find(|p| if p.how == "exact" { eppn == p.address } else { eppn.contains(&p.address) })
        .map(|p| (p.name, p.role_title, p.affiliation, p.department))
        .unwrap_or_else(|| (eppn.to_string(), "Member".to_string(), "member".to_string(), String::new()))
}
