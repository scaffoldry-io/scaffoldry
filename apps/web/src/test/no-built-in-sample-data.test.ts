import { describe, it, expect } from "vitest";
import { readdirSync, readFileSync } from "node:fs";
import { join } from "node:path";

const src = join(__dirname, "..");
const screens = readdirSync(src).filter((f) => f.endsWith(".tsx"));

/**
 * `AdminDesk.tsx` keeps its INITIAL_* lists as a fallback for when the server's own list does not
 * arrive, and says so in a banner. Every other screen shows what the server holds or nothing.
 */
const FALLBACK_ALLOWED = new Set(["AdminDesk.tsx"]);

describe("no screen carries built-in sample records", () => {
  it("has no SEEDED_ sample constants", () => {
    const hits = screens.filter((f) => /\bSEEDED_[A-Z_]+/.test(readFileSync(join(src, f), "utf8")));
    expect(hits).toEqual([]);
  });

  it("starts no state from a literal array of objects", () => {
    const hits = screens
      .filter((f) => !FALLBACK_ALLOWED.has(f))
      .filter((f) => /useState<[^>]*\[\]>\(\s*\[\s*\{/.test(readFileSync(join(src, f), "utf8")));
    expect(hits).toEqual([]);
  });
});
