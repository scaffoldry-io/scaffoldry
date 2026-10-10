import { useEffect } from "react";
import { explainError } from "./explainError";

export type ToastTone = "success" | "error";

export interface ToastProps {
  tone: ToastTone;
  /** For a failure, give the error instead and the message and next step come from it. */
  error?: unknown;
  message?: string;
  onClose: () => void;
  autoDismissMs?: number;
}

export function Toast({
  tone,
  error,
  message: givenMessage,
  onClose,
  autoDismissMs = 4000,
}: ToastProps) {
  const explained = error !== undefined ? explainError(error) : undefined;
  const message = givenMessage ?? [explained?.message, explained?.next].filter(Boolean).join(" ");
  useEffect(() => {
    if (tone === "success" && autoDismissMs) {
      const timer = setTimeout(onClose, autoDismissMs);
      return () => clearTimeout(timer);
    }
  }, [tone, autoDismissMs, onClose]);

  const role = tone === "error" ? "alert" : "status";
  const ariaLive = tone === "error" ? "assertive" : "polite";

  return (
    <div
      role={role}
      aria-live={ariaLive}
      className={`fixed bottom-4 right-4 z-50 flex items-center gap-3 px-4 py-3 rounded-lg shadow-lg border text-sm max-w-md ${
        tone === "error"
          ? "bg-red-50 text-red-900 border-red-300 dark:bg-red-950 dark:text-red-200 dark:border-red-800"
          : "bg-emerald-50 text-emerald-900 border-emerald-300 dark:bg-emerald-950 dark:text-emerald-200 dark:border-emerald-800"
      }`}
    >
      <span className="font-bold select-none" aria-hidden="true">
        {tone === "error" ? "✕" : "✓"}
      </span>
      <span className="flex-1">{message}</span>
      <button
        type="button"
        onClick={onClose}
        aria-label="Dismiss notification"
        className="text-gray-500 hover:text-gray-700 dark:text-gray-400 dark:hover:text-gray-200 font-bold ml-2 select-none"
      >
        ×
      </button>
    </div>
  );
}
