# Guests

A guest is a person outside the directory who proves they own an email address and may use one form. Guests are off until a Platform Admin turns them on.

## Turn it on

| Setting | Meaning |
| --- | --- |
| `guests.enabled` | `true` lets people ask for links. The default is `false`. Turning it off ends every guest session at once |
| `forms.disabled` | A list of form ids that are closed to guests, whatever their audience |
| `guests.captcha_hook` | Empty. An institution that adds a check, such as a CAPTCHA, names it here. See `guests::captcha_ok` |
| `mail.enabled`, `mail.smtp`, `platform.base_url` | Mail must be on and complete, or no link can be sent |

A form is open to guests when its view is of kind `form` and its config says `"audience": "Guests"`.

## How a guest signs in

1. The person asks for a link: `POST /guest/{app}/{form}/start` with `email`. The answer is always `If that address can use this form, a link is on its way.` So the form cannot be used to find out who has applied.
2. The mail holds `{platform.base_url}/g/{token}`. The token is 128 random bits. Only its hash is stored. It works once, for 15 minutes.
3. `POST /guest/verify` with the token makes a session of eight hours. The session is an `api_tokens` row of kind `guest`, limited to the app, and it records the form.

A guest's address inside the system is `guest:{id}@guest.invalid`. It is not a person of the directory. It cannot be added to a workspace, and `resolve_user` knows nothing of it.

## What a guest session reaches

The guest routes (below) and sign-out, and nothing else. Every other route, the tool endpoint included, answers `token_scope`. A guest who is disabled, or guests turned off, stop at once, because the session is checked on every request.

## Limits

Each start counts in `rate_limits` for the current clock hour:

| For | Limit |
| --- | --- |
| One address (in lower case) | 3 |
| One network address (the first value of `X-Forwarded-For`) | 20 |
| One form | 200 |

Past a limit the answer is `429` with `rate_limited`, and nothing is sent.

A hidden `website` field in the start form catches bots. A request that fills it gets the same answer as everyone, and nothing happens.

## What a start does not do

Nothing is made, and no mail is sent, when guests are off, the form is missing, closed or not open to guests, mail is off, the address is not an address, or the guest is disabled. The answer is the same for every one of them.

## A form for guests

A form view has an `audience`: `Members` (the default) or `Guests`. Saving a view cannot set it to `Guests`. Opening a form to guests is a proposal that a second person decides.

1. An owner or admin of the app calls `POST /apps/{slug}/views/{id}/guest-exposure`. The server runs the `guest_exposure` check and keeps the answer with the proposal. The change list reads: `This form lets people outside the institution create records in {table}. They can see only their own.`
2. A different owner or admin calls `POST /apps/{slug}/views/{id}/guest-exposure/{xid}/decide` with `decision` (`approve` or `reject`) and a `rationale`. The person who proposed cannot decide. A decision is made once.
3. An approval sets the audience. It is refused when a check failed, and when the form changed after it was proposed.

`GET /apps/{slug}/views/{id}/guest-exposure` lists the proposals for a form. Each proposal and each decision is in the ledger as `GuestExposureChanged`.

The check:

| Outcome | When |
| --- | --- |
| Fails the save | The form lists a link or a lookup, or a field of another table. Validation refuses it before any proposal is made |
| `fail` | A field the form lists is sensitive and is not marked `"own_data": true`, or a sensitive field is listed as read only |
| `warn` | The table has sensitive fields the form does not list. Guests do not see them |
| `pass` | Neither |

Once a form is open, its listed fields and read-only fields are the ones that were approved. A save that changes them is refused. Set the audience back to `Members` first, then propose again.

### The form's settings

These go in the form's `config`.

| Key | Meaning | Default |
| --- | --- | --- |
| `form_fields` | The fields a guest fills. Each may carry `own_data: true` | Required |
| `read_only_fields` | Fields a guest sees but cannot change, such as a status | None |
| `max_per_day` | Submissions one guest may make in a day, 1 to 100 | 3 |
| `max_total` | Submissions the form takes in all | 10,000 |
| `locks_when` | `{ "field": "status", "in": ["decided"] }`. When it holds, the guest can no longer change the record | None |
| `success_message` | Shown after a submit | A plain thanks |

`guests.max_file_mb` (settings, closed list) is the size of the largest file a guest may send. The default is 10.

### What a guest may do

Only these routes, each under `/api/v1/guest/{app}/{form}`. Each checks that the session names this app and this form, before anything else. A session for another form answers `token_scope`.

| Route | What it does |
| --- | --- |
| `POST /records` | Makes a record from the form's fields. Any other field in the request is refused by name |
| `GET /records/{id}` | Reads a record the guest made, with the form's fields and the read-only fields |
| `PUT /records/{id}` | Changes a record the guest made, with the form's fields, until `locks_when` holds (`409`) |
| `POST /files` | Sends a file. A scan is needed whatever `files.require_scan` says, and the file must fit `guests.max_file_mb` |

