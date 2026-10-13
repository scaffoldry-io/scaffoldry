import { useId, useState } from "react";
import type { KeyboardEvent, ReactNode } from "react";
import { EmptyState } from "../EmptyState";

/** Colors that stay apart for the common kinds of color blindness. Color is never the only signal: see `PatternDefs`. */
export const COLORS = ["#0072B2", "#E69F00", "#009E73", "#D55E00", "#CC79A7", "#56B4E9", "#7a6f00", "#6b6b6b"];

export const formatNumber = (n: number): string => n.toLocaleString("en-US", { maximumFractionDigits: 2 });

/** Round tick values from `min` that cover `max`, about five intervals apart. */
export function niceTicks(min: number, max: number): number[] {
  if (!(max > min)) return [min, min + 1];
  const raw = (max - min) / 5;
  const exp = Math.floor(Math.log10(raw));
  const f = raw / 10 ** exp;
  const nice = f <= 1 ? 1 : f <= 2 ? 2 : f <= 5 ? 5 : 10;
  const step = nice * 10 ** exp;
  const ticks: number[] = [];
  for (let v = min; v < max + step * 0.999; v += step) ticks.push(Number(v.toFixed(10)));
  return ticks;
}

/** One pattern for each series. A pattern is drawn over the series' color, so two series differ by shape as well as by hue. */
export function PatternDefs({ prefix, count }: { prefix: string; count: number }) {
  const tile = (i: number) => {
    const c = COLORS[i % COLORS.length];
    const base = <rect width="8" height="8" fill={c} />;
    const ink = { stroke: "#ffffff", strokeWidth: 1.5, opacity: 0.85 } as const;
    switch (i % 6) {
      case 0:
        return base; // solid
      case 1:
        return (<>{base}<path d="M-1,1 l2,-2 M0,8 l8,-8 M7,9 l2,-2" {...ink} /></>); // diagonal
      case 2:
        return (<>{base}<circle cx="4" cy="4" r="1.6" fill="#ffffff" opacity="0.85" /></>); // dots
      case 3:
        return (<>{base}<path d="M0,2 H8 M0,6 H8" {...ink} /></>); // horizontal
      case 4:
        return (<>{base}<path d="M2,0 V8 M6,0 V8" {...ink} /></>); // vertical
      default:
        return (<>{base}<path d="M0,0 L8,8 M8,0 L0,8" {...ink} /></>); // crosshatch
    }
  };
  return (
    <defs>
      {Array.from({ length: count }, (_, i) => (
        <pattern key={i} id={`${prefix}-p${i}`} width="8" height="8" patternUnits="userSpaceOnUse">
          {tile(i)}
        </pattern>
      ))}
    </defs>
  );
}

export const patternFill = (prefix: string, i: number) => `url(#${prefix}-p${i})`;

/** The point the arrow keys have reached, as a category and a series. */
export interface Cursor {
  c: number;
  s: number;
}

/**
 * Arrow keys over a grid of points (categories across, series down). With `list` the points are one run, as in a pie, and
 * Up and Down go back and forward like Left and Right.
 */
export function useCursor(categories: number, seriesCount: number, list = false) {
  const [cursor, setCursor] = useState<Cursor | null>(null);
  const onKeyDown = (e: KeyboardEvent) => {
    const at = cursor ?? { c: 0, s: 0 };
    let next: Cursor | null = null;
    const key = list ? ({ ArrowDown: "ArrowRight", ArrowUp: "ArrowLeft" } as Record<string, string>)[e.key] ?? e.key : e.key;
    switch (key) {
      case "ArrowRight":
        next = cursor ? { ...at, c: Math.min(categories - 1, at.c + 1) } : at;
        break;
      case "ArrowLeft":
        next = cursor ? { ...at, c: Math.max(0, at.c - 1) } : at;
        break;
      case "ArrowDown":
        next = cursor ? { ...at, s: Math.min(seriesCount - 1, at.s + 1) } : at;
        break;
      case "ArrowUp":
        next = cursor ? { ...at, s: Math.max(0, at.s - 1) } : at;
        break;
      case "Home":
        next = { ...at, c: 0 };
        break;
      case "End":
        next = { ...at, c: categories - 1 };
        break;
      default:
        return;
    }
    e.preventDefault();
    setCursor(next);
  };
  return { cursor, onKeyDown };
}

export function Legend({ items, prefix }: { items: { label: string; detail?: string }[]; prefix: string }) {
  return (
    <ul role="list" aria-label="Legend" className="mt-2 flex flex-wrap gap-x-4 gap-y-1 text-sm">
      {items.map((it, i) => (
        <li key={it.label} className="inline-flex items-center gap-1.5">
          <svg width="16" height="16" aria-hidden="true">
            <PatternDefs prefix={`${prefix}-lg${i}`} count={i + 1} />
            <rect width="16" height="16" rx="2" fill={patternFill(`${prefix}-lg${i}`, i)} />
          </svg>
          <span>{it.label}</span>
          {it.detail && <span className="text-gray-600 dark:text-gray-400">{it.detail}</span>}
        </li>
      ))}
    </ul>
  );
}

/** The exact data as a table. */
export function DataTable({ title, head, rows }: { title: string; head: string[]; rows: (string | number)[][] }) {
  return (
    <table aria-label={title} className="mt-2 w-full text-sm border-collapse">
      <thead>
        <tr>
          {head.map((h) => (
            <th key={h} scope="col" className="text-left border-b px-2 py-1">
              {h}
            </th>
          ))}
        </tr>
      </thead>
      <tbody>
        {rows.map((r, i) => (
          <tr key={i}>
            {r.map((cell, j) => (
              <td key={j} className="border-b px-2 py-1">
                {typeof cell === "number" ? formatNumber(cell) : cell}
              </td>
            ))}
          </tr>
        ))}
      </tbody>
    </table>
  );
}

/**
 * What every chart shares: a name, the chart or the table of its exact data, a live region that says what the arrow keys
 * reached, and an empty state when there is nothing to draw.
 */
export function ChartFrame({
  title,
  empty,
  announcement,
  table,
  onKeyDown,
  children,
  legend,
}: {
  title: string;
  empty: boolean;
  announcement: string;
  table: ReactNode;
  onKeyDown: (e: KeyboardEvent) => void;
  children: ReactNode;
  legend: ReactNode;
}) {
  const [asTable, setAsTable] = useState(false);
  const id = useId();
  if (empty) return <EmptyState title="Nothing to chart yet" description={`There is no data for ${title.toLowerCase()}.`} />;
  return (
    <figure className="m-0">
      <figcaption className="flex items-center justify-between gap-2">
        <span className="font-semibold">{title}</span>
        <button type="button" onClick={() => setAsTable((v) => !v)} className="text-sm underline" aria-controls={id}>
          {asTable ? "View as chart" : "View as table"}
        </button>
      </figcaption>
      <div id={id}>
        {asTable ? (
          table
        ) : (
          <div role="group" aria-label={`${title}. Use the arrow keys to move between points.`} tabIndex={0} onKeyDown={onKeyDown} className="outline-offset-2">
            {children}
          </div>
        )}
      </div>
      {!asTable && legend}
      <div role="status" aria-live="polite" className="sr-only">
        {announcement}
      </div>
    </figure>
  );
}
