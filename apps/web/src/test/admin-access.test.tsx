import "@testing-library/jest-dom";
import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, waitFor, fireEvent, within } from "@testing-library/react";
import { Access } from "../admin/Access";
import { apiClient } from "../api";
import { grantSentence } from "../admin/accessSentences";

const academic = { key: "academic", name: "Academic and administrative", kind: "units", source: "api", status: "Active", created_by: "system", created_at: "", retired_at: null };

const pendingHierarchy = {
  change_id: "c-1",
  kind: "hierarchy",
  grant: null,
  hierarchy: { key: "supervision", name: "Supervision", kind: "people", source: "scim" },
  widens: false,
  reason: "Supervisors see their reports",
  status: "Pending",
  proposed_by: "jordan.lee@state.edu",
  proposed_at: "2026-10-25T00:00:00Z",
};

function asUser(eppn: string, platform = true) {
  vi.spyOn(apiClient, "getCurrentUser").mockResolvedValue({ user: { eppn, name: eppn, affiliation: platform ? "central_admin" : "faculty" }, is_impersonating: false, original_admin: null, is_platform_admin: platform });
}

function quietDefaults() {
  vi.spyOn(apiClient, "accessHierarchies").mockResolvedValue([academic]);
  vi.spyOn(apiClient, "accessGrants").mockResolvedValue({ grants: [], changes: [], next_cursor: null });
  vi.spyOn(apiClient, "accessReviews").mockResolvedValue({ review_days: 365, due: [], conflicts: [], next_cursor: null });
  vi.spyOn(apiClient, "accessReceipts").mockResolvedValue({ receipts: [], next_cursor: null });
}

describe("Access section: hierarchies", () => {
  beforeEach(() => {
    vi.restoreAllMocks();
    quietDefaults();
  });

  it("a hierarchy one admin proposed is not usable, and that admin cannot confirm it", async () => {
    asUser("jordan.lee@state.edu");
    vi.spyOn(apiClient, "accessGrants").mockResolvedValue({ grants: [], changes: [pendingHierarchy], next_cursor: null });
    render(<Access />);
    const pending = await screen.findByTestId("hierarchy-pending-c-1");
    expect(within(pending).getByText(/waiting for a different person to confirm/i)).toBeInTheDocument();
    expect(within(pending).getByText("Supervision")).toBeInTheDocument();
    expect(within(pending).queryByRole("button", { name: /confirm/i })).not.toBeInTheDocument();
    // It is not in the list of hierarchies that exist.
    const live = screen.getByTestId("hierarchy-list");
    expect(within(live).queryByText("Supervision")).not.toBeInTheDocument();
    expect(within(live).getByText("Academic and administrative")).toBeInTheDocument();
  });

  it("a different admin can confirm it, with a reason", async () => {
    asUser("second.admin@state.edu");
    vi.spyOn(apiClient, "accessGrants").mockResolvedValue({ grants: [], changes: [pendingHierarchy], next_cursor: null });
    const decide = vi.spyOn(apiClient, "accessDecideChange").mockResolvedValue({ change_id: "c-1", status: "Confirmed", grant_id: null });
    render(<Access />);
    const pending = await screen.findByTestId("hierarchy-pending-c-1");
    fireEvent.click(within(pending).getByRole("button", { name: /confirm/i }));
    const confirm = await screen.findByRole("button", { name: /confirm hierarchy/i });
    expect(confirm).toBeDisabled();
    fireEvent.change(screen.getByLabelText(/reason for decision/i), { target: { value: "Reviewed" } });
    fireEvent.click(confirm);
    await waitFor(() => expect(decide).toHaveBeenCalledWith("c-1", "confirm", "Reviewed"));
  });

  it("New hierarchy proposes a change and says who must confirm it", async () => {
    asUser("jordan.lee@state.edu");
    const create = vi.spyOn(apiClient, "accessCreateHierarchy").mockResolvedValue({ change_id: "c-9", status: "Pending", message: "A different Platform Admin must confirm this hierarchy before it exists." });
    render(<Access />);
    fireEvent.click(await screen.findByRole("button", { name: /new hierarchy/i }));
    fireEvent.change(await screen.findByLabelText(/^key/i), { target: { value: "campus" } });
    fireEvent.change(screen.getByLabelText(/^name/i), { target: { value: "Campus" } });
    fireEvent.change(screen.getByLabelText(/^kind/i), { target: { value: "people" } });
    fireEvent.change(screen.getByLabelText(/^source/i), { target: { value: "scim" } });
    fireEvent.change(screen.getByLabelText(/^reason/i), { target: { value: "Supervisors" } });
    fireEvent.click(screen.getByRole("button", { name: /propose hierarchy/i }));
    await waitFor(() => expect(create).toHaveBeenCalledWith({ key: "campus", name: "Campus", kind: "people", source: "scim", reason: "Supervisors" }));
    expect(await screen.findByText(/a different platform admin must confirm/i)).toBeInTheDocument();
  });

  it("Retire needs a reason, and says its units stay", async () => {
    asUser("jordan.lee@state.edu");
    vi.spyOn(apiClient, "accessHierarchies").mockResolvedValue([academic, { ...academic, key: "campus", name: "Campus" }]);
    const retire = vi.spyOn(apiClient, "accessRetireHierarchy").mockResolvedValue({ key: "campus", status: "Retired" });
    render(<Access />);
    const row = await screen.findByTestId("hierarchy-row-campus");
    fireEvent.click(within(row).getByRole("button", { name: /retire/i }));
    expect(await screen.findByText(/its units and coordinates stay/i)).toBeInTheDocument();
    const go = screen.getByRole("button", { name: /retire hierarchy/i });
    expect(go).toBeDisabled();
    fireEvent.change(screen.getByLabelText(/reason for decision/i), { target: { value: "Not used" } });
    fireEvent.click(go);
    await waitFor(() => expect(retire).toHaveBeenCalledWith("campus", "Not used"));
    // The academic hierarchy has no Retire.
    expect(within(screen.getByTestId("hierarchy-row-academic")).queryByRole("button", { name: /retire/i })).not.toBeInTheDocument();
  });
});

