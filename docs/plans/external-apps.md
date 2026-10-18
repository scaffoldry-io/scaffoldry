# Outside apps — implementation brief

> **Status: Open**

Hand this file to the build agent. Do one phase per session. Stop when that phase's tests pass. Do not start the next phase.

Run order is `docs/plans/README.md`. Phase 1 needs `mcp-apps.md` phase 4. Phase 3 needs `integrations.md` phase 2. Phase 4 needs `integrations.md` phase 3. Phase 5 needs `access-model.md` phase 4. Phase 7 needs `foundation.md` phase 10.

Faculty want quick, custom web apps with interfaces of their own. Many will build them with a coding agent and host them wherever they like. Those apps should still get institutional data the governed way: as the person using them, within limits a person approved, with every access recorded.

Today an outside app has two bad choices. It can copy data into its own database, where no rule, label or audit reaches it. Or it can hold a person's long-lived agent token, which carries everything that person may do and names no app.

This brief makes an outside app a **registered client**. A person approves the client and its **ceiling**: the apps, tables, fields and operations it may ever use. A user signs in to the client and consents. The client then calls Scaffoldry **as that user**. What it gets is the overlap of the user's own rights and the client's ceiling, and every call is recorded against both.

Scaffoldry stays the place the data lives. The app can live anywhere. Nothing here depends on any hosting product.

## Standard

- OAuth 2.1 (draft-ietf-oauth-v2-1): the authorization code flow is the only grant for people. No implicit grant, no password grant.
- RFC 7636 (PKCE), method `S256` only.
- RFC 9700, OAuth 2.0 Security Best Current Practice: exact redirect URI matching, short-lived access tokens, sender-bound or rotated refresh tokens when they exist.
- RFC 7009 (token revocation), RFC 8414 (authorization server metadata), RFC 9728 (protected resource metadata), RFC 8707 (resource indicators).
- OpenID Connect Core 1.0 and RFC 8693 (token exchange) for phase 7, where the institution's identity provider issues the token.
- OpenAPI 3.1 for the description a client is generated from.
- The MCP authorization specification, which uses the same OAuth 2.1 profile. An MCP client can register the same way.

Record the search in the first commit message: OAuth 2.1, RFC 7636, RFC 9700, RFC 7009, RFC 8414, RFC 9728, RFC 8707, OpenID Connect Core, RFC 8693.

## Decisions already made

| Question | Answer |
| --- | --- |
| Does Scaffoldry sign tokens? | No, as the cross-brief rules say. An access token is a random string stored as a hash, like every other token. Phase 7 accepts tokens the institution's identity provider signed. Scaffoldry still signs none |
| Who registers a client? | Anyone who may propose changes to the workspace whose apps it names. Registration is a proposal. An owner or admin of that workspace approves it. A ceiling that writes, or that reaches more than one workspace, needs a second person |
| What is a ceiling? | The same shape as a custom page's grants in `mcp-apps.md` phase 6: per table, `read` fields, `write` fields, `create`, `delete`. A client never gets more than its ceiling, whoever signs in |
| May a client read a sensitive field? | No. A ceiling that names an effective-sensitive field is refused. Sensitive data stays inside Scaffoldry's own sandboxed pages, which cannot send anything out. Johann may change this |
| How long does an access token last? | One hour. No refresh token in this brief. The user signs in again |
| Which redirect URIs are allowed? | Exact `https` URIs registered with the client. No wildcard, no query string. `http://localhost` only when the setting `oauth.allow_localhost` is true |
| Public or confidential clients? | Both. A browser app is public and uses PKCE alone. A server app is confidential and also authenticates with a secret, shown once and stored as a hash |
| Can a client act with no user present? | Only in phase 5, as its own service identity, through a grant a second person approved. Never as a person who is not there |
| What does the user see when connecting? | The client's name, its operator, the callback origin, and its ceiling as sentences: "Read the title and status of Proposals in Grants Office. Create Proposals." |
| Can the user or an admin cut a client off? | Yes. A user disconnects one client. An admin disables a client for everyone. Both take effect on the next call, and a queued job checks again before it runs |

## What exists today

