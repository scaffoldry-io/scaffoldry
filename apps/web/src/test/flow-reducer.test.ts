import { describe, it, expect } from "vitest";
import { flowReducer, initialEditor, stepAt, toServerFlow, type Command, type EditorState, type Path, type StepKindName } from "../flows/reducer";
import type { ProcessStepItem, WorkflowAutomationRule } from "../types";

// Workflows phase 7, test 1: every edit is a command, and Undo and Redo restore the state.

const rule = (steps: ProcessStepItem[] = []): WorkflowAutomationRule => ({
  id: "r1", app_slug: "grants", name: "Grant review", description: "", enabled: true, trigger: { type: "RecordCreated" }, predicates: [], actions: [], steps,
});
const run = (s: EditorState, ...cmds: Command[]) => cmds.reduce(flowReducer, s);
const steps = (s: EditorState) => s.flow.steps ?? [];
const kindOf = (st: ProcessStepItem) => Object.keys(st.kind)[0];
const branchOf = (st: ProcessStepItem) => (st.kind as any).Branch as { branches: { when: unknown[]; steps: ProcessStepItem[] }[]; otherwise: ProcessStepItem[] };

describe("flowReducer", () => {
  it("adds, updates, moves and removes steps by path", () => {
    let s = run(initialEditor(rule()), { type: "AddStep", path: [0], kind: "status" }, { type: "AddStep", path: [1], kind: "task" });
    expect(steps(s).map(kindOf)).toEqual(["Service", "UserTask"]);
    const [a, b] = steps(s).map((x) => x.id);
    expect(a).not.toBe(b);

    s = run(s, { type: "UpdateStep", path: [0], patch: { kind: { Service: { action: { UpdateRecordStatus: { new_status: "Open" } } } } } });
    expect(steps(s)[0].kind).toEqual({ Service: { action: { UpdateRecordStatus: { new_status: "Open" } } } });

    s = run(s, { type: "MoveStep", from: [1], to: [0] });
    expect(steps(s).map((x) => x.id)).toEqual([b, a]);

    s = run(s, { type: "RemoveStep", path: [0] });
    expect(steps(s).map((x) => x.id)).toEqual([a]);
  });

  it("an update cannot change a step's id", () => {
    const s = run(initialEditor(rule()), { type: "AddStep", path: [0], kind: "status" });
    const id = steps(s)[0].id;
    expect(steps(run(s, { type: "UpdateStep", path: [0], patch: { id: "other" } as any }))[0].id).toBe(id);
  });

  it("adds, removes and reorders the conditions of a branch, and moves a step into one", () => {
    let s = run(initialEditor(rule()), { type: "AddStep", path: [0], kind: "branch" }, { type: "AddStep", path: [1], kind: "notify" });
    expect(branchOf(steps(s)[0]).branches).toHaveLength(1);
    s = run(s, { type: "AddBranch", path: [0] });
    expect(branchOf(steps(s)[0]).branches).toHaveLength(2);
    const first = branchOf(steps(s)[0]).branches[0];
    s = run(s, { type: "ReorderBranches", path: [0], order: [1, 0] });
    expect(branchOf(steps(s)[0]).branches[1]).toEqual(first);

    // Into the second arm, at its start. The path names the arm, then the place in it.
    const notify = steps(s)[1].id;
    s = run(s, { type: "MoveStep", from: [1], to: [0, 1, 0] });
    expect(steps(s)).toHaveLength(1);
    expect(stepAt(s.flow, [0, 1, 0])?.id).toBe(notify);

    s = run(s, { type: "RemoveBranch", path: [0, 1] });
    expect(branchOf(steps(s)[0]).branches).toHaveLength(1);
  });

  it("refuses a move into the step's own branch, and the last condition's removal", () => {
    const s = run(initialEditor(rule()), { type: "AddStep", path: [0], kind: "branch" });
    const into = run(s, { type: "MoveStep", from: [0], to: [0, 0, 0] });
    expect(into.flow).toBe(s.flow);
    expect(into.notice).toMatch(/inside itself/);
    const last = run(s, { type: "RemoveBranch", path: [0, 0] });
    expect(branchOf(steps(last)[0]).branches).toHaveLength(1);
    expect(last.notice).toMatch(/at least one condition/);
  });

  it("refuses to remove a step another step names", () => {
    const create: ProcessStepItem = { id: "make", when: [], kind: { Service: { action: { CreateRecord: { id: "make", app: "grants", table: "logs", fields: [] } } } } as any };
    const update: ProcessStepItem = { id: "touch", when: [], kind: { Service: { action: { UpdateRecord: { target: { Created: { step: "make" } }, fields: [["note", "{make.id}"]] } } } } as any };
    const s = run(initialEditor(rule([create, update])), { type: "RemoveStep", path: [0] });
    expect(steps(s)).toHaveLength(2);
    expect(s.notice).toMatch(/touch/);
  });

  it("sets the trigger", () => {
    const s = run(initialEditor(rule()), { type: "SetTrigger", trigger: { type: "StatusChanged", to_status: "Submitted" } });
    expect(s.flow.trigger).toEqual({ type: "StatusChanged", to_status: "Submitted" });
  });

  it("undoes and redoes every change, and keeps 100 changes", () => {
    const start = initialEditor(rule());
    let s = run(start, { type: "AddStep", path: [0], kind: "status" }, { type: "AddStep", path: [1], kind: "task" });
    const two = s.flow;
    s = run(s, { type: "Undo" });
    expect(steps(s)).toHaveLength(1);
    s = run(s, { type: "Undo" });
    expect(s.flow).toEqual(start.flow);
    s = run(s, { type: "Redo" }, { type: "Redo" });
    expect(s.flow).toEqual(two);
    // A new change clears what could be redone.
    s = run(s, { type: "Undo" }, { type: "AddStep", path: [1], kind: "notify" }, { type: "Redo" });
    expect(steps(s).map(kindOf)).toEqual(["Service", "Service"]);

    let many = start;
    for (let i = 0; i < 130; i++) many = flowReducer(many, { type: "AddStep", path: [0], kind: "status" });
    expect(many.past).toHaveLength(100);
    for (let i = 0; i < 150; i++) many = flowReducer(many, { type: "Undo" });
    expect(steps(many)).toHaveLength(30);
  });

  it("writes the trigger and rule actions as the server reads them", () => {
    const r = { ...rule(), trigger: { type: "StatusChanged" as const, to_status: "Approved" }, actions: [{ type: "NotifyCollaborator" as const, role: "dean", message_template: "Hi" }] };
    const out = toServerFlow(r) as any;
    expect(out.trigger).toEqual({ StatusChanged: { to_status: "Approved" } });
    expect(out.actions).toEqual([{ NotifyCollaborator: { role: "dean", message_template: "Hi" } }]);
    expect(toServerFlow(rule()).trigger).toBe("RecordCreated");
  });
});

