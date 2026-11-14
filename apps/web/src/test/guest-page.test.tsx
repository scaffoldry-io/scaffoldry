import "@testing-library/jest-dom";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { GuestPage } from "../GuestPage";
import { ApiError, guestApi, guestSession, type GuestOverview } from "../api";

const overview = (over: Partial<GuestOverview> = {}): GuestOverview => ({
  form: {
    name: "Ask for help",
    success_message: "Thanks.",
    fields: [
      { name: "kind", label: "What do you need?", type: "Select", required: true, read_only: false, options: [{ id: "grant", label: "A grant" }, { id: "loan", label: "A loan" }] },
      { name: "amount", label: "How much?", type: "Number", required: false, read_only: false, options: [] },
      { name: "status", label: "Status", type: "Select", required: false, read_only: true, options: [{ id: "new", label: "Received" }] },
    ],
  },
  records: [
    { id: "r1", version: 1, data: { kind: "grant", amount: 40, status: "new" }, standing: "Where it stands: Received.", editable: true },
    { id: "r2", version: 3, data: { kind: "loan", amount: 5, status: "decided" }, standing: "Where it stands: Decided. It can no longer be changed.", editable: false },
  ],
  can_add: true,
  remaining_today: 2,
  ...over,
});

const session = { token: "scf_guest", app: "registrar", form: "f1" };

beforeEach(() => {
  sessionStorage.clear();
  guestSession.write(session);
});

afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
  sessionStorage.clear();
});

describe("the guest's status page", () => {
  it("lists the guest's requests with a plain sentence for each, and Edit only while one is open", async () => {
    vi.spyOn(guestApi, "overview").mockResolvedValue(overview());
    render(<GuestPage route={{ kind: "page", app: "registrar", form: "f1" }} />);
    expect(await screen.findByRole("heading", { name: "Ask for help" })).toBeInTheDocument();
    expect(screen.getByText("Where it stands: Received.")).toBeInTheDocument();
    expect(screen.getByText(/can no longer be changed/)).toBeInTheDocument();
    expect(screen.getAllByRole("button", { name: /^Edit/ })).toHaveLength(1);
    expect(screen.getByRole("button", { name: "Add another" })).toBeInTheDocument();
    // The option's words are shown, not its id.
    expect(screen.getAllByText("A grant").length).toBeGreaterThan(0);
  });

  it("has no link out of the guest area, and every control is a native one the keyboard reaches", async () => {
    vi.spyOn(guestApi, "overview").mockResolvedValue(overview());
    const { container } = render(<GuestPage route={{ kind: "page", app: "registrar", form: "f1" }} />);
    await screen.findByRole("heading", { name: "Ask for help" });
    expect(container.querySelectorAll("a, [role='link'], [role='menu'], nav")).toHaveLength(0);
    for (const el of Array.from(container.querySelectorAll("[onclick], [role='button']"))) {
      expect(["BUTTON", "INPUT", "SELECT", "TEXTAREA"]).toContain(el.tagName);
    }
    expect(container.querySelectorAll("[tabindex]:not([tabindex='-1'])")).toHaveLength(0);
  });

  it("says so, and offers no Add another, when the limit is reached", async () => {
    vi.spyOn(guestApi, "overview").mockResolvedValue(overview({ can_add: false, remaining_today: 0 }));
    render(<GuestPage route={{ kind: "page", app: "registrar", form: "f1" }} />);
    await screen.findByRole("heading", { name: "Ask for help" });
    expect(screen.queryByRole("button", { name: "Add another" })).not.toBeInTheDocument();
    expect(screen.getByText(/most this form takes/)).toBeInTheDocument();
  });

  it("adds another request through labelled fields and a form that Enter submits", async () => {
    const get = vi.spyOn(guestApi, "overview").mockResolvedValue(overview());
    const create = vi.spyOn(guestApi, "create").mockResolvedValue({ record: overview().records[0] });
    render(<GuestPage route={{ kind: "page", app: "registrar", form: "f1" }} />);
    fireEvent.click(await screen.findByRole("button", { name: "Add another" }));
    expect(screen.queryByLabelText("Status")).not.toBeInTheDocument();
    fireEvent.change(screen.getByLabelText(/What do you need/), { target: { value: "loan" } });
    fireEvent.change(screen.getByLabelText(/How much/), { target: { value: "12" } });
    fireEvent.submit(screen.getByRole("form", { name: "Add another" }));
    await waitFor(() => expect(create).toHaveBeenCalledWith(session, { kind: "loan", amount: 12 }));
    expect(await screen.findByText("Thanks.")).toBeInTheDocument();
    expect(get.mock.calls.length).toBeGreaterThan(1);
  });

  it("edits an open request with the version it read, and shows a refusal in words", async () => {
    vi.spyOn(guestApi, "overview").mockResolvedValue(overview());
    const update = vi.spyOn(guestApi, "update").mockRejectedValue(new ApiError(409, "This request is being handled and can no longer be changed."));
    render(<GuestPage route={{ kind: "page", app: "registrar", form: "f1" }} />);
    fireEvent.click(await screen.findByRole("button", { name: /^Edit/ }));
    fireEvent.change(screen.getByLabelText(/How much/), { target: { value: "45" } });
    fireEvent.click(screen.getByRole("button", { name: "Save" }));
    await waitFor(() => expect(update).toHaveBeenCalledWith(session, "r1", 1, { kind: "grant", amount: 45 }));
    expect(await screen.findByRole("alert")).toHaveTextContent(/can no longer be changed/);
  });
});

