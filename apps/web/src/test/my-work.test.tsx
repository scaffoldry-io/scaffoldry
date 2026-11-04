import "@testing-library/jest-dom";
import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent, waitFor, within, act } from "@testing-library/react";
import { MyWork } from "../MyWork";
import type { LiveEvent } from "../realtime";

// Workflows phase 6: the My Work page and its review queue. `fetch` is stubbed and the person's own stream is faked.
const live = vi.hoisted(() => ({ listeners: new Set<(e: LiveEvent) => void>() }));
vi.mock("../realtime", async (orig) => ({
  ...(await orig<typeof import("../realtime")>()),
  onMeEvent: (l: (e: LiveEvent) => void) => {
    live.listeners.add(l);
    return () => live.listeners.delete(l);
  },
}));
const emit = (e: LiveEvent) => act(() => live.listeners.forEach((l) => l(e)));

const day = 86_400_000;
type Item = Record<string, unknown>;
const item = (id: string, kind: string, extra: Item = {}): Item => ({
  id, kind, state: "open", title: `${kind} ${id}`, summary: "A step waits", app_slug: "grants", workspace_id: "ws-physics",
  instance_id: `rule:${id}`, step_id: "s", via: "holder", acting_for: null, due_at: new Date(Date.now() + 3 * day).toISOString(),
  created_at: new Date(Date.now() - day).toISOString(), ...extra,
});

interface Stub {
  items: Item[];
  details?: Record<string, unknown>;
  posted: { url: string; body: any }[];
}

function stub(items: Item[], details: Record<string, unknown> = {}): Stub {
  const s: Stub = { items, details, posted: [] };
  global.fetch = vi.fn().mockImplementation((url: string, init?: RequestInit) => {
    const u = String(url);
    const ok = (body: unknown) => Promise.resolve({ ok: true, status: 200, json: async () => body });
    if (init?.method === "POST") {
      s.posted.push({ url: u, body: JSON.parse(String(init.body ?? "{}")) });
      const id = u.split("/me/work/")[1].split("/")[0];
      s.items = s.items.filter((i) => i.id !== id);
      return ok({ instance: { status: "Completed" } });
    }
    if (u.includes("/me/work/summary")) {
      const by_kind: Record<string, number> = {};
      s.items.forEach((i) => (by_kind[i.kind as string] = (by_kind[i.kind as string] ?? 0) + 1));
      return ok({ by_kind, overdue: 0, oldest_days: 1 });
    }
    const one = u.match(/\/me\/work\/([^/?]+)$/);
    if (one) {
      const it = s.items.find((i) => i.id === one[1]) ?? {};
      return ok({ item: it, prompt: "Review the request", history: ["4 Nov 10:00: the process started."], show: { program: "Physics BSc" }, fill: [], reject_comment_required: true, ...(s.details?.[one[1]] as object) });
    }
    return ok({ items: s.items, next_cursor: null });
  }) as unknown as typeof fetch;
  return s;
}