describe("Access section: the grant form", () => {
  beforeEach(() => {
    vi.restoreAllMocks();
    quietDefaults();
    asUser("jordan.lee@state.edu");
  });

  const preview = { grant: {}, impact: { change_id: "00000000-0000-0000-0000-000000000000", kind: "grant", gain: [{ person: "dana@state.edu", container: "workspace:ws-1", to: "viewer" }], lose: [], per_role: { viewer: { gain: 1, lose: 0 } }, people_considered: 1, truncated: false } };

  async function openForm() {
    vi.spyOn(apiClient, "accessGrantAnchors").mockResolvedValue({ units: [{ id: "u-1", name: "Physics", hierarchy: "academic" }], domains: [{ key: "research", name: "Research" }] });
    render(<Access />);
    fireEvent.click(await screen.findByRole("tab", { name: /grants/i }));
    fireEvent.click(await screen.findByRole("button", { name: /new grant/i }));
    return screen.findByTestId("grant-form");
  }

  function fill() {
    fireEvent.change(screen.getByLabelText(/^person/i), { target: { value: "dana@state.edu" } });
    fireEvent.change(screen.getByLabelText(/^role/i), { target: { value: "viewer" } });
    fireEvent.change(screen.getByLabelText(/^unit/i), { target: { value: "u-1" } });
    fireEvent.change(screen.getByLabelText(/valid until/i), { target: { value: "2026-12-01" } });
    fireEvent.change(screen.getByLabelText(/^reason/i), { target: { value: "Needs to see the lab" } });
  }

  it("shows the sentence and the impact before Propose is enabled, and disables it again when a value changes", async () => {
    const prev = vi.spyOn(apiClient, "accessPreviewGrant").mockResolvedValue(preview as never);
    const propose = vi.spyOn(apiClient, "accessProposeGrant").mockResolvedValue({ change_id: "c-2", status: "Pending", widens: true, message: "x" });
    await openForm();
    fill();
    const proposeButton = screen.getByRole("button", { name: /^propose/i });
    expect(proposeButton).toBeDisabled();
    fireEvent.click(screen.getByRole("button", { name: /show what this does/i }));
    expect(await screen.findByTestId("grant-sentence")).toHaveTextContent("dana@state.edu can view everything placed in Physics and the units below it");
    expect(screen.getByTestId("grant-impact")).toHaveTextContent(/1 person gains access/i);
    expect(prev).toHaveBeenCalledTimes(1);
    expect(proposeButton).toBeEnabled();

    // A changed value takes the preview away. Propose is disabled until it is shown again.
    fireEvent.change(screen.getByLabelText(/^role/i), { target: { value: "editor" } });
    await waitFor(() => expect(proposeButton).toBeDisabled());
    expect(screen.queryByTestId("grant-sentence")).not.toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: /show what this does/i }));
    await screen.findByTestId("grant-sentence");
    fireEvent.click(proposeButton);
    await waitFor(() => expect(propose).toHaveBeenCalled());
    expect(propose.mock.calls[0][0]).toMatchObject({ holder_kind: "person", holder_id: "dana@state.edu", role: "editor", anchor_kind: "unit", anchor_id: "u-1", source: "request", reason: "Needs to see the lab" });
  });

  it("a grant the server refuses shows the server's words and stays unproposed", async () => {
    vi.spyOn(apiClient, "accessPreviewGrant").mockRejectedValue(new Error("valid_until may be at most 180 days from now"));
    await openForm();
    fill();
    fireEvent.click(screen.getByRole("button", { name: /show what this does/i }));
    expect(await screen.findByText(/at most 180 days/i)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /^propose/i })).toBeDisabled();
  });
});

