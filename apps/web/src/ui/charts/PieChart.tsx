import { useId } from "react";
import { ChartFrame, DataTable, Legend, PatternDefs, formatNumber, patternFill, useCursor } from "./shared";

export interface PieChartProps {
  title: string;
  slices: { label: string; value: number }[];
  unit: string;
  /** A ring instead of a disc. */
  donut?: boolean;
  /** Most slices drawn. Past this the smallest groups are added together as `Other`. */
  maxSlices?: number;
}

const SIZE = 320;
const R = 130;

/** The largest groups, and the rest added together as `Other` when there are more than `max`. */
export function topSlices(slices: { label: string; value: number }[], max: number): { label: string; value: number }[] {
  const ranked = slices.filter((s) => s.value > 0).sort((a, b) => b.value - a.value);
  if (ranked.length <= max) return ranked;
  const kept = ranked.slice(0, max - 1);
  const other = ranked.slice(max - 1).reduce((sum, s) => sum + s.value, 0);
  return [...kept, { label: "Other", value: other }];
}

const point = (cx: number, cy: number, r: number, angle: number) => [cx + r * Math.sin(angle), cy - r * Math.cos(angle)];

/** A wedge from angle `a0` to `a1`, clockwise from the top. A whole circle is two half arcs, which an arc cannot draw alone. */
function wedge(cx: number, cy: number, r: number, a0: number, a1: number): string {
  if (a1 - a0 >= Math.PI * 2 - 1e-6) {
    const [x0, y0] = point(cx, cy, r, 0);
    const [x1, y1] = point(cx, cy, r, Math.PI);
    return `M${x0},${y0} A${r},${r} 0 1 1 ${x1},${y1} A${r},${r} 0 1 1 ${x0},${y0} Z`;
  }
  const [x0, y0] = point(cx, cy, r, a0);
  const [x1, y1] = point(cx, cy, r, a1);
  return `M${cx},${cy} L${x0},${y0} A${r},${r} 0 ${a1 - a0 > Math.PI ? 1 : 0} 1 ${x1},${y1} Z`;
}

/** A pie or a donut of one group field. At most 12 slices, the rest as `Other`. A pattern as well as a color for each slice. */
export function PieChart({ title, slices, unit, donut = false, maxSlices = 12 }: PieChartProps) {
  const prefix = useId().replace(/:/g, "");
  const shown = topSlices(slices, maxSlices);
  const total = shown.reduce((sum, s) => sum + s.value, 0);
  const { cursor, onKeyDown } = useCursor(shown.length, 1, true);
  const pct = (v: number) => (total > 0 ? Math.round((v / total) * 100) : 0);
  const announce = cursor ? `${shown[cursor.c].label}: ${formatNumber(shown[cursor.c].value)} ${unit}, ${pct(shown[cursor.c].value)}%` : "";
  let angle = 0;
  return (
    <ChartFrame
      title={title}
      empty={total <= 0}
      announcement={announce}
      onKeyDown={onKeyDown}
      legend={<Legend prefix={prefix} items={shown.map((s) => ({ label: s.label, detail: `${formatNumber(s.value)} (${pct(s.value)}%)` }))} />}
      table={<DataTable title={title} head={["Group", "Value", "Share"]} rows={shown.map((s) => [s.label, s.value, `${pct(s.value)}%`])} />}
    >
      <svg viewBox={`0 0 ${SIZE} ${SIZE}`} width="100%" role="presentation" className="max-w-xs">
        <PatternDefs prefix={prefix} count={shown.length} />
        {shown.map((s, i) => {
          const a0 = angle;
          angle += (s.value / total) * Math.PI * 2;
          const focused = cursor?.c === i;
          return (
            <path key={s.label} data-point data-category={i} d={wedge(SIZE / 2, SIZE / 2, R, a0, angle)} fill={patternFill(prefix, i)} stroke={focused ? "currentColor" : "#ffffff"} strokeWidth={focused ? 4 : 1.5}>
              <title>{`${s.label}: ${formatNumber(s.value)} ${unit}, ${pct(s.value)}%`}</title>
            </path>
          );
        })}
        {donut && <circle data-hole cx={SIZE / 2} cy={SIZE / 2} r={R * 0.55} className="fill-white dark:fill-gray-900" />}
      </svg>
    </ChartFrame>
  );
}
