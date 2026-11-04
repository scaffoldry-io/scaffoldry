// Workflows phase 8: the canvas. A second view of the one editor state, drawn as hand-written SVG from `layout`. It
// dispatches the same commands as the outline and nothing else.
//
// Keys: on a step, Enter opens the inspector, Delete removes the step, Alt+ArrowUp and Alt+ArrowDown move it,
// Alt+ArrowRight moves it into the branch above, and Alt+ArrowLeft moves it out of its branch. On a `+`, Enter opens
// the menu of steps to add. With no step selected, the arrow keys pan. Ctrl+Z undoes and Ctrl+Shift+Z redoes.

import React, { useEffect, useMemo, useRef, useState } from "react";
import type { FieldSpec, ProcessStepItem, WorkflowAutomationRule } from "../types";
import { approverWords } from "../processWords";
import { layout, JOIN, type Box, type Insert } from "./layout";
import { numberOf, placeOf, sentenceOf, StepForm, type Problem } from "./Outline";
import { stepAt, type Command, type EditorState, type Path, type StepKindName } from "./reducer";
import { SKIPPED, traceAt, traceName, traceSummary, type TraceEntry } from "./TestRun";

const small = "px-2 py-0.5 rounded border border-slate-300 dark:border-slate-700 text-[11px] text-slate-700 dark:text-slate-300 hover:bg-slate-100 dark:hover:bg-slate-800 cursor-pointer";

/** The `+` menu, grouped as the brief names the groups. Time has no step of its own: a step's due date is in its form. */
const MENU: { group: string; items: { kind: StepKindName; label: string }[]; note?: string }[] = [
  {
    group: "Data",
    items: [
      { kind: "create", label: "Create a record" },
      { kind: "update", label: "Update a record" },
      { kind: "find", label: "Find records" },
      { kind: "status", label: "Set the status" },
      { kind: "audit", label: "Record an audit entry" },
    ],
  },
  {
    group: "People",
    items: [
      { kind: "task", label: "Ask for a decision" },
      { kind: "notify", label: "Notify a role" },
    ],
  },
  { group: "Logic", items: [{ kind: "branch", label: "Branch" }] },
  { group: "Time", items: [], note: "There is no waiting step yet. Set a due date in a decision's form." },
  { group: "Connect", items: [{ kind: "start", label: "Start another flow" }] },
];

/** The kind of a box, in words. Color shows it too, but never alone. */
const KIND_WORD: Record<string, string> = { trigger: "Trigger", service: "Step", task: "Decision by a person", decision: "Branch", call: "Calls a flow", end: "End" };
const KIND_FILL: Record<string, string> = {
  trigger: "fill-blue-50 stroke-blue-400 dark:fill-blue-950 dark:stroke-blue-600",
  service: "fill-white stroke-slate-400 dark:fill-slate-900 dark:stroke-slate-500",
  task: "fill-amber-50 stroke-amber-500 dark:fill-amber-950 dark:stroke-amber-600",
  decision: "fill-violet-50 stroke-violet-500 dark:fill-violet-950 dark:stroke-violet-500",
  call: "fill-emerald-50 stroke-emerald-500 dark:fill-emerald-950 dark:stroke-emerald-600",
};

function triggerWords(flow: WorkflowAutomationRule): string {
  const t = flow.trigger as any;
  switch (t.type) {
    case "RecordCreated":
      return "Starts when a record is created";
    case "RecordUpdated":
      return "Starts when a record is updated";
    case "StatusChanged":
      return `Starts when the status changes to ${t.to_status || "…"}`;
    case "FieldChanged":
      return `Starts when ${t.field_name || "a field"} changes`;
    default:
      return "Starts";
  }
}

/** One sentence for a step's node. A decision says who decides and when it is due. */
function nodeWords(s: ProcessStepItem): string {
  const k = s.kind as any;
  if (k.UserTask) {
    const t = k.UserTask;
    const who = t.approver ? `Asked of ${approverWords(t.approver)}` : t.role ? `Asked of the role ${t.role}` : "Ask for a decision";
    return t.due?.days ? `${who}. Due ${t.due.days} days after it starts` : who;
  }
  if (k.Branch) {
    const n = k.Branch.branches.length;
    return `Branch on ${n} ${n === 1 ? "condition" : "conditions"}, or otherwise`;
  }
  return sentenceOf(s);
}

