// One command for the cheap checks. Boots a throwaway copy of Scaffoldry and runs:
//   audit  the UI audit over every screen at desktop and phone width, plus a click over the buttons
//   smoke  sign-in, reload, My apps, Datasets, and the admin console, with a line per check
//   mcp    the wire-level MCP check
// Usage: node scripts/ui/run.mjs [audit|smoke|mcp|all] [--strict]
// Output is short. The full audit goes to a JSON file whose path is printed.
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { boot } from "../dev/boot.mjs";
import { mcpSmoke } from "../mcp/smoke.mjs";
import { launch, auditPage, deadControls } from "./audit.mjs";

const mode = process.argv.slice(2).find((a) => !a.startsWith("--")) || "audit";
const strict = process.argv.includes("--strict");
const SIZES = { desktop: { width: 1440, height: 900 }, phone: { width: 390, height: 844 } };

const context = async (browser, size, token, issues) => {
  const ctx = await browser.newContext({ viewport: size });
  if (token) await ctx.addInitScript((t) => localStorage.setItem("scaffoldry_token", t), token);
  const page = await ctx.newPage();
  page.on("console", (m) => { if (m.type() === "error") issues.push(`console: ${m.text().slice(0, 100)}`); });
  page.on("pageerror", (e) => issues.push(`page error: ${String(e).slice(0, 100)}`));
  page.on("response", (r) => { if (r.status() >= 400) issues.push(`HTTP ${r.status()} ${r.request().method()} ${r.url().replace(/^https?:\/\/[^/]+/, "").slice(0, 70)}`); });
  return { ctx, page };
};

async function audit(env, browser) {
  const me = await (await fetch(`${env.api}/me/apps`, { headers: { authorization: `Bearer ${env.token}` } })).json().catch(() => ({}));
  const apps = (me.apps || []).slice(0, 2).map((a) => `/app/${a.slug}`);
  const screens = ["/", "/my-apps", "/datasets", "/admin", "/admin/org", "/admin/ledger", "/admin/jobs", ...apps];
  const findings = [];
  for (const [sizeName, size] of Object.entries(SIZES)) {
    for (const screen of screens) {
      const issues = [];
      const { ctx, page } = await context(browser, size, env.token, issues);
      await page.goto(env.web + screen, { waitUntil: "domcontentloaded" }).catch(() => issues.push("did not settle"));
      await page.waitForTimeout(1500);
      const label = `${screen} (${sizeName})`;
      findings.push(...(await auditPage(page, label)));
      findings.push(...issues.slice(0, 5).map((detail) => ({ screen: label, rule: "request-or-console-error", detail })));
      await ctx.close();
    }
  }
  const stats = { clicked: 0 };
  for (const screen of ["/", "/my-apps", ...apps]) {
    const { ctx, page } = await context(browser, SIZES.desktop, env.token, []);
    const one = {};
    findings.push(...(await deadControls(page, env.web + screen, { screen: `${screen} (desktop)`, stats: one, loadWait: 1500 })));
    stats.clicked += one.clicked || 0;
    await ctx.close();
  }
  const file = path.join(os.tmpdir(), `ui-audit-${Date.now()}.json`);
  fs.writeFileSync(file, JSON.stringify(findings, null, 1));
  const byRule = new Map();
  for (const f of findings) byRule.set(f.rule, [...(byRule.get(f.rule) || []), f]);
  console.log(`UI audit: ${findings.length} findings on ${screens.length} screens at 2 sizes, ${stats.clicked} buttons clicked. Full list: ${file}`);
  for (const [rule, list] of [...byRule].sort((a, b) => b[1].length - a[1].length)) {
    console.log(`${rule} x${list.length}`);
    for (const f of list.slice(0, 3)) console.log(`  ${f.screen}: ${f.detail}`);
  }
  return findings.length === 0;
}

async function smoke(env, browser) {
  const results = [];
  const check = (name, ok, detail = "") => results.push({ name, ok: !!ok, detail });
  const issues = [];
  const text = (page) => page.innerText("body").then((t) => t.replace(/\s+/g, " "));

  let { ctx, page } = await context(browser, SIZES.desktop, null, issues);
  await page.goto(env.web + "/", { waitUntil: "domcontentloaded" });
  await page.waitForTimeout(1200);
  check("a visitor with no token sees the sign-in screen", /Sign in to Scaffoldry/.test(await text(page)));
  await page.fill("input", env.token);
  await page.click("button:has-text('Sign In')");
  await page.waitForTimeout(1500);
  issues.length = 0; // The sign-in screen asks the API before there is a token, and gets 401. Count from here.
  check("signing in with a token opens the desk", /WORKSPACES/i.test(await text(page)));
  check("the desk names the person who signed in", /Setup Identity/.test(await text(page)));
  await page.reload({ waitUntil: "domcontentloaded" });
  await page.waitForTimeout(1500);
  const after = await text(page);
  check("a reload stays signed in", !/Sign in to Scaffoldry/.test(after));
  check("a reload shows the same person, not a built-in persona", /Setup Identity/.test(after) && !/Dr\. Sarah Connor\s+Faculty/.test(after));

  await page.getByRole("button", { name: /^my apps/i }).click();
  await page.waitForTimeout(800);
  check("My apps opens", (await page.getByTestId("my-apps-view").count()) === 1);

  for (const [screen, expect, what] of [["/datasets", /Datasets/i, "Datasets"], ["/admin", /ADMIN CONSOLE/i, "the admin console"]]) {
    await page.goto(env.web + screen, { waitUntil: "domcontentloaded" });
    await page.waitForTimeout(1500);
    const t = await text(page);
    check(`${what} opens`, expect.test(t) && !/403 Forbidden/.test(t));
  }
  check("no console or request errors during the flow", issues.filter((i) => !/HTTP 401/.test(i)).length === 0, issues.filter((i) => !/HTTP 401/.test(i)).slice(0, 2).join("; "));
  await ctx.close();
  return results;
}

async function mcp(env) {
  let url = `${env.api}/mcp`;
  const probe = await fetch(url, { method: "POST", headers: { "content-type": "application/json" }, body: "{}" });
  if (probe.status === 404) url = env.api.replace(/\/api\/v1$/, "") + "/mcp";
  return mcpSmoke({ url, token: env.token });
}

const show = (title, results) => {
  console.log(title);
  for (const r of results) console.log(`${r.ok ? "PASS" : "FAIL"} ${r.name}${r.detail ? ` (${r.detail})` : ""}`);
  return results.every((r) => r.ok);
};

const env = await boot({ web: mode !== "mcp" });
let browser;
let ok = true;
try {
  if (mode === "mcp" || mode === "all") ok = show("MCP:", await mcp(env)) && ok;
  if (mode !== "mcp") browser = await launch();
  if (mode === "smoke" || mode === "all") ok = show("UI smoke:", await smoke(env, browser)) && ok;
  if (mode === "audit" || mode === "all") ok = (await audit(env, browser)) && ok;
} catch (e) {
  console.log(`FAIL harness: ${e.message}`);
  ok = false;
} finally {
  if (browser) await browser.close();
  env.stop();
}
process.exit(ok || !strict && mode === "audit" ? 0 : 1);
