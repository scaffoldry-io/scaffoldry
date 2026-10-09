# Guests — implementation brief

Hand this file to Gemini Flash. Do one phase per session. Stop when that phase's tests pass. Do not start the next phase.

Run order is `docs/plans/README.md`. Phase 1 needs `notifications.md` phase 2 and `views.md` phase 6 (the rate limit table). Phase 2 needs `access-rules.md` phase 3, `views.md` phase 7, and `attachments.md` phase 2. Phase 4 needs `lifecycle.md` phase 3.

Applicants are not in the directory. Neither are people who register for an event, submit a conference abstract, request a records transcript, or apply for a small grant. They need to fill in a form, upload a document, and later see where their request stands. They must not see anyone else's. Today every user must be a directory person, so this work would go on a spreadsheet or an outside form tool, outside every control in this plan.

This brief adds a guest: a person who proves they own an email address, can use one form, and can see only what they submitted.

## Decisions already made

| Question | Answer |
| --- | --- |
| How does a guest sign in? | They ask for a link by email, and the link is good once for 15 minutes. No passwords, ever |
| What does a guest session reach? | One app and one form, for eight hours. Nothing else |
| What does a guest see? | Only the records they created, and only the fields the form's author listed |
| Who may open a form to guests? | Anyone who is an owner or admin, but only by a proposal, and a second person must approve it. It is the most exposed surface in the product |
| Can guests be abused? | Limits on every step, an email verification cost on every account, and a kill switch |

## What exists today

Nothing. Every principal is a directory person or a token for one.

## Out of scope

- Guest accounts with passwords or social sign-in.
- Guests who see other guests' records, or any shared view.
- A CAPTCHA provider. A hook is left for the institution to add one.
- Payment.
- Guests in more than one app at once.

## Rules for every phase

1. Write the failing test first. Run it. Confirm it fails because the behavior is missing. Then write the code.
2. Do not add a dependency.
3. A guest's session carries an app and a form. A route that does not name that app and form refuses it, before any other check.
4. A guest can never read a record that another person created, whatever the form says.
5. Use the kit for every screen.

## Phase 1 — a guest and a link

Migration `crates/scaffoldry-core/migrations/0043_guests.sql`:

```sql
CREATE TABLE IF NOT EXISTS guests (
    id UUID PRIMARY KEY,
    email VARCHAR(320) NOT NULL UNIQUE,
    email_verified_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    last_seen_at TIMESTAMPTZ,
    disabled_at TIMESTAMPTZ
);

CREATE TABLE IF NOT EXISTS guest_links (
    token_hash CHAR(64) PRIMARY KEY,
    guest_id UUID NOT NULL REFERENCES guests(id),
    app_slug VARCHAR(64) NOT NULL,
    form_id UUID NOT NULL,
    expires_at TIMESTAMPTZ NOT NULL,
    used_at TIMESTAMPTZ
);
```

A guest's identity inside the system is the address `guest:{id}@guest.invalid`, where `guest.invalid` is a name that cannot resolve. It is never a directory person and `resolve_user` knows nothing about it.

Flow.

1. `POST /guest/{app}/{form}/start`, no sign-in, body `email`. If the form accepts guests and is live, it creates the guest if needed and, only then, emails a one-time link. The response is always the same text, `If that address can use this form, a link is on its way.`, so the form cannot be used to find out who has applied.
2. The link is `{platform.base_url}/g/{token}`. The token is 128 random bits, stored as a hash, valid 15 minutes, used once.
3. `POST /guest/verify` with the token creates a session: an `api_tokens` row of kind `guest`, with `scope` limited to this app and form (`integrations.md` phase 2), eight hours long, and sets `email_verified_at`.

Limits, through the `rate_limits` table from `views.md`: 3 link requests per address per hour, 20 per network address per hour, and 200 per form per hour, with `429` and `rate_limited` beyond. A guest address that bounces repeatedly can be disabled by an administrator. A honeypot field in the start form, hidden from people and from screen readers, rejects submissions that fill it with the same response as success. A setting, `guests.captcha_hook`, is left empty, and the code calls a documented function that returns true, so an institution can add a check without changing the flow.

Settings, in the closed list: `guests.enabled` (boolean, default false) and `forms.disabled` (array of form ids). With `guests.enabled` false, `start` answers the same neutral text and sends nothing.

Tests, with the fake SMTP from `notifications.md`.

1. A start for an address creates a guest and one mail. The neutral response is identical for an address that exists, one that does not, and a disabled one.
2. A link works once. A second use fails. An expired one fails.
3. The session's scope is one app and form, and any other route refuses it with `token_scope`.
4. The fourth request in an hour for an address is `429` and sends nothing.
5. A filled honeypot gets the neutral text and no mail.
6. With `guests.enabled` false nothing is created or sent.
7. The token is stored only as a hash.
8. A guest address cannot be resolved as a directory person, and cannot be added to a workspace.

## Phase 2 — a form for guests

A form view (`views.md` phase 7) gains `audience: Members | Guests`. A guest form is a different, narrower object. Its author lists the fields guests may see and fill, a success message, a maximum number of submissions per guest per day (default 3) and in total (default 10,000), and an optional `locks_when`: a status condition after which the guest can no longer edit.

