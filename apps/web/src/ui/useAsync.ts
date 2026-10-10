import { useState, useEffect, useCallback } from "react";

export function useAsync<T>(fn: () => Promise<T>, immediate = true) {
  const [data, setData] = useState<T | null>(null);
  const [error, setError] = useState<Error | null>(null);
  const [loading, setLoading] = useState<boolean>(immediate);

  const reload = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      const result = await fn();
      setData(result);
      return result;
    } catch (err: unknown) {
      const e = err instanceof Error ? err : new Error(String(err));
      setError(e);
      throw e;
    } finally {
      setLoading(false);
    }
  }, [fn]);

  useEffect(() => {
    if (immediate) {
      reload().catch(() => {});
    }
  }, [immediate, reload]);

  return { data, error, loading, reload };
}
