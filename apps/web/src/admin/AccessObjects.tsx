import { useEffect, useState } from "react";
import { apiClient } from "../api";
import { ErrorState } from "../ui";

const inputClass =
  "w-full text-sm rounded border border-gray-300 dark:border-gray-700 p-2 bg-white dark:bg-gray-800 text-gray-900 dark:text-gray-100";

type ObjectRow = { container: string; kind: string; title: string };
type Held = { grant_id?: number | null; holder: string; role: string; source?: string };

export function AccessObjects() {
  const [q, setQ] = useState("");
  const [rows, setRows] = useState<ObjectRow[]>([]);
  const [more, setMore] = useState<string | null>(null);
  const [chosen, setChosen] = useState<ObjectRow | null>(null);
  const [review, setReview] = useState<Record<string, unknown> | null>(null);
  const [error, setError] = useState<unknown>(null);

  useEffect(() => {
    let live = true;
    const t = setTimeout(() => {
      apiClient
        .accessSearchObjects(q.trim())
        .then((r) => {
          if (!live) return;
          setRows(r.objects);
          setMore(r.next_cursor);
        })
        .catch((e) => live && setError(e));
    }, 150);
    return () => {
      live = false;
      clearTimeout(t);
    };
  }, [q]);

  useEffect(() => {
    if (!chosen) return;
    let live = true;
    setReview(null);
    apiClient
      .accessReviewObject(chosen.container)
      .then((r) => live && setReview(r))
      .catch((e) => live && setError(e));
    return () => {
      live = false;
    };
  }, [chosen]);

  const loadMore = async () => {
    if (!more) return;
    const r = await apiClient.accessSearchObjects(q.trim(), more);
    setRows((p) => [...p, ...r.objects]);
    setMore(r.next_cursor);
  };

  const sealed = Array.isArray(review?.sealed) ? (review?.sealed as string[]) : [];
  const grants = Array.isArray(review?.grants) ? (review?.grants as Held[]) : [];

  return (
    <div className="space-y-6" data-testid="access-objects">
      <label className="block text-sm font-medium text-gray-700 dark:text-gray-300">
        Search objects
        <input type="search" className={`${inputClass} mt-1 max-w-sm`} value={q} onChange={(e) => setQ(e.target.value)} placeholder="A workspace, an app or a table" />
      </label>
      {error !== null && <ErrorState error={error} />}
      <ul className="space-y-1">
        {rows.map((o) => (
          <li key={o.container}>
            <button type="button" onClick={() => setChosen(o)} className="text-left text-sm px-2 py-1 rounded hover:bg-gray-100 dark:hover:bg-gray-800">
              {o.title} <span className="text-gray-500">{o.kind}</span>
            </button>
          </li>
        ))}
        {more && (
          <li>
            <button type="button" onClick={() => void loadMore()} className="text-xs text-blue-600 hover:underline">
              Show more
            </button>
          </li>
        )}
      </ul>
      {chosen && (
        <section aria-label={`Access to ${chosen.title}`} className="space-y-3">
          <h3 className="text-sm font-semibold text-gray-900 dark:text-gray-100">
            {chosen.title} <span className="font-normal text-gray-500">{chosen.container}</span>
          </h3>
          {sealed.length > 0 ? (
            <p className="text-sm text-amber-700 dark:text-amber-400">Sealed: grants above {sealed.join(", ")} do not reach it.</p>
          ) : (
            <p className="text-sm text-gray-600 dark:text-gray-400">Not sealed.</p>
          )}
          {review ? (
            grants.length === 0 ? (
              <p className="text-sm text-gray-600 dark:text-gray-400">No grant reaches this object.</p>
            ) : (
              <ul className="space-y-1 text-sm">
                {grants.map((g, i) => (
                  <li key={`${g.grant_id ?? i}`} className="text-gray-900 dark:text-gray-100">
                    {g.holder} as {g.role}
                    {g.grant_id ? (
                      <>
                        {" "}
                        <a className="text-blue-600 hover:underline text-xs" href={`/admin/access/grants/${g.grant_id}`}>
                          Grant {g.grant_id}
                        </a>
                      </>
                    ) : null}
                  </li>
                ))}
              </ul>
            )
          ) : (
            <p className="text-sm text-gray-500">Reading…</p>
          )}
        </section>
      )}
    </div>
  );
}
