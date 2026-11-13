import { useEffect, useMemo, useState } from "react";
import { apiClient, type AppView, type ViewData } from "../api";

type Mode = "month" | "week" | "day";

const iso = (d: Date) => d.toISOString().slice(0, 10);
const utc = (y: number, m: number, d: number) => new Date(Date.UTC(y, m, d));
const addDays = (d: Date, n: number) => utc(d.getUTCFullYear(), d.getUTCMonth(), d.getUTCDate() + n);
const weekStart = (d: Date) => addDays(d, -d.getUTCDay());

/** The first and last day a view of this kind shows for an anchor date. */
export function rangeOf(mode: Mode, anchor: Date): { from: Date; to: Date } {
  if (mode === "day") return { from: anchor, to: anchor };
  if (mode === "week") return { from: weekStart(anchor), to: addDays(weekStart(anchor), 6) };
  return { from: utc(anchor.getUTCFullYear(), anchor.getUTCMonth(), 1), to: utc(anchor.getUTCFullYear(), anchor.getUTCMonth() + 1, 0) };
}

const heading = (mode: Mode, from: Date, to: Date) => {
  const f = (d: Date, o: Intl.DateTimeFormatOptions) => d.toLocaleDateString("en-US", { timeZone: "UTC", ...o });
  if (mode === "month") return f(from, { month: "long", year: "numeric" });
  if (mode === "week") return `${f(from, { day: "numeric", month: "short" })} – ${f(to, { day: "numeric", month: "short", year: "numeric" })}`;
  return f(from, { weekday: "long", day: "numeric", month: "long", year: "numeric" });
};

/**
 * A calendar over a view's date field. Each range is one request, and the server answers with at most 500 events and how many
 * more the range holds. An event opens the record.
 */
export function CalendarView({ appSlug, view, titleField, onOpenRecord }: { appSlug: string; view: AppView; titleField: string; onOpenRecord?: (id: string) => void }) {
  const [mode, setMode] = useState<Mode>("month");
  const [anchor, setAnchor] = useState<Date>(() => {
    const now = new Date();
    return utc(now.getUTCFullYear(), now.getUTCMonth(), 1);
  });
  const [data, setData] = useState<ViewData | null>(null);
  const [error, setError] = useState<string | null>(null);
  const dateField = String(view.config?.calendar_date_field ?? "");
  const { from, to } = useMemo(() => rangeOf(mode, anchor), [mode, anchor]);
  const [fromKey, toKey] = [iso(from), iso(to)];

  useEffect(() => {
    let live = true;
    setError(null);
    apiClient
      .runView(appSlug, view.id, { from: fromKey, to: toKey })
      .then((d) => live && setData(d))
      .catch((e: unknown) => live && setError(e instanceof Error ? e.message : "The calendar could not be loaded."));
    return () => {
      live = false;
    };
  }, [appSlug, view.id, fromKey, toKey]);

  const step = (n: number) => setAnchor((a) => (mode === "month" ? utc(a.getUTCFullYear(), a.getUTCMonth() + n, 1) : addDays(a, n * (mode === "week" ? 7 : 1))));
  // Changing the mode keeps the range's first day as the anchor, so a week or a day is inside the month that was open.
  const choose = (m: Mode) => {
    setMode(m);
    setAnchor(rangeOf(mode, anchor).from);
  };

  // A month shows whole weeks: from the Sunday on or before its first day to the Saturday on or after its last.
  const days: Date[] = [];
  const [first, last] = mode === "month" ? [weekStart(from), addDays(weekStart(to), 6)] : [from, to];
  for (let d = first; d <= last; d = addDays(d, 1)) days.push(d);
  const events = (data?.records ?? []).filter((r) => typeof r.data[dateField] === "string");
  const on = (d: Date) => events.filter((r) => String(r.data[dateField]).slice(0, 10) === iso(d));
  const more = data?.calendar?.more ?? 0;

  return (
    <div className="space-y-2">
      <div className="flex flex-wrap items-center gap-2">
        <button type="button" onClick={() => step(-1)} className="rounded border px-2 py-1 text-sm">Previous</button>
        <button type="button" onClick={() => step(1)} className="rounded border px-2 py-1 text-sm">Next</button>
        <h2 className="text-base font-semibold">{heading(mode, from, to)}</h2>
        <div className="ml-auto flex gap-1">
          {(["month", "week", "day"] as const).map((m) => (
            <button key={m} type="button" aria-pressed={mode === m} onClick={() => choose(m)} className="rounded border px-2 py-1 text-sm aria-pressed:font-semibold">
              {m[0].toUpperCase() + m.slice(1)}
            </button>
          ))}
        </div>
      </div>
      {error && <p role="alert" className="text-sm text-red-700">{error}</p>}
      <div className={mode === "day" ? "space-y-1" : "grid grid-cols-7 gap-1"}>
        {days.map((d) => (
          <div key={iso(d)} role="group" aria-label={d.toLocaleDateString("en-US", { timeZone: "UTC", weekday: "long", month: "long", day: "numeric" })} className={`min-h-16 rounded border p-1 text-xs ${d < from || d > to ? "opacity-40" : ""}`}>
            <div className="font-medium">{d.getUTCDate()}</div>
            {on(d).map((r) => (
              <button key={r.id} type="button" onClick={() => onOpenRecord?.(r.id)} className="block w-full truncate text-left underline">
                {String(r.data[titleField] ?? "")}
              </button>
            ))}
          </div>
        ))}
      </div>
      {more > 0 && <p className="text-sm text-slate-700">{`+${more.toLocaleString("en-US")} more`}</p>}
    </div>
  );
}
