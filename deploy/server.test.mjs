import { test, before, after } from "node:test";
import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import http from "node:http";

const PORT = 18431;
let root, child;

const get = (rawPath) =>
  new Promise((resolve, reject) => {
    // http.request sends the path as given, so "/../" reaches the server unnormalised.
    http.get({ host: "127.0.0.1", port: PORT, path: rawPath }, (res) => {
      let body = "";
      res.on("data", (c) => (body += c));
      res.on("end", () => resolve({ status: res.statusCode, body }));
    }).on("error", reject);
  });

before(async () => {
  root = fs.mkdtempSync(path.join(os.tmpdir(), "desk-"));
  fs.mkdirSync(path.join(root, "dist"));
  fs.writeFileSync(path.join(root, "dist", "index.html"), "INDEX");
  fs.writeFileSync(path.join(root, "dist", "app.js"), "APP");
  fs.writeFileSync(path.join(root, "secret.txt"), "SECRET");
  child = spawn("node", [new URL("./server.mjs", import.meta.url).pathname], {
    env: { ...process.env, PORT: String(PORT), DIST_DIR: path.join(root, "dist") },
    stdio: "ignore",
  });
  for (let i = 0; i < 50; i++) {
    try { await get("/"); return; } catch { await new Promise((r) => setTimeout(r, 100)); }
  }
  throw new Error("server did not start");
});

after(() => {
  child.kill();
  fs.rmSync(root, { recursive: true, force: true });
});

test("serves a file inside the web root", async () => {
  assert.equal((await get("/app.js")).body, "APP");
});

test("unknown paths fall back to index.html", async () => {
  assert.equal((await get("/some/route")).body, "INDEX");
});

test("a path that climbs out of the web root never returns outside files", async () => {
  for (const p of ["/../secret.txt", "/../../secret.txt", "/%2e%2e/secret.txt", "/..%2fsecret.txt"]) {
    const r = await get(p);
    assert.ok(!r.body.includes("SECRET"), p);
  }
});
