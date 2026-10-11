//! Access rules phase 2: one predicate, two evaluators. `eval` in Rust and `to_sql` over
//! `record_index` must return the same records for every predicate, record, and person.

use postgres::types::ToSql;
use scaffoldry_core::access::{check, row_scope, substitute, Op, Operand, Person, PersonAttr, Pred, RowOp, RowRule, RowScope};
use scaffoldry_engine::access::{eval, to_sql, CompileError, Param};
use scaffoldry_engine::{index_rows, FieldSpec, FieldType};
use serde_json::{json, Value};
use std::collections::BTreeSet;
use uuid::Uuid;

fn db_url() -> String {
    std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://scaffoldry:scaffoldry_dev_password@127.0.0.1:5433/scaffoldry".to_string())
}

/// A connection to a database that has the schema. `ServerState::new` applies the migrations.
fn client() -> postgres::Client {
    static MIGRATED: std::sync::Once = std::sync::Once::new();
    MIGRATED.call_once(|| {
        scaffoldry_server::state::ServerState::new().expect("state");
    });
    postgres::Client::connect(&db_url(), postgres::NoTls).expect("database")
}

fn field(name: &str, t: FieldType) -> FieldSpec {
    FieldSpec::simple(name, name, t, false, false)
}

/// The table every case uses: one field of each index column, and a multi-valued one.
fn fields() -> Vec<FieldSpec> {
    vec![
        field("t", FieldType::Text),
        field("n", FieldType::Number),
        field("d", FieldType::Date),
        field("m", FieldType::MultiSelect),
        field("b", FieldType::Boolean),
    ]
}

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
    fn pick<'a, T>(&mut self, items: &'a [T]) -> &'a T {
        &items[self.below(items.len())]
    }
}

const WORDS: [&str; 7] = ["Alpha", "alpha", "BETA", "gamma", "Physics", "chem", "delta"];
const DATES: [&str; 6] = [
    "2026-01-05",
    "2026-01-05T10:00:00Z",
    "2026-01-05T12:00:00+02:00",
    "2026-02-01",
    "2025-12-31T23:59:59Z",
    "nope",
];

fn long_word() -> String {
    "x".repeat(140)
}

fn random_value(rng: &mut Rng, name: &str) -> Option<Value> {
    // Missing, null, empty, or a value.
    match rng.below(8) {
        0 => return None,
        1 => return Some(Value::Null),
        2 => return Some(json!("")),
        _ => {}
    }
    Some(match name {
        "t" => {
            if rng.below(10) == 0 {
                json!(long_word())
            } else {
                json!(rng.pick(&WORDS))
            }
        }
        "n" => match rng.below(5) {
            0 => json!(rng.below(7) as i64 - 3),
            1 => json!(format!("{}", rng.below(5))),
            2 => json!(format!("{}.5", rng.below(5))),
            3 => json!("abc"),
            _ => json!(rng.below(5) as f64 + 0.25),
        },
        "d" => json!(rng.pick(&DATES)),
        "m" => match rng.below(4) {
            0 => json!([]),
            1 => json!(rng.pick(&WORDS)),
            _ => Value::Array((0..1 + rng.below(3)).map(|_| json!(rng.pick(&WORDS))).collect()),
        },
        _ => match rng.below(5) {
            0 => json!(true),
            1 => json!(false),
            2 => json!("true"),
            3 => json!("false"),
            _ => json!("maybe"),
        },
    })
}

fn random_record(rng: &mut Rng) -> Value {
    let mut map = serde_json::Map::new();
    for f in fields() {
        if let Some(v) = random_value(rng, &f.name) {
            map.insert(f.name.clone(), v);
        }
    }
    Value::Object(map)
}

fn literal(rng: &mut Rng, name: &str) -> Value {
    match name {
        "t" | "m" => {
            if rng.below(12) == 0 {
                json!("x".repeat(128))
            } else {
                json!(rng.pick(&WORDS))
            }
        }
        "n" => match rng.below(4) {
            0 => json!(rng.below(7) as i64 - 3),
            1 => json!(format!("{}", rng.below(5))),
            2 => json!(rng.below(5) as f64 + 0.5),
            _ => json!("abc"),
        },
        "d" => json!(rng.pick(&DATES)),
        _ => match rng.below(3) {
            0 => json!(true),
            1 => json!(false),
            _ => json!("true"),
        },
    }
}

