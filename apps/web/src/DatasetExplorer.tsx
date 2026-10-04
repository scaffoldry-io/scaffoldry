import React, { useState } from "react";
import { DatasetRelationship, PublishedDataset } from "./types";

export const SEEDED_DATASETS: PublishedDataset[] = [
  {
    id: "faculty",
    name: "Faculty & Principal Investigator Directory",
    description: "Authoritative university faculty roster, academic titles, departments, and research specialties.",
    department: "Academic Affairs",
    organization: "University",
    sensitivity_level: "Directory",
    herm_capability_id: "HR-01-ROSTER",
    record_count: 240,
    published_at: "2026-10-04T12:00:00Z",
    fields: [
      { name: "eppn", label: "Identity (ePPN)", field_type: "Text", required: true, ferpa_sensitive: false, ceds_code: "000033" },
      { name: "full_name", label: "Full Name", field_type: "Text", required: true, ferpa_sensitive: false, ceds_code: "000115" },
      { name: "title", label: "Academic Title", field_type: "Text", required: true, ferpa_sensitive: false },
      { name: "department", label: "Department", field_type: "Text", required: true, ferpa_sensitive: false },
      { name: "research_specialty", label: "Research Specialty", field_type: "Text", required: false, ferpa_sensitive: false },
    ],
    sample_data: [
      { eppn: "dr.smith@university.edu", full_name: "Dr. Sarah Smith", title: "Professor of Physics", department: "Physics", research_specialty: "Quantum Lattice Systems" },
      { eppn: "dr.curie@science.state.edu", full_name: "Dr. Marie Curie", title: "Lab Director & Professor", department: "Biology", research_specialty: "Radiological Cellular Assays" },
      { eppn: "dr.alan@university.edu", full_name: "Dr. Alan Turing", title: "Chair of Computing", department: "Computer Science", research_specialty: "Automata & Cryptography" },
      { eppn: "einstein@physics.state.edu", full_name: "Albert Einstein", title: "Research Fellow", department: "Physics", research_specialty: "Relativistic Optics" },
    ],
  },
  {
    id: "programs",
    name: "Academic Programs & Degrees",
    description: "Accredited degree plans, CIP classification taxonomy, and governing faculty boards.",
    department: "Provost & Academic Council",
    organization: "University",
    sensitivity_level: "Public",
    herm_capability_id: "ACA-02-CURRICULUM",
    record_count: 58,
    published_at: "2026-10-04T12:00:00Z",
    fields: [
      { name: "code", label: "Program Code", field_type: "Text", required: true, ferpa_sensitive: false, ceds_code: "000067" },
      { name: "degree_name", label: "Degree Name", field_type: "Text", required: true, ferpa_sensitive: false },
      { name: "department", label: "Governing Department", field_type: "Text", required: true, ferpa_sensitive: false },
      { name: "degree_level", label: "Level", field_type: "Text", required: true, ferpa_sensitive: false },
    ],
    sample_data: [
      { code: "PHYS-PHD", degree_name: "Doctor of Philosophy in Physics", department: "Physics", degree_level: "Doctoral" },
      { code: "BIO-MS", degree_name: "Master of Science in Molecular Biology", department: "Biology", degree_level: "Graduate" },
      { code: "CS-BS", degree_name: "Bachelor of Science in Computer Science", department: "Computer Science", degree_level: "Undergraduate" },
    ],
  },
  {
    id: "courses",
    name: "University Course Catalog",
    description: "Active semester course offerings, credit weights, and assigned faculty instructors.",
    department: "Office of the Registrar",
    organization: "University",
    sensitivity_level: "Directory",
    herm_capability_id: "REG-01-CATALOG",
    record_count: 840,
    published_at: "2026-10-04T12:00:00Z",
    fields: [
      { name: "course_code", label: "Course Code", field_type: "Text", required: true, ferpa_sensitive: false, ceds_code: "000062" },
      { name: "course_title", label: "Course Title", field_type: "Text", required: true, ferpa_sensitive: false, ceds_code: "000064" },
      { name: "credits", label: "Credit Units", field_type: "Number", required: true, ferpa_sensitive: false },
      { name: "instructor_eppn", label: "Lead Instructor", field_type: "Relation", required: true, ferpa_sensitive: false },
      { name: "program_code", label: "Program Plan", field_type: "Relation", required: true, ferpa_sensitive: false },
    ],
    sample_data: [
      { course_code: "PHYS-401", course_title: "Quantum Mechanics I", credits: 4, instructor_eppn: "dr.smith@university.edu", program_code: "PHYS-PHD" },
      { course_code: "BIO-305", course_title: "Cellular Biochemistry Lab", credits: 3, instructor_eppn: "dr.curie@science.state.edu", program_code: "BIO-MS" },
      { course_code: "CS-302", course_title: "Theory of Computation", credits: 3, instructor_eppn: "dr.alan@university.edu", program_code: "CS-BS" },
      { course_code: "PHYS-510", course_title: "Advanced Laser Optics", credits: 3, instructor_eppn: "einstein@physics.state.edu", program_code: "PHYS-PHD" },
    ],
  },
  {
    id: "grants",
    name: "Sponsored Research Projects & Grants",
    description: "Federal, institutional, and foundation grant awards with restricted student research assistantships.",
    department: "Office of Sponsored Research",
    organization: "University",
    sensitivity_level: "Restricted / FERPA",
    herm_capability_id: "RES-01-GRANTS",
    record_count: 115,
    published_at: "2026-10-04T12:00:00Z",
    fields: [
      { name: "award_number", label: "Award Identifier", field_type: "Text", required: true, ferpa_sensitive: false },
      { name: "project_title", label: "Project Title", field_type: "Text", required: true, ferpa_sensitive: false },
      { name: "pi_eppn", label: "Principal Investigator", field_type: "Relation", required: true, ferpa_sensitive: false },
      { name: "amount", label: "Award Amount ($)", field_type: "Number", required: true, ferpa_sensitive: false },
      { name: "has_student_aid", label: "FERPA Student Stipends", field_type: "Boolean", required: false, ferpa_sensitive: true },
    ],
    sample_data: [
      { award_number: "NSF-PHY-2026-01", project_title: "Quantum Lattice Topological Phases", pi_eppn: "dr.smith@university.edu", amount: 750000, has_student_aid: true },
      { award_number: "NIH-BIO-2025-99", project_title: "Radiological DNA Repair Mechanisms", pi_eppn: "dr.curie@science.state.edu", amount: 1200000, has_student_aid: true },
      { award_number: "DARPA-CS-2026-04", project_title: "Cryptographic Formal Verification", pi_eppn: "dr.alan@university.edu", amount: 450000, has_student_aid: false },
    ],
  },
];

