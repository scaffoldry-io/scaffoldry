import { useCallback, useRef, useState } from "react";
import { listLinkOptions } from "./api";
import { LinkChips, LinkPicker } from "./LinkPicker";
import { Banner } from "./ui";

type Linked = { id: string; label: string };

/** The most labels one paste resolves. A longer paste is cut here and says so. */
export const MAX_PASTED_LABELS = 50;
/** Pages of picker options read per label. The search is a contains-match, so an exact match can sit past page one. */
const MAX_PAGES = 4;

export type Resolution = { label: string } & ({ found: Linked } | { problem: "not_found" } | { problem: "several"; count: number } | { problem: "too_many" });

/**
 * Turns one label into a record by exact match on the target's primary field, through the link picker's search, so
 * it sees only the records the person may link to. Nothing is guessed: no match and several matches are reported.
 */
export async function resolveLabel(appSlug: string, tableId: string, label: string): Promise<Resolution> {
  const exact: Linked[] = [];
  let cursor: string | undefined;
  for (let page = 0; page < MAX_PAGES; page++) {
    const res = await listLinkOptions(appSlug, tableId, label, cursor);
    exact.push(...res.options.filter((o) => o.label === label).map((o) => ({ id: o.id, label: o.label })));
    if (!res.next_cursor) {
      cursor = undefined;
      break;
    }
    cursor = res.next_cursor;
  }
  if (exact.length === 1) return { label, found: exact[0] };
  if (exact.length > 1) return { label, problem: "several", count: exact.length };
  return cursor ? { label, problem: "too_many" } : { label, problem: "not_found" };
}

/** The labels in pasted text: one per line or per comma, trimmed, each once. */
export function pastedLabels(text: string): string[] {
  const out: string[] = [];
  for (const part of text.split(/[\n\r\t,]+/).map((s) => s.trim())) {
    if (part && !out.includes(part)) out.push(part);
  }
  return out;
}

function why(r: Resolution): string {
  if ("found" in r) return "";
  if (r.problem === "several") return `"${r.label}" matches ${r.count} records.`;
  if (r.problem === "too_many") return `"${r.label}" is part of too many labels to check. Pick it in the list.`;
  return `"${r.label}" was not found.`;
}

export interface LinkCellProps {
  appSlug: string;
  /** The table the link points at. */
  tableId: string;
  /** The field's label. It names the cell. */
  label: string;
  value: Linked[];
  onChange: (next: Linked[]) => void;
  cardinality?: "one" | "many";
  canOpen?: (id: string) => boolean;
  onOpen?: (id: string) => void;
  readOnly?: boolean;
  /** The grid's handle on the focused cell. The grid moves focus here, so keys and paste start in the cell. */
  cellRef?: (el: HTMLElement | null) => void;
}

/**
 * A link column's cell. It shows chips. Enter opens the picker, and Escape closes it and returns focus to the cell.
 * Pasting labels links each one that matches exactly one record, and a banner names the ones it did not link.
 */
export function LinkCell({ appSlug, tableId, label, value, onChange, cardinality = "many", canOpen, onOpen, readOnly, cellRef }: LinkCellProps) {
  const cell = useRef<HTMLDivElement | null>(null);
  const setCell = useCallback(
    (el: HTMLDivElement | null) => {
      cell.current = el;
      cellRef?.(el);
    },
    [cellRef],
  );
  const [editing, setEditing] = useState(false);
  const [report, setReport] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  const close = () => {
    setEditing(false);
    // The picker unmounts on this render, so focus goes back after it.
    setTimeout(() => cell.current?.focus(), 0);
  };

  const paste = async (text: string) => {
    const all = pastedLabels(text);
    if (all.length === 0) return;
    const labels = all.slice(0, MAX_PASTED_LABELS);
    setBusy(true);
    try {
      const results = await Promise.all(labels.map((l) => resolveLabel(appSlug, tableId, l)));
      const found = results.flatMap((r) => ("found" in r ? [r.found] : []));
      const next = cardinality === "one" ? found.slice(-1) : [...value, ...found.filter((f) => !value.some((v) => v.id === f.id))];
      const problems = results.filter((r) => !("found" in r));
      const lines: string[] = [];
      if (cardinality === "one" && found.length > 1) lines.push(`This field holds one record, so only "${found[found.length - 1].label}" was linked.`);
      if (problems.length > 0) lines.push(`${problems.length} ${problems.length === 1 ? "label was" : "labels were"} not linked: ${problems.map(why).join(" ")}`);
      if (all.length > labels.length) lines.push(`Only the first ${MAX_PASTED_LABELS} labels were read.`);
      setReport(lines.length ? lines.join(" ") : null);
      if (found.length > 0) onChange(next);
    } catch (e) {
      setReport(e instanceof Error ? `The labels could not be checked: ${e.message}` : "The labels could not be checked.");
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="relative">
      <div
        ref={setCell}
        data-link-cell=""
        role="button"
        tabIndex={0}
        aria-label={label}
        aria-haspopup="listbox"
        aria-expanded={editing}
        aria-busy={busy}
        className="min-h-[1.25rem] focus:outline focus:outline-2 focus:outline-blue-500 rounded"
        onKeyDown={(e) => {
          if (e.target !== e.currentTarget || readOnly) return;
          if (e.key === "Enter") {
            e.preventDefault();
            setEditing(true);
          }
        }}
        onPaste={(e) => {
          if (readOnly) return;
          e.preventDefault();
          void paste(e.clipboardData.getData("text/plain"));
        }}
      >
        {!editing && <LinkChips value={value} canOpen={canOpen} onOpen={onOpen} />}
      </div>
      {editing && (
        <div className="absolute z-50 left-0 top-full mt-1 min-w-[220px] rounded-lg border bg-white dark:bg-slate-800 p-2 shadow-xl">
          <LinkPicker
            appSlug={appSlug}
            tableId={tableId}
            value={value}
            cardinality={cardinality}
            canOpen={canOpen}
            onOpen={onOpen}
            autoFocus
            onEscape={close}
            onChange={(next) => {
              onChange(next);
              if (cardinality === "one") close();
            }}
          />
        </div>
      )}
      {report && (
        <Banner kind="warning" label="Paste result">
          {report}
        </Banner>
      )}
    </div>
  );
}
