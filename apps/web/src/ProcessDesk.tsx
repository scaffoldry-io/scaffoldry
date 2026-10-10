import React, { useState, useEffect } from "react";
import { Persona, RegisteredApp } from "./types";

export interface ProcessInstance {
  id: string;
  rule_id: string;
  app_slug: string;
  record_id: string;
  status: "Waiting" | "Completed" | "Rejected" | "Failed";
  waiting_step_id?: string | null;
  role?: string | null;
  prompt?: string | null;
  log?: string[];
  /** Why nobody can decide the step, when the server knows. */
  no_approver?: string | null;
  /** Who may decide it now. */
  approvers?: { eppn: string; display_name: string; via: unknown }[];
}

export interface ProcessDeskProps {
  apps: { slug: string; title?: string }[] | RegisteredApp[];
  callerPersona: Persona;
  callerWorkspaceRole?: string | null;
}

export const ProcessDesk: React.FC<ProcessDeskProps> = ({ apps }) => {
  const [instances, setInstances] = useState<ProcessInstance[]>([]);
  const [, setLoading] = useState<boolean>(true);
  const [errorMap, setErrorMap] = useState<Record<string, string>>({});

  useEffect(() => {
    let isMounted = true;
    const fetchAll = async () => {
      try {
        const results = await Promise.all(
          apps.map(async (app) => {
            try {
              const res = await fetch(`/api/v1/apps/${app.slug}/processes?status=Waiting&mine=true`);
              if (res.ok) {
                const data = await res.json();
                return Array.isArray(data) ? data : [];
              }
            } catch {
              return [];
            }
            return [];
          })
        );
        if (isMounted) {
          setInstances(results.flat());
        }
      } finally {
        if (isMounted) setLoading(false);
      }
    };
    fetchAll();
    return () => {
      isMounted = false;
    };
  }, [apps]);

  const handleDecide = async (inst: ProcessInstance, decision: "approve" | "reject") => {
    try {
      const res = await fetch(
        `/api/v1/apps/${inst.app_slug}/processes/${inst.id}/decide`,
        {
          method: "POST",
          headers: { "Content-Type": "application/json" },
          body: JSON.stringify({ decision }),
        }
      );
      if (res.ok && (res.status === 200 || res.status === 204)) {
        setInstances((prev) => prev.filter((i) => i.id !== inst.id));
        setErrorMap((prev) => {
          const next = { ...prev };
          delete next[inst.id];
          return next;
        });
      } else {
        setErrorMap((prev) => ({
          ...prev,
          [inst.id]: `HTTP ${res.status}`,
        }));
      }
    } catch (err: any) {
      setErrorMap((prev) => ({
        ...prev,
        [inst.id]: `Error: ${err?.message || "Failed"}`,
      }));
    }
  };

  // The server decides who may decide. The desk shows what `mine=true` returned.
  const visibleInstances = instances;

  return (
    <div
      data-testid="process-desk"
      className="bg-white dark:bg-slate-900 border border-slate-200 dark:border-slate-800 rounded-xl p-5 shadow-xs space-y-4"
    >
      <div className="flex items-center justify-between">
        <div className="flex items-center gap-2">
          <span className="text-lg">⚖️</span>
          <h2 className="text-base font-bold text-slate-900 dark:text-white">Decisions</h2>
        </div>
        {visibleInstances.length > 0 && (
          <span className="text-xs px-2 py-0.5 rounded-full bg-amber-100 dark:bg-amber-950/60 text-amber-800 dark:text-amber-300 font-mono font-bold">
            {visibleInstances.length} pending
          </span>
        )}
      </div>

      {visibleInstances.length === 0 ? (
        <div className="py-6 text-center text-xs text-slate-500 dark:text-slate-400">
          No decisions waiting
        </div>
      ) : (
        <div className="divide-y divide-slate-100 dark:divide-slate-800">
          {visibleInstances.map((inst) => (
            <div
              key={inst.id}
              data-testid={`process-row-${inst.id}`}
              className="py-3.5 flex flex-col sm:flex-row sm:items-center justify-between gap-3 text-xs"
            >
              <div className="space-y-1 flex-1 min-w-0">
                <div className="font-semibold text-slate-900 dark:text-slate-100">
                  {inst.prompt || "Decision requested"}
                </div>
                <div className="flex flex-wrap items-center gap-2 text-[11px] text-slate-500 dark:text-slate-400">
                  {inst.role && (
                    <span className="px-1.5 py-0.5 rounded bg-slate-100 dark:bg-slate-800 font-medium">
                      Role: {inst.role}
                    </span>
                  )}
                  <span>
                    Record: <code className="font-mono text-slate-700 dark:text-slate-300">{inst.record_id}</code>
                  </span>
                  <span>
                    Rule: <code className="font-mono text-slate-700 dark:text-slate-300">{inst.rule_id}</code>
                  </span>
                </div>
                {errorMap[inst.id] && (
                  <div className="text-[11px] font-semibold text-rose-600 dark:text-rose-400">
                    {errorMap[inst.id]}
                  </div>
                )}
              </div>

              <div className="flex items-center gap-2 shrink-0">
                <button
                  type="button"
                  data-testid={`process-approve-${inst.id}`}
                  onClick={() => handleDecide(inst, "approve")}
                  className="px-3 py-1.5 rounded-lg bg-emerald-600 hover:bg-emerald-700 text-white font-semibold cursor-pointer shadow-xs transition-colors"
                >
                  Approve
                </button>
                <button
                  type="button"
                  data-testid={`process-reject-${inst.id}`}
                  onClick={() => handleDecide(inst, "reject")}
                  className="px-3 py-1.5 rounded-lg bg-slate-100 hover:bg-rose-50 text-slate-700 hover:text-rose-700 dark:bg-slate-800 dark:hover:bg-rose-950/40 dark:text-slate-300 dark:hover:text-rose-300 font-semibold border border-slate-200 dark:border-slate-700 hover:border-rose-300 cursor-pointer transition-colors"
                >
                  Reject
                </button>
              </div>
            </div>
          ))}
        </div>
      )}
    </div>
  );
};
