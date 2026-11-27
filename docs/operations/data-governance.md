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
| `PUT /admin/datasets/{id}/label-floor` | Body `floor` (a list of category ids, at most 20, each defined in the settings) and `reason`. Every row of the dataset carries these categories, whatever its own tags say. A Platform Admin or a compliance officer may raise it. Lowering it needs a Platform Admin. The floor, the flag Cedar reads on each row and the ledger entry (`DataLabelChanged`, setting `label_floor`) commit together. The answer holds `floor` and `was` |

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
