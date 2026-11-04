import "@testing-library/jest-dom";
import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent, within } from "@testing-library/react";
import React, { useReducer } from "react";
import { Canvas } from "../flows/Canvas";
import { Outline } from "../flows/Outline";
import { layout } from "../flows/layout";
import { flowReducer, initialEditor, type Command, type EditorState } from "../flows/reducer";
import type { ProcessStepItem, WorkflowAutomationRule } from "../types";

// Workflows phase 8: the canvas, a second view of the one editor state.

const status = (id: string, s = ""): ProcessStepItem => ({ id, when: [], kind: { Service: { action: { UpdateRecordStatus: { new_status: s } } } } as any });
const notify = (id: string): ProcessStepItem => ({ id, when: [], kind: { Service: { action: { NotifyCollaborator: { role: "dean", message_template: "Done" } } } } as any });
const branch3 = (id: string): ProcessStepItem => ({
  id,
  when: [],
  kind: {
    Branch: {
      branches: [
        { when: [{ field_name: "gpa", operator: "GreaterThan", expected_value: "3.5" }], steps: [status(`${id}-a1`, "Honors"), notify(`${id}-a2`)] },
        { when: [{ field_name: "gpa", operator: "LessThan", expected_value: "2" }], steps: [status(`${id}-b1`, "Review")] },
      ],
      otherwise: [status(`${id}-c1`, "Standard"), status(`${id}-c2`, "Filed"), notify(`${id}-c3`)],
    },
  },
});
const flow = (steps: ProcessStepItem[]): WorkflowAutomationRule => ({
  id: "r1", app_slug: "grants", name: "Grant review", description: "", enabled: true, trigger: { type: "RecordCreated" }, predicates: [], actions: [], steps,
});
const fields = [{ name: "gpa", label: "GPA", field_type: "Number", required: false, ferpa_sensitive: false } as any];

/** The canvas and the outline over one reducer, as the builder holds them. */
function Both({ initial, problems = [] }: { initial: WorkflowAutomationRule; problems?: any[] }) {
  const [state, dispatch] = useReducer(flowReducer, initial, initialEditor);
  return (
    <>
      <Canvas state={state} dispatch={dispatch} fields={fields} appSlug="grants" problems={problems} />
      <Outline state={state} dispatch={dispatch} fields={fields} appSlug="grants" problems={problems} />
    </>
  );
}

const outlineOrder = () =>
  within(screen.getByRole("region", { name: "Steps" }))
    .getAllByRole("listitem")
    .filter((li) => /^step \d+$/.test(li.getAttribute("aria-label") ?? ""))
    .map((li) => li.querySelector("span.font-semibold")?.textContent);

beforeEach(() => {
  vi.restoreAllMocks();
  global.fetch = vi.fn().mockResolvedValue({ ok: true, status: 200, json: async () => ({}) }) as unknown as typeof fetch;
});

describe("layout", () => {
  // 1. Deterministic, and a branch of three columns places its steps without overlap.
  it("gives the same boxes twice, and three columns that do not overlap", () => {
    const f = flow([status("a", "Open"), branch3("split"), notify("tell")]);
    const one = layout(f);
    const two = layout(structuredClone(f));
    expect(two).toEqual(one);

    const decision = one.boxes.find((b) => b.kind === "decision")!;
    expect(decision).toBeDefined();
    expect(one.boxes.some((b) => b.kind === "join")).toBe(true);
    const columns = [0, 1, 2].map((arm) => one.boxes.filter((b) => b.path && b.path.length === 3 && b.path[0] === 1 && b.path[1] === arm));
    expect(columns.map((c) => c.length)).toEqual([2, 1, 3]);
    // Columns sit side by side, left to right.
    expect(columns[0][0].x + columns[0][0].w).toBeLessThanOrEqual(columns[1][0].x);
    expect(columns[1][0].x + columns[1][0].w).toBeLessThanOrEqual(columns[2][0].x);
    // No two boxes overlap.
    for (let i = 0; i < one.boxes.length; i++)
      for (let j = i + 1; j < one.boxes.length; j++) {
        const a = one.boxes[i];
        const b = one.boxes[j];
        const overlap = a.x < b.x + b.w && b.x < a.x + a.w && a.y < b.y + b.h && b.y < a.y + a.h;
        expect(overlap, `${a.key} overlaps ${b.key}`).toBe(false);
      }
  });
});

