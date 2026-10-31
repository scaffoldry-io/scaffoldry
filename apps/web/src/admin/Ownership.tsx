import { useCallback, useEffect, useState } from "react";
import { apiClient, type OwnershipOrphans, type OwnershipPreview } from "../api";
import { EmptyState, ErrorState, FormField, Skeleton } from "../ui";

const inputClass =
  "w-full text-sm rounded border border-gray-300 dark:border-gray-700 p-2 bg-white dark:bg-gray-800 text-gray-900 dark:text-gray-100";

/** Why a person has gone, as the second half of a sentence. */
function gone(why: string | null): string {
  if (why === "inactive") return "is no longer active";
  if (why === "on hold") return "is on hold";
  return "has no current appointment";
}

// Ownership: the workspaces nobody is looking after, and a transfer that is previewed before it is made.
export function Ownership() {
  const [orphans, setOrphans] = useState<OwnershipOrphans | null>(null);
  const [error, setError] = useState<unknown>(null);
  const [from, setFrom] = useState("");
  const [to, setTo] = useState("");
  const [reason, setReason] = useState("");
  const [preview, setPreview] = useState<OwnershipPreview | null>(null);
  const [busy, setBusy] = useState(false);
  const [message, setMessage] = useState<{ text: string; error?: boolean } | null>(null);

  const read = useCallback(async () => {
    try {
      setOrphans(await apiClient.ownershipOrphans());
      setError(null);
    } catch (e) {
      setError(e);
    }
  }, []);
  useEffect(() => {
    void read();
  }, [read]);

  // A preview is for two people. Change either and it no longer applies.
  const change = (set: (v: string) => void) => (value: string) => {
    set(value);
    setPreview(null);
    setMessage(null);
  };

  const runPreview = async () => {
    setBusy(true);
    setMessage(null);
    try {
      setPreview(await apiClient.ownershipPreview({ from: from.trim(), to: to.trim() }));
    } catch (e) {
      setPreview(null);
      setMessage({ text: e instanceof Error ? e.message : "The preview failed.", error: true });
    } finally {
      setBusy(false);
    }
  };

  const runTransfer = async () => {
    setBusy(true);
    setMessage(null);
    try {
      const done = await apiClient.ownershipTransfer({ from: from.trim(), to: to.trim(), reason: reason.trim() });
      setMessage({ text: `${done.workspaces} workspace${done.workspaces === 1 ? "" : "s"} moved to ${done.to}.` });
      setPreview(null);
      setReason("");
      await read();
    } catch (e) {
      setMessage({ text: e instanceof Error ? e.message : "The transfer failed.", error: true });
      await read();
    } finally {
      setBusy(false);
    }
  };

  if (error !== null) return <ErrorState error={error} onRetry={() => void read()} />;
  if (!orphans) return <Skeleton rows={3} />;

  const nothing = orphans.workspaces.length + orphans.proposals.length + orphans.clients.length === 0;
  const ready = preview !== null && preview.can_transfer && preview.from === from.trim() && preview.to === to.trim() && reason.trim().length > 0;

  return (
    <div className="space-y-8 text-sm" data-testid="admin-ownership">
      <section aria-label="Orphaned" className="space-y-3">
        <h3 className="text-sm font-semibold text-gray-800 dark:text-gray-200">Nobody is looking after these</h3>
        {nothing ? (
          <EmptyState title="Every workspace has an owner who is here." description="A workspace appears here when each of its owners is no longer active, is on hold, or has no current appointment." />
        ) : (
          <div data-testid="ownership-orphans" className="space-y-4">
            {orphans.workspaces.map((w) => (
              <div key={w.id} className="rounded border border-amber-300 dark:border-amber-700 p-3">
                <div className="font-medium text-gray-900 dark:text-gray-100">{w.name}</div>
                <div className="text-xs font-mono text-gray-500">{w.id}</div>
                {w.owners.length === 0 ? (
                  <div className="text-gray-700 dark:text-gray-300">It has no owner.</div>
                ) : (
                  <ul className="list-disc pl-5 text-gray-700 dark:text-gray-300">
                    {w.owners.map((o) => (
                      <li key={o.eppn}>
                        {o.eppn} {gone(o.why)}.
                      </li>
                    ))}
                  </ul>
                )}
              </div>
            ))}
            {orphans.proposals.length > 0 && (
              <div>
                <h4 className="font-medium text-gray-800 dark:text-gray-200">Open proposals from people who have gone</h4>
                <ul className="list-disc pl-5 text-gray-700 dark:text-gray-300">
                  {orphans.proposals.map((p) => (
                    <li key={p.id}>
                      {p.name} in {p.workspace_id}, proposed by {p.proposed_by}, who {gone(p.why)}.
                    </li>
                  ))}
                </ul>
              </div>
            )}
            {orphans.clients.length > 0 && (
              <div>
                <h4 className="font-medium text-gray-800 dark:text-gray-200">Outside apps from people who have gone</h4>
                <ul className="list-disc pl-5 text-gray-700 dark:text-gray-300">
                  {orphans.clients.map((c) => (
                    <li key={c.id}>
                      {c.name}, proposed by {c.proposed_by}, who {gone(c.why)}.
                    </li>
                  ))}
                </ul>
              </div>
            )}
          </div>
        )}
      </section>

      <section aria-label="Transfer" className="space-y-3 max-w-xl">
        <h3 className="text-sm font-semibold text-gray-800 dark:text-gray-200">Hand over everything one person owns</h3>
        <FormField id="ownership-from" label="From" hint="The person who is leaving, by address.">
          {(props) => <input {...props} className={inputClass} value={from} onChange={(e) => change(setFrom)(e.target.value)} />}
        </FormField>
        <FormField id="ownership-to" label="To" hint="The person who takes over. They must be active.">
          {(props) => <input {...props} className={inputClass} value={to} onChange={(e) => change(setTo)(e.target.value)} />}
        </FormField>
        <FormField id="ownership-reason" label="Reason" hint="Written to the ledger before anything changes.">
          {(props) => <input {...props} className={inputClass} value={reason} onChange={(e) => setReason(e.target.value)} />}
        </FormField>
        <div className="flex gap-2">
          <button
            type="button"
            disabled={busy || from.trim() === "" || to.trim() === ""}
            onClick={() => void runPreview()}
            className="px-3 py-1.5 rounded border border-gray-300 dark:border-gray-700 text-gray-800 dark:text-gray-200 disabled:opacity-50"
          >
            Preview transfer
          </button>
          <button
            type="button"
            disabled={busy || !ready}
            onClick={() => void runTransfer()}
            className="px-3 py-1.5 rounded bg-amber-600 text-white disabled:opacity-50"
          >
            Transfer
          </button>
        </div>
        {message && (
          <div role={message.error ? "alert" : "status"} className={message.error ? "text-red-700 dark:text-red-400" : "text-green-700 dark:text-green-400"}>
            {message.text}
          </div>
        )}
        {preview && preview.blockers.length > 0 && (
          <div role="alert" className="text-red-700 dark:text-red-400">
            {preview.blockers.map((b) => (
              <p key={b}>{b}</p>
            ))}
          </div>
        )}
        {preview && (
          <div data-testid="ownership-preview" className="space-y-3">
            {preview.workspaces.map((w) => (
              <div key={w.id} className="rounded border border-gray-200 dark:border-gray-800 p-3 space-y-1">
                <div className="font-medium text-gray-900 dark:text-gray-100">{w.name}</div>
                {[...w.gains, ...w.loses].map((line) => (
                  <p key={line} className="text-gray-700 dark:text-gray-300">
                    {line}
                  </p>
                ))}
                {w.open_proposals.map((p) => (
                  <p key={p.id} className="text-gray-700 dark:text-gray-300">
                    Waiting on the owner: {p.summary} ({p.app_slug})
                  </p>
                ))}
                {w.process_steps.map((p) => (
                  <p key={p.id} className="text-gray-700 dark:text-gray-300">
                    Process step waiting: {p.prompt || p.id} ({p.app_slug})
                  </p>
                ))}
                {w.tokens.map((t) => (
                  <p key={t.id} className="text-gray-700 dark:text-gray-300">
                    Agent token revoked: {t.label}
                  </p>
                ))}
              </div>
            ))}
            {preview.clients_from_created.length > 0 && (
              <p className="text-gray-700 dark:text-gray-300">
                Outside apps {preview.from} proposed stay as they are: {preview.clients_from_created.map((c) => `${c.name} (${c.status})`).join(", ")}.
              </p>
            )}
          </div>
        )}
      </section>
    </div>
  );
}
