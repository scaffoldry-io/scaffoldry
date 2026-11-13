# The connections panel

The Connections section of the admin console shows every data connection. Only a Platform Admin sees it.

## What each row says

- **Last success.** When a run last applied or confirmed its source.
- **Last result.** What the last run did, in words: `Added 12, changed 3, removed 1`. A run that found the same file says `No change since the last run`. A run that failed says why.
- **State.**
  - `Healthy`: the last run did not fail.
  - `Failing`: the last run failed. The dataset keeps its last good data.
  - `Needs review`: the source changed shape and a proposal waits for a person.
  - `Disabled`: the schedule does not run it.
- **Next run.** From the schedule. A connection that is due now says now. A disabled connection, or one with no schedule, has none.

## What you can do

- **Run now.** Runs the connection once, at once.
- **Runs.** The newest 100 runs, newest first, each with its counts or its error.
- **Edit mapping.** Opens the column map with up to three values the last run wrote for each column. The map is read here. A column that the source adds, drops or changes comes as a proposal, as described in `drop-directory.md`.
- **Disable and Enable.** Each asks for a reason, and the reason goes on the ledger.
- **Approve smaller file.** Shown when a run stopped because the file lost more than half its rows. It asks for a reason.

## Who is told

A failed run tells the connection's owner and every Platform Admin who may read the dataset. It tells them once. While the connection keeps failing, later retries tell no one. When a run succeeds and a later run fails, that failure tells them again.

A change that needs review stops its run as a failure, so it tells them once in the same way.

The notice names the dataset and the table. It carries no error text and no record value.

## The Overview

The Overview shows how many connections are healthy, failing and needing review, and how old the oldest successful sync is. A disabled connection does not count toward that age, because it is off on purpose.