export const SEEDED_RELATIONSHIPS: DatasetRelationship[] = [
  {
    id: "rel_course_instructor",
    name: "Course Instructor Reference",
    source_dataset_id: "courses",
    target_dataset_id: "faculty",
    source_field: "instructor_eppn",
    target_field: "eppn",
    relationship_type: "OneToMany",
    display_field: "full_name",
  },
  {
    id: "rel_course_program",
    name: "Course Program Affiliation",
    source_dataset_id: "courses",
    target_dataset_id: "programs",
    source_field: "program_code",
    target_field: "code",
    relationship_type: "OneToMany",
    display_field: "degree_name",
  },
  {
    id: "rel_grant_pi",
    name: "Research PI Reference",
    source_dataset_id: "grants",
    target_dataset_id: "faculty",
    source_field: "pi_eppn",
    target_field: "eppn",
    relationship_type: "OneToMany",
    display_field: "full_name",
  },
];

interface DatasetExplorerProps {
  onUseInApp?: (dataset: PublishedDataset) => void;
}

export const DatasetExplorer: React.FC<DatasetExplorerProps> = ({ onUseInApp }) => {
  const [datasets] = useState<PublishedDataset[]>(SEEDED_DATASETS);
  const [relationships] = useState<DatasetRelationship[]>(SEEDED_RELATIONSHIPS);
  const [selectedDatasetId, setSelectedDatasetId] = useState<string>("courses");
  const [searchQuery, setSearchQuery] = useState("");
  const [filterDept, setFilterDept] = useState("all");
  const [viewMode, setViewMode] = useState<"catalog" | "lattice">("catalog");
  const [activeTab, setActiveTab] = useState<"records" | "schema" | "relationships">("records");

  const selectedDataset = datasets.find((d) => d.id === selectedDatasetId) || datasets[0];

  const filteredDatasets = datasets.filter((ds) => {
    const matchesSearch =
      ds.name.toLowerCase().includes(searchQuery.toLowerCase()) ||
      ds.description.toLowerCase().includes(searchQuery.toLowerCase()) ||
      ds.department.toLowerCase().includes(searchQuery.toLowerCase());
    const matchesDept = filterDept === "all" || ds.department.toLowerCase().includes(filterDept.toLowerCase());
    return matchesSearch && matchesDept;
  });

  const getDatasetRelationships = (datasetId: string) => {
    return relationships.filter(
      (r) => r.source_dataset_id === datasetId || r.target_dataset_id === datasetId
    );
  };

  const getRelationshipTargetName = (rel: DatasetRelationship, currentId: string) => {
    const isSource = rel.source_dataset_id === currentId;
    const otherId = isSource ? rel.target_dataset_id : rel.source_dataset_id;
    const other = datasets.find((d) => d.id === otherId);
    return other ? other.name : otherId;
  };

  return (
    <div className="flex-1 flex flex-col min-h-0 bg-slate-50 dark:bg-[#0b0f19] text-slate-800 dark:text-slate-100">
      {/* Top Banner and Navigation Bar */}
      <div className="bg-white dark:bg-[#111827] border-b border-slate-200 dark:border-slate-800 px-6 py-5">
        <div className="flex flex-col lg:flex-row lg:items-center lg:justify-between gap-4">
          <div>
            <div className="flex items-center gap-2">
              <span className="text-xl">🗄️</span>
              <h1 className="text-xl font-bold text-slate-900 dark:text-white tracking-tight">
                Published Institutional Datasets & Relational Lattice
              </h1>
              <span className="px-2.5 py-0.5 rounded-full text-xs font-semibold bg-blue-100 text-blue-800 dark:bg-blue-900/60 dark:text-blue-300">
                Data Layer
              </span>
            </div>
            <p className="text-xs text-slate-500 dark:text-slate-400 mt-1 max-w-2xl">
              Discover verified institutional datasets, inspect foreign key relationships, and link authoritative data into custom applications with automated Cedar policy safeguards.
            </p>
          </div>

          <div className="flex items-center gap-3">
            {/* View Mode Toggle */}
            <div className="inline-flex rounded-lg p-1 bg-slate-100 dark:bg-slate-800 border border-slate-200 dark:border-slate-700 text-xs font-semibold">
              <button
                onClick={() => setViewMode("catalog")}
                className={`px-3 py-1.5 rounded-md transition-colors ${
                  viewMode === "catalog"
                    ? "bg-white dark:bg-slate-700 text-blue-600 dark:text-blue-400 shadow-sm"
                    : "text-slate-600 dark:text-slate-300 hover:text-slate-900 dark:hover:text-white"
                }`}
              >
                Catalog & Table View
              </button>
              <button
                onClick={() => setViewMode("lattice")}
                className={`px-3 py-1.5 rounded-md transition-colors ${
                  viewMode === "lattice"
                    ? "bg-white dark:bg-slate-700 text-blue-600 dark:text-blue-400 shadow-sm"
                    : "text-slate-600 dark:text-slate-300 hover:text-slate-900 dark:hover:text-white"
                }`}
              >
                Relational Lattice Graph
              </button>
            </div>

            {onUseInApp && (
              <button
                onClick={() => onUseInApp(selectedDataset)}
                className="inline-flex items-center gap-2 px-3.5 py-1.5 rounded-lg text-xs font-semibold bg-blue-600 hover:bg-blue-500 text-white shadow-sm transition-colors"
              >
                <span>➕</span>
                <span>Use in New Application</span>
              </button>
            )}
          </div>
        </div>

        {/* Filters and Search Bar */}
        <div className="mt-4 flex flex-wrap items-center gap-3 pt-3 border-t border-slate-100 dark:border-slate-800/80">
          <div className="relative flex-1 min-w-[240px] max-w-md">
            <span className="absolute inset-y-0 left-0 pl-3 flex items-center pointer-events-none text-slate-400 text-xs">
              🔍
            </span>
            <input
              type="text"
              placeholder="Search datasets by name, keyword, or department..."
              value={searchQuery}
              onChange={(e) => setSearchQuery(e.target.value)}
              className="w-full pl-9 pr-3 py-1.5 text-xs bg-slate-50 dark:bg-slate-900 border border-slate-200 dark:border-slate-700 rounded-lg text-slate-900 dark:text-white placeholder-slate-400 focus:outline-none focus:ring-2 focus:ring-blue-500"
            />
          </div>

          <div className="flex items-center gap-2 text-xs">
            <span className="text-slate-500 dark:text-slate-400 font-medium">Department:</span>
            <select
              value={filterDept}
              onChange={(e) => setFilterDept(e.target.value)}
              className="px-2.5 py-1.5 bg-slate-50 dark:bg-slate-900 border border-slate-200 dark:border-slate-700 rounded-lg text-slate-800 dark:text-slate-200 text-xs focus:outline-none focus:ring-2 focus:ring-blue-500"
            >
              <option value="all">All Departments</option>
              <option value="Academic Affairs">Academic Affairs</option>
              <option value="Registrar">Office of the Registrar</option>
              <option value="Sponsored Research">Sponsored Research</option>
              <option value="Provost">Provost & Council</option>
            </select>
          </div>
        </div>
      </div>

      {/* Main Content Area */}
      {viewMode === "catalog" ? (
        <div className="flex-1 flex flex-col md:flex-row min-h-0 overflow-hidden">
          {/* Sidebar Dataset Directory */}
          <div className="w-full md:w-80 lg:w-96 border-r border-slate-200 dark:border-slate-800 bg-white dark:bg-[#111827] flex flex-col min-h-0 overflow-y-auto">
            <div className="p-3 bg-slate-50 dark:bg-slate-900/60 border-b border-slate-200 dark:border-slate-800 text-[11px] font-bold uppercase tracking-wider text-slate-500 dark:text-slate-400 flex items-center justify-between">
              <span>Published Catalog ({filteredDatasets.length})</span>
              <span className="text-blue-600 dark:text-blue-400 font-normal lowercase">authoritative</span>
            </div>

            <div className="divide-y divide-slate-100 dark:divide-slate-800/60">
              {filteredDatasets.map((ds) => {
                const isSelected = ds.id === selectedDataset.id;
                const relCount = getDatasetRelationships(ds.id).length;

                return (
                  <button
                    key={ds.id}
                    onClick={() => setSelectedDatasetId(ds.id)}
                    className={`w-full text-left p-4 transition-colors flex flex-col gap-1.5 ${
                      isSelected
                        ? "bg-blue-50/70 dark:bg-blue-950/30 border-l-4 border-blue-600 dark:border-blue-500"
                        : "hover:bg-slate-50 dark:hover:bg-slate-800/50"
                    }`}
                  >
                    <div className="flex items-start justify-between gap-2">
                      <h3 className={`text-xs font-bold leading-tight ${isSelected ? "text-blue-700 dark:text-blue-400" : "text-slate-900 dark:text-white"}`}>
                        {ds.name}
                      </h3>
                      <span
                        className={`text-[10px] px-1.5 py-0.5 rounded font-medium shrink-0 ${
                          ds.sensitivity_level.includes("FERPA")
                            ? "bg-rose-100 text-rose-800 dark:bg-rose-950/60 dark:text-rose-300"
                            : ds.sensitivity_level === "Public"
                            ? "bg-emerald-100 text-emerald-800 dark:bg-emerald-950/60 dark:text-emerald-300"
                            : "bg-amber-100 text-amber-800 dark:bg-amber-950/60 dark:text-amber-300"
                        }`}
                      >
                        {ds.sensitivity_level}
                      </span>
                    </div>

                    <p className="text-[11px] text-slate-500 dark:text-slate-400 line-clamp-2">
                      {ds.description}
                    </p>

                    <div className="flex items-center gap-3 text-[10px] text-slate-400 dark:text-slate-500 pt-1">
                      <span>🏛️ {ds.department}</span>
                      <span>📊 {ds.record_count.toLocaleString()} rows</span>
                      <span>🔗 {relCount} {relCount === 1 ? "relation" : "relations"}</span>
                    </div>
                  </button>
                );
              })}
            </div>
          </div>

          {/* Dataset Detail & Data Grid Pane */}
          <div className="flex-1 flex flex-col min-h-0 bg-slate-50 dark:bg-[#0b0f19] overflow-y-auto">
            {/* Detail Header */}
            <div className="p-6 bg-white dark:bg-[#111827] border-b border-slate-200 dark:border-slate-800">
              <div className="flex flex-col md:flex-row md:items-center justify-between gap-4">
                <div>
                  <div className="flex items-center gap-3">
                    <h2 className="text-lg font-bold text-slate-900 dark:text-white">
                      {selectedDataset.name}
                    </h2>
                    <span className="font-mono text-xs px-2 py-0.5 rounded bg-slate-100 dark:bg-slate-800 text-slate-600 dark:text-slate-300 border border-slate-200 dark:border-slate-700">
                      dataset:{selectedDataset.id}
                    </span>
                  </div>
                  <p className="text-xs text-slate-600 dark:text-slate-300 mt-1 max-w-3xl">
                    {selectedDataset.description}
                  </p>
                </div>

                <div className="flex items-center gap-2">
                  {onUseInApp && (
                    <button
                      onClick={() => onUseInApp(selectedDataset)}
                      className="px-3 py-1.5 rounded-lg text-xs font-semibold bg-blue-600 hover:bg-blue-500 text-white shadow-sm transition-colors flex items-center gap-1.5"
                    >
                      <span>🔗</span>
                      <span>Link in Application</span>
                    </button>
                  )}
                </div>
              </div>

              {/* Badges and Governance Metadata */}
              <div className="mt-4 flex flex-wrap items-center gap-3 text-xs">
                <div className="px-2.5 py-1 rounded bg-slate-100 dark:bg-slate-800 text-slate-700 dark:text-slate-300 flex items-center gap-1.5 border border-slate-200 dark:border-slate-700">
                  <span className="text-slate-400">Authority:</span>
                  <span className="font-medium">{selectedDataset.department}</span>
                </div>

                <div className="px-2.5 py-1 rounded bg-slate-100 dark:bg-slate-800 text-slate-700 dark:text-slate-300 flex items-center gap-1.5 border border-slate-200 dark:border-slate-700">
                  <span className="text-slate-400">Total Live Rows:</span>
                  <span className="font-bold text-slate-900 dark:text-white font-mono">
                    {selectedDataset.record_count.toLocaleString()}
                  </span>
                </div>

                {selectedDataset.herm_capability_id && (
                  <div className="px-2.5 py-1 rounded bg-purple-50 dark:bg-purple-950/50 text-purple-700 dark:text-purple-300 border border-purple-200 dark:border-purple-800 flex items-center gap-1 font-mono text-[11px]">
                    <span>HERM:</span>
                    <span>{selectedDataset.herm_capability_id}</span>
                  </div>
                )}

                <div className="px-2.5 py-1 rounded bg-slate-100 dark:bg-slate-800 text-slate-700 dark:text-slate-300 flex items-center gap-1.5 border border-slate-200 dark:border-slate-700 text-[11px]">
                  <span className="text-slate-400">Security Gate:</span>
                  <span className="font-medium text-emerald-600 dark:text-emerald-400">
                    Cedar ABAC Enforced
                  </span>
                </div>
              </div>

              {/* Sub-tabs */}
              <div className="flex gap-2 mt-5 border-b border-slate-200 dark:border-slate-800 -mb-6">
                <button
                  onClick={() => setActiveTab("records")}
                  className={`pb-2.5 px-3 text-xs font-semibold border-b-2 transition-colors ${
                    activeTab === "records"
                      ? "border-blue-600 text-blue-600 dark:text-blue-400 dark:border-blue-400"
                      : "border-transparent text-slate-500 hover:text-slate-700 dark:text-slate-400 dark:hover:text-slate-200"
                  }`}
                >
                  Tabular Records Preview ({selectedDataset.sample_data?.length || 0})
                </button>
                <button
                  onClick={() => setActiveTab("schema")}
                  className={`pb-2.5 px-3 text-xs font-semibold border-b-2 transition-colors ${
                    activeTab === "schema"
                      ? "border-blue-600 text-blue-600 dark:text-blue-400 dark:border-blue-400"
                      : "border-transparent text-slate-500 hover:text-slate-700 dark:text-slate-400 dark:hover:text-slate-200"
                  }`}
                >
                  Schema & Standard CEDS Elements ({selectedDataset.fields.length})
                </button>
                <button
                  onClick={() => setActiveTab("relationships")}
                  className={`pb-2.5 px-3 text-xs font-semibold border-b-2 transition-colors ${
                    activeTab === "relationships"
                      ? "border-blue-600 text-blue-600 dark:text-blue-400 dark:border-blue-400"
                      : "border-transparent text-slate-500 hover:text-slate-700 dark:text-slate-400 dark:hover:text-slate-200"
                  }`}
                >
                  Cross-Dataset Relationships ({getDatasetRelationships(selectedDataset.id).length})
                </button>
              </div>
            </div>

            {/* Sub-tab Content Panels */}
            <div className="p-6">
              {activeTab === "records" && (
                <div className="bg-white dark:bg-[#111827] border border-slate-200 dark:border-slate-800 rounded-xl overflow-hidden shadow-sm">
                  <div className="px-4 py-3 bg-slate-50 dark:bg-slate-900 border-b border-slate-200 dark:border-slate-800 flex items-center justify-between">
                    <span className="text-xs font-semibold text-slate-700 dark:text-slate-200">
                      Sample Relational Records
                    </span>
                    <span className="text-[11px] text-slate-400">
                      Synchronized from operational store
                    </span>
                  </div>

                  <div className="overflow-x-auto">
                    <table className="w-full text-left border-collapse text-xs">
                      <thead>
                        <tr className="bg-slate-100/70 dark:bg-slate-800/50 border-b border-slate-200 dark:border-slate-700/60 font-semibold text-slate-700 dark:text-slate-300">
                          {selectedDataset.fields.map((f) => (
                            <th key={f.name} className="px-4 py-2.5 whitespace-nowrap">
                              <div className="flex items-center gap-1.5">
                                <span>{f.label}</span>
                                {f.field_type === "Relation" ? (
                                  <span className="text-[10px] px-1 py-0.2 rounded bg-purple-100 text-purple-800 dark:bg-purple-950 dark:text-purple-300 font-mono">
                                    🔗 rel
                                  </span>
                                ) : (
                                  <span className="text-[10px] font-mono text-slate-400 font-normal">
                                    ({f.field_type.toLowerCase()})
                                  </span>
                                )}
                              </div>
                            </th>
                          ))}
                        </tr>
                      </thead>
                      <tbody className="divide-y divide-slate-100 dark:divide-slate-800 font-mono text-[11px]">
                        {(selectedDataset.sample_data || []).map((row, idx) => (
                          <tr key={idx} className="hover:bg-slate-50 dark:hover:bg-slate-800/40">
                            {selectedDataset.fields.map((f) => (
                              <td key={f.name} className="px-4 py-2.5 whitespace-nowrap text-slate-800 dark:text-slate-200">
                                {typeof row[f.name] === "boolean" ? (
                                  row[f.name] ? (
                                    <span className="text-emerald-600 dark:text-emerald-400 font-bold">YES</span>
                                  ) : (
                                    <span className="text-slate-400">NO</span>
                                  )
                                ) : f.field_type === "Relation" ? (
                                  <span className="text-blue-600 dark:text-blue-400 underline font-medium">
                                    {String(row[f.name])}
                                  </span>
                                ) : (
                                  String(row[f.name] || "-")
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

              {activeTab === "schema" && (
                <div className="bg-white dark:bg-[#111827] border border-slate-200 dark:border-slate-800 rounded-xl overflow-hidden shadow-sm">
                  <div className="px-4 py-3 bg-slate-50 dark:bg-slate-900 border-b border-slate-200 dark:border-slate-800">
                    <h4 className="text-xs font-bold text-slate-800 dark:text-slate-200">
                      Formal Schema Fields & Standard Governance Elements
                    </h4>
                  </div>
                  <div className="divide-y divide-slate-100 dark:divide-slate-800">
                    {selectedDataset.fields.map((f) => (
                      <div key={f.name} className="p-4 flex flex-col md:flex-row md:items-center justify-between gap-3 text-xs">
                        <div>
                          <div className="flex items-center gap-2">
                            <span className="font-bold text-slate-900 dark:text-white">{f.label}</span>
                            <span className="font-mono text-[11px] text-slate-400">({f.name})</span>
                            <span className="px-1.5 py-0.5 rounded text-[10px] bg-slate-100 dark:bg-slate-800 text-slate-600 dark:text-slate-300 font-mono">
                              {f.field_type}
                            </span>
                            {f.required && (
                              <span className="px-1.5 py-0.5 rounded text-[10px] bg-amber-100 text-amber-800 dark:bg-amber-950 dark:text-amber-300">
                                Required
                              </span>
                            )}
                            {f.ferpa_sensitive && (
                              <span className="px-1.5 py-0.5 rounded text-[10px] bg-rose-100 text-rose-800 dark:bg-rose-950 dark:text-rose-300 font-semibold">
                                🔒 FERPA Sensitive
                              </span>
                            )}
                          </div>
                        </div>

                        <div className="flex items-center gap-2">
                          {f.ceds_code ? (
                            <span className="px-2 py-1 rounded bg-blue-50 dark:bg-blue-950/60 text-blue-700 dark:text-blue-300 font-mono text-[11px] border border-blue-200 dark:border-blue-800">
                              CEDS v11: {f.ceds_code}
                            </span>
                          ) : (
                            <span className="text-slate-400 text-[11px]">Unmapped custom attribute</span>
                          )}
                        </div>
                      </div>
                    ))}
                  </div>
                </div>
              )}

              {activeTab === "relationships" && (
                <div className="space-y-4">
                  <div className="bg-white dark:bg-[#111827] border border-slate-200 dark:border-slate-800 rounded-xl overflow-hidden shadow-sm">
                    <div className="px-4 py-3 bg-slate-50 dark:bg-slate-900 border-b border-slate-200 dark:border-slate-800 flex items-center justify-between">
                      <h4 className="text-xs font-bold text-slate-800 dark:text-slate-200">
                        Linked Relational Connections
                      </h4>
                      <span className="text-[11px] text-slate-400">
                        Foreign key lookups and cardinality
                      </span>
                    </div>

                    <div className="divide-y divide-slate-100 dark:divide-slate-800">
                      {getDatasetRelationships(selectedDataset.id).map((rel) => {
                        const isSource = rel.source_dataset_id === selectedDataset.id;

                        return (
                          <div key={rel.id} className="p-4 flex flex-col md:flex-row md:items-center justify-between gap-3 text-xs">
                            <div className="flex items-start gap-3">
                              <span className="text-lg">🔗</span>
                              <div>
                                <div className="flex items-center gap-2">
                                  <span className="font-bold text-slate-900 dark:text-white">{rel.name}</span>
                                  <span className="px-1.5 py-0.5 rounded text-[10px] bg-purple-100 text-purple-800 dark:bg-purple-950 dark:text-purple-300 font-mono">
                                    {rel.relationship_type === "OneToMany" ? "1 : N" : "1 : 1"}
                                  </span>
                                </div>
                                <p className="text-[11px] text-slate-500 dark:text-slate-400 mt-1 font-mono">
                                  {isSource ? (
                                    <>
                                      <span className="text-blue-600 dark:text-blue-400 font-bold">{rel.source_field}</span>
                                      {" ➔ "}
                                      <span>{getRelationshipTargetName(rel, selectedDataset.id)}</span>.
                                      <span className="text-emerald-600 dark:text-emerald-400 font-bold">{rel.target_field}</span>
                                    </>
                                  ) : (
                                    <>
                                      <span>{getRelationshipTargetName(rel, selectedDataset.id)}</span>.
                                      <span className="text-blue-600 dark:text-blue-400 font-bold">{rel.source_field}</span>
                                      {" ➔ "}
                                      <span className="text-emerald-600 dark:text-emerald-400 font-bold">{rel.target_field}</span>
                                    </>
                                  )}
                                </p>
                              </div>
                            </div>

                            <div className="text-right">
                              <span className="text-[11px] text-slate-400">Resolved Label: </span>
                              <span className="font-mono text-xs font-semibold text-slate-700 dark:text-slate-300">
                                {rel.display_field}
                              </span>
                            </div>
                          </div>
                        );
                      })}
                    </div>
                  </div>
                </div>
              )}
            </div>
          </div>
        </div>
      ) : (
        /* Relational Lattice Graph Topology View */
        <div className="flex-1 p-6 overflow-y-auto">
          <div className="max-w-6xl mx-auto space-y-6">
            <div className="bg-white dark:bg-[#111827] border border-slate-200 dark:border-slate-800 rounded-xl p-5 shadow-sm">
              <h3 className="text-sm font-bold text-slate-900 dark:text-white flex items-center gap-2">
                <span>🌐</span>
                <span>Institutional Relational Lattice Map</span>
              </h3>
              <p className="text-xs text-slate-500 dark:text-slate-400 mt-1">
                Topological view of published datasets and cross-departmental references. Applications can pull records across any connected node.
              </p>
            </div>

            <div className="grid grid-cols-1 md:grid-cols-2 lg:grid-cols-4 gap-4">
              {datasets.map((ds) => {
                const connectedRels = getDatasetRelationships(ds.id);
                const isSelected = ds.id === selectedDataset.id;

                return (
                  <div
                    key={ds.id}
                    onClick={() => {
                      setSelectedDatasetId(ds.id);
                      setViewMode("catalog");
                    }}
                    className={`cursor-pointer bg-white dark:bg-[#111827] border rounded-xl p-5 shadow-sm hover:border-blue-500 transition-all flex flex-col justify-between gap-3 ${
                      isSelected
                        ? "border-blue-600 dark:border-blue-500 ring-2 ring-blue-500/20"
                        : "border-slate-200 dark:border-slate-800"
                    }`}
                  >
                    <div>
                      <div className="flex items-start justify-between gap-2 mb-2">
                        <span className="text-2xl">
                          {ds.id === "courses" ? "📚" : ds.id === "faculty" ? "👩‍🏫" : ds.id === "grants" ? "🔬" : "🎓"}
                        </span>
                        <span
                          className={`text-[10px] px-1.5 py-0.5 rounded font-semibold ${
                            ds.sensitivity_level.includes("FERPA")
                              ? "bg-rose-100 text-rose-800 dark:bg-rose-950/60 dark:text-rose-300"
                              : "bg-emerald-100 text-emerald-800 dark:bg-emerald-950/60 dark:text-emerald-300"
                          }`}
                        >
                          {ds.sensitivity_level}
                        </span>
                      </div>

                      <h4 className="text-xs font-bold text-slate-900 dark:text-white leading-snug">
                        {ds.name}
                      </h4>
                      <p className="text-[11px] text-slate-500 dark:text-slate-400 mt-1 line-clamp-2">
                        {ds.description}
                      </p>
                    </div>

                    <div className="pt-3 border-t border-slate-100 dark:border-slate-800/80">
                      <div className="text-[10px] font-bold uppercase text-slate-400 tracking-wider mb-1.5">
                        Connected Links ({connectedRels.length})
                      </div>
                      <div className="space-y-1">
                        {connectedRels.map((r) => (
                          <div
                            key={r.id}
                            className="text-[11px] font-mono flex items-center justify-between text-slate-600 dark:text-slate-300 bg-slate-50 dark:bg-slate-800/60 px-2 py-1 rounded"
                          >
                            <span className="truncate">{getRelationshipTargetName(r, ds.id)}</span>
                            <span className="text-[9px] text-purple-600 dark:text-purple-400 font-bold ml-1">
                              {r.relationship_type === "OneToMany" ? "1:N" : "1:1"}
                            </span>
                          </div>
                        ))}
                      </div>
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
