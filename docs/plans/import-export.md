# Import, export, bundles, templates, and snapshots — implementation brief

Hand this file to Gemini Flash. Do one phase per session. Stop when that phase's tests pass. Do not start the next phase.

Run order is `docs/plans/README.md`. Phase 1 needs `jobs.md` phase 2, `calc-graph.md` phase 4, and `record-history.md` phase 1. Phase 2 needs `record-history.md` phase 4 and `attachments.md` phase 1. Phases 3 to 5 need `mcp-apps.md` phase 4a. Phase 4 needs `access-rules.md` phase 5.

Most faculty start from a spreadsheet. They have a list of applicants, a budget, a roster. They will ask an agent to turn it into an app. They will also want to take data out, copy an app that worked for one department, and go back to how things were last week. Today there is a CSV helper in the engine and nothing around it: no mapping, no check before writing, no report of what failed, no spreadsheets, no way to copy or restore an app.

This brief builds the whole path in and out, and makes approved templates the fast, safe way to start.

## Decisions already made

| Question | Answer |
| --- | --- |
| Is an import ever live at once? | It always has a dry run first, runs as a job, and reports every rejected row |
| Does an import skip rules? | No. Each row goes through `write_record`, so types, constraints, and access rules all apply, and every row is in history as one import batch |
| Is an export unrestricted? | No. It applies the person's row and column rules, and sensitive fields need the export permission. Sensitive exports are logged as disclosures |
| What is an app bundle? | One document with the app's definition, never live data by default. Importing a bundle creates a proposal, never a live app |
| What is a template? | An approved bundle in an institution catalog. An unchanged template skips the sensitivity review it already had |
| What does restore do? | It creates a new app from a snapshot. It never overwrites a live app |

## What exists today

| Fact | Where |
| --- | --- |
| `parse_csv_to_records`, `export_records_to_csv`, `escape_csv_cell` | `scaffoldry-engine/src/lib.rs` |
| A CSV import modal in the web app | `apps/web/src/CsvImportModal.tsx` |
| No spreadsheet reading or writing, no bundle, no snapshot | |

## Out of scope

- Importing live from another tool's API. A spreadsheet or CSV export is the supported path in this plan.
- Fetching attachment files from addresses in an import. They are listed in the report for a person to handle.
- Scheduled exports.
- Full-appliance backup. That is the operator's database dump and files directory.

## Rules for every phase

1. Write the failing test first. Run it. Confirm it fails because the behavior is missing. Then write the code.
2. Add `calamine`, `rust_xlsxwriter`, and `flate2` only in the phase that names them, after Johann's yes, and run the license audit.
3. A row is never written without passing `write_record`.
4. Use the kit for every screen.

## Phase 1 — import

Upload. `POST /apps/{slug}/imports`, app write access: a file (CSV, JSON array, or JSON Lines in this phase) is stored through the file store as an `import` attachment with a 24-hour life. It returns an import id.

Analyze. The job `import_analyze` reads the first 5,000 rows, detects the delimiter and encoding (UTF-8, with a byte-order mark accepted, and a clear error for anything else), and for each column proposes a field type by examining values: number, date, date-time, boolean, a select when the column has at most 20 distinct values over at least 20 rows, an email, a URL, a collaborator when every value resolves to a person, and text otherwise. It returns the columns, the sample, the inferred type, and the matching target field by normalized name when the target table already has one.

Map. The person confirms or changes the mapping in a screen. New columns become fields in a proposal that adds them, and the import waits for it. A mapping may choose an `upsert_key` field: rows that match an existing record by that field's value update it instead of creating one. Without a key, every row creates a record.

Dry run. `POST /apps/{slug}/imports/{id}/dry-run` enqueues `import_validate`, which runs every row through validation without writing and returns counts of valid, invalid, creates, and updates, and the first 100 errors with row number, column, and a plain sentence. Link columns resolve by an exact match on the target's primary field within the person's scope. An unmatched or ambiguous value is an error, never a guess. An unknown select option follows the mapping's setting: `reject` (default), `blank`, or `propose_option`, which adds the option to the proposal.

Run. `POST /apps/{slug}/imports/{id}/run` enqueues `import_run`: batches of 1,000 rows, each batch one transaction through `write_record` with a history batch of kind `import`, a checkpoint after each, and a rejected-rows file written as it goes. It can be cancelled. The result holds counts and the id of the rejected-rows file, a CSV with the original row and the reason. A run needs an unexpired dry run of the same mapping with no blocking errors, or an explicit `accept_errors: true` that skips invalid rows. Limits: `import.max_rows` (default 500,000), `import.max_mb` (default 100), both in the closed settings list.

A ledger entry, `DataImported`, added to `DecisionType::ALL`, holds the app, table, counts, and the SHA-256 of the file, only when a run writes more than 1,000 rows.

Screen. A wizard from the kit with four steps: `Choose a file`, `Match columns`, `Check`, `Import`. Each shows progress as text and a bar, errors in a table, and a `Download the rejected rows` link. A person can leave and return, because the job continues, and the Background work indicator shows it.

Replace `CsvImportModal.tsx`'s logic with this flow. Keep its entry points.

