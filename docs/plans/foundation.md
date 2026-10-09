# Foundation — implementation brief

Hand this file to Gemini Flash. Do one phase per session. Stop when that phase's tests pass. Do not start the next phase.

Run order is `docs/plans/README.md`. This plan comes first. It adds no feature. It makes the server refuse what it cannot prove, store what it accepts, and decide access from stored facts.

Scaffoldry is an MCP-first platform. Faculty will drive it from desktop agents and ask for thousands of different things. An agent's request is untrusted input. The platform is only as governed as its weakest route. Today several routes do not check the caller at all.

## Decisions already made

Do not reopen these.

| Question | Answer |
| --- | --- |
| Where does the model run? | Outside Scaffoldry. The institution's agent calls the MCP server. Scaffoldry ships no model and calls no model |
| What is the ledger? | The SHA-256 chain in the `governance_ledger` table. Not Git |
| What is a proposal? | A row. Not a Git branch. See `docs/plans/mcp-apps.md` |
| What is Git for? | A later export of app definitions to the institution's GitHub or GitLab. Not built in this plan |
| What is the calculation engine? | `scaffoldry-engine`, native Rust. Not Arrow. Not DataFusion |
| What scale? | One institution. Thousands of signed-in users. Tables up to millions of rows |
| Where is the truth? | PostgreSQL. A `RwLock` map in `ServerState` is a cache of small things, never the only copy |
| Is there a production mode? | No. The server has one behavior. There is no `SCAFFOLDRY_ENV`, and no setting that relaxes a check |
| Where is institution configuration? | In the `platform_settings` table. A Platform Admin edits it. Every change is a ledger entry |
| What stays in the environment? | Facts about the process only: `DATABASE_URL`, `HOST`, `PORT`, `SCAFFOLDRY_DB_WORKERS`, `SCAFFOLDRY_WEB_DIR` |
| How does an agent sign in? | With a token its user minted in the settings pane. The token is a random string. Scaffoldry stores its SHA-256 hash. It is not a JWT |
| What is `jsonwebtoken` for? | Verifying sign-in tokens issued by the institution's identity provider. Nothing else. Scaffoldry signs no token |

## What is wrong today

Each row was read in the code on 2026-10-08.

| Fact | Where |
| --- | --- |
| `POST /api/v1/auth/token` is public. It signs a `central_admin` token for `jordan.lee@state.edu`. It is off only when `SCAFFOLDRY_ENV=production` | `routes/auth.rs` `issue_test_token`, `guard.rs` `PUBLIC_PATHS` |
| Tokens are HS256 with one shared secret. A real identity provider cannot issue one. `aud` is never checked | `jwt.rs` `validate_jwt` |
| A stored session never expires | `guard.rs` `session_user` |
| If PostgreSQL is down, the server starts on memory and loses every write | `state.rs` `ServerState::new` |
| An empty ledger table is refilled with demo entries at boot. Deleting every row hides all history | `repository.rs` `verify_and_initialize_ledger` |
| If the ledger insert fails, the entry is kept in memory only and the request succeeds | `state.rs` `append_ledger_entry` |
| Callers discard the ledger result with `let _ =`. A change can commit with no audit entry | `service/workspaces.rs`, `routes/auth.rs`, `routes/apps.rs` |
| Nothing stops `UPDATE` or `DELETE` on `governance_ledger` | `0002_workspaces_and_ledger.sql` |
| An unknown `decision_type` string is read as `PolicyRevision` | `repository.rs` `parse_decision_type` |
| A workspace create or update through the API never reaches PostgreSQL. Only boot seeding writes workspaces | `service/workspaces.rs`. `ServerState::persist_workspace` has no caller outside tests |
| Every migration file runs on every boot. There is no record of what ran | `repository.rs` `connect` |
| One PostgreSQL connection on one thread serves every request | `repository.rs` `with_client` |
| Record list, get, update, and delete ignore the caller | `service/records.rs`. The parameter is named `_caller` |
| App access compares the caller's department to a department the caller sends in the body | `routes/apps.rs` `create_app_in_workspace`, `update_app` |
| The workspace id in `POST /workspaces/{id}/apps` is ignored. An app belongs to no workspace | `routes/apps.rs`. The parameter is named `_ws_id` |
| `GET /apps/{slug}`, automations list and create, and process list do not check access | `routes/apps.rs` |
| Any faculty or staff member may decide any waiting step in any app | `routes/apps.rs` `decide_app_process` |
| A dataset marked `Restricted / FERPA` returns its sample rows to any signed-in user | `routes/datasets.rs` `get_dataset` |
| The record policy guesses the caller's department: `physics` if the eppn contains "physics", else `biology` | `scaffoldry-policy/src/lib.rs` `authorize_record_action` |
| Publishing an app sets `custom_domain_verified = true`. Nothing verifies the domain | `routes/apps.rs` `publish_app` |
| There is no server image. Compose starts PostgreSQL only. The deploy workflow ships static files | `deploy/` |
| README names Arrow, DataFusion, and embedded Git. None is in the code | `README.md`, `docs/ARCHITECTURE.md` section 6 |

