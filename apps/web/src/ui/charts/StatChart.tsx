import { formatNumber } from "./shared";

export interface StatChartProps {
  label: string;
  value: number;
  unit: string;
  /** A second number to set the first against, such as the same count for last term. */
  comparison?: { label: string; value: number };
}

/** The difference in words: `Up 12 from 30 last term (40%)`. */
export function differenceInWords(value: number, comparison: { label: string; value: number }): string {
  const diff = value - comparison.value;
  const from = `${formatNumber(comparison.value)} ${comparison.label}`;
  if (diff === 0) return `No change from ${from}`;
  const share = comparison.value !== 0 ? ` (${Math.round((Math.abs(diff) / Math.abs(comparison.value)) * 100)}%)` : "";
  return `${diff > 0 ? "Up" : "Down"} ${formatNumber(Math.abs(diff))} from ${from}${share}`;
}

/** One number, large, with an optional comparison. The difference is said in words, not only shown by an arrow or a color. */
export function StatChart({ label, value, unit, comparison }: StatChartProps) {
  return (
    <div role="group" aria-label={label} className="rounded border p-4">
      <div className="text-sm text-gray-700 dark:text-gray-300">{label}</div>
      <div className="flex items-baseline gap-2">
        <span className="text-4xl font-bold tabular-nums">{formatNumber(value)}</span>
        <span className="text-sm text-gray-700 dark:text-gray-300">{unit}</span>
      </div>
      {comparison && (
        <>
          <div className="mt-2 flex items-baseline gap-1 text-sm text-gray-700 dark:text-gray-300">
            <span className="tabular-nums">{formatNumber(comparison.value)}</span>
            <span>{comparison.label}</span>
          </div>
          <p className="mt-1 text-sm font-medium">{differenceInWords(value, comparison)}</p>
        </>
      )}
    </div>
  );
}
