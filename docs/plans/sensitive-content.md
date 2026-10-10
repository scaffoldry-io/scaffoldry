# Sensitive content — implementation brief

Hand this file to Gemini Flash. Do one phase per session. Stop when that phase's tests pass. Do not start the next phase.

Run order is `docs/plans/README.md`. This brief is not in the run order yet. Johann orders it. Phase 1 needs `admin-console.md` phase 4 (first part) and `foundation.md` phase 8. Phase 2 needs `jobs.md` phase 1, `admin-console.md` phase 4, and `ux-standards.md` phase 1. Phase 3 needs `record-history.md` phase 1, `import-export.md` phase 1, and `attachments.md` phase 2. Phase 4 needs `record-history.md` phase 2.

Today a field is sensitive only because a person said so. A faculty member who types a Social Security number into a Notes field has made a sensitive record that nothing marks. Scaffoldry can protect only what it knows about.

This brief adds detection. The organization lists what counts as sensitive. Scaffoldry looks for it in stored records, in new writes, in imports, and in text files. It never acts alone. A finding goes to a compliance officer, who confirms or dismisses it. A confirmed finding raises the field's label, which protects it at once.

## Standard

| Standard | Use |
| --- | --- |
| NIST SP 800-122, Guide to Protecting the Confidentiality of PII | What counts as PII, and why confidentiality impact decides the protection |
| NIST SP 800-53 Rev 5, families RA (risk assessment), SI (system integrity), and PT (PII processing) | The controls this behavior supports. Confirm each id against the catalog after `oscal-catalog.md` phase 1 |
| ISO/IEC 7812-1 | The card number format and its Luhn check digit |
| PCI DSS | Why a primary account number must be found and kept out of general data stores |
| SSA number assignment rules | An SSN never has area `000`, `666`, or `900`–`999`, group `00`, or serial `0000` |

The standards search is recorded here, as `standards-first` requires. No standard defines a portable detector format, so detectors are Scaffoldry settings validated by a published schema.

## Decisions already made

| Question | Answer |
| --- | --- |
| Who defines what is sensitive? | The organization, in a setting. A Platform Admin edits it. Each change is a ledger entry |
| Does a detector change data or labels by itself? | Never. A detector writes a finding. A person decides. Only a confirmed finding raises a label, and a label can only add protection |
| What does a finding store? | The field, the detector, counts, and up to five record ids. Never a matched value. A value would put the sensitive data in a second place |
| How are patterns written? | As a shape, not a regular expression. `#` is a digit, `A` is a letter, `?` is a letter or digit. Anything else is literal. A shape runs in time proportional to the text, so a bad pattern cannot stall the server |
| What is scanned? | Text and number fields of records, and the text of plain-text attachments (`txt`, `csv`, `json`, `md`). Other file types show `unsupported` and are never reported as clean |
| Is there a new dependency? | No. The matchers, Luhn, and the shape reader are short and hand-written |

## What exists today

| Fact | Where |
| --- | --- |
| A record is flagged sensitive when it holds a value in a field that is already labeled sensitive. Values are never read | `admin-console.md` phase 4, `jobs.md` phase 3 |
| Attachments are scanned for malware by an antivirus daemon. Content is not read | `attachments.md` phase 2 |
| `ARCHITECTURE.md` section 6.1 promises a "Data Sensitivity Scan" of proposals. It reads the schema only. No brief builds it | `docs/ARCHITECTURE.md` |
| Record history keeps old values so restore works. A viewer who may not see a field gets no values | `record-history.md` |

## Out of scope

- Detecting names, addresses, or free-text meaning. That needs a language model, which this project does not link.
- Reading PDFs and Office files. An operator can add a text-extraction daemon later, reached over a socket like the antivirus daemon.
- Changing or removing stored values. Redaction and tokenization are separate work.
- Outbound email scanning.
- A detector for every country. The organization writes a shape for others.

## Open decisions

Johann decides these before phase 3. Each has a recommendation.

| Decision | Recommendation |
| --- | --- |
| May a detector have the action `block`, which rejects a write that matches? | Yes, but off unless a Platform Admin sets it per detector. The default action is `flag`. A block stops legitimate work when a detector is wrong |
| Are `us_ssn` and `payment_card` on by default? | Yes, in `flag` mode. Everything else is off |
| Should `ferpa_sensitive` be renamed to a general sensitive flag? | Not in this brief. A confirmed finding sets the existing flag and records the category in the label note. A rename touches every brief |
| Where does this sit in the run order? | After `jobs.md` phase 1. Before `row-scale.md` phase 6, so the million-row proof includes a scan |

## Rules for every phase

