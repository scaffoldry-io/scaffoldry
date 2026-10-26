import { useCallback, useEffect, useState } from "react";
import { apiClient, type AccessRequest } from "./api";

/** One line that says what a request is for. */
function what(r: AccessRequest): string {
  return `${r.role} on ${r.object_kind} ${r.object_id} for ${r.days} days`;
}

function stateOf(r: AccessRequest): string {
  if (r.status === "Pending") return `Your request waits for ${r.waiting_on && r.waiting_on.length > 0 ? r.waiting_on.join(", ") : "an approver"}.`;
  if (r.status === "Waiting for person") return "Waiting for you to confirm.";
  return r.status;
}

/** A person's own requests for access, and the ones that wait for their decision. */
export function AccessRequestsPanel() {
  const [mine, setMine] = useState<AccessRequest[]>([]);
  const [toDecide, setToDecide] = useState<AccessRequest[]>([]);
  const [notes, setNotes] = useState<Record<string, string>>({});
  const [failure, setFailure] = useState<string | null>(null);

  const load = useCallback(async () => {
    try {
      const [m, d] = await Promise.all([apiClient.myAccessRequests(), apiClient.accessRequestsToDecide()]);
      setMine(m?.requests ?? []);
      setToDecide(d?.requests ?? []);
    } catch (e) {
      setFailure(e instanceof Error ? e.message : "Your requests could not be read.");
    }
  }, []);
  useEffect(() => {
    void load();
  }, [load]);

  const act = async (run: () => Promise<unknown>) => {
    setFailure(null);
    try {
      await run();
      await load();
    } catch (e) {
      setFailure(e instanceof Error ? e.message : "That did not work.");
    }
  };

  return (
    <div className="space-y-3 text-xs" data-testid="access-requests">
      <h3 className="text-xs font-semibold text-slate-900 dark:text-white uppercase tracking-wider">Access requests</h3>
      {failure && <p role="alert" className="text-red-600">{failure}</p>}
      {mine.length === 0 ? (
        <p className="text-slate-400">You have not asked for access.</p>
      ) : (
        <ul className="space-y-2">
          {mine.map((r) => (
            <li key={r.id} className="flex items-start justify-between gap-3">
              <div>
                <div className="font-medium text-slate-900 dark:text-white">{r.reason}</div>
                <div className="text-slate-600 dark:text-slate-300">{what(r)}</div>
                {r.task && <div className="text-slate-500">Filed by an AI tool while it was: {r.task}</div>}
                <div className="text-slate-500">{stateOf(r)}</div>
              </div>
              <div className="shrink-0 flex gap-2">
                {r.status === "Waiting for person" && (
                  <button type="button" aria-label={`Confirm: ${r.reason}`} onClick={() => void act(() => apiClient.confirmAccessRequest(r.id))} className="px-2 py-1 rounded bg-emerald-50 text-emerald-700 font-medium">
                    Confirm
                  </button>
                )}
                {(r.status === "Waiting for person" || r.status === "Pending") && (
                  <button type="button" aria-label={`Withdraw: ${r.reason}`} onClick={() => void act(() => apiClient.withdrawAccessRequest(r.id))} className="px-2 py-1 rounded bg-slate-100 text-slate-700 font-medium">
                    Withdraw
                  </button>
                )}
              </div>
            </li>
          ))}
        </ul>
      )}
      {toDecide.length > 0 && (
        <div className="space-y-2">
          <h4 className="font-semibold text-slate-900 dark:text-white">Waiting for your decision</h4>
          <ul className="space-y-2">
            {toDecide.map((r) => (
              <li key={r.id} className="space-y-1">
                <div className="font-medium text-slate-900 dark:text-white">
                  {r.requester}: {r.reason}
                </div>
                <div className="text-slate-600 dark:text-slate-300">{what(r)}</div>
                <input
                  aria-label={`Note for ${r.reason}`}
                  value={notes[r.id] ?? ""}
                  onChange={(e) => setNotes({ ...notes, [r.id]: e.target.value })}
                  placeholder="Why"
                  className="w-full rounded border border-slate-300 dark:border-slate-700 bg-white dark:bg-slate-900 p-1"
                />
                <div className="flex gap-2">
                  <button type="button" aria-label={`Approve: ${r.reason}`} onClick={() => void act(() => apiClient.decideAccessRequest(r.id, "approve", notes[r.id] ?? ""))} className="px-2 py-1 rounded bg-emerald-50 text-emerald-700 font-medium">
                    Approve
                  </button>
                  <button type="button" aria-label={`Reject: ${r.reason}`} onClick={() => void act(() => apiClient.decideAccessRequest(r.id, "reject", notes[r.id] ?? ""))} className="px-2 py-1 rounded bg-red-50 text-red-700 font-medium">
                    Reject
                  </button>
                </div>
              </li>
            ))}
          </ul>
        </div>
      )}
    </div>
  );
}
