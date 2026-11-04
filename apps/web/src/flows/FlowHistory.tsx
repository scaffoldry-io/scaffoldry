// Workflows phase 10: a saved flow's Runs and Versions tabs. Runs lists each run with the failure rate and the circuit
// breaker; a run opens its timeline, and a failed one can be retried with a reason. Versions lists who wrote and who
// approved each version, and draws two of them on the canvas with every step marked added, changed or removed.

import { useEffect, useState } from "react";
import { apiClient } from "../api";
import type { ProcessStepItem, WorkflowAutomationRule } from "../types";
import { Canvas } from "./Canvas";
import { numberOf } from "./Outline";
import { initialEditor } from "./reducer";

const tab = (on: boolean) =>
  `px-2.5 py-1 rounded text-[11px] font-semibold cursor-pointer ${on ? "bg-blue-600 text-white" : "border border-slate-300 dark:border-slate-700 text-slate-700 dark:text-slate-300"}`;
const button = "px-3 py-1 rounded bg-blue-600 hover:bg-blue-700 text-white text-[11px] font-semibold cursor-pointer disabled:opacity-50 disabled:cursor-default";
const input = "px-2.5 py-1 rounded border border-slate-300 dark:border-slate-700 bg-white dark:bg-slate-800 text-slate-900 dark:text-white";

const day = (iso: string) => new Date(iso).toLocaleDateString("en-GB", { day: "numeric", month: "short", timeZone: "UTC" });
const when = (iso: string) => new Date(iso).toLocaleString("en-GB", { day: "numeric", month: "short", hour: "2-digit", minute: "2-digit", timeZone: "UTC" });
const seconds = (ms: number | null | undefined) => (ms == null ? "still running" : ms < 1000 ? "under a second" : `${Math.round(ms / 1000)} s`);

export function FlowHistory({ flowId, appTitle }: { flowId: string; appTitle: string }) {
  const [view, setView] = useState<"runs" | "versions">("runs");
  return (
    <section aria-label="Flow history" className="space-y-2 text-xs">
      <div role="tablist" aria-label={`History of the flow in ${appTitle}`} className="flex gap-1">
        <button type="button" role="tab" aria-selected={view === "runs"} className={tab(view === "runs")} onClick={() => setView("runs")}>
          Runs
        </button>
        <button type="button" role="tab" aria-selected={view === "versions"} className={tab(view === "versions")} onClick={() => setView("versions")}>
          Versions
        </button>
      </div>
      <div role="tabpanel">{view === "runs" ? <Runs flowId={flowId} /> : <Versions flowId={flowId} />}</div>
    </section>
  );
}

