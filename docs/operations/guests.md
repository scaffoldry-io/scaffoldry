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

The guest routes and sign-out, and nothing else. Every other route, the tool endpoint included, answers `token_scope`. A guest who is disabled, or guests turned off, stop at once, because the session is checked on every request.

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
