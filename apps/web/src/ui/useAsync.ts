import { useState, useEffect, useCallback, useRef } from "react";

/**
 * Run an async function and track its result.
 *
 * `reload` is stable. It always calls the latest `fn`, so an inline arrow is safe.
 * The first fetch runs once per mount. Pass `deps` to fetch again when inputs change.
 */
export function useAsync<T>(fn: () => Promise<T>, immediate = true, deps: readonly unknown[] = []) {
  const fnRef = useRef(fn);
  fnRef.current = fn;
  const [data, setData] = useState<T | null>(null);
  const [error, setError] = useState<Error | null>(null);
  const [loading, setLoading] = useState<boolean>(immediate);

  const reload = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      const result = await fnRef.current();
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
  }, []);

  useEffect(() => {
    if (immediate) {
      void reload();
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [immediate, reload, ...deps]);

  return { data, error, loading, reload };
}
