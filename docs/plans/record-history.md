# Record history — implementation brief

Hand this file to Gemini Flash. Do one phase per session. Stop when that phase's tests pass. Do not start the next phase.

Run order is `docs/plans/README.md`. Phase 1 needs `live-data.md` phases 1 and 2 and `row-scale.md` phase 1. Masking uses `effective_ferpa_sensitive` (`admin-console.md` phase 4, first step), which exists from the start and returns the manifest flag until labels exist. Phase 4 needs `jobs.md` phase 1.

The ledger records decisions: a publication, an approval, a policy change. It does not record that a registrar changed a student's status from `Pending` to `Admitted` at 14:03 on Tuesday. Today a record's previous value is gone the moment it is saved. Nobody can see who changed it, undo a mistaken paste, restore last week's value, discuss a record with a colleague, or answer a student who asks who has seen their data.

This brief adds four things that share one mechanism: a history of every change, undo and restore built on it, comments with mentions, and a log of disclosures of sensitive data.

## Standard

| Standard | Use |
| --- | --- |
| FERPA, 34 CFR 99.32 | An institution keeps a record of each request for and disclosure of personally identifiable information from education records, with listed exceptions. Whether a given event is a disclosure is a decision for counsel. The log records every candidate event so counsel can decide |
| NIST SP 800-53 AU-2, AU-3, AU-12 | Which events are logged, what each entry holds, and that the system can generate them |

## Decisions already made

| Question | Answer |
| --- | --- |
| Where is a change recorded? | In `record_history`, in the same transaction as the write, through `write_record`. Never in a second step |
| Is history append-only? | Yes. A trigger refuses `UPDATE` and `DELETE`. Only the retention purge in `lifecycle.md` may delete, through one named, ledgered path |
| Does history hold sensitive values? | Yes, because restore needs them. At read time a viewer who may not see a field sees that it changed and not what it held |
| Are computed fields recorded? | No. Only fields a person or agent can write. A computed value follows from them |
| Who can undo? | The person who made the change, or an owner or admin of the workspace, and only while nothing later touched the same fields |
| Are comments part of the record? | Yes. They follow the record's access, retention, and holds |
| Does the disclosure log decide what is a disclosure? | No. It records candidate events: an export, a shared link opened, a webhook delivery, a bulk read by an agent, a sync out |

## What exists today

| Fact | Where |
| --- | --- |
| A record update replaces the JSON document and keeps nothing | `service/records.rs` |
| `version` guards concurrent saves | `live-data.md` phase 2 |
| Every record write goes through one function | `row-scale.md` rule 4 |
| The grid has a one-level undo batch of its own | `data-grid-parity.md` phase 2 |
| Workspace roles are `owner`, `admin`, `editor`, `viewer` | `state.rs` |

## Out of scope

- A branch or merge model for records.
- Offline editing and later sync.
- History of structure. That is the manifest's version and the proposals.
- Rich-text comments, attachments in comments, and reactions.
- Threaded comments deeper than one level.

## Rules for every phase

1. Write the failing test first. Run it. Confirm it fails because the behavior is missing. Then write the code.
2. Do not add a dependency.
3. History and the write it describes commit together or not at all.
4. Every read of history, comments, or disclosures passes through the same access functions as the record itself. A person who cannot read a record cannot read its history.
5. Use the kit in `apps/web/src/ui/` for every screen.

## Phase 1 — history

Migration `crates/scaffoldry-core/migrations/0027_record_history.sql`:

