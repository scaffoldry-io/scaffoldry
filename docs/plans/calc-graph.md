# Calculation graph — implementation brief

Hand this file to Gemini Flash. Do one phase per session. Stop when that phase's tests pass. Do not start the next phase.

Run order is `docs/plans/README.md`. Phase 1 replaces the formula work in `data-grid-parity.md` phase 3. Phase 2 needs `links.md` phase 3 and `row-scale.md` phase 3. Phases 3 and 4 need `jobs.md` phase 1.

A grant tracker has `budget`, `spent`, `remaining = {budget} - {spent}`, and `status = IF({remaining} < 0, "Over", "OK")`. A student table rolls up credits from linked courses. A date field shows days until a deadline. These values depend on each other. Today each is computed when its own record is written, with no declared order, no check that `remaining` and `status` do not depend on each other in a circle, and nothing that updates `days_left` tomorrow.

This brief makes calculation a graph: declared, ordered, checked for cycles, recomputed in the right order, and kept correct by scheduled and background work.

## Decisions already made

| Question | Answer |
| --- | --- |
| How many formula evaluators? | One, in Rust. The TypeScript evaluator is deleted. The browser asks the server for a preview |
| Is a formula allowed to loop or call out? | No. There is no loop, no I/O, and no regular expression engine. A formula is a bounded expression |
| What does a cycle do? | It is a validation failure that names the cycle in plain words. A proposal containing one is not stored |
| Where is a computed value kept? | In the record, written by the server. A caller cannot set it |
| When is a volatile value (today, now) recomputed? | By a nightly scheduled job, in the institution's time zone |

## What exists today

| Fact | Where |
| --- | --- |
| `evaluate_formula` splits on one operator with no precedence | `scaffoldry-engine/src/lib.rs` |
| A second splitter, `evaluateClientFormula`, exists in TypeScript | `apps/web/src/computedFields.ts` |
| `data-grid-parity.md` phase 3 replaces the splitter with a grammar, and keeps both implementations on shared vectors | `data-grid-parity.md` |
| Count, rollup, and lookup are written when a record changes | `row-scale.md` phase 3 |

## Out of scope

- Cross-row formulas that scan a table. A formula reads its own row, and linked values arrive through lookup, rollup, and count fields.
- User-defined functions.
- Regular expressions.
- Arbitrary precision decimals beyond the `Currency` field's fixed scale.

## Rules for every phase

1. Write the failing test first. Run it. Confirm it fails because the behavior is missing. Then write the code.
2. Do not add a dependency.
3. A formula is data. It is validated when an app is proposed, never at the first read.
4. A recalculation is a job when it can touch more than 200 records.

## Phase 1 — one evaluator

This phase replaces `data-grid-parity.md` phase 3. Do that phase's grammar, semantics, and shared vectors, with these changes.

1. The evaluator is Rust only. Delete `evaluateClientFormula` and every call to it in `apps/web/src`. Delete the shared-vector rule that kept two implementations equal. The vectors become the Rust test suite.
2. Add `POST /api/v1/formula/preview`: body `expression`, `table_id`, `app_slug`, and an optional record. App read access. It validates and evaluates, and returns the result or each problem with its position. Rate-limited to 20 a second per person. The formula editor in the builder calls it as the person types, debounced.
3. Extend the function set. Each function has a row in a table in the brief's session output, with its argument types, result type, and what it does with blank.

| Group | Functions |
| --- | --- |
| Logic | `IF`, `SWITCH`, `AND`, `OR`, `NOT`, `ISBLANK`, `BLANK` |
| Math | `ROUND`, `CEILING`, `FLOOR`, `ABS`, `MOD`, `MIN`, `MAX` |
| Text | `CONCAT`, `LEN`, `LEFT`, `RIGHT`, `MID`, `UPPER`, `LOWER`, `TRIM`, `SUBSTITUTE`, `FIND` |
| Date | `TODAY`, `NOW`, `DATEADD`, `DATEDIFF`, `YEAR`, `MONTH`, `DAY`, `WEEKDAY`, `FORMAT` |
| List | `ARRAYJOIN`, `ARRAYUNIQUE`, `COUNT`, `SUM`, `AVERAGE` over a lookup or multi-valued field |

