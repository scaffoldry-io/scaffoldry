import "@testing-library/jest-dom";
import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent, waitFor, within } from "@testing-library/react";
import { OutsideApps } from "../admin/OutsideApps";
import { AdminConsole } from "../admin/AdminConsole";
import { AdminDesk } from "../AdminDesk";
import { apiClient, type AppClientRow, type ClientUsage } from "../api";

const active: AppClientRow = {
  client_id: "scc_a",
  name: "Grant tracker",
  operator: "Physics Department",
  kind: "public",
  status: "Active",
  sentences: ["Read the title and status of Proposals in Grants Office."],
  workspaces: [{ id: "ws-optics", name: "Optics Lab" }],
  connected_users: 2,
  last_call_at: "2026-10-30T01:02:03Z",
  service_identity: false,
};

const pending: AppClientRow = {
  ...active,
  client_id: "scc_p",
  name: "Nightly summary",
  operator: "Registrar",
  kind: "confidential",
  status: "Pending",
  sentences: ["Read the name of Students in Registry."],
  workspaces: [{ id: "ws-reg", name: "Registry" }],
  connected_users: 0,
  last_call_at: null,
};

function usage(): ClientUsage {
  const days = Array.from({ length: 30 }, (_, i) => ({ day: `2026-10-${String(i + 1).padStart(2, "0")}`, calls: 0, records_read: 0, records_written: 0 }));
  days[29] = { day: "2026-10-30", calls: 6, records_read: 6, records_written: 2 };
  return { client_id: "scc_a", days };
}

