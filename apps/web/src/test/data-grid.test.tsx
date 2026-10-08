import React from "react";
import { describe, it, expect, vi } from "vitest";
import { render, screen, fireEvent, within } from "@testing-library/react";
import "@testing-library/jest-dom";
import { DataGrid } from "../DataGrid";
import { FieldSpec } from "../types";

const mockFields: FieldSpec[] = [
  {
    name: "name",
    label: "Name",
    field_type: "Text",
    required: true,
    ferpa_sensitive: false,
  },
  {
    name: "amount",
    label: "Amount",
    field_type: "Number",
    required: false,
    ferpa_sensitive: false,
  },
  {
    name: "active",
    label: "Active",
    field_type: "Checkbox",
    required: false,
    ferpa_sensitive: false,
  },
  {
    name: "status",
    label: "Status",
    field_type: "Select",
    select_options: ["Draft", "Review", "Approved"],
    required: false,
    ferpa_sensitive: false,
  },
  {
    name: "calculated_total",
    label: "Calculated Total",
    field_type: "Formula",
    formula_expression: "{amount} * 1.05",
    required: false,
    ferpa_sensitive: false,
  },
];

const mockRecords: Record<string, unknown>[] = [
  {
    id: "rec-1",
    name: "Item Alpha",
    amount: 150,
    active: false,
    status: "Draft",
    calculated_total: 157.5,
  },
  {
    id: "rec-2",
    name: "Item Beta",
    amount: 250,
    active: true,
    status: "Approved",
    calculated_total: 262.5,
  },
];

describe("DataGrid Phase 1 — one grid, type-aware edit", () => {
  it("Number cell commits on Enter and ignores Escape", () => {
    const handlePatch = vi.fn();
    render(
      <DataGrid
        fields={mockFields}
        records={mockRecords}
        onPatch={handlePatch}
      />
    );

    // Click to focus amount cell
    const amountCell = screen.getByTestId("cell-rec-1-amount");
    fireEvent.click(amountCell);

    // Number input should be visible
    const input = screen.getByRole("spinbutton") as HTMLInputElement;
    expect(input).toBeInTheDocument();

    // Changing value then pressing Escape cancels without calling onPatch
    fireEvent.change(input, { target: { value: "300" } });
    fireEvent.keyDown(input, { key: "Escape" });
    expect(handlePatch).not.toHaveBeenCalled();

    // Focus again, change value, and press Enter to commit
    fireEvent.click(amountCell);
    const input2 = screen.getByRole("spinbutton") as HTMLInputElement;
    fireEvent.change(input2, { target: { value: "500" } });
    fireEvent.keyDown(input2, { key: "Enter" });
    expect(handlePatch).toHaveBeenCalledWith("rec-1", "amount", 500);
  });

  it("Checkbox toggles without an input of type text", () => {
    const handlePatch = vi.fn();
    render(
      <DataGrid
        fields={mockFields}
        records={mockRecords}
        onPatch={handlePatch}
      />
    );

    const checkboxCell = screen.getByTestId("cell-rec-1-active");
    // Verify no text input exists in the checkbox cell
    expect(checkboxCell.querySelector('input[type="text"]')).toBeNull();

    // Find the checkbox and click it
    const checkbox = checkboxCell.querySelector('input[type="checkbox"]') as HTMLInputElement;
    expect(checkbox).toBeInTheDocument();
    expect(checkbox.checked).toBe(false);

    fireEvent.click(checkbox);
    expect(handlePatch).toHaveBeenCalledWith("rec-1", "active", true);
  });

  it("Select lists select_options", () => {
    const handlePatch = vi.fn();
    render(
      <DataGrid
        fields={mockFields}
        records={mockRecords}
        onPatch={handlePatch}
      />
    );

    const selectCell = screen.getByTestId("cell-rec-1-status");
    fireEvent.click(selectCell);

    const select = selectCell.querySelector("select") as HTMLSelectElement;
    expect(select).toBeInTheDocument();

    const options = Array.from(select.options).map((opt) => opt.value);
    expect(options).toEqual(expect.arrayContaining(["Draft", "Review", "Approved"]));
  });

  it("Formula cell has no textbox", () => {
    const handlePatch = vi.fn();
    render(
      <DataGrid
        fields={mockFields}
        records={mockRecords}
        onPatch={handlePatch}
      />
    );

    const formulaCell = screen.getByTestId("cell-rec-1-calculated_total");
    fireEvent.click(formulaCell);

    // Formula cell must be read-only and contain no textbox/input
    expect(formulaCell.querySelector("input")).toBeNull();
    expect(formulaCell.querySelector("textarea")).toBeNull();

    // Even on Enter or F2 or typing, it should not open an editor or call onPatch
    fireEvent.keyDown(formulaCell, { key: "Enter" });
    fireEvent.keyDown(formulaCell, { key: "F2" });
    fireEvent.keyDown(formulaCell, { key: "a" });
    expect(formulaCell.querySelector("input")).toBeNull();
    expect(handlePatch).not.toHaveBeenCalled();
  });

  it("Tab moves focus to the next field", () => {
    const handlePatch = vi.fn();
    render(
      <DataGrid
        fields={mockFields}
        records={mockRecords}
        onPatch={handlePatch}
      />
    );

    const firstCell = screen.getByTestId("cell-rec-1-name");
    fireEvent.click(firstCell);

    // Press Tab on the focused cell / active input
    const textInput = firstCell.querySelector("input") || firstCell;
    fireEvent.keyDown(textInput, { key: "Tab" });

    // Focus should have moved to the next field: "amount"
    const nextCell = screen.getByTestId("cell-rec-1-amount");
    expect(nextCell).toHaveAttribute("data-focused", "true");
  });
});

