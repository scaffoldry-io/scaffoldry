// @vitest-environment node
// The page runtime: the loader compiles a page inside the frame and the bridge talks to the host.
// The shell is filled here the way `render_page` fills it on the server, from the same files.
import { readFileSync } from "node:fs";
import { JSDOM } from "jsdom";
import { describe, expect, it } from "vitest";

const ui = new URL("../../../../crates/scaffoldry-server/src/ui/", import.meta.url);
const read = (name: string) => readFileSync(new URL(name, ui), "utf8");

type Rpc = { id: number; method: string; params: { name: string; arguments: Record<string, unknown> } };

function shell(source: string, format: "module" | "bundle" = "module"): string {
  const meta = { app: { slug: "scholarships", page_id: "board", title: "Board" }, grants: [{ table_id: "applications", read: ["title"], write: [], create: false, delete: false }], format };
  const markers: Record<string, string> = {
    "@@REACT@@": read("vendor/react.js"),
    "@@REACT_DOM@@": read("vendor/react-dom.js"),
    "@@SUCRASE@@": read("vendor/sucrase.js"),
    "@@BRIDGE@@": read("bridge.js"),
    "@@LOADER@@": read("loader.js"),
    "@@META@@": JSON.stringify(meta).replaceAll("<", "\\u003c"),
    "@@SOURCE@@": JSON.stringify(source).replaceAll("<", "\\u003c"),
  };
  const template = read("page-shell.html");
  // One pass over the template, so a page that contains a marker's text is not filled twice.
  return template.replace(/@@[A-Z_]+@@/g, (m) => markers[m] ?? m);
}

/** Opens the page in jsdom. `reply` plays the host: it answers each `page_call` request. */
async function open(source: string, reply: (rpc: Rpc) => unknown, format: "module" | "bundle" = "module", extra: Record<string, unknown> = {}) {
  const requests: Rpc[] = [];
  /** The messages of the MCP Apps handshake (ui/initialize and ui/notifications/initialized), kept apart from the calls. */
  const handshake: Rpc[] = [];
  const dom = new JSDOM(shell(source, format), {
    runScripts: "dangerously",
    pretendToBeVisual: true,
    beforeParse(win) {
      Object.assign(win, extra);
      win.postMessage = ((rpc: Rpc) => {
        if (rpc.method !== "tools/call") handshake.push(rpc);
        else requests.push(rpc);
        queueMicrotask(() => {
          const result = reply(rpc);
          win.dispatchEvent(new win.MessageEvent("message", { data: { jsonrpc: "2.0", id: rpc.id, result: { structuredContent: result } }, source: win }));
        });
      }) as never;
    },
  });
  for (let i = 0; i < 50; i++) {
    await new Promise((r) => setTimeout(r, 20));
    if (dom.window.document.getElementById("root")!.textContent) break;
  }
  await new Promise((r) => setTimeout(r, 50));
  return { doc: dom.window.document, requests, handshake, win: dom.window };
}

const PAGE = `
import { useEffect, useState } from "react";
import { records } from "scaffoldry";
export default function Page() {
  const [n, setN] = useState(null);
  useEffect(() => { records.list({ table_id: "applications" }).then((r) => setN(r.records.length)); }, []);
  return <p id="out">{n === null ? "loading" : "rows:" + n}</p>;
}`;

