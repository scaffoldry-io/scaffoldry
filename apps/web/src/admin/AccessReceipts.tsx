import { useCallback, useState } from "react";
import { apiClient, type AccessReceiptRow } from "../api";
import { DataTable, StatusBadge, type DataTableColumn } from "../ui";

export function AccessReceipts() {
  const [outcome, setOutcome] = useState("");
  const [checked, setChecked] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  const load = useCallback(
    async (cursor?: string) => {
      const page = await apiClient.accessReceipts({ cursor, outcome: outcome || undefined });
      return { rows: page.receipts, next_cursor: page.next_cursor ?? undefined };
    },
    [outcome]
  );

  const columns: DataTableColumn<AccessReceiptRow>[] = [
    { id: "at", header: "When", accessorKey: "at" },
    { id: "actor", header: "Who", cell: ({ row }) => (row.original.acting_as ? `${row.original.actor} as ${row.original.acting_as}` : row.original.actor) },
    { id: "action", header: "Did", accessorKey: "action" },
    { id: "object", header: "To", cell: ({ row }) => `${row.original.object_kind} ${row.original.object_id}` },
    { id: "outcome", header: "Outcome", cell: ({ row }) => <StatusBadge tone={row.original.outcome === "allow" ? "ok" : "warn"} text={row.original.outcome} /> },
  ];

  const check = async () => {
    setBusy(true);
    try {
      const v = await apiClient.accessVerifyReceipts();
      setChecked(
        v.ok
          ? `The chain is unbroken: ${v.batches_checked} ${v.batches_checked === 1 ? "batch" : "batches"} checked.`
          : `The chain is broken at batch ${v.failed_batch ?? "unknown"}${v.reason ? `: ${v.reason}` : ""}.`
      );
    } catch (e) {
      setChecked(e instanceof Error ? e.message : "The chain could not be checked.");
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="space-y-4" data-testid="access-receipts">
      <div className="flex items-center justify-between gap-4">
        <label className="text-sm text-gray-700 dark:text-gray-300 flex items-center gap-2">
          Outcome
          <select className="text-sm rounded border border-gray-300 dark:border-gray-700 p-1.5 bg-white dark:bg-gray-800" value={outcome} onChange={(e) => setOutcome(e.target.value)}>
            <option value="">All</option>
            <option value="deny">Denied</option>
            <option value="allow">Allowed</option>
          </select>
        </label>
        <button type="button" disabled={busy} onClick={() => void check()} className="px-3 py-1.5 text-sm font-medium rounded-md border border-gray-300 dark:border-gray-700 hover:bg-gray-50 dark:hover:bg-gray-800 disabled:opacity-50">
          Check the chain
        </button>
      </div>
      {checked && <p role="status" className="text-sm text-gray-800 dark:text-gray-200">{checked}</p>}
      <DataTable<AccessReceiptRow> key={outcome} columns={columns} load={load} rowKey={(r) => String(r.id)} />
    </div>
  );
}