1. Write the failing test first. Run it. Confirm it fails because the behavior is missing. Then write the code.
2. Do not add a dependency.
3. A finding, a log line, an error message, and a test failure never contain a matched value. A test proves it by scanning the output for the planted value.
4. A detector can raise protection only through a person's confirmation.
5. The scan job never holds a database lock for longer than one batch.
6. Use the kit for every screen.

## Phase 1 — detectors and findings

Add `crates/scaffoldry-engine/src/detect.rs`. It is pure. It reads text and returns matches. It does no input or output.

Built-in kinds:

| Kind | Rule |
| --- | --- |
| `us_ssn` | Nine digits as `###-##-####` or `### ## ####`. The SSA rules above must hold. An unseparated nine-digit number matches only when `allow_unseparated` is true, because order numbers look the same |
| `payment_card` | 13 to 19 digits, with single spaces or hyphens between groups, that pass the Luhn check |
| `email` | A local part, `@`, and a domain with a dot |

Custom detectors use `shape`, with an optional `checksum` of `luhn`. A shape is at most 64 characters.

Setting `sensitive.detectors`, in the closed settings list from `foundation.md` phase 8. It is a list of at most 50 objects:

```json
{ "id": "ssn", "name": "Social Security number", "kind": "us_ssn", "category": "SSN",
  "action": "flag", "enabled": true }
```

`action` is `flag` or `warn`. `block` is allowed only if the open decision says so. Publish `governance/schema/sensitive-detectors.schema.json` and validate every save against it. A bad shape or a duplicate id is 400, with the path of the error.

Migration `crates/scaffoldry-core/migrations/00NN_content_findings.sql`. The number comes from the README when the brief is ordered.

```sql
CREATE TABLE IF NOT EXISTS content_findings (
    id UUID PRIMARY KEY,
    app_slug VARCHAR(64) NOT NULL,
    table_id VARCHAR(64) NOT NULL,
    field VARCHAR(64) NOT NULL,
    detector_id VARCHAR(64) NOT NULL,
    category VARCHAR(64) NOT NULL,
    source VARCHAR(12) NOT NULL,
    match_count BIGINT NOT NULL,
    record_count BIGINT NOT NULL,
    sample_record_ids TEXT[] NOT NULL DEFAULT '{}',
    status VARCHAR(12) NOT NULL DEFAULT 'open',
    decided_by VARCHAR(255),
    decided_at TIMESTAMPTZ,
    reason TEXT,
    first_seen TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    last_seen TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE (app_slug, table_id, field, detector_id)
);
```

`source` is `scan`, `write`, `import`, or `file`. `status` is `open`, `confirmed`, or `dismissed`. The unique key makes a repeat finding update the row instead of adding one.

Tests.

1. `4111 1111 1111 1111` is a card. `4111 1111 1111 1112` is not.
2. `123-45-6789` is an SSN by shape. `000-12-3456`, `666-12-3456`, `912-12-3456`, `123-00-4567`, and `123-45-0000` are not.
3. A nine-digit number without separators matches only with `allow_unseparated`.
4. A shape of `AA-####` matches `CS-1042` and not `C-1042`. A shape over 64 characters is rejected on save.
5. Text of one million characters scans in time proportional to its length. Measure it and record the time.
6. Saving an invalid detector list is 400 with the error path. A valid save writes a ledger entry with the old and new ids and no other content.
7. Detect output contains offsets and counts. A test plants a known SSN and proves the planted text appears nowhere in the findings, the logs, or the ledger.

## Phase 2 — scan stored records

Add the job kind `scan_records`, with the other kinds in `jobs.md`. It reads one table in batches of 1,000 records by `id`, runs the enabled detectors over each text and number field, and upserts `content_findings`. A number field is read as its digits. The job keeps its cursor in its payload, so a restart resumes.

A Platform Admin or `compliance` starts a scan for one table, or for every table, from the new Admin console section `Sensitive content`. A scan also runs when a detector is added or changed.

Routes.

| Method | Path | Who | Behavior |
| --- | --- | --- | --- |
| POST | `/admin/content-scans` | Platform Admin or `compliance` | Body `app_slug` and `table_id`, or `all: true`. Returns a job id |
| GET | `/admin/content-findings` | Same | Findings, filtered by `status`, `category`, `app_slug`. Paged |
| POST | `/admin/content-findings/{id}/decide` | Same | Body `decision` (`confirm` or `dismiss`) and `reason`. A confirm upserts the field's label with the flag true and a note naming the category. Both write a ledger entry |

A dismissed finding stays dismissed until the detector changes. A new match in a confirmed field adds to its counts and changes nothing else.

The screen: a table of open findings with the app, field, category, counts, and `Review`. `Review` opens a drawer with the sample record ids as links that follow normal access rules, the consequence sentence for `Confirm` (`This field will be treated as sensitive. Exports and reads will be restricted.`), a required reason, and `Dismiss`. Use `ConfirmAction`.

