import { useCallback, useEffect, useState } from "react";
import { apiClient, type AuthorityChange, type AuthorityImpact, type AuthorityTier, type AuthorityView } from "../api";
import { ConfirmAction, EmptyState, ErrorState, FormField, PageHeader, Skeleton } from "../ui";

const inputClass = "w-full rounded border border-gray-300 dark:border-gray-700 bg-white dark:bg-gray-900 p-2 text-sm";
const buttonClass = "px-3 py-1.5 text-sm font-medium rounded border disabled:opacity-50 disabled:cursor-not-allowed";

type Unit = { id: string; name: string };
type Mode = { kind: "tier"; tier: AuthorityTier["tier"] } | { kind: "signal" };
type Approver = "Position" | "Person" | "Role";

interface Form {
  path: string;
  approverType: Approver;
  approverValue: string;
  check: string;
  signalTier: string;
  reason: string;
}

/** A sentence for who must say yes. */
export function pathSentence(path: string): string {
  if (path === "direct") return "An owner or admin of the workspace decides, and may decide their own change.";
  if (path === "second_person") return "A second owner or admin decides, other than the proposer.";
  return "A named approver decides, other than the proposer.";
}

function approverLabel(spec: Record<string, unknown> | null): string {
  if (!spec) return "an approver";
  const position = spec.Position as { key?: string } | undefined;
  const person = spec.Person as { eppn?: string } | undefined;
  const role = spec.Role as { name?: string } | undefined;
  if (position?.key) return `the ${position.key} position`;
  if (person?.eppn) return person.eppn;
  if (role?.name) return `the ${role.name} role`;
  return "an approver";
}

function specOf(form: Form): Record<string, unknown> {
  const v = form.approverValue.trim();
  if (form.approverType === "Position") return { Position: { key: v, from: "Workspace", walk_up: true } };
  if (form.approverType === "Person") return { Person: { eppn: v } };
  return { Role: { name: v } };
}

function changeOf(mode: Mode, form: Form): Record<string, unknown> {
  if (mode.kind === "signal") return { kind: "signal", check_name: form.check, tier: form.signalTier };
  const row: Record<string, unknown> = { tier: mode.tier, path: form.path };
  if (form.path === "approver") row.approver = specOf(form);
  return { kind: "row", row };
}

function sourceText(t: AuthorityTier): string {
  if (t.source.kind === "built_in") return "Built in";
  if (t.source.kind === "institution") return "The institution";
  return `Set by ${t.source.name ?? "a unit"}`;
}

function counts(c: { direct: number; second_person: number; approver: number }): string {
  return `${c.direct} direct, ${c.second_person} second person, ${c.approver} approver`;
}

