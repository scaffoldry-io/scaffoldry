# Sending mail

The server can mail a notice to the person it is for. Mail is off until a Platform Admin turns it on. A mail holds the notice's title, its short body, and a link. It never holds a record's values, a token, or an attachment.

## What you need

- A mail relay that takes TLS, on port 465 (TLS from the start) or 587 (STARTTLS). The server never sends mail in the clear, and it refuses a relay on 587 that does not offer STARTTLS.
- The relay's certificate must chain to a root in the server's system certificate store. On a container image that is usually `/etc/ssl/certs`. A relay with a certificate from the institution's own authority needs that authority added to the store.
- The master key for the secrets store, if the relay wants a password. See [master-key.md](master-key.md).

## Turn it on

Do these in the console, or with `PUT /api/v1/settings/{key}`, as a Platform Admin.

1. Store the relay's password as a secret, for example `smtp_password`, under **Secrets**.
2. Add the relay to `egress.allow`, for example `{ "host": "smtp.state.edu", "ports": [587] }`. The server connects to nothing that is not on this list. A name that resolves to a private address needs a `cidr` entry for that network as well.
3. Set `mail.smtp`:

   ```json
   { "host": "smtp.state.edu", "port": 587, "security": "starttls", "from": "Scaffoldry <desk@state.edu>", "username": "scaffoldry", "secret": "smtp_password" }
   ```

   `username` and `secret` go together. Leave both out for a relay that takes mail without a sign-in.
4. Set `platform.base_url` to the address people use for the desk, for example `https://desk.state.edu`. Links in mail start with it.
5. Set `mail.enabled` to `true`.
6. Send a test with `POST /api/v1/admin/mail/test` and a `reason`. It goes to you only and is written to the ledger.

## Watch it

`GET /api/v1/admin/mail` shows whether mail is on, what is missing if it cannot go, how many mails wait and since when, and the last failures. A failure shows the recipient's domain and what went wrong, never the address.

## How it behaves

| Situation | What happens |
| --- | --- |
| `mail.enabled` is false | Nothing is queued and nothing is sent. Notices still reach the inbox. |
| The relay refuses or cannot be reached | The mail is tried again after 1 minute, 10 minutes, 1 hour and 6 hours. Then it is marked failed. |
| A person has had 50 mails in the last hour | Further notices stay in the inbox and go into that person's next morning digest. |
| The server has sent 600 mails in the last minute | The rest wait for the next run, 15 seconds later. |
| Someone follows a link | The desk asks them to sign in if they have no session, then opens what the notice is about. Only the person the notice was for can follow it. |

The `send_email` job does the sending every 15 seconds. Each run claims up to 20 mails, so several server instances can run it at once without sending a mail twice. A server that stops in the middle of a run leaves its mails to be tried again five minutes later. In that rare case one mail can arrive twice.

The password is read from the secrets store at the start of each run. It is never written to a log, an error, the ledger, or the queue. An error from the relay is recorded as what failed and the reply code only, because a relay can repeat back what it was sent.

## Preferences and the digest

Each person chooses, per kind of notice, where it goes: the inbox, mail only, both, a morning digest, or off. Someone who has not chosen gets the institution's default from the setting `notifications.defaults`, an object from kind to channel, and with no default, both. Three kinds can never be turned off: a decision waiting on the person, a change to their own access, and an infected file.

The `send_digests` job runs at 07:00 in `platform.timezone`. It sends each person one mail listing their unread digest notices, grouped by app, with a link for each. Changing `platform.timezone` moves the digest to 07:00 in the new zone.
