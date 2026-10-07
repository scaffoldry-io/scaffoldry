import React, { useState } from "react";
import { Collaborator, LedgerEntryItem, Persona, Workspace } from "./types";

interface WorkspaceSettingsModalProps {
  workspace: Workspace;
  activePersona: Persona;
  allPersonas: Persona[];
  isOpen: boolean;
  onClose: () => void;
  onSave: (updated: Workspace, auditEntry: LedgerEntryItem) => void;
  ledgerEntries: LedgerEntryItem[];
}

export const WorkspaceSettingsModal: React.FC<WorkspaceSettingsModalProps> = ({
  workspace,
  activePersona,
  allPersonas,
  isOpen,
  onClose,
  onSave,
  ledgerEntries,
}) => {
  const [tab, setTab] = useState<"general" | "access" | "members" | "audit">("general");

  // Form states
  const [name, setName] = useState(workspace.name);
  const [code, setCode] = useState(workspace.code || "DEPT");
  const [department, setDepartment] = useState(workspace.department);
  const [description, setDescription] = useState(workspace.description);
  const [icon, setIcon] = useState(workspace.icon);
  const [visibility, setVisibility] = useState<"restricted" | "departmental" | "institutional">(workspace.visibility);
  const [dataClassification, setDataClassification] = useState(workspace.data_classification || "Level 3 Internal");
  const [collaborators, setCollaborators] = useState<Collaborator[]>(workspace.collaborators || []);

  // Add Member State
  const [selectedDirectoryEppn, setSelectedDirectoryEppn] = useState<string>(allPersonas[0]?.eppn || "");
  const [newMemberRole, setNewMemberRole] = useState<"owner" | "admin" | "editor" | "viewer">("editor");

  if (!isOpen) return null;

  const handleAddMember = (e: React.FormEvent) => {
    e.preventDefault();
    if (!selectedDirectoryEppn) return;

    if (collaborators.some((c) => c.eppn === selectedDirectoryEppn)) {
      alert("This user is already a member of this workspace.");
      return;
    }

    const persona = allPersonas.find((p) => p.eppn === selectedDirectoryEppn);
    const newCollab: Collaborator = {
      id: `collab-${Date.now()}`,
      eppn: selectedDirectoryEppn,
      name: persona?.name || selectedDirectoryEppn,
      role: newMemberRole,
      department: persona?.department || department,
      scoped_affiliation: persona?.affiliation || "staff",
      added_at: new Date().toISOString(),
    };

    const updatedCollabs = [...collaborators, newCollab];
    setCollaborators(updatedCollabs);

    const updatedWs: Workspace = {
      ...workspace,
      collaborators: updatedCollabs,
    };

    const auditEntry: LedgerEntryItem = {
      sequence: ledgerEntries.length,
      timestamp_iso: new Date().toISOString(),
      principal: activePersona.eppn,
      organization_code: `DEPT-${workspace.department.toUpperCase()}`,
      app_slug: workspace.id,
      decision_type: "WorkspaceMemberAdded",
      oscal_control_id: "AC-02",
      rationale: `Added collaborator ${newCollab.name} (${newCollab.eppn}) with role ${newCollab.role}`,
      payload: { workspace_id: workspace.id, member_eppn: newCollab.eppn, role: newCollab.role },
      previous_hash: ledgerEntries[ledgerEntries.length - 1]?.entry_hash || "0000000000000000000000000000000000000000000000000000000000000000",
      payload_hash: `sha256-${Date.now().toString(16)}`,
      entry_hash: `sha256-${Date.now().toString(16)}${Math.random().toString(16).slice(2, 10)}`,
    };

    onSave(updatedWs, auditEntry);
  };

  const handleUpdateRole = (eppn: string, newRole: "owner" | "admin" | "editor" | "viewer") => {
    const updatedCollabs = collaborators.map((c) => (c.eppn === eppn ? { ...c, role: newRole } : c));
    setCollaborators(updatedCollabs);

    const updatedWs: Workspace = {
      ...workspace,
      collaborators: updatedCollabs,
    };

    const auditEntry: LedgerEntryItem = {
      sequence: ledgerEntries.length,
      timestamp_iso: new Date().toISOString(),
      principal: activePersona.eppn,
      organization_code: `DEPT-${workspace.department.toUpperCase()}`,
      app_slug: workspace.id,
      decision_type: "WorkspaceMemberRoleUpdated",
      oscal_control_id: "AC-03",
      rationale: `Updated collaborator ${eppn} role to ${newRole}`,
      payload: { workspace_id: workspace.id, member_eppn: eppn, new_role: newRole },
      previous_hash: ledgerEntries[ledgerEntries.length - 1]?.entry_hash || "0000000000000000000000000000000000000000000000000000000000000000",
      payload_hash: `sha256-${Date.now().toString(16)}`,
      entry_hash: `sha256-${Date.now().toString(16)}${Math.random().toString(16).slice(2, 10)}`,
    };

    onSave(updatedWs, auditEntry);
  };

  const handleRemoveMember = (eppn: string) => {
    const target = collaborators.find((c) => c.eppn === eppn);
    if (!target) return;

    if (target.role === "owner") {
      const ownerCount = collaborators.filter((c) => c.role === "owner").length;
      if (ownerCount <= 1) {
        alert("Cannot remove the only workspace owner. Assign another owner before removing.");
        return;
      }
    }

    const updatedCollabs = collaborators.filter((c) => c.eppn !== eppn);
    setCollaborators(updatedCollabs);

    const updatedWs: Workspace = {
      ...workspace,
      collaborators: updatedCollabs,
    };

    const auditEntry: LedgerEntryItem = {
      sequence: ledgerEntries.length,
      timestamp_iso: new Date().toISOString(),
      principal: activePersona.eppn,
      organization_code: `DEPT-${workspace.department.toUpperCase()}`,
      app_slug: workspace.id,
      decision_type: "WorkspaceMemberRemoved",
      oscal_control_id: "AC-02",
      rationale: `Removed collaborator ${eppn} from workspace`,
      payload: { workspace_id: workspace.id, member_eppn: eppn },
      previous_hash: ledgerEntries[ledgerEntries.length - 1]?.entry_hash || "0000000000000000000000000000000000000000000000000000000000000000",
      payload_hash: `sha256-${Date.now().toString(16)}`,
      entry_hash: `sha256-${Date.now().toString(16)}${Math.random().toString(16).slice(2, 10)}`,
    };

    onSave(updatedWs, auditEntry);
  };

  const handleSaveGeneralOrAccess = (e: React.FormEvent) => {
    e.preventDefault();
    const updatedWs: Workspace = {
      ...workspace,
      name,
      code,
      department,
      description,
      icon,
      visibility,
      data_classification: dataClassification,
      collaborators,
    };

    const auditEntry: LedgerEntryItem = {
      sequence: ledgerEntries.length,
      timestamp_iso: new Date().toISOString(),
      principal: activePersona.eppn,
      organization_code: `DEPT-${department.toUpperCase()}`,
      app_slug: workspace.id,
      decision_type: "WorkspaceUpdated",
      oscal_control_id: "AC-03",
      rationale: `Workspace security & configuration updated (visibility: ${visibility}, classification: ${dataClassification})`,
      payload: {
        workspace_id: workspace.id,
        name,
        department,
        visibility,
        data_classification: dataClassification,
      },
      previous_hash: ledgerEntries[ledgerEntries.length - 1]?.entry_hash || "0000000000000000000000000000000000000000000000000000000000000000",
      payload_hash: `sha256-${Date.now().toString(16)}`,
      entry_hash: `sha256-${Date.now().toString(16)}${Math.random().toString(16).slice(2, 10)}`,
    };

    onSave(updatedWs, auditEntry);
  };

  // Generate live Cedar ABAC Policy preview based on current form inputs
  const cedarPolicySnippet = `// NIST OSCAL AC-02 / AC-03 Lattice for Workspace::"${workspace.id}"
permit (
    principal,
    action in [Action::"access_workspace"],
    resource == Workspace::"${workspace.id}"
)
when {
    ${
      visibility === "institutional"
        ? 'true // Campus-Wide institutional visibility tier'
        : visibility === "departmental"
        ? `resource.is_member || principal.department == "${department}" || principal.scoped_affiliation == "central_admin"`
        : 'resource.is_member || principal.scoped_affiliation == "central_admin"'
    }
};

permit (
    principal,
    action in [Action::"manage_workspace"],
    resource == Workspace::"${workspace.id}"
)
when {
    resource.member_role in ["owner", "admin"] || principal.scoped_affiliation == "central_admin"
};`;

  // Workspace-specific ledger entries
  const workspaceLedger = ledgerEntries.filter(
    (l) => l.app_slug === workspace.id || l.rationale.includes(workspace.id) || l.rationale.includes(workspace.name)
  );

  return (
    <div
      data-testid="workspace-settings-modal"
      className="fixed inset-0 z-50 flex items-center justify-center p-4 bg-slate-900/60 backdrop-blur-xs animate-fade-in"
    >
      <div className="bg-white dark:bg-slate-900 rounded-xl border border-slate-200 dark:border-slate-800 shadow-2xl w-full max-w-3xl overflow-hidden flex flex-col max-h-[90vh]">
        {/* Header */}
        <div className="px-6 py-4 border-b border-slate-200 dark:border-slate-800 flex items-center justify-between">
          <div className="flex items-center gap-3">
            <span className="text-2xl p-2 rounded-lg bg-blue-50 dark:bg-blue-950/60 border border-blue-200 dark:border-blue-800">
              {icon}
            </span>
            <div>
              <h2 className="text-base font-bold text-slate-900 dark:text-white flex items-center gap-2">
                <span>Workspace Security &amp; Configuration</span>
                <span className="text-[10px] font-mono uppercase px-2 py-0.5 rounded bg-blue-100 dark:bg-blue-900/40 text-blue-700 dark:text-blue-300">
                  {workspace.id}
                </span>
              </h2>
              <p className="text-xs text-slate-500">
                Governance, collaborator roles, and Cedar ABAC sharing boundaries (OSCAL AC-02, AC-03).
              </p>
            </div>
          </div>
          <button
            type="button"
            data-testid="ws-settings-close-btn"
            onClick={onClose}
            className="p-1.5 rounded-lg text-slate-400 hover:text-slate-600 dark:hover:text-slate-200 hover:bg-slate-100 dark:hover:bg-slate-800 transition-colors"
          >
            ✕
          </button>
        </div>

        {/* Tab Navigation */}
        <div className="flex border-b border-slate-200 dark:border-slate-800 px-6 bg-slate-50/50 dark:bg-slate-950/40 text-xs">
          <button
            type="button"
            data-testid="ws-settings-tab-general"
            onClick={() => setTab("general")}
            className={`py-3 px-3.5 font-medium border-b-2 transition-colors cursor-pointer ${
              tab === "general"
                ? "border-blue-600 text-blue-600 dark:text-blue-400 font-semibold"
                : "border-transparent text-slate-500 hover:text-slate-700 dark:hover:text-slate-300"
            }`}
          >
            General &amp; Details
          </button>
          <button
            type="button"
            data-testid="ws-settings-tab-access"
            onClick={() => setTab("access")}
            className={`py-3 px-3.5 font-medium border-b-2 transition-colors cursor-pointer ${
              tab === "access"
                ? "border-blue-600 text-blue-600 dark:text-blue-400 font-semibold"
                : "border-transparent text-slate-500 hover:text-slate-700 dark:hover:text-slate-300"
            }`}
          >
            Sharing &amp; Policy
          </button>
          <button
            type="button"
            data-testid="ws-settings-tab-members"
            onClick={() => setTab("members")}
            className={`py-3 px-3.5 font-medium border-b-2 transition-colors cursor-pointer ${
              tab === "members"
                ? "border-blue-600 text-blue-600 dark:text-blue-400 font-semibold"
                : "border-transparent text-slate-500 hover:text-slate-700 dark:hover:text-slate-300"
            }`}
          >
            Collaborators ({collaborators.length})
          </button>
          <button
            type="button"
            data-testid="ws-settings-tab-audit"
            onClick={() => setTab("audit")}
            className={`py-3 px-3.5 font-medium border-b-2 transition-colors cursor-pointer ${
              tab === "audit"
                ? "border-blue-600 text-blue-600 dark:text-blue-400 font-semibold"
                : "border-transparent text-slate-500 hover:text-slate-700 dark:hover:text-slate-300"
            }`}
          >
            Audit Ledger ({workspaceLedger.length})
          </button>
        </div>

        {/* Tab Body */}
        <div className="flex-1 overflow-y-auto p-6 text-xs space-y-4">
          {tab === "general" && (
            <form onSubmit={handleSaveGeneralOrAccess} className="space-y-4">
              <div className="grid grid-cols-1 sm:grid-cols-2 gap-4">
                <div>
                  <label className="block text-slate-600 dark:text-slate-300 font-medium mb-1">
                    Workspace Name
                  </label>
                  <input
                    type="text"
                    data-testid="ws-settings-name-input"
                    value={name}
                    onChange={(e) => setName(e.target.value)}
                    required
                    className="w-full px-3 py-2 rounded-lg border border-slate-300 dark:border-slate-700 bg-white dark:bg-slate-800 text-slate-900 dark:text-white"
                  />
                </div>
                <div>
                  <label className="block text-slate-600 dark:text-slate-300 font-medium mb-1">
                    Department Realm
                  </label>
                  <input
                    type="text"
                    data-testid="ws-settings-dept-input"
                    value={department}
                    onChange={(e) => setDepartment(e.target.value)}
                    required
                    className="w-full px-3 py-2 rounded-lg border border-slate-300 dark:border-slate-700 bg-white dark:bg-slate-800 text-slate-900 dark:text-white"
                  />
                </div>
                <div>
                  <label className="block text-slate-600 dark:text-slate-300 font-medium mb-1">
                    Workspace Code
                  </label>
                  <input
                    type="text"
                    data-testid="ws-settings-code-input"
                    value={code}
                    onChange={(e) => setCode(e.target.value)}
                    required
                    className="w-full px-3 py-2 rounded-lg border border-slate-300 dark:border-slate-700 bg-white dark:bg-slate-800 text-slate-900 dark:text-white font-mono uppercase"
                  />
                </div>
              </div>

              <div>
                <label className="block text-slate-600 dark:text-slate-300 font-medium mb-1">
                  Description &amp; Research Scope
                </label>
                <textarea
                  rows={2}
                  data-testid="ws-settings-desc-input"
                  value={description}
                  onChange={(e) => setDescription(e.target.value)}
                  className="w-full px-3 py-2 rounded-lg border border-slate-300 dark:border-slate-700 bg-white dark:bg-slate-800 text-slate-900 dark:text-white"
                />
              </div>

              <div className="grid grid-cols-1 sm:grid-cols-2 gap-4">
                <div>
                  <label className="block text-slate-600 dark:text-slate-300 font-medium mb-1">
                    Workspace Icon
                  </label>
                  <select
                    data-testid="ws-settings-icon-select"
                    value={icon}
                    onChange={(e) => setIcon(e.target.value)}
                    className="w-full px-3 py-2 rounded-lg border border-slate-300 dark:border-slate-700 bg-white dark:bg-slate-800 text-slate-900 dark:text-white"
                  >
                    <option value="🔬">🔬 Microscope (Biology)</option>
                    <option value="⚡">⚡ Laser / Quantum (Physics)</option>
                    <option value="🛡️">🛡️ Shield (Compliance)</option>
                    <option value="💻">💻 Computer (Computer Science)</option>
                    <option value="📊">📊 Chart (Data Science)</option>
                    <option value="🏛️">🏛️ Pillar (Governance)</option>
                  </select>
                </div>
                <div>
                  <label className="block text-slate-600 dark:text-slate-300 font-medium mb-1">
                    Data Sensitivity / Classification
                  </label>
                  <select
                    data-testid="ws-settings-classification-select"
                    value={dataClassification}
                    onChange={(e) => setDataClassification(e.target.value)}
                    className="w-full px-3 py-2 rounded-lg border border-slate-300 dark:border-slate-700 bg-white dark:bg-slate-800 text-slate-900 dark:text-white"
                  >
                    <option value="Level 1 Public">Level 1: Public</option>
                    <option value="Level 2 Campus-Wide">Level 2: Campus-Wide Internal</option>
                    <option value="Level 3 Internal">Level 3: Departmental Internal</option>
                    <option value="Level 4 Restricted">Level 4: Restricted / Confidential</option>
                  </select>
                </div>
              </div>

              <div className="pt-2 flex justify-end">
                <button
                  type="submit"
                  data-testid="ws-settings-save-btn"
                  className="px-4 py-2 bg-blue-600 hover:bg-blue-700 text-white font-semibold rounded-lg shadow-xs cursor-pointer transition-colors"
                >
                  Save Workspace Configuration
                </button>
              </div>
            </form>
          )}

          {tab === "access" && (
            <form onSubmit={handleSaveGeneralOrAccess} className="space-y-4">
              <div className="p-4 bg-slate-50 dark:bg-slate-800/50 rounded-lg border border-slate-200 dark:border-slate-700 space-y-3">
                <label className="block text-slate-700 dark:text-slate-200 font-bold">
                  Workspace Sharing &amp; Visibility Tier
                </label>
                <select
                  data-testid="ws-settings-visibility-select"
                  value={visibility}
                  onChange={(e) => setVisibility(e.target.value as any)}
                  className="w-full px-3 py-2 rounded-lg border border-slate-300 dark:border-slate-700 bg-white dark:bg-slate-800 text-slate-900 dark:text-white font-semibold"
                >
                  <option value="restricted">🔒 Restricted (Members-Only: explicit collaborators only)</option>
                  <option value="departmental">🏢 Departmental (All verified faculty and students in {department})</option>
                  <option value="institutional">🌐 Institutional (Campus-Wide: all authenticated institutional users)</option>
                </select>
                <p className="text-[11px] text-slate-500">
                  Changing this tier immediately updates the Cedar ABAC policy guard and re-evaluates access for all campus personas.
                </p>
              </div>

              <div>
                <label className="block text-slate-600 dark:text-slate-300 font-semibold mb-1">
                  Active Cedar ABAC Policy Guard Preview
                </label>
                <pre
                  data-testid="ws-cedar-policy-preview"
                  className="p-3 rounded-lg bg-slate-950 text-sky-300 font-mono text-[11px] overflow-x-auto border border-slate-800 leading-relaxed"
                >
                  {cedarPolicySnippet}
                </pre>
              </div>

              <div className="pt-2 flex justify-end">
                <button
                  type="submit"
                  data-testid="ws-settings-save-access-btn"
                  className="px-4 py-2 bg-blue-600 hover:bg-blue-700 text-white font-semibold rounded-lg shadow-xs cursor-pointer transition-colors"
                >
                  Apply Sharing Policy
                </button>
              </div>
            </form>
          )}

          {tab === "members" && (
            <div className="space-y-5">
              {/* Add Member Form */}
              <form
                onSubmit={handleAddMember}
                className="p-4 bg-slate-50 dark:bg-slate-800/40 rounded-lg border border-slate-200 dark:border-slate-700 space-y-3"
              >
                <div className="font-bold text-slate-800 dark:text-slate-200">
                  Invite Directory User to Workspace
                </div>
                <div className="grid grid-cols-1 sm:grid-cols-3 gap-3">
                  <div className="sm:col-span-2">
                    <label className="block text-[11px] text-slate-500 mb-1">Select Campus Persona</label>
                    <select
                      data-testid="ws-add-member-eppn-input"
                      value={selectedDirectoryEppn}
                      onChange={(e) => setSelectedDirectoryEppn(e.target.value)}
                      className="w-full px-3 py-1.5 rounded-lg border border-slate-300 dark:border-slate-700 bg-white dark:bg-slate-800 text-slate-900 dark:text-white"
                    >
                      {allPersonas.map((p) => (
                        <option key={p.eppn} value={p.eppn}>
                          {p.name} ({p.eppn}) - {p.department} [{p.affiliation}]
                        </option>
                      ))}
                    </select>
                  </div>
                  <div>
                    <label className="block text-[11px] text-slate-500 mb-1">Assigned Role</label>
                    <select
                      data-testid="ws-add-member-role-select"
                      value={newMemberRole}
                      onChange={(e) => setNewMemberRole(e.target.value as any)}
                      className="w-full px-3 py-1.5 rounded-lg border border-slate-300 dark:border-slate-700 bg-white dark:bg-slate-800 text-slate-900 dark:text-white"
                    >
                      <option value="viewer">Viewer (Read-only)</option>
                      <option value="editor">Editor (Read &amp; Write records)</option>
                      <option value="admin">Admin (Manage settings)</option>
                      <option value="owner">Owner (Full sovereignty)</option>
                    </select>
                  </div>
                </div>
                <div className="flex justify-end pt-1">
                  <button
                    type="submit"
                    data-testid="ws-add-member-btn"
                    className="px-3.5 py-1.5 bg-blue-600 hover:bg-blue-700 text-white font-semibold rounded-lg text-xs cursor-pointer transition-colors"
                  >
                    + Add Collaborator
                  </button>
                </div>
              </form>

              {/* Members Table */}
              <div>
                <div className="font-bold text-slate-800 dark:text-slate-200 mb-2">
                  Active Members &amp; Collaborator Roles ({collaborators.length})
                </div>
                <div className="border border-slate-200 dark:border-slate-800 rounded-lg overflow-hidden">
                  <table className="w-full text-left border-collapse">
                    <thead>
                      <tr className="bg-slate-50 dark:bg-slate-800/80 border-b border-slate-200 dark:border-slate-800 text-slate-500 uppercase text-[10px]">
                        <th className="py-2.5 px-3">Collaborator</th>
                        <th className="py-2.5 px-3">Department</th>
                        <th className="py-2.5 px-3">Role</th>
                        <th className="py-2.5 px-3 text-right">Actions</th>
                      </tr>
                    </thead>
                    <tbody className="divide-y divide-slate-100 dark:divide-slate-800">
                      {collaborators.map((c) => (
                        <tr key={c.eppn} className="hover:bg-slate-50/50 dark:hover:bg-slate-800/30">
                          <td className="py-2.5 px-3">
                            <div className="font-semibold text-slate-900 dark:text-white">{c.name}</div>
                            <div className="font-mono text-[10px] text-slate-400">{c.eppn}</div>
                          </td>
                          <td className="py-2.5 px-3 text-slate-600 dark:text-slate-400">{c.department}</td>
                          <td className="py-2.5 px-3">
                            <select
                              data-testid={`ws-member-role-select-${c.eppn}`}
                              value={c.role}
                              onChange={(e) => handleUpdateRole(c.eppn, e.target.value as any)}
                              className="px-2 py-1 rounded border border-slate-300 dark:border-slate-700 bg-white dark:bg-slate-800 text-slate-900 dark:text-white font-medium"
                            >
                              <option value="owner">Owner</option>
                              <option value="admin">Admin</option>
                              <option value="editor">Editor</option>
                              <option value="viewer">Viewer</option>
                            </select>
                          </td>
                          <td className="py-2.5 px-3 text-right">
                            <button
                              type="button"
                              data-testid={`ws-member-remove-btn-${c.eppn}`}
                              onClick={() => handleRemoveMember(c.eppn)}
                              className="px-2.5 py-1 text-rose-600 hover:text-rose-700 hover:bg-rose-50 dark:hover:bg-rose-950/40 rounded transition-colors"
                            >
                              Remove
                            </button>
                          </td>
                        </tr>
                      ))}
                    </tbody>
                  </table>
                </div>
              </div>
            </div>
          )}

          {tab === "audit" && (
            <div className="space-y-3">
              <div className="font-bold text-slate-800 dark:text-slate-200">
                Cryptographic Decision Ledger for {workspace.name} (OSCAL AC-02, AC-03)
              </div>
              {workspaceLedger.length === 0 ? (
                <div className="p-8 text-center text-slate-400 bg-slate-50 dark:bg-slate-800/30 rounded-lg">
                  No cryptographic decision ledger entries recorded yet for this workspace.
                </div>
              ) : (
                <div className="space-y-2">
                  {workspaceLedger.map((entry) => (
                    <div
                      key={entry.entry_hash}
                      className="p-3 rounded-lg border border-slate-200 dark:border-slate-800 bg-slate-50 dark:bg-slate-800/40 font-mono text-[11px] space-y-1"
                    >
                      <div className="flex items-center justify-between">
                        <span className="font-bold text-blue-600 dark:text-blue-400">{entry.decision_type}</span>
                        <span className="text-[10px] text-slate-400">{entry.timestamp_iso}</span>
                      </div>
                      <div className="text-slate-700 dark:text-slate-300">{entry.rationale}</div>
                      <div className="text-[10px] text-slate-500 truncate">
                        Block SHA-256: <span className="text-emerald-600 dark:text-emerald-400">{entry.entry_hash}</span>
                      </div>
                    </div>
                  ))}
                </div>
              )}
            </div>
          )}
        </div>

        {/* Footer */}
        <div className="px-6 py-3 border-t border-slate-200 dark:border-slate-800 bg-slate-50/50 dark:bg-slate-950/40 flex justify-between items-center text-xs">
          <span className="text-slate-500 font-mono text-[11px]">
            Governance Standard: NIST OSCAL 1.1.2 &middot; Cedar ABAC
          </span>
          <button
            type="button"
            data-testid="ws-settings-done-btn"
            onClick={onClose}
            className="px-4 py-1.5 bg-slate-200 dark:bg-slate-700 hover:bg-slate-300 dark:hover:bg-slate-600 text-slate-800 dark:text-slate-200 font-medium rounded-lg transition-colors"
          >
            Close
          </button>
        </div>
      </div>
    </div>
  );
};
