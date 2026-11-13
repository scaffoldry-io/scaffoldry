import { useEffect, useMemo, useState } from "react";
import type { KeyboardEvent } from "react";
import { apiClient, type AppView, type DashboardWidgetData } from "./api";
import { PageFrame } from "./PageFrame";
import { Markdown } from "./ui/Markdown";
import { BarChart, StatChart } from "./ui/charts";

/** What a widget shows. A chart draws a summary view. A page is a custom page the app already has approved. */
export type WidgetBody =
  | { type: "view"; view_id: string }
  | { type: "chart"; view_id: string }
  | { type: "stat"; table_id: string; fn: string; field?: string }
  | { type: "text"; markdown: string }
  | { type: "page"; page_id: string };

/** Selecting a row in `source_widget` filters this widget by `target_field = <that row's source_field>`. The server does it. */
export interface SelectBy {
  source_widget: string;
  source_field: string;
  target_field: string;
}

export interface DashboardWidget {
  id: string;
  title: string;
  x: number;
  y: number;
  w: number;
  h: number;
  body: WidgetBody;
  select_by?: SelectBy;
}

/** The grid is 12 columns wide. Below this width in pixels the widgets stack in reading order. */
export const COLUMNS = 12;
export const STACK_BELOW = 900;
const MAX_ROWS = 200;

type Place = Pick<DashboardWidget, "x" | "y" | "w" | "h">;
type Selected = { id: string; data: Record<string, unknown> };

/** Whether the widget fits the grid and clears the others. Says why not, in words. */
export function problemWith(next: DashboardWidget, all: DashboardWidget[]): string | null {
  const name = `"${next.title}"`;
  if (next.x < 0) return `${name} would run past the left edge`;
  if (next.y < 0) return `${name} would run past the top`;
  if (next.w < 1 || next.h < 1) return `${name} needs a width and a height of at least 1`;
  if (next.x + next.w > COLUMNS) return `${name} would run past the right edge. The grid is ${COLUMNS} columns wide`;
  if (next.y + next.h > MAX_ROWS) return `${name} would run past the bottom`;
  const hit = all.find((o) => o.id !== next.id && !(next.x + next.w <= o.x || o.x + o.w <= next.x || next.y + next.h <= o.y || o.y + o.h <= next.y));
  return hit ? `${name} would overlap "${hit.title}"` : null;
}

/** Reading order: top to bottom, then left to right. */
const reading = (a: Place, b: Place) => a.y - b.y || a.x - b.x;

function useWide(): boolean {
  const [wide, setWide] = useState(() => window.innerWidth >= STACK_BELOW);
  useEffect(() => {
    const on = () => setWide(window.innerWidth >= STACK_BELOW);
    window.addEventListener("resize", on);
    return () => window.removeEventListener("resize", on);
  }, []);
  return wide;
}

const widgetsOf = (view: AppView): DashboardWidget[] => (Array.isArray(view.config?.widgets) ? (view.config.widgets as DashboardWidget[]) : []);

const cellText = (v: unknown): string => (v === null || v === undefined ? "" : typeof v === "object" ? JSON.stringify(v) : String(v));

/** One widget's data from the server. A text or page widget has none to fetch. */
function useWidgetData(appSlug: string, dashboard: string, widget: DashboardWidget, select: string | undefined) {
  const [data, setData] = useState<DashboardWidgetData | null>(null);
  const [error, setError] = useState<string | null>(null);
  const needsData = widget.body.type === "view" || widget.body.type === "chart" || widget.body.type === "stat";
  useEffect(() => {
    if (!needsData) return;
    let live = true;
    setError(null);
    apiClient
      .dashboardWidget(appSlug, dashboard, widget.id, select === undefined ? {} : { select })
      .then((d) => live && setData(d))
      .catch((e: unknown) => live && setError(e instanceof Error ? e.message : "This widget could not be loaded."));
    return () => {
      live = false;
    };
  }, [appSlug, dashboard, widget.id, select, needsData]);
  return { data, error, loading: needsData && data === null && error === null };
}

interface BodyProps {
  appSlug: string;
  dashboard: string;
  widget: DashboardWidget;
  select: string | undefined;
  /** The record selected in the widget this one is linked to. */
  linkedId: string | null;
  selected: Selected | undefined;
  onSelect: (row: Selected | null) => void;
}

