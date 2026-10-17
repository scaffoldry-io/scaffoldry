// Boots a throwaway copy of Scaffoldry: a new database, seeded, the API server on 8080, and the web server on 5173.
// Ports are fixed because the web app's proxy and the API's allowed origin name them. Everything is stopped and the
// database is dropped by stop(). Needs the server binary (cargo build -p scaffoldry-server) and psql.
import { spawn, spawnSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../..");
const DEFAULT_URL = "postgres://scaffoldry:scaffoldry_dev_password@127.0.0.1:5433/scaffoldry";

async function waitFor(url, what, ms = 60000) {
  const end = Date.now() + ms;
  while (Date.now() < end) {
    try { await fetch(url); return; } catch { await new Promise((r) => setTimeout(r, 300)); }
  }
  throw new Error(`${what} did not answer at ${url} within ${ms / 1000}s`);
}

async function free(port) {
  try { await fetch(`http://127.0.0.1:${port}/`); } catch { return; }
  throw new Error(`Port ${port} is already in use. Stop whatever holds it, then run again.`);
}

export async function boot({ web = true } = {}) {
  const base = new URL(process.env.DATABASE_URL || DEFAULT_URL);
  const name = `ui_${Date.now().toString(36)}`;
  const admin = new URL(base); admin.pathname = "/postgres";
  const dbUrl = new URL(base); dbUrl.pathname = `/${name}`;
  const env = { ...process.env, PGPASSWORD: decodeURIComponent(base.password) };
  const psql = (sql) => {
    const r = spawnSync("psql", [admin.toString(), "-v", "ON_ERROR_STOP=1", "-c", sql], { env, encoding: "utf8" });
    if (r.status !== 0) throw new Error(`psql failed: ${r.stderr || r.stdout}`);
  };
  const bin = path.join(root, "target/debug/scaffoldry-server");
  if (!fs.existsSync(bin)) throw new Error(`No server binary at ${bin}. Run: cargo build -p scaffoldry-server`);
  await free(8080);
  if (web) await free(5173);

  psql(`CREATE DATABASE ${name}`);
  const serverEnv = { ...process.env, DATABASE_URL: dbUrl.toString(), HOST: "127.0.0.1", PORT: "8080" };
  const run = (...args) => spawnSync(bin, args, { env: serverEnv, encoding: "utf8" });
  const seeded = run("seed-demo");
  if (seeded.status !== 0) { psql(`DROP DATABASE IF EXISTS ${name}`); throw new Error(`seed-demo failed: ${seeded.stderr}`); }
  const token = run("setup-token").stdout.trim().split("\n").pop();

  const children = [];
  // Each child leads its own process group, so stopping it also stops what npx started.
  const start = (cmd, args, opts) => { const c = spawn(cmd, args, { stdio: "ignore", detached: true, ...opts }); children.push(c); return c; };
  start(bin, [], { env: serverEnv });
  if (web) start("npx", ["vite", "--host", "127.0.0.1", "--port", "5173", "--strictPort"], { cwd: path.join(root, "apps/web"), env: process.env });

  const stop = () => {
    for (const c of children) { try { process.kill(-c.pid, "SIGTERM"); } catch { /* already gone */ } }
    try { psql(`DROP DATABASE IF EXISTS ${name} WITH (FORCE)`); } catch { /* the server may still hold it for a moment */ }
  };
  try {
    await waitFor("http://127.0.0.1:8080/api/v1/auth/me", "the API server");
    if (web) await waitFor("http://127.0.0.1:5173/", "the web server");
  } catch (e) { stop(); throw e; }
  return { api: "http://127.0.0.1:8080/api/v1", web: "http://127.0.0.1:5173", token, stop };
}
