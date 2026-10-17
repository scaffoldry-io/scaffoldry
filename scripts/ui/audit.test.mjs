// Tests for the UI audit itself. Each page below has known defects, so a rule that stops working fails here.
// Run: node --test scripts/ui/
import { test, before, after } from "node:test";
import assert from "node:assert/strict";
import { launch, auditPage, deadControls } from "./audit.mjs";

let browser;
before(async () => { browser = await launch(); });
after(async () => { await browser.close(); });

const html = (body) => "data:text/html;charset=utf-8," + encodeURIComponent(`<!doctype html><meta name=viewport content="width=device-width"><body style="margin:0">${body}</body>`);
const audit = async (body, opts = {}) => {
  const page = await browser.newPage({ viewport: { width: 1000, height: 700 } });
  await page.goto(html(body));
  const found = await auditPage(page, "fixture", opts);
  await page.close();
  return found;
};
const rules = (found) => found.map((f) => f.rule);

test("a clean page has no findings", async () => {
  const found = await audit(`<main><h1>Records</h1><button>Save</button><a href="/x">Open</a></main>`);
  assert.deepEqual(found, []);
});

test("an icon and a leading plus on one button is a doubled plus", async () => {
  const found = await audit(`<button><svg width="10" height="10"></svg>+ New App</button><button>+ Add</button>`);
  const doubled = found.filter((f) => f.rule === "doubled-symbol");
  assert.equal(doubled.length, 1);
  assert.match(doubled[0].detail, /New App/);
});

test("a control with no name is found, and one with a label or title is not", async () => {
  const found = await audit(`<button></button><button><svg width="8" height="8"></svg></button><button aria-label="Close"><svg width="8" height="8"></svg></button><button title="Help"></button>`);
  assert.equal(found.filter((f) => f.rule === "unnamed-control").length, 2);
});

test("placeholder text shows up", async () => {
  const found = await audit(`<p>Total: undefined</p><p>Owner: NaN</p><p>Lorem ipsum</p>`);
  assert.equal(found.filter((f) => f.rule === "placeholder-text").length, 3);
});

test("overlays that overlap, that sit off screen, and too many at once", async () => {
  const box = (css, text) => `<div style="position:fixed;${css};width:200px;height:50px;background:#eee">${text}</div>`;
  const found = await audit(
    box("top:10px;left:10px", "Toast A") + box("top:20px;left:20px", "Toast B") + box("top:300px;left:-500px", "Lost") +
    box("top:400px;left:300px", "C") + box("top:500px;left:300px", "D")
  );
  assert.ok(rules(found).includes("overlay-overlap"), JSON.stringify(found));
  assert.ok(rules(found).includes("overlay-offscreen"), JSON.stringify(found));
  assert.ok(rules(found).includes("too-many-overlays"), JSON.stringify(found));
});

test("a page wider than the window", async () => {
  const found = await audit(`<div style="width:3000px">wide</div>`);
  assert.ok(rules(found).includes("horizontal-overflow"));
});

test("clipped text with no title is found, and with a title is not", async () => {
  const clip = (extra) => `<div ${extra} style="width:50px;overflow:hidden;text-overflow:ellipsis;white-space:nowrap">A very long label that cannot fit</div>`;
  assert.equal((await audit(clip(""))).filter((f) => f.rule === "clipped-text").length, 1);
  assert.equal((await audit(clip('title="A very long label that cannot fit"'))).filter((f) => f.rule === "clipped-text").length, 0);
});

test("the same two-word button twice on one screen", async () => {
  const found = await audit(`<button>Add Record</button><button>Add Record</button><button>Open</button><button>Open</button>`);
  const dup = found.filter((f) => f.rule === "duplicate-control");
  assert.equal(dup.length, 1, "a one-word row action repeated is normal");
  assert.match(dup[0].detail, /Add Record/);
});

test("a button that does nothing is found, and one that changes the page is not", async () => {
  const page = await browser.newPage({ viewport: { width: 1000, height: 700 } });
  const url = html(`<button>Does nothing</button><button onclick="document.body.append('changed')">Does something</button><button onclick="return false" disabled>Off</button>`);
  const found = await deadControls(page, url, { settle: 100 });
  await page.close();
  assert.deepEqual(found.map((f) => f.detail), ["Does nothing"]);
});

test("a button covered by an overlay cannot be clicked", async () => {
  const page = await browser.newPage({ viewport: { width: 1000, height: 700 } });
  const url = html(`<button style="position:absolute;top:10px;left:10px" onclick="document.body.append('x')">Covered</button><div style="position:fixed;top:0;left:0;width:300px;height:100px;background:#ccc;z-index:10">Banner</div>`);
  const found = await deadControls(page, url, { settle: 100, clickTimeout: 800 });
  await page.close();
  assert.ok(found.some((f) => f.rule === "unclickable-control" && f.detail === "Covered"), JSON.stringify(found));
});

test("a popup that a click opens and that sits off screen or covers another is reported against that click", async () => {
  const page = await browser.newPage({ viewport: { width: 1000, height: 700 } });
  const layer = (css, text) => `<div style="position:fixed;${css};width:200px;height:50px;background:#eee">${text}</div>`;
  const markup = layer("top:10px;left:10px", "Saved") + layer("top:20px;left:20px", "Also saved") + layer("top:300px;left:-500px", "Lost");
  const url = html(`<button id="b">Save everything</button><script>document.getElementById("b").onclick = () => document.body.insertAdjacentHTML("beforeend", ${JSON.stringify(markup)});</script>`);
  const stats = {};
  const found = await deadControls(page, url, { settle: 100, stats });
  await page.close();
  const rulesFound = found.map((f) => f.rule);
  assert.ok(rulesFound.includes("overlay-overlap"), JSON.stringify(found));
  assert.ok(rulesFound.includes("overlay-offscreen"), JSON.stringify(found));
  assert.match(found.find((f) => f.rule === "overlay-overlap").detail, /after clicking "Save everything"/);
  assert.equal(stats.clicked, 1);
});

test("a click that opens nothing new adds no findings", async () => {
  const page = await browser.newPage({ viewport: { width: 1000, height: 700 } });
  const found = await deadControls(page, html(`<button onclick="document.body.append('ok')">Fine</button>`), { settle: 100 });
  await page.close();
  assert.deepEqual(found, []);
});

test("a toast on top of a full-screen modal is fine, and one hidden under it is reported", async () => {
  const modal = '<div style="position:fixed;inset:0;background:#ddd;z-index:20">Studio</div>';
  const toast = (z) => `<div style="position:fixed;bottom:20px;right:20px;width:200px;height:50px;background:#222;color:#fff;z-index:${z}">Saved</div>`;
  const on = await audit(modal + toast(50));
  assert.deepEqual(on.filter((f) => f.rule.startsWith("overlay")), [], JSON.stringify(on));
  const under = await audit(modal + toast(10));
  assert.ok(under.some((f) => f.rule === "overlay-hidden" && /Saved/.test(f.detail)), JSON.stringify(under));
});

test("an ordinary notice with role=status that is below the fold is not an overlay", async () => {
  const found = await audit(`<div style="height:2000px">Long page</div><div role="status">Nothing is wrong here</div><div role="alert">Neither here</div>`);
  assert.deepEqual(found.filter((f) => f.rule.startsWith("overlay")), [], JSON.stringify(found));
});
