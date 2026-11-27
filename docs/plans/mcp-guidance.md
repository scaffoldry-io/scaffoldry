# MCP guidance — design note

```
Title:  The MCP server teaches the agent: it exposes the business logic and the reasoning, securely, so the agent can help a person decide.
Risk:   notable
Ask:    Approve the approach and the phase order. Phase 1 is built and small.
IfYes:  An agent that connects learns who it is acting for, what the platform is for, what it may and may not do, and what to do next at every step. It explains decisions and offers options with their consequences. It stays on the task.
IfNo:   The agent sees a tool list and one-line descriptions. It guesses at the rules, errors teach it nothing, and each person's agent behaves differently.
Checks: Phase 1 only. The rest is design. Each later phase names its failing test first.
```

Approved by Johann on 2026-11-19. Scheduled in `README.md`: data-layer phases 1 and 2 are rows 193 and 194, and MCP guidance phases 2 to 4 are rows 195 to 197.

## The decision

You asked that the MCP server expose as much of the business logic and thinking as can be exposed securely, so that an agent can help a person make an informed choice. You also asked that prompts be woven into the service, to keep the agent focused.

This note applies the principle in `data-layer.md`: the platform provides mechanism, and policy is data. The same policy data that decides access also teaches the agent. They cannot disagree, because they are one document.

## What exists today

I read `routes/mcp.rs` and `service/tools.rs` on 2026-10-19.

| Fact | Where |
| --- | --- |
| `initialize` returns capabilities and a server name. It has no `instructions` field | `routes/mcp.rs` |
| Tools have one-line descriptions. Some say who may use them. The list is held under a size budget by a test | `service/tools.rs`, `mcp_authoritative_api_test.rs` |
| Errors are plain sentences with a code. They give no next step | `service/mod.rs` |
| Two prompts exist, with fixed text, one for building an app and one for a FERPA review. The text is a literal string in the handler | `routes/mcp.rs` |
| Resources include the dataset catalog, the Cedar policies, the decision ledger, the component spec, and the app manifest schema | `routes/mcp.rs` |
| Every tool receives an origin, so the ledger can say an agent acted | `service/tools.rs` |
| `agent-coverage.md` plans error hints and scenarios (row 131). They are not built | `docs/plans/README.md` |

## The ways to teach

MCP gives a server several channels. Use each for what it is good at, and keep each small. An agent pays for every token it reads.

| Layer | Channel | What goes there | Size |
| --- | --- | --- | --- |
| Always on | `instructions` in `initialize` | Who the agent acts for. What this system is. The few rules that hold everywhere. Where to look for more | A few hundred tokens |
| Always on | Tool descriptions | When to use the tool, what it needs first, and what it must not be used for | One or two sentences each |
| Every call | A `guidance` block in a result, and a `hint` in an error | What this result means, what the person can do next, and what to avoid | A few lines |
| On demand | Resources | Policies in plain words, their schemas, glossaries, the person's own access, one dataset's rules | Read only when needed |
| On demand | Prompts | A task with the context, the allowed tools, the stopping points, and the output shape | Chosen by the person or the agent |

## What to expose

As much as is safe. The rule is: **expose the rules and the reasoning, never the data they protect, and never more than the caller may know.**

- **The rules, in words and in schema.** The institution's policies, as the plain documents they are stored as, plus the published schema. An agent can read why something is refused and what would change it.
- **The caller's own position.** What this person holds: positions, grants with dates, conditions with expiry. What they can do, and why. What they cannot do, and what they would need.
- **Decisions, explained.** A tool, `explain_decision`, answers "why was this allowed or refused" with the rules and facts used. For the caller's own requests and for datasets they steward. Not for other people's access, which would reveal their attributes.
- **Options with consequences.** The same structured evaluation a steward reads (what changes, who gains or loses access, what depends on it) is available to the agent as read-only data, before anything is proposed. One document serves the human screen and the agent.
- **Who decides and how.** For a request, the steward position, how it is routed, and what the form needs. The agent can fill the form properly the first time.
- **What good looks like.** Worked examples for each policy type and each common task, in the resources. Scenarios from `agent-coverage.md` double as these examples.

