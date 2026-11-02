import { useCallback, useEffect, useState, type FormEvent } from "react";
import { apiClient, type SecretListing } from "../api";
import { ConfirmAction, EmptyState, ErrorState, PageHeader, Skeleton } from "../ui";

const buttonClass = "px-3 py-1.5 text-sm font-medium rounded border border-gray-300 dark:border-gray-700 disabled:opacity-50";
const inputClass = "w-full text-sm rounded border border-gray-300 dark:border-gray-700 p-2 bg-white dark:bg-gray-800";

function when(iso: string): string {
  const d = new Date(iso);
  return Number.isNaN(d.getTime()) ? iso : d.toLocaleString("en-US", { dateStyle: "medium", timeStyle: "short" });
}

/** A value field that is only ever written: masked, empty when it opens, never filled from the server. */
function ValueField({ id, label, value, onChange }: { id: string; label: string; value: string; onChange: (v: string) => void }) {
  return (
    <label htmlFor={id} className="block text-xs font-medium">
      {label}
      <input id={id} type="password" autoComplete="off" spellCheck={false} className={inputClass} value={value} onChange={(e) => onChange(e.target.value)} />
    </label>
  );
}

type Draft = { name: string; value: string; reason: string };

/** A form that writes one secret: a new one, or a new value for one that exists. It clears itself when it closes. */
function SecretForm({ name, onDone, onCancel }: { name?: string; onDone: () => void; onCancel: () => void }) {
  const [draft, setDraft] = useState<Draft>({ name: name ?? "", value: "", reason: "" });
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<unknown>(null);
  const set = (p: Partial<Draft>) => setDraft((d) => ({ ...d, ...p }));
  const replacing = name !== undefined;

  const submit = async (e: FormEvent) => {
    e.preventDefault();
    setBusy(true);
    setError(null);
    try {
      await apiClient.putSecret(draft.name.trim(), draft.value, draft.reason.trim());
      setDraft({ name: "", value: "", reason: "" });
      onDone();
    } catch (err) {
      setError(err);
    } finally {
      setBusy(false);
    }
  };

  return (
    <form onSubmit={(e) => void submit(e)} className="space-y-3 rounded border border-gray-200 dark:border-gray-800 p-3">
      {error !== null && <ErrorState error={error} />}
      {!replacing && (
        <label htmlFor="secret-name" className="block text-xs font-medium">
          Name
          <input id="secret-name" className={inputClass} value={draft.name} onChange={(e) => set({ name: e.target.value })} placeholder="registrar_hook" />
        </label>
      )}
      <ValueField id={replacing ? `secret-new-${name}` : "secret-value"} label={replacing ? "New value" : "Value"} value={draft.value} onChange={(value) => set({ value })} />
      <p className="text-xs text-gray-500 dark:text-gray-400">The value is stored encrypted. No one can read it back, here or anywhere.</p>
      <label htmlFor={replacing ? `secret-reason-${name}` : "secret-reason"} className="block text-xs font-medium">
        Reason
        <textarea id={replacing ? `secret-reason-${name}` : "secret-reason"} rows={2} className={inputClass} value={draft.reason} onChange={(e) => set({ reason: e.target.value })} />
      </label>
      <div className="flex gap-2">
        <button type="submit" className={buttonClass} disabled={busy || !draft.name.trim() || !draft.value || !draft.reason.trim()}>
          {replacing ? "Save new value" : "Save secret"}
        </button>
        <button type="button" className={buttonClass} onClick={onCancel}>
          Cancel
        </button>
      </div>
    </form>
  );
}

// The Secrets section of the console (integrations phase 1). Names, dates and what uses each secret. Never a value.
export function Secrets() {
  const [rows, setRows] = useState<SecretListing[] | null>(null);
  const [error, setError] = useState<unknown>(null);
  const [adding, setAdding] = useState(false);
  const [replacing, setReplacing] = useState<string | null>(null);
  const [deleting, setDeleting] = useState<string | null>(null);

  const load = useCallback(async () => {
    setError(null);
    try {
      setRows(await apiClient.listSecrets());
    } catch (e) {
      setError(e);
      setRows([]);
    }
  }, []);

  useEffect(() => {
    void load();
  }, [load]);

  const done = () => {
    setAdding(false);
    setReplacing(null);
    setDeleting(null);
    void load();
  };

  return (
    <div className="space-y-4">
      <PageHeader title="Secrets" description="Credentials the server uses to reach other systems. A secret is written here and used by name. It is never shown again." />
      {error !== null ? (
        <ErrorState error={error} />
      ) : (
        <>
          {!adding && (
            <button type="button" className={buttonClass} onClick={() => setAdding(true)}>
              Add secret
            </button>
          )}
          {adding && <SecretForm onDone={done} onCancel={() => setAdding(false)} />}
          {rows === null ? (
            <Skeleton rows={3} />
          ) : rows.length === 0 ? (
            <EmptyState title="No secrets" description="Add one when a webhook or a feed needs a credential." />
          ) : (
            <ul className="space-y-2">
              {rows.map((s) => (
                <li key={s.name} className="rounded border border-gray-200 dark:border-gray-800 p-3 space-y-2">
                  <div className="flex flex-wrap items-center justify-between gap-2">
                    <div>
                      <div className="font-mono text-sm font-semibold">{s.name}</div>
                      <div className="text-xs text-gray-500 dark:text-gray-400">
                        Added {when(s.created_at)} by {s.created_by}
                        {s.rotated_at ? ` · Replaced ${when(s.rotated_at)}` : ""}
                      </div>
                    </div>
                    <span className="text-xs text-gray-600 dark:text-gray-300">{s.references === 0 ? "Not used" : `Used by ${s.references}`}</span>
                  </div>
                  {replacing === s.name ? (
                    <SecretForm name={s.name} onDone={done} onCancel={() => setReplacing(null)} />
                  ) : deleting === s.name ? (
                    <ConfirmAction
                      verb="Delete"
                      target={s.name}
                      consequence={`Anything that uses ${s.name} must be changed first. A deleted secret cannot be recovered.`}
                      onConfirm={(reason) => apiClient.deleteSecret(s.name, reason)}
                      onSuccess={done}
                    />
                  ) : (
                    <div className="flex gap-2">
                      <button type="button" className={buttonClass} aria-label={`Replace ${s.name}`} onClick={() => setReplacing(s.name)}>
                        Replace
                      </button>
                      <button type="button" className={buttonClass} aria-label={`Delete ${s.name}`} onClick={() => setDeleting(s.name)}>
                        Delete
                      </button>
                    </div>
                  )}
                </li>
              ))}
            </ul>
          )}
        </>
      )}
    </div>
  );
}
