import "@testing-library/jest-dom";
import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent, within, waitFor } from "@testing-library/react";
import { FlowHistory } from "../flows/FlowHistory";

// Workflows phase 10: the Runs and Versions tabs of a saved flow.

const step = (id: string, note: string) => ({ id, when: [], kind: { Service: { action: { CreateRecord: { id, app: "housing", table: "logs", fields: [["note", note]] } } } } });

const runs = {
  runs: [
    { instance_id: "open-file:run-1", status: "Failed", started_at: "2026-11-04T10:00:00Z", duration_ms: 1200, trigger: "Started when record 3f2a9b1c was created", started_by: "einstein@physics.state.edu", writes: 2, version: 1 },
    { instance_id: "open-file:run-0", status: "Completed", started_at: "2026-11-03T10:00:00Z", duration_ms: 800, trigger: "Started when record 9c1d was created", started_by: "bohr@physics.state.edu", writes: 3, version: 1 },
  ],
  next_cursor: null,
  last_7_days: { runs: 2, failed: 1, sentence: "1 of 2 runs failed in the last 7 days (50%)." },
  breaker: { failures_in_a_row: 1, off: false, sentence: "The flow is on. 1 of the last runs failed in a row; it turns off at 20." },
};
const timeline = {
  instance_id: "open-file:run-1", status: "Failed", steps: [
    { step: "a", path: [0], outcome: "done", effects: [{ sentence: "created 1 record in Housing / Logs and set note", records: [{ app: "housing", table: "logs", id: "rec-9", link: "/apps/housing/records/rec-9" }] }] },
    { step: "b", path: [1], outcome: "done", effects: [{ sentence: "created 1 record in Aid / Awards and set amount", records: [{ app: "aid", table: "awards", id: "rec-7", link: null }] }] },
    { step: "c", path: [2], outcome: "failed", effects: [] },
  ],
  first_error: { step: "c", message: "the run would make 3 writes, and the limit is 2 writes per run" },
  retries: [],
  retry: { allowed: true },
};
const versions = { versions: [
  { version: 1, author: "einstein@physics.state.edu", approvers: ["bohr@physics.state.edu"], proposal_id: "p-1", summary: "First version", at: "2026-11-01T10:00:00Z", live: false },
  { version: 2, author: "einstein@physics.state.edu", approvers: [], proposal_id: null, summary: "Second version", at: "2026-11-02T10:00:00Z", live: true },
] };
const compare = {
  from: 1, to: 2,
  steps: [
    { step: "a", change: "changed", path: [0], sentence: "Step 1 is changed." },
    { step: "b", change: "same", path: [1], sentence: "Step 2 is the same." },
    { step: "d", change: "added", path: [2], sentence: "Step 3 is added." },
    { step: "x", change: "removed", path: [2], sentence: "Step 3 of version 1 is removed." },
  ],
  sentences: ["Step 1 is changed.", "Step 3 is added.", "Step 3 of version 1 is removed."],
  from_steps: [step("a", "\"opened\""), step("b", "\"kept\""), step("x", "\"old\"")],
  to_steps: [step("a", "\"opened again\""), step("b", "\"kept\""), step("d", "\"new\"")],
};

let posted: { url: string; body: any }[];
beforeEach(() => {
  vi.restoreAllMocks();
  posted = [];
  global.fetch = vi.fn().mockImplementation((url: string, init?: RequestInit) => {
    const u = String(url);
    const ok = (body: unknown) => Promise.resolve({ ok: true, status: 200, json: async () => body });
    if (init?.method === "POST") {
      posted.push({ url: u, body: JSON.parse(String(init.body ?? "{}")) });
      return ok({ instance_id: "open-file:run-1", status: "Completed", version: 1 });
    }
    if (u.includes("/versions/compare")) return ok(compare);
    if (u.endsWith("/versions")) return ok(versions);
    if (u.includes("/flows/runs/")) return ok(posted.length ? { ...timeline, status: "Completed", retries: [{ by: "einstein@physics.state.edu", reason: "Raised the limit" }], retry: { allowed: false, why: "Only a failed run can be retried." } } : timeline);
    if (u.includes("/runs")) return ok(runs);
    return ok({});
  }) as unknown as typeof fetch;
});