describe("DataGrid Phase 2 — rectangle, paste, clear, fill, undo", () => {
  it("Shift+arrow selects two cells. Delete calls onPatch twice.", () => {
    const handlePatch = vi.fn();
    render(
      <DataGrid
        fields={mockFields}
        records={mockRecords}
        onPatch={handlePatch}
      />
    );

    const cell1 = screen.getByTestId("cell-rec-1-name");
    fireEvent.click(cell1);

    // Cancel edit or move with Shift+ArrowRight
    fireEvent.keyDown(cell1, { key: "ArrowRight", shiftKey: true });

    // Now selection covers (rec-1, name) and (rec-1, amount)
    const cell2 = screen.getByTestId("cell-rec-1-amount");
    expect(cell1).toHaveAttribute("data-selected", "true");
    expect(cell2).toHaveAttribute("data-selected", "true");

    // Press Delete to clear selection
    fireEvent.keyDown(cell2, { key: "Delete" });

    expect(handlePatch).toHaveBeenCalledTimes(2);
    expect(handlePatch).toHaveBeenCalledWith("rec-1", "name", "");
    expect(handlePatch).toHaveBeenCalledWith("rec-1", "amount", "");
  });

  it("Paste of 10\\t20 writes those two fields on the focus row", async () => {
    const handlePatch = vi.fn();
    Object.assign(navigator, {
      clipboard: {
        readText: vi.fn().mockResolvedValue("10\t20"),
        writeText: vi.fn().mockResolvedValue(undefined),
      },
    });

    render(
      <DataGrid
        fields={mockFields}
        records={mockRecords}
        onPatch={handlePatch}
      />
    );

    const cell1 = screen.getByTestId("cell-rec-1-name");
    fireEvent.click(cell1);

    // Fire paste or Ctrl+V
    fireEvent.keyDown(cell1, { key: "v", ctrlKey: true });

    // Wait for clipboard promise resolution
    await vi.waitFor(() => {
      expect(handlePatch).toHaveBeenCalledWith("rec-1", "name", "10");
      expect(handlePatch).toHaveBeenCalledWith("rec-1", "amount", 20);
    });
  });

  it("Paste does not write a Formula field", async () => {
    const handlePatch = vi.fn();
    Object.assign(navigator, {
      clipboard: {
        readText: vi.fn().mockResolvedValue("10\tDraft\t999"),
        writeText: vi.fn().mockResolvedValue(undefined),
      },
    });

    render(
      <DataGrid
        fields={mockFields}
        records={mockRecords}
        onPatch={handlePatch}
      />
    );

    // Focus on active (checkbox, col 2), next is status (select, col 3), next is calculated_total (formula, col 4)
    const activeCell = screen.getByTestId("cell-rec-1-active");
    fireEvent.click(activeCell);

    fireEvent.keyDown(activeCell, { key: "v", ctrlKey: true });

    await vi.waitFor(() => {
      expect(handlePatch).toHaveBeenCalledWith("rec-1", "status", "Draft");
    });

    // Make sure formula field is never patched
    const formulaPatches = handlePatch.mock.calls.filter(
      (call) => call[1] === "calculated_total"
    );
    expect(formulaPatches).toHaveLength(0);
  });

  it("Ctrl+Z restores the pre-paste values", async () => {
    let recordsState = [...mockRecords];
    const handlePatch = vi.fn((recId, fieldName, value) => {
      recordsState = recordsState.map((r) =>
        r.id === recId ? { ...r, [fieldName]: value } : r
      );
    });

    Object.assign(navigator, {
      clipboard: {
        readText: vi.fn().mockResolvedValue("NewName\t999"),
        writeText: vi.fn().mockResolvedValue(undefined),
      },
    });

    const { rerender } = render(
      <DataGrid
        fields={mockFields}
        records={recordsState}
        onPatch={handlePatch}
      />
    );

    const cell1 = screen.getByTestId("cell-rec-1-name");
    fireEvent.click(cell1);

    fireEvent.keyDown(cell1, { key: "v", ctrlKey: true });

    await vi.waitFor(() => {
      expect(handlePatch).toHaveBeenCalledWith("rec-1", "name", "NewName");
      expect(handlePatch).toHaveBeenCalledWith("rec-1", "amount", 999);
    });

    rerender(
      <DataGrid
        fields={mockFields}
        records={recordsState}
        onPatch={handlePatch}
      />
    );

    // Press Ctrl+Z to undo the paste gesture
    const currentCell = screen.getByTestId("cell-rec-1-name");
    fireEvent.keyDown(currentCell, { key: "z", ctrlKey: true });

    // Pre-paste values for rec-1 were name: "Item Alpha", amount: 150
    expect(handlePatch).toHaveBeenCalledWith("rec-1", "name", "Item Alpha");
    expect(handlePatch).toHaveBeenCalledWith("rec-1", "amount", 150);
  });
});

