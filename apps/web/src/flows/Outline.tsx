// Workflows phase 7: the outline editor. A nested list of a flow's steps, each with its form, and every operation as a
// button and a key. It dispatches the commands of `flows/reducer.ts` and nothing else. The server checks the flow 400 ms
// after the last edit, and the outline shows what it says at each step. The browser never decides whether a flow is
// valid.
//
// Keys on a step: Alt+ArrowUp and Alt+ArrowDown move it, Alt+ArrowRight moves it into the branch above, Alt+ArrowLeft
// moves it out of its branch, Alt+Enter adds a step after it, and Delete removes it. Ctrl+Z undoes and Ctrl+Shift+Z
// redoes.

import React, { useEffect, useReducer, useRef, useState } from "react";
import { apiClient, type FlowCheck } from "../api";
import type { ApproverSpec, FieldSpec, ProcessStepItem, WorkflowAutomationRule, WorkflowPredicate, WorkflowTriggerEvent } from "../types";
import { WhoDecides } from "../WhoDecides";
import { stepPreview, type DecisionChoice, type LateChoice } from "../processWords";
import { flowReducer, initialEditor, newStep, stepAt, toServerFlow, type Command, type EditorState, type Path, type StepKindName } from "./reducer";

export type Problem = FlowCheck["problems"][number];

/** How long after the last edit the server checks the flow. */
export const VALIDATE_DELAY_MS = 400;

const input = "w-full px-2.5 py-1 rounded border border-slate-300 dark:border-slate-700 bg-white dark:bg-slate-800 text-slate-900 dark:text-white";
const small = "px-2 py-0.5 rounded border border-slate-300 dark:border-slate-700 text-[11px] text-slate-700 dark:text-slate-300 hover:bg-slate-100 dark:hover:bg-slate-800 cursor-pointer disabled:opacity-40 disabled:cursor-default";

/** A step's number in the outline: `2`, or `2.1.1` for the first step of the first condition of step 2. */
export const numberOf = (path: Path) => path.map((n) => n + 1).join(".");

const samePath = (a?: number[] | null, b?: number[]) => !!a && !!b && a.length === b.length && a.every((x, i) => x === b[i]);

const ADD_KINDS: { kind: StepKindName; label: string }[] = [
  { kind: "status", label: "Add a status step" },
  { kind: "notify", label: "Add a notification step" },
  { kind: "audit", label: "Add an audit entry step" },
  { kind: "task", label: "Add an approval step" },
  { kind: "branch", label: "Add a branch" },
  { kind: "create", label: "Add a create-record step" },
  { kind: "update", label: "Add an update-record step" },
  { kind: "find", label: "Add a find-records step" },
  { kind: "start", label: "Add a step that starts another flow" },
];

const KIND_NAMES: { kind: StepKindName; label: string }[] = [
  { kind: "status", label: "Set the status" },
  { kind: "notify", label: "Notify a role" },
  { kind: "audit", label: "Record an audit entry" },
  { kind: "task", label: "Ask for a decision" },
  { kind: "create", label: "Create a record" },
  { kind: "update", label: "Update a record" },
  { kind: "find", label: "Find records" },
  { kind: "start", label: "Start another flow" },
];

const OPERATORS: { value: WorkflowPredicate["operator"]; label: string }[] = [
  { value: "Equals", label: "is" },
  { value: "NotEquals", label: "is not" },
  { value: "GreaterThan", label: "is greater than" },
  { value: "LessThan", label: "is less than" },
  { value: "Contains", label: "contains" },
];

const clone = <T,>(v: T): T => JSON.parse(JSON.stringify(v));

/** The kind name of a step, as the add buttons and the kind choice name it. */
export function kindName(s: ProcessStepItem): StepKindName | "branch" | "other" {
  const k = s.kind as any;
  if (k.Branch) return "branch";
  if (k.UserTask) return "task";
  const a = k.Service?.action ?? {};
  if (a.UpdateRecordStatus) return "status";
  if (a.NotifyCollaborator) return "notify";
  if (a.CreateLedgerAuditEntry) return "audit";
  if (a.CreateRecord) return "create";
  if (a.UpdateRecord) return "update";
  if (a.FindRecord) return "find";
  if (a.StartFlow) return "start";
  return "other";
}

/** Where a step sits, so its status field is named as the old builder named it. */
export type Place = { arm: "top" } | { arm: "condition"; index: number } | { arm: "otherwise" };

