# Sensitive content: scans, findings and authorized stores

Scans look for patterns such as Social Security numbers and card numbers in stored records. They write findings. They never change a label by themselves. A person decides.

## What a scan does

The job `scan_records` reads one table in batches of 1,000 records, in the order the table is already indexed in. It runs the enabled detectors over each text and number field. A number field is read as its digits. A field is read up to its first 64 KB.

A detector belongs to the first category that lists it. A detector no category lists is not run.

A scan is queued when someone asks for it (`POST /admin/content-scans`), and for every table when a detector or a category is added or changed. A scan already waiting for a table is not queued again.

A batch is one transaction. The job's checkpoint is saved in the same transaction. A worker that dies resumes after the last batch that counted, and no batch is counted twice. No batch holds a lock for longer than it takes to read 1,000 records and write a few rows.

A scan of a table starts its counts again, so a finding's counts are those of the last scan. A finding's status, its decision and its first-seen time stay.

## A finding

One row for each field and detector: the field, the detector, the category, `source` (`scan`), the number of matches, the number of records, up to five record ids, and its status (`open`, `confirmed` or `dismissed`).

A finding never holds a matched value. A value would put the sensitive data in a second place. The ledger, the job's log and its checkpoint hold none either. A test plants known values and checks the whole database for them.

- **Confirm** puts the finding's category on the field's label and sets the stored flags of the table's records again, in one transaction. The field is protected from the next request, on every server, with no job to wait for. A viewer who may not see sensitive fields no longer sees it.
- **Dismiss** changes nothing about the field. The finding stays dismissed through later scans.
- A new match in a field that was confirmed adds to its counts and changes nothing else.
- A decision is made once. Every decision is a ledger entry (`ContentFindingDecided`) with a reason.
- **When a detector changes**, its dismissed findings are open again, because the reason for a dismissal may not hold for a detector that changed. Every table is scanned again.

## Authorized stores

A store is a place the organization has said may hold a category: a workspace, an app, a table or a field. Designating one is a person's confirmation. The fields it covers are labeled with the category at once, and a match of that category inside it raises no finding. The match is counted as `expected` in the Overview.

Removing a designation leaves the labels in place until a person removes them. Matches there raise findings again from the next scan.

A designation may label at most 5,000 fields. Designate a smaller place if it holds more.

## Labels with categories

A label may carry categories as well as the flag. A label with categories is protected only through them: it is protected when one of its categories is marked `protected` in the settings. A label with no category is the flag it always was. The manifest's own flag protects on its own, whatever a label says.

## Routes

All are for a Platform Admin or a compliance officer.

| Route | What it does |
| --- | --- |
| `POST /admin/content-scans` | Body `app_slug` and `table_id`, or `all: true`. Answers `202` with the job id, or the number queued |
| `GET /admin/content-findings` | Filters `status`, `category`, `app_slug`. Paged |
| `POST /admin/content-findings/{id}/decide` | Body `decision` (`confirm` or `dismiss`) and `reason` |
| `GET /admin/authorized-stores` | The designations |
| `PUT /admin/authorized-stores` | Body `category`, `scope`, the scope's ids, and `reason` |
| `DELETE /admin/authorized-stores` | The same body. Removes the designation |
| `GET /admin/detector-stats` | For each detector: findings, confirmed, dismissed, and the dismissed share of those decided |
| `GET /admin/sensitivity-settings` | The categories, the detectors and which presets are on, for a compliance officer who cannot read the settings |

`PUT /settings/{key}` takes an optional `reason` in the query. It goes into the ledger entry. The console asks for one when a detector's action becomes `block`.

The Overview has `sensitive_content`: the open findings and the matches expected inside authorized stores.

## New writes, imports, and files

Every detector has an action: `flag`, `warn`, or `block`. The organization chooses it in `sensitivity.detectors`.

- **A write.** Only the fields the write changed are read. A field named as an authorized store for the detector's category is never a finding. `flag` records a finding after the write. `warn` also puts `sensitive_content` (field, detector, category, never the value) in the REST response. `block` refuses the write with `422` and the code `sensitive_content_blocked`. The message names the field, the detector, and the authorized stores for the category.
- **An import.** The dry run reports `sensitive_content` as column, field, detector, category, rows, and action. Rows that hit a `block` detector are errors. The run records one finding for each field and detector, with counts and without sample record ids.
- **A file.** After the antivirus result, a file a record holds is read once if it is `.txt`, `.csv`, `.json`, or `.md`, up to 4 MB. Findings have the source `file`. `content_scan_state` is `clean`, `found`, `unsupported`, or `skipped` (not yet read). A file that cannot be read says `Content not scanned` on the record. It is never shown as clean.

A write over 64 KB in one field is read only up to 64 KB, and the finding says so in its `note`.

## What is detected, and what is not

Detected: United States Social Security numbers and payment card numbers, in the text of fields and in the text of `.txt`, `.csv`, `.json` and `.md` files. The organization may add detectors and switch categories on.

Not detected:

- **PDFs and Office files.** They are not read. The record shows `Content not scanned`. A PDF with a Social Security number in it raises no finding.
- **Pictures, scans and other binary files.**
- **Values split across fields,** and values written in words.
- **Text past the read limits:** 64 KB in one field, and the first 4 MB of a file. The finding says when only part was read.
- **Anything before the first scan.** The Overview shows the tables no scan has covered.

A clean result means no detector matched in what was read. It does not mean the data holds nothing sensitive.

## History, exports, and the Overview

A confirmed label masks old versions in record history at read time. An export or snapshot that includes a confirmed field needs the export permission and writes one disclosure and one ledger entry. No export code knows the category: the label is enough. Findings are removed with their app. The Overview shows the open findings, the oldest, the last finished scan, the tables never scanned, and the matches expected inside authorized stores.

## What a scan costs

Measured on a runner like CI's, in release mode, on a table of 1,000,000 rows with one in a thousand holding a Social Security number:

- the whole scan took about 210 seconds, in 1,001 batches;
- the longest single batch, and so the longest any lock was held, was 409 ms;
- 404 record writes made through the real route while the scan ran all completed, with a median of 15 ms and the slowest at 73 ms.

The proof is `content_scan_scale_test`. It runs in the million-rows job, in release mode.

## The console

`Sensitive content` has three tabs.

- **Findings.** A table of findings, filtered by state and category. `Review` opens a drawer with the counts, links to the sample records (they open under the usual access rules), and `Confirm` and `Dismiss`, each with a sentence of what it does and a required reason. `Scan every table` queues scans.
- **Categories.** The categories, the presets with `Switch on` for one that is off, and each detector with its action and its stats. Changing an action to `block` shows the dismissed share first and asks for a reason. Changing the settings is a Platform Admin's.
- **Authorized stores.** The designations with `Remove`, and `Add` with a category and a searchable choice of an app, a table or a field.
