# Data governance

A compliance officer can raise a field's sensitivity at once, with no proposal. This page says how, and what it changes.

## Why a label and not an edit

Changing a field's sensitivity in an app's manifest is a structural change, so it needs a proposal and a second person. A compliance officer must be able to protect a field now. A label is an overlay on one field that can only add protection.

- A label of `true` protects the field.
- A label of `false` does nothing to a field the manifest or a category already protects. A label never lowers anything.
- Removing a label returns the field to what the manifest and the categories say.

## What a label changes

A label takes effect for every request after it is saved, on every server. For a protected field:

- a viewer's list no longer holds it, and a viewer's search on the table is refused;
- a new record with a value in it is flagged sensitive;
- every record of the table that already holds a value in it is flagged sensitive.

The flag of a record is set again from the table's effective sensitive fields. A record is flagged when it holds a value in any of them. A value is not `null`, not empty text and not an empty list. The same test runs when a record is written, so a write and a label agree. Removing a label can unflag a record, and it stays flagged if another field of the table still protects it.

A field of a table counts whether or not a view lists it. A sensitive field that no view shows is still sensitive.

## Who may

A Platform Admin and a compliance officer. Any other caller is `403`.

## The console

`Data governance` has two tabs.

**Fields.** Every field of every app. Filter by Sensitive, Not sensitive or Suggested, and search by app, table, field or label. Select rows, then `Mark sensitive` or `Remove label`. Each asks for a reason and sends one request. A badge says when a field is sensitive only through a label. A row opens to show where the field is used.

A field is `Suggested` when its CEDS code is protected under the standard and the field is not protected. The suggestion comes from the standard, never from the field's name.

**Datasets.** The published datasets, each with a classification list. Choose a level, give a reason and press `Save`.

## Routes

| Route | What it does |
| --- | --- |
| `GET /admin/fields` | One row for each field. Query: `search`, `app`, `table`, `sensitive`, `type`, `suggested`, `cursor`, `limit` (up to 500). Each row has the manifest flag, the label's flag, the effective flag, the CEDS code, `suggested` and `used_in` |
| `PUT /admin/labels` | Body `changes` (1 to 200 of `{ app_slug, table_id, field, ferpa_sensitive }`, where `null` removes the label) and `reason`. Every field must exist, or the whole request is `400` and nothing changes. One ledger entry, `DataLabelChanged`, `RA-02` |
| `GET /admin/datasets` | The published datasets: id, name, department, level, record count |
| `PUT /admin/datasets/{id}/classification` | Body `sensitivity_level` (`Public`, `Directory`, `Internal` or `Restricted / FERPA`) and `reason`. The column and the stored payload change together. Ledger `DataLabelChanged` |
| `PUT /admin/datasets/{id}/label-floor` | Body `floor` (a list of category ids, at most 20, each defined in the settings) and `reason`. Every row of the dataset carries these categories, whatever its own tags say. A Platform Admin or a compliance officer may raise it. It does not lower one: lowering is a label change that two approvers decide (see below), and this route answers `409` to it. The floor, the flag Cedar reads on each row and the ledger entry (`DataLabelChanged`, setting `label_floor`) commit together. The answer holds `floor` and `was` |

## Changing a label

A change to tags or to a floor is a **label change**. It is proposed, read, and decided. Nothing applies by itself. Lowering a floor goes through this path only.

| Route | What it does |
| --- | --- |
| `POST /data/datasets/{id}/label-changes` | Proposes a change. Body: `kind` (`tags` or `floor`), `reason`, and either `add`, `remove` and `records` (1 to 1,000 row ids) or `floor`. For the dataset's steward, a Platform Admin or a compliance officer. It is stored as `Pending`. It applies nothing |
| `GET /data/label-changes` | The changes the caller may decide. Query: `status`, `dataset` |
| `GET /data/label-changes/{id}` | The change and its evaluation, built now: `locked_out` (first when it is not empty), `what_changes`, `why`, `access`, `depends_on`, `options`, `undo` |
| `POST /data/label-changes/{id}/decide` | Body: `decision` (`approve`, `decline` or `change`) and `reason`. When the evaluation lists people who would be locked out of open work, an approval also names `open_work`: `now`, or `new_work_only`, which holds the change until that work has ended. There is no default |
| `POST /data/label-changes/{id}/apply` | Applies a change that was approved and held, once no one is locked out any more |
| `POST /data/label-changes/{id}/plan/approve` | The steward approves the communications plan of a scheduled change. Body: `reason`. Each person on the plan is told, and each notice is recorded. Nothing is sent before this. Approving again sends only what is missing |
| `POST /data/label-changes/{id}/withdraw` | The steward withdraws a scheduled change before its date. Body: `reason`. Its exceptions end and the people who were told are told |
| `POST /data/label-changes/{id}/exceptions` | A person who will lose access asks to keep it. Body: `justification` (1 to 500 characters) and `expires_at`. Not before the plan is approved, and not after the date |
| `GET /data/label-changes/{id}/exceptions` | The exceptions asked for a change, for its steward |
| `POST /data/label-changes/{id}/exceptions/{exception}/decide` | The steward approves or declines. Body: `decision`, `reason`. An approval records a viewer grant on the app with the exception's expiry |