/** Where the step at `path` sits: at the top, in a condition's arm, or in `otherwise`. */
export function placeOf(flow: WorkflowAutomationRule, path: Path): Place {
  if (path.length < 3) return { arm: "top" };
  const parent = stepAt(flow, path.slice(0, -2));
  const b = (parent?.kind as any)?.Branch;
  const arm = path[path.length - 2];
  return b && arm === b.branches.length ? { arm: "otherwise" } : { arm: "condition", index: arm };
}

function statusLabel(place: Place): string {
  if (place.arm === "condition") return place.index === 0 ? "Then set status to" : `Then set status to, condition ${place.index + 1}`;
  if (place.arm === "otherwise") return "Otherwise set status to";
  return "Set status to";
}

interface OutlineProps {
  state: EditorState;
  dispatch: (c: Command) => void;
  fields: FieldSpec[];
  appSlug: string;
  problems: Problem[];
  /** The server's answer, when there is one: what the flow may touch, who approves it, and the flow in sentences. */
  check?: FlowCheck | null;
}

export function Outline({ state, dispatch, fields, appSlug, problems, check }: OutlineProps) {
  const steps = state.flow.steps ?? [];
  const onKeys = (e: React.KeyboardEvent) => {
    const t = e.target as HTMLElement;
    if (["INPUT", "TEXTAREA", "SELECT"].includes(t.tagName)) return;
    if (!(e.ctrlKey || e.metaKey)) return;
    const key = e.key.toLowerCase();
    if (key === "z" && !e.shiftKey) {
      e.preventDefault();
      dispatch({ type: "Undo" });
    } else if ((key === "z" && e.shiftKey) || key === "y") {
      e.preventDefault();
      dispatch({ type: "Redo" });
    }
  };
  const located = problems.filter((p) => p.path && p.path.length);
  const summary =
    problems.length === 0
      ? ""
      : `${problems.length} ${problems.length === 1 ? "problem" : "problems"}. ` +
        problems.map((p) => (p.path && p.path.length ? `Step ${numberOf(p.path)}: ${p.message}` : `The flow: ${p.message}`)).join(" ");

  return (
    <section role="region" aria-label="Steps" onKeyDown={onKeys} className="space-y-3 text-xs">
      <div className="flex flex-wrap items-end gap-3">
        <Trigger trigger={state.flow.trigger} fields={fields} dispatch={dispatch} />
        <div className="flex gap-1 ml-auto">
          <button type="button" className={small} disabled={state.past.length === 0} onClick={() => dispatch({ type: "Undo" })}>
            Undo
          </button>
          <button type="button" className={small} disabled={state.future.length === 0} onClick={() => dispatch({ type: "Redo" })}>
            Redo
          </button>
        </div>
      </div>

      <p role="status" aria-label="Check result" className={summary ? "text-rose-700 dark:text-rose-300" : "sr-only"}>
        {summary}
      </p>
      {state.notice && (
        <p role="status" aria-label="Editor notice" className="text-amber-700 dark:text-amber-300">
          {state.notice}
        </p>
      )}
      {problems.filter((p) => !p.path || !p.path.length).map((p, i) => (
        <p key={i} className="text-rose-700 dark:text-rose-300">
          {p.message}
        </p>
      ))}

      {steps.length === 0 ? (
        <p className="text-slate-500">This flow has no steps yet.</p>
      ) : (
        <StepList list={steps} parent={[]} place={{ arm: "top" }} ctx={{ dispatch, fields, appSlug, problems: located, flow: state.flow }} />
      )}

      <div className="flex flex-wrap gap-1.5 pt-1">
        {ADD_KINDS.map(({ kind, label }) => (
          <button key={kind} type="button" aria-label={label} className={small} onClick={() => dispatch({ type: "AddStep", path: [steps.length], kind })}>
            + {label.replace(/^Add /, "")}
          </button>
        ))}
      </div>

      {check && (check.grants || check.description.length > 0) && (
        <div className="rounded-lg border border-slate-200 dark:border-slate-700 bg-slate-50 dark:bg-slate-800/40 p-2 space-y-1" data-testid="flow-check">
          {check.description.map((d, i) => (
            <p key={i}>{d}</p>
          ))}
          {check.grants && <p>{check.grants}</p>}
          {check.parties.length > 1 && <p>Each of these workspaces approves it: {check.parties.join(", ")}.</p>}
        </div>
      )}
    </section>
  );
}