describe("grantSentence", () => {
  it("says who, what, where, how far down, in which domain and up to what", () => {
    const base = { holder: "Dana Reyes", role: "viewer", anchor: "Physics", depth: null as number | null, domain: null as string | null, clearance: null as string | null, anchorKind: "unit" };
    expect(grantSentence(base)).toBe("Dana Reyes can view everything placed in Physics and the units below it.");
    expect(grantSentence({ ...base, depth: 0 })).toBe("Dana Reyes can view everything placed in Physics itself.");
    expect(grantSentence({ ...base, depth: 1 })).toBe("Dana Reyes can view everything placed in Physics and the units one level below it.");
    expect(grantSentence({ ...base, domain: "Research", clearance: "Internal" })).toBe("Dana Reyes can view everything placed in Physics and the units below it, in Research, up to Internal.");
    expect(grantSentence({ ...base, role: "editor", anchorKind: "container", anchor: "the Biology workspace" })).toBe("Dana Reyes can edit everything in the Biology workspace.");
    expect(grantSentence({ ...base, holder: "Every department chair", anchorKind: "holder_unit", anchor: "their own unit" })).toBe("Every department chair can view everything placed in their own unit and the units below it.");
  });
});

describe("Access section: people and explain", () => {
  beforeEach(() => {
    vi.restoreAllMocks();
    quietDefaults();
    asUser("jordan.lee@state.edu");
  });

  it("Explain shows each grant as a sentence, with a link to the grant and a path through named units", async () => {
    vi.spyOn(apiClient, "accessSearchPeople").mockResolvedValue({ people: [{ eppn: "dana@state.edu", name: "Dana Reyes" }], next_cursor: null });
    vi.spyOn(apiClient, "accessReviewPerson").mockResolvedValue({ person: "dana@state.edu", holder_set: { positions: [], units: [], groups: [] }, grants: [], by_hierarchy: {} });
    const explain = vi.spyOn(apiClient, "accessExplain").mockResolvedValue({
      person: "dana@state.edu",
      object: "workspace:ws-physics",
      role: "viewer",
      platform_admin: false,
      reached: [{ grant_id: 12, source: "stored", holder: "person:dana@state.edu", role: "viewer", anchor: "unit:u-1", anchor_name: "Science", depth: null, domain: null, path: ["u-1", "u-2"], path_names: ["Science", "Physics"] }],
      missed: [{ grant_id: 13, source: "stored", holder: "person:dana@state.edu", role: "editor", anchor: "unit:u-9", anchor_name: "Chemistry", depth: null, domain: null, axis: "unit:academic" }],
      sealed_by: [],
      policy: null,
      guards: [],
      rules: [],
    } as never);
    render(<Access />);
    fireEvent.click(await screen.findByRole("tab", { name: /people/i }));
    fireEvent.change(await screen.findByLabelText(/search people/i), { target: { value: "dana" } });
    fireEvent.click(await screen.findByRole("button", { name: /dana reyes/i }));
    fireEvent.change(await screen.findByLabelText(/^object/i), { target: { value: "workspace:ws-physics" } });
    fireEvent.click(screen.getByRole("button", { name: /^explain/i }));
    await waitFor(() => expect(explain).toHaveBeenCalledWith("dana@state.edu", "workspace:ws-physics"));
    const reached = await screen.findByTestId("explain-reached-12");
    expect(reached).toHaveTextContent("dana@state.edu can view everything placed in Science and the units below it");
    expect(within(reached).getByText("Science › Physics")).toBeInTheDocument();
    expect(within(reached).getByRole("link", { name: /grant 12/i })).toHaveAttribute("href", "/admin/access/grants/12");
    expect(reached).not.toHaveTextContent("u-1");
    const missed = screen.getByTestId("explain-missed-13");
    expect(missed).toHaveTextContent(/does not reach this object/i);
    expect(missed).toHaveTextContent(/unit/i);
  });
});

