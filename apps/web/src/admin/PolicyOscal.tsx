import { useMemo, useState } from "react";
import {
  apiClient,
  ApiError,
  type OscalRequirement,
  type PolicyCaseResult,
  type PolicyCaseRow,
  type PolicyProblem,
  type PolicySimulation,
  type PolicyVersionRow,
} from "../api";
import { ConfirmAction, ControlLabel, EmptyState, ErrorState, PageHeader, Skeleton, StatusBadge, Tabs, useAsync } from "../ui";
import { downloadJson } from "./download";

const buttonClass = "px-3 py-1.5 text-sm font-medium rounded border border-gray-300 dark:border-gray-700 disabled:opacity-50";
const inputClass = "rounded border border-gray-300 dark:border-gray-700 bg-white dark:bg-gray-800 px-2 py-1.5 text-sm";
const monoClass = `${inputClass} w-full font-mono text-xs`;
const STATUSES = ["implemented", "partial", "planned", "alternative", "not-applicable"];
const TABS = [
  { id: "active", label: "Active policy" },
  { id: "versions", label: "Versions" },
  { id: "cases", label: "Test cases" },
  { id: "simulator", label: "Simulator" },
  { id: "oscal", label: "OSCAL requirements" },
];

/** Each problem the server found in a source, one per line, with its place. */
function problemsOf(error: unknown): string[] {
  if (error instanceof ApiError && error.fields) return Object.values(error.fields);
  return [];
}

function Problems({ lines }: { lines: string[] }) {
  if (lines.length === 0) return null;
  return (
    <ul role="alert" className="list-disc pl-5 text-sm text-red-700 dark:text-red-300" aria-label="Problems in the policy">
      {lines.map((l) => (
        <li key={l}>{l}</li>
      ))}
    </ul>
  );
}

const problemLine = (p: PolicyProblem) => `line ${p.line}, column ${p.column}: ${p.message}`;

function statusTone(status: string): "ok" | "warn" | "neutral" {
  return status === "Active" ? "ok" : status === "Draft" ? "warn" : "neutral";
}

/** A name for each control that a policy or a requirement mentions. */
function useTitles() {
  const rows = useAsync(() => apiClient.adminOscalRequirements());
  const titles = useMemo(() => Object.fromEntries((rows.data ?? []).filter((r) => r.title).map((r) => [r.control_id, r.title as string])), [rows.data]);
  return titles;
}

