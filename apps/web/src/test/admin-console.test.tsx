import "@testing-library/jest-dom";
import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, waitFor, fireEvent } from "@testing-library/react";
import { AdminConsole } from "../admin/AdminConsole";
import { DataTable, DataTableColumn } from "../ui";
import { apiClient, AdminOverviewData } from "../api";
import { Persona } from "../types";

const mockFaculty: Persona = {
  eppn: "faculty.marie@state.edu",
  name: "Dr. Marie Curie",
  affiliation: "faculty",
  department: "Physics",
  roleTitle: "Professor",
  isAdmin: false,
};

const mockPlatformAdmin: Persona = {
  eppn: "jordan.lee@state.edu",
  name: "Jordan Lee",
  affiliation: "central_admin",
  department: "Central Enterprise IT",
  roleTitle: "Enterprise Architect",
  isAdmin: true,
};

const mockOverviewData: AdminOverviewData = {
  server: {
    version: "0.1.0",
    applied_migrations: ["0001_initial_schema.sql", "0002_workspaces_and_ledger.sql"],
    database_worker_count: 8,
  },
  people: {
    active: 2,
    on_hold: 1,
    inactive: 0,
    platform_admins: 1,
    active_tokens: {
      agent: 1,
      scim: 1,
    },
  },
  organization: {
    unit_count: 2,
  },
  workspaces: {
    workspace_count: 1,
    app_count: 4,
  },
  processes: {
    waiting_instance_count: 0,
  },
  ledger: {
    entry_count: 12,
    head_hash: "a1b2c3d4e5f67890",
  },
};

describe("Admin Console Shell & Overview", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("a faculty persona sees admin-access-denied", async () => {
    vi.spyOn(apiClient, "getCurrentUser").mockResolvedValue({
      user: { affiliation: "faculty" },
      is_impersonating: false,
      original_admin: null,
      is_platform_admin: false,
    });

    render(<AdminConsole activePersona={mockFaculty} />);

    expect(screen.getByTestId("admin-access-denied")).toBeInTheDocument();
    expect(
      screen.getByText(/403 Forbidden: Cedar Policy Authorization Required/i)
    ).toBeInTheDocument();
  });

  it("a Platform Admin sees the overview cards from a stubbed response", async () => {
    vi.spyOn(apiClient, "getCurrentUser").mockResolvedValue({
      user: { affiliation: "central_admin" },
      is_impersonating: false,
      original_admin: null,
      is_platform_admin: true,
    });
    vi.spyOn(apiClient, "getAdminOverview").mockResolvedValue(mockOverviewData);

    render(<AdminConsole activePersona={mockPlatformAdmin} adminTab="overview" />);

    expect(screen.queryByTestId("admin-access-denied")).not.toBeInTheDocument();

    await waitFor(() => {
      expect(screen.getByTestId("admin-overview")).toBeInTheDocument();
    });

    // Server card
    const serverCard = screen.getByTestId("admin-overview-server-card");
    expect(serverCard).toHaveTextContent("v0.1.0");
    expect(serverCard).toHaveTextContent("8"); // workers

    // People card
    const peopleCard = screen.getByTestId("admin-overview-people-card");
    expect(peopleCard).toHaveTextContent("2"); // active
    expect(peopleCard).toHaveTextContent("1"); // on hold

    // Organization card
    const orgCard = screen.getByTestId("admin-overview-org-card");
    expect(orgCard).toHaveTextContent("2"); // units

    // Workspaces card
    const wsCard = screen.getByTestId("admin-overview-workspaces-card");
    expect(wsCard).toHaveTextContent("1"); // workspaces
    expect(wsCard).toHaveTextContent("4"); // apps

    // Processes card
    const procCard = screen.getByTestId("admin-overview-processes-card");
    expect(procCard).toHaveTextContent("0"); // waiting instances

    // Ledger card
    const ledgerCard = screen.getByTestId("admin-overview-ledger-card");
    expect(ledgerCard).toHaveTextContent("12 Entries");
    expect(ledgerCard).toHaveTextContent("a1b2c3d4e5f67890");
  });

  it("DataTable with a stubbed load shows the first page and Load more calls load with the cursor", async () => {
    interface TestItem {
      id: string;
      name: string;
    }

    const columns: DataTableColumn<TestItem, unknown>[] = [
      {
        accessorKey: "name",
        header: "Name",
      },
    ];

    const mockLoad = vi.fn().mockImplementation(async (cursor?: string) => {
      if (!cursor) {
        return {
          rows: [
            { id: "1", name: "Alpha User" },
            { id: "2", name: "Beta User" },
          ],
          next_cursor: "offset:2",
        };
      }
      return {
        rows: [{ id: "3", name: "Gamma User" }],
        next_cursor: undefined,
      };
    });

    render(<DataTable columns={columns} load={mockLoad} rowKey="id" />);

    // First page rows
    await waitFor(() => {
      expect(screen.getByText("Alpha User")).toBeInTheDocument();
      expect(screen.getByText("Beta User")).toBeInTheDocument();
    });
    expect(mockLoad).toHaveBeenCalledTimes(1);
    expect(mockLoad).toHaveBeenCalledWith(undefined, undefined, undefined, undefined, false);

    // Find and click Load more
    const loadMoreBtn = screen.getByRole("button", { name: /Load more/i });
    expect(loadMoreBtn).toBeInTheDocument();

    fireEvent.click(loadMoreBtn);

    // Second page row appended
    await waitFor(() => {
      expect(screen.getByText("Gamma User")).toBeInTheDocument();
    });
    expect(mockLoad).toHaveBeenCalledTimes(2);
    expect(mockLoad).toHaveBeenLastCalledWith("offset:2", undefined, undefined, undefined, false);

    // Load more button should disappear
    expect(screen.queryByRole("button", { name: /Load more/i })).not.toBeInTheDocument();
  });
});

