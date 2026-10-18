# MCP guidance — design note

```
Title:  The MCP server teaches the agent: it exposes the business logic and the reasoning, securely, so the agent can help a person decide.
Risk:   notable
Ask:    Approve the approach and the phase order. Phase 1 is built and small.
IfYes:  An agent that connects learns who it is acting for, what the platform is for, what it may and may not do, and what to do next at every step. It explains decisions and offers options with their consequences. It stays on the task.
IfNo:   The agent sees a tool list and one-line descriptions. It guesses at the rules, errors teach it nothing, and each person's agent behaves differently.
Checks: Phase 1 only. The rest is design. Each later phase names its failing test first.
```

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
| 2 | Every error code has a hint and every hint names a next step. A refusal names the rule that refused and the request path | Error hints from `agent-coverage.md` row 131. A `guidance` block on the main results |
| 3 | `explain_decision` explains the caller's own refusal in words and names the rule. It refuses to explain another person's | The explanation tool |
| 4 | A prompt is built from policy data. Changing the policy changes the prompt, with no code change. Each prompt names its tools and stopping points. No prompt contains a record value | Generated prompts. The first five above |
| 5 | `tools/list` omits a tool the caller cannot use. It signals a change when access changes | Tools shown by access |
| 6 | Untrusted fields are marked in every result that carries typed text. A tool that would change access returns a proposal and a link. The ledger names the agent as the proposer | Untrusted marking and propose-then-confirm |
| 7 | A recorded scenario for each of the first five prompts passes against the real server | Scenarios as the contract, from `agent-coverage.md` |

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

Read this file, `agent-coverage.md`, and `data-layer.md`. Phase 1 is built. Do one phase at a time. Stop when its tests pass.
