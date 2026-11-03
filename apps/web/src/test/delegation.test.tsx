import "@testing-library/jest-dom";
import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { DelegationPanel, DelegationBanner, PersonDelegations } from "../DelegationPanel";
import { apiClient, type DelegationRow } from "../api";

const row = (over: Partial<DelegationRow> = {}): DelegationRow => ({
  id: "d1",
  delegator: "rivera@state.edu",
  delegator_name: "Dr. Rivera",
  delegate: "ada@state.edu",
  delegate_name: "Ada Okoro",
  position_key: "chair",
  starts_at: "2026-10-01T00:00:00Z",
  ends_at: "2026-10-14T12:00:00Z",
  reason: "Conference",
  created_at: "2026-10-01T00:00:00Z",
  revoked_at: null,
  state: "active",
  ...over,
});

// Approvers phase 4, test 8: the form needs a person, both dates and a reason. A received delegation shows on the desk.
describe("delegation in the settings pane", () => {
  beforeEach(() => vi.restoreAllMocks());

  it("shows the positions held, and asks for a person, dates and a reason before it delegates", async () => {
    vi.spyOn(apiClient, "listMyDelegations").mockResolvedValue({ made: [], received: [], positions: ["chair"] });
    vi.spyOn(apiClient, "searchDelegates").mockResolvedValue([{ eppn: "ada@state.edu", name: "Ada Okoro" }]);
    const create = vi.spyOn(apiClient, "createDelegation").mockResolvedValue(row());
    render(<DelegationPanel />);
    expect(await screen.findByText(/You hold: chair/)).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Delegate while away" }));
    const save = screen.getByRole("button", { name: "Delegate" });
    expect(save).toBeDisabled();

    fireEvent.change(screen.getByLabelText("Person"), { target: { value: "ada" } });
    fireEvent.click(await screen.findByRole("option", { name: /Ada Okoro.*ada@state.edu/ }));
    expect(save).toBeDisabled();
    fireEvent.change(screen.getByLabelText("From"), { target: { value: "2026-10-01" } });
    fireEvent.change(screen.getByLabelText("Until"), { target: { value: "2026-10-14" } });
    expect(save).toBeDisabled();
    fireEvent.change(screen.getByLabelText("Reason"), { target: { value: "Conference" } });
    expect(save).toBeEnabled();
    fireEvent.click(save);
    await waitFor(() =>
      expect(create).toHaveBeenCalledWith(expect.objectContaining({ delegate: "ada@state.edu", reason: "Conference", starts_at: expect.stringMatching(/^2026-10-01/), ends_at: expect.stringMatching(/^2026-10-14/) }))
    );
  });

  it("lists delegations made, with Revoke, and the ones received", async () => {
    vi.spyOn(apiClient, "listMyDelegations").mockResolvedValue({
      made: [row(), row({ id: "d2", state: "revoked", revoked_at: "2026-10-02T00:00:00Z" })],
      received: [row({ id: "d3", delegator_name: "Dr. Chen", delegator: "chen@state.edu" })],
      positions: ["chair"],
    });
    const revoke = vi.spyOn(apiClient, "revokeDelegation").mockResolvedValue(row({ state: "revoked" }));
    render(<DelegationPanel />);
    expect(await screen.findAllByText(/Ada Okoro/)).toHaveLength(2);
    expect(screen.getAllByRole("button", { name: /Revoke/ })).toHaveLength(1);
    expect(screen.getByText(/Dr. Chen/)).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: /Revoke/ }));
    await waitFor(() => expect(revoke).toHaveBeenCalledWith("d1"));
  });
});

describe("a delegation received, on the desk", () => {
  beforeEach(() => vi.restoreAllMocks());

  it("says whom the person is deciding for and until when", async () => {
    vi.spyOn(apiClient, "listMyDelegations").mockResolvedValue({ made: [], received: [row(), row({ id: "old", state: "ended" })], positions: [] });
    render(<DelegationBanner />);
    expect(await screen.findByText("You are deciding for Dr. Rivera until 14 Oct.")).toBeInTheDocument();
    expect(screen.getAllByText(/You are deciding for/)).toHaveLength(1);
  });

  it("shows nothing when nothing is received", async () => {
    vi.spyOn(apiClient, "listMyDelegations").mockResolvedValue({ made: [], received: [], positions: [] });
    const { container } = render(<DelegationBanner />);
    await waitFor(() => expect(apiClient.listMyDelegations).toHaveBeenCalled());
    expect(container).toBeEmptyDOMElement();
  });
});

describe("a person's delegations, in the admin People drawer", () => {
  beforeEach(() => vi.restoreAllMocks());

  it("lists what the person made and received", async () => {
    vi.spyOn(apiClient, "listDelegations").mockImplementation(async (f) =>
      f.delegator ? [row()] : [row({ id: "d9", delegator: "chen@state.edu", delegate: "rivera@state.edu", state: "ended" })]
    );
    render(<PersonDelegations eppn="rivera@state.edu" />);
    expect(await screen.findByText(/To ada@state.edu as chair/)).toBeInTheDocument();
    expect(screen.getByText(/From chen@state.edu/)).toBeInTheDocument();
    expect(screen.getByText("Ended")).toBeInTheDocument();
  });
});
