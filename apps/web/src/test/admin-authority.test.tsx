import "@testing-library/jest-dom";
import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, waitFor, fireEvent, within } from "@testing-library/react";
import { Authority } from "../admin/Authority";
import { apiClient, type AuthorityView, type AuthorityImpact } from "../api";

const ROOT = "00000000-0000-0000-0000-000000000001";
const PHYSICS = "00000000-0000-0000-0000-0000000000a1";

function view(overrides: Partial<AuthorityView> = {}): AuthorityView {
  return {
    organization_id: PHYSICS,
    unit_name: "Physics",
    tiers: [
      { tier: "Low", raised_by: [], path: "direct", approver: null, resolved: [], source: { kind: "built_in" } },
      { tier: "Moderate", raised_by: [{ check: "sensitivity", sentence: "The change adds or exposes a field that holds sensitive data." }], path: "second_person", approver: null, resolved: [], source: { kind: "built_in" } },
      { tier: "High", raised_by: [], path: "second_person", approver: null, resolved: [], source: { kind: "built_in" } },
    ],
    rows: [],
    signals: [],
    ...overrides,
  };
}

const impact: AuthorityImpact = {
  proposals: 12,
  window_days: 90,
  before: { direct: 7, second_person: 5, approver: 0 },
  after: { direct: 7, second_person: 0, approver: 5 },
  load: [{ eppn: "holder.one@state.edu", display_name: "Holder One", before: 0, after: 5 }],
  unassigned: [],
};

function quiet(v: AuthorityView = view()) {
  vi.spyOn(apiClient, "getCurrentUser").mockResolvedValue({ user: { eppn: "jordan.lee@state.edu", name: "Jordan", affiliation: "central_admin" }, is_impersonating: false, original_admin: null, is_platform_admin: true });
  vi.spyOn(apiClient, "accessGrantAnchors").mockResolvedValue({ units: [{ id: ROOT, name: "Institution", hierarchy: "academic" }, { id: PHYSICS, name: "Physics", hierarchy: "academic" }], domains: [] } as never);
  vi.spyOn(apiClient, "authorityShow").mockResolvedValue(v);
  vi.spyOn(apiClient, "authorityChanges").mockResolvedValue({ changes: [] });
}

async function openChange(tier = "Moderate") {
  render(<Authority />);
  const row = await screen.findByTestId(`authority-tier-${tier}`);
  fireEvent.click(within(row).getByRole("button", { name: "Change" }));
  return screen.findByTestId("authority-form");
}

