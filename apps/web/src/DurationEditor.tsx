import React, { useState, useEffect } from "react";
import { parseDuration, formatDuration, describeDuration } from "./utils/durationDatetime";

export interface DurationEditorProps {
  id: string;
  label: string;
  value: number | null | undefined;
  format?: string;
  onChange: (value: number | null) => void;
  disabled?: boolean;
  className?: string;
}

export const DurationEditor: React.FC<DurationEditorProps> = ({
  id,
  label,
  value,
  format = "h:mm",
  onChange,
  disabled = false,
  className = "",
}) => {
  const initialText = value !== null && value !== undefined ? formatDuration(value, format) : "";
  const [text, setText] = useState<string>(initialText);
  const [isEditing, setIsEditing] = useState<boolean>(false);

  useEffect(() => {
    if (!isEditing) {
      setText(value !== null && value !== undefined ? formatDuration(value, format) : "");
    }
  }, [value, format, isEditing]);

  const parsed = text.trim() === "" ? null : parseDuration(text);
  const isInvalid = text.trim() !== "" && parsed === null;

  const handleBlur = () => {
    setIsEditing(false);
    if (text.trim() === "") {
      onChange(null);
    } else if (parsed !== null) {
      onChange(parsed);
      setText(formatDuration(parsed, format));
    }
    // If invalid, do not commit
  };

  const handleKeyDown = (e: React.KeyboardEvent<HTMLInputElement>) => {
    if (e.key === "Enter") {
      e.currentTarget.blur();
    } else if (e.key === "Escape") {
      setIsEditing(false);
      setText(value !== null && value !== undefined ? formatDuration(value, format) : "");
    }
  };

  return (
    <div className={`space-y-1 ${className}`}>
      <div className="flex items-center justify-between">
        <label htmlFor={id} className="block text-xs font-medium text-slate-700 dark:text-slate-300">
          {label}
        </label>
        <span className="text-[10px] text-slate-400 font-mono uppercase">
          {format}
        </span>
      </div>

      <div className="relative">
        <input
          id={id}
          type="text"
          value={text}
          disabled={disabled}
          placeholder="e.g. 1:30, 90m, 1.5h, 2d"
          onChange={(e) => {
            setIsEditing(true);
            setText(e.target.value);
          }}
          onBlur={handleBlur}
          onKeyDown={handleKeyDown}
          aria-invalid={isInvalid}
          aria-describedby={`${id}-feedback`}
          className={`w-full px-3 py-1.5 text-xs rounded-lg border bg-white dark:bg-slate-900 text-slate-900 dark:text-slate-100 transition-colors focus:outline-none focus:ring-2 ${
            isInvalid
              ? "border-red-400 dark:border-red-600 focus:ring-red-500/20"
              : "border-slate-300 dark:border-slate-700 focus:border-blue-500 focus:ring-blue-500/20"
          } ${disabled ? "opacity-60 cursor-not-allowed" : ""}`}
        />
      </div>

      <div id={`${id}-feedback`} className="text-[11px] min-h-[16px] transition-all">
        {isInvalid ? (
          <span className="text-red-600 dark:text-red-400 font-medium">
            Invalid duration format (e.g. 1:30, 90m, 1.5h, 2d)
          </span>
        ) : parsed !== null ? (
          <span className="text-blue-600 dark:text-blue-400 font-medium">
            Interpreted as: {describeDuration(parsed)}
          </span>
        ) : (
          <span className="text-slate-400">
            Accepts 1:30, 90m, 1.5h, 2d
          </span>
        )}
      </div>
    </div>
  );
};
