import React, { useEffect, useRef, useState } from "react";
import { apiClient } from "./api";
import { Banner, EmptyState, StatusBadge } from "./ui";
import { CatalogEntry, PublishedDataset } from "./types";

/** A catalog entry as the "use in an application" step reads it. A restricted field has no name, so it is left out. */
function asPublished(d: CatalogEntry): PublishedDataset {
  return {
    id: d.id,
    name: d.name,
    description: d.description,
    department: d.department,
    organization: d.organization,
    sensitivity_level: d.sensitivity_level,
    herm_capability_id: d.herm_capability_id ?? undefined,
    record_count: d.record_count ?? 0,
    published_at: d.published_at ?? "",
    fields: d.fields
      .filter((f) => !f.restricted && f.name)
      .map((f) => ({ name: f.name ?? "", label: f.label, field_type: f.field_type ?? "Text", required: !!f.required, ferpa_sensitive: !!f.ferpa_sensitive, ceds_code: f.ceds_code ?? undefined })),
  };
}

/** How many rows, in words. A small group has no number. */
function rowsText(d: CatalogEntry): string {
  if (d.record_count !== null) return `${d.record_count.toLocaleString()} ${d.record_count === 1 ? "row" : "rows"}`;
  return d.size?.label ?? "size not shown";
}

interface DatasetExplorerProps {
  onUseInApp?: (dataset: PublishedDataset) => void;
}

