# Data grid parity — implementation brief

> **Status: phases 1 to 6 built, with the deviations in "Checked on 2026-11-19" and "Phase 6 — close the gaps". Phase 3 built with one parser. Phase 6 added column resize, Shift+click multi-sort and the shared grid in the builder data tab.**  
> **Test Coverage:** `apps/web/src/test/data-grid.test.tsx` (phases 1, 2, 4, 6), `apps/web/src/test/builder-usability.test.tsx` (phases 5, 6), `crates/scaffoldry-engine/tests/computed_fields_test.rs` (phase 3).


Hand this file to Gemini Flash. Do one phase per session. Stop when that phase's tests pass. Do not start the next phase.

Run order is `docs/plans/README.md`. The grid is a view of one app. The app sits in a workspace. The workspace sits in one organization unit. Do not add org controls to `DataGrid`. Do not start phase 4 of this plan until organization phase 1 has given `workspaces.organization_id` a value.

`docs/plans/row-scale.md` binds this plan. The grid holds one page of rows, not the table. Sort and filter are sent to the server and the grid reloads from the first page. A footer summary calls the aggregate route from row-scale phase 4. Do not sort, filter, or total in the browser. Where a phase below says otherwise, this paragraph wins.

`docs/plans/calc-graph.md` phase 1 replaces this plan's formula work: one evaluator in Rust and none in TypeScript, so rule 6 below, which keeps two implementations equal, no longer applies. `docs/plans/views.md` moves view settings (column order, width, hidden, frozen, sort, filter, footer) off the manifest into `app_views`, and its phase 3 covers windowed scrolling. Where this plan says a view setting is stored on `AppView` in the manifest, read it as `app_views`.

The grid is the web desk's main view. The agent is the primary surface of Scaffoldry. A department chair who would otherwise open Airtable or Smartsheet must be able to change columns, edit many cells, and calculate across a row without leaving the grid.

This is not a clone of those products. Scaffoldry keeps their grid behavior and drops their project-management and marketplace surface.

## What already exists

Do not replace these. Extend them.

| Fact | Where |
| --- | --- |
| Field types, filters, sorts, formula/lookup/count/rollup | `apps/web/src/types.ts` `FieldSpec`, `AppView` |
| Same model in Rust | `crates/scaffoldry-engine/src/lib.rs` `FieldType`, `AppView`, `evaluate_formula`, `compute_field_value` |
| Client formula and CSV | `apps/web/src/computedFields.ts` |
| Live grid is a plain `<table>` with one text/number editor | `apps/web/src/MultiViewWorkspace.tsx` around the grid branch |
| Richer field editing lives in the builder, separate from that grid | `apps/web/src/AppBuilder.tsx` |
| Record create/update API already exists | `crates/scaffoldry-server/src/service/records.rs` |
| `@tanstack/react-table` is installed and unused | `apps/web/package.json` |

`evaluate_formula` (Rust) and `evaluateClientFormula` (TypeScript) are the same weak splitter: one operator, no precedence, no functions. `{budget} - {spent}` only works because `-` is checked last. Do not patch the splitter. Replace it in phase 3.

## Parity target

In scope, in this order:

1. Type-aware cell editing and keyboard movement.
2. Rectangle selection, copy, paste, clear, fill-down, one undo batch.
3. Column chrome on a view: resize, reorder, hide, freeze, sort, footer summary.
4. Column schema in the builder: add, rename, change type, delete, set the formula. One formula per field, applied to every row.
5. A real expression language for that formula, evaluated per row.

Out of scope. Do not build these, and do not add placeholders for them:

- Gantt, predecessors, dependencies, resource leveling, baselines.
- Per-cell formulas, Excel `A1` references, cross-sheet references, `ARRAYFORMULA`.
- Attachments, comments, proofs, buttons, barcodes, collaborator pickers.
- Automations, forms, interfaces, or a second schema editor. Workflows and forms already exist.
- Any new npm or crate dependency. Forbidden in particular: AG Grid, Handsontable, Glide Data Grid, HyperFormula, DataFusion. HyperFormula is not on the license whitelist. DataFusion is named in `docs/ARCHITECTURE.md` and is not in this pass; calculation stays in `evaluate_formula`.
- `ALTER TABLE` for a user column. User fields stay on the manifest and in record JSON. Architecture rule 4 in `docs/ARCHITECTURE.md`.
- A Cedar check inside the React component. Authorization stays on the API.

