# MCP apps — implementation brief

Hand this file to Gemini Flash. Do one phase per session. Stop when that phase's tests pass. Do not start the next phase.

Run order is `docs/plans/README.md`. `docs/plans/foundation.md` phases 2 to 9 come before phase 1 of this plan.

An agent connects with a token its user minted in the settings pane (foundation phases 7 and 9). The user pastes the server's MCP URL and that token into the desktop tool. Do not add another way for an agent to sign in.

Scaffoldry is MCP-first. A faculty member opens a desktop agent, connects it to the institution's Scaffoldry server, and asks for an app. The agent builds it through MCP tools. The web desk is a second client of the same service layer. It is not the primary one.

An app has two layers. They are governed in different ways.

| Layer | Form | How it is governed |
| --- | --- | --- |
| Data contract: tables, fields, sensitivity, links, process rules | Declarative JSON in the manifest | Validated against a schema. Proposed, checked, approved |
| Pages | A built-in view, or a custom page: one JavaScript module an agent wrote | A custom page runs in a sandbox with no network and no token. A reviewer approves what it may read and write, not its code |

A custom page may render anything. It has no power of its own. Its one capability is a message to its host, and the host turns that message into one tool call under the signed-in user's rights. Do not grow the component catalog in `routes/framework.rs` to chase custom layouts. A layout the catalog cannot express is a custom page.

The platform must hold up when thousands of people ask an agent for thousands of different things. That is a property of the tool surface, not of any one tool:

1. Few tools, each general. An agent composes them. Do not add a tool per use case.
2. One dispatch point. It signs in, validates, authorizes, and audits. A tool cannot skip a step, because the tool does not perform the step.
3. Every structural change is a proposal. Nothing an agent sends changes a live app until a person with authority approves it.
4. Every answer is bounded. A list has a page size. A manifest has size limits.

## Standard

| Standard | Use |
| --- | --- |
| Model Context Protocol, revision `2025-06-18` | Tools, resources, prompts over JSON-RPC 2.0 on one HTTP endpoint. Also accept `2024-11-05` in `initialize` |
| MCP Apps extension (SEP-1865) | `ui://` resources with `text/html;profile=mcp-app`, linked from a tool. A custom page is served in this form, so one artifact runs in the desktop agent and in the web desk |
| HTML `iframe` `sandbox`, and Content Security Policy Level 3 | The page sandbox |
| JSON Schema 2020-12 | Tool input schemas, and the app manifest schema |
| RFC 6750 bearer token | An agent sends a token its user minted. Built in foundation phase 7 |
| RFC 9728 protected resource metadata | Names the institution's identity provider. Built in foundation phase 10 |

Do not adopt an MCP SDK crate. The protocol surface used here is six methods.

## What already exists

| Fact | Where |
| --- | --- |
| One JSON-RPC handler, 1,171 lines, one `match` on tool name | `crates/scaffoldry-server/src/routes/mcp.rs` |
| Mounted at `/api/mcp` and `/api/v1/mcp` | `routes/mod.rs` |
| Workspace tools call the service layer and are authorized | `service/workspaces.rs` |
| Two `ui://` screens rendered with escaped strings | `service/governance.rs` |
| App manifest type | `scaffoldry-engine/src/lib.rs` `AppManifest` |

These parts are not real. Each is deleted or replaced in phase 1.

| Tool or resource | What it does today |
| --- | --- |
| `query_dataset` | Returns rows typed into the source file. It reads no dataset |
| `simulate_cedar_policy` | Checks whether the affiliation string contains "compliance" or "faculty". It does not call Cedar |
| `policies://cedar` | Returns a policy text that is not the policy the engine runs |
| `compliance://oscal` | Returns three fixed strings |
| `create_app_proposal` | Checks nothing about the caller. Reports `ferpa_scan: Passed` without scanning. Registers an empty manifest under the slug at once, which replaces a live app of that name. Stores no proposal |
| `calculate_formula` | A second aggregate calculator beside `evaluate_formula` |
| `notifications/initialized` | Returns a JSON-RPC result. A notification gets no result |
| The web AI drawer | Matches keywords and waits on a timer. `apps/web/src/AIAssistantDrawer.tsx` |

## Out of scope

- A model, a model proxy, an API key for a model, or a chat screen. The agent is the user's own.
- An MCP SDK, a JSON Schema validator crate, or a rate-limit crate.
- A tool that runs SQL, a script, or a formula across tables on the agent's behalf.
- Server-sent events, sampling, elicitation, and resource subscriptions.
- Git branches. A proposal is a row.
- A tool per department, per form type, or per report.
- New entries in the component catalog. No layout language, no slot system, no theme options.
- A bundler, or any build step on the server. JSX is compiled inside the sandbox when the page opens.
- npm packages inside a page. A page imports `react`, `react/jsx-runtime`, and `scaffoldry` and nothing else.
- A page that calls a tool other than `page_call`. A page never decides a proposal or a process step.

## Rules for every phase

