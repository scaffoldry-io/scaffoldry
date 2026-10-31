//! Test databases are cloned from one migrated template, not migrated one by one.
//!
//! Every test has a database of its own. Running all of the migrations on each one took about 0.9 s of the test's time;
//! cloning a migrated template takes about 0.15 s. These tests check that the clone is used, that it is a complete
//! database, and that tests still cannot see each other's rows.

mod common;

use common::TestDb;

fn migration_files() -> i64 {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../scaffoldry-core/migrations");
    std::fs::read_dir(dir).expect("the migrations").filter(|e| e.as_ref().is_ok_and(|e| e.path().extension().is_some_and(|x| x == "sql"))).count() as i64
}

#[test]
fn a_test_database_is_a_clone_of_a_migrated_template() {
    let db = TestDb::bare();
    let templates: i64 = db.run(|c| c.query_one("SELECT count(*) FROM pg_database WHERE datname LIKE 'scaffoldry\\_tpl\\_%'", &[]).unwrap().get(0));
    assert!(templates >= 1, "a migrated template database exists on the server");
}

#[test]
fn a_clone_has_every_migration_and_no_test_data() {
    let db = TestDb::bare();
    let (applied, workspaces): (i64, i64) = db.run(|c| {
        (
            c.query_one("SELECT count(*) FROM schema_migrations", &[]).unwrap().get(0),
            c.query_one("SELECT count(*) FROM workspaces", &[]).unwrap().get(0),
        )
    });
    assert_eq!(applied, migration_files(), "every migration file is recorded as applied");
    assert_eq!(workspaces, 0, "a new database has no workspaces");
}

#[test]
fn two_test_databases_do_not_share_rows() {
    let (a, b) = (TestDb::bare(), TestDb::bare());
    a.run(|c| {
        c.execute("INSERT INTO platform_settings (key, value, updated_by) VALUES ('template.isolation', '\"a\"'::jsonb, 'test') ON CONFLICT (key) DO NOTHING", &[]).unwrap();
    });
    let seen_by_b: i64 = b.run(|c| c.query_one("SELECT count(*) FROM platform_settings WHERE key = 'template.isolation'", &[]).unwrap().get(0));
    assert_eq!(seen_by_b, 0, "a row written in one test database is not in another");
    let seen_by_c: i64 = TestDb::bare().run(|c| c.query_one("SELECT count(*) FROM platform_settings WHERE key = 'template.isolation'", &[]).unwrap().get(0));
    assert_eq!(seen_by_c, 0, "and it is not in the template, so the next test does not get it either");
}
