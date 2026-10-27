import { useCallback, useEffect, useState } from "react";
import { apiClient, type AuthorityLoad } from "../api";
import { EmptyState, ErrorState, Skeleton } from "../ui";

/** Seconds as the biggest two units a person reads at a glance. */
export function duration(seconds: number): string {
  const minutes = Math.round(seconds / 60);
  if (minutes < 1) return "under a minute";
  if (minutes < 60) return `${minutes} min`;
  const hours = Math.floor(minutes / 60);
  if (hours < 24) return `${hours} h ${minutes % 60} min`;
  return `${Math.floor(hours / 24)} days ${hours % 24} h`;
}

// The Load tab: what the matrix did to real proposals in the last 90 days. Counts, names and times, no content.
export function AuthorityLoadPanel() {
  const [data, setData] = useState<AuthorityLoad | null>(null);
  const [error, setError] = useState<unknown>(null);

  const read = useCallback(async () => {
    try {
      setData(await apiClient.authorityLoad());
      setError(null);
    } catch (e) {
      setError(e);
    }
  }, []);
  useEffect(() => {
    void read();
  }, [read]);

  if (error !== null) return <ErrorState error={error} onRetry={() => void read()} />;
  if (!data) return <Skeleton rows={3} />;

  const { paths, decision_time: time } = data;
  return (
    <div data-testid="authority-load" className="space-y-6 text-sm">
      <section aria-label="Paths" className="space-y-1">
        <div className="font-medium text-gray-900 dark:text-gray-100">
          {data.proposals} proposals in the last {data.window_days} days
        </div>
        <div className="text-gray-700 dark:text-gray-300">
          {paths.direct} direct, {paths.second_person} second person, {paths.approver} approver
        </div>
        {data.unassigned > 0 && <div className="text-amber-700 dark:text-amber-300">{data.unassigned} proposals have no one to decide them.</div>}
      </section>

      <section aria-label="Time to decide" className="space-y-1">
        <h3 className="text-sm font-semibold text-gray-800 dark:text-gray-200">Time from proposal to decision</h3>
        {time.decided === 0 || time.median_seconds === null || time.p95_seconds === null ? (
          <div className="text-gray-500">Nothing was decided in the last {data.window_days} days.</div>
        ) : (
          <div data-testid="authority-load-time" className="text-gray-700 dark:text-gray-300">
            {time.decided} decided. Median {duration(time.median_seconds)}. 95th percentile {duration(time.p95_seconds)}.
          </div>
        )}
      </section>

      <section aria-label="Approvers" className="space-y-2">
        <h3 className="text-sm font-semibold text-gray-800 dark:text-gray-200">Items per week, for each named approver</h3>
        {data.approvers.length === 0 ? (
          <EmptyState title="No named approver received a proposal." description="Only the approver path names people. Direct and second-person changes go to the workspace's owners and admins." />
        ) : (
          data.approvers.map((a) => (
            <div key={a.eppn} data-testid={`authority-load-${a.eppn}`} className="border border-gray-200 dark:border-gray-800 rounded-lg p-3">
              <div className="text-gray-900 dark:text-gray-100">{a.display_name}</div>
              <div className="text-gray-700 dark:text-gray-300">
                Median {a.median}. 95th percentile {a.p95}. Busiest week {a.largest}.
              </div>
            </div>
          ))
        )}
      </section>
    </div>
  );
}