## Rules for every phase

1. Write the failing test first. Run it. Confirm it fails because the behavior is missing. Then write the code.
2. Do not add a dependency. Use `@tanstack/react-table`, which is already installed, for the phase 1 grid. Do not wrap it in a second table framework.
3. Do not virtualize. Do not add `@tanstack/react-virtual`. Revisit only after a grid is measured above 2,000 rows.
4. View chrome and field schema are different writes.
   - View chrome (order, width, hidden, frozen, sort, filter, summary, density) is stored on `AppView`. It does not change the field list.
   - Field schema (add, rename, type, delete, formula, select options) mutates `AppTable.fields` in the builder's in-memory manifest. It does not call SQL.
5. Computed fields are not editable: `Formula`, `Lookup`, `Count`, `Rollup`, `Autonumber`, `CreatedTime`, `LastModifiedTime`.
6. Keep both formula implementations on the vectors in phase 3. Rust is the source of truth. If a vector passes in Rust and fails in TypeScript, the phase is not done.
7. Match the surrounding file's style. Do not reformat unrelated code. Do not rename existing tests.

## Formula grammar (phase 3 only)

Airtable shape, which this repo already uses. A formula reads the current row. It does not read other rows. Totals across rows are the footer summary or a `Rollup` field.

```
expr    = or
or      = and ("OR" and)*
and     = not ("AND" not)*
not     = "NOT" not | cmp
cmp     = add (("=" | "!=" | ">" | ">=" | "<" | "<=") add)?
add     = mul (("+" | "-") mul)*
mul     = unary (("*" | "/") unary)*
unary   = "-" unary | primary
primary = number | string | "{" field "}" | call | "(" expr ")"
call    = NAME "(" [expr ("," expr)*] ")"
field   = letters, digits, underscore
NAME    = IF | AND | OR | NOT | ROUND | ABS | CONCAT | LEN | BLANK | ISBLANK
```

Semantics:

- `{field}` is the current record only. Missing field yields blank, not an error string.
- Blank is JSON null in Rust and `null` in TypeScript.
- `+` concatenates when either side is a string. Otherwise it adds numbers.
- `-`, `*`, `/` are numeric. Non-numeric operand or division by zero yields blank.
- Comparisons are numeric when both sides are numbers, otherwise case-sensitive strings. Blank equals blank.
- `IF(cond, a, b)`: blank, `0`, and `""` are false. Anything else is true.
- `AND` / `OR` take one or more arguments. `NOT` takes one.
- `ROUND(n, digits)` digits default 0. `ABS` takes one number.
- `CONCAT` takes one or more values and stringifies them. Blank contributes nothing.
- `LEN` takes one value. `ISBLANK` takes one value. `BLANK()` takes none.
- No other function. An unknown function yields blank.
- Reject an expression longer than 500 characters, or a parse tree deeper than 32, by returning blank.
- Operators and function names are case-insensitive. Field names are case-sensitive.

Footer summary is not a formula. It is a reduce over the visible rows of one column: `none | sum | avg | min | max | count`.

## Shared formula vectors

Both `crates/scaffoldry-engine/tests/computed_fields_test.rs` and `apps/web/src/test/builder-usability.test.tsx` must assert these against the record `{budget: 500000, spent: 120000, rate: 0.2, first_name: "Ada", last_name: "Lovelace", title: ""}`.

| Expression | Result |
| --- | --- |
| `{budget}` | 500000 |
| `{budget} * 0.20` | 100000 |
| `{budget} / 10` | 50000 |
| `{budget} - {spent}` | 380000 |
| `{first_name} + " " + {last_name}` | `Ada Lovelace` |
| `{budget} - {spent} * {rate}` | 476000 (multiplication before subtraction) |
| `({budget} - {spent}) * {rate}` | 76000 |
| `IF({spent} > 100000, {budget} - {spent}, 0)` | 380000 |
| `IF(ISBLANK({title}), "untitled", {title})` | `untitled` |
| `ROUND({budget} * {rate}, 0)` | 100000 |
| `AND({spent} > 0, {budget} > {spent})` | true |
| `{missing}` | blank |
| `{budget} / 0` | blank |

