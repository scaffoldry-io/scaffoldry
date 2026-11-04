import "@testing-library/jest-dom";
import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent, within } from "@testing-library/react";
import { ProcessInsights, ProcessInsightsPanel } from "../ProcessInsights";
import { apiClient, type ProcessAnalytics } from "../api";

const DAY = 86400;

const analytics: ProcessAnalytics = {
  rules: [
    {
      rule_id: "proposal-approval",
      name: "Proposal approval",
      finished: 10,
      outcomes: { Completed: 0.8, Rejected: 0.2, Failed: 0, Cancelled: 0 },
      steps: [
        {
          step_id: "dean",
          name: "Dean review",
          entered: 12,
          median_seconds: 6 * DAY,
          p90_seconds: 11 * DAY,
          waiting: 2,
          waiting_by_age: { under_1_day: 0, "1_to_3_days": 1, "3_to_7_days": 0, "7_to_14_days": 1, "14_to_30_days": 0, over_30_days: 0 },
        },
        {
          step_id: "chair",
          name: "Chair review",
          entered: 12,
          median_seconds: 3 * 3600,
          p90_seconds: null,
          waiting: 0,
          waiting_by_age: { under_1_day: 0, "1_to_3_days": 0, "3_to_7_days": 0, "7_to_14_days": 0, "14_to_30_days": 0, over_30_days: 0 },
        },
      ],
    },
  ],
  slowest: "Dean review takes a median of 6 days.",
};

// Process v2 phase 1: a table by step in words and numbers, a bar of waiting by age, and the slowest step.
describe("process insights", () => {
  beforeEach(() => vi.restoreAllMocks());

  it("shows each step in words and numbers, the waiting bar, and the slowest step", async () => {
    const get = vi.spyOn(apiClient, "getProcessAnalytics").mockResolvedValue(analytics);
    render(<ProcessInsights appSlug="grants" />);
    expect(await screen.findByText("Dean review takes a median of 6 days.")).toBeInTheDocument();
    expect(get).toHaveBeenCalledWith("grants", {});

    const dean = screen.getByRole("row", { name: /Dean review/ });
    expect(within(dean).getByText("6 days")).toBeInTheDocument();
    expect(within(dean).getByText("11 days")).toBeInTheDocument();
    expect(within(dean).getByText("12")).toBeInTheDocument();
    const chair = screen.getByRole("row", { name: /Chair review/ });
    expect(within(chair).getByText("3 hours")).toBeInTheDocument();
    expect(within(chair).getByText("Not enough yet")).toBeInTheDocument();

    const bar = screen.getByRole("img", { name: "Dean review: 2 waiting. 1 for 1 to 3 days, 1 for 7 to 14 days." });
    expect(bar).toBeInTheDocument();
    expect(screen.getByText("8 of 10 finished were completed. 2 were rejected.")).toBeInTheDocument();
  });

  it("asks again for the chosen dates", async () => {
    const get = vi.spyOn(apiClient, "getProcessAnalytics").mockResolvedValue(analytics);
    render(<ProcessInsights appSlug="grants" />);
    await screen.findByText("Dean review takes a median of 6 days.");
    fireEvent.change(screen.getByLabelText("From"), { target: { value: "2026-10-01" } });
    fireEvent.change(screen.getByLabelText("To"), { target: { value: "2026-10-31" } });
    fireEvent.click(screen.getByRole("button", { name: "Show" }));
    expect(get).toHaveBeenLastCalledWith("grants", { from: "2026-10-01", to: "2026-10-31" });
  });

  it("says so when nothing has run yet", async () => {
    vi.spyOn(apiClient, "getProcessAnalytics").mockResolvedValue({ rules: [], slowest: null });
    render(<ProcessInsights appSlug="grants" />);
    expect(await screen.findByText("No process has run in this app yet.")).toBeInTheDocument();
  });

  it("lets the console pick an app", async () => {
    vi.spyOn(apiClient, "listMyApps").mockResolvedValue([{ slug: "grants", title: "Grants" } as never]);
    const get = vi.spyOn(apiClient, "getProcessAnalytics").mockResolvedValue(analytics);
    render(<ProcessInsightsPanel />);
    fireEvent.change(await screen.findByLabelText("App"), { target: { value: "grants" } });
    expect(await screen.findByText("Dean review takes a median of 6 days.")).toBeInTheDocument();
    expect(get).toHaveBeenCalledWith("grants", {});
  });
});
