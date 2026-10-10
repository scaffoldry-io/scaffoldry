# Sensitive content — implementation brief

Hand this file to Gemini Flash. Do one phase per session. Stop when that phase's tests pass. Do not start the next phase.

Run order is `docs/plans/README.md`. Phase 1 needs `admin-console.md` phase 4 (first part) and `foundation.md` phase 8. Phase 2 needs `admin-console.md` phase 4 (the rest), `jobs.md` phase 1, and `ux-standards.md` phase 1. Phase 3 needs `record-history.md` phase 1, `import-export.md` phase 1, and `attachments.md` phase 2. Phase 4 needs `record-history.md` phase 2.

Today a field is sensitive only because a person said so, and the only kind of sensitive is FERPA. A faculty member who types a Social Security number into a Notes field has made a sensitive record that nothing marks. A research group that holds health data has no way to say so. Scaffoldry can protect only what it knows about.

This brief makes sensitivity something the organization defines, and adds detection.

- A **category** is a named kind of sensitive data, such as FERPA, PHI, or PCI. The organization defines its categories. Presets for common ones are one switch away.
- A **detector** finds a pattern in text, such as a Social Security number or a card number. It belongs to a category.
- An **authorized store** is a place the organization has said may hold a category.
- A **finding** is a detector match outside an authorized store. A compliance officer confirms or dismisses it. A confirmed finding raises a label, which protects the field at once.

## Standard

| Standard | Use |
| --- | --- |
| NIST SP 800-122, Guide to Protecting the Confidentiality of PII | What counts as PII, and why confidentiality impact decides the protection |
| NIST SP 800-53 Rev 5, families RA (risk assessment), SI (system integrity), and PT (PII processing) | The controls this behavior supports. Confirm each id against the catalog after `oscal-catalog.md` phase 1 |
| ISO/IEC 7812-1 | The card number format and its Luhn check digit |
| PCI DSS | Why a primary account number must be found and kept in known places |
| SSA number assignment rules | An SSN never has area `000`, `666`, or `900`–`999`, group `00`, or serial `0000` |

The standards search is recorded here, as `standards-first` requires. No standard defines a portable detector format, so categories and detectors are Scaffoldry settings validated by a published schema.

## Decisions already made

| Question | Answer |
| --- | --- |
| Who defines what is sensitive? | The organization. Categories, detectors, protections, and authorized stores are data in settings. The code holds only the detector algorithms, because a checksum needs code |
| How does an organization turn on FERPA, PHI, or PCI? | A **preset** is a file in the release that holds one category with its detectors. A Platform Admin switches it on. Switching on copies it into the organization's settings. From then on it is the organization's, and they can edit it. A new release never changes it silently |
| What does a category protect? | A category marked `protected` is treated exactly as the FERPA flag is treated today: viewers without permission see no values, an export needs the export permission, a download or export is a disclosure, and no shared link includes the field |
| Does a detector change data or labels by itself? | Never. A detector writes a finding. A person decides. Only a confirmed finding raises a label, and a label can only add protection |
| What may a detector do on a match? | Its `action` is `flag`, `warn`, or `block`. All three are always available. The organization picks per detector. The shipped default is `flag` |
| How does an organization decide when to move from `flag` to `block`? | By measurement. Each detector shows its confirmed and dismissed counts. A detector that is almost always confirmed is a candidate for `block`. One that is mostly dismissed is not |
| Where may a category live? | In its **authorized stores**. A store is a workspace, an app, a table, or a field. Designating one is a person's confirmation: the field is labeled with the category at once, and matches inside it raise no finding. Outside every authorized store, the detector's action applies |
| Which detectors are on by default? | `us_ssn` and `payment_card`, in `flag` mode, through the `pii` and `pci` presets. Everything else is off |
| What does a finding store? | The field, the detector, counts, and up to five record ids. Never a matched value. A value would put the sensitive data in a second place |
| How are custom patterns written? | As a shape, not a regular expression. `#` is a digit, `A` is a letter, `?` is a letter or digit. Anything else is literal. A shape runs in time proportional to the text, so a bad pattern cannot stall the server |
| What is scanned? | Text and number fields of records, and the text of plain-text attachments (`txt`, `csv`, `json`, `md`). Other file types show `unsupported` and are never reported as clean |
| Is there a new dependency? | No. The matchers, Luhn, and the shape reader are short and hand-written |

