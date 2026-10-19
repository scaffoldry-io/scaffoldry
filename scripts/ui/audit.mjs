// A UI audit that reads the page, not a picture of it. It finds the faults a reviewer finds by eye: a doubled
// plus on a button, a control with no name, text like "undefined", overlays that cover each other or sit off screen,
// a page wider than the window, clipped text with no tooltip, the same button twice, and buttons that do nothing.
// Findings are short lines, so a run costs few tokens. Used by run.mjs, and tested by audit.test.mjs.
import fs from "node:fs";
import { chromium } from "playwright-core";

/** The Chromium to use: CHROME_PATH, else the one preinstalled under PLAYWRIGHT_BROWSERS_PATH or /opt/pw-browsers. */
export function chromePath() {
  if (process.env.CHROME_PATH) return process.env.CHROME_PATH;
  const root = process.env.PLAYWRIGHT_BROWSERS_PATH || "/opt/pw-browsers";
  const dir = fs.readdirSync(root).filter((d) => /^chromium-\d+$/.test(d)).sort().pop();
  if (!dir) throw new Error(`No Chromium under ${root}. Set CHROME_PATH.`);
  return `${root}/${dir}/chrome-linux/chrome`;
}

export const launch = () => chromium.launch({ executablePath: chromePath(), args: ["--no-sandbox"] });

