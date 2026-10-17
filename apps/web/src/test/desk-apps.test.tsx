import React from "react";
import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, waitFor, fireEvent } from "@testing-library/react";
import "@testing-library/jest-dom";
import AdminDesk from "../AdminDesk";
import { setAuthToken } from "../api";

const ME = { user: { eppn: "gus@state.edu", name: "Gus", affiliation: "central_admin", department: "Biology" } };
const MANIFEST = {
  slug: "alpha", title: "Alpha Register", description: "From the server", organization_code: "BIO", department: "Biology",
  custom_domain_verified: false, tables: [], views: [], ceds_mappings: {}, workspace_id: "ws-cs-research",
};

function server(replies: Record<string, unknown>) {
  const urls: string[] = [];
  global.fetch = vi.fn().mockImplementation((url) => {
    const path = String(url).replace(/^.*\/api\/v1/, "");
    urls.push(path);
    const hit = Object.entries(replies).find(([k]) => path.split("?")[0] === k);
    const body = hit ? hit[1] : [];
    return Promise.resolve({ ok: true, status: 200, statusText: "", json: async () => body });
  }) as unknown as typeof fetch;
  return urls;
}

function openDesk() {
  localStorage.clear();
  localStorage.setItem("scaffoldry_token", "tok");
  setAuthToken("tok");
  window.history.pushState({}, "", "/");
  return render(<AdminDesk initialPath="/" />);
}

describe("the desk's apps come from the server", () => {
  beforeEach(() => vi.clearAllMocks());

  it("asks about the server's apps and not the built-in sample apps", async () => {
    const urls = server({
      "/auth/me": ME,
      "/me/apps": { apps: [{ slug: "alpha", title: "Alpha Register", description: "", department: "Biology", workspace_id: "ws-cs-research", scope: "workspace" }] },
      "/apps/alpha": MANIFEST,
    });
    openDesk();
    await waitFor(() => expect(urls.some((u) => u.startsWith("/apps/alpha/processes"))).toBe(true));
    expect(urls.filter((u) => /^\/apps\/(?!alpha)[^/]+\/processes/.test(u))).toEqual([]);
    expect(screen.queryByTestId("offline-sample-banner")).not.toBeInTheDocument();
  });

  it("says plainly when it is showing built-in sample apps because the server's list did not arrive", async () => {
    server({ "/auth/me": ME });
    openDesk();
    expect(await screen.findByTestId("offline-sample-banner")).toHaveTextContent(/built-in sample apps/i);
  });
});

type Call = { method: string; path: string; body?: any };
function serverWith(handler: (c: Call) => { status?: number; body?: unknown } | undefined) {
  const calls: Call[] = [];
  global.fetch = vi.fn().mockImplementation((url, options: RequestInit = {}) => {
    const path = String(url).replace(/^.*\/api\/v1/, "");
    const call: Call = { method: (options.method || "GET").toUpperCase(), path, body: options.body ? JSON.parse(String(options.body)) : undefined };
    calls.push(call);
    const known: Record<string, unknown> = { "/auth/me": ME, "/me/apps": { apps: [] } };
    const reply = handler(call) ?? { body: known[path.split("?")[0]] ?? [] };
    const status = reply.status ?? 200;
    return Promise.resolve({ ok: status < 400, status, statusText: "", json: async () => reply.body });
  }) as unknown as typeof fetch;
  return calls;
}

describe("New App creates the app on the server, and says only what happened", () => {
  beforeEach(() => vi.clearAllMocks());

  const clickNewApp = async () => {
    openDesk();
    await waitFor(() => expect(screen.queryByTestId("offline-sample-banner")).not.toBeInTheDocument());
    fireEvent.click(await screen.findByRole("button", { name: /^new app$/i }));
  };

  it("asks the server, with no made-up domain, and reports success only after it answers 201", async () => {
    const calls = serverWith((c) =>
      c.method === "POST" && c.path === "/workspaces/ws-cs-research/apps"
        ? { status: 201, body: { ...MANIFEST, slug: c.body.slug, title: c.body.title } }
        : undefined
    );
    await clickNewApp();
    expect(await screen.findByText(/Created new app/)).toBeInTheDocument();
    const post = calls.find((c) => c.method === "POST")!;
    expect(post.body.workspace_id).toBe("ws-cs-research");
    expect(post.body.slug).toMatch(/^app-/);
    expect(post.body.custom_domain).toBeUndefined();
  });

  it("says the app is waiting for approval when the server answers 202", async () => {
    serverWith((c) => (c.method === "POST" ? { status: 202, body: { proposal_id: "p-1", status: "Pending" } } : undefined));
    await clickNewApp();
    expect(await screen.findByText(/submitted for approval/i)).toBeInTheDocument();
    expect(screen.queryByText(/Created new app/)).not.toBeInTheDocument();
  });

  it("says the app was not created when the server refuses", async () => {
    serverWith((c) => (c.method === "POST" ? { status: 403, body: { error: "Forbidden: Cedar policy denied app creation in workspace" } } : undefined));
    await clickNewApp();
    expect(await screen.findByText(/could not create the app/i)).toBeInTheDocument();
    expect(screen.queryByText(/Created new app/)).not.toBeInTheDocument();
  });
});