## What exists today

| Fact | Where |
| --- | --- |
| A field has one boolean, `ferpa_sensitive`. A record is flagged when it holds a value in such a field. Values are never read | `admin-console.md` phase 4, `jobs.md` phase 3 |
| Every read of sensitivity goes through `effective_ferpa_sensitive`, after `admin-console.md` phase 4 (first part) | `admin-console.md` |
| Attachments are scanned for malware by an antivirus daemon. Content is not read | `attachments.md` phase 2 |
| `ARCHITECTURE.md` section 6.1 promises a "Data Sensitivity Scan" of proposals. It reads the schema only. No brief builds it | `docs/ARCHITECTURE.md` |
| Record history keeps old values so restore works. A viewer who may not see a field gets no values | `record-history.md` |
| The Cedar schema has `Record.is_ferpa_sensitive` and no category | `crates/scaffoldry-policy/schema/scaffoldry.cedarschema` |

## Out of scope

- Detecting names, addresses, or free-text meaning. That needs a language model, which this project does not link.
- Reading PDFs and Office files. An operator can add a text-extraction daemon later, reached over a socket like the antivirus daemon.
- Changing or removing stored values. Redaction and tokenization are separate work.
- Outbound email scanning.
- Pushing a changed preset into organizations that already switched it on. They can copy the new version by hand.
- Different protections for different categories. In this brief a category is protected or it is not. Per-category rules arrive through workspace guards, which can name a category once Cedar carries it (phase 1).

## Rules for every phase

1. Write the failing test first. Run it. Confirm it fails because the behavior is missing. Then write the code.
2. Do not add a dependency.
3. A finding, a log line, an error message, and a test failure never contain a matched value. A test proves it by scanning the output for the planted value.
4. A detector raises protection only through a person's confirmation or a person's designation of an authorized store.
5. The scan job never holds a database lock for longer than one batch.
6. Do not hard-code a category name in code, except the legacy mapping in phase 1. Behavior comes from settings.
7. Use the kit for every screen.

## Phase 1 — categories, presets, and detectors

**Categories.** The setting `sensitivity.categories` is a list of at most 50 objects, in the closed settings list from `foundation.md` phase 8:

```json
{ "id": "pci", "name": "Payment card data", "source": "PCI DSS",
  "protected": true, "detectors": ["payment_card"] }
```

`source` is a free label for the statute or standard, shown to reviewers. `detectors` names detector ids.

**Presets.** Files in `governance/sensitivity-presets/`, one per category, validated by the same schema: `ferpa.json`, `phi.json`, `pci.json`, `pii.json`. A preset holds a category and its detectors. A Platform Admin switches one on from the settings route. The route copies it into `sensitivity.categories` and `sensitivity.detectors` and writes one ledger entry. Switching on a preset whose id already exists is 409. Ship these presets:

| Preset | Category | Detectors |
| --- | --- | --- |
| `pii` | `pii` | `us_ssn` (`flag`), `email` (off) |
| `pci` | `pci` | `payment_card` (`flag`) |
| `phi` | `phi` | none built in. A medical record number is the organization's own shape, so the preset carries a disabled example |
| `ferpa` | `ferpa` | none built in. A student id is the institution's own shape, so the preset carries a disabled example |

`pii` and `pci` are switched on in a new installation. The others are off.

**Detectors.** Add `crates/scaffoldry-engine/src/detect.rs`. It is pure. It reads text and returns matches. It does no input or output.

| Kind | Rule |
| --- | --- |
| `us_ssn` | Nine digits as `###-##-####` or `### ## ####`. The SSA rules above must hold. An unseparated nine-digit number matches only when `allow_unseparated` is true, because order numbers look the same |
| `payment_card` | 13 to 19 digits, with single spaces or hyphens between groups, that pass the Luhn check |
| `email` | A local part, `@`, and a domain with a dot |
| `shape` | A custom shape of at most 64 characters, with an optional `checksum` of `luhn` |

