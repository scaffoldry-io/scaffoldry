import { useCallback, useEffect, useMemo, useState, type FormEvent } from "react";
import { ProcessInsightsPanel } from "../ProcessInsights";
import {
  apiClient,
  type AdminInstanceApprovers,
  type AdminProcessDetail,
  type AdminProcessInstance,
  type AdminProcessRow,
  type AdminProcessVersion,
  type AdminUserRow,
  type ProcessSimulation,
} from "../api";
import {
  ConfirmAction,
  DataTable,
  Drawer,
  EmptyState,
  ErrorState,
  FormField,
  PageHeader,
  Skeleton,
  StatusBadge,
  Tabs,
  useAsync,
  type DataTableColumn,
} from "../ui";

type TabId = "definitions" | "instances" | "insights";

const buttonClass = "px-3 py-1.5 text-sm font-medium rounded border border-gray-300 dark:border-gray-700 disabled:opacity-50";
const smallButton = "px-2 py-1 text-xs font-medium rounded border border-gray-300 dark:border-gray-700 hover:bg-gray-50 dark:hover:bg-gray-800";
const inputClass = "w-full text-sm rounded border border-gray-300 dark:border-gray-700 p-2 bg-white dark:bg-gray-800 text-gray-900 dark:text-gray-100";

function when(iso: string | null): string {
  if (!iso) return "never";
  const d = new Date(iso);
  return Number.isNaN(d.getTime()) ? iso : d.toLocaleString("en-US", { dateStyle: "medium", timeStyle: "short" });
}

export interface ProcessesProps {
  initialTab?: TabId;
  /** Opens the app's workflow tab in the app builder. */
  onEditInBuilder?: (appSlug: string) => void;
}

// The Business processes section of the console (admin console phase 6). Definitions are edited in the app builder. The
// console governs them: what each does, its versions, turning one off or on, and trying one out. Instances are the
// processes that wait on a person: who can decide, who it is reassigned to, and cancelling one that cannot go on.
export function Processes({ initialTab = "definitions", onEditInBuilder }: ProcessesProps) {
  const [tab, setTab] = useState<TabId>(initialTab);
  return (
    <div className="space-y-6" data-testid="admin-processes">
      <PageHeader
        title="Business processes"
        description="The processes that wait on a person to decide. Definitions are edited in the app builder. Here you see what each does, switch one off or on, and look after the ones that are stuck."
      />
      <Tabs
        ariaLabel="Business processes"
        selectedId={tab}
        onSelect={(id) => setTab(id as TabId)}
        tabs={[
          { id: "definitions", label: "Definitions" },
          { id: "instances", label: "Instances" },
          { id: "insights", label: "Insights" },
        ]}
      />
      {tab === "definitions" && <Definitions onEditInBuilder={onEditInBuilder} />}
      {tab === "instances" && <Instances />}
      {tab === "insights" && <ProcessInsightsPanel />}
    </div>
  );
}

// --- Definitions ---------------------------------------------------------------------------------------------------------

