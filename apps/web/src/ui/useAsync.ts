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
      // The failure is in `error`, so a screen shows it. It is not thrown again.
      setError(e);
      return undefined;
    } finally {
      setLoading(false);
    }
  }, [fn]);

  useEffect(() => {
    if (immediate) {
      void reload();
    }
  }, [immediate, reload]);

  return { data, error, loading, reload };
}
