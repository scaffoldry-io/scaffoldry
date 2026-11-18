import React from "react";
import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
import "@testing-library/jest-dom";
import { MultiViewWorkspace } from "../MultiViewWorkspace";
import { setAuthToken } from "../api";
import type { RegisteredApp } from "../types";

const five = ["a", "b", "c", "d", "e"].map((n) => ({
  name: n, label: `Field ${n.toUpperCase()}`, field_type: "Text", required: false, ferpa_sensitive: false,
}));

function seededApp(): RegisteredApp {
  return {
    slug: "seeded", title: "Seeded", orgCode: "BIO", department: "Biology", customDomain: "", verified: false,
    hermCapability: "", cedsDomain: "", status: "Published", updatedAt: "", recordsCount: 0, workspaceId: "ws-1", collaborators: [],
    manifest: {
      slug: "seeded", title: "Seeded", description: "", organization_code: "BIO", department: "Biology", custom_domain_verified: false,
      tables: [{ id: "t", name: "t", slug: "t", fields: five }],
      views: [],
      ceds_mappings: {},
    },
  } as unknown as RegisteredApp;
}

const rows = [{ id: "r1", data: { _table_id: "t", a: "Alpha", b: "Bravo", c: "x", d: "y", e: "z" }, version: 1 }];

function serve() {
  global.fetch = vi.fn().mockImplementation((url) => {
    const path = String(url).replace(/^.*\/api\/v1/, "");
    const body = /\/records$/.test(path) ? { records: rows } : {};
    return Promise.resolve({ ok: true, status: 200, statusText: "", json: async () => body });
  }) as unknown as typeof fetch;
}

const open = () => render(<MultiViewWorkspace app={seededApp()} onBack={() => {}} onOpenStudio={() => {}} />);

describe("an app with no saved view", () => {
  beforeEach(() => {
    setAuthToken("tok");
    serve();
  });

  it("shows the table's fields and its records in the grid", async () => {
    open();
    expect(await screen.findByText("Alpha")).toBeInTheDocument();
    expect(screen.getByText(/5 schema fields/)).toBeInTheDocument();
    for (const n of ["A", "B", "C", "D", "E"]) expect(screen.getAllByText(`Field ${n}`).length).toBeGreaterThan(0);
  });

  it("uses the same fields in the gallery and the intake form", async () => {
    open();
    await screen.findByText("Alpha");
    fireEvent.click(screen.getByText("Gallery"));
    expect(screen.getByText("Field C:")).toBeInTheDocument();
    fireEvent.click(screen.getByText("Intake Form"));
    expect(screen.getByText("Field E")).toBeInTheDocument();
  });
});
