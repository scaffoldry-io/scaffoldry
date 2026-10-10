import React from "react";
import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import "@testing-library/jest-dom";
import { AdminConsoleView } from "../AdminConsoleView";
import { Persona } from "../types";

const mockCentralAdmin: Persona = {
  eppn: "jordan.lee@state.edu",
  name: "Jordan Lee",
  affiliation: "central_admin",
  department: "Central IT",
  roleTitle: "Platform Admin",
  isAdmin: true,
};

const mockFaculty: Persona = {
  eppn: "prof.curie@science.state.edu",
  name: "Dr. Marie Curie",
  affiliation: "faculty",
  department: "Biology",
  roleTitle: "Professor",
  isAdmin: false,
};

describe("Phase 3: Platform Admin Organization Screen", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("as faculty persona, still sees admin-access-denied", () => {
    render(
      <AdminConsoleView
        activePersona={mockFaculty}
        isImpersonating={false}
        realAdmin={null}
        handleStopImpersonation={vi.fn()}
        navigateTo={vi.fn()}
        adminTab="org"
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

    expect(screen.getByTestId("admin-access-denied")).toBeInTheDocument();
  });

  it("as central_admin, opens Organization tab, sees root node, and submits create unit form", async () => {
    const mockFetch = vi.fn().mockImplementation((url, options) => {
      const urlStr = String(url);
      if (urlStr.endsWith("/orgs") && (!options || options.method === "GET" || !options.method)) {
        return Promise.resolve({
          ok: true,
          json: async () => [
            {
              id: "00000000-0000-0000-0000-000000000001",
              parent_id: null,
              name: "State University",
              code: "INST",
              org_type: "Institution",
            },
          ],
        });
      }
      if (urlStr.endsWith("/orgs") && options?.method === "POST") {
        return Promise.resolve({
          ok: true,
          json: async () => ({
            id: "11111111-1111-1111-1111-111111111111",
            parent_id: "00000000-0000-0000-0000-000000000001",
            name: "College of Sciences",
            code: "SCI",
            org_type: "College",
          }),
        });
      }
      return Promise.resolve({
        ok: true,
        json: async () => [],
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
        adminTab="org"
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

    // 1. As central_admin, open the Organization tab and see a root node
    await waitFor(() => {
      expect(screen.getByTestId("org-node-INST")).toBeInTheDocument();
    });

    expect(screen.getByTestId("org-detail")).toBeInTheDocument();
    expect(screen.getByTestId("org-create-form")).toBeInTheDocument();

    // 2. Submit the create form and assert fetch was called with POST /api/v1/orgs
    const nameInput = screen.getByLabelText(/Name/i);
    const codeInput = screen.getByLabelText(/Code/i);
    fireEvent.change(nameInput, { target: { value: "College of Sciences" } });
    fireEvent.change(codeInput, { target: { value: "SCI" } });

    const createForm = screen.getByTestId("org-create-form");
    fireEvent.submit(createForm);

    await waitFor(() => {
      expect(mockFetch).toHaveBeenCalledWith(
        expect.stringMatching(/\/api\/v1\/orgs/),
        expect.objectContaining({
          method: "POST",
          body: expect.stringContaining("College of Sciences"),
        })
      );
    });
  });
});
