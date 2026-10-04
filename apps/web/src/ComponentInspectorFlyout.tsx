import React from "react";
import { GovernedComponentSpec } from "./types";

interface ComponentInspectorFlyoutProps {
  component: GovernedComponentSpec | null;
  isOpen: boolean;
  onClose: () => void;
  onUpdateComponent: (updated: GovernedComponentSpec) => void;
  onDeleteComponent: (id: string) => void;
  onMoveComponent: (id: string, direction: "up" | "down") => void;
  availableColumns?: string[];
}

export const ComponentInspectorFlyout: React.FC<ComponentInspectorFlyoutProps> = ({
  component,
  isOpen,
  onClose,
  onUpdateComponent,
  onDeleteComponent,
  onMoveComponent,
  availableColumns = ["id", "title", "department", "status", "budget", "submitted_at"],
}) => {
  if (!isOpen || !component) return null;

  const updateProp = (field: string, val: any) => {
    onUpdateComponent({
      ...component,
      config: {
        ...component.config,
        [field]: val,
      },
    });
  };

  const updateLayoutWidth = (width: "third" | "half" | "two-thirds" | "full") => {
    onUpdateComponent({
      ...component,
      layout: {
        ...component.layout,
        width,
      },
    });
  };

  const updateAccessGuard = (key: string, val: any) => {
    onUpdateComponent({
      ...component,
      access_guard: {
        ...component.access_guard,
        [key]: val,
      },
    });
  };

  const toggleColumnVisibility = (col: string) => {
    const currentCols: string[] = component.config.visible_columns || availableColumns;
    let nextCols: string[];
    if (currentCols.includes(col)) {
      nextCols = currentCols.filter((c) => c !== col);
    } else {
      nextCols = [...currentCols, col];
    }
    updateProp("visible_columns", nextCols);
  };

  return (
    <div
      data-testid="component-inspector-flyout"
      className="fixed inset-y-0 right-0 w-96 bg-white dark:bg-slate-900 border-l border-slate-200 dark:border-slate-800 shadow-2xl z-40 flex flex-col transition-all duration-200 animate-slide-left"
    >
      {/* Header */}
      <div className="px-5 py-4 border-b border-slate-200 dark:border-slate-800 flex items-center justify-between bg-slate-50/70 dark:bg-slate-800/40">
        <div>
          <div className="flex items-center gap-2">
            <span className="text-[10px] uppercase font-bold tracking-wider px-2 py-0.5 rounded bg-blue-100 text-blue-800 dark:bg-blue-950 dark:text-blue-300">
              {component.type}
            </span>
            <span className="text-xs text-slate-400 font-mono">#{component.id.slice(0, 6)}</span>
          </div>
          <h3 className="text-sm font-bold text-slate-900 dark:text-white mt-1">
            Component Configuration
          </h3>
        </div>
        <button
          type="button"
          onClick={onClose}
          className="text-slate-400 hover:text-slate-600 dark:hover:text-slate-200 p-1.5 rounded-md hover:bg-slate-100 dark:hover:bg-slate-800 text-sm"
          title="Close Inspector"
        >
          ✕
        </button>
      </div>

      {/* Scrollable Body */}
      <div className="flex-1 overflow-y-auto p-5 space-y-6 text-xs">
        {/* Section: General & Layout */}
        <div className="space-y-3">
          <label className="text-[11px] font-bold uppercase tracking-wider text-slate-500 dark:text-slate-400">
            General &amp; Layout
          </label>
          <div>
            <span className="text-slate-600 dark:text-slate-300 font-medium">Component Title</span>
            <input
              type="text"
              value={component.title}
              onChange={(e) => onUpdateComponent({ ...component, title: e.target.value })}
              className="mt-1 w-full px-3 py-1.5 rounded border border-slate-300 dark:border-slate-700 bg-white dark:bg-slate-800 text-slate-900 dark:text-white focus:outline-blue-500"
            />
          </div>

          <div>
            <span className="text-slate-600 dark:text-slate-300 font-medium">Layout Slot Width</span>
            <div className="grid grid-cols-4 gap-1.5 mt-1">
              {(["third", "half", "two-thirds", "full"] as const).map((w) => (
                <button
                  key={w}
                  type="button"
                  onClick={() => updateLayoutWidth(w)}
                  className={`py-1.5 px-2 text-center rounded border font-medium cursor-pointer transition-colors ${
                    component.layout.width === w
                      ? "border-blue-600 bg-blue-50 text-blue-700 dark:bg-blue-950 dark:text-blue-300 font-bold"
                      : "border-slate-200 dark:border-slate-700 text-slate-600 dark:text-slate-400 hover:bg-slate-50 dark:hover:bg-slate-800"
                  }`}
                >
                  {w === "third" ? "1/3" : w === "half" ? "1/2" : w === "two-thirds" ? "2/3" : "Full"}
                </button>
              ))}
            </div>
          </div>
        </div>

        {/* Section: Type-Specific Properties */}
        {component.type === "stat-metric" && (
          <div className="space-y-3 pt-4 border-t border-slate-200 dark:border-slate-800">
            <label className="text-[11px] font-bold uppercase tracking-wider text-slate-500 dark:text-slate-400">
              Metric &amp; Aggregation
            </label>
            <div>
              <span className="text-slate-600 dark:text-slate-300 font-medium">Subtitle / Period</span>
              <input
                type="text"
                value={component.config.subtitle || ""}
                onChange={(e) => updateProp("subtitle", e.target.value)}
                placeholder="e.g. Fiscal Year 2026"
                className="mt-1 w-full px-3 py-1.5 rounded border border-slate-300 dark:border-slate-700 bg-white dark:bg-slate-800 text-slate-900 dark:text-white"
              />
            </div>
            <div>
              <span className="text-slate-600 dark:text-slate-300 font-medium">Aggregation Function</span>
              <select
                value={component.config.aggregation || "count"}
                onChange={(e) => updateProp("aggregation", e.target.value)}
                className="mt-1 w-full px-3 py-1.5 rounded border border-slate-300 dark:border-slate-700 bg-white dark:bg-slate-800 text-slate-900 dark:text-white"
              >
                <option value="count">Count Records (COUNT)</option>
                <option value="sum">Sum Numeric Field (SUM)</option>
                <option value="avg">Average Numeric Field (AVG)</option>
              </select>
            </div>
            {(component.config.aggregation === "sum" || component.config.aggregation === "avg") && (
              <div>
                <span className="text-slate-600 dark:text-slate-300 font-medium">Target Field</span>
                <input
                  type="text"
                  value={component.config.target_field || "budget"}
                  onChange={(e) => updateProp("target_field", e.target.value)}
                  className="mt-1 w-full px-3 py-1.5 rounded border border-slate-300 dark:border-slate-700 bg-white dark:bg-slate-800 text-slate-900 dark:text-white"
                />
              </div>
            )}
            <div>
              <span className="text-slate-600 dark:text-slate-300 font-medium">Filter By Status (Optional)</span>
              <input
                type="text"
                value={component.config.filter_status || ""}
                onChange={(e) => updateProp("filter_status", e.target.value)}
                placeholder="e.g. Approved, Under Review"
                className="mt-1 w-full px-3 py-1.5 rounded border border-slate-300 dark:border-slate-700 bg-white dark:bg-slate-800 text-slate-900 dark:text-white"
              />
            </div>
            <div>
              <span className="text-slate-600 dark:text-slate-300 font-medium">Trend Indicator</span>
              <input
                type="text"
                value={component.config.trend_text || ""}
                onChange={(e) => updateProp("trend_text", e.target.value)}
                placeholder="e.g. +14% vs last cycle"
                className="mt-1 w-full px-3 py-1.5 rounded border border-slate-300 dark:border-slate-700 bg-white dark:bg-slate-800 text-slate-900 dark:text-white"
              />
            </div>
          </div>
        )}

        {component.type === "tabular-grid" && (
          <div className="space-y-3 pt-4 border-t border-slate-200 dark:border-slate-800">
            <label className="text-[11px] font-bold uppercase tracking-wider text-slate-500 dark:text-slate-400">
              TanStack Table Configuration
            </label>
            <div>
              <span className="text-slate-600 dark:text-slate-300 font-medium">Visible Columns (TanStack)</span>
              <div className="space-y-1.5 mt-1.5 border border-slate-200 dark:border-slate-700 rounded p-2 bg-slate-50 dark:bg-slate-800/50">
                {availableColumns.map((col) => {
                  const isVisible = (component.config.visible_columns || availableColumns).includes(col);
                  return (
                    <label key={col} className="flex items-center gap-2 cursor-pointer text-slate-700 dark:text-slate-300">
                      <input
                        type="checkbox"
                        checked={isVisible}
                        onChange={() => toggleColumnVisibility(col)}
                        className="rounded border-slate-300 text-blue-600 focus:ring-blue-500"
                      />
                      <span className="font-mono text-[11px]">{col}</span>
                    </label>
                  );
                })}
              </div>
            </div>
            <div>
              <span className="text-slate-600 dark:text-slate-300 font-medium">Column Rollup Calculation</span>
              <select
                value={component.config.rollup_type || "none"}
                onChange={(e) => updateProp("rollup_type", e.target.value)}
                className="mt-1 w-full px-3 py-1.5 rounded border border-slate-300 dark:border-slate-700 bg-white dark:bg-slate-800 text-slate-900 dark:text-white"
              >
                <option value="none">No Footer Rollup</option>
                <option value="sum">Sum Totals (SUM)</option>
                <option value="avg">Average Values (AVG)</option>
                <option value="count">Record Count (COUNT)</option>
              </select>
            </div>
            <div className="flex items-center gap-2 pt-1">
              <input
                type="checkbox"
                id="enable-search"
                checked={component.config.enable_search ?? true}
                onChange={(e) => updateProp("enable_search", e.target.checked)}
                className="rounded border-slate-300 text-blue-600 focus:ring-blue-500"
              />
              <label htmlFor="enable-search" className="text-slate-700 dark:text-slate-300 cursor-pointer">
                Enable Live Text Filter Bar
              </label>
            </div>
          </div>
        )}

        {component.type === "kanban-stage" && (
          <div className="space-y-3 pt-4 border-t border-slate-200 dark:border-slate-800">
            <label className="text-[11px] font-bold uppercase tracking-wider text-slate-500 dark:text-slate-400">
              Kanban Stage Settings
            </label>
            <div>
              <span className="text-slate-600 dark:text-slate-300 font-medium">Stage Grouping Field</span>
              <input
                type="text"
                value={component.config.stage_field || "status"}
                onChange={(e) => updateProp("stage_field", e.target.value)}
                className="mt-1 w-full px-3 py-1.5 rounded border border-slate-300 dark:border-slate-700 bg-white dark:bg-slate-800 text-slate-900 dark:text-white"
              />
            </div>
            <div>
              <span className="text-slate-600 dark:text-slate-300 font-medium">Card Title Field</span>
              <input
                type="text"
                value={component.config.card_title_field || "title"}
                onChange={(e) => updateProp("card_title_field", e.target.value)}
                className="mt-1 w-full px-3 py-1.5 rounded border border-slate-300 dark:border-slate-700 bg-white dark:bg-slate-800 text-slate-900 dark:text-white"
              />
            </div>
          </div>
        )}

        {component.type === "intake-form" && (
          <div className="space-y-3 pt-4 border-t border-slate-200 dark:border-slate-800">
            <label className="text-[11px] font-bold uppercase tracking-wider text-slate-500 dark:text-slate-400">
              Intake Form Properties
            </label>
            <div>
              <span className="text-slate-600 dark:text-slate-300 font-medium">Submit Button Label</span>
              <input
                type="text"
                value={component.config.submit_button_label || "Submit Record"}
                onChange={(e) => updateProp("submit_button_label", e.target.value)}
                className="mt-1 w-full px-3 py-1.5 rounded border border-slate-300 dark:border-slate-700 bg-white dark:bg-slate-800 text-slate-900 dark:text-white"
              />
            </div>
            <div>
              <span className="text-slate-600 dark:text-slate-300 font-medium">Success Notification Message</span>
              <input
                type="text"
                value={component.config.success_message || "Record successfully submitted and staged."}
                onChange={(e) => updateProp("success_message", e.target.value)}
                className="mt-1 w-full px-3 py-1.5 rounded border border-slate-300 dark:border-slate-700 bg-white dark:bg-slate-800 text-slate-900 dark:text-white"
              />
            </div>
          </div>
        )}

        {component.type === "rich-banner" && (
          <div className="space-y-3 pt-4 border-t border-slate-200 dark:border-slate-800">
            <label className="text-[11px] font-bold uppercase tracking-wider text-slate-500 dark:text-slate-400">
              Banner Style &amp; Body
            </label>
            <div>
              <span className="text-slate-600 dark:text-slate-300 font-medium">Banner Variant</span>
              <select
                value={component.config.variant || "info"}
                onChange={(e) => updateProp("variant", e.target.value)}
                className="mt-1 w-full px-3 py-1.5 rounded border border-slate-300 dark:border-slate-700 bg-white dark:bg-slate-800 text-slate-900 dark:text-white"
              >
                <option value="info">Informational (Blue)</option>
                <option value="warning">Compliance Warning (Amber)</option>
                <option value="success">Approved Notice (Emerald)</option>
              </select>
            </div>
            <div>
              <span className="text-slate-600 dark:text-slate-300 font-medium">Banner Content (Markdown)</span>
              <textarea
                rows={3}
                value={component.config.content || ""}
                onChange={(e) => updateProp("content", e.target.value)}
                className="mt-1 w-full px-3 py-1.5 rounded border border-slate-300 dark:border-slate-700 bg-white dark:bg-slate-800 text-slate-900 dark:text-white"
              />
            </div>
          </div>
        )}

        {/* Section: Design System Tokens */}
        <div className="space-y-3 pt-4 border-t border-slate-200 dark:border-slate-800">
          <label className="text-[11px] font-bold uppercase tracking-wider text-slate-500 dark:text-slate-400">
            Design Tokens (Org Theme)
          </label>
          <div>
            <span className="text-slate-600 dark:text-slate-300 font-medium">Approved Color Accent</span>
            <div className="flex items-center gap-2 mt-2">
              {[
                { id: "blue", label: "Navy", bg: "bg-blue-600" },
                { id: "emerald", label: "Emerald", bg: "bg-emerald-600" },
                { id: "amber", label: "Amber", bg: "bg-amber-600" },
                { id: "indigo", label: "Indigo", bg: "bg-indigo-600" },
                { id: "slate", label: "Slate", bg: "bg-slate-600" },
              ].map((token) => (
                <button
                  key={token.id}
                  type="button"
                  onClick={() => updateProp("accent_color", token.id)}
                  className={`w-7 h-7 rounded-full ${token.bg} transition-all ${
                    (component.config.accent_color || "blue") === token.id
                      ? "ring-2 ring-offset-2 ring-blue-500 scale-110"
                      : "opacity-70 hover:opacity-100"
                  }`}
                  title={token.label}
                />
              ))}
            </div>
          </div>
        </div>

        {/* Section: Data Security & Cedar Guards */}
        <div className="space-y-3 pt-4 border-t border-slate-200 dark:border-slate-800">
          <label className="text-[11px] font-bold uppercase tracking-wider text-slate-500 dark:text-slate-400">
            Sovereign Access Guard
          </label>
          <div>
            <span className="text-slate-600 dark:text-slate-300 font-medium">Minimum Role (EduPerson)</span>
            <select
              value={component.access_guard?.required_affiliation || "student"}
              onChange={(e) => updateAccessGuard("required_affiliation", e.target.value)}
              className="mt-1 w-full px-3 py-1.5 rounded border border-slate-300 dark:border-slate-700 bg-white dark:bg-slate-800 text-slate-900 dark:text-white"
            >
              <option value="student">Student (All Affiliated Users)</option>
              <option value="staff">Staff Only</option>
              <option value="faculty">Faculty Only</option>
              <option value="admin">Administrator Only</option>
            </select>
          </div>
          <div>
            <span className="text-slate-600 dark:text-slate-300 font-medium">Classification Ceiling</span>
            <select
              value={component.access_guard?.classification_max || "internal"}
              onChange={(e) => updateAccessGuard("classification_max", e.target.value)}
              className="mt-1 w-full px-3 py-1.5 rounded border border-slate-300 dark:border-slate-700 bg-white dark:bg-slate-800 text-slate-900 dark:text-white"
            >
              <option value="public">Public (Unrestricted)</option>
              <option value="internal">Internal Operational</option>
              <option value="restricted">Restricted (FERPA Strict)</option>
            </select>
          </div>
        </div>
      </div>

      {/* Footer Actions */}
      <div className="p-4 border-t border-slate-200 dark:border-slate-800 bg-slate-50/70 dark:bg-slate-800/40 flex items-center justify-between">
        <div className="flex items-center gap-1.5">
          <button
            type="button"
            onClick={() => onMoveComponent(component.id, "up")}
            className="p-1.5 rounded border border-slate-300 dark:border-slate-700 hover:bg-white dark:hover:bg-slate-800 text-slate-600 dark:text-slate-300 text-xs font-semibold"
            title="Move Component Up"
          >
            ↑ Up
          </button>
          <button
            type="button"
            onClick={() => onMoveComponent(component.id, "down")}
            className="p-1.5 rounded border border-slate-300 dark:border-slate-700 hover:bg-white dark:hover:bg-slate-800 text-slate-600 dark:text-slate-300 text-xs font-semibold"
            title="Move Component Down"
          >
            ↓ Down
          </button>
        </div>
        <button
          type="button"
          onClick={() => onDeleteComponent(component.id)}
          className="px-3 py-1.5 rounded bg-rose-50 text-rose-600 dark:bg-rose-950/60 dark:text-rose-300 hover:bg-rose-100 dark:hover:bg-rose-900/60 font-semibold text-xs transition-colors"
        >
          Remove Component
        </button>
      </div>
    </div>
  );
};
