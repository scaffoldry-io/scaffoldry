import { useId } from "react";
import { ChartFrame, COLORS, DataTable, Legend, PatternDefs, formatNumber, niceTicks, patternFill, useCursor } from "./shared";
import type { Series } from "./types";

export interface LineChartProps {
  title: string;
  /** The date buckets, in order: `January`, `February`. */
  categories: string[];
  series: Series[];
  unit: string;
  /** Fill under each line. */
  area?: boolean;
  xLabel?: string;
  yLabel?: string;
}

const W = 640;
const H = 320;
const M = { l: 64, r: 24, t: 20, b: 64 };
/** Each series has its own dash as well as its own color. */
const DASHES = ["1 0", "6 4", "2 3", "10 3 2 3", "1 4", "8 2"];

/** A line, or an area, for one or more series over a date bucket. Points are real elements the keyboard can reach. */
export function LineChart({ title, categories, series, unit, area = false, xLabel, yLabel }: LineChartProps) {
  const prefix = useId().replace(/:/g, "");
  const { cursor, onKeyDown } = useCursor(categories.length, series.length);
  const plotW = W - M.l - M.r;
  const plotH = H - M.t - M.b;
  const ticks = niceTicks(0, Math.max(0, ...series.flatMap((s) => s.values)));
  const top = ticks[ticks.length - 1];
  const y = (v: number) => M.t + plotH * (1 - v / top);
  const x = (c: number) => M.l + (categories.length > 1 ? (plotW * c) / (categories.length - 1) : plotW / 2);
  const step = Math.ceil(categories.length / 12);
  const announce = cursor
    ? `${categories[cursor.c]}${series.length > 1 ? `, ${series[cursor.s].name}` : ""}: ${formatNumber(series[cursor.s].values[cursor.c] ?? 0)} ${unit}`
    : "";
  const path = (s: Series, close: boolean) => {
    const pts = categories.map((_, c) => `${x(c)},${y(s.values[c] ?? 0)}`);
    const line = pts.map((p, i) => `${i === 0 ? "M" : "L"}${p}`).join(" ");
    return close ? `${line} L${x(categories.length - 1)},${y(0)} L${x(0)},${y(0)} Z` : line;
  };
  return (
    <ChartFrame
      title={title}
      empty={categories.length === 0}
      announcement={announce}
      onKeyDown={onKeyDown}
      legend={<Legend prefix={prefix} items={series.map((s) => ({ label: s.name }))} />}
      table={<DataTable title={title} head={[xLabel ?? "Category", ...series.map((s) => s.name)]} rows={categories.map((cat, c) => [cat, ...series.map((s) => s.values[c] ?? 0)])} />}
    >
      <svg viewBox={`0 0 ${W} ${H}`} width="100%" role="presentation" className="max-w-full">
        <PatternDefs prefix={prefix} count={series.length} />
        {ticks.map((t) => (
          <g key={t} data-tick>
            <line x1={M.l} x2={W - M.r} y1={y(t)} y2={y(t)} stroke="currentColor" opacity={0.15} />
            <text x={M.l - 6} y={y(t)} textAnchor="end" dominantBaseline="middle" fontSize="11" fill="currentColor">
              {formatNumber(t)}
            </text>
          </g>
        ))}
        <line x1={M.l} x2={M.l} y1={M.t} y2={M.t + plotH} stroke="currentColor" />
        <line x1={M.l} x2={W - M.r} y1={M.t + plotH} y2={M.t + plotH} stroke="currentColor" />
        {categories.map((cat, c) =>
          c % step === 0 ? (
            <text key={cat + c} x={x(c)} y={M.t + plotH + 16} textAnchor="middle" fontSize="11" fill="currentColor">
              {cat}
            </text>
          ) : null
        )}
        {series.map((s, i) => (
          <g key={s.name}>
            {area && <path data-area d={path(s, true)} fill={patternFill(prefix, i)} opacity={0.3} stroke="none" />}
            <path data-line d={path(s, false)} fill="none" stroke={COLORS[i % COLORS.length]} strokeWidth={2.5} strokeDasharray={DASHES[i % DASHES.length]} />
            {categories.map((cat, c) => {
              const v = s.values[c] ?? 0;
              const focused = cursor?.c === c && cursor.s === i;
              return (
                <g key={cat + c}>
                  <circle data-point data-category={c} data-series={i} cx={x(c)} cy={y(v)} r={focused ? 7 : 4} fill={COLORS[i % COLORS.length]} stroke={focused ? "currentColor" : "#ffffff"} strokeWidth={focused ? 3 : 1.5}>
                    <title>{`${cat}${series.length > 1 ? `, ${s.name}` : ""}: ${formatNumber(v)} ${unit}`}</title>
                  </circle>
                  {series.length === 1 && categories.length <= 12 && (
                    <text data-value x={x(c)} y={y(v) - 10} textAnchor="middle" fontSize="11" fontWeight="600" fill="currentColor">
                      {formatNumber(v)}
                    </text>
                  )}
                </g>
              );
            })}
</g>
        ))}
        {xLabel && (
          <text x={M.l + plotW / 2} y={H - 12} textAnchor="middle" fontSize="12" fill="currentColor">
            {xLabel}
          </text>
        )}
        {yLabel && (
          <text transform={`translate(14 ${M.t + plotH / 2}) rotate(-90)`} textAnchor="middle" fontSize="12" fill="currentColor">
            {yLabel}
          </text>
        )}
      </svg>
    </ChartFrame>
  );
}
