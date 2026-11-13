import { useCallback, useEffect, useState } from "react";
import { apiClient, type DataConnection, type DataConnectionMapping, type DataConnectionRun } from "../api";
import { ConfirmAction, EmptyState, ErrorState, PageHeader, Skeleton } from "../ui";

const buttonClass = "px-3 py-1.5 text-sm font-medium rounded border border-gray-300 dark:border-gray-700 disabled:opacity-50";

function when(iso: string | null, none: string): string {
  if (!iso) return none;
  const d = new Date(iso);
  return Number.isNaN(d.getTime()) ? iso : d.toLocaleString("en-US", { dateStyle: "medium", timeStyle: "short" });
}

const tone: Record<DataConnection["state"], string> = {
  Healthy: "text-emerald-700 dark:text-emerald-400",
  "Needs review": "text-amber-700 dark:text-amber-400",
  Failing: "text-rose-700 dark:text-rose-400",
  Disabled: "text-gray-500 dark:text-gray-400",
};

/** What one run did, in words. */
function said(r: DataConnectionRun): string {
  if (r.state === "failed") return r.error ?? "The run failed";
  if (r.state === "running") return "Running now";
  if (r.state === "unchanged") return "No change since the last run";
  const parts = [
    ["Added", r.added],
    ["changed", r.changed],
    ["removed", r.removed],
  ].filter(([, n]) => (n as number) > 0).map(([w, n]) => `${w} ${n}`);
  return parts.length === 0 ? "No records changed" : parts.join(", ");
}

/** A connection's last runs, newest first: the server keeps the newest 100. */
function History({ id }: { id: string }) {
  const [runs, setRuns] = useState<DataConnectionRun[] | null>(null);
  const [error, setError] = useState<unknown>(null);

  useEffect(() => {
    let live = true;
    apiClient
      .dataConnectionRuns(id)
      .then((r) => live && setRuns(r))
      .catch((e) => {
        if (live) {
          setError(e);
          setRuns([]);
        }
      });
    return () => {
      live = false;
    };
  }, [id]);

  if (error !== null) return <ErrorState error={error} />;
  if (runs === null) return <Skeleton rows={2} />;
  if (runs.length === 0) return <p className="text-xs text-gray-500">No runs yet.</p>;
  return (
    <ul className="space-y-1 text-xs" aria-label="Run history">
      {runs.map((r) => (
        <li key={r.id} className="flex flex-wrap gap-x-3 gap-y-0.5 border-t border-gray-100 dark:border-gray-800 pt-1">
          <span className="text-gray-500">{when(r.started_at, "")}</span>
          <span className="font-semibold">{r.state}</span>
          <span className={r.state === "failed" ? "text-rose-700 dark:text-rose-400" : ""}>{said(r)}</span>
        </li>
      ))}
    </ul>
  );
}

/** The column map with what the last run wrote. The map is read here: a source that changes shape makes a proposal. */
function Mapping({ id }: { id: string }) {
  const [map, setMap] = useState<DataConnectionMapping | null>(null);
  const [error, setError] = useState<unknown>(null);

  useEffect(() => {
    let live = true;
    apiClient
      .dataConnectionMapping(id)
      .then((m) => live && setMap(m))
      .catch((e) => live && setError(e));
    return () => {
      live = false;
    };
  }, [id]);

  if (error !== null) return <ErrorState error={error} />;
  if (map === null) return <Skeleton rows={2} />;
  return (
    <div className="space-y-2">
      <table className="w-full text-xs" aria-label="Column map">
        <thead>
          <tr className="text-left text-gray-500">
            <th className="pr-3 py-1">Source column</th>
            <th className="pr-3">Field</th>
            <th className="pr-3">Type</th>
            <th>From the last file</th>
          </tr>
        </thead>
        <tbody>
          {map.columns.map((c) => (
            <tr key={c.field} className="border-t border-gray-100 dark:border-gray-800">
              <td className="pr-3 py-1">
                {c.source}
                {c.key && <span className="ml-1 text-gray-500">(key)</span>}
              </td>
              <td className="pr-3 font-mono">{c.field}</td>
              <td className="pr-3">{c.type}</td>
              <td>{c.sample.length === 0 ? <span className="text-gray-500">nothing synced yet</span> : c.sample.map(String).join(", ")}</td>
            </tr>
          ))}
        </tbody>
      </table>
      <p className="text-xs text-gray-500">{map.changes}</p>
    </div>
  );
}

type Open = { id: string; what: "runs" | "mapping" | "switch" | "shrink" };

