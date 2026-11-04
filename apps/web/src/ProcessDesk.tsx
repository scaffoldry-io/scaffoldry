import React, { useState, useEffect } from "react";
import { Persona, RegisteredApp } from "./types";
import { getAuthToken } from "./api";
import { dueWords, tallyWords } from "./processWords";

/** The signed-in person's token, as the header every API request carries. */
const authHeaders = (): Record<string, string> => {
  const token = getAuthToken();
  return token ? { Authorization: `Bearer ${token}` } : {};
};

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
  /** Process v2 phase 4: the voters of a committee step, and the counts so far. */
  voters?: string[];
  tally?: { approve: number; reject: number; voters: number };
  /** Process v2 phase 3: the step's next timer. The first one is when the step is due. */
  next_timer?: { fire_at: string; action: unknown; sentence: string };
}

const authJson = (): Record<string, string> => ({ "Content-Type": "application/json", ...authHeaders() });

/** A committee step: the tally in words, the person's own vote, and a way to cast or change it. Names nobody else. */
const CommitteeVote: React.FC<{ inst: ProcessInstance; me: string }> = ({ inst, me }) => {
  const [mine, setMine] = useState<"approve" | "reject" | null>(null);
  const [tally, setTally] = useState(inst.tally);
  const [changing, setChanging] = useState(false);
  const [comment, setComment] = useState("");
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let live = true;
    fetch(`/api/v1/apps/${inst.app_slug}/processes/${inst.id}/votes`, { headers: authHeaders() })
      .then((r) => (r.ok ? r.json() : null))
      .then((body) => {
        if (!live || !body) return;
        const own = (body.votes ?? []).find((v: { voter: string }) => v.voter.toLowerCase() === me.toLowerCase());
        setMine(own ? own.vote : null);
        if (body.tally) setTally(body.tally);
      })
      .catch(() => {});
    return () => {
      live = false;
    };
  }, [inst.app_slug, inst.id, me]);

  const vote = async (decision: "approve" | "reject") => {
    setError(null);
    const body = decision === "reject" ? { decision, comment: comment.trim() } : { decision };
    const res = await fetch(`/api/v1/apps/${inst.app_slug}/processes/${inst.id}/decide`, { method: "POST", headers: authJson(), body: JSON.stringify(body) });
    if (!res.ok) {
      setError(`Your vote was not counted (HTTP ${res.status}).`);
      return;
    }
    setMine(decision);
    setChanging(false);
    setComment("");
  };

  const other = mine === "approve" ? "reject" : "approve";
  const choosing = mine === null || changing;
  return (
    <div className="space-y-1.5 text-[11px]">
      {tally && <div className="font-medium text-slate-700 dark:text-slate-300">{tallyWords(tally)}</div>}
      {mine && <div>You voted to {mine}.</div>}
      {mine && !changing && (
        <button type="button" onClick={() => setChanging(true)} className="underline cursor-pointer">
          Change my vote
        </button>
      )}
      {choosing && (
        <div className="flex flex-wrap items-end gap-2">
          {(mine === null || other === "approve") && (
            <button type="button" onClick={() => vote("approve")} className="px-2.5 py-1 rounded bg-emerald-600 text-white font-semibold cursor-pointer">
              Vote to approve
            </button>
          )}
          {(mine === null || other === "reject") && (
            <>
              <label className="flex flex-col">
                <span className="text-slate-500">Why you reject</span>
                <input aria-label="Why you reject" value={comment} onChange={(e) => setComment(e.target.value)} className="px-2 py-1 rounded border border-slate-300 dark:border-slate-700 bg-white dark:bg-slate-800" />
              </label>
              <button type="button" onClick={() => vote("reject")} className="px-2.5 py-1 rounded border border-slate-300 dark:border-slate-700 font-semibold cursor-pointer">
                Vote to reject
              </button>
            </>
          )}
        </div>
      )}
      {error && <div className="font-semibold text-rose-600 dark:text-rose-400">{error}</div>}
    </div>
  );
};

export interface ProcessDeskProps {
  apps: { slug: string; title?: string }[] | RegisteredApp[];
  callerPersona: Persona;
  callerWorkspaceRole?: string | null;
}

export const ProcessDesk: React.FC<ProcessDeskProps> = ({ apps, callerPersona }) => {
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
              const res = await fetch(`/api/v1/apps/${app.slug}/processes?status=Waiting&mine=true`, { headers: authHeaders() });
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
          headers: { "Content-Type": "application/json", ...authHeaders() },
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
                {inst.next_timer && (
                  <div data-testid={`process-due-${inst.id}`} className="text-[11px] font-medium text-slate-600 dark:text-slate-300">
                    {dueWords(inst.next_timer.fire_at)}
                  </div>
                )}
                {inst.voters && inst.voters.length > 0 && <CommitteeVote inst={inst} me={callerPersona.eppn} />}
                {errorMap[inst.id] && (
                  <div className="text-[11px] font-semibold text-rose-600 dark:text-rose-400">
                    {errorMap[inst.id]}
                  </div>
                )}
              </div>

              {!(inst.voters && inst.voters.length > 0) && <div className="flex items-center gap-2 shrink-0">
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
              </div>}
            </div>
          ))}
        </div>
      )}
    </div>
  );
};
