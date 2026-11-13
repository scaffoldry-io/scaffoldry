# The drop directory

A data connection of kind `file_drop` copies a file into a read-only dataset on a schedule. The file comes from a directory that you mount. The server opens no network connection to read it.

## Set it up

1. Make a directory on the host, such as `/srv/scaffoldry/drop`.
2. Mount it read-only into every server instance, at the same path.
3. Set `SCAFFOLDRY_DROP_DIR` to that path.
4. Have the institution's own job place files there. For example, the registrar's export can arrive by SFTP into a directory you share with the mount.

A connection names a file by a path inside that directory, such as `sis/programs.csv`. The server refuses an absolute path, a path with `..`, and a link that leaves the directory.

## What a file may hold

- `csv`: a header row, then one row for each record. The header must match the connection's column map exactly. A column that is missing or extra stops the run with `schema_drift`.
- `jsonl`: one JSON object on each line.

Each row needs a value in the key column. Two rows may not share a key. A file may be up to 256 MB. A run holds the whole file in memory.

## What a run does

- The same file as last time ends the run as `unchanged`. Nothing is written.
- A fault in the file, such as a bad number or a repeated key, stops the run before anything is written.
- A file with fewer than half the rows of the last run stops with `source_shrank`. A Platform Admin can approve it, and the approval is on the ledger. This guards against a file that was cut short.
- Otherwise the run writes only the rows that changed, in batches of 1,000. A row whose key is gone goes to trash. It comes back if the key returns.

## When the source changes shape

A column the source added, a column it dropped, or a column that no longer holds what it was mapped to stops the run. The run makes one proposal for the dataset and says what differs: `The source added "Campus". The source no longer has "Dept code".`

- The connection shows `Needs review` until a person decides the proposal. A Platform Admin decides it, like any change to an app.
- Approving it changes the dataset's fields and the connection's column map together. The next run goes ahead and adds the new column's values.
- A dropped column's field is retired, not deleted. Its values stay in the records for the retention period.
- A column whose values no longer fit its type becomes text. The proposal carries a preview of the conversion with counts. The first run asks for the preview and the next one proposes. Nothing converts on its own. The next run writes the values in their new type.
- Rejecting the proposal leaves the connection stopped. A run does not propose the same change again. Put the source back as it was and the connection goes on. Or run it again by hand with `repropose`.
- A dropped key column is never proposed. Change the connection by hand.

## Who can change a dataset

Only its sync. Every route that writes a record answers `read_only_source`, and a database trigger refuses any other writer. Each change shows in the record's history with the sync as the actor.

## Without the variable

With no `SCAFFOLDRY_DROP_DIR`, a run fails with a clear message. The demo datasets are connected to their fixture files only when the variable is set.
