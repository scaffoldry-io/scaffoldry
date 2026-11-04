import "@testing-library/jest-dom";
import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent, waitFor, within, cleanup } from "@testing-library/react";
import { WorkflowBuilder } from "../WorkflowBuilder";
import { ProcessDesk } from "../ProcessDesk";
import type { WorkflowAutomationRule, Persona } from "../types";

// Process v2 phase 5: the builder and desk screens for timers, committees and branches. `fetch` is stubbed.

const persona: Persona = { eppn: "ash@state.edu", name: "Ash", affiliation: "faculty", department: "Physics", roleTitle: "Lecturer", isAdmin: false };
const app = { slug: "grants", title: "Grants" };

function builderFetch() {
  global.fetch = vi.fn().mockImplementation((url: string) => {
    const u = String(url);
    let body: unknown = {};
    if (u.endsWith("/automations/approver-options")) body = { positions: [{ key: "chair", name: "Chair" }] };
    if (u.endsWith("/automations/preview-approvers")) body = { approvers: [], unit_name: "Physics", start_unit_name: "Physics", problem: null };
    return Promise.resolve({ ok: true, status: 200, json: async () => body });
  }) as unknown as typeof fetch;
}

function openBuilder(onSaveRule: (r: WorkflowAutomationRule) => void, rules: WorkflowAutomationRule[] = []) {
  render(
    <WorkflowBuilder
      appSlug="grants"
      appTitle="Grants"
      fields={[{ name: "gpa", label: "GPA", field_type: "Number", required: false, ferpa_sensitive: false } as any]}
      rules={rules}
      onSaveRule={onSaveRule}
      onDeleteRule={() => {}}
      onToggleRule={() => {}}
    />
  );
  fireEvent.click(screen.getByRole("button", { name: /Create Automation Rule/ }));
  fireEvent.change(screen.getByPlaceholderText(/Dean Fellowship Escalation/), { target: { value: "Grant review" } });
}

/** A desk whose one waiting decision is `inst`, and whose votes route answers `votes`. Returns what was posted. */
function deskFetch(inst: Record<string, unknown>, votes?: unknown) {
  const posted: { url: string; body: any }[] = [];
  global.fetch = vi.fn().mockImplementation((url: string, init?: RequestInit) => {
    const u = String(url);
    if (init?.method === "POST") {
      posted.push({ url: u, body: JSON.parse(String(init.body ?? "{}")) });
      return Promise.resolve({ ok: true, status: 200, json: async () => ({ ...inst, status: "Waiting" }) });
    }
    if (u.includes("/votes")) return Promise.resolve({ ok: true, status: 200, json: async () => votes });
    return Promise.resolve({ ok: true, status: 200, json: async () => [inst] });
  }) as unknown as typeof fetch;
  return posted;
}

const days = (n: number) => new Date(Date.now() + n * 86_400_000);

