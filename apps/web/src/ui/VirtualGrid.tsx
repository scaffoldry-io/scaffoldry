import { useVirtualizer } from "@tanstack/react-virtual";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";

/** What the server says about how many rows a view holds. */
export type RowCount = { value: number; exact: true } | { at_least: number };

/** `1,000,000 records`, or `Over 250,000 records` when the count ran out of time. */
export function rowCountText(count: RowCount): string {
  if ("at_least" in count) {
    return count.at_least > 0 ? `Over ${count.at_least.toLocaleString("en-US")} records` : "Too many records to count";
  }
  return `${count.value.toLocaleString("en-US")} ${count.value === 1 ? "record" : "records"}`;
}

export interface VirtualGridColumn {
  key: string;
  header: string;
}

export interface VirtualGridProps<T> {
  /** Names the grid for assistive technology. */
  label: string;
  columns: VirtualGridColumn[];
  /** How many rows the view holds. */
  total: number;
  /** Asks the server for the rows from `offset`. The server filters, sorts and counts. This only draws. */
  fetchWindow: (offset: number, limit: number) => Promise<T[]>;
  windowSize?: number;
  rowHeight?: number;
  viewportHeight?: number;
  /** Rows drawn beyond the edge of the viewport on each side. */
  overscan?: number;
  /**
   * The tallest the scroll area may be. A browser stops honoring an element past about 17 million pixels in some cases
   * and 33 million in others, so a longer list scrolls faster than its pixels, and the grid keeps its own offset.
   */
  maxScrollPx?: number;
}

const cell = (row: unknown, key: string): string => {
  const v = (row as Record<string, unknown>)?.[key];
  return v === null || v === undefined ? "" : String(v);
};

/**
 * A table of any length. It draws the rows in view and a few beyond, asks the server for the window of 200 under
 * them, and shows a skeleton row until that window arrives. Page Down, Page Up, Ctrl+Home and Ctrl+End move around.
 * `aria-rowcount` is every record and the header, and `aria-rowindex` is the row's place in the whole list.
 */
export function VirtualGrid<T>({
  label,
  columns,
  total,
  fetchWindow,
  windowSize = 200,
  rowHeight = 32,
  viewportHeight = 480,
  overscan = 8,
  maxScrollPx = 10_000_000,
}: VirtualGridProps<T>) {
  const viewport = useRef<HTMLDivElement>(null);
  const windows = useRef(new Map<number, T[]>());
  const asked = useRef(new Set<number>());
  const [, bump] = useState(0);
  const [failed, setFailed] = useState<string | null>(null);

  const full = total * rowHeight;
  const scrollPx = Math.min(full, maxScrollPx);
  const ratio = scrollPx > 0 ? full / scrollPx : 1;
  const ratioRef = useRef(ratio);
  ratioRef.current = ratio;

  const virtualizer = useVirtualizer({
    count: total,
    getScrollElement: () => viewport.current,
    estimateSize: () => rowHeight,
    overscan,
    // The element's scroll position is in real pixels. The list is in the pixels it would have if it were whole.
    observeElementOffset: (instance, cb) => {
      const el = instance.scrollElement;
      if (!el) return;
      const read = (scrolling: boolean) => cb(el.scrollTop * ratioRef.current, scrolling);
      read(false);
      const onScroll = () => read(true);
      el.addEventListener("scroll", onScroll, { passive: true });
      return () => el.removeEventListener("scroll", onScroll);
    },
    scrollToFn: (offset, _options, instance) => {
      instance.scrollElement?.scrollTo?.({ top: offset / ratioRef.current });
    },
  });

  const items = virtualizer.getVirtualItems();
  const offset = virtualizer.scrollOffset ?? 0;

  const load = useCallback(
    (start: number) => {
      if (asked.current.has(start) || windows.current.has(start)) return;
      asked.current.add(start);
      fetchWindow(start, windowSize)
        .then((rows) => {
          windows.current.set(start, rows);
          bump((n) => n + 1);
        })
        .catch((e: unknown) => {
          asked.current.delete(start);
          setFailed(e instanceof Error ? e.message : "The rows did not load");
        });
    },
    [fetchWindow, windowSize]
  );

  const starts = useMemo(() => {
    const set = new Set<number>();
    for (const v of items) set.add(Math.floor(v.index / windowSize) * windowSize);
    return [...set];
  }, [items, windowSize]);

  useEffect(() => {
    starts.forEach(load);
  }, [starts, load]);

  // The library clamps its own jumps to the real height of the element, which is shorter than the list. So the grid
  // moves the element itself, in real pixels.
  const goTo = (virtualOffset: number) => {
    const top = Math.min(Math.max(0, virtualOffset), Math.max(0, full - viewportHeight)) / ratioRef.current;
    viewport.current?.scrollTo?.({ top });
  };

  const onKeyDown = (e: React.KeyboardEvent) => {
    if (e.key === "PageDown") {
      e.preventDefault();
      goTo(offset + viewportHeight);
    } else if (e.key === "PageUp") {
      e.preventDefault();
      goTo(offset - viewportHeight);
    } else if (e.ctrlKey && e.key === "Home") {
      e.preventDefault();
      goTo(0);
    } else if (e.ctrlKey && e.key === "End") {
      e.preventDefault();
      goTo(full);
    }
  };

  return (
    <div role="grid" aria-label={label} aria-rowcount={total + 1} aria-colcount={columns.length} tabIndex={0} onKeyDown={onKeyDown} className="border rounded">
      <div role="row" aria-rowindex={1} className="flex font-semibold bg-gray-50 dark:bg-gray-800 border-b">
        {columns.map((c, i) => (
          <div key={c.key} role="columnheader" aria-colindex={i + 1} className="flex-1 px-3 py-2">
            {c.header}
          </div>
        ))}
      </div>
      {failed && (
        <div role="alert" className="px-3 py-2 text-sm text-red-700">
          {failed}
        </div>
      )}
      <div ref={viewport} role="rowgroup" data-viewport data-offset={Math.round(offset)} style={{ height: viewportHeight, overflow: "auto", position: "relative" }}>
        <div style={{ height: scrollPx, position: "relative" }}>
          <div style={{ position: "sticky", top: 0, height: viewportHeight, overflow: "hidden" }}>
            {items.map((v) => {
              const start = Math.floor(v.index / windowSize) * windowSize;
              const row = windows.current.get(start)?.[v.index - start];
              const style = { position: "absolute" as const, left: 0, right: 0, top: 0, height: rowHeight, transform: `translateY(${v.start - offset}px)` };
              if (row === undefined) {
                return (
                  <div key={v.key} role="row" aria-rowindex={v.index + 2} aria-busy="true" style={style} className="flex">
                    {columns.map((c, i) => (
                      <div key={c.key} role="gridcell" aria-colindex={i + 1} className="flex-1 px-3 py-2">
                        <span aria-hidden="true" className="block h-3 w-3/4 animate-pulse rounded bg-gray-200 dark:bg-gray-700" />
                        <span className="sr-only">Loading</span>
                      </div>
                    ))}
                  </div>
                );
              }
              return (
                <div key={v.key} role="row" aria-rowindex={v.index + 2} style={style} className="flex border-b">
                  {columns.map((c, i) => (
                    <div key={c.key} role="gridcell" aria-colindex={i + 1} className="flex-1 px-3 py-2 truncate">
                      {cell(row, c.key)}
                    </div>
                  ))}
                </div>
              );
            })}
          </div>
        </div>
      </div>
    </div>
  );
}