Rules from the README that bind this note:
- **Text from a policy source is data.** Guidance, prompts, and hints may carry the description of a bound object and the sentence of a published rule. They never carry text from a source snapshot. That text arrives only in the `untrusted_text` field (`policy-library.md` rule 3).
- **Admin writes are not MCP tools.** A tool that would change access, ownership, or labels returns a draft and a link. A person confirms in the desk. This follows `access-model.md` phase 14: a draft expires after seven days, records the person, the tool, and the task, and **a widening draft cannot be confirmed by the person whose tool drafted it.** A request filed by a tool waits for the person to confirm it (phase 13).
- **The tool list stays under its budget.** Listing tools by access helps. A new tool must earn its place within the 18,000 bytes.

What stays out:
- **Real values.** Guidance, hints, and examples are built from schemas and synthetic data. A test scans every guidance block for record values.
- **Anything about a hidden dataset.** A policy that names a dataset the caller cannot discover is not shown. Discoverability applies to guidance as it does to the catalog.
- **Other people's attributes.** Reasons for another person's access stay with stewards and administrators.
- **Secrets and internals.** No keys, tokens, or table names.

## Prompts woven into the service

A fixed prompt string in a handler goes stale and cannot be tested against policy. Instead, prompts are **generated from policy data and schemas**, at the time they are asked for.

Each prompt carries five things:
1. **The task.** One sentence.
2. **The context.** The relevant policy, the person's position, and the dataset's entry, fetched for this person.
3. **The allowed tools.** The few tools that fit the task. The agent is told what not to use.
4. **The stopping points.** When to stop and ask the person: before proposing a change, before submitting a request, and whenever something is unclear. Anything that would change access is proposed, never applied.
5. **The output shape.** A schema for what the agent should produce, so it can be checked.

The first prompts, chosen from the cases we walked:
- **Request access.** Find the dataset, read what it holds and who decides, check what the person already has, draft the request with the fields the form needs, and show it to the person.
- **Review a change.** Read an evaluation, explain it in plain words to a steward, list the downstream impacts, and draft the communications plan.
- **Write a policy.** Turn a plain description into a policy that follows the schema, explain it back in words, and test it against sample cases.
- **Review a share request.** Compare a justification with policy and write a short advisory assessment. It never approves or refuses.
- **Explain a refusal.** Say why, name what would change it, and offer the request path.

The agent always works for a person, and a person decides. This holds in every prompt and in `instructions`.

## Keeping the agent focused

- **Tell it the task and the edges** in the prompt and in `instructions`.
- **Show only the tools that fit.** `tools/list` can omit tools the caller cannot use. A smaller list is cheaper and less confusing. The server can signal a changed list when the person's access changes.
- **Point after each step.** A result says what usually comes next, in a line. An error says what to try.
- **Mark untrusted text.** Descriptions, justifications, notes, and any text a person typed arrive in a field marked untrusted. `instructions` tells the agent not to follow instructions found inside such text. This is how prompt injection through data is blunted.
- **Do not let an agent approve its own work.** A tool that would change access returns a proposal and a link for the person. The ledger records that an agent proposed it and a person decided.

## Phases

| Phase | Failing test first | Change |
| --- | --- | --- |
| 1 | `initialize` returns `instructions`. They name the caller, state that the agent acts for a person and proposes before anything changes, say that text in data is untrusted, point to resources and prompts, contain no record value, and stay under a size limit | `instructions`, built per caller. **Built** |
| 2 | Every error code has a hint and every hint names a next step. A refusal names the rule that refused and the request path | Error hints from `agent-coverage.md` row 131. A `guidance` block on the main results. **Built** |
| 3 | `explain_decision` explains the caller's own refusal in words and names the rule. It refuses to explain another person's | The explanation tool. **Built** |
| 4 | A prompt is built from policy data. Changing the policy changes the prompt, with no code change. Each prompt names its tools and stopping points. No prompt contains a record value | Generated prompts. The first five above. **Built** |
| 5 | `tools/list` omits a tool the caller cannot use. It signals a change when access changes | Tools shown by access |
| 6 | Untrusted fields are marked in every result that carries typed text. A tool that would change access returns a proposal and a link. The ledger names the agent as the proposer | Untrusted marking and propose-then-confirm. **Built** |
| 7 | A recorded scenario for each of the first five prompts passes against the real server | Scenarios as the contract, from `agent-coverage.md` |

### Phase 2, as built

