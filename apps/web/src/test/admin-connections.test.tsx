import "@testing-library/jest-dom";
import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent, waitFor, within } from "@testing-library/react";
import { Connections } from "../admin/Connections";
import { Overview } from "../admin/Overview";
import { apiClient, type AdminOverviewData, type DataConnection, type DataConnectionRun } from "../api";

const healthy: DataConnection = {
  id: "c1",
  name: "Programs",
  kind: "file_drop",
  dataset_app: "programs",
  dataset_table: "programs",
  state: "Healthy",
  enabled: true,
  review_note: null,
  last_result: "Added 12, changed 3, removed 1",
  last_success: "2026-11-12T02:30:00Z",
  next_run: "2026-11-13T02:30:00Z",
  last_run: { id: "r1", state: "done", started_at: "2026-11-12T02:30:00Z", finished_at: "2026-11-12T02:30:05Z", added: 12, changed: 3, removed: 1, unchanged: 400, error: null },
};

const failing: DataConnection = {
  ...healthy,
  id: "c2",
  name: "Faculty",
  state: "Failing",
  last_result: "The file sis/faculty.csv is not in the drop directory",
  last_run: { ...healthy.last_run!, id: "r2", state: "failed", error: "The file sis/faculty.csv is not in the drop directory" },
};

const run = (n: number, over: Partial<DataConnectionRun> = {}): DataConnectionRun => ({
  id: `run-${n}`,
  state: "done",
  started_at: "2026-11-12T02:30:00Z",
  finished_at: "2026-11-12T02:30:05Z",
  added: n,
  changed: 0,
  removed: 0,
  unchanged: 0,
  error: null,
  ...over,
});

// Connections phase 6, the panel.
describe("the Connections panel", () => {
  beforeEach(() => vi.restoreAllMocks());

  it("lists each connection with its kind, dataset, last result in words, state and next run", async () => {
    vi.spyOn(apiClient, "listDataConnections").mockResolvedValue([healthy]);
    render(<Connections />);
    const row = (await screen.findByText("Programs")).closest("tr") as HTMLElement;
    expect(within(row).getByText("file_drop")).toBeInTheDocument();
    expect(within(row).getByText("programs")).toBeInTheDocument();
    expect(within(row).getByText("Added 12, changed 3, removed 1")).toBeInTheDocument();
    expect(within(row).getByText("Healthy")).toBeInTheDocument();
    expect(within(row).getByTestId("next-run")).not.toHaveTextContent("none");
    expect(within(row).getByTestId("last-success")).not.toHaveTextContent("never");
  });

  it("shows a failing connection with its error", async () => {
    vi.spyOn(apiClient, "listDataConnections").mockResolvedValue([failing]);
    render(<Connections />);
    const row = (await screen.findByText("Faculty")).closest("tr") as HTMLElement;
    expect(within(row).getByText("Failing")).toBeInTheDocument();
    expect(within(row).getByText("The file sis/faculty.csv is not in the drop directory")).toBeInTheDocument();
  });

  it("says why a connection needs review", async () => {
    vi.spyOn(apiClient, "listDataConnections").mockResolvedValue([{ ...failing, state: "Needs review", review_note: "The proposal p1 for the change in the source is waiting for approval." }]);
    render(<Connections />);
    expect(await screen.findByText("Needs review")).toBeInTheDocument();
    expect(screen.getByText(/waiting for approval/)).toBeInTheDocument();
  });

  it("runs a connection now and shows the new result", async () => {
    const list = vi.spyOn(apiClient, "listDataConnections").mockResolvedValueOnce([healthy]).mockResolvedValue([{ ...healthy, last_result: "No change since the last run" }]);
    const go = vi.spyOn(apiClient, "runDataConnection").mockResolvedValue(run(1, { state: "unchanged" }));
    render(<Connections />);
    fireEvent.click(await screen.findByRole("button", { name: "Run now Programs" }));
    await waitFor(() => expect(go).toHaveBeenCalledWith("c1"));
    expect(await screen.findByText("No change since the last run")).toBeInTheDocument();
    expect(list).toHaveBeenCalledTimes(2);
  });

  it("disables a connection only with a reason, and offers Enable for one that is off", async () => {
    vi.spyOn(apiClient, "listDataConnections").mockResolvedValue([healthy, { ...healthy, id: "c3", name: "Courses", state: "Disabled", enabled: false, next_run: null }]);
    const set = vi.spyOn(apiClient, "setDataConnectionEnabled").mockResolvedValue(undefined);
    render(<Connections />);
    fireEvent.click(await screen.findByRole("button", { name: "Disable Programs" }));
    const confirm = screen.getByRole("button", { name: "Disable Programs" });
    expect(confirm).toBeDisabled();
    fireEvent.change(screen.getByRole("textbox", { name: /reason/i }), { target: { value: "The registrar is moving the export" } });
    fireEvent.click(confirm);
    await waitFor(() => expect(set).toHaveBeenCalledWith("c1", false, "The registrar is moving the export"));
    expect(screen.getByRole("button", { name: "Enable Courses" })).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Disable Courses" })).not.toBeInTheDocument();
    expect(screen.getByTestId("next-run-c3")).toHaveTextContent("none");
  });

  it("opens the run history, newest first, up to the last 100 runs", async () => {
    vi.spyOn(apiClient, "listDataConnections").mockResolvedValue([healthy]);
    const runs = Array.from({ length: 100 }, (_, i) => run(i + 1));
    runs[1] = run(2, { state: "failed", error: "The source is not reachable", added: 0 });
    const get = vi.spyOn(apiClient, "dataConnectionRuns").mockResolvedValue(runs);
    render(<Connections />);
    fireEvent.click(await screen.findByRole("button", { name: "Runs of Programs" }));
    await waitFor(() => expect(get).toHaveBeenCalledWith("c1"));
    const history = await screen.findByRole("list", { name: "Run history" });
    expect(within(history).getAllByRole("listitem")).toHaveLength(100);
    expect(within(history).getByText("The source is not reachable")).toBeInTheDocument();
    expect(within(history).getAllByText(/^Added \d+$/).length).toBeGreaterThan(90);
  });

  it("opens the column map with a sample of the last file", async () => {
    vi.spyOn(apiClient, "listDataConnections").mockResolvedValue([healthy]);
    const get = vi.spyOn(apiClient, "dataConnectionMapping").mockResolvedValue({
      columns: [
        { source: "Code", field: "code", type: "Text", key: true, sample: ["P1", "P2", "P3"] },
        { source: "Credits", field: "credits", type: "Number", key: false, sample: [3, 4, 3] },
      ],
      changes: "A column that the source adds, drops or changes comes as a proposal. Nothing here edits the map.",
    });
    render(<Connections />);
    fireEvent.click(await screen.findByRole("button", { name: "Edit mapping of Programs" }));
    await waitFor(() => expect(get).toHaveBeenCalledWith("c1"));
    const map = await screen.findByRole("table", { name: "Column map" });
    expect(within(map).getByText("P1, P2, P3")).toBeInTheDocument();
    expect(within(map).getByText("3, 4, 3")).toBeInTheDocument();
    expect(within(map).getByText("Number")).toBeInTheDocument();
    expect(screen.getByText(/comes as a proposal/)).toBeInTheDocument();
  });
});

