import React from "react";
import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, waitFor, fireEvent } from "@testing-library/react";
import { setAuthToken } from "../api";
import { DatasetExplorer } from "../DatasetExplorer";

const dataset = (id: string, name: string, department: string, rows: number) => ({
  id,
  name,
  description: `${name} description`,
  department,
  organization: "Org",
  sensitivity_level: "Public",
  herm_capability_id: "X",
  record_count: rows,
  published_at: "2026-10-01T00:00:00Z",
  relationship_count: 0,
  fields: [{ name: "code", label: "Code", field_type: "Text", required: true, ferpa_sensitive: false }],
  sample_data: [{ code: `${id}-001` }],
});

function serve(handler: (path: string) => { status?: number; body: unknown }) {
  global.fetch = vi.fn().mockImplementation((url) => {
    const path = String(url).replace(/^.*\/api\/v1/, "");
    const r = handler(path);
    const status = r.status ?? 200;
    return Promise.resolve({ ok: status < 400, status, statusText: "", json: async () => r.body });
  }) as unknown as typeof fetch;
}

describe("the Datasets screen", () => {
  beforeEach(() => setAuthToken("tok"));

  it("shows the datasets the server holds, with its counts, and none of the built-in samples", async () => {
    serve((p) => {
      if (p === "/datasets") return { body: { total: 2, datasets: [dataset("rooms", "Room Inventory", "Facilities", 17), dataset("fleet", "Vehicle Fleet", "Transport", 4)] } };
      return { body: { relationships: [] } };
    });
    render(<DatasetExplorer />);
    expect((await screen.findAllByText("Room Inventory")).length).toBeGreaterThan(0);
    expect(screen.getAllByText("Vehicle Fleet").length).toBeGreaterThan(0);
    expect(screen.queryByText(/Faculty & Principal Investigator Directory/)).toBeNull();
    expect(screen.queryByText(/Total Live Rows: 840/)).toBeNull();
    // The department filter lists the server's departments, not a built-in list.
    const options = Array.from(document.querySelectorAll("select option")).map((o) => o.textContent);
    expect(options).toEqual(expect.arrayContaining(["Facilities", "Transport"]));
    expect(options).not.toContain("Office of the Registrar");
  });

  it("says so when the server holds no datasets", async () => {
    serve(() => ({ body: { total: 0, datasets: [] } }));
    render(<DatasetExplorer />);
    expect(await screen.findByText("No datasets are published yet.")).toBeTruthy();
  });

  it("shows an error with Retry when the load fails, and loads on Retry", async () => {
    let up = false;
    serve((p) => {
      if (!up) return { status: 500, body: { error: "down" } };
      return p === "/datasets" ? { body: { total: 1, datasets: [dataset("rooms", "Room Inventory", "Facilities", 17)] } } : { body: { relationships: [] } };
    });
    render(<DatasetExplorer />);
    expect(await screen.findByRole("alert")).toBeTruthy();
    expect(screen.queryByText("Room Inventory")).toBeNull();
    up = true;
    fireEvent.click(screen.getByText("Retry"));
    await waitFor(() => expect(screen.getAllByText("Room Inventory").length).toBeGreaterThan(0));
  });
});
