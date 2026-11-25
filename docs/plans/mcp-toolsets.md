# MCP toolsets — design note

```
Title:  The tool list is split into areas, called toolsets. A token lists only the toolsets it was granted, and each list has its own budget.
Risk:   notable
Ask:    Approve the toolsets, the way a token gets them, and the per-list budget. Then phase 1 can start.
IfYes:  New tools fit by area. An agent sees the 15 to 25 tools its task needs, not all 45. The 24 KB budget holds for every list an agent can receive.
IfNo:   One flat list of 45 tools at 23,995 of 24,000 bytes. The next tool needs a budget raise or a merge, and every session pays for every tool.
Checks: None yet. Each phase names its failing test first.
```

## The decision

Johann asked whether the tool problem needs a scoped MCP server for each area of the platform. **No.** One server and one endpoint are enough. The list a caller sees is chosen by the authorization on the request. A separate server per area would give each client several connections, several consents, and several tokens, and the areas share most of their basic tools.

## Standards search

- **MCP 2026-07-28, `server/tools`.** The tool set "**MUST NOT** vary per-connection or as a side effect of other requests on the connection. The set **MAY** vary by the authorization presented on the request — for example, returning only the tools the caller's granted scopes permit — since credentials are per-request input, not connection state." That is exactly this design.
- **The same revision removed sessions.** A tool such as GitHub's `enable_toolset`, which grows the list in the middle of a session, is no longer allowed. A toolset must be granted when the token is issued.
- **MCP authorization (2025-11-25 and later).** A protected resource lists `scopes_supported`. A call that needs more gets `403` with `WWW-Authenticate: Bearer error="insufficient_scope", scope="..."`, and the client may step up. This design uses those words unchanged (RFC 6750 §3.1).
- **GitHub's MCP server** groups tools into toolsets and starts from a default set. That is prior art for the grouping. Its dynamic mode, which enables a toolset during a session, is the part the new spec rules out.
- **OAuth 2.0 scope (RFC 6749 §3.3).** A toolset is a scope token, `mcp:<toolset>`.

## What exists

- `tools/list` already varies by authorization in `routes/mcp.rs`. It hides Platform Admin tools from others, hides tools an administrator turned off, and narrows to a limited token's scope (`token_scope::lists_tool`).
- The consent screen already asks the person to choose an access level for an agent session: read, propose or write. `oauth::scope_for` stores that level on the token.
- `/.well-known/oauth-protected-resource` lists seven scopes. None of them names a set of tools.
- Before phase 3, the budget tests (`mcp_conformance`, `mcp_authoritative_api_test`) measured one list: everything a desk token sees. That list is not an agent's context, so phase 3 measures the lists an agent can receive instead.

## The toolsets

Sizes are the bytes of name, description and input schema, measured on 2026-11-25. The listed size is about 1.45 times larger, because each entry also carries an example call and annotations.

| Toolset | Tools | Raw bytes |
| --- | --- | --- |
| `core`, always listed | `begin_task`, `list_workspaces`, `get_workspace`, `list_apps`, `describe_app`, `list_datasets`, `get_agent_kit`, `list_my_work`, `complete_task`, `get_job`, `cancel_job`, `request_access`, `explain_access` | 2,986 |
| `data` | `create_record`, `list_records`, `get_record`, `update_record`, `delete_record`, `aggregate_records`, `get_view_data`, `list_views`, `save_view`, `delete_view`, `export_records`, `decide_process` | 5,183 |
| `build` | `propose_app_change`, `list_proposals`, `get_proposal`, `preview_proposal`, `decide_proposal`, `get_page`, `page_call`, `update_workspace`, `manage_workspace_member` | 4,188 |
| `access` | `draft_access_change`, `access_suggestions` | 707 |
| `governance` | `governance_report`, `record_governance_decision`, `simulate_cedar_policy`, `list_library_policies`, `read_library_snapshot`, `find_library_rules`, `propose_library_rules`, `list_library_questions` | 2,991 |
| `admin` | `admin_inventory` | 434 |

Every tool is in exactly one toolset. A test holds that.

## How a token gets its toolsets