describe("Outside apps", () => {
  beforeEach(() => {
    vi.restoreAllMocks();
  });

  it("lists each app with its operator, kind, status, workspaces, ceiling in sentences, people connected and last call", async () => {
    vi.spyOn(apiClient, "listClients").mockResolvedValue([active, pending]);
    render(<OutsideApps />);
    const row = await screen.findByTestId("outside-app-scc_a");
    expect(within(row).getByText("Grant tracker")).toBeInTheDocument();
    expect(within(row).getByText(/Physics Department/)).toBeInTheDocument();
    expect(within(row).getByText("public")).toBeInTheDocument();
    expect(within(row).getByText("Active")).toBeInTheDocument();
    expect(within(row).getByText(/Optics Lab/)).toBeInTheDocument();
    expect(within(row).getByText("Read the title and status of Proposals in Grants Office.")).toBeInTheDocument();
    expect(within(row).getByText("2 people connected")).toBeInTheDocument();
    expect(within(row).getByText(/Last call: /)).toBeInTheDocument();
    const other = screen.getByTestId("outside-app-scc_p");
    expect(within(other).getByText("0 people connected")).toBeInTheDocument();
    expect(within(other).getByText("Never called")).toBeInTheDocument();
  });

  it("offers Approve and Reject on a pending app, and Disable on an active one", async () => {
    vi.spyOn(apiClient, "listClients").mockResolvedValue([active, pending]);
    render(<OutsideApps />);
    const wait = await screen.findByTestId("outside-app-scc_p");
    expect(within(wait).getByRole("button", { name: "Approve" })).toBeInTheDocument();
    expect(within(wait).getByRole("button", { name: "Reject" })).toBeInTheDocument();
    expect(within(wait).queryByRole("button", { name: "Disable" })).not.toBeInTheDocument();
    const live = screen.getByTestId("outside-app-scc_a");
    expect(within(live).getByRole("button", { name: "Disable" })).toBeInTheDocument();
    expect(within(live).queryByRole("button", { name: "Approve" })).not.toBeInTheDocument();
  });

  it("approves a pending app with a reason, and reads the list again", async () => {
    const list = vi.spyOn(apiClient, "listClients").mockResolvedValue([pending]);
    const decide = vi.spyOn(apiClient, "decideClient").mockResolvedValue({ ...pending, status: "Active" });
    render(<OutsideApps />);
    const row = await screen.findByTestId("outside-app-scc_p");
    fireEvent.click(within(row).getByRole("button", { name: "Approve" }));
    fireEvent.change(within(row).getByLabelText(/Reason/), { target: { value: "Reviewed with the registrar" } });
    fireEvent.click(within(row).getByRole("button", { name: "Approve the app" }));
    await waitFor(() => expect(decide).toHaveBeenCalledWith("scc_p", "approve", "Reviewed with the registrar"));
    await waitFor(() => expect(list).toHaveBeenCalledTimes(2));
  });

  it("rejects a pending app with a reason", async () => {
    vi.spyOn(apiClient, "listClients").mockResolvedValue([pending]);
    const decide = vi.spyOn(apiClient, "decideClient").mockResolvedValue({ ...pending, status: "Rejected" });
    render(<OutsideApps />);
    const row = await screen.findByTestId("outside-app-scc_p");
    fireEvent.click(within(row).getByRole("button", { name: "Reject" }));
    fireEvent.change(within(row).getByLabelText(/Reason/), { target: { value: "Asks for more than it needs" } });
    fireEvent.click(within(row).getByRole("button", { name: "Reject the app" }));
    await waitFor(() => expect(decide).toHaveBeenCalledWith("scc_p", "reject", "Asks for more than it needs"));
  });

  it("disables an active app with a reason, and says what that does", async () => {
    const list = vi.spyOn(apiClient, "listClients").mockResolvedValue([active]);
    const disable = vi.spyOn(apiClient, "disableClient").mockResolvedValue({ ...active, status: "Disabled" });
    render(<OutsideApps />);
    const row = await screen.findByTestId("outside-app-scc_a");
    fireEvent.click(within(row).getByRole("button", { name: "Disable" }));
    expect(within(row).getByText(/stops on its next call/i)).toBeInTheDocument();
    const submit = within(row).getByRole("button", { name: "Disable the app" });
    expect(submit).toBeDisabled();
    fireEvent.change(within(row).getByLabelText(/Reason/), { target: { value: "No longer used" } });
    fireEvent.click(submit);
    await waitFor(() => expect(disable).toHaveBeenCalledWith("scc_a", "No longer used"));
    await waitFor(() => expect(list).toHaveBeenCalledTimes(2));
  });

  it("shows thirty days of usage: calls, records read and records written", async () => {
    vi.spyOn(apiClient, "listClients").mockResolvedValue([active]);
    const read = vi.spyOn(apiClient, "clientUsage").mockResolvedValue(usage());
    render(<OutsideApps />);
    const row = await screen.findByTestId("outside-app-scc_a");
    fireEvent.click(within(row).getByRole("button", { name: "Usage" }));
    const panel = await screen.findByTestId("outside-app-usage-scc_a");
    expect(read).toHaveBeenCalledWith("scc_a");
    expect(within(panel).getByText("Last 30 days: 6 calls, 6 records read, 2 records written")).toBeInTheDocument();
    expect(within(panel).getAllByRole("row")).toHaveLength(31);
    expect(within(panel).getByRole("row", { name: /2026-10-30/ })).toHaveTextContent("6");
  });

  it("says so when there are no outside apps", async () => {
    vi.spyOn(apiClient, "listClients").mockResolvedValue([]);
    render(<OutsideApps />);
    expect(await screen.findByText("No outside apps")).toBeInTheDocument();
  });

  it("shows an error with a retry when the list cannot be read", async () => {
    const list = vi.spyOn(apiClient, "listClients").mockRejectedValueOnce(new Error("down")).mockResolvedValue([active]);
    render(<OutsideApps />);
    fireEvent.click(await screen.findByRole("button", { name: /retry/i }));
    expect(await screen.findByTestId("outside-app-scc_a")).toBeInTheDocument();
    expect(list).toHaveBeenCalledTimes(2);
  });

  it("is a section of the admin console", async () => {
    vi.spyOn(apiClient, "getCurrentUser").mockResolvedValue({ user: { eppn: "jordan.lee@state.edu", name: "Jordan", affiliation: "central_admin" }, is_impersonating: false, original_admin: null, is_platform_admin: true });
    vi.spyOn(apiClient, "listClients").mockResolvedValue([active]);
    render(<AdminConsole activePersona={{ eppn: "jordan.lee@state.edu", name: "Jordan", affiliation: "central_admin", department: "IT", roleTitle: "Admin", isAdmin: true }} adminTab="clients" />);
    expect(await screen.findByTestId("admin-outside-apps")).toBeInTheDocument();
    expect(await screen.findByTestId("outside-app-scc_a")).toBeInTheDocument();
  });

  it("has an Outside apps entry in the console navigation", async () => {
    vi.spyOn(apiClient, "listClients").mockResolvedValue([active]);
    window.history.pushState(null, "", "/admin");
    render(<AdminDesk />);
    fireEvent.click(screen.getByTitle("User Account & Persona Menu"));
    fireEvent.click(screen.getByTestId("switch-account-modal-btn"));
    fireEvent.click(screen.getByTestId("login-as-jordan.lee@state.edu-btn"));
    fireEvent.click(screen.getByTestId("admin-clients-tab-btn"));
    expect(await screen.findByTestId("admin-outside-apps")).toBeInTheDocument();
    expect(window.location.pathname).toBe("/admin/clients");
  });
});
