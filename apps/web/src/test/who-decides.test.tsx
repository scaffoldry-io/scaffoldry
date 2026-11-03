import "@testing-library/jest-dom";
import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { WorkflowBuilder } from "../WorkflowBuilder";
import type { WorkflowAutomationRule } from "../types";

// Approvers phase 5: the user-task editor says who decides, and previews who that is today. `fetch` is stubbed.
type Reply = { problem?: string | null; approvers?: unknown[]; unit_name?: string | null; start_unit_name?: string | null };
function stub(reply: Reply) {
  const posted: { url: string; body: any }[] = [];
  global.fetch = vi.fn().mockImplementation((url: string, init?: RequestInit) => {
    const u = String(url);
    let body: unknown = {};
    if (u.endsWith("/automations/approver-options")) body = { positions: [{ key: "chair", name: "Chair" }, { key: "dean", name: "Dean" }] };
    if (u.endsWith("/automations/preview-approvers")) {
      posted.push({ url: u, body: JSON.parse(String(init?.body ?? "{}")) });
      body = { approvers: [], unit_id: null, unit_name: null, start_unit_name: "Physics", problem: null, ...reply };
    }
    return Promise.resolve({ ok: true, status: 200, statusText: "", json: async () => body });
  }) as unknown as typeof fetch;
  return posted;
}

function open(onSaveRule: (r: WorkflowAutomationRule) => void) {
  render(<WorkflowBuilder appSlug="admissions" appTitle="Admissions" fields={[{ name: "gpa", label: "GPA", field_type: "Number", required: false, ferpa_sensitive: false } as any]} rules={[]} onSaveRule={onSaveRule} onDeleteRule={() => {}} onToggleRule={() => {}} />);
  fireEvent.click(screen.getByRole("button", { name: /Create Automation Rule/ }));
  fireEvent.change(screen.getByPlaceholderText(/Dean Fellowship Escalation/), { target: { value: "Chair approves" } });
  fireEvent.click(screen.getByLabelText("Add an approval step"));
}

describe("who decides an approval step", () => {
  beforeEach(() => vi.restoreAllMocks());

  // 1. A position and a starting point post the preview and show the names.
  it("previews who a position goes to today", async () => {
    const posted = stub({ approvers: [{ eppn: "rivera@state.edu", display_name: "Dr. Rivera", via: "Holder" }], unit_name: "Physics" });
    open(() => {});
    fireEvent.change(await screen.findByLabelText("Position"), { target: { value: "chair" } });
    fireEvent.change(screen.getByLabelText("Starting from"), { target: { value: "Workspace" } });
    expect(await screen.findByText("Today this would go to: Dr. Rivera, Chair of Physics.")).toBeInTheDocument();
    const last = posted[posted.length - 1];
    expect(last.url).toContain("/apps/admissions/automations/preview-approvers");
    expect(last.body.approver).toEqual({ Position: { key: "chair", from: "Workspace", walk_up: true } });
  });

  // 2. A vacant position is a warning in words.
  it("says in words when the position is vacant", async () => {
    stub({ problem: "position_vacant", start_unit_name: "Physics" });
    open(() => {});
    fireEvent.change(await screen.findByLabelText("Position"), { target: { value: "chair" } });
    const warning = await screen.findByText("This position is vacant at Physics and above.");
    expect(warning.closest('[role="status"]')).not.toBeNull();
  });

  // 3. The saved rule holds the position spec and no typed role.
  it("saves the position as the approver, with no typed role", async () => {
    stub({ approvers: [{ eppn: "rivera@state.edu", display_name: "Dr. Rivera", via: "Holder" }], unit_name: "Physics" });
    const save = vi.fn();
    open(save);
    fireEvent.change(await screen.findByLabelText("Position"), { target: { value: "chair" } });
    fireEvent.click(screen.getByLabelText("If the position is vacant, ask the next unit up"));
    fireEvent.click(screen.getByRole("button", { name: "Save Rule" }));
    await waitFor(() => expect(save).toHaveBeenCalled());
    const rule: WorkflowAutomationRule = save.mock.calls[0][0];
    const task = (rule.steps ?? [])[0].kind as any;
    expect(task.UserTask.approver).toEqual({ Position: { key: "chair", from: "Submitter", walk_up: false } });
    expect(task.UserTask.role).toBe("");
  });

  // 4. The workspace role option, the old behaviour, still saves a Role.
  it("saves a workspace role as a Role approver", async () => {
    stub({ problem: "role_empty" });
    const save = vi.fn();
    open(save);
    fireEvent.click(await screen.findByLabelText("A workspace role (the old behaviour)"));
    fireEvent.change(screen.getByLabelText("Role name"), { target: { value: "chair" } });
    fireEvent.click(screen.getByRole("button", { name: "Save Rule" }));
    await waitFor(() => expect(save).toHaveBeenCalled());
    const task = (save.mock.calls[0][0].steps ?? [])[0].kind as any;
    expect(task.UserTask.approver).toEqual({ Role: { name: "chair" } });
  });
});