fn random_cmp(rng: &mut Rng) -> Pred {
    let f = rng.pick(&["t", "n", "d", "m", "b"]);
    let ops: &[Op] = match *f {
        "t" | "m" => &[Op::Eq, Op::Ne, Op::In, Op::IsEmpty, Op::IsNotEmpty],
        _ => &[Op::Eq, Op::Ne, Op::In, Op::IsEmpty, Op::IsNotEmpty, Op::Gt, Op::Lt],
    };
    let op = *rng.pick(ops);
    let value = match rng.below(6) {
        0 if matches!(*f, "t" | "m") => Operand::Person(*rng.pick(&[PersonAttr::Department, PersonAttr::Affiliation, PersonAttr::UnitIds, PersonAttr::GroupIds])),
        _ if op == Op::In => Operand::Literal(Value::Array((0..1 + rng.below(3)).map(|_| literal(rng, f)).collect())),
        _ => Operand::Literal(literal(rng, f)),
    };
    // Sometimes the left side is the person.
    if rng.below(8) == 0 {
        let attr = *rng.pick(&["department", "affiliation", "unit_ids", "group_ids"]);
        return Pred::Cmp { field: format!("person.{attr}"), op: if op == Op::Gt || op == Op::Lt { Op::Eq } else { op }, value: Operand::Literal(literal(rng, "t")) };
    }
    Pred::Cmp { field: f.to_string(), op, value }
}

fn random_pred(rng: &mut Rng, depth: usize) -> Pred {
    if depth == 0 || rng.below(3) == 0 {
        return if rng.below(15) == 0 { Pred::True } else { random_cmp(rng) };
    }
    match rng.below(3) {
        0 => Pred::And((0..rng.below(4)).map(|_| random_pred(rng, depth - 1)).collect()),
        1 => Pred::Or((0..rng.below(4)).map(|_| random_pred(rng, depth - 1)).collect()),
        _ => Pred::Not(Box::new(random_pred(rng, depth - 1))),
    }
}

fn random_person(rng: &mut Rng) -> Person {
    Person {
        address: format!("p{}@x.edu", rng.below(3)),
        department: if rng.below(5) == 0 { String::new() } else { rng.pick(&WORDS).to_string() },
        affiliation: rng.pick(&WORDS).to_string(),
        unit_ids: (0..rng.below(3)).map(|_| rng.pick(&WORDS).to_string()).collect(),
        position_keys: vec![],
        group_ids: (0..rng.below(3)).map(|_| rng.pick(&WORDS).to_string()).collect(),
    }
}

fn boxed(params: Vec<Param>) -> Vec<Box<dyn ToSql + Sync>> {
    params
        .into_iter()
        .map(|p| -> Box<dyn ToSql + Sync> {
            match p {
                Param::Text(t) => Box::new(t),
                Param::Texts(t) => Box::new(t),
                Param::Num(n) => Box::new(n),
                Param::Nums(n) => Box::new(n),
            }
        })
        .collect()
}

fn insert(tx: &mut postgres::Transaction, slug: &str, id: &str, data: &Value) {
    tx.execute(
        "INSERT INTO dataset_records (id, app_slug, data, ceds_mapping, is_ferpa_sensitive, table_id) VALUES ($1, $2, $3, '{}'::jsonb, false, 't1')",
        &[&id, &slug, data],
    )
    .unwrap();
    for f in fields() {
        let value = data.get(&f.name).unwrap_or(&Value::Null);
        for (ord, (num, txt, ts)) in index_rows(&f, value).into_iter().enumerate() {
            tx.execute(
                "INSERT INTO record_index (app_slug, table_id, field, record_id, num, txt, ts, ord) VALUES ($1, 't1', $2, $3, $4, $5, $6::text::timestamptz, $7)",
                &[&slug, &f.name, &id, &num, &txt, &ts, &(ord as i16)],
            )
            .unwrap();
        }
    }
}

/// The ids the compiled SQL returns, and the ids `eval` accepts, for the records inserted.
fn both(tx: &mut postgres::Transaction, slug: &str, pred: &Pred, records: &[(String, Value)]) -> Result<(BTreeSet<String>, BTreeSet<String>), CompileError> {
    let (sql, params) = to_sql(pred, &fields(), "r.id", slug, "t1", 1)?;
    let all = boxed(params);
    let refs: Vec<&(dyn ToSql + Sync)> = all.iter().map(|b| b.as_ref()).collect();
    let statement = format!("SELECT r.id FROM dataset_records r WHERE r.app_slug = $1 AND r.table_id = $2 AND r.deleted_at IS NULL AND {sql}");
    let from_sql: BTreeSet<String> = tx.query(&statement, &refs).unwrap_or_else(|e| panic!("{e}\n{statement}")).iter().map(|r| r.get(0)).collect();
    let from_eval: BTreeSet<String> = records.iter().filter(|(_, d)| eval(pred, d, &fields())).map(|(id, _)| id.clone()).collect();
    Ok((from_sql, from_eval))
}