describe("My Work", () => {
  beforeEach(() => {
    vi.restoreAllMocks();
    live.listeners.clear();
  });

  // 1. The four tabs show their items. Counts match the summary.
  it("shows each kind in its tab with the summary's count", async () => {
    stub([item("d1", "decision"), item("d2", "decision"), item("t1", "task"), item("r1", "review"), item("w1", "waiting")]);
    render(<MyWork />);
    expect(await screen.findByRole("tab", { name: "Decisions (2)" })).toBeInTheDocument();
    expect(screen.getByRole("tab", { name: "Tasks (1)" })).toBeInTheDocument();
    expect(screen.getByRole("tab", { name: "Reviews (1)" })).toBeInTheDocument();
    expect(screen.getByRole("tab", { name: "Waiting on others (1)" })).toBeInTheDocument();
    expect(await screen.findAllByTestId(/^work-row-/)).toHaveLength(2);
    fireEvent.click(screen.getByRole("tab", { name: "Tasks (1)" }));
    expect(await screen.findByTestId("work-row-t1")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("tab", { name: "Waiting on others (1)" }));
    expect(await screen.findByTestId("work-row-w1")).toBeInTheDocument();
  });

  it("says plainly when nothing waits", async () => {
    stub([]);
    render(<MyWork />);
    expect(await screen.findByText("Nothing is waiting on you.")).toBeInTheDocument();
  });

  // 2. A delegate item shows `Acting for` text.
  it("marks a delegate's item with who they act for", async () => {
    stub([item("d1", "decision", { via: "delegate", acting_for: "Dr. Rivera" })]);
    render(<MyWork />);
    expect(await within(await screen.findByTestId("work-row-d1")).findByText("Acting for Dr. Rivera")).toBeInTheDocument();
  });

  // 3. Approving moves to the next item and the remaining count goes down. A rejection without a required comment is
  // not sent.
  it("moves to the next item after a decision, and needs a comment to reject", async () => {
    const s = stub([item("d1", "decision"), item("d2", "decision"), item("d3", "decision")]);
    render(<MyWork />);
    fireEvent.click(await screen.findByTestId("work-row-d1"));
    const drawer = await screen.findByRole("dialog", { name: /decision d1/ });
    expect(within(drawer).getByText("3 remaining")).toBeInTheDocument();
    expect(await within(drawer).findByText("Review the request")).toBeInTheDocument();

    fireEvent.click(within(drawer).getByRole("button", { name: "Reject" }));
    expect(s.posted).toHaveLength(0);
    expect(within(drawer).getByText("Say why you reject.")).toBeInTheDocument();

    fireEvent.click(within(drawer).getByRole("button", { name: "Approve" }));
    await waitFor(() => expect(s.posted).toHaveLength(1));
    expect(s.posted[0].url).toBe("/api/v1/me/work/d1/act");
    expect(s.posted[0].body).toEqual({ decision: "approve" });
    const next = await screen.findByRole("dialog", { name: /decision d2/ });
    expect(within(next).getByText("2 remaining")).toBeInTheDocument();

    fireEvent.click(within(next).getByRole("button", { name: "Skip" }));
    expect(await screen.findByRole("dialog", { name: /decision d3/ })).toBeInTheDocument();
  });

  // 4. A `Complete` task form blocks `Done` until the required fields are filled and sends the values.
  it("holds Done until the required fields are filled, then sends them", async () => {
    const s = stub([item("t1", "task")], { t1: { fill: ["award_letter", "notes"], require: ["award_letter"] } });
    render(<MyWork />);
    fireEvent.click(await screen.findByRole("tab", { name: "Tasks (1)" }));
    fireEvent.click(await screen.findByTestId("work-row-t1"));
    const drawer = await screen.findByRole("dialog", { name: /task t1/ });
    const done = await within(drawer).findByRole("button", { name: "Done" });
    expect(done).toBeDisabled();
    expect(within(drawer).getByLabelText("award_letter (required)")).toBeInTheDocument();
    fireEvent.change(within(drawer).getByLabelText("notes"), { target: { value: "On file" } });
    expect(done).toBeDisabled();
    fireEvent.change(within(drawer).getByLabelText("award_letter (required)"), { target: { value: "L-114" } });
    expect(done).toBeEnabled();
    fireEvent.click(done);
    await waitFor(() => expect(s.posted).toHaveLength(1));
    expect(s.posted[0].body).toEqual({ decision: "done", fields: { award_letter: "L-114", notes: "On file" } });
  });

  // 5. An overdue item has the text `overdue` in its accessible name.
  it("names an overdue item as overdue", async () => {
    stub([item("d1", "decision", { due_at: new Date(Date.now() - 3 * day).toISOString() })]);
    render(<MyWork />);
    const row = await screen.findByTestId("work-row-d1");
    expect(row).toHaveAccessibleName(expect.stringMatching(/3 days overdue/));
    expect(within(row).getByText(/3 days overdue/)).toBeInTheDocument();
  });

  // 6. A `work` event adds an item without moving the focus or the scroll position.
  it("adds an item from a work event and keeps the focus and the scroll", async () => {
    const s = stub([item("d1", "decision"), item("d2", "decision")]);
    render(<MyWork />);
    const row = await screen.findByTestId("work-row-d2");
    row.focus();
    const list = screen.getByTestId("work-list");
    list.scrollTop = 120;
    s.items = [...s.items, item("d9", "decision", { due_at: new Date(Date.now() + 9 * day).toISOString() })];
    emit({ event: "work", data: { id: "d9", counts: { decision: 3 } } });
    expect(await screen.findByTestId("work-row-d9")).toBeInTheDocument();
    expect(document.activeElement).toBe(screen.getByTestId("work-row-d2"));
    expect(screen.getByTestId("work-list").scrollTop).toBe(120);
    expect(screen.getByRole("tab", { name: "Decisions (3)" })).toBeInTheDocument();
  });

  // 7. The drawer is operable by keyboard: open with Enter, act, move on, Escape returns focus to the row.
  it("works from the keyboard", async () => {
    const s = stub([item("d1", "decision"), item("d2", "decision")]);
    render(<MyWork />);
    const first = await screen.findByTestId("work-row-d1");
    first.focus();
    fireEvent.keyDown(first, { key: "Enter" });
    const drawer = await screen.findByRole("dialog", { name: /decision d1/ });
    await waitFor(() => expect(drawer.contains(document.activeElement)).toBe(true));
    const approve = within(drawer).getByRole("button", { name: "Approve" });
    approve.focus();
    fireEvent.click(approve);
    await waitFor(() => expect(s.posted).toHaveLength(1));
    const next = await screen.findByRole("dialog", { name: /decision d2/ });
    fireEvent.keyDown(next, { key: "Escape" });
    await waitFor(() => expect(screen.queryByRole("dialog")).not.toBeInTheDocument());
    expect(document.activeElement).toBe(screen.getByTestId("work-row-d2"));
  });
});
