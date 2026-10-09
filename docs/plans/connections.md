# Data connections — implementation brief

Hand this file to Gemini Flash. Do one phase per session. Stop when that phase's tests pass. Do not start the next phase.

Run order is `docs/plans/README.md`. Phase 1 needs `jobs.md` phase 2, `links.md` phase 3, `access-rules.md` phase 3, and `integrations.md` phase 1. Phase 6 needs `notifications.md` phase 1.

The most useful data in an institution is not in Scaffoldry. Courses, programs, faculty, enrollments, and grants live in the student information system, the HR system, and the grants system. Apps need to point at them: an application links to a program, a course shows its instructor. Those records are authoritative. A department must not edit them, and an agent must not be able to rewrite them.

Today a "published dataset" holds a few sample rows typed into the code. This brief makes datasets real: governed, scheduled, read-only copies of external data, with a history of every sync, and a gate that stops a changed source from silently changing the model.

## Decisions already made

| Question | Answer |
| --- | --- |
| Live proxy or copy? | A copy, refreshed on a schedule. Apps stay fast, and a source outage does not stop them |
| Can a person edit a dataset? | No. Every write route answers `read_only_source`. Only the sync changes it |
| Where does a dataset live? | In a system app in a system workspace that only Platform Admins own. Everything that works on app tables works on it: views, search, history, export, links |
| What if the source changes shape? | The sync stops and creates a proposal for the new shape. Nothing is added or dropped on its own |
| What if the source deletes a row? | The record goes to trash, with the sync as the actor. It is restorable, and retention applies |
| Who is the sensitivity decided by? | A Platform Admin, at the connection. Every field starts with that connection's label |

## What exists today

| Fact | Where |
| --- | --- |
| `PublishedDataset` with `sample_data` typed in code | `scaffoldry-core/src/dataset.rs`, `state.rs` |
| `DatasetRelationship` joins datasets by field value | `scaffoldry-core/src/dataset.rs` |
| The dataset catalog route returns sample rows to any signed-in user | `routes/datasets.rs` |
| No sync, no connector, no external connection | |

## Out of scope

- Writing back to a source system.
- Real-time change capture from a source.
- A connector for every product. Three are built here, and a fourth is a file drop an institution can feed from anything.
- Transforming data beyond selecting, renaming, and typing columns.
- Joins across sources.

## Rules for every phase

1. Write the failing test first. Run it. Confirm it fails because the behavior is missing. Then write the code.
2. Add a crate only in the phase that names it, after Johann's yes, and run the license audit.
3. A connection reaches the network only through `net::egress`. A file drop reaches nothing.
4. A dataset record is written only by the sync job. A test scans for any other writer.
5. A secret is used by name and never shown.

## Phase 1 — datasets and the file-drop connector

Migration `0036_connections.sql`:

```sql
CREATE TABLE IF NOT EXISTS connections (
    id UUID PRIMARY KEY,
    name VARCHAR(120) NOT NULL UNIQUE,
    kind VARCHAR(12) NOT NULL,
    config JSONB NOT NULL,
    secret VARCHAR(64),
    dataset_app VARCHAR(64) NOT NULL,
    dataset_table VARCHAR(64) NOT NULL,
    key_field VARCHAR(64) NOT NULL,
    column_map JSONB NOT NULL,
    sensitivity VARCHAR(32) NOT NULL,
    every_minutes INTEGER,
    at_time VARCHAR(5),
    enabled BOOLEAN NOT NULL DEFAULT TRUE,
    created_by VARCHAR(255) NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE IF NOT EXISTS sync_runs (
    id UUID PRIMARY KEY,
    connection_id UUID NOT NULL REFERENCES connections(id),
    state VARCHAR(10) NOT NULL,
    started_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    finished_at TIMESTAMPTZ,
    added BIGINT NOT NULL DEFAULT 0,
    changed BIGINT NOT NULL DEFAULT 0,
    removed BIGINT NOT NULL DEFAULT 0,
    unchanged BIGINT NOT NULL DEFAULT 0,
    error TEXT,
    source_hash CHAR(64)
);
CREATE INDEX IF NOT EXISTS idx_sync_runs_connection ON sync_runs (connection_id, started_at DESC);
```

`kind` is `file_drop`, `postgres`, or `http`. This phase builds `file_drop`.

