# Workspace guards — implementation brief

Hand this file to Gemini Flash. Do one phase per session. Stop when that phase's tests pass. Do not start the next phase.

Run order is `docs/plans/README.md`. Phase 1 needs `foundation.md` phases 5 and 6. Phases 2 and 3 need `admin-console.md` phase 1 and `ux-standards.md` phase 1.

A workspace owner must be able to say, "Only faculty and staff may export from this workspace." The platform must then enforce that sentence on every path, and show the owner who it affects before it takes effect.

Today the workspace has a field, `cedar_policy_guard`, that holds text. Nothing evaluates it. Seed data holds `permit` rules there that would widen access if anyone ever did evaluate them. An unevaluated rule is worse than no rule, because everyone who reads it believes it works. This brief removes the field and replaces it with guards that are evaluated, bounded, explained, and tested.

## Standard

| Standard | Use |
| --- | --- |
| Cedar policy language, with a Cedar schema | The policy language and its type check. The `cedar-policy` crate is already a dependency |
| NIST SP 800-53 AC-3 (access enforcement) and AC-6 (least privilege) | What a guard is for. Guards can only remove access |

## The design

A guard is a `forbid` rule that applies to one workspace. It can only take access away. It can never grant access. Cedar already works this way: one matching `forbid` overrides every `permit`. So the platform's institutional policy decides what is allowed, and a workspace can only narrow that.

Owners do not write Cedar. They choose from a short list of templates and fill in the parameters. The platform turns each into a `forbid` rule. A Platform Admin may edit raw Cedar.

## What exists today

| Fact | Where |
| --- | --- |
| `WorkspaceRecord.cedar_policy_guard` is stored and returned. No code evaluates it | `state.rs`, `repository.rs`, `service/workspaces.rs` |
| The MCP tool `update_workspace` accepts `cedar_policy_guard`, so an agent can write it | `routes/mcp.rs` |
| `AutomationRule.cedar_policy_guard` is a policy name such as `policy-ferpa-34cfr99`. The engine only checks that it is set and whether it contains `ferpa` | `workflow.rs`, `automation.rs` |
| Four functions build Cedar entity JSON by hand, each differently | `scaffoldry-policy/src/lib.rs` |
| One of them guesses a principal's department from the eppn text | `authorize_record_action` |
| A missing attribute, such as `member_role` for a non-member, becomes a Cedar evaluation error that reads as a denial | `authorize_workspace_action` |
| Policies are not validated against a schema | `ScaffoldryPolicyEngine::new` |

## Out of scope

- A guard that grants access. There is no such thing.
- Guards on a single app or table. A guard covers a workspace.
- A guard that reads record field values. Guards use the sensitivity flag, which already exists.
- An agent that writes guards. Guards change access, so they are not MCP tools. `get_workspace` returns them as sentences, read-only.
- Time windows, IP ranges, or device rules.

## Rules for every phase

1. Write the failing test first. Run it. Confirm it fails because the behavior is missing. Then write the code.
2. Do not add a dependency.
3. Every access decision goes through one function, `service::access::decide`. No route, service, or engine calls a `policy_engine.authorize_*` method directly after phase 2.
4. A guard can never contain a `permit`. A test proves it.
5. A change to a guard is a ledger entry written before the change.

## Phase 1 — one schema, one entity builder

This phase changes no decision. It makes every later decision typed and checkable.