describe("DataGrid Phase 4 — column chrome", () => {
  it("Hiding a field removes its header", () => {
    const handlePatch = vi.fn();
    render(
      <DataGrid
        fields={mockFields}
        records={mockRecords}
        hidden_columns={["amount"]}
        onPatch={handlePatch}
      />
    );

    expect(screen.queryByText("Amount")).toBeNull();
    expect(screen.getByText("Name")).toBeInTheDocument();
  });

  it("column_order renders headers in that order", () => {
    const handlePatch = vi.fn();
    render(
      <DataGrid
        fields={mockFields}
        records={mockRecords}
        column_order={["status", "name", "amount", "active", "calculated_total"]}
        onPatch={handlePatch}
      />
    );

    const headers = screen.getAllByRole("columnheader").map((th) => th.textContent?.trim());
    const statusIndex = headers.findIndex((h) => h?.includes("Status"));
    const nameIndex = headers.findIndex((h) => h?.includes("Name"));
    expect(statusIndex).toBeLessThan(nameIndex);
  });

  it("Summary min on a number column shows the minimum of the visible rows", () => {
    const handlePatch = vi.fn();
    render(
      <DataGrid
        fields={mockFields}
        records={mockRecords}
        column_summary={[["amount", "min"]]}
        onPatch={handlePatch}
      />
    );

    const footer = screen.getByTestId("summary-amount");
    expect(footer).toHaveTextContent("150");
  });
});