Datasets. The system workspace `system-datasets` and the app kind `dataset` are created at boot. An `AppManifest` gains `kind: App | Dataset`, default `App`. Writes to a dataset's records through any route are refused with `409` and error code `read_only_source`. Add the code to the closed list in `ux-standards.md`. A dataset's fields carry the connection's sensitivity as the `ferpa_sensitive` flag or classification, which a Platform Admin or `compliance` may raise per field through labels. Dataset access: `list_datasets` and `get_dataset` return metadata to any signed-in person, and rows follow the access rules of the dataset's table, so a restricted dataset needs a rule or a workspace membership. The old sample-data route and its typed rows are deleted. The seeded demo datasets are created by `seed-demo` as file-drop connections over fixture files.

File drop. `SCAFFOLDRY_DROP_DIR` names a directory the operator mounts, so no network is involved. A connection's `config` is `{ "path": "sis/programs.csv", "format": "csv" | "jsonl" }`, resolved inside the drop directory after rejecting any `..`, absolute path, or symbolic link that leaves it. The institution's own job places the file there, for example by SFTP to that mount.

The sync job `sync_connection(id)`:

1. Read the file, hash it, and finish at once with `unchanged` if the hash equals the last run's `source_hash`.
2. Check the columns against `column_map` (source column, target field, type). A missing mapped column or an unmapped extra column stops the run with `schema_drift` (phase 2).
3. Upsert by `key_field` in batches of 1,000, through `write_record` with `actor_kind` `sync` and a history batch, comparing values so an unchanged record is not rewritten and gets no new history.
4. Trash any record whose key is absent from the file, but only if the file holds at least 50 percent as many rows as the last run, otherwise stop with `source_shrank` and require a person to approve, to protect against a truncated file.
5. Record the counts in `sync_runs`.

Schedule: `every_minutes` or a daily `at_time` in `platform.timezone`, registered through the scheduler from `jobs.md`. `POST /admin/connections/{id}/run` runs now.

Routes, Platform Admin, all in `ADMIN_ROUTES`: `GET`, `POST`, and `PATCH` on `/admin/connections`, `GET /admin/connections/{id}/runs`, and the run route. Each change is a ledger entry, `ConnectionChanged`, added to `DecisionType::ALL`.

Tests.

1. A fixture CSV of 1,000 rows syncs into a dataset table. A second run on the same file is `unchanged` and writes nothing.
2. A changed file updates only the changed rows, adds new ones, and trashes removed ones, with exact counts.
3. A file with 30 percent of the previous rows stops with `source_shrank` and changes nothing.
4. A path that escapes the drop directory by `..` or a link is refused.
5. A write to a dataset record through the REST route, the MCP tool, an import, and a page call is `read_only_source`. A scan test finds no writer except the sync.
6. The history of a changed record shows the sync as actor and kind `sync`.
7. The old sample-data catalog route no longer returns rows to a signed-in person who lacks access.
8. A schedule fires once under two scheduler threads.

## Phase 2 — when the source changes shape

1. On `schema_drift`, the run stops and the connection's state shows `Needs review`. The job creates a proposal for the dataset app that adds the new columns as fields, retires the missing ones (`lifecycle.md`), or both, with the checks of `mcp-apps.md`. The proposal names the connection and shows the difference in words: `The source added "Campus". The source no longer has "Dept code".`
2. Approving the proposal updates the manifest and `column_map` together, and the next run proceeds. Rejecting it leaves the connection stopped, and the console says so.
3. A changed type (text that now holds numbers) is a flagged `data_loss` conversion preview from `calc-graph.md` phase 3.
4. Every run is in `sync_runs`, with counts, and a failed run keeps its error. The last 100 per connection are kept.

Tests.

1. A file with a new column stops the run and creates one proposal naming the column. Approving it lets the next run add the column's values.
2. A file missing a mapped column stops the run, and the proposal retires the field. No data is dropped until retention.
3. Rejecting a proposal leaves the connection `Needs review` and no data changed.
4. A type change shows a conversion preview with counts.

## Phase 3 — PostgreSQL

Needs a PostgreSQL TLS connector, after Johann's yes.

`config` for `postgres`: `{ "host", "port", "database", "user", "tls": "require" | "verify-full", "query": "SELECT ..." }`, with `secret` naming the password. The host must be on `egress.allow`, and an internal source needs a `cidr` entry on purpose.

The connection runs `BEGIN READ ONLY`, sets `statement_timeout` to the connection's limit (default 5 minutes), runs the one configured `SELECT`, and streams rows through a cursor in batches of 5,000 so memory stays flat. The query is checked: it must parse as a single statement beginning with `SELECT` or `WITH`, with no semicolon outside a string, and runs in a read-only transaction regardless, so a mistake cannot write. TLS is required by default, and `verify-full` checks the name against the certificate. The password never appears in a log or error.

