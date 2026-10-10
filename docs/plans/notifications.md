# Notifications — implementation brief

Hand this file to Gemini Flash. Do one phase per session. Stop when that phase's tests pass. Do not start the next phase.

Run order is `docs/plans/README.md`. Phase 1 needs `jobs.md` phase 2 and `approvers.md` phase 3. Phase 2 needs `integrations.md` phase 1. Phase 4 needs the producers' briefs.

A chair is asked to approve an admission. A reviewer is mentioned in a comment. An import finishes. A sync fails overnight. Every one of these needs a person to be told, or it does not happen. Today a waiting approval is a row in a queue that someone must remember to open, and `Notify` in a process is a log line.

This brief adds an inbox, email, preferences, and digests, with one rule above the rest: a notice carries names and links, never record values.

## Decisions already made

| Question | Answer |
| --- | --- |
| Who is told? | The result of a recipient specification: a person, the approvers of a step, a position, or a role. Never the person who caused the event |
| What does a notice contain? | A title and a short body built from a template with a closed set of placeholders: people's names, app and table names, a process prompt, a count. A field value cannot be a placeholder |
| How does email leave? | Through the outbox and the job queue, over SMTP, to the host the institution allowlisted, with the password in the secrets store |
| Can a person turn everything off? | Not decisions that wait on them, or changes to their own access. Those always reach the inbox |
| Do notices carry sign-in links? | They carry plain links. Following one requires signing in. No token is ever in a link |

## What exists today

| Fact | Where |
| --- | --- |
| `Notify` effects append a line to the instance log | `workflow.rs`, `automation.rs` |
| No notifications table, no outbox, no mail client | |
| A user settings pane exists | `foundation.md` phase 9 |
| The desk has no bell and no inbox | |

## Out of scope

- Push to phones and chat tools.
- Reading mail or replying by mail.
- Bounce processing.
- Marketing or bulk mail.
- Rich HTML mail. Mail is plain text.

## Rules for every phase

1. Write the failing test first. Run it. Confirm it fails because the behavior is missing. Then write the code.
2. Add `lettre` in phase 2 only, with the license audit.
3. Mail and notices are built only by `render(kind, args)`. A test proves no record value can enter.
4. Use the kit for every screen.

## Phase 1 — the inbox

Migration `crates/scaffoldry-core/migrations/0041_notifications.sql`:

```sql
CREATE TABLE IF NOT EXISTS notifications (
    id UUID PRIMARY KEY,
    recipient VARCHAR(255) NOT NULL,
    kind VARCHAR(32) NOT NULL,
    app_slug VARCHAR(64),
    title VARCHAR(200) NOT NULL,
    body VARCHAR(500) NOT NULL,
    link JSONB,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    read_at TIMESTAMPTZ,
    email_state VARCHAR(10) NOT NULL DEFAULT 'none'
);
CREATE INDEX IF NOT EXISTS idx_notifications_recipient ON notifications (recipient, created_at DESC);
CREATE INDEX IF NOT EXISTS idx_notifications_unread ON notifications (recipient) WHERE read_at IS NULL;
```

Kinds, a closed list in code: `mention`, `decision_waiting`, `decision_made`, `process_escalated`, `process_overdue`, `proposal_pending`, `proposal_decided`, `job_finished`, `import_finished`, `sync_failed`, `webhook_disabled`, `file_infected`, `assigned`, `delegation_received`, `access_changed`, `view_shared`.

Recipient specifications: `Person(address)`, `Approvers(step, instance)` (resolved with `resolve_approvers`, so a delegate is told as well), `Position(key, unit)`, `Workspace(workspace, roles)`. `notify(spec, kind, args)` resolves, removes the actor, removes people who are inactive or on hold, de-duplicates, and inserts one row each. A notice for a person who may not read the target app is not created, so a notice cannot reveal an app.

Rendering. `render(kind, args) -> (title, body, link)`. `args` is a typed struct per kind with fields that are names, counts, and ids. There is no `String` field a caller can fill with record content. Titles are at most 200 characters and bodies 500, cut at a word. Example: title `Decision needed: Admissions`, body `Review "Approve this admission" for Applicants. 3 are waiting.`

Routes, signed in.

