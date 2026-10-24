// Plays the host of an MCP Apps view (specification 2026-01-26) around one HTML document in jsdom, and prints what happened.
// The document is the top window of its jsdom, so `window.parent` is the window itself: what the view posts arrives as a
// `message` event with no source, and the host answers with an event whose source is the window, as a real host's would be.
const fs = require("fs");
const path = require("path");
const { JSDOM } = require(path.join(process.argv[3], "node_modules", "jsdom"));

const scenario = JSON.parse(fs.readFileSync(process.argv[2], "utf8"));
const tick = (ms = 40) => new Promise((r) => setTimeout(r, ms));

async function main() {
  const html = scenario.html;
  const dom = new JSDOM(html, { runScripts: "dangerously", pretendToBeVisual: true });
  const w = dom.window;
  const outgoing = [];
  w.addEventListener("message", (e) => {
    if (e.source === null && e.data && e.data.jsonrpc === "2.0") outgoing.push(e.data);
  });
  let errors = 0;
  w.addEventListener("error", () => { errors += 1; });
  // A listener that throws is an error of the view, so it is counted, not allowed to stop the harness.
  const send = (data) => {
    try { w.dispatchEvent(new w.MessageEvent("message", { data: Object.assign({ jsonrpc: "2.0" }, data), source: w })); } catch (e) { errors += 1; }
  };
  const find = (method) => outgoing.find((m) => m.method === method);
  const out = { steps: {} };

  await tick();
  out.firstMethod = outgoing[0] && outgoing[0].method;
  out.initialize = find("ui/initialize") || null;

  if (scenario.kind === "bridge") {
    // A call made before the handshake is over must wait.
    w.scaffoldry.records.list({ table_id: "grants" }).catch(() => {});
    await tick();
    out.callsBeforeHandshake = outgoing.filter((m) => m.method === "tools/call").length;
    send({ id: out.initialize.id, result: { protocolVersion: "2026-01-26", hostCapabilities: {}, hostInfo: { name: "test", version: "1" }, hostContext: {} } });
    await tick();
    out.initializedSent = !!find("ui/notifications/initialized");
    out.callsAfterHandshake = outgoing.filter((m) => m.method === "tools/call").map((m) => m.params);
    // Names and ids that belong to every object must not reach a function or a waiting call.
    for (const name of ["constructor", "__proto__", "toString", "hasOwnProperty"]) send({ method: name, params: {} });
    for (const id of ["constructor", "__proto__", "toString"]) send({ id, result: {} });
    await tick();
    out.errors = errors;
    return out;
  }

  // The built-in screens.
  send({ id: out.initialize.id, result: { protocolVersion: "2026-01-26", hostCapabilities: {}, hostInfo: { name: "test", version: "1" }, hostContext: {} } });
  await tick();
  out.initializedSent = !!find("ui/notifications/initialized");
  const before = outgoing.filter((m) => m.method === "tools/call").length;
  out.callsBeforeResult = before;
  send({ method: "ui/notifications/tool-input", params: { arguments: scenario.input || {} } });
  send({ method: "ui/notifications/tool-result", params: scenario.result });
  await tick(40);
  // The host answers any read the screen makes for itself.
  for (const call of outgoing.filter((m) => m.method === "tools/call" && m.params.name === "get_record")) {
    send({ id: call.id, result: { structuredContent: scenario.record || {} } });
  }
  await tick(40);
  const doc = w.document;
  out.text = doc.body.textContent;
  out.imgCount = doc.querySelectorAll("img, script[src], iframe, object, embed").length;
  out.inlineHandlers = doc.querySelectorAll("[onerror], [onclick], [onload]").length;
  out.buttons = Array.from(doc.querySelectorAll("button")).map((b) => b.textContent.trim());

  if (scenario.rationale !== undefined) {
    const box = doc.querySelector("input, textarea");
    if (box) { box.value = scenario.rationale; box.dispatchEvent(new w.Event("input", { bubbles: true })); }
  }
  const press = (label) => Array.from(doc.querySelectorAll("button")).find((b) => b.textContent.trim() === label);
  const clicks = [];
  for (const label of scenario.press || []) {
    const b = press(label);
    if (!b) { clicks.push({ label, found: false }); continue; }
    b.click();
    await tick(30);
    const call = outgoing.filter((m) => m.method === "tools/call").slice(-1)[0];
    clicks.push({ label, found: true, call: call && call.params });
    if (call && scenario.reply) send({ id: call.id, result: scenario.reply });
    await tick(30);
  }
  out.clicks = clicks;
  out.textAfter = doc.body.textContent;
  out.calls = outgoing.filter((m) => m.method === "tools/call").map((m) => m.params);
  for (const name of ["constructor", "__proto__", "toString", "hasOwnProperty"]) send({ method: name, params: {} });
  for (const id of ["constructor", "__proto__", "toString"]) send({ id, result: {} });
  await tick();
  out.errors = errors;
  out.textAfterHostile = doc.body.textContent;
  return out;
}

main().then((r) => { process.stdout.write(JSON.stringify(r)); process.exit(0); }, (e) => { process.stderr.write(String(e && e.stack || e)); process.exit(1); });
