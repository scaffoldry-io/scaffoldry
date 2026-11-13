import "@testing-library/jest-dom";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { ShareDialog } from "../ShareDialog";
import { SharedView } from "../SharedView";
import { Shares } from "../admin/Shares";
import { apiClient, type ShareOptions, type ShareRow } from "../api";

const row = (over: Partial<ShareRow> = {}): ShareRow => ({
  id: "s1", app: "grants", view_id: "v1", scope: "institution", allow_download: false, expires_at: "2026-12-13T00:00:00Z",
  created_by: "chair@state.edu", created_at: "2026-11-13T00:00:00Z", revoked_at: null, last_used_at: null, use_count: 0, state: "Active", ...over,
});

const options = (over: Partial<ShareOptions> = {}): ShareOptions => ({
  scopes: ["institution", "public"], public: { allowed: true, reason: null }, default_days: 30, max_days: 90, ...over,
});

afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
});

describe("the Share dialog", () => {
  beforeEach(() => {
    Object.assign(navigator, { clipboard: { writeText: vi.fn().mockResolvedValue(undefined) } });
  });

  it("offers only the scopes that are on, and says in a sentence who can open each", async () => {
    vi.spyOn(apiClient, "listShares").mockResolvedValue({ shares: [], options: options({ scopes: ["institution"], public: { allowed: false, reason: "A Platform Admin has not turned on public links." } }) });
    render(<ShareDialog appSlug="grants" viewId="v1" viewName="All students" />);
    expect(await screen.findByRole("radio", { name: /Institution/ })).toBeInTheDocument();
    expect(screen.queryByRole("radio", { name: /Public/ })).not.toBeInTheDocument();
    expect(screen.getByText(/A Platform Admin has not turned on public links\./)).toBeInTheDocument();
    expect(screen.getByTestId("who-can-open")).toHaveTextContent(/signed in at the institution/i);
    expect(screen.getByTestId("who-can-open")).toHaveTextContent(/only their own rows/i);
  });

  // Test 8 of the brief: the link is shown once, and a public share shows the warning.
  it("shows the link once, with Copy, and not again after the dialog is closed and opened", async () => {
    const list = vi.spyOn(apiClient, "listShares").mockResolvedValue({ shares: [], options: options() });
    const make = vi.spyOn(apiClient, "createShare").mockResolvedValue({ share: row(), token: "a".repeat(32), link: `/s/${"a".repeat(32)}`, warnings: [] });
    const { unmount } = render(<ShareDialog appSlug="grants" viewId="v1" viewName="All students" />);
    fireEvent.click(await screen.findByRole("button", { name: "Create link" }));
    await waitFor(() => expect(make).toHaveBeenCalledWith("grants", "v1", { scope: "institution", expires_in_days: 30, allow_download: false }));
    expect(await screen.findByText(`${window.location.origin}/s/${"a".repeat(32)}`)).toBeInTheDocument();
    expect(screen.getByText(/shown only once/i)).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Copy link" }));
    await waitFor(() => expect(navigator.clipboard.writeText).toHaveBeenCalledWith(`${window.location.origin}/s/${"a".repeat(32)}`));

    unmount();
    list.mockResolvedValue({ shares: [row()], options: options() });
    render(<ShareDialog appSlug="grants" viewId="v1" viewName="All students" />);
    await screen.findByRole("button", { name: "Revoke link created 2026-11-13" });
    expect(screen.queryByText(/a{32}/)).not.toBeInTheDocument();
  });

  it("warns about a public link, and passes on what the server warns about it", async () => {
    vi.spyOn(apiClient, "listShares").mockResolvedValue({ shares: [], options: options() });
    const make = vi.spyOn(apiClient, "createShare").mockResolvedValue({
      share: row({ scope: "public" }), token: "b".repeat(32), link: `/s/${"b".repeat(32)}`,
      warnings: ["A row rule of this table depends on who is looking. A public link has no person, so it shows no rows."],
    });
    render(<ShareDialog appSlug="grants" viewId="v1" viewName="All students" />);
    fireEvent.click(await screen.findByRole("radio", { name: /Public/ }));
    expect(screen.getByRole("status", { name: "Public link" })).toHaveTextContent(/anyone on the internet/i);
    expect(screen.getByTestId("who-can-open")).toHaveTextContent(/anyone with the link/i);
    fireEvent.click(screen.getByRole("button", { name: "Create link" }));
    await waitFor(() => expect(make).toHaveBeenCalledWith("grants", "v1", expect.objectContaining({ scope: "public" })));
    expect(await screen.findByText(/shows no rows/)).toBeInTheDocument();
  });

  it("sets the expiry and the download choice, and revokes a link from the list", async () => {
    const list = vi.spyOn(apiClient, "listShares").mockResolvedValue({ shares: [row()], options: options() });
    const make = vi.spyOn(apiClient, "createShare").mockResolvedValue({ share: row(), token: "c".repeat(32), link: `/s/${"c".repeat(32)}`, warnings: [] });
    const revoke = vi.spyOn(apiClient, "revokeShare").mockResolvedValue(undefined);
    render(<ShareDialog appSlug="grants" viewId="v1" viewName="All students" />);
    fireEvent.change(await screen.findByLabelText("Days until it expires"), { target: { value: "7" } });
    fireEvent.click(screen.getByLabelText("Allow download"));
    fireEvent.click(screen.getByRole("button", { name: "Create link" }));
    await waitFor(() => expect(make).toHaveBeenCalledWith("grants", "v1", { scope: "institution", expires_in_days: 7, allow_download: true }));
    list.mockResolvedValue({ shares: [], options: options() });
    fireEvent.click(await screen.findByRole("button", { name: "Revoke link created 2026-11-13" }));
    await waitFor(() => expect(revoke).toHaveBeenCalledWith("grants", "v1", "s1"));
  });

  it("will not take an expiry past the most the server allows", async () => {
    vi.spyOn(apiClient, "listShares").mockResolvedValue({ shares: [], options: options() });
    const make = vi.spyOn(apiClient, "createShare");
    render(<ShareDialog appSlug="grants" viewId="v1" viewName="All students" />);
    fireEvent.change(await screen.findByLabelText("Days until it expires"), { target: { value: "120" } });
    expect(screen.getByRole("button", { name: "Create link" })).toBeDisabled();
    expect(screen.getByText(/1 to 90 days/)).toBeInTheDocument();
    expect(make).not.toHaveBeenCalled();
  });
});