The steward is the holder of the dataset's `steward_position`, found from its unit upward. A Platform Admin or a compliance officer may also decide.

Raising protection takes one approval. Removing it takes two, from two people, and neither is the proposer. Removing is lowering a floor, or removing a tag whose category is protected or has a requirement. Approving applies the change, sets again the flag Cedar reads, writes a history row for each row whose tags changed, and writes one `DataLabelChanged` ledger entry. The entry names the proposer, the deciders, the review finding if there is one, and the choice about open work. Apply on a date is below.

### Apply on a date

A steward may approve a change with `effective_at` instead of applying it now. Nothing changes and nobody is told yet. The change is `Scheduled`, and the answer holds a communications plan. The plan follows `governance/schema/label-change-plan.schema.json`. The server builds it from the evaluation: the people who lose access, the owners and admins of the app's workspace, the stewards, and the people who hold a compliance role. Nobody types the lists.

The steward reads the plan and approves it. Only then is each person on it told. A notice goes to the inbox, and to email where the person's preferences allow. It names the dataset and the days left, and no record value. The notice kinds are forced into the inbox. Each notice is recorded in `label_change_audience`.

**The notice period.** The date must be at least the minimum away. A shorter date is a 400. A steward may choose a longer one, up to 366 days. The minimum is the setting `sensitivity.notice_days`, by class of category. `protected` is a category marked protected, or one that has a requirement, or one that settings no longer define. `other` is any other category. A change takes the class of the most protective category it touches.

| Class | Raising (adds a label) | Lowering (only removes one) |
| --- | --- | --- |
| `protected` | 30 days | 14 days |
| `other` | 14 days | 7 days |

These are the defaults. An institution sets its own, as `{ "protected": { "raise": 45, "lower": 21 } }`. The schema is `sensitivity-settings.schema.json`.

**Exceptions.** After the plan is approved, a person who will lose access may ask to keep it: a justification and an `expires_at` after the date. The steward decides. An approved exception is a viewer grant on the app, source `request`, with that expiry. It waives the categories the change adds, for that person, on that dataset. It does so only after the change is applied, and only while the grant is neither revoked nor over. A request nobody decided by the date lapses. When a change tags named rows and other rows already carry the category, the exception is refused, because it would open those rows too.

**Reading it coming.** A read of the table's records returns `upcoming_changes` to a person who has a recorded notice. Each entry has the date, the days left, the categories added and removed, a sentence, whether the reader loses access, and the reader's own exception. It names no other person and no row.

**The date.** The job `apply_label_changes` runs every minute. It applies each scheduled change whose date has come and whose plan was approved, through the same path as "apply now", with one `DataLabelChanged` entry. Scheduling, approving the plan and withdrawing each write their own entry, with `setting` `label_change_schedule`.

**Withdrawing.** The steward may withdraw a change before its date. The job reads only scheduled changes, so nothing is left to run. Exceptions end and their grants are revoked. The people who were told are told.

Not built: reminders between the notice and the date, a notice on the day and a confirmation afterward, acknowledgements, the closing report, and the urgent path.

## Who may read a row

A category in the settings may carry a requirement, `requires`: a list of a position key and a reach. Any one of them is enough. The reach is `unit` (the unit where the holder holds the position), `unit_and_below`, or `anywhere` (any unit).

```json
{ "id": "phi", "name": "PHI", "protected": true, "detectors": [],
  "requires": [{ "position": "registrar", "reach": "unit_and_below" }] }
```

A row carries a category when its dataset's floor holds it, when its own tags hold it (`_tags` in the record's data), or when the table has a field that carries it. A reader may read the row when they hold one of the category's positions today, at a unit that covers the row's unit. The row's unit is its unit column. A row with no unit takes the dataset's own unit. A row with neither is read by nobody, unless the reach is `anywhere`. A category with no `requires` asks nothing, so nothing changes until an institution sets one. A Platform Admin is not held back, as with a row rule.

A table that names a unit column may also set `unit_required: true`. A row that names no unit is then refused, by a write, an import row and a sync. Taking the flag on needs a proposal and is refused while a row names none.

**Row tags.** A record's data may hold `_tags`, a list of category ids. A write that leaves `_tags` out keeps the stored tags. A write that names them replaces them. A tag is added by anyone who may write the row and who still reads it afterwards. A tag whose category is protected or carries a requirement is removed only by a Platform Admin or a compliance officer. Undo and restore never change tags. A tag change shows in the record's history as the field `_tags`.

The labels, the records' flags, the cache counter and the ledger entry commit in one transaction.

## Where a field is used

`used_in` is a count of:

- views of the table that list, sort, filter or group by the field, or show it on a form;
- other fields of the table whose formula names it;
- custom page grants that read or write it;
- rules whose predicates, step conditions or effects name it.

A rule names a field without a table. It counts for every table of its app that has a field of that name.
