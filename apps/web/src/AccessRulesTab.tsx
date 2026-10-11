import React, { useState, useMemo } from "react";
import { AppTable, FieldSpec, RegisteredApp, AccessRowRule, AccessColumnRule } from "./types";
import { apiClient } from "./api";
import { Banner } from "./ui/Banner";

export interface AccessRulesTabProps {
  app: RegisteredApp;
  tables: AppTable[];
  onSaveProposal?: (updatedApp: RegisteredApp) => void;
  onOpenProposals?: () => void;
}

const SUPPORTED_FIELD_TYPES = ["Text", "Select", "Collaborator", "Number", "Date", "Boolean"];

function fieldLabel(name: string, fields: FieldSpec[]): string {
  const f = fields.find((x) => x.name === name);
  return f?.label || name;
}

function roleNounPlural(r: string): string {
  const norm = r.toLowerCase();
  if (norm === "owner") return "owners";
  if (norm === "admin") return "admins";
  if (norm === "editor") return "editors";
  if (norm === "viewer") return "viewers";
  return norm + "s";
}

function formatWho(exceptRoles: string[]): string {
  if (!exceptRoles || exceptRoles.length === 0) return "Everyone";
  const plurals = exceptRoles.map(roleNounPlural);
  if (plurals.length === 1) return `People who are not ${plurals[0]}`;
  if (plurals.length === 2) return `People who are not ${plurals[0]} or ${plurals[1]}`;
  const init = plurals.slice(0, -1).join(", ");
  return `People who are not ${init}, or ${plurals[plurals.length - 1]}`;
}

function formatOps(ops: string[]): string {
  const hasRead = ops.includes("Read");
  const hasCreate = ops.includes("Create");
  const hasUpdate = ops.includes("Update");
  const hasDelete = ops.includes("Delete");

  if (hasRead && !hasCreate && !hasUpdate && !hasDelete) return "can see only rows";
  if (hasRead && hasUpdate && !hasCreate && !hasDelete) return "can see and change rows";
  if (!hasRead && hasUpdate) return "can change rows";

  const parts: string[] = [];
  if (hasRead) parts.push("see rows");
  if (hasCreate) parts.push("add rows");
  if (hasUpdate) parts.push("change rows");
  if (hasDelete) parts.push("remove rows");
  return parts.length > 0 ? `can ${parts.join(" and ")}` : "has no actions on rows";
}

export function predInWords(pred: any, fields: FieldSpec[]): string {
  if (!pred) return "";
  if (pred.True !== undefined) return "all rows";
  if (pred.False !== undefined) return "no rows";

  if (pred.Cmp) {
    const { field, op, value } = pred.Cmp;
    const label = fieldLabel(field, fields);

    let valStr = "";
    if (value?.Person) {
      const p = value.Person;
      if (p === "Address") valStr = "themselves";
      else if (p === "Unit") valStr = "their unit";
      else if (p === "Department") valStr = "their department";
      else if (p === "Position") valStr = "their positions";
      else valStr = `their ${p.toLowerCase()}`;
    } else if (value?.Literal) {
      valStr = typeof value.Literal === "string" ? `"${value.Literal}"` : String(value.Literal);
    } else {
      valStr = JSON.stringify(value);
    }

    if (op === "Eq") return `${label} is ${valStr}`;
    if (op === "Ne") return `${label} is not ${valStr}`;
    if (op === "In") return `${label} is one of ${valStr}`;
    if (op === "IsEmpty") return `${label} is empty`;
    if (op === "IsNotEmpty") return `${label} is not empty`;
    return `${label} ${op} ${valStr}`;
  }

  if (pred.And) {
    const list = pred.And.map((p: any) => predInWords(p, fields));
    return list.join(" and ");
  }

  if (pred.Or) {
    const list = pred.Or.map((p: any) => predInWords(p, fields));
    return list.join(" or ");
  }

  if (pred.Not) {
    return `not (${predInWords(pred.Not, fields)})`;
  }

  return JSON.stringify(pred);
}

export function ruleSentence(rule: AccessRowRule, fields: FieldSpec[]): string {
  const who = formatWho(rule.except_roles);
  const ops = formatOps(rule.ops);
  const cond = predInWords(rule.visible_when, fields);
  return `${who} ${ops} where ${cond}.`;
}

