import React from "react";
import { AppTable } from "./types";

export interface CsvImportModalProps {
  isOpen: boolean;
  onClose: () => void;
  activeTable: AppTable;
  csvRawText: string;
  onParseCsvText: (text: string) => void;
  csvParsedHeaders: string[];
  csvParsedRows: Record<string, string>[];
  csvFieldMapping: Record<string, string>;
  setCsvFieldMapping: React.Dispatch<React.SetStateAction<Record<string, string>>>;
  onExecuteCsvImport: () => void;
}

export const CsvImportModal: React.FC<CsvImportModalProps> = ({
  isOpen,
  onClose,
  activeTable,
  csvRawText,
  onParseCsvText,
  csvParsedHeaders,
  csvParsedRows,
  csvFieldMapping,
  setCsvFieldMapping,
  onExecuteCsvImport,
}) => {
  if (!isOpen) return null;

  return (
    <div
      data-testid="csv-import-modal"
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/50 backdrop-blur-xs p-4"
    >
      <div className="bg-white dark:bg-slate-900 border border-slate-200 dark:border-slate-800 rounded-2xl shadow-2xl max-w-xl w-full p-6 space-y-4">
        <div className="flex items-center justify-between pb-3 border-b border-slate-100 dark:border-slate-800">
          <div>
            <h3 className="text-base font-bold text-slate-900 dark:text-white flex items-center gap-2">
              <span>📥</span>
              <span>Import CSV into {activeTable.name}</span>
            </h3>
            <p className="text-xs text-slate-500 mt-0.5">
              Paste standard RFC 4180 CSV data to map columns and import records.
            </p>
          </div>
          <button
            type="button"
            data-testid="csv-modal-close-btn"
            onClick={onClose}
            className="text-slate-400 hover:text-slate-600 dark:hover:text-white font-bold cursor-pointer"
          >
            ✕
          </button>
        </div>

        <div className="space-y-2">
          <label className="text-xs font-semibold text-slate-700 dark:text-slate-300">
            CSV Text Content:
          </label>
          <textarea
            data-testid="csv-textarea-input"
            rows={5}
            value={csvRawText}
            onChange={(e) => onParseCsvText(e.target.value)}
            placeholder={`title,budget,status\n"Advanced Nanomaterials",600000,Approved\n"Deep Sea Robotic Swarm",450000,Under Review`}
            className="w-full p-3 rounded-lg border border-slate-200 dark:border-slate-700 bg-slate-50 dark:bg-slate-800 font-mono text-xs text-slate-800 dark:text-white focus:outline-blue-500"
          />
        </div>

        {csvParsedHeaders.length > 0 && (
          <div className="space-y-3 pt-2">
            <div className="flex items-center justify-between">
              <span className="text-xs font-bold text-slate-800 dark:text-slate-200">
                Map Columns ({csvParsedRows.length} rows found)
              </span>
            </div>
            <div className="max-h-48 overflow-y-auto space-y-2 border border-slate-100 dark:border-slate-800 rounded-lg p-3">
              {csvParsedHeaders.map((header) => (
                <div
                  key={header}
                  className="flex items-center justify-between text-xs gap-3"
                >
                  <span className="font-mono font-medium text-slate-700 dark:text-slate-300 truncate w-1/2">
                    {header}
                  </span>
                  <select
                    data-testid={`csv-map-select-${header}`}
                    value={csvFieldMapping[header] || ""}
                    onChange={(e) =>
                      setCsvFieldMapping((prev) => ({
                        ...prev,
                        [header]: e.target.value,
                      }))
                    }
                    className="px-2 py-1 rounded border border-slate-200 dark:border-slate-700 bg-white dark:bg-slate-800 text-xs w-1/2"
                  >
                    <option value="">(Ignore / Skip)</option>
                    {activeTable.fields.map((f) => (
                      <option key={f.name} value={f.name}>
                        {f.label} ({f.name})
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
          <button
            type="button"
            data-testid="csv-execute-import-btn"
            disabled={csvParsedRows.length === 0}
            onClick={onExecuteCsvImport}
            className="px-4 py-1.5 rounded-lg text-xs font-bold bg-blue-600 hover:bg-blue-700 disabled:opacity-50 text-white cursor-pointer shadow-xs"
          >
            Import {csvParsedRows.length} Record
            {csvParsedRows.length === 1 ? "" : "s"}
          </button>
        </div>
      </div>
    </div>
  );
};