1. Write the failing test first. Run it. Confirm it fails because the behavior is missing. Then write the code.
2. Do not add a dependency. The one exception is `react`, `react-dom`, and `sucrase` as `devDependencies` in phase 6.
3. A tool handler does not authorize. The dispatcher does, before the handler runs.
4. A tool handler does not parse arguments by hand. It deserializes them into a struct with `#[serde(deny_unknown_fields)]`. A failure is a tool error naming the field.
5. REST routes that do the same work call the same service function. Do not copy logic into a route.
6. No tool returns more than 200 rows or 256 KB. `get_page` alone may return 1 MB, because it carries the React runtime and the compiler. Measure it and lower the limit if it is smaller.

## Phase 1 — a tool registry with one gate

New file `crates/scaffoldry-server/src/service/tools.rs`:

```rust
pub enum Scope {
    SignedIn,                    // any authenticated caller
    App(AppAction),              // reads `app_slug` from the arguments, calls access::authorize_app
    Workspace(&'static str),     // reads `workspace_id`, calls the existing workspace check with this Cedar action
    PlatformAdmin,
}

pub struct Tool {
    pub name: &'static str,
    pub description: &'static str,
    pub input_schema: fn() -> serde_json::Value,
    pub scope: Scope,
    pub read_only: bool,
    pub run: fn(&AuthUser, serde_json::Value, &SharedState) -> Result<serde_json::Value, ServiceError>,
}

pub static TOOLS: &[Tool] = &[ /* one entry per tool */ ];

pub fn call(caller: &AuthUser, name: &str, args: serde_json::Value, state: &SharedState)
    -> Result<serde_json::Value, ServiceError>
```

`call` finds the tool, checks `scope`, then runs it. An unknown tool is `NotFound`. A missing `app_slug` or `workspace_id` for a scoped tool is `BadRequest`.

In `routes/mcp.rs`:

- `tools/list` is built from `TOOLS`. Each entry carries `annotations.readOnlyHint` from `read_only`.
- `tools/call` runs `tools::call` inside `tokio::task::spawn_blocking`. The result is returned as `structuredContent`, and as one `text` block holding the same JSON.
- `initialize` answers with the client's requested version when it is `2025-06-18` or `2024-11-05`. Otherwise it answers `2025-06-18`.
- A request with no `id` returns HTTP 202 with an empty body.
- `GET /mcp` returns 405. Delete `get_mcp_overview`.

Move these existing tools into `TOOLS` unchanged in behavior: `list_workspaces`, `get_workspace`, `update_workspace`, `manage_workspace_member`, `create_record`, `list_datasets`, `record_governance_decision`, `verify_decision_ledger`, `export_oscal_compliance`, `get_framework_spec`.

Fix or delete:

| Item | Action |
| --- | --- |
| `simulate_cedar_policy` | Call the policy engine. Scope `PlatformAdmin` |
| `policies://cedar` | Return the policy source the engine was built from. Store that string on `ScaffoldryPolicyEngine` |
| `query_dataset` | Delete. Phase 2 replaces it |
| `create_app_proposal` | Delete. Phase 4 replaces it |
| `calculate_formula` | Delete |
| `compliance://oscal` | Delete the resource. `export_oscal_compliance` is the real one |
| `apps/web/src/AIAssistantDrawer.tsx` | Delete the file, its button, and its tests |

Tests in `mcp_authoritative_api_test.rs`:

1. `tools/list` returns exactly the names in `TOOLS`.
2. Table-driven: for every tool whose scope is `App` or `Workspace`, a caller with no role in that workspace gets a tool error. The test iterates `TOOLS`. A tool added later is covered without a new test.
3. A call with an unknown argument name returns a tool error that names the argument.
4. A notification returns 202 and no body.
5. `simulate_cedar_policy` for a student exporting a FERPA record returns `Deny`, and the reasons list is the engine's.

## Phase 2 — data tools

Needs `docs/plans/live-data.md` phases 1 and 2. Records are read from PostgreSQL, a page at a time.

| Tool | Scope | Arguments | Result |
| --- | --- | --- | --- |
| `list_apps` | `Workspace("access_workspace")` | `workspace_id` | slug, title, table names |
| `describe_app` | `App(Read)` | `app_slug` | The stored manifest and its `version` |
| `list_records` | `App(Read)` | `app_slug`, `table_id`, `filter` (the existing `CompoundFilter`), `sort`, `limit` up to 200, `cursor` | Rows with `id` and `version`, and `next_cursor` |
| `get_record` | `App(Read)` | `app_slug`, `record_id` | One row with `version` |
| `update_record` | `App(WriteRecords)` | `app_slug`, `record_id`, `version`, `data` | The new row. A stale `version` is a tool error with the current version |
| `delete_record` | `App(WriteRecords)` | `app_slug`, `record_id`, `version` | `deleted: true` |

Needs `docs/plans/row-scale.md` phase 1 for the `table_id` column and the paging index.

With no `filter` and no `sort`, rows come back in `(created_at, id)` order from the paging index.

