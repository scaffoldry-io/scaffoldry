// Workflows phase 8: where the canvas draws each part of a flow. A pure function of the flow, so the same flow always
// draws the same. Steps stack top to bottom. A branch is a decision node with its arms in columns side by side and a
// join where they meet again. Flows are block-structured, so no edge crosses another and nothing is placed by hand.

import type { ProcessStepItem, WorkflowAutomationRule } from "../types";
import type { Path } from "./reducer";

export type BoxKind = "trigger" | "service" | "task" | "decision" | "join" | "call" | "end";

export interface Box {
  /** Stable within one layout: the step id, or a name for the trigger, a join and the end. */
  key: string;
  kind: BoxKind;
  /** The step's path, for a box that is a step. */
  path: Path | null;
  x: number;
  y: number;
  w: number;
  h: number;
}

/** A place a step can be added or moved to: the insertion path the reducer takes. */
export interface Insert {
  key: string;
  path: Path;
  x: number;
  y: number;
}

export interface Edge {
  from: { x: number; y: number };
  to: { x: number; y: number };
}

/** Where an arm of a branch is named: the branch's path and the arm's index, with `otherwise` last. */
export interface ArmLabel {
  key: string;
  path: Path;
  x: number;
  y: number;
}

export interface Layout {
  boxes: Box[];
  labels: ArmLabel[];
  inserts: Insert[];
  edges: Edge[];
  width: number;
  height: number;
}

export const NODE_W = 220;
export const NODE_H = 60;
export const JOIN = 24;
export const GAP_Y = 44;
export const GAP_X = 28;
const MARGIN = 24;

type Arms = { branches: { steps: ProcessStepItem[] }[]; otherwise: ProcessStepItem[] };
const branchOf = (s: ProcessStepItem): Arms | undefined => (s.kind as any)?.Branch;
const armsOf = (b: Arms) => [...b.branches.map((a) => a.steps), b.otherwise];

function kindOf(s: ProcessStepItem): BoxKind {
  const k = s.kind as any;
  if (k.Branch) return "decision";
  if (k.UserTask) return "task";
  if (k.Service?.action?.StartFlow) return "call";
  return "service";
}

/** The width a list of steps needs. */
function listWidth(steps: ProcessStepItem[]): number {
  return Math.max(NODE_W, ...steps.map(stepWidth));
}

function stepWidth(s: ProcessStepItem): number {
  const b = branchOf(s);
  if (!b) return NODE_W;
  const cols = armsOf(b).map(listWidth);
  return Math.max(NODE_W, cols.reduce((a, c) => a + c, 0) + GAP_X * (cols.length - 1));
}

/** The height of a list of steps, gaps between them included. An empty list keeps room for its insertion point. */
function listHeight(steps: ProcessStepItem[]): number {
  if (steps.length === 0) return JOIN;
  return steps.reduce((h, s) => h + stepHeight(s), 0) + GAP_Y * (steps.length - 1);
}

function stepHeight(s: ProcessStepItem): number {
  const b = branchOf(s);
  if (!b) return NODE_H;
  return NODE_H + GAP_Y + Math.max(...armsOf(b).map(listHeight)) + GAP_Y + JOIN;
}

export function layout(flow: WorkflowAutomationRule): Layout {
  const steps = flow.steps ?? [];
  const out: Layout = { boxes: [], labels: [], inserts: [], edges: [], width: 0, height: 0 };
  const width = listWidth(steps);
  const cx = MARGIN + width / 2;

  out.boxes.push({ key: "trigger", kind: "trigger", path: null, x: cx - NODE_W / 2, y: MARGIN, w: NODE_W, h: NODE_H });
  const top = MARGIN + NODE_H + GAP_Y;
  const bottom = placeList(steps, [], cx, top, { x: cx, y: MARGIN + NODE_H }, out);
  const endY = Math.max(bottom.y, top) + GAP_Y;
  out.boxes.push({ key: "end", kind: "end", path: null, x: cx - JOIN / 2, y: endY, w: JOIN, h: JOIN });
  out.edges.push({ from: bottom, to: { x: cx, y: endY } });
  out.width = width + MARGIN * 2;
  out.height = endY + JOIN + MARGIN;
  return out;
}

/**
 * Places `steps` centered on `cx`, starting at `y`, joined to `from`. Returns the point the next thing joins to. An
 * insertion point sits in the gap before each step and after the last.
 */
function placeList(steps: ProcessStepItem[], parent: Path, cx: number, y: number, from: { x: number; y: number }, out: Layout): { x: number; y: number } {
  let at = from;
  let cy = y;
  const insertAt = (i: number, py: number) => out.inserts.push({ key: `ins:${[...parent, i].join(".")}`, path: [...parent, i], x: cx, y: py });
  if (steps.length === 0) {
    insertAt(0, y + JOIN / 2);
    out.edges.push({ from: at, to: { x: cx, y: y + JOIN } });
    return { x: cx, y: y + JOIN };
  }
  steps.forEach((s, i) => {
    const path = [...parent, i];
    insertAt(i, cy - GAP_Y / 2);
    const b = branchOf(s);
    out.boxes.push({ key: s.id, kind: kindOf(s), path, x: cx - NODE_W / 2, y: cy, w: NODE_W, h: NODE_H });
    out.edges.push({ from: at, to: { x: cx, y: cy } });
    if (!b) {
      at = { x: cx, y: cy + NODE_H };
      cy += NODE_H + GAP_Y;
      return;
    }
    // Arms in columns, left to right, centered under the decision.
    const arms = armsOf(b);
    const widths = arms.map(listWidth);
    const total = widths.reduce((a, c) => a + c, 0) + GAP_X * (arms.length - 1);
    let left = cx - total / 2;
    const armTop = cy + NODE_H + GAP_Y;
    const armsBottom = armTop + Math.max(...arms.map(listHeight));
    const joinY = armsBottom + GAP_Y;
    const ends = arms.map((arm, k) => {
      const acx = left + widths[k] / 2;
      left += widths[k] + GAP_X;
      out.labels.push({ key: `arm:${s.id}:${k}`, path: [...path, k], x: acx + 14, y: armTop - 6 });
      return placeList(arm, [...path, k], acx, armTop, { x: cx, y: cy + NODE_H }, out);
    });
    out.boxes.push({ key: `join:${s.id}`, kind: "join", path: null, x: cx - JOIN / 2, y: joinY, w: JOIN, h: JOIN });
    ends.forEach((e) => out.edges.push({ from: e, to: { x: cx, y: joinY } }));
    at = { x: cx, y: joinY + JOIN };
    cy = joinY + JOIN + GAP_Y;
  });
  insertAt(steps.length, cy - GAP_Y / 2);
  return at;
}