describe("the canvas", () => {
  // 2. Dragging a node to a new place dispatches MoveStep, and the outline shows the same order.
  it("moves a step by drag, and the outline follows", () => {
    const seen: Command[] = [];
    function Spy() {
      const [state, inner] = useReducer(flowReducer, flow([status("a", "Open"), notify("tell"), status("z", "Closed")]), initialEditor);
      const dispatch = (c: Command) => {
        seen.push(c);
        inner(c);
      };
      return (
        <>
          <Canvas state={state} dispatch={dispatch} fields={fields} appSlug="grants" problems={[]} />
          <Outline state={state} dispatch={dispatch} fields={fields} appSlug="grants" problems={[]} />
        </>
      );
    }
    render(<Spy />);
    expect(outlineOrder()).toEqual(["Set the status to Open", "Notify dean", "Set the status to Closed"]);
    const canvas = screen.getByRole("group", { name: "Flow canvas" });
    fireEvent.mouseDown(within(canvas).getByRole("button", { name: /^step 3:/ }));
    fireEvent.mouseUp(within(canvas).getByRole("button", { name: "Insert a step before step 1" }));
    expect(seen).toContainEqual({ type: "MoveStep", from: [2], to: [0] });
    expect(outlineOrder()).toEqual(["Set the status to Closed", "Set the status to Open", "Notify dean"]);
  });

  // 3. Adding through `+` gives the same state as adding through the outline.
  it("adds the same step through + as through the outline", () => {
    const start = flow([status("a", "Open")]);
    const viaOutline = flowReducer(initialEditor(start), { type: "AddStep", path: [1], kind: "task" });

    let last: EditorState | null = null;
    function Watch() {
      const [state, dispatch] = useReducer(flowReducer, start, initialEditor);
      last = state;
      return <Canvas state={state} dispatch={dispatch} fields={fields} appSlug="grants" problems={[]} />;
    }
    render(<Watch />);
    fireEvent.click(screen.getByRole("button", { name: "Insert a step after step 1" }));
    const menu = screen.getByRole("menu", { name: "Add a step" });
    expect(within(menu).getByRole("group", { name: "Data" })).toBeInTheDocument();
    expect(within(menu).getByRole("group", { name: "People" })).toBeInTheDocument();
    expect(within(menu).getByRole("group", { name: "Logic" })).toBeInTheDocument();
    expect(within(menu).getByRole("group", { name: "Time" })).toBeInTheDocument();
    expect(within(menu).getByRole("group", { name: "Connect" })).toBeInTheDocument();
    fireEvent.click(within(menu).getByRole("menuitem", { name: "Ask for a decision" }));
    expect(last!.flow).toEqual(viaOutline.flow);
  });

  // 4. A branch with a problem shows the icon and the message in its accessible name, and the first problem is
  // announced.
  it("names a branch's problem in its accessible name", () => {
    const problems = [{ message: "step 'split': branches nest 3 levels deep here", step: "split", path: [1] }];
    render(<Both initial={flow([status("a", "Open"), branch3("split")])} problems={problems} />);
    const canvas = screen.getByRole("group", { name: "Flow canvas" });
    const node = within(canvas).getByRole("button", { name: /^step 2:/ });
    expect(node).toHaveAccessibleName(expect.stringContaining("Problem: step 'split': branches nest 3 levels deep here"));
    expect(within(node).getByText("⚠")).toBeInTheDocument();
    expect(within(canvas).getByRole("status", { name: "Canvas check" })).toHaveTextContent("step 'split': branches nest 3 levels deep here");
  });

  // 5. A full edit by keyboard alone: add, change, move, remove.
  it("does a whole edit from the keyboard", () => {
    render(<Both initial={flow([status("a", "Open")])} />);
    const canvas = screen.getByRole("group", { name: "Flow canvas" });

    // Add: Enter on the insertion point after step 1 opens the menu; Enter on an item adds it.
    const plus = within(canvas).getByRole("button", { name: "Insert a step after step 1" });
    plus.focus();
    fireEvent.keyDown(plus, { key: "Enter" });
    const item = within(screen.getByRole("menu", { name: "Add a step" })).getByRole("menuitem", { name: "Notify a role" });
    expect(document.activeElement).toBe(within(screen.getByRole("menu", { name: "Add a step" })).getAllByRole("menuitem")[0]);
    fireEvent.keyDown(item, { key: "Enter" });
    expect(outlineOrder()).toEqual(["Set the status to Open", "Notify a role"]);

    // Change: Enter on a node opens the inspector; type in its form.
    const second = within(canvas).getByRole("button", { name: /^step 2:/ });
    second.focus();
    fireEvent.keyDown(second, { key: "Enter" });
    const inspector = screen.getByRole("region", { name: "Inspector" });
    fireEvent.change(within(inspector).getByLabelText("Notify role"), { target: { value: "chair" } });
    expect(outlineOrder()).toEqual(["Set the status to Open", "Notify chair"]);

    // Move: Alt+ArrowUp on the node.
    const moved = within(canvas).getByRole("button", { name: /^step 2:/ });
    fireEvent.keyDown(moved, { key: "ArrowUp", altKey: true });
    expect(outlineOrder()).toEqual(["Notify chair", "Set the status to Open"]);

    // Remove: Delete on the node.
    fireEvent.keyDown(within(canvas).getByRole("button", { name: /^step 1:/ }), { key: "Delete" });
    expect(outlineOrder()).toEqual(["Set the status to Open"]);
  });

  it("zooms with + and -, fits, and pans with the arrow keys when nothing is selected", () => {
    render(<Both initial={flow([status("a", "Open"), notify("tell")])} />);
    const canvas = screen.getByRole("group", { name: "Flow canvas" });
    const svg = canvas.querySelector("svg")!;
    const box = () => svg.getAttribute("viewBox")!.split(" ").map(Number);
    const [, , w0] = box();
    fireEvent.click(screen.getByRole("button", { name: "Zoom in" }));
    expect(box()[2]).toBeLessThan(w0);
    fireEvent.click(screen.getByRole("button", { name: "Zoom out" }));
    fireEvent.click(screen.getByRole("button", { name: "Zoom out" }));
    expect(box()[2]).toBeGreaterThan(w0);
    fireEvent.click(screen.getByRole("button", { name: "Fit" }));
    expect(box()[2]).toBe(w0);
    const x0 = box()[0];
    fireEvent.keyDown(canvas, { key: "ArrowRight" });
    expect(box()[0]).toBeGreaterThan(x0);
  });

  // 6. At 1,000 pixels wide, 40 steps render in under 200 ms.
  it("renders 40 steps in under 200 ms at 1,000 px", () => {
    const steps: ProcessStepItem[] = [];
    for (let i = 0; i < 40; i++) steps.push(i % 10 === 5 ? branch3(`b${i}`) : status(`s${i}`, `S${i}`));
    const state = initialEditor(flow(steps));
    const t0 = performance.now();
    render(
      <div style={{ width: 1000 }}>
        <Canvas state={state} dispatch={() => {}} fields={fields} appSlug="grants" problems={[]} width={1000} />
      </div>
    );
    const ms = performance.now() - t0;
    console.info(`canvas: 40 steps rendered in ${ms.toFixed(1)} ms`);
    expect(within(screen.getByRole("group", { name: "Flow canvas" })).getAllByRole("button", { name: /^step \d+:/ })).toHaveLength(40);
    expect(ms).toBeLessThan(200);
  });
});

