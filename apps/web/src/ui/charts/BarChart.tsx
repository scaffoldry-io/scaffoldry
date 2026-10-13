import { useId } from "react";
import { ChartFrame, DataTable, Legend, PatternDefs, formatNumber, niceTicks, patternFill, useCursor } from "./shared";
import type { Series } from "./types";

export interface BarChartProps {
  title: string;
  categories: string[];
  series: Series[];
  /** What a value counts, for the announcement: `applications`. */
  unit: string;
  /** Stack the series in one bar instead of side by side. */
  stacked?: boolean;
  xLabel?: string;
  yLabel?: string;
}

const W = 640;
const H = 320;
const M = { l: 64, r: 16, t: 20, b: 64 };

/** Bars, grouped or stacked, drawn as SVG: a pattern as well as a color for each series, ticks, value labels, a legend. */
export function BarChart({ title, categories, series, unit, stacked = false, xLabel, yLabel }: BarChartProps) {
  const prefix = useId().replace(/:/g, "");
  const { cursor, onKeyDown } = useCursor(categories.length, series.length);
  const plotW = W - M.l - M.r;
  const plotH = H - M.t - M.b;
  const totals = categories.map((_, c) => series.reduce((sum, s) => sum + (s.values[c] ?? 0), 0));
  const max = stacked ? Math.max(0, ...totals) : Math.max(0, ...series.flatMap((s) => s.values));
  const ticks = niceTicks(0, max);
  const top = ticks[ticks.length - 1];
  const y = (v: number) => M.t + plotH * (1 - v / top);
  const band = categories.length ? plotW / categories.length : plotW;
  const inner = stacked ? band * 0.6 : (band * 0.8) / Math.max(1, series.length);
  const step = Math.ceil(categories.length / 12);

  const announce = cursor
    ? `${categories[cursor.c]}${series.length > 1 ? `, ${series[cursor.s].name}` : ""}: ${formatNumber(series[cursor.s].values[cursor.c] ?? 0)} ${unit}`
    : "";

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
        {categories.map((cat, c) => {
          let stackTop = 0;
          return (
            <g key={cat + c}>
              {c % step === 0 && (
                <text x={M.l + band * c + band / 2} y={M.t + plotH + 16} textAnchor="middle" fontSize="11" fill="currentColor">
                  {cat}
                </text>
              )}
              {series.map((s, i) => {
                const v = s.values[c] ?? 0;
                const x = stacked ? M.l + band * c + (band - inner) / 2 : M.l + band * c + band * 0.1 + inner * i;
                const from = stacked ? stackTop : 0;
                stackTop += v;
                const yTop = y(stacked ? stackTop : v);
                const height = Math.max(0, y(from) - yTop);
                const focused = cursor?.c === c && cursor.s === i;
                return (
                  <g key={s.name}>
                    <rect
                      data-point
                      data-category={c}
                      data-series={i}
                      x={x}
                      y={yTop}
                      width={inner}
                      height={height}
                      fill={patternFill(prefix, i)}
                      stroke={focused ? "currentColor" : "none"}
                      strokeWidth={focused ? 3 : 0}
                    >
                      <title>{`${cat}${series.length > 1 ? `, ${s.name}` : ""}: ${formatNumber(v)} ${unit}`}</title>
                    </rect>
                    {inner >= 20 && (!stacked || height >= 14) && (
                      <text data-value x={x + inner / 2} y={stacked ? yTop + height / 2 : yTop - 4} textAnchor="middle" dominantBaseline={stacked ? "middle" : "auto"} fontSize="11" fontWeight="600" fill="currentColor" stroke={stacked ? "#ffffff" : "none"} strokeWidth={stacked ? 3 : 0} paintOrder="stroke">
                        {formatNumber(v)}
                      </text>
                    )}
                  </g>
                );
              })}
            </g>
          );
        })}
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
