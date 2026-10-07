import React from "react";
import { AppTable, CompoundFilter, FilterClause, RowDensity, SortRule } from "./types";

export interface DataShapingToolbarProps {
  activeTable: AppTable;
  compoundFilter: CompoundFilter;
  setCompoundFilter: React.Dispatch<React.SetStateAction<CompoundFilter>>;
  isFilterPopoverOpen: boolean;
  setIsFilterPopoverOpen: React.Dispatch<React.SetStateAction<boolean>>;
  handleUpdateFilterClause: (id: string, updates: Partial<FilterClause>) => void;
  handleRemoveFilterClause: (id: string) => void;
  handleAddFilterClause: () => void;
  sortRules: SortRule[];
  setSortRules: React.Dispatch<React.SetStateAction<SortRule[]>>;
  isSortPopoverOpen: boolean;
  setIsSortPopoverOpen: React.Dispatch<React.SetStateAction<boolean>>;
  handleUpdateSortRule: (id: string, updates: Partial<SortRule>) => void;
  handleRemoveSortRule: (id: string) => void;
  handleAddSortRule: () => void;
  groupByField: string | null;
  setGroupByField: (field: string | null) => void;
  rowDensity: RowDensity;
  setRowDensity: (d: RowDensity) => void;
  tableSearchFilter: string;
  setTableSearchFilter: (s: string) => void;
  onOpenCsvModal: () => void;
  onExportCsv: (records: Record<string, any>[]) => void;
  onAddRow: () => void;
}

export const DataShapingToolbar: React.FC<DataShapingToolbarProps> = ({
  activeTable,
  compoundFilter,
  setCompoundFilter,
  isFilterPopoverOpen,
  setIsFilterPopoverOpen,
  handleUpdateFilterClause,
  handleRemoveFilterClause,
  handleAddFilterClause,
  sortRules,
  setSortRules,
  isSortPopoverOpen,
  setIsSortPopoverOpen,
  handleUpdateSortRule,
  handleRemoveSortRule,
  handleAddSortRule,
  groupByField,
  setGroupByField,
  rowDensity,
  setRowDensity,
  tableSearchFilter,
  setTableSearchFilter,
  onOpenCsvModal,
  onExportCsv,
  onAddRow,
}) => {
  return (
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
          onClick={onOpenCsvModal}
          className="px-2.5 py-1 rounded-lg text-xs font-semibold bg-white dark:bg-slate-800 border border-slate-200 dark:border-slate-700 text-slate-700 dark:text-slate-300 hover:bg-slate-100 dark:hover:bg-slate-700 cursor-pointer flex items-center gap-1 transition-colors"
          title="Import records from CSV"
        >
          <span>📥</span>
          <span>Import CSV</span>
        </button>

        <button
          type="button"
          data-testid="toolbar-export-csv-btn"
          onClick={() => onExportCsv(activeTable.records || [])}
          className="px-2.5 py-1 rounded-lg text-xs font-semibold bg-white dark:bg-slate-800 border border-slate-200 dark:border-slate-700 text-slate-700 dark:text-slate-300 hover:bg-slate-100 dark:hover:bg-slate-700 cursor-pointer flex items-center gap-1 transition-colors"
          title="Export table records to CSV"
        >
          <span>📤</span>
          <span>Export CSV</span>
        </button>

        <button
          type="button"
          data-testid="toolbar-add-record-btn"
          onClick={onAddRow}
          className="px-2.5 py-1 rounded-lg text-xs font-semibold bg-blue-600 hover:bg-blue-700 text-white cursor-pointer flex items-center gap-1 shadow-xs transition-colors"
        >
          <span className="font-bold">+</span>
          <span>Add Record</span>
        </button>
      </div>
    </div>
  );
};