fn scratch(c: &mut postgres::Client) -> String {
    let _ = c;
    format!("acc-{}", &Uuid::new_v4().to_string()[..8])
}

// 1. The 2,000-case equivalence test.
#[test]
fn eval_and_sql_agree_on_two_thousand_random_cases() {
    let mut c = client();
    let slug = scratch(&mut c);
    let mut compiled = 0;
    for case in 0..2000u64 {
        let seed = 0x9E37_79B9_7F4A_7C15u64.wrapping_mul(case + 1) | 1;
        let mut rng = Rng(seed);
        let mut tx = c.transaction().unwrap();
        let records: Vec<(String, Value)> = (0..14).map(|k| (format!("{slug}-{case}-{k}"), random_record(&mut rng))).collect();
        for (id, data) in &records {
            insert(&mut tx, &slug, id, data);
        }
        let pred = substitute(&random_pred(&mut rng, 4), &random_person(&mut rng));
        match both(&mut tx, &slug, &pred, &records) {
            Ok((sql, rust)) => {
                compiled += 1;
                assert_eq!(sql, rust, "case {case} seed {seed:#x}\npredicate: {pred:?}\nrecords: {records:?}");
            }
            // Ordering text is the one thing a rule may not compile. The generator never asks for it.
            Err(e) => panic!("case {case} seed {seed:#x} did not compile: {e}\n{pred:?}"),
        }
        tx.rollback().unwrap();
    }
    assert_eq!(compiled, 2000);
}

// 2. substitute folds constants.
#[test]
fn substitute_folds_what_the_person_decides() {
    let physics = Person { address: "a@x.edu".into(), department: "Physics".into(), ..Default::default() };
    let chemistry = Person { address: "b@x.edu".into(), department: "Chemistry".into(), ..Default::default() };
    let same = Pred::Cmp { field: "person.address".into(), op: Op::Eq, value: Operand::Person(PersonAttr::Address) };
    assert_eq!(RowScope::of(substitute(&same, &physics)), RowScope::AllowAll);
    let dept = Pred::Cmp { field: "person.department".into(), op: Op::Eq, value: Operand::Literal(json!("physics")) };
    assert_eq!(RowScope::of(substitute(&dept, &physics)), RowScope::AllowAll, "case does not matter");
    assert_eq!(RowScope::of(substitute(&dept, &chemistry)), RowScope::DenyAll);

    // A rule about the record stays, with the person's address in it.
    let mine = Pred::Cmp { field: "t".into(), op: Op::Eq, value: Operand::Person(PersonAttr::Address) };
    match RowScope::of(substitute(&mine, &physics)) {
        RowScope::Mixed(Pred::Cmp { value: Operand::Literal(v), .. }) => assert_eq!(v, json!("a@x.edu")),
        other => panic!("{other:?}"),
    }
}

#[test]
fn row_scope_skips_excepted_roles_and_platform_admins_and_combines_the_rest() {
    let me = Person { address: "a@x.edu".into(), ..Default::default() };
    let rule = |id: &str, except: &[&str]| RowRule {
        id: id.into(),
        table_id: "t1".into(),
        ops: vec![RowOp::Read],
        visible_when: Pred::Cmp { field: "t".into(), op: Op::Eq, value: Operand::Person(PersonAttr::Address) },
        except_roles: except.iter().map(|s| s.to_string()).collect(),
        message: None,
    };
    let rules = vec![rule("one", &["admin"]), rule("two", &[])];
    assert!(matches!(row_scope(&me, Some("viewer"), false, &rules, "t1", RowOp::Read), RowScope::Mixed(Pred::And(ps)) if ps.len() == 2));
    assert!(matches!(row_scope(&me, Some("admin"), false, &rules, "t1", RowOp::Read), RowScope::Mixed(Pred::Cmp { .. })), "one rule is skipped");
    assert_eq!(row_scope(&me, None, true, &rules, "t1", RowOp::Read), RowScope::AllowAll, "a Platform Admin skips all");
    assert_eq!(row_scope(&me, Some("viewer"), false, &rules, "t1", RowOp::Update), RowScope::AllowAll, "no rule for that operation");
    assert_eq!(row_scope(&me, Some("viewer"), false, &rules, "other", RowOp::Read), RowScope::AllowAll, "no rule for that table");
}

