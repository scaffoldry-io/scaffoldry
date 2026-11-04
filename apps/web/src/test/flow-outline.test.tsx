import "@testing-library/jest-dom";
import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { render, screen, fireEvent, within, act } from "@testing-library/react";
import { Outline, FlowEditor } from "../flows/Outline";
import { initialEditor, flowReducer, type Command, type EditorState } from "../flows/reducer";
import type { ProcessStepItem, WorkflowAutomationRule } from "../types";

// Workflows phase 7, tests 3 and 4: the outline issues every command, and an edit is checked by the server after 400 ms.

const status = (id: string, s = ""): ProcessStepItem => ({ id, when: [], kind: { Service: { action: { UpdateRecordStatus: { new_status: s } } } } as any });
const flow = (): WorkflowAutomationRule => ({
  id: "r1", app_slug: "grants", name: "Grant review", description: "", enabled: true, trigger: { type: "RecordCreated" }, predicates: [], actions: [],
  steps: [
    status("a", "Open"),
    {
      id: "split",
      when: [],
      kind: {
        Branch: {
          branches: [
            { when: [{ field_name: "gpa", operator: "GreaterThan", expected_value: "3.5" }], steps: [status("hi", "Honors")] },
            { when: [{ field_name: "gpa", operator: "LessThan", expected_value: "2" }], steps: [] },
          ],
          otherwise: [status("lo", "Standard")],
        },
      },
    },
    { id: "tell", when: [], kind: { Service: { action: { NotifyCollaborator: { role: "dean", message_template: "Done" } } } } as any },
  ],
});
const fields = [{ name: "gpa", label: "GPA", field_type: "Number", required: false, ferpa_sensitive: false } as any];

/** An editor with one change already made, so Undo is offered, and one taken back, so Redo is. */
function withHistory(): EditorState {
  let s = initialEditor(flow());
  s = flowReducer(s, { type: "SetTrigger", trigger: { type: "RecordUpdated" } });
  s = flowReducer(s, { type: "SetTrigger", trigger: { type: "RecordCreated" } });
  return flowReducer(s, { type: "Undo" });
}

function renderOutline() {
  const dispatch = vi.fn<(c: Command) => void>();
  render(<Outline state={withHistory()} dispatch={dispatch} fields={fields} appSlug="grants" problems={[]} />);
  return dispatch;
}

const item = (label: string) => screen.getByRole("listitem", { name: label });

