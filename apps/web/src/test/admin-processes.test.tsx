import "@testing-library/jest-dom";
import { afterEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { Overview } from "../admin/Overview";
import { Processes } from "../admin/Processes";
import { apiClient, type AdminOverviewData, type AdminProcessInstance, type AdminProcessRow, type AdminUserRow } from "../api";

const row: AdminProcessRow = {
  rule_id: "rule-1",
  name: "Review request",
  app_slug: "requests",
  workspace_id: "ws-1",
  workspace_name: "Physics",
  version: 2,
  enabled: true,
  trigger: "When a record is created in this app.",
  step_count: 1,
  user_task_roles: ["owner"],
  waiting_count: 3,
  last_changed_by: "einstein@physics.state.edu",
  last_changed_at: "2026-11-10T09:00:00Z",
};

const instance = (over: Partial<AdminProcessInstance> = {}): AdminProcessInstance => ({
  id: "rule-1:rec-1",
  app_slug: "requests",
  rule_id: "rule-1",
  rule_name: "Review request",
  rule_version: 2,
  record_id: "rec-1",
  status: "Waiting",
  waiting_step_id: "decide",
  deciders: ["Einstein"],
  no_approver: null,
  no_approver_reason: null,
  assigned_to: null,
  started_at: "2026-11-01T09:00:00Z",
  age_days: 18,
  stale: true,
  ...over,
});

const user = (name: string): AdminUserRow => ({
  id: `id-${name}`,
  user_name: name,
  display_name: name.split("@")[0],
  email: name,
  affiliation: "faculty",
  units: [],
  active: true,
  hold: false,
  platform_admin: false,
  unit_admin_of: [],
  active_agent_tokens: 0,
  latest_token_use: null,
});

function stubDefinitions() {
  vi.spyOn(apiClient, "adminProcesses").mockResolvedValue({ rows: [row], next_cursor: null });
  vi.spyOn(apiClient, "adminProcess").mockResolvedValue({
    rule: { id: "rule-1", name: "Review request", app_slug: "requests", enabled: true, version: 2 },
    sentences: ["When a record is created in this app.", "Then wait for a decision by anyone with the role owner."],
    workspace_id: "ws-1",
    workspace_name: "Physics",
    versions: [
      { version: 1, changed_by: "einstein@physics.state.edu", changed_at: "2026-11-01T09:00:00Z", summary: "Save the rule", proposal_id: null, sha256: "a".repeat(64) },
      { version: 2, changed_by: "jordan.lee@state.edu", changed_at: "2026-11-10T09:00:00Z", summary: "Turned on", proposal_id: null, sha256: "b".repeat(64) },
    ],
    instance_counts: { Waiting: 3, Completed: 1, Rejected: 0, Failed: 0 },
  });
}

afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
});

async function openDrawer() {
  stubDefinitions();
  render(<Processes />);
  fireEvent.click(await screen.findByText("Review request"));
  return await screen.findByTestId("admin-process-detail");
}

