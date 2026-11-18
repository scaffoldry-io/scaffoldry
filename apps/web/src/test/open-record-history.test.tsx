import React from "react";
import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import "@testing-library/jest-dom";
import { MultiViewWorkspace } from "../MultiViewWorkspace";
import { apiClient, setAuthToken } from "../api";
import type { RegisteredApp } from "../types";

const fields = [
  { name: "name", label: "Name", field_type: "Text", required: false, ferpa_sensitive: false },
  { name: "status", label: "Status", field_type: "Text", required: false, ferpa_sensitive: false },
];

const app = {
  slug: "reg", title: "Register", orgCode: "BIO", department: "Biology", customDomain: "", verified: false,
  hermCapability: "", cedsDomain: "", status: "Published", updatedAt: "", recordsCount: 0, workspaceId: "ws-1", collaborators: [],
  manifest: {
    slug: "reg", title: "Register", description: "", organization_code: "BIO", department: "Biology", custom_domain_verified: false,
    tables: [{ id: "t", name: "t", slug: "t", fields }],
    views: [{ id: "v1", table_id: "t", title: "All", view_type: "Grid", fields }],
    ceds_mappings: {},
  },
} as unknown as RegisteredApp;

const entry = {
  id: 9, record_id: "r1", table_id: "t", version: 3, action: "update",
  changes: { status: { old: "Pending", new: "Admitted" } },
  actor: "rivera@state.edu", actor_name: "Dr. Rivera", actor_kind: "user", token_label: null, at: "2026-03-03T14:03:00Z",
};

describe("opening a record from the app page", () => {
  beforeEach(() => {
    setAuthToken("tok");
    global.fetch = vi.fn().mockImplementation((url) => {
      const path = String(url).replace(/^.*\/api\/v1/, "");
      const body = /\/records$/.test(path) ? { records: [{ id: "r1", data: { _table_id: "t", name: "Row One", status: "Admitted" }, version: 3 }] } : {};
      return Promise.resolve({ ok: true, status: 200, statusText: "", json: async () => body });
    }) as unknown as typeof fetch;
  });

  it("shows the record's history from the server", async () => {
    const history = vi.spyOn(apiClient, "getRecordHistory").mockResolvedValue({ entries: [entry], next_cursor: null });
    render(<MultiViewWorkspace app={app} onBack={() => {}} onOpenStudio={() => {}} />);
    await screen.findByText("Row One");
    fireEvent.click(screen.getByRole("button", { name: /open row one/i }));
    await waitFor(() => expect(history).toHaveBeenCalledWith("reg", "r1"));
    expect(await screen.findByText(/Dr\. Rivera changed Status from Pending to Admitted/)).toBeInTheDocument();
  });

  it("keeps inline editing working", async () => {
    render(<MultiViewWorkspace app={app} onBack={() => {}} onOpenStudio={() => {}} />);
    await screen.findByText("Row One");
    expect(screen.getByTestId("cell-r1-name")).toBeInTheDocument();
  });
});
