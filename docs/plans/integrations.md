# Integrations — implementation brief

Hand this file to Gemini Flash. Do one phase per session. Stop when that phase's tests pass. Do not start the next phase.

Run order is `docs/plans/README.md`. Phase 1 needs `foundation.md` phase 8. Phase 2 needs `foundation.md` phase 7 and `mcp-apps.md` phase 1. Phase 4 needs `jobs.md` phases 1 and 2, `mcp-apps.md` phase 4a, and `record-history.md` phase 4.

An institution connects an application to the world: a webhook to the registrar's workflow tool, a nightly feed from the student information system, a token for an agent that may touch one app and nothing else. Each is a way for data to leave, or for something to act, and each needs a rule. Today webhooks are disabled and cannot succeed, there is no outbound client, no place to keep a credential, and an agent token carries every right of the person who made it.

This brief builds the controls first: an encrypted secrets store, an allowlist for everything that leaves, and tokens that can be narrowed. Then webhooks and an API description, on top of them.

## Decisions already made

| Question | Answer |
| --- | --- |
| Where do outbound credentials live? | In a secrets table, encrypted with a key held in a file the operator mounts. The API can write a secret and use it. It can never read one back |
| Where can the server connect? | Only to hosts a Platform Admin has allowlisted, with private and metadata addresses blocked unless an entry names that network on purpose |
| Is a webhook free to send any data? | No. Creating one is a proposal. A payload holds ids unless an approved list of fields is named, and a sensitive field raises a flagged check |
| Does a rule choose a URL? | No. A rule names an approved webhook. A rule never holds a URL |
| Can a token be narrower than its owner? | Yes. A token can be limited to apps, tables, operations, and tools. A limited token can never exceed its owner |

## What exists today

| Fact | Where |
| --- | --- |
| `WebhookDispatch { target_url }` is logged as `Webhook not sent` and never delivered | `scaffoldry-engine/src/automation.rs` |
| There is no HTTP client dependency | `Cargo.toml` |
| `api_tokens` has a `kind` and the owner's full rights | `foundation.md` phase 7 |
| Settings are a closed list in `platform_settings` | `foundation.md` phase 8 |

## Out of scope

- Inbound webhooks. Nothing outside can start a process through a public URL.
- OAuth client flows to third-party services.
- Mutual TLS.
- A catalog of prebuilt connectors.

## Rules for every phase

1. Write the failing test first. Run it. Confirm it fails because the behavior is missing. Then write the code.
2. Add only the crates the master brief lists for this file, and run `governance/scripts/audit-licenses.py` on them.
3. No outbound connection exists outside `net::egress`. A test scans the source for direct client use.
4. A secret never appears in a log, a response, a job, a ledger entry, or a test failure message.

## Phase 1 — secrets and egress

Needs `aes-gcm` and, for the egress tests, no network.

Master key. `SCAFFOLDRY_MASTER_KEY_FILE` names a file holding 32 random bytes in base64. It is a process fact like the database address. If any secret exists and the file is missing or wrong, boot fails. If none exists and the file is missing, the server starts and the secrets routes answer `503` with error code `master_key_missing`. The operator document says how to generate it and to keep it apart from backups of the database. Add the code to the closed list in `ux-standards.md`.

Migration `0037_secrets.sql`:

```sql
CREATE TABLE IF NOT EXISTS secrets (
    id UUID PRIMARY KEY,
    name VARCHAR(64) NOT NULL UNIQUE,
    ciphertext BYTEA NOT NULL,
    nonce BYTEA NOT NULL,
    created_by VARCHAR(255) NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    rotated_at TIMESTAMPTZ
);
```

AES-256-GCM, a fresh 96-bit nonce per write, and the secret's id as associated data so a ciphertext cannot be moved to another row. Names are `^[a-z][a-z0-9_]{1,63}$`.

Routes, Platform Admin, all in `ADMIN_ROUTES`: `GET /admin/secrets` (name, created, rotated, and how many things reference it, never a value), `PUT /admin/secrets/{name}` (body `value` and `reason`, creates or replaces), `DELETE /admin/secrets/{name}` (refused with `409` and the referencing items if any use it). Each change is a ledger entry, `SecretChanged`, added to `DecisionType::ALL`, holding the name and never the value. The subcommand `scaffoldry-server rotate-master-key {old_file} {new_file}` re-encrypts every secret in one transaction. Use `std::env::args`.

