import React from "react";
import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import "@testing-library/jest-dom";
import { ProcessDesk } from "../ProcessDesk";
import { Persona, RegisteredApp } from "../types";

const mockPersona: Persona = {
  eppn: "sarah.connor@state.edu",
  name: "Dr. Sarah Connor",
  affiliation: "faculty",
  department: "Computer Science",
  roleTitle: "Department Chair",
  isAdmin: false,
};

const mockApp: RegisteredApp = {
  slug: "cs-admissions",
  title: "CS Admissions Review",
  orgCode: "DIV-SCIENCES",
  department: "Computer Science",
  customDomain: "admissions.cs.state.edu",
  verified: true,
  hermCapability: "Academic Operations",
  cedsDomain: "PostsecondaryStudent",
  status: "Published",
  updatedAt: "2026-10-08",
  recordsCount: 1,
  workspaceId: "ws-cs-research",
  collaborators: [],
  manifest: {
    app_id: "cs-admissions",
    title: "CS Admissions Review",
    description: "Admissions review app",
    datasets: [],
    views: [],
  },
};

describe("Phase 5: ProcessDesk Decisions Queue", () => {
  beforeEach(() => {
    vi.restoreAllMocks();
  });

  afterEach(() => {
    vi.restoreAllMocks();
  });

  it("a waiting row renders the prompt", async () => {
    const mockWaitingInstance = {
      id: "rule-approve-cand:rec-101",
      rule_id: "rule-approve-cand",
      app_slug: "cs-admissions",
      record_id: "rec-101",
      status: "Waiting",
      waiting_step_id: "step-chair-signoff",
      role: "chair",
      prompt: "Please review and approve graduate applicant admission",
      log: [],
    };

    global.fetch = vi.fn().mockImplementation((url: string) => {
      if (url.includes("/api/v1/apps/cs-admissions/processes")) {
        return Promise.resolve({
          ok: true,
          status: 200,
          json: async () => [mockWaitingInstance],
        });
      }
      return Promise.resolve({
        ok: true,
        status: 200,
        json: async () => [],
      });
    });

    render(
      <ProcessDesk
        apps={[mockApp]}
        callerPersona={mockPersona}
        callerWorkspaceRole="chair"
      />
    );

    expect(screen.getByTestId("process-desk")).toBeInTheDocument();
    expect(screen.getByText("Decisions")).toBeInTheDocument();

    await waitFor(() => {
      expect(
        screen.getByTestId("process-row-rule-approve-cand:rec-101")
      ).toBeInTheDocument();
    });

    expect(
      screen.getByText("Please review and approve graduate applicant admission")
    ).toBeInTheDocument();
  });

  it('Approve calls fetch with the decide path and body {"decision":"approve"}', async () => {
    const mockWaitingInstance = {
      id: "rule-approve-cand:rec-101",
      rule_id: "rule-approve-cand",
      app_slug: "cs-admissions",
      record_id: "rec-101",
      status: "Waiting",
      waiting_step_id: "step-chair-signoff",
      role: "chair",
      prompt: "Please review and approve graduate applicant admission",
      log: [],
    };

    const fetchMock = vi.fn().mockImplementation((url: string, options?: any) => {
      if (url.includes("/api/v1/apps/cs-admissions/processes?status=Waiting")) {
        return Promise.resolve({
          ok: true,
          status: 200,
          json: async () => [mockWaitingInstance],
        });
      }
      if (url.includes("/api/v1/apps/cs-admissions/processes/rule-approve-cand:rec-101/decide")) {
        return Promise.resolve({
          ok: true,
          status: 200,
          json: async () => ({ ...mockWaitingInstance, status: "Completed" }),
        });
      }
      return Promise.resolve({
        ok: true,
        status: 200,
        json: async () => [],
      });
    });
    global.fetch = fetchMock;

    render(
      <ProcessDesk
        apps={[mockApp]}
        callerPersona={mockPersona}
        callerWorkspaceRole="chair"
      />
    );

    await waitFor(() => {
      expect(
        screen.getByTestId("process-approve-rule-approve-cand:rec-101")
      ).toBeInTheDocument();
    });

    fireEvent.click(
      screen.getByTestId("process-approve-rule-approve-cand:rec-101")
    );

    await waitFor(() => {
      expect(fetchMock).toHaveBeenCalledWith(
        "/api/v1/apps/cs-admissions/processes/rule-approve-cand:rec-101/decide",
        expect.objectContaining({
          method: "POST",
          body: JSON.stringify({ decision: "approve" }),
        })
      );
    });

    // On 200, row is removed
    await waitFor(() => {
      expect(
        screen.queryByTestId("process-row-rule-approve-cand:rec-101")
      ).not.toBeInTheDocument();
    });
  });

  it("A 403 leaves the row on screen", async () => {
    const mockWaitingInstance = {
      id: "rule-approve-cand:rec-101",
      rule_id: "rule-approve-cand",
      app_slug: "cs-admissions",
      record_id: "rec-101",
      status: "Waiting",
      waiting_step_id: "step-chair-signoff",
      role: "chair",
      prompt: "Please review and approve graduate applicant admission",
      log: [],
    };

    const fetchMock = vi.fn().mockImplementation((url: string, options?: any) => {
      if (url.includes("/api/v1/apps/cs-admissions/processes?status=Waiting")) {
        return Promise.resolve({
          ok: true,
          status: 200,
          json: async () => [mockWaitingInstance],
        });
      }
      if (url.includes("/api/v1/apps/cs-admissions/processes/rule-approve-cand:rec-101/decide")) {
        return Promise.resolve({
          ok: false,
          status: 403,
          json: async () => ({ error: "Forbidden" }),
        });
      }
      return Promise.resolve({
        ok: true,
        status: 200,
        json: async () => [],
      });
    });
    global.fetch = fetchMock;

    render(
      <ProcessDesk
        apps={[mockApp]}
        callerPersona={mockPersona}
        callerWorkspaceRole="chair"
      />
    );

    await waitFor(() => {
      expect(
        screen.getByTestId("process-approve-rule-approve-cand:rec-101")
      ).toBeInTheDocument();
    });

    fireEvent.click(
      screen.getByTestId("process-approve-rule-approve-cand:rec-101")
    );

    // Row must still be on screen and show 403
    await waitFor(() => {
      expect(
        screen.getByTestId("process-row-rule-approve-cand:rec-101")
      ).toBeInTheDocument();
    });

    expect(screen.getByText(/403/)).toBeInTheDocument();
  });
});