| Method | Path | Behavior |
| --- | --- | --- |
| GET | `/api/v1/me/notifications` | The caller's, newest first, paged. Query `unread=true` |
| GET | `/api/v1/me/notifications/count` | Unread count |
| POST | `/api/v1/me/notifications/read` | Body ids, or `all: true` |

Retention: a scheduled job `purge_notifications` deletes read ones older than 90 days and any older than 365.

Screen, from the kit. A bell in the top bar with the unread count in its accessible name, a panel with the list, unread distinguished by a marker and text (`Unread`), each opening its link, `Mark all read`, and an `EmptyState`. `me/events` from `realtime.md` phase 3 updates the count without reload when it exists, and the bell polls every 60 seconds until then.

Tests.

1. `notify` for `Approvers` includes a delegate and excludes the submitter and the actor.
2. A person who cannot read the app gets no notice about it.
3. Rendering with an args struct that contains a record value is a compile error. A test lists every kind's args fields and asserts none is free text.
4. A body over 500 characters is cut at a word boundary.
5. `read` marks only the caller's own. Another person's id is ignored.
6. The purge job removes a 91-day-old read notice and keeps an unread one.
7. Web: the bell's accessible name includes the count. Opening a notice follows its link and marks it read.

## Phase 2 — email

Add `lettre` (MIT) with `rustls` and the system certificate store.

Settings, in the closed list: `mail.enabled` (boolean, default false), `mail.smtp` (`{ host, port, security: "starttls" | "tls", from, username, secret }` where `secret` names a secret), `platform.base_url` (the address used in links). Nothing sends mail while `mail.enabled` is false.

Migration `0048_email_outbox.sql`:

```sql
CREATE TABLE IF NOT EXISTS email_outbox (
    id UUID PRIMARY KEY,
    to_address VARCHAR(320) NOT NULL,
    subject VARCHAR(200) NOT NULL,
    body TEXT NOT NULL,
    state VARCHAR(10) NOT NULL DEFAULT 'queued',
    attempts INTEGER NOT NULL DEFAULT 0,
    next_attempt_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    last_error TEXT,
    notification_id UUID,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    sent_at TIMESTAMPTZ
);
CREATE INDEX IF NOT EXISTS idx_email_due ON email_outbox (next_attempt_at) WHERE state = 'queued';
```

A notice that should be mailed inserts an outbox row in the same transaction. A scheduled job, `send_email`, every 15 seconds, claims up to 20 due rows with `FOR UPDATE SKIP LOCKED`, resolves the SMTP host through `net::egress` and connects to the pinned address with the TLS name set from the host, sends, and records the result. A failure retries after 1 minute, 10 minutes, 1 hour, 6 hours, and then the row is `failed`. The password is read with `secrets::get` at send time and never logged.

Mail content is plain text: the title, the body, and `Open it: {platform.base_url}/n/{notification id}`. The route `/n/{id}` requires sign-in and then redirects to the notice's target. Nothing else appears: no field value, no token, no attachment.

Limits: at most 50 mails per recipient per hour, and 600 per minute for the server. Past the per-recipient limit, further notices stay in the inbox and are included in the next digest instead.

