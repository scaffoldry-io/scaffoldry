import { useCallback, useEffect, useState } from "react";
import { apiClient, type AppClientRow, type ClientUsage } from "../api";
import { ConfirmAction, EmptyState, ErrorState, PageHeader, Skeleton, StatusBadge, type BadgeTone } from "../ui";

const buttonClass = "px-3 py-1.5 text-sm font-medium rounded border border-gray-300 dark:border-gray-700 disabled:opacity-50";

function tone(status: AppClientRow["status"]): BadgeTone {
  if (status === "Active") return "ok";
  if (status === "Pending") return "warn";
  return "bad";
}

function count(n: number, one: string, many: string): string {
  return `${n} ${n === 1 ? one : many}`;
}

type Acting = { id: string; what: "approve" | "reject" | "disable" };

// The Outside apps section of the console: every registered app, what it may reach, who connected to it, and what it did.
export function OutsideApps() {
  const [rows, setRows] = useState<AppClientRow[] | null>(null);
  const [error, setError] = useState<unknown>(null);
  const [acting, setActing] = useState<Acting | null>(null);
  const [usage, setUsage] = useState<Record<string, ClientUsage>>({});

  const load = useCallback(async () => {
    try {
      setRows(await apiClient.listClients());
      setError(null);
    } catch (e) {
      setError(e);
    }
  }, []);
  useEffect(() => {
    void load();
  }, [load]);

  const toggleUsage = async (id: string) => {
    if (usage[id]) {
      setUsage(({ [id]: _gone, ...rest }) => rest);
      return;
    }
    try {
      const u = await apiClient.clientUsage(id);
      setUsage((current) => ({ ...current, [id]: u }));
    } catch (e) {
      setError(e);
    }
  };

  return (
    <div className="space-y-6" data-testid="admin-outside-apps">
      <PageHeader title="Outside apps" description="Apps run elsewhere that read your data as the person using them, within a ceiling a person approved" />
      {error !== null && <ErrorState error={error} onRetry={() => void load()} />}
      {rows === null ? (
        error === null && <Skeleton rows={3} />
      ) : rows.length === 0 ? (
        <EmptyState title="No outside apps" description="An app is registered by a proposal, and an owner or admin of its workspace approves it." />
      ) : (
        rows.map((c) => {
          const u = usage[c.client_id];
          const totals = u ? u.days.reduce((t, d) => ({ calls: t.calls + d.calls, read: t.read + d.records_read, written: t.written + d.records_written }), { calls: 0, read: 0, written: 0 }) : null;
          return (
            <section key={c.client_id} data-testid={`outside-app-${c.client_id}`} className="border border-gray-200 dark:border-gray-800 rounded-lg p-4 space-y-3 text-sm">
              <div className="flex flex-wrap items-center justify-between gap-2">
                <div>
                  <div className="font-semibold text-gray-900 dark:text-gray-100">{c.name}</div>
                  <div className="text-xs text-gray-500">
                    Run by {c.operator} · <span>{c.kind}</span>
                    {c.service_identity && " · acts as itself"}
                  </div>
                </div>
                <StatusBadge tone={tone(c.status)} text={c.status} />
              </div>

              <div className="text-gray-700 dark:text-gray-300">Workspaces: {c.workspaces.length === 0 ? "none" : c.workspaces.map((w) => w.name).join(", ")}</div>
              <ul className="list-disc pl-5 text-gray-700 dark:text-gray-300">
                {c.sentences.map((s) => (
                  <li key={s}>{s}</li>
                ))}
              </ul>
              <div className="flex flex-wrap gap-4 text-xs text-gray-500">
                <span>{count(c.connected_users, "person connected", "people connected")}</span>
                <span>{c.last_call_at ? `Last call: ${new Date(c.last_call_at).toLocaleString()}` : "Never called"}</span>
              </div>

              <div className="flex flex-wrap gap-2">
                <button type="button" className={buttonClass} onClick={() => void toggleUsage(c.client_id)}>
                  Usage
                </button>
                {c.status === "Pending" && (
                  <>
                    <button type="button" className={buttonClass} onClick={() => setActing({ id: c.client_id, what: "approve" })}>
                      Approve
                    </button>
                    <button type="button" className={buttonClass} onClick={() => setActing({ id: c.client_id, what: "reject" })}>
                      Reject
                    </button>
                  </>
                )}
                {c.status === "Active" && (
                  <button type="button" className={buttonClass} onClick={() => setActing({ id: c.client_id, what: "disable" })}>
                    Disable
                  </button>
                )}
              </div>

              {acting?.id === c.client_id && (
                <div className="border-t border-gray-200 dark:border-gray-800 pt-3">
                  <ConfirmAction
                    verb={acting.what === "approve" ? "Approve" : acting.what === "reject" ? "Reject" : "Disable"}
                    target="the app"
                    consequence={
                      acting.what === "approve"
                        ? "The app can then ask people to connect. What it may reach is the ceiling shown above, and no more."
                        : acting.what === "reject"
                          ? "The app is turned down and never runs."
                          : "The app stops on its next call, for everyone. This cannot be undone here."
                    }
                    onConfirm={async (reason) => {
                      if (acting.what === "disable") await apiClient.disableClient(c.client_id, reason);
                      else await apiClient.decideClient(c.client_id, acting.what, reason);
                    }}
                    onSuccess={() => {
                      setActing(null);
                      void load();
                    }}
                  />
                </div>
              )}

              {u && totals && (
                <div data-testid={`outside-app-usage-${c.client_id}`} className="space-y-2 border-t border-gray-200 dark:border-gray-800 pt-3">
                  <div className="font-medium text-gray-900 dark:text-gray-100">
                    Last 30 days: {count(totals.calls, "call", "calls")}, {count(totals.read, "record read", "records read")}, {count(totals.written, "record written", "records written")}
                  </div>
                  <table className="w-full text-xs">
                    <thead>
                      <tr className="text-left text-gray-500">
                        <th className="py-1">Day</th>
                        <th>Calls</th>
                        <th>Records read</th>
                        <th>Records written</th>
                      </tr>
                    </thead>
                    <tbody>
                      {u.days.map((d) => (
                        <tr key={d.day} className="border-t border-gray-100 dark:border-gray-800">
                          <td className="py-1">{d.day}</td>
                          <td>{d.calls}</td>
                          <td>{d.records_read}</td>
                          <td>{d.records_written}</td>
                        </tr>
                      ))}
                    </tbody>
                  </table>
                </div>
              )}
            </section>
          );
        })
      )}
    </div>
  );
}
