import { useAsync, ErrorState, Skeleton, PageHeader } from "../ui";
import { apiClient, type AdminOverviewData } from "../api";

export function Overview() {
  const { data, error, loading, reload } = useAsync<AdminOverviewData>(
    () => apiClient.getAdminOverview(),
    true
  );

  if (loading) {
    return (
      <div className="space-y-6" data-testid="admin-overview-loading">
        <PageHeader
          title="Institutional Administrative Console"
          description="Real-time sovereign platform state, security controls, and resource telemetry"
        />
        <div className="grid grid-cols-1 md:grid-cols-2 lg:grid-cols-3 gap-5">
          <Skeleton rows={3} />
          <Skeleton rows={3} />
          <Skeleton rows={3} />
        </div>
      </div>
    );
  }

  if (error || !data) {
    return (
      <div className="space-y-6">
        <PageHeader
          title="Institutional Administrative Console"
          description="Real-time sovereign platform state, security controls, and resource telemetry"
        />
        <div data-testid="admin-load-error">
          <ErrorState
            message={error ? error.message : "Failed to load administrative overview"}
            next="Ensure the platform backend API is active and the session holds administrative scope."
            onRetry={reload}
          />
        </div>
      </div>
    );
  }

  return (
    <div className="space-y-6" data-testid="admin-overview">
      <PageHeader
        title="Institutional Administrative Console"
        description="Real-time sovereign platform state, security controls, and resource telemetry"
      />

      <div className="grid grid-cols-1 md:grid-cols-2 lg:grid-cols-3 gap-5">
        {/* SERVER CARD */}
        <div
          data-testid="admin-overview-server-card"
          className="bg-white dark:bg-slate-900 border border-slate-200 dark:border-slate-800 rounded-xl p-5 shadow-xs hover:border-slate-300 dark:hover:border-slate-700 transition-all"
        >
          <div className="flex items-center justify-between mb-3">
            <span className="text-xs font-bold uppercase tracking-wider text-slate-500 dark:text-slate-400">
              Platform Server
            </span>
            <span className="px-2 py-0.5 rounded text-[10px] font-mono font-semibold bg-emerald-50 text-emerald-700 dark:bg-emerald-950/40 dark:text-emerald-400 border border-emerald-200 dark:border-emerald-800/60">
              v{data.server.version}
            </span>
          </div>
          <div className="space-y-2 mt-4">
            <div className="flex justify-between items-center text-xs">
              <span className="text-slate-600 dark:text-slate-400">Database Workers</span>
              <span className="font-mono font-semibold text-slate-900 dark:text-slate-100">
                {data.server.database_worker_count}
              </span>
            </div>
            <div className="flex justify-between items-center text-xs">
              <span className="text-slate-600 dark:text-slate-400">Applied Migrations</span>
              <span className="font-mono font-semibold text-slate-900 dark:text-slate-100">
                {data.server.applied_migrations?.length ?? 0}
              </span>
            </div>
          </div>
        </div>

        {/* PEOPLE CARD */}
        <div
          data-testid="admin-overview-people-card"
          className="bg-white dark:bg-slate-900 border border-slate-200 dark:border-slate-800 rounded-xl p-5 shadow-xs hover:border-slate-300 dark:hover:border-slate-700 transition-all"
        >
          <div className="flex items-center justify-between mb-3">
            <span className="text-xs font-bold uppercase tracking-wider text-slate-500 dark:text-slate-400">
              People &amp; Directory
            </span>
            <span className="px-2 py-0.5 rounded text-[10px] font-medium bg-blue-50 text-blue-700 dark:bg-blue-950/40 dark:text-blue-400 border border-blue-200 dark:border-blue-800/60">
              {data.people.platform_admins} Admins
            </span>
          </div>
          <div className="grid grid-cols-3 gap-2 mt-4 text-center">
            <div className="bg-slate-50 dark:bg-slate-800/50 p-2.5 rounded-lg border border-slate-100 dark:border-slate-800">
              <div className="text-lg font-bold text-slate-900 dark:text-slate-100">
                {data.people.active}
              </div>
              <div className="text-[10px] font-medium text-slate-500 dark:text-slate-400 mt-0.5">
                Active
              </div>
            </div>
            <div className="bg-amber-50/50 dark:bg-amber-950/20 p-2.5 rounded-lg border border-amber-200/50 dark:border-amber-900/30">
              <div className="text-lg font-bold text-amber-700 dark:text-amber-400">
                {data.people.on_hold}
              </div>
              <div className="text-[10px] font-medium text-amber-600 dark:text-amber-500 mt-0.5">
                On Hold
              </div>
            </div>
            <div className="bg-slate-50 dark:bg-slate-800/50 p-2.5 rounded-lg border border-slate-100 dark:border-slate-800">
              <div className="text-lg font-bold text-slate-500 dark:text-slate-400">
                {data.people.inactive}
              </div>
              <div className="text-[10px] font-medium text-slate-400 dark:text-slate-500 mt-0.5">
                Inactive
              </div>
            </div>
          </div>
        </div>

        {/* ORGANIZATION CARD */}
        <div
          data-testid="admin-overview-org-card"
          className="bg-white dark:bg-slate-900 border border-slate-200 dark:border-slate-800 rounded-xl p-5 shadow-xs hover:border-slate-300 dark:hover:border-slate-700 transition-all"
        >
          <div className="flex items-center justify-between mb-3">
            <span className="text-xs font-bold uppercase tracking-wider text-slate-500 dark:text-slate-400">
              Institutional Hierarchy
            </span>
          </div>
          <div className="mt-4 flex items-baseline gap-2">
            <span className="text-3xl font-extrabold text-slate-900 dark:text-slate-100">
              {data.organization.unit_count}
            </span>
            <span className="text-xs font-medium text-slate-500 dark:text-slate-400">
              Governed Units
            </span>
          </div>
          <p className="text-xs text-slate-500 dark:text-slate-400 mt-3">
            Multi-level academic units with NIST hierarchical scope inheritance.
          </p>
        </div>

        {/* WORKSPACES & APPS CARD */}
        <div
          data-testid="admin-overview-workspaces-card"
          className="bg-white dark:bg-slate-900 border border-slate-200 dark:border-slate-800 rounded-xl p-5 shadow-xs hover:border-slate-300 dark:hover:border-slate-700 transition-all"
        >
          <div className="flex items-center justify-between mb-3">
            <span className="text-xs font-bold uppercase tracking-wider text-slate-500 dark:text-slate-400">
              Workspaces &amp; Apps
            </span>
          </div>
          <div className="grid grid-cols-2 gap-3 mt-4">
            <div className="bg-slate-50 dark:bg-slate-800/50 p-2.5 rounded-lg border border-slate-100 dark:border-slate-800">
              <div className="text-2xl font-bold text-slate-900 dark:text-slate-100">
                {data.workspaces.workspace_count}
              </div>
              <div className="text-xs text-slate-500 dark:text-slate-400 mt-0.5">
                Workspaces
              </div>
            </div>
            <div className="bg-slate-50 dark:bg-slate-800/50 p-2.5 rounded-lg border border-slate-100 dark:border-slate-800">
              <div className="text-2xl font-bold text-slate-900 dark:text-slate-100">
                {data.workspaces.app_count}
              </div>
              <div className="text-xs text-slate-500 dark:text-slate-400 mt-0.5">
                Published Apps
              </div>
            </div>
          </div>
        </div>

        {/* PROCESSES CARD */}
        <div
          data-testid="admin-overview-processes-card"
          className="bg-white dark:bg-slate-900 border border-slate-200 dark:border-slate-800 rounded-xl p-5 shadow-xs hover:border-slate-300 dark:hover:border-slate-700 transition-all"
        >
          <div className="flex items-center justify-between mb-3">
            <span className="text-xs font-bold uppercase tracking-wider text-slate-500 dark:text-slate-400">
              Governed Processes
            </span>
          </div>
          <div className="mt-4 flex items-baseline gap-2">
            <span className="text-3xl font-extrabold text-slate-900 dark:text-slate-100">
              {data.processes.waiting_instance_count}
            </span>
            <span className="text-xs font-medium text-slate-500 dark:text-slate-400">
              Waiting Task Instances
            </span>
          </div>
          <p className="text-xs text-slate-500 dark:text-slate-400 mt-3">
            Workflow tasks awaiting user decision or Cedar policy evaluation.
          </p>
        </div>

        {/* LEDGER CARD */}
        <div
          data-testid="admin-overview-ledger-card"
          className="bg-white dark:bg-slate-900 border border-slate-200 dark:border-slate-800 rounded-xl p-5 shadow-xs hover:border-slate-300 dark:hover:border-slate-700 transition-all"
        >
          <div className="flex items-center justify-between mb-3">
            <span className="text-xs font-bold uppercase tracking-wider text-slate-500 dark:text-slate-400">
              Cryptographic Ledger
            </span>
            <span className="px-2 py-0.5 rounded text-[10px] font-mono bg-purple-50 text-purple-700 dark:bg-purple-950/40 dark:text-purple-400 border border-purple-200 dark:border-purple-800/60">
              {data.ledger.entry_count} Entries
            </span>
          </div>
          <div className="mt-4 space-y-1">
            <span className="text-[11px] text-slate-500 dark:text-slate-400 block">
              Head Chain Hash
            </span>
            <span className="font-mono text-xs text-slate-800 dark:text-slate-200 break-all bg-slate-50 dark:bg-slate-800/60 p-2 rounded block border border-slate-200/60 dark:border-slate-700/60">
              {data.ledger.head_hash || "Genesis"}
            </span>
          </div>
        </div>
      </div>
    </div>
  );
}