describe("a flow's runs", () => {
  it("lists the runs with the failure rate and the breaker, and opens a timeline", async () => {
    render(<FlowHistory flowId="open-file" appTitle="Housing" />);
    expect(await screen.findByText("1 of 2 runs failed in the last 7 days (50%).")).toBeInTheDocument();
    expect(screen.getByText(/The flow is on\./)).toBeInTheDocument();
    const row = screen.getByRole("row", { name: /Failed.*einstein/ });
    expect(within(row).getByText("Started when record 3f2a9b1c was created")).toBeInTheDocument();
    expect(within(row).getByText("2 writes")).toBeInTheDocument();

    fireEvent.click(within(row).getByRole("button", { name: "Open the run of 4 Nov" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("Step 3 failed: the run would make 3 writes, and the limit is 2 writes per run");
    const tl = screen.getByRole("region", { name: "Run timeline" });
    // A record the viewer may open is a link; one they may not is named only.
    expect(within(tl).getByRole("link", { name: "Open record rec-9" })).toHaveAttribute("href", "/apps/housing/records/rec-9");
    expect(within(tl).queryByRole("link", { name: "Open record rec-7" })).not.toBeInTheDocument();
    expect(within(tl).getByText("a record in aid / awards (rec-7)")).toBeInTheDocument();
  });

  it("retries a failed run with a reason", async () => {
    render(<FlowHistory flowId="open-file" appTitle="Housing" />);
    fireEvent.click(await screen.findByRole("button", { name: "Open the run of 4 Nov" }));
    const retry = await screen.findByRole("button", { name: "Retry this run" });
    expect(retry).toBeDisabled();
    fireEvent.change(screen.getByLabelText("Why retry it"), { target: { value: "Raised the limit" } });
    fireEvent.click(retry);
    await waitFor(() => expect(posted).toHaveLength(1));
    expect(posted[0].url).toBe("/api/v1/flows/runs/open-file%3Arun-1/retry");
    expect(posted[0].body).toEqual({ reason: "Raised the limit" });
    expect(await screen.findByText("The retried run completed.")).toBeInTheDocument();
  });
});

describe("a flow's versions", () => {
  // 4. Comparing two versions marks the changed step as changed in text.
  it("lists versions and marks added, changed and removed steps on the canvas", async () => {
    render(<FlowHistory flowId="open-file" appTitle="Housing" />);
    fireEvent.click(screen.getByRole("tab", { name: "Versions" }));
    const v1 = await screen.findByRole("row", { name: /Version 1/ });
    expect(within(v1).getByText("bohr@physics.state.edu")).toBeInTheDocument();
    fireEvent.change(screen.getByLabelText("Compare version"), { target: { value: "1" } });
    fireEvent.change(screen.getByLabelText("With version"), { target: { value: "2" } });
    fireEvent.click(screen.getByRole("button", { name: "Compare" }));
    const after = await screen.findByRole("group", { name: "Version 2" });
    expect(within(after).getByRole("button", { name: /^step 1:/ })).toHaveAccessibleName(expect.stringContaining("Compared with version 1: changed"));
    expect(within(within(after).getByRole("button", { name: /^step 1:/ })).getByText("Changed")).toBeInTheDocument();
    expect(within(after).getByRole("button", { name: /^step 3:/ })).toHaveAccessibleName(expect.stringContaining("Compared with version 1: added"));
    const before = screen.getByRole("group", { name: "Version 1" });
    expect(within(before).getByRole("button", { name: /^step 3:/ })).toHaveAccessibleName(expect.stringContaining("Compared with version 2: removed"));
    expect(screen.getByText("Step 3 of version 1 is removed.")).toBeInTheDocument();
    // A comparison is read only: there is nothing to add.
    expect(within(after).queryByRole("button", { name: /^Insert a step/ })).not.toBeInTheDocument();
  });
});