Keep the four assertions already in `test_formula_evaluation`. Add the new ones beside them. Do not weaken the string-concatenation assertion.

## Phase 1 — one grid, type-aware edit

Create `apps/web/src/DataGrid.tsx`. Render it from the grid branch of `MultiViewWorkspace.tsx`. Delete that branch's `<table>`. Leave kanban, calendar, gallery, and form alone.

Props, no more:

```ts
type CellAddr = { recordId: string; fieldName: string };

type DataGridProps = {
  fields: FieldSpec[];
  records: Record<string, unknown>[];
  rowDensity?: RowDensity;
  onPatch: (recordId: string, fieldName: string, value: unknown) => void;
};
```

`onPatch` is the existing `handleUpdateCell`. Do not call `fetch`.

Behavior:

- Build the table with `useReactTable`. Column id is `field.name`.
- Click a cell to focus it. A focused editable cell shows a native control:
  - Text, Email, Phone, Url: `<input type="text">` (email/tel/url types).
  - Number, Currency, Percent: `<input type="number">`. Display currency with `currency_symbol` and `precision` when not editing. Percent displays as the stored number plus `%`.
  - Date: `<input type="date">`.
  - Checkbox: `<input type="checkbox">`. Click toggles and calls `onPatch` immediately. No text editor.
  - Select: `<select>` of `select_options`.
  - MultiSelect: a popover of checkboxes. Commit on close.
  - Rating: five buttons, values 1–5.
  - Relation: read-only text of the stored id.
  - Computed types listed above: read-only. Click focuses but does not open an editor.
- Enter or F2 edits the focused cell. Enter commits and moves down. Shift+Enter moves up. Tab commits and moves right. Shift+Tab moves left. Escape cancels without calling `onPatch`.
- Arrow keys move focus when not editing.
- A printable key on a focused single cell replaces its value and opens the editor (type-to-replace). Not on Checkbox, Select, MultiSelect, Rating, or read-only types.
- FERPA badge stays on the header when `ferpa_sensitive` is true. Do not mask the value. The user already passed the API to see the row.

Tests in `apps/web/src/test/data-grid.test.tsx` (Testing Library, same setup as `builder-usability.test.tsx`):

1. Number cell commits on Enter and ignores Escape.
2. Checkbox toggles without an input of type text.
3. Select lists `select_options`.
4. Formula cell has no textbox.
5. Tab moves focus to the next field.

## Phase 2 — rectangle, paste, clear, fill, undo

Extend `DataGrid` only. Do not change the formula module.

Selection is `{ anchor: CellAddr, focus: CellAddr }`. The rectangle is every visible row between the two record ids and every visible field between the two field names, in on-screen order.

- Shift+click and Shift+arrow move `focus` and keep `anchor`.
- A plain click sets both to that cell.
- Delete and Backspace call `onPatch` with `""` for every editable cell in the rectangle. One user action, many patches.
- Ctrl+D or Cmd+D copies the top editable cell of each selected column down the rectangle.
- Ctrl+C or Cmd+C writes TSV: rows separated by `\n`, cells by `\t`, no header row.
- Ctrl+V or Cmd+V reads TSV from the clipboard. Paste origin is `focus`. Walk down rows and right across fields. Skip read-only fields. Do not create rows. Do not use `parseCsv` unless the clipboard has no tab.
- Typing does not change a multi-cell selection. It only type-to-replaces when anchor equals focus.
- Ctrl+Z undoes the last gesture (one paste, one clear, or one fill). Ctrl+Shift+Z redoes it. Stack depth 50. Session memory only. A single-cell `onPatch` is also one gesture.

Tests:

1. Shift+arrow selects two cells. Delete calls `onPatch` twice.
2. Paste of `10\t20` writes those two fields on the focus row.
3. Paste does not write a Formula field.
4. Ctrl+Z restores the pre-paste values.

Clipboard in jsdom: stub `navigator.clipboard`.

## Phase 3 — formula language

Touch only:

- `crates/scaffoldry-engine/src/lib.rs` `evaluate_formula` and `parse_operand`
- `apps/web/src/computedFields.ts` `evaluateClientFormula` and `parseOperand`
- the two test files named above

Replace the splitter with a recursive descent parser of the grammar above. `compute_field_value` stays as it is. Lookup, Count, and Rollup are unchanged.