function Definitions({ onEditInBuilder }: { onEditInBuilder?: (appSlug: string) => void }) {
  const [offOnly, setOffOnly] = useState(false);
  const [version, setVersion] = useState(0);
  const [selected, setSelected] = useState<string | null>(null);
  const filterValues = useMemo(() => ({ offOnly, version }), [offOnly, version]);

  const load = useCallback(async (cursor?: string, search?: string, f?: Record<string, unknown>) => {
    const res = await apiClient.adminProcesses({ cursor, search, limit: "50", enabled: f?.offOnly ? "false" : undefined });
    return { rows: res.rows, next_cursor: res.next_cursor ?? undefined };
  }, []);

  const columns: DataTableColumn<AdminProcessRow>[] = [
    {
      id: "name",
      header: "Process",
      cell: ({ row }) => (
        <div>
          <div className="font-medium">{row.original.name}</div>
          <div className="text-xs text-gray-500">{row.original.app_slug}</div>
        </div>
      ),
    },
    { id: "workspace_name", header: "Workspace", accessorKey: "workspace_name" },
    { id: "version", header: "Version", accessorKey: "version" },
    {
      id: "enabled",
      header: "State",
      cell: ({ row }) => (row.original.enabled ? <StatusBadge tone="ok" text="On" /> : <StatusBadge tone="neutral" text="Off" />),
    },
    { id: "trigger", header: "Starts", accessorKey: "trigger" },
    { id: "step_count", header: "Steps", accessorKey: "step_count" },
    { id: "user_task_roles", header: "Decided by", cell: ({ row }) => row.original.user_task_roles.join(", ") || "No one" },
    { id: "waiting_count", header: "Waiting", accessorKey: "waiting_count" },
    {
      id: "last_changed",
      header: "Last changed",
      cell: ({ row }) => (row.original.last_changed_by ? `${row.original.last_changed_by}, ${when(row.original.last_changed_at)}` : "Not recorded"),
    },
  ];

  return (
    <div className="space-y-4">
      <DataTable<AdminProcessRow>
        columns={columns}
        load={load}
        rowKey="rule_id"
        onRowOpen={(r) => setSelected(r.rule_id)}
        filterValues={filterValues}
        emptyState={<EmptyState title="No process matches" description="A process is made in the app builder, with a step that waits for a person." />}
        filters={
          <label className="inline-flex items-center gap-1.5 text-xs text-gray-700 dark:text-gray-300">
            <input type="checkbox" checked={offOnly} onChange={(e) => setOffOnly(e.target.checked)} />
            Only those turned off
          </label>
        }
      />
      <Drawer open={selected !== null} onClose={() => setSelected(null)} title="Process" testId="admin-process-detail" size="xl">
        {selected && <ProcessDetail ruleId={selected} onChanged={() => setVersion((v) => v + 1)} onEditInBuilder={onEditInBuilder} />}
      </Drawer>
    </div>
  );
}

function Sentences({ sentences, label }: { sentences: string[]; label: string }) {
  return (
    <ol className="list-decimal pl-5 space-y-1 text-sm" aria-label={label}>
      {sentences.map((s, i) => (
        <li key={i}>{s}</li>
      ))}
    </ol>
  );
}

function ProcessDetail({ ruleId, onChanged, onEditInBuilder }: { ruleId: string; onChanged: () => void; onEditInBuilder?: (appSlug: string) => void }) {
  const { data, error, loading, reload } = useAsync<AdminProcessDetail>(() => apiClient.adminProcess(ruleId), true, [ruleId]);
  const [showJson, setShowJson] = useState(false);
  const [switching, setSwitching] = useState(false);
  const [simulating, setSimulating] = useState(false);
  const [viewing, setViewing] = useState<AdminProcessVersion | null>(null);
  const [viewError, setViewError] = useState<unknown>(null);

  if (error) return <ErrorState error={error} onRetry={reload} />;
  if (loading || !data) return <Skeleton rows={5} />;

  const rule = data.rule;
  const enabled = rule.enabled;

  const view = async (v: number) => {
    setViewError(null);
    try {
      setViewing(await apiClient.adminProcessVersion(ruleId, v));
    } catch (e) {
      setViewError(e);
    }
  };

  return (
    <div className="space-y-5">
      <div>
        <h3 className="text-base font-semibold">{rule.name}</h3>
        <p className="text-xs text-gray-500">
          {rule.app_slug}
          {data.workspace_name ? `, in ${data.workspace_name}` : ""}. Version {rule.version}, {enabled ? "on" : "off"}.
        </p>
        <p className="text-xs text-gray-500">
          {Object.entries(data.instance_counts)
            .map(([status, n]) => `${n} ${status.toLowerCase()}`)
            .join(", ")}
        </p>
      </div>

      <section className="space-y-2">
        <h4 className="text-sm font-semibold">What it does</h4>
        <Sentences sentences={data.sentences} label="What it does" />
        <button type="button" className={smallButton} onClick={() => setShowJson((s) => !s)}>
          {showJson ? "Hide JSON" : "Show JSON"}
        </button>
        {showJson && <pre className="text-xs overflow-x-auto rounded bg-gray-50 dark:bg-gray-900 p-2">{JSON.stringify(rule, null, 2)}</pre>}
      </section>

      <div className="flex flex-wrap gap-2">
        <button type="button" className={buttonClass} onClick={() => setSwitching((s) => !s)}>
          {enabled ? "Disable" : "Enable"}
        </button>
        <button type="button" className={buttonClass} onClick={() => setSimulating((s) => !s)}>
          Simulate
        </button>
        <button type="button" className={buttonClass} onClick={() => onEditInBuilder?.(rule.app_slug)} disabled={!onEditInBuilder}>
          Edit in app builder
        </button>
      </div>

      {switching && (
        <div className="rounded border p-3">
          <ConfirmAction
            verb={enabled ? "Disable" : "Enable"}
            target={rule.name}
            consequence={
              enabled
                ? "No new instance starts. Instances that already wait can still be decided."
                : "The process starts again for the next record that matches. This saves a new version."
            }
            onConfirm={async (reason) => {
              await apiClient.setProcessEnabled(ruleId, !enabled, reason);
            }}
            onSuccess={() => {
              setSwitching(false);
              onChanged();
              void reload();
            }}
          />
        </div>
      )}

      {simulating && <Simulate ruleId={ruleId} />}

      <section className="space-y-2">
        <h4 className="text-sm font-semibold">Versions</h4>
        <ul className="space-y-1 text-xs">
          {[...data.versions].reverse().map((v) => (
            <li key={v.version} className="flex flex-wrap items-center gap-x-3 gap-y-1 border-t border-gray-100 dark:border-gray-800 pt-1">
              <span className="font-medium">Version {v.version}</span>
              <span>
                {v.changed_by}, {when(v.changed_at)}
              </span>
              <span className="text-gray-500">{v.summary}</span>
              <span className="font-mono text-gray-400" title={v.sha256}>
                {v.sha256.slice(0, 12)}
              </span>
              <button type="button" className={smallButton} aria-label={`View version ${v.version}`} onClick={() => void view(v.version)}>
                View
              </button>
            </li>
          ))}
        </ul>
        {viewError !== null && <ErrorState error={viewError} />}
        {viewing && (
          <div className="rounded border p-3 space-y-2" aria-label={`Version ${viewing.version}, read only`}>
            <p className="text-xs text-gray-500">Version {viewing.version}, read only. It cannot be changed here.</p>
            <Sentences sentences={viewing.sentences} label={`Version ${viewing.version}`} />
            {showJson && <pre className="text-xs overflow-x-auto rounded bg-gray-50 dark:bg-gray-900 p-2">{JSON.stringify(viewing.rule, null, 2)}</pre>}
            <button type="button" className={smallButton} onClick={() => setViewing(null)}>
              Close version
            </button>
          </div>
        )}
      </section>
    </div>
  );
}