There is no list, search, aggregate, export, history, comment or link route for a guest. They answer `token_scope`.

### The own-records rule

A record made through a guest form carries `_submitter` (the guest's address) and `_form` (the form). They are set by the server. Nobody else can set them: a record made or saved by anyone else loses them if it names them, and a save by a member keeps the ones already there, so editing a guest's record in the grid does not cut the guest off from it.

A guest reads or changes a record only when `_submitter` is their address. The rule is built into `scope::allows_data`, and `scope::scope_for` shows a guest no rows to any query. It does not ask the table's access rules, so it holds when the table has none.

### Limits

The `429` for the per-day limit names the number. The total limit answers `409` and names the number.

### Other places that take an address

A guest address is refused as the holder of a grant and as a delegate, as it is for a workspace.

## What the guest sees afterward

The web page is `/g/{app}/{form}`. It is deliberately plain. It has no menu, no navigation and no link, so nothing in it leads to the rest of the product. Every control is a native button or field, so the keyboard reaches all of it.

- `/g/{link}` trades the link from the mail for a session, keeps the session for the browser tab, and moves to the page. A link that was used or has expired says so in words.
- `/g/{app}/{form}` lists the guest's own requests, newest first. Each has a plain sentence for where it stands, such as `Where it stands: Received.`, and the form's fields and read-only fields. `Edit` shows while the request is open. `Add another` shows while the guest is within the limits, and a sentence says so when they are not. A page with no session, or an expired one, asks for an email address and sends a link.
- `/g/stop/{link}` stops mail. It does nothing until the guest presses `Stop these mails`, so a mail scanner that opens the link changes nothing.

The server side is `GET /guest/{app}/{form}`. It answers the form's fields, the guest's own records (at most 100) each with `standing` and `editable`, `can_add` and `remaining_today`. It is the one list a guest has, and it holds only what the guest made.

### Mail to guests

A status change tells the guest, once, by mail. The status is the form's `status_field`, else the field `locks_when` reads, else the first read-only field. A save that does not change it sends nothing. A save by a member, an automation or the guest counts.

The mail is a title and two links, never a value: one to `/g/{app}/{form}`, which asks for sign-in again, and one to `/g/stop/{link}`. The stop link is made for each mail, and only its hash is kept.

Stopping sets `notification_prefs` to `off` for the guest's address and the kind `guest_status`. It covers every form of that guest, not one form. Stopping twice is fine. A guest who is disabled, or whose address was never verified, is sent nothing. The 50 mails an hour per address still apply.

A process step tells a guest through the status field it sets, the same way a member's save does. There is no separate path.

## Operating guests

The console's `Guests` panel is for a Platform Admin. Every route is in `ADMIN_ROUTES`, so any other caller answers `403`. Every change asks for a reason and is a ledger entry of the type `GuestChanged`. The ledger names a guest by id and never holds an address.

| Route | What it does |
| --- | --- |
| `GET /admin/guests` | The forms open to guests, the guests (the newest 200), and the signs of abuse |
| `POST /admin/guests/forms/{id}/close` | Adds the form to `forms.disabled`. It takes effect on the next request, on every server |
| `POST /admin/guests/forms/{id}/open` | Takes it out again |
| `POST /admin/guests/{id}/disable` and `.../enable` | A disabled guest's session stops on its next request |
| `POST /admin/guests/{id}/erase` | See below |

The panel shows for each form: the app, the submissions, the last submission, the link requests in the last day, and the rate-limit hits. A hit is a request past the form's limit of 200 an hour. For each guest it shows the address, the forms used, the submissions, when they were last seen, and the state (`Active`, `Disabled` or `Erased`).

### Closing a form

A closed form refuses every new link request and every guest request. A start answers the same neutral sentence as always and sends nothing, so it still does not say whether an address has applied. A guest who is signed in is told `This form is not accepting responses.` The records stay, and members read them as before.

### Erasing a guest

`Erase` is for a person's request to be forgotten. It is separate from retention, which is about records. It:

- replaces the address with `erased:` and a hash of it, so the person can apply again later as a new guest;
- removes mail that has not been sent, the links not yet used, and the person's mail choices;
- revokes the person's sessions, and disables the guest;
- keeps the records, with the same submitter. The panel shows the guest as `Former applicant`;
- writes a ledger entry that names the guest by id.

Erasing cannot be undone, and erasing twice answers `409`.

### Signs of abuse

- Addresses with three or more failed mails (a failed mail is one in the outbox in the state `failed`).
- Networks that went past their limit of 20 link requests an hour in the last day, with how many requests went over.

### Retention

The job `purge_guests` runs daily. It removes a guest who has no records and has not been seen for 90 days. A guest who made a record is kept, because the record names them. A guest never seen is measured from when they were made.

### Indexes

Records made through a guest form are the only ones in two partial indexes (`idx_dataset_records_guest_submitter` and `idx_dataset_records_guest_form`). The limits, the status page and the purge look records up by `_submitter` and `_form` through them, and no other write pays for them.
