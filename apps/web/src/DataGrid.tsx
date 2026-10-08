import React, { useState, useRef, useEffect, useMemo, useCallback } from "react";
import {
  useReactTable,
  getCoreRowModel,
  flexRender,
  ColumnDef,
} from "@tanstack/react-table";
import { FieldSpec, RowDensity, FieldType, AppView, AppTable } from "./types";

export type CellAddr = { recordId: string; fieldName: string };

export type SelectionRange = {
  anchor: CellAddr;
  focus: CellAddr;
};

export type DataGridProps = {
  fields: FieldSpec[];
  records: Record<string, unknown>[];
  tables?: AppTable[];
  rowDensity?: RowDensity;
  column_order?: string[];
  column_widths?: [string, number][];
  hidden_columns?: string[];
  frozen_through?: string;
  column_summary?: [string, string][];
  onViewChange?: (patch: Partial<AppView>) => void;
  onPatch: (recordId: string, fieldName: string, value: unknown) => void;
};

type PatchRecord = {
  recordId: string;
  fieldName: string;
  oldValue: unknown;
  newValue: unknown;
};

const READ_ONLY_TYPES = new Set<FieldType>([
  "Formula",
  "Lookup",
  "Count",
  "Rollup",
  "Autonumber",
  "CreatedTime",
  "LastModifiedTime",
]);

function isReadOnlyField(field?: FieldSpec): boolean {
  if (!field) return true;
  return READ_ONLY_TYPES.has(field.field_type);
}

function formatDisplayValue(field: FieldSpec, value: unknown, tables?: AppTable[]): string {
  if (value === null || value === undefined || value === "") {
    return "—";
  }
  if (field.field_type === "Relation") {
    const targetTable = tables?.find((t) => t.id === field.target_table_id);
    const displayCol = field.target_display_field || "name";
    const prefix = field.display_label_override || "";
    const ids = Array.isArray(value)
      ? value
      : typeof value === "string" && value.includes(",")
      ? value.split(",").map((s) => s.trim())
      : [String(value)];
    if (targetTable && targetTable.records) {
      return ids
        .map((id) => {
          const m = targetTable.records?.find(
            (r) => String(r.id) === String(id) || String(r.key) === String(id)
          );
          if (m) {
            return `${prefix}${m[displayCol] || m.name || m.title || id}`;
          }
          return `${prefix}${id}`;
        })
        .join(", ");
    }
    return ids.map((id) => `${prefix}${id}`).join(", ");
  }
  if (field.field_type === "Currency") {
    const symbol = field.currency_symbol || "$";
    const precision = field.precision ?? 2;
    const num = Number(value);
    return isNaN(num) ? String(value) : `${symbol}${num.toFixed(precision)}`;
  }
  if (field.field_type === "Percent") {
    return `${value}%`;
  }
  if (field.field_type === "Number") {
    const num = Number(value);
    return isNaN(num) ? String(value) : num.toLocaleString();
  }
  if (Array.isArray(value)) {
    return value.join(", ");
  }
  return String(value);
}

function getInputType(fieldType: FieldType): string {
  switch (fieldType) {
    case "Email":
      return "email";
    case "Phone":
      return "tel";
    case "Url":
      return "url";
    case "Number":
    case "Currency":
    case "Percent":
      return "number";
    case "Date":
      return "date";
    default:
      return "text";
  }
}

function getDensityClasses(rowDensity?: RowDensity): string {
  switch (rowDensity) {
    case "compact":
      return "py-1 px-2.5 text-xs";
    case "tall":
      return "py-3 px-3.5 text-sm";
    case "extra_tall":
      return "py-4 px-4 text-sm";
    case "medium":
    default:
      return "py-2 px-3 text-xs";
  }
}

