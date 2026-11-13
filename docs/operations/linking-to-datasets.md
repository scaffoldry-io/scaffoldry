# Linking an app to a dataset

A dataset is a read-only copy of another system's data, filled by a connection. An app can link its records to a dataset's records. The link holds the dataset record's id. The sync keeps a record's id by its key, so the link holds when the record's other fields change.

## The link field

In the app's manifest, a `Link` field names the dataset app and its table:

```json
{
  "name": "program",
  "field_type": "Link",
  "link_target_app": "registrar-programs",
  "link_target_table": "programs",
  "link_cardinality": "One"
}
```

A lookup shows a value from the linked record, and shows the dataset's current value:

```json
{ "name": "program_name", "field_type": "Lookup", "computed_via": "program", "target_display_field": "name" }
```

Rules:

- The app that is named must be a dataset, and must have that table. A proposal that names another app or a missing table is refused, with the problem in words.
- The link is one-way. Only the sync writes a dataset, so there is no related field on the dataset's side. The link can only unlink when the record goes.
- A lookup can read a dataset. A rollup or a count cannot yet.
- A person may link only to a record of a dataset they may read. A record they may not see is refused in the same words as one that does not exist.
- The picker and the lookup follow the dataset's access rules. A person who may not read the dataset sees no options and no looked-up value, and a field hidden from them there is hidden here.

## When the source removes a record

The link is removed, and the record that held it shows no link. If the record returns, with the same key, the link returns with it.

## Turning a value join into a link

An older app may join a text field to a dataset by value: a `relationships` entry with `target_app` set to the dataset. The preview says how many values match one record, none, or several, and how many are blank. The conversion is proposed with a link to the dataset, flagged when any value matches none or several. Nothing is converted until a person approves it.
