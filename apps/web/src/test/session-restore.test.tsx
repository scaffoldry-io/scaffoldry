import React from "react";
import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen } from "@testing-library/react";
import "@testing-library/jest-dom";
import AdminDesk from "../AdminDesk";
import { setAuthToken } from "../api";

// A reload restores the stored token. The desk must then show who the token belongs to, not a built-in persona.
function stub(me: { status: number; body: unknown }) {
  global.fetch = vi.fn().mockImplementation((url) => {
    const isMe = String(url).endsWith("/auth/me");
    const status = isMe ? me.status : 200;
    const body = isMe ? me.body : [];
    return Promise.resolve({ ok: status < 400, status, statusText: "", json: async () => body });
  }) as unknown as typeof fetch;
}

function reloadWithStoredToken() {
  localStorage.clear();
  localStorage.setItem("scaffoldry_token", "stored-token");
  setAuthToken("stored-token");
  window.history.pushState({}, "", "/");
  render(<AdminDesk initialPath="/" />);
}

describe("restoring a session after a reload", () => {
  beforeEach(() => vi.clearAllMocks());

  it("shows the person the stored token belongs to", async () => {
    stub({ status: 200, body: { user: { eppn: "records.admin@state.edu", name: "Records Admin", affiliation: "central_admin", department: "Central IT" } } });
    reloadWithStoredToken();
    expect(await screen.findByText(/central_admin · Central IT/i)).toBeInTheDocument();
    expect(screen.queryByText(/faculty · Computer Science/i)).not.toBeInTheDocument();
  });

  it("keeps the session when the server cannot be reached", async () => {
    global.fetch = vi.fn().mockRejectedValue(new TypeError("Failed to fetch")) as unknown as typeof fetch;
    reloadWithStoredToken();
    await new Promise((r) => setTimeout(r, 300));
    expect(screen.queryByText(/Sign in to Scaffoldry/i)).not.toBeInTheDocument();
    expect(localStorage.getItem("scaffoldry_token")).toBe("stored-token");
  });

  it("asks for a sign-in when the stored token is no longer valid", async () => {
    stub({ status: 401, body: { error: "Unauthorized" } });
    reloadWithStoredToken();
    expect(await screen.findByText(/Sign in to Scaffoldry/i)).toBeInTheDocument();
  });
});