`filter` and `sort` are translated to SQL on the JSONB column with bound parameters. A field name is checked against the manifest before it enters the statement. A name that is not a field of that table is `BadRequest`. Do not build SQL from a string the caller sent.

In this phase a `filter` or `sort` is accepted only when the table holds 10,000 rows or fewer. Probe with `SELECT count(*) FROM (SELECT 1 FROM dataset_records WHERE app_slug = $1 AND table_id = $2 LIMIT 10001) t`. Above that, return `BadRequest` with the text `This table is too large to filter or sort on a field that is not indexed.` `docs/plans/row-scale.md` phase 2 lifts the limit for indexed fields.

A field marked `ferpa_sensitive` in the manifest is removed from every row returned to a caller whose workspace role is `viewer`. Do this in the service function, so REST gets it too.

`cursor` is the last `(created_at, id)` pair, base64 of JSON. Do not use `OFFSET`.

The REST record routes call these same service functions.

Tests:

1. 450 records, `limit` 200: three calls return 200, 200, 50 and no row twice.
2. A `filter` on a field that is not in the manifest is a tool error, and no SQL runs.
3. `update_record` with a stale version is a tool error and the row is unchanged.
4. A field marked `ferpa_sensitive` is absent from `list_records` rows for a caller whose workspace role is `viewer`.

## Phase 3 — the manifest has a format

Views leave the manifest in `views.md` phase 1. After that, this validator does not check view contents, and a `View` page names an id in `app_views`. Access rules and column rules from `access-rules.md` join the manifest in that brief.

An agent needs a contract to build against. Today the contract is whatever `serde` accepts.

Add pages to the manifest in `scaffoldry-engine/src/lib.rs`. `AppManifest` gains `pages: Vec<AppPage>` with `#[serde(default)]`.

```rust
pub struct AppPage {
    pub id: String,
    pub title: String,
    pub kind: PageKind,
}

pub enum PageKind {
    View { view_id: String },
    Custom { source_sha256: String, grants: Vec<PageGrant> },
}

pub struct PageGrant {
    pub table_id: String,
    #[serde(default)] pub read: Vec<String>,   // field names the page may receive
    #[serde(default)] pub write: Vec<String>,  // field names the page may set
    #[serde(default)] pub create: bool,
    #[serde(default)] pub delete: bool,
}
```

A grant is the whole of what a custom page can do. There is no wildcard. A page with no grant for a table cannot see that table.

Write `governance/schema/app-manifest.schema.json`, JSON Schema 2020-12, by hand from `AppManifest`, `AppTable`, `AppView`, `FieldSpec`, `FieldType`, `AppPage`, `PageKind`, and `PageGrant`. Serve it as the resource `scaffoldry://schema/app-manifest`. `scaffoldry://framework/component-spec` stays.

Add `validate_manifest(m: &AppManifest) -> Result<(), Vec<String>>` in `scaffoldry-engine`. It returns every problem, not the first.

| Rule | Limit |
| --- | --- |
| `slug` | `^[a-z0-9][a-z0-9-]{1,62}$` |
| Tables per app | 50 |
| Fields per table | 200 |
| Views per app | 200 |
| Field names in one table | Unique. `^[a-z][a-z0-9_]{0,62}$` |
| A formula field | `formula_expression` parses. Every `{field}` it names exists in the same table |
| A lookup, count, or rollup field | Its relationship and target field exist |
| A view | Its `table_id` exists. Every field it lists exists |
| Pages per app | 50. Page `id` is `^[a-z][a-z0-9-]{0,62}$` and unique |
| A `View` page | Its `view_id` exists |
| A `Custom` page | `source_sha256` is 64 hex characters. Every grant names a table that exists. Every field in `read` and `write` exists in that table. A computed field is not in `write`. `write` is a subset of `read` |
| Serialized manifest | 256 KB. Page source is stored apart from the manifest and does not count |
| `custom_domain_verified` | Must equal the live value. A proposal can never set it to true. Nothing in this plan verifies a domain, so nothing sets it |

Every path that stores a manifest calls `validate_manifest`. That is `create_app_in_workspace`, `update_app`, and phase 4.

Tests:

1. `governance/scripts/validate-oscal.py` already loads `jsonschema` in CI. Add a sibling script `validate-manifest.py` that checks every manifest fixture under `crates/scaffoldry-engine/tests/fixtures/manifests/` against the schema. Add it to `ci.yml` as a hard gate.
2. A Rust test deserializes the same fixtures and calls `validate_manifest`. Valid fixtures pass in both. Each invalid fixture fails in both.
3. A manifest with a formula naming a missing field returns that field's name in the error list.
4. A custom page whose grant writes a formula field is refused. A grant naming a field that does not exist is refused.

## Phase 4 — proposals

Needs `docs/plans/live-data.md` phase 2 for `app_manifests.version`.

Migration `crates/scaffoldry-core/migrations/0011_app_proposals.sql`:

