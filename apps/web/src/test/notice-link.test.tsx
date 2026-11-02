import React from "react";
import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import "@testing-library/jest-dom";
import AdminDesk from "../AdminDesk";
import { setAuthToken } from "../api";

// Notifications phase 2, test 7: a mailed link is /n/{id}. Without a session the desk asks for a sign-in first, and
// then goes to the notice's target. The link itself carries no token.
function stub(target: string) {
  const calls: string[] = [];
  global.fetch = vi.fn().mockImplementation((url) => {
    const u = String(url);
    calls.push(u);
    let body: unknown = [];
    if (u.endsWith("/auth/me")) body = { user: { eppn: "chair@state.edu", name: "Chair", affiliation: "faculty", department: "Physics" } };
    if (u.endsWith("/me/notifications/n1/open")) body = { path: target };
    if (u.endsWith("/me/notifications/count")) body = { unread: 0 };
    return Promise.resolve({ ok: true, status: 200, statusText: "", json: async () => body });
  }) as unknown as typeof fetch;
  return calls;
}

describe("following a link from a mail", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    localStorage.clear();
    setAuthToken(null);
  });

  it("signs in first and then goes to the notice's target", async () => {
    const calls = stub("/proposals");
    window.history.pushState({}, "", "/n/n1");
    render(<AdminDesk initialPath="/n/n1" />);
    expect(await screen.findByText(/Sign in to Scaffoldry/i)).toBeInTheDocument();
    expect(calls.some((c) => c.includes("/open"))).toBe(false);
    fireEvent.change(screen.getByLabelText("Paste your token"), { target: { value: "a-token" } });
    fireEvent.submit(screen.getByLabelText("Paste your token").closest("form")!);
    await waitFor(() => expect(window.location.pathname).toBe("/proposals"));
    expect(calls.some((c) => c.endsWith("/me/notifications/n1/open"))).toBe(true);
  });

  it("goes straight to the target with a session", async () => {
    stub("/app/admissions");
    localStorage.setItem("scaffoldry_token", "stored-token");
    setAuthToken("stored-token");
    window.history.pushState({}, "", "/n/n1");
    render(<AdminDesk initialPath="/n/n1" />);
    await waitFor(() => expect(window.location.pathname).toBe("/app/admissions"));
  });
});
