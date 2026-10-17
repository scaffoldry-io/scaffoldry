import React from "react";
import { describe, it, expect, vi } from "vitest";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import "@testing-library/jest-dom";
import { ImportWizard } from "../ImportWizard";
import { AppTable } from "../types";

const mockTable: AppTable = {
  id: "tbl-faculty",
  name: "Faculty Directory",
  fields: [
    { name: "name", label: "Full Name", type: "text" },
    { name: "email", label: "Email Address", type: "email" },
    { name: "salary", label: "Salary", type: "number" },
    {
      name: "dept",
      label: "Department",
      type: "select",
      options: ["CS", "Math", "Physics"],
    },
  ],
  records: [],
};

describe("ImportWizard — Phase 1 Import Screen", () => {
  it("renders the 4 steps: Choose a file, Match columns, Check, Import", () => {
    render(
      <ImportWizard
        isOpen={true}
        onClose={vi.fn()}
        activeTable={mockTable}
      />
    );

    expect(screen.getByRole("dialog")).toBeInTheDocument();
    expect(screen.getByText("1. Choose a file")).toBeInTheDocument();
    expect(screen.getByText("2. Match columns")).toBeInTheDocument();
    expect(screen.getByText("3. Check")).toBeInTheDocument();
    expect(screen.getByText("4. Import")).toBeInTheDocument();
  });

  it("Step 1: permits entering CSV text or selecting a file and moves to Match columns", () => {
    render(
      <ImportWizard
        isOpen={true}
        onClose={vi.fn()}
        activeTable={mockTable}
      />
    );

    const textarea = screen.getByTestId("csv-textarea-input");
    fireEvent.change(textarea, {
      target: {
        value: `name,email,salary,dept\nAlice,alice@state.edu,90000,CS\nBob,bob@state.edu,85000,Math`,
      },
    });

    const nextBtn = screen.getByRole("button", { name: /match columns/i });
    fireEvent.click(nextBtn);

    // Step 2: Match columns
    expect(screen.getByText(/2 rows found/i)).toBeInTheDocument();
    expect(screen.getByTestId("csv-map-select-name")).toHaveValue("name");
    expect(screen.getByTestId("csv-map-select-email")).toHaveValue("email");
    expect(screen.getByTestId("csv-map-select-salary")).toHaveValue("salary");
    expect(screen.getByTestId("csv-map-select-dept")).toHaveValue("dept");
  });

  it("Step 2: allows setting upsert key and unknown select option strategy", () => {
    render(
      <ImportWizard
        isOpen={true}
        onClose={vi.fn()}
        activeTable={mockTable}
        initialText={`name,email,salary,dept\nAlice,alice@state.edu,90000,CS`}
      />
    );

    // Advance to Match columns
    fireEvent.click(screen.getByRole("button", { name: /match columns/i }));

    const upsertSelect = screen.getByLabelText(/upsert key/i);
    fireEvent.change(upsertSelect, { target: { value: "email" } });
    expect(upsertSelect).toHaveValue("email");

    const selectStrategy = screen.getByLabelText(/unknown select option strategy/i);
    expect(selectStrategy).toHaveValue("reject");
    fireEvent.change(selectStrategy, { target: { value: "blank" } });
    expect(selectStrategy).toHaveValue("blank");
  });

  it("Step 3: Check (dry run) displays progress bar, validation counts, and errors table", async () => {
    // CSV with one valid row and one row with invalid number and unknown department
    const csvContent = `name,email,salary,dept\nAlice,alice@state.edu,90000,CS\nBob,bob-invalid,not_a_number,Biology`;

    render(
      <ImportWizard
        isOpen={true}
        onClose={vi.fn()}
        activeTable={mockTable}
        initialText={csvContent}
      />
    );

    // Step 1 -> Step 2
    fireEvent.click(screen.getByRole("button", { name: /match columns/i }));

    // Step 2 -> Step 3: Check
    const checkBtn = screen.getByRole("button", { name: /check/i });
    fireEvent.click(checkBtn);

    // Progress bar in Check step
    const progressbar = screen.getByRole("progressbar");
    expect(progressbar).toBeInTheDocument();

    await waitFor(() => {
      expect(screen.getByRole("status")).toHaveTextContent(/validation complete/i);
    });

    // Check errors table
    expect(screen.getByRole("table")).toBeInTheDocument();
    expect(screen.getAllByText("Bob").length).toBeGreaterThan(0);
    expect(screen.getByText(/not a valid number/i)).toBeInTheDocument();
  });

  it("Step 4: Import executes run, shows progress bar, and displays download rejected rows link", async () => {
    const onComplete = vi.fn();
    const csvContent = `name,email,salary,dept\nAlice,alice@state.edu,90000,CS\nBadRow,bad,bad,BadDept`;

    render(
      <ImportWizard
        isOpen={true}
        onClose={vi.fn()}
        activeTable={mockTable}
        initialText={csvContent}
        onImportComplete={onComplete}
      />
    );

    // Move to step 2
    fireEvent.click(screen.getByRole("button", { name: /match columns/i }));
    // Move to step 3 (Check)
    fireEvent.click(screen.getByRole("button", { name: /check/i }));

    await waitFor(() => {
      expect(screen.getByRole("status")).toHaveTextContent(/validation complete/i);
    });

    // Allow errors to proceed
    const acceptErrorsCheckbox = screen.getByLabelText(/skip invalid rows/i);
    fireEvent.click(acceptErrorsCheckbox);

    // Move to step 4 (Import)
    const importBtn = screen.getByRole("button", { name: /^import/i });
    fireEvent.click(importBtn);

    // Step 4 progress bar & rejected rows link
    await waitFor(() => {
      expect(screen.getByRole("status")).toHaveTextContent(/import complete/i);
    });

    const rejectedLink = screen.getByText(/download the rejected rows/i);
    expect(rejectedLink).toBeInTheDocument();
  });

  it("accessibility: closes on Escape and announces progress via ARIA live region", () => {
    const onClose = vi.fn();
    render(
      <ImportWizard
        isOpen={true}
        onClose={onClose}
        activeTable={mockTable}
      />
    );

    // Live region status element exists
    const statusRegion = screen.getByRole("status");
    expect(statusRegion).toBeInTheDocument();
    expect(statusRegion).toHaveAttribute("aria-live", "polite");

    // Press Escape
    fireEvent.keyDown(window, { key: "Escape" });
    expect(onClose).toHaveBeenCalled();
  });
});
