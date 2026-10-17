import React from "react";
import { describe, it, expect, vi, beforeEach } from "vitest";
import { renderHook, act, waitFor } from "@testing-library/react";
import { apiClient, setAuthToken } from "../api";
import { useAppRecords } from "../useAppRecords";

type Call = { method: string; path: string; body?: any };
type Reply = { status?: number; body?: unknown };

/** A fetch stub that answers by method and path, and keeps what it was asked. */
function server(replies: (call: Call) => Reply | undefined) {
  const calls: Call[] = [];
  global.fetch = vi.fn().mockImplementation((url, options: RequestInit = {}) => {
    const path = String(url).replace(/^.*\/api\/v1/, "");
    const call: Call = { method: (options.method || "GET").toUpperCase(), path, body: options.body ? JSON.parse(String(options.body)) : undefined };
    calls.push(call);
    const reply = replies(call) ?? { status: 404, body: { error: "not found" } };
    const status = reply.status ?? 200;
    return Promise.resolve({ ok: status < 400, status, statusText: "", json: async () => reply.body });
  }) as unknown as typeof fetch;
  return calls;
}

const rec = (id: string, name: string, version = 1) => ({ id, data: { _table_id: "t", name }, version });

describe("record calls", () => {
  beforeEach(() => setAuthToken("tok"));

  it("lists, creates, updates with the version, and deletes", async () => {
    const calls = server((c) => {
      if (c.method === "GET" && c.path === "/apps/a/records") return { body: { app_slug: "a", total: 1, records: [rec("r1", "One")] } };
      if (c.method === "POST" && c.path === "/apps/a/records") return { status: 201, body: rec("r2", "Two") };
      if (c.method === "PATCH" && c.path === "/apps/a/records/r1") return { body: rec("r1", "One!", 2) };
      if (c.method === "DELETE" && c.path === "/apps/a/records/r1") return { status: 204 };
    });
    expect((await apiClient.listRecords("a")).map((r) => r.id)).toEqual(["r1"]);
    expect((await apiClient.createRecord("a", { _table_id: "t", name: "Two" })).id).toBe("r2");
    expect((await apiClient.updateRecord("a", "r1", 1, { name: "One!" })).version).toBe(2);
    await apiClient.deleteRecord("a", "r1");
    expect(calls.find((c) => c.method === "POST")?.body).toEqual({ data: { _table_id: "t", name: "Two" } });
    expect(calls.find((c) => c.method === "PATCH")?.body).toEqual({ version: 1, data: { name: "One!" } });
  });
});

describe("useAppRecords", () => {
  beforeEach(() => setAuthToken("tok"));

  it("loads the stored rows and adds a created one", async () => {
    server((c) => {
      if (c.method === "GET") return { body: { records: [rec("r1", "One")] } };
      if (c.method === "POST") return { status: 201, body: rec("r2", "Two") };
    });
    const { result } = renderHook(() => useAppRecords("a"));
    await waitFor(() => expect(result.current.records.map((r) => r.id)).toEqual(["r1"]));
    await act(async () => { await result.current.create({ _table_id: "t", name: "Two" }); });
    expect(result.current.records.map((r) => r.id)).toEqual(["r2", "r1"]);
  });

  it("saves with the version it read and keeps the new version", async () => {
    const calls = server((c) => {
      if (c.method === "GET") return { body: { records: [rec("r1", "One", 4)] } };
      if (c.method === "PATCH") return { body: rec("r1", "Edited", 5) };
    });
    const { result } = renderHook(() => useAppRecords("a"));
    await waitFor(() => expect(result.current.records).toHaveLength(1));
    await act(async () => { await result.current.save("r1", { name: "Edited" }); });
    expect(calls.find((c) => c.method === "PATCH")?.body.version).toBe(4);
    expect(result.current.records[0].version).toBe(5);
    expect(result.current.notice).toBeNull();
  });

  it("on a conflict reloads the row, says so, and does not resend the stale body", async () => {
    const calls = server((c) => {
      if (c.method === "GET" && c.path === "/apps/a/records") return { body: { records: [rec("r1", "Mine", 1)] } };
      if (c.method === "PATCH") return { status: 409, body: { error: "stale", code: "version_conflict" } };
      if (c.method === "GET" && c.path === "/apps/a/records/r1") return { body: rec("r1", "Theirs", 2) };
    });
    const { result } = renderHook(() => useAppRecords("a"));
    await waitFor(() => expect(result.current.records).toHaveLength(1));
    await act(async () => { await result.current.save("r1", { name: "Typed" }); });
    expect(result.current.records[0].data.name).toBe("Theirs");
    expect(result.current.notice).toBe("Someone else saved this record. Reloaded.");
    expect(calls.filter((c) => c.method === "PATCH")).toHaveLength(1);
  });

  it("reports a load failure and does not invent rows", async () => {
    server(() => ({ status: 500, body: { error: "boom" } }));
    const { result } = renderHook(() => useAppRecords("a"));
    await waitFor(() => expect(result.current.error).toBeTruthy());
    expect(result.current.records).toEqual([]);
  });
});
