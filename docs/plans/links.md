# Record links — implementation brief

Hand this file to Gemini Flash. Do one phase per session. Stop when that phase's tests pass. Do not start the next phase.

Run order is `docs/plans/README.md`. Phase 1 needs `mcp-apps.md` phase 3, `row-scale.md` phases 1 and 2, and `record-history.md` phase 1. Phase 5 needs `jobs.md` phase 1. This brief changes `row-scale.md` phase 3.

A course points at its instructor. A grant points at its principal investigator. An application points at a program. Today a table is joined to another by matching the text in one field to the text in another. Rename a program code and every application that held the old text points at nothing, and no error says so. Two programs with the same code give an application two parents. A person cannot click from an application to its program, and a program cannot show its applications.

This brief makes a link a first-class thing: a record points at another record by its identity.

## Decisions already made

| Question | Answer |
| --- | --- |
| What is the truth? | The record's own JSON, which holds the linked record ids. `record_links` is an index derived from it, written in the same transaction, and can be rebuilt |
| Are links two-way? | By default, yes. A link field has a related field on the other table, created in the same proposal |
| Does updating the other side change that record's version? | No. A mirrored change is derived, like a rollup, and must not make someone's save conflict |
| What happens when a linked record is deleted? | By default the link is removed and remembered, so restoring the record restores the link. A field may instead refuse the delete while links exist |
| Can a link cross apps? | Only to a dataset app, which is read-only (`connections.md`) |
| Does a link respect access rules? | Yes. A person can link only to records they can see, and a lookup never returns a hidden field |

## What exists today

| Fact | Where |
| --- | --- |
| `TableRelationship` joins `source_field` to `target_field` by value | `scaffoldry-engine/src/lib.rs` |
| A `Relation` field names `linked_dataset_id` and `linked_field` | `FieldSpec` |
| `row-scale.md` phase 3 finds linked records through the index by value | `row-scale.md` |
| No `record_links` table exists | |

## Out of scope

- Links to records in another ordinary app.
- Links with their own fields (a join record). Make that a table with two link fields.
- Ordered lists of links with drag-to-reorder.
- A graph view.

## Rules for every phase

1. Write the failing test first. Run it. Confirm it fails because the behavior is missing. Then write the code.
2. Do not add a dependency.
3. A link and its mirror are written in the transaction of the record that changed.
4. A link never reveals a record the person could not open.
5. Use the kit for every screen.

## Phase 1 — the link field and its index

Migration `crates/scaffoldry-core/migrations/0031_record_links.sql`:

```sql
CREATE TABLE IF NOT EXISTS record_links (
    app_slug VARCHAR(64) NOT NULL,
    field VARCHAR(64) NOT NULL,
    from_record VARCHAR(64) NOT NULL,
    to_record VARCHAR(64) NOT NULL,
    ord SMALLINT NOT NULL DEFAULT 0,
    PRIMARY KEY (from_record, field, to_record)
);
CREATE INDEX IF NOT EXISTS idx_record_links_reverse ON record_links (to_record, field);
```

Add `FieldType::Link` and in `FieldSpec`: `link_target_table: Option<String>`, `link_target_app: Option<String>` (dataset apps only), `link_cardinality: One | Many`, `link_related_field: Option<String>`, `link_on_delete: Unlink | Restrict`. The record value is a list of record ids, or one id.

`write_record` validates each id exists in the target table and is visible to the writer, enforces cardinality (`One` accepts at most one), and writes `record_links` rows in the same transaction. A missing or invisible target is `400` with error code `bad_request` and the field named. The same function removes link rows for ids no longer in the value.

The manifest validator checks: the target table exists, a `link_related_field` names a link field on the target that points back, and `link_target_app` is a dataset app.

History records the id list change. The `visible_changes` function shows display names, not ids.

Tests.

1. Create an application linking to a program. `record_links` holds one row. Change the link. The old row is gone and the new one is present.
2. A `One` field refuses two ids. A missing id is `400` naming the field. An id from another table is `400`.
3. A link to a record the writer cannot see is refused the same way as a missing one.
4. The link list in the record's JSON and `record_links` always agree after a mixed sequence of writes. `rebuild_links(app, table)` from the JSON reproduces the table exactly.
5. The validator refuses a related field that does not point back.
6. At 200,000 links, finding a record's links and its reverse links each uses an index and has no sequential scan.

## Phase 2 — two-way, and what delete does

1. A `Link` field with `link_related_field` has a mirror field on the target table. The proposal that adds the field adds both. The validator requires both or neither.
2. Writing one side updates the other side's JSON and `record_links` in the same transaction. The mirror write records a history entry of `actor_kind` `system` with the batch of the user's change, and does not raise the version.
3. Deleting a record (`lifecycle.md` moves it to trash) removes every link to and from it, and stores the removed links with the trash entry. Restoring the record restores those links that still point at live records.
4. `link_on_delete: Restrict` refuses the delete with `409` and a count: `3 applications still link to this program.`
5. Many-to-many is two `Many` fields that are each other's related field.

Tests.

