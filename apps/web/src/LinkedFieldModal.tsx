import React, { useState, useEffect } from "react";
import { AppTable, FieldSpec, LinkCardinality } from "./types";

export interface LinkedFieldModalProps {
  isOpen: boolean;
  onClose: () => void;
  tables: AppTable[];
  activeTableId: string;
  initialField?: FieldSpec | null;
  onSave: (field: FieldSpec) => void;
}

export const LinkedFieldModal: React.FC<LinkedFieldModalProps> = ({
  isOpen,
  onClose,
  tables,
  activeTableId,
  initialField,
  onSave,
}) => {
  const availableTables = tables.filter((t) => t.id !== activeTableId);
  const defaultTarget = availableTables[0] || tables[0];

  const [label, setLabel] = useState<string>(
    initialField?.label || (defaultTarget ? `${defaultTarget.name} Link` : "Linked Record")
  );
  const [targetTableId, setTargetTableId] = useState<string>(
    initialField?.target_table_id || defaultTarget?.id || ""
  );
  const [cardinality, setCardinality] = useState<LinkCardinality>(
    initialField?.cardinality || (initialField?.allow_multiple ? "multiple" : "single")
  );
  const selectedTargetTable = tables.find((t) => t.id === targetTableId) || defaultTarget;

  const [targetDisplayField, setTargetDisplayField] = useState<string>(
    initialField?.target_display_field ||
      selectedTargetTable?.primary_field ||
      selectedTargetTable?.fields[0]?.name ||
      "name"
  );
  const [labelOverride, setLabelOverride] = useState<string>(
    initialField?.display_label_override || ""
  );

  const [filterEnabled, setFilterEnabled] = useState<boolean>(
    Boolean(initialField?.link_filter && initialField.link_filter.field)
  );
  const [filterField, setFilterField] = useState<string>(
    initialField?.link_filter?.field || selectedTargetTable?.fields[0]?.name || "status"
  );
  const [filterOperator, setFilterOperator] = useState<
    "equals" | "not_equals" | "contains" | "greater_than" | "less_than"
  >(initialField?.link_filter?.operator || "equals");
  const [filterValue, setFilterValue] = useState<string>(
    initialField?.link_filter?.value || "Active"
  );

  useEffect(() => {
    if (isOpen) {
      if (initialField) {
        setLabel(initialField.label || "");
        const tgtId = initialField.target_table_id || defaultTarget?.id || "";
        setTargetTableId(tgtId);
        setCardinality(initialField.cardinality || (initialField.allow_multiple ? "multiple" : "single"));
        const tgtTable = tables.find((t) => t.id === tgtId) || defaultTarget;
        setTargetDisplayField(
          initialField.target_display_field ||
          tgtTable?.primary_field ||
          tgtTable?.fields[0]?.name ||
          "name"
        );
        setLabelOverride(initialField.display_label_override || "");
        setFilterEnabled(Boolean(initialField.link_filter && initialField.link_filter.field));
        setFilterField(initialField.link_filter?.field || tgtTable?.fields[0]?.name || "status");
        setFilterOperator(initialField.link_filter?.operator || "equals");
        setFilterValue(initialField.link_filter?.value || "Active");
      } else {
        setLabel(defaultTarget ? `${defaultTarget.name} Link` : "Linked Record");
        setTargetTableId(defaultTarget?.id || "");
        setCardinality("single");
        setTargetDisplayField(
          defaultTarget?.primary_field ||
          defaultTarget?.fields[0]?.name ||
          "name"
        );
        setLabelOverride("");
        setFilterEnabled(false);
        setFilterField(defaultTarget?.fields[0]?.name || "status");
        setFilterOperator("equals");
        setFilterValue("Active");
      }
    }
  }, [isOpen, initialField]);

  if (!isOpen) return null;

  const handleTableChange = (newTableId: string) => {
    setTargetTableId(newTableId);
    const newT = tables.find((t) => t.id === newTableId);
    if (newT) {
      setTargetDisplayField(newT.primary_field || newT.fields[0]?.name || "name");
      setFilterField(newT.fields[0]?.name || "status");
    }
  };

  const handleSave = () => {
    const rawName = label
      .toLowerCase()
      .replace(/[^a-z0-9_]+/g, "_")
      .replace(/^_+|_+$/g, "");
    const fieldName = initialField?.name || (rawName.endsWith("_id") ? rawName : `${rawName}_id`);

    const linkSpec: FieldSpec = {
      name: fieldName,
      label: label.trim() || "Linked Record",
      field_type: "Relation",
      required: initialField?.required ?? false,
      ferpa_sensitive: initialField?.ferpa_sensitive ?? false,
      target_table_id: targetTableId,
      target_display_field: targetDisplayField,
      display_label_override: labelOverride.trim() || undefined,
      cardinality,
      allow_multiple: cardinality === "multiple",
      link_filter: filterEnabled && filterField ? {
        field: filterField,
        operator: filterOperator,
        value: filterValue,
      } : undefined,
    };

    onSave(linkSpec);
    onClose();
  };

  return (
    <div
      data-testid="link-characteristics-modal"
      className="fixed inset-0 z-50 flex items-center justify-center bg-slate-900/60 backdrop-blur-xs p-4 animate-fade-in"
      onClick={onClose}
    >
      <div
        className="bg-white dark:bg-slate-900 border border-slate-200 dark:border-slate-800 rounded-2xl shadow-2xl max-w-lg w-full p-6 text-slate-800 dark:text-slate-200 space-y-5"
        onClick={(e) => e.stopPropagation()}
      >
        <div className="flex items-center justify-between pb-3 border-b border-slate-100 dark:border-slate-800">
          <div className="flex items-center gap-2.5">
            <div className="w-8 h-8 rounded-lg bg-purple-100 dark:bg-purple-950/70 border border-purple-200 dark:border-purple-800 flex items-center justify-center text-purple-600 dark:text-purple-400 font-bold">
              🔗
            </div>
            <div>
              <h2 className="text-base font-bold text-slate-900 dark:text-white">
                {initialField ? "Edit Linked Field Characteristics" : "Add Linked Field (Airtable-style)"}
              </h2>
              <p className="text-xs text-slate-500 dark:text-slate-400">
                Configure relational linkage, cardinality, display labels, and filters.
              </p>
            </div>
          </div>
          <button
            type="button"
            data-testid="close-link-modal-btn"
            onClick={onClose}
            className="text-slate-400 hover:text-slate-600 dark:hover:text-slate-200 text-lg leading-none cursor-pointer"
          >
            ✕
          </button>
        </div>

        <div className="space-y-4 text-xs">
          {/* Field Label */}
          <div className="space-y-1.5">
            <label className="font-semibold text-slate-700 dark:text-slate-300 block">
              Field Label
            </label>
            <input
              type="text"
              data-testid="link-field-label-input"
              value={label}
              onChange={(e) => setLabel(e.target.value)}
              placeholder="e.g. Lead Investigator"
              className="w-full px-3 py-2 rounded-lg border border-slate-300 dark:border-slate-700 bg-slate-50 dark:bg-slate-800 text-slate-900 dark:text-white focus:outline-none focus:ring-2 focus:ring-purple-500 font-medium"
            />
          </div>

          {/* Linked Target Table */}
          <div className="space-y-1.5">
            <label className="font-semibold text-slate-700 dark:text-slate-300 block">
              Link to Table
            </label>
            <select
              data-testid="link-target-table-select"
              value={targetTableId}
              onChange={(e) => handleTableChange(e.target.value)}
              className="w-full px-3 py-2 rounded-lg border border-slate-300 dark:border-slate-700 bg-slate-50 dark:bg-slate-800 text-slate-900 dark:text-white focus:outline-none focus:ring-2 focus:ring-purple-500 cursor-pointer font-medium"
            >
              {tables.map((t) => (
                <option key={t.id} value={t.id}>
                  {t.icon || "📑"} {t.name} ({t.fields.length} fields)
                </option>
              ))}
            </select>
          </div>

          {/* Cardinality (Single vs Multiple Records) */}
          <div className="space-y-2 pt-1 border-t border-slate-100 dark:border-slate-800">
            <div className="flex items-center justify-between">
              <label className="font-semibold text-slate-700 dark:text-slate-300">
                Cardinality (Link Multiple Records)
              </label>
              <label className="inline-flex items-center gap-1.5 text-xs text-slate-600 dark:text-slate-400 cursor-pointer">
                <input
                  type="checkbox"
                  data-testid="link-allow-multiple-checkbox"
                  checked={cardinality === "multiple"}
                  onChange={(e) => setCardinality(e.target.checked ? "multiple" : "single")}
                  className="rounded text-purple-600 focus:ring-purple-500 h-4 w-4"
                />
                <span>Allow linking to multiple records</span>
              </label>
            </div>
            <div className="grid grid-cols-2 gap-2">
              <button
                type="button"
                data-testid="link-cardinality-single"
                onClick={() => setCardinality("single")}
                className={`py-2 px-3 rounded-lg border text-left cursor-pointer transition-all ${
                  cardinality === "single"
                    ? "border-purple-500 bg-purple-50/70 dark:bg-purple-950/40 text-purple-900 dark:text-purple-200 font-semibold"
                    : "border-slate-200 dark:border-slate-700 bg-white dark:bg-slate-800/50 text-slate-600 dark:text-slate-400"
                }`}
              >
                <div className="text-xs font-semibold">Single Record (1:1 / N:1)</div>
                <div className="text-[10px] text-slate-400 mt-0.5">e.g. Primary Lead, Sponsor</div>
              </button>
              <button
                type="button"
                data-testid="link-cardinality-multiple"
                onClick={() => setCardinality("multiple")}
                className={`py-2 px-3 rounded-lg border text-left cursor-pointer transition-all ${
                  cardinality === "multiple"
                    ? "border-purple-500 bg-purple-50/70 dark:bg-purple-950/40 text-purple-900 dark:text-purple-200 font-semibold"
                    : "border-slate-200 dark:border-slate-700 bg-white dark:bg-slate-800/50 text-slate-600 dark:text-slate-400"
                }`}
              >
                <div className="text-xs font-semibold">Multiple Records (1:N / M:N)</div>
                <div className="text-[10px] text-slate-400 mt-0.5">e.g. Co-Investigators, Allocations</div>
              </button>
            </div>
          </div>

          {/* Display Field & Label Override */}
          <div className="grid grid-cols-2 gap-3 pt-1 border-t border-slate-100 dark:border-slate-800">
            <div className="space-y-1">
              <label className="font-semibold text-slate-700 dark:text-slate-300 block">
                Target Display Field
              </label>
              <select
                data-testid="link-display-field-select"
                value={targetDisplayField}
                onChange={(e) => setTargetDisplayField(e.target.value)}
                className="w-full px-2.5 py-1.5 rounded-lg border border-slate-300 dark:border-slate-700 bg-slate-50 dark:bg-slate-800 text-slate-900 dark:text-white focus:outline-none focus:ring-1 focus:ring-purple-500 cursor-pointer text-xs"
              >
                {selectedTargetTable?.fields.map((f) => (
                  <option key={f.name} value={f.name}>
                    {f.label} ({f.field_type})
                  </option>
                ))}
              </select>
            </div>
            <div className="space-y-1">
              <label className="font-semibold text-slate-700 dark:text-slate-300 block">
                Label Override / Prefix
              </label>
              <input
                type="text"
                data-testid="link-label-override-input"
                value={labelOverride}
                onChange={(e) => setLabelOverride(e.target.value)}
                placeholder="e.g. Lead PI: "
                className="w-full px-2.5 py-1.5 rounded-lg border border-slate-300 dark:border-slate-700 bg-slate-50 dark:bg-slate-800 text-slate-900 dark:text-white focus:outline-none focus:ring-1 focus:ring-purple-500 text-xs"
              />
            </div>
          </div>

          {/* Filter Linked Records (Limit Record Selection) */}
          <div className="space-y-2 pt-1 border-t border-slate-100 dark:border-slate-800">
            <div className="flex items-center justify-between">
              <label className="font-semibold text-slate-700 dark:text-slate-300">
                Filter Linked Records (Airtable-style)
              </label>
              <label className="inline-flex items-center gap-1.5 text-xs text-slate-600 dark:text-slate-400 cursor-pointer">
                <input
                  type="checkbox"
                  data-testid="link-filter-enable-checkbox"
                  checked={filterEnabled}
                  onChange={(e) => setFilterEnabled(e.target.checked)}
                  className="rounded text-purple-600 focus:ring-purple-500 h-4 w-4"
                />
                <span>Limit record selection with filter</span>
              </label>
            </div>

            {filterEnabled && (
              <div className="p-3 rounded-lg bg-slate-50 dark:bg-slate-800/70 border border-slate-200 dark:border-slate-700 grid grid-cols-3 gap-2 animate-fade-in">
                <div>
                  <label className="text-[10px] text-slate-400 block mb-0.5">Where field</label>
                  <select
                    data-testid="link-filter-field-select"
                    value={filterField}
                    onChange={(e) => setFilterField(e.target.value)}
                    className="w-full px-2 py-1 rounded border border-slate-300 dark:border-slate-700 bg-white dark:bg-slate-900 text-xs"
                  >
                    {selectedTargetTable?.fields.map((f) => (
                      <option key={f.name} value={f.name}>
                        {f.label}
                      </option>
                    ))}
                  </select>
                </div>
                <div>
                  <label className="text-[10px] text-slate-400 block mb-0.5">Operator</label>
                  <select
                    data-testid="link-filter-operator-select"
                    value={filterOperator}
                    onChange={(e) => setFilterOperator(e.target.value as any)}
                    className="w-full px-2 py-1 rounded border border-slate-300 dark:border-slate-700 bg-white dark:bg-slate-900 text-xs"
                  >
                    <option value="equals">equals</option>
                    <option value="not_equals">does not equal</option>
                    <option value="contains">contains</option>
                  </select>
                </div>
                <div>
                  <label className="text-[10px] text-slate-400 block mb-0.5">Value</label>
                  <input
                    type="text"
                    data-testid="link-filter-value-input"
                    value={filterValue}
                    onChange={(e) => setFilterValue(e.target.value)}
                    placeholder="e.g. Physics or Active"
                    className="w-full px-2 py-1 rounded border border-slate-300 dark:border-slate-700 bg-white dark:bg-slate-900 text-xs"
                  />
                </div>
              </div>
            )}
          </div>
        </div>

        <div className="flex items-center justify-end gap-2.5 pt-3 border-t border-slate-100 dark:border-slate-800">
          <button
            type="button"
            data-testid="cancel-link-field-btn"
            onClick={onClose}
            className="px-3.5 py-1.5 rounded-lg border border-slate-300 dark:border-slate-700 text-xs font-semibold hover:bg-slate-50 dark:hover:bg-slate-800 cursor-pointer"
          >
            Cancel
          </button>
          <button
            type="button"
            data-testid="save-linked-field-btn"
            onClick={handleSave}
            className="px-4 py-1.5 rounded-lg bg-purple-600 hover:bg-purple-700 text-white text-xs font-semibold shadow-xs cursor-pointer transition-colors"
          >
            {initialField ? "Update Linked Field" : "Create Linked Field"}
          </button>
        </div>
      </div>
    </div>
  );
};
