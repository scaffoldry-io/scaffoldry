import React, { useState, useMemo } from "react";
import { ManifestRenderer } from "./ManifestRenderer";
import { RegisteredApp } from "./types";
import { DataGrid } from "./DataGrid";

interface Props {
  app: RegisteredApp;
  onBack: () => void;
  onOpenStudio: () => void;
  onRecordCreated?: (record: Record<string, unknown>) => void;
}

type TabularViewMode = "grid" | "kanban" | "calendar" | "gallery" | "form";

interface TabularRecord {
  id: string;
  [key: string]: unknown;
}

const SEEDED_RECORDS_BY_SLUG: Record<string, TabularRecord[]> = {
  "admissions-fellowship-eval": [
    {
      id: "rec-101",
      applicant_id: "STU-88219",
      applicant_name: "Eleanor Vance",
      gpa: 3.92,
      department: "Physics",
      reviewer_faculty: "dr.smith@university.edu",
      committee_decision: "Approved",
      submission_date: "2026-10-02",
      stipend_amount: 38000,
    },
    {
      id: "rec-102",
      applicant_id: "STU-88220",
      applicant_name: "Julian Bashir",
      gpa: 3.84,
      department: "Biology",
      reviewer_faculty: "dr.curie@science.state.edu",
      committee_decision: "Under Review",
      submission_date: "2026-10-03",
      stipend_amount: 36500,
    },
    {
      id: "rec-103",
      applicant_id: "STU-88221",
      applicant_name: "Ada Lovelace",
      gpa: 4.0,
      department: "Computer Science",
      reviewer_faculty: "dr.alan@university.edu",
      committee_decision: "Approved",
      submission_date: "2026-10-04",
      stipend_amount: 42000,
    },
    {
      id: "rec-104",
      applicant_id: "STU-88222",
      applicant_name: "Arthur Dent",
      gpa: 3.15,
      department: "Physics",
      reviewer_faculty: "dr.smith@university.edu",
      committee_decision: "Draft",
      submission_date: "2026-10-05",
      stipend_amount: 32000,
    },
  ],
  "lab-safety-inspection": [
    {
      id: "rec-201",
      lab_id: "LAB-BIO-402",
      inspector: "Dr. Marie Curie",
      hazard_level: "High",
      chemical_inventory_certified: true,
      inspection_date: "2026-10-01",
      status: "Approved",
      deficiencies_count: 0,
    },
    {
      id: "rec-202",
      lab_id: "LAB-PHYS-108",
      inspector: "Dr. Sarah Smith",
      hazard_level: "Moderate",
      chemical_inventory_certified: true,
      inspection_date: "2026-10-03",
      status: "Under Review",
      deficiencies_count: 2,
    },
    {
      id: "rec-203",
      lab_id: "LAB-CHEM-301",
      inspector: "Dr. Alan Turing",
      hazard_level: "Critical",
      chemical_inventory_certified: false,
      inspection_date: "2026-10-04",
      status: "Action Required",
      deficiencies_count: 5,
    },
  ],
};