Tests.

1. A file of 10,000 rows with 30 bad ones: the dry run reports 30 errors with row numbers and sentences, and writes nothing.
2. Run writes 9,970 records, each in history as an `import` batch, and the rejected file holds the 30 rows with reasons.
3. An `upsert_key` updates matching records and creates the others.
4. A link value that matches two targets is an error and is never guessed.
5. An unknown select option is rejected by default, blank by mapping, and added to the proposal by `propose_option`.
6. Kill the job mid-run: it resumes from its checkpoint and no row is written twice.
7. A file over the row limit is refused before the job starts.
8. Type inference on a fixture of 20 columns chooses the expected types, including a select, a date, and a collaborator.
9. An import by a person with row rules (`access-rules.md`) cannot create a row outside their `create` scope: those rows are rejected with a reason.
10. Web: the four steps are reachable by keyboard and announce progress.

## Phase 2 — spreadsheets, exports, and the disclosure of exports

Add `calamine` (reading) and `rust_xlsxwriter` (writing), after Johann's yes.

1. Import accepts `.xlsx`. Each sheet is offered as a source table. Dates in the file are read as dates, not numbers. Formulas are read as their stored values. A file with macros (`.xlsm`) is refused. An archive that expands to more than 20 times its size is refused.
2. Export. `POST /apps/{slug}/exports` takes a table or a view, the fields, and the format (`csv`, `xlsx`, `json`, `jsonl`). It enqueues `export_run`, which streams the filtered, scoped rows in batches to a file in the store with a 24-hour life, applies row scope and field visibility, and checks the Cedar `export` action once for the whole request. A field the person may not export is left out and named in the result: `Left out: Student ID.` The person who asked can download it. Nobody else can.
3. A CSV cell that starts with `=`, `+`, `-`, or `@` is written with a leading apostrophe in a spreadsheet export and escaped in CSV, so opening the file cannot run a formula.
4. An export that includes a sensitive field writes a disclosure entry (`record-history.md` phase 4) and the ledger entry `DataExported`, added to `DecisionType::ALL`, with counts and the fields, never values. It is limited to 100,000 records per request. Larger tables export in parts through repeated requests with a filter.
5. The MCP tool `export_records` returns a job id. The job result says where to download through the authorized route, and the agent cannot read the file's bytes through any tool, so an agent cannot pipe a full export into a model. It can request an export and tell the person.

Tests.

1. An `.xlsx` with two sheets imports each as a table. A date cell becomes a date.
2. A macro file and a decompression bomb are refused.
3. A cell `=HYPERLINK(...)` is neutralized in CSV and XLSX.
4. An export applies row and column rules, names what it left out, and a different person cannot download it.
5. A sensitive export writes one disclosure and one ledger entry, and no values.
6. An export of 100,001 records is refused with advice to narrow it.
7. The export tool returns a job id and no bytes.

## Phase 3 — app bundles

Add `flate2` (compression of the side files only), after Johann's yes.

A bundle is one JSON document with a schema at `governance/schema/app-bundle.schema.json`:

| Part | Holds |
| --- | --- |
| `bundle_version` | The version of this format |
| `manifest` | Tables, fields, links, access rules, pages (with `source_sha256`), integrations (webhooks without secrets) |
| `views` | Collaborative and locked views. Personal views are never included |
| `rules` | Process definitions |
| `page_sources` | The source of each custom page, keyed by hash |
| `sample_data` | Optional, at most 5,000 records, with sensitive fields removed |
| `sha256` | A hash over the rest, to detect damage |

Export. `POST /apps/{slug}/bundle`, app owner or admin. The bundle never holds secrets, tokens, member lists, comments, history, or attachments. A compressed side file may hold sample data.

Import. `POST /workspaces/{id}/bundle-import` with a bundle creates a proposal, never a live app. The server verifies the hash, validates against the schema and the manifest validator, rewrites ids that collide, suggests a new slug on collision, and returns the proposal with its checks and change list. Page sources are stored by hash as in `mcp-apps.md` phase 4. Webhooks in a bundle arrive disabled and must be approved again with their own host check and secret, which the importing workspace must supply by name.

Tests.

1. Export then import into another workspace yields a proposal whose change list matches the source app.
2. A bundle with a changed byte fails the hash check and creates nothing.
3. A bundle holds no secret, token, member, comment, or history. A scan test on the output proves it.
4. A slug collision suggests a free slug. Ids are rewritten without collision.
5. A bundle's webhook arrives disabled and fails `webhook_host` until allowlisted.
6. A bundle's custom page keeps its source hash, and a modified source fails the page check.
7. The schema validates the exported document.

## Phase 4 — templates

Migration `0035_import_export.sql`:

```sql
CREATE TABLE IF NOT EXISTS app_templates (
    id UUID PRIMARY KEY,
    name VARCHAR(120) NOT NULL,
    description TEXT NOT NULL DEFAULT '',
    category VARCHAR(60) NOT NULL DEFAULT 'General',
    bundle JSONB NOT NULL,
    bundle_sha256 CHAR(64) NOT NULL,
    version INTEGER NOT NULL DEFAULT 1,
    status VARCHAR(8) NOT NULL DEFAULT 'approved',
    unit_id UUID,
    published_by VARCHAR(255) NOT NULL,
    published_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
```