export function DataGrid({
  fields,
  records,
  tables,
  rowDensity = "medium",
  column_order,
  column_widths,
  hidden_columns,
  frozen_through,
  column_summary,
  onViewChange,
  onPatch,
}: DataGridProps) {
  const [selection, setSelection] = useState<SelectionRange | null>(null);
  const [isEditing, setIsEditing] = useState<boolean>(false);
  const [activeLinkPicker, setActiveLinkPicker] = useState<{ recordId: string; fieldName: string } | null>(null);
  const [copyToast, setCopyToast] = useState<string | null>(null);
  const isMouseDownRef = useRef<boolean>(false);
  const [editValue, setEditValue] = useState<string>("");
  const [multiSelectOpen, setMultiSelectOpen] = useState<boolean>(false);
  const [draftMultiSelect, setDraftMultiSelect] = useState<string[]>([]);
  const [activeHeaderMenu, setActiveHeaderMenu] = useState<string | null>(null);
  const [draggedColName, setDraggedColName] = useState<string | null>(null);
  const isCancellingRef = useRef<boolean>(false);
  const inputRef = useRef<HTMLInputElement | HTMLSelectElement | null>(null);
  const activeCellRef = useRef<HTMLTableCellElement | null>(null);

  const undoStack = useRef<PatchRecord[][]>([]);
  const redoStack = useRef<PatchRecord[][]>([]);

  // Visible and Ordered Fields
  const visibleFields = useMemo(() => {
    const hiddenSet = new Set(hidden_columns || []);
    const unhidden = fields.filter((f) => !hiddenSet.has(f.name));

    if (column_order && column_order.length > 0) {
      const orderMap = new Map(column_order.map((name, idx) => [name, idx]));
      return [...unhidden].sort((a, b) => {
        const idxA = orderMap.has(a.name) ? orderMap.get(a.name)! : 9999;
        const idxB = orderMap.has(b.name) ? orderMap.get(b.name)! : 9999;
        return idxA - idxB;
      });
    }

    return unhidden;
  }, [fields, hidden_columns, column_order]);

  const widthsMap = useMemo(() => {
    const map = new Map<string, number>();
    (column_widths || []).forEach(([name, w]) => {
      map.set(name, Math.max(72, w));
    });
    return map;
  }, [column_widths]);

  const getRecordId = useCallback(
    (record: Record<string, unknown>, index: number): string => {
      return String(record.id ?? record.key ?? index + 1);
    },
    []
  );

  const pushUndoGesture = useCallback((gesture: PatchRecord[]) => {
    if (gesture.length === 0) return;
    undoStack.current.push(gesture);
    if (undoStack.current.length > 50) {
      undoStack.current.shift();
    }
    redoStack.current = [];
  }, []);

  const handleUndo = useCallback(() => {
    if (undoStack.current.length === 0) return;
    const gesture = undoStack.current.pop()!;
    gesture.forEach(({ recordId, fieldName, oldValue }) => {
      onPatch(recordId, fieldName, oldValue);
    });
    redoStack.current.push(gesture);
    if (redoStack.current.length > 50) {
      redoStack.current.shift();
    }
  }, [onPatch]);

  const handleRedo = useCallback(() => {
    if (redoStack.current.length === 0) return;
    const gesture = redoStack.current.pop()!;
    gesture.forEach(({ recordId, fieldName, newValue }) => {
      onPatch(recordId, fieldName, newValue);
    });
    undoStack.current.push(gesture);
    if (undoStack.current.length > 50) {
      undoStack.current.shift();
    }
  }, [onPatch]);

  const getSelectionBounds = useCallback(() => {
    if (!selection) return { minRow: 0, maxRow: 0, minCol: 0, maxCol: 0 };
    const { anchor, focus } = selection;

    const anchorRowIdx = records.findIndex(
      (r, idx) => getRecordId(r, idx) === anchor.recordId
    );
    const anchorColIdx = visibleFields.findIndex((f) => f.name === anchor.fieldName);
    const focusRowIdx = records.findIndex(
      (r, idx) => getRecordId(r, idx) === focus.recordId
    );
    const focusColIdx = visibleFields.findIndex((f) => f.name === focus.fieldName);

    const aRow = anchorRowIdx !== -1 ? anchorRowIdx : 0;
    const fRow = focusRowIdx !== -1 ? focusRowIdx : 0;
    const aCol = anchorColIdx !== -1 ? anchorColIdx : 0;
    const fCol = focusColIdx !== -1 ? focusColIdx : 0;

    return {
      minRow: Math.min(aRow, fRow),
      maxRow: Math.max(aRow, fRow),
      minCol: Math.min(aCol, fCol),
      maxCol: Math.max(aCol, fCol),
    };
  }, [selection, records, visibleFields, getRecordId]);

  const isCellSelected = useCallback(
    (recordId: string, fieldName: string) => {
      if (!selection) return false;
      const { minRow, maxRow, minCol, maxCol } = getSelectionBounds();

      const rowIdx = records.findIndex(
        (r, idx) => getRecordId(r, idx) === recordId
      );
      const colIdx = visibleFields.findIndex((f) => f.name === fieldName);

      if (rowIdx === -1 || colIdx === -1) return false;
      return (
        rowIdx >= minRow &&
        rowIdx <= maxRow &&
        colIdx >= minCol &&
        colIdx <= maxCol
      );
    },
    [selection, getSelectionBounds, records, visibleFields, getRecordId]
  );

  const columns = useMemo<ColumnDef<Record<string, unknown>>[]>(() => {
    return visibleFields.map((field) => ({
      id: field.name,
      accessorFn: (row) => row[field.name],
      header: () => (
        <div className="flex items-center justify-between gap-1.5 select-none w-full group">
          <div className="flex items-center gap-1.5 truncate">
            <span className="font-semibold text-slate-700 dark:text-slate-200">
              {field.label}
            </span>
            {field.ferpa_sensitive && (
              <span className="text-[9px] px-1 py-0.5 rounded bg-rose-100 text-rose-800 dark:bg-rose-950 dark:text-rose-300 font-bold">
                FERPA
              </span>
            )}
            {field.field_type === "Relation" && field.linked_dataset_id && (
              <span className="text-[9px] px-1 py-0.5 rounded bg-purple-100 text-purple-800 dark:bg-purple-950 dark:text-purple-300 font-bold">
                🔗 {field.linked_dataset_id}
              </span>
            )}
          </div>

          {/* Column Menu Trigger */}
          <div className="relative">
            <button
              type="button"
              onClick={(e) => {
                e.stopPropagation();
                setActiveHeaderMenu(activeHeaderMenu === field.name ? null : field.name);
              }}
              className="opacity-0 group-hover:opacity-100 p-0.5 rounded hover:bg-slate-200 dark:hover:bg-slate-700 text-slate-400 hover:text-slate-600 dark:hover:text-slate-200 cursor-pointer"
              aria-label={`Column options for ${field.label}`}
            >
              ▾
            </button>

            {/* Popover Header Menu */}
            {activeHeaderMenu === field.name && (
              <div
                className="absolute z-50 right-0 top-full mt-1 w-44 bg-white dark:bg-slate-800 border border-slate-200 dark:border-slate-700 rounded-lg shadow-xl py-1 text-xs text-slate-700 dark:text-slate-200"
                onClick={(e) => e.stopPropagation()}
              >
                <button
                  type="button"
                  onClick={() => {
                    setActiveHeaderMenu(null);
                    onViewChange?.({
                      sort_rules: [{ id: `sort-${field.name}`, field_name: field.name, direction: "asc" }],
                    });
                  }}
                  className="w-full text-left px-3 py-1.5 hover:bg-slate-100 dark:hover:bg-slate-700 cursor-pointer"
                >
                  Sort Ascending
                </button>
                <button
                  type="button"
                  onClick={() => {
                    setActiveHeaderMenu(null);
                    onViewChange?.({
                      sort_rules: [{ id: `sort-${field.name}`, field_name: field.name, direction: "desc" }],
                    });
                  }}
                  className="w-full text-left px-3 py-1.5 hover:bg-slate-100 dark:hover:bg-slate-700 cursor-pointer"
                >
                  Sort Descending
                </button>
                <button
                  type="button"
                  onClick={() => {
                    setActiveHeaderMenu(null);
                    const currentHidden = hidden_columns || [];
                    if (!currentHidden.includes(field.name)) {
                      onViewChange?.({
                        hidden_columns: [...currentHidden, field.name],
                      });
                    }
                  }}
                  className="w-full text-left px-3 py-1.5 hover:bg-slate-100 dark:hover:bg-slate-700 cursor-pointer text-rose-600 dark:text-rose-400"
                >
                  Hide Column
                </button>
                <button
                  type="button"
                  onClick={() => {
                    setActiveHeaderMenu(null);
                    onViewChange?.({ frozen_through: field.name });
                  }}
                  className="w-full text-left px-3 py-1.5 hover:bg-slate-100 dark:hover:bg-slate-700 cursor-pointer border-t border-slate-100 dark:border-slate-700"
                >
                  Freeze through this column
                </button>

                {(field.field_type === "Number" ||
                  field.field_type === "Currency" ||
                  field.field_type === "Percent") && (
                  <div className="border-t border-slate-100 dark:border-slate-700 pt-1">
                    <div className="px-3 py-1 text-[10px] font-semibold text-slate-400 uppercase">
                      Summary
                    </div>
                    {["sum", "avg", "min", "max", "count", "none"].map((agg) => (
                      <button
                        key={agg}
                        type="button"
                        onClick={() => {
                          setActiveHeaderMenu(null);
                          const curSummaries = (column_summary || []).filter(
                            ([name]) => name !== field.name
                          );
                          const nextSummaries =
                            agg === "none"
                              ? curSummaries
                              : [...curSummaries, [field.name, agg] as [string, string]];
                          onViewChange?.({ column_summary: nextSummaries });
                        }}
                        className="w-full text-left px-4 py-1 hover:bg-slate-100 dark:hover:bg-slate-700 cursor-pointer text-xs"
                      >
                        {agg.toUpperCase()}
                      </button>
                    ))}
                  </div>
                )}
              </div>
            )}
          </div>
        </div>
      ),
    }));
  }, [visibleFields, activeHeaderMenu, hidden_columns, column_summary, onViewChange]);

  const table = useReactTable({
    data: records,
    columns,
    getCoreRowModel: getCoreRowModel(),
  });

  const moveFocus = useCallback(
    (direction: "up" | "down" | "left" | "right", shiftKey: boolean = false) => {
      if (!selection || records.length === 0 || visibleFields.length === 0) return;

      const currentRowIdx = records.findIndex(
        (r, idx) => getRecordId(r, idx) === selection.focus.recordId
      );
      const currentColIdx = visibleFields.findIndex(
        (f) => f.name === selection.focus.fieldName
      );

      if (currentRowIdx === -1 || currentColIdx === -1) return;

      let nextRowIdx = currentRowIdx;
      let nextColIdx = currentColIdx;

      switch (direction) {
        case "up":
          nextRowIdx = Math.max(currentRowIdx - 1, 0);
          break;
        case "down":
          nextRowIdx = Math.min(currentRowIdx + 1, records.length - 1);
          break;
        case "left":
          if (currentColIdx > 0) {
            nextColIdx = currentColIdx - 1;
          } else if (currentRowIdx > 0) {
            nextRowIdx = currentRowIdx - 1;
            nextColIdx = visibleFields.length - 1;
          }
          break;
        case "right":
          if (currentColIdx < visibleFields.length - 1) {
            nextColIdx = currentColIdx + 1;
          } else if (currentRowIdx < records.length - 1) {
            nextRowIdx = currentRowIdx + 1;
            nextColIdx = 0;
          }
          break;
      }

      const nextRecordId = getRecordId(records[nextRowIdx], nextRowIdx);
      const nextFieldName = visibleFields[nextColIdx].name;
      const nextCell: CellAddr = { recordId: nextRecordId, fieldName: nextFieldName };

      if (shiftKey) {
        setSelection({
          anchor: selection.anchor,
          focus: nextCell,
        });
        setIsEditing(false);
      } else {
        setSelection({
          anchor: nextCell,
          focus: nextCell,
        });
      }
    },
    [selection, records, visibleFields, getRecordId]
  );

  const commitEdit = useCallback(
    (recordId: string, field: FieldSpec, valueToCommit: string) => {
      if (isCancellingRef.current) return;
      let finalValue: unknown = valueToCommit;
      if (
        field.field_type === "Number" ||
        field.field_type === "Currency" ||
        field.field_type === "Percent"
      ) {
        if (valueToCommit.trim() === "") {
          finalValue = null;
        } else {
          const num = Number(valueToCommit);
          finalValue = isNaN(num) ? valueToCommit : num;
        }
      }
      const record = records.find((r, idx) => getRecordId(r, idx) === recordId);
      const oldValue = record?.[field.name];
      pushUndoGesture([{ recordId, fieldName: field.name, oldValue, newValue: finalValue }]);
      onPatch(recordId, field.name, finalValue);
    },
    [records, getRecordId, onPatch, pushUndoGesture]
  );

  const handleClear = useCallback(() => {
    if (!selection) return;
    const { minRow, maxRow, minCol, maxCol } = getSelectionBounds();
    const gesture: PatchRecord[] = [];

    for (let r = minRow; r <= maxRow; r++) {
      const rec = records[r];
      const recId = getRecordId(rec, r);
      for (let c = minCol; c <= maxCol; c++) {
        const field = visibleFields[c];
        if (!isReadOnlyField(field)) {
          const oldVal = rec?.[field.name];
          gesture.push({
            recordId: recId,
            fieldName: field.name,
            oldValue: oldVal,
            newValue: "",
          });
          onPatch(recId, field.name, "");
        }
      }
    }

    if (gesture.length > 0) {
      pushUndoGesture(gesture);
    }
  }, [selection, getSelectionBounds, records, visibleFields, getRecordId, onPatch, pushUndoGesture]);

  const handleFillDown = useCallback(() => {
    if (!selection) return;
    const { minRow, maxRow, minCol, maxCol } = getSelectionBounds();
    if (minRow === maxRow) return;

    const gesture: PatchRecord[] = [];
    for (let c = minCol; c <= maxCol; c++) {
      const field = visibleFields[c];
      if (isReadOnlyField(field)) continue;
      const topRecord = records[minRow];
      const topVal = topRecord?.[field.name];

      for (let r = minRow + 1; r <= maxRow; r++) {
        const rec = records[r];
        const recId = getRecordId(rec, r);
        const oldVal = rec?.[field.name];
        gesture.push({
          recordId: recId,
          fieldName: field.name,
          oldValue: oldVal,
          newValue: topVal,
        });
        onPatch(recId, field.name, topVal);
      }
    }

    if (gesture.length > 0) {
      pushUndoGesture(gesture);
    }
  }, [selection, getSelectionBounds, records, visibleFields, getRecordId, onPatch, pushUndoGesture]);

  const handleCopy = useCallback(async () => {
    if (!selection) return;
    const { minRow, maxRow, minCol, maxCol } = getSelectionBounds();
    const rows: string[] = [];
    for (let r = minRow; r <= maxRow; r++) {
      const rowRecord = records[r];
      const cols: string[] = [];
      for (let c = minCol; c <= maxCol; c++) {
        const field = visibleFields[c];
        const val = rowRecord?.[field.name];
        cols.push(val !== undefined && val !== null ? String(val) : "");
      }
      rows.push(cols.join("\t"));
    }
    const tsv = rows.join("\n");
    if (navigator?.clipboard?.writeText) {
      try {
        await navigator.clipboard.writeText(tsv);
      } catch {}
    }
    const count = (maxRow - minRow + 1) * (maxCol - minCol + 1);
    setCopyToast(`Copied ${count} cell${count > 1 ? "s" : ""} to clipboard`);
    setTimeout(() => setCopyToast(null), 2500);
  }, [selection, getSelectionBounds, records, visibleFields]);

  useEffect(() => {
    const handleMouseUp = () => {
      isMouseDownRef.current = false;
    };
    window.addEventListener("mouseup", handleMouseUp);
    return () => window.removeEventListener("mouseup", handleMouseUp);
  }, []);

  useEffect(() => {
    const handleWindowCopy = (e: ClipboardEvent) => {
      if (!selection || isEditing) return;
      const activeEl = document.activeElement;
      if (activeEl && (activeEl.tagName === "INPUT" || activeEl.tagName === "TEXTAREA")) return;
      e.preventDefault();
      handleCopy();
    };
    window.addEventListener("copy", handleWindowCopy);
    return () => window.removeEventListener("copy", handleWindowCopy);
  }, [selection, isEditing, handleCopy]);

  const handlePaste = useCallback(async () => {
    if (!selection) return;
    const targetFocus = selection.focus;
    const startRowIdx = records.findIndex(
      (r, idx) => getRecordId(r, idx) === targetFocus.recordId
    );
    const startColIdx = visibleFields.findIndex((f) => f.name === targetFocus.fieldName);
    if (startRowIdx === -1 || startColIdx === -1) return;

    let text = "";
    if (navigator?.clipboard?.readText) {
      try {
        text = await navigator.clipboard.readText();
      } catch {
        // ignore
      }
    }
    if (!text) return;

    const lines = text.replace(/\r\n/g, "\n").replace(/\r/g, "\n").split("\n");
    if (lines.length > 1 && lines[lines.length - 1] === "") {
      lines.pop();
    }

    const gesture: PatchRecord[] = [];
    lines.forEach((line, rOffset) => {
      const targetRowIdx = startRowIdx + rOffset;
      if (targetRowIdx >= records.length) return;
      const record = records[targetRowIdx];
      const recId = getRecordId(record, targetRowIdx);

      const rawCells = line.includes("\t") ? line.split("\t") : [line];
      rawCells.forEach((rawVal, cOffset) => {
        const targetColIdx = startColIdx + cOffset;
        if (targetColIdx >= visibleFields.length) return;
        const field = visibleFields[targetColIdx];
        if (isReadOnlyField(field)) return;

        let parsedVal: unknown = rawVal;
        if (
          field.field_type === "Number" ||
          field.field_type === "Currency" ||
          field.field_type === "Percent"
        ) {
          if (rawVal.trim() === "") {
            parsedVal = null;
          } else {
            const num = Number(rawVal);
            parsedVal = isNaN(num) ? rawVal : num;
          }
        }

        const oldVal = record[field.name];
        gesture.push({
          recordId: recId,
          fieldName: field.name,
          oldValue: oldVal,
          newValue: parsedVal,
        });
        onPatch(recId, field.name, parsedVal);
      });
    });

    if (gesture.length > 0) {
      pushUndoGesture(gesture);
    }
  }, [selection, records, visibleFields, getRecordId, onPatch, pushUndoGesture]);

  const handleCellClick = (
    e: React.MouseEvent,
    recordId: string,
    field: FieldSpec,
    currentValue: unknown
  ) => {
    isCancellingRef.current = false;
    const clickedCell: CellAddr = { recordId, fieldName: field.name };

    if (e.shiftKey && selection) {
      setSelection({
        anchor: selection.anchor,
        focus: clickedCell,
      });
      setIsEditing(false);
      return;
    }

    setSelection({
      anchor: clickedCell,
      focus: clickedCell,
    });

    if (isReadOnlyField(field)) {
      setIsEditing(false);
      return;
    }

    if (field.field_type === "Checkbox") {
      setIsEditing(false);
      return;
    }

    if (field.field_type === "Rating") {
      setIsEditing(false);
      return;
    }

    if (field.field_type === "MultiSelect") {
      const arr = Array.isArray(currentValue)
        ? (currentValue as string[])
        : typeof currentValue === "string" && currentValue
        ? currentValue.split(",").map((s) => s.trim())
        : [];
      setDraftMultiSelect(arr);
      setMultiSelectOpen(true);
      setIsEditing(true);
      return;
    }

    setIsEditing(true);
    setEditValue(
      currentValue !== undefined && currentValue !== null
        ? String(currentValue)
        : ""
    );
  };

  const handleInputKeyDown = (
    e: React.KeyboardEvent,
    recordId: string,
    field: FieldSpec
  ) => {
    if (e.key === "Enter") {
      e.preventDefault();
      commitEdit(recordId, field, editValue);
      setIsEditing(false);
      if (e.shiftKey) {
        moveFocus("up");
      } else {
        moveFocus("down");
      }
    } else if (e.key === "Tab") {
      e.preventDefault();
      commitEdit(recordId, field, editValue);
      setIsEditing(false);
      if (e.shiftKey) {
        moveFocus("left");
      } else {
        moveFocus("right");
      }
    } else if (e.key === "Escape") {
      e.preventDefault();
      isCancellingRef.current = true;
      setIsEditing(false);
    }
  };

  const handleCellKeyDown = (
    e: React.KeyboardEvent,
    recordId: string,
    field: FieldSpec,
    currentValue: unknown
  ) => {
    if ((e.ctrlKey || e.metaKey) && !e.shiftKey && e.key.toLowerCase() === "z") {
      e.preventDefault();
      handleUndo();
      return;
    }

    if (
      (e.ctrlKey || e.metaKey) &&
      ((e.shiftKey && e.key.toLowerCase() === "z") || e.key.toLowerCase() === "y")
    ) {
      e.preventDefault();
      handleRedo();
      return;
    }

    if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "c") {
      e.preventDefault();
      handleCopy();
      return;
    }

    if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "v") {
      e.preventDefault();
      handlePaste();
      return;
    }

    if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "d") {
      e.preventDefault();
      handleFillDown();
      return;
    }

    if (e.key === "Delete" || e.key === "Backspace") {
      e.preventDefault();
      handleClear();
      return;
    }

    if (e.key === "ArrowUp") {
      e.preventDefault();
      moveFocus("up", e.shiftKey);
      return;
    } else if (e.key === "ArrowDown") {
      e.preventDefault();
      moveFocus("down", e.shiftKey);
      return;
    } else if (e.key === "ArrowLeft") {
      e.preventDefault();
      moveFocus("left", e.shiftKey);
      return;
    } else if (e.key === "ArrowRight") {
      e.preventDefault();
      moveFocus("right", e.shiftKey);
      return;
    } else if (e.key === "Tab") {
      e.preventDefault();
      if (e.shiftKey) moveFocus("left");
      else moveFocus("right");
      return;
    }

    if (isReadOnlyField(field)) {
      return;
    }

    if (e.key === "Enter" || e.key === "F2") {
      e.preventDefault();
      if (field.field_type === "Checkbox") {
        onPatch(recordId, field.name, !currentValue);
      } else if (field.field_type === "MultiSelect") {
        const arr = Array.isArray(currentValue)
          ? (currentValue as string[])
          : typeof currentValue === "string" && currentValue
          ? currentValue.split(",").map((s) => s.trim())
          : [];
        setDraftMultiSelect(arr);
        setMultiSelectOpen(true);
        setIsEditing(true);
      } else if (field.field_type !== "Rating") {
        isCancellingRef.current = false;
        setIsEditing(true);
        setEditValue(
          currentValue !== undefined && currentValue !== null
            ? String(currentValue)
            : ""
        );
      }
    } else if (
      e.key.length === 1 &&
      !e.ctrlKey &&
      !e.metaKey &&
      !e.altKey &&
      field.field_type !== "Checkbox" &&
      field.field_type !== "Select" &&
      field.field_type !== "MultiSelect" &&
      field.field_type !== "Rating"
    ) {
      const isSingleCell =
        selection?.anchor.recordId === selection?.focus.recordId &&
        selection?.anchor.fieldName === selection?.focus.fieldName;
      if (isSingleCell) {
        isCancellingRef.current = false;
        setIsEditing(true);
        setEditValue(e.key);
      }
    }
  };

  useEffect(() => {
    if (isEditing && inputRef.current) {
      inputRef.current.focus();
      if ("select" in inputRef.current && typeof inputRef.current.select === "function") {
        inputRef.current.select();
      }
    }
  }, [isEditing, selection]);

  useEffect(() => {
    if (!isEditing && selection && activeCellRef.current) {
      activeCellRef.current.focus();
    }
  }, [isEditing, selection]);

  const densityClass = getDensityClasses(rowDensity);

  // Compute footer column summary
  const summaryByField = useMemo(() => {
    const map = new Map<string, string>();
    if (column_summary && column_summary.length > 0) {
      column_summary.forEach(([name, agg]) => {
        map.set(name, agg);
      });
    } else {
      // Default: sum for Number and Currency, nothing otherwise
      visibleFields.forEach((f) => {
        if (f.field_type === "Number" || f.field_type === "Currency") {
          map.set(f.name, "sum");
        }
      });
    }
    return map;
  }, [column_summary, visibleFields]);

  const columnSummaries = useMemo(() => {
    const res: Record<string, string> = {};
    visibleFields.forEach((field) => {
      const agg = summaryByField.get(field.name);
      if (!agg || agg === "none") return;

      const nums: number[] = [];
      let count = 0;
      records.forEach((r) => {
        const val = r[field.name];
        if (val !== null && val !== undefined && val !== "") {
          count++;
          const n = Number(val);
          if (!isNaN(n)) nums.push(n);
        }
      });

      let calculated = "";
      if (agg === "count") {
        calculated = `${count}`;
      } else if (nums.length > 0) {
        switch (agg) {
          case "sum": {
            const sumVal = nums.reduce((a, b) => a + b, 0);
            calculated = `${sumVal}`;
            break;
          }
          case "avg": {
            const avgVal = nums.reduce((a, b) => a + b, 0) / nums.length;
            calculated = `${Number(avgVal.toFixed(2))}`;
            break;
          }
          case "min": {
            const minVal = Math.min(...nums);
            calculated = `${minVal}`;
            break;
          }
          case "max": {
            const maxVal = Math.max(...nums);
            calculated = `${maxVal}`;
            break;
          }
        }
      }

      if (calculated) {
        res[field.name] = calculated;
      }
    });
    return res;
  }, [visibleFields, summaryByField, records]);

  return (
    <div className="w-full overflow-x-auto select-text font-sans">
      <table className="w-full text-left text-xs border-collapse">
        <thead>
          {table.getHeaderGroups().map((headerGroup) => (
            <tr
              key={headerGroup.id}
              className="bg-slate-50 dark:bg-slate-800/60 border-b border-slate-200 dark:border-slate-800 text-slate-600 dark:text-slate-400 font-semibold"
            >
              {headerGroup.headers.map((header) => {
                const colName = header.column.id;
                const width = widthsMap.get(colName);
                const isFrozen =
                  frozen_through &&
                  visibleFields.findIndex((f) => f.name === colName) <=
                    visibleFields.findIndex((f) => f.name === frozen_through);

                return (
                  <th
                    key={header.id}
                    draggable
                    onDragStart={() => setDraggedColName(colName)}
                    onDragOver={(e) => e.preventDefault()}
                    onDrop={() => {
                      if (draggedColName && draggedColName !== colName) {
                        const curOrder =
                          column_order && column_order.length > 0
                            ? [...column_order]
                            : visibleFields.map((f) => f.name);
                        const fromIdx = curOrder.indexOf(draggedColName);
                        const toIdx = curOrder.indexOf(colName);
                        if (fromIdx !== -1 && toIdx !== -1) {
                          curOrder.splice(fromIdx, 1);
                          curOrder.splice(toIdx, 0, draggedColName);
                          onViewChange?.({ column_order: curOrder });
                        }
                      }
                      setDraggedColName(null);
                    }}
                    style={{ width: width ? `${width}px` : undefined }}
                    className={`py-2.5 px-3 min-w-[72px] border-r border-slate-200/60 dark:border-slate-800/60 last:border-r-0 relative ${
                      isFrozen ? "sticky left-0 bg-slate-100 dark:bg-slate-800 z-20" : ""
                    }`}
                  >
                    {header.isPlaceholder
                      ? null
                      : flexRender(header.column.columnDef.header, header.getContext())}
                  </th>
                );
              })}
            </tr>
          ))}
        </thead>
        <tbody className="divide-y divide-slate-100 dark:divide-slate-800/60">
          {table.getRowModel().rows.map((row, rowIdx) => {
            const recordId = getRecordId(row.original, rowIdx);
            return (
              <tr
                key={row.id}
                className="hover:bg-slate-50/50 dark:hover:bg-slate-800/30 transition-colors"
              >
                {row.getVisibleCells().map((cell) => {
                  const fieldName = cell.column.id;
                  const field = visibleFields.find((f) => f.name === fieldName);
                  if (!field) return null;

                  const cellVal = cell.getValue();
                  const isFocus =
                    selection?.focus.recordId === recordId &&
                    selection?.focus.fieldName === fieldName;
                  const selected = isCellSelected(recordId, fieldName);
                  const isCellEditing = isFocus && isEditing;
                  const readOnly = isReadOnlyField(field);

                  const { minRow, maxRow, minCol, maxCol } = getSelectionBounds();
                  const colIdx = visibleFields.findIndex((f) => f.name === fieldName);

                  const isTopEdge = selected && rowIdx === minRow;
                  const isBottomEdge = selected && rowIdx === maxRow;
                  const isLeftEdge = selected && colIdx === minCol;
                  const isRightEdge = selected && colIdx === maxCol;
                  const isBottomRightCorner = selected && isBottomEdge && isRightEdge;

                  const borderClasses = selected
                    ? `${isTopEdge ? "border-t-2 border-t-blue-600 dark:border-t-blue-500" : ""} ${
                        isBottomEdge ? "border-b-2 border-b-blue-600 dark:border-b-blue-500" : ""
                      } ${isLeftEdge ? "border-l-2 border-l-blue-600 dark:border-l-blue-500" : ""} ${
                        isRightEdge ? "border-r-2 border-r-blue-600 dark:border-r-blue-500" : ""
                      }`
                    : "";

                  return (
                    <td
                      key={cell.id}
                      ref={isFocus && !isCellEditing ? activeCellRef : undefined}
                      tabIndex={isFocus ? 0 : -1}
                      data-testid={`cell-${recordId}-${fieldName}`}
                      data-focused={isFocus ? "true" : "false"}
                      data-selected={selected ? "true" : "false"}
                      data-editing={isCellEditing ? "true" : "false"}
                      onMouseDown={(e) => {
                        if (e.button !== 0) return;
                        const targetTag = (e.target as HTMLElement).tagName.toLowerCase();
                        if (targetTag === "input" || targetTag === "select" || targetTag === "button" || targetTag === "a") return;
                        isMouseDownRef.current = true;
                        const cellAddr: CellAddr = { recordId, fieldName: field.name };
                        if (e.shiftKey && selection) {
                          setSelection({ anchor: selection.anchor, focus: cellAddr });
                          setIsEditing(false);
                        } else {
                          setSelection({ anchor: cellAddr, focus: cellAddr });
                        }
                      }}
                      onMouseEnter={() => {
                        if (isMouseDownRef.current && selection) {
                          const cellAddr: CellAddr = { recordId, fieldName: field.name };
                          setSelection((prev) => (prev ? { anchor: prev.anchor, focus: cellAddr } : null));
                          setIsEditing(false);
                        }
                      }}
                      onClick={(e) => {
                        const targetTag = (e.target as HTMLElement).tagName.toLowerCase();
                        if (targetTag === "button" || targetTag === "select") return;
                        handleCellClick(e, recordId, field, cellVal);
                      }}
                      onKeyDown={(e) =>
                        handleCellKeyDown(e, recordId, field, cellVal)
                      }
                      className={`relative border-r border-slate-200/60 dark:border-slate-800/60 last:border-r-0 outline-none transition-shadow ${densityClass} ${
                        isFocus
                          ? "ring-2 ring-blue-500 ring-inset z-10 bg-blue-50/20 dark:bg-blue-950/20"
                          : selected
                          ? "bg-blue-100/50 dark:bg-blue-900/30"
                          : ""
                      } ${borderClasses}`}
                    >
                      {/* Checkbox Type */}
                      {field.field_type === "Checkbox" ? (
                        <div className="flex items-center">
                          <input
                            type="checkbox"
                            checked={Boolean(cellVal)}
                            onChange={(e) => {
                              const oldVal = Boolean(cellVal);
                              const newVal = e.target.checked;
                              pushUndoGesture([
                                {
                                  recordId,
                                  fieldName: field.name,
                                  oldValue: oldVal,
                                  newValue: newVal,
                                },
                              ]);
                              onPatch(recordId, field.name, newVal);
                            }}
                            className="rounded text-blue-600 cursor-pointer h-4 w-4"
                          />
                        </div>
                      ) : field.field_type === "Rating" ? (
                        /* Rating Type */
                        <div className="flex items-center gap-1">
                          {[1, 2, 3, 4, 5].map((star) => (
                            <button
                              key={star}
                              type="button"
                              onClick={(e) => {
                                e.stopPropagation();
                                const clickedCell: CellAddr = {
                                  recordId,
                                  fieldName: field.name,
                                };
                                setSelection({
                                  anchor: clickedCell,
                                  focus: clickedCell,
                                });
                                pushUndoGesture([
                                  {
                                    recordId,
                                    fieldName: field.name,
                                    oldValue: cellVal,
                                    newValue: star,
                                  },
                                ]);
                                onPatch(recordId, field.name, star);
                              }}
                              className={`text-sm cursor-pointer transition-colors ${
                                star <= Number(cellVal || 0)
                                  ? "text-amber-400 hover:text-amber-500"
                                  : "text-slate-300 dark:text-slate-600 hover:text-amber-300"
                              }`}
                              aria-label={`Rate ${star}`}
                            >
                              ★
                            </button>
                          ))}
                        </div>
                      ) : field.field_type === "Select" && isCellEditing ? (
                        /* Select Editing */
                        <select
                          ref={inputRef as React.RefObject<HTMLSelectElement>}
                          value={String(cellVal ?? "")}
                          onChange={(e) => {
                            commitEdit(recordId, field, e.target.value);
                            setIsEditing(false);
                          }}
                          onKeyDown={(e) =>
                            handleInputKeyDown(e, recordId, field)
                          }
                          className="w-full px-1.5 py-0.5 text-xs rounded border border-blue-500 bg-white dark:bg-slate-900 text-slate-900 dark:text-white focus:outline-none"
                        >
                          <option value="">Select...</option>
                          {(field.select_options || []).map((opt) => (
                            <option key={opt} value={opt}>
                              {opt}
                            </option>
                          ))}
                        </select>
                      ) : field.field_type === "MultiSelect" && isCellEditing && multiSelectOpen ? (
                        /* MultiSelect Popover */
                        <div className="relative">
                          <div className="text-slate-800 dark:text-slate-200">
                            {formatDisplayValue(field, cellVal, tables)}
                          </div>
                          <div className="absolute z-30 top-full left-0 mt-1 min-w-[180px] bg-white dark:bg-slate-800 border border-slate-200 dark:border-slate-700 rounded-lg shadow-xl p-2.5 flex flex-col gap-1.5">
                            <div className="text-[11px] font-semibold text-slate-500 dark:text-slate-400 pb-1 border-b border-slate-100 dark:border-slate-700">
                              {field.label}
                            </div>
                            {(field.select_options || []).map((opt) => {
                              const checked = draftMultiSelect.includes(opt);
                              return (
                                <label
                                  key={opt}
                                  className="flex items-center gap-2 text-xs text-slate-700 dark:text-slate-200 cursor-pointer hover:bg-slate-50 dark:hover:bg-slate-700/50 p-1 rounded"
                                >
                                  <input
                                    type="checkbox"
                                    checked={checked}
                                    onChange={(e) => {
                                      if (e.target.checked) {
                                        setDraftMultiSelect((prev) => [...prev, opt]);
                                      } else {
                                        setDraftMultiSelect((prev) =>
                                          prev.filter((o) => o !== opt)
                                        );
                                      }
                                    }}
                                    className="rounded text-blue-600"
                                  />
                                  <span>{opt}</span>
                                </label>
                              );
                            })}
                            <div className="pt-2 border-t border-slate-100 dark:border-slate-700 flex justify-end">
                              <button
                                type="button"
                                onClick={() => {
                                  setMultiSelectOpen(false);
                                  setIsEditing(false);
                                  pushUndoGesture([
                                    {
                                      recordId,
                                      fieldName: field.name,
                                      oldValue: cellVal,
                                      newValue: draftMultiSelect,
                                    },
                                  ]);
                                  onPatch(recordId, field.name, draftMultiSelect);
                                }}
                                className="px-2.5 py-1 text-xs font-semibold bg-blue-600 text-white rounded hover:bg-blue-700 cursor-pointer shadow-xs"
                              >
                                Done
                              </button>
                            </div>
                          </div>
                        </div>
                      ) : isCellEditing && !readOnly ? (
                        /* Text, Email, Phone, Url, Number, Currency, Percent, Date */
                        <input
                          ref={inputRef as React.RefObject<HTMLInputElement>}
                          type={getInputType(field.field_type)}
                          value={editValue}
                          onChange={(e) => setEditValue(e.target.value)}
                          onKeyDown={(e) =>
                            handleInputKeyDown(e, recordId, field)
                          }
                          className="w-full px-1.5 py-0.5 text-xs rounded border border-blue-500 bg-white dark:bg-slate-900 text-slate-900 dark:text-white focus:outline-none"
                        />
                      ) : field.field_type === "Relation" ? (
                        /* Linked Field (Airtable-style Relation) */
                        <div className="relative inline-flex items-center gap-1.5 flex-wrap">
                          {(() => {
                            const targetTable = tables?.find((t) => t.id === field.target_table_id);
                            const displayCol = field.target_display_field || "name";
                            const prefix = field.display_label_override || "";
                            const rawVal = cellVal;
                            const ids = Array.isArray(rawVal)
                              ? rawVal.map(String)
                              : typeof rawVal === "string" && rawVal.includes(",")
                              ? rawVal.split(",").map((s) => s.trim())
                              : rawVal !== null && rawVal !== undefined && rawVal !== ""
                              ? [String(rawVal)]
                              : [];

                            const filteredCandidates = (targetTable?.records || []).filter((tr) => {
                              if (!field.link_filter || !field.link_filter.field) return true;
                              const val = String(tr[field.link_filter.field] ?? "").toLowerCase();
                              const expected = String(field.link_filter.value ?? "").toLowerCase();
                              if (field.link_filter.operator === "equals") return val === expected;
                              if (field.link_filter.operator === "not_equals") return val !== expected;
                              if (field.link_filter.operator === "contains") return val.includes(expected);
                              return true;
                            });

                            const isPickerOpen =
                              activeLinkPicker?.recordId === recordId &&
                              activeLinkPicker?.fieldName === field.name;

                            return (
                              <>
                                {ids.length > 0 ? (
                                  ids.map((id) => {
                                    const matched = targetTable?.records?.find(
                                      (r) => String(r.id) === id || String(r.key) === id
                                    );
                                    const label = matched
                                      ? `${prefix}${matched[displayCol] || matched.name || matched.title || id}`
                                      : `${prefix}${id}`;
                                    return (
                                      <span
                                        key={id}
                                        data-testid={`link-pill-${recordId}-${id}`}
                                        className="inline-flex items-center gap-1 px-2 py-0.5 rounded-md text-xs font-semibold bg-purple-50 dark:bg-purple-950/70 text-purple-700 dark:text-purple-300 border border-purple-200 dark:border-purple-800 shadow-2xs"
                                      >
                                        <span className="text-[10px]">🔗</span>
                                        <span>{label}</span>
                                      </span>
                                    );
                                  })
                                ) : (
                                  <span className="text-slate-400 text-xs italic">Empty link</span>
                                )}

                                <button
                                  type="button"
                                  data-testid={`open-link-picker-${recordId}-${field.name}`}
                                  onClick={(e) => {
                                    e.stopPropagation();
                                    setActiveLinkPicker(
                                      isPickerOpen ? null : { recordId, fieldName: field.name }
                                    );
                                  }}
                                  className="p-1 rounded text-purple-600 dark:text-purple-400 hover:bg-purple-100 dark:hover:bg-purple-900/50 text-xs font-bold cursor-pointer"
                                  title="Add or edit linked records"
                                >
                                  +
                                </button>

                                {isPickerOpen && (
                                  <div
                                    data-testid={`link-record-picker-${recordId}-${field.name}`}
                                    className="absolute z-50 left-0 top-full mt-1 min-w-[200px] max-w-xs bg-white dark:bg-slate-800 border border-slate-200 dark:border-slate-700 rounded-lg shadow-xl p-2.5 space-y-2 text-xs"
                                    onClick={(e) => e.stopPropagation()}
                                  >
                                    <div className="font-semibold text-slate-700 dark:text-slate-200 border-b border-slate-100 dark:border-slate-700 pb-1 flex items-center justify-between">
                                      <span>Link to {targetTable?.name || "Record"}</span>
                                      <button
                                        type="button"
                                        onClick={() => setActiveLinkPicker(null)}
                                        className="text-slate-400 hover:text-slate-600 dark:hover:text-slate-200"
                                      >
                                        ✕
                                      </button>
                                    </div>

                                    {field.link_filter && (
                                      <div className="text-[10px] text-purple-600 dark:text-purple-400 bg-purple-50 dark:bg-purple-950/40 p-1 rounded font-mono">
                                        Filter: {field.link_filter.field} {field.link_filter.operator} &quot;{field.link_filter.value}&quot;
                                      </div>
                                    )}

                                    <div className="max-h-40 overflow-y-auto space-y-1">
                                      {filteredCandidates.map((tr) => {
                                        const trId = String(tr.id);
                                        const isLinked = ids.includes(trId);
                                        const trLabel = `${prefix}${tr[displayCol] || tr.name || tr.title || trId}`;
                                        return (
                                          <button
                                            key={trId}
                                            type="button"
                                            onClick={() => {
                                              const isMultiple = field.cardinality === "multiple" || field.allow_multiple;
                                              let nextIds: string[];
                                              if (isMultiple) {
                                                nextIds = isLinked
                                                  ? ids.filter((id) => id !== trId)
                                                  : [...ids, trId];
                                              } else {
                                                nextIds = isLinked ? [] : [trId];
                                                setActiveLinkPicker(null);
                                              }
                                              const patchVal = isMultiple ? nextIds : (nextIds[0] || "");
                                              pushUndoGesture([
                                                {
                                                  recordId,
                                                  fieldName: field.name,
                                                  oldValue: cellVal,
                                                  newValue: patchVal,
                                                },
                                              ]);
                                              onPatch(recordId, field.name, patchVal);
                                            }}
                                            className={`w-full text-left px-2 py-1.5 rounded flex items-center justify-between cursor-pointer transition-colors ${
                                              isLinked
                                                ? "bg-purple-50 dark:bg-purple-950/50 text-purple-700 dark:text-purple-300 font-semibold"
                                                : "hover:bg-slate-50 dark:hover:bg-slate-700/50 text-slate-700 dark:text-slate-300"
                                            }`}
                                          >
                                            <span className="truncate">{trLabel}</span>
                                            {isLinked && <span className="text-purple-600 text-xs">✓</span>}
                                          </button>
                                        );
                                      })}
                                      {filteredCandidates.length === 0 && (
                                        <div className="text-slate-400 text-center py-2 text-[11px]">
                                          No records match filter
                                        </div>
                                      )}
                                    </div>
                                  </div>
                                )}
                              </>
                            );
                          })()}
                        </div>
                      ) : (
                        /* Non-editing display */
                        <div className="flex items-center justify-between text-slate-800 dark:text-slate-200 min-h-[1.25rem]">
                          <span
                            className={`${
                              field.field_type === "Number" ||
                              field.field_type === "Currency" ||
                              field.field_type === "Percent"
                                ? "font-mono font-medium text-emerald-600 dark:text-emerald-400"
                                : readOnly
                                ? "text-slate-600 dark:text-slate-400 font-mono"
                                : ""
                            }`}
                          >
                            {formatDisplayValue(field, cellVal, tables)}
                          </span>
                        </div>
                      )}
                      {isBottomRightCorner && (
                        <div
                          data-testid="grid-fill-handle"
                          className="absolute -bottom-1 -right-1 w-2 h-2 bg-blue-600 dark:bg-blue-400 border border-white dark:border-slate-900 z-20 cursor-crosshair shadow-2xs"
                        />
                      )}
                    </td>
                  );
                })}
              </tr>
            );
          })}
        </tbody>
        {/* Footer Summary Row */}
        <tfoot>
          <tr className="bg-slate-100/80 dark:bg-slate-800/80 font-mono text-[11px] text-slate-600 dark:text-slate-300 border-t border-slate-200 dark:border-slate-800">
            {visibleFields.map((field) => {
              const summaryText = columnSummaries[field.name];
              return (
                <td
                  key={field.name}
                  data-testid={`summary-${field.name}`}
                  className="py-2.5 px-3 border-r border-slate-200/60 dark:border-slate-800/60 last:border-r-0 font-medium"
                >
                  {summaryText ? (
                    <span>{summaryText}</span>
                  ) : (
                    <span className="text-slate-400">—</span>
                  )}
                </td>
              );
            })}
          </tr>
        </tfoot>
      </table>
      {(() => {
        const { minRow, maxRow, minCol, maxCol } = getSelectionBounds();
        const totalSelected = (maxRow - minRow + 1) * (maxCol - minCol + 1);
        if (!selection || totalSelected <= 1) return null;

        return (
          <div
            data-testid="selection-status-badge"
            className="fixed bottom-6 left-1/2 -translate-x-1/2 z-40 bg-slate-900/95 dark:bg-slate-100 text-white dark:text-slate-900 px-3.5 py-1.5 rounded-lg shadow-xl border border-slate-700/60 dark:border-slate-300 flex items-center gap-2.5 text-xs font-medium backdrop-blur-xs select-none animate-fade-in"
          >
            <span>
              {maxRow - minRow + 1} × {maxCol - minCol + 1} cells selected ({totalSelected})
            </span>
            <div className="h-3.5 w-px bg-slate-700 dark:bg-slate-300" />
            <button
              type="button"
              data-testid="grid-bulk-copy-btn"
              onClick={(e) => {
                e.stopPropagation();
                handleCopy();
              }}
              className="inline-flex items-center gap-1 px-2.5 py-1 rounded bg-blue-600 hover:bg-blue-700 text-white font-semibold cursor-pointer transition-colors shadow-2xs text-xs"
              title="Copy selected cells as TSV (Ctrl+C)"
            >
              📋 Copy (Ctrl+C)
            </button>
            <button
              type="button"
              data-testid="grid-bulk-clear-btn"
              onClick={(e) => {
                e.stopPropagation();
                handleClear();
              }}
              className="inline-flex items-center gap-1 px-2 py-0.5 rounded bg-slate-800 dark:bg-slate-200 hover:bg-slate-700 dark:hover:bg-slate-300 text-slate-300 dark:text-slate-700 font-medium cursor-pointer transition-colors text-xs"
              title="Clear values (Delete)"
            >
              ✕ Clear
            </button>
          </div>
        );
      })()}
      {copyToast && (
        <div
          data-testid="grid-copy-toast"
          className="fixed top-16 right-6 z-50 bg-slate-900 text-white px-3.5 py-2 rounded-lg text-xs font-semibold shadow-xl border border-slate-700 flex items-center gap-2 animate-fade-in"
        >
          <span className="text-emerald-400">✓</span>
          <span>{copyToast}</span>
        </div>
      )}
    </div>
  );
}