describe("Admin People", () => {
  const row = {
    id: "u-1",
    user_name: "pat@state.edu",
    display_name: "Pat Doe",
    email: "pat@state.edu",
    affiliation: "faculty",
    units: [{ id: "o-1", name: "Biology" }],
    active: true,
    hold: false,
    platform_admin: false,
    unit_admin_of: [],
    active_agent_tokens: 1,
    latest_token_use: null,
  };
  const detail = {
    user: row,
    appointments: [
      { organization_id: "o-1", unit: "Biology", scoped_affiliation: "unit_admin", role_title: "Org Unit Admin", source: "api" },
    ],
    workspace_memberships: [],
    tokens: [
      { id: "t-1", kind: "agent", label: "Lab notebook agent", created_at: "2026-10-01T00:00:00Z", expires_at: "2026-12-01T00:00:00Z", last_used_at: null, revoked: false },
    ],
    ledger: [],
  };

  beforeEach(() => {
    vi.clearAllMocks();
    vi.spyOn(apiClient, "getCurrentUser").mockResolvedValue({
      user: { affiliation: "central_admin" },
      is_impersonating: false,
      original_admin: null,
      is_platform_admin: true,
    });
    vi.spyOn(apiClient, "adminListUsers").mockResolvedValue({ users: [row], next_cursor: null });
    vi.spyOn(apiClient, "adminGetUser").mockResolvedValue(detail);
    vi.spyOn(apiClient, "listOrganizations").mockResolvedValue([
      { id: "o-0", parent_id: null, name: "Institution", code: "INST", org_type: "Institution" },
      { id: "o-1", parent_id: "o-0", name: "Biology", code: "BIO", org_type: "Department" },
    ]);
  });

  const openDrawer = async () => {
    render(<AdminConsole activePersona={mockPlatformAdmin} adminTab="people" />);
    fireEvent.click(await screen.findByText("pat@state.edu"));
    return screen.findByTestId("admin-user-detail");
  };

  it("the detail drawer shows tokens and appointments", async () => {
    await openDrawer();
    expect(screen.getByTestId("admin-user-tokens")).toHaveTextContent("Lab notebook agent");
    expect(screen.getByTestId("admin-user-appointments")).toHaveTextContent("unit_admin in Biology");
  });

  it("Hold without a reason is not sent", async () => {
    const hold = vi.spyOn(apiClient, "adminHoldUser").mockResolvedValue({ user: { ...row, hold: true } });
    await openDrawer();
    fireEvent.click(screen.getByTestId("admin-user-action-hold"));
    const confirm = await screen.findByRole("button", { name: /confirm hold/i });
    expect(confirm).toBeDisabled();
    fireEvent.click(confirm);
    expect(hold).not.toHaveBeenCalled();
  });

  it("Confirm sends the hold with the reason", async () => {
    const hold = vi.spyOn(apiClient, "adminHoldUser").mockResolvedValue({ user: { ...row, hold: true } });
    await openDrawer();
    fireEvent.click(screen.getByTestId("admin-user-action-hold"));
    fireEvent.change(await screen.findByLabelText(/reason for decision/i), { target: { value: "Pending review" } });
    fireEvent.click(screen.getByRole("button", { name: /confirm hold/i }));
    await waitFor(() => expect(hold).toHaveBeenCalledWith("u-1", true, "Pending review"));
  });

  it("the create form posts the new user", async () => {
    const create = vi.spyOn(apiClient, "adminCreateUser").mockResolvedValue({ user: row });
    render(<AdminConsole activePersona={mockPlatformAdmin} adminTab="people" />);
    fireEvent.click(await screen.findByTestId("admin-user-create-open"));
    const form = await screen.findByTestId("admin-user-create");
    fireEvent.change(form.querySelector("#create-userName")!, { target: { value: "new@state.edu" } });
    fireEvent.change(form.querySelector("#create-reason")!, { target: { value: "New hire" } });
    fireEvent.click(screen.getByRole("button", { name: /create user/i }));
    await waitFor(() => expect(create).toHaveBeenCalled());
    expect(create.mock.calls[0][0]).toMatchObject({ userName: "new@state.edu", reason: "New hire" });
  });
});