## Out of scope

- Any new screen, grid behavior, or workflow feature.
- A new crate, except `jsonwebtoken` in phase 10.
- An ORM, a migration tool, or a pool crate.
- Async database code. The driver stays `postgres`.
- Rewriting Cedar policies that this plan does not name.
- Editing `.github/workflows/deploy.yml` or deleting `deploy/Dockerfile.web`. Johann decides what the cloud demo becomes.

## Rules for every phase

1. Write the failing test first. Run it. Confirm it fails because the behavior is missing. Then write the code.
2. A write that fails returns an error to the caller. Do not write `let _ =` in front of a repository call or a ledger call.
3. Access is decided from stored rows. Never read a department, a role, or an owner from the request body to decide access.
4. Do not rename an existing test. If a test asserted the old open behavior, change its assertion and say so in the session output.

## Phase 1 — the documents say what the code is

Edit only `README.md` and `docs/ARCHITECTURE.md`.

| File | Change |
| --- | --- |
| `README.md` architecture table | Calculation Engine is `scaffoldry-engine` (Rust). Audit Ledger is a SHA-256 hash chain in PostgreSQL. Add a row: Agent Interface, Model Context Protocol |
| `README.md` capabilities | Replace "AI-Native App Creation" text with: an institution's own agent builds apps through the MCP server. Scaffoldry runs no model. Replace "immutable Git history" with "hash-chained ledger" |
| `README.md` local development | The API server listens on `8080` unless `PORT` is set. Fix the stated port |
| `ARCHITECTURE.md` section 5 backup | Remove "bundles the local `.git` ledger". The ledger is in the `pg_dump` |
| `ARCHITECTURE.md` section 6.1 | A proposal is a row with a draft manifest. Approval writes the manifest and a ledger entry in one transaction. Remove branch, merge, and signed commit |
| `ARCHITECTURE.md` section 6.3 | Mode B is a planned export. Mark it "not built" |
| `ARCHITECTURE.md` section 7 rule 4 | Remove "Arrow columnar memory". User fields live in record JSONB |
| `ARCHITECTURE.md` layers 2 and 3 | Remove "sub-millisecond", "microsecond", and "under 5ms". No test measures them |

No test. Paste the diff.

## Phase 2 — a failed write fails the request

Ledger:

1. `append_ledger_entry` returns the repository error. Delete the branch that appends to memory after a database error. The memory path remains only when `repository` is `None`.
2. Every caller of `append_ledger_entry` propagates the error with `?`. Append the ledger entry before the change it records. If the append fails, the change does not happen.
3. `parse_decision_type` returns `Result`. An unknown string is `RepositoryError::TamperDetected`.
4. An empty ledger is legal only on first install. Phase 3 adds the table that tells you. Until then, leave the empty check alone.

Sessions: a session older than one hour is refused in `session_user`. Compare `created_at`.

Do not touch `SCAFFOLDRY_ENV` or `/auth/token` in this phase. Phases 7 and 8 remove them.

Tests in `server_hardening_test.rs`:

1. A session whose `created_at` is two hours old gets 401.
2. A ledger row with `decision_type = 'Nonsense'` makes `verify_and_initialize_ledger` return `TamperDetected`.
3. Make the ledger insert fail inside the test. `update_workspace` returns an error and the stored workspace is unchanged.

## Phase 3 — migrations run once, and the ledger is append-only

Migration `crates/scaffoldry-core/migrations/0008_schema_migrations.sql`:

```sql
CREATE TABLE IF NOT EXISTS schema_migrations (
    filename VARCHAR(128) PRIMARY KEY,
    applied_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
```

