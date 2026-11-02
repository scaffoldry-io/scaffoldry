import React from "react";
import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent, waitFor, act } from "@testing-library/react";
import "@testing-library/jest-dom";
import type { RegisteredApp } from "../types";

// The subscribe double: it keeps the handler each view registers and counts unsubscribes.
const live = vi.hoisted(() => ({
  handlers: [] as Array<(e: { event: string; data: any }) => void>,
  unsubscribed: 0,
}));
vi.mock("../realtime", async (original) => ({
  ...(await original<typeof import("../realtime")>()),
  subscribe: (_slug: string, onEvent: (e: { event: string; data: any }) => void) => {
    live.handlers.push(onEvent);
    return () => {
      live.unsubscribed += 1;
    };
  },
}));

import { MultiViewWorkspace } from "../MultiViewWorkspace";
import { setAuthToken } from "../api";
import { parseEvents } from "../realtime";

const fields = [
  { name: "name", label: "Name", field_type: "Text", required: false, ferpa_sensitive: false },
  { name: "amount", label: "Amount", field_type: "Number", required: false, ferpa_sensitive: false },
];

function appOf(slug: string): RegisteredApp {
  return {
    slug, title: "Register", orgCode: "BIO", department: "Biology", customDomain: "", verified: false,
    hermCapability: "", cedsDomain: "", status: "Published", updatedAt: "", recordsCount: 0, workspaceId: "ws-1", collaborators: [],
    manifest: {
      slug, title: "Register", description: "", organization_code: "BIO", department: "Biology", custom_domain_verified: false,
      tables: [{ id: "t", name: "t", slug: "t", fields }],
      views: [{ id: "v1", table_id: "t", title: "All", view_type: "Grid", fields }],
      ceds_mappings: {},
    },
  } as unknown as RegisteredApp;
}

type Call = { method: string; path: string; body?: any };
type Reply = { status?: number; body?: unknown };

/** A fetch stub. `rows` is what a list answers, and may change between calls. */
function server(rows: () => any[], extra: (c: Call) => Reply | undefined = () => undefined) {
  const calls: Call[] = [];
  global.fetch = vi.fn().mockImplementation((url, options: RequestInit = {}) => {
    const path = String(url).replace(/^.*\/api\/v1/, "");
    const call: Call = { method: (options.method || "GET").toUpperCase(), path, body: options.body ? JSON.parse(String(options.body)) : undefined };
    calls.push(call);
    const reply = extra(call) ?? (call.method === "GET" && /\/records$/.test(path) ? { body: { records: rows() } } : { status: 404, body: {} });
    const status = reply.status ?? 200;
    return Promise.resolve({ ok: status < 400, status, statusText: "", json: async () => reply.body });
  }) as unknown as typeof fetch;
  return calls;
}

const row = (id: string, name: string, amount = 1, version = 1) => ({ id, data: { _table_id: "t", name, amount }, version });
const open = () => render(<MultiViewWorkspace app={appOf("reg")} onBack={() => {}} onOpenStudio={() => {}} />);
const emit = (event: string, data: any = {}) =>
  act(() => {
    for (const h of live.handlers) h({ event, data });
  });
const change = (record: string, version: number, kind = "update") => emit("change", { table: "t", record, version, kind });
const fetchesOf = (calls: Call[], path: string) => calls.filter((c) => c.method === "GET" && c.path === path).length;

