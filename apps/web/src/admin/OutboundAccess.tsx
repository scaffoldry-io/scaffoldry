import { useCallback, useEffect, useState, type FormEvent } from "react";
import { apiClient, type EgressEntry, type EgressTestResult } from "../api";
import { ConfirmAction, EmptyState, ErrorState, PageHeader, Skeleton } from "../ui";

const buttonClass = "px-3 py-1.5 text-sm font-medium rounded border border-gray-300 dark:border-gray-700 disabled:opacity-50";
const inputClass = "w-full text-sm rounded border border-gray-300 dark:border-gray-700 p-2 bg-white dark:bg-gray-800";

/** "443, 8443" as [443, 8443]. Null when any part is not a port. */
function parsePorts(text: string): number[] | null {
  const parts = text.split(",").map((p) => p.trim()).filter((p) => p !== "");
  const ports = parts.map((p) => (/^\d+$/.test(p) ? Number(p) : NaN));
  return ports.length > 0 && ports.every((p) => p >= 1 && p <= 65535) ? ports : null;
}

const nameOf = (e: EgressEntry): string => e.host ?? e.cidr ?? "";

type Kind = "host" | "cidr";

/** One entry, added or edited, and the reason for the change. */
function EntryForm({ entry, onSave, onCancel }: { entry?: EgressEntry; onSave: (next: EgressEntry, reason: string) => Promise<void>; onCancel: () => void }) {
  const [kind, setKind] = useState<Kind>(entry?.cidr !== undefined ? "cidr" : "host");
  const [name, setName] = useState(entry ? nameOf(entry) : "");
  const [ports, setPorts] = useState(entry ? entry.ports.join(", ") : "443");
  const [reason, setReason] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<unknown>(null);
  const parsed = parsePorts(ports);
  const ready = name.trim() !== "" && parsed !== null && reason.trim() !== "";
  const subject = entry ? nameOf(entry) : "new";

  const submit = async (e: FormEvent) => {
    e.preventDefault();
    if (!ready || parsed === null) return;
    setBusy(true);
    setError(null);
    try {
      await onSave(kind === "host" ? { host: name.trim(), ports: parsed } : { cidr: name.trim(), ports: parsed }, reason.trim());
    } catch (err) {
      setError(err);
      setBusy(false);
    }
  };

  return (
    <form onSubmit={(e) => void submit(e)} className="space-y-3 rounded border border-gray-200 dark:border-gray-800 p-3">
      {error !== null && <ErrorState error={error} />}
      {!entry && (
        <label htmlFor="egress-kind" className="block text-xs font-medium">
          Kind
          <select id="egress-kind" className={inputClass} value={kind} onChange={(e) => setKind(e.target.value as Kind)}>
            <option value="host">A host by name</option>
            <option value="cidr">A network on purpose</option>
          </select>
        </label>
      )}
      <label htmlFor={`egress-name-${subject}`} className="block text-xs font-medium">
        {kind === "host" ? "Host" : "Network"}
        <input id={`egress-name-${subject}`} className={inputClass} value={name} onChange={(e) => setName(e.target.value)} placeholder={kind === "host" ? "registrar.example.edu" : "10.20.0.0/16"} spellCheck={false} />
      </label>
      <label htmlFor={`egress-ports-${subject}`} className="block text-xs font-medium">
        Ports
        <input id={`egress-ports-${subject}`} className={inputClass} value={ports} onChange={(e) => setPorts(e.target.value)} />
      </label>
      {parsed === null && <p className="text-xs text-rose-600">Give ports from 1 to 65535, separated by commas.</p>}
      {kind === "cidr" && <p className="text-xs text-gray-500 dark:text-gray-400">A network entry is the only way to reach a private address. Name only a network you mean to reach.</p>}
      <label htmlFor={`egress-reason-${subject}`} className="block text-xs font-medium">
        Reason
        <textarea id={`egress-reason-${subject}`} rows={2} className={inputClass} value={reason} onChange={(e) => setReason(e.target.value)} />
      </label>
      <div className="flex gap-2">
        <button type="submit" className={buttonClass} disabled={busy || !ready}>
          Save entry
        </button>
        <button type="button" className={buttonClass} onClick={onCancel}>
          Cancel
        </button>
      </div>
    </form>
  );
}

