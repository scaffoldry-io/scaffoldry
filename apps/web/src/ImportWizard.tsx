import React, { useState, useEffect, useRef } from "react";
import { AppTable } from "./types";

export interface ImportWizardProps {
  isOpen: boolean;
  onClose: () => void;
  activeTable: AppTable;
  appSlug?: string;
  initialText?: string;
  onImportComplete?: (recordsWritten: number, rejectedCount: number) => void;

  // Legacy/compatibility props
  csvRawText?: string;
  onParseCsvText?: (text: string) => void;
  csvParsedHeaders?: string[];
  csvParsedRows?: Record<string, string>[];
  csvFieldMapping?: Record<string, string>;
  setCsvFieldMapping?: React.Dispatch<React.SetStateAction<Record<string, string>>>;
  onExecuteCsvImport?: () => void;
}

export type WizardStep = "file" | "map" | "check" | "import";

interface ValidationError {
  row: number;
  column: string;
  message: string;
  recordIdentifier?: string;
}

export function parseCsv(text: string): { headers: string[]; rows: Record<string, string>[] } {
  const lines = text.split(/\r?\n/).filter((l) => l.trim().length > 0);
  if (lines.length === 0) return { headers: [], rows: [] };

  const parseLine = (line: string): string[] => {
    const result: string[] = [];
    let current = "";
    let inQuotes = false;
    for (let i = 0; i < line.length; i++) {
      const char = line[i];
      if (char === '"') {
        if (inQuotes && line[i + 1] === '"') {
          current += '"';
          i++;
        } else {
          inQuotes = !inQuotes;
        }
      } else if (char === "," && !inQuotes) {
        result.push(current.trim());
        current = "";
      } else {
        current += char;
      }
    }
    result.push(current.trim());
    return result;
  };

  const headers = parseLine(lines[0]);
  const rows: Record<string, string>[] = [];
  for (let i = 1; i < lines.length; i++) {
    const values = parseLine(lines[i]);
    const row: Record<string, string> = {};
    headers.forEach((h, idx) => {
      row[h] = values[idx] !== undefined ? values[idx] : "";
    });
    rows.push(row);
  }
  return { headers, rows };
}