// 3. A field outside the indexed set is an error, not a scan.
#[test]
fn a_field_outside_the_indexed_set_does_not_compile() {
    let p = Pred::Cmp { field: "unindexed".into(), op: Op::Eq, value: Operand::Literal(json!("x")) };
    assert_eq!(to_sql(&p, &fields(), "r.id", "a", "t", 1).unwrap_err(), CompileError::NotIndexed("unindexed".into()));
    let text_order = Pred::Cmp { field: "t".into(), op: Op::Gt, value: Operand::Literal(json!("x")) };
    assert!(matches!(to_sql(&text_order, &fields(), "r.id", "a", "t", 1).unwrap_err(), CompileError::TextOrdering(_)));
}

// 4. A multi-valued field matches on any of its values, in both evaluators.
#[test]
fn a_multi_valued_field_matches_on_any_value() {
    let mut c = client();
    let slug = scratch(&mut c);
    let mut tx = c.transaction().unwrap();
    let records = vec![(format!("{slug}-1"), json!({ "m": ["a", "b"] }))];
    insert(&mut tx, &slug, &records[0].0, &records[0].1);
    let in_set = |vals: Value| Pred::Cmp { field: "m".into(), op: Op::In, value: Operand::Literal(vals) };
    let (sql, rust) = both(&mut tx, &slug, &in_set(json!(["b", "c"])), &records).unwrap();
    assert_eq!((sql.len(), rust.len()), (1, 1), "[a, b] matches In [b, c]");
    let (sql, rust) = both(&mut tx, &slug, &in_set(json!(["c"])), &records).unwrap();
    assert_eq!((sql.len(), rust.len()), (0, 0), "and not In [c]");
    tx.rollback().unwrap();
}

// 5. A literal over 128 characters is rejected.
#[test]
fn a_literal_over_128_characters_is_rejected() {
    let long = Pred::Cmp { field: "t".into(), op: Op::Eq, value: Operand::Literal(json!("x".repeat(129))) };
    assert!(check(&long).is_err());
    assert!(to_sql(&long, &fields(), "r.id", "a", "t", 1).is_err());
    let nested = Pred::Not(Box::new(Pred::And(vec![Pred::Cmp { field: "t".into(), op: Op::In, value: Operand::Literal(json!(["ok", "y".repeat(200)])) }])));
    assert!(check(&nested).is_err());
    let exactly = Pred::Cmp { field: "t".into(), op: Op::Eq, value: Operand::Literal(json!("x".repeat(128))) };
    assert!(check(&exactly).is_ok());
}

// 6. At 200,000 rows a compiled predicate does not scan.
#[test]
fn a_compiled_predicate_over_200000_rows_does_not_scan() {
    let mut c = client();
    let slug = scratch(&mut c);
    c.execute(
        "INSERT INTO dataset_records (id, app_slug, data, ceds_mapping, is_ferpa_sensitive, table_id, created_at) \
         SELECT $1::text || lpad(g::text, 8, '0'), $1::text, jsonb_build_object('_table_id', 't1', 't', 'p' || (g % 5000), 'n', g % 100), '{}'::jsonb, false, 't1', \
                TIMESTAMPTZ '2026-01-01 00:00:00+00' + g * interval '1 millisecond' FROM generate_series(1, 200000) g",
        &[&slug],
    )
    .unwrap();
    c.execute(
        "INSERT INTO record_index (app_slug, table_id, field, record_id, txt, ord) \
         SELECT $1::text, 't1', 't', $1::text || lpad(g::text, 8, '0'), 'p' || (g % 5000), 0 FROM generate_series(1, 200000) g",
        &[&slug],
    )
    .unwrap();
    c.execute(
        "INSERT INTO record_index (app_slug, table_id, field, record_id, num, ord) \
         SELECT $1::text, 't1', 'n', $1::text || lpad(g::text, 8, '0'), g % 100, 0 FROM generate_series(1, 200000) g",
        &[&slug],
    )
    .unwrap();
    c.batch_execute("ANALYZE dataset_records; ANALYZE record_index").unwrap();

    let eq = |f: &str, v: Value| Pred::Cmp { field: f.into(), op: Op::Eq, value: Operand::Literal(v) };
    let preds = [
        eq("t", json!("p42")),
        Pred::And(vec![eq("t", json!("p42")), Pred::Not(Box::new(eq("n", json!(7))))]),
        Pred::Or(vec![eq("t", json!("p42")), eq("t", json!("p43"))]),
    ];
    let cleanup = |c: &mut postgres::Client| {
        let _ = c.execute("DELETE FROM dataset_records WHERE app_slug = $1", &[&slug]);
    };
    for pred in preds {
        let (sql, params) = to_sql(&pred, &fields(), "r.id", &slug, "t1", 1).unwrap();
        let all = boxed(params);
        let refs: Vec<&(dyn ToSql + Sync)> = all.iter().map(|b| b.as_ref()).collect();
        let statement = format!("EXPLAIN SELECT r.id FROM dataset_records r WHERE r.app_slug = $1 AND r.table_id = $2 AND r.deleted_at IS NULL AND {sql}");
        let plan: String = c.query(&statement, &refs).unwrap().iter().map(|r| r.get::<_, String>(0)).collect::<Vec<_>>().join("\n");
        let scanned = plan.contains("Seq Scan on dataset_records") || plan.contains("Seq Scan on record_index");
        if scanned {
            cleanup(&mut c);
        }
        assert!(!scanned, "{pred:?}\n{plan}");
    }
    cleanup(&mut c);
}

