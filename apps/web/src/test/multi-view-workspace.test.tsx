import React from "react";
import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import "@testing-library/jest-dom";
import { MultiViewWorkspace } from "../MultiViewWorkspace";
import { PublishedAppView } from "../PublishedAppView";
import { setAuthToken } from "../api";
import type { RegisteredApp } from "../types";

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
function server(rows: any[], extra: (c: Call) => { status?: number; body?: unknown } | undefined = () => undefined) {
  const calls: Call[] = [];
  global.fetch = vi.fn().mockImplementation((url, options: RequestInit = {}) => {
    const path = String(url).replace(/^.*\/api\/v1/, "");
    const call: Call = { method: (options.method || "GET").toUpperCase(), path, body: options.body ? JSON.parse(String(options.body)) : undefined };
    calls.push(call);
    const reply = extra(call) ?? (call.method === "GET" && /\/records$/.test(path) ? { body: { records: rows } } : { status: 404, body: {} });
    const status = reply.status ?? 200;
    return Promise.resolve({ ok: status < 400, status, statusText: "", json: async () => reply.body });
  }) as unknown as typeof fetch;
  return calls;
}

const row = (id: string, name: string, amount = 1, version = 1) => ({ id, data: { _table_id: "t", name, amount }, version });
const open = (slug = "reg") => render(<MultiViewWorkspace app={appOf(slug)} onBack={() => {}} onOpenStudio={() => {}} />);

describe("MultiViewWorkspace shows stored rows", () => {
  beforeEach(() => setAuthToken("tok"));

  it("shows what is stored and nothing made up", async () => {
    server([row("r1", "Real Row")]);
    open();
    expect(await screen.findByText("Real Row")).toBeInTheDocument();
    expect(screen.queryByText(/Sample 1|Sample 2/)).not.toBeInTheDocument();
  });

  it("does not show seeded people for an app that happens to share a seeded slug", async () => {
    server([]);
    const { container } = open("admissions-fellowship-eval");
    await waitFor(() => expect(screen.queryByText(/Loading records/i)).not.toBeInTheDocument());
    expect(container.querySelectorAll('[data-testid^="cell-"]')).toHaveLength(0);
  });

  it("says so when the records cannot be loaded, and invents none", async () => {
    server([], (c) => (c.method === "GET" ? { status: 500, body: { error: "boom" } } : undefined));
    open();
    expect(await screen.findByText(/could not be loaded/i)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /retry/i })).toBeInTheDocument();
    expect(screen.queryByText(/Sample 1/)).not.toBeInTheDocument();
  });

  it("adds a row by creating a record on the server", async () => {
    const calls = server([row("r1", "Real Row")], (c) =>
      c.method === "POST" ? { status: 201, body: row("r2", "", 0) } : undefined
    );
    open();
    await screen.findByText("Real Row");
    fireEvent.click(screen.getByText("+ Add Record"));
    await waitFor(() => expect(calls.some((c) => c.method === "POST")).toBe(true));
    const post = calls.find((c) => c.method === "POST")!;
    expect(post.path).toBe("/apps/reg/records");
    expect(post.body.data._table_id).toBe("t");
  });

  it("saves a cell edit with the version it read, and on a conflict reloads the row and says so", async () => {
    const calls = server([row("r1", "Mine", 5, 3)], (c) => {
      if (c.method === "PATCH") return { status: 409, body: { error: "stale", code: "version_conflict" } };
      if (c.method === "GET" && c.path === "/apps/reg/records/r1") return { body: row("r1", "Mine", 9, 4) };
    });
    open();
    await screen.findByText("Mine");
    fireEvent.click(screen.getByTestId("cell-r1-amount"));
    const input = screen.getByRole("spinbutton") as HTMLInputElement;
    fireEvent.change(input, { target: { value: "500" } });
    fireEvent.keyDown(input, { key: "Enter" });
    await waitFor(() => expect(calls.some((c) => c.method === "PATCH")).toBe(true));
    const patch = calls.find((c) => c.method === "PATCH")!;
    expect(patch.body.version).toBe(3);
    expect(patch.body.data.amount).toBe(500);
    expect(await screen.findByText("Someone else saved this record. Reloaded.")).toBeInTheDocument();
    expect(calls.filter((c) => c.method === "PATCH")).toHaveLength(1);
  });
});

describe("PublishedAppView", () => {
  beforeEach(() => setAuthToken("tok"));

  it("shows the app's own rows from the server and holds none of its own", async () => {
    server([row("r1", "Stored Row")]);
    render(<PublishedAppView app={appOf("reg")} />);
    expect(screen.getByTestId("published-app-runtime")).toBeInTheDocument();
    expect(await screen.findByText("Stored Row")).toBeInTheDocument();
    // None of the proposals mock-up that used to be here.
    expect(screen.queryByText(/Quantum Optomechanics|Prof\. Curie|Sample data|committed to sovereign ledger/)).not.toBeInTheDocument();
  });

  it("goes back to the desk and opens the studio for this app", async () => {
    server([]);
    const onBack = vi.fn();
    const onOpenBuilder = vi.fn();
    render(<PublishedAppView app={appOf("reg")} onBackToDesk={onBack} onOpenBuilder={onOpenBuilder} />);
    fireEvent.click(await screen.findByText(/Configure in Studio/));
    expect(onOpenBuilder).toHaveBeenCalledWith("reg");
  });
});