describe("DataGrid Bulk Cell Selection, Copy & Linked Field Characteristics", () => {
  it("supports bulk mouse drag cell selection and visual selection rectangle", () => {
    const handlePatch = vi.fn();
    render(
      <DataGrid
        fields={mockFields}
        records={mockRecords}
        onPatch={handlePatch}
      />
    );

    const cell1 = screen.getByTestId("cell-rec-1-name");
    const cell2 = screen.getByTestId("cell-rec-2-amount");

    // Initiate drag selection
    fireEvent.mouseDown(cell1);
    fireEvent.mouseEnter(cell2);
    fireEvent.mouseUp(window);

    // Bounding rectangle: (rec-1, name), (rec-1, amount), (rec-2, name), (rec-2, amount)
    expect(screen.getByTestId("cell-rec-1-name")).toHaveAttribute("data-selected", "true");
    expect(screen.getByTestId("cell-rec-1-amount")).toHaveAttribute("data-selected", "true");
    expect(screen.getByTestId("cell-rec-2-name")).toHaveAttribute("data-selected", "true");
    expect(screen.getByTestId("cell-rec-2-amount")).toHaveAttribute("data-selected", "true");

    // Selection status badge & fill handle
    const statusBadge = screen.getByTestId("selection-status-badge");
    expect(statusBadge).toBeInTheDocument();
    expect(statusBadge).toHaveTextContent(/2 × 2 cells selected \(4\)/i);
    expect(screen.getByTestId("grid-fill-handle")).toBeInTheDocument();
  });

  it("copies bulk selected cells as TSV via bulk copy button and keyboard shortcut", async () => {
    const handlePatch = vi.fn();
    const writeTextMock = vi.fn().mockResolvedValue(undefined);
    Object.assign(navigator, {
      clipboard: {
        writeText: writeTextMock,
        readText: vi.fn(),
      },
    });

    render(
      <DataGrid
        fields={mockFields}
        records={mockRecords}
        onPatch={handlePatch}
      />
    );

    const cell1 = screen.getByTestId("cell-rec-1-name");
    const cell2 = screen.getByTestId("cell-rec-2-amount");

    // Select 2x2 cell region
    fireEvent.mouseDown(cell1);
    fireEvent.mouseEnter(cell2);
    fireEvent.mouseUp(window);

    // Click bulk copy button
    const copyBtn = screen.getByTestId("grid-bulk-copy-btn");
    expect(copyBtn).toBeInTheDocument();
    fireEvent.click(copyBtn);

    expect(writeTextMock).toHaveBeenCalled();
    const copiedTsv = writeTextMock.mock.calls[0][0];
    // TSV must contain Item Alpha\t150 and Item Beta\t280
    expect(copiedTsv).toContain("Item Alpha\t150");
    expect(copiedTsv).toContain("Item Beta\t250");
  });

  it("renders linked relation fields with label overrides and filtered record picker", () => {
    const handlePatch = vi.fn();
    const linkedField: FieldSpec = {
      name: "lead_investigator_id",
      label: "Lead Investigator",
      field_type: "Relation",
      required: false,
      ferpa_sensitive: false,
      target_table_id: "tbl-faculty",
      target_display_field: "name",
      display_label_override: "PI: ",
      cardinality: "single",
      link_filter: {
        field: "department",
        operator: "equals",
        value: "Physics",
      },
    };

    const mockTables = [
      {
        id: "tbl-faculty",
        name: "Faculty Directory",
        slug: "faculty",
        fields: [
          { name: "id", label: "ID", field_type: "Text" as const, required: true, ferpa_sensitive: false },
          { name: "name", label: "Name", field_type: "Text" as const, required: true, ferpa_sensitive: false },
          { name: "department", label: "Department", field_type: "Text" as const, required: true, ferpa_sensitive: false },
        ],
        records: [
          { id: "FAC-01", name: "Dr. Marie Curie", department: "Physics" },
          { id: "FAC-02", name: "Dr. Alan Turing", department: "Computer Science" },
        ],
      },
    ];

    const recordsWithLink = [
      { id: "rec-1", name: "Project Alpha", lead_investigator_id: "FAC-01" },
    ];

    render(
      <DataGrid
        fields={[mockFields[0], linkedField]}
        records={recordsWithLink}
        tables={mockTables}
        onPatch={handlePatch}
      />
    );

    // Chip must render with label override "PI: Dr. Marie Curie"
    expect(screen.getByText("PI: Dr. Marie Curie")).toBeInTheDocument();

    // Open link picker
    const pickerBtn = screen.getByTestId("open-link-picker-rec-1-lead_investigator_id");
    fireEvent.click(pickerBtn);

    const picker = screen.getByTestId("link-record-picker-rec-1-lead_investigator_id");
    expect(picker).toBeInTheDocument();
    // Due to filter (department equals Physics), Dr. Marie Curie appears, but Alan Turing is filtered out
    expect(within(picker).getByText("PI: Dr. Marie Curie")).toBeInTheDocument();
    expect(screen.queryByText("PI: Dr. Alan Turing")).not.toBeInTheDocument();
  });
});
