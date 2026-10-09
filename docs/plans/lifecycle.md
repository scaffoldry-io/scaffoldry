# Records lifecycle — implementation brief

Hand this file to Gemini Flash. Do one phase per session. Stop when that phase's tests pass. Do not start the next phase.

Run order is `docs/plans/README.md`. Phase 1 needs `links.md` phase 2 and `record-history.md` phase 1. Phase 2 needs `mcp-apps.md` phase 4a. Phases 3 and 4 need `jobs.md` phase 2 and `attachments.md` phase 1.

A coordinator deletes the wrong row. A registrar must keep admissions files for a set time and destroy them afterward. General counsel says: hold everything about this one case until further notice. Today `delete_record` removes the row at once and nothing else exists. There is no way back from a mistake, no way to say how long data is kept, no way to prove it was destroyed on schedule, and no way to stop destruction when a dispute begins.

This brief adds a trash, retention schedules with proof of purge, and legal holds that override both.

## Standard

| Standard | Use |
| --- | --- |
| NIST SP 800-53 SI-12 (information management and retention) and MP-6 (media sanitization) | Keeping information for a defined period, and destroying it with a record that it was destroyed |
| Institutional records schedules | Set by the institution's records officer. Scaffoldry supplies the mechanism and ships placeholders, not legal advice |

## Decisions already made

| Question | Answer |
| --- | --- |
| Does delete remove data? | No. It moves a record to trash. Only a retention job destroys data |
| What survives a purge? | A ledger entry with counts and the policy used, and a certificate with a hash of the destroyed ids. Not the data, and not the ids |
| What is a hold? | A named, ledgered instruction that stops every destruction of the records it covers. It overrides every retention policy |
| Who places and releases a hold? | A Platform Admin or `compliance` places it. A different person releases it |
| Who sees that a hold exists? | Platform Admins and `compliance` only. Editors see no sign, and a purge simply does not happen |
| Are retention numbers decided here? | No. The defaults are long placeholders. The records officer sets real ones |

## What exists today

| Fact | Where |
| --- | --- |
| `delete_record` removes the row from the in-memory map and, after `live-data.md`, from the table | `service/records.rs` |
| `data_classification` on a workspace has four levels | `guards.md`, `admin-console.md` |
| History and attachments are planned and have no delete path | `record-history.md`, `attachments.md` |
| Removing a field from a manifest drops it from the definition and leaves values in each record | `mcp-apps.md` |

## Out of scope

- Amendment of education records under FERPA (a different process).
- Automated records-officer workflows.
- Certified destruction of backups. Operators handle that, and the Deployment notes say so.
- Purging the audit ledger. The ledger is permanent.

## Rules for every phase

1. Write the failing test first. Run it. Confirm it fails because the behavior is missing. Then write the code.
2. Do not add a dependency.
3. Every destruction of stored data goes through `purge::destroy(...)`, which checks `is_held` first. A test scans the source for `DELETE FROM` on lifecycle tables outside it.
4. Every purge writes a ledger entry before it deletes.
5. Use the kit for every screen.

## Phase 1 — trash

Migration `crates/scaffoldry-core/migrations/0042_lifecycle.sql`:

```sql
ALTER TABLE dataset_records ADD COLUMN IF NOT EXISTS deleted_at TIMESTAMPTZ;
ALTER TABLE dataset_records ADD COLUMN IF NOT EXISTS deleted_by VARCHAR(255);
ALTER TABLE dataset_records ADD COLUMN IF NOT EXISTS delete_batch UUID;

DROP INDEX IF EXISTS idx_dataset_records_page;
CREATE INDEX IF NOT EXISTS idx_dataset_records_live
    ON dataset_records (app_slug, table_id, created_at, id) WHERE deleted_at IS NULL;
CREATE INDEX IF NOT EXISTS idx_dataset_records_trash
    ON dataset_records (app_slug, deleted_at) WHERE deleted_at IS NOT NULL;

CREATE TABLE IF NOT EXISTS trash_links (
    record_id VARCHAR(64) NOT NULL,
    field VARCHAR(64) NOT NULL,
    other_record VARCHAR(64) NOT NULL,
    direction VARCHAR(4) NOT NULL,
    PRIMARY KEY (record_id, field, other_record, direction)
);
```

