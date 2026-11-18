import React from "react";
import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen } from "@testing-library/react";
import "@testing-library/jest-dom";
import AdminDesk from "../AdminDesk";
import { MultiViewWorkspace } from "../MultiViewWorkspace";
import { setAuthToken } from "../api";
import type { RegisteredApp } from "../types";

const ME = { user: { eppn: "gus@state.edu", name: "Gus", affiliation: "central_admin", department: "Biology" } };

function serve() {
  global.fetch = vi.fn().mockImplementation((url) => {
    const path = String(url).replace(/^.*\/api\/v1/, "").split("?")[0];
    const body = path === "/auth/me" ? ME : path === "/me/apps" ? { apps: [] } : [];
    return Promise.resolve({ ok: true, status: 200, statusText: "", json: async () => body });
  }) as unknown as typeof fetch;
}

function openAt(path: string) {
  localStorage.clear();
  localStorage.setItem("scaffoldry_token", "tok");
  setAuthToken("tok");
  window.history.pushState({}, "", path);
  return render(<AdminDesk initialPath={path} />);
}

describe("phone width (390px)", () => {
  beforeEach(() => serve());

  it("hides the /admin console badge below sm so it cannot cover the logo text", async () => {
    openAt("/admin/connections");
    const badge = await screen.findByText("/admin console");
    expect(badge.className).toMatch(/\bhidden\b/);
    expect(badge.className).toMatch(/\bsm:inline-block\b/);
  });

  it("lets the header's brand side shrink instead of pushing the user menu out", async () => {
    const { container } = openAt("/admin/connections");
    await screen.findByText("/admin console");
    const header = container.querySelector("header")!;
    expect(header.firstElementChild!.className).toMatch(/\bmin-w-0\b/);
    expect(header.lastElementChild!.className).toMatch(/\bshrink-0\b/);
  });

  it("scrolls the status filter inside its own row", async () => {
    openAt("/");
    const label = await screen.findByText("Filter by Status:");
    expect(label.parentElement!.className).toMatch(/\boverflow-x-auto\b/);
  });

  it("scrolls the app's view tabs inside their own row", async () => {
    const field = { name: "n", label: "N", field_type: "Text", required: false, ferpa_sensitive: false };
    const app = {
      slug: "a", title: "A", orgCode: "", department: "", customDomain: "", verified: false, hermCapability: "", cedsDomain: "",
      status: "Draft", updatedAt: "", recordsCount: 0, workspaceId: "w", collaborators: [],
      manifest: { slug: "a", title: "A", description: "", organization_code: "", department: "", custom_domain_verified: false, tables: [{ id: "t", name: "t", slug: "t", fields: [field] }], views: [], ceds_mappings: {} },
    } as unknown as RegisteredApp;
    render(<MultiViewWorkspace app={app} onBack={() => {}} onOpenStudio={() => {}} />);
    const tab = screen.getByText("Intake Form").closest("button")!;
    expect(tab.className).toMatch(/\bshrink-0\b/);
    expect(tab.parentElement!.className).toMatch(/\boverflow-x-auto\b/);
  });
});
