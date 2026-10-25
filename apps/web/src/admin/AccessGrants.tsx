import { useCallback, useEffect, useMemo, useState, type FormEvent } from "react";
import { apiClient, type AccessChange, type AccessGrantRow, type AccessImpact, type NewAccessGrant } from "../api";
import { ConfirmAction, DataTable, Drawer, ErrorState, FormField, StatusBadge, type DataTableColumn } from "../ui";
import { grantSentence } from "./accessSentences";
import type { AccessViewer } from "./Access";

const inputClass =
  "w-full text-sm rounded border border-gray-300 dark:border-gray-700 p-2 bg-white dark:bg-gray-800 text-gray-900 dark:text-gray-100";
const buttonClass = "px-2 py-1 text-xs font-medium rounded border border-gray-300 dark:border-gray-700 hover:bg-gray-50 dark:hover:bg-gray-800";

const ROLES = ["viewer", "commenter", "editor", "admin", "owner"];
const SOURCES = ["request", "appointment", "membership", "rule", "task", "flow"];
const CLEARANCES = ["", "Public", "Directory", "Internal", "Restricted"];

export function AccessGrants({ viewer }: { viewer: AccessViewer }) {
  const [state, setState] = useState("active");
  const [creating, setCreating] = useState(false);
  const [revoking, setRevoking] = useState<AccessGrantRow | null>(null);
  const [version, setVersion] = useState(0);
  const [pending, setPending] = useState<AccessChange[]>([]);
  const [deciding, setDeciding] = useState<AccessChange | null>(null);

  const loadPending = useCallback(async () => {
    try {
      const page = await apiClient.accessGrants({ state: "pending" });
      setPending(page.changes.filter((c) => c.kind === "grant"));
    } catch {
      setPending([]);
    }
  }, []);
  useEffect(() => {
    void loadPending();
  }, [loadPending, version]);

  const load = useCallback(
    async (cursor?: string, search?: string) => {
      const page = await apiClient.accessGrants({ state, cursor, holder: search });
      return { rows: page.grants, next_cursor: page.next_cursor ?? undefined };
    },
    [state]
  );

  const columns: DataTableColumn<AccessGrantRow>[] = [
    { id: "holder_id", header: "Holder", cell: ({ row }) => (row.original.holder_kind === "person" ? row.original.holder_id : `${row.original.holder_kind}: ${row.original.holder_id}`) },
    { id: "role", header: "Role", accessorKey: "role" },
    { id: "anchor", header: "Where", cell: ({ row }) => row.original.anchor_name ?? row.original.anchor_id ?? "the holder's own unit" },
    { id: "source", header: "Source", accessorKey: "source" },
    { id: "valid_until", header: "Until", cell: ({ row }) => row.original.valid_until ?? "no end date" },
    {
      id: "state",
      header: "State",
      cell: ({ row }) => (row.original.revoked_at ? <StatusBadge tone="neutral" text="Revoked" /> : <StatusBadge tone="ok" text="Active" />),
    },
    {
      id: "actions",
      header: "",
      cell: ({ row }) =>
        row.original.revoked_at ? null : (
          <button type="button" className={buttonClass} onClick={() => setRevoking(row.original)}>
            Revoke
          </button>
        ),
    },
  ];

  return (
    <div className="space-y-6" data-testid="access-grants">
      <div className="flex items-center justify-between gap-4">
        <p className="text-sm text-gray-600 dark:text-gray-400">
          {viewer.platform ? "All grants." : "You see the grants anchored in the units you administer."}
        </p>
        <div className="flex items-center gap-3">
          <label className="text-sm text-gray-700 dark:text-gray-300 flex items-center gap-2">
            Show
            <select className={inputClass} value={state} onChange={(e) => setState(e.target.value)}>
              <option value="active">Active</option>
              <option value="revoked">Revoked</option>
              <option value="expired">Expired</option>
              <option value="all">All</option>
            </select>
          </label>
          <button type="button" onClick={() => setCreating(true)} className="px-3 py-1.5 text-sm font-medium rounded-md bg-blue-600 text-white hover:bg-blue-700">
            New grant
          </button>
        </div>
      </div>

      {pending.length > 0 && (
        <section aria-label="Waiting for confirmation" className="space-y-2">
          <h3 className="text-sm font-semibold text-gray-800 dark:text-gray-200">Waiting for confirmation</h3>
          {pending.map((c) => (
            <div key={c.change_id} className="border border-amber-200 dark:border-amber-900/60 rounded-lg p-3 flex items-center justify-between gap-4 text-sm">
              <div>
                <div className="text-gray-900 dark:text-gray-100">
                  {String(c.grant?.holder_id ?? "")} as {String(c.grant?.role ?? "")}
                </div>
                <div className="text-xs text-gray-500">Proposed by {c.proposed_by}. {c.reason}</div>
              </div>
              {c.proposed_by.toLowerCase() !== viewer.eppn.toLowerCase() && (
                <button type="button" className={buttonClass} onClick={() => setDeciding(c)}>
                  Confirm or reject
                </button>
              )}
            </div>
          ))}
        </section>
      )}

      <DataTable<AccessGrantRow> key={`${state}-${version}`} columns={columns} load={load} rowKey={(r) => String(r.id)} />

      <Drawer open={creating} onClose={() => setCreating(false)} title="New grant">
        <GrantForm
          onProposed={() => {
            setVersion((v) => v + 1);
          }}
        />
      </Drawer>
      <Drawer open={revoking !== null} onClose={() => setRevoking(null)} title="Revoke a grant">
        {revoking && (
          <ConfirmAction
            verb="Revoke"
            target="grant"
            consequence={`${revoking.holder_id} loses the ${revoking.role} access this grant gives. It applies at once.`}
            onConfirm={async (reason) => {
              await apiClient.accessRevokeGrant(revoking.id, reason);
            }}
            onSuccess={() => {
              setRevoking(null);
              setVersion((v) => v + 1);
            }}
          />
        )}
      </Drawer>
      <Drawer open={deciding !== null} onClose={() => setDeciding(null)} title="Decide on a grant">
        {deciding && <DecideGrant change={deciding} onDone={() => { setDeciding(null); setVersion((v) => v + 1); }} />}
      </Drawer>
    </div>
  );
}

