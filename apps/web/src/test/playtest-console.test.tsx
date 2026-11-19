import "@testing-library/jest-dom";
import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { render, screen, waitFor, fireEvent, within } from "@testing-library/react";
import AdminDesk from "../AdminDesk";
import { AdminConsoleView } from "../AdminConsoleView";
import { Connections } from "../admin/Connections";
import { apiClient, setAuthToken, type DataConnection } from "../api";
import type { Persona } from "../types";

const ME = { user: { eppn: "gus@state.edu", name: "Gus", affiliation: "central_admin", department: "Biology" } };
const MANIFEST = {
  slug: "alpha", title: "Alpha Register", description: "From the server", organization_code: "BIO", department: "Biology",
  custom_domain_verified: false, tables: [], views: [], ceds_mappings: {}, workspace_id: "ws-cs-research",
};

function server(replies: Record<string, unknown>) {
  global.fetch = vi.fn().mockImplementation((url) => {
    const path = String(url).replace(/^.*\/api\/v1/, "");
    const hit = Object.entries(replies).find(([k]) => path.split("?")[0] === k);
    return Promise.resolve({ ok: true, status: 200, statusText: "", json: async () => (hit ? hit[1] : []) });
  }) as unknown as typeof fetch;
}

function openDesk() {
  localStorage.clear();
  localStorage.setItem("scaffoldry_token", "tok");
  setAuthToken("tok");
  window.history.pushState({}, "", "/");
  return render(<AdminDesk initialPath="/" />);
}

const APPS = {
  "/auth/me": ME,
  "/me/apps": { apps: [{ slug: "alpha", title: "Alpha Register", description: "", department: "Biology", workspace_id: "ws-cs-research", scope: "workspace" }] },
  "/apps/alpha": MANIFEST,
};

const realWidth = window.innerWidth;
function setWidth(w: number) {
  Object.defineProperty(window, "innerWidth", { configurable: true, writable: true, value: w });
}

describe("the desk on a phone", () => {
  beforeEach(() => vi.clearAllMocks());
  afterEach(() => setWidth(realWidth));

  it("starts with the navigation rail collapsed below 768px", async () => {
    setWidth(390);
    server(APPS);
    const { container } = openDesk();
    await screen.findByText("Alpha Register");
    expect(container.querySelector("aside")).toHaveClass("w-16");
  });

  it("starts with the navigation rail open on a wide screen", async () => {
    setWidth(1280);
    server(APPS);
    const { container } = openDesk();
    await screen.findByText("Alpha Register");
    expect(container.querySelector("aside")).toHaveClass("w-64");
  });

  it("lets the main area shrink so a wide table scrolls inside its own box", async () => {
    setWidth(390);
    server(APPS);
    const { container } = openDesk();
    await screen.findByText("Alpha Register");
    expect(container.querySelector("main")).toHaveClass("min-w-0");
  });

  it("keeps the wordmark whole and hides the tagline until there is room", async () => {
    server(APPS);
    openDesk();
    await screen.findByText("Alpha Register");
    expect(screen.getByText("SCAFFOLDRY")).toHaveClass("whitespace-nowrap");
    expect(screen.getByText("Institutional Platform")).toHaveClass("hidden", "xl:inline");
  });

  it("shows no HERM label when the app has no HERM capability", async () => {
    server(APPS);
    openDesk();
    await screen.findByText("Alpha Register");
    expect(screen.queryByText(/HERM:/)).not.toBeInTheDocument();
  });
});

const admin: Persona = {
  eppn: "jordan.lee@state.edu", name: "Jordan Lee", affiliation: "central_admin",
  department: "Central IT", roleTitle: "Architect", isAdmin: true,
};

function consoleView(tab: "settings" | "ledger", extra: Record<string, unknown> = {}) {
  return render(
    <AdminConsoleView
      activePersona={admin} isImpersonating={false} realAdmin={null} handleStopImpersonation={vi.fn()}
      navigateTo={vi.fn()} adminTab={tab} apps={[]} ledger={[]}
      setNotificationToast={vi.fn()} personas={[admin]}
      handleStartImpersonation={vi.fn()} {...extra}
    />
  );
}

