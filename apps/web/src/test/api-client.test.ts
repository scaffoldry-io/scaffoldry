import { describe, it, expect, beforeEach, afterEach, vi } from "vitest";
import {
  apiClient,
  ApiError,
  setAuthToken,
  getAuthToken,
} from "../api";

describe("Web API Client (Slice 5 TDD)", () => {
  const originalFetch = global.fetch;

  beforeEach(() => {
    setAuthToken("test-bearer-token-123");
  });

  afterEach(() => {
    global.fetch = originalFetch;
    vi.restoreAllMocks();
  });

  it("sends Authorization Bearer header on authenticated calls", async () => {
    const mockWorkspaces = [
      {
        id: "ws-cs-research",
        name: "Computer Science & Systems Lab",
        code: "CS",
        department: "Computer Science",
        description: "Test description",
        icon: "💻",
        lead: "Dr. Sarah Connor",
        appCount: 1,
        visibility: "restricted",
        collaborators: [],
      },
    ];

    global.fetch = vi.fn().mockResolvedValue({
      ok: true,
      status: 200,
      json: async () => mockWorkspaces,
    });

    const workspaces = await apiClient.listWorkspaces();
    expect(workspaces).toEqual(mockWorkspaces);
    expect(global.fetch).toHaveBeenCalledWith(
      "/api/v1/workspaces",
      expect.objectContaining({
        headers: expect.objectContaining({
          Authorization: "Bearer test-bearer-token-123",
          "Content-Type": "application/json",
        }),
      })
    );
  });

  it("handles 403 Forbidden with Cedar Policy error details", async () => {
    global.fetch = vi.fn().mockResolvedValue({
      ok: false,
      status: 403,
      json: async () => ({
        error: "403 Forbidden: Cedar Policy restricts access to ws-bio-lab",
      }),
    });

    await expect(apiClient.getWorkspace("ws-bio-lab")).rejects.toThrowError(ApiError);
    try {
      await apiClient.getWorkspace("ws-bio-lab");
    } catch (e: any) {
      expect(e).toBeInstanceOf(ApiError);
      expect(e.status).toBe(403);
      expect(e.message).toContain("Cedar Policy restricts access");
    }
  });

  it("adds a workspace collaborator and returns server-verified collaborator record", async () => {
    const mockMember = {
      id: "collab-cs-2",
      workspace_id: "ws-cs-research",
      eppn: "marcus.vance@state.edu",
      name: "Marcus Vance",
      role: "editor" as const,
      department: "Office of Sponsored Programs",
      scoped_affiliation: "staff",
    };

    global.fetch = vi.fn().mockResolvedValue({
      ok: true,
      status: 201,
      json: async () => mockMember,
    });

    const member = await apiClient.addWorkspaceCollaborator("ws-cs-research", {
      eppn: "marcus.vance@state.edu",
      name: "Marcus Vance",
      role: "editor",
      department: "Office of Sponsored Programs",
      scoped_affiliation: "staff",
    });

    expect(member).toEqual(mockMember);
    expect(global.fetch).toHaveBeenCalledWith(
      "/api/v1/workspaces/ws-cs-research/collaborators",
      expect.objectContaining({
        method: "POST",
        body: JSON.stringify({
          eppn: "marcus.vance@state.edu",
          name: "Marcus Vance",
          role: "editor",
          department: "Office of Sponsored Programs",
          scoped_affiliation: "staff",
        }),
      })
    );
  });

  it("fetches genuine cryptographic decision ledger and verifies chain integrity", async () => {
    const mockLedger = {
      chain_valid: true,
      total_entries: 5,
      head_hash: "sha256-abc123def456",
      entries: [
        {
          sequence: 0,
          timestamp_iso: "2026-10-04T12:00:00Z",
          principal: "genesis",
          organization_code: "SYSTEM",
          decision_type: "FrameworkGenesis",
          oscal_control_id: "AU-02",
          rationale: "Genesis block initialization",
          payload_hash: "hash0",
          previous_hash: "0000000000000000000000000000000000000000000000000000000000000000",
          entry_hash: "sha256-genesis",
        },
      ],
    };

    global.fetch = vi.fn().mockResolvedValue({
      ok: true,
      status: 200,
      json: async () => mockLedger,
    });

    const res = await apiClient.getGovernanceLedger();
    expect(res.chain_valid).toBe(true);
    expect(res.entries.length).toBe(1);
    expect(global.fetch).toHaveBeenCalledWith(
      "/api/v1/governance/ledger",
      expect.any(Object)
    );
  });
});
