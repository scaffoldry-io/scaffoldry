import { useCallback, useEffect, useState } from "react";
import { apiClient, type WebhookDelivery, type WebhookRow } from "../api";
import { ConfirmAction, EmptyState, ErrorState, PageHeader, Skeleton } from "../ui";

const buttonClass = "px-3 py-1.5 text-sm font-medium rounded border border-gray-300 dark:border-gray-700 disabled:opacity-50";

function when(iso: string | null): string {
  if (!iso) return "never";
  const d = new Date(iso);
  return Number.isNaN(d.getTime()) ? iso : d.toLocaleString("en-US", { dateStyle: "medium", timeStyle: "short" });
}

const tone: Record<WebhookRow["state"], string> = {
  Active: "text-emerald-700 dark:text-emerald-400",
  Failing: "text-amber-700 dark:text-amber-400",
  Disabled: "text-rose-700 dark:text-rose-400",
};

/** A webhook's recent deliveries: what happened to each, the status the receiver answered, and the error. */
function Deliveries({ app, id }: { app: string; id: string }) {
  const [log, setLog] = useState<WebhookDelivery[] | null>(null);
  const [error, setError] = useState<unknown>(null);

  useEffect(() => {
    let live = true;
    apiClient
      .webhookDeliveries(app, id)
      .then((l) => live && setLog(l))
      .catch((e) => {
        if (live) {
          setError(e);
          setLog([]);
        }
      });
    return () => {
      live = false;
    };
  }, [app, id]);

  if (error !== null) return <ErrorState error={error} />;
  if (log === null) return <Skeleton rows={2} />;
  if (log.length === 0) return <p className="text-xs text-gray-500">No deliveries yet.</p>;
  return (
    <ul className="space-y-1 text-xs" aria-label="Recent deliveries">
      {log.map((d) => (
        <li key={d.id} className="flex flex-wrap gap-x-3 gap-y-0.5 border-t border-gray-100 dark:border-gray-800 pt-1">
          <span className="font-mono">{d.event ?? "event"}</span>
          <span>{d.state}</span>
          {d.last_status !== null && <span>{d.last_status}</span>}
          <span>{d.attempts === 1 ? "1 try" : `${d.attempts} tries`}</span>
          <span className="text-gray-500">{when(d.created_at)}</span>
          {d.last_error && <span className="w-full text-rose-700 dark:text-rose-400">{d.last_error}</span>}
        </li>
      ))}
    </ul>
  );
}

// The Webhooks section of the console (integrations phase 5): every webhook across apps.
export function Webhooks() {
  const [rows, setRows] = useState<WebhookRow[] | null>(null);
  const [error, setError] = useState<unknown>(null);
  const [open, setOpen] = useState<string | null>(null);
  const [switching, setSwitching] = useState<string | null>(null);

  const load = useCallback(async () => {
    setError(null);
    try {
      setRows(await apiClient.listWebhooks());
    } catch (e) {
      setError(e);
      setRows([]);
    }
  }, []);

  useEffect(() => {
    void load();
  }, [load]);

  const keyOf = (r: WebhookRow) => `${r.app}/${r.id}`;

  return (
    <div className="space-y-4">
      <PageHeader title="Webhooks" description="Every webhook across apps. A webhook turns itself off after 20 failures in a row. Turning one off or on here needs a reason." />
      {error !== null ? (
        <ErrorState error={error} />
      ) : rows === null ? (
        <Skeleton rows={3} />
      ) : rows.length === 0 ? (
        <EmptyState title="No webhooks" description="An app adds one in its manifest, and a proposal approves it." />
      ) : (
        <ul className="space-y-2">
          {rows.map((r) => (
            <li key={keyOf(r)} className="rounded border border-gray-200 dark:border-gray-800 p-3 space-y-2">
              <div className="flex flex-wrap items-center justify-between gap-2">
                <div>
                  <div className="text-sm font-semibold">{r.name}</div>
                  <div className="text-xs text-gray-500 dark:text-gray-400">{r.app_title}</div>
                </div>
                <span className={`text-xs font-semibold ${tone[r.state]}`}>{r.state}</span>
              </div>
              <div className="text-xs text-gray-600 dark:text-gray-300 space-y-0.5">
                <div className="font-mono">{r.host}</div>
                <div>{r.events.join(", ")}</div>
                <div>
                  Last success {when(r.last_success)}
                  {r.consecutive_failures > 0 && ` · ${r.consecutive_failures === 1 ? "1 failure" : `${r.consecutive_failures} failures`} in a row`}
                </div>
              </div>
              {switching === keyOf(r) ? (
                <ConfirmAction
                  verb={r.state === "Disabled" ? "Enable" : "Disable"}
                  target={r.name}
                  consequence={
                    r.state === "Disabled"
                      ? "Deliveries start again. What happened while it was off is not sent."
                      : "Nothing more is sent. What is queued now ends as dead, with this reason, and stays in the log."
                  }
                  onConfirm={(reason) => apiClient.setWebhookEnabled(r.app, r.id, r.state === "Disabled", reason)}
                  onSuccess={() => {
                    setSwitching(null);
                    void load();
                  }}
                />
              ) : (
                <div className="flex gap-2">
                  <button type="button" className={buttonClass} aria-label={`Deliveries of ${r.name}`} onClick={() => setOpen(open === keyOf(r) ? null : keyOf(r))}>
                    Deliveries
                  </button>
                  <button type="button" className={buttonClass} aria-label={`${r.state === "Disabled" ? "Enable" : "Disable"} ${r.name}`} onClick={() => setSwitching(keyOf(r))}>
                    {r.state === "Disabled" ? "Enable" : "Disable"}
                  </button>
                </div>
              )}
              {open === keyOf(r) && <Deliveries app={r.app} id={r.id} />}
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}
