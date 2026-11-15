import { useCallback, useEffect, useState } from "react";
import { apiClient, type AdminDataset, type AdminFieldRow } from "../api";
import { ConfirmAction, EmptyState, ErrorState, PageHeader, Skeleton, Tabs } from "../ui";

const buttonClass = "px-3 py-1.5 text-sm font-medium rounded border border-gray-300 dark:border-gray-700 disabled:opacity-50";
const key = (r: AdminFieldRow) => `${r.app}/${r.table}/${r.field}`;

const SHOW = [
  { id: "", label: "All fields" },
  { id: "sensitive=true", label: "Sensitive" },
  { id: "sensitive=false", label: "Not sensitive" },
  { id: "suggested=true", label: "Suggested" },
];

function FieldsTab() {
  const [rows, setRows] = useState<AdminFieldRow[] | null>(null);
  const [next, setNext] = useState<string | null>(null);
  const [error, setError] = useState<unknown>(null);
  const [show, setShow] = useState("");
  const [search, setSearch] = useState("");
  const [picked, setPicked] = useState<Set<string>>(new Set());
  const [open, setOpen] = useState<string | null>(null);
  const [act, setAct] = useState<"mark" | "remove" | null>(null);

  const load = useCallback(
    async (cursor?: string) => {
      setError(null);
      try {
        const q: Record<string, string> = { search };
        if (show) {
          const [k, v] = show.split("=");
          q[k] = v;
        }
        if (cursor) q.cursor = cursor;
        const page = await apiClient.adminFields(q);
        setRows((old) => (cursor && old ? [...old, ...page.fields] : page.fields));
        setNext(page.next_cursor);
      } catch (e) {
        setError(e);
        setRows([]);
      }
    },
    [show, search]
  );

  useEffect(() => {
    setPicked(new Set());
    void load();
  }, [load]);

  if (error !== null) return <ErrorState error={error} />;
  if (rows === null) return <Skeleton rows={4} />;

  const chosen = rows.filter((r) => picked.has(key(r)));
  const removable = chosen.filter((r) => r.overlay_flag);
  const toggle = (r: AdminFieldRow) =>
    setPicked((p) => {
      const n = new Set(p);
      if (n.has(key(r))) n.delete(key(r));
      else n.add(key(r));
      return n;
    });

  const send = (label: boolean | null) => async (reason: string) => {
    const targets = label === null ? removable : chosen;
    await apiClient.putLabels(
      targets.map((r) => ({ app_slug: r.app, table_id: r.table, field: r.field, ferpa_sensitive: label })),
      reason
    );
  };

  return (
    <div className="space-y-3">
      <div className="flex flex-wrap items-end gap-3">
        <div>
          <label htmlFor="dg-show" className="block text-xs text-gray-500">Show</label>
          <select id="dg-show" value={show} onChange={(e) => setShow(e.target.value)} className="rounded border px-2 py-1.5 text-sm">
            {SHOW.map((s) => (
              <option key={s.id} value={s.id}>{s.label}</option>
            ))}
          </select>
        </div>
        <div>
          <label htmlFor="dg-search" className="block text-xs text-gray-500">Search</label>
          <input id="dg-search" type="search" value={search} onChange={(e) => setSearch(e.target.value)} className="rounded border px-2 py-1.5 text-sm" />
        </div>
        <button type="button" className={buttonClass} disabled={chosen.length === 0} onClick={() => setAct("mark")}>
          Mark sensitive
        </button>
        <button type="button" className={buttonClass} disabled={removable.length === 0} onClick={() => setAct("remove")}>
          Remove label
        </button>
      </div>

      {act && (
        <div className="rounded border p-3">
          <ConfirmAction
            verb={act === "mark" ? "Mark sensitive" : "Remove label from"}
            target={`${act === "mark" ? chosen.length : removable.length} fields`}
            consequence={
              act === "mark"
                ? "These fields are protected now, with no proposal. Records that hold them are flagged, and a viewer no longer sees them."
                : "The label comes off. A field the manifest or a category protects stays protected. Records are flagged again."
            }
            onConfirm={send(act === "mark" ? true : null)}
            onSuccess={() => {
              setAct(null);
              setPicked(new Set());
              void load();
            }}
          />
        </div>
      )}

      {rows.length === 0 ? (
        <EmptyState title="No fields match" description="Change the filter or the search." />
      ) : (
        <div className="overflow-x-auto">
          <table className="w-full text-sm" aria-label="Fields">
            <thead>
              <tr className="text-left text-xs text-gray-500">
                <th className="pr-2 py-1"><span className="sr-only">Select</span></th>
                <th className="pr-3">App</th>
                <th className="pr-3">Table</th>
                <th className="pr-3">Field</th>
                <th className="pr-3">Type</th>
                <th className="pr-3">Sensitive</th>
                <th className="pr-3">CEDS</th>
                <th>Used in</th>
              </tr>
            </thead>
            <tbody>
              {rows.map((r) => (
                <FieldRow key={key(r)} r={r} picked={picked.has(key(r))} onPick={() => toggle(r)} expanded={open === key(r)} onExpand={() => setOpen(open === key(r) ? null : key(r))} />
              ))}
            </tbody>
          </table>
        </div>
      )}
      {next && (
        <button type="button" className={buttonClass} onClick={() => void load(next)}>
          Load more
        </button>
      )}
    </div>
  );
}

