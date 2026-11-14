import "@testing-library/jest-dom";
import { afterEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { Guests } from "../admin/Guests";
import { apiClient, type AdminGuests } from "../api";

const panel = (over: Partial<AdminGuests> = {}): AdminGuests => ({
  forms: [{ app: "registrar", form_id: "f1", name: "Ask for help", submissions: 12, last_submission: "2026-11-13T10:00:00Z", link_requests_today: 4, rate_limit_hits: 0, closed: false }],
  guests: [
    { id: "g1", address: "ada@example.org", state: "Active", last_seen: "2026-11-13T09:00:00Z", submissions: 2, forms_used: 1 },
    { id: "g2", address: "Former applicant", state: "Erased", last_seen: null, submissions: 1, forms_used: 1 },
  ],
  abuse: { bounces: [{ address: "bob@example.org", bounces: 3 }], networks: [{ network: "203.0.113.9", hits: 5 }] },
  ...over,
});

afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
});

describe("the Guests panel", () => {
  it("shows the forms, the guests and the signs of abuse", async () => {
    vi.spyOn(apiClient, "adminGuests").mockResolvedValue(panel());
    render(<Guests />);
    const forms = await screen.findByRole("table", { name: "Forms open to guests" });
    expect(within(forms).getByText("Ask for help")).toBeInTheDocument();
    expect(within(forms).getByText("12")).toBeInTheDocument();
    const guests = screen.getByRole("table", { name: "Guests" });
    expect(within(guests).getByText("ada@example.org")).toBeInTheDocument();
    expect(within(guests).getByText("Former applicant")).toBeInTheDocument();
    expect(screen.getByText("bob@example.org: 3 failed mails")).toBeInTheDocument();
    expect(screen.getByText("203.0.113.9: 5 requests over the limit")).toBeInTheDocument();
    // An erased guest has nothing left to act on.
    expect(screen.queryByRole("button", { name: /Erase Former applicant/ })).not.toBeInTheDocument();
  });

  it("closes a form only with a reason, and says what it does first", async () => {
    vi.spyOn(apiClient, "adminGuests").mockResolvedValue(panel());
    const close = vi.spyOn(apiClient, "closeGuestForm").mockResolvedValue(undefined);
    render(<Guests />);
    fireEvent.click(await screen.findByRole("button", { name: "Close Ask for help" }));
    expect(screen.getByText(/existing records are kept/)).toBeInTheDocument();
    const reason = screen.getByRole("textbox");
    fireEvent.change(reason, { target: { value: "Too many applications." } });
    // The form asks with a button of the same name as the row's, and it is the one added last.
    fireEvent.click(screen.getAllByRole("button", { name: "Close Ask for help" }).at(-1) as HTMLElement);
    await waitFor(() => expect(close).toHaveBeenCalledWith("f1", "Too many applications."));
  });

  it("says in words what Erase does, and erases with a reason", async () => {
    vi.spyOn(apiClient, "adminGuests").mockResolvedValue(panel());
    const act = vi.spyOn(apiClient, "actOnGuest").mockResolvedValue(undefined);
    render(<Guests />);
    fireEvent.click(await screen.findByRole("button", { name: "Erase ada@example.org" }));
    expect(screen.getByText(/Their records stay and show Former applicant/)).toBeInTheDocument();
    fireEvent.change(screen.getByRole("textbox"), { target: { value: "Asked to be forgotten." } });
    fireEvent.click(screen.getAllByRole("button", { name: "Erase ada@example.org" }).at(-1) as HTMLElement);
    await waitFor(() => expect(act).toHaveBeenCalledWith("g1", "erase", "Asked to be forgotten."));
  });

  it("offers Open for a closed form and Enable for a disabled guest", async () => {
    vi.spyOn(apiClient, "adminGuests").mockResolvedValue(
      panel({ forms: [{ ...panel().forms[0], closed: true }], guests: [{ ...panel().guests[0], state: "Disabled" }] })
    );
    render(<Guests />);
    expect(await screen.findByRole("button", { name: "Open Ask for help" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Enable ada@example.org" })).toBeInTheDocument();
  });

  it("says plainly when there is nothing to worry about", async () => {
    vi.spyOn(apiClient, "adminGuests").mockResolvedValue(panel({ forms: [], guests: [], abuse: { bounces: [], networks: [] } }));
    render(<Guests />);
    expect(await screen.findByText(/No form is open to guests/)).toBeInTheDocument();
    expect(screen.getByText(/Nothing to show/)).toBeInTheDocument();
  });
});
