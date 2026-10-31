import "@testing-library/jest-dom";
import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent, waitFor, within } from "@testing-library/react";
import { Ownership } from "../admin/Ownership";
import { Workspaces } from "../admin/Workspaces";
import { apiClient, type OwnershipOrphans, type OwnershipPreview } from "../api";

const orphans: OwnershipOrphans = {
  workspaces: [
    { id: "ws-lone", name: "Optics Lab", owners: [{ eppn: "gone.owner@state.edu", why: "on hold" }] },
    { id: "ws-empty", name: "Old Archive", owners: [] },
  ],
  proposals: [{ id: "p-1", name: "intake-form", proposed_by: "gone.owner@state.edu", why: "inactive", workspace_id: "ws-lone" }],
  clients: [{ id: "tracker", name: "Grade Tracker", proposed_by: "gone.owner@state.edu", why: "no appointment" }],
};

const clean: OwnershipPreview = {
  from: "gone.owner@state.edu",
  to: "new.owner@state.edu",
  can_transfer: true,
  blockers: [],
  workspaces: [
    {
      id: "ws-lone",
      name: "Optics Lab",
      to_role: null,
      gains: ["new.owner@state.edu becomes an owner of Optics Lab: they can manage its members, apps and settings."],
      loses: ["gone.owner@state.edu loses their owner role in Optics Lab, and with it every right the role gave."],
      open_proposals: [{ id: "p-9", app_slug: "waiting-app", summary: "A change waiting for a decision" }],
      process_steps: [],
      tokens: [{ id: "t-1", label: "Claude on laptop" }],
    },
  ],
  clients_from_created: [{ client_id: "tracker", name: "Grade Tracker", status: "Active" }],
};

const blocked: OwnershipPreview = { ...clean, can_transfer: false, blockers: ["new.owner@state.edu is not active, so they cannot take over a workspace."] };

function fill(label: string, value: string) {
  fireEvent.change(screen.getByLabelText(label), { target: { value } });
}

