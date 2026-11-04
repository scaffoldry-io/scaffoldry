// Workflows phase 9: a test run of the flow in the editor. The server runs the draft in dry mode against a record and
// returns a trace. Nothing is written, sent or stored. The canvas and the outline mark each step with what it did.

import { useState } from "react";
import { apiClient } from "../api";
import type { WorkflowAutomationRule } from "../types";
import { numberOf } from "./Outline";
import { toServerFlow, type Path } from "./reducer";

export interface TraceEntry {
  step: string;
  path: Path;
  outcome: "ran" | "would write" | "skipped (condition was false)" | "failed" | string;
  sentence: string;
  approvers?: string[];
  count?: number;
  records?: { id: string | null; values?: Record<string, unknown> }[];
  hidden?: string;
}

export interface TestRunResult {
  trace: TraceEntry[];
  first_failure: { step: string; path: Path; reason: string } | null;
  writes: number;
}

export const SKIPPED = "skipped (condition was false)";

/** The trace entry for a path, if the run reached it. */
export function traceAt(trace: TraceEntry[] | undefined, path: Path): TraceEntry | undefined {
  return trace?.find((t) => t.path.length === path.length && t.path.every((x, i) => x === path[i]));
}

const cap = (s: string) => s.charAt(0).toUpperCase() + s.slice(1);

/** What a step did in the run, in words, for its accessible name and its line in the outline. */
export function traceWords(t: TraceEntry): string {
  if (t.outcome === SKIPPED) return "skipped, its condition was false";
  if (t.outcome === "failed") return `failed: ${t.sentence}`;
  return `${t.outcome}: ${t.sentence}`;
}

/** The same, as the canvas says it: whether the step is on the path, then what it did. */
export function traceName(t: TraceEntry): string {
  if (t.outcome === SKIPPED) return "Test run: skipped, its condition was false";
  if (t.outcome === "failed") return `Test run: failed: ${t.sentence}`;
  return `Test run: on the path. ${cap(t.outcome)}: ${t.sentence}`;
}

/** One sentence for the whole run. */
export function traceSummary(trace: TraceEntry[]): string {
  const on = trace.filter((t) => t.outcome !== SKIPPED && t.outcome !== "failed").length;
  const skipped = trace.filter((t) => t.outcome === SKIPPED).length;
  const failed = trace.find((t) => t.outcome === "failed");
  return `Test run: ${on} ${on === 1 ? "step" : "steps"} on the path, ${skipped} skipped, ${failed ? `failed at step ${numberOf(failed.path)}` : "no failure"}.`;
}

const input = "w-full px-2.5 py-1 rounded border border-slate-300 dark:border-slate-700 bg-white dark:bg-slate-800 text-slate-900 dark:text-white";
const button = "px-3 py-1 rounded bg-blue-600 hover:bg-blue-700 text-white text-[11px] font-semibold cursor-pointer disabled:opacity-50";

/** Asks the server for a test run of `flow` against a record, shows the result, and hands the trace to the views. */
export function TestRunPanel({ flow, onTrace }: { flow: WorkflowAutomationRule; onTrace: (trace: TraceEntry[] | null) => void }) {
  const [recordId, setRecordId] = useState("");
  const [result, setResult] = useState<TestRunResult | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  const run = async () => {
    setBusy(true);
    setError(null);
    try {
      const r = await apiClient.testRunFlow(flow.id, { flow: toServerFlow(flow), record_id: recordId.trim() || undefined });
      const clean: TestRunResult = { trace: r?.trace ?? [], first_failure: r?.first_failure ?? null, writes: r?.writes ?? 0 };
      setResult(clean);
      onTrace(clean.trace);
    } catch (e) {
      setResult(null);
      onTrace(null);
      setError(e instanceof Error ? e.message : "The test run could not start.");
    } finally {
      setBusy(false);
    }
  };

  return (
    <section aria-label="Test run" className="rounded-lg border border-slate-200 dark:border-slate-700 bg-white dark:bg-slate-900 p-3 space-y-2 text-xs">
      <div className="flex flex-wrap items-end gap-2">
        <label className="block grow">
          <span className="block text-[11px] text-slate-500 mb-1">{result ? "Run again with record id" : "Record id"}</span>
          <input aria-label={result ? "Run again with record id" : "Record id"} value={recordId} onChange={(e) => setRecordId(e.target.value)} placeholder="The id of a record to test with" className={input} />
        </label>
        <button type="button" className={button} disabled={busy} onClick={run}>
          {result ? "Run again" : "Run a test"}
        </button>
      </div>
      <p className="text-slate-500">A test run reads real data and writes nothing. Each decision is taken as approved.</p>
      {error && <p className="text-rose-700 dark:text-rose-300">{error}</p>}
      {result && (
        <div className="space-y-1">
          {result.first_failure && (
            <p role="alert" className="text-rose-700 dark:text-rose-300 font-semibold">
              Step {numberOf(result.first_failure.path)} failed: {result.first_failure.reason}
            </p>
          )}
          <p className="text-slate-600 dark:text-slate-400">
            {traceSummary(result.trace)} Nothing was written, sent or stored. A real run would make {result.writes} {result.writes === 1 ? "write" : "writes"}.
          </p>
          <ol className="list-decimal pl-5 space-y-0.5">
            {result.trace.map((t) => (
              <li key={t.path.join(".")} className={t.outcome === SKIPPED ? "text-slate-400" : undefined}>
                Step {numberOf(t.path)}: {traceWords(t)}
                {t.hidden && <span className="text-slate-500"> ({t.hidden})</span>}
              </li>
            ))}
          </ol>
        </div>
      )}
    </section>
  );
}
