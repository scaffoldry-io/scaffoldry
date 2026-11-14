import "@testing-library/jest-dom";
import { afterEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { DataGovernance } from "../admin/DataGovernance";
import { apiClient, type AdminFieldRow } from "../api";

const row = (over: Partial<AdminFieldRow> = {}): AdminFieldRow => ({
  app: "registrar", app_title: "Registrar", table: "students", field: "program", label: "Program", type: "Text",
  manifest_flag: false, overlay_flag: false, overlay_note: null, effective: false, ceds_code: null, suggested: false,
  used_in: 2, used_in_parts: { views: 1, formulas: 0, pages: 0, rules: 1 }, ...over,
});

const page = (fields: AdminFieldRow[], next: string | null = null) => ({ fields, total: fields.length, next_cursor: next });

afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
});

describe("Data governance, the fields tab", () => {
  it("shows a badge when a field is sensitive only through a label, and when the standard suggests one", async () => {
    vi.spyOn(apiClient, "adminFields").mockResolvedValue(
      page([
        row({ field: "program", effective: true, overlay_flag: true, overlay_note: "Programs are protected." }),
        row({ field: "gpa", label: "GPA", ceds_code: "000127", suggested: true }),
        row({ field: "ssn", label: "Tax number", manifest_flag: true, effective: true }),
      ])
    );
    render(<DataGovernance />);
    const table = await screen.findByRole("table", { name: "Fields" });
    const rows = within(table).getAllByRole("row");
    expect(within(rows[1]).getByText("Sensitive only through a label")).toBeInTheDocument();
    expect(within(rows[2]).getByText("Suggested by the standard")).toBeInTheDocument();
    expect(within(rows[3]).queryByText("Sensitive only through a label")).not.toBeInTheDocument();
  });

  it("asks the server for each filter, and sends the search", async () => {
    const get = vi.spyOn(apiClient, "adminFields").mockResolvedValue(page([row()]));
    render(<DataGovernance />);
    await screen.findByRole("table", { name: "Fields" });
    fireEvent.change(screen.getByLabelText("Show"), { target: { value: "suggested=true" } });
    await waitFor(() => expect(get).toHaveBeenLastCalledWith(expect.objectContaining({ suggested: "true" })));
    fireEvent.change(screen.getByLabelText("Search"), { target: { value: "gpa" } });
    await waitFor(() => expect(get).toHaveBeenLastCalledWith(expect.objectContaining({ search: "gpa", suggested: "true" })));
  });

  it("expands a row to say where the field is used", async () => {
    vi.spyOn(apiClient, "adminFields").mockResolvedValue(page([row()]));
    render(<DataGovernance />);
    fireEvent.click(await screen.findByRole("button", { name: "Where program is used" }));
    expect(screen.getByText(/Views: 1\. Formulas: 0\. Custom pages: 0\. Rules: 1\./)).toBeInTheDocument();
  });

  it("marks the selected fields sensitive with one request and a reason", async () => {
    vi.spyOn(apiClient, "adminFields").mockResolvedValue(page([row(), row({ field: "gpa", label: "GPA" })]));
    const put = vi.spyOn(apiClient, "putLabels").mockResolvedValue({ changed: 2 });
    render(<DataGovernance />);
    await screen.findByRole("table", { name: "Fields" });
    expect(screen.getByRole("button", { name: "Mark sensitive" })).toBeDisabled();
    fireEvent.click(screen.getByLabelText("Select Registrar students program"));
    fireEvent.click(screen.getByLabelText("Select Registrar students gpa"));
    fireEvent.click(screen.getByRole("button", { name: "Mark sensitive" }));
    expect(screen.getByText(/protected now, with no proposal/)).toBeInTheDocument();
    fireEvent.change(screen.getByRole("textbox", { name: /reason/i }), { target: { value: "Grades are protected." } });
    fireEvent.click(screen.getAllByRole("button", { name: "Mark sensitive 2 fields" }).at(-1) as HTMLElement);
    await waitFor(() =>
      expect(put).toHaveBeenCalledWith(
        [
          { app_slug: "registrar", table_id: "students", field: "program", ferpa_sensitive: true },
          { app_slug: "registrar", table_id: "students", field: "gpa", ferpa_sensitive: true },
        ],
        "Grades are protected."
      )
    );
  });

  it("removes a label only from the selected fields that have one", async () => {
    vi.spyOn(apiClient, "adminFields").mockResolvedValue(page([row({ overlay_flag: true, effective: true }), row({ field: "ssn", manifest_flag: true, effective: true })]));
    const put = vi.spyOn(apiClient, "putLabels").mockResolvedValue({ changed: 1 });
    render(<DataGovernance />);
    await screen.findByRole("table", { name: "Fields" });
    fireEvent.click(screen.getByLabelText("Select Registrar students program"));
    fireEvent.click(screen.getByLabelText("Select Registrar students ssn"));
    fireEvent.click(screen.getByRole("button", { name: "Remove label" }));
    fireEvent.change(screen.getByRole("textbox", { name: /reason/i }), { target: { value: "No longer needed." } });
    fireEvent.click(screen.getAllByRole("button", { name: "Remove label from 1 fields" }).at(-1) as HTMLElement);
    await waitFor(() => expect(put).toHaveBeenCalledWith([{ app_slug: "registrar", table_id: "students", field: "program", ferpa_sensitive: null }], "No longer needed."));
  });
});

describe("Data governance, the datasets tab", () => {
  it("saves a classification only with a different level and a reason", async () => {
    vi.spyOn(apiClient, "adminFields").mockResolvedValue(page([]));
    vi.spyOn(apiClient, "adminDatasets").mockResolvedValue({
      datasets: [{ id: "d1", name: "Enrolment", department: "Registrar", sensitivity_level: "Directory", record_count: 120 }],
      levels: ["Public", "Directory", "Internal", "Restricted / FERPA"],
    });
    const save = vi.spyOn(apiClient, "classifyDataset").mockResolvedValue(undefined);
    render(<DataGovernance />);
    fireEvent.click(await screen.findByRole("tab", { name: "Datasets" }));
    const button = await screen.findByRole("button", { name: "Save Enrolment" });
    expect(button).toBeDisabled();
    fireEvent.change(screen.getByLabelText("Classification of Enrolment"), { target: { value: "Restricted / FERPA" } });
    expect(button).toBeDisabled();
    fireEvent.change(screen.getByLabelText("Reason for Enrolment"), { target: { value: "It holds grades." } });
    expect(button).toBeEnabled();
    fireEvent.click(button);
    await waitFor(() => expect(save).toHaveBeenCalledWith("d1", "Restricted / FERPA", "It holds grades."));
  });
});