```sql
CREATE TABLE IF NOT EXISTS record_history (
    id BIGSERIAL PRIMARY KEY,
    app_slug VARCHAR(64) NOT NULL,
    table_id VARCHAR(64) NOT NULL,
    record_id VARCHAR(64) NOT NULL,
    version INTEGER NOT NULL,
    action VARCHAR(10) NOT NULL,
    changes JSONB NOT NULL,
    actor VARCHAR(255) NOT NULL,
    actor_kind VARCHAR(12) NOT NULL,
    token_label VARCHAR(255),
    batch_id UUID NOT NULL,
    reverts_batch UUID,
    proposal_id UUID,
    at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
CREATE INDEX IF NOT EXISTS idx_history_record ON record_history (record_id, id DESC);
CREATE INDEX IF NOT EXISTS idx_history_app ON record_history (app_slug, id DESC);
CREATE INDEX IF NOT EXISTS idx_history_batch ON record_history (batch_id);
CREATE INDEX IF NOT EXISTS idx_history_actor ON record_history (actor, id DESC);

CREATE OR REPLACE FUNCTION record_history_immutable() RETURNS trigger AS $$
BEGIN
    IF TG_OP = 'DELETE' AND current_setting('scaffoldry.purge', true) = 'on' THEN
        RETURN OLD;
    END IF;
    RAISE EXCEPTION 'record_history is append-only';
END;
$$ LANGUAGE plpgsql;

DROP TRIGGER IF EXISTS record_history_no_change ON record_history;
CREATE TRIGGER record_history_no_change
    BEFORE UPDATE OR DELETE ON record_history
    FOR EACH ROW EXECUTE FUNCTION record_history_immutable();
```

`action` is `create`, `update`, `delete`, `restore`, or `comment`. `actor_kind` is `user`, `agent`, `automation`, `import`, `sync`, `system`, or `guest`. `token_label` is the label of the agent token, when there is one, so a reviewer can tell which agent acted. `changes` is `{ "field": { "old": ..., "new": ... } }`. A `create` has only `new`. A `delete` has only `old`. `batch_id` groups the records written by one action: a multi-cell paste, an import batch, one automation run.

`write_record` computes the difference between the stored and the new document, over writable fields only, and inserts one history row per record changed. It skips a write that changes nothing. Pass the actor, kind, token label, and batch id into `write_record` from the call site. Every path that writes a record sets them. A write with no actor is a compile error, because the parameter is not optional.

Reading.

| Method | Path | Who | Behavior |
| --- | --- | --- | --- |
| GET | `/apps/{slug}/records/{id}/history` | Read access to the record | Newest first, paged by id. Each entry: version, action, actor name and address, kind, token label, time, and the changes |
| GET | `/apps/{slug}/history` | App `owner` or `admin`, Platform Admin | An app-wide feed. Filters `table`, `actor`, `kind`, `from`, `to`. Paged |

Masking. When the reader may not see a field (the `ferpa_sensitive` viewer rule, or a column rule from `access-rules.md`), the entry shows `{ "field": { "changed": true } }` with no values. Do this in one function, `visible_changes(entry, reader)`, used by both routes and by the MCP tool `get_record_history`.

Tests.

1. Create, update two fields, and delete a record. `history` returns three entries in order with the right `old` and `new` values.
2. An update that changes nothing writes no history.
3. A computed field changing is not in `changes`.
4. A history row cannot be updated or deleted by SQL. A delete with `scaffoldry.purge` set to `on` works.
5. A history row and its record write fail together: make the history insert fail, and the record is unchanged.
6. A viewer who may not see a sensitive field gets `changed: true` and no values. An editor gets values.
7. A user with no read access to the record gets 404 on its history.
8. An agent token's label is in the entry. A paste of ten cells writes ten rows sharing one `batch_id`.
9. `write_record` cannot be called without an actor. (A compile-fail check, or a grep test that every caller passes one.)

## Phase 2 — undo and restore

Undo reverses a batch.

| Method | Path | Behavior |
| --- | --- | --- |
| POST | `/apps/{slug}/history/{batch_id}/undo` | The actor of the batch, or an app owner or admin. For each record in the batch, if every field the batch changed still holds the value the batch wrote, write the old values as a new batch with `reverts_batch` set and `action` of `update`, `restore`, or (for an undone create) `delete`. If any record has moved on, nothing is written and the response is 409 `undo_conflict` listing the records and fields |
| POST | `/apps/{slug}/records/{id}/restore` | App `owner`, `admin`, or `editor`. Body `to_version` and optional `fields`. Writes the old values of those fields as a new `restore` entry. Subject to the same `version` check as any save |

Undoing an undo is allowed and is itself undoable: it is the same operation on the reverting batch.

Add error code `undo_conflict` to the closed list in `ux-standards.md`.

The grid. Ctrl+Z and Ctrl+Shift+Z call undo on the person's last batch in this app within one hour, and redo on the last one they undid. The grid's own undo stack is deleted. A toast says what was undone in words: `Undid pasting 10 cells in Applicants.` A conflict shows `Someone changed this since. Undo stopped.` through `explainError`.

