# Row scale — implementation brief

Hand this file to Gemini Flash. Do one phase per session. Stop when that phase's tests pass. Do not start the next phase.

Run order is `docs/plans/README.md`. `docs/plans/live-data.md` phases 1 and 2 come before this plan. After them, records live in PostgreSQL only and carry a `version`.

One table in one app must hold a million rows and still list, sort, filter, total, and search in the time a person will wait. Today it cannot. The server clones every record of the app in memory, then filters and sorts the clone.

## The rule this plan keeps

A user's schema is JSON. It stays JSON.

| Thing | Where it lives | Does PostgreSQL know the user's schema? |
| --- | --- | --- |
| An app's tables and fields | The manifest, one JSONB document | No |
| A record's values | `dataset_records.data`, one JSONB document | No |
| What this plan adds | Three fixed structures, the same for every app | No. A field name is a string in a row, never a column |

No statement in this plan runs `ALTER TABLE`, `CREATE TABLE`, or `CREATE INDEX` for a user's table or field. Every structure is created once by a migration. Architecture rule 4 in `docs/ARCHITECTURE.md` section 7 holds.

`dataset_records.data` is the truth. `record_index` is derived from it. It is written in the same transaction as the record. It can be deleted and rebuilt from the records at any time. Phase 2 writes the function that rebuilds it.

## Standard

Force.com stores custom fields in generic columns and indexes them through a pivot table of typed value columns keyed by field. This plan is that design at one tenant's size. Record the reference in the phase 2 commit message: Weissman and Bobrowski, "The Design of the Force.com Multitenant Internet Application Development Platform", SIGMOD 2009.

PostgreSQL features used: B-tree row comparison for keyset paging, `jsonb_to_tsvector`, a stored generated column, and a GIN index on `tsvector`. All are in PostgreSQL 17.

## What already exists

| Fact | Where |
| --- | --- |
| One row per record. `data` is JSONB | `dataset_records` in `0002_workspaces_and_ledger.sql` |
| The table a record belongs to is a key inside the JSON, `_table_id` | `routes/records.rs` `create_table_record` |
| A GIN index on the whole document. No query uses it | `idx_dataset_records_data_gin` |
| Filter, sort, and paging run in Rust over a clone of every record of the app | `routes/records.rs` `list_table_records` |
| Filter and sort types | `scaffoldry-engine/src/lib.rs` `CompoundFilter`, `FilterOperator`, `SortRule` |
| Formula, lookup, count, and rollup are computed from records handed in by the caller | `scaffoldry-engine/src/lib.rs` `compute_field_value` |
| A relationship joins two tables by field value, not by record id | `scaffoldry-engine/src/lib.rs` `TableRelationship` |

## Out of scope

- DataFusion, DuckDB, Arrow, or any second query engine.
- A table, a partition, or an index per user table or per user field.
- A cache, a queue, or a background worker. Derived rows are written in the request that changes the record.
- Grid virtualization and windowed scrolling. They are `views.md` phase 3. Until then the desk pages with the cursor.
- Sorting or indexing a `Lookup` field. A multi-valued field indexes one row per value and cannot be used as a sort key.
- Relationships to published datasets (`linked_dataset_id`). Only relationships between tables of one app.

## Rules for every phase

1. Write the failing test first. Run it. Confirm it fails because the behavior is missing. Then write the code.
2. Do not add a dependency.
3. Every value from a caller is a bound parameter. A field name is checked against the manifest before it is used, and then it is also a bound parameter. It is never pasted into SQL text.
4. A record and its derived rows are written in one transaction on one connection. Add one repository function, `write_record`, and route every record write through it.
5. A scale claim is proved by a query plan, not by a stopwatch alone. Each phase's scale test runs `EXPLAIN (FORMAT JSON)` on the statement and fails if the plan holds a `Seq Scan` on `dataset_records` or `record_index`.

## The scale fixture