// The structural checks the server's validator makes before anything else: every step has a kind it knows, ids are
// unique, and every step a target or an expression names exists. A broken structure here is what the fuzz looks for.
function structuralProblems(flow: WorkflowAutomationRule): string[] {
  const out: string[] = [];
  const ids: string[] = [];
  const named: string[] = [];
  const walk = (list: ProcessStepItem[]) => {
    for (const st of list) {
      ids.push(st.id);
      const k = kindOf(st);
      if (!["Service", "UserTask", "Branch"].includes(k)) out.push(`unknown kind ${k}`);
      const action = (st.kind as any).Service?.action;
      if (action?.CreateRecord || action?.FindRecord) ids.push((action.CreateRecord ?? action.FindRecord).id);
      JSON.stringify(st.kind, (key, v) => {
        if ((key === "Created" || key === "Found") && v?.step) named.push(v.step);
        if (typeof v === "string") for (const m of v.matchAll(/\{([a-z0-9_-]+)\./gi)) if (!["trigger", "run", "input"].includes(m[1])) named.push(m[1]);
        return v;
      });
      if (k === "Branch") {
        const b = branchOf(st);
        if (b.branches.length === 0) out.push(`branch ${st.id} has no condition`);
        b.branches.forEach((arm) => walk(arm.steps));
        walk(b.otherwise);
      }
    }
  };
  walk(flow.steps ?? []);
  const stepIds = new Set<string>();
  const dupe = (list: ProcessStepItem[]): void => list.forEach((st) => {
    if (stepIds.has(st.id)) out.push(`step id ${st.id} twice`);
    stepIds.add(st.id);
    if (kindOf(st) === "Branch") { branchOf(st).branches.forEach((a) => dupe(a.steps)); dupe(branchOf(st).otherwise); }
  });
  dupe(flow.steps ?? []);
  for (const n of named) if (!ids.includes(n)) out.push(`names ${n}, which is not a step`);
  return out;
}

// Every path in the flow, so the fuzz picks real places as often as made-up ones.
function allPaths(list: ProcessStepItem[], prefix: Path = []): Path[] {
  const out: Path[] = [];
  list.forEach((st, i) => {
    out.push([...prefix, i]);
    if (kindOf(st) === "Branch") {
      const b = branchOf(st);
      [...b.branches.map((a) => a.steps), b.otherwise].forEach((arm, a) => out.push(...allPaths(arm, [...prefix, i, a]).concat([[...prefix, i, a, arm.length]])));
    }
  });
  return out;
}

describe("flowReducer fuzz", () => {
  it("200 random commands never break the structure", () => {
    // A small fixed-seed generator, so a failure repeats.
    let seed = 7;
    const rand = (n: number) => {
      seed = (seed * 1103515245 + 12345) % 2147483648;
      return seed % n;
    };
    const kinds: StepKindName[] = ["status", "notify", "audit", "task", "branch", "create", "update", "find"];
    const ref: ProcessStepItem = { id: "touch", when: [], kind: { Service: { action: { UpdateRecord: { target: { Created: { step: "make" } }, fields: [["note", "{make.id}"]] } } } } as any };
    const make: ProcessStepItem = { id: "make", when: [], kind: { Service: { action: { CreateRecord: { id: "make", app: "grants", table: "logs", fields: [] } } } } as any };
    let s = initialEditor(rule([make, ref]));
    let changed = 0;
    for (let i = 0; i < 200; i++) {
      const paths = allPaths(steps(s));
      const pick = (): Path => (paths.length && rand(5) > 0 ? paths[rand(paths.length)] : [rand(4), rand(3), rand(3)]);
      const options: Command[] = [
        { type: "AddStep", path: [...pick().slice(0, -1), rand(4)], kind: kinds[rand(kinds.length)] },
        { type: "RemoveStep", path: pick() },
        { type: "MoveStep", from: pick(), to: pick() },
        { type: "UpdateStep", path: pick(), patch: { when: [{ field_name: "gpa", operator: "Equals", expected_value: String(i) }] } },
        { type: "UpdateStep", path: pick(), patch: { kind: { Nonsense: {} } } as any },
        { type: "AddBranch", path: pick() },
        { type: "RemoveBranch", path: [...pick(), rand(3)] },
        { type: "ReorderBranches", path: pick(), order: [rand(3), rand(3)] },
        { type: "SetTrigger", trigger: { type: "RecordUpdated" } },
        { type: "Undo" },
        { type: "Redo" },
      ];
      const cmd = options[rand(options.length)];
      const before = s.flow;
      s = flowReducer(s, cmd);
      if (s.flow !== before) changed++;
      expect(structuralProblems(s.flow), `after ${i}: ${JSON.stringify(cmd)}`).toEqual([]);
    }
    // Most commands apply: the fuzz is not passing by refusing everything.
    expect(changed).toBeGreaterThan(50);
  });
});
