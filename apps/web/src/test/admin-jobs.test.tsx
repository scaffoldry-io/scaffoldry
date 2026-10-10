import "@testing-library/jest-dom";
import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, waitFor, fireEvent } from "@testing-library/react";
import { Jobs } from "../admin/Jobs";
import { apiClient, AdminJobRow, AdminJobsResponse } from "../api";

const mockRunningJob: AdminJobRow = {
  id: "job-1111-2222-3333-4444",
  kind: "export_oscal_catalog",
  owner: "lead.engineer@state.edu",
  state: "running",
  progress: 65,
  attempts: 1,
  created: "2026-10-10T12:00:00Z",
  age: 45,
  last_log_line: "Batch 65/100 processed successfully",
  error: null,
};

const mockFailedJob: AdminJobRow = {
  id: "job-5555-6666-7777-8888",
  kind: "dataset_sync_ferpa",
  owner: "compliance.officer@state.edu",
  state: "failed",
  progress: null,
  attempts: 3,
  created: "2026-10-10T11:00:00Z",
  age: 3600,
  last_log_line: "Database deadlock encountered during transaction",
  error: "Critical failure: connection dropped by remote peer",
};

describe("Admin Jobs Queue Component", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("the panel shows progress as text and a bar", async () => {
    const mockResponse: AdminJobsResponse = {
      rows: [mockRunningJob],
      next_cursor: null,
    };
    vi.spyOn(apiClient, "getAdminJobs").mockResolvedValue(mockResponse);

    render(<Jobs />);

    await waitFor(() => {
      expect(screen.getByTestId("admin-jobs-panel")).toBeInTheDocument();
    });

    // Check progress text
    const progressText = screen.getByTestId("job-progress-text");
    expect(progressText).toHaveTextContent("65%");

    // Check progress bar
    const progressBar = screen.getByTestId("job-progress-bar");
    expect(progressBar).toBeInTheDocument();
    expect(progressBar).toHaveStyle({ width: "65%" });
  });

  it("a failed job shows its error", async () => {
    const mockResponse: AdminJobsResponse = {
      rows: [mockFailedJob],
      next_cursor: null,
    };
    vi.spyOn(apiClient, "getAdminJobs").mockResolvedValue(mockResponse);

    render(<Jobs />);

    await waitFor(() => {
      expect(screen.getByTestId("admin-jobs-panel")).toBeInTheDocument();
    });

    const errorMsg = screen.getByTestId("job-error-message");
    expect(errorMsg).toBeInTheDocument();
    expect(errorMsg).toHaveTextContent("Critical failure: connection dropped by remote peer");
  });

  it("Cancel asks for a reason before submitting audited action", async () => {
    const mockResponse: AdminJobsResponse = {
      rows: [mockRunningJob],
      next_cursor: null,
    };
    vi.spyOn(apiClient, "getAdminJobs").mockResolvedValue(mockResponse);
    const cancelSpy = vi.spyOn(apiClient, "cancelAdminJob").mockResolvedValue({
      ...mockRunningJob,
      state: "cancelled",
      progress_done: 65,
      progress_total: 100,
      log: [],
      cancel_requested: true,
      created_by: mockRunningJob.owner,
      created_at: mockRunningJob.created,
    });

    render(<Jobs />);

    await waitFor(() => {
      expect(screen.getByTestId("job-cancel-btn")).toBeInTheDocument();
    });

    // Initially modal is closed
    expect(screen.queryByTestId("job-cancel-modal")).not.toBeInTheDocument();

    // Click Cancel
    fireEvent.click(screen.getByTestId("job-cancel-btn"));

    // Modal opens asking for reason
    expect(screen.getByTestId("job-cancel-modal")).toBeInTheDocument();
    const reasonInput = screen.getByTestId("job-cancel-reason-input");
    const confirmBtn = screen.getByTestId("job-confirm-cancel-btn");

    // Empty reason disables submission
    expect(confirmBtn).toBeDisabled();

    // Enter audit reason
    fireEvent.change(reasonInput, {
      target: { value: "Task superseded by urgent policy update" },
    });
    expect(confirmBtn).not.toBeDisabled();

    // Submit cancel
    fireEvent.click(confirmBtn);

    await waitFor(() => {
      expect(cancelSpy).toHaveBeenCalledWith(
        mockRunningJob.id,
        "Task superseded by urgent policy update"
      );
    });
  });
});