function Trigger({ trigger, fields, dispatch }: { trigger: WorkflowTriggerEvent; fields: FieldSpec[]; dispatch: (c: Command) => void }) {
  const set = (t: WorkflowTriggerEvent) => dispatch({ type: "SetTrigger", trigger: t });
  const choose = (type: string) => {
    if (type === "StatusChanged") set({ type, to_status: "Approved" });
    else if (type === "FieldChanged") set({ type, field_name: fields[0]?.name ?? "" });
    else set({ type: type as "RecordCreated" | "RecordUpdated" });
  };
  return (
    <div className="flex flex-wrap items-end gap-2">
      <label className="block">
        <span className="block text-[11px] text-slate-500 mb-1">Starts when</span>
        <select aria-label="Starts when" value={trigger.type} onChange={(e) => choose(e.target.value)} className={input}>
          <option value="RecordCreated">A record is created</option>
          <option value="RecordUpdated">A record is updated</option>
          <option value="StatusChanged">The status changes</option>
          <option value="FieldChanged">A field changes</option>
        </select>
      </label>
      {trigger.type === "StatusChanged" && (
        <label className="block">
          <span className="block text-[11px] text-slate-500 mb-1">Status it changes to</span>
          <input aria-label="Status it changes to" value={trigger.to_status} onChange={(e) => set({ type: "StatusChanged", to_status: e.target.value })} className={input} />
        </label>
      )}
      {trigger.type === "FieldChanged" && (
        <label className="block">
          <span className="block text-[11px] text-slate-500 mb-1">Field that changes</span>
          <select aria-label="Field that changes" value={trigger.field_name} onChange={(e) => set({ type: "FieldChanged", field_name: e.target.value })} className={input}>
            {fields.map((f) => (
              <option key={f.name} value={f.name}>
                {f.label || f.name}
              </option>
            ))}
          </select>
        </label>
      )}
    </div>
  );
}

export interface Ctx {
  dispatch: (c: Command) => void;
  fields: FieldSpec[];
  appSlug: string;
  problems: Problem[];
  flow: WorkflowAutomationRule;
}

function StepList({ list, parent, place, ctx }: { list: ProcessStepItem[]; parent: Path; place: Place; ctx: Ctx }) {
  return (
    <ol className="space-y-2">
      {list.map((s, i) => (
        <StepItem key={s.id} step={s} path={[...parent, i]} index={i} siblings={list} place={place} ctx={ctx} />
      ))}
    </ol>
  );
}

