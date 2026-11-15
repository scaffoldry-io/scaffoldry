# The Flows panel

`Flows` in the admin console lists every flow in every workspace. A Platform Admin reads it and changes it. Other callers get 403 on every route.

## What a row shows

- The flow's name, its app, and its home workspace.
- Each workspace the flow reaches, who approved it for that workspace, and when. A workspace with no recorded approval says `not recorded`.
- What the flow may read, write, and create, in plain sentences.
- The last run and its result, and the failures in the last 7 days out of the runs in the last 7 days.
- The state: `Active`, `Disabled`, or `Tripped`.

`Disabled` means an administrator turned the flow off. `Tripped` means the breaker turned it off after 20 failed runs in a row. A new version of the flow clears a trip. `Disabled` shows over `Tripped`.

Two filters narrow the list. `Crosses workspaces` keeps flows that reach more than one workspace. `Reads sensitive data` keeps flows with a sensitive field in what they may read. Sensitive means the field's effective sensitivity, so labels and categories count.

## Turning a flow off or on

`Disable` and `Enable` need a reason of 1 to 500 characters. Each one saves the setting `flows.disabled` and writes one ledger entry of type `PolicyRevision` with the reason. The flow stops from its next run, on every server, because every server reads the setting. A run that has started finishes. Nothing is undone.

## The Overview

The Overview shows how many flows there are, how many cross workspaces, how many are tripped, and how many work items the last nightly reconcile repaired. The last number says `none yet` until a reconcile has finished.

## Routes

| Method | Path | Behavior |
| --- | --- | --- |
| GET | `/admin/flows` | Query `crosses_workspaces`, `reads_sensitive`, `state`, `search` |
| POST | `/admin/flows/{rule_id}/disable` | Body `reason` |
| POST | `/admin/flows/{rule_id}/enable` | Body `reason` |