function DecideGrant({ change, onDone }: { change: AccessChange; onDone: () => void }) {
  const [decision, setDecision] = useState<"confirm" | "reject">("confirm");
  const [impact, setImpact] = useState<AccessImpact | null>(null);
  const [error, setError] = useState<unknown>(null);
  useEffect(() => {
    let live = true;
    apiClient
      .accessChangeImpact(change.change_id)
      .then((i) => live && setImpact(i))
      .catch((e) => live && setError(e));
    return () => {
      live = false;
    };
  }, [change.change_id]);
  return (
    <div className="space-y-4">
      <p className="text-sm text-gray-700 dark:text-gray-300">
        {change.proposed_by} proposed giving {String(change.grant?.holder_id ?? "")} the {String(change.grant?.role ?? "")} role. {change.reason}
      </p>
      {error !== null && <ErrorState error={error} />}
      {impact && <ImpactSummary impact={impact} />}
      <fieldset className="flex gap-4 text-sm">
        <legend className="sr-only">Decision</legend>
        <label className="flex items-center gap-1.5">
          <input type="radio" name="grant-decision" checked={decision === "confirm"} onChange={() => setDecision("confirm")} />
          Confirm
        </label>
        <label className="flex items-center gap-1.5">
          <input type="radio" name="grant-decision" checked={decision === "reject"} onChange={() => setDecision("reject")} />
          Reject
        </label>
      </fieldset>
      <ConfirmAction
        key={decision}
        verb={decision === "confirm" ? "Confirm" : "Reject"}
        target="grant"
        consequence={decision === "confirm" ? "The grant takes effect now." : "The grant is not made."}
        onConfirm={async (reason) => {
          await apiClient.accessDecideChange(change.change_id, decision, reason);
        }}
        onSuccess={onDone}
      />
    </div>
  );
}

function plural(n: number, one: string, many: string): string {
  return n === 1 ? one : many;
}

function ImpactSummary({ impact }: { impact: AccessImpact }) {
  const gain = new Set(impact.gain.map((g) => g.person)).size;
  const lose = new Set(impact.lose.map((g) => g.person)).size;
  return (
    <div data-testid="grant-impact" className="text-sm rounded border border-gray-200 dark:border-gray-800 p-3 space-y-1">
      <div>{gain} {plural(gain, "person gains", "people gain")} access.</div>
      {lose > 0 && <div>{lose} {plural(lose, "person loses", "people lose")} access.</div>}
      <div className="text-xs text-gray-500">
        {impact.people_considered} {plural(impact.people_considered, "person", "people")} considered{impact.truncated ? ", and more than that hold it" : ""}.
      </div>
    </div>
  );
}