describe("Access section: who sees what", () => {
  beforeEach(() => {
    vi.restoreAllMocks();
    quietDefaults();
  });

  it("an Org Unit Admin has no Hierarchies or Receipts, and the grants shown are the server's, inside their units", async () => {
    asUser("unit.admin@state.edu", false);
    const grants = vi.spyOn(apiClient, "accessGrants").mockResolvedValue({
      grants: [{ id: 5, holder_kind: "person", holder_id: "dana@state.edu", role: "viewer", hierarchy: "academic", anchor_kind: "unit", anchor_id: "u-1", anchor_name: "Physics", depth: null, domain: null, clearance: null, valid_from: "", valid_until: null, revoked_at: null, revoked_by: null, source: "appointment", reason: "r", created_by: "x", created_at: "" }],
      changes: [],
      next_cursor: null,
    });
    render(<Access />);
    expect(await screen.findByRole("tab", { name: /grants/i })).toBeInTheDocument();
    expect(screen.queryByRole("tab", { name: /hierarchies/i })).not.toBeInTheDocument();
    expect(screen.queryByRole("tab", { name: /receipts/i })).not.toBeInTheDocument();
    expect(await screen.findByText("dana@state.edu")).toBeInTheDocument();
    expect(screen.getByText(/the units you administer/i)).toBeInTheDocument();
    expect(grants).toHaveBeenCalled();
  });
});

describe("Access section: reviews and receipts", () => {
  beforeEach(() => {
    vi.restoreAllMocks();
    quietDefaults();
    asUser("jordan.lee@state.edu");
  });

  it("Reviews lists due grants and conflicts with Keep and Revoke, each with a reason", async () => {
    vi.spyOn(apiClient, "accessReviews").mockResolvedValue({
      review_days: 365,
      due: [{ id: 7, holder_kind: "person", holder_id: "old@state.edu", role: "viewer", hierarchy: "academic", anchor_kind: "unit", anchor_id: "u-1", anchor_name: "Physics", depth: null, domain: null, clearance: null, valid_from: "", valid_until: null, revoked_at: null, revoked_by: null, source: "appointment", reason: "r", created_by: "x", created_at: "" }],
      conflicts: [{ person: "both@state.edu", pair: "the position 'dean' and the role 'admin' on 'research'" }],
      next_cursor: null,
    });
    const review = vi.spyOn(apiClient, "accessReviewGrant").mockResolvedValue({ id: 7, status: "Kept" });
    render(<Access />);
    fireEvent.click(await screen.findByRole("tab", { name: /reviews/i }));
    const row = await screen.findByTestId("review-row-7");
    expect(screen.getByText(/both@state.edu/)).toBeInTheDocument();
    fireEvent.click(within(row).getByRole("button", { name: /keep/i }));
    const go = await screen.findByRole("button", { name: /confirm keep/i });
    expect(go).toBeDisabled();
    fireEvent.change(screen.getByLabelText(/reason for decision/i), { target: { value: "Still needed" } });
    fireEvent.click(go);
    await waitFor(() => expect(review).toHaveBeenCalledWith(7, "keep", "Still needed"));
  });

  it("Receipts lists a page of receipts and can check the chain", async () => {
    vi.spyOn(apiClient, "accessReceipts").mockResolvedValue({
      receipts: [{ id: 3, at: "2026-10-25T00:00:00Z", actor: "dana@state.edu", acting_as: null, action: "read_app", object_kind: "app", object_id: "tickets", outcome: "deny", grant_ids: [], policy_ids: [], narrowed_by: [] }],
      next_cursor: null,
    });
    const verify = vi.spyOn(apiClient, "accessVerifyReceipts").mockResolvedValue({ ok: true, batches_checked: 4, failed_batch: null, reason: null });
    render(<Access />);
    fireEvent.click(await screen.findByRole("tab", { name: /receipts/i }));
    expect(await screen.findByText("dana@state.edu")).toBeInTheDocument();
    expect(screen.getByText(/deny/i)).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: /check the chain/i }));
    await waitFor(() => expect(verify).toHaveBeenCalled());
    expect(await screen.findByText(/unbroken/i)).toBeInTheDocument();
  });
});
