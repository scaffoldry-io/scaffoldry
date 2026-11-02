import { getAuthToken, getBaseUrl } from "./api";

/** One server-sent event from `GET /apps/{slug}/events`: `change`, `resync`, `ping` or `closed`. */
export interface LiveEvent {
  event: string;
  data: any;
}

/** Splits a buffer into whole events. What follows the last blank line is returned as `rest`, to be completed. */
export function parseEvents(buffer: string): { events: LiveEvent[]; rest: string } {
  const text = buffer.replace(/\r\n/g, "\n");
  const blocks = text.split("\n\n");
  const rest = blocks.pop() ?? "";
  const events: LiveEvent[] = [];
  for (const block of blocks) {
    let event = "message";
    const data: string[] = [];
    for (const line of block.split("\n")) {
      if (line.startsWith("event:")) event = line.slice(6).trim();
      else if (line.startsWith("data:")) data.push(line.slice(5).trim());
    }
    if (data.length === 0 && event === "message") continue;
    let parsed: any = {};
    try {
      parsed = data.length ? JSON.parse(data.join("\n")) : {};
    } catch {
      parsed = {};
    }
    events.push({ event, data: parsed });
  }
  return { events, rest };
}

/**
 * Follows the changes to one app. `fetch` with a streaming body, so the token travels in the `Authorization` header
 * as on every other call. A stream that ends or fails is reopened with backoff, from 1 to 30 seconds. After a
 * reopen the view is sent `resync`, because changes made meanwhile were not heard. A `closed` event ends it: the
 * reader may no longer read the app. Returns the function that stops it.
 */
export function subscribe(appSlug: string, onEvent: (e: LiveEvent) => void, options: { firstDelayMs?: number } = {}): () => void {
  const first = options.firstDelayMs ?? 1000;
  let stopped = false;
  let controller: AbortController | null = null;
  let timer: ReturnType<typeof setTimeout> | null = null;
  let delay = first;
  let lost = false;

  const connect = async () => {
    if (stopped) return;
    controller = new AbortController();
    const token = getAuthToken();
    let closed = false;
    try {
      const response = await fetch(`${getBaseUrl()}/apps/${encodeURIComponent(appSlug)}/events`, {
        headers: { Accept: "text/event-stream", ...(token ? { Authorization: `Bearer ${token}` } : {}) },
        signal: controller.signal,
      });
      // No access is not a reason to keep asking.
      if (response.status === 401 || response.status === 403 || response.status === 404) return;
      if (!response.ok || !response.body) throw new Error(`HTTP ${response.status}`);
      if (lost) onEvent({ event: "resync", data: {} });
      lost = false;
      delay = first;
      const reader = response.body.getReader();
      const decoder = new TextDecoder();
      let buffer = "";
      for (;;) {
        const { value, done } = await reader.read();
        if (done) break;
        buffer += decoder.decode(value, { stream: true });
        const { events, rest } = parseEvents(buffer);
        buffer = rest;
        for (const e of events) {
          if (stopped) return;
          if (e.event === "closed") closed = true;
          onEvent(e);
        }
        if (closed) return;
      }
    } catch {
      // Ended or failed: reopen below, unless this was the stop.
    }
    if (stopped || closed) return;
    lost = true;
    timer = setTimeout(() => void connect(), delay);
    delay = Math.min(delay * 2, 30_000);
  };

  void connect();
  return () => {
    stopped = true;
    if (timer) clearTimeout(timer);
    controller?.abort();
  };
}