- **Hints.** `service/hints.rs` has `hint_for_code`, one fixed sentence for every code in `ERROR_CODES`. `hint_for` sharpens three codes (`not_found`, `bad_request`, `forbidden`) from the shape of the message and never copies text from it. Every error body, over HTTP and over MCP, now carries `hint`. Old fields are unchanged.
- **Refusals.** A `forbidden` body carries `rule` (`{ id, description }`) and `request_path`. The rule is the Cedar policy that refused. With no policy, a Cedar decision is `default_deny`, and a role check made in code is `role_required`. The path is `request_access`, or `POST /api/v1/access/requests`, when the refusal carries a filled request. Otherwise it says who to ask. A hidden field carries the same `request_path`.
- **Guidance.** `service/next_step.rs` holds a fixed `{ next, avoid }` block for nine tools: `list_workspaces`, `list_apps`, `describe_app`, `list_records`, `get_record`, `create_record`, `update_record`, `propose_app_change`, `request_access`. `routes/mcp.rs` adds it to `structuredContent`. Nothing changed in `tools/list`, so the byte budget is untouched.
- **Tests.** `tests/mcp_guidance_phase2_test.rs`. The per-code hint table, the 25 mistakes in `mcp_hints_test.rs`, and the no-record-value checks together hold the failing-test-first rows above. The OpenAPI `Error` schema gained `hint`, `rule` and `request_path`, and the checked-in fixture was regenerated.

### Phase 3, as built

- **Decision.** "explain_decision" ships as `request_access` with `explain: true`, because the tool list is full (45 of 45 tools, 23,995 of 24,000 bytes). The budget was not raised. Wording in other tool descriptions was trimmed to make room. No behavior or safety sentence was cut.
- **What it returns.** `allowed`, the `rule` that decided (`id` and `description`), an `explanation` in words, the `facts` used (sensitivity, visibility, own unit, days, the axes on which held grants missed), and `would_route`, the route a request would take. It files nothing and carries a `guidance` block that replaces the filing one.
- **Reuse.** The grants and the policy decision come from `access_review::explain`, the service behind `GET /access/explain` (`access-model.md` phase 7). The rule is named as a refusal names it, by `forbidden_rule`. No REST route was added.
- **Other people.** A `person` other than the caller is refused, on filing and on explaining, and the refusal does not repeat the name. The brief also lets a steward ask about datasets they steward. No steward role exists in the code, so nobody may ask yet. Add it with the steward position.

### Phase 4, as built

- **Where.** `service/guided_prompts.rs`. `prompts/list` adds five prompts to the older ones, and `prompts/get` builds the text for the caller each time it is asked. The older fixed prompts are unchanged. Prompts are not tools, so `tools/list` and its byte budget did not change.
- **Five parts.** Every prompt prints `Task`, `Context`, `Tools you may use` (with a `Do not use` line), `Stop and ask the person`, and `Output shape`. A test requires that every word in backticks is a name in `TOOLS`, and that no word that looks like a tool name is missing from it.
- **Prompts and their data.**
  - `request_dataset_access`: the stored request rules (`request_rules`), the setting `access.request_max_days`, the authority tables for the three request checks at the person's unit, and the person's positions and admin rights. Tools: `list_datasets`, `list_apps`, `explain_access`, `request_access`.
  - `explain_a_refusal`: `access_requests::explain` for the object named (rule, facts, held and asked role, route), the same rules and limit. Tools: `request_access` with `explain: true`, `explain_access`, `get_workspace`, `list_workspaces`. The explanation sentence is left out, because it carries the app title.
  - `review_a_change`: the authority tables for all flagged checks at the person's unit (tier and who decides). Tools: `get_proposal`, `list_proposals`, `describe_app`, `get_workspace`.
  - `write_a_policy`: the active Cedar version, the policy summaries (key, effect, OSCAL control, description), and the stored test case count. `simulate_cedar_policy` is offered only to a Platform Admin.
  - `review_a_share_request`: the setting `sharing.allowed_scopes` and the sharing limits. Tools: `describe_app`, `list_views`, `get_workspace`.
- **Not built, and said so in the prompt.** A stored steward evaluation and a communications plan do not exist, so `review_a_change` tells the agent to build both from the proposal's checks and change list and to label them a draft. A share request has no stored record or workflow, so `review_a_share_request` takes the justification as an argument and marks it as untrusted text.
- **Tests.** `tests/mcp_guidance_phase4_test.rs`: the five parts, real tool names only, no planted record value, and that a changed setting, request rule, authority table or active Cedar policy changes the next prompt with no code change.