export function detectProblems(rules: AccessRowRule[], table: AppTable): string[] {
  const problems: string[] = [];
  const fields = table.fields;
  const fieldNames = new Set(fields.map((f) => f.name));

  for (const rule of rules) {
    const checkPred = (p: any) => {
      if (!p) return;
      if (p.False !== undefined) {
        problems.push(`never_true: Rule '${rule.id}' predicate folds to false for everyone`);
      }
      if (p.Cmp) {
        const { field } = p.Cmp;
        if (!fieldNames.has(field)) {
          problems.push(`field_missing: Field '${field}' is missing or retired in table '${table.name}'`);
        } else {
          const f = fields.find((x) => x.name === field);
          if (f && !SUPPORTED_FIELD_TYPES.includes(f.field_type)) {
            problems.push(`unsupported_field: Field '${field}' has type '${f.field_type}', which cannot be used in access rules`);
          }
        }
      }
      if (p.And) {
        // contradiction Eq and Ne
        const cmps = p.And.filter((x: any) => x.Cmp).map((x: any) => x.Cmp);
        for (const c1 of cmps) {
          for (const c2 of cmps) {
            if (c1.field === c2.field && JSON.stringify(c1.value) === JSON.stringify(c2.value)) {
              if ((c1.op === "Eq" && c2.op === "Ne") || (c1.op === "Ne" && c2.op === "Eq")) {
                problems.push(`never_true: Rule '${rule.id}' contains a contradiction on field '${c1.field}'`);
              }
            }
          }
        }
        p.And.forEach(checkPred);
      }
      if (p.Or) p.Or.forEach(checkPred);
      if (p.Not) checkPred(p.Not);
    };

    checkPred(rule.visible_when);

    if (rule.ops.includes("Read") && rule.except_roles.length === 0 && rule.visible_when?.False !== undefined) {
      problems.push(`hides_all: No role can see any rows in table '${table.name}'`);
    }
  }

  return Array.from(new Set(problems));
}