`status` is `approved` or `retired`. `unit_id` limits a template to a unit and its descendants, null meaning the whole institution. A Platform Admin, or an Org Unit Admin for their own unit, publishes a template from a bundle, and the publication is a proposal-style review with a second person, a ledger entry, and the bundle's checks run in full.

Using one. The MCP tools `list_templates` and `instantiate_template(workspace, template, slug)` create a proposal from the bundle. Its checks add `template_pristine`. When the proposal's manifest, rules, pages, and access rules equal the template's, the review done at publication stands: `sensitivity`, `page_sensitivity`, and `rule_effects` are not raised again for what the template already contains, and the workspace owner can approve in the same request. A workspace classification below what the template needs (`FERPA Sensitive` content into an `Internal` workspace) still raises `sensitivity`. Any change from the template raises the normal checks on the changed parts only.

Screen. In the workspace, `New app` offers templates by category with a description, what it contains in sentences (tables, who sees what, which approvals), and `Use this template`.

Tests.

1. Publishing needs a second person. A single admin cannot publish alone.
2. An unchanged template into a matching workspace is approved in the same request by an owner.
3. The same template into a lower-classification workspace raises `sensitivity`.
4. Changing one field after instantiation raises the checks for that field only.
5. A retired template cannot be instantiated.
6. A unit-limited template is invisible outside its unit.

## Phase 5 — snapshots

Migration `0052_snapshots.sql`:

```sql
CREATE TABLE IF NOT EXISTS app_snapshots (
    id UUID PRIMARY KEY,
    app_slug VARCHAR(64) NOT NULL,
    name VARCHAR(120) NOT NULL,
    kind VARCHAR(10) NOT NULL,
    file_sha256 CHAR(64),
    record_count BIGINT,
    size BIGINT,
    created_by VARCHAR(255) NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    expires_at TIMESTAMPTZ
);
```

`kind` is `manual`, `scheduled`, or `pre_change`. A snapshot job writes the bundle plus every record as compressed JSON Lines into the file store. It holds everything the app's data holds, so it is as sensitive as the app: access is app owner or admin, it is never exposed to an agent, and a sensitive snapshot's download is a disclosure.

Restore creates a new app, named by the person, from a snapshot, by a proposal for the definition and a job for the records, through `write_record` with kind `import`. It never overwrites a live app.

A proposal flagged `data_loss` (a field retired, a type converted with losses, a table archived) requires a `pre_change` snapshot first. Approval enqueues the snapshot, the proposal is `Approving`, and a job completes the approval when the snapshot is done. A snapshot older than seven days does not count. Snapshots expire by the retention policy and not under a hold.

Tests.

1. A manual snapshot of a 20,000-record app restores into a new app with the same records, and the original is untouched.
2. A `data_loss` approval first takes a snapshot and only then applies the change. Failing the snapshot leaves the proposal unapproved.
3. A snapshot is not downloadable by an editor, and no MCP tool returns its content.
4. A sensitive snapshot's download writes a disclosure.
5. A held app's snapshots are not expired.

## Phase 6 — moving off a spreadsheet tool

A mapping preset for files exported from common spreadsheet-database tools: name normalization (spaces and case), splitting a comma-separated multi-select column, recognizing a linked-record column by its header and resolving it by primary field, mapping an attachment column to a report of file addresses (not fetched), reading checkbox columns as booleans, and reading currency strings such as `$1,200.50`. The wizard offers `This file came from another tool` and applies the preset, and the dry run explains each decision it made in a list the person can undo before running.

Tests.

1. A fixture CSV with a multi-select, a linked column, a checkbox, and a currency column maps as described, and each decision is listed.
2. An attachment column produces a report of addresses and fetches none.
3. Turning off one decision changes the dry run.

## How to prompt Gemini

```
Read docs/plans/import-export.md and docs/plans/README.md.
Implement phase N only.
Write the failing test first and run it.
Add only the crates this phase names, after Johann's yes, and run the license audit.
Never write a row without write_record.
Never let a bundle carry a secret, token, member, or live data by default.
Never give an agent the bytes of an export or a snapshot.
Do not read, copy, or adapt another product's source code.
Stop when the tests listed for phase N pass, and paste the command output.
```

## Enterprise objections

| Objection | Answer |
| --- | --- |
| An import will bypass our rules. | Every row passes the same write path, so types, constraints, and access rules all apply, and every row is in history. |
| An export will walk out the door. | It applies row and column rules, needs the export permission for sensitive fields, is logged as a disclosure, and an agent cannot read its bytes. |
| A spreadsheet will carry a formula that runs on open. | Cells that start with a formula character are neutralized in every export format. |
| Bundles will move secrets between apps. | A bundle never holds secrets, tokens, members, or live data, and a test scans for them. |
| A template will spread a mistake. | A template is published by two people, and an instance that differs from it is reviewed on the difference. |
| Restore will overwrite good data. | Restore only creates a new app. |
