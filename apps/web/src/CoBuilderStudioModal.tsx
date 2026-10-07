import React from "react";
import { ManifestRenderer } from "./ManifestRenderer";
import { WorkflowBuilder } from "./WorkflowBuilder";
import { SEEDED_DATASETS } from "./DatasetExplorer";
import { Persona, PublishedDataset, RegisteredApp, WorkflowAutomationRule } from "./types";

export interface CoBuilderStudioModalProps {
  isOpen: boolean;
  onClose: () => void;
  activeStudioApp: RegisteredApp;
  setActiveStudioApp: React.Dispatch<React.SetStateAction<RegisteredApp | null>>;
  studioTab: "schema" | "collaborators" | "automations" | "preview" | "publish";
  setStudioTab: React.Dispatch<React.SetStateAction<"schema" | "collaborators" | "automations" | "preview" | "publish">>;
  automations: Record<string, WorkflowAutomationRule[]>;
  handleSaveRule: (appSlug: string, rule: WorkflowAutomationRule) => void;
  handleDeleteRule: (appSlug: string, ruleId: string) => void;
  handleToggleRule: (appSlug: string, ruleId: string) => void;
  handleAddLinkedDatasetField: (ds: PublishedDataset) => void;
  handleAddFieldToStudioApp: () => void;
  handleToggleFieldFerpa: (fieldName: string) => void;
  handleAddCollaborator: (peer: Persona) => void;
  handlePublishApp: () => void;
  showToast: (msg: string) => void;
  personas: Persona[];
  setApps: React.Dispatch<React.SetStateAction<RegisteredApp[]>>;
}

