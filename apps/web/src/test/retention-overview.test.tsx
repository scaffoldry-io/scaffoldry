import "@testing-library/jest-dom";
import { describe, it, expect, vi, afterEach } from "vitest";
import { render, screen } from "@testing-library/react";
import { Overview } from "../admin/Overview";
import { apiClient, type AdminOverviewData } from "../api";

function overview(retention: AdminOverviewData["retention"]): AdminOverviewData {
  return {
    server: { version: "0.1.0", applied_migrations: [], database_worker_count: 1 },
    people: { active: 1, on_hold: 0, inactive: 0, platform_admins: 1, active_tokens: {} },
    organization: { unit_count: 1 },
    workspaces: { workspace_count: 1, app_count: 1 },
    processes: { waiting_instance_count: 0 },
    ledger: { entry_count: 1, head_hash: "abc" },
    retention,
  };
}

describe("retention on the Overview", () => {
  afterEach(() => vi.restoreAllMocks());

  it("says in a Banner while a retention policy still has the placeholder basis", async () => {
    vi.spyOn(apiClient, "getAdminOverview").mockResolvedValue(overview({ placeholder: true }));
    render(<Overview />);
    expect(await screen.findByText(/Retention policies still have placeholder values/)).toBeInTheDocument();
  });

  it("says nothing once the records officer has set every policy", async () => {
    vi.spyOn(apiClient, "getAdminOverview").mockResolvedValue(overview({ placeholder: false }));
    render(<Overview />);
    await screen.findByText("abc", { exact: false }).catch(() => undefined);
    expect(screen.queryByText(/Retention policies still have placeholder values/)).not.toBeInTheDocument();
  });
});