Delete. `delete_record` sets `deleted_at`, `deleted_by`, and a `delete_batch` shared by a multi-delete, removes the record's `record_index` and `record_links` rows after saving the removed links to `trash_links`, writes a history entry of action `delete`, and emits the realtime event. Every query that lists, counts, totals, searches, aggregates, links, or exports adds `deleted_at IS NULL`. The paging index from `row-scale.md` is now this partial index.

Restore. `POST /apps/{slug}/trash/{batch_or_record}/restore`, editor and above, within `row_scope`. It clears the flags, rebuilds the record's index rows, restores links from `trash_links` that still point at live records, and writes history of action `restore`.

Trash list. `GET /apps/{slug}/trash`, paged, newest first. The person sees what they could see before deletion (`row_scope`), the primary field, who deleted it, when, and the time left before permanent removal from the policy in force. The search column works on trashed rows. Editors see their own deletions. Owners and admins see all.

`delete` through every route and through MCP is the same operation. The tool description says `Moves a record to trash. It can be restored.` There is no tool that destroys.

Screen. A `Trash` tab in the app for editors and above: a table with `Restore`, a selection, and the time left in words (`Removed permanently in 27 days`). Deleting a row in the grid shows a toast with `Undo`, which restores.

Tests.

1. Delete moves a record to trash: it vanishes from lists, counts, totals, search, and link options, and appears in trash with the right time left.
2. Restore brings it back with its links, except to a record that was also deleted.
3. A link to a trashed record is removed from the other record's value and restored with it.
4. A restricted person's trash shows only what they could see.
5. A multi-delete restores as one batch.
6. History has `delete` and `restore` entries.
7. The partial index is used by the list query, and the plan has no sequential scan.
8. Web: the toast `Undo` restores, and `Restore` in trash does too.

## Phase 2 — retired fields and archived structures

Fields. Removing a field in a proposal sets `retired_at` on it. It does not delete values. A retired field is hidden everywhere, is refused on write, is dropped from the indexed set, and is excluded from the formula graph (a formula that reads it fails validation). The flagged check `data_loss` now says `The values stay for 90 days and can be restored.` Restoring a field is a proposal that clears `retired_at`. A scheduled job strips retired values from records after the policy's `keep_retired_days`, through `purge::destroy`.

Apps and workspaces. `POST /apps/{slug}/archive` and `/unarchive`, and the same for workspaces, owner or admin and Platform Admin, with a reason and a ledger entry. An archived item is read-only, hidden from default lists, and kept. Writes are refused with error code `archived`. Add `archived` to the closed list in `ux-standards.md`. Destruction of an archived app or workspace happens only through retention and never while held.

Tests.

1. A retired field is hidden, unwritable, unindexed, and its values are still in the JSON. Restoring it brings the values back.
2. A formula over a retired field fails validation.
3. Archiving an app refuses writes with `archived` and hides it from the default list. Unarchiving restores it.
4. The retired-value strip runs only after `keep_retired_days` and not under a hold.

## Phase 3 — retention and proof

Migration `0050_retention.sql`:

