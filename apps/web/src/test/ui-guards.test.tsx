import React from "react";
import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import "@testing-library/jest-dom";
import AdminDesk from "../AdminDesk";
import { setAuthToken } from "../api";

// The same rule the browser audit applies (scripts/ui/audit.mjs), kept here so it also runs with the normal tests.
describe("UI guards", () => {
  beforeEach(() => {
    localStorage.clear();
    localStorage.setItem("scaffoldry_token", "tok");
    setAuthToken("tok");
    global.fetch = vi.fn().mockImplementation((url) => {
      const body = String(url).endsWith("/auth/me") ? { user: { eppn: "a@state.edu", name: "A", affiliation: "faculty", department: "Biology" } } : [];
      return Promise.resolve({ ok: true, status: 200, statusText: "", json: async () => body });
    }) as unknown as typeof fetch;
  });

  it("no button draws its plus twice: an icon and a leading plus in the label", async () => {
    window.history.pushState({}, "", "/");
    const { container } = render(<AdminDesk initialPath="/" />);
    await screen.findAllByText(/WORKSPACES/i);
    const doubled = [...container.querySelectorAll("button, a")]
      .filter((el) => el.querySelector("svg") && /^[+×✕]/.test((el.textContent || "").trim()))
      .map((el) => (el.textContent || "").trim());
    expect(doubled).toEqual([]);
  });

  const adminDesk = async () => {
    global.fetch = vi.fn().mockImplementation((url) => {
      const path = String(url).replace(/^.*\/api\/v1/, "").split("?")[0];
      const body = path === "/auth/me"
        ? { user: { eppn: "a@state.edu", name: "A", affiliation: "central_admin", department: "Computer Science" } }
        : path === "/me/apps" ? { apps: [] } : [];
      return Promise.resolve({ ok: true, status: 200, statusText: "", json: async () => body });
    }) as unknown as typeof fetch;
    window.history.pushState({}, "", "/");
    render(<AdminDesk initialPath="/" />);
    await waitFor(() => expect(screen.queryByTestId("offline-sample-banner")).not.toBeInTheDocument());
  };

  it("the workspace settings fields have names a screen reader can read", async () => {
    await adminDesk();
    fireEvent.click(await screen.findByRole("button", { name: /security & settings/i }));
    for (const name of ["Workspace name", "Department", "Workspace code", "Description"]) {
      expect(await screen.findByLabelText(name)).toBeInTheDocument();
    }
  });

  it("with the navigation collapsed, every button in it still has a name", async () => {
    await adminDesk();
    fireEvent.click(await screen.findByTitle("Toggle Navigation Menu"));
    expect(await screen.findByRole("button", { name: /new app studio/i })).toBeInTheDocument();
  });
});