describe("the admin console pages", () => {
  beforeEach(() => {
    vi.restoreAllMocks();
    localStorage.clear();
  });

  it.each([
    ["settings", "Platform Settings"],
    ["ledger", "Decision Ledger"],
  ] as const)("the %s page has one kit page header and no old console banner", async (tab, title) => {
    server({ "/settings": {} });
    consoleView(tab);
    const h1s = screen.getAllByRole("heading", { level: 1 });
    expect(h1s).toHaveLength(1);
    expect(h1s[0]).toHaveTextContent(title);
    expect(screen.queryByText("Institutional Administrative Console")).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: /Exit to Workspace/ })).toBeInTheDocument();
  });

  it("shows the saved values of Platform Settings from the flat map the server returns", async () => {
    server({ "/settings": { "tokens.max_days": 45, "tokens.agent_enabled": true } });
    consoleView("settings");
    await waitFor(() => expect(screen.getByTestId("setting-input-tokens.max_days")).toHaveValue(45));
  });

  it("asks for a reason before saving a setting and sends it", async () => {
    const calls: { url: string; method?: string; body?: string }[] = [];
    global.fetch = vi.fn().mockImplementation((url, o: RequestInit = {}) => {
      calls.push({ url: String(url), method: o.method, body: o.body as string });
      return Promise.resolve({ ok: true, status: 200, statusText: "", json: async () => ({ "tokens.max_days": 45 }) });
    }) as unknown as typeof fetch;
    consoleView("settings");
    await waitFor(() => expect(screen.getByTestId("setting-input-tokens.max_days")).toHaveValue(45));
    fireEvent.change(screen.getByTestId("setting-input-tokens.max_days"), { target: { value: "60" } });
    fireEvent.click(screen.getByTestId("setting-save-tokens.max_days"));
    expect(calls.some((c) => c.method === "PUT")).toBe(false);
    fireEvent.change(screen.getByLabelText(/Reason for decision/), { target: { value: "Policy review" } });
    fireEvent.click(screen.getByRole("button", { name: /^Save tokens\.max_days/ }));
    await waitFor(() => expect(calls.some((c) => c.method === "PUT")).toBe(true));
    const put = calls.find((c) => c.method === "PUT")!;
    expect(put.url).toContain("/settings/tokens.max_days?reason=Policy%20review");
    expect(put.body).toBe("60");
  });

  it("shows a failed save as an error, not a success", async () => {
    server({ "/settings": { "tokens.max_days": 45 } });
    vi.spyOn(apiClient, "updateSetting").mockRejectedValue(new Error("nope"));
    const setNotificationToast = vi.fn();
    const setErrorToast = vi.fn();
    consoleView("settings", { setNotificationToast, setErrorToast });
    await waitFor(() => expect(screen.getByTestId("setting-input-tokens.max_days")).toHaveValue(45));
    fireEvent.click(screen.getByTestId("setting-save-tokens.max_days"));
    fireEvent.change(screen.getByLabelText(/Reason for decision/), { target: { value: "why" } });
    fireEvent.click(screen.getByRole("button", { name: /^Save tokens\.max_days/ }));
    await waitFor(() => expect(setErrorToast).toHaveBeenCalledWith(expect.stringContaining('Failed to update "tokens.max_days"')));
    expect(setNotificationToast).not.toHaveBeenCalled();
  });
});

const healthy: DataConnection = {
  id: "c1", name: "Programs", kind: "file_drop", dataset_app: "programs", dataset_table: "programs",
  state: "Healthy", enabled: true, review_note: null, last_result: "Added 12", last_success: "2026-11-12T02:30:00Z",
  next_run: "2026-11-13T02:30:00Z",
  last_run: { id: "r1", state: "done", started_at: "2026-11-12T02:30:00Z", finished_at: "2026-11-12T02:30:05Z", added: 12, changed: 0, removed: 0, unchanged: 0, error: null },
};

describe("the Connections table", () => {
  beforeEach(() => vi.restoreAllMocks());

  it("keeps the actions in the same row as the kind and state, as small buttons", async () => {
    vi.spyOn(apiClient, "listDataConnections").mockResolvedValue([healthy]);
    render(<Connections />);
    const row = (await screen.findByText("Programs")).closest("tr") as HTMLElement;
    const run = within(row).getByRole("button", { name: "Run now Programs" });
    expect(run).toHaveClass("text-xs");
    expect(within(row).getByText("Healthy")).toBeInTheDocument();
  });
});