1. **The default, from phase 2.** A new session that names no toolsets gets `core` and `data`. That is what most agents do: read and change records. Tokens issued before phase 2 keep every toolset until they expire.
2. **Consent.** The consent screen that asks for read, propose or write also lists the toolsets. Each has one plain sentence, such as "Governance: read the policy library and propose rules." `core` is always on. The person ticks the others, and the choice is stored on the session with its level.
3. **OAuth scope.** A client may ask for `mcp:data mcp:governance` in the `scope` parameter. Consent shows those as pre-ticked, and the person can still untick them. `scopes_supported` lists the `mcp:<toolset>` names.
4. **Limited tokens** (`TokenScope.tools`) keep working. Their tool list is already narrower than any toolset.
5. **Rights still decide.** A toolset only decides what is listed and callable. `admin` is offered only to a Platform Admin, and every call still checks the person's own rights. A toolset never adds a right.

## A call outside the granted toolsets

`tools/call` for a tool outside the token's toolsets is refused. The refusal is the phase 2 error with a hint, plus `required_toolset` and `request_path`, which says "reconnect and grant the governance toolset". Over HTTP, the MCP route answers `403` with `WWW-Authenticate: Bearer error="insufficient_scope", scope="mcp:governance"`, so a client that supports step-up can ask the person. `instructions` names the toolsets the session lacks, in one line, so the agent knows they exist.

## The budget

- **Per list.** Every list an agent can receive must stay inside 24,000 bytes and 45 tools. An agent token holds `core` plus at most 3 chosen toolsets (phase 3). The test lists every such combination for a Platform Admin agent session. A desk sign-in token belongs to a person, lists everything, and is not capped.
- **Per toolset.** Each toolset gets its own byte limit. Together they are what lets a new tool in. A new tool must fit inside its toolset's limit, and the largest agent list must still be under 24 KB. The at-most-3 rule keeps every reachable agent list under the cap without raising it. Phase 3 does not add exclusive toolsets, because that rule replaces them.
- **The default list**, `core` plus `data`, is about 12 KB listed. That halves what a typical session loads today.

## Phases

| Phase | Failing test first | Change |
| --- | --- | --- |
| 1 | Every tool is in exactly one toolset. A token whose `toolsets` is `["data", "governance"]` lists `core`, `data` and `governance` only. A token with no `toolsets` value lists what it lists today. A call to a tool outside the granted toolsets is refused with `required_toolset`. `admin` is never listed to a non-admin, even when it is asked for. The largest reachable list stays inside 24 KB and 45 tools | `Toolset` enum and `TOOLS[*].toolset`. `TokenScope.toolsets`, `None` meaning the default. The filter in `tools/list` and the check in `tools/call`. Budget tests measure the largest reachable list and each toolset |
| 2 | The consent screen lists the toolsets with a sentence each. An OAuth `scope` of `mcp:governance` pre-ticks it. The session stores the choice, and a refreshed token keeps it. `scopes_supported` lists the toolsets. A call outside the toolsets over HTTP gets `403` with `insufficient_scope` and the scope to ask for | Consent and the session column. Discovery. The `WWW-Authenticate` header. `instructions` names the missing toolsets |
| 3 | A new tool added to a toolset over its byte limit fails the build. A token cannot hold more than 3 toolsets. Every agent combination stays inside 24 KB and 45 tools | Per-toolset limits in the conformance test. The at-most-3 rule, in place of exclusivity rules |

Phase 1 changes no client. A token with no `toolsets` value keeps every toolset, so every call that worked before still works. Only a token with an explicit `toolsets` value is narrowed. Before phase 2, a client has no way to ask for fewer toolsets, so it could not ask for more either. Phase 2 adds the choice at consent, and only then do new sessions default to `core` and `data`. Tokens issued earlier keep every toolset until they expire.

## Phase 1, as built

Phase 1 is in. Tests: `crates/scaffoldry-server/tests/mcp_toolsets_phase1_test.rs`.