describe("the loader", () => {
  it("compiles a page that imports react and scaffoldry, and mounts it", async () => {
    const { doc, requests, handshake } = await open(PAGE, () => ({ records: [{ id: "a" }, { id: "b" }], next_cursor: null }));
    expect(doc.getElementById("out")?.textContent).toBe("rows:2");
    expect(requests).toHaveLength(1);
    expect(handshake.map((m) => m.method)).toEqual(["ui/initialize", "ui/notifications/initialized"]);
    expect(requests[0].method).toBe("tools/call");
    expect(requests[0].params).toEqual({ name: "page_call", arguments: { app_slug: "scholarships", page_id: "board", op: "list", table_id: "applications" } });
  });

  it("shows an error panel for a page that imports anything else", async () => {
    const { doc } = await open(`import _ from "lodash";\nexport default function P() { return <p>never</p>; }`, () => ({}));
    const text = doc.getElementById("root")!.textContent!;
    expect(text).toContain("lodash");
    expect(doc.getElementById("out")).toBeNull();
  });

  it("shows an error panel for a page that does not compile", async () => {
    const { doc } = await open(`export default function P( { return <p>`, () => ({}));
    expect(doc.getElementById("root")!.textContent).toMatch(/could not run/i);
  });

  it("shows an error panel for a page with no default export", async () => {
    const { doc } = await open(`export const x = 1;`, () => ({}));
    expect(doc.getElementById("root")!.textContent).toMatch(/default export/i);
  });
});

describe("a bundle", () => {
  // A stand-in for a charting library: the bundle carries its own drawing code and mounts into #root.
  const CHART = `
    (function () {
      function bars(el, values) {
        var svg = document.createElementNS("http://www.w3.org/2000/svg", "svg");
        values.forEach(function (v, i) {
          var r = document.createElementNS("http://www.w3.org/2000/svg", "rect");
          r.setAttribute("x", i * 12); r.setAttribute("height", v); svg.appendChild(r);
        });
        el.appendChild(svg);
      }
      scaffoldry.records.list({ table_id: "applications" }).then(function (r) {
        bars(document.getElementById("root"), r.records.map(function (x) { return x.data.amount; }));
      });
    })();`;

  it("runs as written, reaches the bridge through the global, and draws", async () => {
    const { doc, requests } = await open(CHART, () => ({ records: [{ data: { amount: 3 } }, { data: { amount: 5 } }, { data: { amount: 8 } }] }), "bundle");
    expect(doc.querySelectorAll("#root svg rect")).toHaveLength(3);
    expect(requests[0].params.arguments.op).toBe("list");
  });

  it("shows a refused request as an error inside the frame", async () => {
    // A browser applies the frame's policy and refuses the request, and the bundle's promise rejects. jsdom does not
    // raise `unhandledrejection` itself, so the test raises it, the way a browser would for the refused call.
    const { doc, win } = await open(`document.getElementById("root").dataset.ran = "1";`, () => ({}), "bundle");
    const ev = new win.Event("unhandledrejection");
    Object.assign(ev, { reason: new TypeError("Failed to fetch") });
    win.dispatchEvent(ev);
    expect(doc.getElementById("root")!.textContent).toContain("Failed to fetch");
  });

  it("is not compiled, so JSX in it is an error", async () => {
    const { doc } = await open(`var x = <p>hi</p>;`, () => ({}), "bundle");
    expect(doc.getElementById("root")!.textContent).toMatch(/could not run/i);
  });
});

describe("the files", () => {
  const FORBIDDEN = ["fetch", "XMLHttpRequest", "localStorage", "sessionStorage", "cookie", "eval", "new Function", "WebSocket"];
  // A comment that names one of these would trip the test too, so the files do not.
  for (const file of ["bridge.js", "loader.js"]) {
    it(`${file} uses none of the forbidden names`, () => {
      const text = read(file);
      for (const word of FORBIDDEN) expect(text, `${file} contains ${word}`).not.toContain(word);
    });
  }
  it("bridge.js is under 150 lines and loader.js under 80", () => {
    expect(read("bridge.js").split("\n").length).toBeLessThan(150);
    expect(read("loader.js").split("\n").length).toBeLessThan(80);
  });
  it("the shell's policy forbids network, eval, and forms", () => {
    const shellText = read("page-shell.html");
    const csp = /Content-Security-Policy"\s+content="([^"]+)"/.exec(shellText)![1];
    expect(csp).toContain("default-src 'none'");
    expect(csp).toContain("connect-src 'none'");
    expect(csp).not.toContain("unsafe-eval");
    expect(shellText.indexOf("Content-Security-Policy")).toBeLessThan(shellText.indexOf("<script"));
  });
});