function StepItem({ step, path, index, siblings, place, ctx }: { step: ProcessStepItem; path: Path; index: number; siblings: ProcessStepItem[]; place: Place; ctx: Ctx }) {
  const { dispatch } = ctx;
  const n = numberOf(path);
  const parent = path.slice(0, -1);
  const above = siblings[index - 1];
  const aboveBranch = above && (above.kind as any).Branch ? ((above.kind as any).Branch as { branches: { steps: ProcessStepItem[] }[] }) : null;
  const nested = path.length > 1;

  const addAfter = () => dispatch({ type: "AddStep", path: [...parent, index + 1], kind: "status" });
  const remove = () => dispatch({ type: "RemoveStep", path });
  const up = () => dispatch({ type: "MoveStep", from: path, to: [...parent, index - 1] });
  const down = () => dispatch({ type: "MoveStep", from: path, to: [...parent, index + 2] });
  const indent = () => aboveBranch && dispatch({ type: "MoveStep", from: path, to: [...parent, index - 1, 0, aboveBranch.branches[0]?.steps.length ?? 0] });
  const outdent = () => dispatch({ type: "MoveStep", from: path, to: [...path.slice(0, -3), path[path.length - 3] + 1] });

  const onKey = (e: React.KeyboardEvent<HTMLLIElement>) => {
    if (e.target !== e.currentTarget) return;
    const act = (f: () => void) => {
      e.preventDefault();
      e.stopPropagation();
      f();
    };
    if (e.key === "Delete") act(remove);
    else if (e.altKey && e.key === "Enter") act(addAfter);
    else if (e.altKey && e.key === "ArrowUp" && index > 0) act(up);
    else if (e.altKey && e.key === "ArrowDown" && index < siblings.length - 1) act(down);
    else if (e.altKey && e.key === "ArrowRight" && aboveBranch) act(indent);
    else if (e.altKey && e.key === "ArrowLeft" && nested) act(outdent);
  };

  const mine = ctx.problems.filter((p) => samePath(p.path, path));
  return (
    <li
      aria-label={`step ${n}`}
      tabIndex={0}
      onKeyDown={onKey}
      className="rounded-lg border border-slate-200 dark:border-slate-700 bg-white dark:bg-slate-900 p-2.5 space-y-2 focus:outline-none focus:ring-2 focus:ring-blue-500"
    >
      <div className="flex flex-wrap items-center gap-1.5">
        <span className="font-mono text-[11px] text-slate-500">{n}</span>
        <span className="font-semibold text-slate-800 dark:text-slate-200">{sentenceOf(step)}</span>
        <span className="ml-auto flex flex-wrap gap-1">
          {index > 0 && (
            <button type="button" className={small} aria-label={`Move step ${n} up`} onClick={up}>
              ↑
            </button>
          )}
          {index < siblings.length - 1 && (
            <button type="button" className={small} aria-label={`Move step ${n} down`} onClick={down}>
              ↓
            </button>
          )}
          {aboveBranch && (
            <button type="button" className={small} aria-label={`Move step ${n} into the branch above`} onClick={indent}>
              → into branch
            </button>
          )}
          {nested && (
            <button type="button" className={small} aria-label={`Move step ${n} out of the branch`} onClick={outdent}>
              ← out of branch
            </button>
          )}
          <button type="button" className={small} aria-label={`Add a step after step ${n}`} onClick={addAfter}>
            + after
          </button>
          <button type="button" className={small} aria-label={`Remove step ${n}`} onClick={remove}>
            Remove
          </button>
        </span>
      </div>

      <StepForm step={step} path={path} place={place} ctx={ctx} />

      {mine.map((p, i) => (
        <p key={i} className="text-rose-700 dark:text-rose-300">
          <span aria-hidden="true">⚠ </span>
          <span>{p.message}</span>
        </p>
      ))}
    </li>
  );
}

/** What a step does, in a few words, for its line in the outline. */
export function sentenceOf(s: ProcessStepItem): string {
  const k = s.kind as any;
  const a = k.Service?.action ?? {};
  switch (kindName(s)) {
    case "status":
      return a.UpdateRecordStatus.new_status ? `Set the status to ${a.UpdateRecordStatus.new_status}` : "Set the status";
    case "notify":
      return a.NotifyCollaborator.role ? `Notify ${a.NotifyCollaborator.role}` : "Notify a role";
    case "audit":
      return "Record an audit entry";
    case "task":
      return "Ask for a decision";
    case "branch":
      return "Branch";
    case "create":
      return a.CreateRecord.table ? `Create a record in ${a.CreateRecord.table}` : "Create a record";
    case "update":
      return "Update a record";
    case "find":
      return a.FindRecord.table ? `Find records in ${a.FindRecord.table}` : "Find records";
    case "start":
      return a.StartFlow.flow_id ? `Start the flow ${a.StartFlow.flow_id}` : "Start another flow";
    default:
      return `A step that ${Object.keys(a)[0] ?? "does something"}`;
  }
}