1. Linking an application to a program shows the application in the program's mirror field.
2. Unlinking from the program side removes it from the application.
3. The mirror write does not change the other record's version.
4. Deleting a program removes the links and restoring it brings them back. Restoring after an application was also deleted restores only the link to the live application.
5. A `Restrict` field refuses the delete and reports the count.
6. A many-to-many pair keeps both sides in step across 50 random changes.

## Phase 3 — lookup, rollup, count

Lookup, rollup, and count fields go through a link field, by name: `computed_via: <link field>`. The validator requires it. They use `record_links`, not the by-value join.

1. A lookup collects the target field's values for the linked records. A rollup applies `sum`, `avg`, `min`, `max`, `count`, or `count_distinct` over a numeric target field in SQL. A count is the number of links.
2. A rollup at write time is computed by one aggregate statement over the link rows and the target values in `record_index`. It follows the dependency order from `calc-graph.md`.
3. The target field must be indexed. The validator adds it to the indexed set.
4. A lookup through a link never returns a field that is hidden for the person (`access-rules.md`). The serializer drops it.
5. Replace the by-value rule in `row-scale.md` phase 3 with this one. Its tests stay and now use links. `TableRelationship` is deprecated and the validator warns about it until phase 5 converts existing apps.

Tests.

1. A parent with three children shows the sum of their amounts. Adding, changing, moving, and removing a child each updates the sum.
2. A lookup of `title` through a link on a page of 200 rows runs a fixed number of statements, independent of the page size. Count them in the test.
3. A hidden target field is absent from a lookup for a restricted person.
4. A rollup over 200,000 children of 1,000 parents updates one parent in under 200 ms.
5. The plans for the aggregate and the lookup have no sequential scan.

## Phase 4 — choosing a record to link

`GET /apps/{slug}/tables/{table_id}/link-options?q=&cursor=&limit=`. App read access. It searches the target table's primary field and its first three text fields, within the person's `row_scope`, paged to at most 50, and returns `{ id, label, secondary }` where `secondary` is a second field chosen by the target table's `primary_field` setting. It returns no other fields. For a dataset target it returns the same.

The cell editor and the record drawer use it. The picker is a kit combobox: keyboard operable, announces results, shows `No matches` with the query, and shows the first page without a query. A link renders as chips with the label, and clicking a chip opens the target record's drawer when the person can read it. A chip for a record they cannot open shows the label without a link.

Tests.

1. A restricted person's picker lists only records in their scope.
2. The response holds only `id`, `label`, and `secondary`.
3. A query matches the primary field case-insensitively and pages.
4. Web: the combobox is operable by keyboard, announces the result count, and selecting adds a chip.
5. A chip for an unreadable record has no link.

## Phase 5 — converting value joins

A job, `convert_relation_to_link`, for apps that used `TableRelationship`. `POST /apps/{slug}/relations/{id}/convert-preview` (owner or admin) enqueues a dry run. It reports, for each source record, one of: matched exactly one target, matched none, matched several. It returns counts and 50 examples of each unmatched and ambiguous case, with ids. The conversion is a proposal that adds the link fields and sets the data. It is flagged `data_loss` when any record is unmatched or ambiguous. Approval enqueues the real conversion, which links the matched, leaves the others blank, and writes the old value into history so it is not lost.

Tests.

1. A fixture of 1,000 applications with 970 matches, 20 with no program, and 10 with a duplicate code: the preview reports exactly those counts.
2. Approval links 970 and leaves 30 blank. History holds the old text for the 30.
3. The converted app passes the validator with no `TableRelationship` warning.
4. A converted app's lookups equal what the old join returned for the 970.

## Phase 6 — in the grid

Web only. A link column in the grid shows chips, edits with the picker, supports paste of labels (resolved by exact match on the primary field within scope, with a summary of unmatched labels, never guessed), and supports sorting and filtering by the target's primary field through the index. A record drawer shows a linked records section: the target's fields as a mini table, with `Open` and `Unlink`. The mirror field on the other table is shown the same way.

Tests (vitest, `fetch` stubbed).

1. Pasting three labels resolves two and reports one as not found in a `Banner`.
2. Unlink sends the update and the chip disappears.
3. A link cell's keyboard path: Enter opens the picker, arrows move, Enter selects, Escape closes and returns focus to the cell.

## How to prompt Gemini

```
Read docs/plans/links.md and docs/plans/README.md.
Implement phase N only.
Write the failing test first and run it.
Do not add a dependency.
Write the link and its mirror in the same transaction as the record.
Do not let a person link to a record they cannot see.
Do not read, copy, or adapt another product's source code.
Stop when the tests listed for phase N pass, and paste the command output.
```

## Enterprise objections

| Objection | Answer |
| --- | --- |
| Links stored twice will drift. | The record's JSON is the truth and the table is derived. A rebuild reproduces it, and a test compares them after random writes. |
| A mirror write will cause conflicts. | A mirrored change does not raise the version. |
| Linking will expose records a person should not see. | The writer can link only to what they can see, and a lookup never returns a hidden field. |
| Existing apps will break. | The old join keeps working with a warning until a conversion preview shows what would change, and a person approves it. |
| Deleting a record will leave dangling links. | Links are removed on delete and remembered, so restore brings them back, or the delete is refused when the field says so. |