export const AccessRulesTab: React.FC<AccessRulesTabProps> = ({
  app,
  tables,
  onSaveProposal,
  onOpenProposals,
}) => {
  const [selectedTableId, setSelectedTableId] = useState<string>(tables[0]?.id || "");
  const currentTable = useMemo(
    () => tables.find((t) => t.id === selectedTableId) || tables[0],
    [tables, selectedTableId]
  );

  const initialRows = useMemo(() => app.manifest?.access?.rows || [], [app]);
  const initialColumns = useMemo(() => app.manifest?.access?.columns || [], [app]);

  const [rowRules, setRowRules] = useState<AccessRowRule[]>(initialRows);
  const [columnRules] = useState<AccessColumnRule[]>(initialColumns);

  const tableRules = useMemo(
    () => rowRules.filter((r) => r.table_id === currentTable?.id),
    [rowRules, currentTable]
  );

  // Problem detection
  const problems = useMemo(
    () => (currentTable ? detectProblems(tableRules, currentTable) : []),
    [tableRules, currentTable]
  );

  // Builder Modal / State
  const [isBuilderOpen, setIsBuilderOpen] = useState<boolean>(false);
  const [builderOps, setBuilderOps] = useState<string[]>(["Read"]);
  const [conditions, setConditions] = useState<
    Array<{ field: string; op: "Eq" | "Ne" | "In"; valType: "person" | "literal"; value: string }>
  >([
    {
      field: currentTable?.fields[0]?.name || "advisor",
      op: "Eq",
      valType: "person",
      value: "Address",
    },
  ]);
  const [exceptRoles, setExceptRoles] = useState<string[]>(["owner", "admin"]);
  const [ruleMessage, setRuleMessage] = useState<string>("");

  // Test Person Panel
  const [testPersonEppn, setTestPersonEppn] = useState<string>("rivera@state.edu");
  const [testPersonName, setTestPersonName] = useState<string>("Dr. Rivera");
  const [previewResult, setPreviewResult] = useState<{
    can_see: number;
    can_add: number;
    can_change: number;
    can_remove: number;
    total_rows: number;
    hidden_columns: string[];
  } | null>(null);
  const [isPreviewLoading, setIsPreviewLoading] = useState<boolean>(false);
  const [previewError, setPreviewError] = useState<string | null>(null);

  // Run preview
  const runPreview = async () => {
    if (!currentTable) return;
    setIsPreviewLoading(true);
    setPreviewError(null);
    try {
      const res = await apiClient.previewAccess(app.slug, {
        table_id: currentTable.id,
        eppn: testPersonEppn,
        rules: rowRules,
        column_rules: columnRules,
      });
      setPreviewResult(res);
    } catch (err: any) {
      setPreviewError(err?.message || "Failed to load preview");
    } finally {
      setIsPreviewLoading(false);
    }
  };

  const handleAddCondition = () => {
    if (!currentTable?.fields[0]) return;
    setConditions((prev) => [
      ...prev,
      {
        field: currentTable.fields[0].name,
        op: "Eq",
        valType: "literal",
        value: "",
      },
    ]);
  };

  const handleRemoveCondition = (index: number) => {
    setConditions((prev) => prev.filter((_, i) => i !== index));
  };

  const buildPredicateFromConditions = () => {
    const cmps = conditions.map((c) => {
      let valOperand: any;
      if (c.valType === "person") {
        valOperand = { Person: c.value };
      } else {
        valOperand = { Literal: c.value };
      }
      return {
        Cmp: {
          field: c.field,
          op: c.op,
          value: valOperand,
        },
      };
    });
    if (cmps.length === 1) return cmps[0];
    return { And: cmps };
  };

  const handleSaveRule = () => {
    if (!currentTable) return;
    const newRule: AccessRowRule = {
      id: `rule_${Date.now()}`,
      table_id: currentTable.id,
      ops: builderOps as any,
      visible_when: buildPredicateFromConditions(),
      except_roles: exceptRoles,
      message: ruleMessage.trim() ? ruleMessage.trim().slice(0, 200) : undefined,
    };
    setRowRules((prev) => [...prev, newRule]);
    setIsBuilderOpen(false);
    setRuleMessage("");
  };

  const handleDeleteRule = (id: string) => {
    setRowRules((prev) => prev.filter((r) => r.id !== id));
  };

  const handleSaveProposal = () => {
    const updatedApp: RegisteredApp = {
      ...app,
      manifest: {
        ...app.manifest,
        access: {
          rows: rowRules,
          columns: columnRules,
        },
      },
    };
    if (onSaveProposal) {
      onSaveProposal(updatedApp);
    }
    if (onOpenProposals) {
      onOpenProposals();
    }
  };

  return (
    <div data-testid="access-rules-tab" className="p-6 max-w-5xl mx-auto space-y-6">
      <div className="flex items-center justify-between flex-wrap gap-4">
        <div>
          <h2 className="text-lg font-bold text-slate-900 dark:text-white">Who sees what</h2>
          <p className="text-xs text-slate-500 mt-1">
            Row and column rules narrow visibility by operation, role, and person attributes.
          </p>
        </div>
        <div className="flex items-center gap-3">
          <button
            type="button"
            data-testid="save-access-proposal-btn"
            onClick={handleSaveProposal}
            className="px-4 py-2 bg-blue-600 hover:bg-blue-700 text-white text-xs font-semibold rounded-lg shadow-xs cursor-pointer transition-colors"
          >
            Save proposal
          </button>
        </div>
      </div>

      {/* Problems as Banners in words */}
      {problems.length > 0 && (
        <div data-testid="problems-banner-container" className="space-y-2">
          {problems.map((p, idx) => (
            <Banner key={idx} kind="warning">
              <span className="font-semibold">{p}</span>
            </Banner>
          ))}
        </div>
      )}

      {/* Table Selector */}
      <div className="flex items-center gap-3 border-b border-slate-200 dark:border-slate-800 pb-3">
        <label className="text-xs font-bold text-slate-700 dark:text-slate-300">Table:</label>
        <select
          data-testid="access-table-select"
          value={selectedTableId}
          onChange={(e) => setSelectedTableId(e.target.value)}
          className="text-xs px-3 py-1.5 rounded-lg border border-slate-300 dark:border-slate-700 bg-white dark:bg-slate-800 font-semibold text-slate-900 dark:text-white"
        >
          {tables.map((tbl) => (
            <option key={tbl.id} value={tbl.id}>
              {tbl.name} ({tbl.id})
            </option>
          ))}
        </select>
      </div>

      {/* Rules list formatted as sentences */}
      <div className="bg-white dark:bg-slate-900 border border-slate-200 dark:border-slate-800 rounded-xl p-5 shadow-xs space-y-4">
        <div className="flex items-center justify-between">
          <h3 className="text-xs font-bold uppercase tracking-wider text-slate-500">
            Rules for {currentTable?.name}
          </h3>
          <button
            type="button"
            data-testid="add-rule-btn"
            onClick={() => setIsBuilderOpen(true)}
            className="flex items-center gap-1.5 px-3 py-1.5 rounded-lg bg-slate-100 hover:bg-slate-200 dark:bg-slate-800 dark:hover:bg-slate-700 text-slate-700 dark:text-slate-200 text-xs font-semibold cursor-pointer transition-colors"
          >
            + Add a rule
          </button>
        </div>

        {tableRules.length === 0 ? (
          <p className="text-xs text-slate-400 italic py-4 text-center">
            No access rules configured for this table. Everyone can see all rows.
          </p>
        ) : (
          <div className="divide-y divide-slate-100 dark:divide-slate-800" data-testid="rules-list">
            {tableRules.map((r) => (
              <div key={r.id} className="py-3 flex items-start justify-between gap-4">
                <div className="space-y-1">
                  <p className="text-xs font-medium text-slate-900 dark:text-slate-100" data-testid={`rule-sentence-${r.id}`}>
                    {ruleSentence(r, currentTable?.fields || [])}
                  </p>
                  {r.message && (
                    <p className="text-[11px] text-slate-500 italic">"{r.message}"</p>
                  )}
                </div>
                <button
                  type="button"
                  data-testid={`delete-rule-${r.id}`}
                  onClick={() => handleDeleteRule(r.id)}
                  className="text-slate-400 hover:text-red-500 text-xs p-1 cursor-pointer"
                  title="Remove rule"
                >
                  ✕
                </button>
              </div>
            ))}
          </div>
        )}
      </div>

      {/* Rule Builder Panel / Modal */}
      {isBuilderOpen && (
        <div data-testid="rule-builder" className="bg-white dark:bg-slate-900 border border-blue-200 dark:border-blue-900 rounded-xl p-5 shadow-md space-y-4">
          <div className="flex items-center justify-between border-b border-slate-100 dark:border-slate-800 pb-3">
            <h4 className="text-sm font-bold text-slate-900 dark:text-white">Rule Builder</h4>
            <button
              type="button"
              onClick={() => setIsBuilderOpen(false)}
              className="text-slate-400 hover:text-slate-600 text-xs cursor-pointer"
            >
              ✕
            </button>
          </div>

          {/* Operations as checkboxes */}
          <div className="space-y-1.5">
            <label className="text-xs font-bold text-slate-700 dark:text-slate-300">Operations:</label>
            <div className="flex items-center gap-4 text-xs">
              {[
                { id: "Read", label: "see rows" },
                { id: "Create", label: "add rows" },
                { id: "Update", label: "change rows" },
                { id: "Delete", label: "remove rows" },
              ].map((op) => (
                <label key={op.id} className="flex items-center gap-1.5 cursor-pointer">
                  <input
                    type="checkbox"
                    data-testid={`op-checkbox-${op.id}`}
                    checked={builderOps.includes(op.id)}
                    onChange={(e) => {
                      if (e.target.checked) setBuilderOps((prev) => [...prev, op.id]);
                      else setBuilderOps((prev) => prev.filter((x) => x !== op.id));
                    }}
                    className="rounded text-blue-600 focus:ring-blue-500"
                  />
                  <span>{op.label}</span>
                </label>
              ))}
            </div>
          </div>

          {/* Conditions as rows of field, is / is not / is one of, and value or person attr */}
          <div className="space-y-2">
            <label className="text-xs font-bold text-slate-700 dark:text-slate-300">Conditions:</label>
            <div className="space-y-2">
              {conditions.map((cond, idx) => (
                <div key={idx} className="flex items-center gap-2 flex-wrap" data-testid={`condition-row-${idx}`}>
                  <select
                    data-testid={`condition-field-${idx}`}
                    value={cond.field}
                    onChange={(e) => {
                      const val = e.target.value;
                      setConditions((prev) =>
                        prev.map((c, i) => (i === idx ? { ...c, field: val } : c))
                      );
                    }}
                    className="text-xs px-2.5 py-1.5 rounded border border-slate-300 dark:border-slate-700 bg-white dark:bg-slate-800"
                  >
                    {currentTable?.fields.map((f) => (
                      <option key={f.name} value={f.name}>
                        {f.label} ({f.field_type})
                      </option>
                    ))}
                  </select>

                  <select
                    data-testid={`condition-op-${idx}`}
                    value={cond.op}
                    onChange={(e) => {
                      const val = e.target.value as any;
                      setConditions((prev) =>
                        prev.map((c, i) => (i === idx ? { ...c, op: val } : c))
                      );
                    }}
                    className="text-xs px-2.5 py-1.5 rounded border border-slate-300 dark:border-slate-700 bg-white dark:bg-slate-800"
                  >
                    <option value="Eq">is</option>
                    <option value="Ne">is not</option>
                    <option value="In">is one of</option>
                  </select>

                  <select
                    data-testid={`condition-valtype-${idx}`}
                    value={cond.valType}
                    onChange={(e) => {
                      const val = e.target.value as any;
                      setConditions((prev) =>
                        prev.map((c, i) =>
                          i === idx
                            ? {
                                ...c,
                                valType: val,
                                value: val === "person" ? "Address" : "",
                              }
                            : c
                        )
                      );
                    }}
                    className="text-xs px-2.5 py-1.5 rounded border border-slate-300 dark:border-slate-700 bg-white dark:bg-slate-800"
                  >
                    <option value="person">Person attribute</option>
                    <option value="literal">Value</option>
                  </select>

                  {cond.valType === "person" ? (
                    <select
                      data-testid={`condition-person-val-${idx}`}
                      value={cond.value}
                      onChange={(e) => {
                        const val = e.target.value;
                        setConditions((prev) =>
                          prev.map((c, i) => (i === idx ? { ...c, value: val } : c))
                        );
                      }}
                      className="text-xs px-2.5 py-1.5 rounded border border-slate-300 dark:border-slate-700 bg-white dark:bg-slate-800"
                    >
                      <option value="Address">the person</option>
                      <option value="Unit">their unit</option>
                      <option value="Department">their department</option>
                      <option value="Position">their positions</option>
                    </select>
                  ) : (
                    <input
                      type="text"
                      data-testid={`condition-literal-val-${idx}`}
                      value={cond.value}
                      placeholder="value"
                      onChange={(e) => {
                        const val = e.target.value;
                        setConditions((prev) =>
                          prev.map((c, i) => (i === idx ? { ...c, value: val } : c))
                        );
                      }}
                      className="text-xs px-2.5 py-1.5 rounded border border-slate-300 dark:border-slate-700 bg-white dark:bg-slate-800"
                    />
                  )}

                  {conditions.length > 1 && (
                    <button
                      type="button"
                      onClick={() => handleRemoveCondition(idx)}
                      className="text-slate-400 hover:text-red-500 text-xs p-1"
                    >
                      ✕
                    </button>
                  )}
                </div>
              ))}
            </div>
            <button
              type="button"
              onClick={handleAddCondition}
              className="text-[11px] text-blue-600 hover:text-blue-700 font-semibold cursor-pointer"
            >
              + Add condition
            </button>
          </div>

          {/* Exceptions as roles */}
          <div className="space-y-1.5">
            <label className="text-xs font-bold text-slate-700 dark:text-slate-300">Exceptions (roles):</label>
            <div className="flex items-center gap-4 text-xs">
              {["owner", "admin", "editor", "viewer"].map((r) => (
                <label key={r} className="flex items-center gap-1.5 cursor-pointer">
                  <input
                    type="checkbox"
                    data-testid={`except-role-${r}`}
                    checked={exceptRoles.includes(r)}
                    onChange={(e) => {
                      if (e.target.checked) setExceptRoles((prev) => [...prev, r]);
                      else setExceptRoles((prev) => prev.filter((x) => x !== r));
                    }}
                    className="rounded text-blue-600 focus:ring-blue-500"
                  />
                  <span>{r}</span>
                </label>
              ))}
            </div>
          </div>

          {/* Message box with character count */}
          <div className="space-y-1">
            <div className="flex items-center justify-between">
              <label className="text-xs font-bold text-slate-700 dark:text-slate-300">
                Message (optional):
              </label>
              <span className="text-[10px] text-slate-400 font-mono" data-testid="message-char-count">
                {ruleMessage.length}/200
              </span>
            </div>
            <input
              type="text"
              maxLength={200}
              data-testid="rule-message-input"
              value={ruleMessage}
              placeholder="Explanation when access is restricted..."
              onChange={(e) => setRuleMessage(e.target.value)}
              className="w-full text-xs px-3 py-1.5 rounded border border-slate-300 dark:border-slate-700 bg-white dark:bg-slate-800"
            />
          </div>

          <div className="flex items-center justify-end gap-3 pt-3 border-t border-slate-100 dark:border-slate-800">
            <button
              type="button"
              onClick={() => setIsBuilderOpen(false)}
              className="px-3 py-1.5 rounded text-xs text-slate-600 hover:text-slate-800 dark:text-slate-300 cursor-pointer"
            >
              Cancel
            </button>
            <button
              type="button"
              data-testid="save-rule-btn"
              onClick={handleSaveRule}
              className="px-3.5 py-1.5 rounded bg-blue-600 hover:bg-blue-700 text-white text-xs font-semibold cursor-pointer"
            >
              Add rule
            </button>
          </div>
        </div>
      )}

      {/* Test a person panel */}
      <div data-testid="test-a-person-panel" className="bg-white dark:bg-slate-900 border border-slate-200 dark:border-slate-800 rounded-xl p-5 shadow-xs space-y-4">
        <h3 className="text-xs font-bold uppercase tracking-wider text-slate-500">Test a person</h3>
        <div className="flex items-center gap-3 flex-wrap">
          <input
            type="text"
            data-testid="test-person-name-input"
            value={testPersonName}
            placeholder="Name (e.g. Dr. Rivera)"
            onChange={(e) => setTestPersonName(e.target.value)}
            className="text-xs px-3 py-1.5 rounded border border-slate-300 dark:border-slate-700 bg-white dark:bg-slate-800"
          />
          <input
            type="text"
            data-testid="test-person-eppn-input"
            value={testPersonEppn}
            placeholder="eppn (e.g. rivera@state.edu)"
            onChange={(e) => setTestPersonEppn(e.target.value)}
            className="text-xs px-3 py-1.5 rounded border border-slate-300 dark:border-slate-700 bg-white dark:bg-slate-800 font-mono"
          />
          <button
            type="button"
            data-testid="run-preview-btn"
            onClick={runPreview}
            disabled={isPreviewLoading}
            className="px-3.5 py-1.5 rounded bg-slate-800 hover:bg-slate-700 text-white text-xs font-semibold cursor-pointer transition-colors"
          >
            {isPreviewLoading ? "Testing..." : "Test preview"}
          </button>
        </div>

        {previewError && (
          <p className="text-xs text-red-500 font-medium">{previewError}</p>
        )}

        {previewResult && (
          <div data-testid="preview-counts-result" className="p-4 rounded-lg bg-slate-50 dark:bg-slate-800/50 border border-slate-200 dark:border-slate-700">
            <p className="text-sm font-semibold text-slate-900 dark:text-white">
              {testPersonName || testPersonEppn} would see {previewResult.can_see.toLocaleString()} of{" "}
              {previewResult.total_rows.toLocaleString()} rows and could change{" "}
              {previewResult.can_change.toLocaleString()}.
            </p>
            {previewResult.hidden_columns && previewResult.hidden_columns.length > 0 && (
              <p className="text-xs text-amber-600 dark:text-amber-400 mt-1 font-mono">
                Hidden columns: {previewResult.hidden_columns.join(", ")}
              </p>
            )}
          </div>
        )}
      </div>
    </div>
  );
};
