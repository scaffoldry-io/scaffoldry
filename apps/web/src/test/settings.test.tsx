import React, { useState } from "react";
import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import "@testing-library/jest-dom";
import { UserSettingsModal } from "../UserSettingsModal";
import { AdminConsoleView } from "../AdminConsoleView";
import AdminDesk from "../AdminDesk";
import { Persona } from "../types";

const mockCentralAdmin: Persona = {
  eppn: "jordan.lee@state.edu",
  name: "Jordan Lee",
  affiliation: "central_admin",
  department: "Central Enterprise IT",
  roleTitle: "Enterprise Identity & Security Architect",
  isAdmin: true,
};

const mockFaculty: Persona = {
  eppn: "sarah.connor@state.edu",
  name: "Dr. Sarah Connor",
  affiliation: "faculty",
  department: "Computer Science",
  roleTitle: "Department Chair & Professor",
  isAdmin: false,
};

describe("Phase 9: Settings Panes", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    localStorage.clear();
    localStorage.setItem("scaffoldry_token", "test-token");
  });

  // Test 1: Creating a token shows the secret once. After closing and reopening the pane, the secret is absent and the row is listed.
  it("creating a token shows the secret once, and after closing and reopening the pane the secret is absent and the row is listed", async () => {
    let tokensList: any[] = [];
    const mintedSecret = "scf_agent_test_secret_abc123";

    global.fetch = vi.fn().mockImplementation((url, options) => {
      const urlStr = String(url);
      if (urlStr.endsWith("/settings") && (!options || options.method === "GET" || !options.method)) {
        return Promise.resolve({
          ok: true,
          json: async () => ({
            settings: {
              "tokens.agent_enabled": true,
              "tokens.max_days": 90,
            },
          }),
        });
      }
      if (urlStr.endsWith("/auth/tokens") && (!options || options.method === "GET" || !options.method)) {
        return Promise.resolve({
          ok: true,
          json: async () => ({ tokens: tokensList }),
        });
      }
      if (urlStr.endsWith("/auth/tokens") && options?.method === "POST") {
        const body = JSON.parse(options.body as string);
        const newToken = {
          id: "tok-new-001",
          label: body.label,
          kind: body.kind || "agent",
          created_at: "2026-10-09T00:00:00Z",
          expires_at: "2026-11-08T00:00:00Z",
          last_used_at: null,
        };
        tokensList = [newToken];
        return Promise.resolve({
          ok: true,
          json: async () => ({
            token: mintedSecret,
            id: newToken.id,
            expires_at: newToken.expires_at,
            label: newToken.label,
            kind: newToken.kind,
          }),
        });
      }
      return Promise.resolve({
        ok: true,
        json: async () => ({}),
      });
    });

    // Component wrapper to control isOpen state
    const Wrapper = () => {
      const [isOpen, setIsOpen] = useState(true);
      return (
        <div>
          <button data-testid="toggle-modal-btn" onClick={() => setIsOpen((prev) => !prev)}>
            Toggle
          </button>
          <UserSettingsModal
            isOpen={isOpen}
            onClose={() => setIsOpen(false)}
            eppn="sarah.connor@state.edu"
          />
        </div>
      );
    };

    render(<Wrapper />);

    // Pane opens and shows agent token form
    await waitFor(() => {
      expect(screen.getByTestId("agent-token-form")).toBeInTheDocument();
    });

    expect(screen.queryByTestId("agent-token-secret")).not.toBeInTheDocument();

    // Create token
    const labelInput = screen.getByPlaceholderText(/Cursor Assistant/i);
    fireEvent.change(labelInput, { target: { value: "My Test Agent" } });
    fireEvent.submit(screen.getByTestId("agent-token-form"));

    // Secret is displayed once
    await waitFor(() => {
      const secretInput = screen.getByTestId("agent-token-secret") as HTMLInputElement;
      expect(secretInput).toBeInTheDocument();
      expect(secretInput.value).toBe(mintedSecret);
    });

    expect(screen.getByText(/Copy this token now\. It is not shown again\./i)).toBeInTheDocument();

    // Close and reopen the pane
    fireEvent.click(screen.getByTestId("toggle-modal-btn")); // close
    await waitFor(() => {
      expect(screen.queryByTestId("user-settings")).not.toBeInTheDocument();
    });

    fireEvent.click(screen.getByTestId("toggle-modal-btn")); // reopen
    await waitFor(() => {
      expect(screen.getByTestId("user-settings")).toBeInTheDocument();
    });

    // Secret is absent
    expect(screen.queryByTestId("agent-token-secret")).not.toBeInTheDocument();

    // The row is listed in the token list
    await waitFor(() => {
      expect(screen.getByTestId("agent-token-list")).toHaveTextContent("My Test Agent");
    });
  });

  // Test 2: Revoke calls DELETE /api/v1/auth/tokens/...
  it("revoke calls DELETE /api/v1/auth/tokens/{id}", async () => {
    const mockFetch = vi.fn().mockImplementation((url, options) => {
      const urlStr = String(url);
      if (urlStr.endsWith("/settings")) {
        return Promise.resolve({
          ok: true,
          json: async () => ({
            settings: { "tokens.agent_enabled": true, "tokens.max_days": 90 },
          }),
        });
      }
      if (urlStr.endsWith("/auth/tokens") && (!options || options.method === "GET" || !options.method)) {
        return Promise.resolve({
          ok: true,
          json: async () => ({
            tokens: [
              {
                id: "tok-to-revoke-999",
                label: "Deprecated Bot",
                kind: "agent",
                created_at: "2026-10-09T00:00:00Z",
                expires_at: "2026-11-08T00:00:00Z",
                last_used_at: null,
              },
            ],
          }),
        });
      }
      if (urlStr.includes("/auth/tokens/tok-to-revoke-999") && options?.method === "DELETE") {
        return Promise.resolve({
          ok: true,
          json: async () => ({ message: "Token revoked successfully", revoked_id: "tok-to-revoke-999" }),
        });
      }
      return Promise.resolve({
        ok: true,
        json: async () => ({}),
      });
    });

    global.fetch = mockFetch;

    render(
      <UserSettingsModal
        isOpen={true}
        onClose={vi.fn()}
        eppn="sarah.connor@state.edu"
      />
    );

    await waitFor(() => {
      expect(screen.getByTestId("agent-token-list")).toHaveTextContent("Deprecated Bot");
    });

    const revokeBtn = screen.getByRole("button", { name: "Revoke" });
    fireEvent.click(revokeBtn);

    await waitFor(() => {
      expect(mockFetch).toHaveBeenCalledWith(
        expect.stringMatching(/\/api\/v1\/auth\/tokens\/tok-to-revoke-999/),
        expect.objectContaining({ method: "DELETE" })
      );
    });
  });

  // Test 3: A faculty persona does not see admin-settings-tab-btn
  it("a faculty persona does not see admin-settings-tab-btn", async () => {
    global.fetch = vi.fn().mockResolvedValue({
      ok: true,
      json: async () => [],
    });

    render(<AdminDesk initialPath="/admin" initialEppn="sarah.connor@state.edu" />);

    await waitFor(() => {
      expect(screen.queryByTestId("token-sign-in")).not.toBeInTheDocument();
    });

    // Faculty persona must not see admin-settings-tab-btn
    expect(screen.queryByTestId("admin-settings-tab-btn")).not.toBeInTheDocument();
  });

  // Test 4: Saving tokens.max_days calls PUT /api/v1/settings/tokens.max_days
  it("saving tokens.max_days calls PUT /api/v1/settings/tokens.max_days", async () => {
    const mockFetch = vi.fn().mockImplementation((url, options) => {
      const urlStr = String(url);
      if (urlStr.endsWith("/settings") && (!options || options.method === "GET" || !options.method)) {
        return Promise.resolve({
          ok: true,
          json: async () => ({
            settings: {
              "tokens.max_days": 90,
              "tokens.agent_enabled": true,
            },
          }),
        });
      }
      if (urlStr.endsWith("/settings/tokens.max_days") && options?.method === "PUT") {
        return Promise.resolve({
          ok: true,
          json: async () => ({ message: "Setting updated" }),
        });
      }
      return Promise.resolve({
        ok: true,
        json: async () => ({}),
      });
    });

    global.fetch = mockFetch;

    render(
      <AdminConsoleView
        activePersona={mockCentralAdmin}
        isImpersonating={false}
        realAdmin={null}
        handleStopImpersonation={vi.fn()}
        navigateTo={vi.fn()}
        adminTab="settings"
        apps={[]}
        sourceRules={[]}
        simAction="read"
        setSimAction={vi.fn()}
        simFerpa={false}
        setSimFerpa={vi.fn()}
        simResult={{ decision: "", reason: "", rule: "" }}
        ledger={[]}
        setNotificationToast={vi.fn()}
        handleDownloadOscal={vi.fn()}
        personas={[mockCentralAdmin, mockFaculty]}
        handleStartImpersonation={vi.fn()}
      />
    );

    await waitFor(() => {
      expect(screen.getByTestId("setting-input-tokens.max_days")).toBeInTheDocument();
    });

    const maxDaysInput = screen.getByTestId("setting-input-tokens.max_days");
    fireEvent.change(maxDaysInput, { target: { value: "180" } });

    const saveBtn = screen.getByTestId("setting-save-tokens.max_days");
    fireEvent.click(saveBtn);

    await waitFor(() => {
      expect(mockFetch).toHaveBeenCalledWith(
        expect.stringMatching(/\/api\/v1\/settings\/tokens\.max_days/),
        expect.objectContaining({
          method: "PUT",
          body: "180",
        })
      );
    });
  });

  // Test 5: localStorage holds no value equal to the minted token
  it("localStorage holds no value equal to the minted token", async () => {
    const mintedSecret = "scf_super_secret_agent_key_98765";

    global.fetch = vi.fn().mockImplementation((url, options) => {
      const urlStr = String(url);
      if (urlStr.endsWith("/settings")) {
        return Promise.resolve({
          ok: true,
          json: async () => ({
            settings: { "tokens.agent_enabled": true, "tokens.max_days": 90 },
          }),
        });
      }
      if (urlStr.endsWith("/auth/tokens") && (!options || options.method === "GET" || !options.method)) {
        return Promise.resolve({
          ok: true,
          json: async () => ({ tokens: [] }),
        });
      }
      if (urlStr.endsWith("/auth/tokens") && options?.method === "POST") {
        return Promise.resolve({
          ok: true,
          json: async () => ({
            token: mintedSecret,
            id: "tok-sec-555",
            expires_at: "2026-11-08T00:00:00Z",
            label: "Confidential Token",
            kind: "agent",
          }),
        });
      }
      return Promise.resolve({
        ok: true,
        json: async () => ({}),
      });
    });

    render(
      <UserSettingsModal
        isOpen={true}
        onClose={vi.fn()}
        eppn="sarah.connor@state.edu"
      />
    );

    await waitFor(() => {
      expect(screen.getByTestId("agent-token-form")).toBeInTheDocument();
    });

    const labelInput = screen.getByPlaceholderText(/Cursor Assistant/i);
    fireEvent.change(labelInput, { target: { value: "Confidential Token" } });
    fireEvent.submit(screen.getByTestId("agent-token-form"));

    await waitFor(() => {
      expect(screen.getByTestId("agent-token-secret")).toBeInTheDocument();
    });

    // Check all values in localStorage
    const storedValues: string[] = [];
    for (let i = 0; i < localStorage.length; i++) {
      const key = localStorage.key(i);
      if (key) {
        storedValues.push(localStorage.getItem(key) || "");
      }
    }

    expect(storedValues).not.toContain(mintedSecret);
    for (const val of storedValues) {
      expect(val).not.toContain(mintedSecret);
    }
  });
});
