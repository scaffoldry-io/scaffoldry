# Agent scenarios

Each `*.json` file is a scripted request from a faculty member, run by `tests/agent_scenarios.rs` through the real MCP endpoint (`/api/mcp`) as one persona's bearer token. No language model is called. A scenario tests the server, not an agent's judgment, and the files double as the product's examples.

**Read `target/agent-gaps.md` before planning.** Every run writes it: the scenarios marked `unsupported`, with the ask and the reason. That list is the backlog of what an agent cannot yet do. The file is generated and is never committed. Run the suite to make it: `cargo test -p scaffoldry-server --test agent_scenarios`.

## A scenario

```json
{
  "id": "build-admissions-intake",
  "ask": "The request in the faculty member's words.",
  "persona": "faculty-owner",
  "status": "supported",
  "steps": [
    { "tool": "propose_app_change", "args": { "workspace_id": "$ws", "manifest": {} },
      "save": { "p": "$.proposal_id" }, "expect": { "checks.schema": "pass" } }
  ],
  "after": [ { "ledger_has": "AppPublished" }, { "no_error_leaks": true } ]
}
```

- `persona`: `faculty-owner`, `editor`, `viewer`, `student`, `compliance`, `platform-admin`, or `agent`. A step may name another with `as`.
- `$name` is a saved value. `$ws`, `$app` (a fresh app slug for every run), `$suffix`, and `$i` (the repeat index) are always there. `$.path` picks from a result.
- `expect` maps a path to a value. A path walks object keys, array indexes, and the `name` of an array element, so `checks.schema` reads the outcome of the `schema` check. A value is a literal or one of `{"len": n}`, `{"contains": "text"}`, `{"gt": n}`, `{"absent": true}`, `{"has": {"key": "value"}}`.
- `error` expects a tool error: `{"code": "forbidden", "contains": "text"}`.
- `repeat` runs a step that many times.
- `after`: `ledger_has` (an entry of that type for this run's app), `no_error_leaks` (no stack trace and no token in any response), and `never_seen_by` (a persona never saw some text).
- `status` is `supported` or `unsupported` with an `unsupported_reason`. An unsupported scenario is never deleted to make the suite pass.

A phase that adds or changes a tool or a rule adds scenarios for it in the same change.
