import "@testing-library/jest-dom";
import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, within, fireEvent, waitFor } from "@testing-library/react";
import { apiClient, type AccessChange } from "../api";
import { AccessDrafts } from "../admin/AccessDrafts";

const draft: AccessChange = {
  change_id: "d-1",
  kind: "grant",
  grant: { holder_id: "target@state.edu", role: "viewer" },
  hierarchy: null,
  widens: true,
  reason: "The audit needs it",
  status: "Pending",
  proposed_by: "admin@state.edu",
  proposed_at: "2026-10-26T10:00:00Z",
  proposed_via: "agent",
  drafted_for: "admin@state.edu",
  via_client: "client-1",
  task: "Give the auditor access",
  sentence: "Give person target@state.edu the viewer role on app:audit.",
  expires_at: "2026-11-02T10:00:00Z",
  expired: false,
};
const own: AccessChange = { ...draft, change_id: "p-1", proposed_via: "person", drafted_for: null, task: null, sentence: null, expires_at: null };
const narrowing: AccessChange = { ...draft, change_id: "d-2", kind: "revoke", widens: false, sentence: "Take away: viewer role of target@state.edu." };
const old: AccessChange = { ...draft, change_id: "d-3", expired: true };

describe("Drafts from an AI tool", () => {
  beforeEach(() => vi.restoreAllMocks());

  it("says who a draft was made for, with which task, what it does, and when it ends", () => {
    render(<AccessDrafts changes={[draft, own]} viewer={{ eppn: "second@state.edu", platform: true }} onChanged={() => {}} />);
    const card = screen.getByTestId("draft-d-1");
    expect(within(card).getByText(/drafted by an ai tool used by admin@state.edu/i)).toBeInTheDocument();
    expect(within(card).getByText(/give the auditor access/i)).toBeInTheDocument();
    expect(within(card).getByText(/give person target@state.edu the viewer role/i)).toBeInTheDocument();
    expect(within(card).getByText(/ends 2026-11-02/i)).toBeInTheDocument();
    expect(screen.queryByTestId("draft-p-1")).not.toBeInTheDocument();
  });

  it("does not offer Confirm to the person a widening draft was made for, and says why", () => {
    render(<AccessDrafts changes={[draft]} viewer={{ eppn: "ADMIN@state.edu", platform: true }} onChanged={() => {}} />);
    const card = screen.getByTestId("draft-d-1");
    expect(within(card).queryByRole("button", { name: /confirm/i })).not.toBeInTheDocument();
    expect(within(card).getByText(/a different person must confirm/i)).toBeInTheDocument();
  });

  it("offers Confirm to someone else, and to the same person for a draft that only narrows access", async () => {
    const decide = vi.spyOn(apiClient, "accessDecideChange").mockResolvedValue({ change_id: "d-1", status: "Confirmed", grant_id: 5 });
    const done = vi.fn();
    const { unmount } = render(<AccessDrafts changes={[draft]} viewer={{ eppn: "second@state.edu", platform: true }} onChanged={done} />);
    fireEvent.click(within(screen.getByTestId("draft-d-1")).getByRole("button", { name: /confirm/i }));
    fireEvent.change(await screen.findByLabelText(/reason for decision/i), { target: { value: "Reviewed" } });
    fireEvent.click(screen.getByRole("button", { name: /^confirm draft$/i }));
    await waitFor(() => expect(decide).toHaveBeenCalledWith("d-1", "confirm", "Reviewed"));
    unmount();
    render(<AccessDrafts changes={[narrowing]} viewer={{ eppn: "admin@state.edu", platform: true }} onChanged={() => {}} />);
    expect(within(screen.getByTestId("draft-d-2")).getByRole("button", { name: /confirm/i })).toBeInTheDocument();
  });

  it("shows a draft that ended as expired and cannot be confirmed", () => {
    render(<AccessDrafts changes={[old]} viewer={{ eppn: "second@state.edu", platform: true }} onChanged={() => {}} />);
    const card = screen.getByTestId("draft-d-3");
    expect(within(card).getByText(/expired/i)).toBeInTheDocument();
    expect(within(card).queryByRole("button", { name: /confirm/i })).not.toBeInTheDocument();
  });
});
