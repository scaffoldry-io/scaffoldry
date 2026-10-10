import "@testing-library/jest-dom";
import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, waitFor, fireEvent } from "@testing-library/react";
import { Workspaces } from "../admin/Workspaces";
import {
  apiClient,
  AdminWorkspaceRow,
  AdminAppRow,
  AdminWorkspaceDetail,
} from "../api";

const mockWorkspaceRow: AdminWorkspaceRow = {
  id: "ws-test-1",
  name: "Genomics Research",
  code: "GR-01",
  unit_name: "School of Medicine",
  classification: "Internal",
  visibility: "departmental",
  owners: ["jordan.lee@state.edu"],
  collaborator_count: 3,
  app_count: 2,
  record_count: 15,
  created: "2026-10-10T00:00:00Z",
  organization_id: "org-med-01",
  description: "Medical genomics data",
};

const mockAppRow: AdminAppRow = {
  slug: "variant-caller",
  title: "Variant Caller",
  workspace: "ws-test-1",
  version: "1.2.0",
  table_count: 2,
  page_count: 4,
  custom_page_count: 1,
  record_count_per_table: { variants: 10, runs: 5 },
  updated: "2026-10-10T01:00:00Z",
};

const mockDetail: AdminWorkspaceDetail = {
  id: "ws-test-1",
  name: "Genomics Research",
  code: "GR-01",
  unit_name: "School of Medicine",
  classification: "Internal",
  visibility: "departmental",
  owners: ["jordan.lee@state.edu"],
  collaborator_count: 2,
  app_count: 1,
  record_count: 15,
  created: "2026-10-10T00:00:00Z",
  organization_id: "org-med-01",
  description: "Medical genomics data",
  collaborators: [
    {
      id: "collab-1",
      workspace_id: "ws-test-1",
      eppn: "jordan.lee@state.edu",
      name: "Jordan Lee",
      role: "owner",
      scoped_affiliation: "faculty",
      department: "Medicine",
      added_at: "2026-10-10T00:00:00Z",
    },
    {
      id: "collab-2",
      workspace_id: "ws-test-1",
      eppn: "curie@state.edu",
      name: "Marie Curie",
      role: "admin",
      scoped_affiliation: "faculty",
      department: "Medicine",
      added_at: "2026-10-10T00:00:00Z",
    },
  ],
  apps: [mockAppRow],
};

