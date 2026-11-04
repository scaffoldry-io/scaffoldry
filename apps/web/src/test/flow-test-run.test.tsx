import "@testing-library/jest-dom";
import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent, within, waitFor } from "@testing-library/react";
import React from "react";
import { Canvas } from "../flows/Canvas";
import { Outline } from "../flows/Outline";
import { TestRunPanel, type TraceEntry } from "../flows/TestRun";
import { initialEditor } from "../flows/reducer";
import type { ProcessStepItem, WorkflowAutomationRule } from "../types";

// Workflows phase 9: the trace of a test run, on the canvas, in the outline, and in the panel that asks for it.

const status = (id: string, s: string): ProcessStepItem => ({ id, when: [], kind: { Service: { action: { UpdateRecordStatus: { new_status: s } } } } as any });
const flow: WorkflowAutomationRule = {
  id: "aid-check", app_slug: "grants", name: "Aid check", description: "", enabled: true, trigger: { type: "RecordCreated" }, predicates: [], actions: [],
  steps: [
    status("open", "Open"),
    {
      id: "split",
      when: [],
      kind: { Branch: { branches: [{ when: [{ field_name: "amount", operator: "GreaterThan", expected_value: "100" }], steps: [status("big", "Big")] }], otherwise: [status("mid", "Mid")] } },
    },
  ],
};
const trace: TraceEntry[] = [
  { step: "open", path: [0], outcome: "would write", sentence: "would set the status to Open" },
  { step: "split", path: [1], outcome: "ran", sentence: "No condition held, so it took otherwise." },
  { step: "big", path: [1, 0, 0], outcome: "skipped (condition was false)", sentence: "This step would not run: its condition was false." },
  { step: "mid", path: [1, 1, 0], outcome: "would write", sentence: "would set the status to Mid" },
];

beforeEach(() => vi.restoreAllMocks());

describe("a test run's trace", () => {
  // 6. The canvas marks the path in text for a screen reader.
  it("marks the path taken and the skipped steps on the canvas, in text", () => {
    render(<Canvas state={initialEditor(flow)} dispatch={() => {}} fields={[]} appSlug="grants" problems={[]} trace={trace} />);
    const canvas = screen.getByRole("group", { name: "Flow canvas" });
    expect(within(canvas).getByRole("button", { name: /^step 1:/ })).toHaveAccessibleName(expect.stringContaining("Test run: on the path. Would write: would set the status to Open"));
    expect(within(canvas).getByRole("button", { name: /^step 2\.1\.1:/ })).toHaveAccessibleName(expect.stringContaining("Test run: skipped, its condition was false"));
    expect(within(canvas).getByRole("button", { name: /^step 2\.2\.1:/ })).toHaveAccessibleName(expect.stringContaining("Test run: on the path"));
    expect(within(canvas).getByRole("status", { name: "Test run summary" })).toHaveTextContent("Test run: 3 steps on the path, 1 skipped, no failure.");
    // Shown in the node, not only in its name.
    expect(within(within(canvas).getByRole("button", { name: /^step 2\.1\.1:/ })).getByText("Skipped")).toBeInTheDocument();
  });

  it("lists the same outcomes in the outline", () => {
    render(<Outline state={initialEditor(flow)} dispatch={() => {}} fields={[]} appSlug="grants" problems={[]} trace={trace} />);
    expect(within(screen.getByRole("listitem", { name: "step 2.1.1" })).getByText("Test run: skipped, its condition was false.")).toBeInTheDocument();
    expect(within(screen.getByRole("listitem", { name: "step 1" })).getByText("Test run: would write: would set the status to Open.")).toBeInTheDocument();
  });

  it("runs the draft against a record, shows the first failure, and runs again with another", async () => {
    const posted: { url: string; body: any }[] = [];
    global.fetch = vi.fn().mockImplementation((url: string, init?: RequestInit) => {
      posted.push({ url: String(url), body: JSON.parse(String(init?.body ?? "{}")) });
      const failing = posted.length === 1;
      const body = failing
        ? { trace: [trace[0], { step: "split", path: [1], outcome: "failed", sentence: "flow_grant_exceeded: it reads applications.amount" }], first_failure: { step: "split", path: [1], reason: "flow_grant_exceeded: it reads applications.amount" }, writes: 1 }
        : { trace, first_failure: null, writes: 2 };
      return Promise.resolve({ ok: true, status: 200, json: async () => body });
    }) as unknown as typeof fetch;
    const onTrace = vi.fn();
    render(<TestRunPanel flow={flow} onTrace={onTrace} />);
    fireEvent.change(screen.getByLabelText("Record id"), { target: { value: "rec-1" } });
    fireEvent.click(screen.getByRole("button", { name: "Run a test" }));
    await waitFor(() => expect(posted).toHaveLength(1));
    expect(posted[0].url).toBe("/api/v1/flows/aid-check/test-run");
    expect(posted[0].body.record_id).toBe("rec-1");
    expect(posted[0].body.flow.trigger).toBe("RecordCreated");
    expect(await screen.findByRole("alert")).toHaveTextContent("Step 2 failed: flow_grant_exceeded: it reads applications.amount");
    expect(screen.getByText(/Nothing was written, sent or stored/)).toBeInTheDocument();

    fireEvent.change(screen.getByLabelText("Run again with record id"), { target: { value: "rec-2" } });
    fireEvent.click(screen.getByRole("button", { name: "Run again" }));
    await waitFor(() => expect(posted).toHaveLength(2));
    expect(posted[1].body.record_id).toBe("rec-2");
    await waitFor(() => expect(screen.queryByRole("alert")).not.toBeInTheDocument());
    expect(onTrace).toHaveBeenLastCalledWith(trace);
  });
});
