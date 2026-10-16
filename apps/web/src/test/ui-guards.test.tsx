import React from "react";
import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen } from "@testing-library/react";
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
});