/** What a `+` is called: before a step, after the last one, or into an empty arm. */
function insertLabel(flow: WorkflowAutomationRule, path: Path): string {
  const i = path[path.length - 1];
  const parent = path.slice(0, -1);
  if (stepAt(flow, path)) return `Insert a step before step ${numberOf(path)}`;
  if (i > 0) return `Insert a step after step ${numberOf([...parent, i - 1])}`;
  if (parent.length === 0) return "Insert the first step";
  const owner = stepAt(flow, parent.slice(0, -1));
  const b = (owner?.kind as any)?.Branch;
  const arm = parent[parent.length - 1];
  const where = b && arm === b.branches.length ? "otherwise" : `condition ${arm + 1}`;
  return `Insert a step into ${where} of step ${numberOf(parent.slice(0, -1))}`;
}

/** Splits a sentence into lines that fit a node, at most three. */
function lines(text: string, max = 30): string[] {
  const out: string[] = [];
  let line = "";
  for (const word of text.split(" ")) {
    if ((line + " " + word).trim().length > max && line) {
      out.push(line);
      line = word;
    } else line = (line + " " + word).trim();
  }
  if (line) out.push(line);
  return out.length > 3 ? [...out.slice(0, 2), `${out[2]}…`] : out;
}

interface CanvasProps {
  state: EditorState;
  dispatch: (c: Command) => void;
  fields: FieldSpec[];
  appSlug: string;
  problems: Problem[];
  /** The width to draw at, in pixels. The canvas fills its container when left out. */
  width?: number;
  /** Workflows phase 9: the last test run's trace, to mark the path it took. */
  trace?: TraceEntry[] | null;
  /** Workflows phase 10: a comparison's mark for each step id, against the other version. */
  marks?: Record<string, { change: "added" | "changed" | "removed" | "same"; against: number }>;
  /** Draws the flow without editing: no `+`, no inspector, no drag and no editing keys. */
  readOnly?: boolean;
  /** The canvas's accessible name. */
  label?: string;
}

const MARK_WORD: Record<string, string> = { added: "Added", changed: "Changed", removed: "Removed" };

const OP_WORDS: Record<string, string> = { Equals: "is", NotEquals: "is not", GreaterThan: "is greater than", LessThan: "is less than", Contains: "contains" };

const ZOOM = 1.25;
const PAN = 40;

