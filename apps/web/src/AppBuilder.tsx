import React, { useState } from "react";
import { AppManifest, AppPage, GovernedComponentSpec, GovernedComponentType, RegisteredApp, WorkflowAutomationRule } from "./types";
import { ComponentInspectorFlyout } from "./ComponentInspectorFlyout";
import { WorkflowBuilder } from "./WorkflowBuilder";
import { MultiViewWorkspace } from "./MultiViewWorkspace";

interface AppBuilderProps {
  app: RegisteredApp;
  onBack: () => void;
  onOpenPublishedApp: (slug: string) => void;
  onOpenAiAssistant?: () => void;
  onSaveApp?: (updated: RegisteredApp) => void;
}

export const AppBuilder: React.FC<AppBuilderProps> = ({
  app,
  onBack,
  onOpenPublishedApp,
  onOpenAiAssistant,
  onSaveApp,
}) => {
  const [activeTab, setActiveTab] = useState<"data" | "pages" | "automations" | "settings">("pages");
  const [activePageId, setActivePageId] = useState<string>("page-overview");
  const [selectedComponentId, setSelectedComponentId] = useState<string | null>(null);
  const [isFlyoutOpen, setIsFlyoutOpen] = useState<boolean>(false);
  const [saveStatus, setSaveStatus] = useState<string | null>(null);
  const [automationRules, setAutomationRules] = useState<WorkflowAutomationRule[]>([
    {
      id: "rule-1",
      app_slug: app.slug,
      name: "Honors Research Grant Notification",
      description: "Trigger alert and ledger entry when grant status advances to Approved",
      enabled: true,
      trigger: { type: "StatusChanged", to_status: "Approved" },
      cedar_policy_guard: "permit(principal, action == Action::\"approve\", resource);",
      predicates: [{ field_name: "budget", operator: "GreaterThan", expected_value: "500000" }],
      actions: [
        { type: "NotifyCollaborator", role: "Dean of Research", message_template: "Major grant approved over threshold" },
        { type: "CreateLedgerAuditEntry", summary: "Faculty council grant approval block", oscal_control: "AC-03" },
      ],
    },
  ]);

  // Initialize or derive pages
  const defaultPages: AppPage[] = app.manifest.pages && app.manifest.pages.length > 0
    ? app.manifest.pages
    : [
        {
          id: "page-overview",
          slug: "overview",
          title: "Executive Dashboard",
          icon: "📊",
          description: "High-level metrics and active records queue",
          components: [
            {
              id: "comp-banner-1",
              type: "rich-banner",
              title: "Institutional Review Board Notice",
              slot: "header",
              layout: { width: "full", order: 0 },
              config: {
                variant: "info",
                content: "All proposals submitted under this cycle are subject to 2 CFR 200 uniform guidance and university conflict-of-interest review.",
              },
            },
            {
              id: "comp-stat-1",
              type: "stat-metric",
              title: "Active Proposals",
              slot: "main",
              layout: { width: "third", order: 1 },
              config: {
                subtitle: "Fiscal Year 2026",
                aggregation: "count",
                trend_text: "+14% vs FY25",
                accent_color: "blue",
              },
            },
            {
              id: "comp-stat-2",
              type: "stat-metric",
              title: "Total Requested Budget",
              slot: "main",
              layout: { width: "third", order: 2 },
              config: {
                subtitle: "Across all departments",
                aggregation: "sum",
                target_field: "budget",
                trend_text: "$3.4M cumulative",
                accent_color: "emerald",
              },
            },
            {
              id: "comp-stat-3",
              type: "stat-metric",
              title: "Under Review Queue",
              slot: "main",
              layout: { width: "third", order: 3 },
              config: {
                subtitle: "Faculty committee",
                aggregation: "count",
                filter_status: "Under Review",
                trend_text: "Target 14 days",
                accent_color: "amber",
              },
            },
            {
              id: "comp-grid-1",
              type: "tabular-grid",
              title: "Governed Proposals Registry",
              slot: "main",
              layout: { width: "full", order: 4 },
              config: {
                visible_columns: ["id", "title", "department", "status", "budget", "submitted_at"],
                rollup_type: "sum",
                enable_search: true,
                enable_inline_edit: true,
              },
            },
          ],
        },
        {
          id: "page-stages",
          slug: "stages",
          title: "Workflow Kanban",
          icon: "📋",
          description: "Stage progression and approval board",
          components: [
            {
              id: "comp-kanban-1",
              type: "kanban-stage",
              title: "Proposal Evaluation Stages",
              slot: "main",
              layout: { width: "full", order: 0 },
              config: {
                stage_field: "status",
                card_title_field: "title",
              },
            },
          ],
        },
        {
          id: "page-submit",
          slug: "intake",
          title: "Proposal Submission Form",
          icon: "📝",
          description: "Intake form for principal investigators",
          components: [
            {
              id: "comp-form-1",
              type: "intake-form",
              title: "New Research Grant Intake",
              slot: "main",
              layout: { width: "two-thirds", order: 0 },
              config: {
                submit_button_label: "Submit for Faculty Review",
                success_message: "Proposal successfully registered with institutional hash block.",
              },
            },
          ],
        },
      ];

  const [pages, setPages] = useState<AppPage[]>(defaultPages);

  const activePage = pages.find((p) => p.id === activePageId) || pages[0];
  const selectedComponent =
    activePage.components.find((c) => c.id === selectedComponentId) || null;

  const handleSelectComponent = (comp: GovernedComponentSpec) => {
    setSelectedComponentId(comp.id);
    setIsFlyoutOpen(true);
  };

  const handleUpdateComponent = (updated: GovernedComponentSpec) => {
    setPages((prevPages) =>
      prevPages.map((page) => {
        if (page.id !== activePage.id) return page;
        return {
          ...page,
          components: page.components.map((c) => (c.id === updated.id ? updated : c)),
        };
      })
    );
  };

  const handleDeleteComponent = (id: string) => {
    setPages((prevPages) =>
      prevPages.map((page) => {
        if (page.id !== activePage.id) return page;
        return {
          ...page,
          components: page.components.filter((c) => c.id !== id),
        };
      })
    );
    if (selectedComponentId === id) {
      setSelectedComponentId(null);
      setIsFlyoutOpen(false);
    }
  };

  const handleMoveComponent = (id: string, direction: "up" | "down") => {
    setPages((prevPages) =>
      prevPages.map((page) => {
        if (page.id !== activePage.id) return page;
        const comps = [...page.components];
        const idx = comps.findIndex((c) => c.id === id);
        if (idx === -1) return page;
        if (direction === "up" && idx > 0) {
          const temp = comps[idx - 1];
          comps[idx - 1] = comps[idx];
          comps[idx] = temp;
        } else if (direction === "down" && idx < comps.length - 1) {
          const temp = comps[idx + 1];
          comps[idx + 1] = comps[idx];
          comps[idx] = temp;
        }
        return { ...page, components: comps };
      })
    );
  };

  const handleAddComponent = (type: GovernedComponentType) => {
    const newId = `comp-${type}-${Date.now().toString().slice(-4)}`;
    const newComp: GovernedComponentSpec = {
      id: newId,
      type,
      title: `New ${type.replace("-", " ")}`,
      slot: "main",
      layout: {
        width: type === "stat-metric" ? "third" : type === "intake-form" ? "two-thirds" : "full",
        order: activePage.components.length,
      },
      config: {
        accent_color: "blue",
      },
    };

    setPages((prev) =>
      prev.map((page) => {
        if (page.id !== activePage.id) return page;
        return {
          ...page,
          components: [...page.components, newComp],
        };
      })
    );

    setSelectedComponentId(newId);
    setIsFlyoutOpen(true);
  };

  const handleAddPage = () => {
    const newId = `page-${Date.now().toString().slice(-4)}`;
    const newPage: AppPage = {
      id: newId,
      slug: `custom-page-${pages.length + 1}`,
      title: `New Page ${pages.length + 1}`,
      icon: "📄",
      description: "Custom application page",
      components: [],
    };
    setPages((prev) => [...prev, newPage]);
    setActivePageId(newId);
  };

  const handleSave = () => {
    const updatedManifest: AppManifest = {
      ...app.manifest,
      pages,
    };
    const updatedApp: RegisteredApp = {
      ...app,
      manifest: updatedManifest,
      updatedAt: "Just now",
    };
    if (onSaveApp) {
      onSaveApp(updatedApp);
    }
    setSaveStatus("Saved to Sovereign Manifest");
    setTimeout(() => setSaveStatus(null), 3000);
  };

  return (
    <div className="min-h-screen bg-slate-50 dark:bg-slate-950 text-slate-900 dark:text-slate-100 flex flex-col font-sans">
      {/* Top Application Bar */}
      <header className="sticky top-0 z-30 bg-white/90 dark:bg-slate-900/90 backdrop-blur-md border-b border-slate-200 dark:border-slate-800 px-6 py-3 flex items-center justify-between">
        <div className="flex items-center gap-4">
          <button
            type="button"
            onClick={onBack}
            className="flex items-center gap-1.5 text-xs font-semibold text-slate-500 hover:text-slate-900 dark:hover:text-white px-2.5 py-1.5 rounded-md hover:bg-slate-100 dark:hover:bg-slate-800 transition-colors cursor-pointer"
          >
            ← Back to Desk
          </button>
          <div className="h-4 w-px bg-slate-200 dark:bg-slate-700" />
          <div className="flex items-center gap-3">
            <img src="/logo-mark.png" alt="Scaffoldry" className="h-7 w-auto object-contain" />
            <div>
              <div className="flex items-center gap-2">
                <h1 className="text-sm font-bold text-slate-900 dark:text-white">{app.title}</h1>
                <span className="text-[10px] uppercase font-mono px-2 py-0.5 rounded bg-blue-50 text-blue-700 dark:bg-blue-950 dark:text-blue-300 font-semibold">
                  /builder/{app.slug}
                </span>
                <span className="text-[10px] font-semibold px-2 py-0.5 rounded bg-emerald-50 text-emerald-700 dark:bg-emerald-950 dark:text-emerald-300">
                  {app.status || "Published"}
                </span>
              </div>
            </div>
          </div>
        </div>

        <div className="flex items-center gap-3">
          {saveStatus && (
            <span className="text-xs text-emerald-600 dark:text-emerald-400 font-medium animate-fade-in">
              ✓ {saveStatus}
            </span>
          )}
          {onOpenAiAssistant && (
            <button
              type="button"
              onClick={onOpenAiAssistant}
              className="flex items-center gap-1.5 px-3 py-1.5 rounded-lg border border-purple-200 dark:border-purple-800 bg-purple-50/70 dark:bg-purple-950/40 text-purple-700 dark:text-purple-300 text-xs font-semibold hover:bg-purple-100 dark:hover:bg-purple-900/60 transition-colors cursor-pointer"
            >
              ✨ AI Co-Builder (MCP)
            </button>
          )}
          <button
            type="button"
            onClick={handleSave}
            className="px-3.5 py-1.5 rounded-lg border border-slate-300 dark:border-slate-700 hover:bg-slate-100 dark:hover:bg-slate-800 text-xs font-semibold cursor-pointer"
          >
            Save Changes
          </button>
          <button
            type="button"
            onClick={() => onOpenPublishedApp(app.slug)}
            className="flex items-center gap-1.5 px-3.5 py-1.5 rounded-lg bg-blue-600 hover:bg-blue-700 text-white text-xs font-bold shadow-sm transition-colors cursor-pointer"
          >
            ↗ Launch Live App
          </button>
        </div>
      </header>

      {/* Primary Builder Tabs Navigation */}
      <div className="bg-white dark:bg-slate-900 border-b border-slate-200 dark:border-slate-800 px-6 flex items-center justify-between">
        <div className="flex items-center gap-6 text-xs font-semibold">
          <button
            type="button"
            onClick={() => setActiveTab("pages")}
            className={`py-3 border-b-2 cursor-pointer transition-colors ${
              activeTab === "pages"
                ? "border-blue-600 text-blue-600 dark:text-blue-400 font-bold"
                : "border-transparent text-slate-500 hover:text-slate-800 dark:hover:text-slate-200"
            }`}
          >
            1. Pages &amp; Interface Canvas ({pages.length} Pages)
          </button>
          <button
            type="button"
            onClick={() => setActiveTab("data")}
            className={`py-3 border-b-2 cursor-pointer transition-colors ${
              activeTab === "data"
                ? "border-blue-600 text-blue-600 dark:text-blue-400 font-bold"
                : "border-transparent text-slate-500 hover:text-slate-800 dark:hover:text-slate-200"
            }`}
          >
            2. Data &amp; Schema ({app.manifest.views[0]?.fields.length || 0} Fields)
          </button>
          <button
            type="button"
            onClick={() => setActiveTab("automations")}
            className={`py-3 border-b-2 cursor-pointer transition-colors ${
              activeTab === "automations"
                ? "border-blue-600 text-blue-600 dark:text-blue-400 font-bold"
                : "border-transparent text-slate-500 hover:text-slate-800 dark:hover:text-slate-200"
            }`}
          >
            3. Governed Automations
          </button>
          <button
            type="button"
            onClick={() => setActiveTab("settings")}
            className={`py-3 border-b-2 cursor-pointer transition-colors ${
              activeTab === "settings"
                ? "border-blue-600 text-blue-600 dark:text-blue-400 font-bold"
                : "border-transparent text-slate-500 hover:text-slate-800 dark:hover:text-slate-200"
            }`}
          >
            4. Governance &amp; Vanity Routing
          </button>
        </div>

        {activeTab === "pages" && (
          <div className="flex items-center gap-2 py-2">
            <span className="text-[11px] font-medium text-slate-400">Add Component:</span>
            {(["stat-metric", "tabular-grid", "kanban-stage", "intake-form", "rich-banner"] as const).map((t) => (
              <button
                key={t}
                type="button"
                onClick={() => handleAddComponent(t)}
                className="px-2.5 py-1 rounded bg-slate-100 dark:bg-slate-800 hover:bg-blue-50 dark:hover:bg-blue-950 text-slate-700 dark:text-slate-300 hover:text-blue-600 dark:hover:text-blue-300 text-[11px] font-medium transition-colors cursor-pointer border border-slate-200 dark:border-slate-700"
              >
                + {t.replace("-", " ")}
              </button>
            ))}
          </div>
        )}
      </div>

      {/* Main Workspace Body */}
      <div className="flex-1 flex overflow-hidden">
        {/* TAB 1: DATA & SCHEMA */}
        {activeTab === "data" && (
          <div className="flex-1 overflow-y-auto p-6">
            <div className="max-w-7xl mx-auto space-y-6">
              <div className="bg-white dark:bg-slate-900 border border-slate-200 dark:border-slate-800 rounded-xl p-5 shadow-xs">
                <h3 className="text-sm font-bold text-slate-900 dark:text-white">
                  TanStack Table Data Source &amp; Schema
                </h3>
                <p className="text-xs text-slate-500 mt-1">
                  Manage source fields, NCES CEDS standard mappings, and linked authoritative datasets.
                </p>
              </div>
              <MultiViewWorkspace
                app={app}
                onBack={() => setActiveTab("pages")}
                onOpenStudio={() => {}}
                onRecordCreated={() => {}}
              />
            </div>
          </div>
        )}

        {/* TAB 2: PAGES & INTERFACE CANVAS */}
        {activeTab === "pages" && (
          <div className="flex-1 flex overflow-hidden">
            {/* Left Page List Hierarchy */}
            <aside className="w-64 bg-white dark:bg-slate-900 border-r border-slate-200 dark:border-slate-800 flex flex-col shrink-0">
              <div className="p-4 border-b border-slate-200 dark:border-slate-800 flex items-center justify-between">
                <span className="text-[11px] font-bold uppercase tracking-wider text-slate-500 dark:text-slate-400">
                  Application Pages
                </span>
                <button
                  type="button"
                  onClick={handleAddPage}
                  className="px-2 py-0.5 rounded bg-blue-50 text-blue-600 dark:bg-blue-950 dark:text-blue-300 text-xs font-bold hover:bg-blue-100 dark:hover:bg-blue-900 cursor-pointer"
                >
                  + Page
                </button>
              </div>
              <div className="flex-1 overflow-y-auto p-3 space-y-1">
                {pages.map((p) => {
                  const isSelected = p.id === activePage.id;
                  return (
                    <button
                      key={p.id}
                      type="button"
                      onClick={() => {
                        setActivePageId(p.id);
                        setSelectedComponentId(null);
                        setIsFlyoutOpen(false);
                      }}
                      className={`w-full text-left px-3 py-2.5 rounded-lg text-xs font-semibold flex items-center justify-between transition-colors cursor-pointer ${
                        isSelected
                          ? "bg-blue-50 text-blue-700 dark:bg-blue-950/70 dark:text-blue-300 border border-blue-200 dark:border-blue-800"
                          : "text-slate-600 dark:text-slate-400 hover:bg-slate-100 dark:hover:bg-slate-800/60"
                      }`}
                    >
                      <div className="flex items-center gap-2 truncate">
                        <span>{p.icon || "📄"}</span>
                        <span className="truncate">{p.title}</span>
                      </div>
                      <span className="text-[10px] font-mono opacity-60">
                        {p.components.length}
                      </span>
                    </button>
                  );
                })}
              </div>
            </aside>

            {/* Visual Component Canvas */}
            <main className="flex-1 overflow-y-auto p-6 bg-slate-100/60 dark:bg-slate-950/40">
              <div className="max-w-6xl mx-auto space-y-6">
                {/* Active Page Header Banner */}
                <div className="bg-white dark:bg-slate-900 border border-slate-200 dark:border-slate-800 rounded-xl p-5 shadow-xs flex items-center justify-between">
                  <div>
                    <div className="flex items-center gap-2">
                      <span className="text-xl">{activePage.icon}</span>
                      <h2 className="text-base font-bold text-slate-900 dark:text-white">
                        {activePage.title}
                      </h2>
                      <span className="text-xs font-mono text-slate-400">/{activePage.slug}</span>
                    </div>
                    {activePage.description && (
                      <p className="text-xs text-slate-500 mt-1">{activePage.description}</p>
                    )}
                  </div>
                  <div className="text-xs text-slate-400">
                    Click any widget below to open configuration flyout
                  </div>
                </div>

                {/* Canvas Grid */}
                <div className="grid grid-cols-6 gap-5">
                  {activePage.components.map((comp) => {
                    const isSelected = comp.id === selectedComponentId;
                    const colSpan =
                      comp.layout.width === "third"
                        ? "col-span-6 md:col-span-2"
                        : comp.layout.width === "half"
                        ? "col-span-6 md:col-span-3"
                        : comp.layout.width === "two-thirds"
                        ? "col-span-6 md:col-span-4"
                        : "col-span-6";

                    return (
                      <div
                        key={comp.id}
                        data-testid={`canvas-component-${comp.id}`}
                        onClick={() => handleSelectComponent(comp)}
                        className={`${colSpan} group relative rounded-xl border transition-all duration-150 cursor-pointer ${
                          isSelected
                            ? "ring-2 ring-blue-500 ring-offset-2 border-blue-500 bg-white dark:bg-slate-900 shadow-md"
                            : "border-slate-200 dark:border-slate-800 bg-white dark:bg-slate-900 hover:border-blue-300 dark:hover:border-blue-700 shadow-xs"
                        }`}
                      >
                        {/* Hover & Selection Badge */}
                        <div className="absolute top-2 right-2 z-10 opacity-0 group-hover:opacity-100 transition-opacity flex items-center gap-1 bg-white/90 dark:bg-slate-800/90 backdrop-blur-xs px-2 py-0.5 rounded border border-slate-200 dark:border-slate-700 text-[10px] font-semibold text-slate-500">
                          <span>{comp.type}</span>
                          <span className="text-blue-500">✎ Configure</span>
                        </div>

                        {/* COMPONENT BODY RENDERING */}
                        {comp.type === "stat-metric" && (
                          <div className="p-5">
                            <span className="text-xs font-medium text-slate-500 dark:text-slate-400">
                              {comp.title}
                            </span>
                            <div className="flex items-baseline justify-between mt-2">
                              <span className="text-2xl font-black text-slate-900 dark:text-white">
                                {comp.config.aggregation === "sum" ? "$3,450,000" : "42"}
                              </span>
                              {comp.config.trend_text && (
                                <span className="text-xs font-bold text-emerald-600 dark:text-emerald-400">
                                  {comp.config.trend_text}
                                </span>
                              )}
                            </div>
                            {comp.config.subtitle && (
                              <p className="text-[11px] text-slate-400 mt-1">{comp.config.subtitle}</p>
                            )}
                          </div>
                        )}

                        {comp.type === "tabular-grid" && (
                          <div className="p-5">
                            <div className="flex items-center justify-between pb-3 border-b border-slate-100 dark:border-slate-800">
                              <h4 className="text-xs font-bold text-slate-900 dark:text-white">
                                {comp.title}
                              </h4>
                              <span className="text-[10px] font-mono text-slate-400">
                                TanStack Table v8 Reactive Grid
                              </span>
                            </div>
                            <div className="mt-3 overflow-x-auto">
                              <table className="w-full text-left text-xs">
                                <thead>
                                  <tr className="border-b border-slate-200 dark:border-slate-800 text-slate-500">
                                    {(comp.config.visible_columns || ["id", "title", "department", "status", "budget"]).map(
                                      (col: string) => (
                                        <th key={col} className="pb-2 font-mono text-[11px] uppercase">
                                          {col}
                                        </th>
                                      )
                                    )}
                                  </tr>
                                </thead>
                                <tbody className="divide-y divide-slate-100 dark:divide-slate-800/60">
                                  <tr>
                                    <td className="py-2 font-mono text-slate-400">APP-001</td>
                                    <td className="py-2 font-semibold">Quantum Optomechanics Qubit Study</td>
                                    <td className="py-2">Physics</td>
                                    <td className="py-2">
                                      <span className="px-2 py-0.5 rounded text-[10px] font-bold bg-amber-50 text-amber-700 dark:bg-amber-950 dark:text-amber-300">
                                        Under Review
                                      </span>
                                    </td>
                                    <td className="py-2 font-mono">$450,000</td>
                                  </tr>
                                  <tr>
                                    <td className="py-2 font-mono text-slate-400">APP-002</td>
                                    <td className="py-2 font-semibold">Neural Stem Cell Regeneration</td>
                                    <td className="py-2">Bioengineering</td>
                                    <td className="py-2">
                                      <span className="px-2 py-0.5 rounded text-[10px] font-bold bg-emerald-50 text-emerald-700 dark:bg-emerald-950 dark:text-emerald-300">
                                        Approved
                                      </span>
                                    </td>
                                    <td className="py-2 font-mono">$820,000</td>
                                  </tr>
                                </tbody>
                              </table>
                            </div>
                          </div>
                        )}

                        {comp.type === "kanban-stage" && (
                          <div className="p-5">
                            <h4 className="text-xs font-bold text-slate-900 dark:text-white mb-3">
                              {comp.title}
                            </h4>
                            <div className="grid grid-cols-3 gap-3">
                              {["Under Review (1)", "Approved (1)", "Funded (0)"].map((stg) => (
                                <div
                                  key={stg}
                                  className="bg-slate-50 dark:bg-slate-800/50 p-3 rounded-lg border border-slate-200 dark:border-slate-700"
                                >
                                  <span className="text-[11px] font-bold text-slate-600 dark:text-slate-300">
                                    {stg}
                                  </span>
                                  <div className="mt-2 p-2 bg-white dark:bg-slate-900 rounded border border-slate-200 dark:border-slate-800 text-[11px]">
                                    <div className="font-semibold text-slate-800 dark:text-slate-200">
                                      Quantum Optomechanics
                                    </div>
                                    <div className="text-[10px] text-slate-400 mt-1">$450,000</div>
                                  </div>
                                </div>
                              ))}
                            </div>
                          </div>
                        )}

                        {comp.type === "intake-form" && (
                          <div className="p-5">
                            <h4 className="text-xs font-bold text-slate-900 dark:text-white mb-3">
                              {comp.title}
                            </h4>
                            <div className="space-y-3">
                              <div>
                                <label className="text-[11px] font-medium text-slate-500">Proposal Title</label>
                                <input
                                  type="text"
                                  disabled
                                  placeholder="Enter research proposal title..."
                                  className="mt-1 w-full px-3 py-1.5 rounded border border-slate-200 dark:border-slate-700 bg-slate-50 dark:bg-slate-800 text-xs"
                                />
                              </div>
                              <div className="grid grid-cols-2 gap-3">
                                <div>
                                  <label className="text-[11px] font-medium text-slate-500">Department</label>
                                  <input
                                    type="text"
                                    disabled
                                    placeholder="Physics / Biology"
                                    className="mt-1 w-full px-3 py-1.5 rounded border border-slate-200 dark:border-slate-700 bg-slate-50 dark:bg-slate-800 text-xs"
                                  />
                                </div>
                                <div>
                                  <label className="text-[11px] font-medium text-slate-500">Requested Budget</label>
                                  <input
                                    type="text"
                                    disabled
                                    placeholder="$0.00"
                                    className="mt-1 w-full px-3 py-1.5 rounded border border-slate-200 dark:border-slate-700 bg-slate-50 dark:bg-slate-800 text-xs"
                                  />
                                </div>
                              </div>
                              <button
                                type="button"
                                disabled
                                className="px-3.5 py-1.5 rounded bg-blue-600/70 text-white font-bold text-xs"
                              >
                                {comp.config.submit_button_label || "Submit Record"}
                              </button>
                            </div>
                          </div>
                        )}

                        {comp.type === "rich-banner" && (
                          <div
                            className={`p-4 rounded-xl border ${
                              comp.config.variant === "warning"
                                ? "bg-amber-50/70 border-amber-200 dark:bg-amber-950/40 dark:border-amber-900 text-amber-800 dark:text-amber-200"
                                : comp.config.variant === "success"
                                ? "bg-emerald-50/70 border-emerald-200 dark:bg-emerald-950/40 dark:border-emerald-900 text-emerald-800 dark:text-emerald-200"
                                : "bg-blue-50/70 border-blue-200 dark:bg-blue-950/40 dark:border-blue-900 text-blue-800 dark:text-blue-200"
                            }`}
                          >
                            <h4 className="text-xs font-bold">{comp.title}</h4>
                            <p className="text-xs mt-1 opacity-90">{comp.config.content}</p>
                          </div>
                        )}
                      </div>
                    );
                  })}
                </div>
              </div>
            </main>

            {/* Flyout Inspector Drawer */}
            <ComponentInspectorFlyout
              component={selectedComponent}
              isOpen={isFlyoutOpen}
              onClose={() => {
                setIsFlyoutOpen(false);
                setSelectedComponentId(null);
              }}
              onUpdateComponent={handleUpdateComponent}
              onDeleteComponent={handleDeleteComponent}
              onMoveComponent={handleMoveComponent}
            />
          </div>
        )}

        {/* TAB 3: WORKFLOW AUTOMATIONS */}
        {activeTab === "automations" && (
          <div className="flex-1 overflow-y-auto p-6">
            <div className="max-w-5xl mx-auto space-y-6">
              <div className="bg-white dark:bg-slate-900 border border-slate-200 dark:border-slate-800 rounded-xl p-5 shadow-xs">
                <h3 className="text-sm font-bold text-slate-900 dark:text-white">
                  Governed Event Triggers &amp; Automations
                </h3>
                <p className="text-xs text-slate-500 mt-1">
                  Configure state transitions, Cedar authorization guards, webhook dispatches, and cryptographic ledger attestation.
                </p>
              </div>
              <WorkflowBuilder
                appSlug={app.slug}
                appTitle={app.title}
                fields={app.manifest.views[0]?.fields || []}
                rules={automationRules}
                onSaveRule={(newRule) => setAutomationRules((prev) => [...prev.filter((r) => r.id !== newRule.id), newRule])}
                onDeleteRule={(id) => setAutomationRules((prev) => prev.filter((r) => r.id !== id))}
                onToggleRule={(id) => setAutomationRules((prev) => prev.map((r) => (r.id === id ? { ...r, enabled: !r.enabled } : r)))}
              />
            </div>
          </div>
        )}

        {/* TAB 4: GOVERNANCE & SETTINGS */}
        {activeTab === "settings" && (
          <div className="flex-1 overflow-y-auto p-6">
            <div className="max-w-4xl mx-auto space-y-6">
              <div className="bg-white dark:bg-slate-900 border border-slate-200 dark:border-slate-800 rounded-xl p-5 shadow-xs space-y-4">
                <h3 className="text-sm font-bold text-slate-900 dark:text-white">
                  Vanity Routing &amp; Domain Binding
                </h3>
                <div className="grid grid-cols-2 gap-4 text-xs">
                  <div>
                    <label className="font-semibold text-slate-600 dark:text-slate-300">Custom Domain Host</label>
                    <input
                      type="text"
                      defaultValue={app.customDomain}
                      className="mt-1 w-full px-3 py-1.5 rounded border border-slate-300 dark:border-slate-700 bg-white dark:bg-slate-800 font-mono"
                    />
                  </div>
                  <div>
                    <label className="font-semibold text-slate-600 dark:text-slate-300">Published URL Path</label>
                    <input
                      type="text"
                      readOnly
                      value={`/app/${app.slug}`}
                      className="mt-1 w-full px-3 py-1.5 rounded border border-slate-200 dark:border-slate-800 bg-slate-50 dark:bg-slate-800/50 font-mono text-slate-500"
                    />
                  </div>
                </div>
              </div>

              <div className="bg-white dark:bg-slate-900 border border-slate-200 dark:border-slate-800 rounded-xl p-5 shadow-xs space-y-4">
                <h3 className="text-sm font-bold text-slate-900 dark:text-white">
                  EduPerson Role Scoping &amp; Cedar ABAC
                </h3>
                <div className="space-y-2 text-xs">
                  <div className="p-3 rounded bg-slate-50 dark:bg-slate-800/50 border border-slate-200 dark:border-slate-700 flex items-center justify-between">
                    <div>
                      <span className="font-bold text-slate-800 dark:text-slate-200">Faculty Role Access</span>
                      <p className="text-slate-500 text-[11px]">Can view, review, and approve research proposals</p>
                    </div>
                    <span className="text-[10px] font-mono px-2 py-0.5 rounded bg-emerald-100 text-emerald-800 dark:bg-emerald-950 dark:text-emerald-300 font-bold">
                      Permitted
                    </span>
                  </div>
                  <div className="p-3 rounded bg-slate-50 dark:bg-slate-800/50 border border-slate-200 dark:border-slate-700 flex items-center justify-between">
                    <div>
                      <span className="font-bold text-slate-800 dark:text-slate-200">Student Role Access</span>
                      <p className="text-slate-500 text-[11px]">Can submit proposals via intake form only</p>
                    </div>
                    <span className="text-[10px] font-mono px-2 py-0.5 rounded bg-blue-100 text-blue-800 dark:bg-blue-950 dark:text-blue-300 font-bold">
                      Restricted
                    </span>
                  </div>
                </div>
              </div>
            </div>
          </div>
        )}
      </div>
    </div>
  );
};