// Each command, the control in the outline that issues it, and the command it sends.
const controls: { command: string; control: string; act: () => void; sends: Command }[] = [
  { command: "AddStep", control: "button Add a step after step 1", act: () => fireEvent.click(screen.getByRole("button", { name: "Add a step after step 1" })), sends: { type: "AddStep", path: [1], kind: "status" } },
  { command: "AddStep", control: "button Add a branch", act: () => fireEvent.click(screen.getByRole("button", { name: "Add a branch" })), sends: { type: "AddStep", path: [3], kind: "branch" } },
  { command: "AddStep", control: "button Add a step to condition 2 of step 2", act: () => fireEvent.click(screen.getByRole("button", { name: "Add a step to condition 2 of step 2" })), sends: { type: "AddStep", path: [1, 1, 0], kind: "status" } },
  { command: "AddStep", control: "Alt+Enter on step 3", act: () => fireEvent.keyDown(item("step 3"), { key: "Enter", altKey: true }), sends: { type: "AddStep", path: [3], kind: "status" } },
  { command: "RemoveStep", control: "button Remove step 3", act: () => fireEvent.click(screen.getByRole("button", { name: "Remove step 3" })), sends: { type: "RemoveStep", path: [2] } },
  { command: "RemoveStep", control: "Delete on step 3", act: () => fireEvent.keyDown(item("step 3"), { key: "Delete" }), sends: { type: "RemoveStep", path: [2] } },
  { command: "MoveStep", control: "button Move step 3 up", act: () => fireEvent.click(screen.getByRole("button", { name: "Move step 3 up" })), sends: { type: "MoveStep", from: [2], to: [1] } },
  { command: "MoveStep", control: "button Move step 1 down", act: () => fireEvent.click(screen.getByRole("button", { name: "Move step 1 down" })), sends: { type: "MoveStep", from: [0], to: [2] } },
  { command: "MoveStep", control: "Alt+ArrowUp on step 3", act: () => fireEvent.keyDown(item("step 3"), { key: "ArrowUp", altKey: true }), sends: { type: "MoveStep", from: [2], to: [1] } },
  { command: "MoveStep", control: "Alt+ArrowDown on step 1", act: () => fireEvent.keyDown(item("step 1"), { key: "ArrowDown", altKey: true }), sends: { type: "MoveStep", from: [0], to: [2] } },
  { command: "MoveStep", control: "button Move step 3 into the branch above", act: () => fireEvent.click(screen.getByRole("button", { name: "Move step 3 into the branch above" })), sends: { type: "MoveStep", from: [2], to: [1, 0, 1] } },
  { command: "MoveStep", control: "Alt+ArrowRight on step 3", act: () => fireEvent.keyDown(item("step 3"), { key: "ArrowRight", altKey: true }), sends: { type: "MoveStep", from: [2], to: [1, 0, 1] } },
  { command: "MoveStep", control: "button Move step 2.1.1 out of the branch", act: () => fireEvent.click(screen.getByRole("button", { name: "Move step 2.1.1 out of the branch" })), sends: { type: "MoveStep", from: [1, 0, 0], to: [2] } },
  { command: "MoveStep", control: "Alt+ArrowLeft on step 2.3.1", act: () => fireEvent.keyDown(item("step 2.3.1"), { key: "ArrowLeft", altKey: true }), sends: { type: "MoveStep", from: [1, 2, 0], to: [2] } },
  {
    command: "UpdateStep", control: "input Set status to, in step 1",
    act: () => fireEvent.change(within(item("step 1")).getByLabelText("Set status to"), { target: { value: "Closed" } }),
    sends: { type: "UpdateStep", path: [0], patch: { kind: { Service: { action: { UpdateRecordStatus: { new_status: "Closed" } } } } as any } },
  },
  {
    command: "UpdateStep", control: "select Branch value of condition 1, in step 2",
    act: () => fireEvent.change(within(item("step 2")).getByLabelText("Branch value"), { target: { value: "3.7" } }),
    sends: { type: "UpdateStep", path: [1], patch: { kind: { Branch: { branches: [{ when: [{ field_name: "gpa", operator: "GreaterThan", expected_value: "3.7" }], steps: [status("hi", "Honors")] }, { when: [{ field_name: "gpa", operator: "LessThan", expected_value: "2" }], steps: [] }], otherwise: [status("lo", "Standard")] } } } as any },
  },
  { command: "AddBranch", control: "button Add a condition to step 2", act: () => fireEvent.click(screen.getByRole("button", { name: "Add a condition to step 2" })), sends: { type: "AddBranch", path: [1] } },
  { command: "RemoveBranch", control: "button Remove condition 1 of step 2", act: () => fireEvent.click(screen.getByRole("button", { name: "Remove condition 1 of step 2" })), sends: { type: "RemoveBranch", path: [1, 0] } },
  { command: "ReorderBranches", control: "button Move condition 2 of step 2 up", act: () => fireEvent.click(screen.getByRole("button", { name: "Move condition 2 of step 2 up" })), sends: { type: "ReorderBranches", path: [1], order: [1, 0] } },
  { command: "SetTrigger", control: "select Starts when", act: () => fireEvent.change(screen.getByLabelText("Starts when"), { target: { value: "RecordUpdated" } }), sends: { type: "SetTrigger", trigger: { type: "RecordUpdated" } } },
  { command: "Undo", control: "button Undo", act: () => fireEvent.click(screen.getByRole("button", { name: "Undo" })), sends: { type: "Undo" } },
  { command: "Undo", control: "Ctrl+Z in the outline", act: () => fireEvent.keyDown(screen.getByRole("region", { name: "Steps" }), { key: "z", ctrlKey: true }), sends: { type: "Undo" } },
  { command: "Redo", control: "button Redo", act: () => fireEvent.click(screen.getByRole("button", { name: "Redo" })), sends: { type: "Redo" } },
  { command: "Redo", control: "Ctrl+Shift+Z in the outline", act: () => fireEvent.keyDown(screen.getByRole("region", { name: "Steps" }), { key: "Z", ctrlKey: true, shiftKey: true }), sends: { type: "Redo" } },
];

