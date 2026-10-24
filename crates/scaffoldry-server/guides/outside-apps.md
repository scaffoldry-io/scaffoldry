# Building an app outside Scaffoldry

Use this guide when the person wants a web page or small app that runs somewhere else and still uses institutional data. The data stays in Scaffoldry. The app asks for it, as the person using the app.

## What the app can and cannot do

- The app is a **registered client**. A person proposes it with a **ceiling**: the apps, tables and fields it may ever use, and whether it may create, change or delete. An owner or admin of the workspace approves it. A ceiling that writes needs a second person.
- A person who opens the app signs in and agrees. The app then acts as that person. It gets only what the person may do **and** the ceiling allows.
- **Sensitive fields are not available outside Scaffoldry.** A ceiling that names one is refused. Do not ask for a way around this. Sensitive data stays in Scaffoldry's own pages.
- Every call is recorded. A write names the app in its history.

## Steps

1. **Ask the person** what the page shows or changes. List only the fields it needs. A smaller ceiling is approved faster.
2. **Register the client** with `POST /api/v1/clients`. Send `name`, `operator`, `kind`, `redirect_uris` and `ceiling`.
   - `kind` is `public` for a browser app, and `confidential` for a server app. A confidential client gets a secret once. Keep it on the server only.
   - A redirect address is an exact `https` URL. No wildcard and no query string.
   - Say plainly that the client is pending. It works only after a person approves it. Do not claim it is live before then.
3. **Sign in with the authorization code flow and PKCE.**
   - Make a random `code_verifier` of 43 to 128 characters. Send its SHA-256 as `code_challenge`, with `code_challenge_method=S256`. No other method works.
   - Send the person to `/oauth/authorize` with `client_id`, `redirect_uri`, `response_type=code`, `state` and `code_challenge`.
   - Check that `state` comes back unchanged. Then `POST /oauth/token` with the `code` and your `code_verifier`.
   - The access token lasts one hour. There is no refresh token. Sign in again when it ends.
   - The server describes itself at `/.well-known/oauth-authorization-server`.
4. **Fetch the generated module** with `GET /api/v1/apps/{slug}/client.ts` and your access token. Save it as `client.ts` in the app. It has a type for each table and `list`, `all`, `get`, `create`, `update` and `remove` for the operations your ceiling allows. It uses `fetch` and nothing else. Fetch it again when the app changes.
5. **Use it.** Give it a function that returns the token. It calls that function on every request and never stores the token.

```ts
import { createClient, ScaffoldryError } from "./client";

// The address of Scaffoldry comes from the app's settings. The person gives it to you.
const scaffoldry = createClient({ baseUrl: settings.scaffoldryAddress, token: () => sessionStorage.getItem("token") ?? "" });

const page = await scaffoldry.tables["equipment"].list({ limit: 25 });
for (const row of page.records) console.log(row.data["title"]);
```

## Keep calls few and small

- Ask for a page at a time with `limit`, and follow `next_cursor`. `all` does this for you. Keep `limit` small.
- Do not load every row to filter in the browser. Ask for what the page shows.
- Do not poll. Fetch again when the person asks, or when they act.
- Do not copy records into the app's own store, a file or a log. The platform cannot stop an approved app from keeping what it reads. The ceiling is the control, so keep it small.

## Errors

An error has a sentence and a `code`. The module throws `ScaffoldryError` with `status`, `code` and `remedy`.

- `client_scope`: the ceiling does not cover the call. Ask the owners to widen it. Do not try another route.
- `forbidden`: the person may not do this, whatever the ceiling says.
- `unauthorized`: the token ended or was revoked. Sign in again.
- `version_conflict`: someone else saved first. Read the row again, then retry once.
- `field_hidden` and `field_read_only`: the person or the ceiling does not allow that field.

A person can disconnect the app, and an administrator can disable it. Both take effect on the next call.

## Never

- Never put a client secret, an access token or a `code_verifier` in a page that is sent to the browser from a server you do not control, in a file, or in a log.
- Never ask a person to paste a long-lived agent token into the app. Use the sign-in flow.
- Never use a wildcard redirect, or any PKCE method but `S256`.
