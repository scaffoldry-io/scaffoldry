import "@testing-library/jest-dom";
import { describe, it, expect, vi, afterEach } from "vitest";
import { act, render, screen } from "@testing-library/react";
import { BackgroundWork, type BackgroundJob } from "../ui";

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

describe("BackgroundWork", () => {
  afterEach(() => {
    vi.useRealTimers();
  });

  it("appears while a job runs, with its plain name and progress, and goes when it finishes", async () => {
    vi.useFakeTimers();
    const fetchJobs = vi.fn().mockResolvedValueOnce([job()]).mockResolvedValue([job({ state: "done", progress_done: 100 })]);
    render(<BackgroundWork fetchJobs={fetchJobs} />);

    await act(async () => {
      await vi.advanceTimersByTimeAsync(0);
    });
    const region = screen.getByRole("status");
    expect(region).toHaveTextContent("Background work");
    expect(region).toHaveTextContent("Recalculating a field");
    expect(region).toHaveTextContent("40%");
    expect(screen.getByText("Writes a calculated field's values again after its definition changed.")).toBeInTheDocument();

    await act(async () => {
      await vi.advanceTimersByTimeAsync(5000);
    });
    expect(fetchJobs).toHaveBeenCalledTimes(2);
    expect(screen.queryByRole("status")).not.toBeInTheDocument();
  });

  it("says a queued job is waiting, and shows nothing when no job is active", async () => {
    vi.useFakeTimers();
    const fetchJobs = vi.fn().mockResolvedValue([job({ state: "queued", progress_total: null, progress_done: 0 })]);
    const { unmount } = render(<BackgroundWork fetchJobs={fetchJobs} />);
    await act(async () => {
      await vi.advanceTimersByTimeAsync(0);
    });
    expect(screen.getByRole("status")).toHaveTextContent("Waiting");
    unmount();

    render(<BackgroundWork fetchJobs={vi.fn().mockResolvedValue([job({ state: "failed" }), job({ id: "j2", state: "cancelled" })])} />);
    await act(async () => {
      await vi.advanceTimersByTimeAsync(0);
    });
    expect(screen.queryByRole("status")).not.toBeInTheDocument();
  });

  it("stops polling when it is removed", async () => {
    vi.useFakeTimers();
    const fetchJobs = vi.fn().mockResolvedValue([job()]);
    const { unmount } = render(<BackgroundWork fetchJobs={fetchJobs} />);
    await act(async () => {
      await vi.advanceTimersByTimeAsync(0);
    });
    unmount();
    await act(async () => {
      await vi.advanceTimersByTimeAsync(20000);
    });
    expect(fetchJobs).toHaveBeenCalledTimes(1);
  });
});