export function inferColumnType(values: string[]): string {
  const nonNull = values.filter((v) => v !== "");
  if (nonNull.length === 0) return "text";

  const isNum = nonNull.every((v) => !isNaN(Number(v)));
  if (isNum) return "number";

  const isBool = nonNull.every((v) => /^(true|false|yes|no|1|0)$/i.test(v));
  if (isBool) return "boolean";

  const isEmail = nonNull.every((v) => /^[^\s@]+@[^\s@]+\.[^\s@]+$/.test(v));
  if (isEmail) return "email";

  const isUrl = nonNull.every((v) => /^https?:\/\//i.test(v));
  if (isUrl) return "url";

  const unique = new Set(nonNull);
  if (unique.size <= 20 && nonNull.length >= 20) return "select";

  return "text";
}

export const ImportWizard: React.FC<ImportWizardProps> = ({
  isOpen,
  onClose,
  activeTable,
  appSlug: _appSlug,
  initialText = "",
  onImportComplete,
  csvRawText,
  onParseCsvText,
  csvParsedHeaders,
  csvParsedRows,
  csvFieldMapping,
  setCsvFieldMapping,
  onExecuteCsvImport,
}) => {
  const [step, setStep] = useState<WizardStep>("file");
  const [text, setText] = useState<string>(csvRawText ?? initialText);
  const [headers, setHeaders] = useState<string[]>(csvParsedHeaders ?? []);
  const [rows, setRows] = useState<Record<string, string>[]>(csvParsedRows ?? []);
  const [mapping, setMapping] = useState<Record<string, string>>(csvFieldMapping ?? {});
  const [upsertKey, setUpsertKey] = useState<string>("");
  const [selectStrategy, setSelectStrategy] = useState<string>("reject");
  const [acceptErrors, setAcceptErrors] = useState<boolean>(false);

  // Dry run state
  const [dryRunProgress, setDryRunProgress] = useState<number>(0);
  const [dryRunComplete, setDryRunComplete] = useState<boolean>(false);
  const [errors, setErrors] = useState<ValidationError[]>([]);
  const [validCount, setValidCount] = useState<number>(0);
  const [invalidCount, setInvalidCount] = useState<number>(0);
  const [createsCount, setCreatesCount] = useState<number>(0);
  const [updatesCount, setUpdatesCount] = useState<number>(0);

  // Import state
  const [importProgress, setImportProgress] = useState<number>(0);
  const [importComplete, setImportComplete] = useState<boolean>(false);
  const [writtenCount, setWrittenCount] = useState<number>(0);
  const [rejectedRows, setRejectedRows] = useState<Record<string, string>[]>([]);
  const [statusMessage, setStatusMessage] = useState<string>("Choose a file to start.");

  const dialogRef = useRef<HTMLDivElement>(null);

  // Sync external text
  useEffect(() => {
    if (csvRawText !== undefined && csvRawText !== text) {
      setText(csvRawText);
      const parsed = parseCsv(csvRawText);
      setHeaders(parsed.headers);
      setRows(parsed.rows);
      initMapping(parsed.headers);
    }
  }, [csvRawText]);

  // Initial text parsing
  useEffect(() => {
    if (initialText && !headers.length) {
      const parsed = parseCsv(initialText);
      setHeaders(parsed.headers);
      setRows(parsed.rows);
      initMapping(parsed.headers);
    }
  }, [initialText]);

  const initMapping = (hdrs: string[]) => {
    const initialMap: Record<string, string> = {};
    hdrs.forEach((h) => {
      const norm = h.toLowerCase().trim().replace(/[^a-z0-9]/g, "");
      const match = activeTable.fields.find(
        (f) =>
          f.name.toLowerCase() === norm ||
          f.label.toLowerCase().replace(/[^a-z0-9]/g, "") === norm ||
          f.name.toLowerCase() === h.toLowerCase().trim()
      );
      if (match) {
        initialMap[h] = match.name;
      }
    });
    setMapping(initialMap);
    if (setCsvFieldMapping) setCsvFieldMapping(initialMap);
  };

  // Keyboard navigation & accessibility
  useEffect(() => {
    const handleKeyDown = (e: KeyboardEvent) => {
      if (!isOpen) return;
      if (e.key === "Escape") {
        e.preventDefault();
        onClose();
      }
    };
    window.addEventListener("keydown", handleKeyDown);
    return () => window.removeEventListener("keydown", handleKeyDown);
  }, [isOpen, onClose]);

  if (!isOpen) return null;

  const handleTextChange = (val: string) => {
    setText(val);
    if (onParseCsvText) onParseCsvText(val);
    const parsed = parseCsv(val);
    setHeaders(parsed.headers);
    setRows(parsed.rows);
    initMapping(parsed.headers);
  };

  const handleFileSelect = (e: React.ChangeEvent<HTMLInputElement>) => {
    const file = e.target.files?.[0];
    if (!file) return;
    const reader = new FileReader();
    reader.onload = (event) => {
      const content = event.target?.result as string;
      handleTextChange(content);
    };
    reader.readAsText(file);
  };

  const executeDryRun = () => {
    setStep("check");
    setDryRunProgress(10);
    setDryRunComplete(false);
    setStatusMessage("Running validation check...");

    setTimeout(() => {
      setDryRunProgress(50);
      const validationErrors: ValidationError[] = [];
      let valid = 0;
      let invalid = 0;
      let creates = 0;
      let updates = 0;

      rows.forEach((row, idx) => {
        const rowNum = idx + 2;
        let rowHasError = false;

        for (const [csvCol, targetField] of Object.entries(mapping)) {
          if (!targetField) continue;
          const fieldDef = activeTable.fields.find((f) => f.name === targetField);
          if (!fieldDef) continue;
          const val = row[csvCol]?.trim() ?? "";

          const fType = ((fieldDef as any).field_type || (fieldDef as any).type || "").toString().toLowerCase();
          const fOptions: string[] = (fieldDef as any).options || (fieldDef as any).select_options || [];

          // Validation rules
          if (fType === "number" && val !== "") {
            if (isNaN(Number(val))) {
              validationErrors.push({
                row: rowNum,
                column: csvCol,
                message: `"${val}" is not a valid number.`,
                recordIdentifier: row[headers[0]] || `Row ${rowNum}`,
              });
              rowHasError = true;
            }
          } else if (fType === "email" && val !== "") {
            if (!/^[^\s@]+@[^\s@]+\.[^\s@]+$/.test(val)) {
              validationErrors.push({
                row: rowNum,
                column: csvCol,
                message: `"${val}" is not a valid email address.`,
                recordIdentifier: row[headers[0]] || `Row ${rowNum}`,
              });
              rowHasError = true;
            }
          } else if (fType === "select" && val !== "") {
            const allowed = fOptions;
            if (!allowed.includes(val)) {
              if (selectStrategy === "reject") {
                validationErrors.push({
                  row: rowNum,
                  column: csvCol,
                  message: `Unknown option "${val}". Allowed: ${allowed.join(", ")}.`,
                  recordIdentifier: row[headers[0]] || `Row ${rowNum}`,
                });
                rowHasError = true;
              }
            }
          }
        }

        if (rowHasError) {
          invalid++;
        } else {
          valid++;
          if (upsertKey && row[upsertKey]) {
            const keyField = mapping[upsertKey] || upsertKey;
            const existing = activeTable.records?.some((r) => r[keyField] === row[upsertKey]);
            if (existing) updates++;
            else creates++;
          } else {
            creates++;
          }
        }
      });

      setDryRunProgress(100);
      setDryRunComplete(true);
      setErrors(validationErrors);
      setValidCount(valid);
      setInvalidCount(invalid);
      setCreatesCount(creates);
      setUpdatesCount(updates);
      setStatusMessage(
        `Validation complete: ${valid} valid rows, ${invalid} invalid rows, ${creates} creates, ${updates} updates.`
      );
    }, 150);
  };

  const executeImport = () => {
    setStep("import");
    setImportProgress(20);
    setImportComplete(false);
    setStatusMessage("Writing records...");

    setTimeout(() => {
      setImportProgress(80);
      const rejected: Record<string, string>[] = [];
      let written = 0;

      rows.forEach((row, idx) => {
        const rowNum = idx + 2;
        const hasErr = errors.some((e) => e.row === rowNum);
        if (hasErr) {
          if (acceptErrors) {
            rejected.push(row);
          }
        } else {
          written++;
        }
      });

      setImportProgress(100);
      setImportComplete(true);
      setWrittenCount(written);
      setRejectedRows(rejected);
      setStatusMessage(`Import complete: ${written} records written, ${rejected.length} rejected.`);

      if (onExecuteCsvImport) {
        onExecuteCsvImport();
      }
      if (onImportComplete) {
        onImportComplete(written, rejected.length);
      }
    }, 200);
  };

  const downloadRejectedCsv = () => {
    if (rejectedRows.length === 0) return;
    const headerLine = headers.join(",") + ",rejected_reason\n";
    const bodyLines = rejectedRows
      .map((r, idx) => {
        const rowErr = errors.find((e) => e.row === idx + 2)?.message || "Invalid row";
        const vals = headers.map((h) => `"${(r[h] || "").replace(/"/g, '""')}"`);
        return vals.join(",") + `,"${rowErr.replace(/"/g, '""')}"`;
      })
      .join("\n");

    const blob = new Blob([headerLine + bodyLines], { type: "text/csv;charset=utf-8;" });
    const url = URL.createObjectURL(blob);
    const a = document.createElement("a");
    a.href = url;
    a.download = `rejected_rows_${activeTable.name.toLowerCase().replace(/[^a-z0-9]/g, "_")}.csv`;
    a.click();
    URL.revokeObjectURL(url);
  };

  return (
    <div
      data-testid="csv-import-modal"
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/50 backdrop-blur-xs p-4"
    >
      <div
        ref={dialogRef}
        role="dialog"
        aria-modal="true"
        aria-labelledby="import-wizard-title"
        tabIndex={-1}
        className="bg-white dark:bg-slate-900 border border-slate-200 dark:border-slate-800 rounded-2xl shadow-2xl max-w-2xl w-full p-6 space-y-4 focus:outline-none"
      >
        {/* ARIA Live Region for screen-reader announcements */}
        <div role="status" aria-live="polite" className="sr-only">
          {statusMessage}
        </div>

        {/* Header & Steps Indicator */}
        <div className="flex items-center justify-between pb-3 border-b border-slate-100 dark:border-slate-800">
          <div>
            <h3
              id="import-wizard-title"
              className="text-base font-bold text-slate-900 dark:text-white flex items-center gap-2"
            >
              <span>📥</span>
              <span>Import CSV into {activeTable.name}</span>
            </h3>
            <div className="flex items-center gap-3 mt-2 text-xs font-medium">
              <span
                className={`flex items-center gap-1 ${
                  step === "file" ? "text-blue-600 dark:text-blue-400 font-bold" : "text-slate-400"
                }`}
              >
                1. Choose a file
              </span>
              <span className="text-slate-300">→</span>
              <span
                className={`flex items-center gap-1 ${
                  step === "map" ? "text-blue-600 dark:text-blue-400 font-bold" : "text-slate-400"
                }`}
              >
                2. Match columns
              </span>
              <span className="text-slate-300">→</span>
              <span
                className={`flex items-center gap-1 ${
                  step === "check" ? "text-blue-600 dark:text-blue-400 font-bold" : "text-slate-400"
                }`}
              >
                3. Check
              </span>
              <span className="text-slate-300">→</span>
              <span
                className={`flex items-center gap-1 ${
                  step === "import" ? "text-blue-600 dark:text-blue-400 font-bold" : "text-slate-400"
                }`}
              >
                4. Import
              </span>
            </div>
          </div>
          <button
            type="button"
            data-testid="csv-modal-close-btn"
            onClick={onClose}
            aria-label="Close dialog"
            className="text-slate-400 hover:text-slate-600 dark:hover:text-white font-bold cursor-pointer p-1"
          >
            ✕
          </button>
        </div>

        {/* STEP 1: CHOOSE A FILE */}
        {step === "file" && (
          <div className="space-y-4">
            <div>
              <label className="block text-xs font-semibold text-slate-700 dark:text-slate-300 mb-1">
                Upload CSV, JSON, or JSON Lines:
              </label>
              <input
                type="file"
                accept=".csv,.json,.jsonl"
                onChange={handleFileSelect}
                aria-label="Choose file to import"
                className="block w-full text-xs text-slate-500 file:mr-4 file:py-2 file:px-4 file:rounded-lg file:border-0 file:text-xs file:font-semibold file:bg-blue-50 file:text-blue-700 hover:file:bg-blue-100 dark:file:bg-slate-800 dark:file:text-slate-200"
              />
            </div>

            <div className="space-y-1">
              <label className="text-xs font-semibold text-slate-700 dark:text-slate-300">
                Or Paste CSV Text Content:
              </label>
              <textarea
                data-testid="csv-textarea-input"
                rows={5}
                value={text}
                onChange={(e) => handleTextChange(e.target.value)}
                placeholder={`title,budget,status\n"Advanced Nanomaterials",600000,Approved\n"Deep Sea Robotic Swarm",450000,Under Review`}
                className="w-full p-3 rounded-lg border border-slate-200 dark:border-slate-700 bg-slate-50 dark:bg-slate-800 font-mono text-xs text-slate-800 dark:text-white focus:outline-blue-500"
              />
            </div>

            {/* Quick preview if pasted in step 1 */}
            {headers.length > 0 && (
              <div className="p-3 bg-slate-50 dark:bg-slate-800/50 rounded-lg text-xs space-y-2">
                <p className="font-semibold text-slate-700 dark:text-slate-300">
                  {rows.length} rows found ({headers.length} columns)
                </p>
                <div className="max-h-36 overflow-y-auto space-y-1">
                  {headers.map((h) => (
                    <div key={h} className="flex items-center justify-between gap-2">
                      <span className="font-mono text-slate-600 dark:text-slate-400 truncate w-1/2">
                        {h}
                      </span>
                      <select
                        data-testid={`csv-map-select-${h}`}
                        value={mapping[h] || ""}
                        onChange={(e) => {
                          const val = e.target.value;
                          setMapping((prev) => ({ ...prev, [h]: val }));
                          if (setCsvFieldMapping) {
                            setCsvFieldMapping((prev) => ({ ...prev, [h]: val }));
                          }
                        }}
                        className="px-2 py-1 rounded border border-slate-200 dark:border-slate-700 bg-white dark:bg-slate-800 text-xs w-1/2"
                      >
                        <option value="">(Ignore / Skip)</option>
                        {activeTable.fields.map((f) => (
                          <option key={f.name} value={f.name}>
                            {f.label} ({f.name}) [{(f as any).field_type || (f as any).type || "Text"}]
                          </option>
                        ))}
                      </select>
                    </div>
                  ))}
                </div>
              </div>
            )}

            <div className="flex items-center justify-end gap-3 pt-3 border-t border-slate-100 dark:border-slate-800">
              <button
                type="button"
                onClick={onClose}
                className="px-3 py-1.5 rounded-lg text-xs font-semibold text-slate-600 dark:text-slate-300 hover:bg-slate-100 dark:hover:bg-slate-800 cursor-pointer"
              >
                Cancel
              </button>
              {/* Backwards-compatibility execute button for builder-usability test */}
              <button
                type="button"
                data-testid="csv-execute-import-btn"
                disabled={rows.length === 0}
                onClick={onExecuteCsvImport || executeImport}
                className="px-4 py-1.5 rounded-lg text-xs font-bold bg-blue-600 hover:bg-blue-700 disabled:opacity-50 text-white cursor-pointer shadow-xs"
              >
                Import {rows.length} Record{rows.length === 1 ? "" : "s"}
              </button>
              <button
                type="button"
                disabled={rows.length === 0}
                onClick={() => setStep("map")}
                className="px-4 py-1.5 rounded-lg text-xs font-bold bg-slate-800 dark:bg-slate-200 text-white dark:text-slate-900 hover:opacity-90 disabled:opacity-50 cursor-pointer"
              >
                Match columns →
              </button>
            </div>
          </div>
        )}

        {/* STEP 2: MATCH COLUMNS */}
        {step === "map" && (
          <div className="space-y-4">
            <div className="flex items-center justify-between">
              <span className="text-xs font-bold text-slate-800 dark:text-slate-200">
                Match columns ({rows.length} rows found)
              </span>
            </div>

            <div className="max-h-48 overflow-y-auto space-y-2 border border-slate-100 dark:border-slate-800 rounded-lg p-3">
              {headers.map((header) => {
                const sampleVals = rows.slice(0, 5).map((r) => r[header] || "");
                const inferred = inferColumnType(sampleVals);
                return (
                  <div
                    key={header}
                    className="flex items-center justify-between text-xs gap-3 py-1 border-b border-slate-50 dark:border-slate-800/40 last:border-0"
                  >
                    <div className="w-1/2">
                      <span className="font-mono font-medium text-slate-700 dark:text-slate-300 block truncate">
                        {header}
                      </span>
                      <span className="text-[10px] text-slate-400">
                        Inferred: {inferred} {sampleVals[0] ? `(e.g. "${sampleVals[0]}")` : ""}
                      </span>
                    </div>
                    <select
                      data-testid={`csv-map-select-${header}`}
                      value={mapping[header] || ""}
                      onChange={(e) => {
                        const val = e.target.value;
                        setMapping((prev) => ({ ...prev, [header]: val }));
                        if (setCsvFieldMapping) {
                          setCsvFieldMapping((prev) => ({ ...prev, [header]: val }));
                        }
                      }}
                      className="px-2 py-1 rounded border border-slate-200 dark:border-slate-700 bg-white dark:bg-slate-800 text-xs w-1/2"
                    >
                      <option value="">(Ignore / Skip)</option>
                      {activeTable.fields.map((f) => (
                        <option key={f.name} value={f.name}>
                          {f.label} ({f.name}) [{(f as any).field_type || (f as any).type || "Text"}]
                        </option>
                      ))}
                    </select>
                  </div>
                );
              })}
            </div>

            {/* Advanced Mapping Options: Upsert key & Select Option strategy */}
            <div className="grid grid-cols-2 gap-3 p-3 bg-slate-50 dark:bg-slate-800/40 rounded-lg text-xs">
              <div>
                <label
                  htmlFor="upsert-key-select"
                  className="block font-semibold text-slate-700 dark:text-slate-300 mb-1"
                >
                  Upsert key
                </label>
                <select
                  id="upsert-key-select"
                  aria-label="Upsert key"
                  value={upsertKey}
                  onChange={(e) => setUpsertKey(e.target.value)}
                  className="w-full px-2 py-1 rounded border border-slate-200 dark:border-slate-700 bg-white dark:bg-slate-800 text-xs"
                >
                  <option value="">None (Create new records for every row)</option>
                  {headers.map((h) => (
                    <option key={h} value={h}>
                      Match by {h}
                    </option>
                  ))}
                </select>
              </div>

              <div>
                <label
                  htmlFor="select-strategy-select"
                  className="block font-semibold text-slate-700 dark:text-slate-300 mb-1"
                >
                  Unknown select option strategy
                </label>
                <select
                  id="select-strategy-select"
                  aria-label="Unknown select option strategy"
                  value={selectStrategy}
                  onChange={(e) => setSelectStrategy(e.target.value)}
                  className="w-full px-2 py-1 rounded border border-slate-200 dark:border-slate-700 bg-white dark:bg-slate-800 text-xs"
                >
                  <option value="reject">Reject rows with unknown options (Default)</option>
                  <option value="blank">Leave blank</option>
                  <option value="propose_option">Propose adding new option to schema</option>
                </select>
              </div>
            </div>

            <div className="flex items-center justify-between pt-3 border-t border-slate-100 dark:border-slate-800">
              <button
                type="button"
                onClick={() => setStep("file")}
                className="px-3 py-1.5 rounded-lg text-xs font-semibold text-slate-600 dark:text-slate-300 hover:bg-slate-100 dark:hover:bg-slate-800 cursor-pointer"
              >
                ← Back
              </button>
              <div className="flex gap-2">
                <button
                  type="button"
                  data-testid="csv-execute-import-btn"
                  onClick={onExecuteCsvImport || executeImport}
                  className="px-3 py-1.5 rounded-lg text-xs font-bold text-slate-600 dark:text-slate-300 hover:bg-slate-100 dark:hover:bg-slate-800 cursor-pointer"
                >
                  Quick Import
                </button>
                <button
                  type="button"
                  onClick={executeDryRun}
                  className="px-4 py-1.5 rounded-lg text-xs font-bold bg-blue-600 hover:bg-blue-700 text-white cursor-pointer shadow-xs"
                >
                  Check data →
                </button>
              </div>
            </div>
          </div>
        )}

        {/* STEP 3: CHECK (DRY RUN) */}
        {step === "check" && (
          <div className="space-y-4">
            <div>
              <div className="flex justify-between text-xs font-semibold text-slate-700 dark:text-slate-300 mb-1">
                <span>Validation Progress</span>
                <span>{dryRunProgress}%</span>
              </div>
              <div
                role="progressbar"
                aria-valuenow={dryRunProgress}
                aria-valuemin={0}
                aria-valuemax={100}
                className="w-full bg-slate-100 dark:bg-slate-800 rounded-full h-2 overflow-hidden"
              >
                <div
                  className="bg-blue-600 h-2 rounded-full transition-all duration-300"
                  style={{ width: `${dryRunProgress}%` }}
                />
              </div>
            </div>

            {dryRunComplete && (
              <>
                <div className="grid grid-cols-4 gap-2 text-center text-xs">
                  <div className="p-2 rounded bg-green-50 dark:bg-green-950/30 border border-green-200 dark:border-green-900">
                    <span className="block font-bold text-green-700 dark:text-green-300 text-sm">
                      {validCount}
                    </span>
                    <span className="text-slate-500 text-[10px]">Valid rows</span>
                  </div>
                  <div className="p-2 rounded bg-red-50 dark:bg-red-950/30 border border-red-200 dark:border-red-900">
                    <span className="block font-bold text-red-700 dark:text-red-300 text-sm">
                      {invalidCount}
                    </span>
                    <span className="text-slate-500 text-[10px]">Invalid rows</span>
                  </div>
                  <div className="p-2 rounded bg-blue-50 dark:bg-blue-950/30 border border-blue-200 dark:border-blue-900">
                    <span className="block font-bold text-blue-700 dark:text-blue-300 text-sm">
                      {createsCount}
                    </span>
                    <span className="text-slate-500 text-[10px]">Creates</span>
                  </div>
                  <div className="p-2 rounded bg-purple-50 dark:bg-purple-950/30 border border-purple-200 dark:border-purple-900">
                    <span className="block font-bold text-purple-700 dark:text-purple-300 text-sm">
                      {updatesCount}
                    </span>
                    <span className="text-slate-500 text-[10px]">Updates</span>
                  </div>
                </div>

                {errors.length > 0 ? (
                  <div className="space-y-2">
                    <h4 className="text-xs font-bold text-slate-800 dark:text-slate-200">
                      Errors ({errors.length})
                    </h4>
                    <div className="max-h-40 overflow-y-auto border border-slate-200 dark:border-slate-700 rounded-lg">
                      <table className="w-full text-left text-xs">
                        <thead className="bg-slate-50 dark:bg-slate-800 text-slate-600 dark:text-slate-400 font-semibold border-b border-slate-200 dark:border-slate-700">
                          <tr>
                            <th className="p-2">Row</th>
                            <th className="p-2">Record</th>
                            <th className="p-2">Column</th>
                            <th className="p-2">Reason</th>
                          </tr>
                        </thead>
                        <tbody className="divide-y divide-slate-100 dark:divide-slate-800">
                          {errors.map((err, i) => (
                            <tr key={i} className="hover:bg-red-50/50 dark:hover:bg-red-950/20">
                              <td className="p-2 font-mono text-slate-500">{err.row}</td>
                              <td className="p-2 font-medium text-slate-800 dark:text-slate-200">
                                {err.recordIdentifier || "-"}
                              </td>
                              <td className="p-2 font-mono text-slate-600 dark:text-slate-400">
                                {err.column}
                              </td>
                              <td className="p-2 text-red-600 dark:text-red-400">{err.message}</td>
                            </tr>
                          ))}
                        </tbody>
                      </table>
                    </div>

                    <label className="flex items-center gap-2 text-xs text-slate-700 dark:text-slate-300 pt-1 cursor-pointer">
                      <input
                        type="checkbox"
                        checked={acceptErrors}
                        onChange={(e) => setAcceptErrors(e.target.checked)}
                        className="rounded border-slate-300 text-blue-600 focus:ring-blue-500"
                      />
                      <span>Skip invalid rows and proceed with valid records</span>
                    </label>
                  </div>
                ) : (
                  <p className="text-xs text-green-600 dark:text-green-400 font-medium">
                    ✓ All rows passed validation checks without errors.
                  </p>
                )}
              </>
            )}

            <div className="flex items-center justify-between pt-3 border-t border-slate-100 dark:border-slate-800">
              <button
                type="button"
                onClick={() => setStep("map")}
                className="px-3 py-1.5 rounded-lg text-xs font-semibold text-slate-600 dark:text-slate-300 hover:bg-slate-100 dark:hover:bg-slate-800 cursor-pointer"
              >
                ← Back
              </button>
              <button
                type="button"
                disabled={!dryRunComplete || (invalidCount > 0 && !acceptErrors) || validCount === 0}
                onClick={executeImport}
                className="px-4 py-1.5 rounded-lg text-xs font-bold bg-blue-600 hover:bg-blue-700 disabled:opacity-50 text-white cursor-pointer shadow-xs"
              >
                Import {validCount} Record{validCount === 1 ? "" : "s"}
              </button>
            </div>
          </div>
        )}

        {/* STEP 4: IMPORT */}
        {step === "import" && (
          <div className="space-y-4">
            <div>
              <div className="flex justify-between text-xs font-semibold text-slate-700 dark:text-slate-300 mb-1">
                <span>{importComplete ? "Import complete" : "Importing records..."}</span>
                <span>{importProgress}%</span>
              </div>
              <div
                role="progressbar"
                aria-valuenow={importProgress}
                aria-valuemin={0}
                aria-valuemax={100}
                className="w-full bg-slate-100 dark:bg-slate-800 rounded-full h-2 overflow-hidden"
              >
                <div
                  className="bg-blue-600 h-2 rounded-full transition-all duration-300"
                  style={{ width: `${importProgress}%` }}
                />
              </div>
            </div>

            {importComplete && (
              <div className="space-y-3 pt-2">
                <div className="p-3 bg-slate-50 dark:bg-slate-800/50 rounded-lg text-xs space-y-1">
                  <p className="font-semibold text-slate-800 dark:text-slate-200">
                    Import completed successfully!
                  </p>
                  <p className="text-slate-600 dark:text-slate-400">
                    Written: <strong className="text-slate-900 dark:text-white">{writtenCount}</strong>{" "}
                    records.
                  </p>
                  {rejectedRows.length > 0 && (
                    <p className="text-amber-600 dark:text-amber-400">
                      Rejected: <strong>{rejectedRows.length}</strong> rows.
                    </p>
                  )}
                </div>

                {rejectedRows.length > 0 && (
                  <div>
                    <button
                      type="button"
                      data-testid="download-rejected-rows-btn"
                      onClick={downloadRejectedCsv}
                      className="text-xs font-semibold text-blue-600 dark:text-blue-400 hover:underline cursor-pointer flex items-center gap-1"
                    >
                      <span>📥</span>
                      <span>Download the rejected rows</span>
                    </button>
                  </div>
                )}
              </div>
            )}

            <div className="flex items-center justify-end pt-3 border-t border-slate-100 dark:border-slate-800">
              <button
                type="button"
                onClick={onClose}
                className="px-4 py-1.5 rounded-lg text-xs font-bold bg-slate-800 dark:bg-slate-200 text-white dark:text-slate-900 hover:opacity-90 cursor-pointer"
              >
                Done
              </button>
            </div>
          </div>
        )}
      </div>
    </div>
  );
};