```sql
CREATE TABLE IF NOT EXISTS retention_policies (
    id UUID PRIMARY KEY,
    name VARCHAR(120) NOT NULL,
    scope_kind VARCHAR(14) NOT NULL,
    classification VARCHAR(32),
    workspace_id VARCHAR(64),
    keep_trash_days INTEGER NOT NULL,
    keep_history_days INTEGER,
    keep_retired_days INTEGER NOT NULL,
    max_record_age_days INTEGER,
    on_expiry VARCHAR(5) NOT NULL DEFAULT 'trash',
    basis TEXT NOT NULL,
    active BOOLEAN NOT NULL DEFAULT TRUE,
    created_by VARCHAR(255) NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE IF NOT EXISTS purge_certificates (
    id UUID PRIMARY KEY,
    run_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    policy_id UUID,
    app_slug VARCHAR(64) NOT NULL,
    counts JSONB NOT NULL,
    ids_sha256 CHAR(64) NOT NULL,
    held_skipped INTEGER NOT NULL DEFAULT 0
);
```

`scope_kind` is `classification` or `workspace`. A workspace policy overrides a classification policy. At boot, with no policies, insert one per classification with placeholder values: `keep_trash_days` 365, `keep_history_days` null (keep forever), `keep_retired_days` 365, `max_record_age_days` null. The `basis` of each says `Placeholder. Set by the records officer.` The Overview shows a `Banner` while any policy still has that basis. `on_expiry` of `purge` is accepted only with a non-empty `basis` that is not the placeholder.

Policies are edited by a Platform Admin or `compliance`, with a reason and a ledger entry, `RetentionPolicyChanged`, added to `DecisionType::ALL`. Before saving, `POST /admin/retention/preview` enqueues a job that returns, per app, how many records, history rows, files, and fields the next purge would destroy under the proposed policy, and how many are held.

The job `purge_expired` runs nightly at 02:00 in `platform.timezone`. For each active policy and each app in scope: it finds trashed records past `keep_trash_days`, history past `keep_history_days`, retired values past `keep_retired_days`, and records past `max_record_age_days` (moved to trash, or purged when `on_expiry` is `purge`); it drops any covered by a hold (phase 4); it writes the ledger entry `RecordsPurged` with the counts and the policy id; and then `purge::destroy` deletes, in batches of 1,000, the records, their index and link rows, comments, history (with `scaffoldry.purge` set to `on` for the transaction), and file references (`attachments.md` removes orphan blobs). The disclosure log keeps its entries and their record ids, because the fact of a disclosure outlives the record. After each app, it inserts a purge certificate holding the counts and the SHA-256 of the sorted list of destroyed record ids, which proves which records were destroyed without keeping them.

Routes, `/admin/purges` and `/admin/purges/{id}`, Platform Admin or `compliance`, in `ADMIN_ROUTES`: the certificates, downloadable as JSON.

Tests.

1. A record trashed 366 days ago with `keep_trash_days` 365 is destroyed by the job. One trashed 364 days ago is not.
2. Destruction removes the record, index rows, link rows, comments, history, and file references, and leaves the disclosure log.
3. The ledger entry is written before the first delete. Make the ledger append fail and nothing is destroyed.
4. The certificate's hash equals the hash of the sorted ids that the test destroyed, and the ids are not stored.
5. A workspace policy overrides the classification policy.
6. `on_expiry` of `purge` with the placeholder basis is refused.
7. The preview counts equal what a real run then destroys.
8. A job that dies halfway resumes and the final counts and certificate are correct, with no id destroyed twice.
9. Every `DELETE FROM` on records, history, links, comments, and file references is inside `purge::destroy`. A scan test checks.

## Phase 4 — legal holds

Migration `0051_legal_holds.sql`:

```sql
CREATE TABLE IF NOT EXISTS legal_holds (
    id UUID PRIMARY KEY,
    name VARCHAR(120) NOT NULL,
    case_ref VARCHAR(120) NOT NULL DEFAULT '',
    reason TEXT NOT NULL,
    scope_kind VARCHAR(10) NOT NULL,
    workspace_id VARCHAR(64),
    app_slug VARCHAR(64),
    table_id VARCHAR(64),
    filter JSONB,
    include_future BOOLEAN NOT NULL DEFAULT FALSE,
    created_by VARCHAR(255) NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    released_by VARCHAR(255),
    released_at TIMESTAMPTZ
);

CREATE TABLE IF NOT EXISTS hold_records (
    hold_id UUID NOT NULL REFERENCES legal_holds(id),
    record_id VARCHAR(64) NOT NULL,
    PRIMARY KEY (hold_id, record_id)
);
CREATE INDEX IF NOT EXISTS idx_hold_records_record ON hold_records (record_id);
```

