import { useState } from "react";
import { apiClient, type AccessRequest, type AccessRequestBody } from "../api";
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

/** What came of a request, in a sentence. */
function outcome(r: AccessRequest): string {
  if (r.status === "Approved") return r.route === "self" ? "Done. This session can now do it." : `You have access for ${r.days} days.`;
  if (r.status === "Waiting for person") return "Confirm the request in your requests before it goes on.";
  const who = r.waiting_on && r.waiting_on.length > 0 ? r.waiting_on.join(", ") : "an approver";
  return `Your request waits for ${who}.`;
}

/** The step after a refusal: ask for what was withheld. The server filled in the request; the person says why. */
function AskForAccess({ request }: { request: AccessRequestBody }) {
  const [open, setOpen] = useState(false);
  const [reason, setReason] = useState(request.reason);
  const [busy, setBusy] = useState(false);
  const [failure, setFailure] = useState<string | null>(null);
  const [result, setResult] = useState<AccessRequest | null>(null);

  if (result) return <p className="mt-2 text-sm">{outcome(result)}</p>;
  if (!open) {
    return (
      <div className="mt-3">
        <button type="button" onClick={() => setOpen(true)} className="px-3 py-1.5 text-xs font-medium bg-red-600 hover:bg-red-700 text-white rounded transition shadow-sm">
          Request access
        </button>
      </div>
    );
  }
  const send = async () => {
    setBusy(true);
    setFailure(null);
    try {
      setResult(await apiClient.fileAccessRequest({ ...request, reason }));
    } catch (e) {
      setFailure(e instanceof Error ? e.message : "The request could not be sent.");
    } finally {
      setBusy(false);
    }
  };
  return (
    <div className="mt-3 space-y-2 text-sm">
      <label className="block">
        <span className="text-xs font-medium">Why do you need it?</span>
        <textarea value={reason} onChange={(e) => setReason(e.target.value)} rows={2} maxLength={500} className="mt-1 w-full rounded border border-red-300 bg-white dark:bg-gray-900 p-2 text-sm" />
      </label>
      <p className="text-xs">
        {request.role} access for {request.days} days.
      </p>
      {failure && <p className="text-xs font-medium">{failure}</p>}
      <button type="button" disabled={busy} onClick={() => void send()} className="px-3 py-1.5 text-xs font-medium bg-red-600 hover:bg-red-700 disabled:opacity-60 text-white rounded transition shadow-sm">
        Send request
      </button>
    </div>
  );
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
  const failure = error as { code?: string; status?: number; policy?: { rule?: string }; request?: AccessRequestBody } | undefined;
  // A denial whose policy is bound to a library rule links to the rule. The server gives an API address.
  const ruleAddress = failure?.policy?.rule;
  const policyRule = ruleAddress?.startsWith("/library/rules/") ? ruleAddress.replace("/library/rules/", "/policies/") : undefined;
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
          {policyRule && (
            <p className="mt-1 text-sm">
              <a href={policyRule} className="underline text-red-800 dark:text-red-200">
                Read the rule behind this
              </a>
            </p>
          )}
          {next && (
            <p className="mt-1 text-sm text-red-700 dark:text-red-300">
              {next}
            </p>
          )}
          {failure?.request && <AskForAccess request={failure.request} />}

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
