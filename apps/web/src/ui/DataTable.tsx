import { useState, useEffect, useCallback, type ReactNode } from "react";
import {
  useReactTable,
  getCoreRowModel,
  flexRender,
  type ColumnDef,
} from "@tanstack/react-table";
import { Skeleton } from "./Skeleton";
import { ErrorState } from "./ErrorState";
import { EmptyState } from "./EmptyState";

export type DataTableColumn<TData, TValue = unknown> = ColumnDef<TData, TValue> & {
  sortKey?: string;
};

export interface DataTableProps<TData> {
  columns: DataTableColumn<TData, unknown>[];
  load: (
    cursor?: string,
    search?: string,
    filters?: Record<string, unknown>,
    sortKey?: string,
    sortDesc?: boolean
  ) => Promise<{ rows: TData[]; next_cursor?: string }>;
  rowKey?: keyof TData | ((row: TData) => string);
  onRowOpen?: (row: TData) => void;
  filters?: ReactNode;
  emptyState?: ReactNode;
  filterValues?: Record<string, unknown>;
}

export function DataTable<TData>({
  columns,
  load,
  rowKey,
  onRowOpen,
  filters,
  emptyState,
  filterValues,
}: DataTableProps<TData>) {
  const [rows, setRows] = useState<TData[]>([]);
  const [nextCursor, setNextCursor] = useState<string | undefined>(undefined);
  const [loading, setLoading] = useState(true);
  const [loadingMore, setLoadingMore] = useState(false);
  const [error, setError] = useState<Error | null>(null);
  const [search, setSearch] = useState("");
  const [sortKey, setSortKey] = useState<string | undefined>(undefined);
  const [sortDesc, setSortDesc] = useState(false);

  const fetchInitial = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      const res = await load(undefined, search || undefined, filterValues, sortKey, sortDesc);
      setRows(res.rows);
      setNextCursor(res.next_cursor);
    } catch (err: unknown) {
      setError(err instanceof Error ? err : new Error(String(err)));
    } finally {
      setLoading(false);
    }
  }, [load, search, filterValues, sortKey, sortDesc]);

  useEffect(() => {
    fetchInitial();
  }, [fetchInitial]);

  const handleLoadMore = async () => {
    if (!nextCursor || loadingMore) return;
    setLoadingMore(true);
    try {
      const res = await load(nextCursor, search || undefined, filterValues, sortKey, sortDesc);
      setRows((prev) => [...prev, ...res.rows]);
      setNextCursor(res.next_cursor);
    } catch (err: unknown) {
      setError(err instanceof Error ? err : new Error(String(err)));
    } finally {
      setLoadingMore(false);
    }
  };

  const handleSort = (key: string) => {
    if (sortKey === key) {
      if (sortDesc) {
        setSortKey(undefined);
        setSortDesc(false);
      } else {
        setSortDesc(true);
      }
    } else {
      setSortKey(key);
      setSortDesc(false);
    }
  };

  const handleExportCsv = () => {
    if (rows.length === 0) return;
    const leafCols = table.getVisibleLeafColumns();
    const headers = leafCols.map((c) =>
      typeof c.columnDef.header === "string" ? c.columnDef.header : c.id
    );
    const lines = [headers.join(",")];
    for (const r of rows) {
      const vals = leafCols.map((c) => {
        const val = (r as Record<string, unknown>)[c.id] ?? "";
        return JSON.stringify(String(val));
      });
      lines.push(vals.join(","));
    }
    const blob = new Blob([lines.join("\n")], { type: "text/csv;charset=utf-8;" });
    const url = URL.createObjectURL(blob);
    const a = document.createElement("a");
    a.href = url;
    a.download = "export.csv";
    a.click();
    URL.revokeObjectURL(url);
  };

  const getRowId = (originalRow: TData, index: number) => {
    if (typeof rowKey === "function") return rowKey(originalRow);
    if (typeof rowKey === "string") return String(originalRow[rowKey]);
    return String(index);
  };

  const table = useReactTable({
    data: rows,
    columns: columns as ColumnDef<TData, unknown>[],
    getCoreRowModel: getCoreRowModel(),
    getRowId,
  });

  return (
    <div className="space-y-4">
      {/* Toolbar: Search, Filters, Export */}
      <div className="flex flex-col sm:flex-row items-stretch sm:items-center justify-between gap-3">
        <div className="flex flex-1 items-center gap-3">
          <div className="relative flex-1 max-w-sm">
            <input
              type="search"
              value={search}
              onChange={(e) => setSearch(e.target.value)}
              placeholder="Search..."
              aria-label="Search records"
              className="w-full text-sm rounded-md border border-gray-300 dark:border-gray-700 bg-white dark:bg-gray-800 text-gray-900 dark:text-gray-100 px-3 py-1.5 focus:outline-none focus:ring-2 focus:ring-blue-500"
            />
          </div>
          {filters}
        </div>
        <div className="flex items-center gap-2">
          <button
            type="button"
            onClick={handleExportCsv}
            disabled={rows.length === 0}
            className="px-3 py-1.5 text-xs font-medium rounded border border-gray-300 dark:border-gray-700 hover:bg-gray-50 dark:hover:bg-gray-800 text-gray-700 dark:text-gray-300 disabled:opacity-50 transition"
          >
            Export CSV
          </button>
        </div>
      </div>

      {/* Main Table / States */}
      {error ? (
        <ErrorState error={error} onRetry={fetchInitial} />
      ) : loading ? (
        <div className="p-4 space-y-3 border border-gray-200 dark:border-gray-800 rounded-lg">
          <Skeleton rows={6} className="h-8 w-full" />
        </div>
      ) : rows.length === 0 ? (
        emptyState || (
          <EmptyState title="No records" description="No matching records were found." />
        )
      ) : (
        <div className="overflow-x-auto border border-gray-200 dark:border-gray-800 rounded-lg">
          <table className="min-w-full divide-y divide-gray-200 dark:divide-gray-800 text-left text-sm">
            <thead className="bg-gray-50 dark:bg-gray-900/80 text-gray-700 dark:text-gray-300 font-medium">
              {table.getHeaderGroups().map((headerGroup) => (
                <tr key={headerGroup.id}>
                  {headerGroup.headers.map((header) => {
                    const colDef = header.column.columnDef as DataTableColumn<TData>;
                    const sk = colDef.sortKey;
                    return (
                      <th
                        key={header.id}
                        scope="col"
                        className="px-4 py-3 select-none"
                      >
                        <div className="flex items-center gap-1.5">
                          <span>
                            {header.isPlaceholder
                              ? null
                              : flexRender(
                                  header.column.columnDef.header,
                                  header.getContext()
                                )}
                          </span>
                          {sk && (
                            <button
                              type="button"
                              onClick={() => handleSort(sk)}
                              aria-label={`Sort by ${sk}`}
                              className="text-xs text-gray-400 hover:text-gray-700 dark:hover:text-gray-200 ml-1"
                            >
                              {sortKey === sk ? (sortDesc ? "▼" : "▲") : "↕"}
                            </button>
                          )}
                        </div>
                      </th>
                    );
                  })}
                </tr>
              ))}
            </thead>
            <tbody className="divide-y divide-gray-200 dark:divide-gray-800 bg-white dark:bg-gray-900 text-gray-900 dark:text-gray-100">
              {table.getRowModel().rows.map((row) => (
                <tr
                  key={row.id}
                  tabIndex={0}
                  onClick={() => onRowOpen?.(row.original)}
                  onKeyDown={(e) => {
                    if (e.key === "Enter") {
                      e.preventDefault();
                      onRowOpen?.(row.original);
                    }
                  }}
                  className={`hover:bg-gray-50 dark:hover:bg-gray-800/60 focus:bg-blue-50/50 dark:focus:bg-blue-950/30 focus:outline-none transition-colors ${
                    onRowOpen ? "cursor-pointer" : ""
                  }`}
                >
                  {row.getVisibleCells().map((cell) => (
                    <td key={cell.id} className="px-4 py-3 whitespace-nowrap">
                      {flexRender(cell.column.columnDef.cell, cell.getContext())}
                    </td>
                  ))}
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}

      {/* Load More Button */}
      {nextCursor && !loading && (
        <div className="flex justify-center pt-2">
          <button
            type="button"
            onClick={handleLoadMore}
            disabled={loadingMore}
            className="px-4 py-2 text-sm font-medium rounded-md border border-gray-300 dark:border-gray-700 hover:bg-gray-50 dark:hover:bg-gray-800 text-gray-700 dark:text-gray-300 disabled:opacity-50 transition"
          >
            {loadingMore ? "Loading..." : "Load more"}
          </button>
        </div>
      )}
    </div>
  );
}