```sql
CREATE TABLE IF NOT EXISTS app_proposals (
    id UUID PRIMARY KEY,
    workspace_id VARCHAR(64) NOT NULL REFERENCES workspaces(id),
    app_slug VARCHAR(64) NOT NULL,
    base_version INTEGER,
    manifest JSONB NOT NULL,
    checks JSONB NOT NULL,
    summary TEXT NOT NULL,
    status VARCHAR(16) NOT NULL DEFAULT 'Pending',
    proposed_by VARCHAR(255) NOT NULL,
    decided_by VARCHAR(255),
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    decided_at TIMESTAMPTZ
);
CREATE INDEX IF NOT EXISTS idx_app_proposals_workspace ON app_proposals(workspace_id, status);

CREATE TABLE IF NOT EXISTS app_page_sources (
    sha256 CHAR(64) PRIMARY KEY,
    source TEXT NOT NULL,
    created_by VARCHAR(255) NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
```

Page source is stored by its SHA-256. A row is never updated. A changed page is a new row and a new hash in the manifest. The approved manifest therefore names the exact bytes that will run.

`base_version` is null for a new app. `status` is `Pending`, `Approved`, `Rejected`, or `Superseded`.

| Tool | Scope | Behavior |
| --- | --- | --- |
| `propose_app_change` | `Workspace("access_workspace")`, and the caller's role is `owner`, `admin`, or `editor` | Arguments `workspace_id`, `manifest`, `summary`, and `page_sources`, a list of strings. Stores each source under its hash. Validates. Runs the checks. Stores a row. Changes no live app. Returns the proposal id, the checks, and the change list |
| `list_proposals` | `Workspace("access_workspace")` | Pending first |
| `get_proposal` | `Workspace("access_workspace")` | The row, the checks, and the change list |
| `decide_proposal` | `Workspace("manage_workspace")` | Arguments `proposal_id`, `decision` of `approve` or `reject`, `rationale` |

Checks are computed. They are never constants.

| Check | Fails when |
| --- | --- |
| `schema` | `validate_manifest` returns an error. A failed schema check stores no row and returns the errors |
| `sensitivity` | A field is `ferpa_sensitive`, or its CEDS code is sensitive under `CedsElement::is_ferpa_sensitive`, and the workspace `data_classification` is `Internal` or `Public` |
| `data_loss` | A field or table that exists in the live manifest is absent from the proposal and records exist for that app |
| `stale` | `base_version` is not the live version |
| `page_source` | A `Custom` page names a hash with no row in `app_page_sources`, or a source is over 256 KB, or a source contains the text `</script` in any letter case. A failed `page_source` check stores no proposal row and returns the errors |
| `page_sensitivity` | A custom page is new or changed, and a grant's `read` list holds a `ferpa_sensitive` field |

The change list compares the proposal to the live manifest: tables added and removed, fields added, removed, and retyped, views added and removed, pages added, removed, and changed. It is a list of plain sentences.

For each new or changed custom page, the change list states its grants in words a department chair can check. Example: `Page "Award dashboard" reads Grants: award number, amount, status. It may change: status. It may not create or delete records.` The reviewer approves that sentence. The reviewer is not asked to read JavaScript.

`decide_proposal` with `approve`, in one database transaction on one connection:

1. Refuse when `status` is not `Pending`. 409.
2. Refuse when the live version is not `base_version`. Mark the row `Superseded`. 409.
3. Refuse when `sensitivity`, `data_loss`, `page_sensitivity`, or `rule_effects` failed and `decided_by` equals `proposed_by`. A second person approves a flagged change.
4. Append the ledger entry, `DecisionType::AppPublished`, with the proposal id and the SHA-256 of the manifest in the payload.
5. Write the manifest with `version + 1`.
6. Set `status`, `decided_by`, `decided_at`.

If any step fails, none is kept. After commit, reload the manifest into the engine.

A proposal with no failed check may be approved by the person who proposed it, when that person is an `owner` or `admin`. This keeps a lab director's own small change to one step.

`PUT /apps/{slug}`, `POST /workspaces/{id}/apps`, and `POST /apps/{slug}/publish` are rewritten in phase 4a. Until then they gain only the `validate_manifest` call from phase 3.

Tests:

1. `propose_app_change` on an existing app leaves `describe_app` unchanged.
2. Approving writes the manifest, raises the version by one, and adds one ledger entry whose payload holds the proposal id.
3. Two proposals on the same base version: the second approval is 409 and its row is `Superseded`.
4. A proposal adding a `ferpa_sensitive` field to an `Internal` workspace fails `sensitivity`. The proposer's own approval is refused. A different admin's approval succeeds.
5. Make the ledger insert fail inside the test. The manifest version is unchanged and the proposal is still `Pending`.
6. An editor can propose and cannot decide.
7. A proposal whose custom page names a hash that was not sent in `page_sources` and is not stored returns the `page_source` error and stores no row.
8. A page granted `read` on a `ferpa_sensitive` field fails `page_sensitivity`. The change list holds a sentence naming that field. The proposer's own approval is refused.
9. Changing one character of a page source changes its hash, and the change list reports the page as changed.

## Phase 4a — process definitions and app changes are proposals

