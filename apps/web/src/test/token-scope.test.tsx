import React from "react";
import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import "@testing-library/jest-dom";
import { UserSettingsModal } from "../UserSettingsModal";
import { scopeSentence } from "../tokenScope";

const APPS = [{ slug: "admissions", title: "Admissions", department: "Registrar", workspace_id: "ws-1", description: "" }];
const MANIFEST = { slug: "admissions", title: "Admissions", tables: [{ id: "applicants", name: "Applicants" }, { id: "notes", name: "Notes" }] };

function mockServer(listed: any[] = [], onCreate: (body: any) => void = () => {}) {
  global.fetch = vi.fn().mockImplementation((url, options) => {
    const u = String(url);
    const get = !options || !options.method || options.method === "GET";
    const reply = (body: unknown) => Promise.resolve({ ok: true, status: 200, json: async () => body });
    if (u.endsWith("/settings") && get) return reply({ settings: { "tokens.agent_enabled": true, "tokens.max_days": 90 } });
    if (u.endsWith("/auth/tokens") && get) return reply(listed);
    if (u.endsWith("/auth/tokens") && options?.method === "POST") {
      const body = JSON.parse(options.body as string);
      onCreate(body);
      return reply({ token: "scf_secret", id: "t1", label: body.label, kind: "agent", expires_at: "2026-11-01T00:00:00Z", scope: body.scope });
    }
    if (u.endsWith("/me/apps")) return reply({ apps: APPS });
    if (u.endsWith("/apps/admissions")) return reply(MANIFEST);
    return reply({});
  });
}

describe("Integrations phase 2: Limit this token", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    localStorage.clear();
    localStorage.setItem("scaffoldry_token", "test-token");
  });

  it("the sentence matches the boxes", () => {
    const names = { apps: { admissions: "Admissions" }, tables: { applicants: "Applicants", notes: "Notes" } };
    expect(scopeSentence({ apps: ["admissions"], tables: { applicants: ["read", "create"] } }, names)).toBe(
      "This token can read and add Applicants in Admissions. It cannot do anything else."
    );
    expect(scopeSentence({ apps: ["admissions"], tables: { applicants: ["read"], notes: ["read", "update", "delete"] } }, names)).toBe(
      "This token can read, change, and delete Applicants and Notes in Admissions. It cannot do anything else."
    );
    expect(scopeSentence(null)).toBe("This token has all the rights of its owner.");
  });

  it("choosing an app, a table and operations shows the sentence and mints a limited token", async () => {
    const created: any[] = [];
    mockServer([], (b) => created.push(b));
    render(<UserSettingsModal isOpen onClose={() => {}} eppn="sarah.connor@state.edu" />);
    await screen.findByTestId("agent-token-form");

    fireEvent.click(screen.getByLabelText(/limit this token/i));
    fireEvent.click(await screen.findByLabelText("Admissions"));
    fireEvent.click(await screen.findByLabelText("Applicants"));
    fireEvent.click(screen.getByLabelText("Add"));
    expect(screen.getByTestId("token-scope-sentence")).toHaveTextContent("This token can read and add Applicants in Admissions. It cannot do anything else.");

    fireEvent.change(screen.getByPlaceholderText(/Cursor Assistant/i), { target: { value: "Intake agent" } });
    fireEvent.submit(screen.getByTestId("agent-token-form"));
    await waitFor(() => expect(created).toHaveLength(1));
    expect(created[0].scope).toEqual({ apps: ["admissions"], tables: { applicants: ["read", "create"] } });
  });

  it("a limited token cannot be minted with nothing chosen", async () => {
    const created: any[] = [];
    mockServer([], (b) => created.push(b));
    render(<UserSettingsModal isOpen onClose={() => {}} eppn="sarah.connor@state.edu" />);
    await screen.findByTestId("agent-token-form");
    fireEvent.click(screen.getByLabelText(/limit this token/i));
    fireEvent.change(screen.getByPlaceholderText(/Cursor Assistant/i), { target: { value: "Nothing" } });
    fireEvent.submit(screen.getByTestId("agent-token-form"));
    expect(await screen.findByText(/choose an app and a table/i)).toBeInTheDocument();
    expect(created).toHaveLength(0);
  });

  it("the token list shows each token's limit as a sentence", async () => {
    mockServer([
      { id: "a", label: "Limited", kind: "agent", created_at: "2026-10-01T00:00:00Z", expires_at: "2026-11-01T00:00:00Z", last_used_at: null, scope: { apps: ["admissions"], tables: { applicants: ["read"] } } },
      { id: "b", label: "Open", kind: "agent", created_at: "2026-10-01T00:00:00Z", expires_at: "2026-11-01T00:00:00Z", last_used_at: null, scope: null },
    ]);
    render(<UserSettingsModal isOpen onClose={() => {}} eppn="sarah.connor@state.edu" />);
    expect(await screen.findByText("This token can read Applicants in Admissions. It cannot do anything else.")).toBeInTheDocument();
    expect(screen.getByText("This token has all the rights of its owner.")).toBeInTheDocument();
  });
});
