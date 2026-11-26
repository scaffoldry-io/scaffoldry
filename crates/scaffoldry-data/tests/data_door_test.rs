//! The data crate's functions, run against temporary tables of the shape the migrations make, in a transaction that rolls
//! back. The real schema is exercised by the server's own tests. These show each function does what it says.

use postgres::{Client, NoTls, Transaction};
use scaffoldry_data::{index, keys, labels, records};
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
           PRIMARY KEY (app_slug, table_id, field)) ON COMMIT DROP;
         CREATE TEMP TABLE datasets (id text PRIMARY KEY, app_slug text NOT NULL, table_id text NOT NULL, key_spec jsonb, UNIQUE (app_slug, table_id)) ON COMMIT DROP;
         CREATE TEMP TABLE record_keys (app_slug text NOT NULL, table_id text NOT NULL, key_text text NOT NULL, record_id text NOT NULL, \
           CONSTRAINT uq_record_keys UNIQUE (app_slug, table_id, key_text), CONSTRAINT uq_record_keys_record UNIQUE (record_id)) ON COMMIT DROP;",
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

#[test]
fn a_key_text_is_trimmed_and_folded_by_rule_and_blank_when_a_field_is_empty() {
    let fold = keys::KeySpec { fields: vec!["a".into(), "b".into()], case_fold: true };
    let keep = keys::KeySpec { fields: vec!["a".into(), "b".into()], case_fold: false };
    assert_eq!(keys::key_text(&json!({"a": "  Ab ", "b": 7}), &fold), keys::key_text(&json!({"a": "aB", "b": "7"}), &fold));
    assert_ne!(keys::key_text(&json!({"a": "Ab", "b": 7}), &keep), keys::key_text(&json!({"a": "aB", "b": 7}), &keep));
    assert_eq!(keys::key_text(&json!({"a": " x ", "b": "y"}), &keep), keys::key_text(&json!({"a": "x", "b": " y"}), &keep));
    assert_ne!(keys::key_text(&json!({"a": "xy", "b": ""}), &keep), Some("xy".to_string()), "a blank field leaves no key");
    for blank in [json!({"a": "x"}), json!({"a": "x", "b": null}), json!({"a": "x", "b": "   "})] {
        assert_eq!(keys::key_text(&blank, &keep), None);
    }
    // Two fields do not run together: ("ab", "c") is not ("a", "bc").
    assert_ne!(keys::key_text(&json!({"a": "ab", "b": "c"}), &keep), keys::key_text(&json!({"a": "a", "b": "bc"}), &keep));
}

#[test]
fn a_key_is_unique_freed_by_clear_and_found_by_its_text() {
    let mut c = Client::connect(&url(), NoTls).expect("the test server");
    let mut tx = c.transaction().unwrap();
    shapes(&mut tx);
    tx.execute("INSERT INTO datasets (id, app_slug, table_id) VALUES ('ds1', 'app', 't1'), ('ds2', 'app', 't2')", &[]).unwrap();
    assert!(keys::spec_for(&mut tx, "app", "t1").unwrap().is_none());
    // A table with no key writes nothing.
    keys::write(&mut tx, "app", "t1", "r1", &json!({"code": "A"})).unwrap();
    assert_eq!(tx.query_one("SELECT count(*) FROM record_keys", &[]).unwrap().get::<_, i64>(0), 0);

    let spec = keys::KeySpec { fields: vec!["code".into()], case_fold: true };
    assert_eq!(keys::declare(&mut tx, "app", "t1", Some(&spec)).unwrap(), Ok(0));
    assert_eq!(keys::spec_for(&mut tx, "app", "t1").unwrap(), Some(spec.clone()));
    keys::write(&mut tx, "app", "t1", "r1", &json!({"code": " A1 "})).unwrap();
    assert_eq!(keys::find(&mut tx, "app", "t1", "a1").unwrap(), Some("r1".to_string()));
    assert_eq!(keys::find(&mut tx, "app", "t2", "a1").unwrap(), None, "another table has its own keys");
    // The same record changes its key. Its old text is free.
    keys::write(&mut tx, "app", "t1", "r1", &json!({"code": "B"})).unwrap();
    assert_eq!(keys::find(&mut tx, "app", "t1", "a1").unwrap(), None);
    // A second record cannot take it. The error is the unique violation and says nothing else.
    let mut sp = tx.transaction().unwrap();
    let err = keys::write(&mut sp, "app", "t1", "r2", &json!({"code": "b"})).unwrap_err();
    assert!(keys::is_taken(&err), "{err}");
    drop(sp);
    keys::clear(&mut tx, "r1").unwrap();
    keys::write(&mut tx, "app", "t1", "r2", &json!({"code": "b"})).unwrap();
    // A record with no key text holds none.
    keys::write(&mut tx, "app", "t1", "r2", &json!({"code": ""})).unwrap();
    assert_eq!(keys::find(&mut tx, "app", "t1", "b").unwrap(), None);
}

#[test]
fn declaring_a_key_keys_live_rows_and_lists_the_groups_that_share_one() {
    let mut c = Client::connect(&url(), NoTls).expect("the test server");
    let mut tx = c.transaction().unwrap();
    shapes(&mut tx);
    tx.execute("INSERT INTO datasets (id, app_slug, table_id) VALUES ('ds1', 'app', 't1')", &[]).unwrap();
    let none = json!({});
    for (id, code) in [("r1", "A"), ("r2", " a"), ("r3", "B"), ("r4", "b "), ("r5", "C"), ("r6", "a")] {
        let data = json!({ "code": code });
        records::insert(&mut tx, &new(id, &data, &none)).unwrap();
    }
    // r5 is in trash and holds no key. r7 is in another table.
    records::soft_delete(&mut tx, "app", "r5", None, "ann", None).unwrap();
    let spec = keys::KeySpec { fields: vec!["code".into()], case_fold: true };
    let dups = keys::declare(&mut tx, "app", "t1", Some(&spec)).unwrap().unwrap_err();
    assert_eq!(dups.groups, vec![vec!["r1".to_string(), "r2".to_string(), "r6".to_string()], vec!["r3".to_string(), "r4".to_string()]]);
    assert_eq!(dups.more, 0);

    // With the extras gone, the declaration keys the rest. A repeat of the same key does it again from scratch.
    records::soft_delete(&mut tx, "app", "r2", None, "ann", None).unwrap();
    records::soft_delete(&mut tx, "app", "r6", None, "ann", None).unwrap();
    records::soft_delete(&mut tx, "app", "r4", None, "ann", None).unwrap();
    assert_eq!(keys::declare(&mut tx, "app", "t1", Some(&spec)).unwrap(), Ok(2));
    assert_eq!(keys::find(&mut tx, "app", "t1", "b").unwrap(), Some("r3".to_string()));
    // Taking the key off frees all of them.
    assert_eq!(keys::declare(&mut tx, "app", "t1", None).unwrap(), Ok(0));
    assert!(keys::spec_for(&mut tx, "app", "t1").unwrap().is_none());
    assert_eq!(tx.query_one("SELECT count(*) FROM record_keys", &[]).unwrap().get::<_, i64>(0), 0);
}
