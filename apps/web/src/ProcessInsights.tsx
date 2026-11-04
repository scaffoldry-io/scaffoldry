import { useEffect, useState } from "react";
import { apiClient, type MyApp, type ProcessAnalytics, type ProcessStepAnalytics } from "./api";
import { ErrorState } from "./ui";

/** The age buckets the server counts, in order, with their words. */
const AGES: [string, string, string][] = [
  ["under_1_day", "under a day", "bg-emerald-500"],
  ["1_to_3_days", "1 to 3 days", "bg-lime-500"],
  ["3_to_7_days", "3 to 7 days", "bg-amber-400"],
  ["7_to_14_days", "7 to 14 days", "bg-orange-500"],
  ["14_to_30_days", "14 to 30 days", "bg-rose-500"],
  ["over_30_days", "over 30 days", "bg-rose-700"],
];

/** A duration in words. It rounds the same way as the server's note on the slowest step. */
export function inWords(seconds: number | null): string {
  if (seconds === null) return "Not enough yet";
  const [n, unit] =
    seconds >= 86400 ? [Math.round(seconds / 86400), "day"] : seconds >= 3600 ? [Math.round(seconds / 3600), "hour"] : [Math.max(1, Math.round(seconds / 60)), "minute"];
  return `${n} ${unit}${n === 1 ? "" : "s"}`;
}

function waitingSentence(step: ProcessStepAnalytics): string {
  const parts = AGES.filter(([k]) => (step.waiting_by_age[k] ?? 0) > 0).map(([k, words]) => `${step.waiting_by_age[k]} for ${words}`);
  return `${step.name}: ${step.waiting} waiting.${parts.length ? ` ${parts.join(", ")}.` : ""}`;
}

/** A bar of how many are waiting at a step, split by how long they have waited. The words are its label. */
function WaitingBar({ step }: { step: ProcessStepAnalytics }) {
  return (
    <div role="img" aria-label={waitingSentence(step)} className="flex h-3 w-40 overflow-hidden rounded bg-slate-100 dark:bg-slate-800">
      {step.waiting > 0 &&
        AGES.map(([k, , colour]) => {
          const n = step.waiting_by_age[k] ?? 0;
          return n > 0 ? <div key={k} className={colour} style={{ width: `${(n / step.waiting) * 100}%` }} /> : null;
        })}
    </div>
  );
}

function outcomeSentence(finished: number, outcomes: Record<string, number>): string {
  if (finished === 0) return "None has finished yet.";
  const count = (k: string) => Math.round((outcomes[k] ?? 0) * finished);
  const rest = [
    ["Rejected", "rejected"],
    ["Failed", "failed"],
    ["Cancelled", "cancelled"],
  ]
    .filter(([k]) => count(k) > 0)
    .map(([k, w]) => `${count(k)} ${count(k) === 1 ? "was" : "were"} ${w}.`);
  return [`${count("Completed")} of ${finished} finished were completed.`, ...rest].join(" ");
}

/**
 * The `Process insights` tab (process v2 phase 1): for each rule, a table by step with the figures in words and in
 * numbers, a bar of how many are waiting by age, and a note on the slowest step. It reads the event log only.
 */
