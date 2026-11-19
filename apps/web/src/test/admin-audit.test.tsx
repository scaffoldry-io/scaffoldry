import "@testing-library/jest-dom";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { Audit } from "../admin/Audit";
import { Security } from "../admin/Security";
import { Overview } from "../admin/Overview";
import { PageFrame } from "../PageFrame";
import { apiClient, setAuthToken, type AdminLedgerRow, type AdminOverviewData, type AdminTokenRow } from "../api";

const entry = (sequence: number, over: Partial<AdminLedgerRow> = {}): AdminLedgerRow => ({
  sequence,
  timestamp_iso: "2026-11-18T10:00:00+00:00",
  previous_hash: `prev${sequence}`.padEnd(64, "0"),
  principal: "jordan.lee@state.edu",
  organization_code: "DIV-ADMIN-CONSOLE",
  app_slug: null,
  decision_type: "PolicyRevision",
  oscal_control_id: "AC-02",
  control_title: "Account Management",
  rationale: `Reason ${sequence}`,
  payload: { key: "tokens.max_days", value_sha256: "ab".repeat(32) },
  payload_hash: `pay${sequence}`.padEnd(64, "0"),
  entry_hash: `hash${sequence}`.padEnd(64, "0"),
  ...over,
});

const token = (over: Partial<AdminTokenRow> = {}): AdminTokenRow => ({
  id: "tok-1",
  kind: "agent",
  owner: "ada@state.edu",
  label: "Audit bot",
  state: "active",
  created_at: "2026-11-01T09:00:00Z",
  expires_at: "2026-12-01T09:00:00Z",
  last_used_at: null,
  revoked_at: null,
  original_admin: null,
  ...over,
});

const tools = [
  { name: "create_record", description: "Create a record in an app table.", read_only: false, scope: "app", disabled: false },
  { name: "list_apps", description: "List the apps in a workspace.", read_only: true, scope: "workspace", disabled: false },
  { name: "begin_task", description: "Add one sentence.", read_only: false, scope: "signed_in", disabled: true },
];

afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
});

describe("Audit", () => {
  // Test 10: the payload drawer shows the stored payload.
  it("opens a row to show the stored payload and the hashes, with the control's title beside its id", async () => {
    vi.spyOn(apiClient, "adminLedger").mockResolvedValue({ rows: [entry(7), entry(6)], next_cursor: null });
    render(<Audit />);
    const row = (await screen.findByText("Reason 7")).closest("tr") as HTMLElement;
    expect(within(row).getByText("AC-02 · Account Management")).toBeInTheDocument();
    fireEvent.click(row);
    const drawer = await screen.findByTestId("admin-ledger-entry");
    expect(within(drawer).getByLabelText("Stored payload").textContent).toContain('"value_sha256"');
    expect(within(drawer).getByLabelText("Stored payload").textContent).toContain("tokens.max_days");
    expect(within(drawer).getByText(entry(7).entry_hash)).toBeInTheDocument();
    expect(within(drawer).getByText(entry(7).previous_hash)).toBeInTheDocument();
  });

  it("verifies the whole chain on request and names the first bad entry", async () => {
    vi.spyOn(apiClient, "adminLedger").mockResolvedValue({ rows: [entry(7)], next_cursor: null });
    const verify = vi.spyOn(apiClient, "verifyAdminLedger");
    verify.mockResolvedValueOnce({ valid: true, entries: 7, head_hash: "f".repeat(64), first_bad_sequence: null, detail: null });
    render(<Audit />);
    await screen.findByText("Reason 7");
    expect(screen.queryByText(/The chain is intact/)).not.toBeInTheDocument();
    expect(screen.getByText(/reads the whole ledger/)).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Verify chain" }));
    expect(await screen.findByText(/The chain is intact. 7 entries/)).toBeInTheDocument();

    verify.mockResolvedValueOnce({ valid: false, entries: 3, head_hash: "f".repeat(64), first_bad_sequence: 3, detail: "Cryptographic tamper detected at sequence 3" });
    fireEvent.click(screen.getByRole("button", { name: "Verify chain" }));
    expect(await screen.findByText(/The chain is broken at entry 3/)).toBeInTheDocument();
  });

  it("sends the filters it was given, and the person from the search box", async () => {
    const list = vi.spyOn(apiClient, "adminLedger").mockResolvedValue({ rows: [entry(7)], next_cursor: null });
    render(<Audit />);
    await screen.findByText("Reason 7");
    fireEvent.change(screen.getByLabelText("Decision type"), { target: { value: "TokenRevoked" } });
    fireEvent.change(screen.getByLabelText("Control"), { target: { value: "ia-5" } });
    fireEvent.click(screen.getByRole("button", { name: "Apply filters" }));
    await waitFor(() => expect(list).toHaveBeenLastCalledWith(expect.objectContaining({ decision_type: "TokenRevoked", control: "ia-5", limit: "50" })));
    fireEvent.change(screen.getByLabelText("Search records"), { target: { value: "ada@" } });
    await waitFor(() => expect(list).toHaveBeenLastCalledWith(expect.objectContaining({ principal: "ada@", decision_type: "TokenRevoked" })));
  });

  it("exports the server's own file with the filters in force", async () => {
    vi.spyOn(apiClient, "adminLedger").mockResolvedValue({ rows: [entry(7)], next_cursor: null });
    const exp = vi.spyOn(apiClient, "exportAdminLedger").mockResolvedValue("sequence,timestamp\n7,now\n");
    URL.createObjectURL = vi.fn(() => "blob:x");
    URL.revokeObjectURL = vi.fn();
    render(<Audit />);
    await screen.findByText("Reason 7");
    fireEvent.click(screen.getByRole("button", { name: "Export CSV" }));
    await waitFor(() => expect(exp).toHaveBeenCalledWith("csv", expect.any(Object)));
    expect(URL.createObjectURL).toHaveBeenCalled();
  });

  it("says in words when an export is refused for being too large", async () => {
    vi.spyOn(apiClient, "adminLedger").mockResolvedValue({ rows: [entry(7)], next_cursor: null });
    vi.spyOn(apiClient, "exportAdminLedger").mockRejectedValue(new Error("More than 50000 entries match. Narrow the filter."));
    render(<Audit />);
    await screen.findByText("Reason 7");
    fireEvent.click(screen.getByRole("button", { name: "Export JSON" }));
    expect(await screen.findByText(/Narrow the filter/)).toBeInTheDocument();
  });
});