function WidgetBodyView({ appSlug, dashboard, widget, select, linkedId, selected, onSelect }: BodyProps) {
  const { data, error, loading } = useWidgetData(appSlug, dashboard, widget, select);
  const body = widget.body;
  if (body.type === "text") return <Markdown content={body.markdown} />;
  if (body.type === "page") return <PageFrame appSlug={appSlug} pageId={body.page_id} selection={linkedId} />;
  if (error) return <p role="alert" className="text-sm text-red-700">{error}</p>;
  if (loading || !data) return <p className="text-sm text-slate-500">Loading…</p>;

  const filtered = data.selected ? <p className="mb-1 text-xs text-slate-600">{`Only ${data.selected.field} = ${data.selected.value}`}</p> : null;

  if (body.type === "stat") return <StatChart label={widget.title} value={data.data.stat?.value ?? 0} unit="" />;

  if (body.type === "chart" || data.data.summary) {
    const summary = data.data.summary;
    if (!summary || summary.groups.length === 0) return <>{filtered}<p className="text-sm text-slate-600">No data for this chart.</p></>;
    const measures = summary.groups[0].values.length;
    const names = summary.columns.slice(summary.columns.length - measures);
    return (
      <>
        {filtered}
        <BarChart
          title={widget.title}
          categories={summary.groups.map((g) => g.key.map((k) => k || "(blank)").join(" / "))}
          series={names.map((name, i) => ({ name, values: summary.groups.map((g) => g.values[i] ?? 0) }))}
          unit="records"
        />
      </>
    );
  }

  const records = data.data.records ?? [];
  if (records.length === 0) return <>{filtered}<p className="text-sm text-slate-600">No rows.</p></>;
  const columns = Object.keys(records[0].data).filter((k) => !k.startsWith("_"));
  return (
    <>
      {filtered}
      <table className="w-full text-sm">
        <thead>
          <tr>
            {columns.map((c) => (
              <th key={c} scope="col" className="border-b px-2 py-1 text-left">{c}</th>
            ))}
          </tr>
        </thead>
        <tbody>
          {records.map((r, i) => {
            const on = selected?.id === r.id;
            return (
              <tr key={r.id} className={on ? "bg-blue-50 dark:bg-blue-950/40" : ""}>
                {columns.map((c, n) => (
                  <td key={c} className="px-2 py-1">
                    {n === 0 ? (
                      <button type="button" aria-label={`Select row ${i + 1} of ${widget.title}`} aria-pressed={on} onClick={() => onSelect(on ? null : { id: r.id, data: r.data })} className="underline">
                        {cellText(r.data[c])}
                      </button>
                    ) : (
                      cellText(r.data[c])
                    )}
                  </td>
                ))}
              </tr>
            );
          })}
        </tbody>
      </table>
      {selected && (
        <button type="button" aria-label={`Clear selection in ${widget.title}`} onClick={() => onSelect(null)} className="mt-1 text-xs underline">
          Clear selection
        </button>
      )}
    </>
  );
}

/**
 * A dashboard: widgets on a 12-column grid. Selecting a row in a source widget asks the server for each linked widget again,
 * with the selected value. Nothing is filtered here. Below 900 px the widgets stack in reading order. With `canEdit`, the layout
 * can be changed: arrow keys move a widget, Shift with the arrows resizes it, and four numbers set it exactly.
 */