export function ProcessInsights({ appSlug }: { appSlug: string }) {
  const [data, setData] = useState<ProcessAnalytics | null>(null);
  const [error, setError] = useState<unknown>(null);
  const [from, setFrom] = useState("");
  const [to, setTo] = useState("");

  const load = (range: { from?: string; to?: string }) => {
    setError(null);
    apiClient.getProcessAnalytics(appSlug, range).then(setData).catch(setError);
  };
  useEffect(() => {
    setData(null);
    load({});
  }, [appSlug]);

  return (
    <section className="space-y-4" data-testid="process-insights">
      <form
        className="flex flex-wrap items-end gap-3 text-xs"
        onSubmit={(e) => {
          e.preventDefault();
          load({ ...(from ? { from } : {}), ...(to ? { to } : {}) });
        }}
      >
        <label className="flex flex-col gap-1">
          <span>From</span>
          <input type="date" value={from} onChange={(e) => setFrom(e.target.value)} className="rounded border px-2 py-1 dark:bg-slate-900" />
        </label>
        <label className="flex flex-col gap-1">
          <span>To</span>
          <input type="date" value={to} onChange={(e) => setTo(e.target.value)} className="rounded border px-2 py-1 dark:bg-slate-900" />
        </label>
        <button type="submit" className="rounded bg-blue-600 px-3 py-1 font-semibold text-white">
          Show
        </button>
      </form>

      {error ? (
        <ErrorState error={error} />
      ) : !data ? (
        <p className="text-xs text-slate-500">Loading…</p>
      ) : data.rules.length === 0 ? (
        <p className="text-sm text-slate-600 dark:text-slate-400">No process has run in this app yet.</p>
      ) : (
        <>
          {data.slowest && <p className="rounded border border-amber-200 bg-amber-50 p-3 text-sm text-amber-900 dark:border-amber-900 dark:bg-amber-950/40 dark:text-amber-200">{data.slowest}</p>}
          {data.rules.map((rule) => (
            <div key={rule.rule_id} className="space-y-2 rounded-xl border border-slate-200 bg-white p-4 dark:border-slate-800 dark:bg-slate-900">
              <h3 className="text-sm font-bold text-slate-900 dark:text-white">{rule.name}</h3>
              <p className="text-xs text-slate-600 dark:text-slate-400">{outcomeSentence(rule.finished, rule.outcomes)}</p>
              <table className="w-full text-left text-xs">
                <thead>
                  <tr className="text-slate-500">
                    <th className="py-1">Step</th>
                    <th>Reached</th>
                    <th>Median time</th>
                    <th>9 in 10 within</th>
                    <th>Waiting now</th>
                  </tr>
                </thead>
                <tbody>
                  {rule.steps.map((step) => (
                    <tr key={step.step_id} className="border-t border-slate-100 dark:border-slate-800">
                      <th scope="row" className="py-2 font-medium">
                        {step.name}
                      </th>
                      <td>{step.entered}</td>
                      <td title={step.median_seconds === null ? undefined : `${Math.round(step.median_seconds)} seconds`}>{inWords(step.median_seconds)}</td>
                      <td title={step.p90_seconds === null ? undefined : `${Math.round(step.p90_seconds)} seconds`}>{inWords(step.p90_seconds)}</td>
                      <td>
                        <div className="flex items-center gap-2">
                          <span>{step.waiting}</span>
                          <WaitingBar step={step} />
                        </div>
                      </td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
          ))}
        </>
      )}
    </section>
  );
}

/** The console panel: the same view, for an app the admin picks. */
export function ProcessInsightsPanel() {
  const [apps, setApps] = useState<MyApp[]>([]);
  const [error, setError] = useState<unknown>(null);
  const [slug, setSlug] = useState("");
  useEffect(() => {
    apiClient.listMyApps().then(setApps).catch(setError);
  }, []);
  return (
    <div className="space-y-4" data-testid="process-insights-panel">
      <div>
        <h2 className="text-lg font-bold text-slate-900 dark:text-white">Process insights</h2>
        <p className="text-xs text-slate-500">Where time goes in each app's processes. It shows counts and times, never record values.</p>
      </div>
      {error ? <ErrorState error={error} /> : null}
      <label className="flex flex-col gap-1 text-xs">
        <span>App</span>
        <select value={slug} onChange={(e) => setSlug(e.target.value)} className="max-w-xs rounded border px-2 py-1 dark:bg-slate-900">
          <option value="">Choose an app</option>
          {apps.map((a) => (
            <option key={a.slug} value={a.slug}>
              {a.title}
            </option>
          ))}
        </select>
      </label>
      {slug && <ProcessInsights appSlug={slug} />}
    </div>
  );
}