export function StepForm({ step, path, place, ctx }: { step: ProcessStepItem; path: Path; place: Place; ctx: Ctx }) {
  const { dispatch } = ctx;
  const k = step.kind as any;
  const update = (kind: unknown) => dispatch({ type: "UpdateStep", path, patch: { kind: kind as any } });
  const setAction = (name: string, value: Record<string, unknown>) => update({ Service: { action: { [name]: value } } });
  const name = kindName(step);
  const kindChoice = name !== "branch" && (
    <label className="block">
      <span className="block text-[11px] text-slate-500 mb-1">Step kind</span>
      <select
        aria-label="Step kind"
        value={name === "other" ? "" : name}
        onChange={(e) => update(newStep(step.id, e.target.value as StepKindName, () => step.id).kind)}
        className={input}
      >
        {name === "other" && <option value="">Another kind</option>}
        {KIND_NAMES.map((o) => (
          <option key={o.kind} value={o.kind}>
            {o.label}
          </option>
        ))}
      </select>
    </label>
  );

  if (name === "branch") return <BranchForm step={step} path={path} ctx={ctx} />;
  if (name === "task") return <>{kindChoice}<TaskForm step={step} path={path} ctx={ctx} /></>;

  const a = k.Service?.action ?? {};
  let body: React.ReactNode = <p className="text-slate-500">This step&apos;s settings stay as they are.</p>;
  if (name === "status") {
    const label = statusLabel(place);
    body = (
      <label className="block">
        <span className="block text-[11px] text-slate-500 mb-1">{label}</span>
        <input aria-label={label} value={a.UpdateRecordStatus.new_status} onChange={(e) => setAction("UpdateRecordStatus", { new_status: e.target.value })} className={input} />
      </label>
    );
  } else if (name === "notify") {
    const v = a.NotifyCollaborator;
    body = (
      <div className="grid grid-cols-2 gap-2">
        <Text label="Notify role" value={v.role} onChange={(role) => setAction("NotifyCollaborator", { ...v, role })} />
        <Text label="Message" value={v.message_template} onChange={(message_template) => setAction("NotifyCollaborator", { ...v, message_template })} />
      </div>
    );
  } else if (name === "audit") {
    const v = a.CreateLedgerAuditEntry;
    body = (
      <div className="grid grid-cols-2 gap-2">
        <Text label="Audit summary" value={v.summary} onChange={(summary) => setAction("CreateLedgerAuditEntry", { ...v, summary })} />
        <Text label="Control" value={v.oscal_control} onChange={(oscal_control) => setAction("CreateLedgerAuditEntry", { ...v, oscal_control })} />
      </div>
    );
  } else if (name === "create") {
    const v = a.CreateRecord;
    body = (
      <div className="space-y-2">
        <div className="grid grid-cols-2 gap-2">
          <Text label="App" value={v.app} onChange={(app) => setAction("CreateRecord", { ...v, app })} />
          <Text label="Table" value={v.table} onChange={(table) => setAction("CreateRecord", { ...v, table })} />
        </div>
        <Pairs pairs={v.fields} onChange={(fields) => setAction("CreateRecord", { ...v, fields })} />
      </div>
    );
  } else if (name === "update") {
    const v = a.UpdateRecord;
    body = (
      <div className="space-y-2">
        <TargetChoice label="Which record" value={v.target} flow={ctx.flow} self={step.id} onChange={(target) => setAction("UpdateRecord", { ...v, target })} />
        <Pairs pairs={v.fields} onChange={(fields) => setAction("UpdateRecord", { ...v, fields })} />
      </div>
    );
  } else if (name === "start") {
    const v = a.StartFlow;
    body = <Text label="Flow to start" value={v.flow_id} onChange={(flow_id) => setAction("StartFlow", { ...v, flow_id })} />;
  } else if (name === "find") {
    const v = a.FindRecord;
    body = (
      <div className="grid grid-cols-2 gap-2">
        <Text label="App" value={v.app} onChange={(app) => setAction("FindRecord", { ...v, app })} />
        <Text label="Table" value={v.table} onChange={(table) => setAction("FindRecord", { ...v, table })} />
        <Text label="Match field" value={v.field} onChange={(field) => setAction("FindRecord", { ...v, field })} />
        <Text label="Equals" value={v.equals} onChange={(equals) => setAction("FindRecord", { ...v, equals })} />
        <label className="block">
          <span className="block text-[11px] text-slate-500 mb-1">At most</span>
          <input aria-label="At most" type="number" min={1} max={100} value={v.limit} onChange={(e) => setAction("FindRecord", { ...v, limit: Number(e.target.value) || 1 })} className={input} />
        </label>
      </div>
    );
  }
  return (
    <div className="space-y-2">
      {kindChoice}
      {body}
    </div>
  );
}

function Text({ label, value, onChange }: { label: string; value: string; onChange: (v: string) => void }) {
  return (
    <label className="block">
      <span className="block text-[11px] text-slate-500 mb-1">{label}</span>
      <input aria-label={label} value={value ?? ""} onChange={(e) => onChange(e.target.value)} className={input} />
    </label>
  );
}

