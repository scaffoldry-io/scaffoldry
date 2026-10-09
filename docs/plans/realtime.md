# Real-time updates — implementation brief

Hand this file to Gemini Flash. Do one phase per session. Stop when that phase's tests pass. Do not start the next phase.

Run order is `docs/plans/README.md`. Phase 1 needs `live-data.md` phase 2 and `foundation.md` phase 4. Phase 3 needs `notifications.md` phase 1 and `jobs.md` phase 1.

Two registrars edit the same applicant list. Today the second one learns about the first one's change only when their own save fails with a version conflict. A person watching a board expects to see a card move when a colleague moves it. Hundreds of people building and working at once is the stated load, and a stale screen is the usual way they would overwrite each other.

## Decisions already made

| Question | Answer |
| --- | --- |
| Which transport? | Server-sent events over plain HTTP. It is one-way, works through ordinary proxies, and needs no new dependency or protocol. Editing still goes through the existing save routes |
| How do events cross processes? | PostgreSQL `LISTEN` and `NOTIFY`. The appliance is one process today, and this keeps working when it is two |
| Does an event carry data? | Never a field value. It carries ids and a version. The client fetches the record through the normal, authorized route, so every access rule applies |
| How does the browser authenticate? | The client reads the stream with `fetch` and a streaming reader and sends the token in the `Authorization` header. It does not use `EventSource`, because that cannot set headers and would put a token in a URL |
| Does the plan add cursors or live typing? | No. Presence shows who is in an app. It does not show where |

## What exists today

| Fact | Where |
| --- | --- |
| A save carries a `version`, and a stale save is `409` | `live-data.md` phase 2 |
| Every record write goes through one function | `row-scale.md` rule 4 |
| Out of scope in earlier plans: sockets, cursors, "X is viewing this cell" | `live-data.md` |

## Out of scope

- WebSockets.
- Collaborative text editing inside a cell.
- Cell-level cursors.
- Delivering data in the event.
- Guaranteed delivery. A client that misses events reloads.

## Rules for every phase

1. Write the failing test first. Run it. Confirm it fails because the behavior is missing. Then write the code.
2. Do not add a dependency.
3. An event never contains a field value, a name, or a comment. Ids, kinds, versions, and counts only.
4. A stream authorizes at connect time and again for every app it carries. A person who loses access stops receiving events within 60 seconds.
5. A slow reader never slows a writer.

## Phase 1 — events

In `write_record`, inside the transaction, run:

```sql
SELECT pg_notify('scaffoldry_changes', $1)
```

with `$1` a JSON string of `{ "app": "...", "table": "...", "record": "...", "version": 7, "kind": "update" }`. `kind` is `create`, `update`, `delete`, or `restore`. The payload is far below the 8,000-byte limit. `NOTIFY` is delivered only when the transaction commits, so a rolled-back write sends nothing.

A listener thread owns one dedicated connection, runs `LISTEN scaffoldry_changes`, and republishes each message on a `tokio::sync::broadcast` channel per app slug. If the connection drops, it reconnects with backoff and tells every open stream to send `resync`.

Route: `GET /api/v1/apps/{slug}/events`. It requires read access to the app. It responds `text/event-stream` and sends:

| Event | Data | Meaning |
| --- | --- | --- |
| `change` | `{ table, record, version, kind }` | A record changed |
| `resync` | `{}` | Events may have been missed. Reload |
| `ping` | `{}` | Every 15 seconds |

A reader that falls more than 256 events behind is sent `resync`, and its backlog is dropped. Limits: `realtime.max_streams_per_user` (default 20) and `realtime.max_streams` (default 5,000), both added to the closed settings list in `foundation.md` phase 8. Past the limit, the answer is `429` with error code `rate_limited`. Add `rate_limited` to the closed list in `ux-standards.md`.

Re-authorization: every 60 seconds the stream re-runs the read check for its app and closes with a final `event: closed` if it fails.

Tests.

