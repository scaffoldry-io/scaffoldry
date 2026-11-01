import type { AppTable, FieldSpec } from "./types";

/** The most target fields the mini table shows. The drawer of the target shows the rest. */
export const MINI_TABLE_FIELDS = 4;

/** The ids a link value holds. The server accepts one id or a list. */
export function linkedIds(value: unknown): string[] {
  if (Array.isArray(value)) return value.filter((v): v is string => typeof v === "string" && v !== "");
  return typeof value === "string" && value !== "" ? [value] : [];
}

/** A record's label: its table's primary field, or its id when that is blank or hidden. */
export function recordLabel(table: AppTable | undefined, record: Record<string, unknown> | undefined, id: string): string {
  const v = record && table?.primary_field ? record[table.primary_field] : undefined;
  return typeof v === "string" && v !== "" ? v : typeof v === "number" ? String(v) : id;
}

function cellText(v: unknown): string {
  if (v === null || v === undefined) return "";
  if (Array.isArray(v)) return v.map(cellText).join(", ");
  if (typeof v === "object") return "";
  return String(v);
}

export interface LinkedRecordsProps {
  field: FieldSpec;
  /** The table the field points at, with the records the person has loaded. */
  target: AppTable | undefined;
  value: unknown;
  /** Writes the field. Unlink sends the ids that remain. */
  onChange: (next: string[]) => void;
  /** Opens a linked record's drawer. Without it there is no Open button. */
  onOpen?: (tableId: string, id: string) => void;
}

/**
 * The records one link field holds, as a mini table of the target's fields, each with Open and Unlink. A mirror field
 * is a link field of its own table, so it is shown the same way. A record the person has not loaded shows its id only.
 */
export function LinkedRecords({ field, target, value, onChange, onOpen }: LinkedRecordsProps) {
  const ids = linkedIds(value);
  const columns = (target?.fields ?? []).filter((f) => f.field_type !== "Link" && f.field_type !== "Attachment").slice(0, MINI_TABLE_FIELDS);
  const rows = ids.map((id) => {
    const record = target?.records?.find((r) => String(r.id) === id);
    return { id, record, label: recordLabel(target, record, id) };
  });

  return (
    <div className="space-y-2" data-testid={`linked-records-${field.name}`}>
      <h5 className="text-xs font-semibold text-slate-600 dark:text-slate-300">
        {field.label}
        {target && <span className="font-normal text-slate-400"> · {target.name}</span>}
      </h5>
      {rows.length === 0 ? (
        <p className="text-xs italic text-slate-400">No linked records.</p>
      ) : (
        <table aria-label={field.label} className="w-full text-xs">
          <thead>
            <tr>
              {columns.map((c) => (
                <th key={c.name} scope="col" className="text-left font-semibold text-slate-500 px-2 py-1">
                  {c.label}
                </th>
              ))}
              <th scope="col" className="sr-only">
                Actions
              </th>
            </tr>
          </thead>
          <tbody>
            {rows.map(({ id, record, label }) => (
              <tr key={id} className="border-t border-slate-100 dark:border-slate-800">
                {record ? (
                  columns.map((c) => (
                    <td key={c.name} className="px-2 py-1 text-slate-700 dark:text-slate-200">
                      {cellText(record[c.name])}
                    </td>
                  ))
                ) : (
                  <td colSpan={Math.max(columns.length, 1)} className="px-2 py-1 font-mono text-slate-500">
                    {id}
                  </td>
                )}
                <td className="px-2 py-1 text-right whitespace-nowrap">
                  {onOpen && record && target && (
                    <button type="button" className="underline mr-2" aria-label={`Open ${label}`} onClick={() => onOpen(target.id, id)}>
                      Open
                    </button>
                  )}
                  <button type="button" className="underline text-red-700 dark:text-red-300" aria-label={`Unlink ${label}`} onClick={() => onChange(ids.filter((x) => x !== id))}>
                    Unlink
                  </button>
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
    </div>
  );
}