In `repository.rs`, inside the existing advisory lock: create that table first. Then, for each schema const in order, skip it when its filename is in `schema_migrations`. Otherwise run it and insert the filename in the same transaction. Every file from `0001` to `0007` is already safe to run twice, so an existing database records them on its first boot under this code.

Migration `0009_ledger_append_only.sql`:

```sql
CREATE OR REPLACE FUNCTION governance_ledger_immutable() RETURNS trigger AS $$
BEGIN
    RAISE EXCEPTION 'governance_ledger is append-only';
END;
$$ LANGUAGE plpgsql;

DROP TRIGGER IF EXISTS governance_ledger_no_change ON governance_ledger;
CREATE TRIGGER governance_ledger_no_change
    BEFORE UPDATE OR DELETE OR TRUNCATE ON governance_ledger
    FOR EACH STATEMENT EXECUTE FUNCTION governance_ledger_immutable();
```

Empty ledger rule: `verify_and_initialize_ledger` takes `fresh_install: bool`. It is true only when `0002_workspaces_and_ledger.sql` was applied during this boot. Empty and fresh: write one genesis entry, `DecisionType::PolicyRevision`, principal `system`, rationale `Ledger initialized`. Empty and not fresh: `TamperDetected`. The four demo entries in `seed_genesis_ledger` are still written after the genesis entry on a fresh install. Phase 8 moves them to the `seed-demo` command.

Workspaces: `create_workspace`, `update_workspace`, and every collaborator change in `service/workspaces.rs` call `repo.upsert_workspace`, `repo.upsert_collaborator`, or `repo.remove_collaborator` and return the error.

Tests in `postgres_persistence_test.rs`:

1. Connect twice. `schema_migrations` has one row per file, not two.
2. `DELETE FROM governance_ledger` raises an error.
3. Create a workspace through the service, build a second `ServerState`, and read the workspace back.
4. A database with `schema_migrations` rows and an empty ledger fails `connect` with `TamperDetected`. Drop the trigger inside the test to empty the table.

## Phase 4 — more than one connection

`PostgresRepository` keeps the job channel. Start `SCAFFOLDRY_DB_WORKERS` threads, default 8. Each owns one `postgres::Client`. They share the receiver through `Arc<Mutex<Receiver<WorkerJob>>>`. Migrations and ledger verification run on the first client before the others start.

Do not add a pool crate. Do not change `with_client`'s signature.

`append_ledger_decision` already takes `LOCK TABLE ... IN EXCLUSIVE MODE` inside a transaction. That stays. It is what keeps sequence numbers unique across connections.

Test in `postgres_persistence_test.rs`: two threads each call a test-only `with_client` job that runs `SELECT pg_sleep(0.5)`. Both finish in under 0.9 seconds.

A second test: twenty threads each append one ledger entry. The chain verifies and holds twenty more sequences with no gap.

## Phase 5 — an app lives in a workspace, and access comes from stored rows

Migration `0010_app_workspace.sql`:

```sql
ALTER TABLE app_manifests ADD COLUMN IF NOT EXISTS workspace_id VARCHAR(64) REFERENCES workspaces(id);
CREATE INDEX IF NOT EXISTS idx_app_manifests_workspace ON app_manifests(workspace_id);
```

Add `workspace_id: Option<String>` to `AppManifest` in `scaffoldry-engine/src/lib.rs` with `#[serde(default)]`. `create_app_in_workspace` sets it from the path. It ignores any `workspace_id` in the body. `update_app` keeps the stored `workspace_id` and the stored `department`. It ignores both in the body.

Add one function in a new file `crates/scaffoldry-server/src/service/access.rs`:

```rust
pub enum AppAction { Read, WriteRecords, Manage }

pub fn authorize_app(caller: &AuthUser, slug: &str, action: AppAction, state: &SharedState)
    -> Result<(), ServiceError>
```

It loads the app, then its workspace, then the caller's collaborator row. It calls the existing `authorize_workspace_action` with these action names. Add the matching `permit` rules to `default_institutional_engine`.

| `AppAction` | Cedar action | Allowed for |
| --- | --- | --- |
| `Read` | `read_app` | Any collaborator. Platform Admin. Org Unit Admin in scope |
| `WriteRecords` | `write_record` | `owner`, `admin`, `editor`. Platform Admin |
| `Manage` | `manage_app` | `owner`, `admin`. Platform Admin. Org Unit Admin in scope |