Do not add a function that sums a column. Do not evaluate formulas in the React render by copying a third parser. The grid displays the value `computeFieldValue` already returns.

Run:

```
cargo test -p scaffoldry-engine --test computed_fields_test
cd apps/web && npx vitest run src/test/builder-usability.test.tsx
```

## Phase 4 — column chrome

Extend `AppView` in both languages. Every new field is optional and `#[serde(default)]`.

```rust
pub column_order: Vec<String>,          // field names; empty means manifest order
pub column_widths: Vec<(String, u32)>,   // px, ignore unknown names
pub hidden_columns: Vec<String>,
pub frozen_through: Option<String>,      // freeze this field and those before it
pub column_summary: Vec<(String, String)>, // "sum"|"avg"|"min"|"max"|"count"
```

Mirror that on the TypeScript `AppView` interface.

In `DataGrid`, add optional props for those five values plus `onViewChange(patch)`.

Header menu, one per column:

- Sort ascending, sort descending. Replaces `sort_rules` with one rule. Shift+click appends a rule instead. Reuse `applyMultiSort`.
- Hide.
- Freeze through this column.
- Summary submenu for Number, Currency, and Percent only.

Drag the header to reorder. Drag the header's right edge to resize. Minimum width 72. Persist through `onViewChange`. Do not write these into `fields`.

Footer uses `column_summary`. Default when the array is empty: `sum` for Number and Currency, nothing otherwise. This replaces the hardcoded SUM/AVG block in `MultiViewWorkspace`.

Hidden fields are not in the copy rectangle.

Tests:

1. Hiding a field removes its header.
2. `column_order` renders headers in that order.
3. Summary `min` on a number column shows the minimum of the visible rows.

## Phase 5 — column schema in the builder

Do this only in `AppBuilder.tsx`, on the data-tab grid. The published workspace grid does not add or delete fields.

Header menu gains, builder only:

- Rename. Changes `label`. Changes `name` only when no record has a non-empty value for the old name. Otherwise keep `name` and tell the user the key is kept.
- Change type among Text, Number, Date, Select, Checkbox, MultiSelect, Currency, Percent, Rating, Email, Phone, Url, Formula. Do not offer Lookup, Count, Rollup, or Relation here. Those need a target table and stay on the existing field form.
- Edit formula. Sets `formula_expression` on a Formula field. Show the expression under the header.
- Insert field left, insert field right. New field is Text, name `field_<n>` where n is the field count plus one, label `Field N`.
- Delete field. Block deletion of the primary field. Drop the key from in-memory records of that table.

Changing to Formula clears stored cell values for that key on the in-memory records so the computed value shows.

Do not open a modal library. Use a small popover in the header, the same pattern as the phase 4 menu.

Tests: extend `builder-usability.test.tsx`. One test that inserts a field and one test that a Formula field renders `evaluateClientFormula` output in the builder grid.

## How to prompt Gemini

Paste this, with the phase number filled in:

```
Read docs/plans/data-grid-parity.md and docs/ARCHITECTURE.md section 4 and section 7.
Implement phase N only.
Write the failing test first and run it.
Do not add dependencies.
Do not edit phases other than N.
Stop when the tests listed for phase N pass, and paste the command output.
```

## Enterprise objections

| Objection | Answer |
| --- | --- |
| A browser grid will leak FERPA fields into exports. | Copy and CSV already export only visible columns. Hidden columns are excluded in phase 4. Policy still runs on `update_record`, not in the component. |
| User-added columns will migrate the database. | They are `FieldSpec` entries on the manifest. Record bodies stay JSON. No DDL. |
| A formula language is an injection hole. | The grammar has no calls to the host, no loops, and a 500-character / depth-32 cap. Unknown functions return blank. |
| Two parsers will drift. | There is one parser, in Rust. The builder previews a formula through the server. No browser parser exists to drift. |
| TanStack Table cannot be the product grid. | It is already the architecture choice and already installed. A second grid library violates the license and dependency rules. |


## Checked on 2026-11-19

The earlier completion report overstated the work. It named a browser `evaluateClientFormula` that does not exist, a drag/fill handle, a filter popover, and only one test file. This check reads the code and runs the tests: `data-grid.test.tsx` and `builder-usability.test.tsx` pass 57 of 57, and `computed_fields_test` passes 11 of 11.