describe("Admin Workspaces & Apps Component", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    vi.spyOn(apiClient, "getOrgs").mockResolvedValue([
      { id: "org-med-01", name: "School of Medicine", code: "SOM", org_type: "School", parent_id: null },
      { id: "org-eng-02", name: "School of Engineering", code: "SOE", org_type: "School", parent_id: null },
    ]);
  });

  it("renders workspaces table and apps table across tabs", async () => {
    vi.spyOn(apiClient, "adminListWorkspaces").mockResolvedValue({
      rows: [mockWorkspaceRow],
      next_cursor: null,
    });
    vi.spyOn(apiClient, "adminListApps").mockResolvedValue({
      rows: [mockAppRow],
      next_cursor: null,
    });

    render(<Workspaces />);

    await waitFor(() => {
      expect(screen.getByText("Genomics Research")).toBeInTheDocument();
      expect(screen.getByText("GR-01")).toBeInTheDocument();
      expect(screen.getByText("School of Medicine")).toBeInTheDocument();
    });

    // Switch to Applications tab
    fireEvent.click(screen.getByTestId("admin-workspaces-subtab-apps"));

    await waitFor(() => {
      expect(screen.getByText("Variant Caller")).toBeInTheDocument();
      expect(screen.getByTestId("open-app-variant-caller")).toHaveAttribute("href", "/apps/variant-caller");
    });
  });

  it("opens drawer admin-workspace-detail with collaborators and apps", async () => {
    vi.spyOn(apiClient, "adminListWorkspaces").mockResolvedValue({
      rows: [mockWorkspaceRow],
      next_cursor: null,
    });
    vi.spyOn(apiClient, "adminGetWorkspace").mockResolvedValue(mockDetail);

    render(<Workspaces />);

    await waitFor(() => {
      expect(screen.getByTestId("view-workspace-ws-test-1")).toBeInTheDocument();
    });

    fireEvent.click(screen.getByTestId("view-workspace-ws-test-1"));

    await waitFor(() => {
      expect(screen.getByTestId("admin-workspace-detail")).toBeInTheDocument();
      expect(screen.getByText("Jordan Lee")).toBeInTheDocument();
      expect(screen.getByText("Marie Curie")).toBeInTheDocument();
      expect(screen.getByTestId("drawer-open-app-variant-caller")).toBeInTheDocument();
    });
  });

  it("test 5: web drawer sends only the changed fields with the reason", async () => {
    vi.spyOn(apiClient, "adminListWorkspaces").mockResolvedValue({
      rows: [mockWorkspaceRow],
      next_cursor: null,
    });
    vi.spyOn(apiClient, "adminGetWorkspace").mockResolvedValue(mockDetail);
    const patchSpy = vi.spyOn(apiClient, "adminPatchWorkspace").mockResolvedValue({
      ...mockDetail,
      visibility: "institutional",
    });

    render(<Workspaces />);

    await waitFor(() => {
      expect(screen.getByTestId("view-workspace-ws-test-1")).toBeInTheDocument();
    });
    fireEvent.click(screen.getByTestId("view-workspace-ws-test-1"));

    await waitFor(() => {
      expect(screen.getByTestId("edit-workspace-visibility")).toBeInTheDocument();
    });

    // Only change visibility from "departmental" to "institutional"
    fireEvent.change(screen.getByTestId("edit-workspace-visibility"), {
      target: { value: "institutional" },
    });
    // Enter reason
    fireEvent.change(screen.getByTestId("edit-workspace-reason"), {
      target: { value: "Expanding research visibility across campus" },
    });

    fireEvent.click(screen.getByTestId("save-workspace-settings-btn"));

    await waitFor(() => {
      expect(patchSpy).toHaveBeenCalledTimes(1);
    });

    expect(patchSpy).toHaveBeenCalledWith("ws-test-1", {
      visibility: "institutional",
      reason: "Expanding research visibility across campus",
    });
  });

  it("transfer ownership selects person from search and sends transfer request", async () => {
    vi.spyOn(apiClient, "adminListWorkspaces").mockResolvedValue({
      rows: [mockWorkspaceRow],
      next_cursor: null,
    });
    vi.spyOn(apiClient, "adminGetWorkspace").mockResolvedValue(mockDetail);
    vi.spyOn(apiClient, "adminListUsers").mockResolvedValue({
      users: [
        {
          id: "u-99",
          user_name: "new.director@state.edu",
          display_name: "New Director",
          email: "new.director@state.edu",
          affiliation: "faculty",
          units: [],
          active: true,
          hold: false,
          platform_admin: false,
          unit_admin_of: [],
          active_agent_tokens: 0,
          latest_token_use: null,
        },
      ],
      next_cursor: null,
    });
    const transferSpy = vi.spyOn(apiClient, "adminTransferWorkspaceOwnership").mockResolvedValue({ ok: true });

    render(<Workspaces />);

    await waitFor(() => {
      expect(screen.getByTestId("view-workspace-ws-test-1")).toBeInTheDocument();
    });
    fireEvent.click(screen.getByTestId("view-workspace-ws-test-1"));

    await waitFor(() => {
      expect(screen.getByTestId("transfer-search-input")).toBeInTheDocument();
    });

    fireEvent.change(screen.getByTestId("transfer-search-input"), {
      target: { value: "director" },
    });

    await waitFor(() => {
      expect(screen.getByTestId("select-user-new.director@state.edu")).toBeInTheDocument();
    });

    fireEvent.click(screen.getByTestId("select-user-new.director@state.edu"));

    fireEvent.change(screen.getByTestId("transfer-reason-input"), {
      target: { value: "Leadership rotation for Genomics unit" },
    });

    fireEvent.click(screen.getByTestId("confirm-transfer-btn"));

    await waitFor(() => {
      expect(transferSpy).toHaveBeenCalledTimes(1);
    });

    expect(transferSpy).toHaveBeenCalledWith(
      "ws-test-1",
      "new.director@state.edu",
      "Leadership rotation for Genomics unit"
    );
  });
});