function Simulate({ ruleId }: { ruleId: string }) {
  const [principal, setPrincipal] = useState("");
  const [sample, setSample] = useState('{\n  "id": "sample-1"\n}');
  const [result, setResult] = useState<ProcessSimulation | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  const run = async (e: FormEvent) => {
    e.preventDefault();
    setError(null);
    setResult(null);
    let record: Record<string, unknown>;
    try {
      record = JSON.parse(sample) as Record<string, unknown>;
    } catch {
      setError("The sample record is not valid JSON.");
      return;
    }
    setBusy(true);
    try {
      setResult((await apiClient.simulateProcess(ruleId, { record, principal: principal.trim() })).result);
    } catch (err: unknown) {
      setError(err instanceof Error ? err.message : "The simulation failed");
    } finally {
      setBusy(false);
    }
  };

  return (
    <form onSubmit={run} className="rounded border p-3 space-y-3" aria-label="Simulate">
      <p className="text-xs text-gray-500">Nothing is saved. The process runs once against the sample record, as the person you name.</p>
      <FormField id="simulate-principal" label="Run as" required>
        {(props) => <input {...props} className={inputClass} value={principal} onChange={(e) => setPrincipal(e.target.value)} placeholder="name@state.edu" />}
      </FormField>
      <FormField id="simulate-record" label="Sample record (JSON)">
        {(props) => <textarea {...props} rows={4} className={`${inputClass} font-mono`} value={sample} onChange={(e) => setSample(e.target.value)} />}
      </FormField>
      <button type="submit" className={buttonClass} disabled={busy || principal.trim() === ""}>
        Run simulation
      </button>
      {error && <p role="alert" className="text-sm text-red-600">{error}</p>}
      {result && (
        <ul className="text-sm space-y-1" aria-label="Simulation result">
          <li>{result.trigger_matched ? "The trigger matches." : "The trigger does not match this event."}</li>
          <li>{result.conditions_met ? "The conditions are met." : "The conditions are not met."}</li>
          {result.waiting_instance && <li>It would wait at step {result.waiting_instance.waiting_step_id} for {result.waiting_instance.role ?? "a decision"}.</li>}
          {result.actions_executed.map((a, i) => (
            <li key={i}>{a}</li>
          ))}
        </ul>
      )}
    </form>
  );
}