1. Two clients are connected. A write by one produces one `change` for the other, with the right ids and version.
2. A client with no read access to the app is refused with 403. A client that loses access receives `closed` within 60 seconds.
3. An event's JSON has only the five keys above. A test asserts no other key can appear.
4. A rolled-back write sends nothing.
5. A client reading slowly is sent `resync` after 256 events and the writer's latency is unchanged within a tolerance stated in the test.
6. Killing the listener's connection makes every stream receive `resync` after it reconnects.
7. The twenty-first stream for one user is `429`.

## Phase 2 — the grid listens

Web only, from the kit. Add `apps/web/src/realtime.ts`: one function, `subscribe(appSlug, onEvent)`, that uses `fetch` with a streaming body, parses the stream, sends the `Authorization` header, reconnects with backoff, and returns an unsubscribe function. A test double replaces it in component tests.

In the grid and the other record views:

1. On `change` for a record in the loaded window, refetch that record and update it in place. A short highlight marks it. The highlight has a text label for screen readers: `Updated by someone else`.
2. For a record outside the loaded window, count it. Show one `Banner`: `12 records changed. Refresh.` The button reloads from the first page and clears the count.
3. If the person is editing the changed cell, do not replace what they are typing. Mark the cell `Changed by someone else`, keep their draft, and on save the version check decides, with the usual conflict message through `explainError`.
4. On `resync`, reload the current window and say so in a `Toast`.
5. On `kind: delete`, remove the row with a message `Removed by someone else.`
6. Unsubscribe when the view closes.

Tests (vitest, with the subscribe double).

1. A `change` for a visible record triggers one fetch of that record and updates the row.
2. A `change` outside the window raises the count banner. Refresh reloads and clears it.
3. A change to a cell under edit keeps the draft and shows the marker.
4. `resync` reloads and shows the toast.
5. Leaving the view calls unsubscribe.

## Phase 3 — presence and a channel for the person

Presence. An in-memory map per app of who has a stream open, keyed by address, with the time of the last ping. The listener adds and removes on connect and disconnect and drops anyone silent for 45 seconds. A stream also receives `presence` events: `{ joined | left, count }` and, on connect, the current list. The list holds names and addresses and goes only to people with read access to the app, which every stream-holder already has. The header shows up to five avatars with names and `+3`. Avatars have text names.

A second stream, `GET /api/v1/me/events`, carries events for the signed-in person: `notification` (a count and the new id), `job` (id, state, percent), and `proposal` (id, status). It carries no content. The desk uses it for the bell count and the Background work indicator from `jobs.md` phase 4, which stop polling.

Tests.

1. Two users in an app see each other. One closes the stream, and the other sees `left` within a second. A silent client is dropped after 45 seconds.
2. A person without read access never receives the presence list.
3. A job's progress produces `job` events for its owner only.
4. A new notification produces one `notification` event with an id and a count and no text.
5. Web: avatars show names. The bell count updates from an event with no reload.

## How to prompt Gemini

```
Read docs/plans/realtime.md and docs/plans/README.md.
Implement phase N only.
Write the failing test first and run it.
Do not add a dependency.
Never put a field value, name, or comment in an event.
Do not use EventSource. Do not put a token in a URL.
Do not read, copy, or adapt another product's source code.
Stop when the tests listed for phase N pass, and paste the command output.
```

## Enterprise objections

| Objection | Answer |
| --- | --- |
| Live data will leak across access rules. | An event holds only ids. The client refetches through the same authorized route, so every rule, guard, and mask applies. Test 3 in phase 1 asserts the keys. |
| Thousands of open connections will exhaust the server. | Per-user and global limits return a clear error. A reader that lags is resynced and dropped. |
| `NOTIFY` is lost if the listener is down. | Then every stream resyncs on reconnect. Nothing depends on delivery. |
| Someone who loses access still watches changes. | Streams re-check access every 60 seconds and close. Even before that, an event holds nothing readable. |
| Proxies buffer event streams. | The route sends a ping every 15 seconds and the appropriate headers. The Deployment section of the README notes the proxy settings. |