An app with no `workspace_id` is visible to a Platform Admin only. Use `unit_in_scope` from `service/organizations.rs` for the org check. Do not write a second walk.

Call `authorize_app` first in each of these:

| Route or function | Action |
| --- | --- |
| `get_app`, `get_app_schema`, `get_table_schema` | `Read` |
| `list_records`, `get_record`, `list_table_records`, `get_table_record` | `Read` |
| `create_record`, `update_record`, `delete_record`, and the table variants | `WriteRecords` |
| `update_app`, `publish_app` | `Manage` |

Delete the department guess in `authorize_record_action` (`if identity.eppn.contains("physics")`). `ManifestEngine::submit_record` stops calling Cedar. The service has already decided. Remove `authorize_departmental_action` calls from `routes/apps.rs`.

`publish_app` stores the domain with `custom_domain_verified = false`. Nothing in this plan sets it to true.

Tests in `auth_enforcement_test.rs`. Fixture: workspace A with an owner, an editor, and a viewer. Workspace B with a different owner. One app in A.

1. B's owner gets 403 on `GET /apps/{slug}`, on record list, and on record create.
2. A's viewer gets 200 on record list and 403 on record update.
3. A's editor gets 200 on record update and 403 on `PUT /apps/{slug}`.
4. B's owner sends `PUT /apps/{slug}` with `department` set to their own department. 403. The stored manifest is unchanged.
5. `POST /workspaces/A/apps` with `workspace_id: "B"` in the body stores `A`.

## Phase 6 — the remaining open routes

Same function, same fixture.

| Route | Rule |
| --- | --- |
| `list_app_automations`, `list_app_processes` | `authorize_app` `Read` |
| `create_app_automation`, `simulate_app_automation` | `authorize_app` `Manage`. `simulate` uses the session caller. It ignores `principal` and `affiliation` in the body |
| `decide_app_process` | `authorize_app` `Read`, and the caller's collaborator `role` or `affiliation` equals the waiting step's `role`. A Platform Admin does not bypass the role match. Unknown affiliation is 403. Delete `unwrap_or(EduPersonAffiliation::Faculty)` |
| `get_dataset`, `list_datasets` | `sample_data` is returned only when `sensitivity_level` is `Public` or `Directory`, or the caller is a Platform Admin |
| `GET /governance/ledger`, `GET /governance/oscal/export` | Platform Admin, or affiliation `compliance` |
| `POST /policies/simulate` | Platform Admin, or affiliation `compliance` |

`decide_app_process` writes the changed record with `repo.upsert_record` and returns the error.

Tests:

1. B's owner gets 403 on automation create and on process list for A's app.
2. A step with `role: "admin"` is refused for A's editor and accepted for A's admin.
3. A caller with affiliation `member` gets 403 on decide.
4. A faculty caller reading the `grants` dataset receives no `sample_data` key.
5. A student gets 403 on `GET /governance/ledger`.

## Phase 7 — tokens are rows

Migration `0012_api_tokens.sql`:

```sql
CREATE TABLE IF NOT EXISTS api_tokens (
    token_hash CHAR(64) PRIMARY KEY,
    id UUID NOT NULL UNIQUE,
    kind VARCHAR(16) NOT NULL,
    eppn VARCHAR(255) NOT NULL,
    label VARCHAR(255) NOT NULL DEFAULT '',
    original_admin VARCHAR(255),
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    expires_at TIMESTAMPTZ NOT NULL,
    last_used_at TIMESTAMPTZ,
    revoked_at TIMESTAMPTZ
);
CREATE INDEX IF NOT EXISTS idx_api_tokens_eppn ON api_tokens(eppn);
```

`kind` is `agent`, `impersonation`, `scim`, or `setup`. No other value.

A token is the text `scf_` followed by the hex of two `Uuid::new_v4()` values. The server returns it once. It stores only the SHA-256 hex of the full text. Do not store or log the token.

`session_user` in `guard.rs` becomes: hash the bearer text, load the row, refuse it when it is missing, revoked, or expired, then call `resolve_user(eppn)`. Update `last_used_at` at most once a minute for a given row.

`resolve_user(eppn, state) -> Option<AuthUser>`, in a new file `service/identity.rs`, builds the caller from stored rows. A token carries no name, no affiliation, and no department.

| Field | Source |
| --- | --- |
| `name`, `department`, `role_title` | The SCIM user whose `user_name` is the eppn |
| `affiliation` | `central_admin` when the eppn holds `platform_admin` on the root unit. Otherwise the user's eduPerson affiliation from SCIM `roles`. Otherwise `member` |
| Refused | The SCIM user exists and `active` is false |

