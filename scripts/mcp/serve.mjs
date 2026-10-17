// Runs a disposable Scaffoldry for an MCP client to talk to. Prints the token and the command that attaches Claude Code.
// Stop it with Ctrl-C. The database is deleted. Usage: node scripts/mcp/serve.mjs [--web]
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { boot } from "../dev/boot.mjs";

const env = await boot({ web: process.argv.includes("--web") });
const url = `${env.api}/mcp`;
const config = path.join(fs.mkdtempSync(path.join(os.tmpdir(), "mcp-")), "mcp.json");
fs.writeFileSync(config, JSON.stringify({ mcpServers: { scaffoldry: { type: "http", url, headers: { Authorization: `Bearer ${env.token}` } } } }, null, 2));

console.log(`Scaffoldry is running. API ${env.api}${process.argv.includes("--web") ? `, desk ${env.web}` : ""}`);
console.log(`MCP endpoint: ${url}`);
console.log(`Token (24 hours, and it stops working when the first Platform Admin is appointed): ${env.token}`);
console.log("");
console.log("Attach Claude Code for one run:");
console.log(`  claude --mcp-config ${config}`);
console.log("or add it to your setup:");
console.log(`  claude mcp add --transport http scaffoldry ${url} --header "Authorization: Bearer ${env.token}"`);
console.log("");
console.log("Press Ctrl-C to stop and delete the throwaway database.");

const stop = () => { env.stop(); process.exit(0); };
process.on("SIGINT", stop);
process.on("SIGTERM", stop);
setInterval(() => {}, 1 << 30);