describe("the outline issues every command", () => {
  beforeEach(() => {
    vi.restoreAllMocks();
    global.fetch = vi.fn().mockResolvedValue({ ok: true, status: 200, json: async () => ({}) }) as unknown as typeof fetch;
  });

  it.each(controls)("$command from $control", ({ act: run, sends }) => {
    const dispatch = renderOutline();
    run();
    expect(dispatch).toHaveBeenCalledWith(sends);
  });

  it("covers every command", () => {
    const all = ["AddStep", "RemoveStep", "MoveStep", "UpdateStep", "AddBranch", "RemoveBranch", "ReorderBranches", "SetTrigger", "Undo", "Redo"];
    expect([...new Set(controls.map((c) => c.command))].sort()).toEqual(all.sort());
  });
});

describe("the server checks each edit", () => {
  let posted: { url: string; body: any }[];
  beforeEach(() => {
    vi.restoreAllMocks();
    vi.useFakeTimers();
    posted = [];
    global.fetch = vi.fn().mockImplementation((url: string, init?: RequestInit) => {
      if (String(url).endsWith("/flows/validate")) {
        posted.push({ url: String(url), body: JSON.parse(String(init?.body)) });
        const problems = [{ message: "step 'a': the status is empty", step: "a", path: [0] }];
        return Promise.resolve({ ok: true, status: 200, json: async () => ({ problems, grants: null, parties: ["ws"], checks: [], description: [] }) });
      }
      return Promise.resolve({ ok: true, status: 200, json: async () => ({}) });
    }) as unknown as typeof fetch;
  });
  afterEach(() => vi.useRealTimers());

  it("sends the flow 400 ms after the last edit, and shows and announces the problem at its step", async () => {
    render(<FlowEditor initial={flow()} fields={fields} appSlug="grants" />);
    await act(async () => {
      await vi.advanceTimersByTimeAsync(1000);
    });
    expect(posted).toHaveLength(0);

    const input = within(item("step 1")).getByLabelText("Set status to");
    fireEvent.change(input, { target: { value: "" } });
    await act(async () => {
      await vi.advanceTimersByTimeAsync(300);
    });
    fireEvent.change(input, { target: { value: "X" } });
    fireEvent.change(input, { target: { value: "" } });
    await act(async () => {
      await vi.advanceTimersByTimeAsync(399);
    });
    expect(posted).toHaveLength(0);
    await act(async () => {
      await vi.advanceTimersByTimeAsync(1);
    });
    expect(posted).toHaveLength(1);
    expect(posted[0].url).toBe("/api/v1/flows/validate");
    // The server's shape: the trigger names its variant.
    expect(posted[0].body.trigger).toBe("RecordCreated");
    expect(posted[0].body.steps[0].kind.Service.action.UpdateRecordStatus.new_status).toBe("");

    await act(async () => {
      await vi.advanceTimersByTimeAsync(0);
    });
    expect(within(item("step 1")).getByText("step 'a': the status is empty")).toBeInTheDocument();
    expect(screen.getByRole("status", { name: "Check result" })).toHaveTextContent("1 problem. Step 1: step 'a': the status is empty");
  });
});