The `setup` kind is the one exception. Its eppn is `setup@scaffoldry.local` and it resolves to `central_admin` with no SCIM row.

First boot: when `api_tokens` holds no row and no `platform_admin` role exists, the server creates one `setup` token that expires in 24 hours and prints it to standard output once. `scaffoldry-server setup-token` prints a new one and revokes the old. Read the subcommand from `std::env::args`. Do not add an argument parser.

Routes, all under `/api/v1`:

| Method | Path | Who | Behavior |
| --- | --- | --- | --- |
| GET | `/auth/tokens` | signed in | The caller's `agent` tokens: id, label, created, expires, last used. Never the token or its hash |
| POST | `/auth/tokens` | signed in, and the caller's own token is not an `agent` token | Body `label`, `days`. `days` is capped at 90 until phase 8 makes the cap a setting. Returns the token once |
| DELETE | `/auth/tokens/{id}` | the owner, or a Platform Admin | Sets `revoked_at` |

An agent token cannot mint another token. A leaked agent token must not be able to outlive its own revocation.

Impersonation writes an `impersonation` row for the target eppn with `original_admin` set and a one-hour expiry. `stop-impersonate` revokes it and returns no token. The admin's own token still works. `auth_sessions` is no longer read or written. Leave the table.

The SCIM credential becomes a `scim` row minted by a Platform Admin with `POST /auth/tokens` and `kind: "scim"`. `require_scim_credential` looks the hash up. Delete `SCAFFOLDRY_SCIM_TOKEN` and `ServerState.scim_token`.

Delete: `POST /auth/token`, `issue_test_token`, `resolve_defaults`, `mint_test_jwt`, `mint_impersonation_jwt`, `sign_jwt`, `DEFAULT_SECRET`, `SCAFFOLDRY_JWT_SECRET`, and the HS256 branch of `validate_jwt`. Delete `/.well-known/openid-configuration` and `/.well-known/jwks.json`. Scaffoldry is not an issuer and must not describe one.

Tests: change the `authed_req` helper in each test file so it inserts an `api_tokens` row and a SCIM user for the persona, then sends that token. Do not change the tests that use the helper.

New tests in `auth_enforcement_test.rs`:

1. A minted token works. After `DELETE`, the same token is 401.
2. An expired token is 401.
3. The database holds no column equal to the token text.
4. A request made with an agent token gets 403 on `POST /auth/tokens`.
5. Set the SCIM user to `active: false`. That user's token is 401 on the next request.
6. `POST /api/v1/auth/token` is 404.
7. A second boot on a database that already has a Platform Admin prints no setup token.

ponytail: one indexed lookup per request. Cache rows for a few seconds only when a measurement shows the lookup in a profile.

## Phase 8 — settings are rows, and the mode is gone

Migration `0013_platform_settings.sql`:

```sql
CREATE TABLE IF NOT EXISTS platform_settings (
    key VARCHAR(64) PRIMARY KEY,
    value JSONB NOT NULL,
    updated_by VARCHAR(255) NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
```

The keys are a closed list. Reject any other key with 400. Validate the value's type.

| Key | Type | Default when absent | Meaning |
| --- | --- | --- | --- |
| `oidc.issuer` | string | none | Exact `iss` to accept. Phase 10 |
| `oidc.audience` | string | none | Exact `aud` to accept. Phase 10 |
| `oidc.jwks` | object | none | The identity provider's JWKS document, pasted in. Phase 10 |
| `cors.allowed_origins` | array of strings | empty | Replaces `SCAFFOLDRY_ALLOWED_ORIGINS` |
| `tokens.max_days` | integer 1 to 365 | 90 | Longest life of an agent token |
| `tokens.agent_enabled` | boolean | true | When false, `POST /auth/tokens` is 403 for `agent` |
| `process.stale_days` | integer 1 to 365 | 14 | A waiting process step older than this is shown as stale. Added by `admin-console.md` phase 6 |
| `mcp.disabled_tools` | array of strings | empty | Tool names an administrator has switched off. Each must be a name in `TOOLS`. Added by `admin-console.md` phase 7 |
| `pages.disabled` | array of strings | empty | Custom pages that may not run, as `app_slug/page_id`. Added by `admin-console.md` phase 7 |

