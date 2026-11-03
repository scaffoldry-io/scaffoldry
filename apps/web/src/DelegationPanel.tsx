import { useCallback, useEffect, useState } from "react";
import { apiClient, type DelegationRow, type MyDelegations } from "./api";
import { Banner, ErrorState, FormField } from "./ui";

/** A day and a short month, such as `14 Oct`, in UTC so every reader sees the same day. */
function day(iso: string): string {
  const d = new Date(iso);
  return Number.isNaN(d.getTime()) ? iso : d.toLocaleDateString("en-GB", { day: "numeric", month: "short", timeZone: "UTC" });
}

const STATE_WORDS: Record<DelegationRow["state"], string> = {
  active: "Active",
  scheduled: "Starts later",
  ended: "Ended",
  revoked: "Revoked",
};

function what(d: DelegationRow): string {
  return d.position_key ? `as ${d.position_key}` : "for every position";
}

/** Searches people by the start of their name or address, and shows each with their address. */
function PersonSearch({ onPick }: { onPick: (p: { eppn: string; name: string }) => void }) {
  const [query, setQuery] = useState("");
  const [found, setFound] = useState<{ eppn: string; name: string }[]>([]);
  useEffect(() => {
    if (query.trim().length < 2) {
      setFound([]);
      return;
    }
    let alive = true;
    apiClient
      .searchDelegates(query.trim())
      .then((list) => alive && setFound(list))
      .catch(() => alive && setFound([]));
    return () => {
      alive = false;
    };
  }, [query]);
  return (
    <FormField id="delegate-person" label="Person" hint="Type at least two letters of a name or address.">
      {(props) => (
        <div>
          <input {...props} type="text" value={query} onChange={(e) => setQuery(e.target.value)} className="w-full rounded border border-gray-300 dark:border-gray-700 bg-white dark:bg-gray-900 px-2 py-1 text-sm" autoComplete="off" />
          {found.length > 0 && (
            <ul role="listbox" aria-label="People found" className="mt-1 rounded border border-gray-200 dark:border-gray-700 text-sm">
              {found.map((p) => (
                <li
                  key={p.eppn}
                  role="option"
                  aria-selected={false}
                  tabIndex={0}
                  onClick={() => {
                    onPick(p);
                    setQuery(`${p.name} <${p.eppn}>`);
                    setFound([]);
                  }}
                  onKeyDown={(e) => {
                    if (e.key === "Enter") {
                      onPick(p);
                      setQuery(`${p.name} <${p.eppn}>`);
                      setFound([]);
                    }
                  }}
                  className="cursor-pointer px-2 py-1 hover:bg-gray-100 dark:hover:bg-gray-800"
                >
                  {p.name} <span className="text-gray-500">{p.eppn}</span>
                </li>
              ))}
            </ul>
          )}
        </div>
      )}
    </FormField>
  );
}

/**
 * The `Delegation` section of the settings pane (approvers phase 4). It shows the positions the person holds and lets
 * them name someone to decide for them while away, for dates and with a reason. Active and past delegations are
 * listed, with `Revoke`, and so are the ones the person received.
 */
