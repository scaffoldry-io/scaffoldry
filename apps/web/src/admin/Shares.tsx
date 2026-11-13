import { useCallback, useEffect, useState } from "react";
import { apiClient } from "../api";
import { ConfirmAction, EmptyState, ErrorState, PageHeader, Skeleton } from "../ui";

type Row = Awaited<ReturnType<typeof apiClient.adminShares>>[number];

const buttonClass = "px-3 py-1.5 text-sm font-medium rounded border border-gray-300 dark:border-gray-700 disabled:opacity-50";

function when(iso: string | null): string {
  if (!iso) return "never";
  const d = new Date(iso);
  return Number.isNaN(d.getTime()) ? iso : d.toLocaleString("en-US", { dateStyle: "medium", timeStyle: "short" });
}

// The Shares section of the console (views phase 6): every shared link, with its use, and a way to close an app's links at once.
export function Shares() {
  const [rows, setRows] = useState<Row[] | null>(null);
  const [error, setError] = useState<unknown>(null);
  const [closing, setClosing] = useState<string | null>(null);

  const load = useCallback(async () => {
    setError(null);
    try {
      setRows(await apiClient.adminShares());
    } catch (e) {
      setError(e);
      setRows([]);
    }
  }, []);

  useEffect(() => {
    void load();
  }, [load]);

  const apps = Array.from(new Map((rows ?? []).filter((r) => r.state === "Active").map((r) => [r.app, r.app_title ?? r.app])).entries());

  return (
    <div className="space-y-4">
      <PageHeader title="Shares" description="Every shared link of every app. A link shows one view, read only, and never a sensitive or hidden field. The token is never shown here." />
      {error !== null ? (
        <ErrorState error={error} />
      ) : rows === null ? (
        <Skeleton rows={3} />
      ) : rows.length === 0 ? (
        <EmptyState title="No shared links" description="A Platform Admin turns on a scope in the settings. An owner or admin of an app then shares a view." />
      ) : (
        <>
          <div className="overflow-x-auto">
            <table className="w-full text-sm" aria-label="Shared links">
              <thead>
                <tr className="text-left text-xs text-gray-500">
                  <th className="pr-3 py-1">App</th>
                  <th className="pr-3">View</th>
                  <th className="pr-3">Scope</th>
                  <th className="pr-3">Made by</th>
                  <th className="pr-3">Made</th>
                  <th className="pr-3">Expires</th>
                  <th className="pr-3">Opened</th>
                  <th className="pr-3">Last opened</th>
                  <th>State</th>
                </tr>
              </thead>
              <tbody>
                {rows.map((r) => (
                  <tr key={r.id} className="border-t border-gray-200 dark:border-gray-800">
                    <td className="pr-3 py-1">{r.app_title ?? r.app}</td>
                    <td className="pr-3">{r.view_name ?? "(deleted view)"}</td>
                    <td className="pr-3">{r.scope}</td>
                    <td className="pr-3 text-xs">{r.created_by}</td>
                    <td className="pr-3 text-xs">{when(r.created_at)}</td>
                    <td className="pr-3 text-xs">{when(r.expires_at)}</td>
                    <td className="pr-3">{r.use_count}</td>
                    <td className="pr-3 text-xs">{when(r.last_used_at)}</td>
                    <td className="font-semibold">{r.state}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
          <div className="space-y-2">
            {apps.map(([slug, title]) =>
              closing === slug ? (
                <ConfirmAction
                  key={slug}
                  verb="Revoke all for"
                  target={title}
                  consequence={`Every active link of ${title} stops working now. People who were sent one see that it is closed.`}
                  onConfirm={async (reason) => {
                    await apiClient.revokeAppShares(slug, reason);
                  }}
                  onSuccess={() => {
                    setClosing(null);
                    void load();
                  }}
                />
              ) : (
                <button key={slug} type="button" className={buttonClass} aria-label={`Revoke all for ${title}`} onClick={() => setClosing(slug)}>
                  {`Revoke all for ${title}`}
                </button>
              )
            )}
          </div>
        </>
      )}
    </div>
  );
}