describe("Ownership", () => {
  beforeEach(() => {
    vi.restoreAllMocks();
    vi.spyOn(apiClient, "ownershipOrphans").mockResolvedValue(orphans);
  });

  it("lists the workspaces nobody is looking after, and says why each owner has gone", async () => {
    render(<Ownership />);
    const list = await screen.findByTestId("ownership-orphans");
    expect(within(list).getByText("Optics Lab")).toBeInTheDocument();
    expect(within(list).getByText(/gone\.owner@state\.edu is on hold/)).toBeInTheDocument();
    expect(within(list).getByText("Old Archive")).toBeInTheDocument();
    expect(within(list).getByText(/has no owner/)).toBeInTheDocument();
    // What else lost the person who looked after it.
    expect(screen.getByText(/intake-form/)).toBeInTheDocument();
    expect(screen.getByText(/Grade Tracker/)).toBeInTheDocument();
  });

  it("says so when nothing is orphaned", async () => {
    vi.spyOn(apiClient, "ownershipOrphans").mockResolvedValue({ workspaces: [], proposals: [], clients: [] });
    render(<Ownership />);
    expect(await screen.findByText(/Every workspace has an owner who is here/)).toBeInTheDocument();
  });

  it("keeps Transfer off until a preview with no blocker has been read for these same two people", async () => {
    const preview = vi.spyOn(apiClient, "ownershipPreview").mockResolvedValue(clean);
    render(<Ownership />);
    await screen.findByTestId("ownership-orphans");
    const transfer = screen.getByRole("button", { name: "Transfer" });
    expect(transfer).toBeDisabled();

    fill("From", "gone.owner@state.edu");
    fill("To", "new.owner@state.edu");
    expect(transfer).toBeDisabled();
    fireEvent.click(screen.getByRole("button", { name: "Preview transfer" }));
    await screen.findByTestId("ownership-preview");
    expect(preview).toHaveBeenCalledWith({ from: "gone.owner@state.edu", to: "new.owner@state.edu" });

    fill("Reason", "Moved to another school");
    expect(transfer).toBeEnabled();

    // Change who it is for and the preview no longer applies.
    fill("To", "someone.else@state.edu");
    expect(transfer).toBeDisabled();
    expect(screen.queryByTestId("ownership-preview")).not.toBeInTheDocument();
  });

  it("shows what the new owner gains, what the old owner loses, and what is waiting", async () => {
    vi.spyOn(apiClient, "ownershipPreview").mockResolvedValue(clean);
    render(<Ownership />);
    await screen.findByTestId("ownership-orphans");
    fill("From", "gone.owner@state.edu");
    fill("To", "new.owner@state.edu");
    fireEvent.click(screen.getByRole("button", { name: "Preview transfer" }));
    const view = await screen.findByTestId("ownership-preview");
    expect(within(view).getByText(/becomes an owner of Optics Lab/)).toBeInTheDocument();
    expect(within(view).getByText(/loses their owner role in Optics Lab/)).toBeInTheDocument();
    expect(within(view).getByText(/A change waiting for a decision/)).toBeInTheDocument();
    expect(within(view).getByText(/Claude on laptop/)).toBeInTheDocument();
    expect(within(view).getByText(/Grade Tracker/)).toBeInTheDocument();
  });

  it("states a blocker in a sentence and keeps Transfer off", async () => {
    vi.spyOn(apiClient, "ownershipPreview").mockResolvedValue(blocked);
    render(<Ownership />);
    await screen.findByTestId("ownership-orphans");
    fill("From", "gone.owner@state.edu");
    fill("To", "new.owner@state.edu");
    fill("Reason", "Moved to another school");
    fireEvent.click(screen.getByRole("button", { name: "Preview transfer" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("is not active, so they cannot take over a workspace");
    expect(screen.getByRole("button", { name: "Transfer" })).toBeDisabled();
  });

  it("makes the transfer with the reason, says what moved, and reads the list again", async () => {
    vi.spyOn(apiClient, "ownershipPreview").mockResolvedValue(clean);
    const transfer = vi.spyOn(apiClient, "ownershipTransfer").mockResolvedValue({ status: "transferred", from: clean.from, to: clean.to, workspaces: 1, revoked_tokens: 1 });
    render(<Ownership />);
    await screen.findByTestId("ownership-orphans");
    fill("From", "gone.owner@state.edu");
    fill("To", "new.owner@state.edu");
    fill("Reason", "Moved to another school");
    fireEvent.click(screen.getByRole("button", { name: "Preview transfer" }));
    await screen.findByTestId("ownership-preview");
    fireEvent.click(screen.getByRole("button", { name: "Transfer" }));
    await waitFor(() => expect(transfer).toHaveBeenCalledWith({ from: "gone.owner@state.edu", to: "new.owner@state.edu", reason: "Moved to another school" }));
    expect(await screen.findByText(/1 workspace moved to new\.owner@state\.edu/)).toBeInTheDocument();
    expect(apiClient.ownershipOrphans).toHaveBeenCalledTimes(2);
  });

  it("explains a refusal in the words the server gave", async () => {
    vi.spyOn(apiClient, "ownershipPreview").mockResolvedValue(clean);
    vi.spyOn(apiClient, "ownershipTransfer").mockRejectedValue(new Error("The change failed. Optics Lab has no owner now; run it again."));
    render(<Ownership />);
    await screen.findByTestId("ownership-orphans");
    fill("From", "gone.owner@state.edu");
    fill("To", "new.owner@state.edu");
    fill("Reason", "Moved to another school");
    fireEvent.click(screen.getByRole("button", { name: "Preview transfer" }));
    await screen.findByTestId("ownership-preview");
    fireEvent.click(screen.getByRole("button", { name: "Transfer" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("Optics Lab has no owner now");
  });

  it("is a tab in the workspaces section", async () => {
    vi.spyOn(apiClient, "adminListWorkspaces").mockResolvedValue({ rows: [], next_cursor: null });
    vi.spyOn(apiClient, "getOrgs").mockResolvedValue([]);
    render(<Workspaces />);
    fireEvent.click(await screen.findByTestId("admin-workspaces-subtab-ownership"));
    expect(await screen.findByTestId("ownership-orphans")).toBeInTheDocument();
  });
});
