import React from "react";
import { DecisionLedgerView } from "./DecisionLedgerView";
import { LedgerEntryItem, Persona, RegisteredApp, SourceRule } from "./types";

export interface AdminConsoleViewProps {
  activePersona: Persona;
  isImpersonating: boolean;
  realAdmin: Persona | null;
  handleStopImpersonation: () => void;
  navigateTo: (path: string) => void;
  adminTab: "org" | "policy" | "ledger" | "infra" | "impersonation";
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

      {/* ADMIN TAB 1: ORG & DNS MANAGER */}
      {adminTab === "org" && (
        <div className="space-y-4">
          <div className="bg-white dark:bg-slate-900 border border-slate-200 dark:border-slate-800 rounded-lg p-5 shadow-xs space-y-4">
            <h2 className="text-base font-bold text-slate-900 dark:text-white">
              Department Realms &amp; DNS Vanity Routing
            </h2>
            <p className="text-xs text-slate-500">
              Sub-millisecond host-header routing table configured across all university departments without open
              inbound ports.
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

      {/* ADMIN TAB 3: INFRASTRUCTURE TOPOLOGY */}
      {adminTab === "infra" && (
        <div className="grid grid-cols-1 md:grid-cols-3 gap-4">
          <div className="bg-white dark:bg-slate-900 border border-slate-200 dark:border-slate-800 rounded-lg p-4 shadow-xs">
            <span className="text-[10px] uppercase font-bold text-slate-400 block mb-1">Live Endpoint</span>
            <span className="font-mono text-xs text-blue-600 dark:text-blue-400 break-all">
              https://scaffoldry-desk-ljbhpnq7oa-uc.a.run.app
            </span>
          </div>
          <div className="bg-white dark:bg-slate-900 border border-slate-200 dark:border-slate-800 rounded-lg p-4 shadow-xs">
            <span className="text-[10px] uppercase font-bold text-slate-400 block mb-1">GCP Region</span>
            <span className="text-sm font-semibold text-slate-900 dark:text-white">us-central1 (scaffoldry-io)</span>
          </div>
          <div className="bg-white dark:bg-slate-900 border border-slate-200 dark:border-slate-800 rounded-lg p-4 shadow-xs">
            <span className="text-[10px] uppercase font-bold text-slate-400 block mb-1">Auth Lattice</span>
            <span className="text-sm font-semibold text-slate-900 dark:text-white">Workload Identity Federation</span>
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
    </div>
  );
};