| Phase | State | Evidence |
| --- | --- | --- |
| 1. One grid, type-aware edit | Built. `DataGrid` has more props than the brief lists | `MultiViewWorkspace.tsx`, `DataGrid.tsx`, `data-grid.test.tsx` tests 1–5 |
| 2. Rectangle, paste, clear, fill, undo | Built. Undo is a server batch undo, not a browser stack | `DataGrid.tsx`, `data-grid.test.tsx` |
| 3. Formula language | Built in Rust with the 13 shared vectors. There is no browser parser | `crates/scaffoldry-engine/src/formula.rs`, `computed_fields_test.rs`, `apiClient.previewFormula` |
| 4. Column chrome | Partly built. Hide, freeze, reorder, one-rule sort and the footer summary work. Column resize and Shift+click to add a sort rule are missing | `DataGrid.tsx`, `data-grid.test.tsx` tests 1–3 |
| 5. Column schema in the builder | Built and tested. The builder data tab draws its own table, not `DataGrid` | `AppBuilder.tsx`, `builder-usability.test.tsx` |

Two deviations are accepted. They are simpler and safer than the brief.

- **Undo goes to the server.** A batch id undoes a paste or fill within the undo window. It works across instances and survives a reload. A browser stack would not.
- **One formula parser.** The builder asks the server to preview a formula. Phase 3's "both test files assert the same vector table" no longer applies, because there is no second parser.

## Phase 6 — close the gaps

Needs phases 1 to 5 as checked above.

1. Column resize. A drag handle on each header edge. The width never goes under 72 pixels. Releasing calls `onViewChange` with the new `column_widths`, once.
2. Shift+click on a sort entry in the column menu adds a rule to `sort_rules` instead of replacing them. Sorting stays on the server.
3. The builder data tab renders `DataGrid`, not its own table, and keeps the phase 5 schema actions in the column menu. If a phase 5 action cannot move without losing behavior, keep the builder table and write the reason here.

Tests, written first:

1. Dragging a header edge to 40 pixels stores 72. One `onViewChange` call carries the width.
2. Shift+click on a second column's sort entry gives two `sort_rules` in order. A plain click gives one.
3. The builder data tab renders `DataGrid`. The phase 5 tests in `builder-usability.test.tsx` still pass unchanged.

### Built on 2026-11-19

All three items are done, test-first. Tests: `data-grid.test.tsx` 22 of 22, `builder-usability.test.tsx` 39 of 39 (the five phase 5 tests pass unchanged), full suite 735 of 735.

1. Column resize. Each header has a drag handle on its right edge (`role="separator"`, `data-testid="resize-handle-<field>"`). Dragging draws the width live and calls `onViewChange({ column_widths })` once, on mouse release. The width never goes under 72. From the keyboard, focus the handle and press Left or Right: 8 pixels, or 32 with Shift. Each key press is one `onViewChange` call, because there is no release to wait for.
2. Shift+click on a sort entry in the column menu appends to `sort_rules`. If the column is already a rule, Shift+click changes its direction in place. A plain click replaces the rules. The grid takes the current rules in a new `sort_rules` prop. Sorting stays on the server.
3. The builder data tab renders `DataGrid`. All five phase 5 actions moved with their test ids: rename, change type, edit formula, insert left and right, and delete with the primary field blocked. `DataGrid` gained optional host props for this: `renderHeaderMenu`, `renderHeaderNote`, `headerMenu` and `onHeaderMenuChange`, `leadingColumn`, `trailingHeader`, `groupHeader` and `formatCell`. `MultiViewWorkspace` does not use them.

Deviations:

- The header is now drawn by a function call in the `<thead>`, not by TanStack's `flexRender`. A header component rebuilt on each menu change was remounted, which lost focus and made held button references stale.
- A multi-select cell now draws one pill per value in both grids, not one comma-joined string.
- The builder grid is editable. Before, only checkbox and link cells were. Edits go to the builder's in-memory records, as before.
- The builder keeps its column chrome (width, order, hidden, frozen, summary) in component state per table. It is not saved with the manifest. A "Show hidden columns" button undoes Hide Column.
- The builder header lost the field-type icon. The shared grid does not draw one.
- Grouped builder tables draw a heading row above the first record of each group, not a separate table body.