1. Add `crates/scaffoldry-policy/schema/scaffoldry.cedarschema`, embedded with `include_str!`. Use the Cedar human-readable schema format and the schema and `Validator` types of `cedar-policy` 4.13. Use the real function names from that crate's documentation.

   | Entity type | Attributes |
   | --- | --- |
   | `User` | `eppn`, `name`, `scoped_affiliation`, `department`: String. `unit_ids`: Set of String. `is_platform_admin`: Bool |
   | `Workspace` | `workspace_id`, `department`, `visibility`, `data_classification`, `member_role`, `unit_id`: String. `is_member`: Bool |
   | `App` | `slug`, `department`, `workspace_id`: String |
   | `Record` | `app_slug`, `department`, `workspace_id`: String. `is_ferpa_sensitive`: Bool |
   | `System` | `department`: String. `is_ferpa_sensitive`: Bool |

   Every attribute is required. A non-member has `member_role` set to the empty string. `workspace_id` on `Workspace` equals its id.

   | Action | Applies to |
   | --- | --- |
   | `access_workspace`, `manage_workspace`, `delete_workspace` | `Workspace` |
   | `read_app`, `manage_app`, `create_app`, `update_app`, `publish_app` | `App` |
   | `read`, `write`, `write_record`, `export`, `approve`, `run_automation` | `Record` |
   | `access_admin`, `impersonate`, `record_decision`, `publish_dataset` | `System` |

   The existing policies use `Action::"read"` and friends. Adjust the schema's action list to cover every action name the 21 default policies and the code use. Run `grep -rn 'Action::"' crates/` and paste the list.

2. `crates/scaffoldry-policy/src/entities.rs`: typed structs `PrincipalCtx`, `WorkspaceCtx`, `AppCtx`, `RecordCtx`, `SystemCtx`, each with a method that returns the Cedar entity. Add one `authorize(principal, action, resource) -> AuthorizationResult` that builds the entities and evaluates. Re-implement the four existing `authorize_*` methods as thin calls to it.

   `PrincipalCtx.department` comes from the signed-in user's stored department. Delete the code that guesses it from the eppn text. `unit_ids` is every unit where the person holds a role, plus all ancestors of each, capped at 32 steps. Build it with a pure function over the org list and role rows.

3. The default policy set is validated against the schema when the engine is built. A policy that does not type-check fails startup in a test, and fails a draft in `admin-console.md` phase 5. Make the smallest edits to the default policy text that the schema requires, and list each edit in the session output.

4. Add `@description("...")` to each of the 21 default policies: one plain sentence a student could read, such as `Students cannot publish datasets.` `policy_summaries()`, from `admin-console.md` phase 5, returns it.

5. Decisions with a reason. `AuthorizationResult` gains `deciding_policy: Option<PolicyRef { id, description }>`. For a denial it is the first matching `forbid`. For an implicit deny with no `forbid`, it is `None`.

Golden test, written first. Before changing any code, write `crates/scaffoldry-policy/tests/decision_matrix.rs` and generate `tests/fixtures/decision_matrix.json` from the current four functions over this grid: affiliations `faculty`, `student`, `staff`, `compliance`, `central_admin`, `member`; departments equal and different; visibility `restricted`, `departmental`, `institutional`; member and non-member; member roles `owner`, `admin`, `editor`, `viewer`; the sensitive and non-sensitive flag; every action each function supports. Commit the fixture. After the refactor, the same grid through the new `authorize` must match the fixture exactly. List any row that differs and why. A difference caused by the deleted department guess is acceptable only if the fixture row used a department that the guess would have got wrong.

Tests.

1. The golden matrix matches.
2. The default policy set validates against the schema. A policy referencing a missing attribute does not.
3. A non-member has `member_role` equal to the empty string, and policy 13 now evaluates without error.
4. `unit_ids` for a person in a department two levels below the root holds the department, the college, and the root.
5. Every default policy has a non-empty `@description`.
6. A denial from a `forbid` returns that policy's id and description.
7. `grep -rn "contains(\"physics\")" crates/` prints nothing.

## Phase 2 — guards are evaluated

Migration `crates/scaffoldry-core/migrations/0023_workspace_guards.sql`:

```sql
CREATE TABLE IF NOT EXISTS workspace_guards (
    workspace_id VARCHAR(64) NOT NULL REFERENCES workspaces(id),
    version INTEGER NOT NULL,
    rules JSONB NOT NULL,
    compiled TEXT NOT NULL,
    reason TEXT NOT NULL,
    created_by VARCHAR(255) NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (workspace_id, version)
);

CREATE TABLE IF NOT EXISTS workspace_guards_legacy (
    workspace_id VARCHAR(64) PRIMARY KEY,
    cedar_policy_guard TEXT NOT NULL,
    archived_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
INSERT INTO workspace_guards_legacy (workspace_id, cedar_policy_guard)
    SELECT id, cedar_policy_guard FROM workspaces
    WHERE cedar_policy_guard IS NOT NULL AND cedar_policy_guard <> ''
ON CONFLICT DO NOTHING;
ALTER TABLE workspaces DROP COLUMN IF EXISTS cedar_policy_guard;
```

