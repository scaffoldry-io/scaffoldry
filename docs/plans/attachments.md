# Attachments — implementation brief

Hand this file to Gemini Flash. Do one phase per session. Stop when that phase's tests pass. Do not start the next phase.

Run order is `docs/plans/README.md`. Phase 1 needs `access-rules.md` phase 3 and `record-history.md` phase 4. Phase 2 needs `jobs.md` phase 1. Phase 4 needs `lifecycle.md` phase 3.

An admissions file has a transcript. A grant has a signed award letter. An IRB submission has a protocol document. Faculty will ask for places to put files. A file is also the easiest way to take data out of a governed system, and the easiest way to bring a virus in.

This brief adds a file store that is bounded, scanned, access-controlled, logged when sensitive, and removed by retention.

## Decisions already made

| Question | Answer |
| --- | --- |
| Where are files kept? | On a local volume the operator mounts, in a content-addressed layout. The path is a process fact: `SCAFFOLDRY_FILES_DIR` |
| Is the content trusted? | Never. The server decides the type from the bytes, serves every file as a download except a short list of images, and sends headers that stop a browser running it |
| How are files scanned? | By an antivirus daemon the operator runs, reached over its socket. Scaffoldry does not link scanning code. Without a scanner, files show as not scanned. A setting can refuse uploads when no scanner is configured |
| Who can download? | Anyone who can read the record and the attachment field. A sensitive file's download is logged |
| Is a file copied when linked in two records? | The bytes are stored once, by hash. Each use is its own reference |

## What exists today

Nothing. There is no file type, no upload route, and no storage.

## Out of scope

- Thumbnails and previews of documents.
- Full-text search inside files.
- Versioned files. A replacement is a new attachment and the old one stays in history until retention removes it.
- Encryption at the application layer. Operators use volume encryption, and the Deployment notes say so.
- Signed, public URLs.

## Rules for every phase

1. Write the failing test first. Run it. Confirm it fails because the behavior is missing. Then write the code.
2. Do not add a dependency. The scanner protocol and the type sniffing are short and hand-written.
3. Every download goes through the record and field access checks. There is no route that serves a file by hash alone.
4. A file is never served inline except `png`, `jpeg`, `gif`, and `webp`.
5. Use the kit for every screen.

## Phase 1 — the store, upload, and download

Migration `crates/scaffoldry-core/migrations/0032_attachments.sql`:

```sql
CREATE TABLE IF NOT EXISTS files (
    sha256 CHAR(64) PRIMARY KEY,
    size BIGINT NOT NULL,
    media_type VARCHAR(100) NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE IF NOT EXISTS file_refs (
    id UUID PRIMARY KEY,
    sha256 CHAR(64) NOT NULL REFERENCES files(sha256),
    app_slug VARCHAR(64) NOT NULL,
    table_id VARCHAR(64),
    record_id VARCHAR(64),
    field VARCHAR(64),
    name VARCHAR(255) NOT NULL,
    uploaded_by VARCHAR(255) NOT NULL,
    uploaded_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    scan_state VARCHAR(12) NOT NULL DEFAULT 'skipped',
    attached_at TIMESTAMPTZ,
    deleted_at TIMESTAMPTZ
);
CREATE INDEX IF NOT EXISTS idx_file_refs_record ON file_refs (record_id, field);
CREATE INDEX IF NOT EXISTS idx_file_refs_sha ON file_refs (sha256);
```

Files live at `$SCAFFOLDRY_FILES_DIR/aa/bb/{sha256}` where `aa` and `bb` are the first two byte pairs of the hash. Writes go to a temporary name in the same directory and are renamed after the hash is verified, so a crash never leaves a partial file under a real name. Boot refuses to start if the directory is missing or not writable.

`FieldType::Attachment { allow_multiple }`. A record holds a list of `{ ref_id, name }`. The record write validates each `ref_id` exists, belongs to this app, was uploaded by the writer or already attached to this record, and is not deleted.

Upload. `POST /apps/{slug}/files`, app write access, a streaming body with `Content-Length`, and the file name in a header. The server streams to a temporary file while hashing, enforces the size limit while streaming, and aborts early past it. It reads the first 512 bytes to decide the media type from a short table of signatures in the code: `png`, `jpeg`, `gif`, `webp`, `pdf`, `zip`, the Office zip formats, `csv` and plain text (by being valid UTF-8 with no control bytes), and `unknown`. The type the client sends is ignored. A name with path separators or control characters is cleaned. A blocked type (anything that is an executable, a script, `html`, `svg`, or a shortcut, decided by signature and by name) is refused with `400`. Limits are settings: `files.max_mb` (default 50, at most 500) and a per-app quota `files.app_quota_mb` (default 5,000), both added to the closed list in `foundation.md` phase 8. The response is `{ ref_id, name, size, media_type }`. An upload not attached to a record within 24 hours is removed by a scheduled job, `purge_unattached_files`.

Download. `GET /apps/{slug}/records/{id}/files/{ref_id}`. It checks read access to the record and visibility of the field (`access-rules.md`) and returns the bytes with:

- `Content-Disposition: attachment` with an encoded file name, except for the four image types, which are `inline`.
- `X-Content-Type-Options: nosniff`.
- `Content-Security-Policy: default-src 'none'; sandbox`.
- `Cache-Control: private, no-store`.

A file in an effective-sensitive field logs a disclosure of kind `file_download` (`record-history.md` phase 4), one row per download.

Add error codes `file_too_large`, `file_type_blocked`, and `quota_exceeded` (already added by `jobs.md`) to the closed list in `ux-standards.md`.