/** Field names and the expressions written to them. */
function Pairs({ pairs, onChange }: { pairs: [string, string][]; onChange: (p: [string, string][]) => void }) {
  const set = (i: number, j: 0 | 1, v: string) => onChange(pairs.map((p, k) => (k === i ? ((j === 0 ? [v, p[1]] : [p[0], v]) as [string, string]) : p)));
  return (
    <div className="space-y-1">
      {pairs.map((p, i) => (
        <div key={i} className="grid grid-cols-[1fr_2fr_auto] gap-1.5 items-end">
          <Text label={`Field ${i + 1}`} value={p[0]} onChange={(v) => set(i, 0, v)} />
          <Text label={`Value of field ${i + 1}`} value={p[1]} onChange={(v) => set(i, 1, v)} />
          <button type="button" className={small} aria-label={`Remove field ${i + 1}`} onClick={() => onChange(pairs.filter((_, k) => k !== i))}>
            Remove
          </button>
        </div>
      ))}
      <button type="button" className={small} onClick={() => onChange([...pairs, ["", ""]])}>
        Add a field
      </button>
    </div>
  );
}

/** The record a step acts on: the trigger record, or one an earlier step created or found. */
function TargetChoice({ label, value, flow, self, onChange }: { label: string; value: any; flow: WorkflowAutomationRule; self: string; onChange: (t: unknown) => void }) {
  const options: { key: string; target: unknown; label: string }[] = [{ key: "Trigger", target: "Trigger", label: "The record that started the flow" }];
  const visit = (list: ProcessStepItem[]) => {
    for (const s of list) {
      const a = (s.kind as any).Service?.action ?? {};
      if (a.CreateRecord && s.id !== self) options.push({ key: `Created:${a.CreateRecord.id}`, target: { Created: { step: a.CreateRecord.id } }, label: `The record step ${a.CreateRecord.id} created` });
      if (a.FindRecord && s.id !== self) options.push({ key: `Found:${a.FindRecord.id}`, target: { Found: { step: a.FindRecord.id } }, label: `The records step ${a.FindRecord.id} found` });
      const b = (s.kind as any).Branch;
      if (b) [...b.branches.map((x: any) => x.steps), b.otherwise].forEach(visit);
    }
  };
  visit(flow.steps ?? []);
  const keyOf = (t: any) => (typeof t === "string" ? t : t?.Created ? `Created:${t.Created.step}` : t?.Found ? `Found:${t.Found.step}` : t?.Linked ? `Linked:${t.Linked.link_field}` : "");
  const current = keyOf(value);
  if (!options.some((o) => o.key === current)) options.push({ key: current, target: value, label: current.startsWith("Linked:") ? `The record linked by ${current.slice(7)}` : current });
  return (
    <label className="block">
      <span className="block text-[11px] text-slate-500 mb-1">{label}</span>
      <select aria-label={label} value={current} onChange={(e) => {
          const o = options.find((x) => x.key === e.target.value);
          if (o) onChange(clone(o.target));
        }} className={input}>
        {options.map((o) => (
          <option key={o.key} value={o.key}>
            {o.label}
          </option>
        ))}
      </select>
    </label>
  );
}