describe("the Shares panel of the console", () => {
  it("lists each link with its app, view, scope, use and state, and never a token", async () => {
    vi.spyOn(apiClient, "adminShares").mockResolvedValue([
      { ...row({ use_count: 3 }), app_title: "Grants", view_name: "All students" },
      { ...row({ id: "s2", scope: "public", state: "Revoked", revoked_at: "2026-11-14T00:00:00Z" }), app_title: "Grants", view_name: "All students" },
    ]);
    render(<Shares />);
    const first = (await screen.findAllByText("All students"))[0].closest("tr") as HTMLElement;
    expect(within(first).getByText("Grants")).toBeInTheDocument();
    expect(within(first).getByText("institution")).toBeInTheDocument();
    expect(within(first).getByText("3")).toBeInTheDocument();
    expect(within(first).getByText("Active")).toBeInTheDocument();
    expect(screen.getByText("Revoked")).toBeInTheDocument();
  });

  it("revokes every link of an app only with a reason", async () => {
    vi.spyOn(apiClient, "adminShares").mockResolvedValue([{ ...row(), app_title: "Grants", view_name: "All students" }]);
    const revoke = vi.spyOn(apiClient, "revokeAppShares").mockResolvedValue(1);
    render(<Shares />);
    fireEvent.click(await screen.findByRole("button", { name: "Revoke all for Grants" }));
    const confirm = screen.getByRole("button", { name: "Revoke all for Grants" });
    expect(confirm).toBeDisabled();
    fireEvent.change(screen.getByRole("textbox", { name: /reason/i }), { target: { value: "The app moved to a new owner" } });
    fireEvent.click(confirm);
    await waitFor(() => expect(revoke).toHaveBeenCalledWith("grants", "The app moved to a new owner"));
  });
});

describe("a shared link opened", () => {
  it("shows the view's rows and what the link is", async () => {
    vi.spyOn(apiClient, "sharedView").mockResolvedValue({ name: "All students", kind: "grid", app_title: "Grants", scope: "public", allow_download: false, expires_at: "2026-12-13T00:00:00Z" });
    vi.spyOn(apiClient, "sharedData").mockResolvedValue({ view_id: "v1", records: [{ id: "r1", data: { name: "Ada", program: "p1" } }], next_cursor: null } as never);
    render(<SharedView token={"t".repeat(32)} />);
    expect(await screen.findByRole("heading", { name: "All students" })).toBeInTheDocument();
    expect(screen.getByText("Ada")).toBeInTheDocument();
    expect(screen.getByText(/Read only/)).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Download" })).not.toBeInTheDocument();
  });

  it("says the link is closed when the server does not find it", async () => {
    vi.spyOn(apiClient, "sharedView").mockRejectedValue(Object.assign(new Error("This link does not exist or is no longer open"), { status: 404 }));
    render(<SharedView token={"x".repeat(32)} />);
    expect(await screen.findByText(/no longer open/)).toBeInTheDocument();
  });

  it("offers a download only when the link allows one", async () => {
    vi.spyOn(apiClient, "sharedView").mockResolvedValue({ name: "All students", kind: "grid", app_title: "Grants", scope: "institution", allow_download: true, expires_at: "2026-12-13T00:00:00Z" });
    vi.spyOn(apiClient, "sharedData").mockResolvedValue({ view_id: "v1", records: [], next_cursor: null } as never);
    const get = vi.spyOn(apiClient, "sharedDownload").mockResolvedValue("name\nAda\n");
    URL.createObjectURL = vi.fn(() => "blob:csv");
    URL.revokeObjectURL = vi.fn();
    render(<SharedView token={"d".repeat(32)} />);
    fireEvent.click(await screen.findByRole("button", { name: "Download" }));
    await waitFor(() => expect(get).toHaveBeenCalledWith("d".repeat(32)));
  });
});
