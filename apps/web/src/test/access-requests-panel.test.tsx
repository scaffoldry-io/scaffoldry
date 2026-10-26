import "@testing-library/jest-dom";
import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, waitFor, fireEvent } from "@testing-library/react";
import { apiClient, type AccessRequest } from "../api";
import { AccessRequestsPanel } from "../AccessRequestsPanel";

const waiting: AccessRequest = { id: "r-1", status: "Waiting for person", route: "approver", days: 14, object_kind: "app", object_id: "scholarships", role: "viewer", reason: "Audit", task: "Check the audit log", filed_by_client: "client-1" };
const pending: AccessRequest = { id: "r-2", status: "Pending", route: "approver", days: 7, object_kind: "app", object_id: "grants", role: "editor", reason: "Enter rows", waiting_on: ["owner@state.edu"] };
const toDecide: AccessRequest = { id: "r-3", status: "Pending", route: "approver", days: 30, object_kind: "app", object_id: "awards", role: "viewer", reason: "Cleared work", requester: "dean@state.edu" };

describe("The panel of access requests", () => {
  beforeEach(() => {
    vi.restoreAllMocks();
    vi.spyOn(apiClient, "myAccessRequests").mockResolvedValue({ requests: [waiting, pending] });
    vi.spyOn(apiClient, "accessRequestsToDecide").mockResolvedValue({ requests: [toDecide] });
  });

  it("shows my requests with their state, and what a tool filed in my name with the task it named", async () => {
    render(<AccessRequestsPanel />);
    expect(await screen.findByText(/check the audit log/i)).toBeInTheDocument();
    expect(screen.getByText(/waits for owner@state.edu/i)).toBeInTheDocument();
  });

  it("confirms a request a tool filed, and withdraws one that is open", async () => {
    const confirm = vi.spyOn(apiClient, "confirmAccessRequest").mockResolvedValue({ ...waiting, status: "Approved", route: "auto" });
    const withdraw = vi.spyOn(apiClient, "withdrawAccessRequest").mockResolvedValue({ ...pending, status: "Withdrawn" });
    render(<AccessRequestsPanel />);
    fireEvent.click(await screen.findByRole("button", { name: /confirm: audit/i }));
    await waitFor(() => expect(confirm).toHaveBeenCalledWith("r-1"));
    fireEvent.click(await screen.findByRole("button", { name: /withdraw: enter rows/i }));
    await waitFor(() => expect(withdraw).toHaveBeenCalledWith("r-2"));
  });

  it("lists what I may decide and sends my decision with a note", async () => {
    const decide = vi.spyOn(apiClient, "decideAccessRequest").mockResolvedValue({ ...toDecide, status: "Approved" });
    render(<AccessRequestsPanel />);
    expect(await screen.findByText(/dean@state.edu/)).toBeInTheDocument();
    fireEvent.change(screen.getByLabelText(/note for cleared work/i), { target: { value: "Reviewed" } });
    fireEvent.click(screen.getByRole("button", { name: /approve: cleared work/i }));
    await waitFor(() => expect(decide).toHaveBeenCalledWith("r-3", "approve", "Reviewed"));
  });

  it("says so when there is nothing to show", async () => {
    vi.spyOn(apiClient, "myAccessRequests").mockResolvedValue({ requests: [] });
    vi.spyOn(apiClient, "accessRequestsToDecide").mockResolvedValue({ requests: [] });
    render(<AccessRequestsPanel />);
    expect(await screen.findByText(/you have not asked for access/i)).toBeInTheDocument();
    expect(screen.queryByText(/waiting for your decision/i)).not.toBeInTheDocument();
  });
});