export function Canvas({ state, dispatch, fields, appSlug, problems, width, trace, marks, readOnly, label }: CanvasProps) {
  const flow = state.flow;
  const L = useMemo(() => layout(flow), [flow]);
  const [zoom, setZoom] = useState(1);
  const [pan, setPan] = useState({ x: 0, y: 0 });
  const [selected, setSelected] = useState<string | null>(null);
  const [menu, setMenu] = useState<Insert | null>(null);
  const dragging = useRef<Path | null>(null);
  const dropped = useRef(false);
  const menuRef = useRef<HTMLDivElement>(null);
  const insertRefs = useRef(new Map<string, SVGGElement>());

  const selectedBox = selected ? L.boxes.find((b) => b.key === selected && b.path) : undefined;
  const selectedStep = selectedBox?.path ? stepAt(flow, selectedBox.path) : undefined;
  useEffect(() => {
    if (selected && !selectedBox) setSelected(null);
  }, [selected, selectedBox]);
  useEffect(() => {
    if (menu) menuRef.current?.querySelector<HTMLElement>('[role="menuitem"]')?.focus();
  }, [menu]);

  const problemsAt = (path: Path | null) => (path ? problems.filter((p) => p.path && p.path.length === path.length && p.path.every((x, i) => x === path[i])) : []);
  const first = problems[0];

  const vw = L.width / zoom;
  const vh = L.height / zoom;

  const choose = (kind: StepKindName) => {
    if (!menu) return;
    dispatch({ type: "AddStep", path: menu.path, kind });
    const key = menu.key;
    setMenu(null);
    insertRefs.current.get(key)?.focus();
  };

  const onCanvasKey = (e: React.KeyboardEvent) => {
    if (e.target !== e.currentTarget) return;
    if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "z") {
      e.preventDefault();
      dispatch({ type: e.shiftKey ? "Redo" : "Undo" });
      return;
    }
    if (selected) return;
    const d: Record<string, [number, number]> = { ArrowLeft: [-PAN, 0], ArrowRight: [PAN, 0], ArrowUp: [0, -PAN], ArrowDown: [0, PAN] };
    const step = d[e.key];
    if (step) {
      e.preventDefault();
      setPan((p) => ({ x: p.x + step[0], y: p.y + step[1] }));
    }
  };

  const onNodeKey = (e: React.KeyboardEvent, box: Box) => {
    const path = box.path!;
    const i = path[path.length - 1];
    const parent = path.slice(0, -1);
    const siblings = parent.length === 0 ? flow.steps ?? [] : (() => {
      const owner = stepAt(flow, parent.slice(0, -1));
      const b = (owner?.kind as any)?.Branch;
      return b ? [...b.branches.map((a: any) => a.steps), b.otherwise][parent[parent.length - 1]] ?? [] : [];
    })();
    const above = siblings[i - 1];
    const aboveBranch = above && (above.kind as any).Branch;
    const act = (c: Command | (() => void)) => {
      e.preventDefault();
      e.stopPropagation();
      if (typeof c === "function") c();
      else dispatch(c);
    };
    if (e.key === "Enter" || e.key === " ") act(() => setSelected(box.key));
    else if (e.key === "Escape") act(() => setSelected(null));
    else if (e.key === "Delete") act({ type: "RemoveStep", path });
    else if (e.altKey && e.key === "ArrowUp" && i > 0) act({ type: "MoveStep", from: path, to: [...parent, i - 1] });
    else if (e.altKey && e.key === "ArrowDown" && i < siblings.length - 1) act({ type: "MoveStep", from: path, to: [...parent, i + 2] });
    else if (e.altKey && e.key === "ArrowRight" && aboveBranch) act({ type: "MoveStep", from: path, to: [...parent, i - 1, 0, aboveBranch.branches[0]?.steps.length ?? 0] });
    else if (e.altKey && e.key === "ArrowLeft" && path.length > 1) act({ type: "MoveStep", from: path, to: [...path.slice(0, -3), path[path.length - 3] + 1] });
  };

  const drop = (to: Path) => {
    const from = dragging.current;
    dragging.current = null;
    if (!from) return;
    dropped.current = true;
    dispatch({ type: "MoveStep", from, to });
  };

  return (
    <div role="group" aria-label={label ?? "Flow canvas"} tabIndex={0} onKeyDown={onCanvasKey} className="relative space-y-2 text-xs focus:outline-none focus:ring-2 focus:ring-blue-500 rounded-lg" style={width ? { width } : undefined}>
      <div className="flex flex-wrap items-center gap-1">
        <button type="button" className={small} aria-label="Zoom in" onClick={() => setZoom((z) => z * ZOOM)}>
          +
        </button>
        <button type="button" className={small} aria-label="Zoom out" onClick={() => setZoom((z) => z / ZOOM)}>
          −
        </button>
        <button
          type="button"
          className={small}
          onClick={() => {
            setZoom(1);
            setPan({ x: 0, y: 0 });
          }}
        >
          Fit
        </button>
        <span className="text-slate-500 ml-2">{Math.round(zoom * 100)}%</span>
      </div>
      {trace && trace.length > 0 && (
        <p role="status" aria-label="Test run summary" className="text-emerald-800 dark:text-emerald-300">
          {traceSummary(trace)}
        </p>
      )}
      <p role="status" aria-label="Canvas check" className={first ? "text-rose-700 dark:text-rose-300" : "sr-only"}>
        {first ? `${problems.length === 1 ? "1 problem" : `${problems.length} problems`}. First: ${first.path?.length ? `step ${numberOf(first.path)}: ` : ""}${first.message}` : ""}
      </p>

      <div className="flex flex-col lg:flex-row gap-3">
        <svg
          viewBox={`${pan.x} ${pan.y} ${vw} ${vh}`}
          width="100%"
          height={Math.min(L.height, 720)}
          preserveAspectRatio="xMidYMin meet"
          className="rounded-lg border border-slate-200 dark:border-slate-700 bg-slate-50 dark:bg-slate-950 select-none"
          onMouseUp={() => (dragging.current = null)}
          onMouseLeave={() => (dragging.current = null)}
        >
          {L.edges.map((e, i) => {
            const mid = (e.from.y + e.to.y) / 2;
            return <polyline key={i} points={`${e.from.x},${e.from.y} ${e.from.x},${mid} ${e.to.x},${mid} ${e.to.x},${e.to.y}`} className="fill-none stroke-slate-400 dark:stroke-slate-600" strokeWidth={1.5} aria-hidden="true" />;
          })}

          {L.labels.map((l) => {
            const b = (stepAt(flow, l.path.slice(0, -1))?.kind as any)?.Branch;
            const arm = l.path[l.path.length - 1];
            const pred = b?.branches[arm]?.when?.[0];
            const text = !b ? "" : arm >= b.branches.length ? "Otherwise" : pred?.field_name ? `If ${pred.field_name} ${OP_WORDS[pred.operator] ?? pred.operator} ${pred.expected_value}` : `Condition ${arm + 1}`;
            return (
              <text key={l.key} x={l.x} y={l.y} className="fill-violet-700 dark:fill-violet-300 text-[10px]" aria-hidden="true">
                {text.length > 28 ? `${text.slice(0, 27)}…` : text}
              </text>
            );
          })}

          {L.boxes.map((b) => {
            if (b.kind === "join") return <circle key={b.key} cx={b.x + JOIN / 2} cy={b.y + JOIN / 2} r={JOIN / 2 - 4} className="fill-violet-200 stroke-violet-500 dark:fill-violet-900" aria-hidden="true" />;
            if (b.kind === "end")
              return (
                <g key={b.key} aria-label="End" role="img">
                  <circle cx={b.x + JOIN / 2} cy={b.y + JOIN / 2} r={JOIN / 2} className="fill-slate-700 dark:fill-slate-300" />
                  <text x={b.x + JOIN + 6} y={b.y + JOIN / 2 + 4} className="fill-slate-600 dark:fill-slate-400 text-[11px]">
                    End
                  </text>
                </g>
              );
            const step = b.path ? stepAt(flow, b.path) : undefined;
            const words = b.kind === "trigger" ? triggerWords(flow) : step ? nodeWords(step) : "";
            const mine = problemsAt(b.path);
            const ran = b.path ? traceAt(trace ?? undefined, b.path) : undefined;
            const name = b.path
              ? `step ${numberOf(b.path)}: ${KIND_WORD[b.kind]}. ${words}${mine.length ? `. Problem: ${mine.map((p) => p.message).join(" ")}` : ""}${ran ? `. ${traceName(ran)}` : ""}`
              : `${KIND_WORD.trigger}. ${words}`;
            const mark = step ? marks?.[step.id] : undefined;
            const marked = mark && mark.change !== "same" ? mark : undefined;
            const fullName = marked ? `${name}. Compared with version ${marked.against}: ${marked.change}` : name;
            const skipped = ran?.outcome === SKIPPED;
            const failed = ran?.outcome === "failed";
            const runWord = !ran ? null : skipped ? "Skipped" : failed ? "Failed" : "On the path";
            const isSel = selected === b.key;
            const interactive = !!b.path;
            return (
              <g
                key={b.key}
                role={interactive ? "button" : "img"}
                aria-label={fullName}
                aria-pressed={interactive ? isSel : undefined}
                tabIndex={interactive ? 0 : undefined}
                className={interactive ? "cursor-pointer focus:outline-none" : undefined}
                onKeyDown={interactive && !readOnly ? (e) => onNodeKey(e, b) : undefined}
                onClick={interactive && !readOnly ? () => setSelected(b.key) : undefined}
                onMouseDown={interactive && !readOnly ? () => (dragging.current = b.path) : undefined}
                onMouseUp={
                  interactive && !readOnly
                    ? (e) => {
                        // Dropped on a step: it goes before that step.
                        if (dragging.current && dragging.current.join() !== b.path!.join()) {
                          e.stopPropagation();
                          drop(b.path!);
                        }
                      }
                    : undefined
                }
              >
                <rect
                  x={b.x}
                  y={b.y}
                  width={b.w}
                  height={b.h}
                  rx={b.kind === "decision" ? 18 : 8}
                  strokeWidth={isSel || (ran && !skipped) || marked ? 3 : 1.5}
                  strokeDasharray={marked?.change === "removed" ? "6 4" : undefined}
                  opacity={skipped ? 0.45 : 1}
                  className={`${KIND_FILL[b.kind]} ${mine.length || failed ? "!stroke-rose-600" : ran && !skipped ? "!stroke-emerald-600" : ""}`}
                />
                {marked && (
                  <text x={b.x + b.w - 8} y={b.y + 15} textAnchor="end" className={`text-[10px] font-bold ${marked.change === "removed" ? "fill-rose-600" : marked.change === "added" ? "fill-emerald-700 dark:fill-emerald-300" : "fill-amber-700 dark:fill-amber-300"}`}>
                    {MARK_WORD[marked.change]}
                  </text>
                )}
                {runWord && (
                  <text x={b.x + b.w - 8} y={b.y + b.h - 6} textAnchor="end" className={`text-[10px] font-semibold ${skipped ? "fill-slate-400" : failed ? "fill-rose-600" : "fill-emerald-700 dark:fill-emerald-300"}`}>
                    {runWord}
                  </text>
                )}
                <text x={b.x + 10} y={b.y + 15} className="fill-slate-500 dark:fill-slate-400 text-[10px] uppercase tracking-wide">
                  {b.path ? `${numberOf(b.path)} · ${KIND_WORD[b.kind]}` : KIND_WORD.trigger}
                </text>
                {lines(words).map((l, i) => (
                  <text key={i} x={b.x + 10} y={b.y + 31 + i * 13} className="fill-slate-900 dark:fill-slate-100 text-[11px]">
                    {l}
                  </text>
                ))}
                {mine.length > 0 && (
                  <text x={b.x + b.w - 18} y={b.y + 16} className="fill-rose-600 text-[13px]" aria-hidden="true">
                    ⚠
                  </text>
                )}
              </g>
            );
          })}

          {!readOnly && L.inserts.map((ins) => {
            const label = insertLabel(flow, ins.path);
            return (
              <g
                key={ins.key}
                ref={(el) => {
                  if (el) insertRefs.current.set(ins.key, el);
                  else insertRefs.current.delete(ins.key);
                }}
                role="button"
                aria-label={label}
                aria-haspopup="menu"
                tabIndex={0}
                className="cursor-pointer focus:outline-none"
                onClick={() => {
                  if (dropped.current) {
                    dropped.current = false;
                    return;
                  }
                  setMenu(ins);
                }}
                onKeyDown={(e) => {
                  if (e.key === "Enter" || e.key === " ") {
                    e.preventDefault();
                    e.stopPropagation();
                    setMenu(ins);
                  }
                }}
                onMouseUp={(e) => {
                  if (dragging.current) {
                    e.stopPropagation();
                    drop(ins.path);
                  }
                }}
              >
                <circle cx={ins.x} cy={ins.y} r={9} className="fill-white stroke-blue-500 dark:fill-slate-900" strokeWidth={1.5} />
                <text x={ins.x} y={ins.y + 4} textAnchor="middle" className="fill-blue-600 text-[13px] font-bold" aria-hidden="true">
                  +
                </text>
              </g>
            );
          })}
        </svg>

        {!readOnly && selectedStep && selectedBox?.path && (
          <section role="region" aria-label="Inspector" className="lg:w-80 shrink-0 rounded-lg border border-slate-200 dark:border-slate-700 bg-white dark:bg-slate-900 p-3 space-y-2">
            <div className="flex items-center justify-between">
              <h5 className="font-bold text-slate-900 dark:text-white">Step {numberOf(selectedBox.path)}</h5>
              <button type="button" className={small} onClick={() => setSelected(null)}>
                Close
              </button>
            </div>
            <StepForm step={selectedStep} path={selectedBox.path} place={placeOf(flow, selectedBox.path)} ctx={{ dispatch, fields, appSlug, problems, flow }} />
            {problemsAt(selectedBox.path).map((p, i) => (
              <p key={i} className="text-rose-700 dark:text-rose-300">
                {p.message}
              </p>
            ))}
          </section>
        )}
      </div>

      {menu && (
        <div
          ref={menuRef}
          role="menu"
          aria-label="Add a step"
          className="absolute z-10 left-4 top-12 w-72 rounded-lg border border-slate-200 dark:border-slate-700 bg-white dark:bg-slate-900 shadow-lg p-2 space-y-2"
          onKeyDown={(e) => {
            if (e.key === "Escape") {
              e.preventDefault();
              const key = menu.key;
              setMenu(null);
              insertRefs.current.get(key)?.focus();
            }
          }}
        >
          <p className="text-slate-500">{insertLabel(flow, menu.path)}</p>
          {MENU.map((g) => (
            <div key={g.group} role="group" aria-label={g.group} className="space-y-0.5">
              <p className="text-[10px] uppercase font-bold tracking-wide text-slate-400">{g.group}</p>
              {g.items.map((it) => (
                <button
                  key={it.kind}
                  type="button"
                  role="menuitem"
                  className="block w-full text-left px-2 py-1 rounded hover:bg-slate-100 dark:hover:bg-slate-800 focus:bg-slate-100 dark:focus:bg-slate-800 cursor-pointer"
                  onClick={() => choose(it.kind)}
                  onKeyDown={(e) => {
                    if (e.key === "Enter" || e.key === " ") {
                      e.preventDefault();
                      choose(it.kind);
                    }
                  }}
                >
                  {it.label}
                </button>
              ))}
              {g.note && <p className="px-2 text-slate-500">{g.note}</p>}
            </div>
          ))}
          <button type="button" className={small} onClick={() => setMenu(null)}>
            Cancel
          </button>
        </div>
      )}
    </div>
  );
}