Add a test helper in `crates/scaffoldry-server/tests/common/scale.rs`:

```rust
pub fn seed_scale(repo: &PostgresRepository, app_slug: &str, table_id: &str, rows: i64)
```

It inserts with one `INSERT ... SELECT ... FROM generate_series(1, $rows)`. Each record has thirty fields: ten numbers, ten short strings, five dates, five booleans. Values come from the series number so the test can predict them. After phase 2 it also fills `record_index` for the fields the test marks indexed.

Tests in phases 1 to 5 use 200,000 rows. Phase 6 uses 1,000,000.

## Phase 1 — a record knows its table, and a list is a page

Migration `crates/scaffoldry-core/migrations/0014_record_table_id.sql`:

```sql
ALTER TABLE dataset_records ADD COLUMN IF NOT EXISTS table_id VARCHAR(64) NOT NULL DEFAULT '';
UPDATE dataset_records SET table_id = COALESCE(data->>'_table_id', '') WHERE table_id = '';
CREATE INDEX IF NOT EXISTS idx_dataset_records_page
    ON dataset_records (app_slug, table_id, created_at, id);
```

`table_id` is an opaque id. It is the same value the JSON already holds. Every record write sets the column from `_table_id`. Keep the key in the JSON. Other code reads it.

`list_records` in the repository takes `table_id`, `limit`, and an optional cursor of `(created_at, id)`:

```sql
SELECT id, app_slug, table_id, data, ceds_mapping, is_ferpa_sensitive, created_at::text, version
FROM dataset_records
WHERE app_slug = $1 AND table_id = $2 AND (created_at, id) > ($3, $4)
ORDER BY created_at, id
LIMIT $5
```

Without a cursor, omit the row comparison. Do not use `OFFSET` here. `views.md` phase 3 measures it for scrollbar jumps. `lifecycle.md` phase 1 replaces this paging index with a partial one that skips trashed rows. Delete the Rust filter, sort, and `skip`/`take` in `list_table_records`. The REST `offset` parameter becomes `cursor`. The response's `total` field is removed. Counting a million rows on every page is the cost this plan exists to remove. Phase 4 gives a count on request.

Tests in a new `crates/scaffoldry-server/tests/row_scale_test.rs`:

1. A record created with `_table_id: "t1"` has `table_id = 't1'` in the column.
2. 200,000 rows. Paging with `limit` 200 from the start, then from the returned cursor, yields 400 distinct ids in `(created_at, id)` order.
3. The plan for the page statement holds no `Seq Scan`.
4. A page from the middle of the table, reached by cursor, returns in under 200 ms. Print the measured time.

## Phase 2 — the pivot index

Migration `0015_record_index.sql`:

```sql
CREATE TABLE IF NOT EXISTS record_index (
    app_slug VARCHAR(64) NOT NULL,
    table_id VARCHAR(64) NOT NULL,
    field VARCHAR(64) NOT NULL,
    record_id VARCHAR(64) NOT NULL REFERENCES dataset_records(id) ON DELETE CASCADE,
    num DOUBLE PRECISION,
    txt VARCHAR(128),
    ts TIMESTAMPTZ,
    ord SMALLINT NOT NULL DEFAULT 0,
    PRIMARY KEY (record_id, field, ord)
);
CREATE INDEX IF NOT EXISTS idx_record_index_num ON record_index (app_slug, table_id, field, num, record_id);
CREATE INDEX IF NOT EXISTS idx_record_index_txt ON record_index (app_slug, table_id, field, txt, record_id);
CREATE INDEX IF NOT EXISTS idx_record_index_ts  ON record_index (app_slug, table_id, field, ts, record_id);
```

One row per record per indexed field value. A multi-valued field (`MultiSelect`, a multi-person field) writes one row per value, numbered by `ord`. Exactly one of `num`, `txt`, `ts` is set. All three are null when the record's value is blank. A blank value still gets a row, so a sorted list does not lose the record.