The legacy table is an archive for a database administrator. No route and no screen reads it. Boot writes one ledger entry, `PolicyRevision`, holding the number of archived rows, the first time the archive is non-empty. Remove `cedar_policy_guard` from `WorkspaceRecord`, `UpdateWorkspacePayload`, the repository SQL, the MCP `update_workspace` schema, the web types, and the seed data.

Remove `cedar_policy_guard` from `AutomationRule`. Rules no longer carry a policy name. Delete the substring check in `automation.rs`. A rule's effects are authorized with the Cedar action `run_automation` and the triggering user as principal, through `decide`. Remove the `Cedar Gated` label in `WorkflowBuilder.tsx`.

Templates. A closed list in `scaffoldry-core`, stored as JSON with a schema (`governance/schema/workspace-guard.schema.json`), validated on save:

| `template` | Parameters | Plain sentence | Compiles to a `forbid` on |
| --- | --- | --- | --- |
| `deny_export_unless_affiliation` | `affiliations` | `Only {list} may export from this workspace.` | `export` when the principal's affiliation is not in the list |
| `deny_write_for_affiliation` | `affiliations` | `{list} may not change records in this workspace.` | `write`, `write_record` when it is in the list |
| `deny_access_outside_units` | `unit_ids` | `Only people in {unit names} may use this workspace.` | `access_workspace`, `read_app`, `write_record`, `export`, `approve` when `principal.unit_ids` contains none of the units |
| `deny_sensitive_unless_affiliation` | `affiliations` | `Only {list} may see or export sensitive student data here.` | `read_app`, `export` when `resource.is_ferpa_sensitive` and the affiliation is not in the list |

Affiliations are the closed eduPerson list plus `compliance` and `central_admin`. A unit id must exist.

Each compiled rule has an explicit id, `guard-{workspace_id}-{n}`, a `@description` holding the plain sentence, and a condition `resource.workspace_id == "{workspace_id}"`. Give each policy its id with `Policy::parse(Some(id), text)`, because Cedar's default ids collide when several texts are parsed separately. The compiled text is stored in `compiled` so the auditor reads exactly what ran.

Raw Cedar. A Platform Admin may supply `source` instead of template rules. It must parse, type-check against the schema, hold only `forbid` policies, and be at most 8 KB. Anything else is 400 with each error and its line.

Limits: at most 20 rules.

Evaluation. Add `decide(state, caller, action, resource) -> Decision { allowed, policy }` in `service/access.rs`. It builds the policy set from the active institutional policy plus the compiled guards of the resource's workspace, parsed once and cached by `(workspace_id, version)`. Replace every direct `policy_engine.authorize_*` call with `decide`. Run `grep -rn "policy_engine\." crates/` and paste the output before and after. After, only `decide` and the policy admin routes appear.

Routes.

| Method | Path | Who | Behavior |
| --- | --- | --- | --- |
| GET | `/workspaces/{id}/guards` | Anyone who can read the workspace | Current rules, their sentences, version, and history |
| PUT | `/workspaces/{id}/guards` | Workspace `owner` or `admin`, an Org Unit Admin in scope, or a Platform Admin. Raw `source`: Platform Admin only | Body `rules` or `source`, and `reason`. Appends a ledger entry `PolicyRevision`, `AC-03`, holding the workspace id, the new version, and the SHA-256 of `compiled`. Then writes the new version |
| POST | `/workspaces/{id}/guards/impact` | Same | Body the proposed `rules`. Returns, for each current collaborator and each action in the set `access_workspace`, `read_app`, `write_record`, `export`, `approve`, whether the decision would change. Only the changes. Sensitive and non-sensitive resources are evaluated separately |
| POST | `/workspaces/{id}/guards/test` | Same | Body the proposed `rules`, a user, an action, and the sensitivity flag. Returns the decision now and under the proposal, each with the deciding policy |