The setting `sensitivity.detectors` is a list of at most 100 objects:

```json
{ "id": "ssn", "kind": "us_ssn", "action": "flag", "enabled": true }
```

Publish `governance/schema/sensitivity-settings.schema.json` and validate every save of either setting against it. A bad shape, an unknown detector id in a category, or a duplicate id is 400, with the path of the error.

**Effective sensitivity.** In `scaffoldry-engine`, replace the body of `effective_ferpa_sensitive` so it returns a `Sensitivity { protected: bool, categories: BTreeSet<String> }`. Keep the function name until phase 4, and add the method `is_protected()`. Callers that asked a yes-or-no question call `is_protected()`. A manifest `ferpa_sensitive: true` maps to the category `ferpa`. This mapping is the one place the name `ferpa` appears in code. A field is protected when any of its categories is marked `protected` in settings, or when it carries the legacy flag.

**Cedar.** Add `categories: Set<String>` to `Record` in the Cedar schema and `RecordCtx` in the entity builder. No default policy reads it. The golden matrix from `guards.md` phase 1 must still match. This lets a workspace guard name a category later.

Tests.

1. `4111 1111 1111 1111` is a card. `4111 1111 1111 1112` is not.
2. `123-45-6789` is an SSN by shape. `000-12-3456`, `666-12-3456`, `912-12-3456`, `123-00-4567`, and `123-45-0000` are not.
3. A nine-digit number without separators matches only with `allow_unseparated`.
4. A shape of `AA-####` matches `CS-1042` and not `C-1042`. A shape over 64 characters is rejected on save.
5. Text of one million characters scans in time proportional to its length. Measure it and record the time.
6. Switching on the `pci` preset adds its category and detector to settings and writes one ledger entry. Switching it on twice is 409. Editing the copied category does not touch the preset file.
7. A field with the legacy flag is protected and its categories hold `ferpa`. A field whose only category is marked `protected: false` is not protected.
8. An unknown detector id in a category is 400 with the path of the error.
9. Detect output contains offsets and counts. A test plants a known SSN and proves the planted text appears nowhere in the findings, the logs, or the ledger.
10. `grep -rn '"ferpa"' crates/ --include=*.rs` prints only the legacy mapping and tests. Paste the output.
11. The golden decision matrix from `guards.md` phase 1 still matches, with `categories` present on `Record`.

## Phase 2 — scan stored records and review findings

Migration `crates/scaffoldry-core/migrations/0059_content_findings.sql`:

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

CREATE TABLE IF NOT EXISTS authorized_stores (
    category VARCHAR(64) NOT NULL,
    scope VARCHAR(12) NOT NULL,
    app_slug VARCHAR(64) NOT NULL DEFAULT '',
    table_id VARCHAR(64) NOT NULL DEFAULT '',
    field VARCHAR(64) NOT NULL DEFAULT '',
    workspace_id VARCHAR(64) NOT NULL DEFAULT '',
    reason TEXT NOT NULL,
    set_by VARCHAR(255) NOT NULL,
    set_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (category, scope, workspace_id, app_slug, table_id, field)
);

