import { useCallback, useEffect, useState } from "react";
import { apiClient, ApiError, type AppRecord } from "./api";

/** The text shown when a save lost to someone else's. */
export const CONFLICT_NOTICE = "Someone else saved this record. Reloaded.";

/**
 * The stored rows of one app. Nothing is invented: before the load finishes, or when it fails, there are no rows.
 * A save names the version it read. On a conflict the row is reloaded and the typed value is not sent again.
 */
export function useAppRecords(slug: string | null) {
  const [records, setRecords] = useState<AppRecord[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);

  const load = useCallback(async () => {
    if (!slug) {
      setRecords([]);
      return;
    }
    setLoading(true);
    setError(null);
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

  const create = useCallback(
    async (data: Record<string, unknown>) => {
      if (!slug) throw new Error("No app is open.");
      const created = await apiClient.createRecord(slug, data);
      setRecords((prev) => [created, ...prev]);
      return created;
    },
    [slug]
  );

  const save = useCallback(
    async (id: string, data: Record<string, unknown>) => {
      if (!slug) throw new Error("No app is open.");
      const current = records.find((r) => r.id === id);
      if (!current) throw new Error("That record is not loaded.");
      try {
        const saved = await apiClient.updateRecord(slug, id, current.version, data);
        setRecords((prev) => prev.map((r) => (r.id === id ? saved : r)));
        setNotice(null);
      } catch (e) {
        if (e instanceof ApiError && e.status === 409) {
          const fresh = await apiClient.getRecord(slug, id);
          setRecords((prev) => prev.map((r) => (r.id === id ? fresh : r)));
          setNotice(CONFLICT_NOTICE);
          return;
        }
        throw e;
      }
    },
    [slug, records]
  );

  const remove = useCallback(
    async (id: string) => {
      if (!slug) throw new Error("No app is open.");
      await apiClient.deleteRecord(slug, id);
      setRecords((prev) => prev.filter((r) => r.id !== id));
    },
    [slug]
  );

  return { records, loading, error, notice, clearNotice: () => setNotice(null), reload: load, create, save, remove };
}
