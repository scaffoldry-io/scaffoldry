import { useState, type FormEvent } from "react";
import { ErrorState } from "./ErrorState";

export interface ConfirmActionProps {
  verb: string;
  target: string;
  consequence: string;
  audited?: boolean;
  requireReason?: boolean;
  onConfirm: (reason: string) => Promise<void>;
  onSuccess?: () => void;
}

export function ConfirmAction({
  verb,
  target,
  consequence,
  audited = true,
  requireReason = true,
  onConfirm,
  onSuccess,
}: ConfirmActionProps) {
  const [reason, setReason] = useState("");
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const isValid = !requireReason || reason.trim().length > 0;

  const handleSubmit = async (e: FormEvent) => {
    e.preventDefault();
    if (!isValid || loading) return;

    setLoading(true);
    setError(null);
    try {
      await onConfirm(reason);
      onSuccess?.();
    } catch (err: unknown) {
      setError(err instanceof Error ? err.message : "Action failed");
    } finally {
      setLoading(false);
    }
  };

  const buttonLabel = `${verb} ${target}`;

  return (
    <form onSubmit={handleSubmit} className="space-y-4">
      <div className="text-sm text-gray-700 dark:text-gray-300">
        <p>{consequence}</p>
        {audited && (
          <p className="mt-1 text-xs text-gray-500 dark:text-gray-400 font-medium">
            This is recorded in the audit ledger.
          </p>
        )}
      </div>

      {error && (
        <ErrorState
          message={error}
          onRetry={() => handleSubmit({ preventDefault: () => {} } as FormEvent)}
        />
      )}

      {requireReason && (
        <div className="space-y-1">
          <label
            htmlFor="confirm-reason"
            className="block text-xs font-medium text-gray-700 dark:text-gray-300"
          >
            Reason for decision <span className="text-red-500">*</span>
          </label>
          <textarea
            id="confirm-reason"
            rows={2}
            value={reason}
            onChange={(e) => setReason(e.target.value)}
            disabled={loading}
            placeholder="Explain the administrative or operational reason..."
            className="w-full text-sm rounded border border-gray-300 dark:border-gray-700 p-2 bg-white dark:bg-gray-800 text-gray-900 dark:text-gray-100 disabled:opacity-50"
          />
        </div>
      )}

      <div className="flex justify-end gap-2">
        <button
          type="submit"
          disabled={!isValid || loading}
          className="inline-flex items-center gap-2 px-4 py-2 text-sm font-medium rounded-md bg-blue-600 text-white hover:bg-blue-700 disabled:opacity-50 disabled:cursor-not-allowed transition shadow-sm"
        >
          {loading && (
            <span
              className="inline-block w-3.5 h-3.5 border-2 border-white border-t-transparent rounded-full animate-spin"
              aria-hidden="true"
            />
          )}
          <span>{buttonLabel}</span>
        </button>
      </div>
    </form>
  );
}