`get_workspace` in MCP returns the guard sentences, read-only. There is no MCP tool that writes a guard.

Tests.

1. A workspace with `deny_export_unless_affiliation: [staff]`. A faculty member who could export now cannot. The response names the guard and carries its sentence. A staff member still can.
2. Every compiled policy parses and is a `forbid`. A template list that would compile a `permit` fails. A raw `source` containing `permit` is 400.
3. A guard on workspace A has no effect on a request in workspace B, including a request by the same person.
4. A guard cannot grant: with the institutional policy denying an action, adding any guard leaves it denied.
5. `deny_access_outside_units` with the college unit allows a person in a department under that college and denies a person in another college.
6. Rules over 20, or a unit that does not exist, are 400.
7. A ledger failure makes `PUT` fail and the version unchanged.
8. `impact` for a four-member workspace lists exactly the members and actions whose result changes, and nothing else.
9. `grep -rn cedar_policy_guard crates/ apps/web/src` prints nothing. Paste the output.
10. An automation whose triggering user is forbidden `run_automation` by a guard applies no effects.
11. The archive holds the old text, and the boot ledger entry holds the count.

## Phase 3 — the rules screen

Needs `ux-standards.md` phase 1. Web only, built from the kit.

In the workspace settings (`WorkspaceSettingsModal.tsx`), add a `Rules` tab, `data-testid="guard-rules"`. Move nothing else.

- A list of rule cards. Each shows the plain sentence, who added it and when, and `Remove`.
- `Add a rule` opens a drawer: a template choice described by its sentence, its parameter controls (affiliations as checkboxes with plain names, units from a searchable list showing names and codes), and a live preview of the sentence.
- Before `Save`, the drawer shows `Who this affects` from the impact route: `3 of 14 people would lose the ability to export.` It lists them by name. When no one is affected it says so.
- `Save` is a `ConfirmAction`: the consequence sentence, a required reason, the ledger note.
- `Test a person`: choose a person and an action, and see the decision now and under the draft, in words, with the deciding policy's sentence.
- A Platform Admin sees `Advanced`, a tab with the compiled Cedar, read-only, and a switch to edit raw source with the parse and type errors listed by line.
- A denial anywhere in the desk that came from a guard shows the guard's sentence, through `explainError`.

Tests (vitest, `fetch` stubbed).

1. Adding a rule shows the sentence preview, then the impact list, and `Save` stays disabled until a reason is typed.
2. `Save` sends `PUT /api/v1/workspaces/{id}/guards` with the rules and the reason.
3. When the impact response is empty, the drawer says `No one is affected.`
4. A faculty member who is not an owner does not see `Add a rule`.
5. A Platform Admin sees `Advanced` and a faculty owner does not.
6. A `403` carrying a guard policy shows that sentence in the toast.

## How to prompt Gemini

```
Read docs/plans/guards.md, docs/plans/ux-standards.md, and docs/plans/README.md.
Implement phase N only.
Write the failing test first and run it.
Do not add a dependency.
A guard can only forbid. Never compile a permit.
Do not call policy_engine.authorize_* outside service::access::decide after phase 2.
Do not leave a field that holds policy text that nothing evaluates.
Stop when the tests listed for phase N pass, and paste the command output.
```

## Enterprise objections

| Objection | Answer |
| --- | --- |
| A workspace owner will write a rule that opens data to everyone. | A guard compiles only to `forbid`. A test proves a guard cannot turn a denial into an allow. |
| An owner will lock the whole department out and not know it. | The screen shows who is affected, by name, before the rule is saved. |
| Owners cannot write Cedar. | They do not. They choose a sentence and fill in the blanks. The Cedar is generated and shown to the auditor. |
| A guard on one workspace will leak into another. | Every compiled rule is scoped to its workspace id twice: once by which guards are loaded, and once in the rule's own condition. Test 3 proves it. |
| Raw Cedar will crash the engine. | It must pass the schema check before it is stored. It is Platform Admin only and limited to 8 KB. |
| Removing the old field loses data. | The old text is archived in a table, and the migration records the count in the ledger. The old text was never enforced. |
| An agent will write a guard to lock others out. | There is no MCP tool that writes a guard. |