export const DatasetExplorer: React.FC<DatasetExplorerProps> = ({ onUseInApp }) => {
  const [datasets, setDatasets] = useState<CatalogEntry[]>([]);
  const [total, setTotal] = useState(0);
  const [truncated, setTruncated] = useState(false);
  const [load, setLoad] = useState<{ state: "loading" | "ready" | "error"; message?: string }>({ state: "loading" });
  const [searching, setSearching] = useState(false);
  const [reloads, setReloads] = useState(0);
  const [selectedDatasetId, setSelectedDatasetId] = useState<string>("");
  // What is typed, and the words the server was last asked about. The server is asked a moment after typing stops.
  const [searchQuery, setSearchQuery] = useState("");
  const [asked, setAsked] = useState("");
  const [filterDept, setFilterDept] = useState("all");
  const [activeTab, setActiveTab] = useState<"records" | "schema" | "sample">("records");
  const loaded = useRef(false);

  useEffect(() => {
    const timer = setTimeout(() => setAsked(searchQuery.trim()), 300);
    return () => clearTimeout(timer);
  }, [searchQuery]);

  useEffect(() => {
    let live = true;
    if (loaded.current) setSearching(true);
    else setLoad({ state: "loading" });
    apiClient
      .searchCatalog(asked)
      .then((result) => {
        if (!live) return;
        loaded.current = true;
        setDatasets(result.datasets);
        setTotal(result.total ?? result.datasets.length);
        setTruncated(!!result.truncated);
        setSearching(false);
        setLoad({ state: "ready" });
      })
      .catch((e) => {
        if (!live) return;
        setSearching(false);
        setLoad({ state: "error", message: e instanceof Error ? e.message : "The datasets did not load." });
      });
    return () => {
      live = false;
    };
  }, [asked, reloads]);

  const selectedDataset: CatalogEntry | undefined = datasets.find((d) => d.id === selectedDatasetId) || datasets[0];

  // The server has matched the words. The department list narrows what it returned.
  const filteredDatasets = datasets.filter((ds) => filterDept === "all" || ds.department.toLowerCase().includes(filterDept.toLowerCase()));

  if (load.state === "loading") {
    return <div className="flex-1 p-6 text-sm text-slate-500 dark:text-slate-400">Loading datasets…</div>;
  }
  if (load.state === "error") {
    return (
      <div className="flex-1 p-6 text-sm" role="alert">
        <p className="text-rose-700 dark:text-rose-300">The datasets did not load. {load.message}</p>
        <button
          onClick={() => setReloads((n) => n + 1)}
          className="mt-3 px-3 py-1.5 rounded-lg text-xs font-semibold bg-blue-600 hover:bg-blue-500 text-white"
        >
          Retry
        </button>
      </div>
    );
  }
  if (!selectedDataset && asked === "") {
    return <div className="flex-1 p-6 text-sm text-slate-500 dark:text-slate-400">No datasets are published yet.</div>;
  }

  const chip = "px-2.5 py-1 rounded bg-slate-100 dark:bg-slate-800 text-slate-700 dark:text-slate-300 flex items-center gap-1.5 border border-slate-200 dark:border-slate-700";
  const tabClass = (on: boolean) =>
    `pb-2.5 px-3 text-xs font-semibold border-b-2 transition-colors ${
      on
        ? "border-blue-600 text-blue-600 dark:text-blue-400 dark:border-blue-400"
        : "border-transparent text-slate-500 hover:text-slate-700 dark:text-slate-400 dark:hover:text-slate-200"
    }`;
  const canUse = !!onUseInApp && !!selectedDataset && selectedDataset.kind !== "table";

  return (
    <div className="flex-1 flex flex-col min-h-0 bg-slate-50 dark:bg-[#0b0f19] text-slate-800 dark:text-slate-100">
      {/* Top Banner and Navigation Bar */}
      <div className="bg-white dark:bg-[#111827] border-b border-slate-200 dark:border-slate-800 px-6 py-5">
        <div className="flex flex-col lg:flex-row lg:items-center lg:justify-between gap-4">
          <div>
            <div className="flex items-center gap-2">
              <span className="text-xl">🗄️</span>
              <h1 className="text-xl font-bold text-slate-900 dark:text-white tracking-tight">Institutional Data Catalog</h1>
              <span className="px-2.5 py-0.5 rounded-full text-xs font-semibold bg-blue-100 text-blue-800 dark:bg-blue-900/60 dark:text-blue-300">
                Data Layer
              </span>
            </div>
            <p className="text-xs text-slate-500 dark:text-slate-400 mt-1 max-w-2xl">
              Find what data exists, see its fields, owner and a made-up sample, and never a real value. You see the datasets you may discover.
            </p>
          </div>

          <div className="flex items-center gap-3">
            {canUse && selectedDataset && (
              <button
                onClick={() => onUseInApp?.(asPublished(selectedDataset))}
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
            <span className="absolute inset-y-0 left-0 pl-3 flex items-center pointer-events-none text-slate-400 text-xs">🔍</span>
            <input
              type="search"
              aria-label="Search the catalog"
              placeholder="Search by words, function, unit, category or field name..."
              value={searchQuery}
              onChange={(e) => setSearchQuery(e.target.value)}
              className="w-full pl-9 pr-3 py-1.5 text-xs bg-slate-50 dark:bg-slate-900 border border-slate-200 dark:border-slate-700 rounded-lg text-slate-900 dark:text-white placeholder-slate-400 focus:outline-none focus:ring-2 focus:ring-blue-500"
            />
          </div>

          <div className="flex items-center gap-2 text-xs">
            <span className="text-slate-500 dark:text-slate-400 font-medium">Department:</span>
            <select
              aria-label="Department"
              value={filterDept}
              onChange={(e) => setFilterDept(e.target.value)}
              className="px-2.5 py-1.5 bg-slate-50 dark:bg-slate-900 border border-slate-200 dark:border-slate-700 rounded-lg text-slate-800 dark:text-slate-200 text-xs focus:outline-none focus:ring-2 focus:ring-blue-500"
            >
              <option value="all">All Departments</option>
              {[...new Set(datasets.map((d) => d.department))].sort().map((dept) => (
                <option key={dept} value={dept}>
                  {dept}
                </option>
              ))}
            </select>
          </div>
          <span role="status" aria-live="polite" className="text-[11px] text-slate-500 dark:text-slate-400">
            {searching ? "Searching…" : asked ? `${total} ${total === 1 ? "dataset matches" : "datasets match"}` : ""}
          </span>
        </div>
      </div>

      {/* Main Content Area */}
      <div className="flex-1 flex flex-col md:flex-row min-h-0 overflow-hidden">
        {/* Sidebar Dataset Directory */}
        <div className="w-full md:w-80 lg:w-96 border-r border-slate-200 dark:border-slate-800 bg-white dark:bg-[#111827] flex flex-col min-h-0 overflow-y-auto">
          <div className="p-3 bg-slate-50 dark:bg-slate-900/60 border-b border-slate-200 dark:border-slate-800 text-[11px] font-bold uppercase tracking-wider text-slate-500 dark:text-slate-400 flex items-center justify-between">
            <span>Catalog ({filteredDatasets.length})</span>
            <span className="text-blue-600 dark:text-blue-400 font-normal lowercase">{truncated ? `first ${datasets.length} of ${total}` : "authoritative"}</span>
          </div>

          {filteredDatasets.length === 0 && (
            <div className="p-4">
              <EmptyState title="No dataset matches" description="Try other words, or a function, a unit, a category or a field name. You see only the datasets you may discover." />
            </div>
          )}

          <div className="divide-y divide-slate-100 dark:divide-slate-800/60">
            {filteredDatasets.map((ds) => {
              const isSelected = !!selectedDataset && ds.id === selectedDataset.id;

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

                  <p className="text-[11px] text-slate-500 dark:text-slate-400 line-clamp-2">{ds.description}</p>

                  <div className="flex items-center gap-3 text-[10px] text-slate-400 dark:text-slate-500 pt-1">
                    <span>🏛️ {ds.department}</span>
                    <span>📊 {rowsText(ds)}</span>
                  </div>
                </button>
              );
            })}
          </div>
        </div>

        {/* Dataset Detail Pane */}
        <div className="flex-1 flex flex-col min-h-0 bg-slate-50 dark:bg-[#0b0f19] overflow-y-auto">
          {!selectedDataset ? (
            <div className="p-6">
              <EmptyState title="Nothing to show" description="No dataset matches these words." />
            </div>
          ) : (
            <>
              <div className="p-6 bg-white dark:bg-[#111827] border-b border-slate-200 dark:border-slate-800">
                <div className="flex flex-col md:flex-row md:items-center justify-between gap-4">
                  <div>
                    <div className="flex items-center gap-3">
                      <h2 className="text-lg font-bold text-slate-900 dark:text-white">{selectedDataset.name}</h2>
                      <span className="font-mono text-xs px-2 py-0.5 rounded bg-slate-100 dark:bg-slate-800 text-slate-600 dark:text-slate-300 border border-slate-200 dark:border-slate-700">
                        dataset:{selectedDataset.dataset_id ?? selectedDataset.id}
                      </span>
                    </div>
                    <p className="text-xs text-slate-600 dark:text-slate-300 mt-1 max-w-3xl">{selectedDataset.description}</p>
                  </div>

                  <div className="flex items-center gap-2">
                    {canUse && (
                      <button
                        onClick={() => onUseInApp?.(asPublished(selectedDataset))}
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
                  <div className={chip}>
                    <span className="text-slate-400">Authority:</span>
                    <span className="font-medium">{selectedDataset.department}</span>
                  </div>

                  {selectedDataset.unit && (
                    <div className={chip} data-testid="catalog-unit">
                      <span className="text-slate-400">Unit:</span>
                      <span className="font-medium">{selectedDataset.unit.name}</span>
                    </div>
                  )}

                  {selectedDataset.steward !== undefined && (
                    <div className={chip} data-testid="catalog-steward">
                      <span className="text-slate-400">Steward:</span>
                      <span className="font-medium">
                        {selectedDataset.steward ? selectedDataset.steward.position ?? selectedDataset.steward.position_key : "No steward position set"}
                      </span>
                    </div>
                  )}

                  <div className={chip}>
                    <span className="text-slate-400">Size:</span>
                    <span className="font-bold text-slate-900 dark:text-white font-mono">{selectedDataset.size?.label ?? rowsText(selectedDataset)}</span>
                  </div>

                  {selectedDataset.freshness && (
                    <div className={chip}>
                      <span className="text-slate-400">Latest data:</span>
                      <span className="font-medium">{selectedDataset.freshness}</span>
                    </div>
                  )}

                  {selectedDataset.used_by_apps !== undefined && (
                    <div className={chip}>
                      <span className="text-slate-400">Used by:</span>
                      <span className="font-medium">
                        {selectedDataset.used_by_apps} {selectedDataset.used_by_apps === 1 ? "app" : "apps"}
                      </span>
                    </div>
                  )}

                  {selectedDataset.herm_capability_id && (
                    <div className="px-2.5 py-1 rounded bg-purple-50 dark:bg-purple-950/50 text-purple-700 dark:text-purple-300 border border-purple-200 dark:border-purple-800 flex items-center gap-1 font-mono text-[11px]">
                      <span>HERM:</span>
                      <span>{selectedDataset.herm_capability_id}</span>
                    </div>
                  )}

                  <div className={`${chip} text-[11px]`}>
                    <span className="text-slate-400">Security Gate:</span>
                    <span className="font-medium text-emerald-600 dark:text-emerald-400">Cedar ABAC Enforced</span>
                  </div>
                </div>

                {selectedDataset.categories && selectedDataset.categories.length > 0 && (
                  <div className="mt-3 flex flex-wrap items-center gap-2" data-testid="catalog-categories">
                    <span className="text-[11px] text-slate-400">Categories:</span>
                    {selectedDataset.categories.map((c) => (
                      <StatusBadge key={c.id} tone="info" text={c.name} />
                    ))}
                  </div>
                )}

                {/* Sub-tabs */}
                <div className="flex gap-2 mt-5 border-b border-slate-200 dark:border-slate-800 -mb-6">
                  <button onClick={() => setActiveTab("records")} className={tabClass(activeTab === "records")}>
                    Records ({selectedDataset.record_count ?? selectedDataset.size?.label ?? "size not shown"})
                  </button>
                  <button onClick={() => setActiveTab("schema")} className={tabClass(activeTab === "schema")}>
                    Schema & Standard CEDS Elements ({selectedDataset.fields.length})
                  </button>
                  {selectedDataset.sample && (
                    <button onClick={() => setActiveTab("sample")} className={tabClass(activeTab === "sample")}>
                      Sample
                    </button>
                  )}
                </div>
              </div>

              {/* Sub-tab Content Panels */}
              <div className="p-6">
                {activeTab === "records" && (
                  <div
                    data-testid="dataset-records-note"
                    className="bg-white dark:bg-[#111827] border border-slate-200 dark:border-slate-800 rounded-xl shadow-sm p-5 space-y-2"
                  >
                    <h4 className="text-xs font-bold text-slate-800 dark:text-slate-200">{rowsText(selectedDataset)}, read-only</h4>
                    <p className="text-xs text-slate-600 dark:text-slate-400">
                      The catalog shows what a dataset holds, never its rows. A dataset is a copy of another system&apos;s data. Its
                      rows are read through its own app, under that app&apos;s access rules, and only its sync changes them.
                    </p>
                  </div>
                )}

                {activeTab === "schema" && (
                  <div className="bg-white dark:bg-[#111827] border border-slate-200 dark:border-slate-800 rounded-xl overflow-hidden shadow-sm">
                    <div className="px-4 py-3 bg-slate-50 dark:bg-slate-900 border-b border-slate-200 dark:border-slate-800">
                      <h4 className="text-xs font-bold text-slate-800 dark:text-slate-200">Formal Schema Fields & Standard Governance Elements</h4>
                    </div>
                    <div className="divide-y divide-slate-100 dark:divide-slate-800">
                      {selectedDataset.fields.map((f, i) =>
                        f.restricted ? (
                          <div key={`restricted-${i}`} className="p-4 text-xs text-slate-500 dark:text-slate-400 italic">
                            🔒 {f.label}
                          </div>
                        ) : (
                          <div key={f.name ?? i} className="p-4 flex flex-col md:flex-row md:items-center justify-between gap-3 text-xs">
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
                        ),
                      )}
                    </div>
                  </div>
                )}

                {activeTab === "sample" && selectedDataset.sample && (
                  <div className="space-y-3" data-testid="catalog-sample">
                    <Banner kind="info">{selectedDataset.sample.note}</Banner>
                    <div className="bg-white dark:bg-[#111827] border border-slate-200 dark:border-slate-800 rounded-xl overflow-x-auto shadow-sm">
                      <table className="min-w-full text-xs">
                        <thead>
                          <tr className="bg-slate-50 dark:bg-slate-900 text-left text-slate-500 dark:text-slate-400">
                            {selectedDataset.fields
                              .filter((f) => !f.restricted)
                              .map((f) => (
                                <th key={f.name} className="px-3 py-2 font-semibold">
                                  {f.label}
                                </th>
                              ))}
                          </tr>
                        </thead>
                        <tbody>
                          {selectedDataset.sample.rows.map((row, i) => (
                            <tr key={i} className="border-t border-slate-100 dark:border-slate-800">
                              {selectedDataset.fields
                                .filter((f) => !f.restricted)
                                .map((f) => (
                                  <td key={f.name} className="px-3 py-2 font-mono text-slate-700 dark:text-slate-300">
                                    {String(row[f.name ?? ""] ?? "")}
                                  </td>
                                ))}
                            </tr>
                          ))}
                        </tbody>
                      </table>
                    </div>
                  </div>
                )}
              </div>
            </>
          )}
        </div>
      </div>
    </div>
  );
};
