// Workflows phase 7: every edit to a flow is a command, and this pure reducer applies it. The outline (and later the
// canvas) dispatches these commands and nothing else. The state is the flow's JSON, with no layout.
//
// A path is a list of numbers. The first is a step's index in `steps`. When that step is a branch, the next number is
// the arm, with `otherwise` after the last condition, then the step's index in that arm, and so on. The server's
// `POST /flows/validate` reports problems with the same paths.
//
// The reducer refuses a command whose result would break the flow's structure: a step kind nobody knows, two steps
// with one id, a branch with no condition, or a step that names a step the flow no longer has. It does not decide
// whether the flow is valid. The server does.

import type { ProcessStepItem, WorkflowAutomationRule, WorkflowTriggerEvent } from "../types";

export type Path = number[];

export type StepKindName = "status" | "notify" | "audit" | "task" | "branch" | "create" | "update" | "find" | "start";

export type Command =
  | { type: "AddStep"; path: Path; kind: StepKindName }
  | { type: "RemoveStep"; path: Path }
  | { type: "MoveStep"; from: Path; to: Path }
  | { type: "UpdateStep"; path: Path; patch: Partial<Omit<ProcessStepItem, "id">> }
  | { type: "AddBranch"; path: Path }
  | { type: "RemoveBranch"; path: Path }
  | { type: "ReorderBranches"; path: Path; order: number[] }
  | { type: "SetTrigger"; trigger: WorkflowTriggerEvent }
  | { type: "Undo" }
  | { type: "Redo" };

export interface EditorState {
  flow: WorkflowAutomationRule;
  past: WorkflowAutomationRule[];
  future: WorkflowAutomationRule[];
  /** Why the last command changed nothing, in words. Empty after a command that applied. */
  notice: string;
}

/** How many changes Undo can take back. */
export const UNDO_LIMIT = 100;

const KNOWN_KINDS = ["Service", "UserTask", "Branch"];

export function initialEditor(flow: WorkflowAutomationRule): EditorState {
  return { flow: { ...flow, steps: flow.steps ?? [] }, past: [], future: [], notice: "" };
}

interface BranchKind {
  branches: { when: any[]; steps: ProcessStepItem[] }[];
  otherwise: ProcessStepItem[];
}

const branchOf = (s: ProcessStepItem | undefined): BranchKind | undefined => (s?.kind as any)?.Branch;

/** The arms of a branch in path order: each condition's steps, then `otherwise`. */
const armsOf = (b: BranchKind): ProcessStepItem[][] => [...b.branches.map((a) => a.steps), b.otherwise];

/** The list that holds the step at `path` (or that `path` inserts into), and the index in it. */
function locate(steps: ProcessStepItem[], path: Path): { list: ProcessStepItem[]; index: number } | undefined {
  if (path.length === 0 || path.length % 2 === 0) return undefined;
  let list = steps;
  for (let d = 0; d < path.length - 1; d += 2) {
    const b = branchOf(list[path[d]]);
    if (!b) return undefined;
    const arm = armsOf(b)[path[d + 1]];
    if (!arm) return undefined;
    list = arm;
  }
  return { list, index: path[path.length - 1] };
}

export function stepAt(flow: WorkflowAutomationRule, path: Path): ProcessStepItem | undefined {
  const at = locate(flow.steps ?? [], path);
  return at ? at.list[at.index] : undefined;
}

function walk(steps: ProcessStepItem[], f: (s: ProcessStepItem) => void) {
  for (const s of steps) {
    f(s);
    const b = branchOf(s);
    if (b) armsOf(b).forEach((arm) => walk(arm, f));
  }
}

/** The ids later steps may name: every step's id, and the id a finding or creating action gives what it returns. */
function producedIds(steps: ProcessStepItem[]): Set<string> {
  const out = new Set<string>();
  walk(steps, (s) => {
    out.add(s.id);
    const action = (s.kind as any)?.Service?.action;
    const made = action?.CreateRecord ?? action?.FindRecord;
    if (typeof made?.id === "string") out.add(made.id);
  });
  return out;
}

/** The steps a step names: a `Created` or `Found` target, and the scope of a `{scope.field}` reference. */
function namedBy(s: ProcessStepItem): string[] {
  const out: string[] = [];
  const visit = (v: unknown, key?: string) => {
    if (typeof v === "string") {
      for (const m of v.matchAll(/\{([A-Za-z0-9_-]+)\.[A-Za-z0-9_]+\}/g)) if (!["trigger", "run", "input"].includes(m[1])) out.push(m[1]);
    } else if (Array.isArray(v)) {
      v.forEach((x) => visit(x));
    } else if (v && typeof v === "object") {
      if ((key === "Created" || key === "Found") && typeof (v as any).step === "string") out.push((v as any).step);
      for (const [k, x] of Object.entries(v)) if (!(k === "Branch")) visit(x, k);
    }
  };
  // A branch's own conditions; its arms are steps of their own.
  const b = branchOf(s);
  visit(b ? { when: s.when, conditions: b.branches.map((a) => a.when) } : { when: s.when, kind: s.kind });
  return out;
}