Tests.

1. Paste ten cells, undo: all ten return and one new batch exists with `reverts_batch` set.
2. Paste ten cells, another user edits one of them, undo: 409 listing that record, and nothing changed.
3. Undo of a create deletes the record to trash (`lifecycle.md`), and history records it.
4. Undo then undo again restores the original change.
5. A different editor cannot undo someone else's batch. An app admin can.
6. `restore` of one field to version 2 writes only that field and bumps the version once.
7. Web: Ctrl+Z calls the route and shows the toast. A 409 shows the stopped message.

## Phase 3 — comments and mentions

Migration `0028_record_comments.sql`:

```sql
CREATE TABLE IF NOT EXISTS record_comments (
    id UUID PRIMARY KEY,
    app_slug VARCHAR(64) NOT NULL,
    table_id VARCHAR(64) NOT NULL,
    record_id VARCHAR(64) NOT NULL,
    parent_id UUID,
    author VARCHAR(255) NOT NULL,
    body TEXT NOT NULL,
    mentions TEXT[] NOT NULL DEFAULT '{}',
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    edited_at TIMESTAMPTZ,
    deleted_at TIMESTAMPTZ
);
CREATE INDEX IF NOT EXISTS idx_comments_record ON record_comments (record_id, created_at);
```

Add the role `commenter`, ranking above `viewer` and below `editor`, to the workspace role list, and the Cedar action `comment` to the schema from `guards.md`. A `commenter`, `editor`, `admin`, and `owner` may comment. `access-rules.md` phase 1 migrates the member table. Until then, add the value to the check constraint or validation list only.

Body is plain text up to 4,000 characters. A mention is written `@[address]`. The server parses mentions, keeps only addresses of people who have read access to the app, and drops the rest without telling the author which were dropped, so a comment cannot probe who has access. The mention picker offers only people with read access.

Routes: `GET`, `POST` on `/apps/{slug}/records/{id}/comments`, and `PATCH`, `DELETE` on `/apps/{slug}/comments/{id}`. An author edits or soft-deletes their own comment within 15 minutes. An app owner or admin may soft-delete any. A deleted comment shows as `Comment removed`, with its author and time, and its body is cleared from the row. Every comment write also writes a `record_history` entry with `action` of `comment`. History records that a comment was added and by whom. It does not hold the text.

A mention creates a notification once `notifications.md` exists. Until then it only stores the mention.

Tests.

1. A viewer cannot comment. A commenter can and cannot edit the record.
2. A mention of someone without read access is dropped silently. A mention of someone with access is stored.
3. A body of 4,001 characters is 400.
4. An author cannot edit after 15 minutes. An admin can delete.
5. A deleted comment's body is gone from the response and from the database row.
6. Comments are not returned to someone who cannot read the record.
7. Web: the picker lists only people with access. A deleted comment shows the placeholder.

## Phase 4 — the disclosure log

Migration `0029_disclosure_log.sql`:

```sql
CREATE TABLE IF NOT EXISTS disclosure_log (
    id UUID PRIMARY KEY,
    at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    actor VARCHAR(255) NOT NULL,
    kind VARCHAR(20) NOT NULL,
    app_slug VARCHAR(64) NOT NULL,
    table_id VARCHAR(64),
    record_count BIGINT NOT NULL,
    fields TEXT[] NOT NULL,
    recipient VARCHAR(255),
    note TEXT NOT NULL DEFAULT '',
    records_truncated BOOLEAN NOT NULL DEFAULT FALSE
);
CREATE INDEX IF NOT EXISTS idx_disclosure_app ON disclosure_log (app_slug, at DESC);

CREATE TABLE IF NOT EXISTS disclosure_records (
    disclosure_id UUID NOT NULL REFERENCES disclosure_log(id),
    record_id VARCHAR(64) NOT NULL,
    PRIMARY KEY (disclosure_id, record_id)
);
CREATE INDEX IF NOT EXISTS idx_disclosure_record ON disclosure_records (record_id);
```