describe("process screens", () => {
  beforeEach(() => vi.restoreAllMocks());

  // 1. The step preview sentence matches the chosen options.
  it("previews the whole step from the chosen options and saves them", async () => {
    builderFetch();
    const saved = vi.fn();
    openBuilder(saved);
    fireEvent.click(screen.getByLabelText("Add an approval step"));
    fireEvent.change(await screen.findByLabelText("Position"), { target: { value: "chair" } });
    fireEvent.change(screen.getByLabelText("How many must agree"), { target: { value: "AtLeast" } });
    fireEvent.change(screen.getByLabelText("How many"), { target: { value: "2" } });
    fireEvent.change(screen.getByLabelText("Due after (days)"), { target: { value: "7" } });
    fireEvent.change(screen.getByLabelText("If it is late"), { target: { value: "EscalateUp" } });

    expect(screen.getByTestId("step-preview")).toHaveTextContent(
      "Asked of the chair of the submitter's unit. At least 2 of them must approve. If no one decides in 7 days, it goes to the next level up."
    );
    fireEvent.change(screen.getByLabelText("If it is late"), { target: { value: "Remind" } });
    expect(screen.getByTestId("step-preview")).toHaveTextContent("If no one decides in 7 days, they are reminded.");
    fireEvent.change(screen.getByLabelText("If it is late"), { target: { value: "Fail" } });
    fireEvent.change(screen.getByLabelText("How many must agree"), { target: { value: "AllOf" } });
    expect(screen.getByTestId("step-preview")).toHaveTextContent("All of them must approve. If no one decides in 7 days, it fails.");

    fireEvent.click(screen.getByRole("button", { name: "Save Rule" }));
    await waitFor(() => expect(saved).toHaveBeenCalled());
    const task = (saved.mock.calls[0][0] as any).steps.find((s: any) => s.kind.UserTask).kind.UserTask;
    expect(task.decision).toBe("AllOf");
    expect(task.due).toEqual({ days: 7 });
    expect(task.on_due).toEqual([{ Fail: { reason: "Nobody decided within 7 days." } }]);
  });

  // 2. A committee step shows the tally and the person's own vote, and `Change my vote` posts the new vote.
  it("shows a committee's tally and my vote, and changes it", async () => {
    const inst = {
      id: "review:rec-1", rule_id: "review", app_slug: "grants", record_id: "rec-1", status: "Waiting", waiting_step_id: "committee",
      prompt: "Vote on the grant", voters: ["ash@state.edu", "bea@state.edu", "cy@state.edu"], tally: { approve: 2, reject: 0, voters: 3 },
    };
    const posted = deskFetch(inst, {
      voters: inst.voters, tally: inst.tally,
      votes: [{ voter: "ash@state.edu", vote: "approve", at: "2026-11-04T10:00:00Z" }, { voter: "bea@state.edu", vote: "approve", at: "2026-11-04T11:00:00Z" }],
    });
    render(<ProcessDesk apps={[app]} callerPersona={persona} />);
    const row = await screen.findByTestId("process-row-review:rec-1");
    expect(within(row).getByText("2 of 3 have approved")).toBeInTheDocument();
    expect(await within(row).findByText("You voted to approve.")).toBeInTheDocument();
    expect(within(row).queryByText(/bea@state.edu/)).not.toBeInTheDocument();

    fireEvent.click(within(row).getByRole("button", { name: "Change my vote" }));
    fireEvent.change(within(row).getByLabelText("Why you reject"), { target: { value: "The budget is too high" } });
    fireEvent.click(within(row).getByRole("button", { name: "Vote to reject" }));
    await waitFor(() => expect(posted).toHaveLength(1));
    expect(posted[0].url).toBe("/api/v1/apps/grants/processes/review:rec-1/decide");
    expect(posted[0].body).toEqual({ decision: "reject", comment: "The budget is too high" });
  });

  // 3. A branch shows its nested steps and the saved rule's JSON has the branch shape.
  it("saves a branch with its nested steps and shows them nested", async () => {
    builderFetch();
    const saved = vi.fn();
    openBuilder(saved);
    fireEvent.click(screen.getByLabelText("Add a branch"));
    fireEvent.change(screen.getByLabelText("Branch field"), { target: { value: "gpa" } });
    fireEvent.change(screen.getByLabelText("Branch test"), { target: { value: "GreaterThan" } });
    fireEvent.change(screen.getByLabelText("Branch value"), { target: { value: "3.5" } });
    fireEvent.change(screen.getByLabelText("Then set status to"), { target: { value: "Honors" } });
    fireEvent.change(screen.getByLabelText("Otherwise set status to"), { target: { value: "Standard" } });
    fireEvent.click(screen.getByRole("button", { name: "Save Rule" }));
    await waitFor(() => expect(saved).toHaveBeenCalled());
    const rule = saved.mock.calls[0][0] as any;
    const branch = rule.steps.find((s: any) => s.kind.Branch).kind.Branch;
    expect(branch.branches).toHaveLength(1);
    expect(branch.branches[0].when).toEqual([{ field_name: "gpa", operator: "GreaterThan", expected_value: "3.5" }]);
    expect(branch.branches[0].steps[0].kind).toEqual({ Service: { action: { UpdateRecordStatus: { new_status: "Honors" } } } });
    expect(branch.otherwise[0].kind).toEqual({ Service: { action: { UpdateRecordStatus: { new_status: "Standard" } } } });

    // The saved rule reads back with its steps nested under each condition.
    cleanup();
    render(<WorkflowBuilder appSlug="grants" appTitle="Grants" fields={[]} rules={[rule]} onSaveRule={() => {}} onDeleteRule={() => {}} onToggleRule={() => {}} />);
    const steps = screen.getByTestId("rule-steps");
    const arm = within(steps).getByTestId("branch-arm-0");
    expect(arm).toHaveTextContent("If gpa is greater than 3.5");
    expect(within(arm).getByText("Set the status to Honors.")).toBeInTheDocument();
    expect(within(within(steps).getByTestId("branch-otherwise")).getByText("Set the status to Standard.")).toBeInTheDocument();
  });

  // 4. The due date is shown as a date and in words.
  it("shows when a decision is due, as a date and in words", async () => {
    const soon = days(3);
    deskFetch({
      id: "chair:rec-2", rule_id: "chair", app_slug: "grants", record_id: "rec-2", status: "Waiting", waiting_step_id: "chair", prompt: "Decide",
      next_timer: { fire_at: soon.toISOString(), action: "EscalateUp", sentence: "On X, this will be sent higher." },
    });
    render(<ProcessDesk apps={[app]} callerPersona={persona} />);
    const due = await screen.findByTestId("process-due-chair:rec-2");
    const date = soon.toLocaleDateString("en-GB", { day: "numeric", month: "short" });
    expect(due).toHaveTextContent(`Due ${date}, in 3 days`);
  });

  it("says how late an overdue decision is", async () => {
    deskFetch({
      id: "chair:rec-3", rule_id: "chair", app_slug: "grants", record_id: "rec-3", status: "Waiting", waiting_step_id: "chair", prompt: "Decide",
      next_timer: { fire_at: days(-2).toISOString(), action: "Remind", sentence: "" },
    });
    render(<ProcessDesk apps={[app]} callerPersona={persona} />);
    expect(await screen.findByTestId("process-due-chair:rec-3")).toHaveTextContent("2 days overdue");
  });
});
