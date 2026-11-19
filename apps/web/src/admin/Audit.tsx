import { useCallback, useMemo, useState, type FormEvent } from "react";
import { apiClient, type AdminLedgerRow, type LedgerVerification } from "../api";
import { Banner, ControlLabel, DataTable, Drawer, EmptyState, ErrorState, FormField, PageHeader, StatusBadge, explainError, type DataTableColumn } from "../ui";
import { downloadText } from "./download";

const buttonClass = "px-3 py-1.5 text-sm font-medium rounded border border-gray-300 dark:border-gray-700 disabled:opacity-50";
const inputClass = "w-full text-sm rounded border border-gray-300 dark:border-gray-700 p-2 bg-white dark:bg-gray-800 text-gray-900 dark:text-gray-100";

interface Filters {
  decision_type: string;
  app: string;
  control: string;
  from: string;
  to: string;
}

const NO_FILTERS: Filters = { decision_type: "", app: "", control: "", from: "", to: "" };

function when(iso: string): string {
  const d = new Date(iso);
  return Number.isNaN(d.getTime()) ? iso : d.toLocaleString("en-US", { dateStyle: "medium", timeStyle: "short" });
}

/** What the server was asked for: the filters that are filled in, and the person from the search box. */
function paramsOf(f: Filters, principal?: string): Record<string, string | undefined> {
  const out: Record<string, string | undefined> = {};
  for (const [k, v] of Object.entries(f)) out[k] = v.trim() || undefined;
  out.principal = principal?.trim() || undefined;
  return out;
}

// The Audit section of the console (admin console phase 7). The ledger a page at a time with the filters the database
// indexes, each entry with the payload it was stored with, a check of the whole chain, and an export.
export function Audit() {
  const [draft, setDraft] = useState<Filters>(NO_FILTERS);
  const [applied, setApplied] = useState<Filters>(NO_FILTERS);
  const [selected, setSelected] = useState<AdminLedgerRow | null>(null);
  const [verifying, setVerifying] = useState(false);
  const [verification, setVerification] = useState<LedgerVerification | null>(null);
  const [problem, setProblem] = useState<string | null>(null);
  const [exporting, setExporting] = useState<"json" | "csv" | null>(null);
  const [search, setSearch] = useState<string | undefined>(undefined);

  const load = useCallback(async (cursor?: string, who?: string, f?: Record<string, unknown>) => {
    setSearch(who);
    const res = await apiClient.adminLedger({ ...paramsOf((f?.filters as Filters) ?? NO_FILTERS, who), cursor, limit: "50" });
    return { rows: res.rows, next_cursor: res.next_cursor ?? undefined };
  }, []);
  const filterValues = useMemo(() => ({ filters: applied }), [applied]);

  const columns: DataTableColumn<AdminLedgerRow>[] = [
    { id: "sequence", header: "#", accessorKey: "sequence" },
    { id: "timestamp_iso", header: "When", cell: ({ row }) => when(row.original.timestamp_iso) },
    { id: "principal", header: "Who", accessorKey: "principal" },
    { id: "decision_type", header: "Decision", accessorKey: "decision_type" },
    { id: "control", header: "Control", cell: ({ row }) => <ControlLabel id={row.original.oscal_control_id} title={row.original.control_title} /> },
    { id: "app_slug", header: "App", cell: ({ row }) => row.original.app_slug ?? "" },
    { id: "rationale", header: "Why", accessorKey: "rationale" },
  ];

  const verify = async () => {
    setVerifying(true);
    setProblem(null);
    try {
      setVerification(await apiClient.verifyAdminLedger());
    } catch (e) {
      setVerification(null);
      setProblem(explainError(e).message);
    } finally {
      setVerifying(false);
    }
  };

  const exportAs = async (format: "json" | "csv") => {
    setExporting(format);
    setProblem(null);
    try {
      const text = await apiClient.exportAdminLedger(format, paramsOf(applied, search));
      downloadText(`scaffoldry-ledger.${format}`, text, format === "csv" ? "text/csv;charset=utf-8" : "application/json");
    } catch (e) {
      setProblem(explainError(e).message);
    } finally {
      setExporting(null);
    }
  };

  const apply = (e: FormEvent) => {
    e.preventDefault();
    setApplied(draft);
  };

  return (
    <div className="space-y-6" data-testid="admin-audit">
      <PageHeader
        title="Audit"
        description="Every decision the platform recorded, newest first. Each entry holds the hash of the one before it, so a change to an old entry breaks the chain after it."
      />

      <section className="rounded border p-3 space-y-2" aria-label="Chain check">
        <div className="flex flex-wrap items-center gap-3">
          <button type="button" className={buttonClass} onClick={() => void verify()} disabled={verifying}>
            {verifying ? "Verifying…" : "Verify chain"}
          </button>
          <p className="text-xs text-gray-500">This reads the whole ledger from the start. On a large ledger it takes a while.</p>
        </div>
        {verification &&
          (verification.valid ? (
            <Banner kind="info" label="Chain check">
              The chain is intact. {verification.entries} entries, ending at <span className="font-mono">{verification.head_hash.slice(0, 16)}</span>.
            </Banner>
          ) : (
            <Banner kind="error" label="Chain check">
              The chain is broken at entry {verification.first_bad_sequence}. {verification.detail ?? ""}
            </Banner>
          ))}
      </section>

      <form onSubmit={apply} className="grid grid-cols-1 sm:grid-cols-3 gap-3 items-end" aria-label="Ledger filters">
        <FormField id="audit-decision" label="Decision type">
          {(props) => <input {...props} className={inputClass} value={draft.decision_type} onChange={(e) => setDraft({ ...draft, decision_type: e.target.value })} placeholder="PolicyRevision" />}
        </FormField>
        <FormField id="audit-control" label="Control">
          {(props) => <input {...props} className={inputClass} value={draft.control} onChange={(e) => setDraft({ ...draft, control: e.target.value })} placeholder="AC-2" />}
        </FormField>
        <FormField id="audit-app" label="App">
          {(props) => <input {...props} className={inputClass} value={draft.app} onChange={(e) => setDraft({ ...draft, app: e.target.value })} placeholder="app slug" />}
        </FormField>
        <FormField id="audit-from" label="From">
          {(props) => <input {...props} type="date" className={inputClass} value={draft.from} onChange={(e) => setDraft({ ...draft, from: e.target.value })} />}
        </FormField>
        <FormField id="audit-to" label="To">
          {(props) => <input {...props} type="date" className={inputClass} value={draft.to} onChange={(e) => setDraft({ ...draft, to: e.target.value })} />}
        </FormField>
        <div className="flex gap-2">
          <button type="submit" className={buttonClass}>
            Apply filters
          </button>
          <button
            type="button"
            className={buttonClass}
            onClick={() => {
              setDraft(NO_FILTERS);
              setApplied(NO_FILTERS);
            }}
          >
            Clear
          </button>
        </div>
      </form>

      <div className="flex flex-wrap items-center gap-2" aria-label="Export">
        <button type="button" className={buttonClass} disabled={exporting !== null} onClick={() => void exportAs("json")}>
          Export JSON
        </button>
        <button type="button" className={buttonClass} disabled={exporting !== null} onClick={() => void exportAs("csv")}>
          Export CSV
        </button>
        <p className="text-xs text-gray-500">An export holds the entries that match the filters, at most 50,000. A JSON export starts with the head hash and a check of the chain.</p>
      </div>
      {problem && <ErrorState message={problem} />}

      <DataTable<AdminLedgerRow>
        columns={columns}
        load={load}
        rowKey="sequence"
        onRowOpen={setSelected}
        filterValues={filterValues}
        clientExport={false}
        emptyState={<EmptyState title="No entry matches" description="Widen the filters, or clear them." />}
      />

      <Drawer open={selected !== null} onClose={() => setSelected(null)} title="Ledger entry" testId="admin-ledger-entry" size="xl">
        {selected && <Entry row={selected} />}
      </Drawer>
    </div>
  );
}

