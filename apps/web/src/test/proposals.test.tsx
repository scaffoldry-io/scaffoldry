import "@testing-library/jest-dom";
import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, waitFor, fireEvent } from "@testing-library/react";
import { Proposals } from "../Proposals";
import { WorkflowBuilder } from "../WorkflowBuilder";
import { Overview } from "../admin/Overview";
import { apiClient, type ProposalRow } from "../api";

const flagged: ProposalRow = {
  id: "p-1",
  proposal_id: "p-1",
  workspace_id: "ws-1",
  app_slug: "awards",
  base_version: 3,
  digest: "a".repeat(64),
  status: "Pending",
  summary: "Add a dashboard page",
  proposed_by: "owner@state.edu",
  decided_by: null,
  created_at: "2026-10-10T12:00:00Z",
  decided_at: null,
  checks: [
    { name: "schema", outcome: "pass", messages: [] },
    { name: "page_sensitivity", outcome: "fail", messages: ["page 'Award dashboard' reads the sensitive field 'ssn' of table 'grants'"] },
    { name: "rule_approver", outcome: "warn", messages: ["rule 'Fast track', step 'decide': nobody can decide this today"] },
    { name: "stale", outcome: "pass", messages: [] },
  ],
  changes: [
    "Page \"Award dashboard\" is added.",
    "Page \"Award dashboard\" reads Grants: award number, amount, status. It may change: status. It may not create or delete records.",
  ],
};

const done: ProposalRow = { ...flagged, id: "p-0", proposal_id: "p-0", status: "Approved", summary: "Earlier change", decided_by: "admin@state.edu" };

describe("Proposals", () => {
  beforeEach(() => {
    vi.restoreAllMocks();
    vi.spyOn(apiClient, "listProposals").mockResolvedValue([flagged, done]);
  });

  it("lists proposals with who proposed them and the checks as badges", async () => {
    render(<Proposals workspaceId="ws-1" viewerEppn="admin@state.edu" />);
    expect(await screen.findByText("Add a dashboard page")).toBeInTheDocument();
    expect(screen.getAllByText("owner@state.edu").length).toBeGreaterThan(0);
    expect(screen.getAllByText(/page reads sensitive data/i).length).toBeGreaterThan(0);
    // Pending first.
    const rows = screen.getAllByRole("row");
    expect(rows[1]).toHaveTextContent("Add a dashboard page");
  });

  it("a row opens a drawer with the change list in sentences and each check in words", async () => {
    render(<Proposals workspaceId="ws-1" viewerEppn="admin@state.edu" />);
    fireEvent.click(await screen.findByText("Add a dashboard page"));
    const drawer = await screen.findByRole("dialog");
    expect(drawer).toHaveTextContent("It may change: status. It may not create or delete records.");
    expect(drawer).toHaveTextContent("reads the sensitive field 'ssn'");
    expect(drawer).toHaveTextContent("nobody can decide this today");
  });

  it("disables Approve for the proposer on a flagged proposal and says why", async () => {
    render(<Proposals workspaceId="ws-1" viewerEppn="owner@state.edu" />);
    fireEvent.click(await screen.findByText("Add a dashboard page"));
    const approve = await screen.findByRole("button", { name: /^approve$/i });
    expect(approve).toBeDisabled();
    expect(screen.getByText("A second person must approve this change.")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /^reject$/i })).toBeEnabled();
  });

  it("another admin approves with a reason", async () => {
    const decide = vi.spyOn(apiClient, "decideProposal").mockResolvedValue({ ...flagged, status: "Approved" });
    render(<Proposals workspaceId="ws-1" viewerEppn="admin@state.edu" />);
    fireEvent.click(await screen.findByText("Add a dashboard page"));
    const approve = await screen.findByRole("button", { name: /^approve$/i });
    expect(approve).toBeEnabled();
    fireEvent.click(approve);
    const confirm = await screen.findByRole("button", { name: /confirm approval/i });
    expect(confirm).toBeDisabled();
    fireEvent.change(screen.getByLabelText(/reason for decision/i), { target: { value: "Checked the grants" } });
    fireEvent.click(confirm);
    await waitFor(() => expect(decide).toHaveBeenCalledWith("p-1", "approve", "Checked the grants", "a".repeat(64)));
  });
});

describe("WorkflowBuilder with approval", () => {
  const rule = { id: "r1", app_slug: "awards", name: "Rule", description: "", enabled: true, trigger: { type: "RecordCreated" as const }, predicates: [], actions: [] };
  const props = { appSlug: "awards", appTitle: "Awards", fields: [], rules: [], onDeleteRule: vi.fn(), onToggleRule: vi.fn() };

  it("a 202 shows 'Submitted for approval.' with a View proposal link and no live rule", async () => {
    const onSaveRule = vi.fn();
    const onViewProposal = vi.fn();
    const submit = vi.fn().mockResolvedValue({ status: "pending", proposalId: "p-9" });
    render(<WorkflowBuilder {...props} onSaveRule={onSaveRule} submitRule={submit} onViewProposal={onViewProposal} />);
    fireEvent.click(screen.getByTestId("apply-template-approval"));
    expect(await screen.findByText("Submitted for approval.")).toBeInTheDocument();
    expect(onSaveRule).not.toHaveBeenCalled();
    fireEvent.click(screen.getByRole("button", { name: /view proposal/i }));
    expect(onViewProposal).toHaveBeenCalledWith("p-9");
    expect(submit).toHaveBeenCalled();
    void rule;
  });

  it("a live save still reaches onSaveRule", async () => {
    const onSaveRule = vi.fn();
    const submit = vi.fn().mockResolvedValue({ status: "live" });
    render(<WorkflowBuilder {...props} onSaveRule={onSaveRule} submitRule={submit} />);
    fireEvent.click(screen.getByTestId("apply-template-approval"));
    await waitFor(() => expect(onSaveRule).toHaveBeenCalled());
    expect(screen.queryByText("Submitted for approval.")).not.toBeInTheDocument();
  });
});

describe("Overview", () => {
  it("shows the pending proposals count", async () => {
    vi.spyOn(apiClient, "getAdminOverview").mockResolvedValue({
      server: { version: "1", applied_migrations: [], database_worker_count: 1 },
      people: { active: 1, on_hold: 0, inactive: 0, platform_admins: 1, active_tokens: {} },
      organization: { unit_count: 1 },
      workspaces: { workspace_count: 1, app_count: 1 },
      processes: { waiting_instance_count: 0 },
      proposals: { pending: 4 },
      ledger: { entry_count: 1, head_hash: "abc" },
    });
    render(<Overview />);
    expect(await screen.findByTestId("admin-overview-proposals-card")).toHaveTextContent("4");
  });
});
