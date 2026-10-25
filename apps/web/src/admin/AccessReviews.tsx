import { useCallback, useEffect, useState } from "react";
import { apiClient, type AccessGrantRow } from "../api";
import { ConfirmAction, Drawer, EmptyState, ErrorState } from "../ui";

const buttonClass = "px-2 py-1 text-xs font-medium rounded border border-gray-300 dark:border-gray-700 hover:bg-gray-50 dark:hover:bg-gray-800";

export function AccessReviews() {
  const [due, setDue] = useState<AccessGrantRow[]>([]);
  const [conflicts, setConflicts] = useState<{ person: string; pair: string }[]>([]);
  const [days, setDays] = useState(365);
  const [more, setMore] = useState<string | null>(null);
  const [error, setError] = useState<unknown>(null);
  const [deciding, setDeciding] = useState<{ grant: AccessGrantRow; decision: "keep" | "revoke" } | null>(null);

  const load = useCallback(async () => {
    try {
      const r = await apiClient.accessReviews();
      setDue(r.due);
      setConflicts(r.conflicts);
      setDays(r.review_days);
      setMore(r.next_cursor);
      setError(null);
    } catch (e) {
      setError(e);
    }
  }, []);
  useEffect(() => {
    void load();
  }, [load]);

  const loadMore = async () => {
    if (!more) return;
    const r = await apiClient.accessReviews(more);
    setDue((d) => [...d, ...r.due]);
    setMore(r.next_cursor);
  };

  return (
    <div className="space-y-6" data-testid="access-reviews">
      {error !== null && <ErrorState error={error} onRetry={() => void load()} />}
      {conflicts.length > 0 && (
        <section aria-label="Conflicts" className="space-y-2">
          <h3 className="text-sm font-semibold text-gray-800 dark:text-gray-200">One person holds both sides of a conflict</h3>
          <ul className="space-y-1 text-sm">
            {conflicts.map((c) => (
              <li key={`${c.person}-${c.pair}`} className="text-gray-900 dark:text-gray-100">
                {c.person} holds {c.pair}.
              </li>
            ))}
          </ul>
        </section>
      )}
      <section aria-label="Due for review" className="space-y-2">
        <h3 className="text-sm font-semibold text-gray-800 dark:text-gray-200">Grants not reviewed for {days} days</h3>
        {due.length === 0 ? (
          <EmptyState title="Nothing is due" description="Every grant has been reviewed within the review period." />
        ) : (
          due.map((g) => (
            <div key={g.id} data-testid={`review-row-${g.id}`} className="border border-gray-200 dark:border-gray-800 rounded-lg p-3 flex items-center justify-between gap-4 text-sm">
              <div>
                <div className="text-gray-900 dark:text-gray-100">
                  {g.holder_id} as {g.role} at {g.anchor_name ?? g.anchor_id ?? "their own unit"}
                </div>
                <div className="text-xs text-gray-500">
                  {g.source}. {g.reason}
                </div>
              </div>
              <div className="flex gap-2">
                <button type="button" className={buttonClass} onClick={() => setDeciding({ grant: g, decision: "keep" })}>
                  Keep
                </button>
                <button type="button" className={buttonClass} onClick={() => setDeciding({ grant: g, decision: "revoke" })}>
                  Revoke
                </button>
              </div>
            </div>
          ))
        )}
        {more && (
          <button type="button" onClick={() => void loadMore()} className="text-xs text-blue-600 hover:underline">
            Show more
          </button>
        )}
      </section>
      <Drawer open={deciding !== null} onClose={() => setDeciding(null)} title={deciding?.decision === "keep" ? "Keep a grant" : "Revoke a grant"}>
        {deciding && (
          <ConfirmAction
            verb="Confirm"
            target={deciding.decision}
            consequence={
              deciding.decision === "keep"
                ? `${deciding.grant.holder_id} keeps the access. The review clock starts again.`
                : `${deciding.grant.holder_id} loses the access at once.`
            }
            onConfirm={async (reason) => {
              await apiClient.accessReviewGrant(deciding.grant.id, deciding.decision, reason);
            }}
            onSuccess={() => {
              setDeciding(null);
              void load();
            }}
          />
        )}
      </Drawer>
    </div>
  );
}