/** What breaks the structure of a flow, in words. Empty means the structure holds. */
export function structureProblems(flow: WorkflowAutomationRule): string[] {
  const steps = flow.steps ?? [];
  const out: string[] = [];
  const seen = new Set<string>();
  const produced = producedIds(steps);
  walk(steps, (s) => {
    const kind = Object.keys(s.kind ?? {})[0];
    if (!KNOWN_KINDS.includes(kind)) out.push(`Step ${s.id} has a kind the editor does not know.`);
    if (seen.has(s.id)) out.push(`Two steps have the id ${s.id}.`);
    seen.add(s.id);
    const b = branchOf(s);
    if (b && b.branches.length === 0) out.push(`The branch ${s.id} needs at least one condition.`);
    for (const n of namedBy(s)) if (!produced.has(n)) out.push(`Step ${s.id} names ${n}, which is not a step of this flow.`);
  });
  return out;
}

function nextId(flow: WorkflowAutomationRule): string {
  let n = 0;
  for (const id of producedIds(flow.steps ?? [])) {
    const m = /^step-(\d+)$/.exec(id);
    if (m) n = Math.max(n, Number(m[1]));
  }
  return `step-${n + 1}`;
}

const service = (action: Record<string, unknown>) => ({ Service: { action } });

/** A new step of a kind, with empty settings. The server says what is missing. */
export function newStep(id: string, kind: StepKindName, nextIdAfter: () => string): ProcessStepItem {
  const status = (sid: string): ProcessStepItem => ({ id: sid, when: [], kind: service({ UpdateRecordStatus: { new_status: "" } }) as any });
  switch (kind) {
    case "status":
      return status(id);
    case "notify":
      return { id, when: [], kind: service({ NotifyCollaborator: { role: "", message_template: "" } }) as any };
    case "audit":
      return { id, when: [], kind: service({ CreateLedgerAuditEntry: { summary: "", oscal_control: "AC-03" } }) as any };
    case "task":
      return {
        id,
        when: [],
        kind: { UserTask: { role: "", prompt: "Review and decide", approve: [{ UpdateRecordStatus: { new_status: "Approved" } }], reject: [{ UpdateRecordStatus: { new_status: "Rejected" } }] } },
      };
    case "branch": {
      const then = nextIdAfter();
      const otherwise = nextIdAfter();
      return {
        id,
        when: [],
        kind: { Branch: { branches: [{ when: [{ field_name: "", operator: "Equals", expected_value: "" }], steps: [status(then)] }], otherwise: [status(otherwise)] } },
      };
    }
    case "create":
      return { id, when: [], kind: service({ CreateRecord: { id, app: "", table: "", fields: [] } }) as any };
    case "update":
      return { id, when: [], kind: service({ UpdateRecord: { target: "Trigger", fields: [] } }) as any };
    case "find":
      return { id, when: [], kind: service({ FindRecord: { id, app: "", table: "", field: "", equals: "", limit: 1 } }) as any };
    case "start":
      return { id, when: [], kind: service({ StartFlow: { flow_id: "", inputs: [] } }) as any };
  }
}