| Fact | Where |
| --- | --- |
| An agent token is a random string stored as a SHA-256 hash, with an owner, an expiry and a revocation time | `api_tokens`, `service/identity.rs` |
| The REST API and the MCP endpoint share one service layer | `routes/`, `service/tools.rs` |
| Allowed browser origins are the setting `cors.allowed_origins` | `foundation.md` phase 8 |
| A custom page's grants name per-table `read`, `write`, `create` and `delete`, and `page_call` applies them | `mcp-apps.md` phase 6, planned |
| Narrowed tokens are planned: an effective right is the intersection of the token's scope and the person's rights | `integrations.md` phase 2 |
| An API description per app is planned | `integrations.md` phase 3 |
| Verifying identity-provider tokens is planned | `foundation.md` phase 10 |
| The desk signs in with a pasted token until the browser sign-in decision is made | `README.md` "Open decisions" |

## Out of scope

- Hosting outside apps. They run wherever their builder puts them.
- Refresh tokens. Add them with rotation when people find hourly sign-in too frequent.
- Dynamic client registration (RFC 7591). Every client is registered by proposal.
- Publishing a client library to a package registry. Phase 4 serves a generated file.
- Copying data into an outside app's own store. The platform cannot stop an approved client from keeping what it reads. The ceiling, the ban on sensitive fields, and the record of every call are the controls.

## Rules for every phase

1. Write the failing test first. Run it. Confirm it fails because the behavior is missing. Then write the code.
2. Do not add a dependency.
3. A client call is checked in this order, and stops at the first refusal: the token, the client's status, the user's status, the ceiling, then the user's own rights through the same service function every other caller uses. A ceiling never widens a right.
4. Every registration, approval, disablement and disconnection is a ledger entry written before the change.
5. Every write made through a client records the client id in its history origin. Every disclosure and receipt records it too.
6. Add every new error code to the closed list in `ux-standards.md` and to `explainError`.
7. Use the next free migration number on `main`.

## Phase 1 — registered clients and their ceilings

Migration `*_app_clients.sql`:

```sql
CREATE TABLE IF NOT EXISTS app_clients (
    client_id VARCHAR(64) PRIMARY KEY,
    name VARCHAR(255) NOT NULL,
    operator VARCHAR(255) NOT NULL,
    kind VARCHAR(16) NOT NULL,
    secret_hash CHAR(64),
    redirect_uris TEXT[] NOT NULL,
    ceiling JSONB NOT NULL,
    status VARCHAR(10) NOT NULL DEFAULT 'Pending',
    proposed_by VARCHAR(255) NOT NULL,
    proposed_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    decided_by VARCHAR(255),
    decided_at TIMESTAMPTZ,
    disabled_by VARCHAR(255),
    disabled_at TIMESTAMPTZ
);
```

`kind` is `public` or `confidential`. `status` is `Pending`, `Active`, `Rejected` or `Disabled`. `ceiling` is `{ "apps": [ { "app_slug", "tables": [ { "table_id", "read": [...], "write": [...], "create": bool, "delete": bool } ] } ] }`. Add `governance/schema/app-client.schema.json`.

Routes:

| Method | Path | Who | Behavior |
| --- | --- | --- | --- |
| POST | `/clients` | anyone who may propose to every workspace the ceiling names | Validates the schema, the redirect URIs and the ceiling. Refuses an effective-sensitive field with `400 client_sensitive_field`. Stores `Pending` |
| GET | `/clients`, `/clients/{id}` | the proposer, owners and admins of a named workspace, a Platform Admin | The client, its ceiling as sentences, and its status |
| POST | `/clients/{id}/decide` | an owner or admin of every named workspace, not the proposer when a second person is needed | `approve` or `reject`. On approve of a confidential client, the secret is generated and returned once |
| POST | `/clients/{id}/disable` | a Platform Admin, or an owner of a named workspace | Ledger first. Every token of the client stops working on its next use |

1. A ceiling that only reads, inside one workspace, may be approved by an owner or admin who is not the proposer, or by the proposer when they own the workspace. A ceiling that writes, or names apps in more than one workspace, needs a person other than the proposer.
2. A ceiling's fields must exist in the live manifest. When an app changes and a ceiling names a field that is gone or newly sensitive, the client keeps working for everything else and that field is dropped from its responses. The client's page in the console shows the drop.
3. The ceiling is rendered as sentences by one function. The consent screen in phase 2 and the console use it.

Tests.

