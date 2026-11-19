//! The data crate's functions, run against temporary tables of the shape the migrations make, in a transaction that rolls
//! back. The real schema is exercised by the server's own tests. These show each function does what it says.

use postgres::{Client, NoTls, Transaction};
use scaffoldry_data::{index, labels, records};
use serde_json::json;

const URL: &str = "postgres://scaffoldry:scaffoldry_dev_password@127.0.0.1:5433/scaffoldry";

fn url() -> String {
    std::env::var("DATABASE_URL").unwrap_or_else(|_| URL.to_string())
}

fn shapes(tx: &mut Transaction<'_>) {
    tx.batch_execute(
        "CREATE TEMP TABLE dataset_records (id text PRIMARY KEY, app_slug text NOT NULL, data jsonb NOT NULL, ceds_mapping jsonb NOT NULL, \
           is_ferpa_sensitive boolean NOT NULL, created_by text, table_id text NOT NULL, version int NOT NULL DEFAULT 1, \
           created_at timestamptz NOT NULL DEFAULT NOW(), deleted_at timestamptz, deleted_by text, delete_batch uuid) ON COMMIT DROP;
         CREATE TEMP TABLE record_index (app_slug text NOT NULL, table_id text NOT NULL, field text NOT NULL, record_id text NOT NULL, \
           num float8, txt text, ts timestamptz, ord int2 NOT NULL, PRIMARY KEY (record_id, field, ord)) ON COMMIT DROP;
         CREATE TEMP TABLE data_labels (app_slug text NOT NULL, table_id text NOT NULL, field text NOT NULL, ferpa_sensitive boolean NOT NULL, \
           note text NOT NULL, set_by text NOT NULL, categories text[] NOT NULL DEFAULT '{}', set_at timestamptz NOT NULL DEFAULT NOW(), \
           PRIMARY KEY (app_slug, table_id, field)) ON COMMIT DROP;",
    )
    .expect("temporary tables");
}

fn new<'a>(id: &'a str, data: &'a serde_json::Value, none: &'a serde_json::Value) -> records::NewRecord<'a> {
    records::NewRecord { id, app_slug: "app", data, ceds_mapping: none, is_ferpa_sensitive: false, created_by: Some("ann"), table_id: "t1" }
}

#[test]
fn a_record_is_inserted_saved_trashed_and_restored() {
    let mut c = Client::connect(&url(), NoTls).expect("the test server");
    let mut tx = c.transaction().unwrap();
    shapes(&mut tx);
    let (v1, v2, none) = (json!({"a": 1}), json!({"a": 2}), json!({}));
    records::insert(&mut tx, &new("r1", &v1, &none)).unwrap();
    assert_eq!(records::live_for_update(&mut tx, "app", "r1").unwrap(), Some((v1.clone(), 1)));
    assert_eq!(records::save_data(&mut tx, "app", "r1", &v2, "t2").unwrap(), 2);
    records::set_data(&mut tx, "app", "r1", &v1).unwrap();
    assert_eq!(records::live_version(&mut tx, "app", "r1").unwrap(), Some(2));

    assert!(records::soft_delete(&mut tx, "app", "r1", Some(1), "ann", None).unwrap().is_none(), "a stale version moves nothing");
    let gone = records::soft_delete(&mut tx, "app", "r1", Some(2), "ann", None).unwrap().expect("trashed");
    assert_eq!((gone.version, gone.table_id.as_str(), &gone.data), (2, "t2", &v1));
    assert_eq!(records::live_version(&mut tx, "app", "r1").unwrap(), None);
    assert!(records::live_for_update(&mut tx, "app", "r1").unwrap().is_none());

    assert!(records::restore(&mut tx, "app", "r1").unwrap().is_some());
    assert!(records::restore(&mut tx, "app", "r1").unwrap().is_none(), "a live record is not in trash");
    assert_eq!(records::live_version(&mut tx, "app", "r1").unwrap(), Some(2));
}

#[test]
fn an_upsert_raises_the_version_of_an_existing_record() {
    let mut c = Client::connect(&url(), NoTls).expect("the test server");
    let mut tx = c.transaction().unwrap();
    shapes(&mut tx);
    let (v1, v2, none) = (json!({"a": 1}), json!({"a": 2}), json!({}));
    assert!(records::data_for_update(&mut tx, "r1").unwrap().is_none());
    assert_eq!(records::upsert(&mut tx, &new("r1", &v1, &none)).unwrap(), 1);
    assert_eq!(records::data_for_update(&mut tx, "r1").unwrap(), Some(v1));
    assert_eq!(records::upsert(&mut tx, &new("r1", &v2, &none)).unwrap(), 2);
}

#[test]
fn index_rows_are_stored_and_removed_by_record_field_and_table() {
    let mut c = Client::connect(&url(), NoTls).expect("the test server");
    let mut tx = c.transaction().unwrap();
    shapes(&mut tx);
    let row = |field: &str| index::IndexRow { field: field.into(), ord: 0, num: Some(1.0), txt: Some("x".into()), ts: None };
    let rows = vec![("r1".to_string(), row("f")), ("r1".to_string(), row("g")), ("r2".to_string(), row("f"))];
    index::insert_rows(&mut tx, "app", "t1", &rows).unwrap();
    index::insert_rows(&mut tx, "app", "t1", &rows).unwrap();
    let count = |tx: &mut Transaction<'_>| tx.query_one("SELECT count(*) FROM record_index", &[]).unwrap().get::<_, i64>(0);
    assert_eq!(count(&mut tx), 3, "a repeat leaves rows as they are");
    index::delete_for_record(&mut tx, "r1").unwrap();
    assert_eq!(count(&mut tx), 1);
    index::delete_for_fields(&mut tx, "app", "t1", &["f".to_string()]).unwrap();
    assert_eq!(count(&mut tx), 0);
    index::insert_rows(&mut tx, "app", "t1", &rows).unwrap();
    index::delete_for_table(&mut tx, "app", "t1").unwrap();
    assert_eq!(count(&mut tx), 0);
}

#[test]
fn labels_are_set_listed_given_a_category_and_removed() {
    let mut c = Client::connect(&url(), NoTls).expect("the test server");
    let mut tx = c.transaction().unwrap();
    shapes(&mut tx);
    labels::set_sensitive(&mut tx, "app", "t1", "ssn", true, "why", "ann").unwrap();
    labels::add_category(&mut tx, "app", "t1", "ssn", "pii", "n2", "bob").unwrap();
    labels::add_category(&mut tx, "app", "t1", "ssn", "pii", "n2", "bob").unwrap();
    labels::add_category(&mut tx, "app", "t1", "email", "contact", "n3", "bob").unwrap();
    let mut all = labels::list(&mut tx).unwrap();
    all.sort_by(|a, b| a.field.cmp(&b.field));
    assert_eq!(all.len(), 2);
    assert_eq!((all[0].field.as_str(), all[0].ferpa_sensitive, all[0].categories.clone()), ("email", false, vec!["contact".to_string()]));
    assert_eq!((all[1].field.as_str(), all[1].ferpa_sensitive, all[1].categories.clone()), ("ssn", true, vec!["pii".to_string()]));
    labels::remove(&mut tx, "app", "t1", "ssn").unwrap();
    assert_eq!(labels::list(&mut tx).unwrap().len(), 1);
}