export const MultiViewWorkspace: React.FC<Props> = ({
  app,
  onBack,
  onOpenStudio,
  onRecordCreated,
}) => {
  const [viewMode, setViewMode] = useState<TabularViewMode>("grid");
  const [searchQuery, setSearchQuery] = useState("");

  // Dynamic records stored in state
  const [records, setRecords] = useState<TabularRecord[]>(() => {
    if (SEEDED_RECORDS_BY_SLUG[app.slug]) {
      return SEEDED_RECORDS_BY_SLUG[app.slug];
    }
    // Generate starter records based on fields
    const fields = app.manifest.views[0]?.fields || [];
    return [
      {
        id: "rec-001",
        ...Object.fromEntries(
          fields.map((f, i) => [
            f.name,
            f.field_type === "Number"
              ? (i + 1) * 15
              : f.field_type === "Relation"
              ? "dr.smith@university.edu"
              : `${f.label} Sample 1`,
          ])
        ),
        status: "Approved",
        submission_date: "2026-10-02",
      },
      {
        id: "rec-002",
        ...Object.fromEntries(
          fields.map((f, i) => [
            f.name,
            f.field_type === "Number"
              ? (i + 1) * 25
              : f.field_type === "Relation"
              ? "dr.curie@science.state.edu"
              : `${f.label} Sample 2`,
          ])
        ),
        status: "Under Review",
        submission_date: "2026-10-04",
      },
    ];
  });

  const fields = app.manifest.views[0]?.fields || [];

  // Filtered records based on search query
  const filteredRecords = useMemo(() => {
    if (!searchQuery.trim()) return records;
    const q = searchQuery.toLowerCase();
    return records.filter((r) =>
      Object.values(r).some((v) => String(v).toLowerCase().includes(q))
    );
  }, [records, searchQuery]);

  // Handle cell edit
  const handleUpdateCell = (recordId: string, fieldName: string, value: unknown) => {
    setRecords((prev) =>
      prev.map((r) => (r.id === recordId ? { ...r, [fieldName]: value } : r))
    );
  };

  // Add new blank record
  const handleAddNewRecord = () => {
    const newId = `rec-${Date.now().toString().slice(-4)}`;
    const newRow: TabularRecord = {
      id: newId,
      ...Object.fromEntries(
        fields.map((f) => [
          f.name,
          f.field_type === "Number" ? 0 : f.field_type === "Boolean" ? false : "",
        ])
      ),
      status: "Draft",
      submission_date: new Date().toISOString().split("T")[0],
    };
    setRecords((prev) => [newRow, ...prev]);
  };

  // Handle Form View submission
  const handleFormSubmit = (data: Record<string, unknown>) => {
    const newId = `rec-${Date.now().toString().slice(-4)}`;
    const newRow: TabularRecord = {
      id: newId,
      ...data,
      status: "Approved",
      submission_date: new Date().toISOString().split("T")[0],
    };
    setRecords((prev) => [newRow, ...prev]);
    if (onRecordCreated) onRecordCreated(newRow);
  };


  // Export records as CSV
  const handleExportCsv = () => {
    const headers = ["ID", ...fields.map((f) => f.label), "Status", "Date"];
    const rows = filteredRecords.map((r) => [
      r.id,
      ...fields.map((f) => `"${String(r[f.name] ?? "").replace(/"/g, '""')}"`),
      r.status ?? "",
      r.submission_date ?? "",
    ]);
    const csvContent =
      "data:text/csv;charset=utf-8," +
      [headers.join(","), ...rows.map((row) => row.join(","))].join("\n");
    const encodedUri = encodeURI(csvContent);
    const link = document.createElement("a");
    link.setAttribute("href", encodedUri);
    link.setAttribute("download", `${app.slug}-export-${Date.now()}.csv`);
    document.body.appendChild(link);
    link.click();
    document.body.removeChild(link);
  };

  // Kanban status columns
  const kanbanStatuses = ["Draft", "Under Review", "Approved", "Action Required"];

  return (
    <div className="flex-1 flex flex-col h-full bg-slate-50 dark:bg-slate-950 text-slate-900 dark:text-slate-100 overflow-hidden">
      {/* Top Application Bar */}
      <header className="px-6 py-3.5 bg-white dark:bg-slate-900 border-b border-slate-200 dark:border-slate-800 flex flex-wrap items-center justify-between gap-4 shrink-0 shadow-xs">
        <div className="flex items-center gap-3">
          <button
            type="button"
            onClick={onBack}
            className="p-1.5 rounded-lg border border-slate-200 dark:border-slate-800 hover:bg-slate-100 dark:hover:bg-slate-800 text-slate-600 dark:text-slate-300 text-xs font-semibold flex items-center gap-1.5 cursor-pointer transition-colors"
          >
            <span>←</span>
            <span>All Apps</span>
          </button>

          <div>
            <div className="flex items-center gap-2">
              <h1 className="text-base font-bold text-slate-900 dark:text-white">
                {app.title}
              </h1>
              <span className="px-2 py-0.5 rounded text-[10px] font-bold uppercase tracking-wider bg-blue-50 text-blue-700 dark:bg-blue-950/60 dark:text-blue-300 border border-blue-200 dark:border-blue-800">
                {app.department}
              </span>
              <span className="px-2 py-0.5 rounded text-[10px] font-medium bg-emerald-50 text-emerald-700 dark:bg-emerald-950/60 dark:text-emerald-300 border border-emerald-200 dark:border-emerald-800">
                ✓ Governed
              </span>
            </div>
            <div className="flex items-center gap-3 text-xs text-slate-500 dark:text-slate-400 mt-0.5">
              <span className="font-mono text-[11px] text-blue-600 dark:text-blue-400">
                🔗 {app.customDomain}
              </span>
              <span>·</span>
              <span>
                {filteredRecords.length} records · {fields.length} schema fields
              </span>
            </div>
          </div>
        </div>

        {/* Right Top Actions */}
        <div className="flex items-center gap-2">
          <button
            type="button"
            onClick={handleExportCsv}
            className="px-3 py-1.5 rounded-lg border border-slate-200 dark:border-slate-800 hover:bg-slate-50 dark:hover:bg-slate-800 text-slate-700 dark:text-slate-300 text-xs font-semibold cursor-pointer transition-colors"
          >
            📥 Export CSV
          </button>

          <button
            type="button"
            onClick={onOpenStudio}
            className="px-3 py-1.5 rounded-lg border border-purple-200 dark:border-purple-800 bg-purple-50 dark:bg-purple-950/40 hover:bg-purple-100 dark:hover:bg-purple-900/60 text-purple-700 dark:text-purple-300 text-xs font-semibold cursor-pointer transition-colors"
          >
            🛠️ Configure in Studio
          </button>

          <button
            type="button"
            onClick={handleAddNewRecord}
            className="px-3.5 py-1.5 rounded-lg bg-blue-600 hover:bg-blue-700 text-white text-xs font-semibold shadow-xs cursor-pointer transition-colors"
          >
            + Add Record
          </button>
        </div>
      </header>

      {/* Views Navigation Toolbar */}
      <div className="px-6 py-2.5 bg-slate-100/70 dark:bg-slate-900/40 border-b border-slate-200 dark:border-slate-800 flex flex-wrap items-center justify-between gap-3 text-xs shrink-0">
        {/* View Mode Buttons */}
        <div className="flex items-center gap-1 bg-white dark:bg-slate-900 p-1 rounded-lg border border-slate-200 dark:border-slate-800 shadow-xs">
          <button
            type="button"
            onClick={() => setViewMode("grid")}
            className={`px-3 py-1.5 rounded-md font-semibold cursor-pointer transition-colors flex items-center gap-1.5 ${
              viewMode === "grid"
                ? "bg-blue-600 text-white shadow-xs"
                : "text-slate-600 dark:text-slate-400 hover:text-slate-900 dark:hover:text-white"
            }`}
          >
            <span>▦</span>
            <span>Grid View</span>
          </button>

          <button
            type="button"
            onClick={() => setViewMode("kanban")}
            className={`px-3 py-1.5 rounded-md font-semibold cursor-pointer transition-colors flex items-center gap-1.5 ${
              viewMode === "kanban"
                ? "bg-blue-600 text-white shadow-xs"
                : "text-slate-600 dark:text-slate-400 hover:text-slate-900 dark:hover:text-white"
            }`}
          >
            <span>☷</span>
            <span>Kanban Board</span>
          </button>

          <button
            type="button"
            onClick={() => setViewMode("calendar")}
            className={`px-3 py-1.5 rounded-md font-semibold cursor-pointer transition-colors flex items-center gap-1.5 ${
              viewMode === "calendar"
                ? "bg-blue-600 text-white shadow-xs"
                : "text-slate-600 dark:text-slate-400 hover:text-slate-900 dark:hover:text-white"
            }`}
          >
            <span>📅</span>
            <span>Calendar View</span>
          </button>

          <button
            type="button"
            onClick={() => setViewMode("gallery")}
            className={`px-3 py-1.5 rounded-md font-semibold cursor-pointer transition-colors flex items-center gap-1.5 ${
              viewMode === "gallery"
                ? "bg-blue-600 text-white shadow-xs"
                : "text-slate-600 dark:text-slate-400 hover:text-slate-900 dark:hover:text-white"
            }`}
          >
            <span>🪟</span>
            <span>Gallery</span>
          </button>

          <button
            type="button"
            onClick={() => setViewMode("form")}
            className={`px-3 py-1.5 rounded-md font-semibold cursor-pointer transition-colors flex items-center gap-1.5 ${
              viewMode === "form"
                ? "bg-blue-600 text-white shadow-xs"
                : "text-slate-600 dark:text-slate-400 hover:text-slate-900 dark:hover:text-white"
            }`}
          >
            <span>📝</span>
            <span>Intake Form</span>
          </button>
        </div>

        {/* Search and Filters */}
        <div className="flex items-center gap-2">
          <div className="relative">
            <input
              type="text"
              value={searchQuery}
              onChange={(e) => setSearchQuery(e.target.value)}
              placeholder="Filter records in view..."
              className="w-56 px-3 py-1.5 text-xs rounded-lg bg-white dark:bg-slate-900 border border-slate-200 dark:border-slate-800 text-slate-800 dark:text-slate-200 placeholder-slate-400 focus:outline-none focus:ring-1 focus:ring-blue-500"
            />
            {searchQuery && (
              <button
                type="button"
                onClick={() => setSearchQuery("")}
                className="absolute right-2.5 top-1.5 text-slate-400 hover:text-slate-600"
              >
                ✕
              </button>
            )}
          </div>


        </div>
      </div>

      {/* Main Viewport Container */}
      <div className="flex-1 overflow-auto p-4 md:p-6">
        {/* VIEW 1: REACTIVE SPREADSHEET GRID */}
        {viewMode === "grid" && (
          <div className="bg-white dark:bg-slate-900 rounded-xl border border-slate-200 dark:border-slate-800 shadow-xs flex flex-col overflow-hidden">
            <DataGrid
              fields={fields}
              records={filteredRecords}
              tables={app.manifest.tables}
              rowDensity={app.manifest.views[0]?.row_density}
              onPatch={handleUpdateCell}
            />

            {/* Quick Add Row */}
            <div className="p-3 border-t border-slate-200 dark:border-slate-800 bg-slate-50/50 dark:bg-slate-900/50">
              <button
                type="button"
                onClick={handleAddNewRecord}
                className="inline-flex items-center gap-1.5 px-3 py-1.5 text-xs font-semibold text-blue-600 dark:text-blue-400 hover:text-blue-700 dark:hover:text-blue-300 cursor-pointer"
              >
                <span>+ Click to add a new record</span>
              </button>
            </div>
          </div>
        )}

        {/* VIEW 2: KANBAN BOARD */}
        {viewMode === "kanban" && (
          <div className="flex gap-4 overflow-x-auto pb-4 items-start">
            {kanbanStatuses.map((statusCol) => {
              const colRecords = filteredRecords.filter(
                (r) => (r.status || "Draft") === statusCol
              );

              return (
                <div
                  key={statusCol}
                  className="w-72 shrink-0 bg-slate-100 dark:bg-slate-900/80 rounded-xl border border-slate-200 dark:border-slate-800 flex flex-col max-h-[75vh]"
                >
                  {/* Column Header */}
                  <div className="p-3 border-b border-slate-200 dark:border-slate-800 flex items-center justify-between">
                    <div className="flex items-center gap-2">
                      <span className="font-bold text-xs text-slate-900 dark:text-white">
                        {statusCol}
                      </span>
                      <span className="px-1.5 py-0.2 rounded-full text-[10px] font-mono bg-white dark:bg-slate-800 text-slate-600 dark:text-slate-300 font-bold">
                        {colRecords.length}
                      </span>
                    </div>
                  </div>

                  {/* Cards List */}
                  <div className="p-2 space-y-2 overflow-y-auto flex-1">
                    {colRecords.map((r) => (
                      <div
                        key={r.id}
                        className="bg-white dark:bg-slate-800 border border-slate-200 dark:border-slate-700/60 rounded-lg p-3 shadow-xs space-y-2 hover:border-blue-400 transition-all cursor-pointer"
                      >
                        <div className="flex items-center justify-between">
                          <span className="font-mono text-[10px] text-slate-400">{r.id}</span>
                          <span className="font-mono text-[10px] text-slate-400">
                            {String(r.submission_date || "")}
                          </span>
                        </div>

                        <div className="font-bold text-xs text-slate-900 dark:text-white">
                          {String(r[fields[0]?.name] || "Untitled Record")}
                        </div>

                        {/* Additional fields */}
                        <div className="space-y-1 text-[11px] text-slate-500 dark:text-slate-400">
                          {fields.slice(1, 3).map((f) => (
                            <div key={f.name} className="flex justify-between items-center">
                              <span className="text-slate-400">{f.label}:</span>
                              <span className="font-medium text-slate-700 dark:text-slate-300 truncate max-w-[140px]">
                                {String(r[f.name] ?? "—")}
                              </span>
                            </div>
                          ))}
                        </div>

                        {/* Move stage selector */}
                        <div className="pt-2 border-t border-slate-100 dark:border-slate-700/60 flex items-center justify-between text-[10px]">
                          <span className="text-slate-400">Move:</span>
                          <div className="flex gap-1">
                            {kanbanStatuses
                              .filter((s) => s !== statusCol)
                              .slice(0, 2)
                              .map((targetStatus) => (
                                <button
                                  key={targetStatus}
                                  type="button"
                                  onClick={(e) => {
                                    e.stopPropagation();
                                    handleUpdateCell(r.id, "status", targetStatus);
                                  }}
                                  className="px-1.5 py-0.5 rounded bg-slate-100 dark:bg-slate-700 hover:bg-blue-100 dark:hover:bg-blue-900 hover:text-blue-700 text-slate-600 dark:text-slate-300 cursor-pointer"
                                >
                                  → {targetStatus}
                                </button>
                              ))}
                          </div>
                        </div>
                      </div>
                    ))}

                    {colRecords.length === 0 && (
                      <div className="p-4 text-center text-slate-400 text-xs border border-dashed border-slate-200 dark:border-slate-800 rounded-lg">
                        No records in this stage
                      </div>
                    )}
                  </div>
                </div>
              );
            })}
          </div>
        )}

        {/* VIEW 3: CALENDAR VIEW */}
        {viewMode === "calendar" && (
          <div className="bg-white dark:bg-slate-900 rounded-xl border border-slate-200 dark:border-slate-800 p-5 shadow-xs space-y-4">
            <div className="flex items-center justify-between">
              <h3 className="font-bold text-sm text-slate-900 dark:text-white">
                October 2026 Scheduled Events &amp; Submissions
              </h3>
              <span className="text-xs text-slate-500 font-mono">Sovereign Date Lattice</span>
            </div>

            {/* Calendar Grid (October 2026) */}
            <div className="grid grid-cols-7 gap-2">
              {["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"].map((day) => (
                <div
                  key={day}
                  className="text-center font-bold text-xs text-slate-400 py-1 border-b border-slate-200 dark:border-slate-800"
                >
                  {day}
                </div>
              ))}

              {/* 31 Days of October 2026 (starting on Thursday) */}
              {[null, null, null, null, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27, 28, 29, 30, 31].map(
                (dayNum, idx) => {
                  if (dayNum === null) {
                    return (
                      <div
                        key={`empty-${idx}`}
                        className="min-h-[90px] p-2 bg-slate-50/40 dark:bg-slate-950/40 rounded-lg border border-transparent"
                      />
                    );
                  }

                  const dateStr = `2026-10-${String(dayNum).padStart(2, "0")}`;
                  const dayRecords = filteredRecords.filter(
                    (r) => (r.submission_date || "2026-10-04") === dateStr
                  );

                  return (
                    <div
                      key={`day-${dayNum}`}
                      className="min-h-[90px] p-2 bg-slate-50/70 dark:bg-slate-800/30 rounded-lg border border-slate-200 dark:border-slate-800 flex flex-col justify-between"
                    >
                      <div className="flex items-center justify-between text-xs">
                        <span className="font-bold text-slate-700 dark:text-slate-300">
                          {dayNum}
                        </span>
                        {dayRecords.length > 0 && (
                          <span className="w-1.5 h-1.5 rounded-full bg-blue-500" />
                        )}
                      </div>

                      <div className="space-y-1 my-1">
                        {dayRecords.map((r) => (
                          <div
                            key={r.id}
                            className="p-1 rounded bg-blue-100 text-blue-800 dark:bg-blue-950 dark:text-blue-300 text-[10px] font-semibold truncate"
                            title={String(r[fields[0]?.name] || r.id)}
                          >
                            {String(r[fields[0]?.name] || r.id)}
                          </div>
                        ))}
                      </div>
                    </div>
                  );
                }
              )}
            </div>
          </div>
        )}

        {/* VIEW 4: GALLERY CARDS */}
        {viewMode === "gallery" && (
          <div className="grid grid-cols-1 md:grid-cols-2 lg:grid-cols-3 gap-4">
            {filteredRecords.map((r) => (
              <div
                key={r.id}
                className="bg-white dark:bg-slate-900 rounded-xl border border-slate-200 dark:border-slate-800 p-5 shadow-xs space-y-3 hover:border-blue-400 transition-all"
              >
                <div className="flex items-center justify-between">
                  <span className="font-mono text-xs font-semibold text-slate-400">{r.id}</span>
                  <span
                    className={`px-2 py-0.5 rounded text-[10px] font-semibold ${
                      r.status === "Approved"
                        ? "bg-emerald-50 text-emerald-700 dark:bg-emerald-950/60 dark:text-emerald-300 border border-emerald-200 dark:border-emerald-800"
                        : "bg-blue-50 text-blue-700 dark:bg-blue-950/60 dark:text-blue-300 border border-blue-200 dark:border-blue-800"
                    }`}
                  >
                    {String(r.status || "Draft")}
                  </span>
                </div>

                <h3 className="font-bold text-sm text-slate-900 dark:text-white">
                  {String(r[fields[0]?.name] || "Untitled Record")}
                </h3>

                <div className="space-y-1.5 text-xs text-slate-600 dark:text-slate-400 pt-2 border-t border-slate-100 dark:border-slate-800">
                  {fields.slice(1).map((f) => (
                    <div key={f.name} className="flex justify-between items-center text-xs">
                      <span className="text-slate-400">{f.label}:</span>
                      <span className="font-medium text-slate-800 dark:text-slate-200 truncate max-w-[160px]">
                        {String(r[f.name] ?? "—")}
                      </span>
                    </div>
                  ))}
                </div>
              </div>
            ))}
          </div>
        )}

        {/* VIEW 5: INTAKE FORM */}
        {viewMode === "form" && (
          <div className="max-w-2xl mx-auto">
            <ManifestRenderer manifest={app.manifest} onSubmitRecord={handleFormSubmit} />
          </div>
        )}
      </div>
    </div>
  );
};