/** Asks the outbound check about a host and shows what it says, in words. */
function HostTest() {
  const [host, setHost] = useState("");
  const [port, setPort] = useState("443");
  const [result, setResult] = useState<EgressTestResult | null>(null);
  const [error, setError] = useState<unknown>(null);
  const [busy, setBusy] = useState(false);
  const portNumber = parsePorts(port)?.[0];

  const run = async (e: FormEvent) => {
    e.preventDefault();
    if (portNumber === undefined) return;
    setBusy(true);
    setError(null);
    try {
      setResult(await apiClient.testEgress(host.trim(), portNumber));
    } catch (err) {
      setResult(null);
      setError(err);
    } finally {
      setBusy(false);
    }
  };

  return (
    <form onSubmit={(e) => void run(e)} className="space-y-2 rounded border border-gray-200 dark:border-gray-800 p-3">
      <h3 className="text-sm font-semibold">Check a host</h3>
      <p className="text-xs text-gray-500 dark:text-gray-400">Nothing is sent. This asks the same check a real connection passes through.</p>
      <div className="flex flex-wrap items-end gap-2">
        <label htmlFor="egress-test-host" className="text-xs font-medium flex-1 min-w-48">
          Host to test
          <input id="egress-test-host" className={inputClass} value={host} onChange={(e) => setHost(e.target.value)} spellCheck={false} />
        </label>
        <label htmlFor="egress-test-port" className="text-xs font-medium w-24">
          Port
          <input id="egress-test-port" className={inputClass} value={port} onChange={(e) => setPort(e.target.value)} />
        </label>
        <button type="submit" className={buttonClass} disabled={busy || host.trim() === "" || portNumber === undefined}>
          Test
        </button>
      </div>
      {error !== null && <ErrorState error={error} />}
      {result && (
        <p role="status" className={`text-sm ${result.allowed ? "text-emerald-700 dark:text-emerald-400" : "text-rose-700 dark:text-rose-400"}`}>
          {result.explanation}
        </p>
      )}
    </form>
  );
}

// The Outbound access section of the console (integrations phase 5): the allowlist, and a way to test a host.
export function OutboundAccess() {
  const [rows, setRows] = useState<EgressEntry[] | null>(null);
  const [error, setError] = useState<unknown>(null);
  const [adding, setAdding] = useState(false);
  const [editing, setEditing] = useState<number | null>(null);
  const [removing, setRemoving] = useState<number | null>(null);

  const load = useCallback(async () => {
    setError(null);
    try {
      setRows(await apiClient.getEgress());
    } catch (e) {
      setError(e);
      setRows([]);
    }
  }, []);

  useEffect(() => {
    void load();
  }, [load]);

  /** Saves the whole list, then closes the forms and reads it again. */
  const save = async (next: EgressEntry[], reason: string) => {
    await apiClient.saveEgress(next, reason);
    setAdding(false);
    setEditing(null);
    setRemoving(null);
    await load();
  };

  const list = rows ?? [];

  return (
    <div className="space-y-4">
      <PageHeader title="Outbound access" description="Where the server may connect. Nothing leaves until an entry allows it. Each change is recorded with its reason." />
      {error !== null ? (
        <ErrorState error={error} />
      ) : (
        <>
          {!adding && (
            <button type="button" className={buttonClass} onClick={() => setAdding(true)}>
              Add entry
            </button>
          )}
          {adding && <EntryForm onSave={(next, reason) => save([...list, next], reason)} onCancel={() => setAdding(false)} />}
          {rows === null ? (
            <Skeleton rows={3} />
          ) : list.length === 0 ? (
            <EmptyState title="Nothing is allowed" description="No connection leaves the server until you add a host." />
          ) : (
            <ul className="space-y-2">
              {list.map((entry, i) => (
                <li key={`${nameOf(entry)}-${i}`} className="rounded border border-gray-200 dark:border-gray-800 p-3 space-y-2">
                  <div className="flex flex-wrap items-center justify-between gap-2">
                    <div>
                      <div className="font-mono text-sm font-semibold">{nameOf(entry)}</div>
                      <div className="text-xs text-gray-500 dark:text-gray-400">{entry.cidr !== undefined ? "A network, named on purpose" : "A host"}</div>
                    </div>
                    <div className="text-xs text-gray-600 dark:text-gray-300">
                      <span className="sr-only">Ports </span>
                      {entry.ports.join(", ")}
                    </div>
                  </div>
                  {editing === i ? (
                    <EntryForm entry={entry} onSave={(next, reason) => save(list.map((e, j) => (j === i ? next : e)), reason)} onCancel={() => setEditing(null)} />
                  ) : removing === i ? (
                    <ConfirmAction
                      verb="Remove"
                      target={nameOf(entry)}
                      consequence={`Webhooks and feeds that use ${nameOf(entry)} stop reaching it.`}
                      onConfirm={(reason) => apiClient.saveEgress(list.filter((_, j) => j !== i), reason)}
                      onSuccess={() => {
                        setRemoving(null);
                        void load();
                      }}
                    />
                  ) : (
                    <div className="flex gap-2">
                      <button type="button" className={buttonClass} aria-label={`Edit ${nameOf(entry)}`} onClick={() => setEditing(i)}>
                        Edit
                      </button>
                      <button type="button" className={buttonClass} aria-label={`Remove ${nameOf(entry)}`} onClick={() => setRemoving(i)}>
                        Remove
                      </button>
                    </div>
                  )}
                </li>
              ))}
            </ul>
          )}
          <HostTest />
        </>
      )}
    </div>
  );
}
