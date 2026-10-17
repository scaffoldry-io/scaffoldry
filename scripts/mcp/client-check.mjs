// A real MCP client against a real server. It boots a throwaway server, points the Claude Code CLI at /api/v1/mcp over
// HTTP with a bearer token, and has it call tools. Each answer is compared with the REST API's:
//   1. a read: list_workspaces, and the count it reports
//   2. a write: create_record, and the row it made, found through REST
// Two short calls to a small model. It costs a fraction of a cent each, so it is not part of `ui:all`.
// Usage: node scripts/mcp/client-check.mjs   (needs the `claude` command, signed in)
import { spawnSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { boot } from "../dev/boot.mjs";
import { mcpClient } from "./client.mjs";
import { seedFixtures } from "./fixtures.mjs";

const env = await boot({ web: false });
let ok = true;
let cost = 0;
const line = (pass, text) => { console.log(`${pass ? "PASS" : "FAIL"} ${text}`); ok = ok && pass; };
try {
  const auth = { authorization: `Bearer ${env.token}` };
  const rest = async (p) => (await fetch(`${env.api}${p}`, { headers: auth })).json();
  const config = path.join(fs.mkdtempSync(path.join(os.tmpdir(), "mcp-")), "mcp.json");
  fs.writeFileSync(config, JSON.stringify({ mcpServers: { scaffoldry: { type: "http", url: `${env.api}/mcp`, headers: auth } } }));

  const ask = (prompt, tool) => {
    const run = spawnSync("claude", ["-p", prompt, "--mcp-config", config, "--strict-mcp-config", "--allowedTools", `mcp__scaffoldry__${tool}`,
      "--max-turns", "4", "--model", "haiku", "--output-format", "json"], { encoding: "utf8", timeout: 180000 });
    if (run.status !== 0) throw new Error(`claude exited ${run.status}: ${(run.stderr || run.stdout).slice(0, 300)}`);
    const out = JSON.parse(run.stdout);
    cost += out.total_cost_usd ?? 0;
    return String(out.result ?? "");
  };

  // 1. A read.
  const expected = (await rest("/workspaces")).length;
  const said = ask("Call the list_workspaces tool of the scaffoldry server exactly once. Reply with only the number of workspaces it returned, as digits.", "list_workspaces").match(/\d+/)?.[0];
  line(String(said) === String(expected), `a real client read: list_workspaces (client said ${said}, REST says ${expected})`);

  // 2. A write, into an app made through the tools.
  const { fixture } = await seedFixtures(await mcpClient({ url: `${env.api}/mcp`, token: env.token }));
  const reply = ask(
    `Call the create_record tool of the scaffoldry server exactly once with app_slug "${fixture.app}" and data {"_table_id": "t", "name": "Written by a real client", "amount": 42}. Reply with only the id of the record it created.`,
    "create_record"
  );
  const rows = (await rest(`/apps/${fixture.app}/records`)).records ?? [];
  const made = rows.find((r) => r.data?.name === "Written by a real client");
  line(!!made && made.data.amount === 42, "a real client wrote: create_record made the row, found through REST");
  line(!!made && reply.includes(made.id), `the client reported the id of the row it made (${made ? "matches" : "no row"})`);
  console.log(`cost: $${cost.toFixed(4)}`);
} catch (e) {
  line(false, e.message);
} finally {
  env.stop();
}
process.exit(ok ? 0 : 1);
