import React from "react";
import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import "@testing-library/jest-dom";
import AdminDesk from "../AdminDesk";

describe("Phase 4: Org Unit Admin Workspace Rail", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    localStorage.clear(); localStorage.setItem("scaffoldry_token", "test-token");
  });

  it("fixture Org Unit Admin sees the rail and does not see admin-org-tab-btn", async () => {
    // Stub fetch to return orgs for chair
    global.fetch = vi.fn().mockImplementation((url) => {
      const urlStr = String(url);
      if (urlStr.endsWith("/orgs")) {
        return Promise.resolve({
          ok: true,
          json: async () => [
            {
              id: "dept-phys-id",
              parent_id: "college-sci-id",
              name: "Department of Physics",
              code: "PHYS",
              org_type: "Department",
            },
          ],
        });
      }
      return Promise.resolve({
        ok: true,
        json: async () => [],
      });
    });

    render(<AdminDesk initialPath="/" initialEppn="chair.physics@state.edu" />);

    await waitFor(() => {
      expect(screen.getByTestId("unit-admin-rail")).toBeInTheDocument();
    });

    expect(screen.getByText("Your units")).toBeInTheDocument();
    expect(screen.getByTestId("unit-rail-PHYS")).toBeInTheDocument();
    expect(screen.queryByTestId("admin-org-tab-btn")).not.toBeInTheDocument();

    // Selecting a unit reveals New workspace button
    fireEvent.click(screen.getByTestId("unit-rail-PHYS"));
    expect(screen.getByTestId("unit-new-workspace-btn")).toBeInTheDocument();
  });

  it("student collaborator sees neither unit-admin-rail nor admin-org-tab-btn", async () => {
    global.fetch = vi.fn().mockResolvedValue({
      ok: true,
      json: async () => [],
    });

    render(<AdminDesk initialPath="/" initialEppn="student.smith@science.state.edu" />);

    await waitFor(() => {
      expect(screen.queryByTestId("unit-admin-rail")).not.toBeInTheDocument();
      expect(screen.queryByTestId("admin-org-tab-btn")).not.toBeInTheDocument();
    });
  });
});
