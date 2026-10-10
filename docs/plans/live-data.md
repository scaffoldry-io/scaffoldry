# Live data — implementation brief

Hand this file to Gemini Flash. Do one phase per session. Stop when that phase's tests pass. Do not start the next phase.

Run order is `docs/plans/README.md`. This plan does not add a table for users, orgs, or records. Those tables exist. The running process does not use them, and it writes the demo back over them on every boot.

Hundreds of people can build at once on this schema. `dataset_records` is one row per record. `app_manifests` is one row per app. PostgreSQL row locks are the concurrency control. Do not add Redis, a CRDT, a presence service, or a second write queue.

## What is compiled instead of stored

The server builds these in `ServerState::new` (`crates/scaffoldry-server/src/state.rs`) and upserts them into PostgreSQL on every start. A later edit to the same id is overwritten on the next boot.

| Id | Kind |
| --- | --- |
| `ws-bio-lab`, `ws-physics-optics`, `ws-campus-compliance`, `ws-cs-research` | Workspace, plus named collaborators |
| `faculty`, `courses`, `programs`, `grants` | Published dataset, with sample rows inside the payload |
| `rel_course_instructor`, `rel_course_program`, `rel_grant_pi` | Relationship |
| `rule-admissions-auto-approve` | Automation |
| Ledger sequences 0–3 | Written when the stored ledger is empty |

`persons`, `organizations`, `roles`, `academic_plans`, and `facilities` are created by `0001_initial_schema.sql`. No server path reads or writes them. Identity on the desk is `PERSONAS` in `apps/web/src/AdminDesk.tsx`.

`dataset_records` has `list_records` in `repository.rs`. `ServerState.records` is `RwLock::new(HashMap::new())` and is never filled from that table. `update_record` in `crates/scaffoldry-server/src/service/records.rs` replaces the JSON in the map. It does not write PostgreSQL. A restart drops every cell edit.

The browser keeps a second copy:

| Const | File | What it pretends to be |
| --- | --- | --- |
| `SEEDED_RECORDS_BY_SLUG` | `apps/web/src/MultiViewWorkspace.tsx` | The app's records |
| `investigators` and the `APP-101` array | `apps/web/src/PublishedAppView.tsx` | Faculty and proposals |
| `PERSONAS` | `apps/web/src/AdminDesk.tsx` | The directory |
| `kanbanStatuses` | `apps/web/src/MultiViewWorkspace.tsx` | The stage column |
| `INITIAL_SOURCE_RULES` | `apps/web/src/AdminDesk.tsx` | The policy catalog |

The component catalog in `crates/scaffoldry-server/src/routes/framework.rs` stays in code. It is a schema, not an entity.

## Why one map does not carry a hundred editors

`update_record` takes the write lock on every record of every app, then replaces the whole JSON document. Two people editing two cells of one row: the second save restores the first cell to whatever it loaded. Two people editing two apps still wait on the same lock. There is no `version` column, so the database cannot reject the stale write.

The fix is one conditional update per record, and no map. PostgreSQL holds the rows and answers each read.

## Out of scope

- A pool crate. `docs/plans/foundation.md` phase 4 gives the repository eight workers. That phase comes before this plan.
- Authorization. `docs/plans/foundation.md` phase 5 adds `authorize_app` to every record path. Keep those calls.
- WebSockets, cursors, or "X is viewing this cell".
- Moving `academic_plans` or `facilities`. Nothing reads them. Do not invent screens.
- Deleting the demo workspaces from an existing database. Stop writing them. Do not `DELETE`.

## Phase 1 — boot reads, and does not clobber

In `ServerState::new`, delete the block that upserts workspaces, collaborators, datasets, relationships, and automations on every start. Replace it with: if `list_workspaces()` is empty, upsert the demo set once. Same for datasets, relationships, and automations: insert only when that id is absent. An existing row is left as stored.

Records are not loaded at boot. A table can hold millions of rows. Delete `ServerState.records`. `list_records`, `get_record`, `create_record`, `update_record`, `delete_record`, `run_automations`, and `decide_app_process` read and write `dataset_records` through the repository. `repository.rs` already has `list_records` and `upsert_record`. Add `get_record(id)` and `delete_record(id)`. Give `list_records` a `limit` and return at most 1,000 rows. `docs/plans/mcp-apps.md` phase 2 replaces that cap with a cursor. Return the repository error to the caller. Do not add a column in this phase.

When the repository is absent (`ServerState::in_memory()`, which only tests call), keep one `RwLock<HashMap>` behind the same five functions so the existing tests run.

Ledger rows 0–3 stay insert-if-empty. Do not rewrite a ledger that already has rows.

Tests:

1. A unit test or integration test calls the seed function twice. The second call does not change a workspace name that the test set after the first call.
2. `create_record`, build a second `ServerState` on the same database, `get_record` returns the created row.

Run the existing `api_integration_test` suite. The seeded physics workspace must still exist on a fresh process.

## Phase 2 — one row, one version

Migration `crates/scaffoldry-core/migrations/0007_record_version.sql`:

```sql
ALTER TABLE dataset_records ADD COLUMN IF NOT EXISTS version INTEGER NOT NULL DEFAULT 1;
ALTER TABLE app_manifests ADD COLUMN IF NOT EXISTS version INTEGER NOT NULL DEFAULT 1;
```

Register it in `repository.rs` after the last schema const, inside the advisory lock.

`update_record` takes `expected_version: i32`. The statement is:

```sql
UPDATE dataset_records
SET data = $1, version = version + 1
WHERE id = $2 AND version = $3
RETURNING version
```

Zero rows is `409` with body `{"error":"version_conflict","version": <current>}`. The caller reloads.

`GET` record and `GET` list include `version`. The client that saved sends the version it read. A missing `version` on the write is `400`, not a silent overwrite.

Do the same for the app manifest save path: `UPDATE ... WHERE slug = $1 AND version = $2`. Two builders saving one app get one success and one 409. Do not merge the JSON.

Tests:

1. Two updates with the same version: the second returns 409 and the stored field keeps the first write.
2. Update with the returned version succeeds and the new version is one greater.

## Phase 3 — the desk shows stored rows

Remove the record arrays from `MultiViewWorkspace.tsx` and `PublishedAppView.tsx`. The grid loads `GET` records for the open app and saves a cell with the version from that payload. On 409, reload the row and show the text `Someone else saved this record. Reloaded.` Do not retry the stale body.

`kanbanStatuses` is deleted. Stages are `select_options` of the field named by `kanban_column_field`. When that field has no options, the columns are the distinct values already on the loaded records.

`PERSONAS` remains the list used when the API is unreachable in tests. When `GET /scim/v2/Users` returns users, the switcher renders those users and does not offer a persona absent from the response. Do not call SCIM from the browser with the provisioning token. Add `GET /api/v1/directory`, session-authenticated, returning `eppn`, `name`, and `affiliation` from the SCIM user store. The switcher calls that.

`INITIAL_SOURCE_RULES` stays until a policy API exists. Do not move it in this phase.

One vitest: a 409 response leaves the reloaded cell value on screen, not the typed value.

## How to prompt Gemini

```
Read docs/plans/live-data.md and docs/plans/README.md.
Implement phase N only.
Write the failing test first and run it.
Do not add a dependency, a cache service, or a websocket.
Do not delete rows that are already stored.
Stop when the tests listed for phase N pass, and paste the command output.
```