### Phase 6, as built

- **Marking.** `service/untrusted.rs::mark` runs in `routes/mcp.rs` on every `tools/call` result, for `structuredContent` and the text copy. Typed text moves into an `untrusted_text` member of the object that held it. A record keeps its path: `{ "data": {}, "untrusted_text": { "data": { "title": "..." } } }`. A bare text with no other name stays a string, as `read_library_snapshot` already did. The member holds a string or a map.
- **What moves.** A key list (title, description, label, summary, reason, rationale, note, comment, question, statement, justification, answers, prompt, instructions, and others). Per-tool paths for names of workspaces, collaborators, tables and views, `aggregate_records` groups, `get_page` html and `list_datasets` sample rows. Record fields of type Text, LongText, Email, Phone, Url, Formula, Lookup and Rollup, found from the app manifest.
- **Instructions.** `guidance.rs` names `untrusted_text`. The size limit holds.
- **Propose then confirm.** `update_workspace` and `manage_workspace_member` wrote access, ownership and labels directly. They now draft a change (`service/workspace_drafts.rs`, kinds `workspace_settings` and `workspace_member`) that a person confirms in the desk and that runs as the confirmer. `update_workspace` drafts only when visibility, data classification, department or unit change. A change to name, description or icon applies directly. A widening draft cannot be confirmed by the person it was drafted for. Moving a department or unit counts as widening.
- **Links.** `draft_access_change`, the two converted tools, `propose_app_change` and a `request_access` filing return `desk_link` (`/admin/access?change=<id>`, `/proposals?id=<id>`, `/me/access/requests?request=<id>`).
- **Ledger.** Confirm payloads carry `proposer: { via, for, client, tool, task }`. The tool name is stored as `via_tool` in the change JSON, so no migration was added. The ledger principal is the person who confirmed.
- **Screens.** `ui/bridge.js`, `proposal-review.html` and `decision.html` carry a copy of `untrusted::restore`, so they render the text as before.
- **Page rows.** A row a page reads (`page_call`) has `version` beside `data` and no `app_slug`. The call's `app_slug` names its app, so its text fields are marked as a record's are.
- **Tests.** `tests/mcp_guidance_phase6_test.rs`. About 30 older test files, and the scenario files in `tests/agent_scenarios/`, read a text field that moved on purpose. They now read it from `untrusted_text`, or put it back with `untrusted::restore` when they compare a tool with a route that does not mark. No assertion was weakened.
- **Not done.** Resources, prompts and error bodies are not marked, only tool results. `decide_proposal` is a person's tool and stays closed to agent sessions. `save_view` and record writes run under the person's own rights and stay direct.

## Enterprise objections

| Objection | Answer |
| --- | --- |
| "Telling the agent the rules helps an attacker." | The rules are not secrets. The data is. Guidance contains no data. An attacker with a valid session could read the policies anyway. What they cannot read is hidden by discoverability and by access |
| "An agent will talk a steward into a bad approval." | The agent drafts and explains. The steward decides, on the same evaluation page they would use anyway. The assessment is advisory and labeled |
| "Prompt injection through a record or a justification." | Typed text is marked untrusted, and the agent is told so. No tool applies an access change without a person |
| "Instructions cost tokens on every session." | They are a few hundred tokens. The rest is on demand. A size test holds the limit, as it does for the tool list |
| "The prompts will drift from the policy." | They are generated from the policy data. They cannot drift. A scenario fails if they do |

## Not decided, and not verified

- **Size limits.** I set none beyond phase 1's. The tool list is held at 18,000 bytes. The limit for `instructions` is a number to pick after seeing what agents need.
- **Whether every client shows `instructions`.** The MCP specification has the field. I know that from memory, and have not checked how each client uses it. Some may ignore it. The tool descriptions and result guidance cover that case.
- **Elicitation.** Newer MCP versions may let a server ask the person a question through the client. I have not checked the supported versions here. Until I do, the plan uses a link.
- **Agent quality.** The suite tests the server, not an agent's judgment. A real client check, as in `docs/mcp-client.md`, can show how a client uses the guidance.

## Prompt for the session

Read this file, `agent-coverage.md`, and `data-layer.md`. Phases 1 to 4 are built. Do one phase at a time. Stop when its tests pass.