ALTER TABLE data_labels ADD COLUMN IF NOT EXISTS categories TEXT[] NOT NULL DEFAULT '{}';
```

`source` is `scan`, `write`, `import`, or `file`. `status` is `open`, `confirmed`, or `dismissed`. `scope` is `workspace`, `app`, `table`, or `field`. The unique key makes a repeat finding update the row instead of adding one. The label table holds categories as well as the flag. A label with categories is protected only if one of them is marked `protected`.

**The job.** Add the job kind `scan_records`, with the other kinds in `jobs.md`. It reads one table in batches of 1,000 records by `id`, runs the enabled detectors over each text and number field, and upserts `content_findings`. A number field is read as its digits. A match inside an authorized store for that detector's category raises no finding and is counted as `expected` in the Overview. The job keeps its cursor in its payload, so a restart resumes.

A scan also runs when a detector or category is added or changed.

**Routes.**

| Method | Path | Who | Behavior |
| --- | --- | --- | --- |
| POST | `/admin/content-scans` | Platform Admin or `compliance` | Body `app_slug` and `table_id`, or `all: true`. Returns a job id |
| GET | `/admin/content-findings` | Same | Findings, filtered by `status`, `category`, `app_slug`. Paged |
| POST | `/admin/content-findings/{id}/decide` | Same | Body `decision` (`confirm` or `dismiss`) and `reason`. A confirm upserts the field's label with the category and a note. Both write a ledger entry |
| GET | `/admin/authorized-stores` | Same | The list, by category |
| PUT | `/admin/authorized-stores` | Same | Body `category`, `scope`, the scope's ids, and `reason`. Labels the covered fields with the category at once. Writes a ledger entry |
| DELETE | `/admin/authorized-stores` | Same | Same body. Removes the designation. The label stays until a person removes it |
| GET | `/admin/detector-stats` | Same | For each detector: findings, confirmed, dismissed, and the dismissed share |

A dismissed finding stays dismissed until its detector changes. A new match in a confirmed field adds to its counts and changes nothing else.

**The screen.** Admin console, section `Sensitive content`, with tabs `Findings`, `Categories`, and `Authorized stores`.

- `Findings`: a table of open findings with the app, field, category, counts, and `Review`. `Review` opens a drawer with the sample record ids as links that follow normal access rules, the consequence sentence for `Confirm` (`This field will be treated as sensitive. Exports and reads will be restricted.`), a required reason, and `Dismiss`. Use `ConfirmAction`.
- `Categories`: the list from settings, the presets with a `Switch on` button, and for each detector its action and its stats. Changing an action to `block` shows the dismissed share first and asks for a reason.
- `Authorized stores`: the list, and `Add` with a searchable choice of workspace, app, table, or field.

Tests, with `fetch` stubbed for the screen.

1. A table with a planted SSN in a free-text field produces one finding with the right counts and no value.
2. A scan interrupted after 3 batches resumes from its cursor and ends with the same findings as an uninterrupted scan.
3. `Confirm` raises the label with the category. A read by a viewer who may not see sensitive fields is masked at once, before any recompute job runs.
4. `Dismiss` keeps the field unlabeled and the finding does not return on the next scan.
5. A field designated as an authorized store for `pci` is labeled at once. A card number in it raises no finding and counts as `expected`. The same number in another table raises a finding.
6. Removing a designation leaves the label in place.
7. The detector stats count confirmed and dismissed decisions correctly.
8. A faculty member who is not `compliance` gets 403 on every route.
9. A scan of a one-million-row table runs in batches. Measure total time and the longest single lock, and record both. A concurrent record write completes while the scan runs. Record its latency.
10. `grep -rn` for the planted value across the findings table and the ledger prints nothing.

## Phase 3 — new writes, imports, and files

**Write path.** After a record is validated, run the enabled detectors over the fields that changed. Look at no more than 64 KB per field. A match inside an authorized store for its category is ignored. Otherwise apply the detector's action:

| Action | Result |
| --- | --- |
| `flag` | The write succeeds. A finding is upserted with source `write` |
| `warn` | The write succeeds. The response carries `sensitive_content` with the field, the detector name, and the category. Never the value |
| `block` | The write fails with `422` and the error code `sensitive_content_blocked`, naming the field and the detector, and saying which authorized stores exist for the category. Add the code to the closed list from `ux-standards.md` |

**Import.** The dry run in `import-export.md` phase 1 reports, for each column, the detectors that matched and how many rows. The wizard shows it before commit. A `block` detector makes the matching rows rejected rows in the report.

**Files.** Migration `crates/scaffoldry-core/migrations/0060_attachment_content_scan.sql` adds `content_scan_state` to the attachment table: `clean`, `found`, `unsupported`, or `skipped`. After the antivirus result, a job `scan_file_content` reads the text of a plain-text attachment and upserts findings with source `file`. Other types are `unsupported`, and the file screen says `Content not scanned` as text.

Tests.

1. A write with a card number in a text field under `flag` succeeds and leaves a finding. Under `block` it is `422` and nothing is stored.
2. The same write into a field designated as an authorized store for `pci` succeeds with no finding, under every action.
3. A write that changes only a clean field runs no detector over the unchanged sensitive field.
4. The `warn` response contains the field and detector and not the value.
5. An import dry run lists the matching columns and counts. A `block` detector produces rejected rows.
6. A CSV attachment with a planted SSN has `content_scan_state` of `found`. A PDF is `unsupported`.
7. A write of a 10 MB string scans only the first 64 KB and says so in the finding note.
8. Changing a detector from `flag` to `block` takes effect on the next write, with no restart.

## Phase 4 — coverage and surfaces

- A confirmed label masks history at read time. Test an old version that held the value.
- An export or snapshot that includes a confirmed field needs the export permission and writes a disclosure, with no code change beyond the label. Test both.
- Rename `effective_ferpa_sensitive` to `effective_sensitivity` everywhere. The grep from `admin-console.md` phase 4 is the checklist. Paste the output before and after.
- The Overview shows the number of open findings, the oldest open finding, the last scan, the count of tables never scanned, and the `expected` count in authorized stores.
- Findings follow retention. A finding for a deleted app is removed with it.
- The Deployment notes state what is and is not detected, and that PDFs and Office files are not read.

Tests.

1. After `Confirm`, the history entry for an old version shows `changed: true` and no value to a viewer who may not see the field.
2. A sensitive export writes one disclosure and one ledger entry.
3. `grep -rn effective_ferpa_sensitive crates/` prints nothing.
4. The Overview counts match the findings table.
5. Deleting an app removes its findings.

## How to prompt Gemini

```
Read docs/plans/sensitive-content.md, docs/plans/ux-standards.md, and docs/plans/README.md.
Implement phase N only.
Write the failing test first and run it.
Do not add a dependency.
Never store, log, or return a matched value. Counts, field names, and record ids only.
A detector never changes a label. Only a person's confirmation or designation does.
Never use a regular expression for a custom detector. Use the shape reader.
Do not hard-code a category name. Behavior comes from settings.
Do not read, copy, or adapt another product's source code.
Stop when the tests listed for phase N pass, and paste the command output.
```

## Enterprise objections

| Objection | Answer |
| --- | --- |
| The scanner will copy sensitive data into a second place. | A finding holds counts, field names, and at most five record ids. A test plants a value and proves it appears nowhere else |
| A detector will mislabel data and lock people out. | A detector never changes a label. A compliance officer confirms, with a reason, and the change is in the ledger |
| We cannot tell when a detector is ready to block writes. | Each detector shows its confirmed and dismissed counts. Changing to `block` shows the dismissed share first and needs a reason |
| Some systems must hold card or health data. | A Platform Admin or compliance officer designates authorized stores. Matches there are expected, counted, and protected, and raise no findings |
| Our regime is not FERPA. | Categories are data. A preset switches on PHI or PCI. An organization can write its own, with its own shapes, and sees the findings before anything is labeled |
| A preset will change under us. | Switching one on copies it. A new release never edits the organization's copy |
| A custom pattern will take the server down. | There are no regular expressions. A shape runs in time proportional to the text, and it is at most 64 characters |
| Scanning will slow the system for everyone. | It runs as a background job in batches of 1,000 with no lock held across batches. The measured times and the concurrent write latency are on record |
| False positives will bury the reviewers. | An SSN must pass the SSA rules, a card must pass Luhn, and an unseparated number is ignored by default. A dismissal sticks until the detector changes |
| It will give a false sense that all data is checked. | The Overview shows tables never scanned. Files the scanner cannot read show `Content not scanned`, never `clean` |
| A block rule will stop legitimate work. | `block` is the organization's choice per detector, with the dismissed share in front of them. The error names the authorized stores where the data may go |