Internal use only: `secrets::get(name) -> Result<Secret>`. `Secret` has no `Debug` that prints its contents, no `Serialize`, and zeroizes its buffer on drop.

Egress. A new module `net/egress.rs`:

- The setting `egress.allow`: an array of entries, each `{ "host": "name", "ports": [443] }` or `{ "cidr": "10.20.0.0/16", "ports": [5432] }`. Added to the closed list, default empty, meaning nothing may leave.
- `resolve_and_check(host, port, resolver) -> Result<SocketAddr>` resolves the name once, rejects loopback, link-local (including the cloud metadata address), private ranges, unique-local, and carrier-grade NAT addresses unless a `cidr` entry covers it, and checks the host and port against the list. The connection then goes to that resolved address, never to the name again, and TLS uses the name for server name checking.
- No redirects are followed. A redirect response is a failure.
- Connect timeout 5 seconds, total 30 seconds, response limit 5 MB.
- An outbound HTTP client wrapper, `egress::http`, built on `ureq` with `rustls` and the system certificate store, whose only way to open a connection is the function above.

Tests. Inject a resolver so no real network is needed.

1. A name that resolves to a private address is refused unless a `cidr` entry covers it.
2. A name that resolves first to a public address and on a second lookup to a private one connects to the first, because the address is pinned.
3. The cloud metadata address, `127.0.0.1`, `::1`, and `169.254.0.0/16` are refused.
4. A host or port not on the list is refused. An empty list refuses everything.
5. A redirect is a failure.
6. A secret round-trips. A ciphertext copied to another row fails to decrypt. A wrong key fails boot when secrets exist.
7. No response, log line, ledger entry, or error message contains a secret value. Scan them in the test with a known value.
8. `grep -rn "ureq::\|TcpStream::connect\|reqwest" crates/ --include=*.rs` shows only `net/egress.rs`.
9. Deleting a secret that something references is `409` and names the references.
10. Web: the panel shows names and never a value, and `Replace` takes a new value in a write-only field.

## Phase 2 — tokens that can be narrower

Migration `0039_token_scope.sql`:

```sql
ALTER TABLE api_tokens ADD COLUMN IF NOT EXISTS scope JSONB;
```

A null scope means the owner's full rights, as today. A scope is:

```json
{ "apps": ["admissions"], "tables": { "applicants": ["read", "create", "update"] }, "tools": ["list_records", "update_record"], "max_rows_per_call": 100 }
```

`tables` maps a table id to a subset of `read`, `create`, `update`, `delete`. A token whose scope names an app may touch only the listed apps. An omitted `tools` list means the tools that fit the listed operations. Admin tools are never allowed to a scoped token.

Enforcement. `session_user` returns the person and the token's scope. `tools::call` checks the scope first, then the tool's own scope, then the person's rights, so the effective right is the intersection. REST record routes call `scope_allows(scope, app, table, op)` through the same access function. A refusal is `403` with error code `token_scope` and a sentence naming what the token may do. Add the code to the closed list. `max_rows_per_call` lowers any list limit.

`POST /auth/tokens` takes `scope`. A scoped token defaults to 30 days and is at most 365. The user settings pane from `foundation.md` phase 9 gains `Limit this token`: apps, then tables, then the operations as checkboxes, shown as a sentence: `This token can read and add Applicants in Admissions. It cannot do anything else.` The tokens panel in the console shows each token's scope as that sentence.

Tests.

1. A token scoped to one app is refused on another with `token_scope`.
2. A token scoped to `read` on one table cannot create or update there and cannot read another table.
3. The effective right is the intersection. A token scoped to `update` on a table its owner can only read cannot update.
4. A scoped token cannot call any admin tool or mint a token.
5. `max_rows_per_call` caps a list.
6. A null scope behaves as before. The existing tests pass unchanged.
7. Web: the sentence matches the chosen boxes. The panel shows it for existing tokens.

## Phase 3 — an API description per app

`GET /api/v1/apps/{slug}/openapi.json`. App read access. It returns an OpenAPI 3.1 document for the caller: paths for each readable table (`GET` and `POST` on records, `GET`, `PUT`, `DELETE` on one record, and `aggregate`), `components.schemas` with one JSON Schema per table built from its fields, and the bearer security scheme.

