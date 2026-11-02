import { useCallback, useEffect, useRef, useState } from "react";
import { apiClient, ApiError, type AppRecord } from "./api";
import { subscribe, type LiveEvent } from "./realtime";
import { explainError } from "./ui/explainError";

/** The words for an error, as one sentence pair. */
function sayError(e: unknown): string {
  const x = explainError(e);
  return [x.message, x.next].filter(Boolean).join(" ");
}

/** The text shown when a save lost to someone else's: the usual conflict message. */
export const CONFLICT_NOTICE = sayError(new ApiError(409, "conflict", { code: "version_conflict" }));
export const RESYNC_NOTICE = "Reloaded: changes may have been missed.";
export const REMOVED_NOTICE = "Removed by someone else.";
/** How long a row stays marked after someone else changed it. */
export const HIGHLIGHT_MS = 4000;

/** The cell a person is typing in. */
export interface EditingCell {
  recordId: string;
  fieldName: string;
}

/** The names of the fields whose values differ. */
function changedFields(before: Record<string, unknown>, after: Record<string, unknown>): string[] {
  const names = new Set([...Object.keys(before), ...Object.keys(after)]);
  return [...names].filter((k) => JSON.stringify(before[k]) !== JSON.stringify(after[k]));
}

/**
 * The stored rows of one app. Nothing is invented: before the load finishes, or when it fails, there are no rows.
 * A save names the version it read. On a conflict the row is reloaded and the typed value is not sent again.
 *
 * Real-time phase 2: the rows follow the app's event stream while the view is open. A loaded row that someone else
 * changed is fetched again and marked for a moment. A change to a row that is not loaded is counted, for a Refresh.
 * A cell being typed in keeps its draft: the save then names the version the person began with, and the server's
 * version check decides.
 */
