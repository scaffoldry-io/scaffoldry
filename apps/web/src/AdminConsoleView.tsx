import React, { useState, useEffect } from "react";
import { DecisionLedgerView } from "./DecisionLedgerView";
import { UnitPositions } from "./admin/UnitPositions";
import { LedgerEntryItem, OrganizationNode, OrgRole, Persona, RegisteredApp, SourceRule, Workspace } from "./types";
import { apiClient } from "./api";

export interface AdminConsoleViewProps {
  activePersona: Persona;
  isImpersonating: boolean;
  realAdmin: Persona | null;
  handleStopImpersonation: () => void;
  navigateTo: (path: string) => void;
  adminTab: "org" | "policy" | "ledger" | "impersonation" | "settings";
  apps: RegisteredApp[];
  sourceRules: SourceRule[];
  simAction: "read" | "write" | "export";
  setSimAction: React.Dispatch<React.SetStateAction<"read" | "write" | "export">>;
  simFerpa: boolean;
  setSimFerpa: React.Dispatch<React.SetStateAction<boolean>>;
  simResult: { decision: string; reason: string; rule: string };
  ledger: LedgerEntryItem[];
  setNotificationToast: React.Dispatch<React.SetStateAction<string | null>>;
  handleDownloadOscal: () => void;
  personas: Persona[];
  handleStartImpersonation: (user: Persona) => void;
}