Field mapping: text types to `string`, numbers and durations to `number`, booleans to `boolean`, dates to `string` with `format`, selects to `string` with an `enum` of option ids and `x-labels` of their labels, multi-valued fields to arrays, links to arrays of ids, attachments to arrays of reference objects. Computed fields are `readOnly`. A field hidden from this caller is omitted, so the description never reveals it. The response sets `ETag` and honors `If-None-Match`.

The same function feeds the MCP resource `scaffoldry://apps/{slug}/openapi`, so an agent can read the exact shape of a record before writing one.

CI. Add `governance/scripts/validate-openapi.py`, using the `jsonschema` package CI already installs: it loads a generated document for a fixture app, checks the required OpenAPI keys, and validates example records against each component schema. Add it as a hard gate beside the others.

Tests.

1. The document for a fixture app has the expected paths and one schema per table.
2. A sample record validates against its schema. A record with an unknown select id does not.
3. A hidden field is absent for a restricted person and present for an admin.
4. Computed fields are `readOnly`.
5. `If-None-Match` returns `304` when nothing changed and `200` after a manifest change.
6. The CI script passes.

## Phase 4 — webhooks

Needs `ureq`.

Definition. Webhooks live in the manifest: `integrations: { webhooks: [Webhook] }`.

```rust
pub struct Webhook {
    pub id: String, pub name: String,
    pub url: String,                 // https only
    pub secret: String,              // name of a secret, used to sign
    pub events: Vec<WebhookEvent>,   // RecordCreated, RecordUpdated, RecordDeleted, ProcessDecided
    pub tables: Vec<String>,
    pub include_values: bool,
    pub fields: Vec<String>,         // used only when include_values is true
    pub filter: Option<FilterTree>,
}
```

Checks added to proposals (`mcp-apps.md` phase 4a): `webhook_host` (fail when the URL's host and port are not on `egress.allow` at approval time, or the secret does not exist), `webhook_sensitive` (flagged when `include_values` is true and any listed field is effectively sensitive), `webhook_scope` (warn when `include_values` is true with no `filter`, so every record's values leave). `webhook_sensitive` needs a second person.

Payload. `{ "id": event id, "event": "record.updated", "app": ..., "table": ..., "record": ..., "version": ..., "at": ..., "changed": ["field", ...], "values": { ...only the listed fields, when approved } }`. The event id is unique and is the idempotency key. A person's name or an address never appears unless it is a listed field.

Delivery uses an outbox. In `write_record`'s transaction, for each matching webhook (the list is cached per app and refreshed on approval), insert a row in:

```sql
CREATE TABLE IF NOT EXISTS webhook_deliveries (
    id UUID PRIMARY KEY,
    app_slug VARCHAR(64) NOT NULL,
    webhook_id VARCHAR(64) NOT NULL,
    event_id UUID NOT NULL,
    payload JSONB NOT NULL,
    state VARCHAR(10) NOT NULL DEFAULT 'queued',
    attempts INTEGER NOT NULL DEFAULT 0,
    next_attempt_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    last_status INTEGER,
    last_error TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    delivered_at TIMESTAMPTZ
);
CREATE INDEX IF NOT EXISTS idx_webhook_due ON webhook_deliveries (next_attempt_at) WHERE state = 'queued';

CREATE TABLE IF NOT EXISTS webhook_state (
    app_slug VARCHAR(64) NOT NULL,
    webhook_id VARCHAR(64) NOT NULL,
    consecutive_failures INTEGER NOT NULL DEFAULT 0,
    disabled_at TIMESTAMPTZ,
    PRIMARY KEY (app_slug, webhook_id)
);
```

(Put both in `0038_webhooks.sql`.) A scheduled job `deliver_webhooks`, every 10 seconds, claims up to 50 due rows with `FOR UPDATE SKIP LOCKED`, signs and posts each through `egress::http`, and records the result. The signature is `X-Scaffoldry-Signature: t={unix},v1={hex}` where the value is HMAC-SHA256 over `{t}.{body}` with the secret. A `2xx` is delivered. Anything else retries with delays of 1 minute, 5 minutes, 30 minutes, 2 hours, 6 hours, 12 hours, and 24 hours, then the row is `dead`. After 20 consecutive failed deliveries a webhook is disabled in `webhook_state` and its owner is told (`notifications.md`). An owner re-enables it after fixing it. The last 200 deliveries per webhook are kept for the call log and an older one is removed by the scheduled purge. A request body is capped at 256 KB.