// The Connections section of the console (connections phase 6): each data connection's health, history and actions.
export function Connections() {
  const [rows, setRows] = useState<DataConnection[] | null>(null);
  const [error, setError] = useState<unknown>(null);
  const [open, setOpen] = useState<Open | null>(null);
  const [running, setRunning] = useState<string | null>(null);

  const load = useCallback(async () => {
    setError(null);
    try {
      setRows(await apiClient.listDataConnections());
    } catch (e) {
      setError(e);
      setRows([]);
    }
  }, []);

  useEffect(() => {
    void load();
  }, [load]);

  const toggle = (id: string, what: Open["what"]) => setOpen(open?.id === id && open.what === what ? null : { id, what });

  const runNow = async (id: string) => {
    setRunning(id);
    try {
      await apiClient.runDataConnection(id);
      await load();
    } catch (e) {
      setError(e);
    } finally {
      setRunning(null);
    }
  };

  return (
    <div className="space-y-4">
      <PageHeader
        title="Connections"
        description="Each connection copies an outside source into a read-only dataset on a schedule. A failed run keeps the last good data. Turning one off or on needs a reason."
      />
      {error !== null ? (
        <ErrorState error={error} />
      ) : rows === null ? (
        <Skeleton rows={3} />
      ) : rows.length === 0 ? (
        <EmptyState title="No connections" description="A connection fills a dataset from a file drop, a PostgreSQL query or an HTTP service." />
      ) : (
        <div className="overflow-x-auto">
          <table className="w-full text-sm" aria-label="Connections">
            <thead>
              <tr className="text-left text-xs text-gray-500">
                <th className="pr-3 py-1">Connection</th>
                <th className="pr-3">Kind</th>
                <th className="pr-3">Dataset</th>
                <th className="pr-3">Last success</th>
                <th className="pr-3">Last result</th>
                <th className="pr-3">State</th>
                <th>Next run</th>
              </tr>
            </thead>
            <tbody>
              {rows.map((r) => (
                <ConnectionRow key={r.id} r={r} open={open?.id === r.id ? open.what : null} running={running === r.id} toggle={toggle} runNow={runNow} done={() => { setOpen(null); void load(); }} />
              ))}
            </tbody>
          </table>
        </div>
      )}
    </div>
  );
}

function ConnectionRow({ r, open, running, toggle, runNow, done }: { r: DataConnection; open: Open["what"] | null; running: boolean; toggle: (id: string, what: Open["what"]) => void; runNow: (id: string) => void; done: () => void }) {
  const shrank = r.last_run?.state === "failed" && (r.last_run.error ?? "").startsWith("source_shrank");
  return (
    <>
      <tr className="border-t border-gray-200 dark:border-gray-800 align-top">
        <td className="pr-3 py-2 font-semibold">{r.name}</td>
        <td className="pr-3 font-mono text-xs">{r.kind}</td>
        <td className="pr-3">{r.dataset_app}</td>
        <td className="pr-3 text-xs" data-testid="last-success">{when(r.last_success, "never")}</td>
        <td className="pr-3 text-xs">
          {r.last_result}
          {r.review_note && <div className="text-amber-700 dark:text-amber-400">{r.review_note}</div>}
        </td>
        <td className={`pr-3 text-xs font-semibold ${tone[r.state]}`}>{r.state}</td>
        <td className="text-xs" data-testid={`next-run-${r.id}`}>
          <span data-testid="next-run">{when(r.next_run, "none")}</span>
        </td>
      </tr>
      <tr>
        <td colSpan={7} className="pb-3">
          {open === "switch" ? (
            <ConfirmAction
              verb={r.enabled ? "Disable" : "Enable"}
              target={r.name}
              consequence={r.enabled ? "The schedule stops running this connection. The dataset keeps what it has." : "The schedule runs this connection again."}
              onConfirm={(reason) => apiClient.setDataConnectionEnabled(r.id, !r.enabled, reason)}
              onSuccess={done}
            />
          ) : open === "shrink" ? (
            <ConfirmAction
              verb="Run with the smaller file"
              target={r.name}
              consequence="The file has fewer than half the rows of the last run. Running it removes the rows that are gone. They go to trash and can be restored."
              onConfirm={async (reason) => {
                await apiClient.runDataConnection(r.id, { reason });
              }}
              onSuccess={done}
            />
          ) : (
            <div className="flex flex-wrap gap-2">
              <button type="button" className={buttonClass} aria-label={`Run now ${r.name}`} disabled={running} onClick={() => runNow(r.id)}>
                Run now
              </button>
              <button type="button" className={buttonClass} aria-label={`Runs of ${r.name}`} onClick={() => toggle(r.id, "runs")}>
                Runs
              </button>
              <button type="button" className={buttonClass} aria-label={`Edit mapping of ${r.name}`} onClick={() => toggle(r.id, "mapping")}>
                Edit mapping
              </button>
              <button type="button" className={buttonClass} aria-label={`${r.enabled ? "Disable" : "Enable"} ${r.name}`} onClick={() => toggle(r.id, "switch")}>
                {r.enabled ? "Disable" : "Enable"}
              </button>
              {shrank && (
                <button type="button" className={buttonClass} aria-label={`Approve the smaller file for ${r.name}`} onClick={() => toggle(r.id, "shrink")}>
                  Approve smaller file
                </button>
              )}
            </div>
          )}
          {open === "runs" && <History id={r.id} />}
          {open === "mapping" && <Mapping id={r.id} />}
        </td>
      </tr>
    </>
  );
}
