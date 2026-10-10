import type { ReactNode } from "react";

export interface FormFieldProps {
  id: string;
  label: string;
  hint?: string;
  error?: string;
  required?: boolean;
  children: (inputProps: {
    id: string;
    "aria-describedby"?: string;
    "aria-invalid"?: boolean;
    required?: boolean;
  }) => ReactNode;
}

export function FormField({
  id,
  label,
  hint,
  error,
  required,
  children,
}: FormFieldProps) {
  const hintId = hint ? `${id}-hint` : undefined;
  const errorId = error ? `${id}-error` : undefined;
  const describedBy = [errorId, hintId].filter(Boolean).join(" ") || undefined;

  return (
    <div className="space-y-1.5">
      <label
        htmlFor={id}
        className="block text-sm font-medium text-gray-700 dark:text-gray-300"
      >
        {label}
        {required && (
          <span className="text-red-500 ml-1" aria-hidden="true">
            *
          </span>
        )}
      </label>
      {hint && (
        <p id={hintId} className="text-xs text-gray-500 dark:text-gray-400">
          {hint}
        </p>
      )}
      {children({
        id,
        "aria-describedby": describedBy,
        "aria-invalid": Boolean(error),
        required,
      })}
      {error && (
        <p id={errorId} className="text-xs font-medium text-red-600 dark:text-red-400">
          {error}
        </p>
      )}
    </div>
  );
}