Process rules. The effect `WebhookDispatch` now carries `{ webhook_id }`, not a URL. The validator requires an approved webhook in the same app. It enqueues a delivery with event `process.step`. Remove `target_url` from the type, and migrate old rules by dropping the step with a counted ledger note.

Disclosure. When a delivery carries an effective-sensitive field, `log_disclosure` records a `webhook` entry with the host as recipient, aggregated by webhook and hour with the count and record ids up to the cap.

Tests, with a local test server in the process reached through an injected resolver and an allowlist entry.

1. A webhook whose host is not allowlisted fails `webhook_host` at proposal time. One with a missing secret fails too.
2. A write that matches creates a delivery in the same transaction. A rolled-back write creates none.
3. A delivery is signed, and the signature verifies with the secret and the timestamp. A changed body fails verification.
4. A `500` retries on the stated schedule and ends `dead` after the seventh attempt. A `200` after two failures ends `delivered`.
5. Twenty consecutive failures disable the webhook and notify the owner. Re-enabling clears the count.
6. A payload with `include_values` false has no `values`. With true it has only the listed fields.
7. A sensitive listed field raises `webhook_sensitive`. The proposer cannot approve it alone.
8. A response redirect is a failure, and a private address is refused.
9. A process rule with `WebhookDispatch` for an unapproved or unknown webhook fails validation.
10. A sensitive delivery writes one aggregated disclosure row per hour with a count.
11. The call log keeps 200 per webhook after a purge.

## Phase 5 — the panels

Console sections, all in `ADMIN_ROUTES` and built from the kit.

- `Outbound access`: the allowlist as a table of entries with `Add`, `Edit`, `Remove`. Each change asks for a reason and is a settings ledger entry. A `Test` button checks a host through `resolve_and_check` and says in words what it would allow or refuse, and why.
- `Secrets`: from phase 1.
- `Webhooks`: every webhook across apps: app, name, host, events, state (`Active`, `Disabled`, `Failing`), last success, and consecutive failures. A row opens the recent deliveries with status codes and the error. `Disable` and `Enable` ask for a reason.
- `Tokens` (from `admin-console.md` phase 7) shows each token's scope as a sentence.

The Overview gains the count of failing and disabled webhooks and of `dead` deliveries in the last day.

Tests.

1. A faculty caller is 403 on every route in this phase.
2. `Test` on a private address says it is blocked and why.
3. The panel shows a webhook's failures and its recent deliveries.
4. Disabling a webhook stops new deliveries, keeps the queued ones in the log as `dead` with a reason, and writes a ledger entry.
5. Web: the allowlist editor asks for a reason, and `Test` shows the explanation.

## How to prompt Gemini

```
Read docs/plans/integrations.md and docs/plans/README.md.
Implement phase N only.
Write the failing test first and run it.
Add only the crates this brief names, and run the license audit on them.
Never log, return, or store a secret in plain text.
Never open a connection outside net::egress.
Never let a rule hold a URL.
Do not read, copy, or adapt another product's source code.
Stop when the tests listed for phase N pass, and paste the command output.
```

## Enterprise objections

| Objection | Answer |
| --- | --- |
| A webhook will send student data to a service we did not approve. | The destination must be on an allowlist an administrator controls, creating one is a proposal, values need an approved field list, and a sensitive field needs a second person and is logged. |
| The server will be used to probe our internal network. | Private, loopback, and metadata addresses are blocked unless an entry names that network on purpose, the address is pinned after one lookup, and redirects are not followed. |
| Secrets in the database will leak in a dump. | They are encrypted with a key in a separate file. A dump alone is useless, and the key is not in it. |
| A narrow token is a lie if the owner can do more. | The effective right is the intersection. A narrowed token can never do more than it says or more than its owner. |
| A broken endpoint will swamp the queue. | Retries back off, a webhook disables itself after 20 consecutive failures, and the owner is told. |
| An API description will reveal hidden fields. | It is generated for the caller, so a hidden field is not in it. |