// --- Instances -----------------------------------------------------------------------------------------------------------

type InstanceAction = { kind: "who" | "reassign" | "cancel"; inst: AdminProcessInstance };

function Instances() {
  const [flags, setFlags] = useState({ waiting: false, stale: false, assigned: false, unassigned: false });
  const [version, setVersion] = useState(0);
  const [action, setAction] = useState<InstanceAction | null>(null);
  const filterValues = useMemo(() => ({ ...flags, version }), [flags, version]);

  const load = useCallback(async (cursor?: string, search?: string, f?: Record<string, unknown>) => {
    const res = await apiClient.adminProcessInstances({
      cursor,
      search,
      limit: "50",
      status: f?.waiting ? "Waiting" : undefined,
      stale: f?.stale ? "true" : undefined,
      assigned: f?.assigned ? "true" : undefined,
      unassigned: f?.unassigned ? "true" : undefined,
    });
    return { rows: res.rows, next_cursor: res.next_cursor ?? undefined };
  }, []);

  const label = (i: AdminProcessInstance) => `${i.rule_name} on ${i.record_id}`;
  const open = (kind: InstanceAction["kind"], inst: AdminProcessInstance) => (e: { stopPropagation: () => void }) => {
    e.stopPropagation();
    setAction({ kind, inst });
  };

  const columns: DataTableColumn<AdminProcessInstance>[] = [
    {
      id: "rule_name",
      header: "Process",
      cell: ({ row }) => (
        <div>
          <div className="font-medium">{row.original.rule_name}</div>
          <div className="text-xs text-gray-500">
            {row.original.app_slug}, version {row.original.rule_version}
          </div>
        </div>
      ),
    },
    { id: "record_id", header: "Record", accessorKey: "record_id" },
    {
      id: "status",
      header: "Status",
      cell: ({ row }) =>
        row.original.stale ? <StatusBadge tone="warn" text="Waiting, stale" /> : <StatusBadge tone={row.original.status === "Waiting" ? "info" : "neutral"} text={row.original.status} />,
    },
    {
      id: "deciders",
      header: "Who decides",
      cell: ({ row }) =>
        row.original.no_approver ? (
          <span className="text-amber-700 dark:text-amber-400 whitespace-normal">{row.original.no_approver_reason ?? "Nobody can decide this step now."}</span>
        ) : (
          row.original.deciders.join(", ") || "Not waiting"
        ),
    },
    { id: "assigned_to", header: "Assigned to", cell: ({ row }) => row.original.assigned_to ?? "By role" },
    { id: "age_days", header: "Age", cell: ({ row }) => (row.original.age_days === 1 ? "1 day" : `${row.original.age_days} days`) },
    {
      id: "actions",
      header: "",
      cell: ({ row }) => {
        const inst = row.original;
        return (
          <div className="flex gap-1.5">
            <button type="button" className={smallButton} aria-label={`Who can decide ${label(inst)}`} onClick={open("who", inst)}>
              Who can decide
            </button>
            {inst.status === "Waiting" && (
              <>
                <button type="button" className={smallButton} aria-label={`Reassign ${label(inst)}`} onClick={open("reassign", inst)}>
                  Reassign
                </button>
                <button type="button" className={smallButton} aria-label={`Cancel ${label(inst)}`} onClick={open("cancel", inst)}>
                  Cancel
                </button>
              </>
            )}
          </div>
        );
      },
    },
  ];

  const toggle = (key: keyof typeof flags, text: string) => (
    <label key={key} className="inline-flex items-center gap-1.5 text-xs text-gray-700 dark:text-gray-300">
      <input type="checkbox" checked={flags[key]} onChange={(e) => setFlags((f) => ({ ...f, [key]: e.target.checked }))} />
      {text}
    </label>
  );

  const close = () => setAction(null);
  const done = () => {
    setAction(null);
    setVersion((v) => v + 1);
  };

  return (
    <div className="space-y-4">
      <DataTable<AdminProcessInstance>
        columns={columns}
        load={load}
        rowKey="id"
        filterValues={filterValues}
        emptyState={<EmptyState title="No instance matches" description="An instance starts when a record meets a process's trigger." />}
        filters={
          <div className="flex flex-wrap items-center gap-3">
            {toggle("waiting", "Waiting")}
            {toggle("stale", "Stale")}
            {toggle("assigned", "Assigned")}
            {toggle("unassigned", "Unassigned")}
          </div>
        }
      />
      <Drawer
        open={action !== null}
        onClose={close}
        title={action ? (action.kind === "who" ? "Who can decide" : action.kind === "reassign" ? "Reassign" : "Cancel") : ""}
        testId="admin-process-instance-action"
      >
        {action?.kind === "who" && <WhoCanDecide inst={action.inst} />}
        {action?.kind === "reassign" && <Reassign inst={action.inst} onDone={done} />}
        {action?.kind === "cancel" && (
          <ConfirmAction
            verb="Cancel instance of"
            target={action.inst.rule_name}
            consequence="The instance ends as failed and nothing it was waiting to do runs. The record is not changed."
            onConfirm={async (reason) => {
              await apiClient.cancelProcessInstance(action.inst.id, reason);
            }}
            onSuccess={done}
          />
        )}
      </Drawer>
    </div>
  );
}

