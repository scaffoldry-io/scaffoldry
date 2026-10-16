// A real MCP client against a real server. It boots a throwaway server, points the Claude Code CLI at /api/v1/mcp over
// HTTP with a bearer token, asks it to call one read-only tool, and compares the answer with the REST API's.
// One short call to a small model. It costs a few cents, so it is not part of `ui:all`.
// Usage: node scripts/mcp/client-check.mjs   (needs the `claude` command, signed in)
import { spawnSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { boot } from "../dev/boot.mjs";

const env = await boot({ web: false });
let ok = false;
try {
  const auth = { authorization: `Bearer ${env.token}` };
  const expected = (await (await fetch(`${env.api}/workspaces`, { headers: auth })).json()).length;
  const config = path.join(fs.mkdtempSync(path.join(os.tmpdir(), "mcp-")), "mcp.json");
  fs.writeFileSync(config, JSON.stringify({ mcpServers: { scaffoldry: { type: "http", url: `${env.api}/mcp`, headers: auth } } }));

  const prompt = "Call the list_workspaces tool of the scaffoldry server exactly once. Reply with only the number of workspaces it returned, as digits.";
  const run = spawnSync("claude", ["-p", prompt, "--mcp-config", config, "--strict-mcp-config", "--allowedTools", "mcp__scaffoldry__list_workspaces",
    "--max-turns", "4", "--model", "haiku", "--output-format", "json"], { encoding: "utf8", timeout: 180000 });
  if (run.status !== 0) throw new Error(`claude exited ${run.status}: ${(run.stderr || run.stdout).slice(0, 300)}`);
  const out = JSON.parse(run.stdout);
  const answer = String(out.result ?? "").match(/\d+/)?.[0];
  console.log(`${String(answer) === String(expected) ? "PASS" : "FAIL"} a real client called list_workspaces over HTTP (client said ${answer}, REST says ${expected})`);
  if (String(answer) !== String(expected)) console.log(`client replied: ${String(out.result).slice(0, 200)}\ndenied: ${JSON.stringify(out.permission_denials ?? []).slice(0, 200)}`);
  console.log(`cost: $${(out.total_cost_usd ?? 0).toFixed(4)}, turns: ${out.num_turns}`);
  ok = String(answer) === String(expected);
} catch (e) {
  console.log(`FAIL ${e.message}`);
} finally {
  env.stop();
}
process.exit(ok ? 0 : 1);