describe("the builder's two views", () => {
  it("switches between the outline and the canvas, and both show the same steps", async () => {
    const { WorkflowBuilder } = await import("../WorkflowBuilder");
    render(<WorkflowBuilder appSlug="grants" appTitle="Grants" fields={fields} rules={[]} onSaveRule={() => {}} onDeleteRule={() => {}} onToggleRule={() => {}} />);
    fireEvent.click(screen.getByRole("button", { name: /Create Automation Rule/ }));
    fireEvent.click(screen.getByLabelText("Add a notification step"));
    fireEvent.click(screen.getByRole("tab", { name: "Canvas" }));
    expect(screen.getByRole("tab", { name: "Canvas" })).toHaveAttribute("aria-selected", "true");
    const canvas = screen.getByRole("group", { name: "Flow canvas" });
    expect(within(canvas).getByRole("button", { name: /^step 1: Step\. Notify a role/ })).toBeInTheDocument();
    fireEvent.click(within(canvas).getByRole("button", { name: "Insert a step after step 1" }));
    fireEvent.click(screen.getByRole("menuitem", { name: "Set the status" }));
    fireEvent.click(screen.getByRole("tab", { name: "Outline" }));
    expect(outlineOrder()).toEqual(["Notify a role", "Set the status"]);
  });
});