function WhoCanDecide({ inst }: { inst: AdminProcessInstance }) {
  const { data, error, loading, reload } = useAsync<AdminInstanceApprovers>(() => apiClient.adminProcessInstanceApprovers(inst.id), true, [inst.id]);
  if (error) return <ErrorState error={error} onRetry={reload} />;
  if (loading || !data) return <Skeleton rows={3} />;
  return (
    <div className="space-y-3 text-sm">
      {data.approvers.length === 0 ? (
        <p>{data.problem_reason ?? "Nobody can decide this step now."}</p>
      ) : (
        <ul className="space-y-1" aria-label="People who can decide">
          {data.approvers.map((a) => (
            <li key={a.eppn}>
              {a.display_name} ({a.eppn}), by {a.via.toLowerCase()}
            </li>
          ))}
        </ul>
      )}
      {data.unit_name && <p className="text-xs text-gray-500">The position was found at {data.unit_name}.</p>}
    </div>
  );
}

function Reassign({ inst, onDone }: { inst: AdminProcessInstance; onDone: () => void }) {
  const [query, setQuery] = useState("");
  const [people, setPeople] = useState<AdminUserRow[]>([]);
  const [searchError, setSearchError] = useState<unknown>(null);
  // `undefined`: nobody chosen yet. `null`: back to the role.
  const [choice, setChoice] = useState<string | null | undefined>(undefined);

  useEffect(() => {
    if (query.trim().length < 2) {
      setPeople([]);
      return;
    }
    let live = true;
    apiClient
      .adminListUsers({ search: query.trim(), limit: "8" })
      .then((r) => {
        if (live) {
          setSearchError(null);
          setPeople(r.users);
        }
      })
      .catch((e: unknown) => {
        if (live) setSearchError(e);
      });
    return () => {
      live = false;
    };
  }, [query]);

  return (
    <div className="space-y-4">
      <p className="text-sm">
        Choose who decides the waiting step of <span className="font-medium">{inst.rule_name}</span>. They must be able to read the app, and cannot be the person who made the record or started the process.
      </p>
      <label className="block text-xs font-medium">
        Find a person
        <input type="search" aria-label="Find a person" className={`${inputClass} mt-1`} value={query} onChange={(e) => setQuery(e.target.value)} placeholder="Name or user name" />
      </label>
      {searchError !== null && <ErrorState error={searchError} />}
      <ul className="space-y-1">
        {people.map((p) => (
          <li key={p.id} className="flex items-center justify-between gap-2 text-sm">
            <span>
              {p.display_name} ({p.user_name})
            </span>
            <button type="button" className={smallButton} aria-label={`Choose ${p.user_name}`} onClick={() => setChoice(p.user_name)}>
              Choose
            </button>
          </li>
        ))}
      </ul>
      {inst.assigned_to && (
        <button type="button" className={smallButton} onClick={() => setChoice(null)}>
          Return to its role
        </button>
      )}
      {choice !== undefined && (
        <div className="rounded border p-3">
          <ConfirmAction
            verb={choice === null ? "Return to" : "Reassign to"}
            target={choice === null ? "its role" : choice}
            consequence={choice === null ? "Anyone the role resolves to can decide the step again." : `Only ${choice} can decide the step from now on.`}
            onConfirm={async (reason) => {
              await apiClient.reassignProcessInstance(inst.id, choice, reason);
            }}
            onSuccess={onDone}
          />
        </div>
      )}
    </div>
  );
}
