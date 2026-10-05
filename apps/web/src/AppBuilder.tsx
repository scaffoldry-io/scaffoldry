import React, { useState } from "react";
import {
  AppManifest,
  AppPage,
  AppTable,
  GovernedComponentSpec,
  GovernedComponentType,
  RegisteredApp,
  TableRelationship,
  WorkflowAutomationRule,
} from "./types";
import { ComponentInspectorFlyout } from "./ComponentInspectorFlyout";
import { WorkflowBuilder } from "./WorkflowBuilder";

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

  // Multi-Table State & Relational Schema
  const defaultTables: AppTable[] = app.manifest.tables && app.manifest.tables.length > 0
    ? app.manifest.tables
    : [
        {
          id: "tbl-proposals",
          name: "Research Proposals",
          slug: "proposals",
          icon: "📑",
          primary_field: "title",
          description: "Active grant proposals and board evaluations",
          fields: [
            { name: "id", label: "Proposal ID", field_type: "Text", required: true, ferpa_sensitive: false },
            { name: "title", label: "Proposal Title", field_type: "Text", required: true, ferpa_sensitive: false },
            { name: "department", label: "Department", field_type: "Text", required: true, ferpa_sensitive: false },
            {
              name: "lead_investigator_id",
              label: "Lead Investigator",
              field_type: "Relation",
              required: true,
              ferpa_sensitive: false,
              target_table_id: "tbl-investigators",
              target_display_field: "name",
            },
            { name: "status", label: "Review Status", field_type: "Select", required: true, ferpa_sensitive: false },
            { name: "budget", label: "Budget ($)", field_type: "Number", required: true, ferpa_sensitive: false },
            { name: "submitted_at", label: "Submitted", field_type: "Date", required: true, ferpa_sensitive: false },
          ],
          records: [
            { id: "APP-001", title: "Quantum Optomechanics Qubit Study", department: "Physics", lead_investigator_id: "INV-01", status: "Under Review", budget: 450000, submitted_at: "2026-10-02" },
            { id: "APP-002", title: "Neural Stem Cell Regeneration", department: "Bioengineering", lead_investigator_id: "INV-02", status: "Approved", budget: 820000, submitted_at: "2026-09-28" },
            { id: "APP-003", title: "Edge Sensor Fusion Lattice", department: "Computer Science", lead_investigator_id: "INV-02", status: "Funded", budget: 640000, submitted_at: "2026-09-15" },
            { id: "APP-004", title: "High-Entropy Alloy Catalyst Synthesis", department: "Materials Science", lead_investigator_id: "INV-03", status: "Under Review", budget: 380000, submitted_at: "2026-10-01" },
          ],
        },
        {
          id: "tbl-investigators",
          name: "Principal Investigators",
          slug: "investigators",
          icon: "👥",
          primary_field: "name",
          description: "Tenured and tenure-track faculty research leaders",
          fields: [
            { name: "id", label: "Investigator ID", field_type: "Text", required: true, ferpa_sensitive: false },
            { name: "name", label: "Faculty Name", field_type: "Text", required: true, ferpa_sensitive: false },
            { name: "email", label: "Academic Email", field_type: "Text", required: true, ferpa_sensitive: false },
            { name: "department", label: "Department", field_type: "Text", required: true, ferpa_sensitive: false },
            { name: "specialty", label: "Specialty", field_type: "Text", required: false, ferpa_sensitive: false },
            { name: "active_grants_count", label: "Active Grants", field_type: "Number", required: false, ferpa_sensitive: false },
          ],
          records: [
            { id: "INV-01", name: "Dr. Marie Curie", email: "curie@science.state.edu", department: "Physics", specialty: "Radioactive & Quantum Matter", active_grants_count: 3 },
            { id: "INV-02", name: "Dr. Alan Turing", email: "turing@science.state.edu", department: "Computer Science", specialty: "Automata & Cryptography", active_grants_count: 2 },
            { id: "INV-03", name: "Dr. Barbara McClintock", email: "barbara@science.state.edu", department: "Bioengineering", specialty: "Transposable Genetics", active_grants_count: 4 },
          ],
        },
        {
          id: "tbl-allocations",
          name: "Budget Allocations",
          slug: "allocations",
          icon: "💰",
          primary_field: "id",
          description: "Grant expenditure allocations and quarterly disbursement tranches",
          fields: [
            { name: "id", label: "Allocation ID", field_type: "Text", required: true, ferpa_sensitive: false },
            {
              name: "proposal_id",
              label: "Linked Proposal",
              field_type: "Relation",
              required: true,
              ferpa_sensitive: false,
              target_table_id: "tbl-proposals",
              target_display_field: "title",
            },
            { name: "fiscal_quarter", label: "Fiscal Quarter", field_type: "Text", required: true, ferpa_sensitive: false },
            { name: "allocated_amount", label: "Disbursed Amount ($)", field_type: "Number", required: true, ferpa_sensitive: false },
            { name: "disbursement_status", label: "Status", field_type: "Select", required: true, ferpa_sensitive: false },
          ],
          records: [
            { id: "ALC-101", proposal_id: "APP-001", fiscal_quarter: "Q1-2026", allocated_amount: 150000, disbursement_status: "Disbursed" },
            { id: "ALC-102", proposal_id: "APP-001", fiscal_quarter: "Q2-2026", allocated_amount: 300000, disbursement_status: "Approved" },
            { id: "ALC-103", proposal_id: "APP-002", fiscal_quarter: "Q1-2026", allocated_amount: 400000, disbursement_status: "Disbursed" },
            { id: "ALC-104", proposal_id: "APP-002", fiscal_quarter: "Q2-2026", allocated_amount: 420000, disbursement_status: "Pending Review" },
          ],
        },
      ];

  const defaultRelationships: TableRelationship[] = app.manifest.relationships && app.manifest.relationships.length > 0
    ? app.manifest.relationships
    : [
        {
          id: "rel-1",
          name: "Proposal Lead Investigator",
          source_table_id: "tbl-proposals",
          target_table_id: "tbl-investigators",
          source_field: "lead_investigator_id",
          target_field: "id",
          relationship_type: "ManyToOne",
          display_field: "name",
        },
        {
          id: "rel-2",
          name: "Allocation Proposal Binding",
          source_table_id: "tbl-allocations",
          target_table_id: "tbl-proposals",
          source_field: "proposal_id",
          target_field: "id",
          relationship_type: "ManyToOne",
          display_field: "title",
        },
      ];

  const [tables, setTables] = useState<AppTable[]>(defaultTables);
  const [relationships] = useState<TableRelationship[]>(defaultRelationships);
  const [activeTableId, setActiveTableId] = useState<string>("tbl-proposals");
  const [tableSearchFilter, setTableSearchFilter] = useState<string>("");
  const [isAddTableModalOpen, setIsAddTableModalOpen] = useState<boolean>(false);
  const [newTableName, setNewTableName] = useState<string>("");
  const [newTableIcon, setNewTableIcon] = useState<string>("📋");

  const activeTable = tables.find((t) => t.id === activeTableId) || tables[0];

  // Helper to resolve linked record value from another table
  const resolveLinkedDisplay = (targetTableId: string, recordId: string, displayField: string = "name"): string => {
    const targetTable = tables.find((t) => t.id === targetTableId);
    if (!targetTable || !targetTable.records) return recordId;
    const found = targetTable.records.find((r) => r.id === recordId);
    return found ? String(found[displayField] || found.name || found.title || recordId) : recordId;
  };

  const handleUpdateRecordField = (recordId: string, fieldName: string, newValue: any) => {
    setTables((prev) =>
      prev.map((t) => {
        if (t.id !== activeTable.id) return t;
        const updatedRecords = (t.records || []).map((r) =>
          r.id === recordId ? { ...r, [fieldName]: newValue } : r
        );
        return { ...t, records: updatedRecords };
      })
    );
  };

  const handleAddNewTable = (e: React.FormEvent) => {
    e.preventDefault();
    if (!newTableName.trim()) return;

    const newSlug = newTableName.toLowerCase().replace(/\s+/g, "-");
    const newId = `tbl-${Date.now().toString().slice(-4)}`;
    const created: AppTable = {
      id: newId,
      name: newTableName.trim(),
      slug: newSlug,
      icon: newTableIcon,
      primary_field: "name",
      description: `User-defined table for ${newTableName.trim()}`,
      fields: [
        { name: "id", label: "Record ID", field_type: "Text", required: true, ferpa_sensitive: false },
        { name: "name", label: "Name / Title", field_type: "Text", required: true, ferpa_sensitive: false },
        { name: "created_at", label: "Created Date", field_type: "Date", required: true, ferpa_sensitive: false },
      ],
      records: [
        { id: `REC-01`, name: `Sample ${newTableName.trim()} Record`, created_at: "2026-10-04" },
      ],
    };

    setTables((prev) => [...prev, created]);
    setActiveTableId(newId);
    setIsAddTableModalOpen(false);
    setNewTableName("");
  };

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

  // Default pages hierarchy
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
                source_table_id: "tbl-proposals",
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
                source_table_id: "tbl-proposals",
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
                source_table_id: "tbl-proposals",
              },
            },
            {
              id: "comp-grid-1",
              type: "tabular-grid",
              title: "Governed Proposals Registry",
              slot: "main",
              layout: { width: "full", order: 4 },
              config: {
                source_table_id: "tbl-proposals",
                visible_columns: ["id", "title", "department", "lead_investigator_id", "status", "budget", "submitted_at"],
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
                source_table_id: "tbl-proposals",
                stage_field: "status",
                card_title_field: "title",
              },
            },
          ],
        },
        {
          id: "page-investigators",
          slug: "faculty",
          title: "Investigators Directory",
          icon: "👥",
          description: "Principal investigators and research faculty directory",
          components: [
            {
              id: "comp-grid-inv",
              type: "tabular-grid",
              title: "Principal Investigators Table",
              slot: "main",
              layout: { width: "full", order: 0 },
              config: {
                source_table_id: "tbl-investigators",
                visible_columns: ["id", "name", "email", "department", "specialty", "active_grants_count"],
                rollup_type: "count",
                enable_search: true,
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
        source_table_id: activeTable.id,
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
      tables,
      relationships,
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
                <span className="text-[10px] font-semibold px-2 py-0.5 rounded bg-purple-50 text-purple-700 dark:bg-purple-950 dark:text-purple-300 font-mono">
                  {tables.length} Tables · {relationships.length} Relations
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
            data-testid="tab-btn-pages"
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
            data-testid="tab-btn-data"
            onClick={() => setActiveTab("data")}
            className={`py-3 border-b-2 cursor-pointer transition-colors ${
              activeTab === "data"
                ? "border-blue-600 text-blue-600 dark:text-blue-400 font-bold"
                : "border-transparent text-slate-500 hover:text-slate-800 dark:hover:text-slate-200"
            }`}
          >
            2. Multi-Table Schema &amp; Relations ({tables.length} Tables)
          </button>
          <button
            type="button"
            data-testid="tab-btn-automations"
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
            data-testid="tab-btn-settings"
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
        {/* TAB 1: MULTI-TABLE SCHEMA & RELATIONS */}
        {activeTab === "data" && (
          <div className="flex-1 overflow-y-auto p-6 space-y-6">
            <div className="max-w-7xl mx-auto space-y-6">
              {/* TOP TABLE SWITCHER TABS */}
              <div className="bg-white dark:bg-slate-900 border border-slate-200 dark:border-slate-800 rounded-xl p-3 shadow-xs flex items-center justify-between flex-wrap gap-2">
                <div className="flex items-center gap-2 overflow-x-auto">
                  {tables.map((tbl) => {
                    const isSelected = tbl.id === activeTable.id;
                    const recordCount = (tbl.records || []).length;
                    return (
                      <button
                        key={tbl.id}
                        type="button"
                        data-testid={`table-tab-${tbl.slug}`}
                        onClick={() => {
                          setActiveTableId(tbl.id);
                          setTableSearchFilter("");
                        }}
                        className={`flex items-center gap-2 px-3 py-2 rounded-lg text-xs font-semibold transition-all cursor-pointer ${
                          isSelected
                            ? "bg-blue-50 text-blue-700 dark:bg-blue-950 dark:text-blue-300 border border-blue-200 dark:border-blue-800 shadow-xs"
                            : "text-slate-600 dark:text-slate-400 hover:bg-slate-100 dark:hover:bg-slate-800 border border-transparent"
                        }`}
                      >
                        <span>{tbl.icon || "📑"}</span>
                        <span>{tbl.name}</span>
                        <span className="text-[10px] font-mono px-1.5 py-0.2 rounded-full bg-slate-200 dark:bg-slate-800 text-slate-600 dark:text-slate-400">
                          {recordCount}
                        </span>
                      </button>
                    );
                  })}
                  <button
                    type="button"
                    onClick={() => setIsAddTableModalOpen(true)}
                    className="flex items-center gap-1.5 px-3 py-2 rounded-lg border border-dashed border-slate-300 dark:border-slate-700 text-slate-500 hover:text-blue-600 dark:hover:text-blue-400 hover:border-blue-400 text-xs font-semibold cursor-pointer transition-colors"
                  >
                    <span>+</span>
                    <span>Add Table</span>
                  </button>
                </div>

                <div className="flex items-center gap-2 text-xs">
                  <span className="text-slate-400">Active Relationships:</span>
                  <span className="font-mono font-bold px-2 py-0.5 rounded bg-purple-100 text-purple-800 dark:bg-purple-950 dark:text-purple-300">
                    {relationships.length} Linked Foreign Keys
                  </span>
                </div>
              </div>

              {/* RELATIONSHIP LATTICE OVERVIEW BANNER */}
              <div className="bg-purple-50/60 dark:bg-purple-950/30 border border-purple-200 dark:border-purple-800/80 rounded-xl p-4">
                <div className="flex items-center justify-between">
                  <div className="flex items-center gap-2">
                    <span className="text-purple-600 dark:text-purple-400 font-bold text-xs uppercase tracking-wider">
                      🔗 Relational Foreign Key Lattice
                    </span>
                  </div>
                  <span className="text-[11px] text-purple-700 dark:text-purple-300 font-medium">
                    All relation fields resolve linked record labels interactively
                  </span>
                </div>
                <div className="flex flex-wrap gap-2 mt-2">
                  {relationships.map((rel) => {
                    const srcTable = tables.find((t) => t.id === rel.source_table_id);
                    const tgtTable = tables.find((t) => t.id === rel.target_table_id);
                    return (
                      <div
                        key={rel.id}
                        className="inline-flex items-center gap-1.5 px-2.5 py-1 rounded-md bg-white dark:bg-slate-900 border border-purple-200 dark:border-purple-800 text-xs shadow-xs"
                      >
                        <span className="font-semibold text-slate-800 dark:text-slate-200">
                          {srcTable?.name || rel.source_table_id}.{rel.source_field}
                        </span>
                        <span className="text-purple-500 font-mono">→</span>
                        <span className="font-semibold text-purple-700 dark:text-purple-300">
                          {tgtTable?.name || rel.target_table_id}.{rel.display_field}
                        </span>
                        <span className="text-[10px] text-slate-400 font-mono">({rel.relationship_type})</span>
                      </div>
                    );
                  })}
                </div>
              </div>

              {/* ACTIVE TABLE SCHEMA & RECORDS GRID */}
              <div className="bg-white dark:bg-slate-900 border border-slate-200 dark:border-slate-800 rounded-xl p-5 shadow-xs space-y-4">
                <div className="flex items-center justify-between">
                  <div>
                    <div className="flex items-center gap-2">
                      <span className="text-lg">{activeTable.icon || "📑"}</span>
                      <h3 className="text-sm font-bold text-slate-900 dark:text-white">
                        {activeTable.name}
                      </h3>
                      <span className="text-xs font-mono text-slate-400">({activeTable.slug})</span>
                    </div>
                    {activeTable.description && (
                      <p className="text-xs text-slate-500 mt-0.5">{activeTable.description}</p>
                    )}
                  </div>
                  <div className="flex items-center gap-3">
                    <input
                      type="text"
                      placeholder={`Search ${activeTable.name}...`}
                      value={tableSearchFilter}
                      onChange={(e) => setTableSearchFilter(e.target.value)}
                      className="px-2.5 py-1 rounded-lg border border-slate-300 dark:border-slate-700 bg-slate-50 dark:bg-slate-800 text-xs text-slate-800 dark:text-white focus:outline-blue-500"
                    />
                  </div>
                </div>

                {/* Table Columns and Relational Types List */}
                <div className="flex items-center gap-2 overflow-x-auto pb-2 border-b border-slate-100 dark:border-slate-800">
                  <span className="text-[11px] font-bold text-slate-400 shrink-0 uppercase tracking-wider">
                    Columns:
                  </span>
                  {activeTable.fields.map((f) => (
                    <span
                      key={f.name}
                      className="inline-flex items-center gap-1.5 px-2 py-0.5 rounded text-[11px] font-medium bg-slate-100 dark:bg-slate-800 text-slate-700 dark:text-slate-300 border border-slate-200 dark:border-slate-700"
                    >
                      <span className="font-semibold">{f.label}</span>
                      <span
                        className={`text-[9px] uppercase px-1 rounded font-bold ${
                          f.field_type === "Relation"
                            ? "bg-purple-100 text-purple-800 dark:bg-purple-900 dark:text-purple-200"
                            : "bg-slate-200 dark:bg-slate-700 text-slate-600 dark:text-slate-300"
                        }`}
                      >
                        {f.field_type === "Relation" ? "Relation 🔗" : f.field_type}
                      </span>
                    </span>
                  ))}
                </div>

                {/* TanStack Interactive Records Grid with Relational Resolution */}
                <div className="overflow-x-auto border border-slate-200 dark:border-slate-800 rounded-lg">
                  <table className="w-full text-left text-xs">
                    <thead>
                      <tr className="bg-slate-50 dark:bg-slate-800/70 border-b border-slate-200 dark:border-slate-800 text-slate-600 dark:text-slate-400 font-mono text-[11px] uppercase">
                        {activeTable.fields.map((field) => (
                          <th key={field.name} className="py-2.5 px-3 font-semibold">
                            <div className="flex items-center gap-1">
                              <span>{field.label}</span>
                              {field.field_type === "Relation" && (
                                <span className="text-purple-500" title="Relational Linked Record">🔗</span>
                              )}
                            </div>
                          </th>
                        ))}
                      </tr>
                    </thead>
                    <tbody className="divide-y divide-slate-100 dark:divide-slate-800">
                      {(activeTable.records || [])
                        .filter((rec) => {
                          if (!tableSearchFilter.trim()) return true;
                          return Object.values(rec).some((val) =>
                            String(val).toLowerCase().includes(tableSearchFilter.toLowerCase())
                          );
                        })
                        .map((record) => (
                          <tr
                            key={record.id}
                            className="hover:bg-slate-50/70 dark:hover:bg-slate-800/50 transition-colors"
                          >
                            {activeTable.fields.map((field) => {
                              const cellValue = record[field.name];

                              // RELATIONAL LOOKUP FIELD RESOLUTION
                              if (field.field_type === "Relation" && field.target_table_id) {
                                const targetTable = tables.find((t) => t.id === field.target_table_id);
                                const targetRecords = targetTable?.records || [];

                                return (
                                  <td key={field.name} className="py-2.5 px-3">
                                    <div className="relative inline-block">
                                      <select
                                        value={cellValue || ""}
                                        onChange={(e) =>
                                          handleUpdateRecordField(record.id, field.name, e.target.value)
                                        }
                                        className="appearance-none inline-flex items-center gap-1.5 px-2.5 py-1 rounded-md text-xs font-semibold bg-purple-50 hover:bg-purple-100 dark:bg-purple-950/70 dark:hover:bg-purple-900/70 text-purple-700 dark:text-purple-300 border border-purple-200 dark:border-purple-800 cursor-pointer pr-5"
                                        title={`Linked to ${targetTable?.name || "Table"}`}
                                      >
                                        {targetRecords.map((tr) => (
                                          <option key={tr.id} value={tr.id}>
                                            {tr[field.target_display_field || "name"] || tr.name || tr.title || tr.id}
                                          </option>
                                        ))}
                                      </select>
                                      <span className="pointer-events-none absolute right-1.5 top-1/2 -translate-y-1/2 text-[9px] text-purple-500">
                                        ▾
                                      </span>
                                    </div>
                                  </td>
                                );
                              }

                              return (
                                <td key={field.name} className="py-2.5 px-3 text-slate-800 dark:text-slate-200">
                                  {field.name === "budget" || field.name === "allocated_amount" ? (
                                    <span className="font-mono font-medium">${Number(cellValue || 0).toLocaleString()}</span>
                                  ) : field.name === "status" || field.name === "disbursement_status" ? (
                                    <span
                                      className={`px-2 py-0.5 rounded text-[10px] font-bold ${
                                        cellValue === "Approved" || cellValue === "Disbursed"
                                          ? "bg-emerald-50 text-emerald-700 dark:bg-emerald-950 dark:text-emerald-300"
                                          : cellValue === "Funded"
                                          ? "bg-blue-50 text-blue-700 dark:bg-blue-950 dark:text-blue-300"
                                          : "bg-amber-50 text-amber-700 dark:bg-amber-950 dark:text-amber-300"
                                      }`}
                                    >
                                      {cellValue}
                                    </span>
                                  ) : field.name === "id" ? (
                                    <span className="font-mono text-slate-500">{cellValue}</span>
                                  ) : (
                                    <span>{String(cellValue ?? "")}</span>
                                  )}
                                </td>
                              );
                            })}
                          </tr>
                        ))}
                    </tbody>
                  </table>
                </div>
              </div>
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
                      data-testid={`builder-page-${p.slug}`}
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

                    // Determine bound table for this component
                    const boundTable = tables.find((t) => t.id === comp.config.source_table_id) || tables[0];

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
                          <span className="font-mono text-purple-600 dark:text-purple-400">{boundTable.name}</span>
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
                                {comp.config.aggregation === "sum" ? "$3,450,000" : (boundTable.records || []).length}
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
                                  Bound to Table: <strong>{boundTable.name}</strong>
                                </span>
                              </div>
                              <span className="text-[10px] font-mono text-slate-400">
                                TanStack Table v8
                              </span>
                            </div>
                            <div className="mt-3 overflow-x-auto">
                              <table className="w-full text-left text-xs">
                                <thead>
                                  <tr className="border-b border-slate-200 dark:border-slate-800 text-slate-500">
                                    {(comp.config.visible_columns || boundTable.fields.map((f) => f.name)).slice(0, 5).map(
                                      (col: string) => (
                                        <th key={col} className="pb-2 font-mono text-[11px] uppercase">
                                          {col}
                                        </th>
                                      )
                                    )}
                                  </tr>
                                </thead>
                                <tbody className="divide-y divide-slate-100 dark:divide-slate-800/60">
                                  {(boundTable.records || []).slice(0, 3).map((rec) => (
                                    <tr key={rec.id}>
                                      {(comp.config.visible_columns || boundTable.fields.map((f) => f.name))
                                        .slice(0, 5)
                                        .map((col: string) => (
                                          <td key={col} className="py-2 text-slate-700 dark:text-slate-300">
                                            {col === "lead_investigator_id" ? (
                                              <span className="px-2 py-0.5 rounded bg-purple-50 text-purple-700 dark:bg-purple-950 dark:text-purple-300 font-semibold text-[10px]">
                                                👥 {resolveLinkedDisplay("tbl-investigators", rec[col], "name")}
                                              </span>
                                            ) : col === "budget" ? (
                                              <span className="font-mono">${Number(rec[col]).toLocaleString()}</span>
                                            ) : (
                                              <span>{String(rec[col] ?? "")}</span>
                                            )}
                                          </td>
                                        ))}
                                    </tr>
                                  ))}
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
                              {["Under Review (2)", "Approved (1)", "Funded (1)"].map((stg) => (
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
                                    <div className="text-[10px] text-purple-600 dark:text-purple-400 mt-1">
                                      👥 Dr. Marie Curie
                                    </div>
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
                                  <label className="text-[11px] font-medium text-slate-500">Lead Investigator (Relation 🔗)</label>
                                  <input
                                    type="text"
                                    disabled
                                    placeholder="👥 Dr. Marie Curie"
                                    className="mt-1 w-full px-3 py-1.5 rounded border border-purple-200 dark:border-purple-800 bg-purple-50/50 dark:bg-purple-950/30 text-xs text-purple-700 dark:text-purple-300 font-semibold"
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
              tables={tables}
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
                fields={activeTable.fields}
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
                  Multi-Table Storage &amp; Vanity Routing
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
                  Relational Lattice Topology
                </h3>
                <div className="space-y-2 text-xs">
                  {relationships.map((rel) => (
                    <div
                      key={rel.id}
                      className="p-3 rounded bg-slate-50 dark:bg-slate-800/50 border border-slate-200 dark:border-slate-700 flex items-center justify-between"
                    >
                      <div>
                        <span className="font-bold text-slate-800 dark:text-slate-200">{rel.name}</span>
                        <p className="text-slate-500 text-[11px]">
                          Foreign Key Link: {rel.source_table_id}.{rel.source_field} → {rel.target_table_id}.{rel.display_field}
                        </p>
                      </div>
                      <span className="text-[10px] font-mono px-2 py-0.5 rounded bg-purple-100 text-purple-800 dark:bg-purple-950 dark:text-purple-300 font-bold">
                        {rel.relationship_type}
                      </span>
                    </div>
                  ))}
                </div>
              </div>
            </div>
          </div>
        )}
      </div>

      {/* CREATE NEW TABLE MODAL */}
      {isAddTableModalOpen && (
        <div className="fixed inset-0 z-50 flex items-center justify-center p-4 bg-slate-900/60 backdrop-blur-xs animate-fade-in">
          <div className="bg-white dark:bg-slate-900 border border-slate-200 dark:border-slate-800 rounded-xl shadow-2xl w-full max-w-md p-6 space-y-4">
            <div className="flex items-center justify-between pb-3 border-b border-slate-200 dark:border-slate-800">
              <h3 className="text-sm font-bold text-slate-900 dark:text-white">Create New Application Table</h3>
              <button
                type="button"
                onClick={() => setIsAddTableModalOpen(false)}
                className="text-slate-400 hover:text-slate-600 text-sm"
              >
                ✕
              </button>
            </div>
            <form onSubmit={handleAddNewTable} className="space-y-3 text-xs">
              <div>
                <label className="font-semibold text-slate-700 dark:text-slate-300">Table Name</label>
                <input
                  type="text"
                  required
                  placeholder="e.g. Milestone Reviews"
                  value={newTableName}
                  onChange={(e) => setNewTableName(e.target.value)}
                  className="mt-1 w-full px-3 py-1.5 rounded border border-slate-300 dark:border-slate-700 bg-white dark:bg-slate-800"
                />
              </div>
              <div>
                <label className="font-semibold text-slate-700 dark:text-slate-300">Table Icon</label>
                <input
                  type="text"
                  value={newTableIcon}
                  onChange={(e) => setNewTableIcon(e.target.value)}
                  className="mt-1 w-full px-3 py-1.5 rounded border border-slate-300 dark:border-slate-700 bg-white dark:bg-slate-800"
                />
              </div>
              <div className="flex items-center justify-end gap-2 pt-2">
                <button
                  type="button"
                  onClick={() => setIsAddTableModalOpen(false)}
                  className="px-3 py-1.5 rounded border border-slate-300 dark:border-slate-700"
                >
                  Cancel
                </button>
                <button
                  type="submit"
                  className="px-3.5 py-1.5 rounded bg-blue-600 text-white font-bold"
                >
                  Create Table
                </button>
              </div>
            </form>
          </div>
        </div>
      )}
    </div>
  );
};