function FieldRow({ r, picked, onPick, expanded, onExpand }: { r: AdminFieldRow; picked: boolean; onPick: () => void; expanded: boolean; onExpand: () => void }) {
  const throughLabel = r.effective && !r.manifest_flag;
  return (
    <>
      <tr className="border-t border-gray-200 dark:border-gray-800">
        <td className="pr-2 py-1">
          <input type="checkbox" checked={picked} onChange={onPick} aria-label={`Select ${r.app_title} ${r.table} ${r.field}`} />
        </td>
        <td className="pr-3">{r.app_title}</td>
        <td className="pr-3">{r.table}</td>
        <td className="pr-3">{r.label}{r.label !== r.field ? ` (${r.field})` : ""}</td>
        <td className="pr-3 text-xs">{r.type}</td>
        <td className="pr-3">
          {r.effective ? "Yes" : "No"}
          {throughLabel && <span className="ml-2 rounded bg-amber-100 px-1.5 py-0.5 text-xs text-amber-900">Sensitive only through a label</span>}
          {r.suggested && <span className="ml-2 rounded bg-sky-100 px-1.5 py-0.5 text-xs text-sky-900">Suggested by the standard</span>}
        </td>
        <td className="pr-3 text-xs">{r.ceds_code ?? "—"}</td>
        <td>
          <button type="button" className="text-sm underline" aria-expanded={expanded} aria-label={`Where ${r.field} is used`} onClick={onExpand}>
            {r.used_in}
          </button>
        </td>
      </tr>
      {expanded && (
        <tr>
          <td />
          <td colSpan={7} className="pb-2 text-xs text-gray-700">
            {`Views: ${r.used_in_parts.views}. Formulas: ${r.used_in_parts.formulas}. Custom pages: ${r.used_in_parts.pages}. Rules: ${r.used_in_parts.rules}.`}
            {r.overlay_note ? ` Label note: ${r.overlay_note}` : ""}
          </td>
        </tr>
      )}
    </>
  );
}

function DatasetsTab() {
  const [data, setData] = useState<{ datasets: AdminDataset[]; levels: string[] } | null>(null);
  const [error, setError] = useState<unknown>(null);
  const [level, setLevel] = useState<Record<string, string>>({});
  const [reason, setReason] = useState<Record<string, string>>({});
  const [problem, setProblem] = useState<string | null>(null);

  const load = useCallback(async () => {
    setError(null);
    try {
      const d = await apiClient.adminDatasets();
      setData(d);
      setLevel({});
      setReason({});
    } catch (e) {
      setError(e);
    }
  }, []);

  useEffect(() => {
    void load();
  }, [load]);

  if (error !== null) return <ErrorState error={error} />;
  if (data === null) return <Skeleton rows={3} />;
  if (data.datasets.length === 0) return <EmptyState title="No published datasets" description="A dataset appears here once it is published." />;

  const save = async (d: AdminDataset) => {
    setProblem(null);
    try {
      await apiClient.classifyDataset(d.id, level[d.id] ?? d.sensitivity_level, reason[d.id] ?? "");
      await load();
    } catch (e) {
      setProblem(e instanceof Error ? e.message : "That could not be saved.");
    }
  };

  return (
    <div className="space-y-2">
      {problem && <p role="alert" className="text-sm text-red-700">{problem}</p>}
      <div className="overflow-x-auto">
        <table className="w-full text-sm" aria-label="Datasets">
          <thead>
            <tr className="text-left text-xs text-gray-500">
              <th className="pr-3 py-1">Dataset</th>
              <th className="pr-3">Department</th>
              <th className="pr-3">Records</th>
              <th className="pr-3">Classification</th>
              <th className="pr-3">Reason</th>
              <th />
            </tr>
          </thead>
          <tbody>
            {data.datasets.map((d) => {
              const chosen = level[d.id] ?? d.sensitivity_level;
              return (
                <tr key={d.id} className="border-t border-gray-200 dark:border-gray-800">
                  <td className="pr-3 py-1">{d.name}</td>
                  <td className="pr-3">{d.department}</td>
                  <td className="pr-3">{d.record_count}</td>
                  <td className="pr-3">
                    <select aria-label={`Classification of ${d.name}`} value={chosen} onChange={(e) => setLevel((l) => ({ ...l, [d.id]: e.target.value }))} className="rounded border px-2 py-1 text-sm">
                      {data.levels.map((l) => (
                        <option key={l} value={l}>{l}</option>
                      ))}
                    </select>
                  </td>
                  <td className="pr-3">
                    <input aria-label={`Reason for ${d.name}`} value={reason[d.id] ?? ""} onChange={(e) => setReason((r) => ({ ...r, [d.id]: e.target.value }))} className="w-56 rounded border px-2 py-1 text-sm" />
                  </td>
                  <td>
                    <button type="button" className={buttonClass} aria-label={`Save ${d.name}`} disabled={chosen === d.sensitivity_level || !(reason[d.id] ?? "").trim()} onClick={() => void save(d)}>
                      Save
                    </button>
                  </td>
                </tr>
              );
            })}
          </tbody>
        </table>
      </div>
    </div>
  );
}

// The Data governance section of the console (admin console phase 4). A label raises a field's sensitivity at once and can only add
// protection. A dataset's classification is one of four levels.
export function DataGovernance() {
  const [tab, setTab] = useState<"fields" | "datasets">("fields");
  return (
    <div className="space-y-4">
      <PageHeader title="Data governance" description="Raise a field's sensitivity at once with a label, with no proposal. A label can only add protection. Classify the datasets the institution publishes." />
      <Tabs ariaLabel="Data governance" selectedId={tab} onSelect={(id) => setTab(id as "fields" | "datasets")} tabs={[{ id: "fields", label: "Fields" }, { id: "datasets", label: "Datasets" }]} />
      {tab === "fields" ? <FieldsTab /> : <DatasetsTab />}
    </div>
  );
}