// 7. Blank and missing values behave the same in both evaluators for every operation.
#[test]
fn blank_and_missing_values_behave_the_same_for_every_op() {
    let mut c = client();
    let slug = scratch(&mut c);
    let mut tx = c.transaction().unwrap();
    let blanks = [json!({}), json!({ "t": null, "n": null, "d": null, "m": null, "b": null }), json!({ "t": "", "n": "", "d": "", "m": [], "b": "" }), json!({ "t": "set", "n": 1, "d": "2026-01-01", "m": ["set"], "b": true })];
    let records: Vec<(String, Value)> = blanks.iter().enumerate().map(|(i, d)| (format!("{slug}-{i}"), d.clone())).collect();
    for (id, data) in &records {
        insert(&mut tx, &slug, id, data);
    }
    for f in ["t", "n", "d", "m", "b"] {
        let probe = if f == "t" || f == "m" { json!("set") } else if f == "d" { json!("2026-01-01") } else { json!(1) };
        for op in [Op::Eq, Op::Ne, Op::In, Op::IsEmpty, Op::IsNotEmpty, Op::Gt, Op::Lt] {
            if matches!(op, Op::Gt | Op::Lt) && (f == "t" || f == "m") {
                continue;
            }
            for negate in [false, true] {
                let operand = if op == Op::In { json!([probe.clone()]) } else { probe.clone() };
                let cmp = Pred::Cmp { field: f.into(), op, value: Operand::Literal(operand) };
                let pred = if negate { Pred::Not(Box::new(cmp)) } else { cmp };
                let (sql, rust) = both(&mut tx, &slug, &pred, &records).unwrap();
                assert_eq!(sql, rust, "{f} {op:?} negate={negate}");
            }
        }
    }
    tx.rollback().unwrap();
}

// The rules serialize in the shape the published schema describes.
#[test]
fn a_rule_round_trips_in_the_schema_shape() {
    let schema: Value = serde_json::from_str(&std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../governance/schema/access-rules.schema.json")).unwrap()).unwrap();
    let defs = schema["$defs"].as_object().unwrap();
    let rule = RowRule {
        id: "mine".into(),
        table_id: "t1".into(),
        ops: vec![RowOp::Read, RowOp::Update],
        visible_when: Pred::And(vec![Pred::Cmp { field: "t".into(), op: Op::Eq, value: Operand::Person(PersonAttr::Address) }, Pred::Not(Box::new(Pred::True))]),
        except_roles: vec!["admin".into()],
        message: None,
    };
    let json = serde_json::to_value(&rule).unwrap();
    for key in defs["RowRule"]["required"].as_array().unwrap() {
        assert!(json.get(key.as_str().unwrap()).is_some(), "missing {key}");
    }
    assert_eq!(json["visible_when"]["and"][0]["cmp"]["value"], json!({ "person": "address" }));
    assert_eq!(json["visible_when"]["and"][1], json!({ "not": "true" }));
    let back: RowRule = serde_json::from_value(json).unwrap();
    assert_eq!(back, rule);
}