function ActiveTab() {
  const active = useAsync(() => apiClient.adminPolicyActive());
  const titles = useTitles();
  if (active.error) return <ErrorState error={active.error} onRetry={() => void active.reload()} />;
  if (!active.data) return <Skeleton rows={4} />;
  const { version, source, policies } = active.data;
  return (
    <div className="space-y-4">
      <p className="text-sm text-gray-600 dark:text-gray-400">
        Version {version.version}, activated {version.activated_at ?? "at first start"} by {version.activated_by ?? "the system"}. To change it, write a new draft in the Versions tab.
      </p>
      <label htmlFor="policy-source" className="block text-xs text-gray-500">Policy source</label>
      <textarea id="policy-source" readOnly rows={12} value={source} className={monoClass} />
      <div className="overflow-x-auto">
        <table className="w-full text-sm" aria-label="Policies">
          <thead>
            <tr className="text-left text-xs text-gray-500">
              <th className="pr-3">Policy</th>
              <th className="pr-3">Effect</th>
              <th className="pr-3">Control</th>
              <th className="pr-3">Statute</th>
              <th>What it says</th>
            </tr>
          </thead>
          <tbody>
            {policies.map((p) => (
              <tr key={p.id} className="border-t border-gray-200 dark:border-gray-800 align-top">
                <td className="pr-3 py-1 font-mono text-xs">{p.key || p.id}</td>
                <td className="pr-3"><StatusBadge tone={p.effect === "permit" ? "ok" : "bad"} text={p.effect} /></td>
                <td className="pr-3">{p.oscal ? <ControlLabel id={p.oscal} lookup={titles} /> : "None"}</td>
                <td className="pr-3">{p.source || "None"}</td>
                <td>{p.description}</td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
    </div>
  );
}

function CaseResults({ results, failing }: { results: PolicyCaseResult[]; failing: PolicyCaseResult[] }) {
  return (
    <div className="space-y-2">
      {failing.length > 0 ? (
        <div role="alert" className="rounded border border-red-300 p-3 text-sm text-red-800 dark:text-red-200">
          <p>{failing.length === 1 ? "1 test case fails." : `${failing.length} test cases fail.`} The draft cannot be activated.</p>
          <ul className="list-disc pl-5">
            {failing.map((f) => (
              <li key={f.name}>
                {f.name}: expected {f.expected}, got {f.actual}
                {f.error ? ` (${f.error})` : ""}
              </li>
            ))}
          </ul>
        </div>
      ) : (
        <p className="text-sm text-emerald-700 dark:text-emerald-300">All {results.length} test cases pass.</p>
      )}
    </div>
  );
}

function DraftPanel({ row, onChange }: { row: PolicyVersionRow; onChange: () => void }) {
  const [problems, setProblems] = useState<string[] | null>(null);
  const [run, setRun] = useState<PolicyCaseResult[] | null>(null);
  const [error, setError] = useState<unknown>(null);
  const [activating, setActivating] = useState(false);
  const failing = (run ?? []).filter((r) => !r.pass);

  const validate = async () => {
    setError(null);
    try {
      const r = await apiClient.validatePolicy(row.source ?? "");
      setProblems(r.ok ? [] : r.problems.map(problemLine));
    } catch (e) {
      setError(e);
    }
  };
  const runCases = async () => {
    setError(null);
    try {
      setRun((await apiClient.runPolicyCases(row.id)).results);
    } catch (e) {
      setError(e);
    }
  };

  return (
    <div className="space-y-3 rounded border border-gray-200 dark:border-gray-800 p-3">
      <h3 className="text-sm font-semibold">Version {row.version} is a draft</h3>
      <textarea readOnly aria-label={`Source of version ${row.version}`} rows={8} value={row.source ?? ""} className={monoClass} />
      <div className="flex flex-wrap gap-2">
        <button type="button" className={buttonClass} onClick={() => void validate()}>Validate</button>
        <button type="button" className={buttonClass} onClick={() => void runCases()}>Run cases</button>
        <button type="button" className={buttonClass} disabled={run === null || failing.length > 0} onClick={() => setActivating(true)}>
          Activate
        </button>
      </div>
      {error !== null && <ErrorState error={error} />}
      {problems !== null && (problems.length === 0 ? <p className="text-sm text-emerald-700 dark:text-emerald-300">The policy parses and type-checks.</p> : <Problems lines={problems} />)}
      {run !== null && <CaseResults results={run} failing={failing} />}
      {run === null && <p className="text-xs text-gray-500">Run the test cases first. A draft is activated only when every case passes, and by a different Platform Admin from its author.</p>}
      {activating && (
        <ConfirmAction
          verb="Activate"
          target={`version ${row.version}`}
          consequence="Every server runs this policy from the next request. The version now running is retired."
          onConfirm={(reason) => apiClient.activatePolicyVersion(row.id, reason)}
          onSuccess={() => {
            setActivating(false);
            onChange();
          }}
        />
      )}
    </div>
  );
}

function VersionsTab() {
  const versions = useAsync(() => apiClient.adminPolicyVersions());
  const [openId, setOpenId] = useState<string | null>(null);
  const [open, setOpen] = useState<PolicyVersionRow | null>(null);
  const [editing, setEditing] = useState(false);
  const [source, setSource] = useState("");
  const [note, setNote] = useState("");
  const [problems, setProblems] = useState<string[] | null>(null);
  const [error, setError] = useState<unknown>(null);

  const show = async (id: string) => {
    setError(null);
    try {
      setOpen(await apiClient.adminPolicyVersion(id));
      setOpenId(id);
    } catch (e) {
      setError(e);
    }
  };
  const validate = async () => {
    setError(null);
    try {
      const r = await apiClient.validatePolicy(source);
      setProblems(r.ok ? [] : r.problems.map(problemLine));
    } catch (e) {
      setError(e);
    }
  };
  const save = async () => {
    setError(null);
    try {
      const made = await apiClient.createPolicyDraft(source, note);
      setEditing(false);
      setSource("");
      setNote("");
      setProblems(null);
      await versions.reload();
      await show(made.id);
    } catch (e) {
      setProblems(problemsOf(e));
      if (problemsOf(e).length === 0) setError(e);
    }
  };
  const clone = async (id: string) => {
    setError(null);
    try {
      const made = await apiClient.clonePolicyVersion(id);
      await versions.reload();
      await show(made.id);
    } catch (e) {
      setError(e);
    }
  };

  if (versions.error) return <ErrorState error={versions.error} onRetry={() => void versions.reload()} />;
  if (!versions.data) return <Skeleton rows={4} />;
  return (
    <div className="space-y-4">
      <div>
        <button type="button" className={buttonClass} onClick={() => setEditing(true)}>New draft</button>
      </div>
      {error !== null && <ErrorState error={error} />}
      {editing && (
        <div className="space-y-2 rounded border border-gray-200 dark:border-gray-800 p-3">
          <label htmlFor="draft-source" className="block text-xs text-gray-500">Policy source</label>
          <textarea id="draft-source" rows={12} value={source} onChange={(e) => setSource(e.target.value)} className={monoClass} spellCheck={false} />
          <label htmlFor="draft-note" className="block text-xs text-gray-500">Note</label>
          <input id="draft-note" value={note} onChange={(e) => setNote(e.target.value)} className={`${inputClass} w-full`} />
          {problems !== null && (problems.length === 0 ? <p className="text-sm text-emerald-700 dark:text-emerald-300">The policy parses and type-checks.</p> : <Problems lines={problems} />)}
          <div className="flex gap-2">
            <button type="button" className={buttonClass} disabled={!source.trim()} onClick={() => void validate()}>Validate</button>
            <button type="button" className={buttonClass} disabled={!source.trim()} onClick={() => void save()}>Save draft</button>
            <button type="button" className={buttonClass} onClick={() => { setEditing(false); setProblems(null); }}>Cancel</button>
          </div>
        </div>
      )}
      <div className="overflow-x-auto">
        <table className="w-full text-sm" aria-label="Policy versions">
          <thead>
            <tr className="text-left text-xs text-gray-500">
              <th className="pr-3">Version</th>
              <th className="pr-3">Status</th>
              <th className="pr-3">Note</th>
              <th className="pr-3">Written by</th>
              <th className="pr-3">Activated by</th>
              <th />
            </tr>
          </thead>
          <tbody>
            {versions.data.map((v) => (
              <tr key={v.id} className="border-t border-gray-200 dark:border-gray-800">
                <td className="pr-3 py-1">{v.version}</td>
                <td className="pr-3"><StatusBadge tone={statusTone(v.status)} text={v.status} /></td>
                <td className="pr-3">{v.note || "None"}</td>
                <td className="pr-3">{v.created_by} on {v.created_at}</td>
                <td className="pr-3">{v.activated_by ? `${v.activated_by} on ${v.activated_at}` : "Not activated"}</td>
                <td className="space-x-2">
                  <button type="button" className={buttonClass} aria-label={`Open version ${v.version}`} onClick={() => void show(v.id)}>Open</button>
                  <button type="button" className={buttonClass} aria-label={`Clone version ${v.version}`} onClick={() => void clone(v.id)}>Clone</button>
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
      {open && open.id === openId && open.status === "Draft" && <DraftPanel key={open.id} row={open} onChange={() => { void versions.reload(); void show(open.id); }} />}
      {open && open.id === openId && open.status !== "Draft" && (
        <div className="space-y-2">
          <h3 className="text-sm font-semibold">Version {open.version} ({open.status}). Clone it to make a draft, then activate that. This is how you roll back.</h3>
          <textarea readOnly aria-label={`Source of version ${open.version}`} rows={8} value={open.source ?? ""} className={monoClass} />
        </div>
      )}
    </div>
  );
}

const SAMPLE_USER = { type: "User", id: "someone@state.edu", attrs: { eppn: "someone@state.edu", name: "Someone", scoped_affiliation: "faculty", department: "biology", units: [], unit_ids: [], is_platform_admin: false } };
const SAMPLE_RESOURCE = { type: "Record", id: "app-record-target", attrs: { app_slug: "app", department: "biology", workspace_id: "", unit: { type: "Unit", id: "biology" }, is_ferpa_sensitive: false, categories: [] } };
const pretty = (v: unknown) => JSON.stringify(v, null, 2);

/** Parses a JSON field. A mistake is a sentence, not an exception. */
function parseJson(label: string, text: string): { value?: unknown; error?: string } {
  try {
    return { value: JSON.parse(text) };
  } catch (e) {
    return { error: `${label} is not valid JSON: ${e instanceof Error ? e.message : "unreadable"}` };
  }
}

function CasesTab() {
  const cases = useAsync(() => apiClient.adminPolicyCases());
  const [adding, setAdding] = useState(false);
  const [removing, setRemoving] = useState<PolicyCaseRow | null>(null);
  const [name, setName] = useState("");
  const [action, setAction] = useState("read");
  const [expected, setExpected] = useState("Allow");
  const [principal, setPrincipal] = useState(pretty(SAMPLE_USER));
  const [resource, setResource] = useState(pretty(SAMPLE_RESOURCE));
  const [formError, setFormError] = useState<string | null>(null);

  if (cases.error) return <ErrorState error={cases.error} onRetry={() => void cases.reload()} />;
  if (!cases.data) return <Skeleton rows={4} />;

  const ready = () => {
    const p = parseJson("Principal", principal);
    const r = parseJson("Resource", resource);
    if (p.error || r.error) {
      setFormError(p.error ?? r.error ?? null);
      return null;
    }
    setFormError(null);
    return { name, action, expected, principal: p.value, resource: r.value };
  };

  return (
    <div className="space-y-4">
      <div>
        <button type="button" className={buttonClass} onClick={() => setAdding(true)}>Add a case</button>
      </div>
      {adding && (
        <div className="space-y-2 rounded border border-gray-200 dark:border-gray-800 p-3">
          <div className="flex flex-wrap gap-3">
            <div>
              <label htmlFor="case-name" className="block text-xs text-gray-500">Name</label>
              <input id="case-name" value={name} onChange={(e) => setName(e.target.value)} className={`${inputClass} w-72`} />
            </div>
            <div>
              <label htmlFor="case-action" className="block text-xs text-gray-500">Action</label>
              <input id="case-action" value={action} onChange={(e) => setAction(e.target.value)} className={inputClass} />
            </div>
            <div>
              <label htmlFor="case-expected" className="block text-xs text-gray-500">Expected answer</label>
              <select id="case-expected" value={expected} onChange={(e) => setExpected(e.target.value)} className={inputClass}>
                <option>Allow</option>
                <option>Deny</option>
              </select>
            </div>
          </div>
          <label htmlFor="case-principal" className="block text-xs text-gray-500">Principal (Cedar entity JSON)</label>
          <textarea id="case-principal" rows={8} value={principal} onChange={(e) => setPrincipal(e.target.value)} className={monoClass} spellCheck={false} />
          <label htmlFor="case-resource" className="block text-xs text-gray-500">Resource (Cedar entity JSON)</label>
          <textarea id="case-resource" rows={8} value={resource} onChange={(e) => setResource(e.target.value)} className={monoClass} spellCheck={false} />
          {formError && <p role="alert" className="text-sm text-red-700 dark:text-red-300">{formError}</p>}
          {name.trim() && ready() && (
            <ConfirmAction
              verb="Add"
              target="this case"
              consequence="The case runs against every draft from now on."
              onConfirm={async (reason) => {
                const c = ready();
                if (c) await apiClient.addPolicyCase(c, reason);
              }}
              onSuccess={() => {
                setAdding(false);
                setName("");
                void cases.reload();
              }}
            />
          )}
        </div>
      )}
      {cases.data.length === 0 ? (
        <EmptyState title="No test cases" description="Add a case to check a draft before it is switched on." />
      ) : (
        <div className="overflow-x-auto">
          <table className="w-full text-sm" aria-label="Test cases">
            <thead>
              <tr className="text-left text-xs text-gray-500">
                <th className="pr-3">Name</th>
                <th className="pr-3">Action</th>
                <th className="pr-3">Expected</th>
                <th className="pr-3">Added by</th>
                <th />
              </tr>
            </thead>
            <tbody>
              {cases.data.map((c) => (
                <tr key={c.id} className="border-t border-gray-200 dark:border-gray-800">
                  <td className="pr-3 py-1">{c.name}</td>
                  <td className="pr-3 font-mono text-xs">{c.action}</td>
                  <td className="pr-3">{c.expected}</td>
                  <td className="pr-3">{c.created_by}</td>
                  <td>
                    <button type="button" className={buttonClass} aria-label={`Remove ${c.name}`} onClick={() => setRemoving(c)}>Remove</button>
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}
      {removing && (
        <div className="rounded border p-3">
          <ConfirmAction
            verb="Remove"
            target={removing.name}
            consequence="Drafts are no longer checked against this case."
            onConfirm={(reason) => apiClient.removePolicyCase(removing.id, reason)}
            onSuccess={() => {
              setRemoving(null);
              void cases.reload();
            }}
          />
        </div>
      )}
    </div>
  );
}

function SimulatorTab() {
  const versions = useAsync(() => apiClient.adminPolicyVersions());
  const [versionId, setVersionId] = useState("");
  const [action, setAction] = useState("read");
  const [principal, setPrincipal] = useState(pretty(SAMPLE_USER));
  const [resource, setResource] = useState(pretty(SAMPLE_RESOURCE));
  const [result, setResult] = useState<PolicySimulation | null>(null);
  const [formError, setFormError] = useState<string | null>(null);
  const [error, setError] = useState<unknown>(null);

  const run = async () => {
    setError(null);
    setResult(null);
    const p = parseJson("Principal", principal);
    const r = parseJson("Resource", resource);
    if (p.error || r.error) {
      setFormError(p.error ?? r.error ?? null);
      return;
    }
    setFormError(null);
    try {
      setResult(await apiClient.simulatePolicy({ ...(versionId ? { version_id: versionId } : {}), principal: p.value, action, resource: r.value }));
    } catch (e) {
      setError(e);
    }
  };

  return (
    <div className="space-y-3">
      <div className="flex flex-wrap gap-3">
        <div>
          <label htmlFor="sim-version" className="block text-xs text-gray-500">Policy version</label>
          <select id="sim-version" value={versionId} onChange={(e) => setVersionId(e.target.value)} className={inputClass}>
            <option value="">The active version</option>
            {(versions.data ?? []).map((v) => (
              <option key={v.id} value={v.id}>Version {v.version} ({v.status})</option>
            ))}
          </select>
        </div>
        <div>
          <label htmlFor="sim-action" className="block text-xs text-gray-500">Action</label>
          <input id="sim-action" value={action} onChange={(e) => setAction(e.target.value)} className={inputClass} />
        </div>
      </div>
      <label htmlFor="sim-principal" className="block text-xs text-gray-500">Principal (Cedar entity JSON)</label>
      <textarea id="sim-principal" rows={8} value={principal} onChange={(e) => setPrincipal(e.target.value)} className={monoClass} spellCheck={false} />
      <label htmlFor="sim-resource" className="block text-xs text-gray-500">Resource (Cedar entity JSON)</label>
      <textarea id="sim-resource" rows={8} value={resource} onChange={(e) => setResource(e.target.value)} className={monoClass} spellCheck={false} />
      <button type="button" className={buttonClass} onClick={() => void run()}>Run</button>
      {formError && <p role="alert" className="text-sm text-red-700 dark:text-red-300">{formError}</p>}
      {error !== null && <ErrorState error={error} />}
      {result && (
        <div className="rounded border border-gray-200 dark:border-gray-800 p-3 text-sm space-y-1" aria-label="Simulation result">
          <p><StatusBadge tone={result.decision === "Allow" ? "ok" : "bad"} text={result.decision} /> by version {result.version}</p>
          {result.deciding_policy && <p>Decided by {result.deciding_policy.id}: {result.deciding_policy.description}</p>}
          {result.reasons.length > 0 && <p>Policies that matched: {result.reasons.join(", ")}</p>}
          {result.diagnostics.map((d) => (
            <p key={d} className="text-amber-700 dark:text-amber-300">{d}</p>
          ))}
        </div>
      )}
    </div>
  );
}

function OscalTab() {
  const rows = useAsync(() => apiClient.adminOscalRequirements());
  const [editing, setEditing] = useState<OscalRequirement | null>(null);
  const [statement, setStatement] = useState("");
  const [status, setStatus] = useState("implemented");
  const [error, setError] = useState<unknown>(null);

  if (rows.error) return <ErrorState error={rows.error} onRetry={() => void rows.reload()} />;
  if (!rows.data) return <Skeleton rows={4} />;
  const titles = Object.fromEntries(rows.data.filter((r) => r.title).map((r) => [r.control_id, r.title as string]));

  const exportDoc = async () => {
    setError(null);
    try {
      downloadJson("scaffoldry-oscal-1.1.2.json", await apiClient.getOscalComponentDefinition());
    } catch (e) {
      setError(e);
    }
  };

  return (
    <div className="space-y-3">
      <div>
        <button type="button" className={buttonClass} onClick={() => void exportDoc()}>Export</button>
      </div>
      {error !== null && <ErrorState error={error} />}
      <div className="overflow-x-auto">
        <table className="w-full text-sm" aria-label="OSCAL requirements">
          <thead>
            <tr className="text-left text-xs text-gray-500">
              <th className="pr-3">Control</th>
              <th className="pr-3">Statement</th>
              <th className="pr-3">Status</th>
              <th className="pr-3">Policies</th>
              <th className="pr-3">Ledger evidence</th>
              <th />
            </tr>
          </thead>
          <tbody>
            {rows.data.map((r) => (
              <tr key={r.control_id} className="border-t border-gray-200 dark:border-gray-800 align-top">
                <td className="pr-3 py-1"><ControlLabel id={r.control_id} lookup={titles} /></td>
                <td className="pr-3">{r.statement}</td>
                <td className="pr-3">{r.status}</td>
                <td className="pr-3">{r.policy_ids.length === 0 ? "None" : r.policy_ids.join(", ")}</td>
                <td className="pr-3">{r.evidence_count} {r.evidence_count === 1 ? "entry" : "entries"}{r.latest_evidence ? `, latest ${r.latest_evidence}` : ""}</td>
                <td>
                  <button
                    type="button"
                    className={buttonClass}
                    aria-label={`Edit ${r.control_id}`}
                    onClick={() => {
                      setEditing(r);
                      setStatement(r.statement);
                      setStatus(r.status);
                    }}
                  >
                    Edit
                  </button>
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
      {editing && (
        <div className="space-y-2 rounded border border-gray-200 dark:border-gray-800 p-3">
          <h3 className="text-sm font-semibold"><ControlLabel id={editing.control_id} lookup={titles} /></h3>
          <label htmlFor="req-statement" className="block text-xs text-gray-500">Statement</label>
          <textarea id="req-statement" rows={3} value={statement} onChange={(e) => setStatement(e.target.value)} className={`${inputClass} w-full`} />
          <label htmlFor="req-status" className="block text-xs text-gray-500">Implementation status</label>
          <select id="req-status" value={status} onChange={(e) => setStatus(e.target.value)} className={inputClass}>
            {STATUSES.map((s) => (
              <option key={s} value={s}>{s}</option>
            ))}
          </select>
          {statement.trim() && (
            <ConfirmAction
              verb="Save"
              target={editing.control_id}
              consequence="The statement and status go into the OSCAL document the institution exports."
              onConfirm={async (reason) => {
                await apiClient.putOscalRequirement(editing.control_id, { statement, status }, reason);
              }}
              onSuccess={() => {
                setEditing(null);
                void rows.reload();
              }}
            />
          )}
        </div>
      )}
    </div>
  );
}

// The Policy and OSCAL section of the console (admin console phase 5). The running policy is a stored, versioned, testable thing.
// A draft is activated by a different Platform Admin, and only when every test case passes.
export function PolicyOscal() {
  const [tab, setTab] = useState("active");
  return (
    <div className="space-y-4">
      <PageHeader title="Policy and OSCAL" description="The rules every server runs, how they are tested before they change, and the OSCAL document that maps each control to the policies that enforce it." />
      <Tabs ariaLabel="Policy and OSCAL" selectedId={tab} onSelect={setTab} tabs={TABS} />
      {tab === "active" ? <ActiveTab /> : tab === "versions" ? <VersionsTab /> : tab === "cases" ? <CasesTab /> : tab === "simulator" ? <SimulatorTab /> : <OscalTab />}
    </div>
  );
}