Tests.

1. Upload a PNG, attach it to a record, and download it with the right bytes and headers. The stored path matches the hash.
2. A file with the client's `Content-Type: image/png` and the bytes of an executable is refused. A text file named `x.png` is served as an attachment with a text type.
3. A file over `files.max_mb` is refused during streaming, and no file remains on disk.
4. Uploading the same bytes twice stores one blob and two references.
5. A person with no read access to the record gets 404 on the download. A person who can read the record but not the field does too.
6. A sensitive-field download writes one `file_download` disclosure row.
7. A blocked name (`a.exe`, `b.svg`, `c.html`) is refused. A name with `../` is cleaned.
8. An unattached upload older than 24 hours is removed by the job.
9. Boot refuses a missing files directory.

## Phase 2 — scanning

Add the setting `files.scanner`, an object `{ "kind": "clamd", "socket": "/run/clamav/clamd.ctl" }` or none, and `files.require_scan` (boolean, default false). Both are in the closed settings list. Neither turns a check off. `require_scan` only makes uploads fail when no scanner answers.

The scanner speaks the daemon's streaming command protocol over its socket, written by hand in under 100 lines: send the stream command, then length-prefixed chunks, a zero-length terminator, and read the one-line answer. A timeout of 30 seconds is an `error` result.

After upload, a job `scan_file` sets `scan_state` to `clean`, `infected`, `error`, or leaves `skipped` when no scanner is configured. While `pending`, the file cannot be attached or downloaded. An `infected` file is removed from disk when no other reference uses it, its reference is marked deleted, a ledger entry records the event with the hash and the uploader, and the uploader is told (`notifications.md`). The record never holds an infected file.

Where `require_scan` is true and no scanner answers, uploads fail with error code `scanner_unavailable`. Add it to the closed list.

The Overview shows the scanner state: configured, reachable, last result, and the count of `error` and `skipped` files.

Tests, with a fake scanner on a temporary socket that speaks the protocol and a configurable answer.

1. A clean answer sets `clean`. An infected answer deletes the blob, marks the reference, writes a ledger entry, and notifies the uploader.
2. A timeout sets `error`.
3. A `pending` file cannot be attached or downloaded.
4. With no scanner configured the state is `skipped` and a `Banner` states it. With `require_scan` an upload is `scanner_unavailable`.
5. The EICAR test string, sent through the fake scanner's matching rule, is treated as infected. (The real daemon is not run in tests.)
6. The Overview reports the scanner state.

## Phase 3 — in the app

From the kit.

- The cell editor and drawer: a drop target and a `Choose files` button, both keyboard operable, a list with name, size, media type, and scan state as text, `Remove`, and a download link. Progress shows as text and a bar while uploading, with `Cancel`. Errors use `explainError` with the specific code.
- The grid shows file chips: name and size. A record of images can show the first image as a gallery cover. Images load through the same authorized route.
- A `Not scanned` or `Scanning` marker is shown as text with an icon.
- A person without the field's visibility sees nothing at all, not an empty control.

Tests (vitest, `fetch` stubbed).

1. Choosing a file shows progress, then a chip. A refusal shows the specific message.
2. The drop target and the button are reachable and operable by keyboard.
3. A `Not scanned` marker is text.
4. Removing a file removes the chip and sends the update.
5. A hidden attachment field renders nothing.

## Phase 4 — retention, backups, and cleanup

1. A reference removed from a record, or a record that is purged (`lifecycle.md`), marks its `file_refs` row deleted. A scheduled job, `purge_orphan_blobs`, deletes a blob when no live reference remains and none was created in the last 24 hours, and writes a ledger entry with the count only.
2. Under a legal hold (`lifecycle.md` phase 4) a reference is never purged, and its blob stays.
3. Export and import carry attachments inside app bundles (`import-export.md` phase 3). A bundle holds files by hash with their bytes in a side directory, and verifies each hash on import.
4. `docs/ARCHITECTURE.md` section 5 backup: the files directory is part of the snapshot. The backup script (an operator document, not code in this repo) names both the database dump and the directory.

Tests.

1. Removing the only reference leaves the blob until the job runs, then deletes it. A second reference keeps it.
2. A held reference's blob is untouched by the job.
3. Importing a bundle with a corrupted file fails with its name and writes nothing.
4. The purge ledger entry holds a count and no file names.

## How to prompt Gemini

```
Read docs/plans/attachments.md and docs/plans/README.md.
Implement phase N only.
Write the failing test first and run it.
Do not add a dependency.
Never trust a client's content type or file name.
Never serve a file by hash alone.
Never serve anything inline except png, jpeg, gif, and webp.
Do not read, copy, or adapt another product's source code.
Stop when the tests listed for phase N pass, and paste the command output.
```

## Enterprise objections

| Objection | Answer |
| --- | --- |
| A file will run in someone's browser. | Everything except four image types is a download with `nosniff` and a sandboxing policy, and the type is decided from the bytes. |
| Malware will arrive through uploads. | A scanner is checked on every upload, an unscanned or infected file cannot be attached, and an operator can require a scanner. |
| A scanner license will contaminate the project. | The scanner is a separate daemon. Scaffoldry speaks its socket protocol and links nothing. |
| Files will leak around access rules. | There is no route by hash. Every download checks the record and the field, and a sensitive download is logged. |
| Files will pile up forever. | Unattached uploads are removed after 24 hours, and orphan blobs are purged by retention, except under hold. |
| A full disk will corrupt data. | A write goes to a temporary name and is renamed after verification. A quota stops an app before the disk fills. |