describe("the grid listens", () => {
  beforeEach(() => {
    setAuthToken("tok");
    live.handlers.length = 0;
    live.unsubscribed = 0;
  });

  // 1. A change for a visible record triggers one fetch of that record and updates the row.
  it("refetches a visible record once and updates it in place, with a label for screen readers", async () => {
    const calls = server(() => [row("r1", "Before")], (c) => (c.method === "GET" && c.path === "/apps/reg/records/r1" ? { body: row("r1", "After", 1, 2) } : undefined));
    open();
    await screen.findByText("Before");
    change("r1", 2);
    expect(await screen.findByText("After")).toBeInTheDocument();
    expect(fetchesOf(calls, "/apps/reg/records/r1")).toBe(1);
    expect(screen.getByText("Updated by someone else")).toBeInTheDocument();
  });

  it("ignores the echo of this person's own save", async () => {
    const calls = server(() => [row("r1", "Mine")]);
    open();
    await screen.findByText("Mine");
    change("r1", 1);
    await new Promise((r) => setTimeout(r, 20));
    expect(fetchesOf(calls, "/apps/reg/records/r1")).toBe(0);
  });

  // 2. A change outside the window raises the count banner. Refresh reloads and clears it.
  it("counts changes outside the loaded rows, and Refresh reloads and clears the count", async () => {
    let rows = [row("r1", "One")];
    const calls = server(() => rows);
    open();
    await screen.findByText("One");
    change("elsewhere-1", 1, "create");
    change("elsewhere-2", 3);
    expect(await screen.findByText(/2 records changed\./)).toBeInTheDocument();
    rows = [row("r1", "One"), row("elsewhere-1", "New one")];
    const lists = fetchesOf(calls, "/apps/reg/records");
    fireEvent.click(screen.getByRole("button", { name: "Refresh" }));
    expect(await screen.findByText("New one")).toBeInTheDocument();
    expect(fetchesOf(calls, "/apps/reg/records")).toBe(lists + 1);
    expect(screen.queryByText(/records changed\./)).not.toBeInTheDocument();
  });

  // 3. A change to a cell under edit keeps the draft and shows the marker.
  it("keeps the draft of a cell under edit, marks it, and lets the version check decide on save", async () => {
    const calls = server(
      () => [row("r1", "One", 5)],
      (c) => {
        if (c.method === "GET" && c.path === "/apps/reg/records/r1") return { body: row("r1", "One", 9, 2) };
        if (c.method === "PATCH") return { status: 409, body: { error: "stale", code: "version_conflict" } };
      }
    );
    open();
    await screen.findByText("One");
    fireEvent.click(screen.getByTestId("cell-r1-amount"));
    const input = screen.getByRole("spinbutton") as HTMLInputElement;
    fireEvent.change(input, { target: { value: "42" } });
    change("r1", 2);
    expect(await screen.findByText("Changed by someone else")).toBeInTheDocument();
    expect((screen.getByRole("spinbutton") as HTMLInputElement).value).toBe("42");
    fireEvent.keyDown(screen.getByRole("spinbutton"), { key: "Enter" });
    await waitFor(() => expect(calls.some((c) => c.method === "PATCH")).toBe(true));
    // The save names the version the person was editing, not the one that arrived meanwhile.
    expect(calls.find((c) => c.method === "PATCH")!.body.version).toBe(1);
    expect(await screen.findByText(/Someone else changed this first\./)).toBeInTheDocument();
  });

  // 4. `resync` reloads and shows the toast.
  it("reloads on resync and says so", async () => {
    let rows = [row("r1", "Old")];
    const calls = server(() => rows);
    open();
    await screen.findByText("Old");
    const lists = fetchesOf(calls, "/apps/reg/records");
    rows = [row("r1", "Fresh", 1, 4)];
    emit("resync");
    expect(await screen.findByText("Fresh")).toBeInTheDocument();
    expect(fetchesOf(calls, "/apps/reg/records")).toBe(lists + 1);
    expect(screen.getByText(/Reloaded: changes may have been missed\./)).toBeInTheDocument();
  });

  // Phase 2 item 5: a delete removes the row and says so.
  it("removes a row deleted by someone else, with a message", async () => {
    server(() => [row("r1", "Gone soon"), row("r2", "Stays")]);
    open();
    await screen.findByText("Gone soon");
    change("r1", 2, "delete");
    await waitFor(() => expect(screen.queryByText("Gone soon")).not.toBeInTheDocument());
    expect(screen.getByText("Stays")).toBeInTheDocument();
    expect(screen.getByText("Removed by someone else.")).toBeInTheDocument();
  });

  // 5. Leaving the view calls unsubscribe.
  it("unsubscribes when the view closes", async () => {
    server(() => [row("r1", "One")]);
    const view = open();
    await screen.findByText("One");
    expect(live.handlers.length).toBe(1);
    view.unmount();
    expect(live.unsubscribed).toBe(1);
  });
});

