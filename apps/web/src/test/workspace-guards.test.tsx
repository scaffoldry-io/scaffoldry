import "@testing-library/jest-dom";
import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, waitFor, fireEvent } from "@testing-library/react";
import { WorkspaceSettingsModal } from "../WorkspaceSettingsModal";
import { Toast } from "../ui/Toast";
import { ApiError, apiClient } from "../api";
import { Persona, Workspace } from "../types";

const ownerPersona: Persona = {
  eppn: "prof.curie@science.state.edu",
  name: "Dr. Marie Curie",
  affiliation: "faculty",
  department: "biology",
  roleTitle: "Professor",
  isAdmin: false,
};

const nonOwnerFaculty: Persona = {
  eppn: "faculty.other@state.edu",
  name: "Dr. Other Faculty",
  affiliation: "faculty",
  department: "physics",
  roleTitle: "Professor",
  isAdmin: false,
};

const adminPersona: Persona = {
  eppn: "jordan.lee@state.edu",
  name: "Jordan Lee",
  affiliation: "central_admin",
  department: "central_admin",
  roleTitle: "Central Administrator",
  isAdmin: true,
};

const sampleWorkspace: Workspace = {
  id: "ws-bio-lab",
  name: "Biology Lab",
  code: "BIO",
  department: "biology",
  description: "Bio research workspace",
  icon: "🔬",
  lead: "prof.curie@science.state.edu",
  visibility: "restricted",
  allowed_affiliations: ["faculty", "staff"],
  data_classification: "Level 4 Restricted",
  created_at: new Date().toISOString(),
  collaborators: [
    {
      id: "c-1",
      eppn: "prof.curie@science.state.edu",
      name: "Dr. Marie Curie",
      role: "owner",
      department: "biology",
      scoped_affiliation: "faculty",
      added_at: new Date().toISOString(),
    },
    {
      id: "c-2",
      eppn: "student.alice@state.edu",
      name: "Alice Smith",
      role: "viewer",
      department: "biology",
      scoped_affiliation: "student",
      added_at: new Date().toISOString(),
    },
  ],
};

