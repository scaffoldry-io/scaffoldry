import React, { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { getAuthToken } from "./api";
import { onMeEvent } from "./realtime";
import { dueWords } from "./processWords";

// Workflows phase 5 and 6: My Work. One list of what the person is asked to do, in four tabs, with a review drawer that
// moves through the list. `me/events` keeps the counts and the list current without a reload.

const headers = (): Record<string, string> => {
  const token = getAuthToken();
  return token ? { Authorization: `Bearer ${token}` } : {};
};

export interface WorkItem {
  id: string;
  kind: "decision" | "task" | "review" | "waiting";
  title: string;
  summary: string;
  app_slug?: string | null;
  workspace_id?: string | null;
  instance_id?: string | null;
  proposal_id?: string | null;
  via: string;
  acting_for?: string | null;
  due_at?: string | null;
  created_at: string;
}

interface Detail {
  prompt?: string | null;
  show?: Record<string, unknown> | null;
  fill?: string[];
  require?: string[];
  history?: string[];
  reject_comment_required?: boolean;
  tally?: { approve: number; reject: number; voters: number };
  instance?: { approvers?: { eppn: string; display_name?: string }[]; next_timer?: { sentence: string } | null; started_by?: string | null };
}

const TABS: { kind: WorkItem["kind"]; label: string }[] = [
  { kind: "decision", label: "Decisions" },
  { kind: "task", label: "Tasks" },
  { kind: "review", label: "Reviews" },
  { kind: "waiting", label: "Waiting on others" },
];

const isOverdue = (w: WorkItem) => !!w.due_at && new Date(w.due_at).getTime() < Date.now();

/** The row's words for when it is due. Empty when it has no due date. */
const dueOf = (w: WorkItem) => (w.due_at ? dueWords(w.due_at) : "");

async function getJson<T>(url: string): Promise<T | null> {
  try {
    const res = await fetch(url, { headers: headers() });
    return res.ok ? ((await res.json()) as T) : null;
  } catch {
    return null;
  }
}

/** The navigation item: `My Work`, with the open decisions and tasks in its accessible name. */
export const MyWorkNav: React.FC<{ active: boolean; expanded: boolean; onOpen: () => void }> = ({ active, expanded, onOpen }) => {
  const [count, setCount] = useState(0);
  const load = useCallback(async () => {
    const s = await getJson<{ by_kind?: Record<string, number> }>("/api/v1/me/work/summary");
    const by = s?.by_kind;
    if (by && typeof by === "object") setCount((Number(by.decision) || 0) + (Number(by.task) || 0));
  }, []);
  useEffect(() => {
    load();
    return onMeEvent((e) => {
      if (e.event === "work") load();
    });
  }, [load]);
  return (
    <button
      type="button"
      aria-label={`My Work, ${count} open`}
      onClick={onOpen}
      className={`w-full flex items-center gap-3 px-2.5 py-2 rounded-lg text-xs font-medium transition-colors cursor-pointer ${
        active ? "bg-blue-50 text-blue-700 dark:bg-blue-900/30 dark:text-blue-300 font-semibold" : "text-slate-600 dark:text-slate-400 hover:bg-slate-100 dark:hover:bg-slate-800/60"
      }`}
    >
      <span className="text-sm shrink-0" aria-hidden="true">
        ✅
      </span>
      {expanded && (
        <span className="flex-1 text-left flex items-center justify-between truncate">
          <span className="truncate">My Work</span>
          {count > 0 && <span className="text-[10px] px-1.5 py-0.5 rounded-full bg-blue-100 dark:bg-blue-900/40 text-blue-700 dark:text-blue-300 font-mono font-bold">{count}</span>}
        </span>
      )}
    </button>
  );
};

export const MyWork: React.FC<{ query?: string; navigate?: (path: string) => void }> = ({ query, navigate }) => {
  const params = useMemo(() => new URLSearchParams(query ?? (typeof window !== "undefined" ? window.location.search : "")), [query]);
  const [items, setItems] = useState<WorkItem[]>([]);
  const [counts, setCounts] = useState<Record<string, number>>({});
  const [loaded, setLoaded] = useState(false);
  const [tab, setTab] = useState<WorkItem["kind"]>("decision");
  const [workspace, setWorkspace] = useState(params.get("workspace") ?? "");
  const [app, setApp] = useState("");
  const [overdue, setOverdue] = useState(false);
  const [actingFor, setActingFor] = useState("");
  const [search, setSearch] = useState("");
  const [openId, setOpenId] = useState<string | null>(null);
  const [detail, setDetail] = useState<Detail | null>(null);
  const [comment, setComment] = useState("");
  const [fields, setFields] = useState<Record<string, string>>({});
  const [error, setError] = useState<string | null>(null);
  const rows = useRef<Record<string, HTMLDivElement | null>>({});
  const drawer = useRef<HTMLDivElement | null>(null);
  const wantInstance = useRef(params.get("instance"));

  const load = useCallback(async () => {
    const [list, summary] = await Promise.all([
      getJson<{ items?: WorkItem[] }>("/api/v1/me/work?limit=200"),
      getJson<{ by_kind?: Record<string, number> }>("/api/v1/me/work/summary"),
    ]);
    if (list && Array.isArray(list.items)) setItems(list.items);
    if (summary?.by_kind && typeof summary.by_kind === "object") setCounts(summary.by_kind);
    setLoaded(true);
    // A link from a notice names the instance: its item opens in the drawer.
    if (list && Array.isArray(list.items) && wantInstance.current) {
      const hit = list.items.find((i) => i.instance_id === wantInstance.current);
      wantInstance.current = null;
      if (hit) {
        setTab(hit.kind);
        setOpenId(hit.id);
      }
    }
  }, []);

  useEffect(() => {
    load();
    return onMeEvent((e) => {
      if (e.event === "work") load();
    });
  }, [load]);

  const visible = useMemo(() => {
    const q = search.trim().toLowerCase();
    return items.filter(
      (w) =>
        w.kind === tab &&
        (!workspace || w.workspace_id === workspace) &&
        (!app || w.app_slug === app) &&
        (!overdue || isOverdue(w)) &&
        (!actingFor || w.acting_for === actingFor) &&
        (!q || w.title.toLowerCase().includes(q))
    );
  }, [items, tab, workspace, app, overdue, actingFor, search]);

  const open = items.find((w) => w.id === openId) ?? null;

  useEffect(() => {
    setDetail(null);
    setComment("");
    setFields({});
    setError(null);
    if (!openId) return;
    let live = true;
    getJson<Detail>(`/api/v1/me/work/${openId}`).then((d) => {
      if (live) setDetail(d);
    });
    drawer.current?.focus();
    return () => {
      live = false;
    };
  }, [openId]);

  const close = (focusId: string | null) => {
    setOpenId(null);
    if (focusId) setTimeout(() => rows.current[focusId]?.focus(), 0);
  };

  /** The item after `id` in the list as it was, or null at the end. */
  const nextAfter = (id: string, list: WorkItem[]) => {
    const i = list.findIndex((w) => w.id === id);
    return list.slice(i + 1)[0]?.id ?? null;
  };

  const act = async (body: Record<string, unknown>) => {
    if (!open) return;
    setError(null);
    const res = await fetch(`/api/v1/me/work/${open.id}/act`, { method: "POST", headers: { "Content-Type": "application/json", ...headers() }, body: JSON.stringify(body) });
    if (!res.ok) {
      setError(`That was not done (HTTP ${res.status}).`);
      return;
    }
    const next = nextAfter(open.id, visible);
    setItems((prev) => prev.filter((w) => w.id !== open.id));
    setCounts((prev) => ({ ...prev, [open.kind]: Math.max(0, (prev[open.kind] ?? 1) - 1) }));
    if (next) setOpenId(next);
    else close(null);
  };

  const reject = () => {
    if (detail?.reject_comment_required && !comment.trim()) {
      setError("Say why you reject.");
      return;
    }
    act(comment.trim() ? { decision: "reject", comment: comment.trim() } : { decision: "reject" });
  };

  const required = detail?.require ?? [];
  const fillable = detail?.fill ?? [];
  const canFinish = required.every((f) => (fields[f] ?? "").trim() !== "");

  const finish = () => {
    const values: Record<string, string> = {};
    for (const f of fillable) if ((fields[f] ?? "").trim()) values[f] = fields[f].trim();
    act({ decision: "done", fields: values });
  };

  const workspaces = Array.from(new Set(items.map((w) => w.workspace_id).filter(Boolean))) as string[];
  const apps = Array.from(new Set(items.map((w) => w.app_slug).filter(Boolean))) as string[];
  const delegators = Array.from(new Set(items.map((w) => w.acting_for).filter(Boolean))) as string[];
  const remaining = visible.length;
  const field = "px-2 py-1 rounded border border-slate-300 dark:border-slate-700 bg-white dark:bg-slate-800 text-xs";

  return (
    <div data-testid="my-work" className="space-y-4">
      <h1 className="text-lg font-bold text-slate-900 dark:text-white">My Work</h1>
      <div role="tablist" aria-label="My Work" className="flex flex-wrap gap-2">
        {TABS.map((t) => (
          <button
            key={t.kind}
            type="button"
            role="tab"
            aria-selected={tab === t.kind}
            onClick={() => setTab(t.kind)}
            className={`px-3 py-1.5 rounded-lg text-xs font-semibold cursor-pointer ${tab === t.kind ? "bg-blue-600 text-white" : "bg-slate-100 dark:bg-slate-800 text-slate-700 dark:text-slate-300"}`}
          >
            {`${t.label} (${counts[t.kind] ?? 0})`}
          </button>
        ))}
      </div>

      <div className="flex flex-wrap items-end gap-2 text-xs">
        <label className="flex flex-col">
          <span className="text-slate-500">Workspace</span>
          <select aria-label="Workspace" value={workspace} onChange={(e) => setWorkspace(e.target.value)} className={field}>
            <option value="">All</option>
            {workspaces.map((w) => (
              <option key={w} value={w}>
                {w}
              </option>
            ))}
          </select>
        </label>
        <label className="flex flex-col">
          <span className="text-slate-500">App</span>
          <select aria-label="App" value={app} onChange={(e) => setApp(e.target.value)} className={field}>
            <option value="">All</option>
            {apps.map((a) => (
              <option key={a} value={a}>
                {a}
              </option>
            ))}
          </select>
        </label>
        {delegators.length > 0 && (
          <label className="flex flex-col">
            <span className="text-slate-500">Acting for</span>
            <select aria-label="Acting for" value={actingFor} onChange={(e) => setActingFor(e.target.value)} className={field}>
              <option value="">Anyone</option>
              {delegators.map((d) => (
                <option key={d} value={d}>
                  {d}
                </option>
              ))}
            </select>
          </label>
        )}
        <label className="flex items-center gap-1">
          <input type="checkbox" checked={overdue} onChange={(e) => setOverdue(e.target.checked)} />
          <span>Overdue only</span>
        </label>
        <label className="flex flex-col">
          <span className="text-slate-500">Search by title</span>
          <input aria-label="Search by title" value={search} onChange={(e) => setSearch(e.target.value)} className={field} />
        </label>
      </div>

      <div data-testid="work-list" className="max-h-[60vh] overflow-y-auto divide-y divide-slate-100 dark:divide-slate-800 rounded-lg border border-slate-200 dark:border-slate-800">
        {loaded && visible.length === 0 && <p className="p-6 text-center text-xs text-slate-500">Nothing is waiting on you.</p>}
        {visible.map((w) => {
          const due = dueOf(w);
          const late = isOverdue(w);
          const acting = w.via === "delegate" && w.acting_for ? `Acting for ${w.acting_for}` : "";
          return (
            <div
              key={w.id}
              ref={(el) => {
                rows.current[w.id] = el;
              }}
              data-testid={`work-row-${w.id}`}
              role="button"
              tabIndex={0}
              aria-label={[w.title, due, acting].filter(Boolean).join(", ")}
              onClick={() => setOpenId(w.id)}
              onKeyDown={(e) => {
                if (e.key === "Enter") setOpenId(w.id);
              }}
              className="p-3 text-xs grid grid-cols-1 md:grid-cols-4 gap-1 cursor-pointer hover:bg-slate-50 dark:hover:bg-slate-800/40 focus:outline-2 focus:outline-blue-500"
            >
              <span className="font-semibold text-slate-900 dark:text-white">{w.title}</span>
              <span className="text-slate-500">
                {w.app_slug ?? ""} · {w.workspace_id ?? ""}
              </span>
              <span className={late ? "font-semibold text-rose-700 dark:text-rose-300" : "text-slate-600 dark:text-slate-300"}>
                {late && <span aria-hidden="true">⚠ </span>}
                {due}
              </span>
              <span>{acting}</span>
            </div>
          );
        })}
      </div>

      {open && (
        <div
          ref={drawer}
          role="dialog"
          aria-modal="true"
          aria-label={open.title}
          tabIndex={-1}
          onKeyDown={(e) => {
            if (e.key === "Escape") close(open.id);
          }}
          className="fixed inset-y-0 right-0 w-full max-w-md bg-white dark:bg-slate-900 border-l border-slate-200 dark:border-slate-800 p-5 space-y-3 overflow-y-auto shadow-xl text-xs"
        >
          <div className="flex items-center justify-between">
            <h2 className="text-sm font-bold text-slate-900 dark:text-white">{open.title}</h2>
            <span className="text-slate-500">{`${remaining} remaining`}</span>
          </div>
          {detail?.prompt && <p className="font-medium text-slate-800 dark:text-slate-200">{detail.prompt}</p>}
          {detail?.show && Object.keys(detail.show).length > 0 && (
            <dl className="grid grid-cols-2 gap-1">
              {Object.entries(detail.show).map(([k, v]) => (
                <React.Fragment key={k}>
                  <dt className="text-slate-500">{k}</dt>
                  <dd>{String(v ?? "")}</dd>
                </React.Fragment>
              ))}
            </dl>
          )}
          {detail?.tally && <p>{`${detail.tally.approve} of ${detail.tally.voters} have approved`}</p>}

          {open.kind === "decision" && (
            <div className="space-y-2">
              <label className="flex flex-col">
                <span className="text-slate-500">Comment{detail?.reject_comment_required ? " (needed to reject)" : ""}</span>
                <textarea aria-label="Comment" value={comment} onChange={(e) => setComment(e.target.value)} className={field} />
              </label>
              <div className="flex gap-2">
                <button type="button" onClick={() => act(comment.trim() ? { decision: "approve", comment: comment.trim() } : { decision: "approve" })} className="px-3 py-1.5 rounded bg-emerald-600 text-white font-semibold cursor-pointer">
                  Approve
                </button>
                <button type="button" onClick={reject} className="px-3 py-1.5 rounded border border-slate-300 dark:border-slate-700 font-semibold cursor-pointer">
                  Reject
                </button>
              </div>
            </div>
          )}

          {open.kind === "task" && (
            <div className="space-y-2">
              {fillable.map((f) => {
                const label = required.includes(f) ? `${f} (required)` : f;
                return (
                  <label key={f} className="flex flex-col">
                    <span className="text-slate-500">{label}</span>
                    <input aria-label={label} value={fields[f] ?? ""} onChange={(e) => setFields((p) => ({ ...p, [f]: e.target.value }))} className={field} />
                  </label>
                );
              })}
              <label className="flex flex-col">
                <span className="text-slate-500">Reason, to decline</span>
                <input aria-label="Reason, to decline" value={comment} onChange={(e) => setComment(e.target.value)} className={field} />
              </label>
              <div className="flex gap-2">
                <button type="button" disabled={!canFinish} onClick={finish} className="px-3 py-1.5 rounded bg-emerald-600 text-white font-semibold cursor-pointer disabled:opacity-50">
                  Done
                </button>
                <button type="button" onClick={() => act({ decision: "decline", reason: comment.trim() })} className="px-3 py-1.5 rounded border border-slate-300 dark:border-slate-700 font-semibold cursor-pointer">
                  Decline
                </button>
              </div>
            </div>
          )}

          {open.kind === "review" && open.proposal_id && (
            <button type="button" onClick={() => navigate?.(`/proposals?id=${open.proposal_id}`)} className="underline cursor-pointer">
              Open the proposal
            </button>
          )}

          {open.kind === "waiting" && (
            <div className="space-y-1">
              <p>Waiting on {(detail?.instance?.approvers ?? []).map((a) => a.display_name || a.eppn).join(", ") || "the people who decide"}.</p>
              <p>Since {new Date(open.created_at).toLocaleDateString("en-GB", { day: "numeric", month: "short" })}.</p>
              {detail?.instance?.next_timer?.sentence && <p>{detail.instance.next_timer.sentence}</p>}
            </div>
          )}

          {error && <p className="font-semibold text-rose-600 dark:text-rose-400">{error}</p>}
          {detail?.history && detail.history.length > 0 && (
            <ol className="space-y-0.5 text-slate-500">
              {detail.history.map((h, i) => (
                <li key={i}>{h}</li>
              ))}
            </ol>
          )}
          <div className="flex gap-2 pt-2">
            <button
              type="button"
              onClick={() => {
                const next = nextAfter(open.id, visible);
                if (next) setOpenId(next);
                else close(open.id);
              }}
              className="px-3 py-1.5 rounded border border-slate-300 dark:border-slate-700 cursor-pointer"
            >
              Skip
            </button>
            <button type="button" onClick={() => close(open.id)} className="px-3 py-1.5 rounded border border-slate-300 dark:border-slate-700 cursor-pointer">
              Close
            </button>
          </div>
        </div>
      )}
    </div>
  );
};