interface Anchors {
  units: { id: string; name: string; hierarchy: string }[];
  domains: { key: string; name: string }[];
}

function GrantForm({ onProposed }: { onProposed: () => void }) {
  const [anchors, setAnchors] = useState<Anchors>({ units: [], domains: [] });
  const [holderKind, setHolderKind] = useState("person");
  const [holder, setHolder] = useState("");
  const [role, setRole] = useState("viewer");
  const [anchorKind, setAnchorKind] = useState("unit");
  const [unit, setUnit] = useState("");
  const [container, setContainer] = useState("");
  const [depth, setDepth] = useState("");
  const [domain, setDomain] = useState("");
  const [clearance, setClearance] = useState("");
  const [source, setSource] = useState("request");
  const [until, setUntil] = useState("");
  const [reason, setReason] = useState("");
  const [shown, setShown] = useState<{ key: string; impact: AccessImpact } | null>(null);
  const [error, setError] = useState<unknown>(null);
  const [busy, setBusy] = useState(false);
  const [done, setDone] = useState<string | null>(null);

  useEffect(() => {
    let live = true;
    apiClient.accessGrantAnchors().then((a) => live && setAnchors(a)).catch(() => undefined);
    return () => {
      live = false;
    };
  }, []);

  const chosenUnit = anchors.units.find((u) => u.id === unit);
  const body: NewAccessGrant = useMemo(() => {
    const b: NewAccessGrant = { holder_kind: holderKind, holder_id: holder.trim(), role, anchor_kind: anchorKind, source, reason: reason.trim() };
    if (anchorKind === "unit") {
      b.anchor_id = unit;
      if (chosenUnit) b.hierarchy = chosenUnit.hierarchy;
    } else if (anchorKind === "container") {
      b.anchor_id = container.trim();
    }
    if (depth !== "") b.depth = Number(depth);
    if (domain) b.domain = domain;
    if (clearance) b.clearance = clearance;
    if (until) b.valid_until = `${until}T23:59:59Z`;
    return b;
  }, [holderKind, holder, role, anchorKind, unit, chosenUnit, container, depth, domain, clearance, source, until, reason]);
  const key = JSON.stringify(body);
  const complete = body.holder_id !== "" && body.reason !== "" && (anchorKind === "holder_unit" || (body.anchor_id ?? "") !== "");
  // What is shown belongs to the values it was worked out for. A changed value takes it away.
  const previewed = shown !== null && shown.key === key;

  const holderWords = holderKind === "position" ? `Every ${holder.split("@")[0] || "holder of the position"}` : holder.trim();
  const anchorWords = anchorKind === "unit" ? chosenUnit?.name ?? "the chosen unit" : anchorKind === "container" ? container.trim() || "the chosen workspace" : "their own unit";
  const sentence = grantSentence({
    holder: holderWords,
    role,
    anchor: anchorWords,
    anchorKind,
    depth: depth === "" ? null : Number(depth),
    domain: anchors.domains.find((d) => d.key === domain)?.name ?? (domain || null),
    clearance: clearance || null,
  });

  const show = async () => {
    setError(null);
    setBusy(true);
    try {
      const res = await apiClient.accessPreviewGrant(body);
      setShown({ key, impact: res.impact });
    } catch (e) {
      setShown(null);
      setError(e);
    } finally {
      setBusy(false);
    }
  };

  const propose = async (e: FormEvent) => {
    e.preventDefault();
    if (!previewed || busy) return;
    setBusy(true);
    setError(null);
    try {
      const res = await apiClient.accessProposeGrant(body);
      setDone(res.message);
      onProposed();
    } catch (err) {
      setError(err);
    } finally {
      setBusy(false);
    }
  };

  if (done) return <p className="text-sm text-gray-700 dark:text-gray-300">{done}</p>;

  return (
    <form data-testid="grant-form" onSubmit={propose} className="space-y-4">
      {error !== null && (
        <p role="alert" className="text-sm font-medium text-red-600 dark:text-red-400">
          {error instanceof Error ? error.message : "The server refused this grant."}
        </p>
      )}
      <FormField id="grant-holder-kind" label="Holder kind">
        {(props) => (
          <select {...props} className={inputClass} value={holderKind} onChange={(e) => setHolderKind(e.target.value)}>
            <option value="person">A person</option>
            <option value="position">A position</option>
            <option value="unit">Everyone in a unit</option>
            <option value="group">A group</option>
          </select>
        )}
      </FormField>
      <FormField id="grant-holder" label={holderKind === "person" ? "Person" : holderKind === "position" ? "Position (key@unit or key@*)" : holderKind === "unit" ? "Holder unit id" : "Group id"} required>
        {(props) => <input {...props} className={inputClass} value={holder} onChange={(e) => setHolder(e.target.value)} />}
      </FormField>
      <FormField id="grant-role" label="Role">
        {(props) => (
          <select {...props} className={inputClass} value={role} onChange={(e) => setRole(e.target.value)}>
            {ROLES.map((r) => (
              <option key={r} value={r}>
                {r}
              </option>
            ))}
          </select>
        )}
      </FormField>
      <FormField id="grant-anchor-kind" label="Anchored at">
        {(props) => (
          <select {...props} className={inputClass} value={anchorKind} onChange={(e) => setAnchorKind(e.target.value)}>
            <option value="unit">A unit</option>
            <option value="container">A workspace, app or table</option>
            {holderKind === "position" && <option value="holder_unit">The unit where the position is held</option>}
          </select>
        )}
      </FormField>
      {anchorKind === "unit" && (
        <FormField id="grant-unit" label="Unit" required>
          {(props) => (
            <select {...props} className={inputClass} value={unit} onChange={(e) => setUnit(e.target.value)}>
              <option value="">Choose a unit</option>
              {anchors.units.map((u) => (
                <option key={u.id} value={u.id}>
                  {u.name}
                </option>
              ))}
            </select>
          )}
        </FormField>
      )}
      {anchorKind === "container" && (
        <FormField id="grant-container" label="Container" hint="workspace:<id>, app:<slug> or table:<slug>/<table id>" required>
          {(props) => <input {...props} className={inputClass} value={container} onChange={(e) => setContainer(e.target.value)} />}
        </FormField>
      )}
      {anchorKind !== "container" && (
        <FormField id="grant-depth" label="How far down" hint="Empty is every level below. 0 is the unit itself.">
          {(props) => <input {...props} type="number" min={0} className={inputClass} value={depth} onChange={(e) => setDepth(e.target.value)} />}
        </FormField>
      )}
      <FormField id="grant-domain" label="Limited to a domain">
        {(props) => (
          <select {...props} className={inputClass} value={domain} onChange={(e) => setDomain(e.target.value)}>
            <option value="">Any domain</option>
            {anchors.domains.map((d) => (
              <option key={d.key} value={d.key}>
                {d.name}
              </option>
            ))}
          </select>
        )}
      </FormField>
      <FormField id="grant-clearance" label="Up to sensitivity">
        {(props) => (
          <select {...props} className={inputClass} value={clearance} onChange={(e) => setClearance(e.target.value)}>
            {CLEARANCES.map((c) => (
              <option key={c} value={c}>
                {c || "The role's own limit"}
              </option>
            ))}
          </select>
        )}
      </FormField>
      <FormField id="grant-source" label="Source">
        {(props) => (
          <select {...props} className={inputClass} value={source} onChange={(e) => setSource(e.target.value)}>
            {SOURCES.map((s) => (
              <option key={s} value={s}>
                {s}
              </option>
            ))}
          </select>
        )}
      </FormField>
      <FormField id="grant-until" label="Valid until" hint={source === "request" ? "A requested grant needs an end date" : "Empty is no end date"}>
        {(props) => <input {...props} type="date" className={inputClass} value={until} onChange={(e) => setUntil(e.target.value)} />}
      </FormField>
      <FormField id="grant-reason" label="Reason" required>
        {(props) => <input {...props} className={inputClass} value={reason} onChange={(e) => setReason(e.target.value)} />}
      </FormField>

      {previewed && shown && (
        <div className="space-y-2">
          <p data-testid="grant-sentence" className="text-sm font-medium text-gray-900 dark:text-gray-100">
            {sentence}
          </p>
          <ImpactSummary impact={shown.impact} />
        </div>
      )}

      <div className="flex justify-end gap-3">
        <button type="button" disabled={!complete || busy} onClick={() => void show()} className="px-4 py-2 text-sm font-medium rounded-md border border-gray-300 dark:border-gray-700 hover:bg-gray-50 dark:hover:bg-gray-800 disabled:opacity-50">
          Show what this does
        </button>
        <button type="submit" disabled={!previewed || busy} className="px-4 py-2 text-sm font-medium rounded-md bg-blue-600 text-white hover:bg-blue-700 disabled:opacity-50">
          Propose
        </button>
      </div>
    </form>
  );
}
