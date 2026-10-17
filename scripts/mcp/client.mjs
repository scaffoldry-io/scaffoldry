// A small MCP client over HTTP: initialize, then call tools. No dependencies.
export async function mcpClient({ url, token }) {
  let id = 0;
  const headers = { "content-type": "application/json", accept: "application/json, text/event-stream", authorization: `Bearer ${token}` };
  const post = async (body) => {
    const res = await fetch(url, { method: "POST", headers, body: JSON.stringify(body) });
    const text = await res.text();
    return { status: res.status, json: text ? JSON.parse(text) : null };
  };
  const init = await post({ jsonrpc: "2.0", id: ++id, method: "initialize", params: { protocolVersion: "2025-06-18", capabilities: {}, clientInfo: { name: "scaffoldry-scripts", version: "0" } } });
  if (!init.json?.result) throw new Error(`MCP initialize failed: ${JSON.stringify(init.json)}`);
  await post({ jsonrpc: "2.0", method: "notifications/initialized" });
  return {
    /** Calls a tool. `value` is the plain result: the object, or the list under `items`. */
    async call(name, args = {}) {
      const r = (await post({ jsonrpc: "2.0", id: ++id, method: "tools/call", params: { name, arguments: args } })).json;
      if (r?.error) return { ok: false, code: String(r.error.code), message: r.error.message, value: null };
      const result = r.result;
      const sc = result.structuredContent ?? null;
      return { ok: !result.isError, code: sc?.code, message: result.content?.[0]?.text ?? "", value: sc && Array.isArray(sc.items) && Object.keys(sc).length === 1 ? sc.items : sc };
    },
  };
}