export function DelegationPanel() {
  const [data, setData] = useState<MyDelegations | null>(null);
  const [error, setError] = useState<unknown>(null);
  const [open, setOpen] = useState(false);
  const [person, setPerson] = useState<{ eppn: string; name: string } | null>(null);
  const [position, setPosition] = useState("");
  const [from, setFrom] = useState("");
  const [until, setUntil] = useState("");
  const [reason, setReason] = useState("");
  const [saving, setSaving] = useState(false);

  const load = useCallback(async () => {
    try {
      setData(await apiClient.listMyDelegations());
      setError(null);
    } catch (e) {
      setError(e);
    }
  }, []);
  useEffect(() => void load(), [load]);

  const ready = person !== null && from !== "" && until !== "" && reason.trim() !== "" && !saving;

  const submit = async () => {
    if (!ready || !person) return;
    setSaving(true);
    try {
      await apiClient.createDelegation({
        delegate: person.eppn,
        position_key: position || null,
        starts_at: new Date(`${from}T00:00:00Z`).toISOString(),
        ends_at: new Date(`${until}T23:59:59Z`).toISOString(),
        reason: reason.trim(),
      });
      setOpen(false);
      setPerson(null);
      setReason("");
      await load();
    } catch (e) {
      setError(e);
    } finally {
      setSaving(false);
    }
  };

  const revoke = async (id: string) => {
    try {
      await apiClient.revokeDelegation(id);
      await load();
    } catch (e) {
      setError(e);
    }
  };

  const input = "w-full rounded border border-gray-300 dark:border-gray-700 bg-white dark:bg-gray-900 px-2 py-1 text-sm";
  return (
    <section className="space-y-3" aria-labelledby="delegation-heading">
      <h3 id="delegation-heading" className="text-xs font-semibold text-slate-900 dark:text-white uppercase tracking-wider">
        Delegation
      </h3>
      {error !== null && <ErrorState error={error} />}
      {data && (
        <p className="text-xs text-slate-600 dark:text-slate-300">
          {data.positions.length === 0 ? "You hold no position, so there is nothing to delegate." : `You hold: ${data.positions.join(", ")}.`}
        </p>
      )}
      {data && data.positions.length > 0 && !open && (
        <button type="button" onClick={() => setOpen(true)} className="px-2 py-1 rounded bg-blue-600 text-white text-xs font-medium">
          Delegate while away
        </button>
      )}
      {open && (
        <div className="space-y-2 rounded border border-gray-200 dark:border-gray-700 p-3">
          <PersonSearch onPick={setPerson} />
          <FormField id="delegate-position" label="Position">
            {(props) => (
              <select {...props} value={position} onChange={(e) => setPosition(e.target.value)} className={input}>
                <option value="">Every position I hold</option>
                {(data?.positions ?? []).map((p) => (
                  <option key={p} value={p}>
                    {p}
                  </option>
                ))}
              </select>
            )}
          </FormField>
          <div className="grid grid-cols-2 gap-2">
            <FormField id="delegate-from" label="From">
              {(props) => <input {...props} type="date" value={from} onChange={(e) => setFrom(e.target.value)} className={input} />}
            </FormField>
            <FormField id="delegate-until" label="Until" hint="At most 365 days.">
              {(props) => <input {...props} type="date" value={until} onChange={(e) => setUntil(e.target.value)} className={input} />}
            </FormField>
          </div>
          <FormField id="delegate-reason" label="Reason">
            {(props) => <input {...props} type="text" value={reason} onChange={(e) => setReason(e.target.value)} className={input} />}
          </FormField>
          <div className="flex gap-2">
            <button type="button" disabled={!ready} onClick={() => void submit()} className="px-2 py-1 rounded bg-blue-600 text-white text-xs font-medium disabled:opacity-50">
              Delegate
            </button>
            <button type="button" onClick={() => setOpen(false)} className="px-2 py-1 rounded border text-xs">
              Cancel
            </button>
          </div>
        </div>
      )}
      {data && data.made.length > 0 && (
        <ul className="space-y-1 text-xs" aria-label="Delegations you made">
          {data.made.map((d) => (
            <li key={d.id} className="flex items-center justify-between gap-2">
              <span>
                {d.delegate_name ?? d.delegate} decides {what(d)}, {day(d.starts_at)} to {day(d.ends_at)}. <span className="text-gray-500">{STATE_WORDS[d.state]}</span>
              </span>
              {(d.state === "active" || d.state === "scheduled") && (
                <button type="button" onClick={() => void revoke(d.id)} aria-label={`Revoke the delegation to ${d.delegate_name ?? d.delegate}`} className="px-2 py-0.5 rounded bg-red-50 text-red-600 dark:bg-red-950/40 dark:text-red-300">
                  Revoke
                </button>
              )}
            </li>
          ))}
        </ul>
      )}
      {data && data.received.length > 0 && (
        <ul className="space-y-1 text-xs" aria-label="Delegations you received">
          {data.received.map((d) => (
            <li key={d.id}>
              You decide for {d.delegator_name ?? d.delegator} {what(d)}, {day(d.starts_at)} to {day(d.ends_at)}. <span className="text-gray-500">{STATE_WORDS[d.state]}</span>
            </li>
          ))}
        </ul>
      )}
    </section>
  );
}

/** On the desk: `You are deciding for Dr. Rivera until 14 Oct.` for each delegation in force that the person received. */
export function DelegationBanner() {
  const [active, setActive] = useState<DelegationRow[]>([]);
  useEffect(() => {
    let alive = true;
    apiClient
      .listMyDelegations()
      .then((d) => alive && setActive(d.received.filter((r) => r.state === "active")))
      .catch(() => {});
    return () => {
      alive = false;
    };
  }, []);
  if (active.length === 0) return null;
  return (
    <div className="space-y-1">
      {active.map((d) => (
        <Banner key={d.id} kind="info">
          {`You are deciding for ${d.delegator_name ?? d.delegator} until ${day(d.ends_at)}.`}
        </Banner>
      ))}
    </div>
  );
}

/** The admin People drawer: the delegations a person made and received, with their state. */
export function PersonDelegations({ eppn }: { eppn: string }) {
  const [rows, setRows] = useState<DelegationRow[] | null>(null);
  useEffect(() => {
    let alive = true;
    Promise.all([apiClient.listDelegations({ delegator: eppn }), apiClient.listDelegations({ delegate: eppn })])
      .then(([made, got]) => alive && setRows([...made, ...got.filter((g) => !made.some((m) => m.id === g.id))]))
      .catch(() => alive && setRows([]));
    return () => {
      alive = false;
    };
  }, [eppn]);
  return (
    <section className="space-y-2">
      <h3 className="text-sm font-semibold text-gray-900 dark:text-gray-100">Delegations</h3>
      {rows === null ? null : rows.length === 0 ? (
        <p className="text-sm text-gray-500">No delegations.</p>
      ) : (
        <ul className="text-sm space-y-1" data-testid="admin-user-delegations">
          {rows.map((d) => (
            <li key={d.id}>
              {d.delegator.toLowerCase() === eppn.toLowerCase() ? `To ${d.delegate}` : `From ${d.delegator}`} {what(d)}, {day(d.starts_at)} to {day(d.ends_at)}.{" "}
              <span className="text-xs text-gray-500">{STATE_WORDS[d.state]}</span>
            </li>
          ))}
        </ul>
      )}
    </section>
  );
}