`kind` is `export`, `share_link`, `webhook`, `agent_read`, `sync_out`, or `file_download`. Later briefs call `log_disclosure(kind, app, table, records, fields, recipient, note)` at the moment of release. This phase builds the function, the tables, and the two kinds that already exist: `agent_read` and `export` through the existing CSV path.

An entry is written only when the released fields include an effective-sensitive field. Nothing else is a candidate.

`agent_read`: a call through an agent token that returns more than 25 records containing a sensitive field. The entry's `actor` is the user, the `recipient` is the token's label, and `note` says `read by an agent`. The reason: an agent forwards what it reads to a model provider. Whether that is a disclosure is counsel's call. The log makes the question answerable.

`disclosure_records` holds one row per record, up to 100,000 per entry. Above that, `records_truncated` is true and only the count and fields are kept.

Routes.

| Method | Path | Who | Behavior |
| --- | --- | --- | --- |
| GET | `/admin/disclosures` | Platform Admin or `compliance` | Filters `record`, `actor`, `kind`, `app`, `from`, `to`. Paged |
| GET | `/apps/{slug}/records/{id}/disclosures` | App owner or admin, Platform Admin, `compliance` | Every entry that includes this record, newest first. This is the answer to a student's request for their record of disclosures |
| GET | `/admin/disclosures/export` | Platform Admin or `compliance` | CSV or JSON, capped at 50,000 rows. Writing the export is itself a ledger entry |

Add `/admin/disclosures` to `ADMIN_ROUTES`.

Tests.

1. An export that includes a sensitive field writes one entry with the right count and fields and one `disclosure_records` row per record. An export of only non-sensitive fields writes nothing.
2. An agent token reading 26 records with a sensitive field writes an `agent_read` entry whose recipient is the token label. A user session reading the same writes none.
3. An export of 120,000 records sets `records_truncated` and keeps the count.
4. `GET .../records/{id}/disclosures` returns only entries that include that record, and 404s for a caller without access.
5. A faculty caller is 403 on `/admin/disclosures`. A `compliance` caller is 200.
6. Every call site that releases data to an agent or an export goes through `log_disclosure`. A test lists the call sites.

## Phase 5 — the screens

From the kit.

- In the record drawer, three tabs: `Details`, `History`, `Comments`, and for owners, admins, and `compliance`, `Disclosures`.
- `History` is a list of sentences: `Dr. Rivera changed Status from Pending to Admitted, 14:03.` An agent's change reads `Dr. Rivera, through "admissions agent", changed ...`. Each entry has `Restore this value` for editors and above, as a `ConfirmAction`.
- `Comments` has a composer with a mention picker, edit, and remove, and shows commenter-role limits in words.
- `Disclosures` lists each entry as `Exported 340 records including Student ID, by Dr. Rivera, 3 March.` with the recipient.
- An app-wide `Activity` page for owners and admins, from the feed.

Tests (vitest, `fetch` stubbed).

1. A history entry renders as a sentence with names, not ids.
2. `Restore this value` sends the restore request with the field and version.
3. A viewer sees no `Comments` composer. A commenter does.
4. `Disclosures` is absent for an editor.

## How to prompt Gemini

```
Read docs/plans/record-history.md, docs/plans/ux-standards.md, and docs/plans/README.md.
Implement phase N only.
Write the failing test first and run it.
Do not add a dependency.
Write history in the same transaction as the record.
Do not show a field's value to someone who may not see the field.
Do not read, copy, or adapt another product's source code.
Stop when the tests listed for phase N pass, and paste the command output.
```

## Enterprise objections

| Objection | Answer |
| --- | --- |
| History doubles the storage. | It stores only changed writable fields. A computed change is not stored. Retention in `lifecycle.md` removes it with the record. |
| History will expose sensitive values to people who should not see them. | It masks by the same rule as the record, in one function used by every reader. |
| An undo will overwrite someone's newer work. | Undo refuses when any changed field has moved on, and says which. |
| A comment mention will leak who has access. | A mention of a person without access is dropped silently. The picker lists only people with access. |
| The disclosure log will be treated as a legal conclusion. | It is a log of candidate events. Counsel decides what is a disclosure. The note on the log says so. |
| Agents reading data is not a disclosure. | That may be right. The log makes it checkable, and the threshold of 25 records is a constant to tune. |
