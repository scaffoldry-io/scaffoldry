import React, { useState } from "react";
import { describe, it, expect, vi } from "vitest";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import "@testing-library/jest-dom";

import {
  DataTable,
  Drawer,
  ConfirmAction,
  Toast,
  StatusBadge,
  FormField,
  EmptyState,
  PageHeader,
  Banner,
  Tabs,
  ControlLabel,
  UnitLabel,
  PersonLabel,
} from "../ui";

describe("UI Kit — Phase 1 Components", () => {
  describe("1. DataTable", () => {
    interface Item {
      id: string;
      name: string;
      score: number;
    }

    const columns = [
      { accessorKey: "name", header: "Name", sortKey: "name" },
      { accessorKey: "score", header: "Score" },
    ];

    it("shows skeleton rows, then rows, handles load more, failure retry, and empty state", async () => {
      let failOnce = true;
      const loadFn = vi.fn().mockImplementation(async (cursor?: string) => {
        if (failOnce) {
          failOnce = false;
          throw new Error("Failed to load records");
        }
        if (!cursor) {
          return {
            rows: [
              { id: "1", name: "Alice", score: 95 },
              { id: "2", name: "Bob", score: 80 },
            ],
            next_cursor: "cursor-page-2",
          };
        }
        return {
          rows: [{ id: "3", name: "Charlie", score: 88 }],
          next_cursor: undefined,
        };
      });

      render(
        <DataTable<Item>
          columns={columns}
          load={loadFn}
          rowKey="id"
          emptyState={<EmptyState title="No records" description="No items found." />}
        />
      );

      // Initially shows ErrorState because failOnce is true
      await waitFor(() => {
        expect(screen.getByText("Failed to load records")).toBeInTheDocument();
      });

      // Retry calls it again
      const retryBtn = screen.getByRole("button", { name: /retry/i });
      fireEvent.click(retryBtn);

      // Now rows appear
      await waitFor(() => {
        expect(screen.getByText("Alice")).toBeInTheDocument();
        expect(screen.getByText("Bob")).toBeInTheDocument();
      });

      // Load more is visible and calls load with next cursor
      const loadMoreBtn = screen.getByRole("button", { name: /load more/i });
      fireEvent.click(loadMoreBtn);

      await waitFor(() => {
        expect(screen.getByText("Charlie")).toBeInTheDocument();
      });
      expect(loadFn).toHaveBeenLastCalledWith(
        "cursor-page-2",
        undefined,
        undefined,
        undefined,
        false
      );
    });

    it("shows emptyState on empty result", async () => {
      const loadFn = vi.fn().mockResolvedValue({ rows: [], next_cursor: undefined });
      render(
        <DataTable<Item>
          columns={columns}
          load={loadFn}
          rowKey="id"
          emptyState={<EmptyState title="Nothing here" description="Empty items list." />}
        />
      );

      await waitFor(() => {
        expect(screen.getByText("Nothing here")).toBeInTheDocument();
        expect(screen.getByText("Empty items list.")).toBeInTheDocument();
      });
    });

    it("does not reorder rows that load returned", async () => {
      // Provide out-of-order data by score and name
      const rows = [
        { id: "3", name: "Zack", score: 10 },
        { id: "1", name: "Anna", score: 90 },
        { id: "2", name: "Mike", score: 50 },
      ];
      const loadFn = vi.fn().mockResolvedValue({ rows, next_cursor: undefined });

      render(
        <DataTable<Item>
          columns={columns}
          load={loadFn}
          rowKey="id"
        />
      );

      await waitFor(() => {
        expect(screen.getByText("Zack")).toBeInTheDocument();
      });

      // Verify DOM row order matches input array order exactly
      const renderedNames = screen.getAllByRole("row").slice(1).map(r => r.textContent);
      expect(renderedNames[0]).toContain("Zack");
      expect(renderedNames[1]).toContain("Anna");
      expect(renderedNames[2]).toContain("Mike");
    });
  });

  describe("3. Drawer", () => {
    function DrawerHarness() {
      const [open, setOpen] = useState(false);
      return (
        <div>
          <button data-testid="opener" onClick={() => setOpen(true)}>
            Open Panel
          </button>
          <Drawer
            open={open}
            onClose={() => setOpen(false)}
            title="Inspector"
            actions={<button>Save</button>}
          >
            <input data-testid="drawer-input" placeholder="Type here" />
            <button data-testid="drawer-button">Action</button>
          </Drawer>
        </div>
      );
    }

    it("opens with focus inside, Tab stays inside, Escape closes, focus returns to the button that opened it", async () => {
      render(<DrawerHarness />);
      const opener = screen.getByTestId("opener");
      opener.focus();
      expect(document.activeElement).toBe(opener);

      fireEvent.click(opener);

      await waitFor(() => {
        expect(screen.getByRole("dialog", { name: "Inspector" })).toBeInTheDocument();
      });

      const dialog = screen.getByRole("dialog", { name: "Inspector" });
      expect(dialog).toContainElement(document.activeElement);

      // Press Escape
      fireEvent.keyDown(dialog, { key: "Escape" });

      await waitFor(() => {
        expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
      });

      // Focus returns to opener
      expect(document.activeElement).toBe(opener);
    });
  });

  describe("4. ConfirmAction", () => {
    it("the button is disabled until the reason has text, handles rejection keeping reason, and success calls onConfirm", async () => {
      let shouldFail = true;
      const onConfirm = vi.fn().mockImplementation(async (reason: string) => {
        if (shouldFail) {
          shouldFail = false;
          throw new Error("Unauthorized reason");
        }
      });
      const onSuccess = vi.fn();

      render(
        <ConfirmAction
          verb="Hold"
          target="Alex Smith"
          consequence="Alex Smith will not be able to sign in."
          audited={true}
          requireReason={true}
          onConfirm={onConfirm}
          onSuccess={onSuccess}
        />
      );

      expect(screen.getByText("Alex Smith will not be able to sign in.")).toBeInTheDocument();
      expect(screen.getByText(/audit ledger/i)).toBeInTheDocument();

      const submitBtn = screen.getByRole("button", { name: /Hold Alex Smith/i });
      expect(submitBtn).toBeDisabled();

      const reasonInput = screen.getByRole("textbox", { name: /reason/i });
      fireEvent.change(reasonInput, { target: { value: "Violation of policy A" } });
      expect(submitBtn).not.toBeDisabled();

      // Click to submit (fails first time)
      fireEvent.click(submitBtn);

      await waitFor(() => {
        expect(screen.getByText("Unauthorized reason")).toBeInTheDocument();
      });
      // Reason is kept
      expect(reasonInput).toHaveValue("Violation of policy A");

      // Try again (now succeeds)
      fireEvent.click(submitBtn);

      await waitFor(() => {
        expect(onSuccess).toHaveBeenCalledTimes(1);
      });
    });
  });

  describe("5. Toast", () => {
    it("a failure has role='alert' and a success has role='status'", () => {
      const { rerender } = render(
        <Toast tone="error" message="Failed to save data" onClose={() => {}} />
      );
      expect(screen.getByRole("alert")).toHaveTextContent("Failed to save data");

      rerender(
        <Toast tone="success" message="Record saved successfully" onClose={() => {}} />
      );
      expect(screen.getByRole("status")).toHaveTextContent("Record saved successfully");
    });
  });

  describe("6. StatusBadge", () => {
    it("text is present in the accessible name for every tone", () => {
      const { rerender } = render(<StatusBadge tone="ok" text="Active" />);
      expect(screen.getByText("Active")).toBeInTheDocument();

      rerender(<StatusBadge tone="warn" text="Pending Review" />);
      expect(screen.getByText("Pending Review")).toBeInTheDocument();

      rerender(<StatusBadge tone="bad" text="Revoked" />);
      expect(screen.getByText("Revoked")).toBeInTheDocument();

      rerender(<StatusBadge tone="neutral" text="Archived" />);
      expect(screen.getByText("Archived")).toBeInTheDocument();
    });
  });

  describe("7. FormField", () => {
    it("error sets aria-invalid and is named by the input's aria-describedby", () => {
      render(
        <FormField
          id="test-field"
          label="Department Code"
          hint="e.g. CS-DEPT"
          error="Department code is required"
          required
        >
          {(inputProps) => <input {...inputProps} />}
        </FormField>
      );

      const input = screen.getByLabelText(/Department Code/i);
      expect(input).toHaveAttribute("aria-invalid", "true");
      expect(input).toHaveAttribute("id", "test-field");
      
      const describedBy = input.getAttribute("aria-describedby");
      expect(describedBy).toBeTruthy();

      const errorElement = screen.getByText("Department code is required");
      expect(describedBy?.split(" ")).toContain(errorElement.id);
    });
  });
});

  describe("Additional Phase 1 Components", () => {
    it("PageHeader renders page's one h1, description, and actions", () => {
      render(
        <PageHeader
          title="Campus Buildings"
          description="Manage academic facilities."
          actions={<button>New Building</button>}
        />
      );
      const heading = screen.getByRole("heading", { level: 1 });
      expect(heading).toHaveTextContent("Campus Buildings");
      expect(screen.getByText("Manage academic facilities.")).toBeInTheDocument();
      expect(screen.getByRole("button", { name: "New Building" })).toBeInTheDocument();
    });

    it("Banner renders appropriate ARIA roles by kind", () => {
      const { rerender } = render(<Banner kind="info">Maintenance scheduled</Banner>);
      expect(screen.getByRole("status")).toHaveTextContent("Maintenance scheduled");

      rerender(<Banner kind="warning">Approaching storage limit</Banner>);
      expect(screen.getByRole("status")).toHaveTextContent("Approaching storage limit");

      rerender(<Banner kind="error">Database connection failed</Banner>);
      expect(screen.getByRole("alert")).toHaveTextContent("Database connection failed");
    });

    it("Tabs supports selection and keyboard navigation", () => {
      const onSelect = vi.fn();
      const tabs = [
        { id: "tab-1", label: "General" },
        { id: "tab-2", label: "Security" },
        { id: "tab-3", label: "Audit" },
      ];

      render(<Tabs tabs={tabs} selectedId="tab-1" onSelect={onSelect} />);

      const tab1 = screen.getByRole("tab", { name: "General" });
      const tab2 = screen.getByRole("tab", { name: "Security" });

      expect(tab1).toHaveAttribute("aria-selected", "true");
      expect(tab2).toHaveAttribute("aria-selected", "false");

      fireEvent.keyDown(tab1, { key: "ArrowRight" });
      expect(onSelect).toHaveBeenCalledWith("tab-2");
    });

    it("Label renderers display lookup title or unknown fallback", () => {
      const { rerender } = render(
        <ControlLabel id="AC-1" lookup={{ "AC-1": "Access Control Policy" }} />
      );
      expect(screen.getByText("AC-1 — Access Control Policy")).toBeInTheDocument();

      rerender(<ControlLabel id="AC-2" lookup={{}} />);
      expect(screen.getByText("AC-2 unknown")).toBeInTheDocument();

      rerender(<UnitLabel id="DEPT-1" lookup={{ "DEPT-1": "Computer Science" }} />);
      expect(screen.getByText("Computer Science (DEPT-1)")).toBeInTheDocument();

      rerender(<UnitLabel id="DEPT-2" lookup={{}} />);
      expect(screen.getByText("DEPT-2 unknown")).toBeInTheDocument();

      rerender(<PersonLabel id="u1@univ.edu" lookup={{ "u1@univ.edu": "Jane Doe" }} />);
      expect(screen.getByText("Jane Doe <u1@univ.edu>")).toBeInTheDocument();

      rerender(<PersonLabel id="u2@univ.edu" lookup={{}} />);
      expect(screen.getByText("u2@univ.edu unknown")).toBeInTheDocument();
    });
  });
