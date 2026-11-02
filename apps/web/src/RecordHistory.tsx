import { useCallback, useState } from "react";
import { apiClient, restoreRecord, type HistoryEntry } from "./api";
import type { FieldSpec } from "./types";
import { ConfirmAction, EmptyState, ErrorState, Skeleton, useAsync } from "./ui";

/** `14:03` from a stored time, in the time the server wrote. */
function clock(at: string): string {
  const m = at.match(/[T ](\d{2}:\d{2})/);
  return m ? m[1] : at;
}

function shown(v: unknown): string {
  if (v === null || v === undefined || v === "") return "blank";
  if (Array.isArray(v)) return v.length ? v.map(shown).join(", ") : "blank";
  if (typeof v === "object") return "a new value";
  return String(v);
}

function labelOf(name: string, fields: FieldSpec[]): string {
  return fields.find((f) => f.name === name)?.label ?? name;
}

/** Who did it, in words: a person's name, and the agent they acted through. */
function who(e: HistoryEntry): string {
  const name = e.actor_name || e.actor;
  return e.token_label && e.actor_kind !== "user" ? `${name}, through "${e.token_label}",` : name;
}

/** One history entry as a sentence: `Dr. Rivera changed Status from Pending to Admitted, 14:03.` */
export function historySentence(e: HistoryEntry, fields: FieldSpec[]): string {
  const time = clock(e.at);
  if (e.action === "create") return `${who(e)} created the record, ${time}.`;
  if (e.action === "delete") return `${who(e)} moved the record to trash, ${time}.`;
  if (e.action === "restore" && Object.keys(e.changes).length === 0) return `${who(e)} restored the record, ${time}.`;
  const parts = Object.entries(e.changes).map(([field, c]) =>
    c.changed ? `${labelOf(field, fields)}` : `${labelOf(field, fields)} from ${shown(c.old)} to ${shown(c.new)}`,
  );
  return `${who(e)} changed ${parts.join(" and ")}, ${time}.`;
}

export interface RecordHistoryProps {
  appSlug: string;
  recordId: string;
  fields: FieldSpec[];
  /** Editors and above may put a value back. */
  canRestore: boolean;
}

/** A record's history as sentences, newest first, each with `Restore this value` for editors and above. */
export function RecordHistory({ appSlug, recordId, fields, canRestore }: RecordHistoryProps) {
  const load = useCallback(() => apiClient.getRecordHistory(appSlug, recordId), [appSlug, recordId]);
  const { data, error, loading, reload } = useAsync(load);
  const [restoring, setRestoring] = useState<{ id: number; field: string } | null>(null);

  if (loading && !data) return <Skeleton rows={3} />;
  if (error) return <ErrorState error={error} onRetry={() => void reload()} />;
  if (!data || data.entries.length === 0) return <EmptyState title="No history yet" description="Each change to this record will be listed here." />;

  return (
    <ul aria-label="History" className="space-y-2">
      {data.entries.map((e) => (
        <li key={e.id} className="rounded-md border border-gray-200 dark:border-gray-700 p-3 text-sm space-y-2">
          <p>{historySentence(e, fields)}</p>
          {canRestore &&
            Object.entries(e.changes)
              .filter(([, c]) => !c.changed)
              .map(([field]) =>
                restoring?.id === e.id && restoring.field === field ? (
                  <ConfirmAction
                    key={field}
                    verb="Restore"
                    target={labelOf(field, fields)}
                    consequence={`${labelOf(field, fields)} goes back to the value this change wrote. The restore is a new change of its own, and it shows here.`}
                    audited
                    requireReason
                    onConfirm={async () => {
                      const current = await apiClient.getRecord(appSlug, recordId);
                      await restoreRecord(appSlug, recordId, { to_version: e.version, fields: [field], version: current.version });
                    }}
                    onSuccess={() => {
                      setRestoring(null);
                      void reload();
                    }}
                  />
                ) : (
                  <button key={field} type="button" className="text-xs underline" onClick={() => setRestoring({ id: e.id, field })}>
                    Restore this value
                  </button>
                ),
              )}
        </li>
      ))}
    </ul>
  );
}

/** The app-wide activity feed, for owners and admins: the same sentences, every record. */
export function AppActivity({ appSlug, fields }: { appSlug: string; fields: FieldSpec[] }) {
  const load = useCallback(() => apiClient.getAppHistory(appSlug), [appSlug]);
  const { data, error, loading, reload } = useAsync(load);
  if (loading && !data) return <Skeleton rows={5} />;
  if (error) return <ErrorState error={error} onRetry={() => void reload()} />;
  if (!data || data.entries.length === 0) return <EmptyState title="No activity yet" description="Changes to this app's records will be listed here." />;
  return (
    <ul aria-label="Activity" className="space-y-2">
      {data.entries.map((e) => (
        <li key={e.id} className="text-sm">
          {historySentence(e, fields)}
        </li>
      ))}
    </ul>
  );
}
