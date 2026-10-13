import "@testing-library/jest-dom";
import { describe, it, expect, vi, afterEach } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import { Overview } from "../admin/Overview";
import { apiClient, type AdminOverviewData } from "../api";

function overview(files: AdminOverviewData["files"]): AdminOverviewData {
  return {
    server: { version: "0.1.0", applied_migrations: [], database_worker_count: 1 },
    people: { active: 1, on_hold: 0, inactive: 0, platform_admins: 1, active_tokens: {} },
    organization: { unit_count: 1 },
    workspaces: { workspace_count: 1, app_count: 1 },
    processes: { waiting_instance_count: 0 },
    ledger: { entry_count: 1, head_hash: "abc" },
    files,
  };
}

describe("the file scanning card on the Overview", () => {
  afterEach(() => vi.restoreAllMocks());

  it("says in a Banner that files are not scanned when no scanner is set up", async () => {
    vi.spyOn(apiClient, "getAdminOverview").mockResolvedValue(
      overview({ scanner: { configured: false, reachable: false, last_result: null }, error: 0, skipped: 7, pending: 0, infected: 0 }),
    );
    render(<Overview />);
    const card = await screen.findByTestId("admin-overview-files-card");
    expect(card).toHaveTextContent("No virus scanner is set up. Uploaded files are not scanned.");
    expect(screen.getByTestId("overview-scanner-state")).toHaveTextContent("Not set up");
    expect(screen.getByTestId("overview-files-skipped")).toHaveTextContent("7");
  });

  it("shows a reachable scanner, its last result, and the files with an error", async () => {
    vi.spyOn(apiClient, "getAdminOverview").mockResolvedValue(
      overview({ scanner: { configured: true, reachable: true, last_result: "clean" }, error: 2, skipped: 0, pending: 0, infected: 1 }),
    );
    render(<Overview />);
    await waitFor(() => expect(screen.getByTestId("overview-scanner-state")).toHaveTextContent("Reachable"));
    expect(screen.getByTestId("overview-scanner-last")).toHaveTextContent("clean");
    expect(screen.getByTestId("overview-files-error")).toHaveTextContent("2");
    expect(screen.queryByRole("status")).not.toBeInTheDocument();
  });

  it("warns in an alert when the scanner does not answer", async () => {
    vi.spyOn(apiClient, "getAdminOverview").mockResolvedValue(
      overview({ scanner: { configured: true, reachable: false, last_result: "error" }, error: 3, skipped: 0, pending: 4, infected: 0 }),
    );
    render(<Overview />);
    expect(await screen.findByRole("alert")).toHaveTextContent("The virus scanner is not answering");
  });
});