// The Decision authority section of the console: who must say yes to a change, and what a change to that would do.
export function Authority() {
  const [units, setUnits] = useState<Unit[]>([]);
  const [unit, setUnit] = useState("");
  const [view, setView] = useState<AuthorityView | null>(null);
  const [changes, setChanges] = useState<AuthorityChange[]>([]);
  const [me, setMe] = useState("");
  const [error, setError] = useState<unknown>(null);
  const [mode, setMode] = useState<Mode | null>(null);
  const [form, setForm] = useState<Form>({ path: "second_person", approverType: "Position", approverValue: "", check: "", signalTier: "High", reason: "" });
  const [effect, setEffect] = useState<AuthorityImpact | null>(null);
  const [shownFor, setShownFor] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [deciding, setDeciding] = useState<{ change: AuthorityChange; decision: "confirm" | "reject" } | null>(null);

  useEffect(() => {
    let live = true;
    apiClient.getCurrentUser().then((u) => live && setMe(String(u.user?.eppn ?? ""))).catch(() => {});
    apiClient
      .accessGrantAnchors()
      .then((a) => {
        if (!live) return;
        const list = a.units.filter((u) => u.hierarchy === "academic").map((u) => ({ id: u.id, name: u.name }));
        setUnits(list);
        setUnit((current) => current || list[0]?.id || "");
      })
      .catch((e) => live && setError(e));
    return () => {
      live = false;
    };
  }, []);

  const load = useCallback(async () => {
    if (!unit) return;
    try {
      const [v, c] = await Promise.all([apiClient.authorityShow(unit), apiClient.authorityChanges(unit)]);
      setView(v);
      setChanges(c.changes);
      setError(null);
    } catch (e) {
      setError(e);
    }
  }, [unit]);
  useEffect(() => {
    void load();
  }, [load]);

  const unitId = view?.organization_id ?? unit;
  const change = mode ? changeOf(mode, form) : null;
  const valuesKey = mode ? JSON.stringify({ mode, change }) : "";
  const shown = shownFor !== null && shownFor === valuesKey;
  const complete = mode !== null && form.reason.trim() !== "" && (mode.kind === "signal" ? form.check !== "" : form.path !== "approver" || form.approverValue.trim() !== "");
  const checks = Array.from(new Set((view?.tiers ?? []).flatMap((t) => t.raised_by.map((r) => r.check))));

  const open = (m: Mode) => {
    setMode(m);
    setEffect(null);
    setShownFor(null);
    setForm((f) => ({ ...f, path: m.kind === "tier" ? (view?.tiers.find((t) => t.tier === m.tier)?.path ?? "second_person") : f.path, check: checks[0] ?? "", reason: "" }));
  };

  const seeEffect = async () => {
    if (!change) return;
    setBusy(true);
    try {
      setEffect(await apiClient.authorityImpact({ organization_id: unitId, change }));
      setShownFor(valuesKey);
      setError(null);
    } catch (e) {
      setError(e);
    } finally {
      setBusy(false);
    }
  };

  const propose = async () => {
    if (!change || !shown) return;
    setBusy(true);
    try {
      await apiClient.authorityPropose({ organization_id: unitId, change, reason: form.reason.trim() });
      setMode(null);
      setEffect(null);
      setShownFor(null);
      await load();
    } catch (e) {
      setError(e);
    } finally {
      setBusy(false);
    }
  };

  const pathOptions = mode?.kind === "tier" && mode.tier !== "Low" ? ["second_person", "approver"] : ["direct", "second_person", "approver"];
  const pending = changes.filter((c) => c.status === "Pending");

  return (
    <div className="space-y-6" data-testid="admin-authority">
      <PageHeader title="Decision authority" description="Who must say yes to a change, and what a change to that rule would do" />
      {error !== null && <ErrorState error={error} onRetry={() => void load()} />}
      <FormField id="authority-unit" label="Unit">
        {(props) => (
          <select {...props} className={inputClass} value={unit} onChange={(e) => { setUnit(e.target.value); setMode(null); }}>
            {units.map((u) => (
              <option key={u.id} value={u.id}>
                {u.name}
              </option>
            ))}
          </select>
        )}
      </FormField>

      {!view ? (
        <Skeleton rows={3} />
      ) : (
        <>
          <section aria-label="Tiers" className="space-y-2">
            {view.tiers.map((t) => (
              <div key={t.tier} data-testid={`authority-tier-${t.tier}`} className="border border-gray-200 dark:border-gray-800 rounded-lg p-3 text-sm grid gap-2 md:grid-cols-4">
                <div className="font-semibold text-gray-900 dark:text-gray-100">{t.tier}</div>
                <div>
                  <div className="text-xs font-medium text-gray-500">What raises it</div>
                  {t.raised_by.length === 0 ? <div className="text-gray-700 dark:text-gray-300">Nothing raised</div> : t.raised_by.map((r) => <div key={r.check} className="text-gray-700 dark:text-gray-300">{r.sentence}</div>)}
                </div>
                <div>
                  <div className="text-xs font-medium text-gray-500">What it needs</div>
                  <div className="text-gray-700 dark:text-gray-300">{pathSentence(t.path)}</div>
                  {t.path === "approver" && (
                    <div className="text-xs text-gray-500">
                      {approverLabel(t.approver)}. Today: {t.resolved.length === 0 ? "no one" : t.resolved.map((r) => r.display_name).join(", ")}
                    </div>
                  )}
                </div>
                <div className="flex items-start justify-between gap-2">
                  <div>
                    <div className="text-xs font-medium text-gray-500">Where it comes from</div>
                    <div className="text-gray-700 dark:text-gray-300">{sourceText(t)}</div>
                  </div>
                  <button type="button" className={`${buttonClass} border-gray-300 dark:border-gray-700`} onClick={() => open({ kind: "tier", tier: t.tier })}>
                    Change
                  </button>
                </div>
              </div>
            ))}
            <button type="button" className="text-xs text-blue-600 hover:underline" onClick={() => open({ kind: "signal" })}>
              Raise what a check counts as
            </button>
          </section>

          {mode && (
            <section aria-label="Change" data-testid="authority-form" className="border border-gray-200 dark:border-gray-800 rounded-lg p-4 space-y-3">
              <h3 className="text-sm font-semibold text-gray-900 dark:text-gray-100">
                {mode.kind === "tier" ? `Change what a ${mode.tier} change needs in ${view.unit_name}` : `Raise a check in ${view.unit_name}`}
              </h3>
              {mode.kind === "tier" ? (
                <>
                  <FormField id="authority-path" label="Path">
                    {(props) => (
                      <select {...props} className={inputClass} value={form.path} onChange={(e) => setForm({ ...form, path: e.target.value })}>
                        {pathOptions.map((p) => (
                          <option key={p} value={p}>
                            {p}
                          </option>
                        ))}
                      </select>
                    )}
                  </FormField>
                  {form.path === "approver" && (
                    <>
                      <FormField id="authority-approver-type" label="Approver type">
                        {(props) => (
                          <select {...props} className={inputClass} value={form.approverType} onChange={(e) => setForm({ ...form, approverType: e.target.value as Approver })}>
                            <option value="Position">Position</option>
                            <option value="Person">Person</option>
                            <option value="Role">Role</option>
                          </select>
                        )}
                      </FormField>
                      <FormField id="authority-approver-value" label={form.approverType === "Position" ? "Position key" : form.approverType === "Person" ? "Person (eppn)" : "Role name"}>
                        {(props) => <input {...props} className={inputClass} value={form.approverValue} onChange={(e) => setForm({ ...form, approverValue: e.target.value })} />}
                      </FormField>
                    </>
                  )}
                </>
              ) : (
                <>
                  <FormField id="authority-check" label="Check">
                    {(props) => (
                      <select {...props} className={inputClass} value={form.check} onChange={(e) => setForm({ ...form, check: e.target.value })}>
                        {checks.map((c) => (
                          <option key={c} value={c}>
                            {c}
                          </option>
                        ))}
                      </select>
                    )}
                  </FormField>
                  <FormField id="authority-signal-tier" label="Counts as">
                    {(props) => (
                      <select {...props} className={inputClass} value={form.signalTier} onChange={(e) => setForm({ ...form, signalTier: e.target.value })}>
                        <option value="Moderate">Moderate</option>
                        <option value="High">High</option>
                      </select>
                    )}
                  </FormField>
                </>
              )}
              <FormField id="authority-reason" label="Reason (recorded)" required>
                {(props) => <textarea {...props} rows={2} className={inputClass} value={form.reason} onChange={(e) => setForm({ ...form, reason: e.target.value })} />}
              </FormField>
              <div className="flex gap-2">
                <button type="button" className={`${buttonClass} border-gray-300 dark:border-gray-700`} disabled={busy || !complete} onClick={() => void seeEffect()}>
                  See the effect
                </button>
                <button type="button" className={`${buttonClass} bg-blue-600 text-white border-blue-600`} disabled={busy || !complete || !shown} onClick={() => void propose()}>
                  Propose change
                </button>
                <button type="button" className="text-xs text-gray-500 hover:underline" onClick={() => setMode(null)}>
                  Cancel
                </button>
              </div>
              {effect && (
                <div data-testid="authority-effect" className="text-sm space-y-2 border-t border-gray-200 dark:border-gray-800 pt-3">
                  <div className="font-medium text-gray-900 dark:text-gray-100">
                    {effect.proposals} proposals in the last {effect.window_days} days
                  </div>
                  <div className="text-gray-700 dark:text-gray-300">Now: {counts(effect.before)}</div>
                  <div className="text-gray-700 dark:text-gray-300">With this change: {counts(effect.after)}</div>
                  {effect.load.length > 0 && (
                    <ul className="text-gray-700 dark:text-gray-300">
                      {effect.load.map((l) => (
                        <li key={l.eppn}>
                          {l.display_name}: {l.before} now, {l.after} with this change
                        </li>
                      ))}
                    </ul>
                  )}
                  {effect.unassigned.length > 0 && (
                    <div>
                      <div className="text-amber-700 dark:text-amber-300">
                        {effect.unassigned.length} proposals would have no one to decide:
                      </div>
                      <ul className="text-gray-700 dark:text-gray-300">
                        {effect.unassigned.map((u) => (
                          <li key={u.proposal_id}>{u.summary}</li>
                        ))}
                      </ul>
                    </div>
                  )}
                </div>
              )}
            </section>
          )}

          <section aria-label="Rows" className="space-y-2">
            <h3 className="text-sm font-semibold text-gray-800 dark:text-gray-200">Rows that apply here</h3>
            {view.rows.length === 0 ? (
              <EmptyState title="No rows" description="Only the built-in answer applies." />
            ) : (
              view.rows.map((r) => (
                <div key={`${r.organization_id}-${r.tier}`} data-testid="authority-row" className="text-sm border border-gray-200 dark:border-gray-800 rounded-lg p-3">
                  <div className="text-gray-900 dark:text-gray-100">
                    {r.unit_name}: a {r.tier} change needs {r.path === "approver" ? approverLabel(r.approver) : r.path.replace("_", " ")}.
                  </div>
                  {!r.has_effect && <div className="text-xs text-gray-500">The institution already requires more.</div>}
                </div>
              ))
            )}
          </section>

          <section aria-label="Pending changes" className="space-y-2">
            <h3 className="text-sm font-semibold text-gray-800 dark:text-gray-200">Pending changes</h3>
            {pending.length === 0 ? (
              <EmptyState title="Nothing is waiting" description="A change takes effect when a different person confirms it." />
            ) : (
              pending.map((c) => (
                <div key={c.id} data-testid={`authority-change-${c.id}`} className="text-sm border border-gray-200 dark:border-gray-800 rounded-lg p-3 flex items-center justify-between gap-4">
                  <div>
                    <div className="text-gray-900 dark:text-gray-100">
                      {c.unit_name ?? "A unit"}: {JSON.stringify(c.change)}
                    </div>
                    <div className="text-xs text-gray-500">
                      {c.reason}. Proposed by {c.proposed_by}.
                    </div>
                  </div>
                  {c.proposed_by.toLowerCase() === me.toLowerCase() ? (
                    <div className="text-xs text-gray-500">Waiting for a different person to confirm.</div>
                  ) : (
                    <div className="flex gap-2">
                      <button type="button" className={`${buttonClass} border-gray-300 dark:border-gray-700`} onClick={() => setDeciding({ change: c, decision: "confirm" })}>
                        Confirm
                      </button>
                      <button type="button" className={`${buttonClass} border-gray-300 dark:border-gray-700`} onClick={() => setDeciding({ change: c, decision: "reject" })}>
                        Reject
                      </button>
                    </div>
                  )}
                </div>
              ))
            )}
            {deciding && (
              <div className="border border-gray-200 dark:border-gray-800 rounded-lg p-4">
                <ConfirmAction
                  verb={deciding.decision === "confirm" ? "Confirm" : "Reject"}
                  target="the change"
                  consequence={deciding.decision === "confirm" ? "The rule takes effect for decisions made from now on. A proposal already approved is not reopened." : "The change is dropped and the matrix stays as it is."}
                  onConfirm={async (note) => {
                    await apiClient.authorityDecide(deciding.change.id, deciding.decision, note);
                  }}
                  onSuccess={() => {
                    setDeciding(null);
                    void load();
                  }}
                />
              </div>
            )}
          </section>
        </>
      )}
    </div>
  );
}
