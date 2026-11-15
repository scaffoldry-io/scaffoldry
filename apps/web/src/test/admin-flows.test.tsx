import "@testing-library/jest-dom";
import { afterEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { Flows } from "../admin/Flows";
import { apiClient, type AdminFlow } from "../api";

const flow = (over: Partial<AdminFlow> = {}): AdminFlow => ({
  rule_id: "flow-1",
  name: "Open aid file",
  app_slug: "admissions",
  version: 2,
  home_workspace: "ws-a",
  home_workspace_name: "Admissions",
  crosses_workspaces: true,
  reads_sensitive: false,
  parties: [
    { workspace_id: "ws-a", name: "Admissions", approved_by: "co.owner@state.edu", approved_at: "2026-11-10T09:00:00Z" },
    { workspace_id: "ws-b", name: "Financial Aid", approved_by: "aid.owner@state.edu", approved_at: "2026-11-09T15:30:00Z" },
  ],
  grants: "This flow may read Students: email.",
  last_run: { at: "2026-11-14T08:00:00Z", status: "Completed" },
  runs_last_7_days: 10,
  failures_last_7_days: 2,
  state: "Active",
  ...over,
});

afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
});

describe("the Flows panel", () => {
  it("lists a cross-workspace flow with both parties, who approved and when, and its run results", async () => {
    vi.spyOn(apiClient, "adminFlows").mockResolvedValue({ flows: [flow()] });
    render(<Flows />);
    const table = await screen.findByRole("table", { name: "Flows" });
    expect(within(table).getByText("Open aid file")).toBeInTheDocument();
    expect(within(table).getByText(/Financial Aid: aid\.owner@state\.edu/)).toBeInTheDocument();
    expect(within(table).getByText(/Admissions: co\.owner@state\.edu/)).toBeInTheDocument();
    expect(within(table).getByText("This flow may read Students: email.")).toBeInTheDocument();
    expect(within(table).getByText("2 of 10")).toBeInTheDocument();
    expect(within(table).getByText("Active")).toBeInTheDocument();
  });

  it("asks the server for the filter that is ticked", async () => {
    const list = vi.spyOn(apiClient, "adminFlows").mockResolvedValue({ flows: [flow()] });
    render(<Flows />);
    await screen.findByRole("table", { name: "Flows" });
    fireEvent.click(screen.getByLabelText("Reads sensitive data"));
    await waitFor(() => expect(list).toHaveBeenLastCalledWith({ crosses_workspaces: false, reads_sensitive: true }));
    fireEvent.click(screen.getByLabelText("Crosses workspaces"));
    await waitFor(() => expect(list).toHaveBeenLastCalledWith({ crosses_workspaces: true, reads_sensitive: true }));
  });

  it("disables a flow only with a reason, and says what it does first", async () => {
    vi.spyOn(apiClient, "adminFlows").mockResolvedValue({ flows: [flow()] });
    const set = vi.spyOn(apiClient, "setFlowDisabled").mockResolvedValue(undefined);
    render(<Flows />);
    fireEvent.click(await screen.findByRole("button", { name: "Disable Open aid file" }));
    expect(screen.getByText(/stops from its next run, on every server/)).toBeInTheDocument();
    fireEvent.change(screen.getByRole("textbox"), { target: { value: "Aid office asked for a pause." } });
    fireEvent.click(screen.getAllByRole("button", { name: "Disable Open aid file" }).at(-1) as HTMLElement);
    await waitFor(() => expect(set).toHaveBeenCalledWith("flow-1", true, "Aid office asked for a pause."));
  });

  it("offers Enable for a disabled flow, and shows a tripped one as Tripped", async () => {
    vi.spyOn(apiClient, "adminFlows").mockResolvedValue({
      flows: [flow({ state: "Disabled" }), flow({ rule_id: "flow-2", name: "Notify housing", state: "Tripped" })],
    });
    render(<Flows />);
    expect(await screen.findByRole("button", { name: "Enable Open aid file" })).toBeInTheDocument();
    expect(screen.getByText("Tripped")).toBeInTheDocument();
  });

  it("says plainly when no flow matches", async () => {
    vi.spyOn(apiClient, "adminFlows").mockResolvedValue({ flows: [] });
    render(<Flows />);
    expect(await screen.findByText("No flow matches")).toBeInTheDocument();
  });
});