describe("Approvers: the desk shows what the server returned", () => {
  afterEach(() => {
    vi.restoreAllMocks();
  });

  it("asks for mine=true and does not compare roles in the browser", async () => {
    const row = {
      id: "rule-chair:rec-7",
      rule_id: "rule-chair",
      app_slug: "cs-admissions",
      record_id: "rec-7",
      status: "Waiting",
      waiting_step_id: "step-chair",
      // A role the caller does not have. The old browser check would have hidden this row.
      role: "dean",
      prompt: "Sign off as chair",
      log: [],
    };
    const fetchMock = vi.fn().mockImplementation(() =>
      Promise.resolve({ ok: true, status: 200, json: async () => [row] })
    );
    global.fetch = fetchMock;

    render(
      <ProcessDesk
        apps={[mockApp]}
        callerPersona={{ ...mockPersona, affiliation: "student" }}
        callerWorkspaceRole="viewer"
      />
    );

    expect(await screen.findByTestId("process-row-rule-chair:rec-7")).toBeInTheDocument();
    expect(fetchMock.mock.calls[0][0]).toContain("mine=true");
  });

  it("shows nothing when the server returns nothing", async () => {
    global.fetch = vi.fn().mockResolvedValue({ ok: true, status: 200, json: async () => [] });
    render(<ProcessDesk apps={[mockApp]} callerPersona={mockPersona} callerWorkspaceRole="chair" />);
    expect(await screen.findByText("No decisions waiting")).toBeInTheDocument();
  });
});
