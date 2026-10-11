import { useCallback, useState } from "react";
import { restoreTrash, listTrash, type TrashEntry } from "./api";
import { EmptyState, ErrorState, Skeleton, Toast, useAsync } from "./ui";

export function timeLeft(days: number): string {
  if (days <= 0) return "Removed permanently today";
  return days === 1 ? "Removed permanently in 1 day" : `Removed permanently in ${days} days`;
}

/** The trash of an app. Each row has Restore. */
export function Trash({ appSlug }: { appSlug: string }) {
  const load = useCallback(() => listTrash(appSlug), [appSlug]);
  const { data, error, loading, reload } = useAsync(load);
  const [failure, setFailure] = useState<unknown>(null);

  const restore = async (entry: TrashEntry) => {
    try {
      await restoreTrash(appSlug, entry.record_id);
      await reload();
    } catch (e) {
      setFailure(e);
    }
  };

  if (loading && !data) return <Skeleton rows={3} />;
  if (error) return <ErrorState error={error} onRetry={() => reload().catch(() => {})} />;
  if (!data || data.entries.length === 0) {
    return <EmptyState title="The trash is empty" description="Deleted records wait here until they are removed permanently." />;
  }
  return (
    <div>
      <table className="w-full text-sm">
        <thead>
          <tr className="text-left">
            <th>Record</th>
            <th>Deleted by</th>
            <th>Time left</th>
            <th />
          </tr>
        </thead>
        <tbody>
          {data.entries.map((e) => (
            <tr key={e.record_id}>
              <td>{e.title}</td>
              <td>{e.deleted_by_name}</td>
              <td>{timeLeft(e.days_left)}</td>
              <td>
                <button type="button" onClick={() => restore(e)} aria-label={`Restore ${e.title}`}>
                  Restore
                </button>
              </td>
            </tr>
          ))}
        </tbody>
      </table>
      {failure !== null && <Toast tone="error" error={failure} onClose={() => setFailure(null)} />}
    </div>
  );
}