describe("Decision authority", () => {
  beforeEach(() => {
    vi.restoreAllMocks();
  });

  it("shows one row per tier: what raises it, what it needs, and where that comes from", async () => {
    quiet(
      view({
        tiers: [
          view().tiers[0],
          { ...view().tiers[1], path: "approver", approver: { Position: { key: "privacy-officer" } }, resolved: [{ eppn: "holder.one@state.edu", display_name: "Holder One" }], source: { kind: "institution", organization_id: ROOT, name: "Institution" } },
          view().tiers[2],
        ],
      })
    );
    render(<Authority />);
    const moderate = await screen.findByTestId("authority-tier-Moderate");
    expect(within(moderate).getByText(/adds or exposes a field that holds sensitive data/)).toBeInTheDocument();
    expect(within(moderate).getByText(/privacy-officer/)).toBeInTheDocument();
    expect(within(moderate).getByText(/Holder One/)).toBeInTheDocument();
    expect(within(moderate).getByText(/The institution/)).toBeInTheDocument();
    expect(within(screen.getByTestId("authority-tier-Low")).getByText(/Built in/)).toBeInTheDocument();
  });

  // 7. Propose change is disabled until the effect has been shown for the current values.
  it("keeps Propose change disabled until the effect is shown, and disables it again when a value changes", async () => {
    quiet();
    const seen = vi.spyOn(apiClient, "authorityImpact").mockResolvedValue(impact);
    const form = await openChange();
    const propose = within(form).getByRole("button", { name: "Propose change" });
    expect(propose).toBeDisabled();
    fireEvent.change(within(form).getByLabelText("Path"), { target: { value: "approver" } });
    fireEvent.change(within(form).getByLabelText("Position key"), { target: { value: "privacy-officer" } });
    fireEvent.change(within(form).getByLabelText(/Reason/), { target: { value: "Privacy review" } });
    expect(propose).toBeDisabled();

    fireEvent.click(within(form).getByRole("button", { name: "See the effect" }));
    await waitFor(() => expect(seen).toHaveBeenCalledTimes(1));
    const result = await screen.findByTestId("authority-effect");
    expect(result).toHaveTextContent("12 proposals in the last 90 days");
    expect(within(result).getByText(/Holder One/)).toBeInTheDocument();
    expect(propose).toBeEnabled();
    expect(seen.mock.calls[0][0]).toEqual({ organization_id: PHYSICS, change: { kind: "row", row: { tier: "Moderate", path: "approver", approver: { Position: { key: "privacy-officer", from: "Workspace", walk_up: true } } } } });

    fireEvent.change(within(form).getByLabelText("Position key"), { target: { value: "dean" } });
    expect(propose).toBeDisabled();
  });

  it("names the proposals that would have no approver", async () => {
    quiet();
    vi.spyOn(apiClient, "authorityImpact").mockResolvedValue({ ...impact, after: { direct: 7, second_person: 0, approver: 5 }, load: [], unassigned: [{ proposal_id: "p-1", workspace_id: "w", summary: "Add an amount column" }] });
    const form = await openChange();
    fireEvent.change(within(form).getByLabelText("Path"), { target: { value: "approver" } });
    fireEvent.change(within(form).getByLabelText("Position key"), { target: { value: "vacant" } });
    fireEvent.change(within(form).getByLabelText(/Reason/), { target: { value: "Privacy review" } });
    fireEvent.click(within(form).getByRole("button", { name: "See the effect" }));
    const result = await screen.findByTestId("authority-effect");
    expect(within(result).getByText(/no one to decide/i)).toBeInTheDocument();
    expect(within(result).getByText("Add an amount column")).toBeInTheDocument();
  });

  // 8. A row looser than the institution's says so.
  it("says the institution already requires more for a row that has no effect", async () => {
    quiet(view({ rows: [{ organization_id: PHYSICS, unit_name: "Physics", tier: "Moderate", path: "second_person", approver: null, has_effect: false }, { organization_id: ROOT, unit_name: "Institution", tier: "Moderate", path: "approver", approver: { Position: { key: "privacy-officer" } }, has_effect: true }] }));
    render(<Authority />);
    const rows = await screen.findAllByTestId("authority-row");
    expect(within(rows[0]).getByText("The institution already requires more.")).toBeInTheDocument();
    expect(within(rows[1]).queryByText("The institution already requires more.")).not.toBeInTheDocument();
  });

  it("lets a different person confirm or reject a pending change, and not the proposer", async () => {
    quiet();
    vi.spyOn(apiClient, "authorityChanges").mockResolvedValue({
      changes: [
        { id: "c-1", organization_id: PHYSICS, unit_name: "Physics", change: { kind: "row", row: { tier: "High", path: "second_person" } }, reason: "Tighten", status: "Pending", proposed_by: "jordan.lee@state.edu", decided_by: null },
        { id: "c-2", organization_id: PHYSICS, unit_name: "Physics", change: { kind: "signal", check_name: "data_loss", tier: "High" }, reason: "Loss", status: "Pending", proposed_by: "other.admin@state.edu", decided_by: null },
      ],
    });
    render(<Authority />);
    const mine = await screen.findByTestId("authority-change-c-1");
    expect(within(mine).getByText(/waiting for a different person/i)).toBeInTheDocument();
    expect(within(mine).queryByRole("button", { name: /confirm/i })).not.toBeInTheDocument();
    const theirs = screen.getByTestId("authority-change-c-2");
    expect(within(theirs).getByRole("button", { name: "Confirm" })).toBeInTheDocument();
    expect(within(theirs).getByRole("button", { name: "Reject" })).toBeInTheDocument();
  });
});