describe("signing in", () => {
  it("asks for an address when there is no session, hides the honeypot from everyone, and gives the neutral answer", async () => {
    guestSession.clear();
    const start = vi.spyOn(guestApi, "start").mockResolvedValue({ message: "If that address can use this form, a link is on its way." });
    const { container } = render(<GuestPage route={{ kind: "page", app: "registrar", form: "f1" }} />);
    const honeypot = container.querySelector("input[name='website']") as HTMLInputElement;
    expect(honeypot).not.toBeNull();
    expect(honeypot.closest("[aria-hidden='true']")).not.toBeNull();
    expect(honeypot.tabIndex).toBe(-1);
    fireEvent.change(screen.getByLabelText("Email address"), { target: { value: "ada@example.org" } });
    fireEvent.click(screen.getByRole("button", { name: "Send me a link" }));
    await waitFor(() => expect(start).toHaveBeenCalledWith("registrar", "f1", "ada@example.org", ""));
    expect(await screen.findByText("If that address can use this form, a link is on its way.")).toBeInTheDocument();
  });

  it("asks again when the session has run out", async () => {
    vi.spyOn(guestApi, "overview").mockRejectedValue(new ApiError(401, "A valid session is required"));
    render(<GuestPage route={{ kind: "page", app: "registrar", form: "f1" }} />);
    expect(await screen.findByLabelText("Email address")).toBeInTheDocument();
    expect(guestSession.read()).toBeNull();
  });

  it("trades a link for a session, keeps it for the tab, and moves to the page", async () => {
    guestSession.clear();
    vi.spyOn(guestApi, "verify").mockResolvedValue({ token: "scf_new", app: "registrar", form: "f1" });
    vi.spyOn(guestApi, "overview").mockResolvedValue(overview());
    const replace = vi.spyOn(window.history, "replaceState").mockImplementation(() => undefined);
    render(<GuestPage route={{ kind: "link", token: "a".repeat(32) }} />);
    expect(await screen.findByRole("heading", { name: "Ask for help" })).toBeInTheDocument();
    expect(guestSession.read()).toEqual({ token: "scf_new", app: "registrar", form: "f1" });
    expect(replace).toHaveBeenCalledWith(null, "", "/g/registrar/f1");
  });

  it("says in words when a link has been used", async () => {
    guestSession.clear();
    vi.spyOn(guestApi, "verify").mockRejectedValue(new ApiError(404, "This link does not work. It may have been used, or it may have expired."));
    render(<GuestPage route={{ kind: "link", token: "a".repeat(32) }} />);
    expect(await screen.findByRole("alert")).toHaveTextContent(/may have been used/);
  });
});

describe("stopping mail", () => {
  it("stops mail only when the guest presses the button, and says so", async () => {
    const stop = vi.spyOn(guestApi, "stop").mockResolvedValue({ message: "You will not get mail about your requests any more." });
    render(<GuestPage route={{ kind: "stop", token: "b".repeat(32) }} />);
    expect(stop).not.toHaveBeenCalled();
    fireEvent.click(screen.getByRole("button", { name: "Stop these mails" }));
    await waitFor(() => expect(stop).toHaveBeenCalledWith("b".repeat(32)));
    expect(await screen.findByText(/will not get mail/)).toBeInTheDocument();
  });
});