1. A ceiling naming a sensitive field is refused.
2. A read-only ceiling in one workspace is approved by its owner. A writing ceiling proposed by the owner is not in force until a second person approves.
3. A redirect URI with a wildcard, a query string, or `http` other than allowed localhost is refused.
4. The confidential client's secret appears in one response and nowhere else. The table holds its hash.
5. Every decision and disablement writes one ledger entry, before the change.

## Phase 2 — signing in, consent, and tokens

Routes, none of which needs an existing token except where stated:

| Method | Path | Behavior |
| --- | --- | --- |
| GET | `/.well-known/oauth-authorization-server` | RFC 8414 metadata: the endpoints below, `response_types_supported: ["code"]`, `code_challenge_methods_supported: ["S256"]`, `grant_types_supported: ["authorization_code"]` |
| GET | `/.well-known/oauth-protected-resource` | RFC 9728 metadata naming this server as the resource and its authorization server |
| GET | `/oauth/authorize` | Requires `client_id`, `redirect_uri`, `response_type=code`, `state`, `code_challenge`, `code_challenge_method=S256`, and `resource` equal to this server. Needs a signed-in desk session. Shows the consent screen. A bad client or redirect URI shows an error page and never redirects |
| POST | `/oauth/authorize` | The user's consent. Stores a single-use code, valid for 60 seconds, bound to the client, the redirect URI, the user and the challenge. Redirects with `code` and `state` |
| POST | `/oauth/token` | Exchanges a code and its `code_verifier`. A confidential client authenticates with HTTP Basic. Returns an opaque access token of kind `client`, valid for one hour, stored as a hash with the client id and the user |
| POST | `/oauth/revoke` | RFC 7009. The client revokes one of its tokens |
| GET, DELETE | `/me/connections`, `/me/connections/{client_id}` | The user lists and disconnects their clients |

1. A code is used once. A second exchange of the same code revokes every token issued from it.
2. Consent is remembered per user and client until the ceiling changes or the user disconnects. A changed ceiling asks again.
3. The consent screen is built from the kit. It names the client, its operator, the callback origin, and the ceiling's sentences. It has `Allow` and `Deny`. It does not show a client's own description as instructions.
4. Allowed browser origins for client calls are the origins of the client's registered redirect URIs, in addition to `cors.allowed_origins`.

Tests.

1. A full flow with PKCE issues a token. The same code twice fails and revokes the first token.
2. A wrong `code_verifier`, a mismatched redirect URI, a missing `state`, or `plain` as the method is refused.
3. A token is refused after one hour, after the user disconnects, after the client is disabled, and after the user is put on hold.
4. A changed ceiling asks for consent again.
5. The metadata documents validate against the fields their RFCs require.

## Phase 3 — calls as the person, within the ceiling

1. A `client` token resolves to the user, with `via_client` set on the caller. Every route and tool checks the ceiling before its own access check. A route the ceiling cannot express, such as admin routes, proposals, and settings, refuses a client token with `403 client_scope`.
2. Reads drop every field outside the ceiling's `read`. Writes refuse a field outside `write`. Create and delete need their flags.
3. The intersection with the user's rights uses the same functions as every other caller. With `integrations.md` phase 2 in place, a client token is a narrowed token whose scope is the ceiling. Do not build a second mechanism beside it.
4. History origins, disclosures and, when `access-model.md` phase 8 has run, receipts, carry `client_id`.
5. MCP: an MCP client registered as a client may call tools within its ceiling. Tool lists are filtered to what the ceiling allows.

Tests.

1. A viewer's client token cannot write, even when the ceiling allows writes.
2. An editor's client token cannot write a field outside the ceiling, and cannot read a field outside it.
3. A client token is refused on `/admin` routes and on proposal routes with `client_scope`.
4. A record written through a client has the client id in its history.
5. The tool list for a client token contains only tools the ceiling allows.

## Phase 4 — a generated client and a guide for builders

1. `GET /api/v1/apps/{slug}/client.ts` returns one TypeScript module, generated on the server from the app's API description and filtered to the caller's client ceiling when called with a client token. It has a type per table, typed `list`, `get`, `create`, `update` and `remove`, cursor paging, and the error codes. It uses `fetch` and nothing else. It has no dependency.
2. The module takes the access token from a function the app provides. It never stores a token.
3. `scaffoldry://guide/outside-apps`, at most 8 KB: how to register a client, the PKCE flow, using the generated module, keeping calls few and pages small, and that sensitive fields are not available outside Scaffoldry.
4. The agent kit from `agent-coverage.md` phase 4 includes this guide.

