# Using Scaffoldry from an MCP client

Scaffoldry is MCP-first. The server speaks MCP over HTTP at `/api/v1/mcp` and exposes 31 tools. This page is how to attach a client, and how the endpoint is tested.

## Attach a client

Run a disposable server. It makes a throwaway database, seeds it, and prints a token and the attach command.

```
npm run mcp:serve
```

It prints a line like `claude --mcp-config /tmp/mcp-XXXX/mcp.json` and an alternative, `claude mcp add --transport http scaffoldry <url> --header "Authorization: Bearer <token>"`. Use either. Add `--web` to also start the desk on 5173. Ctrl-C stops everything and deletes the database.

The printed token is the setup token. It lasts 24 hours and stops working when the first Platform Admin is appointed. For a long-lived server, a person mints an agent token in the settings pane and the client sends it as `Authorization: Bearer <token>`.

The transport is plain request and response JSON over HTTP POST. A GET returns 405, which tells a client there is no server-sent stream. That is allowed by the spec and clients handle it. Tools that return a list put it under `items` in `structuredContent`, because the spec requires an object there. The text content always holds the plain result.

## How it is tested

| Check | Command | What it proves | Cost |
| --- | --- | --- | --- |
| Protocol tests | `cargo test -p scaffoldry-server --test mcp_protocol_test` | Ping returns an empty result. A bad body gets a JSON-RPC parse error or an invalid-request error. No token is refused. A list result is an object under `items` | Free |
| Agent scenarios | `cargo test -p scaffoldry-server --test agent_scenarios` | Twenty scripted faculty requests run through the endpoint with real personas. In-process, no spec validation | Free |
| Wire check | `npm run ui:mcp` | A booted server, spoken to over HTTP the way a client does: initialize, the initialized notification, the tool list and its size, read tools, errors, ping, parse errors, GET | Free |
| Write checks | `npm run ui:mcp` (the "MCP writes" part) | Through the tools: propose an app, approve it, create five rows, edit one, make a stale edit and get the error that names the current version, delete one, and list what is left | Free |
| Real client | `npm run mcp:client` | The Claude Code CLI attaches over HTTP. It reads with `list_workspaces`, and its count matches REST. It writes with `create_record`, and the row is found through REST with the id it reported | About half a cent |

The real client check found a fault the others missed on its first run. The server returned a bare list as `structuredContent`, and the client rejected it. Run it after any change to how results are returned.

## Numbers worth knowing

- 31 tools. The tool list costs a client about 3,800 tokens of context, on every session that loads it.
- Read tools carry `readOnlyHint`. Write tools do not.

## Fixtures

The write checks are also the fixtures. `scripts/mcp/fixtures.mjs` makes an app called Fixture Register with four rows (one with accents, one edited, one deleted) using only MCP tools. `npm run ui:smoke` and `npm run ui:audit` run it first, so the screens have real data, and the smoke flow checks that the rows, the edit, and the deletion show in the desk. That is the whole path: tool call, database, screen.

## Oddities noticed

- A stale version through MCP comes back with the code `version_conflict`, the same as over HTTP. The message names the current version.
- A proposal made through MCP is held as a proposal. An administrator can approve their own when every check passes, which the fixtures rely on. A change that fails a check would need a second person.

## Not done

- Error hints (README row 131). The tool-list budget is a test: `the_tool_list_stays_inside_its_budget` fails when the list passes 18,000 bytes (about 4,500 tokens). It is 15,343 bytes now.
- Update and delete through a real client. The real client writes with `create_record`. Update and delete are covered over the wire, not by a model.
- A session id, a server-sent stream, and `outputSchema` on tools. None is required.