- `Toolset` is in `scaffoldry-core` (`token_scope.rs`). Every `Tool` has a `toolset`, assigned as the table above says.
- `TokenScope.toolsets` is `Option<Vec<Toolset>>`. `None` means every toolset. `validate` refuses `core`, and an unknown name fails to parse. The mint route (`POST /auth/tokens`) takes it with the rest of the scope. No screen sets it yet.
- `tools/list` drops a tool whose toolset the token lacks. This sits beside the existing filters, so `admin` is still shown only to a Platform Admin, whatever a token asks for.
- `tools/call` is checked first in `token_scope::check_tool`. The refusal is `ServiceError::ToolsetNotGranted`, with the code `token_scope`, the phase 2 `hint`, `required_toolset` and a `request_path` that says to reconnect and grant the toolset. It adds no error code.
- A toolset narrows a session or a limited token. It does not widen one. A read session with `toolsets` of `["governance"]` lists the governance tools a read session lists, and no more.
- Sessions made by consent keep `toolsets: None`. Tests build the others.
- The budget tests measured the largest list: a Platform Admin with every toolset. Phase 3 changed that to the largest agent list.

Listed size of each toolset, measured on 2026-11-25 as the bytes of its entries in `tools/list` (`mcp_conformance`, `each_toolset_lists_its_own_size...`):

| Toolset | Tools | Listed bytes |
| --- | --- | --- |
| `core` | 13 | 4,872 |
| `data` | 12 | 7,330 |
| `build` | 9 | 5,971 |
| `access` | 2 | 1,008 |
| `governance` | 8 | 4,277 |
| `admin` | 1 | 542 |
| Largest list (all) | 45 | 23,995 |

The default list in phase 2, `core` plus `data`, is 12,202 bytes before separators. The desk list (every tool) is 5 bytes under the cap. Phase 3 decided that the cap applies to agent lists, which hold at most 3 toolsets.

## Phase 2, as built

Phase 2 is in. Tests: `crates/scaffoldry-server/tests/mcp_toolsets_phase2_test.rs` and `apps/web/src/test/oauth-consent.test.tsx`.

- **Consent.** `GET /oauth/authorize` for an agent client adds `toolsets`: one row per toolset with `id`, `sentence`, `always_on` and `selected`. `core` is always on. `admin` is listed only to a Platform Admin. `Toolset::sentence` in `scaffoldry-core` holds the sentences.
- **OAuth scope.** The `scope` parameter is read at consent. Each `mcp:<toolset>` that is offered is pre-ticked. Unknown `mcp:` values, `mcp:core`, and other scopes are ignored. With none, `data` is ticked.
- **The choice.** `POST /oauth/authorize` takes `toolsets` (a list of names). An unknown name is a `400`. `core` is dropped, and so is `admin` for anyone but a Platform Admin. An empty list gives `data`. The screen is `OAuthConsent.tsx`.
- **Storage.** Migration `0115_session_toolsets.sql` adds a nullable `toolsets TEXT[]` to `oauth_codes` and `oauth_sessions`. The code carries the choice to the token endpoint. The session keeps it, and a refreshed token reads it from the session. Rows from before stay NULL, which means every toolset, so tokens issued earlier keep every toolset until they expire.
- **Default.** A new agent session that picks nothing gets `core` and `data`. A code from before phase 2 is treated the same way.
- **Discovery.** `scopes_supported` keeps its seven values and adds `mcp:data`, `mcp:build`, `mcp:access`, `mcp:governance` and `mcp:admin`. `core` is not listed, because it is never asked for.
- **Refusal.** When `tools/call` is refused with `ToolsetNotGranted`, the response carries `WWW-Authenticate: Bearer error="insufficient_scope", scope="mcp:<toolset>"`. The JSON-RPC body is unchanged. The MCP route answers every tool error with HTTP `200`, so the status stays `200`. The header is there, but a client that steps up on the status code alone will not see it. Moving the status to `403` is a change to every MCP client, and it is not part of this phase.
- **Instructions.** `instructions` adds one line naming the toolsets the session lacks (never `admin`) with the reconnect hint. A token with every toolset, and a desk token, get no line. The limit of 1,800 bytes holds.
- **Tests that signed in an agent.** `agent_clients_test` and `access_drafts_test` now tick every toolset, since they are about levels and need `build` and `access` tools.