Needs phase 4, `approvers.md` phase 3, and `ux-standards.md` phase 1.

A rule that sets `status` to `Approved` changes real decisions. An agent can write one as easily as it writes a manifest. So a rule is governed the same way: it is proposed, checked, and approved, and the approval is a ledger entry. The same goes for every direct route that changes an app today. After this phase, nothing changes a live app or a live rule except an approved proposal, or an administrator's enable or disable from `admin-console.md` phase 6.

Migration `crates/scaffoldry-core/migrations/0019_rule_versions.sql`:

```sql
CREATE TABLE IF NOT EXISTS workflow_rule_versions (
    rule_id VARCHAR(64) NOT NULL,
    version INTEGER NOT NULL,
    app_slug VARCHAR(64) NOT NULL,
    rule_json JSONB NOT NULL,
    summary TEXT NOT NULL,
    proposal_id UUID,
    changed_by VARCHAR(255) NOT NULL,
    changed_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (rule_id, version)
);
CREATE INDEX IF NOT EXISTS idx_rule_versions_app ON workflow_rule_versions(app_slug);
```

Migration `0024_proposal_rules.sql`:

```sql
ALTER TABLE app_proposals ALTER COLUMN manifest DROP NOT NULL;
ALTER TABLE app_proposals ADD COLUMN IF NOT EXISTS rules JSONB;
```

`rules` is a list of `{ "rule": <AutomationRule>, "base_version": <number> }`. `base_version` is the rule's live version when proposed, and 0 for a new rule. A proposal holds a manifest, rules, or both.

Model, in `scaffoldry-core/src/workflow.rs`. New fields use `#[serde(default)]`.

- `AutomationRule.version: u32`, assigned by the server.
- `ProcessInstance.rule_version: u32`, set from the live rule when the instance is created.
- `describe_rule(rule: &AutomationRule) -> Vec<String>`: plain sentences. Example: `When a record is created in this app.` `If gpa is greater than 3.85.` `Then set status to Approved.` `Then wait for a decision by the chair of the submitter's unit. If that position is vacant, ask the next unit up. If approved: set status to Admitted. If rejected: set status to Denied.` A webhook step is refused, so it has no sentence. A pure function with its own tests.

`save_rule(caller, slug, rule, proposal_id, state)` in a new `service/processes.rs`. It validates, appends a ledger entry `ProcessDefinitionChanged` (`CM-03`) holding the rule id, the new version, the SHA-256 of `rule_json`, and the proposal id, then writes the version row and the live row in one repository transaction. Validation: step ids unique, every `UserTask` passes `validate_approver`, every field named in a `when` or `SetFields` exists in the app's manifest, no `SetFields` names a computed field, and no step is a webhook. It returns every problem, not the first. It is called from proposal approval and from `admin-console.md` phase 6. Nothing else calls it.

`propose_app_change` takes `app_slug` (required when there is no manifest), an optional `manifest`, and optional `rules`. At least one of the two is required.

Checks. Each check has an outcome: `pass`, `warn`, or `fail`. `warn` never blocks. `fail` on `schema` or `rule_schema` stores no proposal and returns the errors. The flagged checks, which need a second person to approve, are `sensitivity`, `data_loss`, `page_sensitivity`, and `rule_effects`.

| Check | Outcome |
| --- | --- |
| `rule_schema` | `fail` when `save_rule`'s validation would fail, including a webhook step |
| `rule_effects` | `fail`-flagged when a `SetFields` effect names a field that is effectively sensitive (`admin-console.md` phase 4, or the manifest flag before that phase) |
| `rule_loop` | `warn` when the trigger is `RecordUpdated` and the rule sets a field its own `when` reads |
| `rule_approver` | `warn` when a user task resolves to no approver today, or only to the proposer. Resolve with `resolve_approvers` for the workspace's unit |
| `stale` | Each rule's `base_version` against its live version |

The change list covers rules with `describe_rule`: rules added, rules changed (the sentences of the new version), and rules enabled or disabled.

Approval. `decide_proposal` runs as in phase 4. For rules it also, in the same transaction, appends one `ProcessDefinitionChanged` entry per rule and calls `save_rule` for each. After commit, reload `ServerState.automations` from the new live rows. A process instance already waiting keeps the `rule_version` it started under. `decide` reads its step from `workflow_rule_versions` for that version. An instance with `rule_version` 0 reads the live rule.

The direct routes become wrappers. Each creates a proposal with the same checks. It then approves it in the same request when all of these hold: the caller is an `owner` or `admin` of the workspace, no check is `fail`, and no flagged check is raised. In that case the route responds as before (`201` or `200`, with the rule or manifest), so existing screens keep working. When a flagged check is raised, it responds `202` with `{ "proposal_id", "status": "Pending", "checks" }` and changes nothing live. A `fail` is `400` with the errors.

| Route | Now |
| --- | --- |
| `POST /api/v1/apps/{slug}/automations` | A rules proposal for one rule |
| `PUT /api/v1/apps/{slug}` | A manifest proposal |
| `POST /api/v1/workspaces/{id}/apps` | A manifest proposal with no base version |
| `POST /api/v1/apps/{slug}/publish` | A manifest proposal that sets `custom_domain` and cannot set `custom_domain_verified` |