`scope_kind` is `workspace`, `app`, `table`, or `query`. A workspace or app hold needs no list: `is_held(app, record)` checks the scopes. A table hold with `include_future` the same. A table hold without `include_future`, and a query hold, pin the records that match when the hold is created: a job, `pin_hold`, writes `hold_records` in batches. A query hold with `include_future` also pins matching records as they are written.

`is_held(app, record_id) -> bool` is the one function. `purge::destroy`, the retired-value strip, the orphan blob purge, and the retention expiry all call it. A held record may still be deleted to trash and restored. It is never destroyed.

Placing a hold. Platform Admin or `compliance`. `POST /admin/holds/preview` enqueues a job that returns the count by app, shown before saving: `This hold covers 1,204 records across 3 apps.` `POST /admin/holds` takes the scope, the name, the case reference, and the reason, writes the ledger entry `LegalHoldChanged` (added to `DecisionType::ALL`) first, then the row, then enqueues `pin_hold` when needed.

Releasing. `POST /admin/holds/{id}/release`, a person other than the one who placed it, with a reason, a ledger entry first. A released hold's pinned rows are removed.

Visibility. Only Platform Admins and `compliance` can list holds or see the marker `Under legal hold` in a record's drawer. Editors and owners see nothing. A held record deleted by an editor goes to trash, and the trash screen shows the normal time left.

Screen, a `Legal holds` panel in the console from the kit: the list with scope in words, count, who and when, `New hold` with the preview, and `Release` as a `ConfirmAction` that asks for a reason and names the second-person rule.

Tests.

1. A held record in trash past retention is not destroyed, and the certificate reports `held_skipped`. After release, the next run destroys it.
2. A workspace hold protects a record created after the hold. A query hold without `include_future` does not protect a later record.
3. Each of the four destruction paths, retention, retired-value strip, orphan blob purge, and app archive purge, calls `is_held`. A scan test finds no destruction that does not.
4. The person who placed a hold cannot release it. Another person can.
5. An editor sees no sign of a hold. A `compliance` caller sees it.
6. The preview count equals the pinned count.
7. Placing and releasing are ledger entries written before the change.
8. A `pin_hold` job killed halfway resumes and pins each record once.

## How to prompt Gemini

```
Read docs/plans/lifecycle.md and docs/plans/README.md.
Implement phase N only.
Write the failing test first and run it.
Do not add a dependency.
Never destroy data outside purge::destroy, and never without checking is_held.
Write the ledger entry before any destruction.
Do not read, copy, or adapt another product's source code.
Stop when the tests listed for phase N pass, and paste the command output.
```

## Enterprise objections

| Objection | Answer |
| --- | --- |
| A mistaken delete will lose a student's record. | Delete is a move to trash, with undo and restore. Only the retention job destroys, and only after the policy period. |
| We cannot prove we destroyed data on schedule. | Every purge writes a ledger entry first and a certificate with a hash of the destroyed ids. |
| Retention will destroy data under a legal dispute. | A hold overrides every destruction path. A scan test proves all of them check it, and release needs a second person. |
| A hold will tip off the people under investigation. | Only administrators and compliance see that a hold exists. Editors see nothing. |
| The default periods are wrong for us. | They are placeholders, labeled as such in the console until a records officer sets real ones, and a policy with a purge action cannot be saved with the placeholder basis. |
| Purge will break links and history. | Links are removed with the record. History is removed by the same policy. The disclosure log is kept, because a disclosure outlives the record. |