export function Dashboard({ appSlug, view, canEdit = false, onSaved }: { appSlug: string; view: AppView; canEdit?: boolean; onSaved?: (view: AppView) => void }) {
  const saved = useMemo(() => widgetsOf(view), [view]);
  const [widgets, setWidgets] = useState<DashboardWidget[]>(saved);
  const [editing, setEditing] = useState(false);
  const [active, setActive] = useState<string | null>(null);
  const [note, setNote] = useState("");
  const [failure, setFailure] = useState<string | null>(null);
  const [selected, setSelected] = useState<Record<string, Selected>>({});
  const wide = useWide();

  useEffect(() => setWidgets(saved), [saved]);

  const ordered = useMemo(() => [...widgets].sort(reading), [widgets]);
  const current = widgets.find((w) => w.id === active) ?? null;

  const place = (next: DashboardWidget): boolean => {
    const problem = problemWith(next, widgets);
    setNote(problem ?? "");
    if (problem) return false;
    setWidgets((all) => all.map((w) => (w.id === next.id ? next : w)));
    return true;
  };

  const onKey = (e: KeyboardEvent, w: DashboardWidget) => {
    const dx = e.key === "ArrowLeft" ? -1 : e.key === "ArrowRight" ? 1 : 0;
    const dy = e.key === "ArrowUp" ? -1 : e.key === "ArrowDown" ? 1 : 0;
    if (!dx && !dy) return;
    e.preventDefault();
    place(e.shiftKey ? { ...w, w: w.w + dx, h: w.h + dy } : { ...w, x: w.x + dx, y: w.y + dy });
  };

  const save = async () => {
    setFailure(null);
    try {
      const res = await apiClient.updateView(appSlug, view.id, { name: view.name, config: { ...view.config, columns: COLUMNS, widgets }, version: view.version });
      setEditing(false);
      onSaved?.(res.view);
    } catch (e) {
      setFailure(e instanceof Error ? e.message : "The layout could not be saved.");
    }
  };

  const cancel = () => {
    setWidgets(saved);
    setEditing(false);
    setActive(null);
    setNote("");
    setFailure(null);
  };

  const selectValue = (w: DashboardWidget): string | undefined => {
    const link = w.select_by;
    const row = link ? selected[link.source_widget] : undefined;
    const value = row?.data[link?.source_field ?? ""];
    return value === undefined || value === null ? undefined : String(value);
  };

  return (
    <div className="space-y-3">
      <div className="flex items-center justify-between gap-2">
        <h1 className="text-lg font-semibold">{view.name}</h1>
        {canEdit && !editing && (
          <button type="button" onClick={() => setEditing(true)} className="rounded border px-3 py-1.5 text-sm font-medium">
            Edit layout
          </button>
        )}
        {editing && (
          <div className="flex gap-2">
            <button type="button" onClick={save} className="rounded border px-3 py-1.5 text-sm font-medium">Save layout</button>
            <button type="button" onClick={cancel} className="rounded border px-3 py-1.5 text-sm font-medium">Cancel</button>
          </div>
        )}
      </div>
      {failure && <p role="alert" className="text-sm text-red-700">{failure}</p>}
      {editing && (
        <div className="rounded border p-3 text-sm">
          <p className="mb-2 text-slate-600">Focus a widget, then use the arrow keys to move it and Shift with the arrow keys to resize it. Or set the numbers.</p>
          {current ? (
            <fieldset className="flex flex-wrap gap-3">
              <legend className="font-medium">{`Place and size of ${current.title}`}</legend>
              {([["X", "x"], ["Y", "y"], ["Width", "w"], ["Height", "h"]] as const).map(([label, key]) => (
                <label key={key} className="flex items-center gap-1">
                  {label}
                  <input
                    type="number"
                    value={current[key]}
                    onChange={(e) => {
                      const n = Number.parseInt(e.target.value, 10);
                      if (Number.isFinite(n)) place({ ...current, [key]: n });
                    }}
                    className="w-16 rounded border px-1 py-0.5"
                  />
                </label>
              ))}
            </fieldset>
          ) : (
            <p>No widget is chosen.</p>
          )}
          <p role="status" data-testid="layout-note" className="mt-2 min-h-5 text-amber-800">{note}</p>
        </div>
      )}
      <div
        className={wide ? "" : "flex flex-col gap-3"}
        style={wide ? { display: "grid", gridTemplateColumns: `repeat(${COLUMNS}, minmax(0, 1fr))`, gridAutoRows: "4rem", gap: "0.75rem" } : undefined}
      >
        {ordered.map((w) => {
          const heading = `${view.id}-${w.id}`;
          const link = w.select_by;
          return (
            <section
              key={w.id}
              aria-labelledby={heading}
              style={wide ? { gridColumn: `${w.x + 1} / span ${w.w}`, gridRow: `${w.y + 1} / span ${w.h}` } : undefined}
              className={`overflow-auto rounded border p-2 ${active === w.id && editing ? "ring-2 ring-blue-500" : ""}`}
            >
              <h2 id={heading} className="mb-1 text-sm font-semibold">{w.title}</h2>
              {editing && (
                <div
                  role="group"
                  tabIndex={0}
                  aria-label={`Move or resize ${w.title}`}
                  onFocus={() => setActive(w.id)}
                  onClick={() => setActive(w.id)}
                  onKeyDown={(e) => onKey(e, w)}
                  className="mb-1 cursor-move rounded border border-dashed px-2 py-0.5 text-xs text-slate-600"
                >
                  {`Column ${w.x + 1}, row ${w.y + 1}, ${w.w} wide, ${w.h} high`}
                </div>
              )}
              <WidgetBodyView
                appSlug={appSlug}
                dashboard={view.id}
                widget={w}
                select={selectValue(w)}
                linkedId={link ? (selected[link.source_widget]?.id ?? null) : null}
                selected={selected[w.id]}
                onSelect={(row) =>
                  setSelected((all) => {
                    const next = { ...all };
                    if (row) next[w.id] = row;
                    else delete next[w.id];
                    return next;
                  })
                }
              />
            </section>
          );
        })}
      </div>
    </div>
  );
}