describe("the Business processes panel", () => {
  it("opens a definition and shows what it does in sentences", async () => {
    const drawer = await openDrawer();
    expect(await within(drawer).findByText("Then wait for a decision by anyone with the role owner.")).toBeInTheDocument();
    expect(within(drawer).getByText(/Version 1/)).toBeInTheDocument();
  });

  it("shows a past version read-only, in sentences", async () => {
    const view = vi.spyOn(apiClient, "adminProcessVersion").mockResolvedValue({
      version: 1,
      changed_by: "einstein@physics.state.edu",
      changed_at: "2026-11-01T09:00:00Z",
      summary: "Save the rule",
      sha256: "a".repeat(64),
      rule: { id: "rule-1" },
      sentences: ["When a record is created in this app.", "Then wait for the old owner."],
    });
    const drawer = await openDrawer();
    fireEvent.click(await within(drawer).findByRole("button", { name: "View version 1" }));
    await waitFor(() => expect(view).toHaveBeenCalledWith("rule-1", 1));
    expect(await within(drawer).findByText("Then wait for the old owner.")).toBeInTheDocument();
  });

  it("does not send Disable without a reason, and sends it with one", async () => {
    const set = vi.spyOn(apiClient, "setProcessEnabled").mockResolvedValue({ rule_id: "rule-1", enabled: false, version: 3 });
    const drawer = await openDrawer();
    fireEvent.click(await within(drawer).findByRole("button", { name: "Disable" }));
    const confirm = await within(drawer).findByRole("button", { name: "Disable Review request" });
    fireEvent.click(confirm);
    expect(set).not.toHaveBeenCalled();
    fireEvent.change(within(drawer).getByRole("textbox", { name: /Reason/ }), { target: { value: "Paused for the audit" } });
    fireEvent.click(within(drawer).getByRole("button", { name: "Disable Review request" }));
    await waitFor(() => expect(set).toHaveBeenCalledWith("rule-1", false, "Paused for the audit"));
  });

  it("opens the app's workflow tab from Edit in app builder", async () => {
    const onEdit = vi.fn();
    stubDefinitions();
    render(<Processes onEditInBuilder={onEdit} />);
    fireEvent.click(await screen.findByText("Review request"));
    const drawer = await screen.findByTestId("admin-process-detail");
    fireEvent.click(await within(drawer).findByRole("button", { name: "Edit in app builder" }));
    expect(onEdit).toHaveBeenCalledWith("requests");
  });

  it("posts the person chosen from the search when it reassigns", async () => {
    vi.spyOn(apiClient, "adminProcessInstances").mockResolvedValue({ rows: [instance()], next_cursor: null });
    const search = vi.spyOn(apiClient, "adminListUsers").mockResolvedValue({ users: [user("cover@state.edu")], next_cursor: null });
    const reassign = vi.spyOn(apiClient, "reassignProcessInstance").mockResolvedValue(undefined);
    render(<Processes initialTab="instances" />);
    fireEvent.click(await screen.findByRole("button", { name: "Reassign Review request on rec-1" }));
    fireEvent.change(await screen.findByRole("searchbox", { name: "Find a person" }), { target: { value: "cov" } });
    await waitFor(() => expect(search).toHaveBeenCalledWith(expect.objectContaining({ search: "cov" })));
    fireEvent.click(await screen.findByRole("button", { name: "Choose cover@state.edu" }));
    fireEvent.change(screen.getByRole("textbox", { name: /Reason/ }), { target: { value: "The owner is away" } });
    fireEvent.click(screen.getByRole("button", { name: "Reassign to cover@state.edu" }));
    await waitFor(() => expect(reassign).toHaveBeenCalledWith("rule-1:rec-1", "cover@state.edu", "The owner is away"));
  });

  it("asks for the Unassigned filter and says in words why nobody can decide", async () => {
    const list = vi.spyOn(apiClient, "adminProcessInstances").mockImplementation(async (params = {}) => ({
      rows: params.unassigned === "true" ? [instance({ no_approver: "role_empty", no_approver_reason: "Nobody in the workspace has the role that decides this step.", deciders: [] })] : [],
      next_cursor: null,
    }));
    render(<Processes initialTab="instances" />);
    await waitFor(() => expect(list).toHaveBeenCalled());
    fireEvent.click(await screen.findByLabelText("Unassigned"));
    expect(await screen.findByText("Nobody in the workspace has the role that decides this step.")).toBeInTheDocument();
    expect(list).toHaveBeenLastCalledWith(expect.objectContaining({ unassigned: "true" }));
  });

  it("cancels only with a reason", async () => {
    vi.spyOn(apiClient, "adminProcessInstances").mockResolvedValue({ rows: [instance()], next_cursor: null });
    const cancel = vi.spyOn(apiClient, "cancelProcessInstance").mockResolvedValue(undefined);
    render(<Processes initialTab="instances" />);
    fireEvent.click(await screen.findByRole("button", { name: "Cancel Review request on rec-1" }));
    fireEvent.click(await screen.findByRole("button", { name: "Cancel instance of Review request" }));
    expect(cancel).not.toHaveBeenCalled();
    fireEvent.change(screen.getByRole("textbox", { name: /Reason/ }), { target: { value: "Raised in error" } });
    fireEvent.click(screen.getByRole("button", { name: "Cancel instance of Review request" }));
    await waitFor(() => expect(cancel).toHaveBeenCalledWith("rule-1:rec-1", "Raised in error"));
  });

  it("shows who can decide an instance, and where the position was found", async () => {
    vi.spyOn(apiClient, "adminProcessInstances").mockResolvedValue({ rows: [instance()], next_cursor: null });
    vi.spyOn(apiClient, "adminProcessInstanceApprovers").mockResolvedValue({
      instance_id: "rule-1:rec-1",
      approvers: [{ eppn: "chair@state.edu", display_name: "Dr. Chair", via: "Position" }],
      unit_id: "u1",
      unit_name: "Physics",
      problem: null,
      problem_reason: null,
    });
    render(<Processes initialTab="instances" />);
    fireEvent.click(await screen.findByRole("button", { name: "Who can decide Review request on rec-1" }));
    expect(await screen.findByText(/Dr\. Chair/)).toBeInTheDocument();
    expect(screen.getByText(/found at Physics/)).toBeInTheDocument();
  });
});

describe("the Overview", () => {
  it("shows the waiting and stale counts", async () => {
    const data = {
      server: { version: "1", applied_migrations: [], database_worker_count: 1 },
      people: { active: 1, on_hold: 0, inactive: 0, platform_admins: 1, active_tokens: {} },
      organization: { unit_count: 1 },
      workspaces: { workspace_count: 1, app_count: 1 },
      processes: { waiting_instance_count: 5, stale: 2 },
      ledger: { entry_count: 1, head_hash: "abc" },
    } as AdminOverviewData;
    vi.spyOn(apiClient, "getAdminOverview").mockResolvedValue(data);
    render(<Overview />);
    const card = await screen.findByTestId("admin-overview-processes-card");
    expect(within(card).getByText("5")).toBeInTheDocument();
    expect(within(card).getByText("2")).toBeInTheDocument();
    expect(within(card).getByText(/stale/i)).toBeInTheDocument();
  });
});
