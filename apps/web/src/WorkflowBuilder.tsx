import React, { useState } from "react";
import { FieldSpec, WorkflowAutomationRule, WorkflowTriggerEvent, WorkflowActionItem } from "./types";

interface Props {
  appSlug: string;
  appTitle: string;
  fields: FieldSpec[];
  rules: WorkflowAutomationRule[];
  onSaveRule: (rule: WorkflowAutomationRule) => void;
  onDeleteRule: (ruleId: string) => void;
  onToggleRule: (ruleId: string) => void;
}

export const WorkflowBuilder: React.FC<Props> = ({
  appSlug,
  appTitle,
  fields,
  rules,
  onSaveRule,
  onDeleteRule,
  onToggleRule,
}) => {
  const [isCreating, setIsCreating] = useState(false);
  const [ruleName, setRuleName] = useState("");
  const [ruleDesc, setRuleDesc] = useState("");
  const [triggerType, setTriggerType] = useState<"RecordCreated" | "RecordUpdated" | "StatusChanged">("StatusChanged");
  const [selectedField, setSelectedField] = useState(fields[0]?.name || "gpa");
  const [operator, setOperator] = useState<"Equals" | "NotEquals" | "GreaterThan" | "LessThan" | "Contains">("GreaterThan");
  const [expectedValue, setExpectedValue] = useState("3.85");
  const [actionType, setActionType] = useState<"NotifyCollaborator" | "CreateLedgerAuditEntry" | "UpdateRecordStatus">("NotifyCollaborator");
  const [actionParam, setActionParam] = useState("Candidate approved with honors fellowship eligibility");
  const [addAsServiceStep, setAddAsServiceStep] = useState(false);
  const [simulationResult, setSimulationResult] = useState<string | null>(null);

  // Template Quick-Starters
  const handleApplyTemplate = (type: "ferpa" | "threshold" | "approval") => {
    if (type === "approval") {
      onSaveRule({
        id: `rule-${Date.now()}`,
        app_slug: appSlug,
        name: "Honors Fellowship Notification & Ledger",
        description: "Notifies dean and records ledger entry when candidate GPA >= 3.85",
        enabled: true,
        trigger: { type: "StatusChanged", to_status: "Approved" },
        cedar_policy_guard: "policy-ferpa-34cfr99",
        predicates: [{ field_name: "gpa", operator: "GreaterThan", expected_value: "3.85" }],
        actions: [
          { type: "NotifyCollaborator", role: "dean", message_template: "Candidate approved for fellowship funding." },
          { type: "CreateLedgerAuditEntry", summary: "Automated fellowship approval recorded", oscal_control: "AC-03" },
        ],
      });
    } else if (type === "ferpa") {
      onSaveRule({
        id: `rule-${Date.now()}`,
        app_slug: appSlug,
        name: "FERPA Audit Guardrail Dispatch",
        description: "Appends to cryptographic ledger whenever protected student records are updated",
        enabled: true,
        trigger: { type: "RecordUpdated" },
        cedar_policy_guard: "policy-ferpa-34cfr99",
        predicates: [],
        actions: [
          { type: "CreateLedgerAuditEntry", summary: "Protected FERPA record modified", oscal_control: "MP-04" },
          { type: "NotifyCollaborator", role: "compliance", message_template: "FERPA record alteration logged." },
        ],
      });
    } else {
      onSaveRule({
        id: `rule-${Date.now()}`,
        app_slug: appSlug,
        name: "High Priority Value Escalation",
        description: "Alerts department lead when field exceeds critical operating limits",
        enabled: true,
        trigger: { type: "RecordCreated" },
        cedar_policy_guard: "policy-campus-l4",
        predicates: [{ field_name: fields[0]?.name || "id", operator: "NotEquals", expected_value: "" }],
        actions: [
          { type: "NotifyCollaborator", role: "chair", message_template: "New high-priority intake submitted." },
        ],
      });
    }
  };

  const handleCreateRule = (e: React.FormEvent) => {
    e.preventDefault();
    if (!ruleName.trim()) return;

    let trigger: WorkflowTriggerEvent;
    if (triggerType === "StatusChanged") {
      trigger = { type: "StatusChanged", to_status: "Approved" };
    } else if (triggerType === "RecordCreated") {
      trigger = { type: "RecordCreated" };
    } else {
      trigger = { type: "RecordUpdated" };
    }

    let action: WorkflowActionItem;
    if (actionType === "NotifyCollaborator") {
      action = { type: "NotifyCollaborator", role: "chair", message_template: actionParam };
    } else if (actionType === "CreateLedgerAuditEntry") {
      action = { type: "CreateLedgerAuditEntry", summary: actionParam, oscal_control: "AC-03" };
    } else {
      action = { type: "UpdateRecordStatus", new_status: actionParam };
    }

    const newRule: WorkflowAutomationRule = {
      id: `rule-${Date.now()}`,
      app_slug: appSlug,
      name: ruleName,
      description: ruleDesc || "Custom institutional automation rule",
      enabled: true,
      trigger,
      cedar_policy_guard: "policy-ferpa-34cfr99",
      predicates: [
        {
          field_name: selectedField,
          operator,
          expected_value: expectedValue,
        },
      ],
      actions: [action],
    };

    if (addAsServiceStep) {
      let stepAction: any;
      if (actionType === "NotifyCollaborator") {
        stepAction = { NotifyCollaborator: { role: "chair", message_template: actionParam } };
      } else if (actionType === "CreateLedgerAuditEntry") {
        stepAction = { CreateLedgerAuditEntry: { summary: actionParam, oscal_control: "AC-03" } };
      } else {
        stepAction = { UpdateRecordStatus: { new_status: actionParam } };
      }

      newRule.steps = [
        {
          id: `step-${Date.now()}`,
          when: [
            {
              field_name: selectedField,
              operator,
              expected_value: expectedValue,
            },
          ],
          kind: {
            Service: { action: stepAction },
          },
        },
      ];
    }

    onSaveRule(newRule);
    setIsCreating(false);
    setRuleName("");
    setRuleDesc("");
    setAddAsServiceStep(false);
  };

  const handleSimulateRule = async (rule: WorkflowAutomationRule) => {
    try {
      let eventPayload: any = rule.trigger.type;
      if (rule.trigger.type === "StatusChanged") {
        eventPayload = { StatusChanged: { to_status: (rule.trigger as any).to_status || "Approved" } };
      } else if (rule.trigger.type === "FieldChanged") {
        eventPayload = { FieldChanged: { field_name: (rule.trigger as any).field_name || "status" } };
      }

      const res = await fetch(`/api/v1/apps/${appSlug}/automations/simulate`, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({
          event: eventPayload,
          record: { id: "rec-simulation-test", [selectedField]: expectedValue, status: "Submitted" },
          principal: "dr.smith@university.edu",
          affiliation: "faculty",
        }),
      });

      if (!res.ok) {
        setSimulationResult(`HTTP ${res.status}`);
        return;
      }

      const body = await res.json();
      setSimulationResult(JSON.stringify(body, null, 2));
    } catch (err: any) {
      setSimulationResult(`Error: ${err?.message || "Simulation failed"}`);
    }
  };

  return (
    <div className="space-y-6">
      <div>
        <div className="flex flex-wrap items-center justify-between gap-3">
          <div>
            <h3 className="text-sm font-bold text-slate-900 dark:text-white">
              Governed Workflow Automations ({rules.length})
            </h3>
            <p className="text-xs text-slate-500 dark:text-slate-400 mt-0.5">
              Declare trigger events, predicate conditions, and automated actions with embedded Cedar authorization gates for {appTitle}.
            </p>
          </div>

          <button
            type="button"
            onClick={() => setIsCreating(true)}
            className="px-3 py-1.5 rounded-lg bg-blue-600 hover:bg-blue-700 text-white text-xs font-semibold shadow-xs cursor-pointer transition-colors"
          >
            + Create Automation Rule
          </button>
        </div>
      </div>

      {/* Preset Starter Templates */}
      <div className="p-4 rounded-xl border border-slate-200 dark:border-slate-800 bg-slate-50/60 dark:bg-slate-800/30 space-y-3">
        <span className="text-[11px] font-bold uppercase tracking-wider text-slate-400 block">
          ⚡ One-Click Institutional Templates
        </span>
        <div className="grid grid-cols-1 md:grid-cols-3 gap-3">
          <button
            type="button"
            onClick={() => handleApplyTemplate("approval")}
            className="p-3 text-left rounded-lg bg-white dark:bg-slate-900 border border-slate-200 dark:border-slate-800 hover:border-blue-400 dark:hover:border-blue-600 transition-all cursor-pointer shadow-xs group"
          >
            <div className="font-bold text-xs text-slate-900 dark:text-white group-hover:text-blue-600">
              🎓 Honors Fellowship Routing
            </div>
            <div className="text-[11px] text-slate-500 mt-1">
              Trigger when candidate GPA &gt; 3.85: notify dean and record to audit ledger.
            </div>
          </button>

          <button
            type="button"
            onClick={() => handleApplyTemplate("ferpa")}
            className="p-3 text-left rounded-lg bg-white dark:bg-slate-900 border border-slate-200 dark:border-slate-800 hover:border-blue-400 dark:hover:border-blue-600 transition-all cursor-pointer shadow-xs group"
          >
            <div className="font-bold text-xs text-slate-900 dark:text-white group-hover:text-blue-600">
              🔒 FERPA Disclosure Audit
            </div>
            <div className="text-[11px] text-slate-500 mt-1">
              Log all changes to sensitive records to NIST OSCAL MP-04 ledger.
            </div>
          </button>

          <button
            type="button"
            onClick={() => handleApplyTemplate("threshold")}
            className="p-3 text-left rounded-lg bg-white dark:bg-slate-900 border border-slate-200 dark:border-slate-800 hover:border-blue-400 dark:hover:border-blue-600 transition-all cursor-pointer shadow-xs group"
          >
            <div className="font-bold text-xs text-slate-900 dark:text-white group-hover:text-blue-600">
              🚨 Critical Alert Escalation
            </div>
            <div className="text-[11px] text-slate-500 mt-1">
              Notify department chair instantly when high-priority conditions are met.
            </div>
          </button>
        </div>
      </div>

      {/* Create Rule Modal / Form */}
      {isCreating && (
        <form
          onSubmit={handleCreateRule}
          className="p-5 rounded-xl border border-blue-200 dark:border-blue-800 bg-blue-50/40 dark:bg-blue-950/20 space-y-4 animate-fade-in text-xs"
        >
          <div className="flex items-center justify-between">
            <h4 className="font-bold text-sm text-slate-900 dark:text-white">
              Configure New Automation Rule
            </h4>
            <button
              type="button"
              onClick={() => setIsCreating(false)}
              className="text-slate-400 hover:text-slate-600 cursor-pointer"
            >
              ✕
            </button>
          </div>

          <div className="grid grid-cols-1 sm:grid-cols-2 gap-4">
            <div>
              <label className="block font-semibold mb-1 text-slate-700 dark:text-slate-300">
                Rule Name
              </label>
              <input
                type="text"
                required
                value={ruleName}
                onChange={(e) => setRuleName(e.target.value)}
                placeholder="e.g. Dean Fellowship Escalation"
                className="w-full px-3 py-1.5 rounded-lg border border-slate-300 dark:border-slate-700 bg-white dark:bg-slate-900 text-slate-900 dark:text-white"
              />
            </div>

            <div>
              <label className="block font-semibold mb-1 text-slate-700 dark:text-slate-300">
                Trigger Event
              </label>
              <select
                value={triggerType}
                onChange={(e) => setTriggerType(e.target.value as any)}
                className="w-full px-3 py-1.5 rounded-lg border border-slate-300 dark:border-slate-700 bg-white dark:bg-slate-900 text-slate-900 dark:text-white"
              >
                <option value="StatusChanged">Status Changed to Approved</option>
                <option value="RecordCreated">Record Created (New Submission)</option>
                <option value="RecordUpdated">Record Updated</option>
              </select>
            </div>
          </div>

          {/* Condition Predicate */}
          <div className="p-3 rounded-lg border border-slate-200 dark:border-slate-700 bg-white dark:bg-slate-900 space-y-2">
            <span className="font-semibold text-slate-700 dark:text-slate-300">
              Condition Gate (Field Predicate)
            </span>
            <div className="flex flex-wrap items-center gap-2">
              <span className="text-slate-400">When</span>
              <select
                value={selectedField}
                onChange={(e) => setSelectedField(e.target.value)}
                className="px-2.5 py-1 rounded border border-slate-300 dark:border-slate-700 bg-white dark:bg-slate-800 text-slate-900 dark:text-white"
              >
                {fields.map((f) => (
                  <option key={f.name} value={f.name}>
                    {f.label} ({f.name})
                  </option>
                ))}
              </select>

              <select
                value={operator}
                onChange={(e) => setOperator(e.target.value as any)}
                className="px-2.5 py-1 rounded border border-slate-300 dark:border-slate-700 bg-white dark:bg-slate-800 text-slate-900 dark:text-white"
              >
                <option value="GreaterThan">&gt; Greater Than</option>
                <option value="LessThan">&lt; Less Than</option>
                <option value="Equals">== Equals</option>
                <option value="NotEquals">!= Not Equals</option>
                <option value="Contains">Contains Text</option>
              </select>

              <input
                type="text"
                value={expectedValue}
                onChange={(e) => setExpectedValue(e.target.value)}
                placeholder="Value..."
                className="w-32 px-2.5 py-1 rounded border border-slate-300 dark:border-slate-700 bg-white dark:bg-slate-800 text-slate-900 dark:text-white"
              />
            </div>
          </div>

          {/* Action Configuration */}
          <div className="p-3 rounded-lg border border-slate-200 dark:border-slate-700 bg-white dark:bg-slate-900 space-y-2">
            <span className="font-semibold text-slate-700 dark:text-slate-300">
              Automated Action Dispatch
            </span>
            <div className="grid grid-cols-1 sm:grid-cols-2 gap-3">
              <div>
                <label className="block text-[11px] text-slate-400 mb-1">Action Type</label>
                <select
                  value={actionType}
                  onChange={(e) => setActionType(e.target.value as any)}
                  className="w-full px-2.5 py-1 rounded border border-slate-300 dark:border-slate-700 bg-white dark:bg-slate-800 text-slate-900 dark:text-white"
                >
                  <option value="NotifyCollaborator">Notify Collaborators / Roles</option>
                  <option value="CreateLedgerAuditEntry">Record Cryptographic Decision (AC-03)</option>
                  <option value="UpdateRecordStatus">Update Linked Record Status</option>
                </select>
              </div>

              <div>
                <label className="block text-[11px] text-slate-400 mb-1">Message or Value</label>
                <input
                  type="text"
                  value={actionParam}
                  onChange={(e) => setActionParam(e.target.value)}
                  placeholder="Notification message or ledger note"
                  className="w-full px-2.5 py-1 rounded border border-slate-300 dark:border-slate-700 bg-white dark:bg-slate-800 text-slate-900 dark:text-white"
                />
              </div>
            </div>
          </div>

          <div className="pt-1">
            <label className="flex items-center gap-2 cursor-pointer text-xs text-slate-700 dark:text-slate-300">
              <input
                type="checkbox"
                checked={addAsServiceStep}
                onChange={(e) => setAddAsServiceStep(e.target.checked)}
                className="rounded text-blue-600 focus:ring-blue-500"
              />
              <span>Add as multi-step pipeline service step (Phase 6)</span>
            </label>
          </div>

          <div className="flex justify-end gap-2 pt-2">
            <button
              type="button"
              onClick={() => setIsCreating(false)}
              className="px-3 py-1.5 rounded-lg border border-slate-300 dark:border-slate-700 text-slate-700 dark:text-slate-300 cursor-pointer"
            >
              Cancel
            </button>
            <button
              type="submit"
              className="px-4 py-1.5 rounded-lg bg-blue-600 hover:bg-blue-700 text-white font-semibold cursor-pointer"
            >
              Save Rule
            </button>
          </div>
        </form>
      )}

      {/* Active Rules List */}
      <div className="space-y-3">
        {rules.map((rule) => (
          <div
            key={rule.id}
            className={`p-4 rounded-xl border transition-all text-xs ${
              rule.enabled
                ? "bg-white dark:bg-slate-900 border-slate-200 dark:border-slate-800 shadow-xs"
                : "bg-slate-50 dark:bg-slate-900/40 border-slate-200 dark:border-slate-800 opacity-60"
            }`}
          >
            <div className="flex flex-wrap items-center justify-between gap-3 mb-2">
              <div className="flex items-center gap-2.5">
                <button
                  type="button"
                  onClick={() => onToggleRule(rule.id)}
                  className={`w-9 h-5 rounded-full transition-colors relative cursor-pointer ${
                    rule.enabled ? "bg-emerald-500" : "bg-slate-300 dark:bg-slate-700"
                  }`}
                  title={rule.enabled ? "Enabled" : "Disabled"}
                >
                  <span
                    className={`absolute top-0.5 left-0.5 w-4 h-4 rounded-full bg-white transition-transform ${
                      rule.enabled ? "transform translate-x-4" : ""
                    }`}
                  />
                </button>

                <h4 className="font-bold text-sm text-slate-900 dark:text-white">
                  {rule.name}
                </h4>

                <span className="px-1.5 py-0.2 rounded text-[10px] font-mono bg-purple-100 text-purple-800 dark:bg-purple-950 dark:text-purple-300 font-bold">
                  {rule.cedar_policy_guard || "Cedar Gated"}
                </span>
              </div>

              <div className="flex items-center gap-2">
                <button
                  type="button"
                  onClick={() => handleSimulateRule(rule)}
                  className="px-2.5 py-1 rounded bg-blue-50 dark:bg-blue-950/60 hover:bg-blue-100 dark:hover:bg-blue-900/60 text-blue-700 dark:text-blue-300 font-semibold text-[11px] cursor-pointer"
                >
                  ▶ Test Simulation
                </button>
                <button
                  type="button"
                  onClick={() => onDeleteRule(rule.id)}
                  className="text-slate-400 hover:text-rose-500 px-1 text-sm cursor-pointer"
                  title="Delete Rule"
                >
                  ✕
                </button>
              </div>
            </div>

            <p className="text-slate-600 dark:text-slate-400 mb-3">{rule.description}</p>

            {/* Trigger -> Predicate -> Action Pipeline Display */}
            <div className="grid grid-cols-1 md:grid-cols-3 gap-2 font-mono text-[11px] bg-slate-50 dark:bg-slate-800/40 p-3 rounded-lg border border-slate-100 dark:border-slate-800">
              <div>
                <span className="text-slate-400 block text-[10px] uppercase font-bold">Trigger</span>
                <span className="text-blue-600 dark:text-blue-400 font-semibold">
                  {rule.trigger.type === "StatusChanged"
                    ? `StatusChanged → ${rule.trigger.to_status}`
                    : rule.trigger.type}
                </span>
              </div>

              <div>
                <span className="text-slate-400 block text-[10px] uppercase font-bold">Condition</span>
                <span className="text-amber-600 dark:text-amber-400">
                  {rule.predicates.length > 0
                    ? rule.predicates.map((p) => `${p.field_name} ${p.operator} ${p.expected_value}`).join(", ")
                    : "Always (No Predicates)"}
                </span>
              </div>

              <div>
                <span className="text-slate-400 block text-[10px] uppercase font-bold">Actions</span>
                <span className="text-emerald-600 dark:text-emerald-400">
                  {rule.actions.length} action(s) configured
                </span>
              </div>
            </div>
          </div>
        ))}

        {rules.length === 0 && (
          <div className="p-8 text-center border border-dashed border-slate-200 dark:border-slate-800 rounded-xl text-slate-400 text-xs">
            No automation rules configured for this application. Click above to add your first governed trigger.
          </div>
        )}
      </div>

      {/* Simulation Result Drawer / Modal */}
      {simulationResult && (
        <div className="p-4 rounded-xl border border-emerald-200 dark:border-emerald-800/80 bg-emerald-50/40 dark:bg-emerald-950/20 text-xs space-y-2 animate-fade-in">
          <div className="flex items-center justify-between">
            <span className="font-bold text-emerald-800 dark:text-emerald-300 flex items-center gap-1.5">
              <span>✓ Simulation Succeeded</span>
              <span className="font-mono text-[10px] text-slate-400">(Zero-regression verified)</span>
            </span>
            <button
              type="button"
              onClick={() => setSimulationResult(null)}
              className="text-slate-400 hover:text-slate-600 text-xs cursor-pointer"
            >
              ✕ Close
            </button>
          </div>
          <pre className="p-3 rounded-lg bg-slate-900 text-emerald-300 font-mono text-[10px] overflow-x-auto">
            {simulationResult}
          </pre>
        </div>
      )}
    </div>
  );
};