function Runs({ flowId }: { flowId: string }) {
  const [status, setStatus] = useState("");
  const [data, setData] = useState<any>(null);
  const [error, setError] = useState<string | null>(null);
  const [open, setOpen] = useState<string | null>(null);
  const load = () =>
    apiClient
      .flowRuns(flowId, status || undefined)
      .then((d) => {
        setData(d);
        setError(null);
      })
      .catch((e) => setError(e instanceof Error ? e.message : "The runs could not be read."));
  useEffect(() => {
    void load();
    // Reloads when the filter changes.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [flowId, status]);

  return (
    <div className="space-y-2">
      {error && <p className="text-rose-700 dark:text-rose-300">{error}</p>}
      {data && (
        <div className="space-y-0.5">
          <p>{data.last_7_days?.sentence}</p>
          <p className={data.breaker?.off ? "text-rose-700 dark:text-rose-300 font-semibold" : "text-slate-600 dark:text-slate-400"}>{data.breaker?.sentence}</p>
        </div>
      )}
      <label className="inline-flex items-center gap-1.5">
        <span>Show</span>
        <select aria-label="Show runs" value={status} onChange={(e) => setStatus(e.target.value)} className={input}>
          <option value="">All runs</option>
          <option value="Failed">Failed runs</option>
          <option value="Completed">Completed runs</option>
          <option value="Waiting">Waiting runs</option>
        </select>
      </label>
      {data && (data.runs ?? []).length === 0 && <p className="text-slate-500">No run to show.</p>}
      {data && (data.runs ?? []).length > 0 && (
        <table className="w-full text-left">
          <thead>
            <tr className="text-slate-500">
              <th className="py-1">Status</th>
              <th>Started</th>
              <th>Took</th>
              <th>Trigger</th>
              <th>By</th>
              <th>Writes</th>
              <th>
                <span className="sr-only">Open</span>
              </th>
            </tr>
          </thead>
          <tbody>
            {data.runs.map((r: any) => (
              <tr key={r.instance_id} className="border-t border-slate-100 dark:border-slate-800">
                <td className={r.status === "Failed" ? "text-rose-700 dark:text-rose-300 font-semibold py-1" : "py-1"}>{r.status}</td>
                <td>{when(r.started_at)}</td>
                <td>{seconds(r.duration_ms)}</td>
                <td>{r.trigger}</td>
                <td>{r.started_by}</td>
                <td>{r.writes == null ? "" : `${r.writes} ${r.writes === 1 ? "write" : "writes"}`}</td>
                <td>
                  <button type="button" className="underline cursor-pointer" aria-label={`Open the run of ${day(r.started_at)}`} onClick={() => setOpen(r.instance_id)}>
                    Open
                  </button>
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
      {open && <Timeline instanceId={open} onRetried={load} />}
    </div>
  );
}

function Timeline({ instanceId, onRetried }: { instanceId: string; onRetried: () => void }) {
  const [tl, setTl] = useState<any>(null);
  const [reason, setReason] = useState("");
  const [done, setDone] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const load = () => apiClient.flowRun(instanceId).then(setTl).catch((e) => setError(e instanceof Error ? e.message : "The run could not be read."));
  useEffect(() => {
    setDone(null);
    void load();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [instanceId]);

  const retry = async () => {
    setError(null);
    try {
      const r = await apiClient.retryFlowRun(instanceId, reason.trim());
      setDone(r?.status === "Completed" ? "The retried run completed." : `The retried run is ${String(r?.status ?? "").toLowerCase()}.`);
      setReason("");
      await load();
      onRetried();
    } catch (e) {
      setError(e instanceof Error ? e.message : "The run could not be retried.");
    }
  };

  if (!tl) return error ? <p className="text-rose-700 dark:text-rose-300">{error}</p> : null;
  const failedAt = (tl.steps ?? []).find((s: any) => s.step === tl.first_error?.step);
  return (
    <section aria-label="Run timeline" className="rounded-lg border border-slate-200 dark:border-slate-700 bg-white dark:bg-slate-900 p-3 space-y-2">
      <p className="font-semibold">
        {tl.trigger}. {tl.status}.
      </p>
      {tl.first_error?.message && (
        <p role="alert" className="text-rose-700 dark:text-rose-300 font-semibold">
          Step {failedAt?.path ? numberOf(failedAt.path) : tl.first_error.step} failed: {tl.first_error.message}
        </p>
      )}
      <ol className="list-decimal pl-5 space-y-1">
        {(tl.steps ?? []).map((s: any, i: number) => (
          <li key={i}>
            <span className="font-semibold">Step {s.path ? numberOf(s.path) : s.step}</span>: {s.outcome}
            {(s.effects ?? []).map((e: any, j: number) => (
              <div key={j} className="pl-2 text-slate-600 dark:text-slate-400">
                {e.sentence}
                {(e.records ?? []).map((r: any) =>
                  r.link ? (
                    <a key={r.id} href={r.link} aria-label={`Open record ${r.id}`} className="ml-1 underline text-blue-700 dark:text-blue-300">
                      {r.id}
                    </a>
                  ) : (
                    <span key={r.id} className="ml-1">
                      a record in {r.app} / {r.table} ({r.id})
                    </span>
                  )
                )}
              </div>
            ))}
          </li>
        ))}
      </ol>
      {(tl.retries ?? []).length > 0 && (
        <p className="text-slate-600 dark:text-slate-400">
          Retried {tl.retries.length} {tl.retries.length === 1 ? "time" : "times"}: {tl.retries.map((r: any) => `by ${r.by}, because ${r.reason}`).join("; ")}.
        </p>
      )}
      {done && <p className="text-emerald-800 dark:text-emerald-300">{done}</p>}
      {error && <p className="text-rose-700 dark:text-rose-300">{error}</p>}
      {tl.retry?.allowed ? (
        <div className="flex flex-wrap items-end gap-2">
          <label className="block grow">
            <span className="block text-[11px] text-slate-500 mb-1">Why retry it</span>
            <input aria-label="Why retry it" value={reason} onChange={(e) => setReason(e.target.value)} className={`${input} w-full`} placeholder="What changed since it failed" />
          </label>
          <button type="button" className={button} disabled={!reason.trim()} onClick={retry}>
            Retry this run
          </button>
        </div>
      ) : (
        tl.retry?.why && tl.status === "Failed" && <p className="text-slate-600 dark:text-slate-400">{tl.retry.why}</p>
      )}
    </section>
  );
}

function flowOf(steps: ProcessStepItem[]): WorkflowAutomationRule {
  return { id: "compare", app_slug: "", name: "", description: "", enabled: true, trigger: { type: "RecordCreated" }, predicates: [], actions: [], steps };
}

function Versions({ flowId }: { flowId: string }) {
  const [list, setList] = useState<any[] | null>(null);
  const [from, setFrom] = useState("");
  const [to, setTo] = useState("");
  const [cmp, setCmp] = useState<any>(null);
  const [error, setError] = useState<string | null>(null);
  useEffect(() => {
    apiClient
      .flowVersions(flowId)
      .then((d) => setList(d?.versions ?? []))
      .catch((e) => setError(e instanceof Error ? e.message : "The versions could not be read."));
  }, [flowId]);

  const compare = () =>
    apiClient
      .compareFlowVersions(flowId, Number(from), Number(to))
      .then(setCmp)
      .catch((e) => setError(e instanceof Error ? e.message : "The versions could not be compared."));

  const marksFor = (side: "from" | "to") => {
    const out: Record<string, { change: any; against: number }> = {};
    for (const s of cmp?.steps ?? []) {
      if (side === "to" && s.change !== "removed") out[s.step] = { change: s.change, against: cmp.from };
      if (side === "from" && (s.change === "removed" || s.change === "changed")) out[s.step] = { change: s.change, against: cmp.to };
    }
    return out;
  };

  return (
    <div className="space-y-2">
      {error && <p className="text-rose-700 dark:text-rose-300">{error}</p>}
      {list && list.length === 0 && <p className="text-slate-500">This flow has no stored version yet.</p>}
      {list && list.length > 0 && (
        <table className="w-full text-left">
          <thead>
            <tr className="text-slate-500">
              <th className="py-1">Version</th>
              <th>Written by</th>
              <th>Approved by</th>
              <th>Proposal</th>
              <th>Summary</th>
            </tr>
          </thead>
          <tbody>
            {list.map((v) => (
              <tr key={v.version} className="border-t border-slate-100 dark:border-slate-800">
                <td className="py-1">
                  Version {v.version}
                  {v.live ? " (live)" : ""}
                </td>
                <td>{v.author}</td>
                <td>{v.approvers?.length ? v.approvers.join(", ") : "Saved without a proposal"}</td>
                <td>{v.proposal_id ?? ""}</td>
                <td>{v.summary}</td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
      {list && list.length > 1 && (
        <div className="flex flex-wrap items-end gap-2">
          <label className="block">
            <span className="block text-[11px] text-slate-500 mb-1">Compare version</span>
            <select aria-label="Compare version" value={from} onChange={(e) => setFrom(e.target.value)} className={input}>
              <option value="">Choose</option>
              {list.map((v) => (
                <option key={v.version} value={v.version}>
                  {v.version}
                </option>
              ))}
            </select>
          </label>
          <label className="block">
            <span className="block text-[11px] text-slate-500 mb-1">With version</span>
            <select aria-label="With version" value={to} onChange={(e) => setTo(e.target.value)} className={input}>
              <option value="">Choose</option>
              {list.map((v) => (
                <option key={v.version} value={v.version}>
                  {v.version}
                </option>
              ))}
            </select>
          </label>
          <button type="button" className={button} disabled={!from || !to || from === to} onClick={compare}>
            Compare
          </button>
        </div>
      )}
      {cmp && (
        <div className="space-y-2">
          <ul className="list-disc pl-5">
            {(cmp.sentences ?? []).map((t: string, i: number) => (
              <li key={i}>{t}</li>
            ))}
          </ul>
          <div className="grid gap-3 lg:grid-cols-2">
            <Canvas state={initialEditor(flowOf(cmp.from_steps ?? []))} dispatch={() => {}} fields={[]} appSlug="" problems={[]} marks={marksFor("from")} readOnly label={`Version ${cmp.from}`} />
            <Canvas state={initialEditor(flowOf(cmp.to_steps ?? []))} dispatch={() => {}} fields={[]} appSlug="" problems={[]} marks={marksFor("to")} readOnly label={`Version ${cmp.to}`} />
          </div>
        </div>
      )}
    </div>
  );
}
