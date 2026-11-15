import { useCallback, useEffect, useState } from "react";
import { apiClient, type AdminFlow, type AdminFlows } from "../api";
import { ConfirmAction, EmptyState, ErrorState, PageHeader, Skeleton } from "../ui";

const buttonClass = "px-3 py-1.5 text-sm font-medium rounded border border-gray-300 dark:border-gray-700 disabled:opacity-50";

function when(iso: string | null): string {
  if (!iso) return "never";
  const d = new Date(iso);
  return Number.isNaN(d.getTime()) ? iso : d.toLocaleString("en-US", { dateStyle: "medium", timeStyle: "short" });
}

// The Flows section of the console (workflows phase 11): every flow across the workspaces, who approved it for each workspace and
// when, what it may touch, and how its runs have gone. Turning one off or on asks for a reason and is a ledger entry.
export function Flows() {
  const [data, setData] = useState<AdminFlows | null>(null);
  const [error, setError] = useState<unknown>(null);
  const [crosses, setCrosses] = useState(false);
  const [sensitive, setSensitive] = useState(false);
  const [act, setAct] = useState<AdminFlow | null>(null);

  const load = useCallback(async () => {
    setError(null);
    try {
      setData(await apiClient.adminFlows({ crosses_workspaces: crosses, reads_sensitive: sensitive }));
    } catch (e) {
      setError(e);
    }
  }, [crosses, sensitive]);

  useEffect(() => {
    void load();
  }, [load]);

  if (error !== null) return <ErrorState error={error} />;
  if (data === null) return <Skeleton rows={3} />;

  return (
    <div className="space-y-6">
      <PageHeader title="Flows" description="Flows that act across apps and workspaces, as the service identity each one has. Each workspace a flow reaches approved it." />

      <div className="flex flex-wrap gap-4 text-sm" role="group" aria-label="Filters">
        <label className="inline-flex items-center gap-2">
          <input type="checkbox" checked={crosses} onChange={(e) => setCrosses(e.target.checked)} />
          Crosses workspaces
        </label>
        <label className="inline-flex items-center gap-2">
          <input type="checkbox" checked={sensitive} onChange={(e) => setSensitive(e.target.checked)} />
          Reads sensitive data
        </label>
      </div>

      {data.flows.length === 0 ? (
        <EmptyState title="No flow matches" description="A flow is made by a proposal that a person approves for each workspace it reaches." />
      ) : (
        <div className="overflow-x-auto">
          <table className="w-full text-sm" aria-label="Flows">
            <thead>
              <tr className="text-left text-xs text-gray-500">
                <th className="pr-3 py-1">Flow</th>
                <th className="pr-3">Home workspace</th>
                <th className="pr-3">Approved by</th>
                <th className="pr-3">May do</th>
                <th className="pr-3">Last run</th>
                <th className="pr-3">Failures, 7 days</th>
                <th className="pr-3">State</th>
                <th />
              </tr>
            </thead>
            <tbody>
              {data.flows.map((f) => (
                <tr key={f.rule_id} className="border-t border-gray-200 dark:border-gray-800 align-top">
                  <td className="pr-3 py-1">
                    <div className="font-medium">{f.name}</div>
                    <div className="text-xs text-gray-500">{f.app_slug}</div>
                    {f.crosses_workspaces && <div className="text-xs">Crosses workspaces</div>}
                    {f.reads_sensitive && <div className="text-xs">Reads sensitive data</div>}
                  </td>
                  <td className="pr-3">{f.home_workspace_name}</td>
                  <td className="pr-3">
                    <ul>
                      {f.parties.map((p) => (
                        <li key={p.workspace_id} className="text-xs">
                          {p.approved_by ? `${p.name}: ${p.approved_by}, ${when(p.approved_at)}` : `${p.name}: not recorded`}
                        </li>
                      ))}
                    </ul>
                  </td>
                  <td className="pr-3 text-xs max-w-md">{f.grants}</td>
                  <td className="pr-3 text-xs">{f.last_run ? `${when(f.last_run.at)}${f.last_run.status ? `, ${f.last_run.status}` : ""}` : "never"}</td>
                  <td className="pr-3">{`${f.failures_last_7_days} of ${f.runs_last_7_days}`}</td>
                  <td className="pr-3 font-semibold">{f.state}</td>
                  <td>
                    <button type="button" className={buttonClass} aria-label={`${f.state === "Disabled" ? "Enable" : "Disable"} ${f.name}`} onClick={() => setAct(f)}>
                      {f.state === "Disabled" ? "Enable" : "Disable"}
                    </button>
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}

      {act && (
        <div className="rounded border p-3">
          <ConfirmAction
            verb={act.state === "Disabled" ? "Enable" : "Disable"}
            target={act.name}
            consequence={
              act.state === "Disabled"
                ? "The flow runs again from the next record it is triggered by."
                : "The flow stops from its next run, on every server. Runs that have started finish, and nothing is undone."
            }
            onConfirm={async (reason) => {
              await apiClient.setFlowDisabled(act.rule_id, act.state !== "Disabled", reason);
            }}
            onSuccess={() => {
              setAct(null);
              void load();
            }}
          />
        </div>
      )}
    </div>
  );
}
