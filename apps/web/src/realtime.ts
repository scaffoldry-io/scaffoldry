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

/** Someone with the app open: their address and their name in words. */
export interface Present {
  address: string;
  name: string;
}

export interface PresenceState {
  present: Present[];
  /** This stream's id, which the desk pings with. */
  stream?: string;
}

/**
 * The presence list after one event: the list on connect, then `joined` and `left`. Any other event returns the
 * state unchanged.
 */
export function presenceAfter(state: PresenceState, e: LiveEvent): PresenceState {
  if (e.event !== "presence" || !e.data) return state;
  const d = e.data;
  if (Array.isArray(d.present)) return { present: d.present as Present[], stream: typeof d.stream === "string" ? d.stream : state.stream };
  if (d.joined?.address) {
    if (state.present.some((p) => p.address === d.joined.address)) return state;
    return { ...state, present: [...state.present, d.joined as Present] };
  }
  if (d.left?.address) return { ...state, present: state.present.filter((p) => p.address !== d.left.address) };
  return state;
}

/**
 * Follows the changes to one app. `fetch` with a streaming body, so the token travels in the `Authorization` header
 * as on every other call. A stream that ends or fails is reopened with backoff, from 1 to 30 seconds. After a
 * reopen the view is sent `resync`, because changes made meanwhile were not heard. A `closed` event ends it: the
 * reader may no longer read the app. Returns the function that stops it.
 */
export function subscribe(appSlug: string, onEvent: (e: LiveEvent) => void, options: { firstDelayMs?: number } = {}): () => void {
  return follow(`/apps/${encodeURIComponent(appSlug)}/events`, onEvent, options);
}

const meListeners = new Set<(e: LiveEvent) => void>();
let stopMe: (() => void) | null = null;

/**
 * The signed-in person's own events (`notification`, `job`, `proposal`, and `resync` after a reconnect). One stream
 * is shared by everything on the page that listens. It opens with the first listener and closes with the last.
 */
export function onMeEvent(listener: (e: LiveEvent) => void): () => void {
  meListeners.add(listener);
  if (!stopMe) stopMe = follow("/me/events", (e) => meListeners.forEach((l) => l(e)));
  return () => {
    meListeners.delete(listener);
    if (meListeners.size === 0 && stopMe) {
      stopMe();
      stopMe = null;
    }
  };
}

/** One server-sent event stream under the API, reopened with backoff. */
export function follow(path: string, onEvent: (e: LiveEvent) => void, options: { firstDelayMs?: number } = {}): () => void {
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
      const response = await fetch(`${getBaseUrl()}${path}`, {
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
