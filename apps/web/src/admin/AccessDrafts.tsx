import { useState } from "react";
import { apiClient, type AccessChange } from "../api";
import { ConfirmAction, Drawer } from "../ui";
import type { AccessViewer } from "./Access";

const buttonClass = "px-2 py-1 text-xs font-medium rounded border border-gray-300 dark:border-gray-700 hover:bg-gray-50 dark:hover:bg-gray-800";

/** Changes an AI tool drafted. Each says who it was made for, the task, what it does, and when it ends. A person decides. */
export function AccessDrafts({ changes, viewer, onChanged }: { changes: AccessChange[]; viewer: AccessViewer; onChanged: () => void }) {
  const [deciding, setDeciding] = useState<{ change: AccessChange; decision: "confirm" | "reject" } | null>(null);
  const drafts = changes.filter((c) => c.proposed_via === "agent");
  if (drafts.length === 0) return null;
  return (
    <section aria-label="Drafted by an AI tool" className="space-y-2" data-testid="access-drafts">
      <h3 className="text-sm font-semibold text-gray-800 dark:text-gray-200">Drafted by an AI tool</h3>
      {drafts.map((c) => {
        const mine = (c.drafted_for ?? c.proposed_by).toLowerCase() === viewer.eppn.toLowerCase();
        const mayConfirm = !c.expired && (!mine || !c.widens);
        return (
          <div key={c.change_id} data-testid={`draft-${c.change_id}`} className="border border-indigo-200 dark:border-indigo-900/60 rounded-lg p-3 text-sm space-y-1">
            <div className="text-xs text-gray-500">Drafted by an AI tool used by {c.drafted_for ?? c.proposed_by}</div>
            {c.task && <div className="text-xs text-gray-500">While it was: {c.task}</div>}
            <div className="text-gray-900 dark:text-gray-100">{c.sentence ?? c.reason}</div>
            <div className="text-xs text-gray-500">{c.reason}</div>
            <div className="text-xs text-gray-500">{c.expired ? "Expired. Ask for it again." : c.expires_at ? `Ends ${c.expires_at.split("T")[0]}.` : ""}</div>
            {!c.expired && c.widens && mine && <div className="text-xs text-amber-700 dark:text-amber-400">A different person must confirm this. It gives access.</div>}
            <div className="flex gap-2 pt-1">
              {mayConfirm && (
                <button type="button" className={buttonClass} onClick={() => setDeciding({ change: c, decision: "confirm" })}>
                  Confirm
                </button>
              )}
              {!c.expired && (
                <button type="button" className={buttonClass} onClick={() => setDeciding({ change: c, decision: "reject" })}>
                  Reject
                </button>
              )}
            </div>
          </div>
        );
      })}
      <Drawer open={deciding !== null} onClose={() => setDeciding(null)} title={deciding?.decision === "confirm" ? "Confirm a draft" : "Reject a draft"}>
        {deciding && (
          <ConfirmAction
            key={`${deciding.change.change_id}-${deciding.decision}`}
            verb={deciding.decision === "confirm" ? "Confirm" : "Reject"}
            target="draft"
            consequence={deciding.decision === "confirm" ? (deciding.change.sentence ?? "The change takes effect now.") : "The change is not made."}
            onConfirm={async (reason) => {
              await apiClient.accessDecideChange(deciding.change.change_id, deciding.decision, reason);
            }}
            onSuccess={() => {
              setDeciding(null);
              onChanged();
            }}
          />
        )}
      </Drawer>
    </section>
  );
}