Admin, all in `ADMIN_ROUTES`: `GET /admin/mail` (state, queue depth, oldest queued, failures with the recipient's domain and the error), `POST /admin/mail/test` (sends one to the caller, body `reason`).

Tests, with a fake SMTP server in the process reached through an injected resolver and an allowlist entry.

1. With `mail.enabled` false nothing is sent and the inbox still works.
2. A notice that should be mailed produces one message to the right address with the right subject and a link under `platform.base_url`. The text contains no string that was a record value in the fixture.
3. A server error retries on the stated schedule and ends `failed` after the last. A success ends `sent`.
4. The SMTP host must be on `egress.allow`. A private address is refused.
5. The password is never in a log, response, or error message.
6. The 51st mail to one person in an hour stays in the inbox and is queued for the digest.
7. `/n/{id}` without a session redirects to sign-in and then to the target.
8. The test mail goes only to the caller and is a ledger entry.

## Phase 3 — preferences and digests

Migration `0049_notification_prefs.sql`:

```sql
CREATE TABLE IF NOT EXISTS notification_prefs (
    eppn VARCHAR(255) NOT NULL,
    kind VARCHAR(32) NOT NULL,
    channel VARCHAR(12) NOT NULL,
    PRIMARY KEY (eppn, kind)
);
```

`channel` is `in_app`, `email`, `both`, `digest`, or `off`. Institution defaults are in the setting `notifications.defaults` (an object from kind to channel). The kinds `decision_waiting`, `access_changed`, and `file_infected` can never be set below `in_app`.

A scheduled job `send_digests` runs at 07:00 in `platform.timezone`, collects each person's unread `digest` notices and any held back by the hourly limit, and sends one mail with a line per notice, grouped by app.

Screen. In the user settings pane, a `Notifications` section: a table of kinds in plain words with a channel choice per row, the forced kinds shown as fixed with the reason, and a line saying whether the institution sends mail at all. In the console, `Notifications` under Settings edits `notifications.defaults` as a table with a reason.

Tests.

1. A person who chose `off` for `mention` gets no notice. For `decision_waiting`, `off` is refused with a message.
2. `digest` produces one mail at 07:00 in the institution's zone with every pending line and no record values.
3. The default applies to a person with no row.
4. Web: the forced rows are not changeable and say why.

## Phase 4 — producers

Wire each producer to `notify`, with one test each that checks recipient, kind, and that nothing but names and counts appears.

| Event | Kind | Who | From |
| --- | --- | --- | --- |
| A comment mentions someone | `mention` | the mentioned person | `record-history.md` |
| A process step starts waiting | `decision_waiting` | approvers, including delegates | `approvers.md` |
| A step is decided | `decision_made` | the record's creator and the starter | `approvers.md` |
| A step is escalated or overdue | `process_escalated`, `process_overdue` | the new approvers, the old ones, and the record's owner | `process-v2.md` |
| A proposal needs deciding | `proposal_pending` | workspace owners and admins other than the proposer | `mcp-apps.md` |
| A proposal is decided | `proposal_decided` | the proposer | `mcp-apps.md` |
| A job finishes or fails | `job_finished` | its owner | `jobs.md` |
| An import finishes | `import_finished` | its owner | `import-export.md` |
| A sync fails | `sync_failed` | the connection's owner and Platform Admins | `connections.md` |
| A webhook is disabled | `webhook_disabled` | the owner and workspace admins | `integrations.md` |
| A file is infected | `file_infected` | the uploader and workspace admins | `attachments.md` |
| A collaborator field adds someone | `assigned` | that person | `field-types.md` |
| A delegation is made | `delegation_received` | the delegate | `approvers.md` |
| Access to a workspace is granted or removed | `access_changed` | the person | `access-rules.md` |
| A view is shared inside the institution | `view_shared` | none by default | `views.md` |

The process `Notify` effect now creates real notices through the recipient specification in its step.

Tests.

1. Each row above has a test that triggers the event and checks the recipients and the absence of record values.
2. A `Notify` effect for a position reaches its holders and delegates.
3. No producer calls the mail client directly. A grep test shows only `notify` writes to `email_outbox`.

## How to prompt Gemini

```
Read docs/plans/notifications.md and docs/plans/README.md.
Implement phase N only.
Write the failing test first and run it.
Add only the crate this brief names, in the phase it names, and run the license audit.
Never put a record value in a notice or an email.
Never put a token in a link.
Never connect to a mail host outside net::egress.
Do not read, copy, or adapt another product's source code.
Stop when the tests listed for phase N pass, and paste the command output.
```

## Enterprise objections

| Objection | Answer |
| --- | --- |
| Email will carry student data to inboxes outside our control. | Mail holds a title, a short body of names and counts, and a link. A template with no free-text placeholder builds it, and a test fails if a record value can enter. |
| Notices will tell people about apps they should not know exist. | A notice is not created for someone who cannot read the app. |
| People will miss decisions. | A waiting decision always reaches the inbox and cannot be turned off, and the console shows what is stuck. |
| Mail will flood people. | There is an hourly limit per person, and the surplus goes into one digest. |
| A mail server credential is a risk. | It lives in the encrypted secrets store, the host must be allowlisted, and it is never logged. |
| Following a link will skip sign-in. | A link requires signing in and carries no token. |
