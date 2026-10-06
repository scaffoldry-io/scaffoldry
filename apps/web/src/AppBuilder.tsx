import React, { useState } from "react";
import {
  AppManifest,
  AppPage,
  AppTable,
  AppView,
  CompoundFilter,
  FilterClause,
  GovernedComponentSpec,
  GovernedComponentType,
  RegisteredApp,
  RowDensity,
  SortRule,
  TableRelationship,
  ViewType,
  WorkflowAutomationRule,
} from "./types";
import { ComponentInspectorFlyout } from "./ComponentInspectorFlyout";
import { WorkflowBuilder } from "./WorkflowBuilder";
import { StandaloneIntakeForm } from "./StandaloneIntakeForm";
import {
  applyCompoundFilter,
  applyMultiSort,
  computeFieldValue,
  exportToCsv,
  getFieldTypeIcon,
  groupRecordsByField,
  parseCsv,
} from "./computedFields";

interface AppBuilderProps {
  app: RegisteredApp;
  onBack: () => void;
  onOpenPublishedApp: (slug: string) => void;
  onOpenIntakeForm?: (tableId?: string) => void;
  onOpenAiAssistant?: () => void;
  onSaveApp?: (updated: RegisteredApp) => void;
}

export const AppBuilder: React.FC<AppBuilderProps> = ({
  app,
  onBack,
  onOpenPublishedApp,
  onOpenIntakeForm,
  onOpenAiAssistant,
  onSaveApp,
}) => {
  const [activeTab, setActiveTab] = useState<"data" | "pages" | "forms" | "automations" | "settings">("pages");
  const [isFullscreen, setIsFullscreen] = useState<boolean>(false);
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
            { name: "ethics_approved", label: "Ethics Approved", field_type: "Checkbox", required: false, ferpa_sensitive: false },
            { name: "tags", label: "Research Tags", field_type: "MultiSelect", required: false, ferpa_sensitive: false, select_options: ["StemCell", "Quantum", "DOE-Grant", "Catalyst", "HighPriority"] },
            { name: "rating", label: "Review Rating", field_type: "Rating", required: false, ferpa_sensitive: false },
            { name: "lead_pi_email", label: "PI Email", field_type: "Lookup", required: false, ferpa_sensitive: false, target_table_id: "tbl-investigators", target_display_field: "email" },
            { name: "total_allocations", label: "Total Disbursed", field_type: "Rollup", required: false, ferpa_sensitive: false, target_table_id: "tbl-allocations", target_display_field: "allocated_amount", rollup_function: "sum" },
            { name: "allocations_count", label: "Disbursement Count", field_type: "Count", required: false, ferpa_sensitive: false, target_table_id: "tbl-allocations" },
            { name: "indirect_cost", label: "Indirect Cost (20%)", field_type: "Formula", required: false, ferpa_sensitive: false, formula_expression: "{budget} * 0.20" },
          ],
          records: [
            { id: "APP-001", title: "Quantum Optomechanics Qubit Study", department: "Physics", lead_investigator_id: "INV-01", status: "Under Review", budget: 450000, submitted_at: "2026-10-02", ethics_approved: true, tags: ["Quantum", "HighPriority"], rating: 5 },
            { id: "APP-002", title: "Neural Stem Cell Regeneration", department: "Bioengineering", lead_investigator_id: "INV-02", status: "Approved", budget: 820000, submitted_at: "2026-09-28", ethics_approved: true, tags: ["StemCell", "DOE-Grant"], rating: 4 },
            { id: "APP-003", title: "Edge Sensor Fusion Lattice", department: "Computer Science", lead_investigator_id: "INV-02", status: "Funded", budget: 640000, submitted_at: "2026-09-15", ethics_approved: false, tags: ["Quantum"], rating: 3 },
            { id: "APP-004", title: "High-Entropy Alloy Catalyst Synthesis", department: "Materials Science", lead_investigator_id: "INV-03", status: "Under Review", budget: 380000, submitted_at: "2026-10-01", ethics_approved: true, tags: ["Catalyst"], rating: 4 },
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

  const defaultSavedViews: AppView[] = [
    {
      id: "view-proposals-grid",
      table_id: "tbl-proposals",
      title: "All Proposals",
      view_type: "Grid",
      row_density: "medium",
    },
    {
      id: "view-proposals-kanban",
      table_id: "tbl-proposals",
      title: "Status Kanban",
      view_type: "Kanban",
      kanban_column_field: "status",
    },
    {
      id: "view-proposals-calendar",
      table_id: "tbl-proposals",
      title: "Submission Timeline",
      view_type: "Calendar",
      calendar_date_field: "submitted_at",
    },
    {
      id: "view-proposals-gallery",
      table_id: "tbl-proposals",
      title: "Grant Showcase",
      view_type: "Gallery",
    },
    {
      id: "view-investigators-grid",
      table_id: "tbl-investigators",
      title: "Faculty Grid",
      view_type: "Grid",
      row_density: "medium",
    },
    {
      id: "view-allocations-grid",
      table_id: "tbl-allocations",
      title: "Allocations Grid",
      view_type: "Grid",
      row_density: "medium",
    },
  ];

  const [savedViews, setSavedViews] = useState<AppView[]>(defaultSavedViews);
  const [activeViewId, setActiveViewId] = useState<string>("view-proposals-grid");
  const [compoundFilter, setCompoundFilter] = useState<CompoundFilter>({
    conjunction: "AND",
    clauses: [],
  });
  const [sortRules, setSortRules] = useState<SortRule[]>([]);
  const [groupByField, setGroupByField] = useState<string | null>(null);
  const [rowDensity, setRowDensity] = useState<RowDensity>("medium");
  const [isFilterPopoverOpen, setIsFilterPopoverOpen] = useState<boolean>(false);
  const [isSortPopoverOpen, setIsSortPopoverOpen] = useState<boolean>(false);
  const [isAddViewModalOpen, setIsAddViewModalOpen] = useState<boolean>(false);
  const [newViewTitle, setNewViewTitle] = useState<string>("New View");
  const [newViewType, setNewViewType] = useState<ViewType>("Grid");

  // Milestone 3: Record Mutations, Detail Drawer, Bulk Selection, and CSV
  const [selectedRecordId, setSelectedRecordId] = useState<string | null>(null);
  const [selectedRowIds, setSelectedRowIds] = useState<Set<string>>(new Set());
  const [isCsvModalOpen, setIsCsvModalOpen] = useState<boolean>(false);
  const [csvRawText, setCsvRawText] = useState<string>("");
  const [csvParsedHeaders, setCsvParsedHeaders] = useState<string[]>([]);
  const [csvParsedRows, setCsvParsedRows] = useState<Record<string, any>[]>([]);
  const [csvFieldMapping, setCsvFieldMapping] = useState<Record<string, string>>({});

  const currentTableViews = savedViews.filter(
    (v) => (v.table_id || "tbl-proposals") === activeTable.id
  );

  const activeView =
    currentTableViews.find((v) => v.id === activeViewId) ||
    currentTableViews[0] ||
    savedViews[0];

  const handleAddFilterClause = () => {
    const firstField = activeTable.fields[0]?.name || "id";
    const newClause: FilterClause = {
      id: `filter-${Date.now()}`,
      field_name: firstField,
      operator: "equals",
      value: "",
    };
    setCompoundFilter((prev) => ({
      ...prev,
      clauses: [...prev.clauses, newClause],
    }));
  };

  const handleUpdateFilterClause = (id: string, updates: Partial<FilterClause>) => {
    setCompoundFilter((prev) => ({
      ...prev,
      clauses: prev.clauses.map((c) => (c.id === id ? { ...c, ...updates } : c)),
    }));
  };

  const handleRemoveFilterClause = (id: string) => {
    setCompoundFilter((prev) => ({
      ...prev,
      clauses: prev.clauses.filter((c) => c.id !== id),
    }));
  };

  const handleAddSortRule = () => {
    const firstField = activeTable.fields[0]?.name || "id";
    const newRule: SortRule = {
      id: `sort-${Date.now()}`,
      field_name: firstField,
      direction: "asc",
    };
    setSortRules((prev) => [...prev, newRule]);
  };

  const handleUpdateSortRule = (id: string, updates: Partial<SortRule>) => {
    setSortRules((prev) =>
      prev.map((r) => (r.id === id ? { ...r, ...updates } : r))
    );
  };

  const handleRemoveSortRule = (id: string) => {
    setSortRules((prev) => prev.filter((r) => r.id !== id));
  };

  const handleCreateNewView = (e: React.FormEvent) => {
    e.preventDefault();
    if (!newViewTitle.trim()) return;
    const created: AppView = {
      id: `view-${Date.now()}`,
      table_id: activeTable.id,
      title: newViewTitle.trim(),
      view_type: newViewType,
      row_density: "medium",
      kanban_column_field: newViewType === "Kanban" ? "status" : undefined,
      calendar_date_field: newViewType === "Calendar" ? "submitted_at" : undefined,
    };
    setSavedViews((prev) => [...prev, created]);
    setActiveViewId(created.id);
    setIsAddViewModalOpen(false);
    setNewViewTitle("");
  };

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

  // Direct Row Mutations
  const handleAddRow = () => {
    const newId = `REC-${Date.now().toString().slice(-4)}`;
    const newRecord: Record<string, any> = { id: newId };
    newRecord[activeTable.primary_field] = `New ${activeTable.name} Record`;
    for (const f of activeTable.fields) {
      if (f.name === "id" || f.name === activeTable.primary_field) continue;
      if (f.field_type === "Number" || f.field_type === "Currency") newRecord[f.name] = 0;
      else if (f.field_type === "Checkbox") newRecord[f.name] = false;
      else if (f.field_type === "Date") newRecord[f.name] = "2026-10-06";
      else if (f.field_type === "Select" && f.select_options && f.select_options.length > 0)
        newRecord[f.name] = f.select_options[0];
      else if (f.field_type === "Rating") newRecord[f.name] = 3;
      else if (!["Formula", "Lookup", "Count", "Rollup"].includes(f.field_type))
        newRecord[f.name] = "";
    }
    setTables((prev) =>
      prev.map((t) =>
        t.id === activeTable.id ? { ...t, records: [...(t.records || []), newRecord] } : t
      )
    );
  };

  const handleDuplicateRecord = (recordId: string) => {
    const rec = (activeTable.records || []).find((r) => r.id === recordId);
    if (!rec) return;
    const newId = `REC-${Date.now().toString().slice(-4)}`;
    const copy: Record<string, any> = {
      ...rec,
      id: newId,
      [activeTable.primary_field]: `${rec[activeTable.primary_field] || "Record"} (Copy)`,
    };
    setTables((prev) =>
      prev.map((t) =>
        t.id === activeTable.id ? { ...t, records: [...(t.records || []), copy] } : t
      )
    );
  };

  const handleDeleteRecord = (recordId: string) => {
    setTables((prev) =>
      prev.map((t) =>
        t.id === activeTable.id
          ? { ...t, records: (t.records || []).filter((r) => r.id !== recordId) }
          : t
      )
    );
    if (selectedRecordId === recordId) setSelectedRecordId(null);
    setSelectedRowIds((prev) => {
      const next = new Set(prev);
      next.delete(recordId);
      return next;
    });
  };

  // Bulk Selection Handlers
  const handleToggleSelectRow = (recordId: string) => {
    setSelectedRowIds((prev) => {
      const next = new Set(prev);
      if (next.has(recordId)) next.delete(recordId);
      else next.add(recordId);
      return next;
    });
  };

  const handleToggleSelectAll = (allIds: string[]) => {
    if (selectedRowIds.size === allIds.length && allIds.length > 0) {
      setSelectedRowIds(new Set());
    } else {
      setSelectedRowIds(new Set(allIds));
    }
  };

  const handleBatchDuplicate = () => {
    const toDuplicate = (activeTable.records || []).filter((r) => selectedRowIds.has(r.id));
    const copies = toDuplicate.map((rec, i) => ({
      ...rec,
      id: `REC-${Date.now().toString().slice(-4)}-${i + 1}`,
      [activeTable.primary_field]: `${rec[activeTable.primary_field] || "Record"} (Copy)`,
    }));
    setTables((prev) =>
      prev.map((t) =>
        t.id === activeTable.id ? { ...t, records: [...(t.records || []), ...copies] } : t
      )
    );
    setSelectedRowIds(new Set());
  };

  const handleBatchDelete = () => {
    setTables((prev) =>
      prev.map((t) =>
        t.id === activeTable.id
          ? { ...t, records: (t.records || []).filter((r) => !selectedRowIds.has(r.id)) }
          : t
      )
    );
    if (selectedRecordId && selectedRowIds.has(selectedRecordId)) setSelectedRecordId(null);
    setSelectedRowIds(new Set());
  };

  // CSV Export & Import Handlers
  const handleExportCsv = (recordsToExport: Record<string, any>[]) => {
    const csvContent = exportToCsv(activeTable.fields, recordsToExport);
    const blob = new Blob([csvContent], { type: "text/csv;charset=utf-8;" });
    const url = URL.createObjectURL(blob);
    const a = document.createElement("a");
    a.href = url;
    a.download = `${activeTable.slug}-${activeView.title.toLowerCase().replace(/\s+/g, "-")}.csv`;
    document.body.appendChild(a);
    a.click();
    document.body.removeChild(a);
    URL.revokeObjectURL(url);
  };

  const handleOpenCsvModal = () => {
    setCsvRawText("");
    setCsvParsedHeaders([]);
    setCsvParsedRows([]);
    setCsvFieldMapping({});
    setIsCsvModalOpen(true);
  };

  const handleParseCsvText = (text: string) => {
    setCsvRawText(text);
    const parsed = parseCsv(text);
    setCsvParsedHeaders(parsed.headers);
    setCsvParsedRows(parsed.rows);

    const initialMapping: Record<string, string> = {};
    for (const h of parsed.headers) {
      const match = activeTable.fields.find(
        (f) =>
          f.name.toLowerCase() === h.toLowerCase() ||
          f.label.toLowerCase() === h.toLowerCase()
      );
      if (match) initialMapping[h] = match.name;
      else initialMapping[h] = "";
    }
    setCsvFieldMapping(initialMapping);
  };

  const handleExecuteCsvImport = () => {
    if (csvParsedRows.length === 0) return;
    const importedRecords: Record<string, any>[] = csvParsedRows.map((row, idx) => {
      const rec: Record<string, any> = {
        id: `REC-IMP-${Date.now().toString().slice(-4)}-${idx + 1}`,
      };
      for (const [csvHeader, targetField] of Object.entries(csvFieldMapping)) {
        if (targetField && row[csvHeader] !== undefined) {
          rec[targetField] = row[csvHeader];
        }
      }
      return rec;
    });

    setTables((prev) =>
      prev.map((t) =>
        t.id === activeTable.id
          ? { ...t, records: [...(t.records || []), ...importedRecords] }
          : t
      )
    );
    setIsCsvModalOpen(false);
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
    <div
      className={`min-h-screen bg-slate-50 dark:bg-slate-950 text-slate-900 dark:text-slate-100 flex flex-col font-sans transition-all duration-200 ${
        isFullscreen ? "fixed inset-0 z-50 overflow-y-auto" : ""
      }`}
    >
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

          <button
            type="button"
            data-testid="fullscreen-mode-toggle"
            onClick={() => setIsFullscreen(!isFullscreen)}
            className={`flex items-center gap-1.5 px-3 py-1.5 rounded-lg border text-xs font-semibold cursor-pointer transition-colors ${
              isFullscreen
                ? "border-blue-500 bg-blue-50 dark:bg-blue-950/60 text-blue-700 dark:text-blue-300 font-bold"
                : "border-slate-300 dark:border-slate-700 hover:bg-slate-100 dark:hover:bg-slate-800 text-slate-700 dark:text-slate-300"
            }`}
            title={isFullscreen ? "Exit Full-Screen Canvas" : "Full-Screen Canvas View"}
          >
            {isFullscreen ? "🗗 Exit Full-Screen" : "⛶ Full-Screen"}
          </button>

          {onOpenIntakeForm && (
            <button
              type="button"
              data-testid="open-standalone-form-btn"
              onClick={() => onOpenIntakeForm(activeTable.id)}
              className="flex items-center gap-1.5 px-3 py-1.5 rounded-lg border border-blue-200 dark:border-blue-800 bg-blue-50/60 dark:bg-blue-950/40 text-blue-700 dark:text-blue-300 hover:bg-blue-100 dark:hover:bg-blue-900/40 text-xs font-semibold cursor-pointer transition-colors"
            >
              📋 Public Form
            </button>
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
            data-testid="tab-btn-forms"
            onClick={() => setActiveTab("forms")}
            className={`py-3 border-b-2 cursor-pointer transition-colors ${
              activeTab === "forms"
                ? "border-blue-600 text-blue-600 dark:text-blue-400 font-bold"
                : "border-transparent text-slate-500 hover:text-slate-800 dark:hover:text-slate-200"
            }`}
          >
            3. Standalone Public Forms
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
            4. Governed Automations
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
            5. Governance &amp; Vanity Routing
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
                {/* VIEW SWITCHER & TABLE HEADER */}
                <div className="flex items-center justify-between flex-wrap gap-3">
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

                  {/* SAVED VIEW SWITCHER TABS */}
                  <div className="flex items-center gap-1.5 overflow-x-auto bg-slate-100 dark:bg-slate-800/60 p-1 rounded-xl">
                    {currentTableViews.map((v) => {
                      const isSel = v.id === activeView.id;
                      const icon =
                        v.view_type === "Grid"
                          ? "▦"
                          : v.view_type === "Kanban"
                          ? "☷"
                          : v.view_type === "Calendar"
                          ? "📅"
                          : "🖼";
                      return (
                        <button
                          key={v.id}
                          type="button"
                          data-testid={`view-tab-${v.id}`}
                          onClick={() => setActiveViewId(v.id)}
                          className={`flex items-center gap-1.5 px-3 py-1.5 rounded-lg text-xs font-semibold cursor-pointer transition-all ${
                            isSel
                              ? "bg-white text-slate-900 dark:bg-slate-900 dark:text-white shadow-xs font-bold"
                              : "text-slate-600 hover:text-slate-900 dark:text-slate-400 dark:hover:text-white hover:bg-white/50"
                          }`}
                        >
                          <span className="text-blue-500">{icon}</span>
                          <span>{v.title}</span>
                          <span className="text-[10px] text-slate-400 uppercase font-mono">
                            {v.view_type}
                          </span>
                        </button>
                      );
                    })}
                    <button
                      type="button"
                      data-testid="btn-add-view"
                      onClick={() => setIsAddViewModalOpen(true)}
                      className="flex items-center gap-1 px-2.5 py-1.5 rounded-lg text-xs font-semibold text-slate-500 hover:text-blue-600 hover:bg-white dark:hover:bg-slate-900 cursor-pointer transition-colors"
                      title="Create New Saved View"
                    >
                      <span>+</span>
                      <span>View</span>
                    </button>
                  </div>
                </div>

                {/* DATA SHAPING TOOLBAR (Filter, Sort, Group, Density, Search) */}
                <div className="flex items-center justify-between flex-wrap gap-2 py-2.5 px-3 bg-slate-50 dark:bg-slate-800/40 border border-slate-200 dark:border-slate-800 rounded-xl">
                  <div className="flex items-center gap-2 flex-wrap">
                    {/* COMPOUND FILTER BUTTON & POPOVER */}
                    <div className="relative">
                      <button
                        type="button"
                        data-testid="toolbar-filter-btn"
                        onClick={() => {
                          setIsFilterPopoverOpen((prev) => !prev);
                          setIsSortPopoverOpen(false);
                        }}
                        className={`flex items-center gap-1.5 px-2.5 py-1 rounded-lg text-xs font-semibold border cursor-pointer transition-colors ${
                          compoundFilter.clauses.length > 0
                            ? "bg-blue-50 border-blue-300 text-blue-700 dark:bg-blue-950 dark:border-blue-700 dark:text-blue-300"
                            : "bg-white border-slate-200 text-slate-700 hover:bg-slate-100 dark:bg-slate-800 dark:border-slate-700 dark:text-slate-300"
                        }`}
                      >
                        <span>⚡ Filter</span>
                        {compoundFilter.clauses.length > 0 && (
                          <span className="px-1.5 py-0.2 rounded-full bg-blue-600 text-white text-[10px] font-bold">
                            {compoundFilter.clauses.length}
                          </span>
                        )}
                      </button>

                      {isFilterPopoverOpen && (
                        <div
                          data-testid="filter-popover"
                          className="absolute left-0 top-full mt-2 w-96 bg-white dark:bg-slate-900 border border-slate-200 dark:border-slate-800 rounded-xl shadow-xl p-4 z-40 space-y-3"
                        >
                          <div className="flex items-center justify-between pb-2 border-b border-slate-100 dark:border-slate-800">
                            <span className="text-xs font-bold text-slate-800 dark:text-slate-200">
                              Compound Filter Conditions
                            </span>
                            <div className="flex items-center gap-2">
                              <span className="text-[11px] text-slate-400">Match:</span>
                              <div className="inline-flex rounded border border-slate-200 dark:border-slate-700 p-0.5">
                                <button
                                  type="button"
                                  onClick={() =>
                                    setCompoundFilter((p) => ({ ...p, conjunction: "AND" }))
                                  }
                                  className={`px-2 py-0.5 text-[10px] font-bold rounded ${
                                    compoundFilter.conjunction === "AND"
                                      ? "bg-blue-600 text-white"
                                      : "text-slate-500"
                                  }`}
                                >
                                  AND
                                </button>
                                <button
                                  type="button"
                                  onClick={() =>
                                    setCompoundFilter((p) => ({ ...p, conjunction: "OR" }))
                                  }
                                  className={`px-2 py-0.5 text-[10px] font-bold rounded ${
                                    compoundFilter.conjunction === "OR"
                                      ? "bg-blue-600 text-white"
                                      : "text-slate-500"
                                  }`}
                                >
                                  OR
                                </button>
                              </div>
                            </div>
                          </div>

                          {compoundFilter.clauses.length === 0 ? (
                            <p className="text-xs text-slate-400 py-2 text-center">
                              No filter conditions configured. All rows shown.
                            </p>
                          ) : (
                            <div className="space-y-2 max-h-60 overflow-y-auto">
                              {compoundFilter.clauses.map((clause, idx) => (
                                <div key={clause.id} className="flex items-center gap-1.5 text-xs">
                                  <span className="text-[10px] text-slate-400 w-8">
                                    {idx === 0 ? "Where" : compoundFilter.conjunction}
                                  </span>
                                  <select
                                    value={clause.field_name}
                                    onChange={(e) =>
                                      handleUpdateFilterClause(clause.id, {
                                        field_name: e.target.value,
                                      })
                                    }
                                    className="px-2 py-1 rounded border border-slate-200 dark:border-slate-700 bg-slate-50 dark:bg-slate-800 text-xs w-28"
                                  >
                                    {activeTable.fields.map((f) => (
                                      <option key={f.name} value={f.name}>
                                        {f.label}
                                      </option>
                                    ))}
                                  </select>
                                  <select
                                    value={clause.operator}
                                    onChange={(e) =>
                                      handleUpdateFilterClause(clause.id, {
                                        operator: e.target.value as any,
                                      })
                                    }
                                    className="px-2 py-1 rounded border border-slate-200 dark:border-slate-700 bg-slate-50 dark:bg-slate-800 text-xs w-24"
                                  >
                                    <option value="equals">is</option>
                                    <option value="not_equals">is not</option>
                                    <option value="contains">contains</option>
                                    <option value="not_contains">not contains</option>
                                    <option value="greater_than">&gt;</option>
                                    <option value="less_than">&lt;</option>
                                    <option value="is_empty">is empty</option>
                                    <option value="is_not_empty">not empty</option>
                                  </select>
                                  {!["is_empty", "is_not_empty"].includes(clause.operator) && (
                                    <input
                                      type="text"
                                      value={clause.value}
                                      onChange={(e) =>
                                        handleUpdateFilterClause(clause.id, {
                                          value: e.target.value,
                                        })
                                      }
                                      placeholder="Value..."
                                      className="px-2 py-1 rounded border border-slate-200 dark:border-slate-700 bg-slate-50 dark:bg-slate-800 text-xs flex-1"
                                    />
                                  )}
                                  <button
                                    type="button"
                                    onClick={() => handleRemoveFilterClause(clause.id)}
                                    className="text-slate-400 hover:text-red-500 px-1 font-bold"
                                  >
                                    ✕
                                  </button>
                                </div>
                              ))}
                            </div>
                          )}

                          <div className="flex items-center justify-between pt-2 border-t border-slate-100 dark:border-slate-800 text-xs">
                            <button
                              type="button"
                              onClick={handleAddFilterClause}
                              className="text-blue-600 dark:text-blue-400 font-semibold hover:underline cursor-pointer"
                            >
                              + Add Condition
                            </button>
                            {compoundFilter.clauses.length > 0 && (
                              <button
                                type="button"
                                onClick={() => setCompoundFilter({ conjunction: "AND", clauses: [] })}
                                className="text-slate-400 hover:text-slate-600 text-xs cursor-pointer"
                              >
                                Clear All
                              </button>
                            )}
                          </div>
                        </div>
                      )}
                    </div>

                    {/* MULTI-COLUMN SORT BUTTON & POPOVER */}
                    <div className="relative">
                      <button
                        type="button"
                        data-testid="toolbar-sort-btn"
                        onClick={() => {
                          setIsSortPopoverOpen((prev) => !prev);
                          setIsFilterPopoverOpen(false);
                        }}
                        className={`flex items-center gap-1.5 px-2.5 py-1 rounded-lg text-xs font-semibold border cursor-pointer transition-colors ${
                          sortRules.length > 0
                            ? "bg-purple-50 border-purple-300 text-purple-700 dark:bg-purple-950 dark:border-purple-700 dark:text-purple-300"
                            : "bg-white border-slate-200 text-slate-700 hover:bg-slate-100 dark:bg-slate-800 dark:border-slate-700 dark:text-slate-300"
                        }`}
                      >
                        <span>⇅ Sort</span>
                        {sortRules.length > 0 && (
                          <span className="px-1.5 py-0.2 rounded-full bg-purple-600 text-white text-[10px] font-bold">
                            {sortRules.length}
                          </span>
                        )}
                      </button>

                      {isSortPopoverOpen && (
                        <div
                          data-testid="sort-popover"
                          className="absolute left-0 top-full mt-2 w-80 bg-white dark:bg-slate-900 border border-slate-200 dark:border-slate-800 rounded-xl shadow-xl p-4 z-40 space-y-3"
                        >
                          <div className="flex items-center justify-between pb-2 border-b border-slate-100 dark:border-slate-800">
                            <span className="text-xs font-bold text-slate-800 dark:text-slate-200">
                              Multi-Column Sorting
                            </span>
                            {sortRules.length > 0 && (
                              <button
                                type="button"
                                onClick={() => setSortRules([])}
                                className="text-slate-400 hover:text-slate-600 text-xs cursor-pointer"
                              >
                                Clear
                              </button>
                            )}
                          </div>

                          {sortRules.length === 0 ? (
                            <p className="text-xs text-slate-400 py-2 text-center">
                              No sort rules active. Default ordering applied.
                            </p>
                          ) : (
                            <div className="space-y-2">
                              {sortRules.map((rule, idx) => (
                                <div key={rule.id} className="flex items-center gap-2 text-xs">
                                  <span className="text-[10px] text-slate-400 w-10">
                                    {idx === 0 ? "Sort by" : "Then by"}
                                  </span>
                                  <select
                                    value={rule.field_name}
                                    onChange={(e) =>
                                      handleUpdateSortRule(rule.id, {
                                        field_name: e.target.value,
                                      })
                                    }
                                    className="px-2 py-1 rounded border border-slate-200 dark:border-slate-700 bg-slate-50 dark:bg-slate-800 text-xs flex-1"
                                  >
                                    {activeTable.fields.map((f) => (
                                      <option key={f.name} value={f.name}>
                                        {f.label}
                                      </option>
                                    ))}
                                  </select>
                                  <select
                                    value={rule.direction}
                                    onChange={(e) =>
                                      handleUpdateSortRule(rule.id, {
                                        direction: e.target.value as any,
                                      })
                                    }
                                    className="px-2 py-1 rounded border border-slate-200 dark:border-slate-700 bg-slate-50 dark:bg-slate-800 text-xs w-20"
                                  >
                                    <option value="asc">Asc (A-Z)</option>
                                    <option value="desc">Desc (Z-A)</option>
                                  </select>
                                  <button
                                    type="button"
                                    onClick={() => handleRemoveSortRule(rule.id)}
                                    className="text-slate-400 hover:text-red-500 px-1 font-bold"
                                  >
                                    ✕
                                  </button>
                                </div>
                              ))}
                            </div>
                          )}

                          <div className="pt-2 border-t border-slate-100 dark:border-slate-800">
                            <button
                              type="button"
                              onClick={handleAddSortRule}
                              className="text-purple-600 dark:text-purple-400 font-semibold hover:underline text-xs cursor-pointer"
                            >
                              + Add Sort Rule
                            </button>
                          </div>
                        </div>
                      )}
                    </div>

                    {/* ROW GROUPING SELECTOR */}
                    <div className="flex items-center gap-1.5 text-xs">
                      <span className="text-slate-400">Group by:</span>
                      <select
                        data-testid="toolbar-group-select"
                        value={groupByField || ""}
                        onChange={(e) => setGroupByField(e.target.value || null)}
                        className="px-2.5 py-1 rounded-lg border border-slate-200 dark:border-slate-700 bg-white dark:bg-slate-800 text-xs text-slate-700 dark:text-slate-300 font-medium"
                      >
                        <option value="">None (Ungrouped)</option>
                        {activeTable.fields.map((f) => (
                          <option key={f.name} value={f.name}>
                            {f.label}
                          </option>
                        ))}
                      </select>
                    </div>

                    {/* ROW DENSITY / HEIGHT SELECTOR */}
                    <div className="flex items-center gap-1.5 text-xs">
                      <span className="text-slate-400">Density:</span>
                      <div className="inline-flex rounded-lg border border-slate-200 dark:border-slate-700 p-0.5 bg-white dark:bg-slate-800">
                        {(["compact", "medium", "tall", "extra_tall"] as RowDensity[]).map((d) => (
                          <button
                            key={d}
                            type="button"
                            data-testid={`density-btn-${d}`}
                            onClick={() => setRowDensity(d)}
                            className={`px-2 py-0.5 rounded text-[10px] font-semibold uppercase transition-all ${
                              rowDensity === d
                                ? "bg-slate-900 text-white dark:bg-slate-100 dark:text-slate-900 shadow-xs"
                                : "text-slate-500 hover:text-slate-800 dark:hover:text-slate-200"
                            }`}
                          >
                            {d.replace("_", " ")}
                          </button>
                        ))}
                      </div>
                    </div>
                  </div>

                  {/* QUICK SEARCH */}
                  <input
                    type="text"
                    placeholder={`Search ${activeTable.name}...`}
                    value={tableSearchFilter}
                    onChange={(e) => setTableSearchFilter(e.target.value)}
                    className="px-2.5 py-1 rounded-lg border border-slate-300 dark:border-slate-700 bg-white dark:bg-slate-800 text-xs text-slate-800 dark:text-white focus:outline-blue-500 w-44"
                  />

                  {/* CSV & RECORD ACTIONS */}
                  <div className="flex items-center gap-1.5">
                    <button
                      type="button"
                      data-testid="toolbar-import-csv-btn"
                      onClick={handleOpenCsvModal}
                      className="px-2.5 py-1 rounded-lg text-xs font-semibold bg-white dark:bg-slate-800 border border-slate-200 dark:border-slate-700 text-slate-700 dark:text-slate-300 hover:bg-slate-100 dark:hover:bg-slate-700 cursor-pointer flex items-center gap-1 transition-colors"
                      title="Import records from CSV"
                    >
                      <span>📥</span>
                      <span>Import CSV</span>
                    </button>

                    <button
                      type="button"
                      data-testid="toolbar-export-csv-btn"
                      onClick={() => handleExportCsv(activeTable.records || [])}
                      className="px-2.5 py-1 rounded-lg text-xs font-semibold bg-white dark:bg-slate-800 border border-slate-200 dark:border-slate-700 text-slate-700 dark:text-slate-300 hover:bg-slate-100 dark:hover:bg-slate-700 cursor-pointer flex items-center gap-1 transition-colors"
                      title="Export table records to CSV"
                    >
                      <span>📤</span>
                      <span>Export CSV</span>
                    </button>

                    <button
                      type="button"
                      data-testid="toolbar-add-record-btn"
                      onClick={handleAddRow}
                      className="px-2.5 py-1 rounded-lg text-xs font-semibold bg-blue-600 hover:bg-blue-700 text-white cursor-pointer flex items-center gap-1 shadow-xs transition-colors"
                    >
                      <span className="font-bold">+</span>
                      <span>Add Record</span>
                    </button>
                  </div>
                </div>

                {/* COMPUTED DATA SHAPING */}
                {(() => {
                  const rawRecords = activeTable.records || [];
                  const searchFiltered = rawRecords.filter((rec) => {
                    if (!tableSearchFilter.trim()) return true;
                    return Object.values(rec).some((val) =>
                      String(val).toLowerCase().includes(tableSearchFilter.toLowerCase())
                    );
                  });
                  const compoundFiltered = applyCompoundFilter(searchFiltered, compoundFilter);
                  const sortedRecords = applyMultiSort(compoundFiltered, sortRules);
                  const recordGroups = groupRecordsByField(sortedRecords, groupByField || undefined);

                  // Density padding classes
                  const cellPadding =
                    rowDensity === "compact"
                      ? "py-1.5 px-3 text-xs"
                      : rowDensity === "tall"
                      ? "py-4 px-3 text-xs"
                      : rowDensity === "extra_tall"
                      ? "py-6 px-3 text-xs"
                      : "py-2.5 px-3 text-xs";

                  // VIEW TYPE: KANBAN
                  if (activeView.view_type === "Kanban") {
                    const stageCol = activeView.kanban_column_field || "status";
                    const statuses = ["Under Review", "Approved", "Funded"];

                    return (
                      <div data-testid="view-kanban-board" className="grid grid-cols-1 md:grid-cols-3 gap-4 pt-2">
                        {statuses.map((stage) => {
                          const stageRecords = sortedRecords.filter(
                            (r) => String(r[stageCol] || "") === stage
                          );
                          const stageTotal = stageRecords.reduce(
                            (acc, r) => acc + (Number(r.budget || r.allocated_amount) || 0),
                            0
                          );

                          return (
                            <div
                              key={stage}
                              data-testid={`kanban-column-${stage.toLowerCase().replace(/\s+/g, "-")}`}
                              className="bg-slate-50 dark:bg-slate-800/50 border border-slate-200 dark:border-slate-800 rounded-xl p-3 flex flex-col space-y-3"
                            >
                              <div className="flex items-center justify-between pb-2 border-b border-slate-200 dark:border-slate-700">
                                <div className="flex items-center gap-2">
                                  <span className="font-bold text-xs text-slate-800 dark:text-slate-200">
                                    {stage}
                                  </span>
                                  <span className="text-[10px] font-mono px-2 py-0.2 rounded-full bg-slate-200 dark:bg-slate-700 text-slate-700 dark:text-slate-300">
                                    {stageRecords.length}
                                  </span>
                                </div>
                                <span className="text-[10px] font-mono text-slate-400">
                                  ${stageTotal.toLocaleString()}
                                </span>
                              </div>

                              <div className="space-y-2.5 flex-1 overflow-y-auto">
                                {stageRecords.map((rec) => (
                                  <div
                                    key={rec.id}
                                    data-testid={`kanban-card-${rec.id}`}
                                    className="bg-white dark:bg-slate-900 border border-slate-200 dark:border-slate-800 rounded-lg p-3 shadow-xs hover:border-blue-400 transition-colors space-y-2"
                                  >
                                    <div className="flex items-center justify-between text-[10px]">
                                      <span className="font-mono text-slate-400">{rec.id}</span>
                                      <div className="flex items-center gap-1.5">
                                        <span className="font-semibold text-blue-600 dark:text-blue-400">
                                          {rec.department || "General"}
                                        </span>
                                        <button
                                          type="button"
                                          data-testid={`card-expand-btn-${rec.id}`}
                                          onClick={() => setSelectedRecordId(rec.id)}
                                          className="text-slate-400 hover:text-blue-600 p-0.5 rounded cursor-pointer"
                                          title="Expand record"
                                        >
                                          ⤢
                                        </button>
                                      </div>
                                    </div>
                                    <h4 className="text-xs font-bold text-slate-900 dark:text-white">
                                      {rec.title || rec.name || rec.id}
                                    </h4>
                                    <div className="flex items-center justify-between pt-1 border-t border-slate-100 dark:border-slate-800 text-[11px]">
                                      <span className="font-mono font-bold text-emerald-600 dark:text-emerald-400">
                                        ${Number(rec.budget || 0).toLocaleString()}
                                      </span>
                                      {rec.rating && (
                                        <span className="text-amber-500 font-bold">
                                          {"★".repeat(Number(rec.rating))}
                                        </span>
                                      )}
                                    </div>
                                  </div>
                                ))}
                              </div>
                            </div>
                          );
                        })}
                      </div>
                    );
                  }

                  // VIEW TYPE: CALENDAR
                  if (activeView.view_type === "Calendar") {
                    const dateCol = activeView.calendar_date_field || "submitted_at";
                    const datesMap = new Map<string, Record<string, any>[]>();
                    for (const r of sortedRecords) {
                      const d = String(r[dateCol] || "Unscheduled");
                      if (!datesMap.has(d)) datesMap.set(d, []);
                      datesMap.get(d)!.push(r);
                    }

                    return (
                      <div data-testid="view-calendar-timeline" className="space-y-4 pt-2">
                        {Array.from(datesMap.entries()).map(([dateStr, dRecords]) => (
                          <div
                            key={dateStr}
                            className="bg-white dark:bg-slate-900 border border-slate-200 dark:border-slate-800 rounded-xl p-4 shadow-xs space-y-3"
                          >
                            <div className="flex items-center justify-between pb-2 border-b border-slate-100 dark:border-slate-800">
                              <div className="flex items-center gap-2">
                                <span className="text-blue-500 text-sm">📅</span>
                                <span className="font-bold text-xs text-slate-900 dark:text-white">
                                  {dateStr}
                                </span>
                                <span className="text-[10px] font-mono px-2 py-0.2 rounded-full bg-blue-50 text-blue-700 dark:bg-blue-950 dark:text-blue-300">
                                  {dRecords.length} Events
                                </span>
                              </div>
                            </div>
                            <div className="grid grid-cols-1 md:grid-cols-2 gap-3">
                              {dRecords.map((rec) => (
                                <div
                                  key={rec.id}
                                  className="p-3 rounded-lg border border-slate-100 dark:border-slate-800 bg-slate-50/50 dark:bg-slate-800/40 flex items-center justify-between"
                                >
                                  <div>
                                    <div className="font-mono text-[10px] text-slate-400">{rec.id}</div>
                                    <div className="text-xs font-bold text-slate-900 dark:text-white mt-0.5">
                                      {rec.title || rec.name}
                                    </div>
                                    <div className="text-[10px] text-slate-500 mt-0.5">
                                      {rec.department}
                                    </div>
                                  </div>
                                  <div className="flex items-center gap-2">
                                    <span className="font-mono text-xs font-bold text-emerald-600 dark:text-emerald-400">
                                      ${Number(rec.budget || 0).toLocaleString()}
                                    </span>
                                    <button
                                      type="button"
                                      data-testid={`card-expand-btn-${rec.id}`}
                                      onClick={() => setSelectedRecordId(rec.id)}
                                      className="text-slate-400 hover:text-blue-600 p-1 rounded cursor-pointer"
                                      title="Expand record"
                                    >
                                      ⤢
                                    </button>
                                  </div>
                                </div>
                              ))}
                            </div>
                          </div>
                        ))}
                      </div>
                    );
                  }

                  // VIEW TYPE: GALLERY
                  if (activeView.view_type === "Gallery") {
                    return (
                      <div data-testid="view-gallery-grid" className="grid grid-cols-1 md:grid-cols-2 lg:grid-cols-3 gap-4 pt-2">
                        {sortedRecords.map((rec) => (
                          <div
                            key={rec.id}
                            data-testid={`gallery-card-${rec.id}`}
                            className="bg-white dark:bg-slate-900 border border-slate-200 dark:border-slate-800 rounded-xl overflow-hidden shadow-xs hover:border-blue-400 transition-all flex flex-col"
                          >
                            <div className="h-20 bg-linear-to-r from-blue-600/20 via-purple-600/20 to-emerald-600/20 p-3 flex items-start justify-between">
                              <span className="font-mono text-[10px] px-2 py-0.5 rounded bg-white/80 dark:bg-slate-900/80 font-bold text-slate-700 dark:text-slate-300">
                                {rec.id}
                              </span>
                              <div className="flex items-center gap-1.5">
                                <span
                                  className={`text-[10px] font-bold px-2 py-0.5 rounded ${
                                    rec.status === "Approved"
                                      ? "bg-emerald-100 text-emerald-800 dark:bg-emerald-950 dark:text-emerald-300"
                                      : "bg-blue-100 text-blue-800 dark:bg-blue-950 dark:text-blue-300"
                                  }`}
                                >
                                  {rec.status || "Active"}
                                </span>
                                <button
                                  type="button"
                                  data-testid={`card-expand-btn-${rec.id}`}
                                  onClick={() => setSelectedRecordId(rec.id)}
                                  className="text-slate-500 hover:text-slate-900 dark:hover:text-white p-0.5 rounded bg-white/60 dark:bg-slate-900/60 cursor-pointer"
                                  title="Expand record"
                                >
                                  ⤢
                                </button>
                              </div>
                            </div>
                            <div className="p-4 flex-1 flex flex-col justify-between space-y-3">
                              <div>
                                <h4 className="text-xs font-bold text-slate-900 dark:text-white">
                                  {rec.title || rec.name}
                                </h4>
                                <p className="text-[11px] text-slate-500 mt-0.5">
                                  {rec.department || "Academic Unit"}
                                </p>
                              </div>
                              <div className="flex items-center justify-between pt-2 border-t border-slate-100 dark:border-slate-800 text-xs">
                                <span className="font-mono font-bold text-emerald-600 dark:text-emerald-400">
                                  ${Number(rec.budget || 0).toLocaleString()}
                                </span>
                                {rec.rating && (
                                  <span className="text-amber-500 text-xs">
                                    {"★".repeat(Number(rec.rating))}
                                  </span>
                                )}
                              </div>
                            </div>
                          </div>
                        ))}
                      </div>
                    );
                  }

                  // VIEW TYPE: GRID (DEFAULT)
                  const allVisibleIds = sortedRecords.map((r) => r.id);
                  const isAllSelected =
                    allVisibleIds.length > 0 &&
                    allVisibleIds.every((id) => selectedRowIds.has(id));

                  return (
                    <div data-testid="view-grid-table" className="overflow-x-auto border border-slate-200 dark:border-slate-800 rounded-lg">
                      <table className="w-full text-left text-xs">
                        <thead>
                          <tr className="bg-slate-50 dark:bg-slate-800/70 border-b border-slate-200 dark:border-slate-800 text-slate-600 dark:text-slate-400 font-mono text-[11px] uppercase">
                            <th className="py-2 px-2.5 w-10 text-center">
                              <input
                                type="checkbox"
                                data-testid="select-all-checkbox"
                                checked={isAllSelected}
                                onChange={() => handleToggleSelectAll(allVisibleIds)}
                                className="rounded border-slate-300 text-blue-600 focus:ring-blue-500 h-3.5 w-3.5 cursor-pointer"
                              />
                            </th>
                            <th className="py-2 px-2 w-16 text-center">Actions</th>
                            {activeTable.fields.map((field) => (
                              <th key={field.name} className={`${cellPadding} font-semibold`}>
                                <div className="flex items-center gap-1.5">
                                  <span className="text-slate-400 font-mono text-[11px]">
                                    {getFieldTypeIcon(field.field_type)}
                                  </span>
                                  <span>{field.label}</span>
                                  {field.field_type === "Relation" && (
                                    <span className="text-purple-500" title="Relational Linked Record">
                                      🔗
                                    </span>
                                  )}
                                </div>
                              </th>
                            ))}
                          </tr>
                        </thead>
                        <tbody className="divide-y divide-slate-100 dark:divide-slate-800">
                          {recordGroups.map((group) => (
                            <React.Fragment key={group.groupValue}>
                              {/* GROUP HEADER ROW */}
                              {groupByField && (
                                <tr className="bg-slate-100/70 dark:bg-slate-800/60 font-semibold text-slate-700 dark:text-slate-300">
                                  <td
                                    colSpan={activeTable.fields.length + 2}
                                    className="py-2 px-3 text-[11px] font-mono flex items-center justify-between"
                                  >
                                    <div className="flex items-center gap-2">
                                      <span className="text-slate-400">▾</span>
                                      <span className="font-bold text-slate-900 dark:text-white">
                                        {group.groupValue}
                                      </span>
                                      <span className="px-2 py-0.2 rounded-full bg-slate-200 dark:bg-slate-700 text-slate-700 dark:text-slate-300 text-[10px]">
                                        {group.records.length} records
                                      </span>
                                    </div>
                                    <span className="text-slate-500">
                                      Subtotal: ${group.totalBudget.toLocaleString()}
                                    </span>
                                  </td>
                                </tr>
                              )}

                              {group.records.map((record) => {
                                const isSelected = selectedRowIds.has(record.id);
                                return (
                                <tr
                                  key={record.id}
                                  className={`hover:bg-slate-50/70 dark:hover:bg-slate-800/50 transition-colors ${
                                    isSelected ? "bg-blue-50/40 dark:bg-blue-950/30" : ""
                                  }`}
                                >
                                  <td className="py-2 px-2.5 text-center">
                                    <input
                                      type="checkbox"
                                      data-testid={`row-select-checkbox-${record.id}`}
                                      checked={isSelected}
                                      onChange={() => handleToggleSelectRow(record.id)}
                                      className="rounded border-slate-300 text-blue-600 focus:ring-blue-500 h-3.5 w-3.5 cursor-pointer"
                                    />
                                  </td>
                                  <td className="py-2 px-1 text-center whitespace-nowrap">
                                    <div className="flex items-center justify-center gap-1">
                                      <button
                                        type="button"
                                        data-testid={`row-expand-btn-${record.id}`}
                                        onClick={() => setSelectedRecordId(record.id)}
                                        className="p-1 text-slate-400 hover:text-blue-600 dark:hover:text-blue-400 cursor-pointer rounded hover:bg-slate-100 dark:hover:bg-slate-700"
                                        title="Expand record"
                                      >
                                        ⤢
                                      </button>
                                      <button
                                        type="button"
                                        data-testid={`row-duplicate-btn-${record.id}`}
                                        onClick={() => handleDuplicateRecord(record.id)}
                                        className="p-1 text-slate-400 hover:text-emerald-600 dark:hover:text-emerald-400 cursor-pointer rounded hover:bg-slate-100 dark:hover:bg-slate-700"
                                        title="Duplicate record"
                                      >
                                        ⧉
                                      </button>
                                      <button
                                        type="button"
                                        data-testid={`row-delete-btn-${record.id}`}
                                        onClick={() => handleDeleteRecord(record.id)}
                                        className="p-1 text-slate-400 hover:text-red-600 dark:hover:text-red-400 cursor-pointer rounded hover:bg-slate-100 dark:hover:bg-slate-700"
                                        title="Delete record"
                                      >
                                        🗑
                                      </button>
                                    </div>
                                  </td>
                                  {activeTable.fields.map((field) => {
                                    const cellValue = [
                                      "Formula",
                                      "Lookup",
                                      "Count",
                                      "Rollup",
                                    ].includes(field.field_type)
                                      ? computeFieldValue(field, record, tables)
                                      : record[field.name];

                                    // RELATIONAL LOOKUP
                                    if (field.field_type === "Relation" && field.target_table_id) {
                                      const targetTable = tables.find(
                                        (t) => t.id === field.target_table_id
                                      );
                                      const targetRecords = targetTable?.records || [];

                                      return (
                                        <td key={field.name} className={cellPadding}>
                                          <div className="relative inline-block">
                                            <select
                                              value={cellValue || ""}
                                              onChange={(e) =>
                                                handleUpdateRecordField(
                                                  record.id,
                                                  field.name,
                                                  e.target.value
                                                )
                                              }
                                              className="appearance-none inline-flex items-center gap-1.5 px-2.5 py-1 rounded-md text-xs font-semibold bg-purple-50 hover:bg-purple-100 dark:bg-purple-950/70 dark:hover:bg-purple-900/70 text-purple-700 dark:text-purple-300 border border-purple-200 dark:border-purple-800 cursor-pointer pr-5"
                                            >
                                              {targetRecords.map((tr) => (
                                                <option key={tr.id} value={tr.id}>
                                                  {tr[field.target_display_field || "name"] ||
                                                    tr.name ||
                                                    tr.title ||
                                                    tr.id}
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

                                    // CHECKBOX
                                    if (field.field_type === "Checkbox") {
                                      return (
                                        <td key={field.name} className={cellPadding}>
                                          <input
                                            type="checkbox"
                                            checked={Boolean(cellValue)}
                                            onChange={(e) =>
                                              handleUpdateRecordField(
                                                record.id,
                                                field.name,
                                                e.target.checked
                                              )
                                            }
                                            className="rounded border-slate-300 text-blue-600 focus:ring-blue-500 h-4 w-4 cursor-pointer"
                                          />
                                        </td>
                                      );
                                    }

                                    // MULTI-SELECT
                                    if (field.field_type === "MultiSelect") {
                                      const items = Array.isArray(cellValue)
                                        ? cellValue
                                        : cellValue
                                        ? [String(cellValue)]
                                        : [];
                                      return (
                                        <td key={field.name} className={cellPadding}>
                                          <div className="flex flex-wrap gap-1">
                                            {items.map((opt: string) => (
                                              <span
                                                key={opt}
                                                className="px-2 py-0.5 rounded-full text-[10px] font-semibold bg-blue-100 text-blue-800 dark:bg-blue-900/60 dark:text-blue-200"
                                              >
                                                {opt}
                                              </span>
                                            ))}
                                          </div>
                                        </td>
                                      );
                                    }

                                    // RATING
                                    if (field.field_type === "Rating") {
                                      const score = Number(cellValue || 0);
                                      return (
                                        <td key={field.name} className={cellPadding}>
                                          <div className="flex items-center gap-0.5 text-amber-500">
                                            {[1, 2, 3, 4, 5].map((star) => (
                                              <span key={star} className="text-xs">
                                                {star <= score ? "★" : "☆"}
                                              </span>
                                            ))}
                                          </div>
                                        </td>
                                      );
                                    }

                                    // LOOKUP
                                    if (field.field_type === "Lookup") {
                                      return (
                                        <td key={field.name} className={cellPadding}>
                                          <span className="inline-flex items-center gap-1 px-2 py-0.5 rounded text-[11px] font-mono bg-purple-50 text-purple-700 dark:bg-purple-950 dark:text-purple-300 border border-purple-200 dark:border-purple-800">
                                            <span className="text-[10px]">🔍</span>
                                            {String(cellValue ?? "")}
                                          </span>
                                        </td>
                                      );
                                    }

                                    // ROLLUP
                                    if (field.field_type === "Rollup") {
                                      return (
                                        <td key={field.name} className={cellPadding}>
                                          <span className="inline-flex items-center gap-1 px-2 py-0.5 rounded text-[11px] font-mono bg-emerald-50 text-emerald-700 dark:bg-emerald-950 dark:text-emerald-300 border border-emerald-200 dark:border-emerald-800">
                                            <span className="text-[10px]">Σ</span>
                                            ${Number(cellValue || 0).toLocaleString()}
                                          </span>
                                        </td>
                                      );
                                    }

                                    // COUNT
                                    if (field.field_type === "Count") {
                                      return (
                                        <td key={field.name} className={cellPadding}>
                                          <span className="inline-flex items-center gap-1 px-2 py-0.5 rounded text-[11px] font-mono bg-slate-100 text-slate-700 dark:bg-slate-800 dark:text-slate-300 border border-slate-200 dark:border-slate-700">
                                            <span className="text-[10px]">#</span>
                                            {cellValue}
                                          </span>
                                        </td>
                                      );
                                    }

                                    // FORMULA
                                    if (field.field_type === "Formula") {
                                      return (
                                        <td key={field.name} className={cellPadding}>
                                          <span className="inline-flex items-center gap-1 px-2 py-0.5 rounded text-[11px] font-mono bg-blue-50 text-blue-700 dark:bg-blue-950 dark:text-blue-300 border border-blue-200 dark:border-blue-800">
                                            <span className="text-[10px]">ƒx</span>
                                            {typeof cellValue === "number"
                                              ? `$${cellValue.toLocaleString()}`
                                              : String(cellValue ?? "")}
                                          </span>
                                        </td>
                                      );
                                    }

                                    return (
                                      <td
                                        key={field.name}
                                        className={`${cellPadding} text-slate-800 dark:text-slate-200`}
                                      >
                                        {field.name === "budget" ||
                                        field.name === "allocated_amount" ||
                                        field.field_type === "Currency" ? (
                                          <span className="font-mono font-medium">
                                            ${Number(cellValue || 0).toLocaleString()}
                                          </span>
                                        ) : field.field_type === "Percent" ? (
                                          <div className="flex items-center gap-1.5">
                                            <span className="font-mono text-xs">{cellValue}%</span>
                                            <div className="w-12 h-1.5 bg-slate-200 dark:bg-slate-700 rounded-full overflow-hidden">
                                              <div
                                                className="h-full bg-blue-500 rounded-full"
                                                style={{
                                                  width: `${Math.min(
                                                    100,
                                                    Math.max(0, Number(cellValue || 0))
                                                  )}%`,
                                                }}
                                              />
                                            </div>
                                          </div>
                                        ) : field.field_type === "Email" ? (
                                          <a
                                            href={`mailto:${cellValue}`}
                                            className="text-blue-600 dark:text-blue-400 hover:underline flex items-center gap-1"
                                          >
                                            <span className="text-[10px]">✉</span>
                                            {cellValue}
                                          </a>
                                        ) : field.field_type === "Url" ? (
                                          <a
                                            href={String(cellValue)}
                                            target="_blank"
                                            rel="noreferrer"
                                            className="text-blue-600 dark:text-blue-400 hover:underline flex items-center gap-1"
                                          >
                                            <span className="text-[10px]">🌐</span>
                                            {cellValue}
                                          </a>
                                        ) : field.name === "status" ||
                                          field.name === "disbursement_status" ? (
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
                              );
                            })}
                          </React.Fragment>
                        ))}

                        {/* INLINE ADD ROW BUTTON */}
                        <tr>
                          <td
                            colSpan={activeTable.fields.length + 2}
                            className="py-2.5 px-4 bg-slate-50/50 dark:bg-slate-800/30 hover:bg-slate-100/70 dark:hover:bg-slate-800/60 transition-colors"
                          >
                            <button
                              type="button"
                              data-testid="grid-add-row-btn"
                              onClick={handleAddRow}
                              className="inline-flex items-center gap-2 text-xs font-semibold text-slate-600 dark:text-slate-300 hover:text-blue-600 dark:hover:text-blue-400 cursor-pointer"
                            >
                              <span className="text-base leading-none font-bold">+</span>
                              <span>Add Row</span>
                            </button>
                          </td>
                        </tr>
                      </tbody>
                    </table>
                  </div>
                );
              })()}

              {/* FLOATING BATCH ACTION BAR */}
              {selectedRowIds.size > 0 && (
                <div
                  data-testid="batch-action-bar"
                  className="fixed bottom-6 left-1/2 -translate-x-1/2 z-50 bg-slate-900 dark:bg-slate-800 text-white px-5 py-2.5 rounded-2xl shadow-2xl flex items-center gap-4 border border-slate-700 animate-in fade-in slide-in-from-bottom-4 duration-200"
                >
                  <div className="flex items-center gap-2">
                    <span className="w-2 h-2 rounded-full bg-blue-400 animate-pulse" />
                    <span className="text-xs font-bold font-mono">
                      {selectedRowIds.size} record{selectedRowIds.size > 1 ? "s" : ""} selected
                    </span>
                  </div>
                  <div className="h-4 w-px bg-slate-700" />
                  <button
                    type="button"
                    data-testid="batch-duplicate-btn"
                    onClick={handleBatchDuplicate}
                    className="px-3 py-1 rounded-lg text-xs font-semibold bg-slate-800 hover:bg-slate-700 text-slate-200 cursor-pointer flex items-center gap-1.5 transition-colors"
                  >
                    <span>⧉</span>
                    <span>Duplicate</span>
                  </button>
                  <button
                    type="button"
                    data-testid="batch-delete-btn"
                    onClick={handleBatchDelete}
                    className="px-3 py-1 rounded-lg text-xs font-semibold bg-red-600 hover:bg-red-700 text-white cursor-pointer flex items-center gap-1.5 transition-colors"
                  >
                    <span>🗑</span>
                    <span>Delete</span>
                  </button>
                  <button
                    type="button"
                    data-testid="batch-clear-btn"
                    onClick={() => setSelectedRowIds(new Set())}
                    className="text-xs text-slate-400 hover:text-white cursor-pointer ml-1"
                  >
                    Clear
                  </button>
                </div>
              )}

              {/* RECORD DETAIL DRAWER / MODAL */}
              {(() => {
                if (!selectedRecordId) return null;
                const activeDetailRecord = (activeTable.records || []).find(
                  (r) => r.id === selectedRecordId
                );
                if (!activeDetailRecord) return null;

                return (
                  <div
                    data-testid="record-detail-drawer"
                    className="fixed inset-0 z-50 flex items-center justify-center bg-black/50 backdrop-blur-xs p-4"
                  >
                    <div className="bg-white dark:bg-slate-900 border border-slate-200 dark:border-slate-800 rounded-2xl shadow-2xl max-w-3xl w-full max-h-[90vh] flex flex-col overflow-hidden">
                      {/* DRAWER HEADER */}
                      <div className="p-5 border-b border-slate-100 dark:border-slate-800 flex items-center justify-between bg-slate-50/70 dark:bg-slate-800/40">
                        <div className="space-y-1 flex-1 mr-4">
                          <div className="flex items-center gap-2">
                            <span
                              className="text-xs font-mono px-2 py-0.5 rounded bg-slate-200 dark:bg-slate-700 text-slate-700 dark:text-slate-300 font-bold"
                              data-testid="detail-record-id-badge"
                            >
                              {activeDetailRecord.id}
                            </span>
                            <span className="text-xs text-slate-400 font-medium">
                              in {activeTable.name}
                            </span>
                          </div>
                          <input
                            type="text"
                            data-testid="detail-primary-title-input"
                            value={String(activeDetailRecord[activeTable.primary_field] || "")}
                            onChange={(e) =>
                              handleUpdateRecordField(
                                activeDetailRecord.id,
                                activeTable.primary_field,
                                e.target.value
                              )
                            }
                            className="text-lg font-bold text-slate-900 dark:text-white bg-transparent border-b border-transparent hover:border-slate-300 focus:border-blue-500 focus:outline-none w-full py-0.5 transition-colors"
                            placeholder="Untitled Record"
                          />
                        </div>
                        <div className="flex items-center gap-2">
                          <button
                            type="button"
                            data-testid="detail-duplicate-btn"
                            onClick={() => handleDuplicateRecord(activeDetailRecord.id)}
                            className="p-1.5 text-slate-400 hover:text-emerald-600 rounded-lg border border-slate-200 dark:border-slate-700 hover:bg-slate-100 dark:hover:bg-slate-800 cursor-pointer text-xs"
                            title="Duplicate"
                          >
                            ⧉ Duplicate
                          </button>
                          <button
                            type="button"
                            data-testid="detail-delete-btn"
                            onClick={() => handleDeleteRecord(activeDetailRecord.id)}
                            className="p-1.5 text-slate-400 hover:text-red-600 rounded-lg border border-slate-200 dark:border-slate-700 hover:bg-slate-100 dark:hover:bg-slate-800 cursor-pointer text-xs"
                            title="Delete"
                          >
                            🗑 Delete
                          </button>
                          <button
                            type="button"
                            data-testid="detail-close-btn"
                            onClick={() => setSelectedRecordId(null)}
                            className="w-8 h-8 rounded-lg text-slate-400 hover:text-slate-700 dark:hover:text-white hover:bg-slate-100 dark:hover:bg-slate-800 flex items-center justify-center font-bold cursor-pointer"
                          >
                            ✕
                          </button>
                        </div>
                      </div>

                      {/* DRAWER CONTENT */}
                      <div className="p-6 overflow-y-auto space-y-6 flex-1">
                        {/* FIELDS SECTION */}
                        <div className="space-y-4">
                          <h4 className="text-xs font-bold font-mono text-slate-400 uppercase tracking-wider">
                            Record Properties
                          </h4>
                          <div className="grid grid-cols-1 md:grid-cols-2 gap-4">
                            {activeTable.fields.map((field) => {
                              const isComputed = [
                                "Formula",
                                "Lookup",
                                "Count",
                                "Rollup",
                              ].includes(field.field_type);
                              const computedVal = isComputed
                                ? computeFieldValue(field, activeDetailRecord, tables)
                                : activeDetailRecord[field.name];

                              return (
                                <div
                                  key={field.name}
                                  className="p-3 rounded-xl border border-slate-200 dark:border-slate-800 bg-slate-50/50 dark:bg-slate-800/30 space-y-1.5"
                                >
                                  <div className="flex items-center justify-between">
                                    <label className="text-xs font-semibold text-slate-700 dark:text-slate-300 flex items-center gap-1.5">
                                      <span className="text-slate-400 font-mono text-[11px]">
                                        {getFieldTypeIcon(field.field_type)}
                                      </span>
                                      <span>{field.label}</span>
                                    </label>
                                    {isComputed && (
                                      <span className="text-[10px] font-mono px-1.5 py-0.2 rounded bg-blue-100 dark:bg-blue-900/60 text-blue-700 dark:text-blue-300 font-bold">
                                        Calculated
                                      </span>
                                    )}
                                  </div>

                                  {isComputed ? (
                                    <div
                                      data-testid={`detail-computed-${field.name}`}
                                      className="px-2.5 py-1.5 rounded-lg bg-white dark:bg-slate-900 border border-slate-200 dark:border-slate-700 font-mono text-xs text-slate-800 dark:text-slate-200"
                                    >
                                      {typeof computedVal === "number"
                                        ? computedVal.toLocaleString()
                                        : String(computedVal ?? "")}
                                    </div>
                                  ) : field.field_type === "Checkbox" ? (
                                    <div className="pt-1">
                                      <input
                                        type="checkbox"
                                        data-testid={`detail-input-${field.name}`}
                                        checked={Boolean(activeDetailRecord[field.name])}
                                        onChange={(e) =>
                                          handleUpdateRecordField(
                                            activeDetailRecord.id,
                                            field.name,
                                            e.target.checked
                                          )
                                        }
                                        className="rounded border-slate-300 text-blue-600 focus:ring-blue-500 h-4 w-4 cursor-pointer"
                                      />
                                    </div>
                                  ) : field.field_type === "Select" ? (
                                    <select
                                      data-testid={`detail-input-${field.name}`}
                                      value={String(activeDetailRecord[field.name] || "")}
                                      onChange={(e) =>
                                        handleUpdateRecordField(
                                          activeDetailRecord.id,
                                          field.name,
                                          e.target.value
                                        )
                                      }
                                      className="w-full px-2.5 py-1.5 rounded-lg border border-slate-200 dark:border-slate-700 bg-white dark:bg-slate-900 text-xs text-slate-800 dark:text-white"
                                    >
                                      {(field.select_options || [
                                        "Under Review",
                                        "Approved",
                                        "Funded",
                                        "Rejected",
                                      ]).map((opt) => (
                                        <option key={opt} value={opt}>
                                          {opt}
                                        </option>
                                      ))}
                                    </select>
                                  ) : field.field_type === "Rating" ? (
                                    <div className="flex items-center gap-1 pt-1">
                                      {[1, 2, 3, 4, 5].map((star) => (
                                        <button
                                          key={star}
                                          type="button"
                                          data-testid={`detail-rating-${field.name}-${star}`}
                                          onClick={() =>
                                            handleUpdateRecordField(
                                              activeDetailRecord.id,
                                              field.name,
                                              star
                                            )
                                          }
                                          className={`text-base cursor-pointer ${
                                            star <= Number(activeDetailRecord[field.name] || 0)
                                              ? "text-amber-500"
                                              : "text-slate-300 dark:text-slate-600"
                                          }`}
                                        >
                                          ★
                                        </button>
                                      ))}
                                    </div>
                                  ) : (
                                    <input
                                      type={
                                        field.field_type === "Number" ||
                                        field.field_type === "Currency"
                                          ? "number"
                                          : "text"
                                      }
                                      data-testid={`detail-input-${field.name}`}
                                      value={activeDetailRecord[field.name] ?? ""}
                                      onChange={(e) => {
                                        const v =
                                          field.field_type === "Number" ||
                                          field.field_type === "Currency"
                                            ? Number(e.target.value)
                                            : e.target.value;
                                        handleUpdateRecordField(
                                          activeDetailRecord.id,
                                          field.name,
                                          v
                                        );
                                      }}
                                      className="w-full px-2.5 py-1.5 rounded-lg border border-slate-200 dark:border-slate-700 bg-white dark:bg-slate-900 text-xs text-slate-800 dark:text-white font-sans"
                                    />
                                  )}
                                </div>
                              );
                            })}
                          </div>
                        </div>

                        {/* REVERSE RELATIONAL SUB-TABLES */}
                        <div
                          data-testid="reverse-relation-section"
                          className="space-y-4 pt-4 border-t border-slate-200 dark:border-slate-800"
                        >
                          <h4 className="text-xs font-bold font-mono text-slate-400 uppercase tracking-wider">
                            Reverse Relational Sub-Tables
                          </h4>

                          {(() => {
                            const relatedTables = tables.filter(
                              (t) => t.id !== activeTable.id
                            );
                            const reverseLinks = relatedTables
                              .map((tbl) => {
                                const relField = tbl.fields.find(
                                  (f) =>
                                    (f.field_type === "Relation" &&
                                      f.target_table_id === activeTable.id) ||
                                    f.name ===
                                      `${activeTable.slug.replace(/s$/, "")}_id` ||
                                    (f.name.endsWith("_id") &&
                                      activeTable.slug.includes(
                                        f.name.replace(/_id$/, "")
                                      ))
                                );
                                if (!relField) return null;
                                const linkedRecords = (tbl.records || []).filter(
                                  (r) =>
                                    String(r[relField.name]) ===
                                    String(activeDetailRecord.id)
                                );
                                return { table: tbl, relField, linkedRecords };
                              })
                              .filter(Boolean) as {
                              table: AppTable;
                              relField: any;
                              linkedRecords: Record<string, any>[];
                            }[];

                            if (reverseLinks.length === 0) {
                              return (
                                <p className="text-xs text-slate-400 italic">
                                  No other tables currently reference {activeTable.name}.
                                </p>
                              );
                            }

                            return (
                              <div className="space-y-4">
                                {reverseLinks.map(
                                  ({ table: relTable, relField, linkedRecords }) => (
                                    <div
                                      key={relTable.id}
                                      className="border border-slate-200 dark:border-slate-800 rounded-xl p-4 bg-white dark:bg-slate-900 space-y-3"
                                    >
                                      <div className="flex items-center justify-between">
                                        <div className="flex items-center gap-2">
                                          <span>{relTable.icon || "📑"}</span>
                                          <span className="font-bold text-xs text-slate-900 dark:text-white">
                                            Linked {relTable.name}
                                          </span>
                                          <span className="text-[10px] font-mono px-2 py-0.2 rounded-full bg-purple-50 text-purple-700 dark:bg-purple-950 dark:text-purple-300 font-bold">
                                            {linkedRecords.length} records
                                          </span>
                                        </div>
                                        <button
                                          type="button"
                                          data-testid={`add-linked-${relTable.id}-btn`}
                                          onClick={() => {
                                            const newRelId = `REC-${Date.now()
                                              .toString()
                                              .slice(-4)}`;
                                            const newRelRec: Record<string, any> = {
                                              id: newRelId,
                                              [relField.name]: activeDetailRecord.id,
                                              [relTable.primary_field]: `New ${
                                                relTable.name
                                              } for ${
                                                activeDetailRecord[
                                                  activeTable.primary_field
                                                ]
                                              }`,
                                            };
                                            setTables((prev) =>
                                              prev.map((t) =>
                                                t.id === relTable.id
                                                  ? {
                                                      ...t,
                                                      records: [
                                                        ...(t.records || []),
                                                        newRelRec,
                                                      ],
                                                    }
                                                  : t
                                              )
                                            );
                                          }}
                                          className="text-xs font-semibold text-purple-600 dark:text-purple-400 hover:underline cursor-pointer"
                                        >
                                          + Add Linked {relTable.name}
                                        </button>
                                      </div>

                                      {linkedRecords.length === 0 ? (
                                        <p className="text-xs text-slate-400 italic">
                                          No linked records yet.
                                        </p>
                                      ) : (
                                        <div className="overflow-x-auto border border-slate-100 dark:border-slate-800 rounded-lg">
                                          <table className="w-full text-left text-xs">
                                            <thead>
                                              <tr className="bg-slate-50 dark:bg-slate-800/60 font-mono text-[10px] text-slate-500">
                                                {relTable.fields
                                                  .filter((f) => f.name !== relField.name)
                                                  .slice(0, 4)
                                                  .map((f) => (
                                                    <th key={f.name} className="py-1.5 px-3">
                                                      {f.label}
                                                    </th>
                                                  ))}
                                              </tr>
                                            </thead>
                                            <tbody className="divide-y divide-slate-100 dark:divide-slate-800">
                                              {linkedRecords.map((lr) => (
                                                <tr key={lr.id}>
                                                  {relTable.fields
                                                    .filter(
                                                      (f) => f.name !== relField.name
                                                    )
                                                    .slice(0, 4)
                                                    .map((f) => (
                                                      <td
                                                        key={f.name}
                                                        className="py-1.5 px-3 text-slate-700 dark:text-slate-300"
                                                      >
                                                        {f.field_type === "Currency" ||
                                                        f.name.includes("amount")
                                                          ? `$${Number(
                                                              lr[f.name] || 0
                                                            ).toLocaleString()}`
                                                          : String(lr[f.name] ?? "")}
                                                      </td>
                                                    ))}
                                                </tr>
                                              ))}
                                            </tbody>
                                          </table>
                                        </div>
                                      )}
                                    </div>
                                  )
                                )}
                              </div>
                            );
                          })()}
                        </div>
                      </div>
                    </div>
                  </div>
                );
              })()}

              {/* CSV IMPORT MODAL */}
              {isCsvModalOpen && (
                <div
                  data-testid="csv-import-modal"
                  className="fixed inset-0 z-50 flex items-center justify-center bg-black/50 backdrop-blur-xs p-4"
                >
                  <div className="bg-white dark:bg-slate-900 border border-slate-200 dark:border-slate-800 rounded-2xl shadow-2xl max-w-xl w-full p-6 space-y-4">
                    <div className="flex items-center justify-between pb-3 border-b border-slate-100 dark:border-slate-800">
                      <div>
                        <h3 className="text-base font-bold text-slate-900 dark:text-white flex items-center gap-2">
                          <span>📥</span>
                          <span>Import CSV into {activeTable.name}</span>
                        </h3>
                        <p className="text-xs text-slate-500 mt-0.5">
                          Paste standard RFC 4180 CSV data to map columns and import records.
                        </p>
                      </div>
                      <button
                        type="button"
                        data-testid="csv-modal-close-btn"
                        onClick={() => setIsCsvModalOpen(false)}
                        className="text-slate-400 hover:text-slate-600 dark:hover:text-white font-bold cursor-pointer"
                      >
                        ✕
                      </button>
                    </div>

                    <div className="space-y-2">
                      <label className="text-xs font-semibold text-slate-700 dark:text-slate-300">
                        CSV Text Content:
                      </label>
                      <textarea
                        data-testid="csv-textarea-input"
                        rows={5}
                        value={csvRawText}
                        onChange={(e) => handleParseCsvText(e.target.value)}
                        placeholder={`title,budget,status\n"Advanced Nanomaterials",600000,Approved\n"Deep Sea Robotic Swarm",450000,Under Review`}
                        className="w-full p-3 rounded-lg border border-slate-200 dark:border-slate-700 bg-slate-50 dark:bg-slate-800 font-mono text-xs text-slate-800 dark:text-white focus:outline-blue-500"
                      />
                    </div>

                    {csvParsedHeaders.length > 0 && (
                      <div className="space-y-3 pt-2">
                        <div className="flex items-center justify-between">
                          <span className="text-xs font-bold text-slate-800 dark:text-slate-200">
                            Map Columns ({csvParsedRows.length} rows found)
                          </span>
                        </div>
                        <div className="max-h-48 overflow-y-auto space-y-2 border border-slate-100 dark:border-slate-800 rounded-lg p-3">
                          {csvParsedHeaders.map((header) => (
                            <div
                              key={header}
                              className="flex items-center justify-between text-xs gap-3"
                            >
                              <span className="font-mono font-medium text-slate-700 dark:text-slate-300 truncate w-1/2">
                                {header}
                              </span>
                              <select
                                data-testid={`csv-map-select-${header}`}
                                value={csvFieldMapping[header] || ""}
                                onChange={(e) =>
                                  setCsvFieldMapping((prev) => ({
                                    ...prev,
                                    [header]: e.target.value,
                                  }))
                                }
                                className="px-2 py-1 rounded border border-slate-200 dark:border-slate-700 bg-white dark:bg-slate-800 text-xs w-1/2"
                              >
                                <option value="">(Ignore / Skip)</option>
                                {activeTable.fields.map((f) => (
                                  <option key={f.name} value={f.name}>
                                    {f.label} ({f.name})
                                  </option>
                                ))}
                              </select>
                            </div>
                          ))}
                        </div>
                      </div>
                    )}

                    <div className="flex items-center justify-end gap-3 pt-3 border-t border-slate-100 dark:border-slate-800">
                      <button
                        type="button"
                        onClick={() => setIsCsvModalOpen(false)}
                        className="px-3 py-1.5 rounded-lg text-xs font-semibold text-slate-600 dark:text-slate-300 hover:bg-slate-100 dark:hover:bg-slate-800 cursor-pointer"
                      >
                        Cancel
                      </button>
                      <button
                        type="button"
                        data-testid="csv-execute-import-btn"
                        disabled={csvParsedRows.length === 0}
                        onClick={handleExecuteCsvImport}
                        className="px-4 py-1.5 rounded-lg text-xs font-bold bg-blue-600 hover:bg-blue-700 disabled:opacity-50 text-white cursor-pointer shadow-xs"
                      >
                        Import {csvParsedRows.length} Record
                        {csvParsedRows.length === 1 ? "" : "s"}
                      </button>
                    </div>
                  </div>
                </div>
              )}


                {/* ADD VIEW MODAL */}
                {isAddViewModalOpen && (
                  <div
                    data-testid="modal-add-view"
                    className="fixed inset-0 bg-slate-900/60 backdrop-blur-xs flex items-center justify-center z-50 p-4"
                  >
                    <div className="bg-white dark:bg-slate-900 border border-slate-200 dark:border-slate-800 rounded-2xl max-w-md w-full p-6 shadow-2xl space-y-4">
                      <div className="flex items-center justify-between pb-3 border-b border-slate-100 dark:border-slate-800">
                        <h3 className="text-base font-bold text-slate-900 dark:text-white">
                          Create New Saved View
                        </h3>
                        <button
                          type="button"
                          onClick={() => setIsAddViewModalOpen(false)}
                          className="text-slate-400 hover:text-slate-600 text-sm font-bold"
                        >
                          ✕
                        </button>
                      </div>

                      <form onSubmit={handleCreateNewView} className="space-y-4 text-xs">
                        <div>
                          <label className="block text-xs font-semibold text-slate-700 dark:text-slate-300 mb-1">
                            View Title
                          </label>
                          <input
                            type="text"
                            required
                            placeholder="e.g. Approved Grants Kanban"
                            value={newViewTitle}
                            onChange={(e) => setNewViewTitle(e.target.value)}
                            className="w-full px-3 py-2 rounded-lg border border-slate-300 dark:border-slate-700 bg-slate-50 dark:bg-slate-800 text-slate-900 dark:text-white"
                          />
                        </div>

                        <div>
                          <label className="block text-xs font-semibold text-slate-700 dark:text-slate-300 mb-1">
                            View Type
                          </label>
                          <div className="grid grid-cols-2 gap-2">
                            {(["Grid", "Kanban", "Calendar", "Gallery"] as ViewType[]).map((vt) => (
                              <button
                                key={vt}
                                type="button"
                                onClick={() => setNewViewType(vt)}
                                className={`p-2.5 rounded-lg border text-left cursor-pointer transition-all ${
                                  newViewType === vt
                                    ? "border-blue-600 bg-blue-50/60 dark:bg-blue-950/60 text-blue-700 dark:text-blue-300"
                                    : "border-slate-200 dark:border-slate-700 hover:bg-slate-50 dark:hover:bg-slate-800 text-slate-700 dark:text-slate-300"
                                }`}
                              >
                                <div className="font-bold flex items-center gap-1.5">
                                  <span>
                                    {vt === "Grid"
                                      ? "▦"
                                      : vt === "Kanban"
                                      ? "☷"
                                      : vt === "Calendar"
                                      ? "📅"
                                      : "🖼"}
                                  </span>
                                  <span>{vt}</span>
                                </div>
                                <div className="text-[10px] text-slate-500 mt-0.5">
                                  {vt === "Grid"
                                    ? "Tabular records matrix"
                                    : vt === "Kanban"
                                    ? "Card stage progression"
                                    : vt === "Calendar"
                                    ? "Timeline agenda"
                                    : "Visual card gallery"}
                                </div>
                              </button>
                            ))}
                          </div>
                        </div>

                        <div className="flex items-center justify-end gap-2 pt-3 border-t border-slate-100 dark:border-slate-800">
                          <button
                            type="button"
                            onClick={() => setIsAddViewModalOpen(false)}
                            className="px-3.5 py-1.5 rounded-lg border border-slate-300 dark:border-slate-700 text-slate-600 dark:text-slate-400 font-semibold cursor-pointer"
                          >
                            Cancel
                          </button>
                          <button
                            type="submit"
                            className="px-4 py-1.5 rounded-lg bg-blue-600 hover:bg-blue-700 text-white font-bold shadow-xs cursor-pointer"
                          >
                            Create View
                          </button>
                        </div>
                      </form>
                    </div>
                  </div>
                )}
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

        {/* TAB 3: STANDALONE INTAKE FORMS */}
        {activeTab === "forms" && (
          <div className="flex-1 overflow-y-auto p-6 bg-slate-100/50 dark:bg-slate-950/50">
            <div className="max-w-4xl mx-auto space-y-6">
              <div className="bg-white dark:bg-slate-900 border border-slate-200 dark:border-slate-800 rounded-xl p-5 shadow-xs flex items-center justify-between flex-wrap gap-4">
                <div>
                  <h3 className="text-sm font-bold text-slate-900 dark:text-white">
                    Standalone Public Intake Forms
                  </h3>
                  <p className="text-xs text-slate-500 mt-1">
                    Live end-user form intake previews, direct submission endpoints, and public share URLs.
                  </p>
                </div>
                {onOpenIntakeForm && (
                  <button
                    type="button"
                    data-testid="launch-public-form-btn"
                    onClick={() => onOpenIntakeForm(activeTable.id)}
                    className="flex items-center gap-1.5 px-3.5 py-2 rounded-lg bg-blue-600 hover:bg-blue-700 text-white text-xs font-bold shadow-xs transition-colors cursor-pointer"
                  >
                    ↗ Open Full-Screen Standalone Form
                  </button>
                )}
              </div>

              {/* Embedded Standalone Form Component */}
              <div className="rounded-2xl border border-slate-200 dark:border-slate-800 shadow-xl overflow-hidden bg-white dark:bg-slate-900">
                <StandaloneIntakeForm
                  app={app}
                  tableId={activeTable.id}
                  onRecordSubmitted={(tblId, rec) => {
                    setTables((prev) =>
                      prev.map((t) =>
                        t.id === tblId
                          ? { ...t, records: [rec, ...(t.records || [])] }
                          : t
                      )
                    );
                    setSaveStatus(`Form submission recorded into ${tblId}`);
                    setTimeout(() => setSaveStatus(null), 3000);
                  }}
                />
              </div>
            </div>
          </div>
        )}

        {/* TAB 4: WORKFLOW AUTOMATIONS */}
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