Which fields are indexed. Add `indexed: bool` to `FieldSpec` with `#[serde(default)]`. Add `indexed_fields(manifest: &AppManifest, table_id: &str) -> BTreeSet<String>` in `scaffoldry-engine`. A field is in the set when any of these holds:

1. `indexed` is true.
2. A view of that table sorts or filters on it.
3. It is the `source_field` or `target_field` of a `TableRelationship`.

`validate_manifest` refuses a table with more than 20 fields in the set, and refuses `indexed: true` on `Lookup`.

Which column holds the value. Add `index_value(field: &FieldSpec, value: &Value) -> (Option<f64>, Option<String>, Option<String>)`:

| Field type | Column | Stored form |
| --- | --- | --- |
| `Number`, `Currency`, `Percent`, `Rating`, `Autonumber`, `Count`, `Rollup` | `num` | The number |
| `Boolean`, `Checkbox` | `num` | 0 or 1 |
| `Date`, `CreatedTime`, `LastModifiedTime` | `ts` | The instant |
| `Text`, `Select`, `Email`, `Phone`, `Url`, `Relation` | `txt` | Lowercase, first 128 characters |
| `Formula` | `num` when the result is a number, else `txt` | As above |

`write_record` deletes the record's `record_index` rows and inserts the current set, in the transaction that writes the record.

When an approved manifest changes the set for a table, the approval transaction fixes the index: one `DELETE` for each field that left the set, one `INSERT ... SELECT` from `dataset_records` for each field that joined it. Put both in `rebuild_index(app_slug, table_id, fields)`. The same function, called with the whole set, rebuilds a table's index from nothing.

Sort. One sort field, which must be in the set:

```sql
SELECT r.* FROM record_index i
JOIN dataset_records r ON r.id = i.record_id
WHERE i.app_slug = $1 AND i.table_id = $2 AND i.field = $3
  AND <cursor>
ORDER BY i.num ASC NULLS LAST, i.record_id ASC
LIMIT $4
```

Use `txt` or `ts` in place of `num` by the field's type. The cursor is `(value, record_id, in_blanks)`. While `in_blanks` is false, `<cursor>` is `(i.num > $v OR (i.num = $v AND i.record_id > $id) OR i.num IS NULL)`. Once a page ends on a blank value, `in_blanks` is true and `<cursor>` is `(i.num IS NULL AND i.record_id > $id)`. Descending order mirrors this, with blanks still last. A second sort field is refused with `BadRequest`.

Filter. `conjunction` `And`, every clause on a field in the set:

| Operator | SQL on the field's column |
| --- | --- |
| `Equals`, `NotEquals` | `=`, `<>` |
| `GreaterThan`, `LessThan` | `>`, `<` |
| `IsEmpty`, `IsNotEmpty` | All three columns null, or not |

When there is a sort, each clause is an `EXISTS` on `record_index` for the same `record_id`. When there is no sort, the first clause drives the statement and the result is ordered by that clause's column and `record_id`.

Anything else is the small-table path from `docs/plans/mcp-apps.md` phase 2: `Contains`, `NotContains`, `conjunction` `Or`, or a field outside the set. It is accepted at 10,000 rows or fewer and refused above that. The refusal names the field and says to mark it indexed.

Tests:

1. Write a record with an indexed number field. One `record_index` row exists with that `num`. Update the value. Still one row, with the new value.
2. Delete the `record_index` rows for a table, call `rebuild_index`, and the rows are equal to what they were.
3. 200,000 rows. Sort by an indexed number, descending, `limit` 200, three pages by cursor: 600 distinct ids in order. The plan holds no `Seq Scan`.
4. 1,000 of the rows have a blank value in the sort field. Paging to the end returns every record once, blanks last.
5. Filter `amount GreaterThan 199000` returns the predicted rows. The plan holds no `Seq Scan`.
6. Filter on a field outside the set, on the 200,000-row table, is `BadRequest` and names the field.
7. Approve a manifest that adds a view sorting on a new field. `record_index` gains 200,000 rows for that field in the same transaction.
8. A table with 21 indexed fields fails `validate_manifest`.

