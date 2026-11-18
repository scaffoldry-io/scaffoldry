import React from "react";
import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, waitFor, fireEvent } from "@testing-library/react";
import "@testing-library/jest-dom";
import AdminDesk from "../AdminDesk";
import { AppBuilder } from "../AppBuilder";
import { setAuthToken } from "../api";
import type { RegisteredApp } from "../types";

const ME = { user: { eppn: "gus@state.edu", name: "Gus", affiliation: "central_admin", department: "Biology" } };
const field = (name: string) => ({ name, label: name, field_type: "Text", required: false, ferpa_sensitive: false });
const MANIFEST = {
  slug: "alpha", title: "Alpha Register", description: "From the server", organization_code: "BIO", department: "Biology",
  custom_domain_verified: false, views: [], ceds_mappings: {}, workspace_id: "ws-cs-research", version: 4,
  tables: [{ id: "loans", name: "Equipment Loans", slug: "loans", fields: [field("borrower")] }],
};

type Call = { method: string; path: string; body?: any };

function server() {
  const calls: Call[] = [];
  global.fetch = vi.fn().mockImplementation((url, options: RequestInit = {}) => {
    const path = String(url).replace(/^.*\/api\/v1/, "").split("?")[0];
    const method = (options.method || "GET").toUpperCase();
    calls.push({ method, path, body: options.body ? JSON.parse(String(options.body)) : undefined });
    let status = 200;
    let body: unknown = [];
    if (path === "/auth/me") body = ME;
    else if (path === "/me/apps")
      body = { apps: [{ slug: "alpha", title: "Alpha Register", description: "", department: "Biology", workspace_id: "ws-cs-research", scope: "workspace" }] };
    else if (path === "/apps/alpha" && method === "PUT") { status = 202; body = {}; }
    else if (path === "/apps/alpha") body = MANIFEST;
    return Promise.resolve({ ok: status < 400, status, statusText: "", json: async () => body });
  }) as unknown as typeof fetch;
  return calls;
}

describe("opening /builder/:slug directly", () => {
  beforeEach(() => {
    localStorage.clear();
    localStorage.setItem("scaffoldry_token", "tok");
    setAuthToken("tok");
  });

  it("shows the app's real tables, never the built-in sample, and saves the real tables", async () => {
    const calls = server();
    window.history.pushState({}, "", "/builder/alpha");
    render(<AdminDesk initialPath="/builder/alpha" />);
    // While the app loads, nothing from the sample app is drawn.
    expect(screen.queryByText(/Biology Lab Equipment/)).not.toBeInTheDocument();
    fireEvent.click(await screen.findByTestId("tab-btn-data"));
    expect((await screen.findAllByText(/Equipment Loans/)).length).toBeGreaterThan(0);
    expect(screen.queryByText(/Biology Lab Equipment/)).not.toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: /save changes/i }));
    await waitFor(() => expect(calls.some((c) => c.method === "PUT")).toBe(true));
    const put = calls.find((c) => c.method === "PUT")!;
    expect(put.path).toBe("/apps/alpha");
    expect(put.body.tables.map((t: any) => t.name)).toEqual(["Equipment Loans"]);
  });
});

describe("the studio and a placeholder app", () => {
  const appOf = (slug: string, tableName: string): RegisteredApp =>
    ({
      slug, title: tableName, orgCode: "", department: "Biology", customDomain: "", verified: false, hermCapability: "", cedsDomain: "",
      status: "Draft", updatedAt: "", recordsCount: 0, workspaceId: "ws", collaborators: [],
      manifest: { slug, title: tableName, description: "", organization_code: "", department: "Biology", custom_domain_verified: false, views: [], ceds_mappings: {}, version: 1,
        tables: [{ id: slug, name: tableName, slug, fields: [field("x")] }] },
    }) as unknown as RegisteredApp;

  it("re-initialises its tables when the real app arrives, and saves those", async () => {
    const calls = server();
    setAuthToken("tok");
    const props = { onBack: () => {}, onOpenPublishedApp: () => {}, routeSlug: "alpha" };
    const { rerender } = render(<AppBuilder app={appOf("bio-lab-inventory", "Sample Table")} {...props} />);
    rerender(<AppBuilder app={appOf("alpha", "Real Table")} {...props} />);
    expect((await screen.findAllByText(/Real Table/)).length).toBeGreaterThan(0);
    fireEvent.click(screen.getByRole("button", { name: /save changes/i }));
    await waitFor(() => expect(calls.some((c) => c.method === "PUT")).toBe(true));
    expect(calls.find((c) => c.method === "PUT")!.body.tables.map((t: any) => t.name)).toEqual(["Real Table"]);
  });

  it("refuses to save while the loaded app is not the one in the address", async () => {
    const calls = server();
    setAuthToken("tok");
    render(<AppBuilder app={appOf("bio-lab-inventory", "Sample Table")} onBack={() => {}} onOpenPublishedApp={() => {}} routeSlug="alpha" />);
    fireEvent.click(screen.getByRole("button", { name: /save changes/i }));
    expect(await screen.findByText(/not saved/i)).toBeInTheDocument();
    expect(calls.some((c) => c.method === "PUT")).toBe(false);
  });
});
