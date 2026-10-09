# Agent coverage — implementation brief

Hand this file to Gemini Flash. Do one phase per session. Stop when that phase's tests pass. Do not start the next phase.

Run order is `docs/plans/README.md`. Phase 1 needs `mcp-apps.md` phase 5. Phase 2 needs `integrations.md` phase 2. Phase 3 runs alongside every other brief: each adds its own scenarios.

Scaffoldry is MCP-first. Thousands of faculty will ask an agent for thousands of different things, and the platform ships no model. Whether it works depends on whether a small, general set of tools can express what faculty ask for, and whether the tools say what went wrong in words an agent can act on. Testing each tool alone does not show that. A scripted set of real requests does.

This brief builds that suite, adds a budget that keeps the tool list small, and makes the platform teach the agent how to use it.

## Decisions already made

| Question | Answer |
| --- | --- |
| Do tests call a model? | No. A scenario is a recorded sequence of tool calls with expected results. It tests the server, not an agent's judgment |
| Few general tools or many specific ones? | Few and general, held by a size budget in a test |
| What do scenarios document? | Each starts with the faculty request in plain words. They double as the product's examples |
| What about requests the platform cannot do? | A scenario is marked unsupported with a reason. The suite lists them. That list is the backlog |

## What exists today

| Fact | Where |
| --- | --- |
| One test file per area of the MCP server | `tests/mcp_authoritative_api_test.rs` |
| A tool registry with one dispatch point | `mcp-apps.md` phase 1 |
| The model is outside the product | `foundation.md` |

## Out of scope

- Running a language model in CI.
- Measuring an agent's quality.
- Per-table generated tools.
- A natural language layer in the server.

## Rules for every phase

1. Write the failing test first. Run it. Confirm it fails because the behavior is missing. Then write the code.
2. Do not add a dependency.
3. A scenario passes against the real server and database. It does not stub a tool.
4. A scenario that is unsupported says why and is never deleted to make the suite pass.

## Phase 1 — the scenario runner

Files live in `crates/scaffoldry-server/tests/agent_scenarios/*.json`. Each is:

```json
{
  "id": "build-admissions-intake",
  "ask": "I need a place to collect scholarship applications and have my chair approve the ones over $5,000.",
  "persona": "faculty-owner",
  "status": "supported",
  "steps": [
    { "tool": "propose_app_change", "args": { "workspace_id": "$ws", "manifest": { ... } }, "save": { "proposal": "$.proposal_id" }, "expect": { "checks.schema": "pass" } },
    { "tool": "decide_proposal", "args": { "proposal_id": "$proposal", "decision": "approve", "rationale": "..." }, "expect": { "status": "Approved" } },
    { "tool": "create_record", "args": { "app_slug": "scholarships", "data": { ... } }, "expect": {} }
  ],
  "after": [ { "ledger_has": "AppPublished" }, { "no_error_leaks": true } ]
}
```

Substitutions use `$name` for saved values and `$.path` to pick from a result. `expect` compares named paths in the result. `error` expects a tool error with a given `code`. `status` is `supported` or `unsupported` with `unsupported_reason`.

The runner, `tests/agent_scenarios.rs`: boots the server against a test database with the demo seed, creates the personas (a faculty owner, an editor, a viewer, a student, a compliance officer, a Platform Admin, and a scoped agent token), runs each scenario's calls through the real MCP endpoint over HTTP as that persona's token, and checks the expectations and the `after` assertions (a ledger entry of a type exists, no response contains a secret or a stack trace, no field a persona may not see appears). It prints a table of scenarios with pass, fail, or unsupported.

The gap register. A run writes `target/agent-gaps.md`, a list of unsupported scenarios with their asks and reasons, and the `README` instructs that this file is read before planning. It is generated, never committed.

Twenty scenarios for what exists after `mcp-apps.md` phase 5, covering:

| Group | Count | Examples |
| --- | --- | --- |
| Build | 5 | Intake app from a description, a second table and a relation, a formula column, an approval process, a change to an existing app |
| Data | 5 | Enter records, update with a version, filter and page through 450 records, a total, a record that fails validation |
| Process | 3 | A step waits, a chair decides, a student is refused |
| Governance | 4 | A proposal needing a second person, a sensitive-field rule, a policy denial with its reason, a ledger check |
| Failure and recovery | 3 | A version conflict then a retry, a missing field fixed after an error hint, a long job polled |

Tests.

1. The runner executes the twenty scenarios and all pass.
2. A scenario whose expectation is changed to a wrong value fails with a diff of expected and actual.
3. A scenario marked unsupported appears in the gap register and does not fail the suite. One marked supported that fails does.
4. Two scenarios run in parallel without sharing state.
5. The `after` assertion `no_error_leaks` fails when a response contains `panicked` or a stack frame. Prove it with a fixture.
6. The runner uses the real HTTP endpoint. A test asserts it does not call service functions directly.