After `jobs.md` phase 3, the copy runs as the job `build_index` and the approval only records `building`. Until then it runs inside the approval.

## Phase 3 — computed values are written, not recomputed

Read first: `compute_field_value` and every caller, the engine tests in `computed_fields_test.rs`, and `apps/web/src/computedFields.ts`. A `Lookup`, `Count`, or `Rollup` names its link field with `computed_via` (`links.md` phase 3).

Linked records are found through `record_links`, not by value. This phase runs after `links.md` phases 1 to 3, which build the link table and the lookup, rollup, and count fields. The tests below still apply and use links.

| Field type | When it is computed | Where the value lives |
| --- | --- | --- |
| `Formula` that names no `Lookup` field | In `write_record`, from the record being written | In `data`, under the field's name |
| `Count`, `Rollup` | In `write_record` of a linked record | In `data` of the record that owns the field |
| `Lookup`, and a `Formula` that names one | When a page is read | Nowhere. It is added to the rows of that page |

`Count` and `Rollup`. When a record in the linked table is created, updated, or deleted, find the owning records through `record_links`, for the old and the new link. For each, run one SQL aggregate over the linked rows and write the result into the owner's `data` and its `record_index` row. Do not raise the owner's `version`. A computed change is not a user edit and must not make someone's save conflict.

`Lookup`. After a page of at most 200 rows is loaded, collect the link ids on that page and fetch the linked records with one statement that joins `record_links` to the target records for those ids. Fill the lookup values from the result. One statement per lookup field per page.

A value in `data` under a computed field's name is never accepted from a caller. `write_record` overwrites it.

Tests:

1. Create a record with fields `budget` and `spent` and a formula `{budget} - {spent}`. The stored `data` holds the result. Update `spent`. The stored result changes.
2. A parent with a `Rollup` sum over children. Add a child: the parent's stored value rises and its `version` does not. Move the child to another parent: both parents are correct. Delete the child: the value falls.
3. A page of 200 rows with one `Lookup` field runs exactly two statements against `record_index` and `dataset_records` combined. Count them in the test.
4. A caller sending a value for a formula field has it replaced by the computed value.
5. 200,000 children across 1,000 parents. Updating one child's amount returns in under 200 ms, and the plans of its statements hold no `Seq Scan`.

## Phase 4 — totals are one statement

Add `aggregate_records` to the service layer, to `TOOLS` with scope `App(Read)`, and to REST as `GET /api/v1/apps/{slug}/tables/{table_id}/aggregate`.

| Argument | Meaning |
| --- | --- |
| `function` | `count`, `sum`, `avg`, `min`, `max` |
| `field` | Required unless `function` is `count`. Must be in the indexed set and stored in `num` |
| `filter` | The phase 2 indexed filter. The small-table path is not offered here |
| `group_by` | Optional. One field in the indexed set |

It runs one statement over `record_index`. With `group_by`, it returns at most 200 groups, largest first, and a flag when more exist. A field hidden from the caller by the `ferpa_sensitive` viewer rule cannot be a `field` or a `group_by`.

This is what a footer total, a kanban column count, and a metric card call. Do not compute any of them in the browser from a page of rows.

Tests:

1. `sum` of an indexed number over 200,000 rows equals the predicted value. The plan holds no `Seq Scan` on `dataset_records`.
2. `count` with `group_by` on a status field with five values returns five groups that add up to 200,000.
3. `sum` on a field outside the set is `BadRequest`.
4. A viewer asking for `sum` of a `ferpa_sensitive` field is refused.

## Phase 5 — search

Migration `0016_record_search.sql`:

