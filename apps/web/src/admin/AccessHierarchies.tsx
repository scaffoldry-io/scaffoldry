import { useCallback, useEffect, useState, type FormEvent } from "react";
import { apiClient, type AccessChange, type AccessHierarchy } from "../api";
import { ConfirmAction, Drawer, ErrorState, FormField, StatusBadge } from "../ui";
import type { AccessViewer } from "./Access";

const inputClass =
  "w-full text-sm rounded border border-gray-300 dark:border-gray-700 p-2 bg-white dark:bg-gray-800 text-gray-900 dark:text-gray-100";
const buttonClass = "px-2 py-1 text-xs font-medium rounded border border-gray-300 dark:border-gray-700 hover:bg-gray-50 dark:hover:bg-gray-800";

export function AccessHierarchies({ viewer }: { viewer: AccessViewer }) {
  const [hierarchies, setHierarchies] = useState<AccessHierarchy[]>([]);
  const [pending, setPending] = useState<AccessChange[]>([]);
  const [error, setError] = useState<unknown>(null);
  const [creating, setCreating] = useState(false);
  const [retiring, setRetiring] = useState<AccessHierarchy | null>(null);
  const [deciding, setDeciding] = useState<AccessChange | null>(null);

  const load = useCallback(async () => {
    try {
      const [h, c] = await Promise.all([apiClient.accessHierarchies(), apiClient.accessGrants({ state: "pending" })]);
      setHierarchies(h);
      setPending(c.changes.filter((x) => x.kind === "hierarchy"));
      setError(null);
    } catch (e) {
      setError(e);
    }
  }, []);

  useEffect(() => {
    void load();
  }, [load]);

  return (
    <div className="space-y-6">
      <div className="flex items-center justify-between">
        <p className="text-sm text-gray-600 dark:text-gray-400">
          A hierarchy is a tree of units. A new one needs a different Platform Admin to confirm it. A hierarchy is never deleted.
        </p>
        <button type="button" onClick={() => setCreating(true)} className="px-3 py-1.5 text-sm font-medium rounded-md bg-blue-600 text-white hover:bg-blue-700">
          New hierarchy
        </button>
      </div>
      {error !== null && <ErrorState error={error} onRetry={() => void load()} />}

      {pending.length > 0 && (
        <section aria-label="Waiting for confirmation" className="space-y-2">
          <h3 className="text-sm font-semibold text-gray-800 dark:text-gray-200">Waiting for confirmation</h3>
          {pending.map((c) => (
            <div key={c.change_id} data-testid={`hierarchy-pending-${c.change_id}`} className="border border-amber-200 dark:border-amber-900/60 rounded-lg p-3 flex items-center justify-between gap-4">
              <div className="text-sm space-y-0.5">
                <div className="font-medium text-gray-900 dark:text-gray-100">{c.hierarchy?.name}</div>
                <div className="text-gray-600 dark:text-gray-400">
                  {c.hierarchy?.key} · {c.hierarchy?.kind} · from {c.hierarchy?.source}. {c.reason}
                </div>
                <div className="text-xs text-gray-500">Proposed by {c.proposed_by}</div>
                {c.proposed_by.toLowerCase() === viewer.eppn.toLowerCase() && (
                  <div className="text-xs font-medium text-amber-700 dark:text-amber-400">Waiting for a different person to confirm. It cannot be used until then.</div>
                )}
              </div>
              {c.proposed_by.toLowerCase() !== viewer.eppn.toLowerCase() && (
                <button type="button" onClick={() => setDeciding(c)} className={buttonClass}>
                  Confirm or reject
                </button>
              )}
            </div>
          ))}
        </section>
      )}

      <section aria-label="Hierarchies" data-testid="hierarchy-list" className="space-y-2">
        {hierarchies.map((h) => (
          <div key={h.key} data-testid={`hierarchy-row-${h.key}`} className="border border-gray-200 dark:border-gray-800 rounded-lg p-3 flex items-center justify-between gap-4">
            <div className="text-sm">
              <div className="font-medium text-gray-900 dark:text-gray-100">{h.name}</div>
              <div className="text-gray-600 dark:text-gray-400">
                {h.key} · {h.kind === "people" ? "a hierarchy of people" : "a hierarchy of units"} · from {h.source}
              </div>
            </div>
            <div className="flex items-center gap-3">
              {h.status === "Active" ? <StatusBadge tone="ok" text="In use" /> : <StatusBadge tone="neutral" text="Retired" />}
              {h.status === "Active" && h.key !== "academic" && (
                <button type="button" onClick={() => setRetiring(h)} className={buttonClass}>
                  Retire
                </button>
              )}
            </div>
          </div>
        ))}
      </section>

      <Drawer open={creating} onClose={() => setCreating(false)} title="New hierarchy">
        <NewHierarchy
          onDone={() => {
            void load();
          }}
        />
      </Drawer>
      <Drawer open={retiring !== null} onClose={() => setRetiring(null)} title="Retire a hierarchy">
        {retiring && (
          <ConfirmAction
            verb="Retire"
            target="hierarchy"
            consequence={`Grants in ${retiring.name} stop reaching. Its units and coordinates stay for audit. A hierarchy is never deleted.`}
            onConfirm={async (reason) => {
              await apiClient.accessRetireHierarchy(retiring.key, reason);
            }}
            onSuccess={() => {
              setRetiring(null);
              void load();
            }}
          />
        )}
      </Drawer>
      <Drawer open={deciding !== null} onClose={() => setDeciding(null)} title="Decide on a hierarchy">
        {deciding && (
          <Decide
            change={deciding}
            onDone={() => {
              setDeciding(null);
              void load();
            }}
          />
        )}
      </Drawer>
    </div>
  );
}