/** Runs inside the page. Returns findings as { rule, detail }. */
function inPage(opts) {
  const out = [];
  const add = (rule, detail) => out.push({ rule, detail: String(detail).replace(/\s+/g, " ").slice(0, 120) });
  const visible = (el) => {
    const r = el.getBoundingClientRect();
    const s = getComputedStyle(el);
    return r.width > 0 && r.height > 0 && s.visibility !== "hidden" && s.display !== "none" && s.opacity !== "0";
  };
  const label = (el) => (el.innerText || "").trim().replace(/\s+/g, " ");
  const controls = [...document.querySelectorAll('button, a[href], [role="button"], input:not([type="hidden"]), select, textarea')].filter(visible);

  // An icon and a leading plus (or close mark) on one button draws the symbol twice.
  for (const el of controls) {
    const text = label(el);
    if (/^[+×✕]/.test(text) && el.querySelector("svg, img")) add("doubled-symbol", `"${text}" has an icon and a leading "${text[0]}"`);
  }

  // A control a screen reader cannot name.
  for (const el of controls) {
    const named = el.getAttribute("aria-label") || el.getAttribute("title") || label(el) || el.getAttribute("alt") ||
      el.querySelector("img[alt]:not([alt=''])") || (el.labels && el.labels.length) || el.getAttribute("placeholder") ||
      (el.tagName === "INPUT" && ["submit", "button"].includes(el.type) && el.value);
    if (!named) add("unnamed-control", el.outerHTML.replace(/\s+/g, " ").replace(/ class="[^"]*"/, "").slice(0, 110));
  }

  // Text that should never reach a person.
  const seen = new Set();
  for (const m of document.body.innerText.matchAll(/(?:^|[^\w])(undefined|NaN|\[object Object\]|lorem ipsum|TODO|coming soon)(?![\w])/gi)) {
    if (seen.has(m[1].toLowerCase())) continue;
    seen.add(m[1].toLowerCase());
    add("placeholder-text", `"${m[1]}" is shown`);
  }

  // Overlays: fixed layers, dialogs, alerts, and raised absolute layers.
  const layers = [...document.querySelectorAll("body *")].filter((el) => {
    if (!visible(el) || !(el.innerText || "").trim()) return false;
    const s = getComputedStyle(el);
    const z = parseInt(s.zIndex, 10);
    // Floating layers only. A notice that sits in the page flow is not an overlay, whatever its role.
    return s.position === "fixed" || (s.position === "absolute" && z >= 30) || (el.matches('[role="dialog"]') && s.position !== "static");
  }).filter((el, _, all) => !all.some((o) => o !== el && o.contains(el)));
  const box = (el) => el.getBoundingClientRect();
  for (const el of layers) {
    const r = box(el);
    const w = Math.max(0, Math.min(r.right, innerWidth) - Math.max(r.left, 0));
    const h = Math.max(0, Math.min(r.bottom, innerHeight) - Math.max(r.top, 0));
    if (w * h < 0.7 * r.width * r.height) add("overlay-offscreen", `"${label(el).slice(0, 40)}" is mostly outside the window`);
  }
  const area = (r) => r.width * r.height;
  const viewport = innerWidth * innerHeight;
  const onTop = (el) => {
    const r = box(el);
    const top = document.elementFromPoint(Math.min(innerWidth - 1, Math.max(0, r.left + r.width / 2)), Math.min(innerHeight - 1, Math.max(0, r.top + r.height / 2)));
    return !!top && (el === top || el.contains(top));
  };
  for (let i = 0; i < layers.length; i++) {
    for (let j = i + 1; j < layers.length; j++) {
      const a = box(layers[i]), b = box(layers[j]);
      const w = Math.min(a.right, b.right) - Math.max(a.left, b.left);
      const h = Math.min(a.bottom, b.bottom) - Math.max(a.top, b.top);
      if (!(w > 0 && h > 0 && w * h > 0.2 * Math.min(area(a), area(b)))) continue;
      const [small, big] = area(a) <= area(b) ? [layers[i], layers[j]] : [layers[j], layers[i]];
      // A small layer on top of a screen-sized one is a toast over a modal. That is normal. Underneath it, it is lost.
      if (area(box(small)) < 0.25 * viewport && area(box(big)) > 0.6 * viewport) {
        if (!onTop(small)) add("overlay-hidden", `"${label(small).slice(0, 40)}" is hidden under "${label(big).slice(0, 30)}"`);
        continue;
      }
      add("overlay-overlap", `"${label(layers[i]).slice(0, 30)}" and "${label(layers[j]).slice(0, 30)}" cover each other`);
    }
  }
  if (layers.length > (opts.maxOverlays ?? 3)) add("too-many-overlays", `${layers.length} overlays are showing at once`);

  // The page scrolls sideways.
  if (document.documentElement.scrollWidth > innerWidth + 1) add("horizontal-overflow", `page is ${document.documentElement.scrollWidth}px wide in a ${innerWidth}px window`);

  // Clipped text with nothing to read it in full.
  let clipped = 0;
  for (const el of document.querySelectorAll("body *")) {
    if (clipped >= 5 || !visible(el) || el.children.length) continue;
    const s = getComputedStyle(el);
    if (s.textOverflow === "ellipsis" && el.scrollWidth > el.clientWidth + 1 && !el.title && !el.closest("[title]") && !el.getAttribute("aria-label")) {
      add("clipped-text", `"${label(el).slice(0, 50)}" is cut off with no tooltip`);
      clipped++;
    }
  }

  // The same multi-word button twice. A one-word row action repeated down a list is normal.
  const names = new Map();
  for (const el of controls.filter((c) => c.matches('button, [role="button"]'))) {
    const n = label(el).toLowerCase().replace(/[^a-z\s]/g, "").replace(/\s+/g, " ").trim();
    if (n.split(" ").length >= 2 && !/\d/.test(label(el))) names.set(n, [...(names.get(n) || []), el]);
  }
  for (const [n, els] of names) if (els.length > 1) add("duplicate-control", `"${label(els[0])}" appears ${els.length} times`);

  return out;
}

/** Findings for the page as it stands now. */
export async function auditPage(page, screen, opts = {}) {
  const found = await page.evaluate(inPage, opts);
  return found.map((f) => ({ screen, ...f }));
}

/**
 * Clicks each button on a fresh load of `url` and reports the ones that change nothing: no new address, no request,
 * no change to the page. A button that cannot be clicked at all is reported too, since something covers it.
 */
export async function deadControls(page, url, { limit = 30, settle = 400, clickTimeout = 1500, deny = /sign out|log ?out|delete|remove|revoke|reset|clear|discard/i, screen = "fixture", prepare, stats = {}, loadWait = 0 } = {}) {
  const out = [];
  // What a click may newly cause. Findings that were already on the page before the click are not the click's.
  const FROM_CLICK = new Set(["overlay-overlap", "overlay-offscreen", "too-many-overlays", "unnamed-control", "placeholder-text", "horizontal-overflow", "doubled-symbol"]);
  const key = (f) => `${f.rule}|${f.detail}`;
  stats.clicked = 0;
  const load = async () => { await page.goto(url); if (prepare) await prepare(page); await page.waitForTimeout(loadWait); };
  const total = Math.min(limit, await (async () => { await load(); return page.locator("button:not([disabled])").count(); })());
  for (let i = 0; i < total; i++) {
    await load();
    const btn = page.locator("button:not([disabled])").nth(i);
    if (!(await btn.isVisible().catch(() => false))) continue;
    const name = ((await btn.innerText().catch(() => "")) || (await btn.getAttribute("aria-label")) || (await btn.getAttribute("title")) || "").trim().replace(/\s+/g, " ");
    if (!name || deny.test(name)) continue;
    let requests = 0;
    const onRequest = () => { requests++; };
    page.on("request", onRequest);
    const snapshot = () => page.evaluate(() => document.body.innerHTML.length + ":" + document.body.innerText.length + ":" + location.href);
    const before = await snapshot();
    const baseline = new Set((await page.evaluate(inPage, {})).map(key));
    try {
      await btn.click({ timeout: clickTimeout });
    } catch {
      page.off("request", onRequest);
      out.push({ screen, rule: "unclickable-control", detail: name });
      continue;
    }
    await page.waitForTimeout(settle);
    page.off("request", onRequest);
    stats.clicked++;
    if (requests === 0 && (await snapshot()) === before) out.push({ screen, rule: "dead-control", detail: name });
    for (const f of await page.evaluate(inPage, {})) {
      if (FROM_CLICK.has(f.rule) && !baseline.has(key(f))) out.push({ screen, rule: f.rule, detail: `${f.detail} (after clicking "${name}")` });
    }
  }
  return out;
}