describe("Workspace Guards Screen (Phase 3)", () => {
  beforeEach(() => {
    vi.restoreAllMocks();
    vi.spyOn(apiClient, "getWorkspaceGuards").mockResolvedValue({
      current: {
        workspace_id: "ws-bio-lab",
        version: 1,
        rules: [],
        compiled: "// Compiled Cedar",
        sentences: [],
        reason: "Initial",
        created_by: "admin",
        created_at: new Date().toISOString(),
      },
      history: [],
    });
    vi.spyOn(apiClient, "listOrganizations").mockResolvedValue([
      { id: "u-1", name: "Biology Dept", code: "BIO", parent_id: null, org_type: "Department" },
      { id: "u-2", name: "Physics Dept", code: "PHYS", parent_id: null, org_type: "Department" },
    ]);
  });

  it("1. Adding a rule shows the sentence preview, then the impact list, and Save stays disabled until a reason is typed", async () => {
    vi.spyOn(apiClient, "impactWorkspaceGuards").mockResolvedValue({
      changes: [
        {
          eppn: "student.alice@state.edu",
          action: "export",
          is_ferpa_sensitive: false,
          current_allowed: true,
          proposed_allowed: false,
        },
      ],
    });

    render(
      <WorkspaceSettingsModal
        workspace={sampleWorkspace}
        activePersona={ownerPersona}
        allPersonas={[ownerPersona, nonOwnerFaculty]}
        isOpen={true}
        onClose={() => {}}
        onSave={() => {}}
        ledgerEntries={[]}
      />
    );

    // Switch to Rules tab
    fireEvent.click(screen.getByTestId("guard-rules"));

    // Click Add a rule
    const addBtn = await screen.findByTestId("add-rule-btn");
    fireEvent.click(addBtn);

    // Sentence preview is displayed
    const preview = await screen.findByTestId("sentence-preview");
    expect(preview).toHaveTextContent(/Only faculty may export from this workspace/i);

    // Impact list is displayed
    const impact = await screen.findByTestId("impact-summary");
    expect(impact).toHaveTextContent(/would lose access/i);

    // Save button stays disabled until reason is typed
    const saveBtn = screen.getByRole("button", { name: /save rule/i });
    expect(saveBtn).toBeDisabled();

    // Type a reason
    const reasonInput = screen.getByLabelText(/reason for decision/i);
    fireEvent.change(reasonInput, { target: { value: "FERPA compliance update" } });

    // Save button is now enabled
    expect(saveBtn).not.toBeDisabled();
  });

  it("2. Save sends PUT /api/v1/workspaces/{id}/guards with the rules and the reason", async () => {
    vi.spyOn(apiClient, "impactWorkspaceGuards").mockResolvedValue({ changes: [] });
    const putSpy = vi.spyOn(apiClient, "putWorkspaceGuards").mockResolvedValue({});

    render(
      <WorkspaceSettingsModal
        workspace={sampleWorkspace}
        activePersona={ownerPersona}
        allPersonas={[ownerPersona]}
        isOpen={true}
        onClose={() => {}}
        onSave={() => {}}
        ledgerEntries={[]}
      />
    );

    fireEvent.click(screen.getByTestId("guard-rules"));
    fireEvent.click(await screen.findByTestId("add-rule-btn"));

    // Type reason
    const reasonInput = await screen.findByLabelText(/reason for decision/i);
    fireEvent.change(reasonInput, { target: { value: "Regulatory alignment reason" } });

    // Click Save
    const saveBtn = screen.getByRole("button", { name: /save rule/i });
    fireEvent.click(saveBtn);

    await waitFor(() => {
      expect(putSpy).toHaveBeenCalledWith("ws-bio-lab", {
        rules: [
          {
            template: "deny_export_unless_affiliation",
            affiliations: ["faculty"],
            unit_ids: [],
          },
        ],
        reason: "Regulatory alignment reason",
      });
    });
  });

  it("3. When the impact response is empty, the drawer says No one is affected.", async () => {
    vi.spyOn(apiClient, "impactWorkspaceGuards").mockResolvedValue({ changes: [] });

    render(
      <WorkspaceSettingsModal
        workspace={sampleWorkspace}
        activePersona={ownerPersona}
        allPersonas={[ownerPersona]}
        isOpen={true}
        onClose={() => {}}
        onSave={() => {}}
        ledgerEntries={[]}
      />
    );

    fireEvent.click(screen.getByTestId("guard-rules"));
    fireEvent.click(await screen.findByTestId("add-rule-btn"));

    const impact = await screen.findByTestId("impact-summary");
    expect(impact).toHaveTextContent("No one is affected.");
  });

  it("4. A faculty member who is not an owner does not see Add a rule", async () => {
    render(
      <WorkspaceSettingsModal
        workspace={sampleWorkspace}
        activePersona={nonOwnerFaculty}
        allPersonas={[nonOwnerFaculty]}
        isOpen={true}
        onClose={() => {}}
        onSave={() => {}}
        ledgerEntries={[]}
      />
    );

    fireEvent.click(screen.getByTestId("guard-rules"));
    await screen.findByTestId("guard-rules-container");

    expect(screen.queryByTestId("add-rule-btn")).not.toBeInTheDocument();
  });

  it("5. A Platform Admin sees Advanced and a faculty owner does not", async () => {
    // 1. Platform Admin
    const { unmount } = render(
      <WorkspaceSettingsModal
        workspace={sampleWorkspace}
        activePersona={adminPersona}
        allPersonas={[adminPersona]}
        isOpen={true}
        onClose={() => {}}
        onSave={() => {}}
        ledgerEntries={[]}
      />
    );

    fireEvent.click(screen.getByTestId("guard-rules"));
    expect(await screen.findByTestId("guard-rules-advanced")).toBeInTheDocument();
    unmount();

    // 2. Faculty Owner
    render(
      <WorkspaceSettingsModal
        workspace={sampleWorkspace}
        activePersona={ownerPersona}
        allPersonas={[ownerPersona]}
        isOpen={true}
        onClose={() => {}}
        onSave={() => {}}
        ledgerEntries={[]}
      />
    );

    fireEvent.click(screen.getByTestId("guard-rules"));
    await screen.findByTestId("guard-rules-container");
    expect(screen.queryByTestId("guard-rules-advanced")).not.toBeInTheDocument();
  });

  it("6. A 403 carrying a guard policy shows that sentence in the toast", async () => {
    const guardPolicySentence = "Only Faculty and Staff may export from this workspace.";
    const guardError = new ApiError(403, "Forbidden", {
      code: "forbidden",
      policy: {
        id: "guard-ws-bio-1",
        description: guardPolicySentence,
      },
    });

    render(<Toast tone="error" error={guardError} onClose={() => {}} />);

    expect(screen.getByText(new RegExp(guardPolicySentence, "i"))).toBeInTheDocument();
  });
});