Ownership. A record created through a guest form stores the guest's address in a system field `_submitter`, of type `Collaborator`, hidden from the member view of the table unless an owner includes it. The form's rules are enforced by the server and cannot be changed by the client:

- A guest may create a record in the form's table, with only the form's fields. Any other field in the request is refused.
- A guest may read and update only records whose `_submitter` is themselves, with the form's fields only, until `locks_when` holds.
- A guest has no list across other people's records, no search, no link picker, no export, no history, no comments, and no aggregate. Those routes refuse a guest session.
- Attachments from a guest require a scan (`attachments.md` phase 2), whatever the setting, with a size limit lower than a member's (default 10 MB, in settings as `guests.max_file_mb`).
- Computed and process fields the author lists are shown read-only, such as a status. Nothing else is.

This is implemented as a built-in row rule, `_submitter == the person`, applied to every guest request through `row_scope`, and a field allowlist applied through `field_visibility`. It does not depend on the author remembering to write it.

Governance. Turning a form's audience to `Guests` is a proposal with the flagged check `guest_exposure`, which needs a second person, and shows in the change list as `This form lets people outside the institution create records in Admissions. They can see only their own.` The check also warns when the table has sensitive fields beyond the listed ones, and fails when a listed field is sensitive and not marked as expected from the person themself. A form cannot list a field of another table or a link to a dataset.

Tests.

1. A guest creates one record. Their session cannot read another record, even by guessing its id, and gets not found.
2. A request with a field outside the form is `400` naming it.
3. After `locks_when` holds, the guest's update is refused with `409` and a message.
4. A guest session on a list, search, aggregate, export, history, comment, or link route is refused.
5. A file from a guest is scanned or refused. A file over the guest limit is refused.
6. The fourth submission in a day is refused with the limit stated.
7. Changing a form to `Guests` raises `guest_exposure`. The proposer cannot approve alone.
8. A form that lists a link field to a dataset fails validation.
9. The row rule holds even when the table has no access rules of its own. Remove every other rule and re-run.

## Phase 3 — what the guest sees afterward

A status page at `/g/{app}/{form}` after sign-in, built from the kit and kept deliberately plain: the guest's records as a list, each opening a view of the listed fields and the status, with `Edit` while open, a plain sentence of where it stands, and `Add another` within the limit. No navigation into the rest of the product exists in a guest session, and a guest session shows no menus that lead elsewhere.

Notices to guests. A process step or a status change can notify the submitter: `notify(Guest(address), kind, args)` queues mail through the outbox with the same content rules: a title and link, never a value. The link is `/g/...` and leads to the sign-in step again. A guest may stop mail for a form with a link in each message, which sets `notification_prefs` for the guest address.

Tests.

1. A guest sees only their records and only listed fields on the status page.
2. A status change sends one mail with a link and no record values. Following the link asks for sign-in.
3. A guest who stopped mail gets none.
4. A guest session has no route to the member desk.
5. Web: the page is operable by keyboard and has no link outside the guest area.

## Phase 4 — operating guests

Console panel `Guests`, all in `ADMIN_ROUTES`, built from the kit:

- `Forms open to guests`: app, form, submissions, last submission, link requests in the last day, rate-limit hits, and a `Close this form` action that adds it to `forms.disabled` with a reason.
- A guest list: address, forms used, submissions, last seen, state, with `Disable`, and `Erase`, which replaces the address with a hash, removes any pending mail, and keeps the records, which then show `Former applicant`, so that data retention and a person's request to be forgotten can be handled separately. Erase is a ledger entry.
- A summary of abuse signals: addresses with repeated bounces, networks that hit limits.

Retention (`lifecycle.md`) applies to guest records and guests. Guests with no records and no activity for 90 days are removed by a scheduled job.

The kill switch is a setting, and closing a form takes effect on the next request. A closed form answers `This form is not accepting responses.`

Tests.

1. Closing a form refuses new starts and submissions at once and keeps existing records.
2. `Erase` removes the address and pending mail, keeps the records, and writes a ledger entry.
3. A guest with no records and 91 days of silence is removed. One with records is not.
4. A faculty caller is 403 on every route here.
5. The summary lists an address after three bounces.

## How to prompt Gemini

```
Read docs/plans/guests.md and docs/plans/README.md.
Implement phase N only.
Write the failing test first and run it.
Do not add a dependency.
Never reveal whether an address has an account or has applied.
Never let a guest read a record another person created.
Never give a guest session any route outside its one form.
Do not read, copy, or adapt another product's source code.
Stop when the tests listed for phase N pass, and paste the command output.
```

## Enterprise objections

| Objection | Answer |
| --- | --- |
| Opening forms to the public will be abused. | Every step is rate limited, an email must be verified, a honeypot and a hook for a challenge exist, each form has a daily and total cap, and an administrator can close a form at once. |
| A guest will read other applicants' data. | A built-in rule limits every guest request to records they created, and a test removes every other rule to prove it holds. |
| The form will reveal whether someone applied. | `start` gives the same answer for any address. |
| An author will expose sensitive fields to guests by mistake. | Opening a form to guests is a proposal that needs a second person, lists the exposure in words, and fails when a sensitive field is listed without being the person's own data. |
| Guests will leave data we must delete. | Retention applies, and `Erase` removes the person's address while keeping the records and the audit trail. |
| Email links will be guessed. | Tokens are 128 bits, hashed, single use, and expire in 15 minutes. |