Do not add a key that turns a check off. There is no `auth.disabled`, no `dev_mode`, and no `allow_insecure`.

Routes: `GET /api/v1/settings` and `PUT /api/v1/settings/{key}`, Platform Admin only. A `PUT` appends a ledger entry, `DecisionType::PolicyRevision`, with the key and the SHA-256 of the new value in the payload, then writes the row. Both happen or neither does.

Settings are loaded into one `RwLock<HashMap<String, Value>>` on `ServerState` at boot and replaced after each `PUT`. The CORS layer reads the map on each request through `AllowOrigin::predicate`.

Remove the mode:

| Today | After |
| --- | --- |
| `SCAFFOLDRY_ENV` | Deleted everywhere. Delete `validate_production_configuration` |
| `ServerState::new` falls back to memory when PostgreSQL is unreachable | `new` returns the error. Add `ServerState::in_memory()` for tests. Nothing in `main.rs` calls it |
| The `CI` variable changes boot behavior | Deleted |
| Demo workspaces, datasets, relationships, automations, and ledger entries are written at boot | Moved to `scaffoldry-server seed-demo`. Boot writes the root organization unit and the genesis ledger entry, and nothing else |
| `SCAFFOLDRY_OIDC_ISSUER`, `SCAFFOLDRY_ALLOWED_ORIGINS` | Deleted. Read the setting |

Tests that need the demo workspaces call the seed function directly.

Tests:

1. `PUT /settings/cors.allowed_origins` as a Platform Admin adds one ledger entry. A preflight from that origin is then allowed without a restart.
2. `PUT /settings/dev_mode` is 400.
3. A faculty caller gets 403 on `GET /settings`.
4. `grep -r SCAFFOLDRY_ENV crates/` prints nothing. Paste the output.
5. `ServerState::new` with an unreachable `DATABASE_URL` returns an error.
6. A fresh database after boot has zero workspaces.

ponytail: the settings map is per process. The appliance is one process. A second process would need a reload signal.

## Phase 9 — the settings panes

Web only. Use the existing `apiClient` in `apps/web/src/api.ts`. Do not add a second HTTP helper or a dependency.

Sign-in: when no token is stored, the desk shows one field, `Paste your token`, `data-testid="token-sign-in"`. It calls `GET /auth/me` with that token and stores it on success. Delete `apiClient.issueToken` and the call that issues a token for the active persona in `AdminDesk.tsx`. The persona switcher stays. It is visible to a Platform Admin only and calls `apiClient.impersonateUser`.

User settings pane, opened from the user menu, `data-testid="user-settings"`. One section, heading `Agent tokens`:

- A list, `data-testid="agent-token-list"`: label, created, expires, last used, and a `Revoke` button per row.
- A form, `data-testid="agent-token-form"`: label, and days up to `tokens.max_days`.
- After create, show the token once in a read-only field, `data-testid="agent-token-secret"`, with the text `Copy this token now. It is not shown again.` and the server's MCP URL beside it. Leaving the pane clears it from memory. Do not write it to `localStorage`.
- Hidden when `tokens.agent_enabled` is false.

Admin console: add a tab `Settings`, `data-testid="admin-settings-tab-btn"`, inside the existing Platform Admin gate. One row per key from phase 8 with its current value, an editor fitted to its type, and `Save`. `oidc.jwks` is a text area that must parse as JSON before `Save` is enabled. Add a button `Mint SCIM token` that shows the token once in the same way.

Tests in a new `apps/web/src/test/settings.test.tsx`, with `fetch` stubbed:

1. Creating a token shows the secret once. After closing and reopening the pane, the secret is absent and the row is listed.
2. `Revoke` calls `DELETE /api/v1/auth/tokens/...`.
3. A faculty persona does not see `admin-settings-tab-btn`.
4. Saving `tokens.max_days` calls `PUT /api/v1/settings/tokens.max_days`.
5. `localStorage` holds no value equal to the minted token.

## Phase 10 — institution sign-in

This phase adds one dependency: `jsonwebtoken` (MIT). Add nothing else.

A person signs in at the institution's identity provider. The desk sends the provider's token to Scaffoldry. Scaffoldry verifies it. Scaffoldry is a resource server. It is not an authorization server and signs nothing.

`session_user` tries the bearer text in this order: an `scf_` token is looked up as in phase 7. Anything else is verified as a JWT when `oidc.issuer`, `oidc.audience`, and `oidc.jwks` are all set. When any is unset, a JWT is 401.

