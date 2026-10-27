import "@testing-library/jest-dom";
import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent, within } from "@testing-library/react";
import { Authority } from "../admin/Authority";
import { apiClient, type AuthorityLoad } from "../api";

const ROOT = "00000000-0000-0000-0000-000000000001";

const load: AuthorityLoad = {
  window_days: 90,
  proposals: 40,
  paths: { direct: 28, second_person: 4, approver: 8 },
  unassigned: 2,
  approvers: [
    { eppn: "holder.one@state.edu", display_name: "Holder One", weeks: [6, 0, 2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0], median: 0, p95: 6, largest: 6 },
    { eppn: "holder.two@state.edu", display_name: "Holder Two", weeks: [1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1], median: 1, p95: 1, largest: 1 },
  ],
  decision_time: { decided: 500, median_seconds: 15000, p95_seconds: 28500 },
};

function quiet() {
  vi.spyOn(apiClient, "getCurrentUser").mockResolvedValue({ user: { eppn: "jordan.lee@state.edu", name: "Jordan", affiliation: "central_admin" }, is_impersonating: false, original_admin: null, is_platform_admin: true });
  vi.spyOn(apiClient, "accessGrantAnchors").mockResolvedValue({ units: [{ id: ROOT, name: "Institution", hierarchy: "academic" }], domains: [] } as never);
  vi.spyOn(apiClient, "authorityShow").mockResolvedValue({ organization_id: ROOT, unit_name: "Institution", tiers: [], rows: [], signals: [] });
  vi.spyOn(apiClient, "authorityChanges").mockResolvedValue({ changes: [] });
}

async function openLoad() {
  render(<Authority />);
  fireEvent.click(await screen.findByRole("tab", { name: "Load" }));
  return screen.findByTestId("authority-load");
}

describe("Decision authority load", () => {
  beforeEach(() => {
    vi.restoreAllMocks();
  });

  it("has a Load tab beside the matrix, and the matrix stays the first tab", async () => {
    quiet();
    render(<Authority />);
    const tabs = await screen.findAllByRole("tab");
    expect(tabs.map((t) => t.textContent)).toEqual(["Matrix", "Load"]);
    expect(tabs[0]).toHaveAttribute("aria-selected", "true");
  });

  it("says where the last 90 days of proposals went, and how many nobody could decide", async () => {
    quiet();
    vi.spyOn(apiClient, "authorityLoad").mockResolvedValue(load);
    const panel = await openLoad();
    expect(await within(panel).findByText(/40 proposals in the last 90 days/)).toBeInTheDocument();
    expect(within(panel).getByText(/28 direct, 4 second person, 8 approver/)).toBeInTheDocument();
    expect(within(panel).getByText(/2 proposals have no one to decide them/)).toBeInTheDocument();
  });

  it("shows each approver's busiest week, 95th percentile and median", async () => {
    quiet();
    vi.spyOn(apiClient, "authorityLoad").mockResolvedValue(load);
    const panel = await openLoad();
    const one = await within(panel).findByTestId("authority-load-holder.one@state.edu");
    expect(one).toHaveTextContent("Holder One");
    expect(one).toHaveTextContent("Median 0");
    expect(one).toHaveTextContent("95th percentile 6");
    expect(one).toHaveTextContent("Busiest week 6");
    expect(screen.getByTestId("authority-load-holder.two@state.edu")).toHaveTextContent("Median 1");
  });

  it("gives the time from proposal to decision in words", async () => {
    quiet();
    vi.spyOn(apiClient, "authorityLoad").mockResolvedValue(load);
    const panel = await openLoad();
    const time = await within(panel).findByTestId("authority-load-time");
    expect(time).toHaveTextContent("500 decided");
    expect(time).toHaveTextContent("Median 4 h 10 min");
    expect(time).toHaveTextContent("95th percentile 7 h 55 min");
  });

  it("says so when nothing was decided or sent to a named approver", async () => {
    quiet();
    vi.spyOn(apiClient, "authorityLoad").mockResolvedValue({ ...load, proposals: 0, paths: { direct: 0, second_person: 0, approver: 0 }, unassigned: 0, approvers: [], decision_time: { decided: 0, median_seconds: null, p95_seconds: null } });
    const panel = await openLoad();
    expect(await within(panel).findByText("Nothing was decided in the last 90 days.")).toBeInTheDocument();
    expect(within(panel).getByText("No named approver received a proposal.")).toBeInTheDocument();
    expect(within(panel).queryByText(/have no one to decide/)).not.toBeInTheDocument();
  });

  it("shows an error with a retry when the load cannot be read", async () => {
    quiet();
    const read = vi.spyOn(apiClient, "authorityLoad").mockRejectedValueOnce(new Error("down")).mockResolvedValue(load);
    render(<Authority />);
    fireEvent.click(await screen.findByRole("tab", { name: "Load" }));
    fireEvent.click(await screen.findByRole("button", { name: /try again|retry/i }));
    expect(await screen.findByText(/40 proposals in the last 90 days/)).toBeInTheDocument();
    expect(read).toHaveBeenCalledTimes(2);
  });
});
