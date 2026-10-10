import { useState } from "react";
import { explainError } from "./explainError";

export interface ErrorStateProps {
  /** Give an error and the message, next step, code, and status come from it. */
  error?: unknown;
  message?: string;
  next?: string;
  code?: string;
  status?: number;
  onRetry?: () => void;
  details?: string;
}

export function ErrorState({
  error,
  message: givenMessage,
  next: givenNext,
  code: givenCode,
  status: givenStatus,
  onRetry,
  details,
}: ErrorStateProps) {
  const [showDetails, setShowDetails] = useState(false);
  const explained = error !== undefined ? explainError(error) : undefined;
  const message = givenMessage ?? explained?.message ?? "Something went wrong.";
  const next = givenNext ?? explained?.next;
  const failure = error as { code?: string; status?: number } | undefined;
  const code = givenCode ?? failure?.code;
  const status = givenStatus ?? failure?.status;

  return (
    <div
      role="alert"
      className="p-4 rounded-lg bg-red-50 dark:bg-red-950/30 border border-red-200 dark:border-red-900 text-red-900 dark:text-red-200"
    >
      <div className="flex items-start gap-3">
        <span className="text-red-600 dark:text-red-400 text-lg leading-none select-none" aria-hidden="true">
          ⚠
        </span>
        <div className="flex-1">
          <p className="font-semibold text-sm">{message}</p>
          {next && (
            <p className="mt-1 text-sm text-red-700 dark:text-red-300">
              {next}
            </p>
          )}

          {(code || status !== undefined || details) && (
            <div className="mt-3">
              <button
                type="button"
                onClick={() => setShowDetails(!showDetails)}
                className="text-xs font-medium text-red-700 dark:text-red-300 underline hover:text-red-800"
              >
                {showDetails ? "Hide details" : "Details"}
              </button>
              {showDetails && (
                <div className="mt-2 text-xs font-mono bg-red-100/50 dark:bg-red-900/40 p-2 rounded border border-red-200 dark:border-red-800 space-y-1">
                  {status !== undefined && <div>Status: {status}</div>}
                  {code && <div>Code: {code}</div>}
                  {details && <div>{details}</div>}
                </div>
              )}
            </div>
          )}

          {onRetry && (
            <div className="mt-3">
              <button
                type="button"
                onClick={onRetry}
                className="px-3 py-1.5 text-xs font-medium bg-red-600 hover:bg-red-700 text-white rounded transition shadow-sm"
              >
                Retry
              </button>
            </div>
          )}
        </div>
      </div>
    </div>
  );
}