export const CoBuilderStudioModal: React.FC<CoBuilderStudioModalProps> = ({
  isOpen,
  onClose,
  activeStudioApp,
  setActiveStudioApp,
  studioTab,
  setStudioTab,
  automations,
  handleSaveRule,
  handleDeleteRule,
  handleToggleRule,
  handleAddLinkedDatasetField,
  handleAddFieldToStudioApp,
  handleToggleFieldFerpa,
  handleAddCollaborator,
  handlePublishApp,
  showToast,
  personas,
  setApps,
}) => {
  if (!isOpen || !activeStudioApp) return null;

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center p-4 bg-slate-900/60 backdrop-blur-xs animate-fade-in">
      <div className="bg-white dark:bg-slate-900 border border-slate-200 dark:border-slate-800 rounded-xl shadow-2xl w-full max-w-4xl max-h-[90vh] flex flex-col overflow-hidden">
        {/* Studio Header */}
        <div className="px-6 py-4 border-b border-slate-200 dark:border-slate-800 flex items-center justify-between">
          <div className="flex items-center gap-3">
            <img src="/logo-mark.png" alt="Scaffoldry" className="h-8 w-auto object-contain shrink-0" />
            <div>
              <div className="flex items-center gap-2">
                <span className="text-[10px] uppercase font-bold tracking-wider px-2 py-0.5 rounded bg-blue-50 text-blue-700 dark:bg-blue-950/60 dark:text-blue-300">
                  App Studio &amp; Co-Builder
                </span>
                <span className="font-mono text-xs text-slate-400">{activeStudioApp.slug}</span>
              </div>
              <h2 className="text-base font-bold text-slate-900 dark:text-white mt-0.5">
                {activeStudioApp.title}
              </h2>
            </div>
          </div>
          <button
            type="button"
            onClick={onClose}
            className="p-1 rounded-md text-slate-400 hover:text-slate-600 dark:hover:text-slate-200 hover:bg-slate-100 dark:hover:bg-slate-800 text-sm"
          >
            ✕
          </button>
        </div>

        {/* Studio Sub-Navigation */}
        <div className="px-6 border-b border-slate-200 dark:border-slate-800 flex items-center gap-4 text-xs font-medium bg-slate-50/60 dark:bg-slate-800/30">
          <button
            type="button"
            onClick={() => setStudioTab("schema")}
            className={`py-3 border-b-2 cursor-pointer transition-colors ${
              studioTab === "schema"
                ? "border-blue-600 text-blue-600 dark:text-blue-400 font-bold"
                : "border-transparent text-slate-500 hover:text-slate-800 dark:hover:text-slate-200"
            }`}
          >
            1. Visual Field Builder ({activeStudioApp.manifest.views[0]?.fields?.length || 0})
          </button>
          <button
            type="button"
            onClick={() => setStudioTab("collaborators")}
            className={`py-3 border-b-2 cursor-pointer transition-colors ${
              studioTab === "collaborators"
                ? "border-blue-600 text-blue-600 dark:text-blue-400 font-bold"
                : "border-transparent text-slate-500 hover:text-slate-800 dark:hover:text-slate-200"
            }`}
          >
            2. Team Collaborators ({activeStudioApp.collaborators.length})
          </button>
          <button
            type="button"
            onClick={() => setStudioTab("automations")}
            className={`py-3 border-b-2 cursor-pointer transition-colors ${
              studioTab === "automations"
                ? "border-blue-600 text-blue-600 dark:text-blue-400 font-bold"
                : "border-transparent text-slate-500 hover:text-slate-800 dark:hover:text-slate-200"
            }`}
          >
            3. Workflow Automations &amp; Triggers ({(automations[activeStudioApp.slug] || []).length})
          </button>
          <button
            type="button"
            onClick={() => setStudioTab("preview")}
            className={`py-3 border-b-2 cursor-pointer transition-colors ${
              studioTab === "preview"
                ? "border-blue-600 text-blue-600 dark:text-blue-400 font-bold"
                : "border-transparent text-slate-500 hover:text-slate-800 dark:hover:text-slate-200"
            }`}
          >
            4. Live Interactive Preview
          </button>
          <button
            type="button"
            onClick={() => setStudioTab("publish")}
            className={`py-3 border-b-2 cursor-pointer transition-colors ${
              studioTab === "publish"
                ? "border-blue-600 text-blue-600 dark:text-blue-400 font-bold"
                : "border-transparent text-slate-500 hover:text-slate-800 dark:hover:text-slate-200"
            }`}
          >
            5. Vanity DNS &amp; Publish
          </button>
        </div>

        {/* Studio Body */}
        <div className="p-6 flex-1 overflow-y-auto">
          {/* TAB 1: VISUAL FIELD BUILDER */}
          {studioTab === "schema" && (
            <div className="space-y-4">
              <div className="flex items-center justify-between">
                <div>
                  <h3 className="text-sm font-semibold text-slate-900 dark:text-white">
                    Form Schema &amp; Data Fields
                  </h3>
                  <p className="text-xs text-slate-500">
                    Design the inputs for this application. Toggle FERPA sensitivity to automatically apply 34 CFR § 99.30 Cedar guardrails.
                  </p>
                </div>
                <div className="flex items-center gap-2">
                  <select
                    onChange={(e) => {
                      const ds = SEEDED_DATASETS.find((d) => d.id === e.target.value);
                      if (ds) {
                        handleAddLinkedDatasetField(ds);
                        e.target.value = "";
                      }
                    }}
                    defaultValue=""
                    className="px-3 py-1.5 text-xs font-semibold rounded bg-purple-50 dark:bg-purple-950/60 text-purple-700 dark:text-purple-300 border border-purple-200 dark:border-purple-800 hover:bg-purple-100 dark:hover:bg-purple-900/60 cursor-pointer focus:outline-none"
                  >
                    <option value="" disabled>
                      🔗 + Link Published Dataset...
                    </option>
                    {SEEDED_DATASETS.map((ds) => (
                      <option key={ds.id} value={ds.id}>
                        {ds.name} ({ds.department})
                      </option>
                    ))}
                  </select>

                  <button
                    type="button"
                    onClick={handleAddFieldToStudioApp}
                    className="px-3 py-1.5 text-xs font-semibold rounded bg-blue-600 hover:bg-blue-700 text-white cursor-pointer"
                  >
                    + Add Input Field
                  </button>
                </div>
              </div>

              <div className="space-y-2">
                {(activeStudioApp.manifest.views[0]?.fields || []).map((field) => (
                  <div
                    key={field.name}
                    className="p-3 rounded-lg border border-slate-200 dark:border-slate-800 bg-slate-50 dark:bg-slate-800/40 flex flex-wrap items-center justify-between gap-3 text-xs"
                  >
                    <div className="flex-1 min-w-[200px]">
                      <div className="font-semibold text-slate-800 dark:text-slate-200">
                        {field.label} {field.required && <span className="text-rose-500">*</span>}
                      </div>
                      <div className="font-mono text-[11px] text-slate-400 flex items-center gap-2 mt-0.5">
                        <span>key: {field.name}</span>
                        <span>·</span>
                        <span>type: {field.field_type}</span>
                        {field.linked_dataset_id && (
                          <span className="px-1.5 py-0.2 rounded bg-purple-100 text-purple-800 dark:bg-purple-950 dark:text-purple-300 font-bold">
                            🔗 {field.linked_dataset_id}
                          </span>
                        )}
                      </div>
                    </div>

                    <div className="flex items-center gap-2">
                      <button
                        type="button"
                        onClick={() => handleToggleFieldFerpa(field.name)}
                        className={`px-2.5 py-1 rounded text-[11px] font-medium border cursor-pointer transition-colors ${
                          field.ferpa_sensitive
                            ? "bg-rose-100 dark:bg-rose-950/60 text-rose-700 dark:text-rose-300 border-rose-300 dark:border-rose-900"
                            : "bg-white dark:bg-slate-800 text-slate-600 dark:text-slate-400 border-slate-200 dark:border-slate-700"
                        }`}
                      >
                        {field.ferpa_sensitive ? "🔒 FERPA Sensitive (Protected)" : "Standard Field"}
                      </button>

                      <span className="text-[11px] font-mono text-slate-400">
                        {activeStudioApp.manifest.ceds_mappings[field.name]
                          ? `CEDS: ${activeStudioApp.manifest.ceds_mappings[field.name]}`
                          : "No CEDS tag"}
                      </span>
                    </div>
                  </div>
                ))}
              </div>
            </div>
          )}

          {/* TAB 2: TEAM COLLABORATORS */}
          {studioTab === "collaborators" && (
            <div className="space-y-4">
              <div>
                <h3 className="text-sm font-semibold text-slate-900 dark:text-white">
                  Co-Building Team &amp; Permissions
                </h3>
                <p className="text-xs text-slate-500">
                  Share and co-build this application with peers in your department or cross-functional compliance officers.
                </p>
              </div>

              <div className="space-y-2">
                {activeStudioApp.collaborators.map((c) => (
                  <div
                    key={c.eppn}
                    className="p-3 rounded-lg border border-slate-200 dark:border-slate-800 bg-slate-50 dark:bg-slate-800/40 flex items-center justify-between text-xs"
                  >
                    <div className="flex items-center gap-3">
                      <div className="w-7 h-7 rounded-full bg-blue-600 text-white font-bold text-xs flex items-center justify-center uppercase">
                        {c.name[0]}
                      </div>
                      <div>
                        <div className="font-semibold text-slate-800 dark:text-slate-200">{c.name}</div>
                        <div className="font-mono text-[11px] text-slate-400">{c.eppn}</div>
                      </div>
                    </div>
                    <span className="font-mono text-xs uppercase px-2 py-0.5 rounded bg-blue-50 text-blue-700 dark:bg-blue-950/60 dark:text-blue-300 font-bold">
                      {c.role}
                    </span>
                  </div>
                ))}
              </div>

              <div className="pt-3 border-t border-slate-200 dark:border-slate-800">
                <div className="text-xs font-semibold text-slate-700 dark:text-slate-300 mb-2">
                  Invite Peer to Co-Build:
                </div>
                <div className="flex flex-wrap gap-2">
                  {personas.filter((p) => !activeStudioApp.collaborators.some((c) => c.eppn === p.eppn)).map((peer) => (
                    <button
                      key={peer.eppn}
                      type="button"
                      onClick={() => handleAddCollaborator(peer)}
                      className="px-2.5 py-1.5 rounded-lg border border-slate-200 dark:border-slate-700 bg-white dark:bg-slate-800 hover:bg-slate-50 text-xs font-medium cursor-pointer"
                    >
                      + {peer.name} ({peer.roleTitle})
                    </button>
                  ))}
                </div>
              </div>
            </div>
          )}

          {/* TAB 3: WORKFLOW AUTOMATIONS & TRIGGERS */}
          {studioTab === "automations" && (
            <WorkflowBuilder
              appSlug={activeStudioApp.slug}
              appTitle={activeStudioApp.manifest.title}
              fields={activeStudioApp.manifest.views[0]?.fields || []}
              rules={automations[activeStudioApp.slug] || []}
              onSaveRule={(rule) => handleSaveRule(activeStudioApp.slug, rule)}
              onDeleteRule={(ruleId) => handleDeleteRule(activeStudioApp.slug, ruleId)}
              onToggleRule={(ruleId) => handleToggleRule(activeStudioApp.slug, ruleId)}
            />
          )}

          {/* TAB 4: LIVE PREVIEW */}
          {studioTab === "preview" && (
            <div className="space-y-4">
              <div>
                <h3 className="text-sm font-semibold text-slate-900 dark:text-white">
                  Live Departmental Form Preview
                </h3>
                <p className="text-xs text-slate-500">
                  Test-run the application as end users will experience it when published.
                </p>
              </div>
              <ManifestRenderer
                manifest={activeStudioApp.manifest}
                onSubmitRecord={() => showToast("Test record submitted successfully in studio preview!")}
              />
            </div>
          )}

          {/* TAB 5: PUBLISH TO VANITY DNS */}
          {studioTab === "publish" && (
            <div className="space-y-5">
              <div>
                <h3 className="text-sm font-semibold text-slate-900 dark:text-white">
                  Publish Application &amp; Bind Vanity DNS Alias
                </h3>
                <p className="text-xs text-slate-500">
                  Deploy this application with sub-millisecond host-header routing on the institutional domain.
                </p>
              </div>

              <div className="p-4 rounded-xl border border-slate-200 dark:border-slate-800 bg-slate-50 dark:bg-slate-800/40 space-y-3 text-xs">
                <div>
                  <label className="block text-[11px] font-semibold text-slate-500 uppercase mb-1">
                    Assigned Institutional Domain
                  </label>
                  <input
                    type="text"
                    value={activeStudioApp.customDomain}
                    onChange={(e) => {
                      const updated = { ...activeStudioApp, customDomain: e.target.value };
                      setActiveStudioApp(updated);
                      setApps((prev) => prev.map((a) => (a.slug === updated.slug ? updated : a)));
                    }}
                    className="w-full px-3 py-2 text-xs font-mono rounded border border-slate-300 dark:border-slate-700 bg-white dark:bg-slate-900 text-blue-600 dark:text-blue-400"
                  />
                </div>

                <div className="flex justify-between items-center pt-2">
                  <div>
                    <div className="font-semibold text-slate-800 dark:text-slate-200">Zero-Open-Port Ingress</div>
                    <div className="text-[11px] text-slate-400">Host router forwards requests directly without exposed hypervisor ports.</div>
                  </div>
                  <span className="font-mono text-emerald-600 font-bold">&lt; 1 ms Latency</span>
                </div>
              </div>

              <div className="flex items-center gap-3">
                <button
                  type="button"
                  onClick={handlePublishApp}
                  className="px-4 py-2 rounded-lg bg-emerald-600 hover:bg-emerald-700 text-white font-semibold text-xs shadow-xs cursor-pointer"
                >
                  ✓ Publish Application to DNS
                </button>
                <span className="text-xs text-slate-400">
                  Status: <strong className="text-slate-700 dark:text-slate-300">{activeStudioApp.status}</strong>
                </span>
              </div>
            </div>
          )}
        </div>

        {/* Studio Footer */}
        <div className="px-6 py-3 border-t border-slate-200 dark:border-slate-800 bg-slate-50 dark:bg-slate-900/80 flex items-center justify-between text-xs">
          <button
            type="button"
            onClick={onClose}
            className="px-3 py-1.5 rounded text-slate-500 hover:text-slate-800 cursor-pointer"
          >
            Close Studio
          </button>
          <button
            type="button"
            onClick={() => {
              onClose();
              showToast(`Changes to "${activeStudioApp.title}" saved.`);
            }}
            className="px-3 py-1.5 rounded bg-blue-600 hover:bg-blue-700 text-white font-semibold cursor-pointer"
          >
            Done Editing
          </button>
        </div>
      </div>
    </div>
  );
};