function TaskForm({ step, path, ctx }: { step: ProcessStepItem; path: Path; ctx: Ctx }) {
  const task = (step.kind as any).UserTask;
  const set = (next: Record<string, unknown>) => ctx.dispatch({ type: "UpdateStep", path, patch: { kind: { UserTask: next } as any } });
  const decision: DecisionChoice = task.decision ?? "Single";
  const agree = typeof decision === "object" ? "AtLeast" : decision;
  const dueDays: number = task.due?.days ?? 0;
  const firstDue = (task.on_due ?? [])[0];
  const late: LateChoice = !firstDue ? "" : typeof firstDue === "string" ? (firstDue as LateChoice) : firstDue.Fail ? "Fail" : "";
  const onDue = (days: number, l: LateChoice) => (l === "" ? [] : [l === "Fail" ? { Fail: { reason: `Nobody decided within ${days} days.` } } : l]);
  const timing = (days: number, l: LateChoice) => {
    const { due: _d, on_due: _o, ...rest } = task;
    return days > 0 ? { ...rest, due: { days }, on_due: onDue(days, l) } : rest;
  };
  const approver: ApproverSpec | undefined = task.approver;

  const onApprover = (spec: ApproverSpec | null) => {
    if (!spec && !approver) return;
    const { approver: _a, ...rest } = task;
    set(spec ? { ...rest, approver: spec, role: "Role" in spec ? spec.Role.name : "" } : { ...rest, role: "" });
  };
  const setDecision = (d: DecisionChoice) => {
    const { decision: _x, ...rest } = task;
    set(d === "Single" ? rest : { ...rest, decision: d });
  };

  return (
    <div className="space-y-2">
      <label className="block">
        <span className="block text-[11px] text-slate-500 mb-1">What to decide</span>
        <input aria-label="What to decide" value={task.prompt} onChange={(e) => set({ ...task, prompt: e.target.value })} className={input} />
      </label>
      <WhoDecides appSlug={ctx.appSlug} onChange={onApprover} />
      <div className="grid grid-cols-2 gap-2">
        <label className="block">
          <span className="block text-[11px] text-slate-500 mb-1">How many must agree</span>
          <select
            aria-label="How many must agree"
            value={agree}
            onChange={(e) => setDecision(e.target.value === "AtLeast" ? { AtLeast: typeof decision === "object" ? decision.AtLeast : 2 } : (e.target.value as "Single" | "AllOf"))}
            className={input}
          >
            <option value="Single">One person</option>
            <option value="AllOf">All of them</option>
            <option value="AtLeast">At least</option>
          </select>
        </label>
        {typeof decision === "object" && (
          <label className="block">
            <span className="block text-[11px] text-slate-500 mb-1">How many</span>
            <input aria-label="How many" type="number" min={1} value={decision.AtLeast} onChange={(e) => setDecision({ AtLeast: Math.max(1, Number(e.target.value) || 1) })} className={input} />
          </label>
        )}
        <label className="block">
          <span className="block text-[11px] text-slate-500 mb-1">Due after (days)</span>
          <input aria-label="Due after (days)" type="number" min={0} max={365} value={dueDays} onChange={(e) => set(timing(Math.max(0, Number(e.target.value) || 0), late))} className={input} />
        </label>
        <label className="block">
          <span className="block text-[11px] text-slate-500 mb-1">If it is late</span>
          <select aria-label="If it is late" value={late} onChange={(e) => set(timing(dueDays, e.target.value as LateChoice))} className={input}>
            <option value="">Nothing happens</option>
            <option value="Remind">Remind them</option>
            <option value="EscalateUp">Ask the next level</option>
            <option value="Fail">Fail the request</option>
          </select>
        </label>
      </div>
      {approver && (
        <p data-testid="step-preview" className="text-xs text-slate-700 dark:text-slate-300 bg-slate-50 dark:bg-slate-800/40 rounded px-2 py-1.5">
          {stepPreview(approver, decision, dueDays, late)}
        </p>
      )}
    </div>
  );
}