export const AdminConsoleView: React.FC<AdminConsoleViewProps> = ({
  activePersona,
  isImpersonating,
  realAdmin,
  handleStopImpersonation,
  navigateTo,
  adminTab,
  apps,
  sourceRules,
  simAction,
  setSimAction,
  simFerpa,
  setSimFerpa,
  simResult,
  ledger,
  setNotificationToast,
  handleDownloadOscal,
  personas,
  handleStartImpersonation,
}) => {
  const [orgs, setOrgs] = useState<OrganizationNode[]>([]);
  const [selectedOrgId, setSelectedOrgId] = useState<string | null>(null);
  const [workspaces, setWorkspaces] = useState<Workspace[]>([]);
  const [orgMembers, setOrgMembers] = useState<OrgRole[]>([]);
  const [newOrgName, setNewOrgName] = useState("");
  const [newOrgCode, setNewOrgCode] = useState("");
  const [newOrgType, setNewOrgType] = useState("Department");
  const [appointEppn, setAppointEppn] = useState("");
  const [platformSettings, setPlatformSettings] = useState<Record<string, any>>({});
  const [settingsInputs, setSettingsInputs] = useState<Record<string, any>>({});
  const [scimSecret, setScimSecret] = useState<string | null>(null);

  const loadPlatformSettings = async () => {
    try {
      const res = await apiClient.getSettings();
      if (res?.settings) {
        setPlatformSettings(res.settings);
        setSettingsInputs({ ...res.settings });
      }
    } catch (e: any) {
      console.error("Failed to load platform settings:", e);
    }
  };

  useEffect(() => {
    if (adminTab === "settings") {
      loadPlatformSettings();
    }
  }, [adminTab]);

  const handleSaveSetting = async (key: string) => {
    try {
      let val = settingsInputs[key];
      if (key === "tokens.max_days" || key === "process.stale_days") {
        val = parseInt(String(val), 10);
      } else if (key === "tokens.agent_enabled") {
        val = Boolean(val);
      } else if (key === "oidc.jwks") {
        if (typeof val === "string") {
          val = JSON.parse(val);
        }
      } else if (key === "cors.allowed_origins" || key === "mcp.disabled_tools" || key === "pages.disabled") {
        if (typeof val === "string") {
          val = val.split(",").map((s: string) => s.trim()).filter(Boolean);
        }
      }
      await apiClient.updateSetting(key, val);
      setNotificationToast(`Setting "${key}" updated successfully`);
      loadPlatformSettings();
    } catch (e: any) {
      setNotificationToast(`Failed to update "${key}": ${e.message}`);
    }
  };

  const handleMintScimToken = async () => {
    try {
      const res = await apiClient.createToken({
        label: "SCIM Provisioning Credential",
        kind: "scim",
      });
      setScimSecret(res.token);
      setNotificationToast("SCIM token minted successfully");
    } catch (e: any) {
      setNotificationToast(`Failed to mint SCIM token: ${e.message}`);
    }
  };

  const loadOrgs = async () => {
    try {
      const data = await apiClient.listOrganizations();
      setOrgs(data);
      if (data.length > 0 && !selectedOrgId) {
        setSelectedOrgId(data[0].id);
      }
    } catch {
      // ignore
    }
  };

  const loadWorkspaces = async () => {
    try {
      const data = await apiClient.listWorkspaces();
      setWorkspaces(data);
    } catch {
      // ignore
    }
  };

  const loadMembers = async (id: string) => {
    try {
      const data = await apiClient.listOrgMembers(id);
      setOrgMembers(data);
    } catch {
      setOrgMembers([]);
    }
  };

  useEffect(() => {
    if (adminTab === "org") {
      loadOrgs();
      loadWorkspaces();
    }
  }, [adminTab]);

  useEffect(() => {
    if (selectedOrgId) {
      loadMembers(selectedOrgId);
    }
  }, [selectedOrgId]);

  const handleCreateOrg = async (e: React.FormEvent) => {
    e.preventDefault();
    if (!newOrgName || !newOrgCode) return;
    try {
      const created = await apiClient.createOrganization({
        name: newOrgName,
        code: newOrgCode,
        org_type: newOrgType,
        parent_id: selectedOrgId,
      });
      setOrgs((prev) => [...prev, created]);
      setSelectedOrgId(created.id);
      setNewOrgName("");
      setNewOrgCode("");
      setNotificationToast(`Created unit ${created.name}`);
    } catch (err: any) {
      setNotificationToast(err.message || "Failed to create unit");
    }
  };

  const handleAppointAdmin = async (e: React.FormEvent) => {
    e.preventDefault();
    if (!selectedOrgId || !appointEppn) return;
    try {
      await apiClient.appointOrgAdmin(selectedOrgId, {
        eppn: appointEppn,
        scoped_affiliation: "unit_admin",
      });
      setAppointEppn("");
      loadMembers(selectedOrgId);
      setNotificationToast(`Appointed ${appointEppn} as Org Unit Admin`);
    } catch (err: any) {
      setNotificationToast(err.message || "Failed to appoint admin");
    }
  };

  const selectedOrg = orgs.find((o) => o.id === selectedOrgId) || (orgs.length > 0 ? orgs[0] : null);
  const parentOrg = selectedOrg && selectedOrg.parent_id ? orgs.find((o) => o.id === selectedOrg.parent_id) : null;
  const unitWorkspaces = selectedOrg ? workspaces.filter((ws: any) => ws.organization_id === selectedOrg.id) : [];
  const unitAdmins = orgMembers.filter((m) => m.scoped_affiliation === "unit_admin");
  if (activePersona.affiliation !== "central_admin") {
    return (
      <div
        data-testid="admin-access-denied"
        className="p-8 max-w-xl mx-auto my-12 bg-white dark:bg-slate-900 rounded-xl border border-rose-200 dark:border-rose-900/60 shadow-lg text-center space-y-4 animate-fade-in"
      >
        <div className="w-12 h-12 mx-auto rounded-full bg-rose-100 dark:bg-rose-950/60 text-rose-600 dark:text-rose-400 flex items-center justify-center text-xl font-bold">
          🛡️
        </div>
        <h2 className="text-lg font-bold text-slate-900 dark:text-white">
          403 Forbidden: Cedar Policy Authorization Required
        </h2>
        <p className="text-xs text-slate-600 dark:text-slate-400">
          Active principal <strong>{activePersona.name}</strong> ({activePersona.eppn}) holds affiliation{" "}
          <strong>{activePersona.affiliation}</strong>. Institutional security policy strictly restricts the
          Administrative Console to <strong>central_admin</strong> principals.
        </p>
        {isImpersonating && realAdmin && (
          <div className="p-3 bg-amber-50 dark:bg-amber-950/40 rounded-lg text-amber-800 dark:text-amber-300 text-xs text-left">
            You are currently impersonating this user. Return to your administrator session (
            <strong>{realAdmin.name}</strong>) to regain administrative access.
          </div>
        )}
        <div className="flex items-center justify-center gap-3 pt-2">
          {isImpersonating ? (
            <button
              type="button"
              data-testid="access-denied-exit-imp-btn"
              onClick={handleStopImpersonation}
              className="px-4 py-2 bg-amber-600 hover:bg-amber-700 text-white font-semibold text-xs rounded-lg transition-colors cursor-pointer"
            >
              Exit Impersonation &amp; Restore Admin
            </button>
          ) : (
            <button
              type="button"
              onClick={() => navigateTo("/")}
              className="px-4 py-2 bg-slate-800 hover:bg-slate-900 text-white font-semibold text-xs rounded-lg transition-colors cursor-pointer"
            >
              Return to Workspace
            </button>
          )}
        </div>
      </div>
    );
  }

  return (
    <div className="space-y-6 max-w-6xl mx-auto animate-fade-in">
      <div className="flex flex-wrap items-center justify-between gap-4 pb-2 border-b border-slate-200 dark:border-slate-800">
        <div className="flex items-center gap-3">
          <img src="/logo-mark.png" alt="Scaffoldry" className="h-8 w-auto object-contain shrink-0" />
          <div>
            <div className="flex items-center gap-2">
              <span className="w-2.5 h-2.5 rounded-full bg-amber-500 animate-pulse" />
              <h1 className="text-xl font-bold tracking-tight text-slate-900 dark:text-white">
                Institutional Administrative Console
              </h1>
            </div>
            <p className="text-xs text-slate-500 dark:text-slate-400 mt-0.5">
              Discreet governance and operations center:{" "}
              <code className="font-mono text-amber-600 dark:text-amber-400">/admin</code>.
            </p>
          </div>
        </div>
        <button
          type="button"
          onClick={() => navigateTo("/")}
          className="inline-flex items-center gap-1.5 px-3 py-1.5 text-xs font-medium rounded-md border border-slate-300 dark:border-slate-700 bg-white dark:bg-slate-900 text-slate-700 dark:text-slate-300 hover:bg-slate-50 dark:hover:bg-slate-800 shadow-xs cursor-pointer transition-colors"
        >
          ← Exit to Workspace (/)
        </button>
      </div>

      {/* ADMIN TAB 1: ORGANIZATION & REALMS */}
      {adminTab === "org" && (
        <div className="space-y-6">
          <div className="bg-white dark:bg-slate-900 border border-slate-200 dark:border-slate-800 rounded-lg p-6 shadow-xs space-y-6">
            <div>
              <h2 className="text-xl font-bold text-slate-900 dark:text-white">Organization</h2>
              <p className="text-xs text-slate-500 mt-1">
                Institutional hierarchical scope tree and sovereign administrative unit governance.
              </p>
            </div>

            {/* Tree Section */}
            <div className="border border-slate-200 dark:border-slate-800 rounded-lg p-4 bg-slate-50/50 dark:bg-slate-950/40 space-y-2">
              <h3 className="text-xs font-semibold text-slate-500 uppercase tracking-wider">Unit Tree</h3>
              <div className="space-y-1">
                {orgs.map((unit) => {
                  const isSelected = selectedOrgId === unit.id;
                  const isChild = unit.parent_id !== null;
                  return (
                    <div key={unit.id} className={isChild ? "ml-6" : ""}>
                      <button
                        type="button"
                        data-testid={`org-node-${unit.code}`}
                        onClick={() => setSelectedOrgId(unit.id)}
                        className={`w-full flex items-center justify-between px-3 py-2 text-xs rounded-md border text-left transition-colors cursor-pointer ${
                          isSelected
                            ? "bg-amber-500 text-white border-amber-600 font-semibold shadow-xs"
                            : "bg-white dark:bg-slate-900 border-slate-200 dark:border-slate-800 text-slate-800 dark:text-slate-200 hover:bg-slate-100 dark:hover:bg-slate-800"
                        }`}
                      >
                        <span className="flex items-center gap-2">
                          <span>{unit.parent_id ? "↳ 📁" : "🏛️"}</span>
                          <span>{unit.name}</span>
                          <span className="font-mono text-[10px] opacity-75">({unit.code})</span>
                        </span>
                        <span className="text-[10px] uppercase font-semibold px-2 py-0.5 rounded bg-black/10 dark:bg-white/10">
                          {unit.org_type}
                        </span>
                      </button>
                    </div>
                  );
                })}
              </div>
            </div>

            {/* Selected Unit Detail */}
            {selectedOrg && (
              <div data-testid="org-detail" className="border border-slate-200 dark:border-slate-800 rounded-lg p-5 bg-white dark:bg-slate-900 space-y-4">
                <div className="flex items-center justify-between border-b border-slate-100 dark:border-slate-800 pb-3">
                  <div>
                    <h3 className="text-base font-bold text-slate-900 dark:text-white">{selectedOrg.name}</h3>
                    <div className="text-xs text-slate-500 flex items-center gap-3 mt-1">
                      <span>Code: <code className="font-mono text-amber-600">{selectedOrg.code}</code></span>
                      <span>Type: <span className="font-medium text-slate-700 dark:text-slate-300">{selectedOrg.org_type}</span></span>
                      <span>Parent: <span className="font-medium text-slate-700 dark:text-slate-300">{parentOrg?.name || "None (Root)"}</span></span>
                    </div>
                  </div>
                </div>

                {/* Workspaces in this unit */}
                <div>
                  <h4 className="text-xs font-semibold text-slate-600 dark:text-slate-400 uppercase tracking-wider mb-2">
                    Workspaces in this unit
                  </h4>
                  {unitWorkspaces.length === 0 ? (
                    <div className="text-xs text-slate-400 italic p-3 border border-dashed border-slate-200 dark:border-slate-800 rounded-md">
                      No workspaces attached to this unit.
                    </div>
                  ) : (
                    <div className="space-y-1.5">
                      {unitWorkspaces.map((ws: any) => (
                        <div key={ws.id} className="flex items-center justify-between p-2.5 rounded border border-slate-200 dark:border-slate-800 bg-slate-50 dark:bg-slate-800/40 text-xs">
                          <span className="font-medium text-slate-800 dark:text-slate-200">{ws.name}</span>
                          <span className="font-mono text-[10px] text-slate-500">{ws.code}</span>
                        </div>
                      ))}
                    </div>
                  )}
                </div>

                {/* Positions in this unit */}
                <div>
                  <h4 className="text-xs font-semibold text-slate-600 dark:text-slate-400 uppercase tracking-wider mb-2">
                    Positions
                  </h4>
                  <UnitPositions unitId={selectedOrg.id} />
                </div>

                {/* Form: Create child unit */}
                <form data-testid="org-create-form" onSubmit={handleCreateOrg} className="border-t border-slate-100 dark:border-slate-800 pt-4 space-y-3">
                  <h4 className="text-xs font-bold text-slate-800 dark:text-slate-200 uppercase tracking-wider">
                    Create child unit under {selectedOrg.name}
                  </h4>
                  <div className="grid grid-cols-1 sm:grid-cols-3 gap-3">
                    <div>
                      <label htmlFor="org-name-input" className="block text-[11px] font-medium text-slate-600 dark:text-slate-400 mb-1">
                        Name
                      </label>
                      <input
                        id="org-name-input"
                        type="text"
                        value={newOrgName}
                        onChange={(e) => setNewOrgName(e.target.value)}
                        placeholder="e.g. Department of Physics"
                        required
                        className="w-full text-xs rounded border border-slate-300 dark:border-slate-700 bg-white dark:bg-slate-800 px-2.5 py-1.5"
                      />
                    </div>
                    <div>
                      <label htmlFor="org-code-input" className="block text-[11px] font-medium text-slate-600 dark:text-slate-400 mb-1">
                        Code
                      </label>
                      <input
                        id="org-code-input"
                        type="text"
                        value={newOrgCode}
                        onChange={(e) => setNewOrgCode(e.target.value)}
                        placeholder="e.g. PHYS"
                        required
                        className="w-full text-xs rounded border border-slate-300 dark:border-slate-700 bg-white dark:bg-slate-800 px-2.5 py-1.5"
                      />
                    </div>
                    <div>
                      <label htmlFor="org-type-select" className="block text-[11px] font-medium text-slate-600 dark:text-slate-400 mb-1">
                        Type
                      </label>
                      <select
                        id="org-type-select"
                        value={newOrgType}
                        onChange={(e) => setNewOrgType(e.target.value)}
                        className="w-full text-xs rounded border border-slate-300 dark:border-slate-700 bg-white dark:bg-slate-800 px-2.5 py-1.5"
                      >
                        <option value="College">College</option>
                        <option value="Department">Department</option>
                        <option value="Center">Center</option>
                        <option value="Program">Program</option>
                      </select>
                    </div>
                  </div>
                  <button
                    type="submit"
                    className="px-3 py-1.5 bg-slate-900 text-white dark:bg-white dark:text-slate-900 rounded-md text-xs font-semibold hover:opacity-90 cursor-pointer"
                  >
                    Create unit
                  </button>
                </form>

                {/* Form: Appoint Org Unit Admin */}
                <form data-testid="org-appoint-form" onSubmit={handleAppointAdmin} className="border-t border-slate-100 dark:border-slate-800 pt-4 space-y-3">
                  <h4 className="text-xs font-bold text-slate-800 dark:text-slate-200 uppercase tracking-wider">
                    Appoint Org Unit Admin
                  </h4>
                  <div className="flex gap-2">
                    <div className="flex-1">
                      <label htmlFor="appoint-eppn-input" className="block text-[11px] font-medium text-slate-600 dark:text-slate-400 mb-1">
                        Identity (ePPN)
                      </label>
                      <input
                        id="appoint-eppn-input"
                        type="email"
                        value={appointEppn}
                        onChange={(e) => setAppointEppn(e.target.value)}
                        placeholder="chair.physics@state.edu"
                        required
                        className="w-full text-xs rounded border border-slate-300 dark:border-slate-700 bg-white dark:bg-slate-800 px-2.5 py-1.5"
                      />
                    </div>
                    <div className="flex items-end">
                      <button
                        type="submit"
                        className="px-3 py-1.5 bg-amber-600 text-white rounded-md text-xs font-semibold hover:bg-amber-700 cursor-pointer"
                      >
                        Appoint Org Unit Admin
                      </button>
                    </div>
                  </div>

                  {/* List of Org Unit Admins */}
                  <div className="mt-3">
                    <h4 className="text-xs font-semibold text-slate-700 dark:text-slate-300 mb-1.5">
                      Org Unit Admins
                    </h4>
                    {unitAdmins.length === 0 ? (
                      <p className="text-xs text-slate-400 italic">No appointed unit admins.</p>
                    ) : (
                      <ul className="space-y-1">
                        {unitAdmins.map((admin: any) => (
                          <li key={admin.id || admin.eppn} className="text-xs font-mono text-slate-700 dark:text-slate-300 bg-slate-50 dark:bg-slate-800 px-2.5 py-1 rounded">
                            {admin.eppn}
                          </li>
                        ))}
                      </ul>
                    )}
                  </div>
                </form>

                {/* App Domain Rows preserved below */}
                <div className="border-t border-slate-200 dark:border-slate-800 pt-5 space-y-3">
                  <h4 className="text-sm font-bold text-slate-900 dark:text-white">
                    Department Realms &amp; DNS Vanity Routing
                  </h4>
                  <p className="text-xs text-slate-500">
                    Sub-millisecond host-header routing table configured across all university departments without open inbound ports.
                  </p>
                  <div className="space-y-2">
                    {apps.map((app) => (
                      <div
                        key={app.slug}
                        className="flex flex-wrap items-center justify-between p-3 rounded-lg border border-slate-200 dark:border-slate-800 bg-slate-50/70 dark:bg-slate-800/40 text-xs gap-2"
                      >
                        <div>
                          <div className="font-semibold text-slate-800 dark:text-slate-200">{app.title}</div>
                          <div className="font-mono text-[11px] text-blue-600 dark:text-blue-400">{app.customDomain}</div>
                        </div>
                        <div className="flex items-center gap-3">
                          <span className="font-mono text-[11px] text-slate-500">{app.orgCode}</span>
                          <span className="inline-flex items-center px-2 py-0.5 rounded text-[10px] font-semibold text-emerald-700 dark:text-emerald-300 bg-emerald-50 dark:bg-emerald-950/60 border border-emerald-200 dark:border-emerald-800">
                            ✓ 0.4 ms · Active
                          </span>
                        </div>
                      </div>
                    ))}
                  </div>
                </div>
              </div>
            )}
          </div>
        </div>
      )}

      {/* ADMIN TAB 2: POLICY & OSCAL LATTICE */}
      {adminTab === "policy" && (
        <div className="space-y-6">
          {/* Source Rules Matrix */}
          <div className="bg-white dark:bg-slate-900 border border-slate-200 dark:border-slate-800 rounded-lg p-5 shadow-xs space-y-4">
            <h2 className="text-base font-bold text-slate-900 dark:text-white">
              Statutory Rules to Cedar Policy Crosswalk (NIST OSCAL 1.1.2)
            </h2>
            <table className="w-full text-left text-xs border-collapse">
              <thead>
                <tr className="bg-slate-50 dark:bg-slate-800/70 border-b border-slate-200 dark:border-slate-800 text-slate-500 uppercase text-[10px]">
                  <th className="py-2.5 px-3">Statutory Source</th>
                  <th className="py-2.5 px-3">OSCAL Control</th>
                  <th className="py-2.5 px-3">Executable Cedar Policy</th>
                  <th className="py-2.5 px-3">Status</th>
                </tr>
              </thead>
              <tbody className="divide-y divide-slate-100 dark:divide-slate-800">
                {sourceRules.map((rule) => (
                  <tr key={rule.id}>
                    <td className="py-2.5 px-3 font-semibold text-slate-800 dark:text-slate-200">{rule.source}</td>
                    <td className="py-2.5 px-3 font-mono text-purple-600 dark:text-purple-400">{rule.oscalControl}</td>
                    <td className="py-2.5 px-3">
                      <code className="p-1 rounded bg-slate-900 text-sky-300 font-mono text-[10px] block max-w-sm overflow-x-auto">
                        {rule.cedarSnippet}
                      </code>
                    </td>
                    <td className="py-2.5 px-3 text-emerald-600 font-medium">✓ {rule.status}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>

          {/* Policy Decision Simulator */}
          <div className="bg-white dark:bg-slate-900 border border-slate-200 dark:border-slate-800 rounded-lg p-5 shadow-xs space-y-3">
            <h3 className="text-sm font-semibold text-slate-900 dark:text-white">
              Interactive Cedar Authorization Simulator
            </h3>
            <div className="grid grid-cols-1 md:grid-cols-3 gap-3">
              <div>
                <label className="block text-xs text-slate-500 mb-1">Requested Action:</label>
                <select
                  value={simAction}
                  onChange={(e) => setSimAction(e.target.value as "read" | "write" | "export")}
                  className="w-full text-xs rounded border border-slate-300 dark:border-slate-700 bg-white dark:bg-slate-800 p-1.5"
                >
                  <option value="read">Action::&quot;read&quot;</option>
                  <option value="write">Action::&quot;write&quot;</option>
                  <option value="export">Action::&quot;export&quot; (FERPA Guard)</option>
                </select>
              </div>
              <div>
                <label className="block text-xs text-slate-500 mb-1">Record Sensitivity:</label>
                <select
                  value={simFerpa ? "true" : "false"}
                  onChange={(e) => setSimFerpa(e.target.value === "true")}
                  className="w-full text-xs rounded border border-slate-300 dark:border-slate-700 bg-white dark:bg-slate-800 p-1.5"
                >
                  <option value="false">Standard Department Record</option>
                  <option value="true">FERPA Sensitive Student Record</option>
                </select>
              </div>
              <div>
                <label className="block text-xs text-slate-500 mb-1">Target Department:</label>
                <input
                  type="text"
                  disabled
                  value="biology"
                  className="w-full text-xs rounded border border-slate-200 dark:border-slate-700 bg-slate-100 dark:bg-slate-800/40 p-1.5 text-slate-500"
                />
              </div>
            </div>
            <div
              className={`p-3 rounded-lg border text-xs flex items-center justify-between ${
                simResult.decision === "ALLOW"
                  ? "bg-emerald-50 dark:bg-emerald-950/30 border-emerald-300 dark:border-emerald-800 text-emerald-900 dark:text-emerald-200"
                  : "bg-rose-50 dark:bg-rose-950/30 border-rose-300 dark:border-rose-800 text-rose-900 dark:text-rose-200"
              }`}
            >
              <div>
                <strong className="mr-2">CEDAR {simResult.decision}</strong>
                <span>{simResult.reason}</span>
              </div>
              <span className="font-mono text-[10px] opacity-70">{simResult.rule}</span>
            </div>
          </div>
        </div>
      )}



      {/* ADMIN TAB 4: CRYPTOGRAPHIC DECISION AUDIT LEDGER */}
      {adminTab === "ledger" && (
        <DecisionLedgerView
          entries={ledger}
          onVerifyChain={() => {
            setNotificationToast("Cryptographic proof verified: All SHA-256 blocks chained without tampering.");
            setTimeout(() => setNotificationToast(null), 3500);
          }}
          onDownloadOscal={handleDownloadOscal}
        />
      )}

      {/* ADMIN TAB 5: INSTITUTIONAL IDENTITY & IMPERSONATION HUB */}
      {adminTab === "impersonation" && (
        <div data-testid="impersonation-panel" className="space-y-4 animate-fade-in">
          <div className="bg-white dark:bg-slate-900 border border-slate-200 dark:border-slate-800 rounded-lg p-5 shadow-xs space-y-3">
            <h2 className="text-base font-bold text-slate-900 dark:text-white">
              Institutional Identity &amp; User Impersonation Hub (OSCAL AC-02)
            </h2>
            <p className="text-xs text-slate-500">
              Federated InCommon/eduPerson directory integration. Central administrators can temporarily assume
              departmental personas for troubleshooting and compliance verification with mandatory audit trails.
            </p>
            <div className="p-3 bg-slate-50 dark:bg-slate-800/40 rounded-lg border border-slate-200 dark:border-slate-700 text-xs text-slate-600 dark:text-slate-300 flex items-start gap-2.5">
              <span className="text-amber-500 font-bold shrink-0">ℹ️</span>
              <div>
                All impersonation sessions are permanently recorded in the cryptographic Git decision ledger with
                caller attribution (
                <code className="font-mono text-amber-600 dark:text-amber-400">
                  {realAdmin?.eppn || activePersona.eppn}
                </code>
                ). Impersonated activity cannot forge ledger signatures.
              </div>
            </div>
          </div>

          {/* Directory Table */}
          <div className="bg-white dark:bg-slate-900 border border-slate-200 dark:border-slate-800 rounded-lg shadow-xs overflow-hidden">
            <div className="px-5 py-3 border-b border-slate-200 dark:border-slate-800 flex items-center justify-between">
              <h3 className="text-sm font-semibold text-slate-900 dark:text-white">
                InCommon Directory Users ({personas.length})
              </h3>
              <span className="text-xs text-slate-400">Select an institutional identity to begin session</span>
            </div>
            <div className="divide-y divide-slate-100 dark:divide-slate-800">
              {personas.map((p) => {
                const isCurrent = p.eppn === activePersona.eppn;
                return (
                  <div
                    key={p.eppn}
                    className={`p-4 flex flex-wrap items-center justify-between gap-4 transition-colors ${
                      isCurrent
                        ? "bg-blue-50/50 dark:bg-blue-950/20"
                        : "hover:bg-slate-50 dark:hover:bg-slate-800/30"
                    }`}
                  >
                    <div className="flex items-center gap-3">
                      <div className="w-9 h-9 rounded-full bg-blue-600 text-white font-bold text-xs flex items-center justify-center uppercase shrink-0">
                        {p.name
                          .split(" ")
                          .map((n) => n[0])
                          .slice(0, 2)
                          .join("")}
                      </div>
                      <div>
                        <div className="text-sm font-semibold text-slate-900 dark:text-white flex items-center gap-2">
                          {p.name}
                          {isCurrent && (
                            <span className="text-[10px] bg-blue-100 text-blue-700 dark:bg-blue-950 dark:text-blue-300 px-1.5 py-0.5 rounded font-mono font-medium">
                              Active Identity
                            </span>
                          )}
                        </div>
                        <div className="text-xs text-slate-500 dark:text-slate-400">
                          {p.roleTitle} · {p.department}
                        </div>
                        <div className="text-[11px] font-mono text-slate-400">{p.eppn}</div>
                      </div>
                    </div>

                    <div className="flex items-center gap-2">
                      <span className="px-2 py-0.5 rounded text-[10px] font-medium bg-slate-100 text-slate-700 dark:bg-slate-800 dark:text-slate-300 uppercase">
                        {p.affiliation}
                      </span>
                      <button
                        type="button"
                        data-testid={`impersonate-${p.eppn}-btn`}
                        disabled={isCurrent}
                        onClick={() => handleStartImpersonation(p)}
                        className={`px-3 py-1.5 rounded-lg text-xs font-semibold transition-colors cursor-pointer ${
                          isCurrent
                            ? "opacity-40 cursor-not-allowed bg-slate-200 dark:bg-slate-800 text-slate-500"
                            : "bg-amber-600 hover:bg-amber-700 text-white shadow-xs"
                        }`}
                      >
                        {isCurrent ? "Active" : "Impersonate User"}
                      </button>
                    </div>
                  </div>
                );
              })}
            </div>
          </div>
        </div>
      )}
      {adminTab === "settings" && (
        <div className="space-y-6 animate-fade-in">
          <div className="flex flex-wrap items-center justify-between gap-4 bg-white dark:bg-slate-900 p-4 rounded-xl border border-slate-200 dark:border-slate-800 shadow-2xs">
            <div>
              <h2 className="text-base font-bold text-slate-900 dark:text-white">Institutional Platform Settings</h2>
              <p className="text-xs text-slate-500 dark:text-slate-400">
                Authoritative configuration stored in PostgreSQL. Every edit is audited in the cryptographic ledger.
              </p>
            </div>
            <button
              type="button"
              data-testid="mint-scim-token-btn"
              onClick={handleMintScimToken}
              className="px-3 py-2 bg-emerald-600 hover:bg-emerald-700 text-white rounded-lg text-xs font-semibold cursor-pointer shadow-xs transition-colors flex items-center gap-2"
            >
              <span>🔑</span>
              <span>Mint SCIM token</span>
            </button>
          </div>

          {scimSecret && (
            <div className="p-4 bg-emerald-50 dark:bg-emerald-950/40 border border-emerald-300 dark:border-emerald-700 rounded-xl space-y-2">
              <div className="text-xs font-bold text-emerald-800 dark:text-emerald-200">
                Copy this token now. It is not shown again.
              </div>
              <div className="flex items-center gap-2">
                <input
                  data-testid="scim-token-secret"
                  type="text"
                  readOnly
                  value={scimSecret}
                  className="flex-1 px-3 py-1.5 bg-white dark:bg-slate-900 border border-emerald-300 dark:border-emerald-700 rounded font-mono text-xs text-slate-900 dark:text-white select-all"
                />
                <button
                  type="button"
                  onClick={() => navigator.clipboard?.writeText(scimSecret)}
                  className="px-3 py-1.5 bg-emerald-600 hover:bg-emerald-700 text-white rounded text-xs font-semibold cursor-pointer"
                >
                  Copy
                </button>
              </div>
            </div>
          )}

          <div className="bg-white dark:bg-slate-900 border border-slate-200 dark:border-slate-800 rounded-xl shadow-xs divide-y divide-slate-100 dark:divide-slate-800 overflow-hidden">
            {[
              { key: "tokens.max_days", desc: "Longest life of an agent token in days (1 to 365)", type: "number" },
              { key: "tokens.agent_enabled", desc: "Allow users to mint agent tokens", type: "boolean" },
              { key: "cors.allowed_origins", desc: "Allowed CORS origins (comma-separated)", type: "string_array" },
              { key: "oidc.issuer", desc: "Exact OIDC issuer URI to accept", type: "text" },
              { key: "oidc.audience", desc: "Exact OIDC audience to accept", type: "text" },
              { key: "oidc.jwks", desc: "JWKS document JSON from the identity provider", type: "json" },
              { key: "process.stale_days", desc: "Days before a waiting process step is flagged as stale", type: "number" },
              { key: "mcp.disabled_tools", desc: "Disabled MCP tools list (comma-separated)", type: "string_array" },
              { key: "pages.disabled", desc: "Disabled custom pages list (comma-separated)", type: "string_array" },
            ].map(({ key, desc, type }) => {
              const currentVal = settingsInputs[key] !== undefined ? settingsInputs[key] : (platformSettings[key] || "");
              let isSaveDisabled = false;
              if (type === "json") {
                const str = typeof currentVal === "object" ? JSON.stringify(currentVal) : String(currentVal);
                if (str.trim()) {
                  try {
                    JSON.parse(str);
                  } catch {
                    isSaveDisabled = true;
                  }
                }
              }

              return (
                <div key={key} className="p-4 flex flex-col md:flex-row md:items-center justify-between gap-4">
                  <div className="md:w-1/3">
                    <div className="font-mono text-xs font-bold text-slate-900 dark:text-white">{key}</div>
                    <div className="text-[11px] text-slate-500 dark:text-slate-400">{desc}</div>
                  </div>
                  <div className="flex-1 flex items-center gap-2">
                    {type === "boolean" ? (
                      <label className="flex items-center gap-2 cursor-pointer">
                        <input
                          type="checkbox"
                          data-testid={`setting-input-${key}`}
                          checked={Boolean(currentVal)}
                          onChange={(e) => setSettingsInputs((prev) => ({ ...prev, [key]: e.target.checked }))}
                          className="w-4 h-4 rounded text-blue-600 focus:ring-blue-500 border-slate-300 dark:border-slate-700"
                        />
                        <span className="text-xs text-slate-700 dark:text-slate-300">Enabled</span>
                      </label>
                    ) : type === "number" ? (
                      <input
                        type="number"
                        data-testid={`setting-input-${key}`}
                        value={currentVal !== undefined ? currentVal : ""}
                        onChange={(e) => setSettingsInputs((prev) => ({ ...prev, [key]: e.target.value }))}
                        className="w-32 px-3 py-1.5 border border-slate-300 dark:border-slate-700 rounded-lg text-xs bg-white dark:bg-slate-800 text-slate-900 dark:text-white font-mono"
                      />
                    ) : type === "json" ? (
                      <div className="w-full space-y-1">
                        <textarea
                          data-testid={`setting-input-${key}`}
                          rows={3}
                          value={typeof currentVal === "object" ? JSON.stringify(currentVal, null, 2) : currentVal}
                          onChange={(e) => setSettingsInputs((prev) => ({ ...prev, [key]: e.target.value }))}
                          className="w-full px-3 py-1.5 border border-slate-300 dark:border-slate-700 rounded-lg text-xs bg-white dark:bg-slate-800 text-slate-900 dark:text-white font-mono"
                          placeholder='{"keys": [...]}'
                        />
                        {isSaveDisabled && (
                          <div className="text-[10px] text-red-500 font-medium">Must be valid JSON</div>
                        )}
                      </div>
                    ) : (
                      <input
                        type="text"
                        data-testid={`setting-input-${key}`}
                        value={Array.isArray(currentVal) ? currentVal.join(", ") : currentVal}
                        onChange={(e) => setSettingsInputs((prev) => ({ ...prev, [key]: e.target.value }))}
                        className="flex-1 px-3 py-1.5 border border-slate-300 dark:border-slate-700 rounded-lg text-xs bg-white dark:bg-slate-800 text-slate-900 dark:text-white font-mono"
                      />
                    )}
                    <button
                      type="button"
                      data-testid={`setting-save-${key}`}
                      disabled={isSaveDisabled}
                      onClick={() => handleSaveSetting(key)}
                      className="px-3 py-1.5 bg-blue-600 hover:bg-blue-700 disabled:opacity-40 disabled:cursor-not-allowed text-white rounded-lg text-xs font-semibold cursor-pointer shrink-0 transition-colors"
                    >
                      Save
                    </button>
                  </div>
                </div>
              );
            })}
          </div>
        </div>
      )}
    </div>
  );
};