```sql
ALTER TABLE dataset_records
    ADD COLUMN IF NOT EXISTS search tsvector
    GENERATED ALWAYS AS (jsonb_to_tsvector('simple', data, '["string", "numeric"]')) STORED;
CREATE INDEX IF NOT EXISTS idx_dataset_records_search ON dataset_records USING gin (search);
DROP INDEX IF EXISTS idx_dataset_records_data_gin;
```

The column is one fixed column for every app. PostgreSQL fills it. No code writes it.

`list_records` gains `search`, a string. It adds `AND r.search @@ plainto_tsquery('simple', $n)`. It combines with the default order and with an indexed sort.

Search reads every value in the record. A caller from whom the `ferpa_sensitive` viewer rule hides any field of that table may not search it. Return `BadRequest` with the text `Search is not available on this table for your role.` Otherwise a hidden value could be found by guessing it.

Tests:

1. 200,000 rows, one holding the word `zebrafish`. Search returns that row. The plan uses `idx_dataset_records_search`.
2. A viewer searching a table with a `ferpa_sensitive` field is refused. An editor is not.
3. `idx_dataset_records_data_gin` no longer exists.

## Phase 6 — the million-row proof

One test, `million_rows`, marked `#[ignore]`. It seeds 1,000,000 rows of thirty fields with five indexed, then runs each of these twenty times and prints the median and the slowest:

| Operation | Limit for the slowest of twenty |
| --- | --- |
| First page, default order | 100 ms |
| A page reached by cursor near the end | 100 ms |
| Sort by an indexed number, a page near the end | 200 ms |
| Indexed range filter returning 200 of 1,000 matches | 200 ms |
| `sum` over the whole table | 1,500 ms |
| `count` grouped by a five-value field | 1,500 ms |
| Search for a word in ten rows | 200 ms |
| Create one record with five indexed fields | 50 ms |
| Update one indexed field | 50 ms |

Each read also passes the `Seq Scan` check. Add a CI step that runs `cargo test --release -- --ignored million_rows`. If a limit fails on the CI runner and the plan is correct, do not raise the limit. Paste the plan and the timings and stop.

Paste the printed table into the session output. Those numbers replace the estimate in this brief.

## How to prompt Gemini

```
Read docs/plans/row-scale.md and docs/plans/README.md.
Implement phase N only.
Write the failing test first and run it.
Do not add a dependency.
Do not run ALTER TABLE, CREATE TABLE, or CREATE INDEX for a user's table or field.
Do not paste a caller's value or a field name into SQL text.
Do not filter, sort, count, or total records in Rust.
Stop when the tests listed for phase N pass, and paste the command output with the measured times.
```

## Enterprise objections

| Objection | Answer |
| --- | --- |
| This abandons the JSON model. | It does not. The manifest and the record are still JSON documents. PostgreSQL holds no column and no index named after a user's field. The three additions are fixed, and the pivot rows can be deleted and rebuilt from the records. |
| An entity-attribute-value table is a known anti-pattern. | It is one when it is the only copy of the data. Here the record is one JSONB document and the pivot is an index over it. Force.com has run this design in production since 2009. |
| Every write now costs several rows. | One row per indexed field, at most 20. Phase 6 measures a create with five at under 50 ms on a million-row table. |
| A user will sort on a field that is not indexed. | A field a view sorts or filters on is indexed without anyone asking. Any other field works up to 10,000 rows, and above that the error names the field to mark. |
| The index will drift from the records. | Both are written in one transaction by one function. `rebuild_index` restores a table's index from its records, and phase 2 test 2 proves it. |
| A rollup over a hundred thousand children will stall a save. | The aggregate runs in PostgreSQL over the pivot index, for the one parent the child points at. Phase 3 test 5 measures it. |
| Search will leak a hidden field. | A caller with any hidden field in a table cannot search that table. |
| A million rows is not our largest table. | Phase 6 states the measured size. Nothing here is specific to a million. The next limit is one PostgreSQL server, and that is measured before it is claimed. |