function Entry({ row }: { row: AdminLedgerRow }) {
  return (
    <div className="space-y-4 text-sm">
      <div>
        <h3 className="text-base font-semibold">
          Entry {row.sequence}: {row.decision_type}
        </h3>
        <p className="text-xs text-gray-500">
          {row.principal}, {when(row.timestamp_iso)}. <ControlLabel id={row.oscal_control_id} title={row.control_title} />
          {row.app_slug ? `, app ${row.app_slug}` : ""}
        </p>
      </div>
      <p>{row.rationale}</p>
      <section className="space-y-1">
        <h4 className="text-sm font-semibold">Payload</h4>
        <pre aria-label="Stored payload" className="text-xs overflow-x-auto rounded bg-gray-50 dark:bg-gray-900 p-2">
          {JSON.stringify(row.payload, null, 2)}
        </pre>
      </section>
      <section className="space-y-1">
        <h4 className="text-sm font-semibold">Hashes</h4>
        <dl className="text-xs font-mono break-all space-y-1">
          <div>
            <dt className="font-sans text-gray-500">This entry</dt>
            <dd>{row.entry_hash}</dd>
          </div>
          <div>
            <dt className="font-sans text-gray-500">The entry before it</dt>
            <dd>{row.previous_hash}</dd>
          </div>
          <div>
            <dt className="font-sans text-gray-500">Payload</dt>
            <dd>{row.payload_hash}</dd>
          </div>
        </dl>
      </section>
      <StatusBadge tone="neutral" text={`Organization ${row.organization_code}`} />
    </div>
  );
}
