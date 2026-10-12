import React from "react";
import { FormField } from "./ui/FormField";
import { Markdown } from "./ui/Markdown";

export interface LongTextEditorProps {
  id: string;
  label: string;
  value: string;
  onChange: (value: string) => void;
  isEditing?: boolean;
  onToggleEdit?: () => void;
  maxLength?: number;
  className?: string;
}

export const LongTextEditor: React.FC<LongTextEditorProps> = ({
  id,
  label,
  value,
  onChange,
  isEditing = false,
  onToggleEdit,
  maxLength = 100000,
  className = "",
}) => {
  const currentLength = (value || "").length;

  if (!isEditing) {
    return (
      <div className={`space-y-1.5 ${className}`}>
        <div className="flex items-center justify-between">
          <label className="block text-xs font-semibold text-slate-700 dark:text-slate-300">
            {label}
          </label>
          {onToggleEdit && (
            <button
              type="button"
              onClick={onToggleEdit}
              className="text-xs text-blue-600 hover:text-blue-700 dark:text-blue-400 font-medium cursor-pointer"
            >
              Edit
            </button>
          )}
        </div>
        <div
          data-testid={`rendered-markdown-${id}`}
          className="p-2.5 rounded-lg border border-slate-200 dark:border-slate-700 bg-white dark:bg-slate-900 min-h-[4rem]"
        >
          {value ? (
            <Markdown content={value} />
          ) : (
            <span className="text-slate-400 italic text-xs">Empty</span>
          )}
        </div>
      </div>
    );
  }

  return (
    <div className={`space-y-1.5 ${className}`}>
      <FormField id={id} label={label}>
        {(inputProps) => (
          <div className="space-y-1">
            <textarea
              {...inputProps}
              aria-label={label}
              value={value || ""}
              onChange={(e) => onChange(e.target.value)}
              rows={5}
              maxLength={maxLength}
              className="w-full px-2.5 py-1.5 rounded-lg border border-slate-200 dark:border-slate-700 bg-white dark:bg-slate-900 text-xs text-slate-800 dark:text-white font-sans focus:outline-none focus:ring-2 focus:ring-blue-500"
              placeholder="Enter markdown text..."
            />
            <div className="flex items-center justify-between text-[11px] text-slate-400">
              <span data-testid="long-text-syntax-help">
                Markdown: **bold**, *italic*, [link](url), - list, 1. list, `code`
              </span>
              <span data-testid="long-text-char-count" className="font-mono">
                {currentLength.toLocaleString()} / {maxLength.toLocaleString()}
              </span>
            </div>
          </div>
        )}
      </FormField>
    </div>
  );
};
