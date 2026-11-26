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
  fields: [{ name: "code", label: "Code", field_type: "Text", required: true, ferpa_sensitive: false }],
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
      return { body: {} };
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

  it("shows no rows in the catalog, and says where the rows are read (connections phase 1)", async () => {
    serve((p) => (p === "/datasets" ? { body: { total: 1, datasets: [dataset("rooms", "Room Inventory", "Facilities", 17)] } } : { body: {} }));
    render(<DatasetExplorer />);
    const note = await screen.findByTestId("dataset-records-note");
    expect(note.textContent).toContain("17 rows, read-only");
    expect(note.textContent).toMatch(/never its rows/);
    expect(note.textContent).toMatch(/only its sync changes them/);
    // No table of rows, and no sample value such as `rooms-001`.
    expect(document.querySelector("tbody")).toBeNull();
    expect(screen.queryByText("rooms-001")).toBeNull();
  });

  // The catalog's by-value relationships are gone. A link to a dataset is a link field of an app.
  it("asks the server for the datasets only, and offers no relationships tab or lattice view", async () => {
    const asked: string[] = [];
    serve((p) => {
      asked.push(p);
      return { body: { total: 1, datasets: [dataset("rooms", "Room Inventory", "Facilities", 17)] } };
    });
    render(<DatasetExplorer />);
    await screen.findAllByText("Room Inventory");
    expect(asked).toEqual(["/datasets"]);
    expect(screen.queryByText(/Relationships/i)).toBeNull();
    expect(screen.queryByText(/Relational Lattice/i)).toBeNull();
    expect(screen.queryByText(/relation/i)).toBeNull();
    expect(screen.getByRole("button", { name: /Records \(17\)/ })).toBeTruthy();
    expect(screen.getByRole("button", { name: /Schema/ })).toBeTruthy();
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
      return p === "/datasets" ? { body: { total: 1, datasets: [dataset("rooms", "Room Inventory", "Facilities", 17)] } } : { body: {} };
    });
    render(<DatasetExplorer />);
    expect(await screen.findByRole("alert")).toBeTruthy();
    expect(screen.queryByText("Room Inventory")).toBeNull();
    up = true;
    fireEvent.click(screen.getByText("Retry"));
    await waitFor(() => expect(screen.getAllByText("Room Inventory").length).toBeGreaterThan(0));
  });

  // Data layer phase 2d: the catalog.
  const entry = (over: Record<string, unknown> = {}) => ({
    ...dataset("ds_1", "Course Enrollment", "Registrar", 12),
    dataset_id: "ds_1",
    kind: "table",
    unit: { id: null, name: "Chemistry", code: "CHEM" },
    steward: null,
    categories: [{ id: "ferpa", name: "FERPA" }],
    used_by_apps: 2,
    size: { masked: false, approx: 12, label: "about 12 rows" },
    freshness: "2026-11-20",
    sample: { synthetic: true, note: "Made up from the schema. It holds no stored value.", rows: [{ code: "Sample Code 1" }, { code: "Sample Code 2" }] },
    ...over,
  });

  it("asks the server for the words typed, and shows only what it returns", async () => {
    const asked: string[] = [];
    serve((p) => {
      asked.push(p);
      if (p.includes("query=")) return { body: { total: 1, truncated: false, datasets: [entry()], facets: { units: [], functions: [], categories: [] } } };
      return { body: { total: 2, truncated: false, datasets: [entry(), dataset("rooms", "Room Inventory", "Facilities", 17)], facets: { units: [], functions: [], categories: [] } } };
    });
    render(<DatasetExplorer />);
    await screen.findAllByText("Room Inventory");
    fireEvent.change(screen.getByLabelText("Search the catalog"), { target: { value: "final_grade ferpa" } });
    await waitFor(() => expect(asked).toContain("/datasets?query=final_grade%20ferpa"));
    await waitFor(() => expect(screen.queryByText("Room Inventory")).toBeNull());
    expect(screen.getAllByText("Course Enrollment").length).toBeGreaterThan(0);
    expect(await screen.findByText("1 dataset matches")).toBeTruthy();
  });

  it("says so when a search finds nothing, and keeps the search box", async () => {
    serve((p) => ({ body: p.includes("query=") ? { total: 0, truncated: false, datasets: [], facets: { units: [], functions: [], categories: [] } } : { total: 1, truncated: false, datasets: [entry()], facets: { units: [], functions: [], categories: [] } } }));
    render(<DatasetExplorer />);
    await screen.findAllByText("Course Enrollment");
    fireEvent.change(screen.getByLabelText("Search the catalog"), { target: { value: "zebra" } });
    expect(await screen.findByText("No dataset matches")).toBeTruthy();
    expect(screen.getByLabelText("Search the catalog")).toBeTruthy();
    expect(screen.queryByText("No datasets are published yet.")).toBeNull();
  });

  it("shows the metadata of an entry: unit, steward, size, freshness, categories", async () => {
    serve(() => ({ body: { total: 1, truncated: false, datasets: [entry()], facets: { units: [], functions: [], categories: [] } } }));
    render(<DatasetExplorer />);
    await screen.findAllByText("Course Enrollment");
    expect(screen.getByTestId("catalog-unit").textContent).toContain("Chemistry");
    expect(screen.getByTestId("catalog-steward").textContent).toContain("No steward position set");
    expect(screen.getAllByText("about 12 rows").length).toBeGreaterThan(0);
    expect(screen.getByText("2026-11-20")).toBeTruthy();
    expect(screen.getByTestId("catalog-categories").textContent).toContain("FERPA");
  });

  it("shows no number for a small group, and a restricted field as a restricted field", async () => {
    const small = entry({
      record_count: null,
      size: { masked: true, label: "fewer than 11 rows" },
      fields: [{ label: "a restricted field", description: "a restricted field", restricted: true }, { name: "code", label: "Code", field_type: "Text", required: true, ferpa_sensitive: false }],
    });
    serve(() => ({ body: { total: 1, truncated: false, datasets: [small], facets: { units: [], functions: [], categories: [] } } }));
    render(<DatasetExplorer />);
    const note = await screen.findByTestId("dataset-records-note");
    expect(note.textContent).toContain("fewer than 11 rows, read-only");
    fireEvent.click(screen.getByRole("button", { name: /Schema/ }));
    expect(screen.getByText(/a restricted field/)).toBeTruthy();
    expect(screen.getByText("(code)")).toBeTruthy();
  });

  it("shows the synthetic sample on its own tab, labelled as made up", async () => {
    serve(() => ({ body: { total: 1, truncated: false, datasets: [entry()], facets: { units: [], functions: [], categories: [] } } }));
    render(<DatasetExplorer />);
    await screen.findAllByText("Course Enrollment");
    fireEvent.click(screen.getByRole("button", { name: "Sample" }));
    const panel = await screen.findByTestId("catalog-sample");
    expect(panel.textContent).toContain("Made up from the schema");
    expect(panel.textContent).toContain("Sample Code 1");
  });

  it("offers to use a published dataset in an application, and an app table not", async () => {
    const published = entry({ kind: "published", id: "courses", name: "Courses" });
    serve((p) => ({ body: { total: 1, truncated: false, datasets: [p === "/datasets" ? entry() : published], facets: { units: [], functions: [], categories: [] } } }));
    const used = vi.fn();
    const { unmount } = render(<DatasetExplorer onUseInApp={used} />);
    await screen.findAllByText("Course Enrollment");
    expect(screen.queryByText("Use in New Application")).toBeNull();
    unmount();
    serve(() => ({ body: { total: 1, truncated: false, datasets: [published], facets: { units: [], functions: [], categories: [] } } }));
    render(<DatasetExplorer onUseInApp={used} />);
    await screen.findAllByText("Courses");
    fireEvent.click(screen.getAllByText("Use in New Application")[0]);
    expect(used).toHaveBeenCalledWith(expect.objectContaining({ id: "courses", record_count: 12 }));
  });
});
