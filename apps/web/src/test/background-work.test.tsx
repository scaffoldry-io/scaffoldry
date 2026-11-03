import "@testing-library/jest-dom";
import { describe, it, expect, vi, afterEach } from "vitest";
import { act, render, screen } from "@testing-library/react";
import { BackgroundWork, type BackgroundJob } from "../ui";
import type { LiveEvent } from "../realtime";

const job = (over: Partial<BackgroundJob> = {}): BackgroundJob => ({
  id: "j1",
  kind: "recalc_field",
  title: "Recalculating a field",
  description: "Writes a calculated field's values again after its definition changed.",
  state: "running",
  progress_done: 40,
  progress_total: 100,
  ...over,
});

/** The person's own stream, faked. */
function stream() {
  const listeners = new Set<(e: LiveEvent) => void>();
  return {
    subscribe: (l: (e: LiveEvent) => void) => {
      listeners.add(l);
      return () => {
        listeners.delete(l);
      };
    },
    emit: (e: LiveEvent) => listeners.forEach((l) => l(e)),
    size: () => listeners.size,
  };
}

// Real-time phase 3: the indicator follows `job` events on the person's stream. It no longer polls.
describe("BackgroundWork", () => {
  afterEach(() => {
    vi.useRealTimers();
  });

  it("appears while a job runs, with its plain name and progress, and goes when an event says it finished", async () => {
    vi.useFakeTimers();
    const live = stream();
    const fetchJobs = vi.fn().mockResolvedValueOnce([job()]).mockResolvedValue([job({ state: "done", progress_done: 100 })]);
    render(<BackgroundWork fetchJobs={fetchJobs} subscribe={live.subscribe} />);

    await act(async () => {
      await vi.advanceTimersByTimeAsync(0);
    });
    const region = screen.getByRole("status");
    expect(region).toHaveTextContent("Background work");
    expect(region).toHaveTextContent("Recalculating a field");
    expect(region).toHaveTextContent("40%");
    expect(screen.getByText("Writes a calculated field's values again after its definition changed.")).toBeInTheDocument();

    // Time alone asks nothing.
    await act(async () => {
      await vi.advanceTimersByTimeAsync(30_000);
    });
    expect(fetchJobs).toHaveBeenCalledTimes(1);

    await act(async () => {
      live.emit({ event: "job", data: { id: "j1", state: "done", percent: 100 } });
      await vi.advanceTimersByTimeAsync(0);
    });
    expect(fetchJobs).toHaveBeenCalledTimes(2);
    expect(screen.queryByRole("status")).not.toBeInTheDocument();
  });

  it("says a queued job is waiting, and shows nothing when no job is active", async () => {
    vi.useFakeTimers();
    const fetchJobs = vi.fn().mockResolvedValue([job({ state: "queued", progress_total: null, progress_done: 0 })]);
    const { unmount } = render(<BackgroundWork fetchJobs={fetchJobs} subscribe={stream().subscribe} />);
    await act(async () => {
      await vi.advanceTimersByTimeAsync(0);
    });
    expect(screen.getByRole("status")).toHaveTextContent("Waiting");
    unmount();

    render(<BackgroundWork fetchJobs={vi.fn().mockResolvedValue([job({ state: "failed" }), job({ id: "j2", state: "cancelled" })])} subscribe={stream().subscribe} />);
    await act(async () => {
      await vi.advanceTimersByTimeAsync(0);
    });
    expect(screen.queryByRole("status")).not.toBeInTheDocument();
  });

  it("stops listening when it is removed", async () => {
    vi.useFakeTimers();
    const live = stream();
    const fetchJobs = vi.fn().mockResolvedValue([job()]);
    const { unmount } = render(<BackgroundWork fetchJobs={fetchJobs} subscribe={live.subscribe} />);
    await act(async () => {
      await vi.advanceTimersByTimeAsync(0);
    });
    expect(live.size()).toBe(1);
    unmount();
    expect(live.size()).toBe(0);
    expect(fetchJobs).toHaveBeenCalledTimes(1);
  });
});