4. Limits. An expression has at most 500 characters and 200 nodes. Evaluation is bounded by node count. A text result is cut at 100,000 characters. `Currency` arithmetic is exact at the field's scale.
5. Errors. A formula that fails on one record yields a blank, and the record is not blocked. The field keeps `last_error` counts per table so an owner sees `Status failed on 14 records.` A validation error is reported before the formula can be saved.

Tests, in the existing engine test files.

1. Every vector from `data-grid-parity.md` phase 3 passes in Rust. Add 10 for each new function group, including blank inputs.
2. `grep -rn evaluateClientFormula apps/web/src` prints nothing. Paste the output.
3. A formula of 201 nodes is refused. One of 501 characters is refused.
4. The preview route returns the same value as saving the formula and reading the record. It returns a position for a syntax error.
5. Precedence: `{a} + {b} * {c}` and `-{a} - -{b}` give the right results.
6. Division by zero and a non-numeric operand give blank, not an error string.
7. Web: the formula editor shows the preview result, and shows a message with a caret for a syntax error.

## Phase 2 — the graph

Build the graph from the manifest. Nodes are `(table, field)`. Edges:

| From | To | When |
| --- | --- | --- |
| a field named in `{...}` | the formula field | Same table |
| a link field | a lookup, rollup, or count field | The computed field goes through that link |
| the target field of the link | the lookup or rollup field | A different table |
| the field a default or constraint reads | that field | Phase 4 |

`build_graph(manifest) -> Result<Graph, CycleError>`. `CycleError` carries the path. Its plain text is `Status depends on Remaining, which depends on Status.` The manifest validator calls it, and a cycle is a failed `schema` check.

`Graph` provides a topological order per table, and `affected(changed_fields) -> Vec<NodeId>`: everything downstream of a change in the order they must run.

On a record write, `write_record` evaluates the affected same-row fields in order and stores them in the same transaction. For each affected cross-table node, it finds the owner records through `record_links` and recomputes them. Up to 200 owners are recomputed inline. More than that enqueues the job `recalc_owners` with the changed record and fields. A computed change does not raise the owner's `version`, and is recorded in neither history nor the realtime feed as a user change.

This replaces the per-write rules in `row-scale.md` phase 3. That phase's tests still pass.

Tests.

1. A manifest where `a = {b}` and `b = {a}` fails validation with the plain sentence. A manifest where `a = {b}`, `b = {c}` is ordered `c`, `b`, `a`.
2. A write to `c` updates `b` then `a` within the one transaction.
3. A parent rollup updates when a child changes, in the order parent field, then the parent's formulas that read the rollup.
4. A parent with 500 children linked to it updates its children's lookups through the job, not inline. Below 200 it is inline.
5. A computed change does not change a version number and does not appear in history.
6. `affected` for a change to a leaf returns exactly its descendants.
7. A manifest with a lookup through a link that is later removed fails validation.

## Phase 3 — when the definition changes

Changing a formula, a field type, a link, or a rollup function changes values already stored. That is a job.

1. Add `FieldSpec.calc_state: ready | calculating`, kept outside the proposal in a small table `calc_status` (add it to migration `0046_calc_status.sql`: `app_slug`, `table_id`, `field`, `state`, `job_id`).
2. Approving a proposal that changes a computed field's definition writes the manifest, marks the field `calculating`, and enqueues `recalc_field` with a `dedupe_key`. The job works in keyset batches of 1,000, evaluates, and writes through `write_record` with `actor_kind` of `system`, which records nothing in history for computed fields. It checkpoints the last record id and sets `ready` at the end.
3. A read of a `calculating` field returns the stored old value and an extra response key, `field_states: { "status": "calculating" }`. The grid shows a spinner in the column header and the words `Calculating, 63% done`.
4. A filter or sort on a `calculating` field is refused with error code `calculating`. Add that code to the closed list in `ux-standards.md`.
5. A write during the job evaluates the new definition immediately, so the job can skip a record whose stored `computed_at` is newer than the job's start.

