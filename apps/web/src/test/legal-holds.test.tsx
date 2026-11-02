import "@testing-library/jest-dom";
import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { LegalHolds } from "../admin/LegalHolds";
import { apiClient, type LegalHold } from "../api";

const hold: LegalHold = {
  id: "h1",
  name: "Smith v. State",
  case_ref: "2026-CV-0142",
  reason: "Counsel asked",
  scope_kind: "app",
  app_slug: "admissions",
  scope: "Every record of the app 'admissions'",
  count: 1204,
  created_by: "compliance.officer@state.edu",
  created_at: "2026-11-02T04:00:00Z",
  released_by: null,
  released_at: null,
  active: true,
};

describe("the Legal holds panel", () => {
  beforeEach(() => vi.restoreAllMocks());

  it("lists each hold with its scope in words, its count, and who placed it", async () => {
    vi.spyOn(apiClient, "listHolds").mockResolvedValue([hold]);
    render(<LegalHolds />);
    expect(await screen.findByText("Every record of the app 'admissions'")).toBeInTheDocument();
    expect(screen.getByText("1,204 records")).toBeInTheDocument();
    expect(screen.getByText(/compliance\.officer@state\.edu/)).toBeInTheDocument();
  });

  it("shows the preview sentence before a new hold is placed", async () => {
    vi.spyOn(apiClient, "listHolds").mockResolvedValue([]);
    vi.spyOn(apiClient, "previewHold").mockResolvedValue({ job_id: "j1" });
    vi.spyOn(apiClient, "getAdminJob").mockResolvedValue({ id: "j1", state: "done", result: { records: 1204, apps: 3, sentence: "This hold covers 1,204 records across 3 apps." } } as never);
    const place = vi.spyOn(apiClient, "placeHold").mockResolvedValue(hold);
    render(<LegalHolds />);
    fireEvent.click(await screen.findByRole("button", { name: "New hold" }));
    fireEvent.change(screen.getByLabelText("Name"), { target: { value: "Smith v. State" } });
    fireEvent.change(screen.getByLabelText("App"), { target: { value: "admissions" } });
    fireEvent.change(screen.getByLabelText("Reason"), { target: { value: "Counsel asked" } });
    fireEvent.click(screen.getByRole("button", { name: "Preview" }));
    expect(await screen.findByText("This hold covers 1,204 records across 3 apps.")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Place hold" }));
    await waitFor(() => expect(place).toHaveBeenCalledWith(expect.objectContaining({ scope_kind: "app", app_slug: "admissions", name: "Smith v. State", reason: "Counsel asked" })));
  });

  it("releases through a confirmation that asks for a reason and names the second-person rule", async () => {
    vi.spyOn(apiClient, "listHolds").mockResolvedValue([hold]);
    const release = vi.spyOn(apiClient, "releaseHold").mockResolvedValue(undefined);
    render(<LegalHolds />);
    fireEvent.click(await screen.findByRole("button", { name: "Release" }));
    expect(screen.getByText(/someone other than the person who placed it/)).toBeInTheDocument();
    fireEvent.change(screen.getByRole("textbox", { name: /reason/i }), { target: { value: "The case settled" } });
    fireEvent.click(screen.getByRole("button", { name: /^Release Smith v\. State$|^Confirm/ }));
    await waitFor(() => expect(release).toHaveBeenCalledWith("h1", "The case settled"));
  });
});

import { RecordDetailDrawer } from "../RecordDetailDrawer";
import type { AppTable } from "../types";

describe("the hold marker in the record drawer", () => {
  beforeEach(() => vi.restoreAllMocks());
  const table = { id: "cases", name: "Cases", slug: "cases", primary_field: "title", fields: [{ name: "title", label: "Title", field_type: "Text", required: false, ferpa_sensitive: false }], records: [{ id: "r1", title: "A" }] } as unknown as AppTable;
  const drawer = () => (
    <RecordDetailDrawer selectedRecordId="r1" onClose={() => {}} activeTable={table} tables={[table]} setTables={() => {}} appSlug="demo" onUpdateRecordField={() => {}} onDuplicateRecord={() => {}} onDeleteRecord={() => {}} />
  );

  it("says Under legal hold to someone the server tells", async () => {
    const asked = vi.spyOn(apiClient, "getRecordHold").mockResolvedValue({ held: true, marker: "Under legal hold", holds: ["Smith v. State"] });
    render(drawer());
    expect(await screen.findByText("Under legal hold")).toBeInTheDocument();
    expect(asked).toHaveBeenCalledWith("demo", "r1");
  });

  it("shows nothing when the server says nothing", async () => {
    const asked = vi.spyOn(apiClient, "getRecordHold").mockResolvedValue(null);
    render(drawer());
    await waitFor(() => expect(asked).toHaveBeenCalled());
    expect(screen.queryByText("Under legal hold")).not.toBeInTheDocument();
  });
});