Listed size of the default list, measured as the bytes of its entries in `tools/list` for a non-admin Write session: 19 tools, 8,652 bytes. A Read or Propose session lists fewer, and a Platform Admin with the same toolsets sees all 25 `core` and `data` tools, 12,202 bytes.

## Phase 3, as built

Phase 3 is in. Tests: `mcp_conformance`, `mcp_toolsets_phase1_test`, `mcp_toolsets_phase2_test`, `mcp_authoritative_api_test` and `apps/web/src/test/oauth-consent.test.tsx`.

**The decision (Johann).**

- The 24,000-byte and 45-tool cap applies to every list an agent can receive.
- An agent token holds `core` plus at most 3 chosen toolsets.
- A desk sign-in token belongs to a person and is not an agent's context. It lists everything (its `toolsets` is `None`) and is not capped. The budget test only prints its size, 23,995 bytes today.
- There are no exclusivity rules. The at-most-3 rule replaces them, and the first design of the budget section no longer applies.

**The rule.**

- `TokenScope::validate` refuses more than 3 toolsets (`MAX_TOOLSETS`). `POST /auth/tokens` answers 400.
- `POST /oauth/authorize` answers 400 with "Choose at most 3 toolsets. Core is always on and does not count."
- The consent screen disables the unticked boxes once 3 are ticked and says "Up to three areas".
- If an OAuth `scope` asks for more than 3, the server pre-ticks the first 3 in the order `data`, `build`, `access`, `governance`, `admin`.
- A session from before phase 2 has no stored toolsets and lists every toolset until it expires. It is the one agent list the rule does not bound.

**The limits.** Each is measured as the toolset's entries in `tools/list`, the way phase 1 did.

| Toolset | Limit (bytes) | Listed today (bytes) | Free |
| --- | --- | --- | --- |
| `core` | 5,000 | 4,872 | 128 |
| `data` | 7,600 | 7,330 | 270 |
| `build` | 6,200 | 5,971 | 229 |
| `governance` | 5,000 | 4,277 | 723 |
| `access` | 2,000 | 1,008 | 992 |
| `admin` | 1,500 | 542 | 958 |

**The structural check.** `core` (5,000) plus the 3 largest other limits (`data`, `build`, `governance`: 18,800) plus the JSON separators (2 brackets and one comma for each of 45 tools, 47 bytes) is 23,847. That is at most 24,000, so no limits were lowered. A test holds the sum.

**The combination check.** A test lists `tools/list` for a Platform Admin Write session granted each of the 26 choices of up to 3 toolsets beyond `core`, and holds every list to 24,000 bytes and 45 tools. A session lists only the tools its level allows, so the largest served list is smaller than the limits allow: 30 tools and 14,026 bytes. The limits are the bound a new tool must respect, and the served lists have room beyond it.

## Enterprise objections

| Objection | Answer |
| --- | --- |
| Hiding tools is security by obscurity. | Hiding is for cost and focus, not security. A call to an unlisted tool is refused on the server, and the person's own rights are still checked on every call. |
| Several toolsets confuse users at consent. | Each toolset has one plain sentence. `core` and `data` are the default, so most people change nothing. |
| An agent will not know a tool exists. | `instructions` names the missing toolsets. A refused call names the toolset and how to get it. |
| This needs several MCP servers to operate. | It needs one. The spec allows the list to vary by the authorization on the request, and that is all this uses. |
| Clients ignore `list_changed`. | This design never needs it. The list is fixed for a token and changes only when a new token is issued. |
| The budget can still creep. | Each toolset has a limit, and the largest reachable list keeps the 24 KB cap. Both are tests. |

## Not decided

- Whether `decide_process` belongs in `data` or in a future `work` toolset with `list_my_work` and `complete_task`. Phase 1 puts it in `data`.
- Whether an institution may change the default toolsets. Phase 1 fixes the default in code.

## Prompt for the session

Read this file, `mcp-guidance.md` and `agent-coverage.md`. Do one phase at a time. Stop when its tests pass.