describe("Security", () => {
  beforeEach(() => {
    vi.spyOn(apiClient, "adminTokens").mockResolvedValue({ rows: [token(), token({ id: "imp-1", kind: "impersonation", label: "", owner: "student@state.edu", original_admin: "jordan.lee@state.edu" })], next_cursor: null });
  });

  it("revokes a token only with a reason, and never shows a hash", async () => {
    const revoke = vi.spyOn(apiClient, "revokeAdminToken").mockResolvedValue();
    render(<Security />);
    await screen.findByText("Audit bot");
    expect(document.body.textContent).not.toMatch(/token_hash/);
    fireEvent.click(screen.getByRole("button", { name: "Revoke Audit bot" }));
    const drawer = await screen.findByTestId("admin-token-revoke");
    const go = within(drawer).getByRole("button", { name: /Revoke Audit bot/ });
    fireEvent.click(go);
    expect(revoke).not.toHaveBeenCalled();
    fireEvent.change(within(drawer).getByRole("textbox"), { target: { value: "The contractor left" } });
    fireEvent.click(go);
    await waitFor(() => expect(revoke).toHaveBeenCalledWith("tok-1", "The contractor left"));
  });

  it("offers End session for an impersonation, and filters to those sessions", async () => {
    const list = vi.spyOn(apiClient, "adminTokens");
    render(<Security />);
    expect(await screen.findByRole("button", { name: "End session student@state.edu" })).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Impersonation sessions" }));
    await waitFor(() => expect(list).toHaveBeenLastCalledWith(expect.objectContaining({ kind: "impersonation" })));
  });

  it("mints a SCIM token and shows it once", async () => {
    const mint = vi.spyOn(apiClient, "mintScimToken").mockResolvedValue({ id: "t9", token: "scim-secret-value", label: "Registrar sync", expires_at: "2027-01-01T00:00:00Z" });
    render(<Security />);
    await screen.findByText("Audit bot");
    fireEvent.click(screen.getByRole("button", { name: "Mint SCIM token" }));
    fireEvent.change(screen.getByLabelText(/Label/), { target: { value: "Registrar sync" } });
    fireEvent.click(screen.getByRole("button", { name: "Create token" }));
    await waitFor(() => expect(mint).toHaveBeenCalledWith("Registrar sync", 90));
    expect(await screen.findByLabelText("New SCIM token")).toHaveTextContent("scim-secret-value");
    fireEvent.click(screen.getByRole("button", { name: "I have copied it" }));
    expect(screen.queryByText("scim-secret-value")).not.toBeInTheDocument();
  });

  // Test 10: toggling a tool sends one PUT of the setting with the new list.
  it("turning a tool off sends one PUT with the new list and the reason", async () => {
    vi.spyOn(apiClient, "adminTools").mockResolvedValue({ tools });
    vi.spyOn(apiClient, "adminPages").mockResolvedValue({ pages: [] });
    const puts: { url: string; body: string }[] = [];
    global.fetch = vi.fn().mockImplementation((url: string, init: RequestInit = {}) => {
      if (init.method === "PUT") puts.push({ url: String(url), body: String(init.body) });
      return Promise.resolve({ ok: true, status: 200, statusText: "", json: async () => ({}) });
    }) as unknown as typeof fetch;
    setAuthToken("tok");
    render(<Security />);
    fireEvent.click(await screen.findByRole("tab", { name: "Agent access" }));
    await screen.findByText("create_record");
    expect(screen.getByText("Off")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Turn off create_record" }));
    const drawer = await screen.findByTestId("admin-agent-change");
    fireEvent.change(within(drawer).getByRole("textbox"), { target: { value: "Pause writes" } });
    fireEvent.click(within(drawer).getByRole("button", { name: /Turn off create_record/ }));
    await waitFor(() => expect(puts).toHaveLength(1));
    expect(puts[0].url).toContain("/settings/mcp.disabled_tools");
    expect(puts[0].url).toContain("reason=Pause%20writes");
    expect(JSON.parse(puts[0].body)).toEqual(["begin_task", "create_record"]);
  });

  it("turning a page off writes pages.disabled with app/page keys", async () => {
    vi.spyOn(apiClient, "adminTools").mockResolvedValue({ tools });
    vi.spyOn(apiClient, "adminPages").mockResolvedValue({
      pages: [{ app_slug: "scholarships", app_title: "Scholarships", page_id: "board", title: "Board", grants: "In grants: reads title, writes nothing.", approved_by: null, disabled: false }],
    });
    const update = vi.spyOn(apiClient, "updateSetting").mockResolvedValue({ key: "pages.disabled", value: [] });
    render(<Security />);
    fireEvent.click(await screen.findByRole("tab", { name: "Agent access" }));
    expect(await screen.findByText("In grants: reads title, writes nothing.")).toBeInTheDocument();
    expect(screen.getByText("Not recorded")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Turn off scholarships/board" }));
    const drawer = await screen.findByTestId("admin-agent-change");
    fireEvent.change(within(drawer).getByRole("textbox"), { target: { value: "Under review" } });
    fireEvent.click(within(drawer).getByRole("button", { name: /Turn off scholarships\/board/ }));
    await waitFor(() => expect(update).toHaveBeenCalledWith("pages.disabled", ["scholarships/board"], "Under review"));
  });
});

describe("the page frame and the overview", () => {
  it("shows the notice for a page an administrator turned off", async () => {
    global.fetch = vi.fn().mockResolvedValue({
      ok: false,
      status: 403,
      statusText: "Forbidden",
      json: async () => ({ error: "This page was disabled by an administrator.", code: "forbidden" }),
    }) as unknown as typeof fetch;
    setAuthToken("tok");
    render(<PageFrame appSlug="scholarships" pageId="board" />);
    expect(await screen.findByRole("alert")).toHaveTextContent("This page was disabled by an administrator.");
    expect(screen.queryByTestId("page-frame")).not.toBeInTheDocument();
  });

  it("lists the recent changes with the title of each control and the active tokens", async () => {
    const data = {
      server: { version: "1", applied_migrations: [], database_worker_count: 1 },
      people: { active: 1, on_hold: 0, inactive: 0, platform_admins: 1, active_tokens: { agent: 2, scim: 1 } },
      organization: { unit_count: 1 },
      workspaces: { workspace_count: 1, app_count: 1 },
      processes: { waiting_instance_count: 0, stale: 0 },
      ledger: {
        entry_count: 9,
        head_hash: "f".repeat(64),
        recent: [{ sequence: 9, timestamp_iso: "2026-11-18T10:00:00Z", principal: "jordan.lee@state.edu", decision_type: "PolicyRevision", oscal_control_id: "AC-02", control_title: "Account Management", rationale: "Checked accounts" }],
      },
    } as AdminOverviewData;
    vi.spyOn(apiClient, "getAdminOverview").mockResolvedValue(data);
    render(<Overview />);
    const recent = await screen.findByTestId("admin-overview-recent-changes");
    expect(within(recent).getByText("AC-02 · Account Management")).toBeInTheDocument();
    expect(within(recent).getByText(/Checked accounts/)).toBeInTheDocument();
    const tokens = screen.getByTestId("admin-overview-tokens-card");
    expect(within(tokens).getByText("agent")).toBeInTheDocument();
    expect(within(tokens).getByText("2")).toBeInTheDocument();
  });
});
