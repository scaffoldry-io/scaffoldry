import { describe, it, expect, vi, beforeEach } from "vitest";
import { setAuthToken } from "../api";
import { subscribe } from "../realtime";

/** A response whose body yields these chunks, then stays open until aborted. */
function streaming(chunks: string[], signal: AbortSignal) {
  const enc = new TextEncoder();
  return new ReadableStream<Uint8Array>({
    start(controller) {
      for (const c of chunks) controller.enqueue(enc.encode(c));
      signal.addEventListener("abort", () => controller.error(new DOMException("aborted", "AbortError")));
    },
  });
}

describe("subscribe", () => {
  beforeEach(() => setAuthToken("tok"));

  it("opens the app's stream with the token, delivers its events, and stops on unsubscribe", async () => {
    const seen: Array<{ url: string; auth: string | null; signal: AbortSignal }> = [];
    global.fetch = vi.fn().mockImplementation((url: string, init: RequestInit) => {
      const headers = new Headers(init.headers);
      seen.push({ url: String(url), auth: headers.get("Authorization"), signal: init.signal! });
      return Promise.resolve({ ok: true, status: 200, body: streaming(["event: ping\ndata: {}\n\nevent: change\nda", "ta: {\"record\":\"r1\",\"version\":2}\n\n"], init.signal!) });
    }) as unknown as typeof fetch;
    const events: Array<{ event: string; data: any }> = [];
    const stop = subscribe("my app", (e) => events.push(e));
    await vi.waitFor(() => expect(events.map((e) => e.event)).toEqual(["ping", "change"]));
    expect(events[1].data).toEqual({ record: "r1", version: 2 });
    expect(seen[0].url).toMatch(/\/apps\/my%20app\/events$/);
    expect(seen[0].auth).toBe("Bearer tok");
    stop();
    expect(seen[0].signal.aborted).toBe(true);
  });

  it("reconnects after the stream ends, and tells the view to resync", async () => {
    let opens = 0;
    global.fetch = vi.fn().mockImplementation((_url: string, init: RequestInit) => {
      opens += 1;
      if (opens === 1) return Promise.resolve({ ok: false, status: 503, body: null });
      return Promise.resolve({ ok: true, status: 200, body: streaming(["event: ping\ndata: {}\n\n"], init.signal!) });
    }) as unknown as typeof fetch;
    const events: string[] = [];
    const stop = subscribe("a", (e) => events.push(e.event), { firstDelayMs: 1 });
    await vi.waitFor(() => expect(opens).toBe(2));
    await vi.waitFor(() => expect(events).toEqual(["resync", "ping"]));
    stop();
  });
});