export function useAppRecords(slug: string | null) {
  const [records, setRecords] = useState<AppRecord[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  /** Rows someone else just changed. */
  const [updatedByOthers, setUpdatedByOthers] = useState<ReadonlySet<string>>(new Set());
  /** The rows that are not loaded and changed since the last load. A row counts once however often it changed. */
  const [outside, setOutside] = useState<ReadonlySet<string>>(new Set());
  /** A short message about what someone else did: a removed row, or a reload. */
  const [liveNotice, setLiveNotice] = useState<string | null>(null);
  /** The cell under edit, when someone else changed its value meanwhile. */
  const [changedUnderEdit, setChangedUnderEdit] = useState<EditingCell | null>(null);

  const recordsRef = useRef<AppRecord[]>([]);
  recordsRef.current = records;
  const editing = useRef<EditingCell | null>(null);
  /** The version a draft was begun on, for a row whose edited value changed underneath it. */
  const heldVersion = useRef(new Map<string, number>());
  /** This person's writes still waiting for an answer. Their events can arrive first, and are not someone else's. */
  const saving = useRef(new Set<string>());
  const deleting = useRef(new Set<string>());

  const load = useCallback(async () => {
    if (!slug) {
      setRecords([]);
      return;
    }
    setLoading(true);
    setError(null);
    setOutside(new Set());
    try {
      setRecords(await apiClient.listRecords(slug));
    } catch (e) {
      setRecords([]);
      setError(e instanceof Error ? e.message : "The records could not be loaded.");
    } finally {
      setLoading(false);
    }
  }, [slug]);

  useEffect(() => {
    void load();
  }, [load]);

  const timers = useRef(new Set<ReturnType<typeof setTimeout>>());
  useEffect(() => () => timers.current.forEach(clearTimeout), []);
  const mark = useCallback((id: string) => {
    setUpdatedByOthers((prev) => new Set(prev).add(id));
    const t = setTimeout(() => {
      timers.current.delete(t);
      setUpdatedByOthers((prev) => {
        const next = new Set(prev);
        next.delete(id);
        return next;
      });
    }, HIGHLIGHT_MS);
    timers.current.add(t);
  }, []);

  const onLive = useCallback(
    async (e: LiveEvent) => {
      if (!slug) return;
      if (e.event === "resync") {
        await load();
        setLiveNotice(RESYNC_NOTICE);
        return;
      }
      if (e.event !== "change") return;
      const id = String(e.data?.record ?? "");
      const version = Number(e.data?.version ?? 0);
      const held = recordsRef.current.find((r) => r.id === id);
      if (e.data?.kind === "delete") {
        // A row this view does not hold, or one this person is deleting, needs nothing.
        if (!held || deleting.current.has(id)) return;
        setRecords((prev) => prev.filter((r) => r.id !== id));
        setLiveNotice(REMOVED_NOTICE);
        return;
      }
      if (!held) {
        setOutside((prev) => (prev.has(id) ? prev : new Set(prev).add(id)));
        return;
      }
      // The echo of this person's own save: the row already has this version, or the save has not answered yet.
      if (version <= held.version || saving.current.has(id)) return;
      let fresh: AppRecord;
      try {
        fresh = await apiClient.getRecord(slug, id);
      } catch {
        return;
      }
      const cell = editing.current;
      if (cell && cell.recordId === id && changedFields(held.data, fresh.data).includes(cell.fieldName)) {
        if (!heldVersion.current.has(id)) heldVersion.current.set(id, held.version);
        setChangedUnderEdit(cell);
      }
      setRecords((prev) => prev.map((r) => (r.id === id ? fresh : r)));
      mark(id);
    },
    [slug, load, mark]
  );

  const onLiveRef = useRef(onLive);
  onLiveRef.current = onLive;
  useEffect(() => {
    if (!slug) return;
    return subscribe(slug, (e) => void onLiveRef.current(e));
  }, [slug]);

  /** The view says which cell is being typed in, or none. */
  const setEditing = useCallback((cell: EditingCell | null) => {
    editing.current = cell;
    if (!cell) setChangedUnderEdit(null);
  }, []);

  /** Reloads from the first page and clears the count of changes elsewhere. */
  const refresh = useCallback(async () => {
    await load();
  }, [load]);

  const create = useCallback(
    async (data: Record<string, unknown>) => {
      if (!slug) throw new Error("No app is open.");
      const created = await apiClient.createRecord(slug, data);
      setRecords((prev) => [created, ...prev]);
      // Its event may have come first and been counted as a change elsewhere.
      setOutside((prev) => {
        if (!prev.has(created.id)) return prev;
        const next = new Set(prev);
        next.delete(created.id);
        return next;
      });
      return created;
    },
    [slug]
  );

  const save = useCallback(
    async (id: string, data: Record<string, unknown>) => {
      if (!slug) throw new Error("No app is open.");
      const current = records.find((r) => r.id === id);
      if (!current) throw new Error("That record is not loaded.");
      // A draft whose value someone else changed meanwhile is saved on the version it began with.
      const version = heldVersion.current.get(id) ?? current.version;
      heldVersion.current.delete(id);
      setChangedUnderEdit(null);
      saving.current.add(id);
      try {
        const saved = await apiClient.updateRecord(slug, id, version, data);
        setRecords((prev) => prev.map((r) => (r.id === id ? saved : r)));
        setNotice(null);
      } catch (e) {
        if (e instanceof ApiError && e.status === 409) {
          const fresh = await apiClient.getRecord(slug, id);
          setRecords((prev) => prev.map((r) => (r.id === id ? fresh : r)));
          setNotice(sayError(e));
          return;
        }
        throw e;
      } finally {
        saving.current.delete(id);
      }
    },
    [slug, records]
  );

  const remove = useCallback(
    async (id: string) => {
      if (!slug) throw new Error("No app is open.");
      deleting.current.add(id);
      try {
        await apiClient.deleteRecord(slug, id);
      } finally {
        deleting.current.delete(id);
      }
      setRecords((prev) => prev.filter((r) => r.id !== id));
    },
    [slug]
  );

  return {
    records,
    loading,
    error,
    notice,
    clearNotice: () => setNotice(null),
    reload: load,
    create,
    save,
    remove,
    updatedByOthers,
    outsideChanges: outside.size,
    refresh,
    liveNotice,
    clearLiveNotice: () => setLiveNotice(null),
    changedUnderEdit,
    setEditing,
  };
}