Tests.

1. The generated module compiles with `tsc --strict` in the test, and its types match the manifest.
2. Called with a client token, the module has no method or field outside the ceiling.
3. A scenario in the agent suite: "Build me a page outside Scaffoldry that lists my lab's equipment." The agent registers a client, and the scenario approves it, signs in, and lists rows through the generated module against the real server.

## Phase 5 — a client's own service identity

Some apps need to work when nobody is signed in: a nightly summary, or a webhook receiver.

1. A confidential client may request a service identity, `client:{client_id}`. Its rights are grants in `access_grants` from `access-model.md` phase 4, with `holder_kind = person` and `holder_id = client:{client_id}`, `source = client`, always within the ceiling. Creating one needs a second person.
2. `POST /oauth/token` with `grant_type=client_credentials` and the client's secret issues a one-hour token for that identity. Add the grant type to the metadata only for clients that have one.
3. Writes by a service identity carry the client as the actor and `ActorKind::Automation`.

Tests.

1. A service identity reads only what its grants and the ceiling both allow.
2. A public client cannot use `client_credentials`.
3. Disabling the client stops its service identity on the next call.

## Phase 6 — the clients panel

Add **Outside apps** to the console, listed under "Panels added by other briefs" in `admin-console.md`.

- Every client: name, operator, kind, status, workspaces, ceiling as sentences, connected users count, last call.
- Usage per client: calls, records read and written per day for 30 days, from one SQL statement.
- `Disable` with a reason. Pending clients with `Approve` and `Reject` for people who may decide.
- Each user's settings pane lists their connections with `Disconnect`.

Tests.

1. Disabling from the panel stops the client's next call.
2. Usage counts match the calls made in the test.
3. A user sees and disconnects only their own connections.

## Phase 7 — tokens from the institution's identity provider

When the institution's identity provider can issue access tokens for Scaffoldry, a client may sign users in there instead.

1. Accept a JWT access token verified with `foundation.md` phase 10, whose audience is this server and whose `azp` or `client_id` claim names a registered, active client. The user is found by the configured subject claim. The token is treated exactly as a phase 2 token, ceiling included.
2. A client may use either path. Both lead to the same checks.

Tests.

1. A verified token for a registered client lists rows within the ceiling.
2. A verified token naming an unregistered client is refused.
3. A token with the wrong audience is refused.

## How to prompt the build agent

```
Read docs/plans/external-apps.md, docs/plans/integrations.md, docs/plans/ux-standards.md, and docs/plans/README.md.
Implement phase N only.
Write the failing test first and run it.
Do not add a dependency.
Scaffoldry signs no token. Access tokens are random strings stored as hashes.
PKCE with S256 only. Exact redirect URIs. One-hour tokens. No refresh tokens.
A ceiling never widens a right, and never names a sensitive field.
Stop when the tests listed for phase N pass, and paste the command output.
```

## Enterprise objections

| Objection | Answer |
| --- | --- |
| Outside apps will leak student data. | A client cannot be approved to read a sensitive field. Everything else it reads is limited by the user's own rights and the ceiling a person approved, and every call is recorded with the client's id. |
| An app will act as a user who is not there. | A person must sign in and consent. Tokens last an hour. Work without a user runs as the client's own identity, with grants a second person approved. |
| A stolen token opens everything. | A client token carries only the overlap of one person's rights and one ceiling, for one hour, and is revoked when the person disconnects or an admin disables the client. |
| This is a new sign-in system to maintain. | It is the published OAuth 2.1 profile that MCP clients already speak. Tokens are the same hashed random strings the platform already stores. Phase 7 lets the institution's own identity provider take over. |
| Builders will copy data out anyway. | The platform cannot prevent an approved client from keeping what it may read. The ceiling keeps that set small, sensitive fields are excluded, and the record of calls shows what each client read. |
| A malicious redirect will steal codes. | Redirect URIs match exactly, codes last 60 seconds and are used once, PKCE binds the code to the client that started the flow, and a replayed code revokes what it issued. |
| We will be tied to one hosting company. | Nothing here names a host. Any app that speaks OAuth and HTTP can be a client. |
