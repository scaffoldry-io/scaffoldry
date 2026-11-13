import { useCallback, useEffect, useState, type KeyboardEvent } from "react";
import { apiClient, type AppView } from "../api";

interface Card {
  id: string;
  version: number;
  data: Record<string, unknown>;
}

interface Column {
  count: number;
  cards: Card[];
  next: string | null;
}

/** Cards fetched for a column at a time. */
export const PAGE = 50;

const text = (v: unknown) => (v === null || v === undefined ? "" : typeof v === "object" ? JSON.stringify(v) : String(v));

/**
 * A board: one column for each option of a single select. The server groups the rows, so each column's count is the
 * aggregate's, and it pages the cards of a column, fifty at a time. A move updates the record with the version the card was
 * read at. With Alt and the left or right arrow, a focused card moves to the next column, and the move is announced.
 */
export function BoardView({
  appSlug,
  view,
  columnField,
  titleField,
}: {
  appSlug: string;
  view: AppView;
  columnField: { name: string; options: { id: string; label: string }[] };
  titleField?: string;
}) {
  const [columns, setColumns] = useState<Record<string, Column> | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [said, setSaid] = useState("");
  const limits = (view.config?.column_limits ?? {}) as Record<string, number>;

  useEffect(() => {
    let live = true;
    (async () => {
      try {
        const groups = await apiClient.runView(appSlug, view.id);
        const counts = new Map((groups.groups?.items ?? []).map((g) => [g.key, g.count]));
        const entries = await Promise.all(
          columnField.options.map(async (o) => {
            const count = counts.get(o.id) ?? 0;
            if (count === 0) return [o.id, { count, cards: [], next: null }] as const;
            const page = await apiClient.runView(appSlug, view.id, { groupPath: [o.id], limit: PAGE });
            return [o.id, { count, cards: (page.records ?? []) as unknown as Card[], next: page.next_cursor ?? null }] as const;
          })
        );
        if (live) setColumns(Object.fromEntries(entries));
      } catch (e) {
        if (live) setError(e instanceof Error ? e.message : "The board could not be loaded.");
      }
    })();
    return () => {
      live = false;
    };
  }, [appSlug, view.id, columnField]);

  const more = useCallback(
    async (id: string, cursor: string) => {
      try {
        const page = await apiClient.runView(appSlug, view.id, { groupPath: [id], limit: PAGE, cursor });
        setColumns((cols) => (cols ? { ...cols, [id]: { ...cols[id], cards: [...cols[id].cards, ...((page.records ?? []) as unknown as Card[])], next: page.next_cursor ?? null } } : cols));
      } catch (e) {
        setError(e instanceof Error ? e.message : "More cards could not be loaded.");
      }
    },
    [appSlug, view.id]
  );

  const move = async (card: Card, from: string, to: string) => {
    if (!columns) return;
    const label = columnField.options.find((o) => o.id === to)?.label ?? to;
    const taken = (cols: Record<string, Column>, id: string, f: (c: Column) => Column) => ({ ...cols, [id]: f(cols[id]) });
    setError(null);
    // The card moves at once, and goes back if the server refuses.
    setColumns((cols) => {
      if (!cols) return cols;
      let next = taken(cols, from, (c) => ({ ...c, count: c.count - 1, cards: c.cards.filter((x) => x.id !== card.id) }));
      next = taken(next, to, (c) => ({ ...c, count: c.count + 1, cards: [{ ...card, data: { ...card.data, [columnField.name]: to } }, ...c.cards] }));
      return next;
    });
    try {
      const { _table_id: _ignored, ...data } = card.data;
      void _ignored;
      const saved = await apiClient.updateRecord(appSlug, card.id, card.version, { ...data, [columnField.name]: to });
      setColumns((cols) => (cols ? taken(cols, to, (c) => ({ ...c, cards: c.cards.map((x) => (x.id === card.id ? { ...x, version: saved.version } : x)) })) : cols));
      setSaid(`Moved to ${label}.`);
    } catch (e) {
      setColumns((cols) => {
        if (!cols) return cols;
        let back = taken(cols, to, (c) => ({ ...c, count: c.count - 1, cards: c.cards.filter((x) => x.id !== card.id) }));
        back = taken(back, from, (c) => ({ ...c, count: c.count + 1, cards: [card, ...c.cards] }));
        return back;
      });
      setError(e instanceof Error ? e.message : "The card could not be moved.");
    }
  };

  const onKey = (e: KeyboardEvent, card: Card, id: string) => {
    if (!e.altKey || (e.key !== "ArrowLeft" && e.key !== "ArrowRight")) return;
    e.preventDefault();
    const at = columnField.options.findIndex((o) => o.id === id);
    const target = columnField.options[at + (e.key === "ArrowRight" ? 1 : -1)];
    if (target) void move(card, id, target.id);
  };

  if (error && !columns) return <p role="alert" className="text-sm text-red-700">{error}</p>;
  if (!columns) return <p className="text-sm text-slate-500">Loading…</p>;

  return (
    <div className="space-y-2">
      {error && <p role="alert" className="text-sm text-red-700">{error}</p>}
      <p role="status" aria-live="polite" className="sr-only">{said}</p>
      <div className="flex gap-3 overflow-x-auto">
        {columnField.options.map((o) => {
          const col = columns[o.id];
          const limit = limits[o.id];
          return (
            <section key={o.id} aria-labelledby={`${view.id}-${o.id}`} className="w-64 shrink-0 rounded border p-2">
              <h2 id={`${view.id}-${o.id}`} className="mb-1 flex items-baseline justify-between text-sm font-semibold">
                <span>{o.label}</span>
                <span className="tabular-nums">{col.count.toLocaleString("en-US")}</span>
              </h2>
              {limit !== undefined && col.count > limit && <p className="mb-1 text-xs text-amber-800">{`Over its limit of ${limit}`}</p>}
              <ul className="space-y-1">
                {col.cards.map((c) => (
                  <li key={c.id} tabIndex={0} onKeyDown={(e) => onKey(e, c, o.id)} className="rounded border bg-white p-2 text-sm dark:bg-gray-900">
                    {text(c.data[titleField ?? Object.keys(c.data).find((k) => !k.startsWith("_") && k !== columnField.name) ?? ""])}
                  </li>
                ))}
              </ul>
              {col.next && (
                <button type="button" onClick={() => void more(o.id, col.next as string)} className="mt-1 text-xs underline">
                  Load more
                </button>
              )}
            </section>
          );
        })}
      </div>
    </div>
  );
}