function BranchForm({ step, path, ctx }: { step: ProcessStepItem; path: Path; ctx: Ctx }) {
  const { dispatch, fields } = ctx;
  const b = (step.kind as any).Branch as { branches: { when: WorkflowPredicate[]; steps: ProcessStepItem[] }[]; otherwise: ProcessStepItem[] };
  const n = numberOf(path);
  const setCondition = (arm: number, pred: WorkflowPredicate) => {
    const next = clone(b);
    next.branches[arm].when = [pred, ...next.branches[arm].when.slice(1)];
    dispatch({ type: "UpdateStep", path, patch: { kind: { Branch: next } as any } });
  };
  return (
    <div className="space-y-2">
      {b.branches.map((arm, k) => {
        const pred: WorkflowPredicate = arm.when[0] ?? { field_name: "", operator: "Equals", expected_value: "" };
        const suffix = k === 0 ? "" : `, condition ${k + 1}`;
        return (
          <div key={k} data-testid={`outline-arm-${k}`} className="border-l-2 border-blue-300 dark:border-blue-700 pl-2 space-y-1.5">
            <div className="flex flex-wrap items-center gap-1.5">
              <span className="font-semibold">If (condition {k + 1})</span>
              <select aria-label={`Branch field${suffix}`} value={pred.field_name} onChange={(e) => setCondition(k, { ...pred, field_name: e.target.value })} className={`${input} w-auto`}>
                {pred.field_name === "" && <option value="">Choose a field</option>}
                {fields.map((f) => (
                  <option key={f.name} value={f.name}>
                    {f.label || f.name}
                  </option>
                ))}
                {pred.field_name !== "" && !fields.some((f) => f.name === pred.field_name) && <option value={pred.field_name}>{pred.field_name}</option>}
              </select>
              <select aria-label={`Branch test${suffix}`} value={pred.operator} onChange={(e) => setCondition(k, { ...pred, operator: e.target.value as WorkflowPredicate["operator"] })} className={`${input} w-auto`}>
                {OPERATORS.map((o) => (
                  <option key={o.value} value={o.value}>
                    {o.label}
                  </option>
                ))}
              </select>
              <input aria-label={`Branch value${suffix}`} value={pred.expected_value} onChange={(e) => setCondition(k, { ...pred, expected_value: e.target.value })} className={`${input} w-28`} />
              <span className="ml-auto flex gap-1">
                {k > 0 && (
                  <button
                    type="button"
                    className={small}
                    aria-label={`Move condition ${k + 1} of step ${n} up`}
                    onClick={() => dispatch({ type: "ReorderBranches", path, order: b.branches.map((_, i) => (i === k - 1 ? k : i === k ? k - 1 : i)) })}
                  >
                    ↑
                  </button>
                )}
                {b.branches.length > 1 && (
                  <button type="button" className={small} aria-label={`Remove condition ${k + 1} of step ${n}`} onClick={() => dispatch({ type: "RemoveBranch", path: [...path, k] })}>
                    Remove condition
                  </button>
                )}
              </span>
            </div>
            <StepList list={arm.steps} parent={[...path, k]} place={{ arm: "condition", index: k }} ctx={ctx} />
            <button type="button" className={small} aria-label={`Add a step to condition ${k + 1} of step ${n}`} onClick={() => dispatch({ type: "AddStep", path: [...path, k, arm.steps.length], kind: "status" })}>
              + step
            </button>
          </div>
        );
      })}
      <div data-testid="outline-otherwise" className="border-l-2 border-slate-300 dark:border-slate-600 pl-2 space-y-1.5">
        <span className="font-semibold">Otherwise</span>
        <StepList list={b.otherwise} parent={[...path, b.branches.length]} place={{ arm: "otherwise" }} ctx={ctx} />
        <button type="button" className={small} aria-label={`Add a step to otherwise of step ${n}`} onClick={() => dispatch({ type: "AddStep", path: [...path, b.branches.length, b.otherwise.length], kind: "status" })}>
          + step
        </button>
      </div>
      <button type="button" className={small} aria-label={`Add a condition to step ${n}`} onClick={() => dispatch({ type: "AddBranch", path })}>
        + condition
      </button>
    </div>
  );
}

/**
 * The editor's state, and the server's check of it. The flow is sent 400 ms after the last edit. An answer to an older
 * flow is dropped.
 */
export function useFlowEditor(initial: WorkflowAutomationRule) {
  const [state, dispatch] = useReducer(flowReducer, initial, initialEditor);
  const [check, setCheck] = useState<FlowCheck | null>(null);
  const first = useRef(state.flow);
  const edited = useRef(false);
  const seq = useRef(0);
  useEffect(() => {
    // Nothing is checked until the first edit. After that every state is, even one that Undo returns to the start.
    if (!edited.current && state.flow === first.current) return;
    edited.current = true;
    const mine = ++seq.current;
    const t = setTimeout(() => {
      apiClient
        .validateFlow(toServerFlow(state.flow))
        .then((c) => {
          if (seq.current === mine) setCheck({ problems: c?.problems ?? [], grants: c?.grants ?? null, parties: c?.parties ?? [], checks: c?.checks ?? [], description: c?.description ?? [] });
        })
        .catch((e: unknown) => {
          if (seq.current === mine) setCheck({ problems: [{ message: e instanceof Error ? e.message : "The server could not check the flow.", path: null }], grants: null, parties: [], checks: [], description: [] });
        });
    }, VALIDATE_DELAY_MS);
    return () => clearTimeout(t);
  }, [state.flow]);
  return { state, dispatch, check, problems: check?.problems ?? [] };
}

/** The outline with its own state, for a screen that only needs the editor. */
export function FlowEditor({ initial, fields, appSlug, onFlowChange }: { initial: WorkflowAutomationRule; fields: FieldSpec[]; appSlug: string; onFlowChange?: (f: WorkflowAutomationRule) => void }) {
  const ed = useFlowEditor(initial);
  useEffect(() => {
    onFlowChange?.(ed.state.flow);
  }, [ed.state.flow, onFlowChange]);
  return <Outline state={ed.state} dispatch={ed.dispatch} fields={fields} appSlug={appSlug} problems={ed.problems} check={ed.check} />;
}