`validate_jwt` accepts RS256 and ES256 against the keys in `oidc.jwks`. It checks `exp`, `iss`, and `aud`. It reads `sub`, or `eppn` when present, and calls `resolve_user`. No other claim is trusted. A token cannot make a Platform Admin. Only a `platform_admin` row in `roles` does.

`/.well-known/oauth-protected-resource` returns `oidc.issuer` in `authorization_servers` and the request's own origin plus `/api/mcp` in `resource`. When `oidc.issuer` is unset, `authorization_servers` is empty.

Do not build the browser redirect to the identity provider in this phase. The token field from phase 9 remains the way in. The redirect is its own brief, written when a pilot names its provider.

Tests in `oidc_auth_test.rs`, with an RSA key pair generated in the test and its JWKS stored through `PUT /settings/oidc.jwks`:

1. A valid RS256 token is accepted.
2. A wrong `aud` is 401. A wrong `iss` is 401. An expired token is 401.
3. An HS256 token is 401.
4. A token with a claim `affiliation: "central_admin"` cannot open `POST /api/v1/orgs`.
5. Replace `oidc.jwks` with a second key through the settings route. A token signed by the first key is 401 on the next request, with no restart.

## Phase 11 — one image

The API server serves the built web files. Enable the `fs` feature of `tower-http`, which is already a dependency. In `build_app_with_state`, fall back to `ServeDir` on `SCAFFOLDRY_WEB_DIR` with `index.html` as the not-found file, when that variable is set. The guard treats any path outside `/api`, `/scim`, and `/.well-known` as public.

Add `deploy/Dockerfile`: build the web app with Node, build the server with `cargo build --release`, copy both into a small runtime image, run as a non-root user. Add a `scaffoldry` service to `deploy/docker-compose.yml` that depends on `postgres` being healthy. Delete `deploy/migrations/`. It is a stale copy.

Test: an integration test sets `SCAFFOLDRY_WEB_DIR` to a temp directory with an `index.html`, requests `/`, and gets that file without a token. `/api/v1/workspaces` without a token is still 401.

## How to prompt Gemini

```
Read docs/plans/foundation.md and docs/plans/README.md.
Implement phase N only.
Write the failing test first and run it.
Do not add a dependency.
Do not write `let _ =` in front of a repository or ledger call.
Do not decide access from a value in the request body.
Stop when the tests listed for phase N pass, and paste the command output.
```

For phase 10 only, replace the dependency line with: `Add jsonwebtoken and nothing else.`

For phases 7 and 8, add: `Do not add a setting, a flag, or a variable that turns a check off.`

## Enterprise objections

| Objection | Answer |
| --- | --- |
| A database administrator can still drop the trigger and rewrite the ledger. | Yes. The trigger stops an application bug and a casual edit. A rewrite changes every later hash, and boot refuses a chain that does not verify. Detecting a rewrite of the whole chain needs a copy of the head hash held outside the database. That is the Git export, and it is not built. |
| A development flag will reach production. | There is no flag. The server has one behavior. Demo data is a command someone runs. The memory store is a constructor only tests call. |
| A setting in the database can switch security off. | The key list is closed and holds no such key. Phase 8 test 2 sends one and gets 400. Every change to a setting is a ledger entry with the admin's name. |
| A minted token is a password that never changes. | It expires, at most 90 days by default. Its owner revokes it in one click. It stops working the moment SCIM marks the owner inactive. It cannot mint another token. Only its hash is stored. |
| The first admin has no way in. | First boot prints a setup token that lasts 24 hours. `scaffoldry-server setup-token` prints a new one for whoever has a shell on the host, who already controls the appliance. |
| Eight connections is not a pool. | It is a fixed pool of eight with no new dependency. The phase 4 test proves two queries overlap. Raise the number with one variable. Replace it when a load test shows the queue. |
| Blocking database calls stall the async runtime. | They do, for the length of one query. The MCP plan runs every tool call on the blocking thread pool at one dispatch point. REST adapters call the same dispatch. |
| The identity provider rotates keys. | A Platform Admin pastes the new JWKS into the `oidc.jwks` setting. It takes effect on the next request. Scaffoldry does not fetch keys itself, so it needs no outbound network access. |
| Removing demo data breaks the demo. | `scaffoldry-server seed-demo` writes it. The demo runs that command once. |