Frontend, from the kit. Create `apps/web/src/Proposals.tsx`, a `Proposals` tab in the workspace for `owner` and `admin`.

- A table, pending first: summary, who proposed it, when, and the checks as `StatusBadge`s.
- A row opens a drawer: the summary, the change list in sentences (including each custom page's grants), and each check with its outcome and detail.
- `Approve` and `Reject` are `ConfirmAction`s with a required reason. When a flagged check is raised and the viewer is the proposer, `Approve` is disabled and the drawer says `A second person must approve this change.`
- `WorkflowBuilder.tsx` and the app builder treat a `202` as success of a different kind: a `Banner` that says `Submitted for approval.` with a `View proposal` link. They do not show the change as live.
- The admin Overview gains a Pending proposals count.

Tests.

1. A rules-only proposal leaves the live rules unchanged. Approving it makes version 1 live, and the version row holds the proposal id.
2. A rule that sets a sensitive field raises `rule_effects`. The proposer cannot approve it. Another admin can.
3. `POST /apps/{slug}/automations` by an owner with a clean rule is 201, creates an approved proposal row, and writes the version. By an owner with a flagged rule it is 202 and the rule is not live.
4. A rule with a webhook step is 400 with the step id and stores no proposal.
5. A rule naming a field that does not exist is 400 and names the field.
6. Two proposals for one rule on the same base version: the second approval is 409 and its row is `Superseded`.
7. An instance waits on version 1. Approve a version 2 that changes the approve actions. Decide the instance. The version 1 actions are applied.
8. `PUT /apps/{slug}` by an editor is 403. By an owner with a clean change it is 200 and writes one proposal row and one ledger entry.
9. A proposal that sets `custom_domain_verified` to true is refused.
10. `describe_rule` on a rule with a condition, a service step, and a user task returns the expected sentences. Unit test in `scaffoldry-core`.
11. A user task that resolves to no approver saves with a `warn`, and the drawer shows it in words.
12. Web: the drawer shows the change list and disables `Approve` for the proposer on a flagged proposal. A `202` from the builder shows the banner and not a live rule.

## Phase 5 — process tools

Needs `approvers.md` phase 3.

The business process desk becomes reachable from an agent.

| Tool | Scope | Behavior |
| --- | --- | --- |
| `list_waiting_decisions` | `SignedIn` | Waiting process instances in apps the caller can read, where `can_decide` is true for the caller. Up to 200 |
| `decide_process` | `App(Read)` plus `can_decide` from `approvers.md` | The body of `decide_app_process`, moved to `service/processes.rs`. The REST route calls it |

Tests: the foundation phase 6 process tests, run again through `tools/call`.

## Phase 6 — a custom page is rendered and bounded

A page is one JSX module with a default export, a React component. It is stored as written. It is compiled in the sandbox when it opens.

```jsx
import { useEffect, useState } from "react";
import { records } from "scaffoldry";

export default function Page() { /* ... */ }
```

A page may import `react`, `react/jsx-runtime`, and `scaffoldry`. Any other import fails with an error panel inside the frame.

Vendored runtime: add `react`, `react-dom`, and `sucrase` (all MIT) to `apps/web` `devDependencies`. Add an npm script `vendor:runtime` that copies the production builds of React and ReactDOM, and the browser build of Sucrase, to `crates/scaffoldry-server/src/ui/vendor/`. Commit the copied files. The server embeds them with `include_str!`. No code in `apps/web/src` imports these three packages for this purpose. They are the only dependencies this plan adds. Record each file's size and version in the session output.

Compile in the sandbox. A static file `crates/scaffoldry-server/src/ui/loader.js`, under 80 lines, runs in the frame:

1. Read the page source from the JSON block in the shell.
2. Compile with Sucrase using `transforms: ["jsx", "imports"]` and `jsxRuntime: "automatic"`. If the automatic runtime does not work with the vendored files, use the classic runtime, put `React` in scope, and say so in the session output.
3. Run the result by creating a `<script>` element whose text is the compiled code and appending it. Never call `eval` or `new Function`. The policy forbids both.
4. Give the compiled code a `require` that returns the vendored React, the JSX runtime, and the bridge for the three allowed names, and throws for any other name.
5. Mount the default export with `ReactDOM.createRoot`.
6. On any error, render the message in the frame. Do not leave a blank frame.

Bridge client: a hand-written file `crates/scaffoldry-server/src/ui/bridge.js`, under 150 lines. It defines the module `scaffoldry`, also reachable as the global of the same name:

```js
scaffoldry.app          // { slug, page_id, title }
scaffoldry.grants       // the page's grants, as stored
scaffoldry.records.list({ table_id, filter, sort, limit, cursor })
scaffoldry.records.get({ table_id, record_id })
scaffoldry.records.create({ table_id, data })
scaffoldry.records.update({ table_id, record_id, version, data })
scaffoldry.records.remove({ table_id, record_id, version })
```

Each method posts one JSON-RPC 2.0 request to `window.parent` with method `tools/call`, params `{ "name": "page_call", "arguments": { ... } }`, and resolves with the matching response. It uses no `fetch`, no `XMLHttpRequest`, and no storage.

Shell: a static file `crates/scaffoldry-server/src/ui/page-shell.html`. `render_page(caller, slug, page_id, state) -> Result<String, ServiceError>` in a new file `service/pages.rs` fills it by plain string replacement of markers: React, ReactDOM, Sucrase, the bridge, the loader, a JSON object with `app` and `grants`, and the page source as a JSON string in `<script type="application/json" id="page-source">`. Replace `<` with `\u003c` inside that JSON. The result holds no record value, no eppn, and no token.

The shell's first element in `<head>`:

```html
<meta http-equiv="Content-Security-Policy"
      content="default-src 'none'; script-src 'unsafe-inline'; style-src 'unsafe-inline'; img-src data: blob:; font-src data:; connect-src 'none'; form-action 'none'; base-uri 'none'">
```

`render_page` is never served as an HTML response. It is returned as a string inside JSON. A browser cannot be sent to it as a document.

| Tool | Scope | Behavior |
| --- | --- | --- |
| `list_pages` | `App(Read)` | id, title, kind, and grants for each page |
| `get_page` | `App(Read)` | `{ "html": render_page(...) }` for a `Custom` page. `BadRequest` for a `View` page |
| `page_call` | `App(Read)` | Arguments `app_slug`, `page_id`, `op` of `list`, `get`, `create`, `update`, or `remove`, and that operation's arguments |

`page_call` is the limit on a page. It does these in order and stops at the first refusal:

1. Load the page from the live manifest. It must be `Custom`.
2. Find the grant for `table_id`. None is 403.
3. `create` needs `create: true`. `remove` needs `delete: true`. `update` and `create` refuse any key in `data` that is not in `write`.
4. Call the same service function the data tools call. The caller's own rights still apply. A grant never widens them.
5. Remove from every returned row each field that is not in `read`.

REST: `GET /api/v1/apps/{slug}/pages/{id}` returns the `get_page` JSON. `POST /api/v1/apps/{slug}/pages/{id}/call` runs `page_call`.

Tests:

1. `get_page` returns a document that contains the stored source and the CSP meta element, and does not contain the caller's eppn. The CSP does not contain `unsafe-eval`.
2. `page_call` `list` on a table with no grant is 403.
3. A page granted `read: ["title"]` receives rows with `id`, `version`, and `title` only.
4. `page_call` `update` setting a field outside `write` is refused and the row is unchanged.
5. A workspace viewer calling `page_call` `update` through a page that grants `write` is refused. The grant did not widen the viewer's rights.
6. `bridge.js` and `loader.js` do not contain `fetch`, `XMLHttpRequest`, `localStorage`, `cookie`, `eval`, or `new Function`.
7. In a jsdom test, the loader compiles a page that imports `react` and `scaffoldry` and mounts it. A page importing `lodash` renders the error panel.

ponytail: syntax errors in a page are found when it opens, not when it is proposed, because the server has no JavaScript parser. An agent should open its page in its own host before proposing. Add a check when a reviewer approves a page that does not compile.

## Phase 7 — the web desk hosts a page

One new component, `apps/web/src/PageFrame.tsx`. It takes `appSlug` and `pageId`.

1. It loads the page with `apiClient` from `GET /apps/{slug}/pages/{id}`.
2. It renders `<iframe sandbox="allow-scripts" srcDoc={html} data-testid="page-frame" />`. The `sandbox` value is exactly `allow-scripts`. Never add `allow-same-origin`, `allow-top-navigation`, `allow-popups`, or `allow-forms`.
3. It listens for `message` events. It ignores any event whose `source` is not this frame's `contentWindow`.
4. It accepts one request shape: JSON-RPC `tools/call` with `name` equal to `page_call`. Anything else gets a JSON-RPC error and is not forwarded.
5. It overwrites `app_slug` and `page_id` in the arguments with its own props, then posts to `/apps/{slug}/pages/{id}/call` through `apiClient`, and posts the response back to the frame.

The page never receives the token. `PageFrame` holds it, as every other desk component does.

The app's navigation lists the manifest's pages. A `View` page opens the existing view. A `Custom` page opens `PageFrame`.

A custom page cannot approve a proposal or decide a process step. Those buttons are drawn by the desk, outside the frame.

Tests in `apps/web/src/test/page-frame.test.tsx`, with `fetch` stubbed:

1. The iframe's `sandbox` attribute equals `allow-scripts`.
2. A `tools/call` for `page_call` from the frame's window results in one POST to the page call route.
3. A `tools/call` naming `update_workspace` results in no `fetch` call.
4. A message from a window that is not the frame results in no `fetch` call.
5. A message whose arguments name another `app_slug` is forwarded with this frame's `appSlug`.

## Phase 8 — pages and review screens inside the agent

Do not start this phase until Johann pastes the current MCP Apps specification text into the session. Do not guess a message name or a metadata key.

Custom pages: `resources/list` adds one `ui://scaffoldry/apps/{slug}/pages/{id}` entry per custom page the caller can read. `resources/read` returns `render_page` with `text/html;profile=mcp-app`. `get_page` links to that resource in the way the specification states. Add to `bridge.js` whatever handshake the specification requires of a page before it may send `tools/call`. Change nothing else in the bridge.

Two built-in screens. Each is one static HTML file under `crates/scaffoldry-server/src/ui/`, embedded with `include_str!`. The file contains no data. It receives the tool result from the host and renders it. Do not build HTML with `format!` from record data.

| Resource | Linked from | Shows |
| --- | --- | --- |
| `ui://scaffoldry/proposal-review` | `get_proposal` | Summary, change list with page grants, checks, Approve and Reject buttons that call `decide_proposal` |
| `ui://scaffoldry/decision` | `list_waiting_decisions` | Prompt, record fields, Approve and Reject buttons that call `decide_process` |

The two existing screens in `service/governance.rs` stay as they are.

Tests:

1. `resources/read` for each built-in URI returns the file with `text/html;profile=mcp-app`. The file contains no `{{`, no eppn, and no record value.
2. `resources/read` for a custom page URI returns the same string as `get_page`.
3. A caller with no role in the workspace gets an error for that page's URI, and the URI is absent from their `resources/list`.

## How to prompt Gemini

```
Read docs/plans/mcp-apps.md, docs/plans/foundation.md "Decisions already made", and docs/plans/README.md.
Implement phase N only.
Write the failing test first and run it.
Do not add a dependency.
Do not authorize inside a tool handler. The dispatcher does it.
Do not return a constant where the plan says a check is computed.
Do not change a live app from propose_app_change.
Do not add a value to an iframe sandbox attribute. Do not loosen the page Content Security Policy.
Do not let a page call any tool but page_call.
Stop when the tests listed for phase N pass, and paste the command output.
```

## Enterprise objections

| Objection | Answer |
| --- | --- |
| An agent will be talked into doing something its user may not do. | The agent holds one token its user minted. The token expires, can be revoked, and cannot mint another. Every tool call passes the same gate as a REST call. Phase 1 test 2 iterates the registry, so a new tool without a scope fails the build. |
| An agent will publish a FERPA field into an open workspace. | It cannot publish. It proposes. The `sensitivity` check is computed from the field flags and CEDS codes, and a flagged proposal needs a second person. |
| Record content will carry instructions to the agent. | It can. Scaffoldry cannot stop a model from reading text. It limits the damage: the agent acts with one user's rights, structural change needs approval, and every decision is in the ledger with the principal. |
| An agent in a loop will exhaust the server. | Every list is capped at 200 rows and 256 KB. Request bodies are capped at 2 MB. Per-caller rate limiting is not built. Put it at the reverse proxy until a measurement asks for it in the server. |
| Two agents will overwrite each other's app. | A proposal names its base version. The second approval is refused and marked `Superseded`. |
| Agent-written JavaScript will run in a dean's browser. | It runs in a frame with `sandbox="allow-scripts"` and a policy of `connect-src 'none'`. It has no origin, no cookie, no storage, no token, and no network. It can ask its host for records its grants name. It can do nothing else. |
| Nobody will review the code. | Nobody is asked to. The reviewer approves the grants, written as a sentence. The code cannot exceed them, because `page_call` enforces them on the server. |
| A page will send student records to another site. | It has no network. It cannot navigate the top window, open a window, or submit a form. A page that reads a FERPA field needs a second approver before it runs at all. |
| A page will show an Approve button that does something else. | A page cannot approve anything. Proposal and process decisions are tools a page may not call, and the desk draws those buttons outside the frame. A page can still mislabel its own record edits. Those edits are within its grants and are attributed to the user. |
| Inside a desktop agent, the host may let a page call any tool. | Then the page is bounded by the user's own rights, not by its grants, and still has no network. The grants are enforced when the call arrives as `page_call`, which is the only call the web desk forwards. |
| A page will break when the platform changes. | The bridge is five methods. They are a published contract. A change to them is a new method, never a changed one. |
| The approved code is not the code that runs. | The manifest names the SHA-256 of the source. Source rows are never updated. The ledger entry for the approval holds the manifest hash. |
| An agent will write a rule that approves everything. | A rule is a proposal like a manifest. It is checked, a rule that sets a sensitive field needs a second person, and every approval is a ledger entry holding the rule's hash. |
| Proposals will slow every small edit. | An owner or admin with a clean change is approved in the same request. The proposal is the audit trail, not a queue. |
| Hand-written JSON-RPC will drift from the protocol. | The surface is six methods. The protocol version is negotiated in `initialize`. Adopt the official SDK when a seventh method needs streaming. |
| The approval and the audit entry can disagree. | They are one transaction. Phase 4 test 5 fails the ledger insert and asserts that nothing changed. |
