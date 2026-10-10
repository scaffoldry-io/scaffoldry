import { useState, useEffect } from "react";
import { PageHeader, StatusBadge, Skeleton, ErrorState, Drawer } from "../ui";
import { apiClient, type AdminJobRow, type AdminJobDetail } from "../api";

export function Jobs() {
  const [jobs, setJobs] = useState<AdminJobRow[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  // Filters
  const [stateFilter, setStateFilter] = useState<string>("all");
  const [kindFilter, setKindFilter] = useState<string>("");
  const [ownerFilter, setOwnerFilter] = useState<string>("");

  // Modals & Drawers
  const [selectedJob, setSelectedJob] = useState<AdminJobDetail | null>(null);

  // Cancel action state
  const [cancellingJob, setCancellingJob] = useState<AdminJobRow | null>(null);
  const [cancelReason, setCancelReason] = useState("");
  const [actionLoading, setActionLoading] = useState(false);
  const [actionError, setActionError] = useState<string | null>(null);

  // Retry action state
  const [retryingJob, setRetryingJob] = useState<AdminJobRow | null>(null);
  const [retryReason, setRetryReason] = useState("");

  const fetchJobs = async () => {
    setLoading(true);
    setError(null);
    try {
      const res = await apiClient.getAdminJobs({
        state: stateFilter === "all" ? undefined : stateFilter,
        kind: kindFilter.trim() || undefined,
        owner: ownerFilter.trim() || undefined,
      });
      setJobs(res.rows);
    } catch (err: unknown) {
      setError(err instanceof Error ? err.message : "Failed to load jobs");
    } finally {
      setLoading(false);
    }
  };

  useEffect(() => {
    fetchJobs();
  }, [stateFilter]);

  const handleOpenDetail = async (id: string) => {
    try {
      const detail = await apiClient.getAdminJob(id);
      setSelectedJob(detail);
    } catch (err: unknown) {
      setError(err instanceof Error ? err.message : "Failed to load job details");
    }
  };

  const handleConfirmCancel = async (e: React.FormEvent) => {
    e.preventDefault();
    if (!cancellingJob || !cancelReason.trim()) return;

    setActionLoading(true);
    setActionError(null);
    try {
      await apiClient.cancelAdminJob(cancellingJob.id, cancelReason.trim());
      setCancellingJob(null);
      setCancelReason("");
      await fetchJobs();
    } catch (err: unknown) {
      setActionError(err instanceof Error ? err.message : "Failed to cancel job");
    } finally {
      setActionLoading(false);
    }
  };

  const handleConfirmRetry = async (e: React.FormEvent) => {
    e.preventDefault();
    if (!retryingJob || !retryReason.trim()) return;

    setActionLoading(true);
    setActionError(null);
    try {
      await apiClient.retryAdminJob(retryingJob.id, retryReason.trim());
      setRetryingJob(null);
      setRetryReason("");
      await fetchJobs();
    } catch (err: unknown) {
      setActionError(err instanceof Error ? err.message : "Failed to retry job");
    } finally {
      setActionLoading(false);
    }
  };

  const getStatusTone = (state: string): "ok" | "warn" | "bad" | "neutral" => {
    switch (state) {
      case "done":
        return "ok";
      case "running":
      case "queued":
        return "warn";
      case "failed":
        return "bad";
      default:
        return "neutral";
    }
  };

  return (
    <div className="space-y-6" data-testid="admin-jobs-panel">
      <PageHeader
        title="Background Jobs & Queue Management"
        description="Monitor asynchronous workload execution, inspect task logs, and manage resilient job lifecycles."
        actions={
          <button
            type="button"
            onClick={fetchJobs}
            data-testid="refresh-jobs-btn"
            className="px-3 py-1.5 text-xs font-medium rounded-lg bg-white dark:bg-slate-800 border border-slate-300 dark:border-slate-700 hover:bg-slate-50 dark:hover:bg-slate-700 text-slate-700 dark:text-slate-300 transition-colors shadow-xs"
          >
            ↻ Refresh Queue
          </button>
        }
      />

      {/* FILTERS */}
      <div className="bg-white dark:bg-slate-900 border border-slate-200 dark:border-slate-800 rounded-xl p-4 shadow-xs flex flex-wrap gap-4 items-center">
        <div>
          <label htmlFor="state-filter" className="block text-xs font-semibold text-slate-600 dark:text-slate-400 mb-1">
            Status
          </label>
          <select
            id="state-filter"
            data-testid="job-state-filter"
            value={stateFilter}
            onChange={(e) => setStateFilter(e.target.value)}
            className="text-xs bg-slate-50 dark:bg-slate-800 border border-slate-300 dark:border-slate-700 rounded-lg px-2.5 py-1.5 text-slate-900 dark:text-white"
          >
            <option value="all">All States</option>
            <option value="queued">Queued</option>
            <option value="running">Running</option>
            <option value="done">Done</option>
            <option value="failed">Failed</option>
            <option value="cancelled">Cancelled</option>
          </select>
        </div>

        <div>
          <label htmlFor="kind-filter" className="block text-xs font-semibold text-slate-600 dark:text-slate-400 mb-1">
            Job Kind
          </label>
          <input
            id="kind-filter"
            type="text"
            data-testid="job-kind-filter"
            placeholder="e.g. export_oscal"
            value={kindFilter}
            onChange={(e) => setKindFilter(e.target.value)}
            onKeyDown={(e) => e.key === "Enter" && fetchJobs()}
            className="text-xs bg-slate-50 dark:bg-slate-800 border border-slate-300 dark:border-slate-700 rounded-lg px-2.5 py-1.5 text-slate-900 dark:text-white placeholder-slate-400"
          />
        </div>

        <div>
          <label htmlFor="owner-filter" className="block text-xs font-semibold text-slate-600 dark:text-slate-400 mb-1">
            Owner (EPPN)
          </label>
          <input
            id="owner-filter"
            type="text"
            data-testid="job-owner-filter"
            placeholder="e.g. admin@state.edu"
            value={ownerFilter}
            onChange={(e) => setOwnerFilter(e.target.value)}
            onKeyDown={(e) => e.key === "Enter" && fetchJobs()}
            className="text-xs bg-slate-50 dark:bg-slate-800 border border-slate-300 dark:border-slate-700 rounded-lg px-2.5 py-1.5 text-slate-900 dark:text-white placeholder-slate-400"
          />
        </div>

        <div className="flex items-end self-end">
          <button
            type="button"
            onClick={fetchJobs}
            data-testid="apply-filters-btn"
            className="px-3 py-1.5 text-xs font-medium rounded-lg bg-amber-600 hover:bg-amber-700 text-white transition-colors shadow-xs"
          >
            Apply Filters
          </button>
        </div>
      </div>

      {/* MAIN CONTENT */}
      {loading ? (
        <div data-testid="jobs-loading" className="space-y-4">
          <Skeleton rows={5} />
        </div>
      ) : error ? (
        <div data-testid="jobs-error">
          <ErrorState message={error} onRetry={fetchJobs} />
        </div>
      ) : jobs.length === 0 ? (
        <div
          data-testid="jobs-empty-state"
          className="bg-white dark:bg-slate-900 border border-slate-200 dark:border-slate-800 rounded-xl p-12 text-center text-slate-500 dark:text-slate-400"
        >
          No jobs found matching the specified criteria.
        </div>
      ) : (
        <div className="bg-white dark:bg-slate-900 border border-slate-200 dark:border-slate-800 rounded-xl overflow-hidden shadow-xs">
          <div className="overflow-x-auto">
            <table className="w-full text-left text-xs text-slate-600 dark:text-slate-300">
              <thead className="bg-slate-50 dark:bg-slate-800/60 text-slate-500 dark:text-slate-400 uppercase font-semibold border-b border-slate-200 dark:border-slate-800">
                <tr>
                  <th className="px-4 py-3">Job ID &amp; Kind</th>
                  <th className="px-4 py-3">Owner</th>
                  <th className="px-4 py-3">Status</th>
                  <th className="px-4 py-3">Progress</th>
                  <th className="px-4 py-3">Attempts</th>
                  <th className="px-4 py-3">Age / Created</th>
                  <th className="px-4 py-3">Last Log Entry</th>
                  <th className="px-4 py-3 text-right">Actions</th>
                </tr>
              </thead>
              <tbody className="divide-y divide-slate-100 dark:divide-slate-800/60 font-mono">
                {jobs.map((job) => (
                  <tr
                    key={job.id}
                    data-testid={`job-row-${job.id}`}
                    className="hover:bg-slate-50/70 dark:hover:bg-slate-800/40 transition-colors"
                  >
                    <td className="px-4 py-3">
                      <div className="font-semibold text-slate-900 dark:text-white font-sans">{job.kind}</div>
                      <div className="text-[11px] text-slate-400">{job.id.slice(0, 8)}...</div>
                    </td>
                    <td className="px-4 py-3 font-sans text-slate-700 dark:text-slate-300">{job.owner}</td>
                    <td className="px-4 py-3 font-sans">
                      <StatusBadge tone={getStatusTone(job.state)} text={job.state.toUpperCase()} />
                      {job.state === "failed" && (
                        <div
                          data-testid="job-error-message"
                          className="mt-1.5 p-2 rounded bg-rose-50 dark:bg-rose-950/40 border border-rose-200 dark:border-rose-900/60 text-rose-700 dark:text-rose-300 text-xs font-sans leading-tight"
                        >
                          <span className="font-semibold">Error:</span> {job.error || "Execution terminated unexpectedly"}
                        </div>
                      )}
                    </td>
                    <td className="px-4 py-3 font-sans">
                      <div className="flex items-center gap-2">
                        <span data-testid="job-progress-text" className="font-medium text-slate-900 dark:text-white min-w-10">
                          {job.progress !== null ? `${Math.round(job.progress)}%` : "—"}
                        </span>
                        {job.progress !== null && (
                          <div
                            data-testid="job-progress-bar-container"
                            className="w-20 bg-slate-200 dark:bg-slate-700 rounded-full h-2 overflow-hidden shrink-0"
                          >
                            <div
                              data-testid="job-progress-bar"
                              className="bg-amber-500 h-2 rounded-full transition-all duration-300"
                              style={{ width: `${Math.min(100, Math.max(0, job.progress))}%` }}
                            />
                            <progress
                              className="sr-only"
                              value={Math.min(100, Math.max(0, job.progress))}
                              max={100}
                            />
                          </div>
                        )}
                      </div>
                    </td>
                    <td className="px-4 py-3 text-slate-700 dark:text-slate-300">{job.attempts}</td>
                    <td className="px-4 py-3 font-sans">
                      <div className="text-slate-900 dark:text-white">{job.age}s ago</div>
                      <div className="text-[11px] text-slate-400">{new Date(job.created).toLocaleTimeString()}</div>
                    </td>
                    <td className="px-4 py-3 text-slate-600 dark:text-slate-400 max-w-xs truncate" title={job.last_log_line || ""}>
                      {job.last_log_line || "—"}
                    </td>
                    <td className="px-4 py-3 font-sans text-right space-x-2 whitespace-nowrap">
                      <button
                        type="button"
                        onClick={() => handleOpenDetail(job.id)}
                        data-testid="job-details-btn"
                        className="px-2.5 py-1 text-xs font-medium rounded border border-slate-300 dark:border-slate-700 text-slate-700 dark:text-slate-300 hover:bg-slate-100 dark:hover:bg-slate-800 transition-colors"
                      >
                        Inspect
                      </button>

                      {job.state === "failed" && (
                        <button
                          type="button"
                          onClick={() => {
                            setRetryingJob(job);
                            setRetryReason("");
                          }}
                          data-testid="job-retry-btn"
                          className="px-2.5 py-1 text-xs font-medium rounded bg-emerald-600 hover:bg-emerald-700 text-white transition-colors"
                        >
                          Retry
                        </button>
                      )}

                      {["queued", "running"].includes(job.state) && (
                        <button
                          type="button"
                          onClick={() => {
                            setCancellingJob(job);
                            setCancelReason("");
                          }}
                          data-testid="job-cancel-btn"
                          className="px-2.5 py-1 text-xs font-medium rounded bg-rose-600 hover:bg-rose-700 text-white transition-colors"
                        >
                          Cancel
                        </button>
                      )}
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        </div>
      )}

      {/* CANCEL MODAL */}
      {cancellingJob && (
        <div
          data-testid="job-cancel-modal"
          className="fixed inset-0 z-50 flex items-center justify-center bg-slate-900/60 backdrop-blur-xs p-4 animate-fade-in"
        >
          <div className="bg-white dark:bg-slate-900 rounded-xl border border-slate-200 dark:border-slate-800 shadow-xl max-w-md w-full p-6 space-y-4">
            <h3 className="text-base font-bold text-slate-900 dark:text-white">
              Cancel Job: {cancellingJob.kind} ({cancellingJob.id.slice(0, 8)})
            </h3>
            <p className="text-xs text-slate-600 dark:text-slate-400">
              Cancelling this task will abort pending and active execution. An audited entry will be written to the sovereign decision ledger.
            </p>

            {actionError && (
              <div className="p-3 bg-rose-50 text-rose-700 text-xs rounded border border-rose-200">
                {actionError}
              </div>
            )}

            <form onSubmit={handleConfirmCancel} data-testid="job-cancel-form" className="space-y-4">
              <div>
                <label htmlFor="cancel-reason" className="block text-xs font-semibold text-slate-700 dark:text-slate-300 mb-1">
                  Reason for Cancellation <span className="text-rose-500">*</span>
                </label>
                <textarea
                  id="cancel-reason"
                  rows={3}
                  data-testid="job-cancel-reason-input"
                  value={cancelReason}
                  onChange={(e) => setCancelReason(e.target.value)}
                  placeholder="Explain why this job is being terminated (audited)..."
                  className="w-full text-xs rounded-lg border border-slate-300 dark:border-slate-700 p-2.5 bg-white dark:bg-slate-800 text-slate-900 dark:text-white"
                  required
                />
              </div>

              <div className="flex justify-end gap-2">
                <button
                  type="button"
                  onClick={() => setCancellingJob(null)}
                  className="px-3 py-1.5 text-xs font-medium rounded-lg border border-slate-300 dark:border-slate-700 hover:bg-slate-100 text-slate-700 dark:text-slate-300"
                >
                  Close
                </button>
                <button
                  type="submit"
                  disabled={!cancelReason.trim() || actionLoading}
                  data-testid="job-confirm-cancel-btn"
                  className="px-3 py-1.5 text-xs font-medium rounded-lg bg-rose-600 hover:bg-rose-700 text-white disabled:opacity-50"
                >
                  {actionLoading ? "Cancelling..." : "Confirm Cancel"}
                </button>
              </div>
            </form>
          </div>
        </div>
      )}

      {/* RETRY MODAL */}
      {retryingJob && (
        <div
          data-testid="job-retry-modal"
          className="fixed inset-0 z-50 flex items-center justify-center bg-slate-900/60 backdrop-blur-xs p-4 animate-fade-in"
        >
          <div className="bg-white dark:bg-slate-900 rounded-xl border border-slate-200 dark:border-slate-800 shadow-xl max-w-md w-full p-6 space-y-4">
            <h3 className="text-base font-bold text-slate-900 dark:text-white">
              Retry Job: {retryingJob.kind} ({retryingJob.id.slice(0, 8)})
            </h3>
            <p className="text-xs text-slate-600 dark:text-slate-400">
              Reset attempts to 0 and re-enqueue this failed job for immediate processing.
            </p>

            {actionError && (
              <div className="p-3 bg-rose-50 text-rose-700 text-xs rounded border border-rose-200">
                {actionError}
              </div>
            )}

            <form onSubmit={handleConfirmRetry} data-testid="job-retry-form" className="space-y-4">
              <div>
                <label htmlFor="retry-reason" className="block text-xs font-semibold text-slate-700 dark:text-slate-300 mb-1">
                  Reason for Retry <span className="text-rose-500">*</span>
                </label>
                <textarea
                  id="retry-reason"
                  rows={3}
                  data-testid="job-retry-reason-input"
                  value={retryReason}
                  onChange={(e) => setRetryReason(e.target.value)}
                  placeholder="Explain why this job is being retried..."
                  className="w-full text-xs rounded-lg border border-slate-300 dark:border-slate-700 p-2.5 bg-white dark:bg-slate-800 text-slate-900 dark:text-white"
                  required
                />
              </div>

              <div className="flex justify-end gap-2">
                <button
                  type="button"
                  onClick={() => setRetryingJob(null)}
                  className="px-3 py-1.5 text-xs font-medium rounded-lg border border-slate-300 dark:border-slate-700 hover:bg-slate-100 text-slate-700 dark:text-slate-300"
                >
                  Close
                </button>
                <button
                  type="submit"
                  disabled={!retryReason.trim() || actionLoading}
                  data-testid="job-confirm-retry-btn"
                  className="px-3 py-1.5 text-xs font-medium rounded-lg bg-emerald-600 hover:bg-emerald-700 text-white disabled:opacity-50"
                >
                  {actionLoading ? "Retrying..." : "Confirm Retry"}
                </button>
              </div>
            </form>
          </div>
        </div>
      )}

      {/* DETAIL DRAWER */}
      <Drawer
        open={Boolean(selectedJob)}
        onClose={() => setSelectedJob(null)}
        title={selectedJob ? `Job ${selectedJob.id}` : "Job Details"}
      >
        {selectedJob && (
          <div className="space-y-6 text-xs text-slate-700 dark:text-slate-300">
            <div>
              <h4 className="font-bold text-slate-900 dark:text-white text-sm mb-2">Job Summary</h4>
              <dl className="grid grid-cols-2 gap-2 bg-slate-50 dark:bg-slate-800/60 p-3 rounded-lg border border-slate-200 dark:border-slate-700">
                <dt className="text-slate-500">Kind:</dt>
                <dd className="font-semibold">{selectedJob.kind}</dd>
                <dt className="text-slate-500">State:</dt>
                <dd className="font-semibold uppercase">{selectedJob.state}</dd>
                <dt className="text-slate-500">Owner:</dt>
                <dd>{selectedJob.owner}</dd>
                <dt className="text-slate-500">Attempts:</dt>
                <dd>{selectedJob.attempts}</dd>
                <dt className="text-slate-500">Created:</dt>
                <dd>{selectedJob.created_at}</dd>
              </dl>
            </div>

            {selectedJob.error && (
              <div>
                <h4 className="font-bold text-rose-600 dark:text-rose-400 text-sm mb-2">Error Details</h4>
                <div data-testid="drawer-job-error" className="bg-rose-50 dark:bg-rose-950/40 p-3 rounded-lg border border-rose-200 dark:border-rose-900/60 text-rose-700 dark:text-rose-300 font-mono text-[11px] whitespace-pre-wrap">
                  {selectedJob.error}
                </div>
              </div>
            )}

            <div>
              <h4 className="font-bold text-slate-900 dark:text-white text-sm mb-2">Execution Logs ({selectedJob.log?.length || 0} lines)</h4>
              <div className="bg-slate-950 text-emerald-400 p-3 rounded-lg font-mono text-[11px] h-64 overflow-y-auto space-y-1">
                {selectedJob.log && selectedJob.log.length > 0 ? (
                  selectedJob.log.map((line, idx) => (
                    <div key={idx} className="leading-relaxed">
                      <span className="text-slate-600 select-none mr-2">[{idx + 1}]</span>
                      {line}
                    </div>
                  ))
                ) : (
                  <div className="text-slate-500 italic">No log entries recorded.</div>
                )}
              </div>
            </div>
          </div>
        )}
      </Drawer>
    </div>
  );
}
