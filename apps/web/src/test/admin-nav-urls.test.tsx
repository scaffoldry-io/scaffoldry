import React from "react";
import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import "@testing-library/jest-dom";
import AdminDesk from "../AdminDesk";
import { setAuthToken } from "../api";

const ME = { user: { eppn: "gus@state.edu", name: "Gus", affiliation: "central_admin", department: "Biology" } };

function serve() {
  global.fetch = vi.fn().mockImplementation((url) => {
    const path = String(url).replace(/^.*\/api\/v1/, "").split("?")[0];
    const body = path === "/auth/me" ? ME : path === "/me/apps" ? { apps: [] } : [];
    return Promise.resolve({ ok: true, status: 200, statusText: "", json: async () => body });
  }) as unknown as typeof fetch;
}

function openAdmin(path = "/admin") {
  localStorage.clear();
  localStorage.setItem("scaffoldry_token", "tok");
  setAuthToken("tok");
  window.history.pushState({}, "", path);
  return render(<AdminDesk initialPath={path} />);
}

describe("every admin nav button keeps the address in step", () => {
  beforeEach(() => serve());

  const byTestId: [string, string][] = [
    ["admin-settings-tab-btn", "/admin/settings"],
    ["admin-positions-tab-btn", "/admin/positions"],
    ["admin-people-tab-btn", "/admin/people"],
    ["admin-org-tab-btn", "/admin/org"],
    ["admin-impersonation-tab-btn", "/admin/impersonation"],
  ];
  for (const [id, path] of byTestId) {
    it(`${id} goes to ${path}`, async () => {
      openAdmin();
      fireEvent.click(await screen.findByTestId(id));
      expect(window.location.pathname).toBe(path);
    });
  }

  it("Policy goes to /admin/policy, Audit to /admin/ledger and Security to /admin/security", async () => {
    openAdmin();
    fireEvent.click(await screen.findByText(/Policy & OSCAL Lattice/));
    expect(window.location.pathname).toBe("/admin/policy");
    fireEvent.click(screen.getByText("Audit"));
    expect(window.location.pathname).toBe("/admin/ledger");
    fireEvent.click(screen.getByTestId("admin-security-tab-btn"));
    expect(window.location.pathname).toBe("/admin/security");
  });

  it("opening /admin/settings selects the settings tab", async () => {
    openAdmin("/admin/settings");
    const button = await screen.findByTestId("admin-settings-tab-btn");
    await waitFor(() => expect(button.className).toMatch(/bg-amber-50/));
  });
});