function Decide({ change, onDone }: { change: AccessChange; onDone: () => void }) {
  const [decision, setDecision] = useState<"confirm" | "reject">("confirm");
  return (
    <div className="space-y-4">
      <p className="text-sm text-gray-700 dark:text-gray-300">
        {change.proposed_by} proposed the hierarchy {change.hierarchy?.name} ({change.hierarchy?.key}). {change.reason}
      </p>
      <fieldset className="flex gap-4 text-sm">
        <legend className="sr-only">Decision</legend>
        <label className="flex items-center gap-1.5">
          <input type="radio" name="decision" checked={decision === "confirm"} onChange={() => setDecision("confirm")} />
          Confirm
        </label>
        <label className="flex items-center gap-1.5">
          <input type="radio" name="decision" checked={decision === "reject"} onChange={() => setDecision("reject")} />
          Reject
        </label>
      </fieldset>
      <ConfirmAction
        key={decision}
        verb={decision === "confirm" ? "Confirm" : "Reject"}
        target="hierarchy"
        consequence={decision === "confirm" ? "The hierarchy exists from now on, with a root unit." : "The hierarchy is not made."}
        onConfirm={async (reason) => {
          await apiClient.accessDecideChange(change.change_id, decision, reason);
        }}
        onSuccess={onDone}
      />
    </div>
  );
}

function NewHierarchy({ onDone }: { onDone: () => void }) {
  const [key, setKey] = useState("");
  const [name, setName] = useState("");
  const [kind, setKind] = useState("units");
  const [source, setSource] = useState("api");
  const [reason, setReason] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<unknown>(null);
  const [message, setMessage] = useState<string | null>(null);
  const valid = key.trim() !== "" && name.trim() !== "" && reason.trim() !== "";

  const submit = async (e: FormEvent) => {
    e.preventDefault();
    if (!valid || busy) return;
    setBusy(true);
    setError(null);
    try {
      const res = await apiClient.accessCreateHierarchy({ key: key.trim(), name: name.trim(), kind, source, reason: reason.trim() });
      setMessage(res.message);
      onDone();
    } catch (err) {
      setError(err);
    } finally {
      setBusy(false);
    }
  };

  if (message) return <p className="text-sm text-gray-700 dark:text-gray-300">{message}</p>;

  return (
    <form onSubmit={submit} className="space-y-4" data-testid="hierarchy-create-form">
      {error !== null && <ErrorState error={error} />}
      <FormField id="hierarchy-key" label="Key" hint="Lowercase letters, digits and -, starting with a letter" required>
        {(props) => <input {...props} className={inputClass} value={key} onChange={(e) => setKey(e.target.value)} />}
      </FormField>
      <FormField id="hierarchy-name" label="Name" required>
        {(props) => <input {...props} className={inputClass} value={name} onChange={(e) => setName(e.target.value)} />}
      </FormField>
      <FormField id="hierarchy-kind" label="Kind" hint="Units hold places. People hold the people appointed in them.">
        {(props) => (
          <select {...props} className={inputClass} value={kind} onChange={(e) => setKind(e.target.value)}>
            <option value="units">Units</option>
            <option value="people">People</option>
          </select>
        )}
      </FormField>
      <FormField id="hierarchy-source" label="Source" hint="Where its units come from">
        {(props) => (
          <select {...props} className={inputClass} value={source} onChange={(e) => setSource(e.target.value)}>
            <option value="api">Made here</option>
            <option value="scim">Kept by SCIM</option>
            <option value="import">Imported</option>
            <option value="connection">A connection</option>
          </select>
        )}
      </FormField>
      <FormField id="hierarchy-reason" label="Reason" required>
        {(props) => <input {...props} className={inputClass} value={reason} onChange={(e) => setReason(e.target.value)} />}
      </FormField>
      <div className="flex justify-end">
        <button type="submit" disabled={!valid || busy} className="px-4 py-2 text-sm font-medium rounded-md bg-blue-600 text-white hover:bg-blue-700 disabled:opacity-50">
          Propose hierarchy
        </button>
      </div>
    </form>
  );
}