Tests, against a second PostgreSQL database in the test environment.

1. A configured query syncs rows into a dataset. The password comes from the secrets store.
2. A statement that would write is refused by the check and by the read-only transaction. Test both.
3. A host that is not on the allowlist is refused. A private address is refused without a `cidr` entry.
4. `verify-full` fails against a certificate for another name.
5. A query that runs past the timeout fails the run with a clear error and changes nothing.
6. A 200,000-row source syncs in batches with bounded memory. Record the peak in the session output.

## Phase 4 — HTTP

`config` for `http`: `{ "url", "format": "json" | "jsonl", "path": "data" }` with an optional `secret` naming a bearer token. The URL is `https` only, on the allowlist, with no redirects, a 5 MB response limit per page, and pagination through a configured `next` path (a response field holding the next URL, checked against the allowlist too). At most 100 pages per run and a 10 minute total.

Tests.

1. A paged JSON source syncs all pages. The `next` address off the allowlist stops the run.
2. A redirect is a failure.
3. A response over the limit is a failure.
4. The bearer token is used and never logged.

## Phase 5 — linking apps to datasets

A link field may target a dataset table: `link_target_app` names a dataset app (`links.md` phase 1). Rules.

- A link to a dataset record stores its id. The dataset is read-only, so the target never changes by a person.
- The link picker searches the dataset within its access rules and returns a label and a secondary field.
- Lookups pull dataset fields, applying their hidden-field rules.
- A sync that trashes a linked record unlinks it, as any delete does, and restores it if the record returns.
- A dataset field is stable: the sync keeps the record's id by `key_field`, so a record whose other fields change keeps its links.

Replace the by-value `DatasetRelationship` with these links. A conversion preview, as in `links.md` phase 5, offers it for apps that used a dataset relationship.

Tests.

1. An application links to a program in a dataset. The program's name changes in the source. The link holds and the lookup shows the new name.
2. The source removes the program. The link is removed and the application shows an unlinked state, then the program returns and the link returns.
3. A restricted dataset's picker returns only what the person may see.
4. The conversion preview reports matched, unmatched, and ambiguous.

## Phase 6 — the connections panel

From the kit, in the console. A table of connections: name, kind, target dataset, last success, last result in words (`Added 12, changed 3, removed 1`), state (`Healthy`, `Needs review`, `Failing`, `Disabled`), and next run. A row opens the run history with each run's counts and error. Actions: `Run now`, `Disable` and `Enable` (with a reason), and `Edit mapping`, which opens the column map with the sample from the last file. A failed sync or a change needing review notifies the connection's owner and Platform Admins (`notifications.md`).

The Overview gains healthy, failing, and needs-review counts, and the age of the oldest successful sync.

Tests.

1. A failing connection shows `Failing` with its error and notifies once, not on every retry.
2. `Disable` stops the schedule and asks for a reason.
3. The history shows the last 100 runs.
4. The Overview counts match the fixture.

## How to prompt Gemini

```
Read docs/plans/connections.md and docs/plans/README.md.
Implement phase N only.
Write the failing test first and run it.
Add only the crate this phase names, after Johann's yes, and run the license audit.
Never let anything but the sync write a dataset record.
Never reach the network outside net::egress.
Never add or drop a field because a source changed. Make a proposal.
Do not read, copy, or adapt another product's source code.
Stop when the tests listed for phase N pass, and paste the command output.
```

## Enterprise objections

| Objection | Answer |
| --- | --- |
| Pulling data from the student system is risky. | The connection is read-only, runs in a read-only transaction, reaches only an allowlisted host over TLS, and uses a credential from the encrypted store. A file drop reaches nothing at all. |
| A department will edit authoritative data. | Every write route to a dataset refuses with `read_only_source`, and a scan test finds no other writer. |
| A bad file will wipe a dataset. | An unchanged file does nothing, a file that lost more than half its rows stops the run, and removed records go to trash, not oblivion. |
| The source will change and break apps. | The sync stops, a proposal describes the difference in words, and a person approves the change. |
| Synced data will be labeled wrongly. | The connection sets a sensitivity, and `compliance` can raise any field's label at once. |
| The sync will leak the password in an error. | The secret is used by name, never logged, and a test checks failure messages. |