Tests, with `fetch` stubbed for the screen.

1. A table with a planted SSN in a free-text field produces one finding with the right counts and no value.
2. A scan interrupted after 3 batches resumes from its cursor and ends with the same findings as an uninterrupted scan.
3. `Confirm` raises the label. A read by a viewer who may not see sensitive fields is masked at once, before any recompute job runs.
4. `Dismiss` keeps the field unlabeled and the finding does not return on the next scan.
5. A faculty member who is not `compliance` gets 403 on every route.
6. A scan of a one-million-row table runs in batches. Measure total time and the longest single lock, and record both. A concurrent record write completes while the scan runs. Record its latency.
7. `grep -rn` for the planted value across the findings table and the ledger prints nothing.

## Phase 3 — new writes, imports, and files

Write path. After a record is validated, run the enabled detectors over the fields that changed. Look at no more than 64 KB per field. Apply each matching detector's action:

| Action | Result |
| --- | --- |
| `flag` | The write succeeds. A finding is upserted with source `write` |
| `warn` | The write succeeds. The response carries `sensitive_content` with the field, the detector name, and the category. Never the value |
| `block` | The write fails with `422` and the error code `sensitive_content_blocked`, naming the field and the detector. Add the code to the closed list from `ux-standards.md` |

Import. The dry run in `import-export.md` phase 1 reports, for each column, the detectors that matched and how many rows. The wizard shows it before commit.

Files. After the antivirus result, a job `scan_file_content` reads the text of a plain-text attachment and upserts findings with source `file`. Add `content_scan_state` to the attachment: `clean`, `found`, `unsupported`, or `skipped`. Other types are `unsupported` and the file screen says `Content not scanned` as text.

Tests.

1. A write with a card number in a text field under `flag` succeeds and leaves a finding. Under `block` it is `422` and nothing is stored.
2. A write that changes only a clean field runs no detector over the unchanged sensitive field.
3. The `warn` response contains the field and detector and not the value.
4. An import dry run lists the matching columns and counts.
5. A CSV attachment with a planted SSN has `content_scan_state` of `found`. A PDF is `unsupported`.
6. A write of a 10 MB string scans only the first 64 KB and says so in the finding note.

## Phase 4 — coverage and surfaces

- A confirmed label masks history at read time. Test an old version that held the value.
- An export or snapshot that includes a confirmed field needs the export permission and writes a disclosure, with no code change beyond the label. Test both.
- The Overview shows the number of open findings, the oldest open finding, the last scan, and the count of tables never scanned.
- Findings follow retention. A finding for a deleted app is removed with it.
- The Deployment notes state what is and is not detected, and that PDFs and Office files are not read.

Tests.

1. After `Confirm`, the history entry for an old version shows `changed: true` and no value to a viewer who may not see the field.
2. A sensitive export writes one disclosure and one ledger entry.
3. The Overview counts match the findings table.
4. Deleting an app removes its findings.

## How to prompt Gemini

```
Read docs/plans/sensitive-content.md, docs/plans/ux-standards.md, and docs/plans/README.md.
Implement phase N only.
Write the failing test first and run it.
Do not add a dependency.
Never store, log, or return a matched value. Counts, field names, and record ids only.
A detector never changes a label. Only a person's confirmed finding does.
Never use a regular expression for a custom detector. Use the shape reader.
Do not read, copy, or adapt another product's source code.
Stop when the tests listed for phase N pass, and paste the command output.
```

## Enterprise objections

| Objection | Answer |
| --- | --- |
| The scanner will copy sensitive data into a second place. | A finding holds counts, field names, and at most five record ids. A test plants a value and proves it appears nowhere else |
| A detector will mislabel data and lock people out. | A detector never changes a label. A compliance officer confirms, with a reason, and the change is in the ledger |
| A custom pattern will take the server down. | There are no regular expressions. A shape runs in time proportional to the text, and it is at most 64 characters |
| Scanning will slow the system for everyone. | It runs as a background job in batches of 1,000 with no lock held across batches. The measured times and the concurrent write latency are on record |
| False positives will bury the reviewers. | An SSN must pass the SSA rules, a card must pass Luhn, and an unseparated number is ignored by default. A dismissal sticks until the detector changes |
| It will give a false sense that all data is checked. | The Overview shows tables never scanned. Files the scanner cannot read show `Content not scanned`, never `clean` |
| We need to detect our own identifiers. | A Platform Admin writes a shape, with an optional Luhn check, and sees the findings before anything is labeled |
| A block rule will stop legitimate work. | `block` is off unless a Platform Admin enables it for one detector. The default is to flag |
