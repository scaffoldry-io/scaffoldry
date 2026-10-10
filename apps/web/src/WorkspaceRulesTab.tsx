import React, { useState, useEffect, useCallback, useMemo } from "react";
import { GuardRule, OrganizationNode, Persona, Workspace, WorkspaceGuardRecord } from "./types";
import { apiClient } from "./api";
import { ConfirmAction } from "./ui/ConfirmAction";
import { Drawer } from "./ui/Drawer";

interface WorkspaceRulesTabProps {
  workspace: Workspace;
  activePersona: Persona;
  allPersonas: Persona[];
}

const AFFILIATIONS = [
  { id: "faculty", label: "Faculty" },
  { id: "student", label: "Students" },
  { id: "staff", label: "Staff" },
  { id: "employee", label: "Employees" },
  { id: "member", label: "Members" },
  { id: "affiliate", label: "Affiliates" },
  { id: "alum", label: "Alumni" },
  { id: "compliance", label: "Compliance Officers" },
  { id: "central_admin", label: "Central Administrators" },
];

const TEMPLATES = [
  {
    id: "deny_export_unless_affiliation",
    title: "Export Restriction",
    description: "Only selected affiliations may export from this workspace.",
    type: "affiliation",
  },
  {
    id: "deny_write_for_affiliation",
    title: "Write Denial",
    description: "Selected affiliations may not change records in this workspace.",
    type: "affiliation",
  },
  {
    id: "deny_access_outside_units",
    title: "Unit Boundary",
    description: "Only people in selected units may use this workspace.",
    type: "unit",
  },
  {
    id: "deny_sensitive_unless_affiliation",
    title: "Sensitive Student Data Restriction",
    description: "Only selected affiliations may see or export sensitive student data here.",
    type: "affiliation",
  },
];

export const computeSentence = (
  template: string,
  affiliations: string[],
  unitIds: string[],
  unitMap: Record<string, string>
): string => {
  switch (template) {
    case "deny_export_unless_affiliation":
      return `Only ${affiliations.length > 0 ? affiliations.join(", ") : "no one"} may export from this workspace.`;
    case "deny_write_for_affiliation":
      return `${affiliations.length > 0 ? affiliations.join(", ") : "no one"} may not change records in this workspace.`;
    case "deny_access_outside_units": {
      const names = unitIds.map((id) => unitMap[id] || id);
      return `Only people in ${names.length > 0 ? names.join(", ") : "no units"} may use this workspace.`;
    }
    case "deny_sensitive_unless_affiliation":
      return `Only ${affiliations.length > 0 ? affiliations.join(", ") : "no one"} may see or export sensitive student data here.`;
    default:
      return "";
  }
};

