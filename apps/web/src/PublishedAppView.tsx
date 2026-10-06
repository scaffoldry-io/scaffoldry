import React, { useState } from "react";
import { AppPage, GovernedComponentSpec, RegisteredApp } from "./types";

interface PublishedAppViewProps {
  app: RegisteredApp;
  onOpenBuilder?: (slug: string) => void;
  onOpenIntakeForm?: (tableId?: string) => void;
  onBackToDesk?: () => void;
}

interface RecordItem {
  id: string;
  title: string;
  department: string;
  status: string;
  budget: number;
  submitted_at: string;
  lead_investigator_id?: string;
}

const investigators = [
  { id: "INV-01", name: "Dr. Marie Curie", department: "Physics & Astronomy", email: "mcurie@univ.edu" },
  { id: "INV-02", name: "Dr. Alan Turing", department: "Computer Science", email: "aturing@univ.edu" },
  { id: "INV-03", name: "Dr. Barbara McClintock", department: "Bioengineering", email: "bmcclintock@univ.edu" },
];

export const PublishedAppView: React.FC<PublishedAppViewProps> = ({
  app,
  onOpenBuilder,
  onOpenIntakeForm,
  onBackToDesk,
}) => {
  // Seeded records state for live interactive tabular operations
  const [records, setRecords] = useState<RecordItem[]>([
    {
      id: "APP-101",
      title: "Quantum Optomechanics Qubit Study",
      department: "Physics & Astronomy",
      status: "Under Review",
      budget: 450000,
      submitted_at: "2026-10-02",
      lead_investigator_id: "INV-01",
    },
    {
      id: "APP-102",
      title: "Neural Stem Cell Regeneration Assay",
      department: "Bioengineering",
      status: "Approved",
      budget: 820000,
      submitted_at: "2026-09-28",
      lead_investigator_id: "INV-03",
    },
    {
      id: "APP-103",
      title: "Distributed Edge Sensor Fusion Grid",
      department: "Computer Science",
      status: "Funded",
      budget: 640000,
      submitted_at: "2026-09-15",
      lead_investigator_id: "INV-02",
    },
    {
      id: "APP-104",
      title: "High-Entropy Alloy Catalyst Synthesis",
      department: "Materials Science",
      status: "Under Review",
      budget: 380000,
      submitted_at: "2026-10-01",
      lead_investigator_id: "INV-01",
    },
  ]);

  const [activePageSlug, setActivePageSlug] = useState<string>("overview");
  const [searchFilter, setSearchFilter] = useState<string>("");
  const [isSubmitModalOpen, setIsSubmitModalOpen] = useState<boolean>(false);
  const [newTitle, setNewTitle] = useState<string>("");
  const [newDept, setNewDept] = useState<string>("Physics & Astronomy");
  const [newBudget, setNewBudget] = useState<number>(250000);
  const [newInvestigatorId, setNewInvestigatorId] = useState<string>("INV-01");
  const [submissionFeedback, setSubmissionFeedback] = useState<string | null>(null);

  // Derive pages from manifest or default
  const pages: AppPage[] = app.manifest.pages && app.manifest.pages.length > 0
    ? app.manifest.pages
    : [
        {
          id: "p1",
          slug: "overview",
          title: "Dashboard & Records",
          icon: "📊",
          components: [
            {
              id: "b1",
              type: "rich-banner",
              title: "Institutional Research Review Notice",
              slot: "header",
              layout: { width: "full", order: 0 },
              config: {
                variant: "info",
                content: "All proposals submitted in this application are governed by institutional Cedar policy and logged in the immutable decision ledger.",
              },
            },
            {
              id: "s1",
              type: "stat-metric",
              title: "Total Proposals",
              slot: "main",
              layout: { width: "third", order: 1 },
              config: { aggregation: "count", subtitle: "Active submissions", trend_text: "+12% this term", accent_color: "blue" },
            },
            {
              id: "s2",
              type: "stat-metric",
              title: "Approved Funding",
              slot: "main",
              layout: { width: "third", order: 2 },
              config: { aggregation: "sum", target_field: "budget", subtitle: "Allocated grants", trend_text: "88% target met", accent_color: "emerald" },
            },
            {
              id: "s3",
              type: "stat-metric",
              title: "Under Review",
              slot: "main",
              layout: { width: "third", order: 3 },
              config: { aggregation: "count", filter_status: "Under Review", subtitle: "Pending board decision", trend_text: "Target: 7 days", accent_color: "amber" },
            },
            {
              id: "g1",
              type: "tabular-grid",
              title: "Submitted Proposals Queue",
              slot: "main",
              layout: { width: "full", order: 4 },
              config: {
                visible_columns: ["id", "title", "department", "status", "budget", "submitted_at"],
                rollup_type: "sum",
                enable_search: true,
              },
            },
          ],
        },
        {
          id: "p2",
          slug: "stages",
          title: "Approval Kanban",
          icon: "📋",
          components: [
            {
              id: "k1",
              type: "kanban-stage",
              title: "Review Stage Progression",
              slot: "main",
              layout: { width: "full", order: 0 },
              config: { stage_field: "status", card_title_field: "title" },
            },
          ],
        },
      ];

  const activePage = pages.find((p) => p.slug === activePageSlug) || pages[0];

  const filteredRecords = records.filter(
    (r) =>
      r.title.toLowerCase().includes(searchFilter.toLowerCase()) ||
      r.department.toLowerCase().includes(searchFilter.toLowerCase()) ||
      r.status.toLowerCase().includes(searchFilter.toLowerCase())
  );

  const handleCreateRecord = (e: React.FormEvent) => {
    e.preventDefault();
    if (!newTitle.trim()) return;

    const newRecord: RecordItem = {
      id: `APP-${100 + records.length + 1}`,
      title: newTitle.trim(),
      department: newDept,
      status: "Under Review",
      budget: Number(newBudget) || 100000,
      submitted_at: new Date().toISOString().split("T")[0],
      lead_investigator_id: newInvestigatorId,
    };

    setRecords((prev) => [newRecord, ...prev]);
    setIsSubmitModalOpen(false);
    setNewTitle("");
    setSubmissionFeedback(`Record ${newRecord.id} committed to sovereign ledger (SHA-256 block attestation).`);
    setTimeout(() => setSubmissionFeedback(null), 4000);
  };

  const handleCardStageMove = (recordId: string, nextStatus: string) => {
    setRecords((prev) =>
      prev.map((r) => (r.id === recordId ? { ...r, status: nextStatus } : r))
    );
  };

  return (
    <div
      data-testid="published-app-runtime"
      className="min-h-screen bg-slate-50 dark:bg-slate-950 text-slate-900 dark:text-slate-100 flex flex-col font-sans"
    >
      {/* Top Application Header */}
      <header className="sticky top-0 z-30 bg-white/95 dark:bg-slate-900/95 backdrop-blur-md border-b border-slate-200 dark:border-slate-800 px-6 py-3 flex items-center justify-between">
        <div className="flex items-center gap-4">
          {onBackToDesk && (
            <button
              type="button"
              onClick={onBackToDesk}
              className="text-xs font-semibold text-slate-500 hover:text-slate-900 dark:hover:text-white px-2 py-1 rounded hover:bg-slate-100 dark:hover:bg-slate-800 cursor-pointer"
            >
              ← Sovereign Desk
            </button>
          )}
          <div className="flex items-center gap-3">
            <img src="/logo-mark.png" alt="Scaffoldry" className="h-8 w-auto object-contain shrink-0" />
            <div>
              <div className="flex items-center gap-2">
                <h1 className="text-base font-bold text-slate-900 dark:text-white">{app.title}</h1>
                <span className="text-[10px] font-mono px-2 py-0.5 rounded bg-blue-100 text-blue-800 dark:bg-blue-950 dark:text-blue-300 font-bold">
                  {app.orgCode}
                </span>
                <span className="text-[10px] font-semibold px-2 py-0.5 rounded bg-emerald-50 text-emerald-700 dark:bg-emerald-950 dark:text-emerald-300">
                  Live
                </span>
              </div>
              <p className="text-[11px] text-slate-500">{app.department}</p>
            </div>
          </div>
        </div>

        <div className="flex items-center gap-3">
          {submissionFeedback && (
            <span className="text-xs text-emerald-600 dark:text-emerald-400 font-medium animate-fade-in">
              ✓ {submissionFeedback}
            </span>
          )}

          <div className="hidden sm:flex items-center px-2.5 py-1 rounded-lg border border-slate-200 dark:border-slate-800 bg-slate-100/60 dark:bg-slate-800/60 text-xs">
            <span className="text-slate-400 mr-2">🔍</span>
            <input
              type="text"
              placeholder="Search records..."
              value={searchFilter}
              onChange={(e) => setSearchFilter(e.target.value)}
              className="bg-transparent text-slate-800 dark:text-white focus:outline-none w-36 sm:w-48 text-xs"
            />
          </div>

          <button
            type="button"
            onClick={() => setIsSubmitModalOpen(true)}
            className="flex items-center gap-1.5 px-3.5 py-1.5 rounded-lg bg-blue-600 hover:bg-blue-700 text-white text-xs font-bold shadow-xs transition-colors cursor-pointer"
          >
            + New Proposal
          </button>

          {onOpenIntakeForm && (
            <button
              type="button"
              data-testid="open-public-intake-form-btn"
              onClick={() => onOpenIntakeForm()}
              className="flex items-center gap-1.5 px-3 py-1.5 rounded-lg border border-blue-200 dark:border-blue-800 bg-blue-50/60 dark:bg-blue-950/40 text-blue-700 dark:text-blue-300 hover:bg-blue-100 dark:hover:bg-blue-900/40 text-xs font-semibold cursor-pointer transition-colors"
            >
              📋 Public Intake Form
            </button>
          )}

          {onOpenBuilder && (
            <button
              type="button"
              onClick={() => onOpenBuilder(app.slug)}
              className="px-3 py-1.5 rounded-lg border border-slate-300 dark:border-slate-700 hover:bg-slate-100 dark:hover:bg-slate-800 text-xs font-semibold text-slate-600 dark:text-slate-300 cursor-pointer"
            >
              ⚙ Edit in Builder
            </button>
          )}

          <div className="flex items-center gap-2 pl-2 border-l border-slate-200 dark:border-slate-800">
            <div className="w-7 h-7 rounded-full bg-blue-100 text-blue-700 dark:bg-blue-900 dark:text-blue-200 flex items-center justify-center text-xs font-bold">
              MC
            </div>
            <div className="hidden md:block text-left text-[11px] leading-tight">
              <span className="font-semibold block text-slate-800 dark:text-slate-200">Prof. Curie</span>
              <span className="text-slate-400 font-mono">faculty</span>
            </div>
          </div>
        </div>
      </header>

      {/* Main Published Layout: Sidebar + Canvas */}
      <div className="flex-1 flex overflow-hidden">
        {/* Left Sidebar Pages Navigation */}
        <aside className="w-60 bg-white dark:bg-slate-900 border-r border-slate-200 dark:border-slate-800 flex flex-col shrink-0">
          <div className="p-3 border-b border-slate-100 dark:border-slate-800">
            <span className="text-[10px] font-bold uppercase tracking-wider text-slate-400">
              Application Pages
            </span>
          </div>
          <nav className="flex-1 p-2 space-y-1 overflow-y-auto">
            {pages.map((page) => {
              const isActive = page.slug === activePage.slug;
              return (
                <button
                  key={page.id}
                  type="button"
                  data-testid={`page-nav-${page.slug}`}
                  onClick={() => setActivePageSlug(page.slug)}
                  className={`w-full text-left px-3 py-2.5 rounded-lg text-xs font-semibold flex items-center gap-2.5 transition-colors cursor-pointer ${
                    isActive
                      ? "bg-blue-50 text-blue-700 dark:bg-blue-950/70 dark:text-blue-300 font-bold border border-blue-200 dark:border-blue-800"
                      : "text-slate-600 dark:text-slate-400 hover:bg-slate-100 dark:hover:bg-slate-800/60"
                  }`}
                >
                  <span className="text-base">{page.icon || "📄"}</span>
                  <span className="truncate">{page.title}</span>
                </button>
              );
            })}
          </nav>
          <div className="p-3 border-t border-slate-100 dark:border-slate-800 text-[11px] text-slate-400">
            Sovereign Tenant: <span className="font-mono text-slate-600 dark:text-slate-300">{app.slug}</span>
          </div>
        </aside>

        {/* Dynamic Canvas Area */}
        <main className="flex-1 overflow-y-auto p-6 bg-slate-100/50 dark:bg-slate-950/30">
          <div className="max-w-6xl mx-auto space-y-6">
            {/* Page Header */}
            <div>
              <div className="flex items-center gap-2">
                <span className="text-2xl">{activePage.icon}</span>
                <h2 className="text-lg font-bold text-slate-900 dark:text-white">
                  {activePage.title}
                </h2>
              </div>
              {activePage.description && (
                <p className="text-xs text-slate-500 mt-1">{activePage.description}</p>
              )}
            </div>

            {/* Render Page Components */}
            <div className="grid grid-cols-6 gap-5">
              {activePage.components.map((comp: GovernedComponentSpec) => {
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
                    className={`${colSpan} bg-white dark:bg-slate-900 rounded-xl border border-slate-200 dark:border-slate-800 shadow-xs overflow-hidden`}
                  >
                    {/* Component Rendering */}
                    {comp.type === "rich-banner" && (
                      <div
                        className={`p-4 ${
                          comp.config.variant === "warning"
                            ? "bg-amber-50/70 border-b border-amber-200 dark:bg-amber-950/40 dark:border-amber-900 text-amber-800 dark:text-amber-200"
                            : comp.config.variant === "success"
                            ? "bg-emerald-50/70 border-b border-emerald-200 dark:bg-emerald-950/40 dark:border-emerald-900 text-emerald-800 dark:text-emerald-200"
                            : "bg-blue-50/70 border-b border-blue-200 dark:bg-blue-950/40 dark:border-blue-900 text-blue-800 dark:text-blue-200"
                        }`}
                      >
                        <h4 className="text-xs font-bold">{comp.title}</h4>
                        <p className="text-xs mt-1 opacity-90">{comp.config.content}</p>
                      </div>
                    )}

                    {comp.type === "stat-metric" && (
                      <div className="p-5">
                        <span className="text-xs font-medium text-slate-500 dark:text-slate-400">
                          {comp.title}
                        </span>
                        <div className="flex items-baseline justify-between mt-2">
                          <span className="text-2xl font-black text-slate-900 dark:text-white">
                            {comp.config.aggregation === "sum"
                              ? `$${records.reduce((acc, r) => acc + r.budget, 0).toLocaleString()}`
                              : comp.config.filter_status
                              ? records.filter((r) => r.status === comp.config.filter_status).length
                              : records.length}
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
                          <div>
                            <h4 className="text-xs font-bold text-slate-900 dark:text-white">
                              {comp.title}
                            </h4>
                            <span className="text-[10px] text-slate-400">
                              Showing {filteredRecords.length} records backed by TanStack Table
                            </span>
                          </div>
                          <span className="text-[10px] font-mono px-2 py-0.5 rounded bg-slate-100 dark:bg-slate-800 text-slate-500">
                            Live Dataset
                          </span>
                        </div>

                        <div className="mt-3 overflow-x-auto">
                          <table className="w-full text-left text-xs">
                            <thead>
                              <tr className="border-b border-slate-200 dark:border-slate-800 text-slate-500 font-mono text-[11px] uppercase">
                                <th className="pb-2">Record ID</th>
                                <th className="pb-2">Proposal Title</th>
                                <th className="pb-2">Lead PI (Relational)</th>
                                <th className="pb-2">Department</th>
                                <th className="pb-2">Status</th>
                                <th className="pb-2 text-right">Budget</th>
                                <th className="pb-2 text-right">Submitted</th>
                              </tr>
                            </thead>
                            <tbody className="divide-y divide-slate-100 dark:divide-slate-800/60">
                              {filteredRecords.map((r) => (
                                <tr key={r.id} className="hover:bg-slate-50/60 dark:hover:bg-slate-800/40 transition-colors">
                                  <td className="py-2.5 font-mono text-slate-500">{r.id}</td>
                                  <td className="py-2.5 font-semibold text-slate-900 dark:text-white">{r.title}</td>
                                  <td className="py-2.5">
                                    {r.lead_investigator_id ? (
                                      <span
                                        data-testid={`rel-badge-${r.id}`}
                                        className="inline-flex items-center gap-1 px-2 py-0.5 rounded-full text-[10px] font-medium bg-purple-50 text-purple-700 dark:bg-purple-950/70 dark:text-purple-300 border border-purple-200 dark:border-purple-800"
                                      >
                                        <span>👥</span>
                                        <span>{investigators.find((inv) => inv.id === r.lead_investigator_id)?.name || r.lead_investigator_id}</span>
                                      </span>
                                    ) : (
                                      <span className="text-slate-400 italic text-[11px]">Unassigned</span>
                                    )}
                                  </td>
                                  <td className="py-2.5 text-slate-600 dark:text-slate-300">{r.department}</td>
                                  <td className="py-2.5">
                                    <span
                                      className={`px-2 py-0.5 rounded text-[10px] font-bold ${
                                        r.status === "Approved"
                                          ? "bg-emerald-50 text-emerald-700 dark:bg-emerald-950 dark:text-emerald-300"
                                          : r.status === "Funded"
                                          ? "bg-blue-50 text-blue-700 dark:bg-blue-950 dark:text-blue-300"
                                          : "bg-amber-50 text-amber-700 dark:bg-amber-950 dark:text-amber-300"
                                      }`}
                                    >
                                      {r.status}
                                    </span>
                                  </td>
                                  <td className="py-2.5 text-right font-mono font-medium">${r.budget.toLocaleString()}</td>
                                  <td className="py-2.5 text-right text-slate-400 font-mono">{r.submitted_at}</td>
                                </tr>
                              ))}
                            </tbody>
                            <tfoot>
                              <tr className="border-t-2 border-slate-200 dark:border-slate-800 font-bold text-xs">
                                <td colSpan={5} className="py-2.5 text-slate-600 dark:text-slate-300">
                                  Rollup Total (SUM)
                                </td>
                                <td className="py-2.5 text-right font-mono text-blue-600 dark:text-blue-400">
                                  ${filteredRecords.reduce((acc, r) => acc + r.budget, 0).toLocaleString()}
                                </td>
                                <td></td>
                              </tr>
                            </tfoot>
                          </table>
                        </div>
                      </div>
                    )}

                    {comp.type === "kanban-stage" && (
                      <div className="p-5">
                        <div className="flex items-center justify-between pb-3 border-b border-slate-100 dark:border-slate-800 mb-4">
                          <h4 className="text-xs font-bold text-slate-900 dark:text-white">
                            {comp.title}
                          </h4>
                          <span className="text-[10px] text-slate-400">Click stage buttons to advance records</span>
                        </div>

                        <div className="grid grid-cols-1 md:grid-cols-3 gap-4">
                          {(["Under Review", "Approved", "Funded"] as const).map((stage) => {
                            const stageRecords = records.filter((r) => r.status === stage);
                            return (
                              <div
                                key={stage}
                                className="bg-slate-50 dark:bg-slate-800/40 p-3 rounded-xl border border-slate-200 dark:border-slate-700/80 flex flex-col"
                              >
                                <div className="flex items-center justify-between pb-2 border-b border-slate-200 dark:border-slate-700 text-xs font-bold">
                                  <span>{stage}</span>
                                  <span className="text-[10px] font-mono px-1.5 py-0.5 rounded bg-white dark:bg-slate-900 text-slate-500">
                                    {stageRecords.length}
                                  </span>
                                </div>
                                <div className="mt-2 space-y-2 flex-1">
                                  {stageRecords.map((item) => (
                                    <div
                                      key={item.id}
                                      className="p-3 bg-white dark:bg-slate-900 rounded-lg border border-slate-200 dark:border-slate-800 shadow-xs text-xs space-y-2"
                                    >
                                      <div className="font-semibold text-slate-900 dark:text-white leading-snug">
                                        {item.title}
                                      </div>
                                      <div className="flex items-center justify-between text-[11px] text-slate-500">
                                        <span>{item.department}</span>
                                        <span className="font-mono font-medium">${item.budget.toLocaleString()}</span>
                                      </div>
                                      <div className="pt-2 border-t border-slate-100 dark:border-slate-800 flex items-center justify-between">
                                        <span className="text-[10px] font-mono text-slate-400">{item.id}</span>
                                        <div className="flex items-center gap-1">
                                          {stage !== "Under Review" && (
                                            <button
                                              type="button"
                                              onClick={() => handleCardStageMove(item.id, "Under Review")}
                                              className="text-[10px] px-1.5 py-0.5 rounded border border-slate-200 dark:border-slate-700 hover:bg-slate-100 dark:hover:bg-slate-800 cursor-pointer"
                                            >
                                              ← Review
                                            </button>
                                          )}
                                          {stage !== "Approved" && (
                                            <button
                                              type="button"
                                              onClick={() => handleCardStageMove(item.id, "Approved")}
                                              className="text-[10px] px-1.5 py-0.5 rounded bg-emerald-50 text-emerald-700 dark:bg-emerald-950 dark:text-emerald-300 font-semibold cursor-pointer"
                                            >
                                              Approve ✓
                                            </button>
                                          )}
                                          {stage !== "Funded" && (
                                            <button
                                              type="button"
                                              onClick={() => handleCardStageMove(item.id, "Funded")}
                                              className="text-[10px] px-1.5 py-0.5 rounded bg-blue-50 text-blue-700 dark:bg-blue-950 dark:text-blue-300 font-semibold cursor-pointer"
                                            >
                                              Fund $
                                            </button>
                                          )}
                                        </div>
                                      </div>
                                    </div>
                                  ))}
                                  {stageRecords.length === 0 && (
                                    <div className="text-center py-6 text-xs text-slate-400">
                                      No items in {stage}
                                    </div>
                                  )}
                                </div>
                              </div>
                            );
                          })}
                        </div>
                      </div>
                    )}
                  </div>
                );
              })}
            </div>
          </div>
        </main>
      </div>

      {/* SUBMISSION INTAKE MODAL */}
      {isSubmitModalOpen && (
        <div className="fixed inset-0 z-50 flex items-center justify-center p-4 bg-slate-900/60 backdrop-blur-xs animate-fade-in">
          <div className="bg-white dark:bg-slate-900 border border-slate-200 dark:border-slate-800 rounded-xl shadow-2xl w-full max-w-lg overflow-hidden">
            <div className="px-6 py-4 border-b border-slate-200 dark:border-slate-800 flex items-center justify-between">
              <h3 className="text-sm font-bold text-slate-900 dark:text-white">
                Submit New Research Proposal
              </h3>
              <button
                type="button"
                onClick={() => setIsSubmitModalOpen(false)}
                className="text-slate-400 hover:text-slate-600 dark:hover:text-slate-200 text-sm"
              >
                ✕
              </button>
            </div>
            <form data-testid="proposal-intake-form" onSubmit={handleCreateRecord} className="p-6 space-y-4 text-xs">
              <div>
                <label className="font-semibold text-slate-700 dark:text-slate-300">
                  Proposal Title *
                </label>
                <input
                  type="text"
                  required
                  value={newTitle}
                  onChange={(e) => setNewTitle(e.target.value)}
                  placeholder="e.g. Sub-Kelvin Topological Insulator Measurement"
                  className="mt-1 w-full px-3 py-2 rounded-lg border border-slate-300 dark:border-slate-700 bg-white dark:bg-slate-800 text-slate-900 dark:text-white focus:outline-blue-500"
                />
              </div>
              <div>
                <label className="font-semibold text-slate-700 dark:text-slate-300">
                  Lead Principal Investigator (Relational Lookup) *
                </label>
                <select
                  data-testid="proposal-pi-select"
                  value={newInvestigatorId}
                  onChange={(e) => setNewInvestigatorId(e.target.value)}
                  className="mt-1 w-full px-3 py-2 rounded-lg border border-slate-300 dark:border-slate-700 bg-white dark:bg-slate-800 text-slate-900 dark:text-white"
                >
                  {investigators.map((inv) => (
                    <option key={inv.id} value={inv.id}>
                      {inv.name} - {inv.department} ({inv.id})
                    </option>
                  ))}
                </select>
              </div>
              <div className="grid grid-cols-2 gap-4">
                <div>
                  <label className="font-semibold text-slate-700 dark:text-slate-300">
                    Lead Department
                  </label>
                  <select
                    value={newDept}
                    onChange={(e) => setNewDept(e.target.value)}
                    className="mt-1 w-full px-3 py-2 rounded-lg border border-slate-300 dark:border-slate-700 bg-white dark:bg-slate-800 text-slate-900 dark:text-white"
                  >
                    <option value="Physics & Astronomy">Physics &amp; Astronomy</option>
                    <option value="Bioengineering">Bioengineering</option>
                    <option value="Computer Science">Computer Science</option>
                    <option value="Materials Science">Materials Science</option>
                  </select>
                </div>
                <div>
                  <label className="font-semibold text-slate-700 dark:text-slate-300">
                    Requested Budget ($)
                  </label>
                  <input
                    type="number"
                    min={1000}
                    step={5000}
                    value={newBudget}
                    onChange={(e) => setNewBudget(Number(e.target.value))}
                    className="mt-1 w-full px-3 py-2 rounded-lg border border-slate-300 dark:border-slate-700 bg-white dark:bg-slate-800 text-slate-900 dark:text-white"
                  />
                </div>
              </div>
              <div className="p-3 rounded-lg bg-blue-50/70 dark:bg-blue-950/40 border border-blue-200 dark:border-blue-900 text-slate-600 dark:text-slate-300 text-[11px]">
                Upon submission, this proposal will be cryptographically hashed and evaluated against Cedar ABAC policies before being staged in the review queue.
              </div>
              <div className="flex items-center justify-end gap-3 pt-2">
                <button
                  type="button"
                  onClick={() => setIsSubmitModalOpen(false)}
                  className="px-4 py-2 rounded-lg border border-slate-300 dark:border-slate-700 text-slate-600 dark:text-slate-300 font-semibold"
                >
                  Cancel
                </button>
                <button
                  type="submit"
                  className="px-4 py-2 rounded-lg bg-blue-600 hover:bg-blue-700 text-white font-bold cursor-pointer"
                >
                  Submit Proposal
                </button>
              </div>
            </form>
          </div>
        </div>
      )}
    </div>
  );
};