describe("this person's own writes are not someone else's", () => {
  beforeEach(() => {
    setAuthToken("tok");
    live.handlers.length = 0;
    live.unsubscribed = 0;
  });

  /** A fetch stub whose writes wait until `release` is called, so an event can arrive first. */
  function slowWrites(rows: any[], answer: (c: Call) => Reply) {
    const calls: Call[] = [];
    let release: () => void = () => {};
    const gate = new Promise<void>((r) => (release = r));
    global.fetch = vi.fn().mockImplementation(async (url, options: RequestInit = {}) => {
      const path = String(url).replace(/^.*\/api\/v1/, "");
      const call: Call = { method: (options.method || "GET").toUpperCase(), path, body: options.body ? JSON.parse(String(options.body)) : undefined };
      calls.push(call);
      const reply = call.method === "GET" && /\/records$/.test(path) ? { body: { records: rows } } : answer(call);
      if (call.method !== "GET") await gate;
      const status = reply.status ?? 200;
      return { ok: status < 400, status, statusText: "", json: async () => reply.body };
    }) as unknown as typeof fetch;
    return { calls, release: () => release() };
  }

  it("a create whose event arrives before its answer is not counted as a change elsewhere", async () => {
    const net = slowWrites([row("r1", "One")], (c) => (c.method === "POST" ? { status: 201, body: row("r2", "", 0) } : { status: 404, body: {} }));
    open();
    await screen.findByText("One");
    fireEvent.click(screen.getByText("+ Add Record"));
    await waitFor(() => expect(net.calls.some((c) => c.method === "POST")).toBe(true));
    change("r2", 1, "create");
    net.release();
    await waitFor(() => expect(screen.getByTestId("cell-r2-name")).toBeInTheDocument());
    expect(screen.queryByText(/changed\./)).not.toBeInTheDocument();
  });

  it("a save whose event arrives before its answer is not marked as someone else's", async () => {
    const net = slowWrites([row("r1", "One", 5)], (c) =>
      c.method === "PATCH" ? { body: row("r1", "One", 6, 2) } : c.path === "/apps/reg/records/r1" ? { body: row("r1", "One", 6, 2) } : { status: 404, body: {} }
    );
    open();
    await screen.findByText("One");
    fireEvent.click(screen.getByTestId("cell-r1-amount"));
    fireEvent.change(screen.getByRole("spinbutton"), { target: { value: "6" } });
    fireEvent.keyDown(screen.getByRole("spinbutton"), { key: "Enter" });
    await waitFor(() => expect(net.calls.some((c) => c.method === "PATCH")).toBe(true));
    change("r1", 2);
    net.release();
    await waitFor(() => expect(screen.getByTestId("cell-r1-amount")).toHaveTextContent("6"));
    expect(screen.queryByText("Updated by someone else")).not.toBeInTheDocument();
  });
});

describe("realtime.ts", () => {
  it("parses server-sent events across chunk boundaries", () => {
    const first = parseEvents("event: change\ndata: {\"record\":\"r1\"}\n\nevent: pi");
    expect(first.events).toEqual([{ event: "change", data: { record: "r1" } }]);
    const second = parseEvents(first.rest + "ng\ndata: {}\n\n");
    expect(second.events).toEqual([{ event: "ping", data: {} }]);
    expect(second.rest).toBe("");
  });
});