Field type conversion. Changing a field's type is a proposal with a dry run. `POST /apps/{slug}/fields/convert-preview`, owner or admin, body table, field, and the new type. It enqueues `convert_preview`, which counts how many values would convert, how many would become blank, and returns the first 50 examples of each, ids only plus the old value. The proposal's flagged check `data_loss` is raised when any value would become blank, and the preview result is attached to it. Approval enqueues `convert_field`, which converts in batches, writes the old value of every changed record into `record_history` as `actor_kind` of `system`, and so can be undone as a batch.

Tests.

1. Changing a formula on a 200,000-row table: the approval returns in under one second, the field shows `calculating`, and after the job every value matches the new formula.
2. A filter on a `calculating` field is `calculating`.
3. A write during the job is correct and is not overwritten by the job.
4. Kill the worker and resume: no record is skipped or computed twice with different results.
5. A preview converting text to number reports the count of non-numeric values and 50 examples. Approval nulls them, and history holds the old values.
6. Undoing a conversion batch restores the old values.

## Phase 4 — time, defaults, and constraints

Volatile formulas. A formula that uses `TODAY()` or `NOW()` is volatile and is marked so at validation. A scheduled job, `recalc_volatile`, runs nightly at 00:05 in `platform.timezone`, a new setting, an IANA zone name defaulting to `UTC`. It recalculates each volatile field in batches. A volatile field is refused on a table over 500,000 rows, with a message that says why. Add the setting to the closed list in `foundation.md` phase 8.

Defaults. `FieldSpec.default` is a literal or an expression, applied when a record is created and the field is not supplied. `FieldSpec.set_on` for a field may be `Create`, or `Change { fields }`, which sets an expression when any of the listed fields change, such as `status_changed_on = NOW()` when `status` changes. These are not computed fields: a person can overwrite them afterwards unless the field is read-only.

Constraints. `FieldSpec.constraints`: `required`, `unique`, and for number fields `min` and `max`. Select values are already limited to their options. A `unique` check runs in the write transaction after taking `pg_advisory_xact_lock` on a hash of the app, table, field, and value, then looks for a clash in `record_index` for an indexed field. A `unique` field is added to the indexed set. A violation is `400` with error code `constraint_violated` and the field and constraint named. Add that code to the closed list. A proposal adding `unique` to a field that already holds duplicates is flagged `data_loss` with the count and 50 examples, computed by a preview job.

Tests.

1. A `TODAY()` field is recomputed by the nightly job and not by anything else. Setting the time zone shifts the run.
2. A volatile field on a 600,000-row table is refused.
3. A default fills a missing value on create and does not override a supplied one.
4. `status_changed_on` changes only when `status` changes.
5. Two concurrent writes of the same `unique` value: exactly one succeeds.
6. A `min` violation is `constraint_violated` naming the field.
7. Adding `unique` to a field with 3 duplicate values raises `data_loss` and shows examples.

## How to prompt Gemini

```
Read docs/plans/calc-graph.md, docs/plans/data-grid-parity.md phase 3, and docs/plans/README.md.
Implement phase N only.
Write the failing test first and run it.
Do not add a dependency.
Do not keep a second formula evaluator.
Do not let a cycle through validation.
Do not read, copy, or adapt another product's source code.
Stop when the tests listed for phase N pass, and paste the command output.
```

## Enterprise objections

| Objection | Answer |
| --- | --- |
| Formulas will be slow at a million rows. | A formula reads one row and is bounded. Heavy recalculation is a job with progress, and a field shows that it is calculating. |
| An agent will write a formula that never ends. | There is no loop, and evaluation is bounded by node count. |
| A cycle will corrupt values. | A cycle cannot pass validation, and the message names it in words. |
| Changing a type will lose data. | The proposal shows a dry run with counts and examples, it is flagged for a second person, and every conversion is undoable from history. |
| A volatile field will recompute constantly. | Only once a night, in the institution's time zone, and not at all on very large tables. |
| Two evaluators will disagree. | There is one. |
