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

// A template is cached on the server, and migrations seed times relative to when they ran: the schedule that expires trial
// copies is first due an hour after it was seeded. A template older than that would hand every test a schedule that is
// already due, and a test that counts the jobs a scheduler tick makes would count one too many. So a template is replaced
// every half hour: its name carries the half hour it was built in.
#[test]
fn a_template_is_replaced_every_half_hour() {
    let t0 = 1_800_000_000_u64; // a multiple of the bucket
    assert_eq!(t0 % common::TEMPLATE_BUCKET_SECS, 0);
    assert_eq!(common::template_name_at(t0), common::template_name_at(t0 + common::TEMPLATE_BUCKET_SECS - 1), "one half hour has one template");
    assert_ne!(common::template_name_at(t0), common::template_name_at(t0 + common::TEMPLATE_BUCKET_SECS), "the next half hour has another");
    // The bucket is no longer than half the hour the seeded schedules are ahead. A longer one fails to compile.
    const { assert!(common::TEMPLATE_BUCKET_SECS <= 1_800) };
}

#[test]
fn the_oldest_template_a_test_can_clone_still_has_its_seeded_times_ahead() {
    let db = TestDb::bare();
    let minutes_ahead: f64 = db.run(|c| c.query_one("SELECT COALESCE(MIN(EXTRACT(EPOCH FROM (next_run_at - NOW()))), 99999)::float8 / 60 FROM job_schedules", &[]).unwrap().get(0));
    assert!(minutes_ahead >= 25.0, "a schedule seeded by a migration is {minutes_ahead:.0} minutes from due. A template this old would start tests with a schedule already due");
}