## Phase 2 — conformance, the budget, and hints

Conformance tests for the protocol, in `tests/mcp_conformance.rs`:

- `initialize` negotiates the version, and an unsupported version gets the highest one the server speaks.
- A notification gets `202` and no body.
- Unknown methods and invalid requests give the right JSON-RPC error codes.
- `tools/list` is valid for every entry: the `inputSchema` is a valid JSON Schema 2020-12 document (checked by a small structural validator in the test), has `additionalProperties: false`, and an example in the tool's metadata validates against it.
- Annotations are truthful. `readOnlyHint` is true exactly for tools with `read_only`. `destructiveHint` is true for tools that remove or revoke. `idempotentHint` is true for tools safe to repeat with the same arguments (updates with a version, deletes to trash). A table in the test lists every tool and the expected hints.
- A tool's description is at most 400 characters and starts with a verb.

The budget. A test serializes `tools/list` and fails if it exceeds 24 KB, or if the tool count exceeds 45. A brief that needs another tool must say what it replaces, or raise the budget in the same change with a stated reason. The budget exists so that an agent's context is not spent on a catalog.

Hints. Every tool error has `code`, `message`, and `hint`, where `hint` is one sentence telling an agent what to do next, for example `Call describe_app to see the field names, then retry.` A table of at least 25 common mistakes in the test, each with the code and a hint that must contain the name of a real tool. Examples: a field that does not exist, a stale version, a missing required field, a select value that is not an option id, a link to a record that is not visible, a proposal that failed a check, a job that is not finished.

Teaching resources and prompts.

- Resource `scaffoldry://guide/building-apps`: a concise authoritative guide, at most 12 KB, of how to build an app here: the workflow (describe, propose, wait for approval), field types and what they store, links by id, selects by id, what needs a second person and why, limits, how to page, how to poll a job, and what an agent must never do (such as put a record value in a webhook or a notice). Every example in the guide is a runnable scenario. The runner executes the code blocks of the guide, so the guide cannot drift.
- Prompts: `build_app_from_description`, `import_spreadsheet`, `design_approval_process`, `explain_denial`. Each is a short template that tells the agent which tools to call in order.
- Resource `scaffoldry://apps/{slug}/openapi` from `integrations.md` phase 3.

Tests.

1. Every tool passes conformance, including a truthful annotation table.
2. The `tools/list` budget test passes at the current size and fails when a fixture tool pushes it over.
3. Each of the 25 mistakes returns a code and a hint containing a real tool name.
4. The guide is at most 12 KB and each code block runs as a scenario step.
5. The prompts list and resolve.

## Phase 3 — every brief adds scenarios

A rolling rule, written into the README: a phase that adds or changes a tool or a rule adds scenarios for it in the same change. Target 60 by the end of the plan, by group:

| Group | By brief |
| --- | --- |
| Records and links | `links.md`, `field-types.md`, `calc-graph.md` |
| History and recovery | `record-history.md`, `lifecycle.md` |
| Rules and sharing | `access-rules.md`, `views.md` |
| Moving data | `import-export.md`, `connections.md` |
| Processes | `approvers.md`, `process-v2.md`, `notifications.md` |
| Integration | `integrations.md`, `attachments.md` |
| People outside | `guests.md` |

Every scenario for a denial checks the reason and the remedy. Every scenario for a sensitive field checks the disclosure log and the ledger. Scenarios for a scoped token confirm what it cannot do, as well as what it can.

Tests. At each brief's completion, the suite passes with its new scenarios, and the gap register lists only what is intentionally unsupported.

## How to prompt Gemini

```
Read docs/plans/agent-coverage.md and docs/plans/README.md.
Implement phase N only.
Write the failing test first and run it.
Do not add a dependency.
Do not call a language model.
Do not stub a tool. Run scenarios against the real server and database.
Do not delete an unsupported scenario to make the suite pass.
Do not read, copy, or adapt another product's source code.
Stop when the tests listed for phase N pass, and paste the command output.
```

## Enterprise objections

| Objection | Answer |
| --- | --- |
| A scripted test says nothing about a real agent. | It does not claim to. It proves the server can do what faculty ask and that errors tell an agent what to do next. A real agent's quality is measured outside this plan. |
| The tool list will grow without limit. | A size and count budget fails the build, and a new tool must say what it replaces. |
| The guide will go out of date. | Its code blocks run as scenarios in CI. |
| An agent will be confused by errors. | Every error has a code and a one-sentence hint naming a real tool, and 25 common mistakes are tested. |
| The suite will hide gaps. | It writes the list of unsupported requests every run, and nothing is deleted to pass. |
