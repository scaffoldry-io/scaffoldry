// A wire-level check of the MCP endpoint, the way a client talks to it. No dependencies. Prints one line per check.
// Used by scripts/ui/run.mjs, or alone: MCP_URL=... MCP_TOKEN=... node scripts/mcp/smoke.mjs
export async function mcpSmoke({ url, token }) {
  const results = [];
  const check = (name, ok, detail = "") => results.push({ name, ok: !!ok, detail: ok ? detail : detail || "failed" });
  let id = 0;
  const call = async (method, params, { auth = true, notification = false } = {}) => {
    const body = { jsonrpc: "2.0", method, ...(params ? { params } : {}), ...(notification ? {} : { id: ++id }) };
    const res = await fetch(url, { method: "POST", headers: { "content-type": "application/json", accept: "application/json, text/event-stream", ...(auth ? { authorization: `Bearer ${token}` } : {}) }, body: JSON.stringify(body) });
    const text = await res.text();
    let json = null; try { json = text ? JSON.parse(text) : null; } catch { /* not JSON */ }
    return { status: res.status, json, text };
  };

  const unauth = await call("tools/list", undefined, { auth: false });
  check("refuses a call with no token", unauth.status === 401, `status ${unauth.status}`);

  const init = await call("initialize", { protocolVersion: "2025-06-18", capabilities: {}, clientInfo: { name: "smoke", version: "0" } });
  check("initialize answers with a version and server name", init.json?.result?.protocolVersion && init.json?.result?.serverInfo?.name, JSON.stringify(init.json?.result?.serverInfo));
  check("initialize offers tools", !!init.json?.result?.capabilities?.tools);

  const note = await call("notifications/initialized", undefined, { notification: true });
  check("a notification gets 202 and no body", note.status === 202 && note.text === "", `status ${note.status}`);

  const listed = await call("tools/list");
  const tools = listed.json?.result?.tools ?? [];
  check("tools/list returns tools", tools.length > 0, `${tools.length} tools`);
  check("every tool has a name, description, and object schema", tools.every((t) => t.name && t.description && t.inputSchema?.type === "object"));
  const size = JSON.stringify(tools).length;
  results.push({ name: "tool list size", ok: true, detail: `${tools.length} tools, about ${Math.round(size / 4)} tokens` });

  const reads = tools.filter((t) => t.annotations?.readOnlyHint && !(t.inputSchema.required || []).length).slice(0, 4);
  for (const t of reads) {
    const r = await call("tools/call", { name: t.name, arguments: {} });
    const sc = r.json?.result?.structuredContent;
    const ok = r.status === 200 && r.json?.result && !r.json.result.isError && sc && typeof sc === "object" && !Array.isArray(sc);
    check(`read tool ${t.name} returns structured content as an object`, ok, ok ? "" : r.text.slice(0, 120));
  }

  const ping = await call("ping");
  check("ping answers with an empty result", ping.json?.result && Object.keys(ping.json.result).length === 0, ping.text.slice(0, 80));

  const raw = await fetch(url, { method: "POST", headers: { "content-type": "application/json", authorization: `Bearer ${token}` }, body: "{not json" });
  const rawJson = await raw.json().catch(() => null);
  check("malformed JSON is a JSON-RPC parse error, code -32700", rawJson?.error?.code === -32700, `status ${raw.status}`);

  for (const method of ["resources/list", "prompts/list"]) {
    const r = await call(method);
    check(`${method} answers`, !!r.json?.result, r.text.slice(0, 80));
  }
  const get = await fetch(url, { headers: { authorization: `Bearer ${token}`, accept: "text/event-stream" } });
  check("a GET is either a stream or a clean 405, so clients know there is no server stream", get.status === 405 || get.headers.get("content-type")?.includes("text/event-stream"), `status ${get.status}`);

  const unknown = await call("tools/call", { name: "no_such_tool", arguments: {} });
  check("an unknown tool is a tool error, not a crash", unknown.status === 200 && (unknown.json?.result?.isError || unknown.json?.error), `status ${unknown.status}`);
  const badMethod = await call("no/such/method");
  check("an unknown method is a JSON-RPC error", !!badMethod.json?.error, badMethod.text.slice(0, 80));
  return results;
}

if (import.meta.url === `file://${process.argv[1]}`) {
  const results = await mcpSmoke({ url: process.env.MCP_URL, token: process.env.MCP_TOKEN });
  for (const r of results) console.log(`${r.ok ? "PASS" : "FAIL"} ${r.name}${r.detail ? ` (${r.detail})` : ""}`);
  process.exit(results.some((r) => !r.ok) ? 1 : 0);
}