/** The flow after `cmd`, or a sentence saying why it cannot apply. */
function apply(flow: WorkflowAutomationRule, cmd: Exclude<Command, { type: "Undo" } | { type: "Redo" }>): WorkflowAutomationRule | string {
  const next: WorkflowAutomationRule = structuredClone(flow);
  const steps = (next.steps ??= []);
  switch (cmd.type) {
    case "AddStep": {
      const at = locate(steps, cmd.path);
      if (!at || at.index < 0 || at.index > at.list.length) return "There is no place there to add a step.";
      // Ids are taken from a copy that grows, so a branch's own steps get ids of their own.
      const scratch: WorkflowAutomationRule = { ...next, steps: [...steps] };
      const take = () => {
        const id = nextId(scratch);
        scratch.steps = [...(scratch.steps ?? []), { id, when: [], kind: service({ UpdateRecordStatus: { new_status: "" } }) as any }];
        return id;
      };
      const id = take();
      at.list.splice(at.index, 0, newStep(id, cmd.kind, take));
      return next;
    }
    case "RemoveStep": {
      const at = locate(steps, cmd.path);
      if (!at || !at.list[at.index]) return "There is no step there to remove.";
      at.list.splice(at.index, 1);
      return next;
    }
    case "MoveStep": {
      const { from } = cmd;
      const to = [...cmd.to];
      const src = locate(steps, from);
      if (!src || !src.list[src.index]) return "There is no step there to move.";
      if (to.length > from.length && from.every((x, i) => to[i] === x)) return "A branch cannot move inside itself.";
      // `to` names a place in the flow as it was. Taking the step out shifts the later places in its list.
      const d = from.length - 1;
      if (to.length > d && from.slice(0, d).every((x, i) => to[i] === x) && to[d] > from[d]) to[d] -= 1;
      const [moving] = src.list.splice(src.index, 1);
      const dst = locate(steps, to);
      if (!dst || dst.index < 0 || dst.index > dst.list.length) return "There is no place there to move the step to.";
      dst.list.splice(dst.index, 0, moving);
      return next;
    }
    case "UpdateStep": {
      const at = locate(steps, cmd.path);
      const s = at?.list[at.index];
      if (!at || !s) return "There is no step there to change.";
      const { id: _ignored, ...patch } = cmd.patch as Partial<ProcessStepItem>;
      at.list[at.index] = { ...s, ...structuredClone(patch), id: s.id };
      return next;
    }
    case "AddBranch": {
      const b = branchOf(stepAt(next, cmd.path));
      if (!b) return "Conditions are added to a branch.";
      b.branches.push({ when: [{ field_name: "", operator: "Equals", expected_value: "" }], steps: [] });
      return next;
    }
    case "RemoveBranch": {
      const b = branchOf(stepAt(next, cmd.path.slice(0, -1)));
      const arm = cmd.path[cmd.path.length - 1];
      if (!b || arm < 0 || arm >= b.branches.length) return "There is no condition there to remove.";
      if (b.branches.length === 1) return "A branch needs at least one condition. Remove the branch instead.";
      b.branches.splice(arm, 1);
      return next;
    }
    case "ReorderBranches": {
      const b = branchOf(stepAt(next, cmd.path));
      if (!b) return "Only a branch has conditions to reorder.";
      const n = b.branches.length;
      const order = cmd.order;
      if (order.length !== n || new Set(order).size !== n || order.some((i) => i < 0 || i >= n)) return "The new order must name each condition once.";
      b.branches = order.map((i) => b.branches[i]);
      return next;
    }
    case "SetTrigger":
      next.trigger = structuredClone(cmd.trigger);
      return next;
  }
}

export function flowReducer(state: EditorState, cmd: Command): EditorState {
  if (cmd.type === "Undo") {
    if (state.past.length === 0) return { ...state, notice: "There is nothing to undo." };
    return { flow: state.past[state.past.length - 1], past: state.past.slice(0, -1), future: [state.flow, ...state.future], notice: "" };
  }
  if (cmd.type === "Redo") {
    if (state.future.length === 0) return { ...state, notice: "There is nothing to redo." };
    return { flow: state.future[0], past: [...state.past, state.flow].slice(-UNDO_LIMIT), future: state.future.slice(1), notice: "" };
  }
  const out = apply(state.flow, cmd);
  if (typeof out === "string") return { ...state, notice: out };
  // A command that leaves the flow as it was is not a change, and Undo does not count it.
  if (out === state.flow || JSON.stringify(out) === JSON.stringify(state.flow)) return { ...state, notice: "" };
  // A change may not break what held before.
  const before = new Set(structureProblems(state.flow));
  const broken = structureProblems(out).find((p) => !before.has(p));
  if (broken) return { ...state, notice: broken };
  return { flow: out, past: [...state.past, state.flow].slice(-UNDO_LIMIT), future: [], notice: "" };
}

/** `{type: "X", ...rest}` as the server reads it: `"X"` alone, or `{X: rest}`. */
function tagged(v: { type: string } & Record<string, unknown>): unknown {
  const { type, ...rest } = v;
  return Object.keys(rest).length ? { [type]: rest } : type;
}

/**
 * The flow as the server's `AutomationRule` reads it. The builder keeps a trigger and rule actions as `{type, ...}`;
 * the server names the variant as the key. Steps already have the server's shape.
 */
export function toServerFlow(rule: WorkflowAutomationRule): Record<string, unknown> {
  return {
    ...rule,
    trigger: tagged(rule.trigger as any),
    actions: (rule.actions ?? []).map((a) => tagged(a as any)),
    steps: rule.steps ?? [],
  };
}