export const WorkspaceRulesTab: React.FC<WorkspaceRulesTabProps> = ({
  workspace,
  activePersona,
  allPersonas,
}) => {
  const [currentGuard, setCurrentGuard] = useState<WorkspaceGuardRecord | null>(null);
  const [units, setUnits] = useState<OrganizationNode[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  // Advanced subtab (Platform Admin only)
  const [subtab, setSubtab] = useState<"rules" | "advanced">("rules");
  const [isRawSource, setIsRawSource] = useState(false);
  const [rawSource, setRawSource] = useState("");
  const [rawSourceErrors, setRawSourceErrors] = useState<string[]>([]);

  // Drawer state for adding rule
  const [isDrawerOpen, setIsDrawerOpen] = useState(false);
  const [selectedTemplate, setSelectedTemplate] = useState("deny_export_unless_affiliation");
  const [selectedAffiliations, setSelectedAffiliations] = useState<string[]>(["faculty"]);
  const [selectedUnitIds, setSelectedUnitIds] = useState<string[]>([]);
  const [unitSearch, setUnitSearch] = useState("");
  const [impactChanges, setImpactChanges] = useState<any[] | null>(null);
  const [impactLoading, setImpactLoading] = useState(false);

  // Removing rule state
  const [ruleToRemoveIndex, setRuleToRemoveIndex] = useState<number | null>(null);

  // Test person state
  const [testUser, setTestUser] = useState(allPersonas[0]?.eppn || "");
  const [testAction, setTestAction] = useState("export");
  const [testResult, setTestResult] = useState<any | null>(null);
  const [testLoading, setTestLoading] = useState(false);

  const isPlatformAdmin =
    activePersona.affiliation === "central_admin" ||
    activePersona.isAdmin ||
    (activePersona as any).is_platform_admin === true;

  const isOwner =
    (workspace.collaborators || []).some((c) => c.eppn === activePersona.eppn && c.role === "owner") ||
    workspace.lead === activePersona.eppn;

  const canManageRules = isPlatformAdmin || isOwner;

  const unitMap = useMemo(() => {
    const map: Record<string, string> = {};
    for (const u of units) {
      map[u.id] = u.name;
    }
    return map;
  }, [units]);

  const loadGuards = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      const res = await apiClient.getWorkspaceGuards(workspace.id);
      if (res && res.current) {
        setCurrentGuard(res.current);
        setRawSource(res.current.compiled || "");
      } else {
        setCurrentGuard(null);
      }
    } catch (e: any) {
      setError(e.message || "Failed to load workspace guards");
    } finally {
      setLoading(false);
    }
  }, [workspace.id]);

  const loadUnits = useCallback(async () => {
    try {
      const orgs = await apiClient.listOrganizations();
      setUnits(orgs || []);
      if (orgs && orgs.length > 0 && selectedUnitIds.length === 0) {
        setSelectedUnitIds([orgs[0].id]);
      }
    } catch {
      // ignore
    }
  }, [selectedUnitIds.length]);

  useEffect(() => {
    loadGuards();
    loadUnits();
  }, [loadGuards, loadUnits]);

  // Draft rule being built in drawer
  const draftRule: GuardRule = useMemo(() => {
    const isUnitType = selectedTemplate === "deny_access_outside_units";
    return {
      template: selectedTemplate,
      affiliations: isUnitType ? [] : selectedAffiliations,
      unit_ids: isUnitType ? selectedUnitIds : [],
    };
  }, [selectedTemplate, selectedAffiliations, selectedUnitIds]);

  const draftSentence = useMemo(() => {
    return computeSentence(
      draftRule.template,
      draftRule.affiliations || [],
      draftRule.unit_ids || [],
      unitMap
    );
  }, [draftRule, unitMap]);

  // Calculate impact whenever draft rule changes while drawer is open
  useEffect(() => {
    if (!isDrawerOpen) {
      setImpactChanges(null);
      return;
    }
    let isCancelled = false;
    const computeImpact = async () => {
      setImpactLoading(true);
      try {
        const existingRules = currentGuard?.rules || [];
        const res = await apiClient.impactWorkspaceGuards(workspace.id, {
          rules: [...existingRules, draftRule],
        });
        if (!isCancelled) {
          setImpactChanges(res.changes || []);
        }
      } catch {
        if (!isCancelled) {
          setImpactChanges([]);
        }
      } finally {
        if (!isCancelled) {
          setImpactLoading(false);
        }
      }
    };
    computeImpact();
    return () => {
      isCancelled = true;
    };
  }, [isDrawerOpen, draftRule, currentGuard?.rules, workspace.id]);

  // Handle Raw Cedar validation
  useEffect(() => {
    if (!isRawSource) {
      setRawSourceErrors([]);
      return;
    }
    const errors: string[] = [];
    const lines = rawSource.split("\n");
    lines.forEach((line, idx) => {
      if (line.includes("permit")) {
        errors.push(`Line ${idx + 1}: Raw Cedar policy cannot contain 'permit'; guards may only forbid`);
      }
    });
    setRawSourceErrors(errors);
  }, [isRawSource, rawSource]);

  // People losing access under the proposed rule
  const affectedLostPeople = useMemo(() => {
    if (!impactChanges) return [];
    const lost = impactChanges.filter((c) => c.current_allowed && !c.proposed_allowed);
    const uniqueEppns = Array.from(new Set(lost.map((c) => c.eppn)));
    return uniqueEppns.map((eppn) => {
      const match =
        allPersonas.find((p) => p.eppn === eppn) ||
        (workspace.collaborators || []).find((c) => c.eppn === eppn);
      return {
        eppn,
        name: match?.name || eppn,
      };
    });
  }, [impactChanges, allPersonas, workspace.collaborators]);

  const handleTestPerson = async () => {
    if (!testUser) return;
    setTestLoading(true);
    try {
      const existingRules = currentGuard?.rules || [];
      const res = await apiClient.testWorkspaceGuards(workspace.id, {
        user: testUser,
        action: testAction,
        rules: existingRules,
      });
      setTestResult(res);
    } catch (e: any) {
      setTestResult({ error: e.message || "Test failed" });
    } finally {
      setTestLoading(false);
    }
  };

  const filteredUnits = useMemo(() => {
    if (!unitSearch.trim()) return units;
    const query = unitSearch.toLowerCase();
    return units.filter(
      (u) => u.name.toLowerCase().includes(query) || u.code.toLowerCase().includes(query)
    );
  }, [units, unitSearch]);

  const existingRules = currentGuard?.rules || [];
  const existingSentences = currentGuard?.sentences || [];

  return (
    <div className="space-y-6" data-testid="guard-rules-container">
      {/* Platform Admin Subtab bar */}
      {isPlatformAdmin && (
        <div className="flex border-b border-slate-200 dark:border-slate-800 space-x-4 text-xs font-medium">
          <button
            type="button"
            onClick={() => setSubtab("rules")}
            className={`py-2 px-3 border-b-2 cursor-pointer ${
              subtab === "rules"
                ? "border-blue-600 text-blue-600 dark:text-blue-400 font-semibold"
                : "border-transparent text-slate-500 hover:text-slate-700 dark:hover:text-slate-300"
            }`}
          >
            Rules List
          </button>
          <button
            type="button"
            data-testid="guard-rules-advanced"
            onClick={() => setSubtab("advanced")}
            className={`py-2 px-3 border-b-2 cursor-pointer ${
              subtab === "advanced"
                ? "border-blue-600 text-blue-600 dark:text-blue-400 font-semibold"
                : "border-transparent text-slate-500 hover:text-slate-700 dark:hover:text-slate-300"
            }`}
          >
            Advanced
          </button>
        </div>
      )}

      {subtab === "rules" && (
        <div className="space-y-6">
          {/* Header & Add Rule Action */}
          <div className="flex items-center justify-between">
            <div>
              <h3 className="text-sm font-semibold text-slate-900 dark:text-slate-100">
                Workspace Guard Rules
              </h3>
              <p className="text-xs text-slate-500 dark:text-slate-400">
                Guards restrict permissions for this workspace. They can only forbid, never widen access.
              </p>
            </div>
            {canManageRules && (
              <button
                type="button"
                data-testid="add-rule-btn"
                onClick={() => setIsDrawerOpen(true)}
                className="px-3 py-1.5 bg-blue-600 hover:bg-blue-700 text-white rounded text-xs font-medium shadow-sm transition"
              >
                Add a rule
              </button>
            )}
          </div>

          {error && (
            <div className="p-3 bg-red-50 text-red-700 dark:bg-red-950 dark:text-red-300 rounded text-xs">
              {error}
            </div>
          )}

          {/* Rule Cards List */}
          {loading ? (
            <div className="text-xs text-slate-500">Loading workspace guards...</div>
          ) : existingRules.length === 0 ? (
            <div
              data-testid="empty-rules"
              className="p-6 text-center border-2 border-dashed border-slate-200 dark:border-slate-800 rounded-lg text-slate-500 text-xs"
            >
              No guard rules defined for this workspace. Institutional defaults apply.
            </div>
          ) : (
            <div className="space-y-3" data-testid="rule-cards-list">
              {existingRules.map((rule, idx) => {
                const sentence =
                  existingSentences[idx] ||
                  computeSentence(rule.template, rule.affiliations || [], rule.unit_ids || [], unitMap);
                return (
                  <div
                    key={idx}
                    data-testid={`rule-card-${idx}`}
                    className="p-4 bg-white dark:bg-slate-900 border border-slate-200 dark:border-slate-800 rounded-lg flex items-start justify-between gap-4 shadow-xs"
                  >
                    <div className="space-y-1">
                      <p className="text-xs font-semibold text-slate-900 dark:text-slate-100">
                        {sentence}
                      </p>
                      <p className="text-2xs text-slate-500 dark:text-slate-400">
                        Added by {currentGuard?.created_by || "Administrator"} on{" "}
                        {currentGuard?.created_at ? new Date(currentGuard.created_at).toLocaleDateString() : "Active"}
                      </p>
                    </div>
                    {canManageRules && (
                      <button
                        type="button"
                        data-testid={`remove-rule-${idx}`}
                        onClick={() => setRuleToRemoveIndex(idx)}
                        className="px-2.5 py-1 text-xs text-red-600 hover:text-red-700 dark:text-red-400 hover:bg-red-50 dark:hover:bg-red-950/40 rounded transition"
                      >
                        Remove
                      </button>
                    )}
                  </div>
                );
              })}
            </div>
          )}

          {/* Remove Rule Confirm Modal */}
          {ruleToRemoveIndex !== null && (
            <div className="p-4 bg-amber-50 dark:bg-amber-950/30 border border-amber-200 dark:border-amber-900/50 rounded-lg space-y-3">
              <h4 className="text-xs font-semibold text-amber-900 dark:text-amber-200">
                Confirm Removal of Guard Rule
              </h4>
              <ConfirmAction
                verb="Remove"
                target="rule"
                consequence="Removing this guard rule will immediately restore permissions granted by institutional policies."
                audited={true}
                requireReason={true}
                onConfirm={async (reason) => {
                  const remaining = existingRules.filter((_, i) => i !== ruleToRemoveIndex);
                  await apiClient.putWorkspaceGuards(workspace.id, {
                    rules: remaining,
                    reason,
                  });
                  setRuleToRemoveIndex(null);
                  await loadGuards();
                }}
                onSuccess={() => setRuleToRemoveIndex(null)}
              />
              <button
                type="button"
                onClick={() => setRuleToRemoveIndex(null)}
                className="mt-2 text-xs text-slate-500 hover:underline"
              >
                Cancel
              </button>
            </div>
          )}

          {/* Test a Person Section */}
          <div className="pt-4 border-t border-slate-200 dark:border-slate-800 space-y-3" data-testid="test-person-section">
            <h4 className="text-xs font-semibold text-slate-900 dark:text-slate-100">
              Test a person
            </h4>
            <p className="text-2xs text-slate-500 dark:text-slate-400">
              Simulate access evaluation for a persona and action against current guards.
            </p>
            <div className="grid grid-cols-1 sm:grid-cols-3 gap-3">
              <div>
                <label className="block text-2xs font-medium text-slate-600 dark:text-slate-400 mb-1">
                  Person
                </label>
                <select
                  data-testid="test-person-select"
                  value={testUser}
                  onChange={(e) => setTestUser(e.target.value)}
                  className="w-full text-xs p-2 rounded border border-slate-300 dark:border-slate-700 bg-white dark:bg-slate-800 text-slate-900 dark:text-slate-100"
                >
                  {allPersonas.map((p) => (
                    <option key={p.eppn} value={p.eppn}>
                      {p.name} ({p.eppn})
                    </option>
                  ))}
                </select>
              </div>
              <div>
                <label className="block text-2xs font-medium text-slate-600 dark:text-slate-400 mb-1">
                  Action
                </label>
                <select
                  data-testid="test-action-select"
                  value={testAction}
                  onChange={(e) => setTestAction(e.target.value)}
                  className="w-full text-xs p-2 rounded border border-slate-300 dark:border-slate-700 bg-white dark:bg-slate-800 text-slate-900 dark:text-slate-100"
                >
                  <option value="export">export (Export tabular records)</option>
                  <option value="write_record">write_record (Modify workspace records)</option>
                  <option value="read_app">read_app (Read workspace apps)</option>
                  <option value="access_workspace">access_workspace (Open workspace)</option>
                  <option value="approve">approve (Approve workflows)</option>
                </select>
              </div>
              <div className="flex items-end">
                <button
                  type="button"
                  data-testid="test-person-btn"
                  onClick={handleTestPerson}
                  disabled={testLoading}
                  className="w-full py-2 px-3 bg-slate-800 hover:bg-slate-900 text-white dark:bg-slate-700 dark:hover:bg-slate-600 rounded text-xs font-medium transition cursor-pointer"
                >
                  {testLoading ? "Evaluating..." : "Run Test"}
                </button>
              </div>
            </div>

            {testResult && (
              <div
                data-testid="test-person-result"
                className="p-3 bg-slate-50 dark:bg-slate-950/50 border border-slate-200 dark:border-slate-800 rounded text-xs space-y-1.5"
              >
                {testResult.error ? (
                  <p className="text-red-600">{testResult.error}</p>
                ) : (
                  <>
                    <div className="flex items-center justify-between">
                      <span className="text-slate-500">Current evaluation:</span>
                      <span
                        className={`font-semibold ${
                          testResult.current?.allowed ? "text-emerald-600" : "text-red-600"
                        }`}
                      >
                        {testResult.current?.allowed ? "Allowed" : "Denied"}
                      </span>
                    </div>
                    {testResult.current?.policy?.description && (
                      <p className="text-2xs text-slate-500 italic">
                        Deciding policy: "{testResult.current.policy.description}"
                      </p>
                    )}
                    <div className="flex items-center justify-between pt-1 border-t border-slate-200 dark:border-slate-800">
                      <span className="text-slate-500">Under draft:</span>
                      <span
                        className={`font-semibold ${
                          testResult.proposed?.allowed ? "text-emerald-600" : "text-red-600"
                        }`}
                      >
                        {testResult.proposed?.allowed ? "Allowed" : "Denied"}
                      </span>
                    </div>
                    {testResult.proposed?.policy?.description && (
                      <p className="text-2xs text-slate-500 italic">
                        Draft deciding policy: "{testResult.proposed.policy.description}"
                      </p>
                    )}
                  </>
                )}
              </div>
            )}
          </div>
        </div>
      )}

      {/* Advanced subtab (Platform Admin only) */}
      {subtab === "advanced" && isPlatformAdmin && (
        <div className="space-y-4" data-testid="advanced-panel">
          <div>
            <h3 className="text-sm font-semibold text-slate-900 dark:text-slate-100">
              Advanced Cedar Source
            </h3>
            <p className="text-xs text-slate-500 dark:text-slate-400">
              Compiled Cedar policies evaluated on all actions in this workspace. Platform Administrators can inspect or edit raw source.
            </p>
          </div>

          <div className="flex items-center space-x-2">
            <input
              type="checkbox"
              id="edit-raw-cedar-toggle"
              data-testid="edit-raw-cedar-toggle"
              checked={isRawSource}
              onChange={(e) => setIsRawSource(e.target.checked)}
              className="rounded border-slate-300 text-blue-600 focus:ring-blue-500"
            />
            <label htmlFor="edit-raw-cedar-toggle" className="text-xs font-medium text-slate-700 dark:text-slate-300">
              Edit raw Cedar source
            </label>
          </div>

          {isRawSource ? (
            <div className="space-y-3">
              <textarea
                data-testid="raw-cedar-input"
                rows={10}
                value={rawSource}
                onChange={(e) => setRawSource(e.target.value)}
                placeholder="forbid (principal, action, resource) when { ... };"
                className="w-full font-mono text-xs p-3 rounded border border-slate-300 dark:border-slate-700 bg-white dark:bg-slate-950 text-slate-900 dark:text-slate-100"
              />
              {rawSourceErrors.length > 0 && (
                <div data-testid="raw-cedar-errors" className="p-2.5 bg-red-50 text-red-700 dark:bg-red-950 dark:text-red-300 rounded text-xs space-y-1">
                  {rawSourceErrors.map((err, idx) => (
                    <div key={idx}>{err}</div>
                  ))}
                </div>
              )}
              <ConfirmAction
                verb="Save"
                target="Cedar source"
                consequence="Direct Cedar source modifications will immediately govern workspace authorization decisions."
                audited={true}
                requireReason={true}
                onConfirm={async (reason) => {
                  if (rawSourceErrors.length > 0) {
                    throw new Error("Cannot save Cedar source with syntax errors.");
                  }
                  await apiClient.putWorkspaceGuards(workspace.id, {
                    source: rawSource,
                    reason,
                  });
                  await loadGuards();
                }}
              />
            </div>
          ) : (
            <div>
              <label className="block text-2xs font-medium text-slate-500 mb-1">
                Compiled Cedar Policies (Read-Only)
              </label>
              <pre
                data-testid="compiled-cedar"
                className="p-3 bg-slate-900 text-slate-100 rounded text-xs font-mono overflow-x-auto whitespace-pre-wrap max-h-72"
              >
                {currentGuard?.compiled || "// No guards compiled for this workspace"}
              </pre>
            </div>
          )}
        </div>
      )}

      {/* Drawer for Adding a Rule */}
      <Drawer
        open={isDrawerOpen}
        onClose={() => setIsDrawerOpen(false)}
        title="Add a Guard Rule"
      >
        <div className="space-y-5 text-xs">
          {/* Template Selection */}
          <div className="space-y-2">
            <label className="block font-medium text-slate-700 dark:text-slate-300">
              Select Rule Template
            </label>
            <div className="space-y-2">
              {TEMPLATES.map((tmpl) => (
                <label
                  key={tmpl.id}
                  className={`flex items-start p-2.5 rounded border cursor-pointer transition ${
                    selectedTemplate === tmpl.id
                      ? "border-blue-500 bg-blue-50/50 dark:bg-blue-950/30 dark:border-blue-800"
                      : "border-slate-200 dark:border-slate-800 hover:bg-slate-50 dark:hover:bg-slate-800/50"
                  }`}
                >
                  <input
                    type="radio"
                    name="guard-template"
                    value={tmpl.id}
                    checked={selectedTemplate === tmpl.id}
                    onChange={() => setSelectedTemplate(tmpl.id)}
                    className="mt-0.5 text-blue-600 focus:ring-blue-500"
                  />
                  <div className="ml-2.5">
                    <p className="font-semibold text-slate-900 dark:text-slate-100">{tmpl.title}</p>
                    <p className="text-2xs text-slate-500 dark:text-slate-400">{tmpl.description}</p>
                  </div>
                </label>
              ))}
            </div>
          </div>

          {/* Parameter Controls */}
          {selectedTemplate === "deny_access_outside_units" ? (
            /* Units Searchable List */
            <div className="space-y-2">
              <label className="block font-medium text-slate-700 dark:text-slate-300">
                Permitted Organizational Units
              </label>
              <input
                type="text"
                data-testid="unit-search-input"
                placeholder="Search units by name or code..."
                value={unitSearch}
                onChange={(e) => setUnitSearch(e.target.value)}
                className="w-full text-xs p-2 rounded border border-slate-300 dark:border-slate-700 bg-white dark:bg-slate-800 text-slate-900 dark:text-slate-100"
              />
              <div className="max-h-40 overflow-y-auto border border-slate-200 dark:border-slate-800 rounded p-2 space-y-1">
                {filteredUnits.length === 0 ? (
                  <div className="text-slate-400 text-2xs p-1">No units found matching search.</div>
                ) : (
                  filteredUnits.map((u) => {
                    const checked = selectedUnitIds.includes(u.id);
                    return (
                      <label
                        key={u.id}
                        className="flex items-center space-x-2 p-1 hover:bg-slate-100 dark:hover:bg-slate-800 rounded cursor-pointer"
                      >
                        <input
                          type="checkbox"
                          checked={checked}
                          onChange={(e) => {
                            if (e.target.checked) {
                              setSelectedUnitIds([...selectedUnitIds, u.id]);
                            } else {
                              setSelectedUnitIds(selectedUnitIds.filter((id) => id !== u.id));
                            }
                          }}
                          className="rounded border-slate-300 text-blue-600 focus:ring-blue-500"
                        />
                        <span className="text-xs text-slate-700 dark:text-slate-300">
                          {u.name} ({u.code})
                        </span>
                      </label>
                    );
                  })
                )}
              </div>
            </div>
          ) : (
            /* Affiliations Checkboxes */
            <div className="space-y-2">
              <label className="block font-medium text-slate-700 dark:text-slate-300">
                Target Affiliations
              </label>
              <div className="grid grid-cols-2 gap-2">
                {AFFILIATIONS.map((aff) => {
                  const checked = selectedAffiliations.includes(aff.id);
                  return (
                    <label
                      key={aff.id}
                      className="flex items-center space-x-2 p-1.5 border border-slate-200 dark:border-slate-800 rounded hover:bg-slate-50 dark:hover:bg-slate-800 cursor-pointer"
                    >
                      <input
                        type="checkbox"
                        checked={checked}
                        onChange={(e) => {
                          if (e.target.checked) {
                            setSelectedAffiliations([...selectedAffiliations, aff.id]);
                          } else {
                            setSelectedAffiliations(selectedAffiliations.filter((id) => id !== aff.id));
                          }
                        }}
                        className="rounded border-slate-300 text-blue-600 focus:ring-blue-500"
                      />
                      <span className="text-xs text-slate-700 dark:text-slate-300">{aff.label}</span>
                    </label>
                  );
                })}
              </div>
            </div>
          )}

          {/* Live Sentence Preview */}
          <div className="space-y-1">
            <label className="block text-2xs font-medium text-slate-500 uppercase tracking-wider">
              Sentence Preview
            </label>
            <div
              data-testid="sentence-preview"
              className="p-3 bg-slate-100 dark:bg-slate-800 border border-slate-200 dark:border-slate-700 rounded text-xs font-semibold text-slate-900 dark:text-slate-100"
            >
              {draftSentence}
            </div>
          </div>

          {/* Impact Route Results */}
          <div className="space-y-1.5">
            <label className="block text-2xs font-medium text-slate-500 uppercase tracking-wider">
              Who this affects
            </label>
            {impactLoading ? (
              <div className="p-3 bg-slate-50 dark:bg-slate-900 border border-slate-200 dark:border-slate-800 rounded text-2xs text-slate-500">
                Evaluating impact against workspace members...
              </div>
            ) : affectedLostPeople.length === 0 ? (
              <div
                data-testid="impact-summary"
                className="p-3 bg-slate-50 dark:bg-slate-900 border border-slate-200 dark:border-slate-800 rounded text-xs text-slate-600 dark:text-slate-300"
              >
                No one is affected.
              </div>
            ) : (
              <div
                data-testid="impact-summary"
                className="p-3 bg-amber-50 dark:bg-amber-950/40 border border-amber-200 dark:border-amber-900/60 rounded text-xs text-amber-900 dark:text-amber-200 space-y-2"
              >
                <p className="font-semibold">
                  {affectedLostPeople.length} of {(workspace.collaborators || []).length || allPersonas.length} people would lose access under this rule.
                </p>
                <ul className="list-disc list-inside space-y-0.5 text-2xs text-amber-800 dark:text-amber-300">
                  {affectedLostPeople.map((person) => (
                    <li key={person.eppn}>{person.name} ({person.eppn})</li>
                  ))}
                </ul>
              </div>
            )}
          </div>

          {/* ConfirmAction Save */}
          <div className="pt-3 border-t border-slate-200 dark:border-slate-800">
            <ConfirmAction
              verb="Save"
              target="rule"
              consequence="Adding this guard rule will immediately constrain permissions for all matching members."
              audited={true}
              requireReason={true}
              onConfirm={async (reason) => {
                const combined = [...existingRules, draftRule];
                await apiClient.putWorkspaceGuards(workspace.id, {
                  rules: combined,
                  reason,
                });
                setIsDrawerOpen(false);
                await loadGuards();
              }}
              onSuccess={() => setIsDrawerOpen(false)}
            />
          </div>
        </div>
      </Drawer>
    </div>
  );
};