const overview = (connections: AdminOverviewData["connections"]): AdminOverviewData => ({
  server: { version: "0.1.0", applied_migrations: [], database_worker_count: 4 },
  people: { active: 3, on_hold: 0, inactive: 0, platform_admins: 1, active_tokens: {} },
  organization: { unit_count: 1 },
  workspaces: { workspace_count: 1, app_count: 1 },
  processes: { waiting_instance_count: 0 },
  ledger: { entry_count: 1, head_hash: "abc" },
  connections,
});

describe("the Overview's connections card", () => {
  beforeEach(() => vi.restoreAllMocks());

  it("counts connections by health and shows the age of the oldest successful sync", async () => {
    vi.spyOn(apiClient, "getAdminOverview").mockResolvedValue(overview({ healthy: 5, failing: 2, needs_review: 1, disabled: 3, oldest_success_age_secs: 2 * 86400 + 3600 }));
    render(<Overview />);
    const card = await screen.findByTestId("admin-overview-connections-card");
    expect(within(card).getByTestId("overview-connections-healthy")).toHaveTextContent("5");
    expect(within(card).getByTestId("overview-connections-failing")).toHaveTextContent("2");
    expect(within(card).getByTestId("overview-connections-review")).toHaveTextContent("1");
    expect(within(card).getByTestId("overview-connections-oldest")).toHaveTextContent("2 days");
  });

  it("says there is no sync yet when none has succeeded", async () => {
    vi.spyOn(apiClient, "getAdminOverview").mockResolvedValue(overview({ healthy: 0, failing: 0, needs_review: 0, disabled: 0, oldest_success_age_secs: null }));
    render(<Overview />);
    expect(await screen.findByTestId("overview-connections-oldest")).toHaveTextContent("none yet");
  });
});
